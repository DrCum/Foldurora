//! Actual egui/model/channel cases, authored UNRUN. No fixture starts a worker
//! or reads a profile; Root owns all execution and native acceptance.
use super::*;
use crate::session_host::slots::{SlotLabel, SlotSummary};
use crate::session_host::transport::copy_fixture::{self, Request};
use aurora_session::{HostId, InstallationId};
use egui::{Event, Modifiers, PointerButton, Pos2, Vec2};

fn slot(index: u8) -> SlotId {
    SlotId::try_from(index).unwrap()
}
fn snapshot() -> HostSnapshot {
    HostSnapshot {
        slots: SlotId::all()
            .map(|id| SlotSummary {
                id,
                label: SlotLabel::new(&"W".repeat(64)).unwrap(),
                available: true,
            })
            .collect(),
        ..Default::default()
    }
}
fn pointer(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}
fn key(key: Key, pressed: bool) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}
#[derive(Clone, Copy)]
enum Late {
    None,
    Discard,
    OtherModal,
    Close,
    FocusLoss,
    MoveWidget,
    ReclipWidget,
    ResenseWidget,
    CoverWidget,
    CoverLayer,
    VisualWidgetCover,
    VisualLayerCover,
    FocusWidget,
    Transform(egui::emath::TSTransform),
}
struct Frames {
    ctx: egui::Context,
    model: CopyUi,
    snapshot: HostSnapshot,
    size: Vec2,
    scale: f32,
    frame: u64,
}
impl Frames {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::Theme {
            panel_alpha: 255,
            ..Default::default()
        }
        .apply(&ctx, 1.0);
        let snapshot = snapshot();
        let mut model = CopyUi::default();
        model.open(slot(0), &snapshot);
        let mut frames = Self {
            ctx,
            model,
            snapshot,
            size: egui::vec2(760.0, 600.0),
            scale: 1.0,
            frame: 0,
        };
        frames.show(vec![], Late::None);
        frames.show(vec![], Late::None);
        frames
    }
    fn show(&mut self, events: Vec<Event>, late: Late) -> Option<Command> {
        self.frame += 1;
        let p = crate::theme::Theme::default().palette();
        let mut action = None;
        let mut raw = egui::RawInput {
            focused: true,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.frame as f64 / 60.0),
            events,
            ..Default::default()
        };
        raw.viewports.entry(raw.viewport_id).or_default().native_pixels_per_point = Some(self.scale);
        let _output = self.ctx.run_ui(raw, |ui| {
            action = action.take().or(self.model.show(ui.ctx(), &p, &self.snapshot, false));
            match late {
                Late::None => {}
                Late::Discard => ui.ctx().request_discard("copy fixture changes layout after the modal body"),
                Late::OtherModal => {
                    widgets::Modal::new(Id::unique("copy_fixture_late_modal")).show(ui.ctx(), |ui| {
                        ui.label("synthetic modal");
                    });
                }
                Late::Close => self.model.close(),
                Late::FocusLoss => ui.ctx().input_mut(|input| input.focused = false),
                Late::MoveWidget
                | Late::ReclipWidget
                | Late::ResenseWidget
                | Late::CoverWidget
                | Late::CoverLayer
                | Late::VisualWidgetCover
                | Late::VisualLayerCover
                | Late::FocusWidget
                | Late::Transform(_) => {
                    if let Some(completed) = &self.model.completed {
                        late_widget(ui, completed.control, late);
                    }
                }
            }
        });
        let command = action.and_then(|action| self.model.take_action(action, &self.ctx, &self.snapshot, false));
        self.model.finish_frame(&self.ctx);
        command
    }
    fn control(&self, predicate: impl Fn(Intent) -> bool) -> Control {
        *self
            .model
            .controls
            .iter()
            .find(|control| predicate(control.intent))
            .expect("settled visible control")
    }
    fn click(&mut self, control: Control, late: Late) -> Option<Command> {
        assert!(
            self.show(
                vec![Event::PointerMoved(control.rect.center()), pointer(control.rect.center(), true)],
                Late::None
            )
            .is_none()
        );
        self.show(vec![pointer(control.rect.center(), false)], late)
    }
    fn inspect(&mut self) -> (Selection, u64) {
        let control = self.control(|intent| matches!(intent, Intent::Inspect(_)));
        let command = self.click(control, Late::None).unwrap();
        let Command::Inspect(selection) = command else { panic!("Inspect") };
        let (controller, inbox) = copy_fixture::controller(self.snapshot.clone());
        let request = controller.inspect_preferences(selection).unwrap();
        assert_eq!(inbox.take(), Some((request, Request::Inspect(selection))));
        assert!(inbox.take().is_none());
        self.model.queued(command, Ok(request));
        (selection, request)
    }
    fn prepare(&mut self) -> Capture {
        let (selection, request) = self.inspect();
        let capture = Capture {
            installation: InstallationId::from_bytes([11; 32]).unwrap(),
            controller: HostId::from_bytes([12; 16]).unwrap(),
            manager: Instance::fresh().unwrap(),
            operation: request,
            selection,
        };
        self.snapshot.completed_request = request;
        self.snapshot.copy_preferences = Some(State {
            capture,
            phase: Phase::Prepared(vec![crate::settings::profile_seed::Summary {
                family: Family::Interface,
                fields: 18,
                changed: 1,
                defaulted: 17,
            }]),
        });
        self.show(vec![], Late::None);
        self.show(vec![], Late::None);
        capture
    }
}

fn late_widget(ui: &mut egui::Ui, control: Control, change: Late) {
    if let Late::Transform(transform) = change {
        ui.ctx().set_transform_layer(control.layer, transform);
        return;
    }
    if matches!(change, Late::CoverLayer | Late::VisualLayerCover) {
        egui::Area::new(Id::unique("copy_guard_late_cover_area"))
            .order(egui::Order::Foreground)
            .interactable(matches!(change, Late::CoverLayer))
            .fixed_pos(control.rect.min)
            .show(ui.ctx(), |ui| {
                ui.allocate_response(control.rect.size(), egui::Sense::click());
            });
        return;
    }
    // Same-id metadata turnover is deliberate here, using actual Ui::interact
    // registration and the final committed registry, not a mock response map.
    ui.ctx().options_mut(|options| options.warn_on_id_clash = false);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("copy_guard_late_widget")
            .layer_id(control.layer)
            .max_rect(ui.ctx().content_rect()),
    );
    child.set_clip_rect(control.paint_clip);
    match change {
        Late::MoveWidget => {
            child.interact(
                control.registered.rect.translate(egui::vec2(11.0, 0.0)),
                control.widget,
                control.registered.sense,
            );
        }
        Late::ReclipWidget => {
            child.set_clip_rect(Rect::from_min_max(
                control.rect.min,
                egui::pos2(control.rect.center().x, control.rect.max.y),
            ));
            child.interact(control.registered.rect, control.widget, control.registered.sense);
        }
        Late::ResenseWidget => {
            child.interact(
                control.registered.rect,
                control.widget,
                control.registered.sense | egui::Sense::drag(),
            );
        }
        Late::CoverWidget => {
            child.interact(
                control.registered.rect,
                Id::unique("copy_guard_late_same_layer_cover"),
                egui::Sense::click(),
            );
        }
        Late::VisualWidgetCover => {
            child.interact(
                control.registered.rect,
                Id::unique("copy_guard_late_visual_cover"),
                egui::Sense::hover(),
            );
        }
        Late::FocusWidget => {
            let mut text = String::new();
            child
                .put(
                    Rect::from_min_size(Pos2::ZERO, egui::vec2(80.0, 24.0)),
                    egui::TextEdit::singleline(&mut text).id(Id::unique("copy_guard_late_editor")),
                )
                .request_focus();
        }
        _ => {}
    }
}

#[test]
fn profile_copy_ui_starts_main_saved_all_four_checked_and_five_bounded_sources() {
    let mut frames = Frames::new();
    let dialog = frames.model.dialog.as_ref().unwrap();
    assert_eq!(dialog.bits, 15);
    assert_eq!(dialog.selection.source, Source::MainSaved);
    assert_eq!(dialog.selection.target, slot(0));
    assert_eq!(sources(&frames.snapshot, slot(0)).len(), 5);
    assert!(!sources(&frames.snapshot, slot(0)).contains(&Source::Slot(slot(0))));
    let required = sources(&frames.snapshot, slot(0)).len() + 1 + Family::ALL.len() + 3;
    assert!(required <= MAX_CONTROLS);
    let picker = frames.control(|intent| matches!(intent, Intent::Picker(_)));
    assert!(frames.click(picker, Late::None).is_none());
    frames.show(vec![], Late::None);
    let visible_sources = frames
        .model
        .controls
        .iter()
        .filter(|control| matches!(control.intent, Intent::Source(_, _)))
        .count();
    assert!((1..=MAX_SOURCES).contains(&visible_sources));
    assert!(frames.model.controls.len() <= MAX_CONTROLS);
    frames.snapshot.slots[1].available = false;
    assert!(!sources(&frames.snapshot, slot(0)).contains(&Source::Slot(slot(1))));
    let before = frames.model.selection();
    frames.model.open(slot(1), &frames.snapshot);
    assert_eq!(frames.model.selection(), before);
}

#[test]
fn profile_copy_ui_actual_inspect_controller_inbox_and_prepared_apply_use_exact_capture() {
    let mut frames = Frames::new();
    let capture = frames.prepare();
    let control = frames.control(|intent| matches!(intent, Intent::Apply(_)));
    let command = frames.click(control, Late::None).unwrap();
    assert_eq!(command, Command::Apply(capture));
    let (controller, inbox) = copy_fixture::controller(frames.snapshot.clone());
    let request = controller.apply_preferences(capture).unwrap();
    assert_eq!(inbox.take(), Some((request, Request::Apply(capture))));
    frames.model.queued(command, Ok(request));
    frames.show(vec![], Late::None);
    assert!(matches!(
        frames.model.operation.as_ref().unwrap().state.as_ref().unwrap().phase,
        Phase::Applying
    ));
    assert!(
        !frames
            .model
            .controls
            .iter()
            .any(|control| matches!(control.intent, Intent::Apply(_))),
        "old Prepared cannot replay queued Apply"
    );
    assert!(inbox.take().is_none());
}

#[test]
fn profile_copy_ui_source_and_family_turnover_never_revive_apply_after_a_b_a() {
    for source_change in [false, true] {
        let mut frames = Frames::new();
        let capture = frames.prepare();
        let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
        frames.show(
            vec![Event::PointerMoved(apply.rect.center()), pointer(apply.rect.center(), true)],
            Late::None,
        );
        let original = frames.model.selection().unwrap();
        if source_change {
            frames.model.change(Some(Source::Slot(slot(1))), None);
            frames.model.change(Some(original.source), None);
        } else {
            frames.model.change(None, Some(1));
            frames.model.change(None, Some(15));
        }
        assert_ne!(frames.model.selection(), Some(original));
        assert_eq!(frames.model.cancellation(), Some((capture.selection, capture.operation)));
        assert!(frames.show(vec![pointer(apply.rect.center(), false)], Late::None).is_none());
    }
}

#[test]
fn profile_copy_ui_final_whole_run_guard_rejects_late_discard_modal_removal_focus_and_other_modal() {
    for late in [Late::Discard, Late::Close, Late::FocusLoss, Late::OtherModal] {
        let mut frames = Frames::new();
        frames.prepare();
        let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
        assert!(frames.click(apply, late).is_none());
        assert!(frames.show(vec![], Late::None).is_none(), "rejected release has no deferred replay");
    }
}

#[test]
fn profile_copy_ui_controller_manager_turnover_and_clip_geometry_revoke_pointer_apply() {
    for manager_change in [false, true] {
        let mut frames = Frames::new();
        frames.prepare();
        let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
        frames.show(
            vec![Event::PointerMoved(apply.rect.center()), pointer(apply.rect.center(), true)],
            Late::None,
        );
        if manager_change {
            let state = frames.snapshot.copy_preferences.as_mut().unwrap();
            state.capture.manager = Instance::fresh().unwrap();
            state.capture.controller = HostId::from_bytes([19; 16]).unwrap();
        } else {
            frames.size = egui::vec2(360.0, 453.0);
        }
        assert!(frames.show(vec![pointer(apply.rect.center(), false)], Late::None).is_none());
        assert!(frames.show(vec![], Late::None).is_none());
    }
}

#[test]
fn profile_copy_ui_modal_default_enter_cannot_turn_inspect_into_newly_enabled_apply() {
    let mut frames = Frames::new();
    let inspect = frames.control(|intent| matches!(intent, Intent::Inspect(_)));
    frames.ctx.memory_mut(|memory| memory.request_focus(inspect.widget));
    assert!(frames.show(vec![key(Key::Enter, true)], Late::None).is_none());
    let command = frames.show(vec![key(Key::Enter, false)], Late::None).unwrap();
    let Command::Inspect(selection) = command else { panic!("Inspect") };
    frames.model.queued(command, Ok(1));
    let capture = Capture {
        installation: InstallationId::from_bytes([21; 32]).unwrap(),
        controller: HostId::from_bytes([22; 16]).unwrap(),
        manager: Instance::fresh().unwrap(),
        operation: 1,
        selection,
    };
    frames.snapshot.copy_preferences = Some(State {
        capture,
        phase: Phase::Prepared(vec![]),
    });
    frames.snapshot.completed_request = 1;
    for _ in 0..3 {
        assert!(
            frames.show(vec![], Late::None).is_none(),
            "Modal synthetic primary click has no original Apply press"
        );
    }
}

#[test]
fn profile_copy_ui_space_and_accessibility_require_settled_original_enabled_control() {
    let mut frames = Frames::new();
    let capture = frames.prepare();
    let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
    frames.ctx.memory_mut(|memory| memory.request_focus(apply.widget));
    assert!(frames.show(vec![key(Key::Space, true)], Late::None).is_none());
    assert_eq!(frames.show(vec![key(Key::Space, false)], Late::None), Some(Command::Apply(capture)));
    let mut accessible = Frames::new();
    let exact = accessible.prepare();
    let enabled = accessible.control(|intent| matches!(intent, Intent::Apply(_)));
    accessible.ctx.memory_mut(|memory| memory.request_focus(enabled.widget));
    accessible.show(vec![], Late::None);
    let click = egui::accesskit::ActionRequest {
        action: egui::accesskit::Action::Click,
        target_tree: egui::accesskit::TreeId::ROOT,
        target_node: enabled.widget.accesskit_id(),
        data: None,
    };
    assert_eq!(
        accessible.show(vec![Event::AccessKitActionRequest(click)], Late::None),
        Some(Command::Apply(exact))
    );
    // A fresh dialog has no settled Apply widget from which AccessKit can gain
    // authority, even when the old widget id is requested explicitly.
    frames.model.close();
    frames.model.open(slot(2), &frames.snapshot);
    let action = egui::accesskit::ActionRequest {
        action: egui::accesskit::Action::Click,
        target_tree: egui::accesskit::TreeId::ROOT,
        target_node: apply.widget.accesskit_id(),
        data: None,
    };
    assert!(frames.show(vec![Event::AccessKitActionRequest(action)], Late::None).is_none());
}

#[test]
fn profile_copy_ui_close_retains_cancel_on_full_real_channel_and_applied_original_receipt() {
    let mut frames = Frames::new();
    let capture = frames.prepare();
    let close = frames.control(|intent| matches!(intent, Intent::Close(_)));
    assert!(frames.click(close, Late::None).is_none());
    assert!(!frames.model.is_open());
    let (controller, inbox) = copy_fixture::controller(frames.snapshot.clone());
    for _ in 0..16 {
        controller.cancel_preferences(capture.selection, capture.operation).unwrap();
    }
    let exact = frames.model.cancellation().unwrap();
    assert_eq!(controller.cancel_preferences(exact.0, exact.1), Err(HostError::Busy));
    assert_eq!(
        frames.model.cancellation(),
        Some(exact),
        "queue-full cannot refund cancellation or original correlation"
    );
    for _ in 0..16 {
        assert!(
            matches!(inbox.take(),Some((_,Request::Cancel(selection,request))) if selection==capture.selection && request==capture.operation)
        );
    }
    let ticket = controller.cancel_preferences(exact.0, exact.1).unwrap();
    frames.model.cancellation_queued(exact.0, exact.1);
    assert_eq!(inbox.take(), Some((ticket, Request::Cancel(capture.selection, capture.operation))));
    assert!(frames.model.cancellation().is_none());
    frames.model.open(slot(2), &frames.snapshot);
    frames.snapshot.copy_preferences = Some(State {
        capture,
        phase: Phase::Terminal(Outcome::Applied),
    });
    frames.model.sync(&frames.snapshot);
    let operation = frames.model.operation.as_ref().unwrap();
    assert_eq!(operation.selection.target, slot(0));
    assert!(matches!(operation.state.as_ref().unwrap().phase, Phase::Terminal(Outcome::Applied)));
    assert_eq!(frames.model.selection().unwrap().target, slot(2));
}

#[test]
fn profile_copy_ui_missing_ack_is_uncertain_and_terminal_snapshot_repaints_without_new_ticket() {
    let mut frames = Frames::new();
    let (selection, request) = frames.inspect();
    frames.snapshot.completed_request = request;
    frames.model.sync(&frames.snapshot);
    assert!(frames.model.operation.as_ref().unwrap().uncertain);
    assert!(frames.model.busy(), "missing acknowledgement is not physical completion proof");
    frames.model.close();
    assert!(
        frames.model.cancellation().is_some(),
        "an uncertain local state cannot erase exact pending cancellation"
    );
    let original = Capture {
        installation: InstallationId::from_bytes([31; 32]).unwrap(),
        controller: HostId::from_bytes([32; 16]).unwrap(),
        manager: Instance::fresh().unwrap(),
        operation: request,
        selection,
    };
    frames.snapshot.copy_preferences = Some(State {
        capture: original,
        phase: Phase::Terminal(Outcome::Applied),
    });
    frames.model.sync(&frames.snapshot);
    assert!(!frames.model.operation.as_ref().unwrap().uncertain);
    frames.snapshot.copy_preferences = Some(State {
        capture: original,
        phase: Phase::Prepared(vec![]),
    });
    frames.model.sync(&frames.snapshot);
    assert!(
        matches!(
            frames.model.operation.as_ref().unwrap().state.as_ref().unwrap().phase,
            Phase::Terminal(Outcome::Applied)
        ),
        "late Prepared cannot erase original Applied"
    );
    let mut a = snapshot();
    let mut b = a.clone();
    let mut prepared = Frames::new();
    let capture = prepared.prepare();
    a.completed_request = capture.operation;
    b.completed_request = capture.operation;
    a.copy_preferences = Some(State {
        capture,
        phase: Phase::Prepared(vec![]),
    });
    b.copy_preferences = Some(State {
        capture,
        phase: Phase::Terminal(Outcome::Applied),
    });
    assert!(!crate::session_host::ui::same_snapshot(&a, &b));
    assert!(crate::session_host::ui::same_snapshot(&a, &a.clone()));
}

#[test]
fn profile_copy_ui_native_enter_space_snapshot_and_logical_reset_preserve_real_release_fence() {
    use winit::event::{ElementState, WindowEvent};
    use winit::keyboard::{Key as NativeKey, NamedKey};
    for native_key in [
        NativeKey::Named(NamedKey::Enter),
        NativeKey::Named(NamedKey::Space),
        NativeKey::Character(" ".into()),
    ] {
        for before_focus in [false, true] {
            let mut model = CopyUi::default();
            let key = if native_key == NativeKey::Named(NamedKey::Enter) {
                Key::Enter
            } else {
                Key::Space
            };
            model.native_key(&native_key, ElementState::Pressed, false, false);
            model.native_key(&native_key, ElementState::Released, false, true);
            model.native_event(&WindowEvent::Focused(false));
            if before_focus {
                model.native_key(&native_key, ElementState::Pressed, false, true);
            }
            model.native_event(&WindowEvent::Focused(true));
            if !before_focus {
                model.native_key(&native_key, ElementState::Pressed, false, true);
            }
            model.native_key(&native_key, ElementState::Pressed, false, false);
            assert!(!model.key_allowed(key));
            model.close();
            model.cancel_gestures();
            assert!(!model.key_allowed(key));
            let ctx = egui::Context::default();
            model.finish_frame(&ctx);
            model.native_key(&native_key, ElementState::Pressed, false, false);
            assert!(!model.key_allowed(key), "logical frame/reset cannot refund a physical held key");
            model.native_key(&native_key, ElementState::Released, false, false);
            model.finish_frame(&ctx);
            model.native_key(&native_key, ElementState::Pressed, false, false);
            assert!(model.key_allowed(key));
        }
    }
}

#[test]
fn profile_copy_ui_empty_groups_and_ime_preedit_have_no_inspect_authority() {
    let mut frames = Frames::new();
    frames.model.change(None, Some(0));
    frames.show(vec![], Late::None);
    assert!(
        !frames
            .model
            .controls
            .iter()
            .any(|control| matches!(control.intent, Intent::Inspect(_)))
    );
    frames.model.change(None, Some(15));
    frames.show(vec![], Late::None);
    frames.show(vec![], Late::None);
    let inspect = frames.control(|intent| matches!(intent, Intent::Inspect(_)));
    frames.ctx.memory_mut(|memory| memory.request_focus(inspect.widget));
    assert!(
        frames
            .show(
                vec![Event::Ime(egui::ImeEvent::Preedit {
                    text: "合成".into(),
                    active_range_chars: None
                })],
                Late::None
            )
            .is_none()
    );
    assert!(
        frames
            .show(vec![key(Key::Enter, true), key(Key::Enter, false)], Late::None)
            .is_none()
    );
    assert!(frames.model.operation.is_none());
}

#[test]
#[ignore = "Root executes separately for en/fr/ja via existing AURORA_COMPACT_HOST_VISUAL_LANGUAGE; actual font/layout acceptance"]
fn profile_copy_ui_actual_compact_modal_and_source_popup_layout_explicit_locale() {
    let locale = std::env::var("AURORA_COMPACT_HOST_VISUAL_LANGUAGE").unwrap();
    assert!(["en", "fr", "ja"].contains(&locale.as_str()));
    struct Restore(String);
    impl Drop for Restore {
        fn drop(&mut self) {
            aurora_i18n::select(&self.0, None);
        }
    }
    let _restore = Restore(aurora_i18n::language().to_owned());
    aurora_i18n::select(&locale, None);
    if locale == "ja" {
        assert!(
            crate::ui::fonts::fallback::wait()
                .iter()
                .any(|face| face.has('あ') && face.has('界'))
        );
    }
    for (physical, scale) in [
        (egui::vec2(1240.0, 800.0), 1.0),
        (egui::vec2(760.0, 600.0), 1.0),
        (egui::vec2(760.0, 600.0), 1.5),
        (egui::vec2(540.0, 680.0), 1.5),
    ] {
        let mut frames = Frames::new();
        frames.size = physical / scale;
        frames.scale = scale;
        crate::ui::fonts::apply(&frames.ctx, crate::ui::fonts::DEFAULT);
        if locale == "ja" {
            for item in &mut frames.snapshot.slots {
                item.label = SlotLabel::new("日本語".repeat(7).as_str()).unwrap();
            }
        }
        for _ in 0..3 {
            frames.show(vec![], Late::None);
        }
        let canvas = Rect::from_min_size(Pos2::ZERO, frames.size);
        let modal = frames
            .ctx
            .data(|data| data.get_temp::<Rect>(Id::unique("host_copy_modal")))
            .unwrap();
        assert!(canvas.expand(1.0).contains_rect(modal), "{locale} {physical:?}/{scale}: {modal:?}");
        for index in 0_usize..3 {
            let footer = frames
                .ctx
                .data(|data| data.get_temp::<Rect>(Id::unique(("host_copy_footer", index))))
                .unwrap();
            assert!(modal.contains_rect(footer) && canvas.contains_rect(footer));
        }
        let picker = frames.control(|intent| matches!(intent, Intent::Picker(_)));
        frames.click(picker, Late::None);
        frames.show(vec![], Late::None);
        assert_eq!(sources(&frames.snapshot, slot(0)).len(), 5);
        let visible_sources = frames
            .model
            .controls
            .iter()
            .filter(|control| matches!(control.intent, Intent::Source(_, _)))
            .count();
        assert!((1..=MAX_SOURCES).contains(&visible_sources));
        let popup_id = frames.ctx.data(|data| data.get_temp::<Id>(Id::unique("host_copy_popup"))).unwrap();
        let popup = frames.ctx.memory(|memory| memory.area_rect(popup_id)).unwrap();
        assert!(canvas.expand(1.0).contains_rect(popup), "source popup {popup:?} outside {canvas:?}");
        for control in &frames.model.controls {
            assert!(canvas.contains_rect(control.rect));
        }
        assert!(frames.model.controls.len() <= MAX_CONTROLS);
    }
}

#[derive(Clone, Copy)]
enum Modality {
    Pointer,
    Enter,
    Space,
    Accessibility,
}
fn release_with(frames: &mut Frames, control: Control, modality: Modality, late: Late) -> Option<Command> {
    match modality {
        Modality::Pointer => frames.click(control, late),
        Modality::Enter | Modality::Space => {
            frames.ctx.memory_mut(|memory| memory.request_focus(control.widget));
            frames.show(vec![], Late::None);
            let key = if matches!(modality, Modality::Enter) {
                Key::Enter
            } else {
                Key::Space
            };
            assert!(frames.show(vec![key_event(key, true, Modifiers::NONE)], Late::None).is_none());
            frames.show(vec![key_event(key, false, Modifiers::NONE)], late)
        }
        Modality::Accessibility => {
            frames.ctx.memory_mut(|memory| memory.request_focus(control.widget));
            frames.show(vec![], Late::None);
            frames.show(
                vec![Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::Click,
                    target_tree: egui::accesskit::TreeId::ROOT,
                    target_node: control.widget.accesskit_id(),
                    data: None,
                })],
                late,
            )
        }
    }
}
fn key_event(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers,
    }
}

#[test]
fn profile_copy_ui_final_committed_turnover_and_coverage_reject_every_modality_without_replay() {
    for modality in [Modality::Pointer, Modality::Enter, Modality::Space, Modality::Accessibility] {
        for late in [
            Late::MoveWidget,
            Late::ReclipWidget,
            Late::ResenseWidget,
            Late::CoverWidget,
            Late::CoverLayer,
            Late::VisualWidgetCover,
            Late::VisualLayerCover,
            Late::FocusWidget,
        ] {
            let mut frames = Frames::new();
            frames.prepare();
            let original = frames.control(|intent| matches!(intent, Intent::Apply(_)));
            assert!(release_with(&mut frames, original, modality, late).is_none());
            let committed = frames
                .ctx
                .viewport(|viewport| viewport.prev_pass.widgets.get(original.widget).copied())
                .unwrap();
            match late {
                Late::MoveWidget => assert_ne!(committed.rect, original.registered.rect),
                Late::ReclipWidget => {
                    assert_ne!(committed.interact_rect, original.registered.interact_rect);
                    assert!(committed.interact_rect.is_positive());
                }
                Late::ResenseWidget => assert_ne!(committed.sense, original.registered.sense),
                Late::CoverWidget | Late::CoverLayer | Late::VisualWidgetCover | Late::VisualLayerCover => assert_eq!(
                    committed, original.registered,
                    "coverage alone must revoke a still-existing enabled widget"
                ),
                Late::FocusWidget => assert_eq!(
                    frames.ctx.memory(|memory| memory.focused()),
                    Some(Id::unique("copy_guard_late_editor"))
                ),
                _ => unreachable!(),
            }
            assert!(
                frames.show(vec![], Late::None).is_none(),
                "a rejected proposal cannot retry on an empty frame"
            );
            assert!(!frames.model.operation.as_ref().unwrap().apply_submitted);
        }
    }
}

#[test]
fn profile_copy_ui_actual_late_transform_rejects_nonfinite_y_inverse_and_nonidentity_for_every_modality() {
    use egui::emath::TSTransform;
    let bad = [
        TSTransform::from_translation(egui::vec2(f32::INFINITY, 0.0)),
        TSTransform::from_translation(egui::vec2(0.0, f32::INFINITY)),
        TSTransform::from_translation(egui::vec2(0.0, f32::NAN)),
        TSTransform::from_scaling(0.0),
        TSTransform::from_scaling(f32::NAN),
        TSTransform::from_scaling(f32::from_bits(1)),
        TSTransform::from_scaling(f32::MAX),
        TSTransform::from_translation(egui::vec2(9.0, 3.0)),
        TSTransform::from_scaling(1.5),
    ];
    for modality in [Modality::Pointer, Modality::Enter, Modality::Space, Modality::Accessibility] {
        for transform in bad {
            let mut frames = Frames::new();
            frames.prepare();
            let original = frames.control(|intent| matches!(intent, Intent::Apply(_)));
            assert!(release_with(&mut frames, original, modality, Late::Transform(transform)).is_none());
            assert_eq!(
                frames
                    .ctx
                    .viewport(|viewport| viewport.prev_pass.widgets.get(original.widget).copied()),
                Some(original.registered),
                "reject transform change while original widget remains committed"
            );
            assert!(frames.show(vec![], Late::None).is_none());
            assert!(!frames.model.operation.as_ref().unwrap().apply_submitted);
        }
    }
    let rect = Rect::from_min_size(egui::pos2(1.0, 1.0), egui::vec2(90.0, 24.0));
    assert!(transformed_rect(rect, None).is_some());
    for invalid in [
        Rect::ZERO,
        Rect::EVERYTHING,
        Rect::from_min_max(egui::pos2(-f32::MAX, 0.0), egui::pos2(f32::MAX, 24.0)),
    ] {
        assert!(transformed_rect(invalid, None).is_none());
    }
}

#[test]
fn profile_copy_ui_final_registry_disabled_hidden_removed_and_layer_turnover_have_no_stale_fallback() {
    for fault in 0..5 {
        let mut frames = Frames::new();
        frames.prepare();
        let original = frames.control(|intent| matches!(intent, Intent::Apply(_)));
        assert!(original.committed(&frames.ctx));
        // A real subsequent committed layout replaces the registry. Keep the
        // original control unchanged; no later-layout proof is reminted.
        let _output = frames.ctx.run_ui(
            egui::RawInput {
                focused: true,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, frames.size)),
                ..Default::default()
            },
            |ui| match fault {
                0 => {
                    frames
                        .model
                        .show(ui.ctx(), &crate::theme::Theme::default().palette(), &frames.snapshot, true);
                }
                1 => {
                    widgets::Modal::new(original.layer.id).show(ui.ctx(), |ui| {
                        ui.set_invisible();
                        ui.interact(original.registered.rect, original.widget, original.registered.sense);
                    });
                }
                2 => {}
                3 => {
                    egui::Area::new(Id::unique("copy_guard_replacement_layer"))
                        .order(egui::Order::Foreground)
                        .show(ui.ctx(), |ui| {
                            ui.interact(original.registered.rect, original.widget, original.registered.sense);
                        });
                }
                4 => {
                    widgets::Modal::new(original.layer.id).show(ui.ctx(), |ui| {
                        ui.interact(original.registered.rect, original.widget, egui::Sense::hover());
                    });
                }
                _ => unreachable!(),
            },
        );
        let committed = frames
            .ctx
            .viewport(|viewport| viewport.prev_pass.widgets.get(original.widget).copied());
        match fault {
            0 => assert!(!committed.unwrap().enabled),
            1 => assert!(!committed.unwrap().visible),
            2 => assert!(committed.is_none()),
            3 => assert_ne!(committed.unwrap().layer_id, original.layer),
            4 => assert!(!committed.unwrap().sense.senses_click()),
            _ => unreachable!(),
        }
        assert!(!original.committed(&frames.ctx));
        assert!(!frames.model.operation.as_ref().unwrap().apply_submitted);
    }
}

#[test]
fn profile_copy_ui_final_registry_guard_also_precedes_inspect_and_cancel_metadata() {
    for cancel in [false, true] {
        let mut frames = Frames::new();
        if cancel {
            frames.prepare();
        }
        let control = frames.control(|intent| {
            if cancel {
                matches!(intent, Intent::Close(_))
            } else {
                matches!(intent, Intent::Inspect(_))
            }
        });
        assert!(frames.click(control, Late::MoveWidget).is_none());
        assert!(frames.model.is_open());
        assert!(frames.model.cancellation().is_none());
        if !cancel {
            assert!(frames.model.operation.is_none());
        }
        assert!(frames.show(vec![], Late::None).is_none());
    }
    // The unchanged contract is one original release, never deferred replay.
    for modality in [Modality::Pointer, Modality::Enter, Modality::Space, Modality::Accessibility] {
        let mut frames = Frames::new();
        let exact = frames.prepare();
        let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
        assert_eq!(release_with(&mut frames, apply, modality, Late::None), Some(Command::Apply(exact)));
        assert!(frames.show(vec![], Late::None).is_none());
    }
}

#[test]
fn profile_copy_ui_raw_modifier_up_and_same_batch_alt_aba_revoke_plain_key_gesture() {
    for key in [Key::Enter, Key::Space] {
        for modifiers in [
            Modifiers::ALT,
            Modifiers::CTRL,
            Modifiers::SHIFT,
            Modifiers::MAC_CMD,
            Modifiers::COMMAND,
        ] {
            let mut frames = Frames::new();
            frames.prepare();
            let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
            frames.ctx.memory_mut(|memory| memory.request_focus(apply.widget));
            frames.show(vec![], Late::None);
            assert!(frames.show(vec![key_event(key, true, Modifiers::NONE)], Late::None).is_none());
            assert!(frames.show(vec![key_event(key, false, modifiers)], Late::None).is_none());
            assert!(frames.show(vec![], Late::None).is_none());
        }
        let mut frames = Frames::new();
        frames.prepare();
        let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
        frames.ctx.memory_mut(|memory| memory.request_focus(apply.widget));
        frames.show(vec![], Late::None);
        frames.show(vec![key_event(key, true, Modifiers::NONE)], Late::None);
        assert!(
            frames
                .show(
                    vec![
                        Event::ModifiersChanged(Modifiers::ALT),
                        Event::ModifiersChanged(Modifiers::NONE),
                        key_event(key, false, Modifiers::NONE)
                    ],
                    Late::None
                )
                .is_none()
        );
        assert!(frames.show(vec![], Late::None).is_none());
    }
}

#[test]
fn profile_copy_ui_native_modifier_events_between_batches_preserve_physical_fence_and_require_fresh_plain_press() {
    use winit::event::{ElementState, WindowEvent};
    use winit::keyboard::{Key as NativeKey, ModifiersState, NamedKey};
    for native_key in [
        NativeKey::Named(NamedKey::Enter),
        NativeKey::Named(NamedKey::Space),
        NativeKey::Character(" ".into()),
    ] {
        for alt_graph in [false, true] {
            let mut frames = Frames::new();
            let exact = frames.prepare();
            let key = if native_key == NativeKey::Named(NamedKey::Enter) {
                Key::Enter
            } else {
                Key::Space
            };
            let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
            frames.ctx.memory_mut(|memory| memory.request_focus(apply.widget));
            frames.show(vec![], Late::None);
            frames.model.native_key(&native_key, ElementState::Pressed, false, false);
            frames.show(vec![key_event(key, true, Modifiers::NONE)], Late::None);
            if alt_graph {
                frames
                    .model
                    .native_key(&NativeKey::Named(NamedKey::AltGraph), ElementState::Pressed, false, false);
            }
            let state = if alt_graph {
                ModifiersState::CONTROL | ModifiersState::ALT
            } else {
                ModifiersState::ALT
            };
            frames.model.native_event(&WindowEvent::ModifiersChanged(state.into()));
            frames
                .model
                .native_event(&WindowEvent::ModifiersChanged(ModifiersState::empty().into()));
            if alt_graph {
                frames
                    .model
                    .native_key(&NativeKey::Named(NamedKey::AltGraph), ElementState::Released, false, true);
                assert!(frames.model.alt_graph, "synthetic modifier release is not a physical release");
                frames
                    .model
                    .native_key(&NativeKey::Named(NamedKey::AltGraph), ElementState::Released, false, false);
            }
            assert!(frames.model.gesture.is_none());
            frames.show(vec![], Late::None);
            frames.show(vec![], Late::None);
            // An apparent fresh=false-repeat down without a real up remains
            // physically held despite modifier ABA or logical UI cancellation.
            frames.model.native_key(&native_key, ElementState::Pressed, false, false);
            assert!(!frames.model.key_allowed(key));
            assert!(frames.show(vec![key_event(key, true, Modifiers::NONE)], Late::None).is_none());
            frames.model.native_key(&native_key, ElementState::Released, false, false);
            assert!(frames.show(vec![key_event(key, false, Modifiers::NONE)], Late::None).is_none());
            frames.show(vec![], Late::None);
            frames.model.native_key(&native_key, ElementState::Pressed, false, false);
            assert!(frames.show(vec![key_event(key, true, Modifiers::NONE)], Late::None).is_none());
            frames.model.native_key(&native_key, ElementState::Released, false, false);
            assert_eq!(
                frames.show(vec![key_event(key, false, Modifiers::NONE)], Late::None),
                Some(Command::Apply(exact))
            );
        }
    }
}
