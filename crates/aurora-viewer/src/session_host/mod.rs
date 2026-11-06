//! Opt-in native session controller. Worlds, logins and GPU ownership remain
//! in separately isolated workers; the host paints only its CPU interface.

pub(crate) mod profile;
mod software;
pub(crate) mod transport;
mod ui;
pub(crate) mod worker;

use crate::theme::Theme;
use crate::ui::{fonts, icons::Icons};
use software::SoftwareWindow;
use std::sync::Arc;
use std::time::{Duration, Instant};
use transport::{HostController, HostSnapshot};
use ui::{Action, HostUi, Notice};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

const POLL: Duration = Duration::from_millis(100);
const MIN_FRAME: Duration = Duration::from_millis(33);
const CLOSE_GRACE: Duration = Duration::from_secs(3);

pub fn run() -> anyhow::Result<()> {
    aurora_i18n::select("default", crate::cli::get().language.as_deref());
    let event_loop = EventLoop::new()?;
    let context = egui::Context::default();
    // The software shell has no world behind its surfaces to blur.
    let theme = Theme {
        panel_alpha: 255,
        ..Default::default()
    };
    theme.apply(&context, 1.0);
    fonts::fallback::wait();
    fonts::apply(&context, fonts::DEFAULT);
    let icons = Icons::load(&context, None);
    let controller = HostController::start()?;
    let snapshot = controller.snapshot();
    let now = Instant::now();
    let mut app = Shell {
        context,
        theme,
        _icons: icons,
        controller,
        snapshot,
        ui: HostUi::default(),
        surface: None,
        error: None,
        next_poll: now,
        repaint: Some(now),
        last_frame: None,
        closing: None,
        notice_at: None,
        world: egui::Rect::NOTHING,
    };
    log::info!("session host: CPU controller starting; worker worlds remain independent");
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.error {
        anyhow::bail!(error);
    }
    Ok(())
}

struct Shell {
    context: egui::Context,
    theme: Theme,
    // Retain the embedded icon textures for the lifetime of the CPU painter.
    _icons: Icons,
    controller: HostController,
    snapshot: Arc<HostSnapshot>,
    ui: HostUi,
    surface: Option<SoftwareWindow>,
    error: Option<String>,
    next_poll: Instant,
    repaint: Option<Instant>,
    last_frame: Option<Instant>,
    closing: Option<Instant>,
    notice_at: Option<Instant>,
    world: egui::Rect,
}

impl Shell {
    fn request_repaint(&mut self) {
        self.repaint = Some(Instant::now());
    }

    fn action(&mut self, action: Action, viewport: Option<aurora_session::HostedViewport>) {
        if self.closing.is_some() {
            return;
        }
        let result = match action {
            Action::OpenLogin(account) => self.controller.open_manual_login(account),
            Action::Activate(worker) => match viewport {
                Some(viewport) => self.controller.activate(worker, viewport),
                None => Err(transport::HostError::Unsupported),
            },
            Action::Detach(worker) => self.controller.detach(worker),
            Action::Close => {
                self.controller.shutdown();
                self.closing = Some(Instant::now());
                self.request_repaint();
                return;
            }
        };
        self.ui.notice = Some(match result {
            Ok(()) => Notice::Queued,
            Err(error) => Notice::Error(error),
        });
        self.notice_at = Some(Instant::now());
        self.request_repaint();
    }

    fn render(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let Some(surface) = &mut self.surface else { return Ok(()) };
        let size = surface.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        let raw = surface.raw_input();
        let native_ready = surface.viewport(self.world, self.context.pixels_per_point()).is_some();
        let context = self.context.clone();
        let p = self.theme.palette();
        let mut action = None;
        let mut world = self.world;
        let output = context.run_ui(raw, |ui| {
            let shown = self.ui.show(ui, &p, &self.snapshot, native_ready, self.closing.is_some());
            // A discarded egui pass must not duplicate a queued action.
            action = action.or(shown.action);
            world = shown.world;
        });
        self.world = world;
        let viewport = surface.viewport(world, context.pixels_per_point());
        let delay = surface.present(event_loop, &context, output, p.panel)?;
        let now = Instant::now();
        self.last_frame = Some(now);
        self.repaint = now.checked_add(delay);
        if let Some(action) = action {
            self.action(action, viewport);
        }
        Ok(())
    }

    fn poll(&mut self, now: Instant) {
        if now < self.next_poll {
            return;
        }
        self.next_poll = now + POLL;
        let snapshot = self.controller.snapshot();
        if !ui::same_snapshot(&snapshot, &self.snapshot) {
            self.repaint = Some(now);
        }
        self.snapshot = snapshot;
        if self
            .notice_at
            .is_some_and(|at| now.saturating_duration_since(at) >= Duration::from_secs(4))
        {
            self.ui.notice = None;
            self.notice_at = None;
            self.repaint = Some(now);
        }
    }
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }
        let title = format!("{} — {}", aurora_i18n::tr!("host-title"), crate::cli::window_title());
        match SoftwareWindow::new(event_loop, &self.context, title) {
            Ok(surface) => {
                surface.window.request_redraw();
                self.surface = Some(surface);
                self.request_repaint();
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.controller.shutdown();
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(surface) = &mut self.surface else { return };
        if surface.window.id() != id {
            return;
        }
        if matches!(event, WindowEvent::CloseRequested) {
            // Closing the host is distinct from quitting any running viewer.
            self.ui.close_prompt = true;
            self.request_repaint();
            return;
        }
        let response = surface.event(&event);
        if response.repaint {
            self.request_repaint();
        }
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } | WindowEvent::Moved(_)
        ) {
            self.request_repaint();
        }
        if matches!(event, WindowEvent::RedrawRequested) {
            if self.last_frame.is_some_and(|last| last.elapsed() < MIN_FRAME) {
                self.request_repaint();
            } else if let Err(error) = self.render(event_loop) {
                self.error = Some(error.to_string());
                self.controller.shutdown();
                event_loop.exit();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        self.poll(now);
        if self
            .closing
            .is_some_and(|at| self.controller.stopped() || now.saturating_duration_since(at) >= CLOSE_GRACE)
        {
            // A nonresponsive bridge is not permission to kill its viewers.
            // Their worker lease restores ordinary reachable windows.
            event_loop.exit();
            return;
        }
        let mut next = self.next_poll;
        if let Some(repaint) = self.repaint {
            let due = self.last_frame.map_or(repaint, |last| repaint.max(last + MIN_FRAME));
            if now >= due {
                self.repaint = None;
                if let Some(surface) = &self.surface {
                    surface.window.request_redraw();
                }
            } else {
                next = next.min(due);
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(next));
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.controller.shutdown();
        // Drop clipboard/surface while the platform connection still lives.
        self.surface = None;
    }
}
