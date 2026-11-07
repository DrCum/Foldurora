use super::*;
use crate::session_host::attention_prefs::PreferenceOperation;
use crate::session_host::slots::{SlotLabel, SlotSummary};
use crate::session_host::transport::{HostChatSnapshot, WorkerSummary};
use aurora_session::chat::{ChatKey, ChatSource, Conversation, ConversationInfo, NativeChatPolicy, ObservedIncoming, ReadMetadata};
use aurora_session::{
    AccountKey, ConversationId, GridKey, HostId, InstallationId, Label, Phase, SessionId, SessionIdentity, WorkerId, WorkerIdentity,
    WorkerStatus,
};
use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect};
use std::sync::Arc;

pub(super) fn source(worker: u8) -> ChatSource {
    ChatSource {
        worker: WorkerIdentity {
            installation: InstallationId::from_bytes([1; 32]).unwrap(),
            host: HostId::from_bytes([2; 16]).unwrap(),
            worker: WorkerId::from_bytes([worker; 16]).unwrap(),
        },
        session: SessionIdentity {
            session: SessionId::from_bytes([worker; 16]).unwrap(),
            account: AccountKey::from_bytes([worker; 32]).unwrap(),
            grid: GridKey::from_bytes([6; 32]).unwrap(),
            generation: 1,
        },
        lease_epoch: 7,
    }
}

pub(super) fn direct() -> Conversation {
    Conversation::Direct(ConversationId::from_bytes([11; 16]).unwrap())
}

pub(super) fn host() -> HostSnapshot {
    let mut snapshot = HostSnapshot::default();
    snapshot.attention_preferences = PreferenceState {
        preferences: Preferences::default(),
        writable: true,
        error: None,
        operation: None,
    };
    for (index, slot) in SlotId::all().enumerate() {
        let source = source(index as u8 + 1);
        snapshot.slots.push(SlotSummary {
            id: slot,
            label: SlotLabel::new("Same profile").unwrap(),
            available: false,
        });
        snapshot.workers.push(WorkerSummary {
            slot: Some(slot),
            binding: source.binding(),
            reachable: true,
            label: "Fallback profile".into(),
            grid: "Fallback grid".into(),
            status: Some(WorkerStatus {
                phase: Phase::Online,
                name: Label::new("Synthetic Resident").unwrap(),
                grid_label: Label::new("Synthetic Grid").unwrap(),
                native_window: index as u64 + 1,
                keyboard_focused: false,
                blocked: false,
                last_frame: None,
            }),
        });
        snapshot.chat.push(Arc::new(HostChatSnapshot {
            source,
            authenticated: true,
            live: true,
            conversations: [Conversation::Nearby, direct()]
                .into_iter()
                .enumerate()
                .map(|(index, conversation)| ConversationInfo {
                    title: Label::new("Synthetic conversation").unwrap(),
                    policy: NativeChatPolicy {
                        key: ChatKey { source, conversation },
                        revision: index as u64 + 1,
                        lease_current: true,
                        ready: true,
                        can_read: true,
                        can_send: true,
                    },
                })
                .collect(),
            reads: [Conversation::Nearby, direct()]
                .into_iter()
                .map(|conversation| {
                    (
                        conversation,
                        ReadMetadata {
                            unread: 7,
                            unread_revision: 1,
                            observed: ObservedIncoming { epoch: 1, through: 7 },
                            can_mark_read: true,
                        },
                    )
                })
                .collect(),
            lines: Vec::new(),
            receipts: Vec::new(),
            read_operations: Vec::new(),
            gap: false,
        }));
    }
    snapshot
}

pub(super) fn row_id(model: &Attention, key: ChatKey) -> Id {
    let row = model.rows().iter().find(|row| row.key == key).unwrap();
    chat_ui::source_id(row.key.source).with(("attention_row", row.key.conversation, row.id()))
}

fn key_event(key: Key, pressed: bool, repeat: bool) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat,
        modifiers: Modifiers::NONE,
    }
}

struct Frames {
    context: egui::Context,
    body: AttentionUi,
    model: Attention,
    host: HostSnapshot,
    frame: u64,
    text: Vec<String>,
    logical: [f32; 2],
    overlay: Option<Rect>,
}

impl Frames {
    fn new() -> Self {
        Self::for_size([760.0, 900.0])
    }

    fn for_size(logical: [f32; 2]) -> Self {
        let context = egui::Context::default();
        crate::theme::Theme {
            panel_alpha: 255,
            ..Default::default()
        }
        .apply(&context, 1.0);
        context.all_styles_mut(|style| style.animation_time = 0.0);
        let mut result = Self {
            context,
            body: AttentionUi::default(),
            model: Attention::default(),
            host: host(),
            frame: 0,
            text: Vec::new(),
            logical,
            overlay: None,
        };
        result.settle();
        result
    }

    fn pass(&mut self, events: Vec<Event>, mode: InputMode, focused: bool) -> Option<Action> {
        self.model.sync(&self.host, &self.host.attention_preferences.preferences);
        self.frame += 1;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(self.logical[0], self.logical[1]))),
            time: Some(self.frame as f64 / 60.0),
            events,
            focused,
            ..Default::default()
        };
        let palette = crate::theme::Theme::default().palette();
        let mut action = None;
        let output = self.context.run_ui(raw, |ui| {
            if let Some(rect) = self.overlay {
                egui::Area::new(Id::unique("synthetic_attention_blocker"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(rect.min)
                    .show(ui.ctx(), |ui| {
                        ui.allocate_exact_size(rect.size(), egui::Sense::click());
                    });
            }
            action = action.take().or(self.body.show(ui, &palette, &mut self.model, &self.host, mode));
        });
        self.text.clear();
        fn collect(shape: &egui::Shape, output: &mut Vec<String>) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, output);
                    }
                }
                egui::Shape::Text(text) => output.push(text.galley.text().to_owned()),
                _ => {}
            }
        }
        for shape in output.shapes {
            collect(&shape.shape, &mut self.text);
        }
        action
    }

    fn show(&mut self, events: Vec<Event>) -> Option<Action> {
        self.pass(events, InputMode::Live, true)
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            assert!(self.show(Vec::new()).is_none());
        }
    }

    fn rect(&mut self, id: Id) -> Rect {
        self.settle();
        self.context
            .data(|data| data.get_temp::<Rect>(id.with("rect")))
            .expect("production Attention control")
    }

    fn pointer(&mut self, at: Pos2, pressed: bool) -> Option<Action> {
        self.show(vec![
            Event::PointerMoved(at),
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ])
    }

    fn click(&mut self, id: Id) -> Option<Action> {
        let at = self.rect(id).center();
        assert!(self.pointer(at, true).is_none());
        self.pointer(at, false)
    }

    fn first(&self) -> ChatKey {
        self.model.rows()[0].key
    }
    fn choose(&mut self, key: ChatKey) {
        assert!(self.click(row_id(&self.model, key)).is_none());
        assert_eq!(self.model.selected(), Some(key));
    }
    fn has(&self, key: &'static str) -> bool {
        self.text.iter().any(|text| text.contains(aurora_i18n::text(key)))
    }
}

fn pointer_event(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn attention_batched_pointer_uses_press_pixels_and_never_an_earlier_release() {
    let mut f = Frames::new();
    let first = f.first();
    let second = f.model.rows()[1].key;
    let a = f.rect(row_id(&f.model, first)).center();
    let b = f.rect(row_id(&f.model, second)).center();
    assert!(
        f.show(vec![
            Event::PointerMoved(a),
            pointer_event(a, true),
            Event::PointerMoved(b),
            pointer_event(b, false)
        ])
        .is_none()
    );
    assert_eq!(f.model.selected(), None);
    assert!(f.show(vec![pointer_event(a, false), pointer_event(a, true)]).is_none());
    assert_eq!(f.model.selected(), None);
    assert!(f.pointer(a, false).is_none());
    assert_eq!(f.model.selected(), Some(first));
}

#[test]
fn attention_queued_press_cannot_borrow_replacement_row_selection_or_filter_base() {
    for replacement in 0..3 {
        let mut f = Frames::new();
        let first = f.first();
        let at = match replacement {
            0 => f.rect(row_id(&f.model, first)).center(),
            1 => {
                f.choose(first);
                f.rect(Id::unique("attention_open")).center()
            }
            _ => {
                f.click(Id::unique("attention_filter_toggle"));
                f.rect(Id::unique(("attention_kind", "nearby"))).center()
            }
        };
        match replacement {
            0 => {
                let source = source(40);
                let chat = Arc::make_mut(&mut f.host.chat[0]);
                chat.source = source;
                for info in &mut chat.conversations {
                    info.policy.key.source = source;
                }
                f.host.workers[0].binding = source.binding();
            }
            1 => {
                f.model.select(f.model.rows()[1].key);
            }
            _ => {
                f.host.attention_preferences.preferences.nearby = true;
            }
        }
        // No intervening paint: this native press refers to the old pixels.
        assert!(f.pointer(at, true).is_none());
        assert!(f.pointer(at, false).is_none());
        if replacement == 0 {
            assert_eq!(f.model.selected(), None);
        }
    }
}

#[test]
fn attention_covered_control_cannot_capture_a_pointer_gesture() {
    let mut f = Frames::new();
    let first = f.first();
    f.choose(first);
    let rect = f.rect(Id::unique("attention_open"));
    f.overlay = Some(rect);
    f.settle();
    assert!(f.pointer(rect.center(), true).is_none());
    assert!(f.pointer(rect.center(), false).is_none());
}

#[test]
fn attention_ime_preedit_commit_and_disabled_cancel_keyboard_gestures() {
    for commit in [false, true] {
        let mut f = Frames::new();
        let selected = f.first();
        f.choose(selected);
        let id = Id::unique("attention_open");
        f.rect(id);
        let widget = f.context.data(|data| data.get_temp::<Id>(id.with("widget"))).unwrap();
        f.context.memory_mut(|memory| memory.request_focus(widget));
        f.settle();
        assert!(
            f.show(vec![
                Event::Ime(egui::ImeEvent::Preedit {
                    text: "あ".into(),
                    active_range_chars: None
                }),
                key_event(Key::Enter, true, false)
            ])
            .is_none()
        );
        assert!(f.show(vec![key_event(Key::Enter, false, false)]).is_none());
        assert!(f.show(vec![key_event(Key::Space, true, false)]).is_none());
        assert!(f.show(vec![key_event(Key::Space, false, false)]).is_none());
        #[allow(deprecated)]
        let finish = if commit {
            egui::ImeEvent::Commit("あ".into())
        } else {
            egui::ImeEvent::Disabled
        };
        assert!(f.show(vec![Event::Ime(finish), key_event(Key::Enter, true, false)]).is_none());
        assert!(f.show(vec![key_event(Key::Enter, false, false)]).is_none());
        f.context.memory_mut(|memory| memory.request_focus(widget));
        f.settle();
        assert!(f.show(vec![key_event(Key::Space, true, false)]).is_none());
        let Some(Action::OpenChat(ticket)) = f.show(vec![key_event(Key::Space, false, false)]) else {
            panic!("fresh key after IME dismissal");
        };
        assert_eq!(ticket.key(), selected);
    }
}

#[test]
fn attention_accessibility_click_uses_focused_prior_painted_identity() {
    let mut f = Frames::new();
    let selected = f.first();
    f.choose(selected);
    let id = Id::unique("attention_open");
    f.rect(id);
    let widget = f.context.data(|data| data.get_temp::<Id>(id.with("widget"))).unwrap();
    f.context.memory_mut(|memory| memory.request_focus(widget));
    f.settle();
    let click = || {
        Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: widget.accesskit_id(),
            data: None,
        })
    };
    let Some(Action::OpenChat(ticket)) = f.show(vec![click()]) else {
        panic!("targeted accessibility click");
    };
    assert_eq!(ticket.key(), selected);
    f.model.select(f.model.rows()[1].key);
    assert!(f.show(vec![click()]).is_none());
}

#[test]
fn attention_surface_failure_recovers_only_unqueued_preferences_and_prunes_revoked_hits() {
    let mut f = Frames::new();
    f.click(Id::unique("attention_filter_toggle"));
    let Some(Action::Preferences(desired)) = f.click(Id::unique(("attention_kind", "nearby"))) else {
        panic!("unqueued desired settings");
    };
    f.body.surface_failed();
    f.settle();
    assert!(f.has("host-attention-save-failed"));
    let Some(Action::Preferences(retry)) = f.click(Id::unique(("attention_kind", "nearby"))) else {
        panic!("retry after surface loss");
    };
    assert_eq!(retry, desired);
    f.body.queued(73, retry);
    f.body.surface_failed();
    f.settle();
    assert!(f.has("host-attention-saving"));
    f.host.attention_preferences.operation = Some(PreferenceOperation {
        ticket: 73,
        preferences: retry,
        error: None,
    });
    f.host.attention_preferences.preferences = retry;
    f.settle();
    assert!(!f.has("host-attention-saving"));
    f.host.chat.clear();
    f.model.sync(&f.host, &f.host.attention_preferences.preferences);
    f.body.sync(&f.model, &f.host.attention_preferences);
    assert!(
        f.body
            .controls
            .iter()
            .all(|control| matches!(control.intent, Intent::Preferences { .. }))
    );
    f.body.surface_failed();
    assert!(f.body.controls.is_empty() && f.body.gesture.is_none());
}

#[test]
fn attention_pointer_selection_is_exact_across_duplicate_labels_and_roster_reorder() {
    let mut f = Frames::new();
    let selected = f.model.rows()[2].key;
    f.choose(selected);
    f.host.workers.reverse();
    f.host.chat.reverse();
    f.host.slots.reverse();
    f.settle();
    assert_eq!(f.model.selected(), Some(selected));
    let Some(Action::OpenChat(ticket)) = f.click(Id::unique("attention_open")) else {
        panic!("explicit passive navigation");
    };
    assert_eq!(f.model.open_chat(ticket), Some(selected));
    assert!(
        f.host
            .chat
            .iter()
            .all(|chat| chat.read_operations.is_empty() && chat.receipts.is_empty())
    );
}

#[test]
fn attention_held_open_rejects_new_observation_and_policy_revision() {
    for policy in [false, true] {
        let mut f = Frames::new();
        let selected = f.first();
        f.choose(selected);
        let at = f.rect(Id::unique("attention_open")).center();
        assert!(f.pointer(at, true).is_none());
        let chat = Arc::make_mut(&mut f.host.chat[0]);
        if policy {
            chat.conversations[1].policy.revision += 1;
        } else {
            chat.reads.get_mut(&direct()).unwrap().observed.through += 1;
        }
        assert!(f.pointer(at, false).is_none());
        assert_eq!(f.model.selected(), Some(selected));
    }
}

#[test]
fn attention_held_row_cannot_select_a_replacement_source() {
    let mut f = Frames::new();
    let selected = f.first();
    let at = f.rect(row_id(&f.model, selected)).center();
    assert!(f.pointer(at, true).is_none());
    let replacement = source(40);
    let chat = Arc::make_mut(&mut f.host.chat[0]);
    chat.source = replacement;
    for conversation in &mut chat.conversations {
        conversation.policy.key.source = replacement;
    }
    f.host.workers[0].binding = replacement.binding();
    assert!(f.pointer(at, false).is_none());
    assert_eq!(f.model.selected(), None);
    assert!(f.model.rows().iter().all(|row| row.key.source != selected.source));
}

#[test]
fn attention_explicit_missing_selection_stays_unavailable() {
    let mut f = Frames::new();
    let selected = f.first();
    f.choose(selected);
    f.host.chat.remove(0);
    f.host.workers.remove(0);
    f.settle();
    assert_eq!(f.model.selected(), Some(selected));
    assert!(f.has("host-attention-unavailable"));
    assert!(f.click(Id::unique("attention_open")).is_none());
}

#[test]
fn attention_dismiss_reappearance_cannot_reuse_a_held_action() {
    let mut f = Frames::new();
    let selected = f.first();
    f.choose(selected);
    let old_id = row_id(&f.model, selected);
    let Some(Action::Dismiss(ticket)) = f.click(Id::unique("attention_dismiss")) else {
        panic!("local dismiss intent");
    };
    assert!(f.model.dismiss(ticket));
    f.settle();
    assert!(!f.model.rows().iter().any(|row| row.key == selected));
    let read = Arc::make_mut(&mut f.host.chat[0]).reads.get_mut(&direct()).unwrap();
    read.unread += 1;
    read.unread_revision += 1;
    read.observed.through += 1;
    f.settle();
    assert_ne!(row_id(&f.model, selected), old_id);
    assert!(!f.model.validate(ticket));
    assert!(f.host.chat.iter().all(|chat| chat.read_operations.is_empty()));
}

#[test]
fn attention_held_next_keeps_press_target_when_an_earlier_row_disappears() {
    let mut f = Frames::new();
    let selected = f.first();
    f.choose(selected);
    let expected = f.model.capture_navigation(Direction::Next).unwrap().key();
    let at = f.rect(Id::unique(("attention_action", "next"))).center();
    assert!(f.pointer(at, true).is_none());
    // The selected first row disappears; the captured next row is still valid.
    f.host.chat.remove(0);
    f.host.workers.remove(0);
    let Some(Action::OpenChat(ticket)) = f.pointer(at, false) else {
        panic!("same captured next row remains valid");
    };
    assert_eq!(ticket.key(), expected);
    assert_eq!(f.model.open_chat(ticket), Some(expected));
}

#[test]
fn attention_held_next_does_not_substitute_after_its_target_disappears() {
    let mut f = Frames::new();
    let selected = f.first();
    f.choose(selected);
    let at = f.rect(Id::unique(("attention_action", "next"))).center();
    assert!(f.pointer(at, true).is_none());
    f.host.chat.remove(1);
    f.host.workers.remove(1);
    assert!(f.pointer(at, false).is_none());
    assert_eq!(f.model.selected(), Some(selected));
}

#[test]
fn attention_keyboard_press_is_captured_and_repeat_cannot_rearm() {
    let mut f = Frames::new();
    let selected = f.first();
    f.choose(selected);
    let id = Id::unique("attention_open");
    f.rect(id);
    let widget = f.context.data(|data| data.get_temp::<Id>(id.with("widget"))).unwrap();
    f.context.memory_mut(|memory| memory.request_focus(widget));
    f.settle();
    assert!(f.show(vec![key_event(Key::Space, true, false)]).is_none());
    assert!(f.show(vec![key_event(Key::Space, true, true)]).is_none());
    let Some(Action::OpenChat(ticket)) = f.show(vec![key_event(Key::Space, false, false)]) else {
        panic!("keyboard release of captured target");
    };
    assert_eq!(ticket.key(), selected);
    assert!(f.show(vec![key_event(Key::Space, false, false)]).is_none());
}

#[test]
fn attention_keyboard_source_change_focus_loss_and_modality_cancel_actions() {
    for reason in 0..4 {
        let mut f = Frames::new();
        let selected = f.first();
        f.choose(selected);
        let id = Id::unique("attention_open");
        f.rect(id);
        let widget = f.context.data(|data| data.get_temp::<Id>(id.with("widget"))).unwrap();
        f.context.memory_mut(|memory| memory.request_focus(widget));
        f.settle();
        assert!(f.show(vec![key_event(Key::Enter, true, false)]).is_none());
        match reason {
            0 => {
                f.model.select(f.model.rows()[1].key);
            }
            1 => {
                assert!(f.pass(Vec::new(), InputMode::Live, false).is_none());
            }
            2 => {
                assert!(f.pass(Vec::new(), InputMode::Navigating, true).is_none());
            }
            _ => f.body.cancel_gestures(),
        }
        assert!(f.show(vec![key_event(Key::Enter, false, false)]).is_none());
    }
}

#[test]
fn attention_filter_defaults_stable_slots_and_exact_save_receipt() {
    let mut f = Frames::new();
    assert!(f.click(Id::unique("attention_filter_toggle")).is_none());
    assert_eq!(f.model.rows().len(), 5);
    assert!(!f.host.attention_preferences.preferences.nearby);
    assert!(f.host.attention_preferences.preferences.direct);
    f.host.slots.reverse();
    f.settle();
    let slot = SlotId::try_from(3).unwrap();
    let Some(Action::Preferences(desired)) = f.click(Id::unique(("attention_slot", slot))) else {
        panic!("save requested");
    };
    assert_eq!(desired.include_slots, [true, true, true, false, true]);
    assert_eq!(f.host.attention_preferences.preferences, Preferences::default());
    f.body.queued(42, desired);
    f.settle();
    assert!(f.has("host-attention-saving"));
    assert!(f.click(Id::unique(("attention_kind", "nearby"))).is_none());
    f.host.attention_preferences.operation = Some(PreferenceOperation {
        ticket: 41,
        preferences: desired,
        error: None,
    });
    f.settle();
    assert!(f.has("host-attention-saving"));
    f.host.attention_preferences.operation = Some(PreferenceOperation {
        ticket: 42,
        preferences: desired,
        error: Some("SYNTHETIC_PRIVATE_PATH".into()),
    });
    // Match the actual producer: a writable store also publishes its failed
    // save diagnostic, while preserving the last successfully saved settings.
    f.host.attention_preferences.error = Some("SYNTHETIC_PRIVATE_PATH".into());
    f.settle();
    assert!(f.has("host-attention-save-failed"));
    assert!(!f.has("host-attention-load-failed"));
    assert_eq!(f.model.rows().len(), 5);
    assert!(f.text.iter().all(|text| !text.contains("SYNTHETIC_PRIVATE_PATH")));
    let Some(Action::Preferences(retry)) = f.click(Id::unique(("attention_slot", slot))) else {
        panic!("retry old published value");
    };
    assert_eq!(retry, desired);
    f.body.queued(43, desired);
    f.host.attention_preferences.preferences = desired;
    f.host.attention_preferences.error = None;
    f.host.attention_preferences.operation = Some(PreferenceOperation {
        ticket: 43,
        preferences: desired,
        error: None,
    });
    f.settle();
    assert!(!f.has("host-attention-saving"));
    assert!(!f.has("host-attention-save-failed"));
    assert_eq!(f.model.rows().len(), 4);
}

#[test]
fn attention_loading_and_failed_preferences_hide_rows_and_disable_actions() {
    let mut f = Frames::new();
    let selected = f.first();
    f.choose(selected);
    f.host.attention_preferences = PreferenceState::default();
    f.settle();
    assert!(f.has("host-attention-loading"));
    assert!(f.text.iter().all(|text| !text.contains("Synthetic conversation")));
    assert!(f.click(Id::unique("attention_open")).is_none());
    f.host.attention_preferences.error = Some("SYNTHETIC_PRIVATE_PATH".into());
    f.settle();
    assert!(f.has("host-attention-load-failed"));
    assert!(f.text.iter().all(|text| !text.contains("SYNTHETIC_PRIVATE_PATH")));
}

#[test]
fn attention_preference_queue_failure_keeps_published_filters() {
    let mut f = Frames::new();
    assert!(f.click(Id::unique("attention_filter_toggle")).is_none());
    let Some(Action::Preferences(desired)) = f.click(Id::unique(("attention_kind", "nearby"))) else {
        panic!("nearby save intent");
    };
    assert!(desired.nearby);
    f.body.queue_failed();
    f.settle();
    assert!(f.has("host-attention-save-failed"));
    assert!(!f.host.attention_preferences.preferences.nearby);
    assert_eq!(f.model.rows().len(), 5);
}

#[test]
fn attention_compact_filters_toggle_without_crowding_reminders_or_actions() {
    for logical in [[540.0, 580.0], [380.0, 420.0], [380.0, 340.0]] {
        let mut f = Frames::for_size(logical);
        let selected = f.first();
        f.choose(selected);
        assert!(f.has("host-attention-filters"));
        assert!(!f.has("host-attention-profiles"));
        let closed = f
            .context
            .data(|data| data.get_temp::<Rect>(Id::unique("attention_list_clip")))
            .unwrap();
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(logical[0], logical[1]));
        for name in ["attention_open", "attention_dismiss"] {
            assert!(canvas.contains_rect(f.rect(Id::unique(name))), "collapsed actions fit {logical:?}");
        }
        assert!(f.click(Id::unique("attention_filter_toggle")).is_none());
        f.settle();
        assert!(f.has("host-attention-profiles"));
        let filter_clip = f
            .context
            .data(|data| data.get_temp::<Rect>(Id::unique("attention_filter_scroll")))
            .unwrap();
        assert!(filter_clip.height() <= 161.0 && canvas.contains_rect(filter_clip));
        let expanded = f
            .context
            .data(|data| data.get_temp::<Rect>(Id::unique("attention_list_clip")))
            .unwrap();
        assert!(expanded.height() >= 48.0, "reminder list stays reachable {logical:?}: {expanded:?}");
        assert!(closed.height() > expanded.height());
        for name in ["attention_open", "attention_dismiss"] {
            assert!(canvas.contains_rect(f.rect(Id::unique(name))), "expanded actions fit {logical:?}");
        }
        // Expansion is presentation only; a reminder still opens its exact
        // selected source, without saving filters or mutating native unread.
        let Some(Action::OpenChat(ticket)) = f.click(Id::unique("attention_open")) else {
            panic!("compact action remains reachable");
        };
        assert_eq!(ticket.key(), selected);
        assert!(f.click(Id::unique("attention_filter_toggle")).is_none());
        f.settle();
        assert!(!f.has("host-attention-profiles"));
        assert_eq!(f.host.attention_preferences.preferences, Preferences::default());
    }
}
