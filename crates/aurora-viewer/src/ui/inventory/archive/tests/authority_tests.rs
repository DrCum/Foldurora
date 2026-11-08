//! Real consumer/control registry and native metadata histories, authored UNRUN.
//! Same actual World/Inventory/menu/read/Activation as existing source tests;
//! no Worker, private file, native window or parallel admission implementation.
use super::*;
use winit::{
    event::{ElementState, WindowEvent},
    keyboard::{Key as NativeKey, KeyCode, ModifiersState, NamedKey, PhysicalKey},
};

#[derive(Clone, Copy)]
pub(super) enum Late {
    None,
    Move,
    Reclip,
    Resense,
    Disable,
    Hide,
    SameLayerCover,
    VisualCover,
    CoverLayer,
    VisualLayer,
    Focus,
    Transform(egui::emath::TSTransform),
}
impl Late {
    pub(super) fn apply(self, ui: &mut egui::Ui, control: Control) {
        match self {
            Self::None => return,
            Self::Transform(_) | Self::Disable | Self::Hide => return,
            Self::CoverLayer | Self::VisualLayer => {
                egui::Area::new(egui::Id::unique("archive_authority_late_area"))
                    .order(egui::Order::Foreground)
                    .interactable(matches!(self, Self::CoverLayer))
                    .fixed_pos(control.rect.min)
                    .show(ui.ctx(), |ui| {
                        ui.allocate_response(control.rect.size(), egui::Sense::click());
                    });
                return;
            }
            _ => {}
        }
        ui.ctx().options_mut(|options| options.warn_on_id_clash = false);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("archive_authority_late_widget")
                .layer_id(control.layer)
                .max_rect(ui.ctx().content_rect()),
        );
        child.set_clip_rect(control.paint_clip);
        match self {
            Self::Move => {
                child.interact(
                    control.registered.rect.translate(egui::vec2(11.0, 0.0)),
                    control.widget,
                    control.registered.sense,
                );
            }
            Self::Reclip => {
                child.set_clip_rect(egui::Rect::from_min_max(
                    control.registered.rect.min,
                    egui::pos2(control.registered.rect.center().x, control.registered.rect.max.y),
                ));
                child.interact(control.registered.rect, control.widget, control.registered.sense);
            }
            Self::Resense => {
                child.interact(
                    control.registered.rect,
                    control.widget,
                    control.registered.sense | egui::Sense::drag(),
                );
            }
            Self::SameLayerCover | Self::VisualCover => {
                child.interact(
                    control.registered.rect,
                    egui::Id::unique("archive_authority_same_layer_cover"),
                    if matches!(self, Self::SameLayerCover) {
                        egui::Sense::click()
                    } else {
                        egui::Sense::hover()
                    },
                );
            }
            Self::Focus => {
                let mut text = String::new();
                child
                    .put(
                        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(80.0, 24.0)),
                        egui::TextEdit::singleline(&mut text).id(egui::Id::unique("archive_authority_late_editor")),
                    )
                    .request_focus();
            }
            _ => {}
        }
    }
}
#[derive(Clone, Copy, Debug)]
enum Modality {
    Pointer,
    Enter,
    Space,
    Accessibility,
}
const MODALITIES: [Modality; 4] = [Modality::Pointer, Modality::Enter, Modality::Space, Modality::Accessibility];
fn key(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}
fn original(frames: &mut Frames, include: bool) -> Control {
    if include {
        let heading = frames.settled_visibility_heading();
        frames.run(vec![egui::Event::PointerMoved(heading.rect.center())]);
        let position = frames.settled_visibility_heading().rect.center();
        frames.pointer(position, egui::PointerButton::Primary, true);
        frames.pointer(position, egui::PointerButton::Primary, false);
        let control = frames.settled(true);
        frames.run(vec![egui::Event::PointerMoved(control.rect.center())]);
        frames.settled(true)
    } else {
        frames.open_context();
        frames.state.archive_context.activation.control.expect("actual Archive control")
    }
}
fn release(frames: &mut Frames, control: Control, modality: Modality, late: Late) {
    trace_release(frames, control, modality, "before");
    match modality {
        Modality::Pointer => {
            frames.pointer(control.rect.center(), egui::PointerButton::Primary, true);
            trace_release(frames, control, modality, "down");
            frames.late = late;
            frames.pointer(control.rect.center(), egui::PointerButton::Primary, false);
        }
        Modality::Enter | Modality::Space => {
            let keycode = if matches!(modality, Modality::Enter) {
                egui::Key::Enter
            } else {
                egui::Key::Space
            };
            frames.frames.ctx.memory_mut(|memory| memory.request_focus(control.widget));
            frames.run(Vec::new());
            frames.run(vec![key(keycode, true)]);
            trace_release(frames, control, modality, "down");
            frames.late = late;
            frames.run(vec![key(keycode, false)]);
        }
        Modality::Accessibility => {
            frames.frames.ctx.memory_mut(|memory| memory.request_focus(control.widget));
            frames.run(Vec::new());
            frames.late = late;
            frames.run(vec![egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: control.widget.accesskit_id(),
                data: None,
            })]);
        }
    }
    trace_release(frames, control, modality, "after");
}

fn trace_release(frames: &Frames, original: Control, modality: Modality, phase: &str) {
    let ctx = &frames.frames.ctx;
    let include = frames
        .state
        .archive_include_context
        .activation
        .control
        .is_some_and(|control| control.widget == original.widget);
    eprintln!(
        "ARCHIVE_RELEASE phase={phase} include={include} modality={modality:?} original={:?} rect={:?} frame={} pass={} proposals={} popup={} focus={:?} raw={:?}",
        original.widget,
        original.rect,
        ctx.cumulative_frame_nr(),
        ctx.cumulative_pass_nr(),
        frames.proposals.len(),
        egui::Popup::is_any_open(ctx),
        ctx.memory(|memory| memory.focused()),
        ctx.input(|input| (
            input.focused,
            input.modifiers,
            input.pointer.button_down(egui::PointerButton::Primary),
            input.key_down(egui::Key::Enter),
            input.key_down(egui::Key::Space)
        ))
    );
    if include {
        let heading = ctx.data(|data| data.get_temp::<(egui::WidgetRect, egui::Frame, egui::Vec2)>(visibility_heading_key()));
        eprintln!(
            "ARCHIVE_INCLUDE_PLACEMENT heading={heading:?} child_area={:?} pointer={:?}",
            egui::AreaState::load(ctx, original.layer.id).map(|area| (area.rect(), area.size)),
            ctx.pointer_hover_pos()
        );
        if let Some((heading, _, _)) = heading {
            eprintln!(
                "ARCHIVE_INCLUDE_PARENT area={:?} bounds={:?}",
                egui::AreaState::load(ctx, heading.layer_id.id).map(|area| (area.rect(), area.size)),
                crate::ui::layout::toolbar_layer_bounds(ctx, heading.layer_id)
            );
        }
    }
    for (kind, activation) in [
        ("edit", &frames.state.archive_context.activation),
        ("include", &frames.state.archive_include_context.activation),
    ] {
        eprintln!(
            "ARCHIVE_ACTIVATION kind={kind} blocked={} input_frame={:?} frame={:?} gesture={:?} released={} settled_focus={:?}",
            activation.blocked,
            activation.input_frame,
            activation.frame,
            activation
                .gesture
                .map(|gesture| (gesture.control.widget, gesture.key, gesture.control.frame)),
            activation.released.is_some(),
            activation.settled_focus
        );
        for (label, control) in [("current", activation.control), ("previous", activation.previous)] {
            if let Some(control) = control {
                eprintln!(
                    "ARCHIVE_RELEASE_CONTROL {kind}/{label} widget={:?} frame={} pass={} same_original={} geometry={} registry={} committed={} rect={:?} registered={:?} structure_present={} same_structure={}",
                    control.widget,
                    control.frame,
                    control.pass,
                    control.same_widget(original),
                    control.geometry(ctx),
                    control.registry(ctx, true),
                    control.committed(ctx),
                    control.rect,
                    control.registered,
                    control.structure.is_some(),
                    control.structure == original.structure
                );
            }
        }
    }
}
fn valid(frames: &Frames) -> bool {
    frames.proposals.last().is_some_and(|proposal| {
        frames
            .state
            .archive_proposal_current(&frames.frames.ctx, true, &frames.prefs, proposal)
    })
}

#[test]
fn inventory_archive_authority_actual_edit_and_include_unchanged_committed_controls_allow_every_modality_without_paint_effect() {
    for include in [false, true] {
        for modality in MODALITIES {
            let mut frames = Frames::new();
            let control = original(&mut frames, include);
            controller_testing::reset_assignments();
            release(&mut frames, control, modality, Late::None);
            assert_eq!(frames.proposals.len(), 1);
            assert!(valid(&frames));
            assert_eq!(controller_testing::assignments(), 0);
            assert!(
                !frames.state.archive.as_ref().unwrap().policy.included(),
                "Include only posts original intent in paint"
            );
            assert!(frames.proposals[0].input_current(InputRead::default()));
        }
    }
}

#[test]
fn inventory_archive_authority_actual_final_registry_geometry_disable_sense_and_coverage_reject_both_intents_all_modalities() {
    for include in [false, true] {
        for modality in MODALITIES {
            for late in [
                Late::Move,
                Late::Reclip,
                Late::Resense,
                Late::Disable,
                Late::Hide,
                Late::SameLayerCover,
                Late::VisualCover,
                Late::CoverLayer,
                Late::VisualLayer,
                Late::Focus,
            ] {
                let mut frames = Frames::new();
                let control = original(&mut frames, include);
                controller_testing::reset_assignments();
                release(&mut frames, control, modality, late);
                assert!(!valid(&frames));
                assert!(!frames.state.archive.as_ref().unwrap().policy.included());
                assert_eq!(controller_testing::assignments(), 0);
                frames.proposals.clear();
                frames.late = Late::None;
                frames.run(Vec::new());
                assert!(
                    frames.proposals.is_empty(),
                    "rejected original cannot replay on a later empty batch"
                );
            }
        }
    }
}

#[test]
fn inventory_archive_authority_actual_late_transform_checks_both_translations_inverse_size_and_original_transform_all_modalities() {
    use egui::emath::TSTransform as T;
    for include in [false, true] {
        for modality in MODALITIES {
            for transform in [
                T::from_translation(egui::vec2(f32::INFINITY, 0.0)),
                T::from_translation(egui::vec2(0.0, f32::INFINITY)),
                T::from_translation(egui::vec2(0.0, f32::NAN)),
                T::from_translation(egui::vec2(10_000.0, 10_000.0)),
                T::from_scaling(0.0),
                T::from_scaling(f32::NAN),
                T::from_scaling(f32::from_bits(1)),
                T::from_scaling(f32::MAX),
                T::from_translation(egui::vec2(9.0, 3.0)),
                T::from_scaling(1.5),
            ] {
                let mut frames = Frames::new();
                let control = original(&mut frames, include);
                release(&mut frames, control, modality, Late::Transform(transform));
                assert!(!valid(&frames));
                let committed = frames
                    .frames
                    .ctx
                    .viewport(|viewport| viewport.prev_pass.widgets.get(control.widget).copied());
                assert_eq!(
                    committed,
                    Some(control.registered),
                    "transform alone revokes a still committed widget"
                );
                assert!(!frames.state.archive.as_ref().unwrap().policy.included());
            }
        }
    }
    let rect = egui::Rect::from_min_size(egui::pos2(1.0, 1.0), egui::vec2(90.0, 24.0));
    assert!(transformed_rect(rect, None).is_some());
    assert!(transformed_rect(rect, Some(T::new(egui::vec2(3.0, 5.0), 1.5))).is_some());
    for rect in [
        egui::Rect::ZERO,
        egui::Rect::EVERYTHING,
        egui::Rect::from_min_max(egui::pos2(-f32::MAX, 0.0), egui::pos2(f32::MAX, 24.0)),
    ] {
        assert!(transformed_rect(rect, None).is_none());
    }
}

fn native(frames: &mut Frames, input: &mut NativeInput, event: WindowEvent) {
    if input.event(&event) {
        frames.state.revoke_archive_gestures();
    }
}
fn native_key(
    frames: &mut Frames,
    input: &mut NativeInput,
    logical: &NativeKey,
    physical: PhysicalKey,
    state: ElementState,
    synthetic: bool,
) {
    if input.key(logical, physical, state, false, synthetic) {
        frames.state.revoke_archive_gestures();
    }
}
fn input_frame(frames: &mut Frames, input: &mut NativeInput, events: Vec<egui::Event>) {
    frames.state.archive_input(input.read());
    frames.run(events);
    input.finish();
}

#[test]
fn inventory_archive_authority_native_modifier_and_hidden_altgraph_aba_between_batches_never_rearm_held_enter_or_space() {
    for include in [false, true] {
        for (keycode, logical) in [
            (egui::Key::Enter, NativeKey::Named(NamedKey::Enter)),
            (egui::Key::Space, NativeKey::Named(NamedKey::Space)),
            (egui::Key::Space, NativeKey::Character(" ".into())),
        ] {
            for alt_graph in [false, true] {
                let mut frames = Frames::new();
                let control = original(&mut frames, include);
                let mut input = NativeInput::default();
                let physical = PhysicalKey::Code(if keycode == egui::Key::Enter {
                    KeyCode::Enter
                } else {
                    KeyCode::Space
                });
                frames.frames.ctx.memory_mut(|memory| memory.request_focus(control.widget));
                input_frame(&mut frames, &mut input, vec![]);
                let original_ticket = frames.state.archive_context.ticket.clone();
                native_key(&mut frames, &mut input, &logical, physical, ElementState::Pressed, false);
                input_frame(&mut frames, &mut input, vec![key(keycode, true)]);
                if alt_graph {
                    native_key(
                        &mut frames,
                        &mut input,
                        &NativeKey::Named(NamedKey::AltGraph),
                        PhysicalKey::Code(KeyCode::AltRight),
                        ElementState::Pressed,
                        false,
                    );
                    assert!(!input.read().plain(), "winit may hide both ordinary modifier bits for AltGraph");
                    native_key(
                        &mut frames,
                        &mut input,
                        &NativeKey::Named(NamedKey::AltGraph),
                        PhysicalKey::Code(KeyCode::AltRight),
                        ElementState::Released,
                        true,
                    );
                    input.finish();
                    assert!(!input.read().plain(), "synthetic AltGraph release does not rearm plain input");
                    native_key(
                        &mut frames,
                        &mut input,
                        &NativeKey::Named(NamedKey::Alt),
                        PhysicalKey::Code(KeyCode::AltRight),
                        ElementState::Released,
                        false,
                    );
                } else {
                    native(&mut frames, &mut input, WindowEvent::ModifiersChanged(ModifiersState::ALT.into()));
                    native(
                        &mut frames,
                        &mut input,
                        WindowEvent::ModifiersChanged(ModifiersState::empty().into()),
                    );
                }
                assert_eq!(
                    frames.state.archive_context.ticket, original_ticket,
                    "modifier revocation never remints original context ticket"
                );
                assert!(frames.state.archive_context.activation.gesture.is_none());
                assert!(frames.state.archive_include_context.activation.gesture.is_none());
                input_frame(&mut frames, &mut input, vec![]);
                input_frame(&mut frames, &mut input, vec![]);
                native_key(&mut frames, &mut input, &logical, physical, ElementState::Pressed, false);
                assert!(
                    !input.read().key_allowed(keycode),
                    "repeat=false down without physical release remains held"
                );
                input_frame(&mut frames, &mut input, vec![key(keycode, true)]);
                assert!(frames.proposals.is_empty());
                native_key(&mut frames, &mut input, &logical, physical, ElementState::Released, false);
                input_frame(&mut frames, &mut input, vec![key(keycode, false)]);
                assert!(frames.proposals.is_empty());
                input_frame(&mut frames, &mut input, vec![]);
                native_key(&mut frames, &mut input, &logical, physical, ElementState::Pressed, false);
                input_frame(&mut frames, &mut input, vec![key(keycode, true)]);
                native_key(&mut frames, &mut input, &logical, physical, ElementState::Released, false);
                input_frame(&mut frames, &mut input, vec![key(keycode, false)]);
                assert_eq!(frames.proposals.len(), 1);
                assert!(valid(&frames));
                assert!(frames.proposals[0].input_current(input.read()));
            }
        }
    }
}

#[test]
fn inventory_archive_authority_raw_modifier_aba_and_checked_native_epoch_exhaustion_fail_closed() {
    for include in [false, true] {
        let mut frames = Frames::new();
        let control = original(&mut frames, include);
        frames.frames.ctx.memory_mut(|memory| memory.request_focus(control.widget));
        frames.run(vec![]);
        frames.run(vec![key(egui::Key::Enter, true)]);
        frames.run(vec![
            egui::Event::ModifiersChanged(egui::Modifiers {
                alt: true,
                ..egui::Modifiers::NONE
            }),
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            key(egui::Key::Enter, false),
        ]);
        assert!(frames.proposals.is_empty());
    }
    let mut frames = Frames::new();
    let control = original(&mut frames, false);
    release(&mut frames, control, Modality::Pointer, Late::None);
    let proposal = frames.proposals.last().unwrap();
    let mut input = NativeInput::default();
    assert!(proposal.input_current(input.read()));
    input.event(&WindowEvent::ModifiersChanged(ModifiersState::ALT.into()));
    input.event(&WindowEvent::ModifiersChanged(ModifiersState::empty().into()));
    input.finish();
    assert!(input.read().plain());
    assert!(
        !proposal.input_current(input.read()),
        "returning plain cannot revive original native epoch"
    );
    let mut input = NativeInput::default();
    input.exhaust_epoch();
    input.event(&WindowEvent::ModifiersChanged(ModifiersState::ALT.into()));
    input.finish();
    assert!(!input.read().plain());
    assert_eq!(input.read().epoch(), 0);
}

#[test]
fn inventory_archive_authority_actual_committed_registry_removal_hidden_disabled_layer_id_and_clip_turnover_have_no_fallback() {
    for include in [false, true] {
        for fault in 0..7 {
            let mut frames = Frames::new();
            let control = original(&mut frames, include);
            assert!(control.committed(&frames.frames.ctx));
            let mut output = frames.frames.ctx.run_ui(
                egui::RawInput {
                    focused: true,
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                    ..Default::default()
                },
                |host| {
                    if fault == 2 {
                        return;
                    }
                    let area = if fault == 3 {
                        egui::Id::unique("archive_authority_other_layer")
                    } else {
                        control.layer.id
                    };
                    egui::Area::new(area)
                        .order(control.layer.order)
                        .fixed_pos(egui::Pos2::ZERO)
                        .show(host.ctx(), |ui| {
                            ui.set_clip_rect(if fault == 6 { egui::Rect::ZERO } else { control.paint_clip });
                            if fault == 0 {
                                ui.disable();
                            }
                            if fault == 1 {
                                ui.set_invisible();
                            }
                            ui.interact(
                                control.registered.rect,
                                if fault == 5 {
                                    control.widget.with("replacement")
                                } else {
                                    control.widget
                                },
                                if fault == 4 {
                                    egui::Sense::hover()
                                } else {
                                    control.registered.sense
                                },
                            );
                        });
                },
            );
            output.textures_delta.clear();
            let committed = frames
                .frames
                .ctx
                .viewport(|viewport| viewport.prev_pass.widgets.get(control.widget).copied());
            match fault {
                0 => assert!(!committed.unwrap().enabled),
                1 => assert!(!committed.unwrap().visible),
                2 | 5 => assert!(committed.is_none()),
                3 => assert_ne!(committed.unwrap().layer_id, control.layer),
                4 => assert!(!committed.unwrap().sense.senses_click()),
                6 => assert!(!committed.unwrap().interact_rect.is_positive()),
                _ => unreachable!(),
            }
            assert!(!control.committed(&frames.frames.ctx));
        }
    }
}

#[test]
fn inventory_archive_authority_modifier_revocation_preserves_context_ticket_popup_drafts_and_app_physical_fence_across_ui_reset() {
    let mut frames = Frames::new();
    let control = original(&mut frames, false);
    let ticket = frames.state.archive_context.ticket.clone();
    frames.state.rename = Some(InlineRename {
        id: id(4),
        name: "Retain draft".into(),
        focus: true,
    });
    frames.state.search = "Retain search".into();
    let mut input = NativeInput::default();
    native_key(
        &mut frames,
        &mut input,
        &NativeKey::Named(NamedKey::Enter),
        PhysicalKey::Code(KeyCode::Enter),
        ElementState::Pressed,
        false,
    );
    native(&mut frames, &mut input, WindowEvent::ModifiersChanged(ModifiersState::ALT.into()));
    native(
        &mut frames,
        &mut input,
        WindowEvent::ModifiersChanged(ModifiersState::empty().into()),
    );
    assert_eq!(frames.state.archive_context.ticket, ticket);
    assert_eq!(frames.state.rename.as_ref().unwrap().name, "Retain draft");
    assert_eq!(frames.state.search, "Retain search");
    assert!(
        egui::Popup::is_any_open(&frames.frames.ctx),
        "modifier revocation does not close an editable context popup"
    );
    assert!(frames.state.archive_context.activation.control.is_none());
    assert!(control.visible());
    frames.state = InventoryUi::default();
    input.finish();
    native_key(
        &mut frames,
        &mut input,
        &NativeKey::Named(NamedKey::Enter),
        PhysicalKey::Code(KeyCode::Enter),
        ElementState::Pressed,
        false,
    );
    assert!(
        !input.read().key_allowed(egui::Key::Enter),
        "UI/source reset cannot reset App-owned physical hold"
    );
    native_key(
        &mut frames,
        &mut input,
        &NativeKey::Named(NamedKey::Enter),
        PhysicalKey::Code(KeyCode::Enter),
        ElementState::Released,
        false,
    );
    input.finish();
    native_key(
        &mut frames,
        &mut input,
        &NativeKey::Named(NamedKey::Enter),
        PhysicalKey::Code(KeyCode::Enter),
        ElementState::Pressed,
        false,
    );
    assert!(input.read().key_allowed(egui::Key::Enter));
}

#[test]
fn inventory_archive_authority_pointer_release_keeps_first_owner_focus_across_late_editor_and_discard_retry() {
    for include in [false, true] {
        let mut frames = Frames::new();
        let control = original(&mut frames, include);
        frames.pointer(control.rect.center(), egui::PointerButton::Primary, true);
        let before = frames.frames.ctx.cumulative_pass_nr();
        frames.late = Late::Focus;
        frames.discard = true;
        controller_testing::reset_assignments();
        frames.pointer(control.rect.center(), egui::PointerButton::Primary, false);
        assert!(
            frames.frames.ctx.cumulative_pass_nr() >= before + 2,
            "real whole-run retry occurred"
        );
        assert!(
            !frames.proposals.is_empty(),
            "retain the original release proposal for final rejection"
        );
        let focused = frames.frames.ctx.memory(|memory| memory.focused());
        assert_eq!(focused, Some(egui::Id::unique("archive_authority_late_editor")));
        let activation = if include {
            &frames.state.archive_include_context.activation
        } else {
            &frames.state.archive_context.activation
        };
        assert!(activation.settled_focus.is_some());
        assert_ne!(activation.settled_focus, Some(focused), "retry cannot remint the release focus");
        assert!(!valid(&frames));
        assert!(!frames.state.archive.as_ref().unwrap().policy.included());
        assert_eq!(controller_testing::assignments(), 0);
    }
}

mod structure_tests;
