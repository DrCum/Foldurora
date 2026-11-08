//! Actual App/physical-window ownership for ordinary Show Viewer. Only App
//! creation and successful owned Window creation mint these private leases.
//! Public observation numbers and raw handles cannot construct an observer.
use super::*;
use crate::session_host::worker::ordinary;
use aurora_session::show_viewer::WindowObservation;
use aurora_session::show_viewer::{ShowViewerOutcome, ShowViewerRequest};
use std::sync::{
    Weak,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

pub(super) struct WindowOwner {
    app: u64,
    next_window: u64,
    current: Weak<Physical>,
}
struct Physical {
    app: u64,
    window: u64,
    actual: Weak<Window>,
    id: WindowId,
    live: AtomicBool,
    observed: bool,
}
pub(super) struct PhysicalWindowOwner {
    physical: Arc<Physical>,
}
impl PhysicalWindowOwner {
    pub(super) fn retire(&self) {
        self.physical.live.store(false, Ordering::Release);
        ordinary::invalidate();
    }
}
impl Drop for PhysicalWindowOwner {
    fn drop(&mut self) {
        self.retire();
    }
}

#[derive(Clone)]
pub(crate) struct OrdinaryObservation {
    physical: Arc<Physical>,
    revision: u64,
}
impl OrdinaryObservation {
    pub(crate) fn same_owner(&self, other: &Self) -> bool {
        self.revision == other.revision && Arc::ptr_eq(&self.physical, &other.physical)
    }
    pub(crate) fn numbers(&self) -> Option<WindowObservation> {
        self.current_revision().then_some(WindowObservation {
            app: self.physical.app,
            window: self.physical.window,
            ordinary: self.revision,
        })
    }
    fn current_revision(&self) -> bool {
        self.physical.observed
            && self.physical.live.load(Ordering::Acquire)
            && self.physical.actual.strong_count() != 0
            && ordinary::revision() == Some(self.revision)
    }
    pub(crate) fn current_window(&self, window: &Arc<Window>) -> bool {
        self.current_revision()
            && self.physical.id == window.id()
            && self.physical.actual.upgrade().is_some_and(|actual| Arc::ptr_eq(&actual, window))
    }
    pub(crate) fn retire_physical(&self) {
        self.physical.live.store(false, Ordering::Release);
        ordinary::invalidate();
    }
}
impl WindowOwner {
    pub(super) fn new() -> Self {
        static APP: AtomicU64 = AtomicU64::new(1);
        let app = APP
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| current.checked_add(1))
            .unwrap_or(0);
        Self {
            app,
            next_window: 0,
            current: Weak::new(),
        }
    }
    pub(super) fn created(&mut self, window: &Arc<Window>, observed: bool) -> Option<PhysicalWindowOwner> {
        self.retire();
        if self.app == 0 {
            return None;
        }
        let Some(next) = self.next_window.checked_add(1) else {
            self.app = 0;
            return None;
        };
        self.next_window = next;
        let physical = Arc::new(Physical {
            app: self.app,
            window: next,
            actual: Arc::downgrade(window),
            id: window.id(),
            live: AtomicBool::new(true),
            observed,
        });
        self.current = Arc::downgrade(&physical);
        Some(PhysicalWindowOwner { physical })
    }
    pub(super) fn retire(&mut self) {
        if let Some(current) = self.current.upgrade() {
            current.live.store(false, Ordering::Release);
        }
        self.current = Weak::new();
        ordinary::invalidate();
    }
    pub(super) fn observe(&self, lease: &PhysicalWindowOwner, window: &Arc<Window>, shown: bool) -> Option<OrdinaryObservation> {
        if !shown
            || self.app == 0
            || !lease.physical.observed
            || !lease.physical.live.load(Ordering::Acquire)
            || self.current.upgrade().is_none_or(|current| !Arc::ptr_eq(&current, &lease.physical))
            || lease.physical.actual.upgrade().is_none_or(|actual| !Arc::ptr_eq(&actual, window))
        {
            return None;
        }
        Some(OrdinaryObservation {
            physical: lease.physical.clone(),
            revision: ordinary::revision()?,
        })
    }
}
impl Drop for WindowOwner {
    fn drop(&mut self) {
        self.retire();
    }
}

impl App {
    pub(super) fn retire_show_window(&mut self) {
        self.show_window.retire();
    }
    pub(super) fn set_login_busy(&mut self, busy: bool) {
        ordinary::invalidate();
        self.login_form.busy = busy;
    }
    pub(super) fn set_screen(&mut self, screen: Screen) {
        ordinary::invalidate();
        self.screen = screen;
    }

    fn ordinary_model_ready(&self) -> bool {
        matches!(self.screen, Screen::Login | Screen::Loading { .. })
            && !self.exit_prompt.is_open()
            && self.quit_at.is_none()
            && !self.gpu_stall.close
            && self.options_ui.capture().is_none()
            && !self.options_ui.joystick_capturing()
            && !crate::window_hold::active()
            && !egui::Popup::is_any_open(&self.egui_ctx)
            && self.egui_ctx.memory(|memory| memory.top_modal_layer().is_none())
            && self.review_input.can_quarantine()
    }
    pub(super) fn observe_show_window(&self) -> Option<OrdinaryObservation> {
        if !self.ordinary_model_ready() {
            return None;
        }
        let gfx = self.gfx.as_ref()?;
        if gfx.renderer.device_lost() || gfx.renderer.stalled_for().is_some() || gfx.renderer.vr_engaged() {
            return None;
        }
        let observation = self.show_window.observe(gfx.show_window_owner.as_ref()?, &gfx.window, gfx.shown)?;
        match native::probe(&gfx.window) {
            Ok(_) => {}
            Err(ProbeFailure::Gone) => {
                observation.retire_physical();
                return None;
            }
            Err(_) => return None,
        }
        (self.ordinary_model_ready() && observation.current_window(&gfx.window)).then_some(observation)
    }
    pub(super) fn ready_show_window(&self) -> Option<AppReady> {
        let gfx = self.gfx.as_ref()?;
        if !gfx.shown || gfx.renderer.device_lost() || gfx.renderer.stalled_for().is_some() {
            return None;
        }
        if !native::physical_live(&gfx.window) {
            return None;
        }
        Some(AppReady {
            window: Arc::downgrade(&gfx.window),
            id: gfx.window.id(),
        })
    }
    fn show_scope_current(
        &self,
        bridge: &crate::session_host::worker::WorkerBridge,
        work: &crate::session_host::worker::ShowWork,
        observation: &OrdinaryObservation,
        window: &Arc<Window>,
    ) -> bool {
        self.ordinary_model_ready()
            && self.gfx.as_ref().is_some_and(|gfx| {
                gfx.shown
                    && Arc::ptr_eq(&gfx.window, window)
                    && !gfx.renderer.device_lost()
                    && gfx.renderer.stalled_for().is_none()
                    && !gfx.renderer.vr_engaged()
            })
            && bridge.ordinary_authorized(observation, window, work.request(), work.received_at(), work.lookup_only())
    }
    pub(super) fn show_owned_viewer(
        &mut self,
        bridge: &crate::session_host::worker::WorkerBridge,
        work: &crate::session_host::worker::ShowWork,
    ) -> ShowResult {
        let rejected = |outcome| ShowResult {
            request: work.request(),
            outcome,
            lookup: false,
        };
        let Some(observation) = self.observe_show_window() else {
            return rejected(ShowViewerOutcome::RejectedNative);
        };
        let Some(window) = self.gfx.as_ref().map(|gfx| gfx.window.clone()) else {
            return rejected(ShowViewerOutcome::RejectedNative);
        };
        if !self.show_scope_current(bridge, work, &observation, &window) {
            return rejected(ShowViewerOutcome::StaleTarget);
        }
        let probe = match native::probe(&window) {
            Ok(probe) => probe,
            Err(ProbeFailure::Gone) => {
                observation.retire_physical();
                return rejected(ShowViewerOutcome::StaleTarget);
            }
            Err(_) => return rejected(ShowViewerOutcome::RejectedNative),
        };
        // Read-only native queries can outlive ingress/lease/source. Recheck
        // original age and the actual current owner immediately before effect.
        if !self.show_scope_current(bridge, work, &observation, &window) {
            return rejected(ShowViewerOutcome::StaleTarget);
        }
        if work.lookup_only() {
            return ShowResult {
                request: work.request(),
                outcome: ShowViewerOutcome::RejectedNative,
                lookup: true,
            };
        }
        if !self.review_input.begin_show() {
            return rejected(ShowViewerOutcome::RejectedNative);
        }
        self.alt_codes.reset();
        self.alt_text.clear();
        // Drain transient queued input only. Canonical text, editor selection
        // and Undo state are untouched; no dialog/popup is dismissed to qualify.
        if let Some(gfx) = &mut self.gfx {
            let _ = gfx.egui_state.take_egui_input(&gfx.window);
        }
        self.egui_ctx.stop_dragging();
        self.egui_ctx.input_mut(|input| {
            input.pointer = Default::default();
            input.events.clear();
            input.keys_down.clear();
        });
        let outcome = perform_show(
            || self.show_scope_current(bridge, work, &observation, &window),
            || work.mark_effect_started(),
            || {
                let restored = native::restore_iconic(&probe);
                if !restored {
                    observation.retire_physical();
                }
                restored
            },
            || native::focus_once(&probe),
        );
        if matches!(outcome, ShowViewerOutcome::ShownLocally | ShowViewerOutcome::ForegroundRefused) {
            window.request_redraw();
        }
        rejected(outcome)
    }
}

/// The synchronous effect sequence shared by the real callee and CPU backend
/// cases. `current` performs fresh actual App/window/source/lease/age queries at
/// every boundary. This function constructs no owner, native permit or retry.
fn perform_show(
    mut current: impl FnMut() -> bool,
    mut started: impl FnMut(),
    mut restore: impl FnMut() -> bool,
    mut focus: impl FnMut() -> bool,
) -> ShowViewerOutcome {
    if !current() {
        return ShowViewerOutcome::StaleTarget;
    }
    started();
    if !restore() || !current() {
        return ShowViewerOutcome::EffectUncertain;
    }
    let foreground = focus();
    if !current() {
        return ShowViewerOutcome::EffectUncertain;
    }
    if foreground {
        ShowViewerOutcome::ShownLocally
    } else {
        ShowViewerOutcome::ForegroundRefused
    }
}

/// Non-serializable App-ready evidence. It is created only from the genuine
/// current Gfx after first presentation, and grants no native/window authority.
pub(crate) struct AppReady {
    window: Weak<Window>,
    id: WindowId,
}
impl AppReady {
    pub(crate) fn current(&self) -> bool {
        self.window
            .upgrade()
            .is_some_and(|window| window.id() == self.id && native::physical_live(&window))
    }
}
/// Only the actual App callee constructs a completion. Correlation is checked
/// again by the original Bridge owner; callers cannot construct ShownLocally.
pub(crate) struct ShowResult {
    request: ShowViewerRequest,
    outcome: ShowViewerOutcome,
    lookup: bool,
}
impl ShowResult {
    pub(crate) fn request(&self) -> ShowViewerRequest {
        self.request
    }
    pub(crate) fn outcome(&self) -> ShowViewerOutcome {
        self.outcome
    }
    pub(crate) fn lookup_allowed(&self) -> bool {
        self.lookup
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
enum ProbeFailure {
    Gone,
    Rejected,
}
#[cfg(windows)]
mod native {
    use super::*;
    use windows_sys::Win32::{
        Foundation::HWND,
        System::Threading::GetCurrentThreadId,
        UI::{
            Input::KeyboardAndMouse::IsWindowEnabled,
            WindowsAndMessaging::{
                GA_ROOT, GW_OWNER, GWL_STYLE, GetAncestor, GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowThreadProcessId,
                IsIconic, IsWindow, IsWindowVisible, SW_RESTORE, SetForegroundWindow, ShowWindow, WS_CHILD,
            },
        },
    };
    pub(super) struct Probe {
        hwnd: HWND,
        iconic: bool,
    }
    fn handle(window: &Window) -> HWND {
        crate::session_host::worker::native_window(window) as HWND
    }
    pub(super) fn physical_live(window: &Window) -> bool {
        let hwnd = handle(window);
        let mut pid = 0;
        // SAFETY: read-only checks of the actual owned winit Window on its
        // creating event thread. No foreign handle can become a lease here.
        unsafe {
            !hwnd.is_null()
                && IsWindow(hwnd) != 0
                && GetWindowThreadProcessId(hwnd, &mut pid) == GetCurrentThreadId()
                && pid == std::process::id()
        }
    }
    pub(super) fn probe(window: &Arc<Window>) -> Result<Probe, ProbeFailure> {
        if !physical_live(window) {
            return Err(ProbeFailure::Gone);
        }
        let hwnd = handle(window);
        // SAFETY: physical_live checked this exact owned Window/process/thread.
        // Queries never change styles, parent, owner, placement or foreground.
        unsafe {
            if GetAncestor(hwnd, GA_ROOT) != hwnd
                || !GetWindow(hwnd, GW_OWNER).is_null()
                || GetWindowLongPtrW(hwnd, GWL_STYLE) & WS_CHILD as isize != 0
                || IsWindowEnabled(hwnd) == 0
                || IsWindowVisible(hwnd) == 0
            {
                return Err(ProbeFailure::Rejected);
            }
            Ok(Probe {
                hwnd,
                iconic: IsIconic(hwnd) != 0,
            })
        }
    }
    pub(super) fn restore_iconic(probe: &Probe) -> bool {
        // SAFETY: only the already checked owned top-level Window is restored,
        // synchronously on its thread. Never restore a managed presentation.
        unsafe {
            if probe.iconic {
                ShowWindow(probe.hwnd, SW_RESTORE);
            }
            IsWindow(probe.hwnd) != 0 && IsIconic(probe.hwnd) == 0
        }
    }
    pub(super) fn focus_once(probe: &Probe) -> bool {
        // SAFETY: App rechecked full original scope/lease/age after native
        // queries, immediately before this one synchronous foreground request.
        unsafe {
            let _ = SetForegroundWindow(probe.hwnd);
            GetForegroundWindow() == probe.hwnd
        }
    }
}
#[cfg(not(windows))]
mod native {
    use super::*;
    pub(super) struct Probe;
    pub(super) fn physical_live(_: &Window) -> bool {
        // The genuine Gfx Window can initialize only the startup clock here.
        // Native Show remains unsupported: probe always rejects below.
        true
    }
    pub(super) fn probe(_: &Arc<Window>) -> Result<Probe, ProbeFailure> {
        Err(ProbeFailure::Rejected)
    }
    pub(super) fn restore_iconic(_: &Probe) -> bool {
        false
    }
    pub(super) fn focus_once(_: &Probe) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn actual_show_effect_gate_rechecks_after_queries_restore_and_focus_without_retry() {
        // No genuine App/window evidence is fabricated. The real callee above
        // invokes this exact gate after native queries and quarantine. A stale
        // query result must cause zero effects; each later loss is uncertainty.
        for (lost_at, expected, restores, focuses) in [
            (1, ShowViewerOutcome::StaleTarget, 0, 0),
            (2, ShowViewerOutcome::EffectUncertain, 1, 0),
            (3, ShowViewerOutcome::EffectUncertain, 1, 1),
            (4, ShowViewerOutcome::ShownLocally, 1, 1),
        ] {
            let checks = Cell::new(0);
            let started = Cell::new(0);
            let restore = Cell::new(0);
            let focus = Cell::new(0);
            let outcome = perform_show(
                || {
                    checks.set(checks.get() + 1);
                    checks.get() != lost_at
                },
                || started.set(started.get() + 1),
                || {
                    restore.set(restore.get() + 1);
                    true
                },
                || {
                    focus.set(focus.get() + 1);
                    true
                },
            );
            assert_eq!(outcome, expected);
            assert_eq!(restore.get(), restores);
            assert_eq!(focus.get(), focuses);
            assert_eq!(started.get(), u32::from(restores != 0));
        }
        let attempts = Cell::new(0);
        assert_eq!(
            perform_show(
                || true,
                || {},
                || true,
                || {
                    attempts.set(attempts.get() + 1);
                    false
                }
            ),
            ShowViewerOutcome::ForegroundRefused
        );
        assert_eq!(attempts.get(), 1);
        let focus = Cell::new(0);
        assert_eq!(
            perform_show(
                || true,
                || {},
                || false,
                || {
                    focus.set(focus.get() + 1);
                    true
                }
            ),
            ShowViewerOutcome::EffectUncertain
        );
        assert_eq!(focus.get(), 0);
    }
}
