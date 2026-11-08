//! Synthetic CPU correlation/UI fixtures only. No genuine managed capability is
//! fabricated; App::current and WorkerBridge remain the actual effect boundary.
use super::*;
use crate::gestures::{Entry, Manager};
use crate::world::inventory::Inventory;
use aurora_assets::gesture::Gesture;
use aurora_session::GridKey;
use egui::{Pos2, Vec2};
fn facts() -> Facts {
    Facts {
        agent: Uuid::from_u128(2),
        login: Uuid::from_u128(3),
        grid: GridKey::from_bytes([1; 32]).unwrap(),
        logical_generation: 1,
        profile_generation: 1,
        managed: None,
    }
}
fn actor() -> State {
    let mut s = State::default();
    s.epoch = 1;
    s.actions = 1;
    s.open = true;
    s.facts = Some(facts());
    s.runtime.bind(s.facts).unwrap();
    s.runtime.window(true).unwrap();
    s
}
fn inventory() -> Inventory {
    use aurora_llsd::{Llsd, llsd_map};
    let root = Uuid::from_u128(100);
    // The seed adds demo items beneath the login skeleton; it does not create
    // the personal root. Use the same initializer as an actual login event.
    let raw = llsd_map! {
        "agent_id" => Llsd::Uuid(facts().agent),
        "inventory-root" => Llsd::Array(vec![llsd_map! { "folder_id" => Llsd::Uuid(root) }]),
        "inventory-skeleton" => Llsd::Array(vec![llsd_map! {
            "folder_id" => Llsd::Uuid(root),
            "parent_id" => Llsd::Uuid(Uuid::nil()),
            "name" => Llsd::String("Fixture inventory".into()),
            "type_default" => Llsd::Integer(8),
            "version" => Llsd::Integer(1)
        }])
    };
    let mut inventory = Inventory::from_login(&raw);
    crate::world::inventory::demo::seed(&mut inventory, facts().agent);
    inventory
}
#[test]
fn board_app_fixture_requires_login_skeleton_before_native_item_resolution() {
    let mut uninitialized = Inventory::default();
    crate::world::inventory::demo::seed(&mut uninitialized, facts().agent);
    assert!(actions::resolve_playback_item(&uninitialized, Uuid::from_u128(8106), PlaybackKind::Gesture).is_err());
    let mut initialized = inventory();
    assert_eq!(initialized.owner, facts().agent);
    assert!(!initialized.root.is_nil());
    let item = actions::resolve_playback_item(&initialized, Uuid::from_u128(8106), PlaybackKind::Gesture).unwrap();
    assert_eq!(item.reference(), Uuid::from_u128(8106));
    initialized.folders.remove(&initialized.root);
    assert!(actions::resolve_playback_item(&initialized, item.reference(), PlaybackKind::Gesture).is_err());
}
#[test]
fn board_app_source_window_action_and_terminal_counter_fences_never_reuse_ready_proofs() {
    let mut state = actor();
    let old = state.proof(None, Instant::now()).unwrap();
    assert!(old.same_owner(state.proof(None, Instant::now() + Duration::from_secs(1)).unwrap()));
    state.cancel_actions();
    assert!(!old.same_owner(state.proof(None, Instant::now()).unwrap()));
    let newer = state.proof(None, Instant::now()).unwrap();
    state.window += 1;
    assert!(!newer.same_owner(state.proof(None, Instant::now()).unwrap()));
    assert_eq!(advance(u64::MAX), 0);
    assert_eq!(advance(0), 0);
    state.local = u64::MAX;
    state.cancel_actions();
    state.cancel_actions();
    assert_eq!(state.local, 0);
    let replacement = actor();
    assert_ne!(old.instance, replacement.instance);
}
#[test]
fn board_app_stale_store_reply_cannot_consume_new_operation_or_new_source_job() {
    let mut state = actor();
    let cancellation = NativeStoreCancellation::new();
    state.job = Some(Job {
        operation: 2,
        cancellation: cancellation.clone(),
        initial: None,
    });
    let owner = state.owner().unwrap();
    assert!(state.take_job(owner, 1).is_none());
    assert_eq!(state.job.as_ref().unwrap().operation, 2);
    let mut wrong = owner;
    wrong.facts.login = Uuid::from_u128(99);
    assert!(state.take_job(wrong, 2).is_none());
    assert!(state.job.is_some());
    wrong = owner;
    wrong.epoch += 1;
    assert!(state.take_job(wrong, 2).is_none());
    cancellation.cancel();
    assert!(state.take_job(owner, 2).is_none());
    assert!(state.job.is_some());
    state.cancel_job();
    assert!(state.job.is_none());
    state.job = Some(Job {
        operation: 3,
        cancellation: NativeStoreCancellation::new(),
        initial: None,
    });
    assert!(state.take_job(owner, 3).is_some());
    assert!(state.job.is_none());
}
#[test]
fn board_app_add_duplicate_checks_actual_resolved_inventory_alias_not_saved_reference() {
    let mut inv = inventory();
    let original = actions::resolve_playback_item(&inv, Uuid::from_u128(8106), PlaybackKind::Gesture).unwrap();
    let mut alias = inv.items[&original.item()].clone();
    alias.id = Uuid::from_u128(990001);
    alias.asset_type = 24;
    alias.asset_id = original.item();
    inv.add_items(vec![alias.clone()]);
    let linked = actions::resolve_playback_item(&inv, alias.id, PlaybackKind::Gesture).unwrap();
    assert_ne!(linked.reference(), original.reference());
    assert_eq!(linked.item(), original.item());
    let id = BoardId::new();
    let data = Catalogue::empty(facts().agent, facts().grid)
        .unwrap()
        .apply(Change::Create { id, name: "First".into() })
        .unwrap()
        .apply(Change::Add {
            board: id,
            tile: Tile {
                id: TileId::new(),
                item: Reference {
                    reference: original.reference(),
                    kind: Kind::Gesture,
                },
            },
        })
        .unwrap();
    assert!(duplicate(&inv, data.board(id).unwrap(), linked));
    assert!(duplicate(&inv, data.board(id).unwrap(), original));
}
struct Frames {
    ctx: egui::Context,
    ui: view::State,
    actor: State,
    data: Catalogue,
    inv: Inventory,
    manager: Manager,
    add: Option<PlaybackItem>,
    open: bool,
    overlay: bool,
}
impl Frames {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let inv = inventory();
        let item = actions::resolve_playback_item(&inv, Uuid::from_u128(8106), PlaybackKind::Gesture).unwrap();
        let board = BoardId::new();
        let data = Catalogue::empty(facts().agent, facts().grid)
            .unwrap()
            .apply(Change::Create {
                id: board,
                name: "First board".into(),
            })
            .unwrap()
            .apply(Change::Add {
                board,
                tile: Tile {
                    id: TileId::new(),
                    item: Reference {
                        reference: item.reference(),
                        kind: Kind::Gesture,
                    },
                },
            })
            .unwrap();
        let mut manager = Manager::default();
        manager.entries.insert(
            item.item(),
            Entry {
                asset: item.asset(),
                definition: Gesture::default(),
            },
        );
        let mut f = Self {
            ctx,
            ui: Default::default(),
            actor: actor(),
            data,
            inv,
            manager,
            add: Some(item),
            open: true,
            overlay: false,
        };
        for _ in 0..5 {
            assert!(f.draw(vec![]).0.is_none());
        }
        f
    }
    fn draw(&mut self, events: Vec<egui::Event>) -> (Option<Action>, Vec<egui::epaint::ClippedShape>) {
        let mut out = None;
        let proof = self.actor.proof(None, Instant::now());
        let mut result = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(1500.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                out = view::show(
                    ui.ctx(),
                    &crate::theme::Theme::default().palette(),
                    &mut self.ui,
                    &mut self.open,
                    view::Context {
                        data: Some(&self.data),
                        selected: Some(self.data.boards[0].id),
                        runtime: &self.actor.runtime,
                        inventory: &self.inv,
                        manager: &self.manager,
                        proof,
                        add: self.add,
                        writable: true,
                        status: "board-ready",
                    },
                );
                if self.overlay {
                    crate::ui::widgets::Modal::new(egui::Id::unique("board_fixture_overlay")).show(ui.ctx(), |ui| {
                        ui.label("Fixture overlay");
                    });
                }
            },
        );
        result.textures_delta.clear();
        (out, result.shapes)
    }
    fn position(&mut self, label: &str) -> Pos2 {
        fn locate(shape: &egui::Shape, label: &str) -> Option<Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos + egui::vec2(3.0, 5.0)),
                egui::Shape::Vec(v) => v.iter().find_map(|s| locate(s, label)),
                _ => None,
            }
        }
        self.draw(vec![]).1.iter().find_map(|s| locate(&s.shape, label)).expect(label)
    }
    fn pointer(&mut self, pos: Pos2, pressed: bool) -> Option<Action> {
        self.draw(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ])
        .0
    }
    fn tap(&mut self, label: &str) -> Option<Action> {
        let pos = self.position(label);
        assert!(self.pointer(pos, true).is_none());
        self.pointer(pos, false)
    }
    fn click(&mut self, label: &str) -> Action {
        let pos = self.position(label);
        assert!(self.pointer(pos, true).is_none());
        self.pointer(pos, false).expect("captured action")
    }
    fn deferred(&mut self, events: Vec<egui::Event>, mut after_first: impl FnMut(&mut Self)) -> super::super::UiActions {
        self.deferred_with(events, true, |f, pass| {
            if pass == 0 {
                after_first(f);
            }
        })
    }
    fn control(&self, pos: Pos2) -> egui::WidgetRect {
        let layer = self.ctx.layer_id_at(pos).expect("painted Board layer");
        self.ctx.viewport(|viewport| {
            viewport
                .prev_pass
                .widgets
                .get_layer(layer)
                .filter(|widget| widget.sense.click && widget.interact_rect.contains(pos))
                .last()
                .copied()
                .expect("actual painted button")
        })
    }
    fn deferred_with(
        &mut self,
        events: Vec<egui::Event>,
        discard: bool,
        mut after_each: impl FnMut(&mut Self, usize),
    ) -> super::super::UiActions {
        let ctx = self.ctx.clone();
        let mut actions = super::super::UiActions::default();
        let mut passes = 0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(1500.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                passes += 1;
                let action = view::show(
                    ui.ctx(),
                    &crate::theme::Theme::default().palette(),
                    &mut self.ui,
                    &mut self.open,
                    view::Context {
                        data: Some(&self.data),
                        selected: Some(self.data.boards[0].id),
                        runtime: &self.actor.runtime,
                        inventory: &self.inv,
                        manager: &self.manager,
                        proof: self.actor.proof(None, Instant::now()),
                        add: self.add,
                        writable: true,
                        status: "board-ready",
                    },
                );
                actions.merge(super::super::UiActions {
                    board: action,
                    ..Default::default()
                });
                after_each(self, ui.ctx().current_pass_index());
                if self.overlay {
                    crate::ui::widgets::Modal::new(egui::Id::unique("board_fixture_overlay")).show(ui.ctx(), |ui| {
                        ui.label("Fixture overlay");
                    });
                }
                if discard && ui.ctx().current_pass_index() == 0 {
                    // A later, unrelated widget asks after the Board has drawn.
                    // No Board native/cache/store effect runs in this closure.
                    ui.ctx().request_discard("later widget changes its layout");
                }
            },
        );
        output.textures_delta.clear();
        assert!(!discard || passes >= 2, "exercise the actual input-free egui retry");
        actions
    }
}
#[test]
fn board_native_ui_late_discard_defers_exact_batched_play_until_final_return_once() {
    let mut f = Frames::new();
    let target = f.add.unwrap();
    let pos = f.position(aurora_i18n::tr!("common-play"));
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let mut actions = f.deferred(vec![egui::Event::PointerMoved(pos), button(true), button(false)], |f| {
        assert!(!f.manager.has_playing(), "no effect before a later discard is known");
        assert!(f.actor.pending.is_empty(), "no cache request from painting");
        assert!(f.actor.job.is_none(), "no store request from painting");
    });
    assert!(!f.manager.has_playing());
    let action = actions.board.take().expect("first pass keeps its immutable gesture");
    assert!(
        f.ui.take_activation(&f.ctx, &action),
        "original release survives the input-free retry"
    );
    assert!(!f.ui.take_activation(&f.ctx, &action), "a clone cannot consume the release twice");
    assert!(action.proof.same_owner(f.actor.proof(None, Instant::now()).unwrap()));
    assert_eq!(action.revision, f.data.revision);
    let Command::Play(ticket, captured) = action.command else {
        panic!("play")
    };
    assert!(captured == target);
    let playback::Click::Ready(ready) = f
        .actor
        .runtime
        .click(ticket, &f.data, captured, &mut f.manager, Instant::now())
        .unwrap()
    else {
        panic!("cached gesture ready")
    };
    assert!(!f.manager.has_playing());
    f.actor
        .runtime
        .start(ready, &f.data, captured, &mut f.manager, Instant::now())
        .unwrap();
    assert!(f.manager.playback_id(target.item()).is_some());
    assert!(actions.board.take().is_none(), "one post-run commit only");
    assert!(f.draw(Vec::new()).0.is_none(), "retry did not leave a replayable release");
}
#[test]
fn board_native_ui_late_discard_cannot_refresh_original_play_after_new_native_run_or_source_fence() {
    for source_loss in [false, true] {
        let mut f = Frames::new();
        let target = f.add.unwrap();
        let pos = f.position(aurora_i18n::tr!("common-play"));
        assert!(f.pointer(pos, true).is_none());
        let mut actions = f.deferred(
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
            |f| {
                assert!(!f.manager.has_playing());
                if source_loss {
                    f.actor.cancel_actions();
                }
                // A native hotkey run arrives before final paint. The held
                // Board Play must never become Stop for this later run.
                f.manager.play(target.item(), Gesture::default(), None);
            },
        );
        let native = f.manager.playback_id(target.item()).unwrap();
        let action = actions.board.take().unwrap();
        assert!(
            !f.ui.take_activation(&f.ctx, &action),
            "the final control no longer represents the original Play"
        );
        assert_eq!(action.revision, f.data.revision);
        let Command::Play(ticket, captured) = action.command else {
            panic!("captured original play")
        };
        assert!(captured == target);
        assert!(!ticket.is_stop());
        if source_loss {
            assert!(!action.proof.same_owner(f.actor.proof(None, Instant::now()).unwrap()));
        }
        assert!(matches!(
            f.actor.runtime.click(ticket, &f.data, captured, &mut f.manager, Instant::now()),
            Err(playback::Error::Stale)
        ));
        assert!(f.manager.playback_id(target.item()) == Some(native));
        assert!(f.actor.pending.is_empty() && f.actor.job.is_none());
    }
}
#[test]
fn board_native_ui_final_modal_cancels_original_release_with_or_without_retry() {
    for discard in [false, true] {
        let mut f = Frames::new();
        let pos = f.position(aurora_i18n::tr!("common-play"));
        assert!(f.pointer(pos, true).is_none());
        let mut actions = f.deferred_with(
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
            discard,
            |f, _| f.overlay = true,
        );
        let action = actions.board.take().expect("release captured before the later modal");
        assert!(matches!(action.command, Command::Play(_, _)));
        assert!(action.proof.same_owner(f.actor.proof(None, Instant::now()).unwrap()));
        assert!(f.ctx.memory(|memory| memory.top_modal_layer().is_some()), "actual final egui modal");
        assert!(!f.ui.take_activation(&f.ctx, &action));
        assert!(!f.ui.take_activation(&f.ctx, &action.clone()), "cancellation is terminal");
        assert!(!f.manager.has_playing());
        assert!(f.actor.pending.is_empty() && f.actor.job.is_none());
    }
}
#[test]
fn board_native_ui_final_hidden_or_removed_control_cancels_without_source_or_revision_change() {
    for hidden in [false, true] {
        let mut f = Frames::new();
        let pos = f.position(aurora_i18n::tr!("common-play"));
        let control = f.control(pos);
        let revision = f.data.revision;
        assert!(f.pointer(pos, true).is_none());
        let mut actions = f.deferred(
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
            |f| {
                if hidden {
                    crate::ui::widgets::hide_windows(&f.ctx, true, false);
                } else {
                    // Isolate local control disappearance from the independent
                    // catalogue/source fences that production also enforces.
                    f.data.boards[0].tiles.clear();
                }
            },
        );
        let action = actions.board.take().expect("original release from the discarded pass");
        assert!(action.proof.same_owner(f.actor.proof(None, Instant::now()).unwrap()));
        assert_eq!(action.revision, revision);
        assert_eq!(f.data.revision, revision);
        assert!(f.ctx.viewport(|viewport| viewport.prev_pass.widgets.get(control.id).is_none()));
        assert!(!f.ui.take_activation(&f.ctx, &action));
        assert!(!f.manager.has_playing());
        assert!(f.actor.pending.is_empty() && f.actor.job.is_none());
    }
}
#[test]
fn board_native_ui_final_moved_control_cancels_original_geometry() {
    let mut f = Frames::new();
    let pos = f.position(aurora_i18n::tr!("common-play"));
    let control = f.control(pos);
    assert!(f.pointer(pos, true).is_none());
    let mut actions = f.deferred(
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }],
        |f| crate::ui::widgets::move_floater(&f.ctx, "gesture_boards", egui::pos2(220.0, 200.0)),
    );
    let action = actions.board.take().expect("original geometry release");
    let final_control = f
        .ctx
        .viewport(|viewport| viewport.prev_pass.widgets.get(control.id).copied())
        .expect("moved control still exists");
    assert_ne!(control.rect, final_control.rect);
    assert!(
        final_control.interact_rect.is_positive(),
        "reject movement even when the button remains visible"
    );
    assert!(!f.ui.take_activation(&f.ctx, &action));
    assert!(!f.manager.has_playing());
    assert!(f.actor.pending.is_empty() && f.actor.job.is_none());
}
#[test]
fn board_native_ui_final_zoom_clipped_control_cancels_original_positive_clip() {
    let mut f = Frames::new();
    let pos = f.position(aurora_i18n::tr!("common-play"));
    let control = f.control(pos);
    assert!(control.interact_rect.is_positive());
    assert!(f.pointer(pos, true).is_none());
    let mut actions = f.deferred(
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }],
        |f| f.ctx.set_zoom_factor(8.0),
    );
    let action = actions.board.take().expect("release before the late zoom");
    assert_eq!(f.ctx.zoom_factor(), 8.0);
    assert!(
        f.ctx.content_rect().height() < 150.0,
        "retry uses the actual smaller logical canvas"
    );
    let final_control = f
        .ctx
        .viewport(|viewport| viewport.prev_pass.widgets.get(control.id).copied())
        .expect("clipped control is still laid out");
    assert!(final_control.rect.is_positive());
    assert!(
        !final_control.interact_rect.is_positive(),
        "the actual egui button has no positive visible clip"
    );
    assert!(!f.ui.take_activation(&f.ctx, &action));
    assert!(!f.manager.has_playing());
    assert!(f.actor.pending.is_empty() && f.actor.job.is_none());
}
#[test]
fn board_native_ui_final_key_focus_preserves_valid_retry_and_cancels_later_editor_ownership() {
    for steal_focus in [false, true] {
        let mut f = Frames::new();
        let _ = f.click(aurora_i18n::tr!("common-play"));
        assert!(f.draw(vec![key(egui::Key::Enter, true, false)]).0.is_none());
        let original_focus = f.ctx.memory(|memory| memory.focused()).unwrap();
        let editor = egui::Id::unique("board_final_fixture_editor");
        let mut actions = f.deferred_with(vec![key(egui::Key::Enter, false, false)], true, |f, _| {
            if steal_focus {
                egui::Area::new(egui::Id::unique("board_final_fixture_editor_area"))
                    .fixed_pos(egui::pos2(900.0, 50.0))
                    .show(&f.ctx, |ui| {
                        let mut text = String::new();
                        ui.add(egui::TextEdit::singleline(&mut text).id(editor)).request_focus();
                    });
            }
        });
        let action = actions.board.take().expect("original keyboard release");
        assert!(matches!(action.command, Command::Play(_, _)));
        assert_eq!(
            f.ctx.memory(|memory| memory.focused()),
            Some(if steal_focus { editor } else { original_focus })
        );
        assert_eq!(f.ui.take_activation(&f.ctx, &action), !steal_focus);
        assert!(!f.ui.take_activation(&f.ctx, &action), "one immutable keyboard release");
        assert!(!f.manager.has_playing());
        assert!(f.actor.pending.is_empty() && f.actor.job.is_none());
    }
}
#[test]
fn board_native_ui_focused_key_rejects_stable_infinite_transform_or_rect_and_keeps_valid_retry_proof() {
    use egui::emath::TSTransform;
    let mut f = Frames::new();
    let original = f.click(aurora_i18n::tr!("common-play"));
    let rect = egui::Rect::from_min_max(egui::pos2(10.0, 10.0), egui::pos2(90.0, 40.0));
    let clip = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(120.0, 60.0));
    let infinite = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(f32::INFINITY, f32::INFINITY));
    for (transform, rect, clip, valid) in [
        (None, rect, clip, true),
        (Some(TSTransform::new(egui::vec2(3.0, 5.0), 1.5)), rect, clip, true),
        (Some(TSTransform::new(egui::vec2(0.0, f32::INFINITY), 1.0)), rect, clip, false),
        (Some(TSTransform::new(egui::vec2(f32::INFINITY, 0.0), 1.0)), rect, clip, false),
        (Some(TSTransform::from_scaling(f32::INFINITY)), rect, clip, false),
        (Some(TSTransform::from_scaling(0.0)), rect, clip, false),
        (Some(TSTransform::from_scaling(f32::from_bits(1))), rect, clip, false),
        (Some(TSTransform::from_translation(egui::vec2(10000.0, 10000.0))), rect, clip, false),
        (None, infinite, infinite, false),
    ] {
        let (captured, accepted, replayed, focused_widget) = view::finite_geometry_tests::key_run(original.clone(), rect, clip, transform);
        assert!(
            focused_widget,
            "invalid geometry cannot be rejected merely because the keyed widget disappeared or lost focus"
        );
        assert_eq!(captured.is_some(), valid);
        assert_eq!(accepted, valid);
        assert!(!replayed, "the original release can only be consumed once");
        if let Some(captured) = captured {
            assert!(captured.proof.same_owner(original.proof));
            assert_eq!(
                captured.proof.at, original.proof.at,
                "a retry cannot renew the original proof timestamp"
            );
            let (Command::Play(ticket, target), Command::Play(original_ticket, original_target)) = (captured.command, &original.command)
            else {
                panic!("original Play")
            };
            assert!(ticket == *original_ticket && target == *original_target);
        }
    }
    assert!(!f.manager.has_playing());
    assert!(f.actor.pending.is_empty() && f.actor.job.is_none());
}
#[test]
fn board_native_ui_real_add_play_remove_stop_capture_exact_payload_without_automatic_effects() {
    let mut f = Frames::new();
    let original = f.add.unwrap();
    let revision = f.data.revision;
    let pos = f.position(aurora_i18n::tr!("board-add"));
    assert!(f.pointer(pos, true).is_none());
    f.add = None;
    // Release over a now disabled Add rejects; selecting another valid item
    // below proves that changing selection never substitutes its target.
    assert!(f.pointer(pos, false).is_none());
    f.add = Some(original);
    let add = f.click(aurora_i18n::tr!("board-add"));
    assert_eq!(add.revision, revision);
    assert!(matches!(add.command,Command::Add(id,item)if id==f.data.boards[0].id&&item==original));
    let play = f.click(aurora_i18n::tr!("common-play"));
    let Command::Play(ticket, target) = play.command else {
        panic!("play")
    };
    assert!(target == original);
    assert_eq!(ticket.item(), original.item());
    assert!(!f.manager.has_playing());
    assert!(f.actor.pending.is_empty());
    let remove = f.click(aurora_i18n::tr!("common-remove"));
    assert!(
        matches!(remove.command,Command::Edit(Change::Remove{board,tile})if board==f.data.boards[0].id&&tile==f.data.boards[0].tiles[0].id)
    );
    let stop = f.click(aurora_i18n::tr!("board-stop"));
    assert!(matches!(stop.command,Command::Stop(board,_)if board==f.data.boards[0].id));
    assert!(!f.manager.has_playing());
    assert_eq!(f.data.revision, revision);
}
#[test]
fn board_native_ui_held_play_source_rebind_close_reopen_and_inventory_retarget_never_substitute() {
    let mut f = Frames::new();
    let original = f.add.unwrap();
    let pos = f.position(aurora_i18n::tr!("board-add"));
    assert!(f.pointer(pos, true).is_none());
    let mut other = f.inv.items[&original.item()].clone();
    other.id = Uuid::from_u128(990002);
    other.asset_id = Uuid::from_u128(990003);
    f.inv.add_items(vec![other.clone()]);
    f.add = Some(actions::resolve_playback_item(&f.inv, other.id, PlaybackKind::Gesture).unwrap());
    let action = f.pointer(pos, false).unwrap();
    assert!(matches!(action.command,Command::Add(_,target)if target==original));
    let pos = f.position(aurora_i18n::tr!("common-play"));
    assert!(f.pointer(pos, true).is_none());
    f.actor.cancel_actions();
    assert!(f.pointer(pos, false).is_none());
    assert!(f.pointer(pos, true).is_none());
    f.actor.window += 1;
    f.open = false;
    f.draw(vec![]);
    f.open = true;
    for _ in 0..3 {
        f.draw(vec![]);
    }
    assert!(f.pointer(pos, false).is_none());
    let pos = f.position(aurora_i18n::tr!("common-play"));
    assert!(f.pointer(pos, true).is_none());
    f.actor.facts.as_mut().unwrap().login = Uuid::from_u128(99);
    assert!(f.pointer(pos, false).is_none());
    assert!(!f.manager.has_playing());
}

#[test]
fn board_native_ui_real_new_rename_delete_confirm_carries_exact_original_catalogue_and_ids() {
    let mut f = Frames::new();
    let first = f.data.boards[0].id;
    assert!(f.tap(aurora_i18n::tr!("board-new")).is_none());
    for _ in 0..3 {
        f.draw(vec![]);
    }
    assert!(f.tap(aurora_i18n::tr!("board-name-placeholder")).is_none());
    f.draw(vec![egui::Event::Text("Second board".into())]);
    let saved = f.click(aurora_i18n::tr!("common-save"));
    assert_eq!(saved.revision, f.data.revision);
    let Command::Edit(Change::Create { id, name }) = saved.command else {
        panic!("create")
    };
    assert_ne!(id, first);
    assert_eq!(name, "Second board");
    f.data = f.data.apply(Change::Create { id, name }).unwrap();
    assert!(f.tap(aurora_i18n::tr!("common-rename")).is_none());
    for _ in 0..3 {
        f.draw(vec![]);
    }
    let renamed = f.click(aurora_i18n::tr!("common-save"));
    assert!(matches!(renamed.command,Command::Edit(Change::Rename{board,name})if board==first&&name=="First board"));
    assert!(f.tap(aurora_i18n::tr!("common-delete")).is_none());
    for _ in 0..3 {
        f.draw(vec![]);
    }
    let deleted = f.click(aurora_i18n::tr!("common-save"));
    assert!(matches!(deleted.command,Command::Edit(Change::Delete{board})if board==first));
    assert_eq!(f.data.boards.len(), 2);
    assert!(!f.manager.has_playing());
}

#[test]
fn board_app_old_stop_cannot_drop_new_cache_context_before_exact_runtime_validation() {
    let mut f = Frames::new();
    let board = f.data.boards[0].id;
    let target = f.add.unwrap();
    let now = Instant::now();
    let old = f.actor.runtime.capture_board(&f.data, board, now).unwrap();
    f.actor.stop_board(board, old, &f.data, &mut f.manager, now).unwrap();
    f.manager.entries.remove(&target.item());
    let ticket = f
        .actor
        .runtime
        .capture(&f.data, board, f.data.boards[0].tiles[0].id, target, &f.manager, now)
        .unwrap();
    assert!(matches!(f.actor.runtime.click(ticket,&f.data,target,&mut f.manager,now),Ok(playback::Click::NeedsCache(t))if t==ticket));
    let proof = f.actor.proof(None, now).unwrap();
    f.actor.pending.insert(target.item(), (ticket, proof, target));
    assert!(matches!(
        f.actor.stop_board(board, old, &f.data, &mut f.manager, now),
        Err(playback::Error::Stale)
    ));
    assert!(
        f.actor
            .pending
            .get(&target.item())
            .is_some_and(|(t, _, item)| *t == ticket && *item == target)
    );
    assert!(f.actor.runtime.pending(target.item()) == Some(ticket));
    let current = f.actor.runtime.capture_board(&f.data, board, now).unwrap();
    assert!(matches!(
        f.actor.stop_board(BoardId::new(), current, &f.data, &mut f.manager, now),
        Err(playback::Error::Stale)
    ));
    assert!(f.actor.pending.contains_key(&target.item()));
    f.actor.stop_board(board, current, &f.data, &mut f.manager, now).unwrap();
    assert!(f.actor.pending.is_empty());
    assert!(f.actor.runtime.pending(target.item()).is_none());
}

#[test]
fn board_native_actual_availability_gate_preserves_temporary_borrow_and_denies_absence_or_lost_device() {
    use std::cell::Cell;
    let mut slot = Some(Cell::new(true));
    assert!(available(slot.as_ref(), None, Cell::get));
    let actual = slot.take().unwrap();
    assert!(available(slot.as_ref(), Some(&actual), Cell::get));
    assert!(!available::<Cell<bool>>(slot.as_ref(), None, Cell::get));
    actual.set(false);
    assert!(!available(slot.as_ref(), Some(&actual), Cell::get));
    actual.set(true);
    slot = Some(actual);
    assert!(available(slot.as_ref(), None, Cell::get));
    // This is a physical availability predicate only: it never mints a managed
    // NativeActionScope or bypasses the separate App source/action permission.
}

fn key(key: egui::Key, pressed: bool, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers: Default::default(),
    }
}
#[test]
fn board_native_ui_queued_pointer_preserves_prior_painted_play_and_old_asset_target() {
    let mut f = Frames::new();
    let target = f.add.unwrap();
    let pos = f.position(aurora_i18n::tr!("common-play"));
    f.manager.play(target.item(), Gesture::default(), None);
    let native = f.manager.playback_id(target.item()).unwrap();
    assert!(f.pointer(pos, true).is_none());
    if let Some(action) = f.pointer(pos, false) {
        let Command::Play(ticket, captured) = action.command else {
            panic!("prior play")
        };
        assert!(captured == target);
        assert!(matches!(
            f.actor.runtime.click(ticket, &f.data, captured, &mut f.manager, Instant::now()),
            Err(playback::Error::Stale)
        ));
    }
    assert!(f.manager.playback_id(target.item()) == Some(native));
    // A deliberate fresh click after paint may stop exactly the visible run.
    let action = f.click(aurora_i18n::tr!("common-stop"));
    let Command::Play(ticket, captured) = action.command else {
        panic!("stop")
    };
    assert!(matches!(
        f.actor.runtime.click(ticket, &f.data, captured, &mut f.manager, Instant::now()),
        Ok(playback::Click::Stopped(_))
    ));
    for _ in 0..3 {
        f.draw(vec![]);
    }
    let pos = f.position(aurora_i18n::tr!("common-play"));
    f.inv.items.get_mut(&target.item()).unwrap().asset_id = Uuid::from_u128(990004);
    assert!(f.pointer(pos, true).is_none());
    let action = f.pointer(pos, false).expect("old painted target");
    let Command::Play(_, captured) = action.command else {
        panic!("prior target")
    };
    assert!(captured == target);
    assert!(!current_target(&f.inv, captured));
    assert!(!f.manager.has_playing());
}
#[test]
fn board_native_ui_exact_nonrepeat_key_keeps_original_intent_and_ignores_other_release() {
    let mut f = Frames::new();
    let target = f.add.unwrap();
    let _ = f.click(aurora_i18n::tr!("common-play"));
    assert!(f.draw(vec![key(egui::Key::Enter, true, false)]).0.is_none());
    f.manager.play(target.item(), Gesture::default(), None);
    let native = f.manager.playback_id(target.item()).unwrap();
    assert!(f.draw(vec![key(egui::Key::Enter, true, true)]).0.is_none());
    assert!(f.draw(vec![key(egui::Key::Space, false, false)]).0.is_none());
    let action = f.draw(vec![key(egui::Key::Enter, false, false)]).0.expect("original exact key");
    let Command::Play(ticket, captured) = action.command else {
        panic!("original play")
    };
    assert!(matches!(
        f.actor.runtime.click(ticket, &f.data, captured, &mut f.manager, Instant::now()),
        Err(playback::Error::Stale)
    ));
    assert!(f.manager.playback_id(target.item()) == Some(native));
}
#[test]
fn board_native_ui_ime_focus_and_removed_overlay_cancel_held_or_queued_activation() {
    let mut f = Frames::new();
    let _ = f.click(aurora_i18n::tr!("common-play"));
    assert!(f.draw(vec![key(egui::Key::Enter, true, false)]).0.is_none());
    assert!(
        f.draw(vec![egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "x".into(),
            active_range_chars: None
        })])
        .0
        .is_none()
    );
    assert!(
        f.draw(vec![
            egui::Event::Ime(egui::ImeEvent::Commit("x".into())),
            key(egui::Key::Enter, false, false)
        ])
        .0
        .is_none()
    );
    let _ = f.click(aurora_i18n::tr!("common-play"));
    assert!(f.draw(vec![key(egui::Key::Space, true, false)]).0.is_none());
    assert!(f.draw(vec![egui::Event::WindowFocused(false)]).0.is_none());
    assert!(
        f.draw(vec![egui::Event::WindowFocused(true), key(egui::Key::Space, false, false)])
            .0
            .is_none()
    );
    let pos = f.position(aurora_i18n::tr!("common-play"));
    f.overlay = true;
    for _ in 0..3 {
        f.draw(vec![]);
    }
    f.overlay = false;
    assert!(f.pointer(pos, true).is_none());
    assert!(f.pointer(pos, false).is_none());
    assert!(!f.manager.has_playing());
}
#[test]
fn board_native_ui_modal_and_newly_rebound_controls_require_fresh_painted_intent() {
    let mut f = Frames::new();
    let pos = f.position(aurora_i18n::tr!("common-play"));
    f.actor.cancel_actions();
    assert!(f.pointer(pos, true).is_none());
    assert!(f.pointer(pos, false).is_none());
    assert!(f.tap(aurora_i18n::tr!("board-new")).is_none());
    for _ in 0..3 {
        f.draw(vec![]);
    }
    assert!(f.draw(vec![key(egui::Key::Space, true, false)]).0.is_none());
    assert!(f.draw(vec![key(egui::Key::Space, false, false)]).0.is_none());
    assert!(f.tap(aurora_i18n::tr!("common-cancel")).is_none());
    for _ in 0..3 {
        f.draw(vec![]);
    }
    let revision = f.data.revision;
    let pos = f.position(aurora_i18n::tr!("common-play"));
    f.data = f
        .data
        .apply(Change::Rename {
            board: f.data.boards[0].id,
            name: "Renamed before input pass".into(),
        })
        .unwrap();
    assert!(f.pointer(pos, true).is_none());
    if let Some(action) = f.pointer(pos, false) {
        assert_eq!(action.revision, revision);
        assert_ne!(action.revision, f.data.revision);
    }
    assert!(!f.manager.has_playing());
}
#[test]
fn board_native_ui_batched_pointer_uses_actual_press_and_release_positions_without_b_retarget() {
    let mut f = Frames::new();
    let a = f.position(aurora_i18n::tr!("board-add"));
    let b = f.position(aurora_i18n::tr!("common-play"));
    let pointer = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    assert!(
        f.draw(vec![pointer(a, true), egui::Event::PointerMoved(b), pointer(b, false)])
            .0
            .is_none()
    );
    // A complete quick click at A may commit A or be conservatively rejected;
    // the trailing move must never turn it into B's Play operation.
    if let Some(action) = f.draw(vec![pointer(a, true), pointer(a, false), egui::Event::PointerMoved(b)]).0 {
        assert!(matches!(action.command,Command::Add(_,target)if Some(target)==f.add));
    }
    assert!(
        f.draw(vec![pointer(b, false), pointer(a, true), egui::Event::PointerMoved(b)])
            .0
            .is_none()
    );
    assert!(f.draw(vec![pointer(b, false)]).0.is_none());
    assert!(!f.manager.has_playing());
}

#[test]
fn board_native_demo_real_login_fixture_has_only_explicit_standalone_memory_identity() {
    let login = crate::demo::events()
        .into_iter()
        .find_map(|event| {
            if let aurora_net::NetEvent::LoggedIn(login) = event {
                Some(login)
            } else {
                None
            }
        })
        .unwrap();
    assert!(login.session_id.is_nil());
    let local = login_identity(&login, login.agent_id, true, false, 1).unwrap();
    assert!(!local.is_nil());
    assert_ne!(Some(local), login_identity(&login, login.agent_id, true, false, 2));
    assert!(login_identity(&login, login.agent_id, false, false, 1).is_none());
    assert!(login_identity(&login, login.agent_id, true, true, 1).is_none());
    assert!(login_identity(&login, login.agent_id, true, false, 0).is_none());
    assert!(login_identity(&login, Uuid::from_u128(99), true, false, 1).is_none());
    let mut state = actor();
    let data = Catalogue::empty(login.agent_id, facts().grid)
        .unwrap()
        .apply(Change::Create {
            id: BoardId::new(),
            name: "Demo edits".into(),
        })
        .unwrap();
    state.snapshot = Some(store::Snapshot::memory(data.clone()));
    state.loaded = true;
    state.close_snapshot(true);
    assert!(state.loaded);
    assert_eq!(*state.snapshot.as_ref().unwrap().data, data);
    state.close_snapshot(false);
    assert!(!state.loaded);
    assert!(state.snapshot.is_none());
    let mut real = (*login).clone();
    real.session_id = Uuid::from_u128(77);
    assert_eq!(login_identity(&real, real.agent_id, false, true, 1), Some(real.session_id));
}
#[test]
fn board_app_current_exact_tile_stop_remains_allowed_under_start_restriction() {
    let mut f = Frames::new();
    let target = f.add.unwrap();
    let board = f.data.boards[0].id;
    let tile = f.data.boards[0].tiles[0].id;
    let now = Instant::now();
    let start = f.actor.runtime.capture(&f.data, board, tile, target, &f.manager, now).unwrap();
    assert!(start_blocked(start, target, true));
    assert!(!start_blocked(start, target, false));
    f.manager.play(target.item(), Gesture::default(), None);
    let native = f.manager.playback_id(target.item()).unwrap();
    let stop = f.actor.runtime.capture(&f.data, board, tile, target, &f.manager, now).unwrap();
    assert!(stop.is_stop());
    assert!(!start_blocked(stop, target, true));
    assert!(matches!(
        f.actor.runtime.click(stop, &f.data, target, &mut f.manager, now),
        Ok(playback::Click::Stopped(_))
    ));
    assert!(f.manager.playback_id(target.item()).is_none());
    f.manager.play(target.item(), Gesture::default(), None);
    assert!(f.manager.playback_id(target.item()) != Some(native));
    assert!(matches!(
        f.actor.runtime.click(stop, &f.data, target, &mut f.manager, now),
        Err(playback::Error::Stale)
    ));
    assert!(f.manager.has_playing());
    let sound = actions::resolve_playback_item(&f.inv, Uuid::from_u128(8105), PlaybackKind::Sound).unwrap();
    let sound_tile = TileId::new();
    f.data = f
        .data
        .apply(Change::Add {
            board,
            tile: Tile {
                id: sound_tile,
                item: Reference {
                    reference: sound.reference(),
                    kind: Kind::Sound,
                },
            },
        })
        .unwrap();
    f.manager.enabled = false;
    let sound_ticket = f.actor.runtime.capture(&f.data, board, sound_tile, sound, &f.manager, now).unwrap();
    assert!(!start_blocked(sound_ticket, sound, true));
    assert!(f.actor.runtime.validate_sound(sound_ticket, &f.data, sound, now).is_ok());
}

#[test]
fn board_native_ui_close_reopen_retires_composition_even_when_terminal_is_consumed_hidden() {
    let mut f = Frames::new();
    assert!(
        f.draw(vec![egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "x".into(),
            active_range_chars: None
        })])
        .0
        .is_none()
    );
    f.open = false;
    assert!(f.draw(vec![]).0.is_none());
    // The ordinary App consumes terminal input while this panel is hidden.
    let mut hidden = f.ctx.run_ui(
        egui::RawInput {
            events: vec![
                egui::Event::Ime(egui::ImeEvent::Commit("x".into())),
                key(egui::Key::Enter, false, false),
            ],
            ..Default::default()
        },
        |_| {},
    );
    hidden.textures_delta.clear();
    f.actor.window += 1;
    f.open = true;
    for _ in 0..4 {
        assert!(f.draw(vec![]).0.is_none());
    }
    assert!(matches!(f.click(aurora_i18n::tr!("common-play")).command, Command::Play(_, _)));
    assert!(!f.manager.has_playing());
}
#[test]
fn board_native_ui_removed_held_tile_release_does_not_stick_or_retarget_next_control() {
    let mut f = Frames::new();
    let pos = f.position(aurora_i18n::tr!("common-play"));
    assert!(f.pointer(pos, true).is_none());
    f.data = f
        .data
        .apply(Change::Remove {
            board: f.data.boards[0].id,
            tile: f.data.boards[0].tiles[0].id,
        })
        .unwrap();
    assert!(f.pointer(pos, false).is_none());
    assert!(f.tap(aurora_i18n::tr!("board-new")).is_none());
    for _ in 0..3 {
        f.draw(vec![]);
    }
    assert!(f.tap(aurora_i18n::tr!("common-cancel")).is_none());
    assert!(matches!(f.click(aurora_i18n::tr!("board-add")).command, Command::Add(_, _)));
    assert!(!f.manager.has_playing());
}

#[test]
fn board_native_ui_ime_terminal_retires_only_command_focus_before_later_enter_confirmation() {
    let mut f = Frames::new();
    let _ = f.click(aurora_i18n::tr!("common-play"));
    assert!(
        f.draw(vec![egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "x".into(),
            active_range_chars: None
        })])
        .0
        .is_none()
    );
    assert!(f.draw(vec![egui::Event::Ime(egui::ImeEvent::Commit("x".into()))]).0.is_none());
    assert!(f.draw(vec![key(egui::Key::Enter, true, false)]).0.is_none());
    assert!(f.draw(vec![key(egui::Key::Enter, false, false)]).0.is_none());
    let _ = f.click(aurora_i18n::tr!("common-play"));
    assert!(f.draw(vec![key(egui::Key::Enter, true, false)]).0.is_none());
    assert!(matches!(
        f.draw(vec![key(egui::Key::Enter, false, false)]).0.unwrap().command,
        Command::Play(_, _)
    ));
    assert!(!f.manager.has_playing());
    assert!(f.tap(aurora_i18n::tr!("board-new")).is_none());
    for _ in 0..3 {
        f.draw(vec![]);
    }
    assert!(f.tap(aurora_i18n::tr!("board-name-placeholder")).is_none());
    let text_focus = f.ctx.memory(|m| m.focused()).unwrap();
    assert!(
        f.draw(vec![egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "x".into(),
            active_range_chars: None
        })])
        .0
        .is_none()
    );
    assert!(f.draw(vec![egui::Event::Ime(egui::ImeEvent::Commit("x".into()))]).0.is_none());
    assert_eq!(f.ctx.memory(|m| m.focused()), Some(text_focus));
}

#[test]
fn board_app_edit_observing_unknown_private_or_changed_store_retires_cache_without_repair() {
    let data = Catalogue::empty(facts().agent, facts().grid)
        .unwrap()
        .apply(Change::Create {
            id: BoardId::new(),
            name: "Prior board".into(),
        })
        .unwrap();
    let bytes = data.encode().unwrap();
    let saved = bytes.clone();
    for error in [
        store::Error::Model(model::Error::Unsupported),
        store::Error::Model(model::Error::Identity),
        store::Error::Model(model::Error::Invalid),
        store::Error::Private,
        store::Error::Conflict,
    ] {
        let mut actor = actor();
        actor.snapshot = Some(store::Snapshot::memory(data.clone()));
        actor.loaded = true;
        actor.store_failure(error, false);
        assert!(actor.snapshot.is_none());
        assert!(actor.loaded);
        assert!(actor.pending.is_empty());
        assert_eq!(bytes, saved);
    }
    let mut actor = actor();
    actor.snapshot = Some(store::Snapshot::memory(data.clone()));
    actor.store_failure(store::Error::Model(model::Error::Name), false);
    assert_eq!(*actor.snapshot.as_ref().unwrap().data, data);
    assert_eq!(actor.status, "board-name-invalid");
    actor.store_failure(store::Error::Model(model::Error::Limit), true);
    assert!(actor.snapshot.is_none());
}

#[test]
fn board_app_unreadable_store_retires_already_issued_ready_before_any_later_refresh() {
    let mut f = Frames::new();
    let target = f.add.unwrap();
    let now = Instant::now();
    f.actor.snapshot = Some(store::Snapshot::memory(f.data.clone()));
    let ticket = f
        .actor
        .runtime
        .capture(&f.data, f.data.boards[0].id, f.data.boards[0].tiles[0].id, target, &f.manager, now)
        .unwrap();
    let playback::Click::Ready(ready) = f.actor.runtime.click(ticket, &f.data, target, &mut f.manager, now).unwrap() else {
        panic!("issued ready")
    };
    let original = f.actor.proof(None, now).unwrap();
    f.actor.store_failure(store::Error::Conflict, false);
    assert!(f.actor.snapshot.is_none());
    // Even a later valid model with identical IDs/revision cannot revive the
    // prior explicit gesture; source/window action epochs were retired first.
    f.actor.snapshot = Some(store::Snapshot::memory(f.data.clone()));
    assert!(!original.same_owner(f.actor.proof(None, now).unwrap()));
    assert!(matches!(
        f.actor.runtime.start(ready, &f.data, target, &mut f.manager, now),
        Err(playback::Error::Stale)
    ));
    assert!(!f.manager.has_playing());
    f.manager.entries.remove(&target.item());
    let fresh = f
        .actor
        .runtime
        .capture(&f.data, f.data.boards[0].id, f.data.boards[0].tiles[0].id, target, &f.manager, now)
        .unwrap();
    assert!(matches!(
        f.actor.runtime.click(fresh, &f.data, target, &mut f.manager, now),
        Ok(playback::Click::NeedsCache(_))
    ));
    f.actor
        .pending
        .insert(target.item(), (fresh, f.actor.proof(None, now).unwrap(), target));
    f.actor.store_failure(store::Error::Private, false);
    assert!(f.actor.pending.is_empty());
    assert!(f.actor.runtime.pending(target.item()).is_none());
}

#[test]
fn board_app_edit_original_empty_invalid_name_and_duplicate_bytes_never_become_user_edit_errors() {
    let f = facts();
    let data = Catalogue::empty(f.agent, f.grid)
        .unwrap()
        .apply(Change::Create {
            id: BoardId::new(),
            name: "Prior valid board".into(),
        })
        .unwrap();
    let mut name = serde_json::to_value(&data).unwrap();
    name["boards"][0]["name"] = "".into();
    let mut duplicate = serde_json::to_value(&data).unwrap();
    let same = duplicate["boards"][0].clone();
    duplicate["boards"].as_array_mut().unwrap().push(same);
    let cases = [
        (Vec::new(), model::Error::Limit),
        (serde_json::to_vec(&name).unwrap(), model::Error::Name),
        (serde_json::to_vec(&duplicate).unwrap(), model::Error::Duplicate),
    ];
    for (bytes, expected) in cases {
        let unchanged = bytes.clone();
        let error = store::decode(Some(&bytes), f).unwrap_err();
        assert_eq!(error, store::Error::Original(expected));
        let mut actor = actor();
        actor.snapshot = Some(store::Snapshot::memory(data.clone()));
        actor.loaded = true;
        actor.store_failure(error, false);
        assert!(actor.snapshot.is_none());
        assert!(actor.loaded);
        assert_eq!(bytes, unchanged);
    }
    let mut actor = actor();
    actor.snapshot = Some(store::Snapshot::memory(data.clone()));
    actor.store_failure(store::Error::Model(model::Error::Name), false);
    assert_eq!(*actor.snapshot.as_ref().unwrap().data, data);
}
