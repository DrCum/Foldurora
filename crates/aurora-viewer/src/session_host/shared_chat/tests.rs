use super::*;
use crate::session_host::chat_window::{edit_input, edit_prefix, suppress_enter_repeat};
use crate::session_host::slots::{SlotLabel, SlotSummary};
use crate::session_host::transport::WorkerSummary;
use aurora_session::chat::{ChatKey, Conversation, ConversationInfo, NativeChatPolicy, SendOutcome, SendReceipt};
use aurora_session::{
    AccountKey, ConversationId, GridKey, HostId, InstallationId, Label, Phase, SessionId, SessionIdentity, WorkerId, WorkerIdentity,
    WorkerStatus,
};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect};

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

fn direct() -> Conversation {
    Conversation::Direct(ConversationId::from_bytes([11; 16]).unwrap())
}

pub(super) fn host() -> HostSnapshot {
    let mut snapshot = HostSnapshot::default();
    for (index, slot) in SlotId::all().enumerate() {
        let source = source(index as u8 + 1);
        snapshot.slots.push(SlotSummary {
            id: slot,
            label: SlotLabel::new("Same label").unwrap(),
            available: false,
        });
        snapshot.workers.push(WorkerSummary {
            slot: Some(slot),
            binding: source.binding(),
            status: Some(WorkerStatus {
                phase: Phase::Online,
                name: Label::new("Same Resident").unwrap(),
                grid_label: Label::new("Same Grid").unwrap(),
                native_window: index as u64 + 1,
                keyboard_focused: false,
                blocked: false,
                last_frame: None,
            }),
            reachable: true,
            label: "Same Resident".into(),
            grid: "Same Grid".into(),
        });
        snapshot.chat.push(Arc::new(HostChatSnapshot {
            source,
            authenticated: true,
            live: true,
            conversations: [Conversation::Nearby, direct()]
                .into_iter()
                .enumerate()
                .map(|(index, conversation)| ConversationInfo {
                    title: Label::new("Same conversation").unwrap(),
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
            lines: Vec::new(),
            receipts: Vec::new(),
            reads: Default::default(),
            read_operations: Vec::new(),
            gap: false,
        }));
    }
    snapshot
}

fn key(key: Key, pressed: bool, repeat: bool, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat,
        modifiers,
    }
}

struct Frames {
    context: egui::Context,
    model: SharedChat,
    host: HostSnapshot,
    frame: u64,
}

impl Frames {
    fn new() -> Self {
        let context = egui::Context::default();
        crate::theme::Theme {
            panel_alpha: 255,
            ..Default::default()
        }
        .apply(&context, 1.0);
        let mut model = SharedChat::default();
        let host = host();
        model.sync(&host, false);
        model.select(source(1));
        Self {
            context,
            model,
            host,
            frame: 0,
        }
    }

    fn raw(&self, mut events: Vec<Event>) -> RawInput {
        let modifiers = events
            .iter()
            .rev()
            .find_map(|event| match event {
                Event::Key { modifiers, .. } | Event::PointerButton { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        events.insert(0, Event::ModifiersChanged(modifiers));
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(680.0, 680.0))),
            time: Some(self.frame as f64 / 60.0),
            events,
            focused: true,
            ..Default::default()
        }
    }

    fn pass(&mut self, mut raw: RawInput, mode: InputMode) -> Option<SendDraft> {
        self.model.sync(&self.host, false);
        self.frame += 1;
        raw.time = Some(self.frame as f64 / 60.0);
        let palette = crate::theme::Theme {
            panel_alpha: 255,
            ..Default::default()
        }
        .palette();
        let mut mode = mode;
        if mode == InputMode::Live
            && let Some(edits) = edit_prefix(
                &mut raw,
                self.model.composer_rect(&self.context),
                egui::Popup::is_any_open(&self.context),
            )
        {
            let ime = edits.events.iter().any(|event| matches!(event, Event::Ime(_)));
            let mut output = self.context.run_ui(edits, |ui| {
                assert!(self.model.show(ui, &palette, InputMode::Editing).is_none());
            });
            output.textures_delta.clear();
            if ime {
                mode = InputMode::Navigating;
            }
        }
        let mut action = None;
        let mut output = self.context.run_ui(raw, |ui| {
            action = action.take().or(self.model.show(ui, &palette, mode));
        });
        assert!(output.shapes.iter().all(|shape| !matches!(&shape.shape, egui::Shape::Callback(_))));
        output.textures_delta.clear();
        action
    }

    fn show(&mut self, events: Vec<Event>) -> Option<SendDraft> {
        self.pass(self.raw(events), InputMode::Live)
    }

    fn settle(&mut self, frames: usize) {
        for _ in 0..frames {
            assert!(self.show(Vec::new()).is_none());
        }
    }

    fn rect(&mut self, id: Id) -> Rect {
        self.settle(3);
        self.context
            .data(|data| data.get_temp::<Rect>(id))
            .expect("actual shared-chat control")
    }

    fn pointer(&mut self, at: Pos2, pressed: bool) -> Option<SendDraft> {
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

    fn click(&mut self, id: Id) -> Option<SendDraft> {
        let at = self.rect(id).center();
        assert!(self.pointer(at, true).is_none());
        self.pointer(at, false)
    }

    fn choose(&mut self, source: ChatSource) {
        assert!(self.click(Id::unique("shared_chat_combo_rect")).is_none());
        assert!(self.click(Id::unique(("shared_chat_pick", chat_ui::source_id(source)))).is_none());
        assert_eq!(self.model.selected, Some(source));
    }

    fn conversation(&mut self, conversation: Conversation) {
        assert!(self.click(Id::unique("host_chat_combo_rect")).is_none());
        assert!(self.click(Id::unique(("host_chat_pick", conversation))).is_none());
    }

    fn type_text(&mut self, text: &str) {
        assert!(self.click(Id::unique("host_chat_input_rect")).is_none());
        assert!(self.show(vec![Event::Text(text.into())]).is_none());
    }

    fn send(&mut self) -> Option<SendDraft> {
        self.click(Id::unique("host_chat_send_rect"))
    }

    fn reopen(&mut self) {
        self.context = egui::Context::default();
        crate::theme::Theme {
            panel_alpha: 255,
            ..Default::default()
        }
        .apply(&self.context, 1.0);
        self.model.cancel_gestures();
        self.settle(3);
    }

    fn flush(&mut self, events: Vec<Event>) {
        self.model.sync(&self.host, false);
        let mut raw = self.raw(events);
        let Some(composer) = self.model.composer_rect(&self.context) else {
            return;
        };
        if let Some(edits) = edit_prefix(&mut raw, Some(composer), egui::Popup::is_any_open(&self.context)) {
            assert!(self.pass(edits, InputMode::Closing).is_none());
        }
        assert!(self.pass(edit_input(raw), InputMode::Closing).is_none());
    }
}

#[test]
fn shared_chat_five_exact_sources_keep_drafts_and_navigation_across_disposable_contexts() {
    let mut frames = Frames::new();
    for worker in 1..=5 {
        frames.choose(source(worker));
        frames.type_text(&format!("Nearby {worker}"));
        frames.conversation(direct());
        frames.type_text(&format!("IM {worker}"));
    }
    assert_eq!(frames.model.states.len(), MAX_WORKERS);
    frames.reopen();
    for worker in 1..=5 {
        frames.choose(source(worker));
        let draft = frames.send().unwrap();
        assert_eq!(
            draft.key,
            ChatKey {
                source: source(worker),
                conversation: direct()
            }
        );
        assert_eq!(draft.text.as_str(), format!("IM {worker}"));
        frames.conversation(Conversation::Nearby);
        assert_eq!(frames.send().unwrap().text.as_str(), format!("Nearby {worker}"));
    }
}

#[test]
fn shared_chat_world_switches_reordering_and_identical_labels_do_not_select_a_sender() {
    let mut frames = Frames::new();
    frames.choose(source(4));
    frames.type_text("Fourth exact source");
    frames.host.active = Some(source(2).worker.worker);
    frames.host.error = Some("Unrelated presentation failure".into());
    frames.host.workers.reverse();
    frames.host.chat.reverse();
    frames.model.sync(&frames.host, false);
    assert_eq!(frames.model.selected, Some(source(4)));
    assert_eq!(frames.send().unwrap().key.source, source(4));
    assert!(
        frames
            .model
            .states
            .iter()
            .all(|state| state.caption().contains("Same Resident") && state.caption().contains("Same Grid"))
    );
    for worker in 1..=5 {
        frames.choose(source(worker));
    }
}

#[test]
fn shared_chat_send_press_cannot_be_retargeted_by_source_navigation() {
    let mut frames = Frames::new();
    frames.type_text("First draft");
    frames.choose(source(2));
    frames.type_text("Second draft");
    frames.choose(source(1));
    let at = frames.rect(Id::unique("host_chat_send_rect")).center();
    assert!(frames.pointer(at, true).is_none());
    frames.model.select(source(2));
    assert!(frames.pointer(at, false).is_none());
    let draft = frames.send().unwrap();
    assert_eq!(draft.key.source, source(2));
    assert_eq!(draft.text.as_str(), "Second draft");
    frames.choose(source(1));
    assert_eq!(frames.send().unwrap().text.as_str(), "First draft");
}

#[test]
fn shared_chat_pending_admission_and_receipts_update_the_captured_source_only() {
    let mut frames = Frames::new();
    frames.type_text("First send");
    let first = frames.send().unwrap();
    frames.choose(source(2));
    frames.type_text("Second draft");
    frames.model.queued(&first, 42);
    assert_eq!(
        frames.send().unwrap().text.as_str(),
        "Second draft",
        "another source's pending send cannot block this lane"
    );
    Arc::make_mut(&mut frames.host.chat[0]).receipts.push(SendReceipt {
        key: first.key,
        request: 42,
        outcome: SendOutcome::Uncertain,
    });
    frames.model.sync(&frames.host, false);
    frames.choose(source(1));
    assert!(frames.send().is_none(), "uncertainty cannot restore or replay consumed text");
    frames.choose(source(2));
    assert_eq!(frames.send().unwrap().text.as_str(), "Second draft");
}

#[test]
fn shared_chat_closed_model_purges_policy_and_generation_loss_without_falling_back() {
    let mut frames = Frames::new();
    frames.type_text("Old private draft");
    frames.choose(source(2));
    frames.type_text("Unaffected draft");
    frames.choose(source(1));
    Arc::make_mut(&mut frames.host.chat[0]).conversations[0].policy.revision += 1;
    frames.model.sync(&frames.host, false); // no surface pass while closed
    frames.reopen();
    assert!(frames.send().is_none());
    let mut replacement = source(1);
    replacement.session.generation += 1;
    let chat = Arc::make_mut(&mut frames.host.chat[0]);
    chat.source = replacement;
    for info in &mut chat.conversations {
        info.policy.key.source = replacement;
    }
    frames.host.workers[0].binding = replacement.binding();
    frames.model.sync(&frames.host, false);
    assert_eq!(frames.model.selected, Some(source(1)), "revoked sender remains unavailable");
    assert!(frames.show(Vec::new()).is_none());
    assert!(!frames.model.states.iter().any(|state| state.ui.source() == source(1)));
    frames.choose(source(2));
    assert_eq!(frames.send().unwrap().text.as_str(), "Unaffected draft");
    frames.choose(replacement);
    assert!(frames.send().is_none());
    frames.model.sync(&frames.host, true);
    assert!(frames.model.states.is_empty());
}

#[test]
fn shared_chat_close_drains_delivered_text_but_never_a_queued_send() {
    let mut frames = Frames::new();
    frames.type_text("Before close");
    let at = frames.rect(Id::unique("host_chat_send_rect")).center();
    frames.flush(vec![
        Event::Text(" plus queued edit".into()),
        key(Key::Enter, true, false, Modifiers::NONE),
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
        Event::WindowFocused(false),
    ]);
    frames.reopen();
    assert_eq!(frames.send().unwrap().text.as_str(), "Before close plus queued edit");
}

#[test]
fn shared_chat_close_drops_destructive_keys_after_undrawn_source_navigation() {
    let command = Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    for (destructive, modifiers) in [(Key::Backspace, Modifiers::NONE), (Key::Delete, Modifiers::NONE), (Key::Z, command)] {
        let mut frames = Frames::new();
        frames.type_text("Keep old");
        frames.settle(65);
        frames.show(vec![Event::Text(" selected draft".into())]);
        frames.settle(65);
        frames.show(vec![key(Key::A, true, false, command)]);
        frames.show(vec![key(Key::A, false, false, command)]);
        assert!(frames.model.composer_rect(&frames.context).is_some());
        let at = frames.rect(Id::unique("shared_chat_combo_rect")).center();
        // None of these queued events receives a normal redraw before Close.
        // Both production flush stages must preserve the old selected text.
        frames.flush(vec![
            Event::PointerMoved(at),
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
            key(destructive, true, false, modifiers),
            key(destructive, false, false, modifiers),
            Event::WindowFocused(false),
        ]);
        assert_eq!(frames.model.selected, Some(source(1)), "Close never commits a source choice");
        frames.reopen();
        assert_eq!(
            frames.send().unwrap().text.as_str(),
            "Keep old selected draft",
            "{destructive:?} after navigation"
        );
    }
}

#[test]
fn shared_chat_close_retains_committed_ime_and_undo_before_navigation_only() {
    let mut frames = Frames::new();
    frames.type_text("Keep checkpoint");
    frames.settle(65);
    frames.show(vec![Event::Text(" remove this edit".into())]);
    frames.settle(65);
    let command = Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    let at = frames.rect(Id::unique("shared_chat_combo_rect")).center();
    frames.flush(vec![
        key(Key::Z, true, false, command),
        key(Key::Z, false, false, command),
        Event::Ime(egui::ImeEvent::Preedit {
            text: "未確定".into(),
            active_range_chars: None,
        }),
        Event::Ime(egui::ImeEvent::Commit("確定".into())),
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        key(Key::Backspace, true, false, Modifiers::NONE),
        key(Key::Backspace, false, false, Modifiers::NONE),
    ]);
    frames.reopen();
    assert_eq!(frames.send().unwrap().text.as_str(), "Keep checkpoint確定");
}

#[test]
fn shared_chat_source_popup_replacement_cancels_its_held_row() {
    let mut frames = Frames::new();
    frames.settle(3);
    frames.click(Id::unique("shared_chat_combo_rect"));
    let at = frames
        .rect(Id::unique(("shared_chat_pick", chat_ui::source_id(source(2)))))
        .center();
    frames.pointer(at, true);
    frames.host.chat.remove(1);
    frames.host.workers.remove(1);
    frames.model.sync(&frames.host, false);
    assert!(frames.pointer(at, false).is_none());
    assert_eq!(frames.model.selected, Some(source(1)));
}

#[test]
fn shared_chat_enter_is_fresh_focus_bound_and_suppressed_by_popup_modal_or_ime() {
    let mut frames = Frames::new();
    frames.type_text("Explicit draft");
    // egui derives repeat from its held-key set, ignoring the raw repeat flag.
    // Begin with a real press rejected by the modal/navigation gate, then hold.
    let raw = frames.raw(vec![key(Key::Enter, true, false, Modifiers::NONE)]);
    assert!(frames.pass(raw, InputMode::Navigating).is_none());
    assert!(frames.show(vec![key(Key::Enter, true, true, Modifiers::NONE)]).is_none());
    frames.show(vec![key(Key::Enter, false, false, Modifiers::NONE)]);
    let raw = frames.raw(vec![key(Key::Enter, true, false, Modifiers::NONE)]);
    assert!(frames.pass(raw, InputMode::Navigating).is_none());
    frames.show(vec![key(Key::Enter, false, false, Modifiers::NONE)]);
    let mut raw = frames.raw(vec![Event::WindowFocused(false), key(Key::Enter, true, false, Modifiers::NONE)]);
    raw.focused = false;
    assert!(frames.pass(raw, InputMode::Live).is_none());
    frames.show(vec![key(Key::Enter, false, false, Modifiers::NONE)]);
    frames.click(Id::unique("host_chat_input_rect"));
    assert!(
        frames
            .show(vec![
                Event::Ime(egui::ImeEvent::Preedit {
                    text: "候補".into(),
                    active_range_chars: None
                }),
                key(Key::Enter, true, false, Modifiers::NONE)
            ])
            .is_none()
    );
    frames.show(vec![key(Key::Enter, false, false, Modifiers::NONE)]);
    assert!(
        frames
            .show(vec![
                Event::Ime(egui::ImeEvent::Commit("確定".into())),
                key(Key::Enter, true, false, Modifiers::NONE)
            ])
            .is_none()
    );
    frames.show(vec![key(Key::Enter, false, false, Modifiers::NONE)]);
    frames.click(Id::unique("host_chat_combo_rect"));
    assert!(frames.show(vec![key(Key::Enter, true, false, Modifiers::NONE)]).is_none());
    frames.show(vec![key(Key::Enter, false, false, Modifiers::NONE)]);
    egui::Popup::close_all(&frames.context);
    frames.click(Id::unique("host_chat_input_rect"));
    assert!(frames.show(vec![key(Key::Enter, true, false, Modifiers::NONE)]).is_some());
}

#[test]
fn shared_chat_native_enter_repeat_remains_suppressed_after_focus_loss() {
    use winit::event::ElementState;
    use winit::keyboard::{Key as NativeKey, ModifiersState, NamedKey};

    let mut frames = Frames::new();
    frames.type_text("Held across focus loss");
    let native_enter = |pressed, repeat, shift| {
        let state = if pressed { ElementState::Pressed } else { ElementState::Released };
        let modifiers = if shift { ModifiersState::SHIFT } else { ModifiersState::empty() };
        // Use the production native boundary before egui-winit's conversion,
        // which always delivers repeat:false and lets egui infer held keys.
        if suppress_enter_repeat(&NativeKey::Named(NamedKey::Enter), state, repeat, modifiers) {
            Vec::new()
        } else {
            vec![key(
                Key::Enter,
                pressed,
                false,
                if shift { Modifiers::SHIFT } else { Modifiers::NONE },
            )]
        }
    };
    assert!(
        frames.show(native_enter(true, true, false)).is_none(),
        "a surface may first observe an OS repeat"
    );
    assert!(frames.show(native_enter(true, false, true)).is_none());
    assert!(frames.show(native_enter(true, true, true)).is_none());
    frames.show(native_enter(false, false, true));
    frames.show(vec![Event::Text("Still here".into())]);
    let raw = frames.raw(native_enter(true, false, false));
    assert!(frames.pass(raw, InputMode::Navigating).is_none());
    let mut raw = frames.raw(vec![Event::WindowFocused(false)]);
    raw.focused = false;
    assert!(frames.pass(raw, InputMode::Live).is_none());
    frames.show(vec![Event::WindowFocused(true)]);
    frames.click(Id::unique("host_chat_input_rect"));
    assert!(
        frames.show(native_enter(true, true, false)).is_none(),
        "focus loss cleared egui's held-key set"
    );
    frames.show(native_enter(false, false, false));
    assert_eq!(
        frames.show(native_enter(true, false, false)).unwrap().text.as_str(),
        "Held across focus loss\n\nStill here"
    );
}

#[test]
fn shared_chat_delivered_text_and_paste_precede_source_and_conversation_navigation() {
    let mut frames = Frames::new();
    frames.type_text("First");
    let at = frames.rect(Id::unique("shared_chat_combo_rect")).center();
    assert!(
        frames
            .show(vec![
                Event::Text(" text".into()),
                Event::Paste(" paste".into()),
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE
                },
            ])
            .is_none()
    );
    frames.pointer(at, false);
    frames.click(Id::unique(("shared_chat_pick", chat_ui::source_id(source(2)))));
    frames.type_text("Second");
    assert_eq!(frames.send().unwrap().text.as_str(), "Second");
    frames.choose(source(1));
    assert_eq!(frames.send().unwrap().text.as_str(), "First text paste");
    frames.click(Id::unique("host_chat_input_rect"));
    frames.show(vec![key(Key::End, true, false, Modifiers::NONE)]);
    frames.show(vec![key(Key::End, false, false, Modifiers::NONE)]);
    let at = frames.rect(Id::unique("host_chat_combo_rect")).center();
    assert!(
        frames
            .show(vec![
                Event::Paste(" before IM".into()),
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE
                },
            ])
            .is_none()
    );
    frames.pointer(at, false);
    frames.click(Id::unique(("host_chat_pick", direct())));
    frames.type_text("Resident draft");
    frames.conversation(Conversation::Nearby);
    assert_eq!(frames.send().unwrap().text.as_str(), "First text paste before IM");
    frames.conversation(direct());
    assert_eq!(frames.send().unwrap().text.as_str(), "Resident draft");
}

#[test]
fn shared_chat_undo_survives_source_navigation_and_a_new_surface_context() {
    let mut frames = Frames::new();
    frames.type_text("First checkpoint");
    frames.settle(65);
    frames.show(vec![Event::Text(" plus edit".into())]);
    frames.settle(65);
    frames.choose(source(2));
    frames.type_text("Second checkpoint");
    frames.reopen();
    frames.choose(source(1));
    frames.click(Id::unique("host_chat_input_rect"));
    let command = Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    let mut restored = String::new();
    for _ in 0..3 {
        frames.show(vec![key(Key::Z, true, false, command)]);
        frames.show(vec![key(Key::Z, false, false, command)]);
        restored = frames.send().map(|draft| draft.text.as_str().to_owned()).unwrap_or_default();
        if restored == "First checkpoint" {
            break;
        }
        frames.click(Id::unique("host_chat_input_rect"));
    }
    assert_eq!(restored, "First checkpoint");
    frames.choose(source(2));
    assert_eq!(frames.send().unwrap().text.as_str(), "Second checkpoint");
}

#[test]
fn shared_chat_shift_enter_inserts_a_line_and_a_rejected_held_enter_never_retries() {
    let mut frames = Frames::new();
    frames.type_text("First line");
    assert!(frames.show(vec![key(Key::Enter, true, false, Modifiers::SHIFT)]).is_none());
    frames.show(vec![key(Key::Enter, false, false, Modifiers::SHIFT)]);
    frames.show(vec![Event::Text("Second line".into())]);
    let sent = frames.show(vec![key(Key::Enter, true, false, Modifiers::NONE)]).unwrap();
    assert_eq!(sent.text.as_str(), "First line\nSecond line");
    frames.model.queue_failed(&sent, HostError::Busy);
    assert!(frames.show(vec![key(Key::Enter, true, true, Modifiers::NONE)]).is_none());
    frames.show(vec![key(Key::Enter, false, false, Modifiers::NONE)]);
    let command = Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    assert_eq!(frames.show(vec![key(Key::Enter, true, false, command)]).unwrap().text, sent.text);
}

#[test]
fn shared_chat_cut_and_unfinished_ime_cannot_delete_or_send_during_close_flush() {
    let mut frames = Frames::new();
    frames.type_text("Keep selected text");
    let command = Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    frames.show(vec![key(Key::A, true, false, command)]);
    frames.show(vec![key(Key::A, false, false, command)]);
    frames.flush(vec![
        Event::Cut,
        Event::Ime(egui::ImeEvent::Preedit {
            text: "未確定".into(),
            active_range_chars: None,
        }),
        key(Key::Enter, true, false, Modifiers::NONE),
    ]);
    frames.reopen();
    assert_eq!(frames.send().unwrap().text.as_str(), "Keep selected text");
}

#[test]
fn shared_chat_label_change_cancels_a_send_press_without_rekeying_its_draft() {
    let mut frames = Frames::new();
    frames.type_text("Same immutable sender");
    let at = frames.rect(Id::unique("host_chat_send_rect")).center();
    frames.pointer(at, true);
    frames.host.slots[0].label = SlotLabel::new("Renamed display label").unwrap();
    frames.model.sync(&frames.host, false);
    assert!(frames.pointer(at, false).is_none());
    let draft = frames.send().unwrap();
    assert_eq!(draft.key.source, source(1));
    assert_eq!(draft.text.as_str(), "Same immutable sender");
}

#[test]
fn shared_chat_same_source_auth_restoration_cannot_reuse_revoked_editor_focus_or_undo() {
    let mut frames = Frames::new();
    frames.type_text("Private revoked draft");
    assert!(frames.model.composer_rect(&frames.context).is_some());
    Arc::make_mut(&mut frames.host.chat[0]).authenticated = false;
    frames.model.sync(&frames.host, false); // retain the actual old context/focus
    Arc::make_mut(&mut frames.host.chat[0]).authenticated = true;
    frames.model.sync(&frames.host, false); // same source and identical policy
    let command = Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    assert!(
        frames
            .show(vec![
                Event::Text("stale typing".into()),
                Event::Paste("stale paste".into()),
                key(Key::Z, true, false, command)
            ])
            .is_none()
    );
    frames.show(vec![key(Key::Z, false, false, command)]);
    assert!(frames.send().is_none(), "revoked focus must not authorize a new composer");
    frames.type_text("Fresh explicit draft");
    assert_eq!(frames.send().unwrap().text.as_str(), "Fresh explicit draft");
}

#[test]
fn shared_chat_navigation_drops_ambiguous_trailing_text_and_enter_in_the_same_batch() {
    let mut frames = Frames::new();
    frames.type_text("First draft");
    frames.click(Id::unique("shared_chat_combo_rect"));
    let at = frames
        .rect(Id::unique(("shared_chat_pick", chat_ui::source_id(source(2)))))
        .center();
    assert!(
        frames
            .show(vec![
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE
                },
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE
                },
                Event::Text("ambiguous typing".into()),
                Event::Paste("ambiguous paste".into()),
                key(Key::Enter, true, false, Modifiers::NONE),
            ])
            .is_none()
    );
    frames.show(vec![key(Key::Enter, false, false, Modifiers::NONE)]);
    assert_eq!(frames.model.selected, Some(source(2)));
    assert!(frames.send().is_none());
    frames.choose(source(1));
    assert_eq!(frames.send().unwrap().text.as_str(), "First draft");
}
