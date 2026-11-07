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
}

impl Frames {
    fn new() -> Self {
        let context = egui::Context::default();
        crate::theme::Theme {
            panel_alpha: 255,
            ..Default::default()
        }
        .apply(&context, 1.0);
        let mut result = Self {
            context,
            body: AttentionUi::default(),
            model: Attention::default(),
            host: host(),
            frame: 0,
            text: Vec::new(),
        };
        result.settle();
        result
    }

    fn pass(&mut self, events: Vec<Event>, mode: InputMode, focused: bool) -> Option<Action> {
        self.model.sync(&self.host, &self.host.attention_preferences.preferences);
        self.frame += 1;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(760.0, 900.0))),
            time: Some(self.frame as f64 / 60.0),
            events,
            focused,
            ..Default::default()
        };
        let palette = crate::theme::Theme::default().palette();
        let mut action = None;
        let output = self.context.run_ui(raw, |ui| {
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
    f.settle();
    assert!(f.has("host-attention-save-failed"));
    assert_eq!(f.model.rows().len(), 5);
    assert!(f.text.iter().all(|text| !text.contains("SYNTHETIC_PRIVATE_PATH")));
    let Some(Action::Preferences(retry)) = f.click(Id::unique(("attention_slot", slot))) else {
        panic!("retry old published value");
    };
    assert_eq!(retry, desired);
    f.body.queued(43, desired);
    f.host.attention_preferences.preferences = desired;
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
