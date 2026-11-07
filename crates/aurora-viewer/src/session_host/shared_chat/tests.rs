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

fn install_read(frames: &mut Frames, incoming: u64) {
    use aurora_session::chat::{ObservedIncoming, ReadMetadata};
    Arc::make_mut(&mut frames.host.chat[0]).reads.insert(
        Conversation::Nearby,
        ReadMetadata {
            unread: incoming,
            unread_revision: incoming + 1,
            observed: ObservedIncoming {
                epoch: 1,
                through: incoming,
            },
            can_mark_read: true,
        },
    );
}

/// Two real egui contexts borrow the production source model. Only the active
/// native-style context is focused; passive rendering must never take its Undo.
struct SurfaceFrames {
    contexts: [egui::Context; 2],
    model: SharedChat,
    host: HostSnapshot,
    active: usize,
    frame: u64,
}

impl SurfaceFrames {
    const SURFACES: [ChatSurface; 2] = [ChatSurface::Shared, ChatSurface::Monitor1];

    fn new() -> Self {
        let base = Frames::new();
        let contexts = std::array::from_fn(|_| {
            let context = egui::Context::default();
            crate::theme::Theme {
                panel_alpha: 255,
                ..Default::default()
            }
            .apply(&context, 1.0);
            context
        });
        let mut result = Self {
            contexts,
            model: base.model,
            host: base.host,
            active: 0,
            frame: 0,
        };
        result.model.select_for(ChatSurface::Monitor1, source(1));
        result
    }

    fn show(&mut self, index: usize, events: Vec<Event>) -> Option<SendDraft> {
        self.frame += 1;
        self.model.sync(&self.host, false);
        let surface = Self::SURFACES[index];
        let modifiers = events
            .iter()
            .rev()
            .find_map(|event| match event {
                Event::Key { modifiers, .. } | Event::PointerButton { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let mut raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(680.0, 680.0))),
            time: Some(self.frame as f64 / 60.0),
            focused: self.active == index,
            events: std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect(),
            ..Default::default()
        };
        let context = &self.contexts[index];
        let palette = crate::theme::Theme::default().palette();
        if let Some(edits) = edit_prefix(
            &mut raw,
            self.model.composer_rect_for(surface, context),
            egui::Popup::is_any_open(context),
        ) {
            let mut output = context.run_ui(edits, |ui| {
                assert!(self.model.show_for(surface, ui, &palette, InputMode::Editing).is_none());
            });
            output.textures_delta.clear();
        }
        let mut action = None;
        let mut output = context.run_ui(raw, |ui| {
            action = action.take().or(self.model.show_for(surface, ui, &palette, InputMode::Live));
        });
        assert!(output.shapes.iter().all(|shape| !matches!(&shape.shape, egui::Shape::Callback(_))));
        output.textures_delta.clear();
        action
    }

    /// Exercise the production body without the native edit-prefix split when
    /// a regression specifically concerns multiple IME events in one pass.
    fn raw_show(&mut self, index: usize, events: Vec<Event>) {
        self.model.sync(&self.host, false);
        self.frame += 1;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(680.0, 680.0))),
            time: Some(self.frame as f64 / 60.0),
            focused: self.active == index,
            events,
            ..Default::default()
        };
        let palette = crate::theme::Theme::default().palette();
        let mut output = self.contexts[index].run_ui(raw, |ui| {
            assert!(self.model.show_for(Self::SURFACES[index], ui, &palette, InputMode::Live).is_none());
        });
        output.textures_delta.clear();
    }

    fn settle(&mut self, index: usize, count: usize) {
        for _ in 0..count {
            assert!(self.show(index, Vec::new()).is_none());
        }
    }

    fn focus(&mut self, index: usize) {
        if self.active != index {
            let old = self.active;
            self.active = index;
            assert!(self.show(old, vec![Event::WindowFocused(false)]).is_none());
            assert!(self.show(index, vec![Event::WindowFocused(true)]).is_none());
        }
        self.settle(index, 3);
    }

    fn rect(&mut self, index: usize, name: &'static str) -> Rect {
        self.settle(index, 3);
        self.contexts[index]
            .data(|data| data.get_temp::<Rect>(Id::unique(name)))
            .expect("production control")
    }

    fn pointer(&mut self, index: usize, at: Pos2, pressed: bool) -> Option<SendDraft> {
        self.show(
            index,
            vec![
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        )
    }

    fn click(&mut self, index: usize, name: &'static str) -> Option<SendDraft> {
        self.focus(index);
        let at = self.rect(index, name).center();
        assert!(self.pointer(index, at, true).is_none());
        self.pointer(index, at, false)
    }

    fn type_text(&mut self, index: usize, text: &str) {
        assert!(self.click(index, "host_chat_input_rect").is_none());
        assert!(self.show(index, vec![Event::Text(text.into())]).is_none());
    }

    fn draft(&self, index: usize, source: ChatSource) -> &str {
        self.model
            .states
            .iter()
            .find(|state| state.ui.source() == source)
            .unwrap()
            .ui
            .retained_for(Self::SURFACES[index])
            .1
    }
}

#[test]
fn shared_chat_two_contexts_keep_source_conversation_and_reading_selection_independent() {
    let mut frames = SurfaceFrames::new();
    let direct_key = ChatKey {
        source: source(1),
        conversation: direct(),
    };
    assert!(frames.model.select_key_for(ChatSurface::Monitor1, direct_key));
    frames.type_text(0, "Shared Nearby");
    frames.type_text(1, "Monitor Direct");
    assert_eq!(frames.draft(0, source(1)), "Shared Nearby");
    assert_eq!(frames.draft(1, source(1)), "Monitor Direct");
    assert_eq!(frames.model.selected, Some(source(1)));
    let state = frames.model.states.iter().find(|state| state.ui.source() == source(1)).unwrap();
    assert_eq!(state.ui.retained_for(ChatSurface::Shared).0.conversation, Conversation::Nearby);
    assert_eq!(state.ui.retained_for(ChatSurface::Monitor1).0, direct_key);
    assert!(state.ui.retained_for(ChatSurface::Shared).2 > 0);
    assert!(state.ui.retained_for(ChatSurface::Monitor1).2 > 0);
    frames.model.select_for(ChatSurface::Monitor1, source(2));
    frames.type_text(1, "Second duplicate-labelled source");
    assert_eq!(frames.model.selected, Some(source(1)));
    assert_eq!(frames.draft(0, source(1)), "Shared Nearby");
    let sent = frames.click(1, "host_chat_send_rect").unwrap();
    assert_eq!(sent.key.source, source(2));
    assert_eq!(sent.text.as_str(), "Second duplicate-labelled source");
    assert!(frames.model.take_read_action().is_none());
    assert!(frames.model.take_read_action_for(ChatSurface::Monitor1).is_none());
    assert!(frames.host.active.is_none(), "view selection creates no activation intent");
}

#[test]
fn shared_chat_two_contexts_share_one_undo_and_passive_or_stale_input_cannot_replace_it() {
    let mut frames = SurfaceFrames::new();
    frames.type_text(0, "Canonical base");
    frames.settle(0, 70);
    assert!(frames.show(0, vec![Event::Text(" plus edit".into())]).is_none());
    frames.settle(0, 70);
    frames.settle(1, 3);
    assert_eq!(frames.draft(1, source(1)), "Canonical base plus edit");
    assert!(frames.click(1, "host_chat_input_rect").is_none());
    assert!(frames.show(1, vec![key(Key::Z, true, false, Modifiers::CTRL)]).is_none());
    frames.show(1, vec![key(Key::Z, false, false, Modifiers::CTRL)]);
    assert_eq!(
        frames.draft(0, source(1)),
        "Canonical base",
        "Undo came from the other context's canonical history"
    );
    assert_eq!(frames.draft(1, source(1)), "Canonical base");
    frames.active = 0; // A delayed formerly focused context has not earned a new composer click.
    assert!(
        frames
            .show(
                0,
                vec![
                    Event::Text(" STALE".into()),
                    Event::Ime(egui::ImeEvent::Commit("STALE IME".into())),
                    key(Key::Enter, true, false, Modifiers::NONE)
                ]
            )
            .is_none()
    );
    assert_eq!(frames.draft(1, source(1)), "Canonical base");
    for (index, surface) in SurfaceFrames::SURFACES.into_iter().enumerate() {
        let editor = frames.model.states[0].ui.retained_for(surface).3;
        assert!(
            egui::TextEdit::load_state(&frames.contexts[index], editor).is_none(),
            "a context cannot retain its own persisted Undo authority"
        );
    }
}

#[test]
fn shared_chat_two_contexts_keep_late_send_and_read_completion_on_their_captured_key() {
    use crate::session_host::chat_broker::{ReadOperation, ReadStatus};
    use aurora_session::chat::{ObservedIncoming, ReadMetadata, ReadOutcome};
    let mut frames = SurfaceFrames::new();
    Arc::make_mut(&mut frames.host.chat[0]).reads.insert(
        Conversation::Nearby,
        ReadMetadata {
            unread: 2,
            unread_revision: 3,
            observed: ObservedIncoming { epoch: 1, through: 2 },
            can_mark_read: true,
        },
    );
    frames.type_text(0, "Captured old text");
    let sent = frames.click(0, "host_chat_send_rect").unwrap();
    assert!(frames.click(0, "host_chat_read_rect").is_none());
    let read = frames.model.take_read_action().unwrap();
    let target = ChatKey {
        source: source(1),
        conversation: direct(),
    };
    assert!(frames.model.select_key_for(ChatSurface::Monitor1, target));
    frames.type_text(1, "Keep Direct draft");
    frames.model.queued(&sent, 81);
    frames.model.queued_read(read, 82);
    assert!(
        frames.click(1, "host_chat_send_rect").is_none(),
        "source's pending operation is canonical"
    );
    let chat = Arc::make_mut(&mut frames.host.chat[0]);
    chat.receipts.push(SendReceipt {
        key: sent.key,
        request: 81,
        outcome: SendOutcome::AcceptedLocally,
    });
    chat.read_operations.push(ReadOperation {
        request: read.request(82),
        status: ReadStatus::Completed(ReadOutcome::Applied),
    });
    frames.model.sync(&frames.host, false);
    frames.settle(0, 2);
    frames.settle(1, 2);
    let operations = frames.model.states[0].ui.operations();
    assert_eq!(operations.0.unwrap().key, sent.key);
    assert_eq!(operations.1.unwrap().request, read.request(82));
    assert_eq!(frames.draft(0, source(1)), "");
    assert_eq!(frames.draft(1, source(1)), "Keep Direct draft");
    let next = frames.click(1, "host_chat_send_rect").unwrap();
    assert_eq!(next.key, target);
    assert_eq!(next.text.as_str(), "Keep Direct draft");
}

#[test]
fn shared_chat_hidden_policy_loss_and_reopen_purge_both_context_authorities() {
    let mut frames = SurfaceFrames::new();
    frames.type_text(0, "PRIVATE canonical draft");
    frames.type_text(1, " plus second surface");
    let old_ids = [ChatSurface::Shared, ChatSurface::Monitor1].map(|surface| frames.model.states[0].ui.retained_for(surface).3);
    // No repaint occurs while the source's policy is revoked then restored.
    for info in &mut Arc::make_mut(&mut frames.host.chat[0]).conversations {
        info.policy.revision += 1;
    }
    frames.model.sync(&frames.host, false);
    assert_eq!(frames.draft(0, source(1)), "");
    assert_eq!(frames.draft(1, source(1)), "");
    for (index, surface) in [ChatSurface::Shared, ChatSurface::Monitor1].into_iter().enumerate() {
        assert_ne!(frames.model.states[0].ui.retained_for(surface).3, old_ids[index]);
        assert!(
            frames
                .show(
                    index,
                    vec![
                        key(Key::Z, true, false, Modifiers::CTRL),
                        Event::Text("OLD".into()),
                        key(Key::Enter, true, false, Modifiers::NONE)
                    ]
                )
                .is_none()
        );
        assert_eq!(frames.draft(index, source(1)), "");
    }
    frames.model.surface_closed(ChatSurface::Monitor1);
    let closed_id = frames.model.states[0].ui.retained_for(ChatSurface::Monitor1).3;
    assert_ne!(closed_id, old_ids[1]);
    frames.contexts[1] = egui::Context::default();
    assert!(frames.show(1, vec![key(Key::Enter, true, false, Modifiers::NONE)]).is_none());
    Arc::make_mut(&mut frames.host.chat[0]).authenticated = false;
    frames.model.sync(&frames.host, false);
    assert!(!frames.model.states.iter().any(|state| state.ui.source() == source(1)));
    assert_eq!(frames.model.selected, Some(source(1)), "no replacement sender is selected");
}

#[test]
fn shared_chat_two_contexts_keep_actual_scrolled_reading_anchors_independent() {
    use aurora_session::chat::{ChatLine, ChatText, LineKind};
    let mut frames = SurfaceFrames::new();
    let chat = Arc::make_mut(&mut frames.host.chat[0]);
    let policy = chat.conversations[0].policy;
    chat.lines = (1..=100)
        .map(|event| ChatLine {
            key: policy.key,
            policy_revision: policy.revision,
            event,
            sender: Label::new("Synthetic sender").unwrap(),
            text: ChatText::new(&format!(
                "Synthetic line {event} with enough content to scroll the production transcript."
            ))
            .unwrap(),
            kind: LineKind::Incoming,
            truncated: false,
        })
        .collect();
    frames.settle(0, 5);
    frames.settle(1, 5);
    let before = frames.model.states[0].ui.reading_for(ChatSurface::Monitor1).unwrap();
    assert!(before.0 > 200.0 && before.1, "fixture is at the transcript tail");
    frames.focus(0);
    let at = frames.rect(0, "host_chat_lines_rect").center();
    assert!(
        frames
            .show(
                0,
                vec![
                    Event::PointerMoved(at),
                    Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        source: egui::MouseWheelSource::Unknown,
                        delta: vec2(0.0, 500.0),
                        modifiers: Modifiers::NONE,
                    }
                ]
            )
            .is_none()
    );
    frames.settle(0, 8);
    let scrolled = frames.model.states[0].ui.reading_for(ChatSurface::Shared).unwrap();
    assert!(scrolled.0 < before.0 - 100.0 && !scrolled.1);
    frames.settle(1, 5);
    let other = frames.model.states[0].ui.reading_for(ChatSurface::Monitor1).unwrap();
    assert!((other.0 - before.0).abs() < 1.0 && other.1, "another surface kept its own tail");
    assert!(frames.model.take_read_action().is_none());
    assert!(frames.model.take_read_action_for(ChatSurface::Monitor1).is_none());
}

#[test]
fn shared_chat_monitor_pressed_send_and_queued_ime_cannot_retarget_after_selection_or_close() {
    let mut frames = SurfaceFrames::new();
    frames.type_text(1, "First monitor draft");
    frames.model.select_for(ChatSurface::Monitor1, source(2));
    frames.type_text(1, "Second monitor draft");
    frames.model.select_for(ChatSurface::Monitor1, source(1));
    frames.focus(1);
    let at = frames.rect(1, "host_chat_send_rect").center();
    assert!(frames.pointer(1, at, true).is_none());
    frames.model.select_for(ChatSurface::Monitor1, source(2));
    assert!(
        frames
            .show(
                1,
                vec![
                    Event::PointerButton {
                        pos: at,
                        button: PointerButton::Primary,
                        pressed: false,
                        modifiers: Modifiers::NONE,
                    },
                    Event::Ime(egui::ImeEvent::Commit("OLD SOURCE COMMIT".into())),
                    key(Key::Enter, true, false, Modifiers::NONE)
                ]
            )
            .is_none()
    );
    assert_eq!(frames.draft(1, source(1)), "First monitor draft");
    assert_eq!(frames.draft(1, source(2)), "Second monitor draft");
    assert_eq!(frames.model.selected, Some(source(1)), "Shared sender was not cycled");
    assert!(frames.click(1, "host_chat_input_rect").is_none());
    frames.model.surface_closed(ChatSurface::Monitor1);
    assert!(
        frames
            .show(
                1,
                vec![Event::Text("CLOSED CONTEXT".into()), key(Key::Enter, true, false, Modifiers::NONE)]
            )
            .is_none()
    );
    assert_eq!(frames.draft(1, source(2)), "Second monitor draft");
}

#[test]
fn shared_chat_late_admission_does_not_consume_newer_same_text_after_another_surfaces_undo() {
    let mut frames = SurfaceFrames::new();
    frames.type_text(0, "Same final text");
    frames.settle(0, 70);
    let earlier = frames.click(0, "host_chat_send_rect").unwrap();
    frames.type_text(1, " plus a later edit");
    frames.settle(1, 70);
    assert!(frames.show(1, vec![key(Key::Z, true, false, Modifiers::CTRL)]).is_none());
    frames.show(1, vec![key(Key::Z, false, false, Modifiers::CTRL)]);
    assert_eq!(frames.draft(1, source(1)), earlier.text.as_str());
    frames.model.queued(&earlier, 99);
    assert_eq!(
        frames.draft(0, source(1)),
        "Same final text",
        "the captured operation cannot consume a newer canonical edit revision"
    );
}

#[test]
fn shared_chat_late_admission_cannot_cross_exact_source_reauthentication_or_clear_in_two_contexts() {
    for clear in [false, true] {
        let mut frames = SurfaceFrames::new();
        Arc::make_mut(&mut frames.host.chat[0]).reads.insert(
            Conversation::Nearby,
            aurora_session::chat::ReadMetadata {
                unread: 1,
                unread_revision: 1,
                observed: aurora_session::chat::ObservedIncoming { epoch: 1, through: 1 },
                can_mark_read: true,
            },
        );
        frames.type_text(0, "Identical recreated text");
        assert!(frames.click(0, "host_chat_read_rect").is_none());
        let old_read = frames.model.take_read_action().unwrap();
        let old = frames.click(0, "host_chat_send_rect").unwrap();
        let old_ids = SurfaceFrames::SURFACES.map(|surface| frames.model.states[0].ui.retained_for(surface).3);
        frames.model.surface_closed(ChatSurface::Shared);
        frames.model.surface_closed(ChatSurface::Monitor1);
        if clear {
            frames.model.clear();
        } else {
            Arc::make_mut(&mut frames.host.chat[0]).authenticated = false;
            frames.model.sync(&frames.host, false);
            assert!(!frames.model.states.iter().any(|state| state.ui.source() == old.key.source));
            Arc::make_mut(&mut frames.host.chat[0]).authenticated = true;
        }
        frames.model.sync(&frames.host, false);
        // Exact source, policy, conversation and local edit count all repeat.
        // The source model allocation is the only distinguishing authority.
        frames.model.select_for(ChatSurface::Shared, old.key.source);
        frames.model.select_for(ChatSurface::Monitor1, old.key.source);
        frames.type_text(1, old.text.as_str());
        let new = frames.click(1, "host_chat_send_rect").unwrap();
        assert_eq!(new.key, old.key);
        assert_eq!(new.captured_policy(), old.captured_policy());
        assert_eq!(new.text, old.text);
        frames.model.queued(&old, 90);
        frames.model.queue_failed(&old, HostError::Busy);
        frames.model.queued_read(old_read, 92);
        frames.model.read_queue_failed(old_read);
        for (index, surface) in SurfaceFrames::SURFACES.into_iter().enumerate() {
            frames.settle(index, 3);
            assert_eq!(frames.draft(index, old.key.source), old.text.as_str());
            let state = frames
                .model
                .states
                .iter()
                .find(|state| state.ui.source() == old.key.source)
                .unwrap();
            assert_ne!(state.ui.retained_for(surface).3, old_ids[index]);
            assert!(
                state.ui.operations().0.is_none(),
                "old admission cannot install new pending authority"
            );
            assert!(
                state.ui.operations().1.is_none(),
                "old Read callbacks cannot install new pending/failure authority"
            );
        }
        frames.model.queued(&new, 91);
        assert_eq!(
            frames.draft(0, old.key.source),
            "",
            "the newly captured admission still consumes its draft"
        );
    }
}

#[test]
fn shared_chat_uncommitted_ime_rolls_back_on_two_context_focus_or_surface_close_without_polluting_undo() {
    for close in [false, true] {
        let mut frames = SurfaceFrames::new();
        frames.type_text(0, "Committed prefix");
        frames.settle(0, 70);
        frames.type_text(0, " plus stable edit");
        frames.settle(0, 70);
        assert!(
            frames
                .show(
                    0,
                    vec![Event::Ime(egui::ImeEvent::Preedit {
                        text: "UNCOMMITTED".into(),
                        active_range_chars: None,
                    })]
                )
                .is_none()
        );
        assert!(frames.draft(0, source(1)).contains("UNCOMMITTED"));
        assert_eq!(
            frames.draft(1, source(1)),
            "Committed prefix plus stable edit",
            "another surface only sees committed text"
        );
        if close {
            let palette = crate::theme::Theme::default().palette();
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(680.0, 680.0))),
                time: Some(20.0),
                focused: true,
                ..Default::default()
            };
            let mut output = frames.contexts[0].run_ui(raw, |ui| {
                assert!(
                    frames
                        .model
                        .show_for(ChatSurface::Shared, ui, &palette, InputMode::Closing)
                        .is_none()
                );
            });
            output.textures_delta.clear();
            frames.model.surface_closed(ChatSurface::Shared);
        }
        frames.focus(1);
        assert_eq!(
            frames.click(1, "host_chat_send_rect").unwrap().text.as_str(),
            "Committed prefix plus stable edit"
        );
        assert!(frames.click(1, "host_chat_input_rect").is_none());
        assert!(frames.show(1, vec![key(Key::Z, true, false, Modifiers::CTRL)]).is_none());
        frames.show(1, vec![key(Key::Z, false, false, Modifiers::CTRL)]);
        assert_eq!(
            frames.draft(1, source(1)),
            "Committed prefix",
            "Undo retains the earlier committed checkpoint"
        );
        let redo = Modifiers {
            ctrl: true,
            command: true,
            shift: true,
            ..Default::default()
        };
        assert!(frames.show(1, vec![key(Key::Z, true, false, redo)]).is_none());
        frames.show(1, vec![key(Key::Z, false, false, redo)]);
        assert_eq!(
            frames.draft(1, source(1)),
            "Committed prefix plus stable edit",
            "Redo restores committed text without the abandoned candidate"
        );
    }
}

#[test]
fn shared_chat_one_native_batch_can_commit_then_preedit_without_lending_the_next_candidate() {
    for split_prefix in [false, true] {
        let mut frames = SurfaceFrames::new();
        frames.type_text(0, "Prefix ");
        frames.show(
            0,
            vec![Event::Ime(egui::ImeEvent::Preedit {
                text: "first candidate".into(),
                active_range_chars: None,
            })],
        );
        let events = vec![
            Event::Ime(egui::ImeEvent::Commit("committed".into())),
            Event::Ime(egui::ImeEvent::Preedit {
                text: "NEXT CANDIDATE".into(),
                active_range_chars: None,
            }),
        ];
        if split_prefix {
            assert!(frames.show(0, events).is_none());
        } else {
            frames.raw_show(0, events);
        }
        frames.focus(1);
        assert_eq!(frames.click(1, "host_chat_send_rect").unwrap().text.as_str(), "Prefix committed");
    }
}

#[test]
fn shared_chat_same_pass_preedit_cancel_does_not_leave_a_checkpoint_that_can_discard_later_typing() {
    for disabled in [false, true] {
        let mut frames = SurfaceFrames::new();
        frames.type_text(0, "Prefix");
        #[allow(deprecated)]
        let ending = if disabled {
            egui::ImeEvent::Disabled
        } else {
            egui::ImeEvent::Preedit {
                text: String::new(),
                active_range_chars: None,
            }
        };
        frames.raw_show(
            0,
            vec![
                Event::Ime(egui::ImeEvent::Preedit {
                    text: "CANCELLED".into(),
                    active_range_chars: None,
                }),
                Event::Ime(ending),
            ],
        );
        assert_eq!(frames.draft(0, source(1)), "Prefix");
        assert!(frames.show(0, vec![Event::Text(" delivered".into())]).is_none());
        frames.focus(1);
        assert_eq!(frames.click(1, "host_chat_send_rect").unwrap().text.as_str(), "Prefix delivered");
    }
}

#[test]
fn shared_chat_same_pass_commit_then_cancel_next_preedit_preserves_the_actual_commit() {
    for disabled in [false, true] {
        let mut frames = SurfaceFrames::new();
        frames.type_text(0, "Prefix ");
        frames.show(
            0,
            vec![Event::Ime(egui::ImeEvent::Preedit {
                text: "first candidate".into(),
                active_range_chars: None,
            })],
        );
        #[allow(deprecated)]
        let ending = if disabled {
            egui::ImeEvent::Disabled
        } else {
            egui::ImeEvent::Preedit {
                text: String::new(),
                active_range_chars: None,
            }
        };
        frames.raw_show(
            0,
            vec![
                Event::Ime(egui::ImeEvent::Commit("committed".into())),
                Event::Ime(egui::ImeEvent::Preedit {
                    text: "CANCELLED NEXT".into(),
                    active_range_chars: None,
                }),
                Event::Ime(ending),
            ],
        );
        frames.focus(1);
        assert_eq!(frames.click(1, "host_chat_send_rect").unwrap().text.as_str(), "Prefix committed");
    }
}

#[test]
fn shared_chat_true_ime_commit_is_shared_but_closed_views_late_ime_cannot_retarget() {
    let mut frames = SurfaceFrames::new();
    frames.type_text(0, "Prefix ");
    assert!(
        frames
            .show(
                0,
                vec![Event::Ime(egui::ImeEvent::Preedit {
                    text: "provisional".into(),
                    active_range_chars: None,
                })]
            )
            .is_none()
    );
    assert!(
        frames
            .show(0, vec![Event::Ime(egui::ImeEvent::Commit("committed".into()))])
            .is_none()
    );
    frames.focus(1);
    assert_eq!(frames.click(1, "host_chat_send_rect").unwrap().text.as_str(), "Prefix committed");
    frames.focus(0);
    frames.click(0, "host_chat_input_rect");
    assert!(
        frames
            .show(
                0,
                vec![Event::Ime(egui::ImeEvent::Preedit {
                    text: " abandoned".into(),
                    active_range_chars: None,
                })]
            )
            .is_none()
    );
    frames.model.surface_closed(ChatSurface::Shared);
    frames.model.select_for(ChatSurface::Shared, source(2));
    assert!(
        frames
            .show(
                0,
                vec![
                    Event::Ime(egui::ImeEvent::Commit("LATE OLD COMMIT".into())),
                    Event::Ime(egui::ImeEvent::Preedit {
                        text: "LATE OLD PREEDIT".into(),
                        active_range_chars: None
                    }),
                ]
            )
            .is_none()
    );
    assert_eq!(frames.draft(0, source(2)), "");
    assert_eq!(frames.draft(1, source(1)), "Prefix committed");
    frames.type_text(0, "Fresh destination");
    assert_eq!(
        frames.click(0, "host_chat_send_rect").unwrap().text.as_str(),
        "Fresh destination",
        "late IME cannot strand the new view's composing flag"
    );
}

#[test]
fn shared_chat_monitor_transfer_flushes_the_delivered_old_focus_prefix_before_rebinding() {
    let mut frames = SurfaceFrames::new();
    frames.type_text(1, "Old source");
    let surface = ChatSurface::Monitor1;
    let context = &frames.contexts[1];
    let mut raw = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(680.0, 680.0))),
        time: Some(10.0),
        focused: true,
        events: vec![
            Event::Text(" delivered suffix".into()),
            key(Key::Enter, true, false, Modifiers::NONE),
        ],
        ..Default::default()
    };
    let edits = edit_prefix(&mut raw, frames.model.composer_rect_for(surface, context), false).unwrap();
    let palette = crate::theme::Theme::default().palette();
    let mut output = context.run_ui(edits, |ui| {
        assert!(frames.model.show_for(surface, ui, &palette, InputMode::Closing).is_none());
    });
    output.textures_delta.clear();
    frames.model.surface_closed(surface);
    frames.model.select_for(surface, source(2));
    assert_eq!(frames.draft(1, source(1)), "Old source delivered suffix");
    assert!(frames.show(1, vec![key(Key::Enter, true, false, Modifiers::NONE)]).is_none());
    assert_eq!(frames.draft(1, source(2)), "");
    assert!(frames.model.take_read_action_for(surface).is_none());
}

#[test]
fn shared_chat_two_closed_views_never_reuse_account_grid_generation_session_or_lease_authority() {
    for changed in 0..5 {
        let mut frames = SurfaceFrames::new();
        frames.type_text(0, "Private earlier identity");
        frames.type_text(1, " plus another surface");
        let old_ids = SurfaceFrames::SURFACES.map(|surface| frames.model.states[0].ui.retained_for(surface).3);
        frames.model.surface_closed(ChatSurface::Shared);
        frames.model.surface_closed(ChatSurface::Monitor1);
        let mut replacement = source(1);
        match changed {
            0 => replacement.session.account = AccountKey::from_bytes([21; 32]).unwrap(),
            1 => replacement.session.grid = GridKey::from_bytes([22; 32]).unwrap(),
            2 => replacement.session.generation += 1,
            3 => replacement.session.session = SessionId::from_bytes([23; 16]).unwrap(),
            _ => replacement.lease_epoch += 1,
        }
        let chat = Arc::make_mut(&mut frames.host.chat[0]);
        chat.source = replacement;
        for info in &mut chat.conversations {
            info.policy.key.source = replacement;
        }
        frames.host.workers[0].binding = replacement.binding();
        frames.model.sync(&frames.host, false);
        assert!(!frames.model.states.iter().any(|state| state.ui.source() == source(1)));
        let fresh = frames.model.states.iter().find(|state| state.ui.source() == replacement).unwrap();
        for (index, surface) in SurfaceFrames::SURFACES.into_iter().enumerate() {
            let retained = fresh.ui.retained_for(surface);
            assert_eq!(retained.1, "");
            assert_ne!(retained.3, old_ids[index]);
        }
        for index in 0..2 {
            assert!(
                frames
                    .show(
                        index,
                        vec![Event::Text("OLD CONTEXT".into()), key(Key::Enter, true, false, Modifiers::NONE)]
                    )
                    .is_none()
            );
            assert!(frames.model.take_read_action_for(SurfaceFrames::SURFACES[index]).is_none());
        }
        assert_eq!(
            frames.model.selected,
            Some(source(1)),
            "the unavailable Shared sender has no fallback"
        );
    }
}

#[test]
fn shared_chat_read_edit_prefix_keeps_live_hit_map_and_retains_delivered_text() {
    let mut frames = Frames::new();
    install_read(&mut frames, 1);
    frames.type_text("draft");
    let at = frames.rect(Id::unique("host_chat_read_rect")).center();
    assert!(
        frames
            .show(vec![
                Event::Text(" plus delivered".into()),
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                }
            ])
            .is_none()
    );
    assert!(frames.model.take_read_action().is_none(), "Editing pass cannot emit Read");
    assert!(frames.pointer(at, false).is_none());
    let read = frames
        .model
        .take_read_action()
        .expect("Live release retains the Read press hit map");
    assert_eq!(read.key.source, source(1));
    let draft = frames.send().unwrap();
    assert_eq!(draft.text.as_str(), "draft plus delivered");
}

#[test]
fn shared_chat_read_press_switch_release_cannot_retarget_and_closed_policy_loss_purges_metadata() {
    let mut frames = Frames::new();
    install_read(&mut frames, 1);
    let at = frames.rect(Id::unique("host_chat_read_rect")).center();
    assert!(frames.pointer(at, true).is_none());
    frames.model.select(source(2));
    assert!(frames.pointer(at, false).is_none());
    assert!(frames.model.take_read_action().is_none());
    frames.choose(source(1));
    let state = frames.model.states.iter().find(|state| state.ui.source() == source(1)).unwrap();
    assert!(state.ui.has_read_metadata());
    let chat = Arc::make_mut(&mut frames.host.chat[0]);
    chat.conversations[0].policy.revision += 1;
    chat.conversations[0].policy.can_read = false;
    chat.reads.clear();
    frames.model.sync(&frames.host, false); // closed/minimized: no UI pass
    let state = frames.model.states.iter().find(|state| state.ui.source() == source(1)).unwrap();
    assert!(!state.ui.has_read_metadata());
}

#[test]
fn shared_chat_external_key_selects_exact_direct_and_syncs_native_read_metadata_before_drawing() {
    let mut frames = Frames::new();
    install_read(&mut frames, 1);
    frames.type_text("Parked Nearby");
    let target = ChatKey {
        source: source(1),
        conversation: direct(),
    };
    frames.model.sync(&frames.host, false);
    assert!(frames.model.states[0].ui.has_read_metadata());
    assert!(frames.model.select_key(target));
    assert!(
        !frames.model.states[0].ui.has_read_metadata(),
        "Nearby metadata cannot survive until the target's first surface pass"
    );
    assert!(frames.model.take_read_action().is_none(), "selection does not acknowledge Read");
    assert!(frames.show(Vec::new()).is_none(), "selection does not send");
    frames.type_text("Exact Direct");
    let draft = frames.send().unwrap();
    assert_eq!(draft.key, target);
    assert_eq!(draft.text.as_str(), "Exact Direct");
    assert!(frames.model.select_key(ChatKey {
        source: source(1),
        conversation: Conversation::Nearby,
    }));
    assert!(frames.model.states[0].ui.has_read_metadata());
    assert_eq!(frames.send().unwrap().text.as_str(), "Parked Nearby");
}

#[test]
fn shared_chat_external_cross_source_navigation_retains_each_conversation_draft_and_undo() {
    let mut frames = Frames::new();
    let targets = [
        ChatKey {
            source: source(1),
            conversation: Conversation::Nearby,
        },
        ChatKey {
            source: source(1),
            conversation: direct(),
        },
        ChatKey {
            source: source(2),
            conversation: direct(),
        },
    ];
    for (index, target) in targets.iter().enumerate() {
        assert!(frames.model.select_key(*target));
        frames.type_text(&format!("Checkpoint {index}"));
        frames.settle(65);
        assert!(frames.show(vec![Event::Text(" plus edit".into())]).is_none());
        frames.settle(65);
    }
    frames.reopen();
    let command = Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    for (index, target) in targets.iter().enumerate() {
        assert!(frames.model.select_key(*target));
        let draft = frames.send().unwrap();
        assert_eq!(draft.key, *target);
        assert_eq!(draft.text.as_str(), format!("Checkpoint {index} plus edit"));
        let checkpoint = format!("Checkpoint {index}");
        let mut restored = String::new();
        for _ in 0..3 {
            frames.click(Id::unique("host_chat_input_rect"));
            assert!(frames.show(vec![key(Key::Z, true, false, command)]).is_none());
            assert!(frames.show(vec![key(Key::Z, false, false, command)]).is_none());
            restored = frames.send().map(|draft| draft.text.as_str().to_owned()).unwrap_or_default();
            if restored == checkpoint {
                break;
            }
        }
        assert_eq!(restored, checkpoint, "Undo belongs to {target:?}");
    }
}

#[test]
fn shared_chat_external_key_rejects_unavailable_policy_source_and_unsupported_targets_without_fallback() {
    for case in 0..13 {
        let mut frames = Frames::new();
        frames.type_text("Keep current sender");
        let mut target = ChatKey {
            source: source(2),
            conversation: direct(),
        };
        let chat = Arc::make_mut(&mut frames.host.chat[1]);
        match case {
            0 => chat.conversations[1].policy.ready = false,
            1 => chat.conversations[1].policy.lease_current = false,
            2 => chat.conversations[1].policy.can_read = false,
            3 => chat.conversations[1].policy.revision = 0,
            4 => chat.authenticated = false,
            5 => chat.live = false,
            6 => target.source.lease_epoch += 1,
            7 => target.source.session.generation += 1,
            8 => target.conversation = Conversation::Direct(ConversationId::from_bytes([99; 16]).unwrap()),
            9 | 10 => {
                let id = ConversationId::from_bytes([99; 16]).unwrap();
                target.conversation = if case == 9 {
                    Conversation::Group(id)
                } else {
                    Conversation::Conference(id)
                };
                chat.conversations[1].policy.key = target;
            }
            11 => chat.conversations[1].policy.key.source = source(3),
            12 => {
                frames.host.workers.remove(1);
            }
            _ => unreachable!(),
        };
        frames.model.sync(&frames.host, false);
        assert!(!frames.model.select_key(target), "rejected case {case}");
        assert_eq!(frames.model.selected, Some(source(1)), "no fallback for case {case}");
        assert!(frames.model.take_read_action().is_none());
        assert!(frames.show(Vec::new()).is_none());
        let draft = frames.send().unwrap();
        assert_eq!(draft.key.source, source(1));
        assert_eq!(draft.key.conversation, Conversation::Nearby);
        assert_eq!(draft.text.as_str(), "Keep current sender");
    }
}

#[test]
fn shared_chat_external_key_accepts_read_only_existing_targets_but_not_an_unbounded_catalogue_tail() {
    let mut frames = Frames::new();
    let target = ChatKey {
        source: source(2),
        conversation: direct(),
    };
    let chat = Arc::make_mut(&mut frames.host.chat[1]);
    chat.conversations[1].policy.can_send = false;
    frames.model.sync(&frames.host, false);
    assert!(frames.model.select_key(target));
    assert_eq!(frames.model.selected, Some(source(2)));
    assert!(frames.send().is_none());

    let chat = Arc::make_mut(&mut frames.host.chat[1]);
    let template = chat.conversations[0].clone();
    chat.conversations = vec![template; aurora_session::chat::MAX_CONVERSATIONS];
    chat.conversations.push(ConversationInfo {
        title: Label::new("Outside retained catalogue").unwrap(),
        policy: NativeChatPolicy {
            key: target,
            revision: 2,
            lease_current: true,
            ready: true,
            can_read: true,
            can_send: true,
        },
    });
    frames.model.sync(&frames.host, false);
    assert!(!frames.model.select_key(target));
    assert!(frames.model.take_read_action().is_none());
}

#[test]
fn shared_chat_external_navigation_preserves_captured_send_and_read_admission_and_completion() {
    use crate::session_host::chat_broker::{ReadOperation, ReadStatus};
    use aurora_session::chat::ReadOutcome;

    let mut frames = Frames::new();
    install_read(&mut frames, 1);
    frames.type_text("Captured Nearby send");
    let sent = frames.send().unwrap();
    assert!(frames.click(Id::unique("host_chat_read_rect")).is_none());
    let read = frames.model.take_read_action().unwrap();
    let target = ChatKey {
        source: source(2),
        conversation: direct(),
    };
    assert!(frames.model.select_key(target));
    frames.type_text("Unrelated Direct draft");
    // Local admission arrives after navigation and must use its captured key.
    frames.model.queued(&sent, 42);
    frames.model.queued_read(read, 43);
    let draft = frames.send().unwrap();
    assert_eq!(draft.key, target);
    assert_eq!(draft.text.as_str(), "Unrelated Direct draft");
    assert!(frames.model.select_key(ChatKey {
        source: source(1),
        conversation: direct(),
    }));
    assert!(frames.send().is_none(), "pending Send still belongs to the original source");
    let chat = Arc::make_mut(&mut frames.host.chat[0]);
    chat.receipts.push(SendReceipt {
        key: sent.key,
        request: 42,
        outcome: SendOutcome::AcceptedLocally,
    });
    chat.read_operations.push(ReadOperation {
        request: read.request(43),
        status: ReadStatus::Completed(ReadOutcome::Applied),
    });
    frames.model.sync(&frames.host, false);
    frames.type_text("Different conversation after completion");
    let draft = frames.send().unwrap();
    assert_eq!(draft.key.conversation, direct());
    assert_eq!(draft.key.source, source(1));
    assert!(frames.model.select_key(sent.key));
    assert!(frames.send().is_none(), "captured old text was consumed, never replayed");
    assert!(frames.click(Id::unique("host_chat_read_rect")).is_none());
    assert_eq!(
        frames.model.take_read_action().unwrap().key,
        read.key,
        "the old conversation's terminal completion released its Read gate"
    );
    assert!(frames.model.select_key(target));
    assert_eq!(frames.send().unwrap().text.as_str(), "Unrelated Direct draft");
}

#[test]
fn shared_chat_external_navigation_flushes_delivered_edits_and_fences_held_pointer_and_enter() {
    let mut frames = Frames::new();
    frames.type_text("Old Nearby");
    let target = ChatKey {
        source: source(2),
        conversation: direct(),
    };
    assert!(frames.model.select_key(target));
    frames.type_text("Target Direct");
    let old = ChatKey {
        source: source(1),
        conversation: Conversation::Nearby,
    };
    assert!(frames.model.select_key(old));
    frames.click(Id::unique("host_chat_input_rect"));
    let at = frames.rect(Id::unique("host_chat_send_rect")).center();
    // This is the existing production edit-only flush composition used by
    // Shell before external selection; neither queued gesture can send here.
    frames.flush(vec![
        Event::Text(" plus delivered".into()),
        key(Key::Enter, true, false, Modifiers::NONE),
        Event::PointerMoved(at),
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
    ]);
    assert!(frames.model.select_key(target));
    assert!(frames.pointer(at, false).is_none());
    assert!(frames.show(vec![key(Key::Enter, true, true, Modifiers::NONE)]).is_none());
    assert!(frames.model.take_read_action().is_none());
    assert!(frames.show(vec![key(Key::Enter, false, false, Modifiers::NONE)]).is_none());
    assert_eq!(frames.send().unwrap().text.as_str(), "Target Direct");
    assert!(frames.model.select_key(old));
    assert_eq!(frames.send().unwrap().text.as_str(), "Old Nearby plus delivered");

    let at = frames.rect(Id::unique("host_chat_send_rect")).center();
    assert!(frames.pointer(at, true).is_none());
    assert!(frames.model.select_key(old), "reopening the same key also cancels its held gesture");
    assert!(frames.pointer(at, false).is_none());
    assert_eq!(frames.send().unwrap().key, old);
}
