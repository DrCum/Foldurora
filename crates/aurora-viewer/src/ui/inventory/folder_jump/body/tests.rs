use super::*;
use aurora_net::inventory::InvFolder;
use egui::{Event, Modifiers, PointerButton, RawInput, vec2};
use uuid::Uuid;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}
fn folder(value: u128, parent: u128, name: &str) -> InvFolder {
    InvFolder {
        id: id(value),
        parent: id(parent),
        name: name.into(),
        type_default: -1,
        ..Default::default()
    }
}
fn key(key: Key, down: bool) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed: down,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

struct Frames {
    ctx: egui::Context,
    body: Body,
    inventory: Inventory,
    source: Source,
    policy: Policy,
    frame: u64,
    size: egui::Vec2,
    discard_next: bool,
}

impl Frames {
    fn new() -> Self {
        let mut inventory = Inventory {
            owner: id(900),
            root: id(1),
            ..Default::default()
        };
        inventory.server_updated(
            vec![
                folder(1, 0, "Inventory"),
                folder(2, 1, "Alpha"),
                folder(3, 1, "Beta"),
                folder(4, 1, "Gamma"),
            ],
            Vec::new(),
        );
        let source = Source {
            agent: id(900),
            owner: id(900),
            session: id(901),
            root: id(1),
            profile: 1,
            window: Uuid::nil(),
        };
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut frames = Self {
            ctx,
            body: Body::default(),
            inventory,
            source,
            policy: Policy {
                allowed: true,
                revision: 1,
            },
            frame: 0,
            size: vec2(520.0, 440.0),
            discard_next: false,
        };
        frames.settle();
        frames
    }

    fn show(&mut self, events: Vec<Event>) -> Option<Action> {
        self.frame += 1;
        let mut result = None;
        let modifiers = events
            .iter()
            .rev()
            .find_map(|event| match event {
                Event::Key { modifiers, .. } | Event::PointerButton { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or(Modifiers::NONE);
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            focused: true,
            time: Some(self.frame as f64 / 60.0),
            events: std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect(),
            ..Default::default()
        };
        let palette = crate::theme::Theme::default().palette();
        let discarded = self.discard_next;
        let output = self.ctx.run_ui(raw, |ui| {
            if self.discard_next {
                self.discard_next = false;
                ui.ctx().request_discard("folder jump fixture layout changed before release paint");
            }
            if let Some(action) = self.body.show(ui, &palette, &self.inventory, Some(self.source), self.policy) {
                result = Some(action);
            }
        });
        if discarded {
            assert!(output.num_completed_passes > 1);
        }
        result
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            assert!(self.show(Vec::new()).is_none());
        }
    }

    fn row(&self, folder: Uuid) -> Control {
        *self
            .body
            .controls
            .iter()
            .find(|control| matches!(control.pointer, Some(Intent::Select(ticket)) if ticket.folder() == folder))
            .expect("actual committed row control")
    }
    fn open(&self) -> Control {
        *self
            .body
            .controls
            .iter()
            .find(|control| matches!(control.pointer, Some(Intent::Open(_))))
            .expect("actual committed Open control")
    }
    fn query(&self) -> Control {
        *self
            .body
            .controls
            .iter()
            .find(|control| Some(control.widget) == self.body.query_widget)
            .expect("actual committed query field")
    }
    fn pointer(&mut self, at: Pos2, down: bool) -> Option<Action> {
        self.show(vec![
            Event::PointerMoved(at),
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: down,
                modifiers: Modifiers::NONE,
            },
        ])
    }
    fn click(&mut self, control: Control) -> Option<Action> {
        let at = control.rect.center();
        assert!(self.pointer(at, true).is_none());
        self.pointer(at, false)
    }
    fn replace_query(&mut self, value: &str) {
        assert!(self.click(self.query()).is_none());
        let command = Modifiers {
            command: true,
            ctrl: !cfg!(target_os = "macos"),
            mac_cmd: cfg!(target_os = "macos"),
            ..Modifiers::NONE
        };
        assert!(
            self.show(vec![
                Event::Key {
                    key: Key::A,
                    physical_key: Some(Key::A),
                    pressed: true,
                    repeat: false,
                    modifiers: command
                },
                Event::Key {
                    key: Key::A,
                    physical_key: Some(Key::A),
                    pressed: false,
                    repeat: false,
                    modifiers: command
                },
                Event::Text(value.into()),
            ])
            .is_none()
        );
        self.settle();
    }
    fn opened(action: Option<Action>) -> Uuid {
        match action {
            Some(Action::Open(ticket)) => ticket.folder(),
            _ => panic!("expected exact captured Open"),
        }
    }
}

#[test]
fn folder_jump_body_real_pointer_selects_exact_folder_then_opens_existing_rooted_window_without_changing_search() {
    let mut frames = Frames::new();
    let mut inventory_ui = crate::ui::inventory::InventoryUi {
        search: "existing ordinary search".into(),
        ..Default::default()
    };
    assert!(frames.click(frames.row(id(3))).is_none());
    frames.settle();
    assert_eq!(frames.body.model.selected(), Some(id(3)));
    let folder = Frames::opened(frames.click(frames.open()));
    inventory_ui.open_folder_window(folder);
    assert_eq!(inventory_ui.search, "existing ordinary search");
    assert_eq!(inventory_ui.windows.len(), 1);
    assert_eq!(inventory_ui.windows[0].root, id(3));
    assert!(!inventory_ui.windows[0].additional);
    assert!(inventory_ui.windows[0].open);
    assert!(frames.inventory.queue.is_empty());
}

#[test]
fn folder_jump_body_queued_press_uses_old_painted_row_and_held_open_rejects_policy_change() {
    let mut frames = Frames::new();
    let old_alpha = frames.row(id(2)).rect.center();
    frames.inventory.server_updated(vec![folder(2, 1, "Zulu")], Vec::new());
    assert!(
        frames.pointer(old_alpha, true).is_none(),
        "first render of queued press cannot borrow replacement Beta intent"
    );
    assert!(frames.pointer(old_alpha, false).is_none());
    frames.settle();
    assert_eq!(
        frames.body.model.selected(),
        Some(id(2)),
        "old Alpha remains selected; replacement first row Beta was never chosen"
    );
    assert!(frames.click(frames.row(id(3))).is_none());
    frames.settle();
    assert_eq!(frames.body.model.selected(), Some(id(3)));
    let open = frames.open().rect.center();
    assert!(frames.pointer(open, true).is_none());
    frames.policy.revision += 1;
    assert!(
        frames.pointer(open, false).is_none(),
        "same UUID with changed current policy cannot finish an old press"
    );
    frames.settle();
    assert_eq!(Frames::opened(frames.click(frames.open())), id(3));
}

#[test]
fn folder_jump_body_captured_keyboard_navigation_and_open_do_not_retarget_after_reorder_or_query_edit() {
    let mut frames = Frames::new();
    assert!(frames.show(vec![key(Key::Enter, true)]).is_none());
    assert!(frames.click(frames.row(id(3))).is_none());
    assert!(
        frames.show(vec![key(Key::Enter, false)]).is_none(),
        "a real pointer selection cancels the held old Open"
    );
    frames.settle();
    assert_eq!(frames.body.model.selected(), Some(id(3)));
    assert!(frames.click(frames.row(id(2))).is_none());
    frames.settle();
    assert!(frames.click(frames.query()).is_none());
    frames.settle();
    assert!(frames.show(vec![key(Key::ArrowDown, true)]).is_none());
    frames.inventory.server_updated(vec![folder(3, 1, "Zulu")], Vec::new());
    assert!(frames.show(vec![key(Key::ArrowDown, false)]).is_none());
    frames.settle();
    assert_eq!(
        frames.body.model.selected(),
        Some(id(2)),
        "held Down captured Beta, not the new index-one Gamma"
    );
    assert!(frames.show(vec![key(Key::ArrowDown, true)]).is_none());
    assert!(frames.show(vec![key(Key::ArrowDown, false)]).is_none());
    frames.settle();
    assert_eq!(frames.body.model.selected(), Some(id(4)));
    assert_eq!(frames.ctx.memory(|memory| memory.focused()), Some(frames.row(id(4)).widget));
    assert!(frames.click(frames.query()).is_none());
    frames.settle();
    assert!(frames.show(vec![key(Key::Enter, true)]).is_none());
    assert!(frames.show(vec![Event::Text("Zulu".into())]).is_none());
    assert!(
        frames.show(vec![key(Key::Enter, false)]).is_none(),
        "query edit invalidates the held selected-folder ticket"
    );
    frames.settle();
    assert_eq!(frames.body.model.selected(), Some(id(3)));
    assert!(frames.show(vec![key(Key::Enter, true)]).is_none());
    assert_eq!(Frames::opened(frames.show(vec![key(Key::Enter, false)])), id(3));
}

#[test]
fn folder_jump_body_batched_press_release_obeys_original_position_and_later_press_survives_old_release() {
    let mut frames = Frames::new();
    let alpha = frames.row(id(2)).rect.center();
    let beta = frames.row(id(3)).rect.center();
    let pointer = |pos, down| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed: down,
        modifiers: Modifiers::NONE,
    };
    assert!(
        frames
            .show(vec![pointer(alpha, true), Event::PointerMoved(beta), pointer(beta, false)])
            .is_none()
    );
    assert_eq!(
        frames.body.model.selected(),
        Some(id(2)),
        "final pointer position cannot replace press-time target"
    );
    assert!(frames.show(vec![pointer(alpha, false), pointer(beta, true)]).is_none());
    assert!(frames.pointer(beta, false).is_none());
    frames.settle();
    assert_eq!(
        frames.body.model.selected(),
        Some(id(3)),
        "an earlier unrelated release cannot erase a later genuine press"
    );
    let open = frames.open().rect.center();
    assert!(frames.pointer(open, true).is_none());
    frames.discard_next = true;
    assert_eq!(
        Frames::opened(frames.pointer(open, false)),
        id(3),
        "only the final committed pass dispatches the original release"
    );
}

#[test]
fn folder_jump_body_ime_keeps_real_query_commit_once_then_blocks_terminal_confirmation_until_fresh_focus() {
    for key_before_commit in [false, true] {
        let mut frames = Frames::new();
        assert!(
            frames
                .show(vec![Event::Ime(egui::ImeEvent::Preedit {
                    text: "Beta".into(),
                    active_range_chars: None
                })])
                .is_none()
        );
        let commit = Event::Ime(egui::ImeEvent::Commit("Beta".into()));
        let confirm = key(Key::Enter, true);
        assert!(
            frames
                .show(if key_before_commit {
                    vec![confirm, commit]
                } else {
                    vec![commit, confirm]
                })
                .is_none()
        );
        assert!(frames.show(vec![key(Key::Enter, false)]).is_none());
        frames.settle();
        assert_eq!(
            frames.body.query, "Beta",
            "actual TextEdit received the genuine commit exactly once"
        );
        assert!(frames.show(vec![key(Key::Enter, true)]).is_none());
        assert!(
            frames.show(vec![key(Key::Enter, false)]).is_none(),
            "later confirmation cannot Open the unchanged query focus"
        );
        assert!(frames.click(frames.query()).is_none(), "fresh explicit focus after composition");
        frames.settle();
        assert!(frames.show(vec![key(Key::Enter, true)]).is_none());
        assert_eq!(Frames::opened(frames.show(vec![key(Key::Enter, false)])), id(3));
    }
}

#[test]
fn folder_jump_body_source_replacement_and_hidden_owner_purge_old_results_and_open_intent() {
    let mut frames = Frames::new();
    frames.replace_query("Beta");
    assert_eq!(frames.body.model.rows().map(|row| row.folder).collect::<Vec<_>>(), [id(3)]);
    let open = frames.open().rect.center();
    assert!(frames.pointer(open, true).is_none());
    frames.source.session = id(902);
    assert!(
        frames.pointer(open, false).is_none(),
        "same folder UUID cannot carry a press across actual source replacement"
    );
    frames.settle();
    assert!(frames.body.query.is_empty(), "old source query is private state too");
    frames.policy.allowed = false;
    assert!(frames.show(Vec::new()).is_none());
    assert_eq!(frames.body.model.source(), None);
    assert_eq!(frames.body.model.rows().len(), 0);
    assert!(!frames.body.controls.iter().any(|control| control.pointer.is_some()));
}

#[test]
fn folder_jump_body_narrow_short_scaled_layout_keeps_real_hits_and_footer_inside_clips() {
    for (width, height, scale) in [(520.0, 440.0, 1.0), (300.0, 260.0, 1.0), (300.0, 260.0, 1.5)] {
        let mut frames = Frames::new();
        frames.size = vec2(width, height);
        frames.ctx.set_pixels_per_point(scale);
        frames
            .inventory
            .server_updated(vec![folder(2, 1, &"Very long folder name 🧥 ".repeat(20))], Vec::new());
        frames.settle();
        let canvas = Rect::from_min_size(Pos2::ZERO, frames.size);
        let open = frames.open();
        assert!(
            open.rect.is_finite() && open.rect.is_positive() && canvas.contains_rect(open.rect),
            "footer at {width}×{height}/{scale}"
        );
        let row = frames.row(id(3));
        assert!(row.rect.is_finite() && row.rect.is_positive() && canvas.contains_rect(row.rect));
        assert!(
            row.rect.bottom() <= open.rect.top(),
            "actual clipped list cannot cover the Open footer"
        );
        assert!(frames.click(row).is_none());
        frames.settle();
        assert_eq!(Frames::opened(frames.click(frames.open())), id(3));
    }
}
