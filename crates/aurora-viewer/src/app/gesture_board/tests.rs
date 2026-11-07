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
    let mut inv = Inventory::default();
    crate::world::inventory::demo::seed(&mut inv, facts().agent);
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
}
impl Frames {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, facts().agent);
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
