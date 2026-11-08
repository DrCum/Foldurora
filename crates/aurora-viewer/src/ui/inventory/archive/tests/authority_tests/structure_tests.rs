//! Actual pinned egui Frame allocations; no hover/ancestor blanket exemption.
use super::*;

#[test]
fn inventory_archive_structure_actual_popup_frame_keeps_both_original_intents_live_for_every_modality() {
    for include in [false, true] {
        for modality in MODALITIES {
            let mut frames = Frames::new();
            let original = original(&mut frames, include);
            let witness = original.structure.expect("real popup Frame sealed after owner draw");
            assert!(witness.allocation_count() > 0);
            assert!(original.committed(&frames.frames.ctx));
            release(&mut frames, original, modality, Late::None);
            assert_eq!(frames.proposals.len(), 1);
            assert!(valid(&frames));
            let current = if include {
                frames.state.archive_include_context.activation.control
            } else {
                frames.state.archive_context.activation.control
            }
            .unwrap();
            assert_eq!(current.structure, Some(witness), "unchanged original/release structural witness");
            assert!(current.committed(&frames.frames.ctx));
            match &frames.proposals[0].intent {
                Intent::Edit { ticket, .. } => assert!(frames.state.archive_context.ticket.as_ref() == Some(ticket)),
                Intent::Include { .. } => assert!(frames.state.archive_context.ticket.is_none()),
            }
        }
    }
}

#[derive(Clone, Copy)]
enum FrameLate {
    None,
    DuplicateOuter,
    VisualControlCover,
    ChangedParent,
}
fn nested(late: FrameLate) -> (egui::Context, Control) {
    let ctx = egui::Context::default();
    crate::theme::Theme::default().apply(&ctx, 1.0);
    let mut saved = None;
    for _ in 0..3 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                focused: true,
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                ..Default::default()
            },
            |host| {
                egui::Area::new(egui::Id::unique("archive_structural_real_menu_area"))
                    .kind(egui::UiKind::Menu)
                    .order(egui::Order::Foreground)
                    .fixed_pos(egui::pos2(100.0, 100.0))
                    .show(host.ctx(), |ui| {
                        let mut prepared = egui::Frame::popup(ui.style()).begin(ui);
                        egui::Frame::group(prepared.content_ui.style()).show(&mut prepared.content_ui, |ui| {
                            let response = crate::ui::menu::editable_row(
                                ui,
                                &crate::theme::Theme::default().palette(),
                                "archive-filled",
                                aurora_i18n::tr!("inventory-archive-archive"),
                                true,
                                None,
                                None,
                                false,
                            );
                            saved = Control::capture(ui, &response, InputRead::default());
                        });
                        let response = prepared.end(ui);
                        match late {
                            FrameLate::None => {}
                            FrameLate::DuplicateOuter => {
                                ui.interact(
                                    response.rect,
                                    egui::Id::unique("archive_structural_same_outer_overlay"),
                                    egui::Sense::hover(),
                                );
                            }
                            FrameLate::VisualControlCover => {
                                if let Some(control) = saved {
                                    ui.interact(
                                        control.registered.rect,
                                        egui::Id::unique("archive_structural_visual_sibling"),
                                        egui::Sense::hover(),
                                    );
                                }
                            }
                            FrameLate::ChangedParent => {
                                ui.disable();
                            }
                        }
                    });
                if let Some(control) = saved.as_mut() {
                    control.structure = host
                        .ctx()
                        .viewport(|viewport| structure::Witness::capture(&viewport.this_pass.widgets, control));
                }
            },
        );
        output.textures_delta.clear();
    }
    (ctx, saved.unwrap())
}

#[test]
fn inventory_archive_structure_actual_nested_frames_seal_unique_exact_allocations_and_original_parent_chain() {
    let (ctx, control) = nested(FrameLate::None);
    assert_eq!(control.structure.unwrap().allocation_count(), 2);
    assert!(control.committed(&ctx), "both real enclosing Frames are background bookkeeping");
    let recaptured = ctx.viewport(|viewport| structure::Witness::capture(&viewport.prev_pass.widgets, &control));
    assert_eq!(recaptured, control.structure);
}

#[test]
fn inventory_archive_structure_real_same_outer_visual_ambiguity_changed_parent_and_other_hover_cover_fail_closed() {
    for late in [FrameLate::DuplicateOuter, FrameLate::VisualControlCover, FrameLate::ChangedParent] {
        let (ctx, control) = nested(late);
        match late {
            FrameLate::DuplicateOuter | FrameLate::ChangedParent => assert!(control.structure.is_none()),
            FrameLate::VisualControlCover => assert!(
                control.structure.is_some(),
                "actual Frame witness remains exact; sibling is still a cover"
            ),
            FrameLate::None => unreachable!(),
        }
        assert!(!control.committed(&ctx));
    }
}

#[test]
fn inventory_archive_structure_previously_covered_control_cannot_seed_any_modality_when_visual_cover_disappears() {
    for include in [false, true] {
        for modality in MODALITIES {
            let mut frames = Frames::new();
            let original = original(&mut frames, include);
            frames.frames.ctx.memory_mut(|memory| memory.request_focus(original.widget));
            frames.cover_original = Some(original);
            frames.run(Vec::new());
            let previous = if include {
                frames.state.archive_include_context.activation.control
            } else {
                frames.state.archive_context.activation.control
            }
            .unwrap();
            assert!(
                previous.registry(&frames.frames.ctx, true),
                "metadata alone remains live beneath actual visual sibling"
            );
            assert!(!previous.committed(&frames.frames.ctx), "the original settled control was covered");
            assert!(frames.frames.ctx.viewport(|viewport| {
                viewport
                    .prev_pass
                    .widgets
                    .get(egui::Id::unique("archive_authority_same_layer_cover"))
                    .is_some()
            }));
            frames.cover_original = None;
            controller_testing::reset_assignments();
            match modality {
                Modality::Pointer => {
                    frames.pointer(original.rect.center(), egui::PointerButton::Primary, true);
                    frames.pointer(original.rect.center(), egui::PointerButton::Primary, false);
                }
                Modality::Enter | Modality::Space => {
                    let keycode = if matches!(modality, Modality::Enter) {
                        egui::Key::Enter
                    } else {
                        egui::Key::Space
                    };
                    frames.run(vec![key(keycode, true)]);
                    frames.run(vec![key(keycode, false)]);
                }
                Modality::Accessibility => {
                    frames.run(vec![egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
                        action: egui::accesskit::Action::Click,
                        target_tree: egui::accesskit::TreeId::ROOT,
                        target_node: original.widget.accesskit_id(),
                        data: None,
                    })]);
                }
            }
            assert!(
                frames.proposals.is_empty(),
                "removing the cover on activation does not grant an original gesture"
            );
            assert!(!frames.state.archive.as_ref().unwrap().policy.included());
            assert_eq!(controller_testing::assignments(), 0);
            frames.run(Vec::new());
            assert!(frames.proposals.is_empty(), "no deferred replay");
            egui::Popup::close_all(&frames.frames.ctx);
            let fresh = super::original(&mut frames, include);
            assert!(
                fresh.committed(&frames.frames.ctx),
                "a genuinely settled uncovered control can accept fresh input"
            );
            release(&mut frames, fresh, modality, Late::None);
            assert_eq!(frames.proposals.len(), 1);
            assert!(valid(&frames));
            assert!(!frames.state.archive.as_ref().unwrap().policy.included());
            assert_eq!(controller_testing::assignments(), 0);
        }
    }
}
