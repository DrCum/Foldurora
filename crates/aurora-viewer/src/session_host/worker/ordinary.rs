//! One-way revocation evidence for actual ordinary-window mutations. Reading
//! this clock cannot mint an App/window lease or native permission. It is never
//! reset by source replacement, recovery, temporary borrows or another App.
use std::sync::atomic::{AtomicU64, Ordering};

static REVISION: AtomicU64 = AtomicU64::new(1);
pub(crate) fn invalidate() {
    let _ = REVISION.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        (current != 0).then(|| current.checked_add(1).unwrap_or(0))
    });
}
pub(crate) fn revision() -> Option<u64> {
    let revision = REVISION.load(Ordering::Acquire);
    (revision != 0).then_some(revision)
}
/// Actual input ownership edges are recorded before any popup/dialog model can
/// open and close inside the same egui run. This only revokes; it never consumes
/// an event or constructs permission from a frame snapshot.
pub(crate) fn native_event(event: &winit::event::WindowEvent) {
    use winit::event::{ElementState, TouchPhase, WindowEvent};
    if matches!(event,WindowEvent::KeyboardInput {event,is_synthetic:false,..} if event.state==ElementState::Pressed)
        || matches!(
            event,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } | WindowEvent::Ime(_)
        )
        || matches!(event,WindowEvent::Touch(touch) if touch.phase==TouchPhase::Started)
    {
        invalidate();
    }
}
pub(crate) fn queued_input(raw: &egui::RawInput) {
    if raw.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::AccessKitActionRequest(_)
                | egui::Event::PointerButton { pressed: true, .. }
                | egui::Event::Key { pressed: true, .. }
                | egui::Event::Ime(_)
        )
    }) {
        invalidate();
    }
}
/// Called when an actual modal/popup body enters interaction ownership. The
/// pass stamp records that mutation's lifetime; it does not mint observations.
pub(crate) fn ui_owner(ctx: &egui::Context, id: egui::Id) {
    let pass = ctx.cumulative_pass_nr();
    let fresh = ctx.data_mut(|data| {
        let key = id.with("ordinary_native_owner");
        let old = data.get_temp::<u64>(key);
        data.insert_temp(key, pass);
        old.is_none_or(|old| old.checked_add(1).is_none_or(|next| next < pass))
    });
    if fresh {
        invalidate();
    }
}
