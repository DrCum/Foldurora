//! World viewport state shared by rendering, HUD layout and input gating.
use super::*;
use crate::window_recovery as recovery;

#[derive(Default)]
pub(super) struct MonitorRecovery {
    window: Option<WindowId>,
    policy: recovery::Recovery,
    pointer: NativePointer,
    demo: Option<DemoRecovery>,
    waiting: Option<Observation>,
}

#[derive(Clone, PartialEq, Eq)]
struct Observation {
    displays: Vec<recovery::Display>,
    geometry: Option<recovery::Geometry>,
    blocked: bool,
}

impl Observation {
    fn read(g: &Gfx, blocked: bool) -> Self {
        let displays = connected_displays(g.window.available_monitors());
        let normal = g.shown
            && g.window.is_visible() == Some(true)
            && g.window.fullscreen().is_none()
            && !g.window.is_maximized()
            && g.window.is_minimized() == Some(false)
            && !g.renderer.vr_engaged();
        // Temporary native states do not describe the normal restore rect.
        // Keep a topology change armed until that geometry is available.
        let geometry = normal.then(|| g.window.outer_position().ok()).flatten().map(|position| {
            let (outer, inner) = (g.window.outer_size(), g.window.inner_size());
            recovery::Geometry {
                position: (position.x, position.y),
                outer: (outer.width, outer.height),
                inner: (inner.width, inner.height),
            }
        });
        Self {
            displays,
            geometry,
            blocked: blocked || !normal,
        }
    }
}

fn connected_displays(monitors: impl Iterator<Item = winit::monitor::MonitorHandle>) -> Vec<recovery::Display> {
    let mut displays = monitors
        .map(|monitor| {
            let (position, size) = (monitor.position(), monitor.size());
            recovery::Display::new((position.x, position.y), (size.width, size.height), monitor.scale_factor())
        })
        .collect::<Vec<_>>();
    displays.sort_unstable();
    displays.dedup();
    displays
}

enum PointerSample {
    #[cfg(target_os = "linux")]
    Waiting,
    Ready(Option<bool>),
}

struct DemoRecovery {
    mode: String,
    clock: Instant,
    reported: bool,
}

impl DemoRecovery {
    fn new() -> Option<Self> {
        let mode = std::env::var("AURORA_DEMO_MONITOR_RECOVERY").ok()?;
        matches!(mode.as_str(), "removed" | "partial" | "drag" | "native").then(|| Self {
            mode,
            clock: Instant::now(),
            reported: false,
        })
    }

    fn sample(&self, frame: u64) -> (Vec<recovery::Display>, recovery::Geometry, bool) {
        let mut displays = vec![recovery::Display::new((0, 0), (1600, 1000), 1.0)];
        if frame < 600 {
            displays.push(recovery::Display::new((1600, 0), (1600, 1000), 1.0));
        }
        (
            displays,
            recovery::Geometry {
                position: (if self.mode == "partial" { 1599 } else { 1800 }, 100),
                outer: (1280, 760),
                inner: (1264, 720),
            },
            self.mode == "drag" && (600..840).contains(&frame),
        )
    }
}

#[derive(Default)]
struct NativePointer {
    #[cfg(target_os = "linux")]
    x11: Option<X11Pointer>,
    #[cfg(target_os = "linux")]
    unavailable: bool,
}

impl NativePointer {
    /// Fail closed when the platform cannot establish native button state.
    fn down(&mut self, window: &Window, now: Instant) -> PointerSample {
        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
                GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON, VK_XBUTTON1, VK_XBUTTON2,
            };
            let _ = (window, now);
            // SAFETY: GetAsyncKeyState reads a virtual key's current state;
            // it neither captures input nor changes another window.
            PointerSample::Ready(Some(unsafe {
                [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2]
                    .into_iter()
                    .any(|key| GetAsyncKeyState(i32::from(key)) < 0)
            }))
        }
        #[cfg(target_os = "linux")]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let Ok(handle) = window.window_handle() else {
                return PointerSample::Ready(None);
            };
            if !matches!(handle.as_raw(), RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)) || self.unavailable {
                return PointerSample::Ready(None);
            }
            if self.x11.is_none() {
                self.x11 = X11Pointer::new();
                self.unavailable = self.x11.is_none();
            }
            self.x11.as_mut().map_or(PointerSample::Ready(None), |pointer| pointer.down(now))
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            let _ = (window, now);
            PointerSample::Ready(None)
        }
    }

    fn cancel(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(pointer) = &mut self.x11 {
            pointer.inflight = None;
        }
    }
}

/// Only while recovery is pending: ask the X server on a worker, never a
/// round trip in the frame loop. Stale or missing replies prohibit recovery.
/// Both channels hold at most one item. Dropping them never joins a worker
/// that could be blocked on a broken display server.
#[cfg(target_os = "linux")]
struct X11Pointer {
    request: crossbeam_channel::Sender<u64>,
    answer: crossbeam_channel::Receiver<(u64, Instant, Option<bool>)>,
    inflight: Option<(u64, Instant)>,
    serial: u64,
}

#[cfg(target_os = "linux")]
impl X11Pointer {
    fn new() -> Option<Self> {
        let (request, requests) = crossbeam_channel::bounded(1);
        let (answer, answers) = crossbeam_channel::bounded(1);
        std::thread::Builder::new()
            .name("display-recovery-pointer".into())
            .spawn(move || {
                use x11rb::{
                    connection::Connection,
                    protocol::xproto::{ConnectionExt, KeyButMask},
                    xcb_ffi::{XCBConnection, load_libxcb},
                };
                let connected = load_libxcb().ok().and_then(|_| XCBConnection::connect(None).ok());
                while let Ok(serial) = requests.recv() {
                    let down = connected.as_ref().and_then(|(connection, screen)| {
                        let root = connection.setup().roots.get(*screen)?.root;
                        let reply = connection.query_pointer(root).ok()?.reply().ok()?;
                        reply.same_screen.then(|| {
                            reply.mask.intersects(
                                KeyButMask::BUTTON1 | KeyButMask::BUTTON2 | KeyButMask::BUTTON3 | KeyButMask::BUTTON4 | KeyButMask::BUTTON5,
                            )
                        })
                    });
                    let _ = answer.try_send((serial, Instant::now(), down));
                }
            })
            .ok()?;
        Some(Self {
            request,
            answer: answers,
            inflight: None,
            serial: 0,
        })
    }

    fn down(&mut self, now: Instant) -> PointerSample {
        while let Ok(answer) = self.answer.try_recv() {
            if let Some(request) = self.inflight.filter(|r| r.0 == answer.0) {
                self.inflight = None;
                return PointerSample::Ready(recovery::pointer_reply(request, answer, now));
            }
        }
        if let Some((_, issued)) = self.inflight {
            if now.saturating_duration_since(issued) > Duration::from_millis(500) {
                self.inflight = None;
                return PointerSample::Ready(None);
            }
            return PointerSample::Waiting;
        }
        self.serial = self.serial.wrapping_add(1);
        if self.request.try_send(self.serial).is_err() {
            return PointerSample::Ready(None);
        }
        self.inflight = Some((self.serial, now));
        PointerSample::Waiting
    }
}

impl App {
    pub(super) fn tick_monitor_recovery(&mut self, event_loop: &ActiveEventLoop) {
        if self.host_fitted() {
            return;
        }
        let native_now = Instant::now();
        let state = &mut self.monitor_recovery;
        let Some(g) = &self.gfx else {
            if state.window.is_some() {
                state.waiting = None;
                state.pointer = NativePointer::default();
                state.policy.suspend(native_now);
                if state.policy.due(native_now) && state.demo.as_ref().is_none_or(|d| d.mode == "native") {
                    let displays = connected_displays(event_loop.available_monitors());
                    state.policy.observe(native_now, &displays, None, true);
                }
            }
            return;
        };
        let recreated = state.window != Some(g.window.id());
        if recreated {
            if state.window.is_none() {
                state.demo = self.demo.then(DemoRecovery::new).flatten();
            }
            state.window = Some(g.window.id());
            state.waiting = None;
            state.pointer = NativePointer::default();
        }
        let synthetic = state.demo.as_ref().is_some_and(|d| d.mode != "native");
        let now = state
            .demo
            .as_ref()
            .filter(|_| synthetic)
            .map_or(native_now, |d| d.clock + Duration::from_secs(self.frame_count / 60));
        if recreated {
            state.policy.window_recreated(now);
        }
        if state.waiting.is_none() && !state.policy.due(now) {
            return;
        }
        let (displays, geometry, blocked) = if let Some(demo) = state.demo.as_ref().filter(|_| synthetic) {
            let (displays, geometry, blocked) = demo.sample(self.frame_count);
            (displays, Some(geometry), blocked)
        } else {
            let fixed = state.demo.is_none() && (self.capture.is_some() || forced_window_size().is_some());
            let interacting = fixed
                || crate::window_hold::active()
                || self.left_down
                || self.right_drag
                || self.egui_ctx.input(|i| i.pointer.any_down())
                || self
                    .geom_changed
                    .is_some_and(|t| native_now.saturating_duration_since(t) < Duration::from_millis(300));
            let observation = Observation::read(g, interacting);
            let visible = observation
                .geometry
                .is_some_and(|geometry| recovery::visible(geometry, &observation.displays));
            let changed = state.waiting.as_ref().is_some_and(|requested| *requested != observation);
            let blocked = if observation.blocked || visible || changed || !state.policy.pending() {
                state.waiting = None;
                state.pointer.cancel();
                true
            } else {
                if state.waiting.is_none() {
                    state.waiting = Some(observation.clone());
                }
                match state.pointer.down(&g.window, native_now) {
                    #[cfg(target_os = "linux")]
                    PointerSample::Waiting => return,
                    PointerSample::Ready(down) => {
                        state.waiting = None;
                        down != Some(false)
                    }
                }
            };
            (observation.displays, observation.geometry, blocked)
        };
        let was_pending = state.policy.pending();
        let plan = state.policy.observe(now, &displays, geometry, blocked);
        if !was_pending && state.policy.pending() {
            log::info!("display recovery: topology changed; waiting for a stable, inaccessible normal window");
        }
        if synthetic {
            if let Some(demo) = &mut state.demo
                && !demo.reported
                && ((was_pending && !state.policy.pending()) || (demo.mode == "partial" && self.frame_count >= 600))
            {
                demo.reported = true;
                log::info!(
                    "demo display recovery: {} at frame {}: {plan:?}; actual window unchanged",
                    demo.mode,
                    self.frame_count
                );
            }
            return;
        }
        let Some(plan) = plan else { return };
        log::info!(
            "display recovery: returning an inaccessible window to {:?}, size {:?}",
            plan.position,
            plan.inner_size
        );
        // Do not focus, unminimize or leave fullscreen. Resizing can answer
        // synchronously without a Resized event on some platforms.
        let window = g.window.clone();
        if let Some((width, height)) = plan.inner_size
            && let Some(applied) = window.request_inner_size(winit::dpi::PhysicalSize::new(width, height))
            && let Some(gfx) = &mut self.gfx
        {
            gfx.renderer.resize(applied.width, applied.height);
        }
        window.set_outer_position(winit::dpi::PhysicalPosition::new(plan.position.0, plan.position.1));
        ui::menu::window_geometry_changed(&self.egui_ctx);
        self.geom_changed = Some(native_now);
        window.request_redraw();
    }

    /// Synthetic display layouts for offline demo captures; never saved.
    pub(super) fn demo_world_viewport(&mut self) {
        let Ok(mode) = std::env::var("AURORA_DEMO_VIEWPORT") else { return };
        let (mode, restrict_ui) = mode.strip_prefix("ui-").map_or((mode.as_str(), false), |mode| (mode, true));
        let mode = mode.strip_prefix("tools-").unwrap_or(mode);
        match mode {
            "left" => self.settings.world_viewport.half(false),
            "right" | "toggle" => self.settings.world_viewport.half(true),
            "inset" | "scaled" => {
                self.settings.world_viewport = crate::world_viewport::ViewportSettings {
                    enabled: true,
                    restrict_ui,
                    left: 25.0,
                    right: 10.0,
                    top: 15.0,
                    bottom: 20.0,
                };
                if mode == "scaled" {
                    self.settings.render_scale_percent = 75;
                }
            }
            _ => {}
        }
        self.settings.world_viewport.restrict_ui = restrict_ui;
    }

    pub(super) fn demo_interface_viewport(&mut self) {
        if !self.demo {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_VIEWPORT") else { return };
        if matches!(mode.as_str(), "ui-toggle" | "ui-tools-toggle") {
            let restricted = !(840..1080).contains(&self.frame_count);
            if self.settings.world_viewport.restrict_ui != restricted {
                self.settings.world_viewport.restrict_ui = restricted;
                log::info!("demo viewport interface: restricted={restricted} frame={}", self.frame_count);
            }
        }
        if mode.starts_with("ui-tools-") {
            // The floating photo panel leaves ordinary windows visible in
            // the same offline acceptance image; output still renders full-frame.
            self.settings.photo_window = crate::settings::PhotoWindowStyle::Panel;
            self.panels.inventory = true;
            self.panels.settings = true;
            self.panels.world_map = true;
            self.panels.minimap = true;
            self.panels.camera_controls = true;
            self.panels.snapshot = true;
            self.snapshots.ui.full_frame = false;
            if self.frame_count < 4 || self.frame_count == 600 {
                ui::options::demo(&mut self.options_ui, "layout");
                let native = self.egui_ctx.content_rect();
                let free_left = if mode.ends_with("left") { native.center().x } else { native.left() };
                for (id, position, size) in [
                    (
                        "inventory",
                        egui::pos2(free_left + 12.0, native.top() + 70.0),
                        egui::vec2(440.0, 580.0),
                    ),
                    (
                        ui::options::WINDOW,
                        egui::pos2(free_left + 470.0, native.top() + 70.0),
                        egui::vec2(700.0, 620.0),
                    ),
                    (
                        "worldmap",
                        egui::pos2(free_left + 12.0, native.top() + 720.0),
                        egui::vec2(650.0, 560.0),
                    ),
                    (
                        "minimap",
                        egui::pos2(free_left + 700.0, native.top() + 760.0),
                        egui::vec2(250.0, 250.0),
                    ),
                ] {
                    ui::widgets::place_floater(&self.egui_ctx, id, position, size);
                }
            }
        }
    }

    pub(super) fn effective_viewport_settings(&self) -> crate::world_viewport::ViewportSettings {
        self.settings.world_viewport.for_hosted_worker(self.host_fitted())
    }

    pub(super) fn world_viewport_for_size(&self, size: (u32, u32), vr: bool) -> aurora_render::WorldViewport {
        self.effective_viewport_settings().render_viewport(
            size,
            self.camera.mouselook(),
            self.interface.hidden,
            self.panels.snapshot || self.snapshots.capturing.is_some(),
            vr,
        )
    }

    pub(super) fn world_pointer_inside(&self) -> bool {
        self.world_viewport
            .is_none_or(|view| view.local(self.cursor_pos.0, self.cursor_pos.1).is_some())
    }

    pub(super) fn world_clip_rect(&self, ppp: f32) -> egui::Rect {
        self.world_viewport.map_or(egui::Rect::EVERYTHING, |view| {
            let [left, top, right, bottom] = view.area();
            egui::Rect::from_min_max(egui::pos2(left / ppp, top / ppp), egui::pos2(right / ppp, bottom / ppp))
        })
    }
}
