use super::super::fixtures::InventoryFixture;
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
    inventory: InventoryFixture,
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
            inventory: InventoryFixture::new(inventory),
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
            assert!(output.platform_output.num_completed_passes > 1);
        }
        if let Some(Action::Open(ticket)) = result
            && !self
                .body
                .take_open(ticket, &self.ctx, &self.inventory, Some(self.source), self.policy)
        {
            result = None;
        }
        self.body.finish_batch(&self.ctx);
        result
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            assert!(self.show(Vec::new()).is_none());
        }
    }

    fn attach_owner(&mut self) -> super::super::Chooser {
        let mut owner = super::super::Chooser::default();
        owner.event(&winit::event::WindowEvent::Focused(true));
        owner.body = std::mem::take(&mut self.body);
        owner.owner = Some(self.source.window);
        owner.source = Some(self.source);
        owner
    }

    /// The actual production owner/Core path: original body proposal, optional
    /// LATER layout discard, whole-context completion, one-shot final dispatch.
    fn show_owner(&mut self, owner: &mut super::super::Chooser, events: Vec<Event>, discard_after: bool, clip_final: bool) -> Option<Uuid> {
        self.frame += 1;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            focused: true,
            time: Some(self.frame as f64 / 60.0),
            events,
            ..Default::default()
        };
        let palette = crate::theme::Theme::default().palette();
        let output = self.ctx.run_ui(raw, |ui| {
            owner.prepare_input(ui.ctx());
            if clip_final && ui.ctx().current_pass_index() > 0 {
                ui.set_clip_rect(Rect::NOTHING);
            }
            assert!(!owner.draw_body(ui, &palette, &self.inventory, Some(self.source), self.policy));
            if discard_after && ui.ctx().current_pass_index() == 0 {
                assert!(owner.owner.is_some(), "a proposal cannot retire its owner or navigate mid-pass");
                ui.ctx().request_discard("a later widget changed the layout after the chooser");
            }
        });
        if discard_after {
            assert!(output.platform_output.num_completed_passes > 1);
        }
        let opened = owner.take_open(&self.ctx, &self.inventory, Some(self.source), self.policy);
        assert!(
            owner
                .take_open(&self.ctx, &self.inventory, Some(self.source), self.policy)
                .is_none(),
            "one completed proposal is consumed once"
        );
        owner.finish_consumed_batch(&self.ctx);
        opened
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
    fn focus_with_tab(&mut self, control: Control) {
        assert!(control.visible(), "keyboard fixture starts from a real visible command");
        // egui 0.37 buttons are focusable, but pointer clicks do not request
        // keyboard focus. Traverse the actual query/results/footer focus chain;
        // do not manufacture focus or bypass the body/native command guards.
        // There are at most MAX_RESULTS rows plus query/Open/Cancel.
        for _ in 0..super::super::model::MAX_RESULTS + 3 {
            if self.ctx.memory(|memory| memory.focused()) == Some(control.widget) {
                return;
            }
            assert!(self.show(vec![key(Key::Tab, true)]).is_none());
            assert!(self.show(vec![key(Key::Tab, false)]).is_none());
        }
        assert_eq!(self.ctx.memory(|memory| memory.focused()), Some(control.widget));
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
        assert!(
            frames
                .show(vec![Event::Ime(egui::ImeEvent::Preedit {
                    text: String::new(),
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

#[test]
fn folder_jump_clipped_focused_row_rejects_held_keyboard_and_accessibility_until_fresh_visible_input() {
    let mut frames = Frames::new();
    frames.size = vec2(300.0, 260.0);
    frames.inventory.server_updated(
        (10..45).map(|value| folder(value, 1, &format!("Folder {value:02}"))).collect(),
        Vec::new(),
    );
    frames.settle();
    let alpha = frames.row(id(2));
    assert!(frames.click(alpha).is_none());
    frames.settle();
    frames.focus_with_tab(alpha);
    assert_eq!(frames.ctx.memory(|memory| memory.focused()), Some(alpha.widget));
    assert!(frames.show(vec![key(Key::Enter, true)]).is_none());
    let point = frames.row(id(2)).rect.center();
    let wheel = |delta| {
        vec![
            Event::PointerMoved(point),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                delta: vec2(0.0, delta),
                modifiers: Modifiers::NONE,
            },
        ]
    };
    for _ in 0..40 {
        if !frames.row(id(2)).visible() {
            break;
        }
        assert!(frames.show(wheel(-1_000.0)).is_none());
        frames.settle();
    }
    assert!(!frames.row(id(2)).visible(), "actual wheel must fully clip the focused first row");
    assert_eq!(frames.ctx.memory(|memory| memory.focused()), Some(alpha.widget));
    assert!(
        frames.show(vec![key(Key::Enter, false)]).is_none(),
        "held release cannot Open its now clipped target"
    );
    let click = || {
        Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: alpha.widget.accesskit_id(),
            data: None,
        })
    };
    assert!(!frames.row(id(2)).visible());
    assert!(
        frames.show(vec![click()]).is_none(),
        "focused accessibility target still needs a visible actual clipped span"
    );
    for _ in 0..40 {
        if !frames.row(id(2)).visible() {
            break;
        }
        assert!(frames.show(wheel(-1_000.0)).is_none());
        frames.settle();
    }
    assert!(!frames.row(id(2)).visible());
    assert!(
        frames.show(vec![key(Key::Enter, true), key(Key::Enter, false)]).is_none(),
        "new capture cannot use an already clipped row"
    );
    for _ in 0..40 {
        if frames.row(id(2)).visible() {
            break;
        }
        assert!(frames.show(wheel(1_000.0)).is_none());
        frames.settle();
    }
    assert!(frames.row(id(2)).visible());
    assert!(frames.show(vec![key(Key::Enter, true)]).is_none());
    assert_eq!(Frames::opened(frames.show(vec![key(Key::Enter, false)])), id(2));
    frames.settle();
    assert_eq!(Frames::opened(frames.show(vec![click()])), id(2));
}

#[test]
fn folder_jump_owner_defers_navigation_until_whole_context_final_pass_and_rechecks_current_clip_once() {
    for clip_final in [false, true] {
        let mut frames = Frames::new();
        let open = frames.open().rect.center();
        let mut owner = frames.attach_owner();
        let pointer = |down| {
            vec![
                Event::PointerMoved(open),
                Event::PointerButton {
                    pos: open,
                    button: PointerButton::Primary,
                    pressed: down,
                    modifiers: Modifiers::NONE,
                },
            ]
        };
        assert!(frames.show_owner(&mut owner, pointer(true), false, false).is_none());
        let opened = frames.show_owner(&mut owner, pointer(false), true, clip_final);
        let mut inventory_ui = crate::ui::inventory::InventoryUi::default();
        if let Some(folder) = opened {
            inventory_ui.open_folder_window(folder);
        }
        assert_eq!(inventory_ui.windows.len(), usize::from(!clip_final));
        if !clip_final {
            assert_eq!(inventory_ui.windows[0].root, id(2));
        }
        assert!(owner.pending_open.is_none());
    }
}

#[test]
fn folder_jump_native_rejected_press_and_up_in_one_batch_cannot_open_then_fresh_batch_works() {
    use winit::event::{ElementState, WindowEvent};
    use winit::keyboard::{Key as NativeKey, NamedKey};
    for native in [
        NativeKey::Named(NamedKey::Enter),
        NativeKey::Named(NamedKey::Space),
        NativeKey::Character(" ".into()),
    ] {
        let logical = if native == NativeKey::Named(NamedKey::Enter) {
            Key::Enter
        } else {
            Key::Space
        };
        let mut frames = Frames::new();
        let open = frames.open();
        assert!(frames.pointer(open.rect.center(), true).is_none());
        assert!(frames.pointer(Pos2::ZERO, false).is_none());
        frames.settle();
        frames.focus_with_tab(open);
        assert_eq!(frames.ctx.memory(|memory| memory.focused()), Some(open.widget));
        let mut owner = frames.attach_owner();
        // The first physical press belonged to an external modal guard; its
        // native observation survives even though no eligible command saw it.
        assert!(!owner.key_edge(&native, ElementState::Pressed, false, false));
        owner.key_edge(&native, ElementState::Released, false, true);
        owner.event(&WindowEvent::Focused(false));
        owner.event(&WindowEvent::Focused(true));
        owner.key_edge(&native, ElementState::Pressed, false, true);
        assert!(frames.show_owner(&mut owner, Vec::new(), false, false).is_none());
        assert!(owner.key_edge(&native, ElementState::Pressed, false, false));
        assert!(!owner.key_edge(&native, ElementState::Released, false, false));
        assert!(
            frames
                .show_owner(&mut owner, vec![key(logical, true), key(logical, false)], false, false)
                .is_none(),
            "latest native Up cannot reauthorize a queued rejected Down"
        );
        assert!(!owner.key_edge(&native, ElementState::Pressed, false, false));
        assert!(!owner.key_edge(&native, ElementState::Released, false, false));
        assert_eq!(
            frames.show_owner(&mut owner, vec![key(logical, true), key(logical, false)], false, false),
            Some(id(2))
        );
    }
}

#[test]
fn folder_jump_owned_ime_quarantine_survives_source_reopen_and_empty_preedit_but_fresh_composition_works() {
    use winit::event::{ElementState, Ime, WindowEvent};
    use winit::keyboard::{Key as NativeKey, NamedKey};
    let mut frames = Frames::new();
    let mut owner = frames.attach_owner();
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Preedit("Old private candidate".into(), None)));
    assert!(owner.owned_composition);
    assert!(
        frames
            .show_owner(
                &mut owner,
                vec![Event::Ime(egui::ImeEvent::Preedit {
                    text: "Old private candidate".into(),
                    active_range_chars: None
                })],
                false,
                false
            )
            .is_none()
    );
    owner.close();
    assert!(owner.ime_quarantined);
    let old_epoch = owner.composition_epoch;
    owner.event(&WindowEvent::Focused(false));
    owner.event(&WindowEvent::Focused(true));
    frames.source.session = id(902);
    owner.owner = Some(frames.source.window);
    owner.source = Some(frames.source);
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Preedit("Old private candidate".into(), None)));
    assert!(
        frames
            .show_owner(
                &mut owner,
                vec![Event::Ime(egui::ImeEvent::Preedit {
                    text: "Old private candidate".into(),
                    active_range_chars: None
                })],
                false,
                false
            )
            .is_none()
    );
    assert_eq!(
        owner.composition_epoch, old_epoch,
        "refocus cannot reclassify an unterminated retired composition as new"
    );
    assert!(owner.body.query.is_empty());
    let query = *owner
        .body
        .controls
        .iter()
        .find(|control| Some(control.widget) == owner.body.query_widget)
        .expect("new actual query");
    let pointer = |down| Event::PointerButton {
        pos: query.rect.center(),
        button: PointerButton::Primary,
        pressed: down,
        modifiers: Modifiers::NONE,
    };
    assert!(
        frames
            .show_owner(
                &mut owner,
                vec![Event::PointerMoved(query.rect.center()), pointer(true)],
                false,
                false
            )
            .is_none()
    );
    assert_eq!(frames.ctx.memory(|memory| memory.focused()), Some(query.widget));
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Preedit(String::new(), None)));
    assert!(
        frames
            .show_owner(
                &mut owner,
                vec![
                    pointer(false),
                    Event::Ime(egui::ImeEvent::Preedit {
                        text: String::new(),
                        active_range_chars: None
                    })
                ],
                false,
                false
            )
            .is_none()
    );
    assert!(
        owner.ime_quarantined && owner.owned_composition && owner.composing(),
        "empty Preedit is not old-candidate retirement proof"
    );
    assert_eq!(frames.ctx.memory(|memory| memory.focused()), Some(query.widget));
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Commit("Old private candidate".into())));
    owner.key_edge(&NativeKey::Named(NamedKey::Enter), ElementState::Pressed, false, false);
    owner.key_edge(&NativeKey::Named(NamedKey::Enter), ElementState::Released, false, false);
    assert!(
        frames
            .show_owner(
                &mut owner,
                vec![
                    Event::Ime(egui::ImeEvent::Commit("Old private candidate".into())),
                    key(Key::Enter, true),
                    key(Key::Enter, false)
                ],
                false,
                false
            )
            .is_none()
    );
    assert!(
        owner.body.query.is_empty(),
        "late original commit cannot retarget the new source query"
    );
    assert!(!owner.ime_quarantined && !owner.owned_composition);
    assert_ne!(frames.ctx.memory(|memory| memory.focused()), Some(query.widget));
    assert!(
        frames
            .show_owner(
                &mut owner,
                vec![Event::PointerMoved(query.rect.center()), pointer(true)],
                false,
                false
            )
            .is_none()
    );
    assert!(frames.show_owner(&mut owner, vec![pointer(false)], false, false).is_none());
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Preedit("Beta".into(), None)));
    assert!(
        frames
            .show_owner(
                &mut owner,
                vec![Event::Ime(egui::ImeEvent::Preedit {
                    text: "Beta".into(),
                    active_range_chars: None
                })],
                false,
                false
            )
            .is_none()
    );
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Preedit(String::new(), None)));
    assert!(
        frames
            .show_owner(
                &mut owner,
                vec![Event::Ime(egui::ImeEvent::Preedit {
                    text: String::new(),
                    active_range_chars: None
                })],
                false,
                false
            )
            .is_none()
    );
    assert!(owner.composing(), "global shortcut observation also waits through empty Preedit");
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Commit("Beta".into())));
    assert!(
        frames
            .show_owner(&mut owner, vec![Event::Ime(egui::ImeEvent::Commit("Beta".into()))], false, false)
            .is_none()
    );
    assert_eq!(
        owner.body.query, "Beta",
        "fresh same-source commit still reaches its real TextEdit once"
    );
    for _ in 0..4 {
        assert!(frames.show_owner(&mut owner, Vec::new(), false, false).is_none());
    }
    assert!(
        frames
            .show_owner(
                &mut owner,
                vec![Event::PointerMoved(query.rect.center()), pointer(true)],
                false,
                false
            )
            .is_none()
    );
    assert!(frames.show_owner(&mut owner, vec![pointer(false)], false, false).is_none());
    assert!(!owner.key_edge(&NativeKey::Named(NamedKey::Enter), ElementState::Pressed, false, false));
    assert!(!owner.key_edge(&NativeKey::Named(NamedKey::Enter), ElementState::Released, false, false));
    assert_eq!(
        frames.show_owner(&mut owner, vec![key(Key::Enter, true), key(Key::Enter, false)], false, false),
        Some(id(3))
    );
}

#[test]
fn folder_jump_ime_quarantine_never_claims_another_actual_editors_preedit_or_commit() {
    use winit::event::{Ime, WindowEvent};
    let mut frames = Frames::new();
    let mut owner = frames.attach_owner();
    let editor = Id::unique("folder_jump_unowned_editor");
    let mut text = String::new();
    let mut render = |owner: &mut super::super::Chooser, events: Vec<Event>| {
        frames.frame += 1;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, frames.size)),
            focused: true,
            time: Some(frames.frame as f64 / 60.0),
            events,
            ..Default::default()
        };
        let _output = frames.ctx.run_ui(raw, |ui| {
            owner.prepare_input(ui.ctx());
            ui.add(egui::TextEdit::singleline(&mut text).id(editor));
            if owner.owner.is_some() {
                assert!(!owner.draw_body(
                    ui,
                    &crate::theme::Theme::default().palette(),
                    &frames.inventory,
                    Some(frames.source),
                    frames.policy
                ));
            }
        });
        owner.finish_consumed_batch(&frames.ctx);
    };
    for _ in 0..4 {
        render(&mut owner, Vec::new());
    }
    let at = frames.ctx.read_response(editor).expect("actual unrelated editor").rect.center();
    let pointer = |down| {
        vec![
            Event::PointerMoved(at),
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: down,
                modifiers: Modifiers::NONE,
            },
        ]
    };
    render(&mut owner, pointer(true));
    render(&mut owner, pointer(false));
    assert_eq!(frames.ctx.memory(|memory| memory.focused()), Some(editor));
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Preedit("Other retained".into(), None)));
    assert!(!owner.owned_composition);
    render(
        &mut owner,
        vec![Event::Ime(egui::ImeEvent::Preedit {
            text: "Other retained".into(),
            active_range_chars: None,
        })],
    );
    owner.event(&WindowEvent::Focused(false));
    owner.event(&WindowEvent::Focused(true));
    assert!(owner.composing(), "focus snapshots cannot certify an unowned composition ended");
    owner.close();
    assert!(
        !owner.ime_quarantined,
        "global IME activity cannot authorize swallowing another editor's text"
    );
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Preedit(String::new(), None)));
    assert!(owner.composing());
    render(
        &mut owner,
        vec![Event::Ime(egui::ImeEvent::Preedit {
            text: String::new(),
            active_range_chars: None,
        })],
    );
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Commit("Other retained".into())));
    render(&mut owner, vec![Event::Ime(egui::ImeEvent::Commit("Other retained".into()))]);
    assert_eq!(text, "Other retained");
}

#[test]
fn folder_jump_owned_native_terminal_dropped_before_egui_acknowledges_only_after_batch_and_preserves_new_epoch() {
    use winit::event::{Ime, WindowEvent};
    let mut frames = Frames::new();
    let mut owner = frames.attach_owner();
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Preedit("Old candidate".into(), None)));
    assert!(
        frames
            .show_owner(
                &mut owner,
                vec![Event::Ime(egui::ImeEvent::Preedit {
                    text: "Old candidate".into(),
                    active_range_chars: None
                })],
                false,
                false
            )
            .is_none()
    );
    owner.close();
    frames.source.session = id(902);
    owner.owner = Some(frames.source.window);
    owner.source = Some(frames.source);
    assert!(frames.show_owner(&mut owner, Vec::new(), false, false).is_none());
    let query = *owner
        .body
        .controls
        .iter()
        .find(|control| Some(control.widget) == owner.body.query_widget)
        .expect("new query");
    let pointer = |down| {
        vec![
            Event::PointerMoved(query.rect.center()),
            Event::PointerButton {
                pos: query.rect.center(),
                button: PointerButton::Primary,
                pressed: down,
                modifiers: Modifiers::NONE,
            },
        ]
    };
    assert!(frames.show_owner(&mut owner, pointer(true), false, false).is_none());
    assert!(frames.show_owner(&mut owner, pointer(false), false, false).is_none());
    // The ordinary App guard consumed this ORIGINAL native terminal before
    // egui; no substitute Commit/RawInput is manufactured for the fixture.
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Commit("Old candidate".into())));
    let old = owner.composition_epoch;
    assert!(owner.ime_quarantined, "a native terminal alone cannot acknowledge a queued batch");
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Preedit("New candidate".into(), None)));
    assert!(owner.composition_epoch > old && owner.owned_composition);
    assert!(
        frames
            .show_owner(
                &mut owner,
                vec![Event::Ime(egui::ImeEvent::Preedit {
                    text: "New candidate".into(),
                    active_range_chars: None
                })],
                true,
                false
            )
            .is_none()
    );
    assert!(
        !owner.ime_quarantined,
        "actual whole-context completion can acknowledge the dropped old terminal"
    );
    assert!(
        owner.owned_composition && owner.composition_epoch > old,
        "the old terminal cannot erase newer owned composition evidence"
    );
    assert!(
        owner.body.query.is_empty(),
        "quarantined batches do not partially admit a newer preedit"
    );
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Disabled));
    // egui-winit 0.37 forwards no egui event for native Disabled. Only the
    // original native terminal plus this completed batch acknowledges it.
    assert!(frames.show_owner(&mut owner, Vec::new(), false, false).is_none());
    assert!(!owner.owned_composition);
    assert!(frames.show_owner(&mut owner, pointer(true), false, false).is_none());
    assert!(frames.show_owner(&mut owner, pointer(false), false, false).is_none());
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Preedit("Beta".into(), None)));
    assert!(
        frames
            .show_owner(
                &mut owner,
                vec![Event::Ime(egui::ImeEvent::Preedit {
                    text: "Beta".into(),
                    active_range_chars: None
                })],
                false,
                false
            )
            .is_none()
    );
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Commit("Beta".into())));
    assert!(
        frames
            .show_owner(&mut owner, vec![Event::Ime(egui::ImeEvent::Commit("Beta".into()))], false, false)
            .is_none()
    );
    assert_eq!(owner.body.query, "Beta");
}

#[test]
fn folder_jump_ime_delete_surrounding_edits_characters_without_ending_composition_or_confirming_open() {
    let mut frames = Frames::new();
    frames.replace_query("é");
    let query = frames.query().widget;
    assert!(
        frames
            .show(vec![Event::Ime(egui::ImeEvent::Preedit {
                text: "Beta".into(),
                active_range_chars: None,
            })])
            .is_none()
    );
    assert_eq!(frames.body.query, "éBeta");
    assert!(
        frames
            .show(vec![
                Event::Ime(egui::ImeEvent::DeleteSurrounding {
                    before_chars: 1,
                    after_chars: 0,
                }),
                key(Key::Enter, true),
                key(Key::Enter, false),
            ])
            .is_none()
    );
    assert_eq!(
        frames.body.query, "Beta",
        "the original event edits the actual TextEdit by character"
    );
    assert!(frames.body.ime && frames.body.ime_pending, "surrounding deletion is not a terminal");
    assert_eq!(frames.ctx.memory(|memory| memory.focused()), Some(query));
    assert!(frames.show(vec![Event::Ime(egui::ImeEvent::Commit("Beta".into()))]).is_none());
    assert_eq!(
        frames.body.query, "Beta",
        "Commit still replaces exactly the retained preedit range"
    );
    assert_ne!(frames.ctx.memory(|memory| memory.focused()), Some(query));
    frames.settle();
    assert!(frames.show(vec![key(Key::Enter, true), key(Key::Enter, false)]).is_none());
    assert!(frames.click(frames.query()).is_none());
    assert_eq!(
        Frames::opened(frames.show(vec![key(Key::Enter, true), key(Key::Enter, false)])),
        id(3)
    );
}

#[test]
fn folder_jump_retired_ime_delete_surrounding_cannot_edit_reopened_query_or_acknowledge_old_epoch() {
    use winit::event::{Ime, WindowEvent};
    let mut frames = Frames::new();
    let mut owner = frames.attach_owner();
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Preedit("Old candidate".into(), None)));
    assert!(
        frames
            .show_owner(
                &mut owner,
                vec![Event::Ime(egui::ImeEvent::Preedit {
                    text: "Old candidate".into(),
                    active_range_chars: None,
                })],
                false,
                false,
            )
            .is_none()
    );
    owner.close();
    let old_epoch = owner.composition_epoch;
    frames.source.session = id(902);
    owner.owner = Some(frames.source.window);
    owner.source = Some(frames.source);
    assert!(frames.show_owner(&mut owner, Vec::new(), false, false).is_none());
    let query = owner.body.query_widget.expect("reopened query");
    let at = frames.ctx.read_response(query).expect("actual reopened field").rect.center();
    for down in [true, false] {
        assert!(
            frames
                .show_owner(
                    &mut owner,
                    vec![
                        Event::PointerMoved(at),
                        Event::PointerButton {
                            pos: at,
                            button: PointerButton::Primary,
                            pressed: down,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                    false,
                    false,
                )
                .is_none()
        );
    }
    assert!(
        frames
            .show_owner(&mut owner, vec![Event::Text("Gamma".into())], false, false)
            .is_none()
    );
    assert_eq!(owner.body.query, "Gamma");
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Preedit(String::new(), None)));
    assert!(
        frames
            .show_owner(
                &mut owner,
                vec![
                    Event::Ime(egui::ImeEvent::Preedit {
                        text: String::new(),
                        active_range_chars: None,
                    }),
                    Event::Ime(egui::ImeEvent::DeleteSurrounding {
                        before_chars: 1,
                        after_chars: 0,
                    }),
                    key(Key::Enter, true),
                    key(Key::Enter, false),
                ],
                true,
                false,
            )
            .is_none()
    );
    assert_eq!(
        owner.body.query, "Gamma",
        "retired surrounding deletion cannot retarget the new editor"
    );
    assert!(owner.ime_quarantined && owner.owned_composition && owner.composing());
    assert_eq!(
        owner.composition_epoch, old_epoch,
        "deletion cannot acknowledge or replace an old epoch"
    );
    owner.native_event(&frames.ctx, &WindowEvent::Ime(Ime::Commit("Old candidate".into())));
    assert!(
        frames
            .show_owner(
                &mut owner,
                vec![Event::Ime(egui::ImeEvent::Commit("Old candidate".into()))],
                false,
                false,
            )
            .is_none()
    );
    assert_eq!(owner.body.query, "Gamma", "the late original commit remains quarantined");
    assert!(!owner.ime_quarantined && !owner.owned_composition);
    assert_ne!(frames.ctx.memory(|memory| memory.focused()), Some(query));
}
