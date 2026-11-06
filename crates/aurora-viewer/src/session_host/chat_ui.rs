//! A chat window is anchored to one exact worker/account/grid/session lease.
//! The active world is deliberately absent from this interface. Only typed
//! literal sends leave this view; changing conversations never retargets text.

use super::transport::{HostChatSnapshot, HostError};
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind};
use aurora_session::chat::{
    ChatKey, ChatSource, ChatText, Conversation, MAX_CONVERSATIONS, MAX_RECEIPTS, MAX_RETAINED_LINES, MAX_TEXT_BYTES, NativeChatPolicy,
    SendOutcome, SendReceipt,
};
use egui::{Id, RichText, TextEdit, vec2};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SendDraft {
    pub key: ChatKey,
    pub policy_revision: u64,
    pub text: ChatText,
}

#[derive(Clone, Copy)]
enum Warning {
    Changed,
    TooLong,
    Invalid,
    Queue(HostError),
}

pub(super) struct ChatUi {
    source: ChatSource,
    conversation: Conversation,
    policy: Option<NativeChatPolicy>,
    // One draft, at most 1023 UTF-8 bytes, bound by policy + conversation.
    draft: String,
    available: bool,
    warning: Option<Warning>,
    receipt: Option<SendReceipt>,
}

pub(super) fn eligible(snapshot: &HostChatSnapshot) -> bool {
    snapshot.source.validate().is_ok()
        && snapshot.authenticated
        && snapshot.live
        && snapshot.conversations.iter().take(MAX_CONVERSATIONS).any(|info| {
            info.policy.key.source == snapshot.source
                && supported(info.policy.key.conversation)
                && info.policy.validate().is_ok()
                && info.policy.lease_current
                && info.policy.ready
                && (info.policy.can_read || info.policy.can_send)
        })
}

fn supported(conversation: Conversation) -> bool {
    matches!(conversation, Conversation::Nearby | Conversation::Direct(_))
}

impl ChatUi {
    pub fn new(source: ChatSource) -> Self {
        Self {
            source,
            conversation: Conversation::Nearby,
            policy: None,
            draft: String::new(),
            available: false,
            warning: None,
            receipt: None,
        }
    }

    pub fn source(&self) -> ChatSource {
        self.source
    }

    /// Called on every snapshot update, including while minimized. Lost
    /// authority purges text immediately, rather than waiting for a repaint.
    pub fn sync(&mut self, snapshot: Option<&HostChatSnapshot>) {
        let current = snapshot.filter(|snapshot| snapshot.source == self.source && eligible(snapshot));
        self.available = current.is_some();
        let policy = current.and_then(|snapshot| {
            snapshot.conversations.iter().take(MAX_CONVERSATIONS).find_map(|info| {
                (info.policy.key == self.key() && info.policy.validate().is_ok() && info.policy.lease_current && info.policy.ready)
                    .then_some(info.policy)
            })
        });
        if self.policy != policy {
            if !self.draft.is_empty() {
                self.warning = Some(Warning::Changed);
            }
            self.draft.clear();
            self.policy = policy;
        }
        if !self.available {
            self.draft.clear();
            if let Some(receipt) = &mut self.receipt
                && receipt.outcome == SendOutcome::Pending
            {
                receipt.outcome = SendOutcome::Uncertain;
            }
            return;
        }
        if let Some(snapshot) = current {
            let wanted = self.receipt.map(|receipt| receipt.request);
            if let Some(receipt) = snapshot
                .receipts
                .iter()
                .take(MAX_RECEIPTS)
                .filter(|receipt| {
                    receipt.key.source == self.source
                        && supported(receipt.key.conversation)
                        && wanted.map_or(receipt.key == self.key(), |request| {
                            receipt.request == request && self.receipt.is_some_and(|old| old.key == receipt.key)
                        })
                })
                .max_by_key(|receipt| receipt.request)
                && (receipt.outcome != SendOutcome::Pending || self.receipt.is_none_or(|old| old.outcome == SendOutcome::Pending))
            {
                self.receipt = Some(*receipt);
            }
        }
    }

    fn key(&self) -> ChatKey {
        ChatKey {
            source: self.source,
            conversation: self.conversation,
        }
    }

    fn select(&mut self, conversation: Conversation) {
        if self.conversation == conversation || !supported(conversation) {
            return;
        }
        if !self.draft.is_empty() {
            self.warning = Some(Warning::Changed);
        }
        self.draft.clear();
        self.policy = None;
        self.conversation = conversation;
        if self.receipt.is_some_and(|receipt| receipt.outcome != SendOutcome::Pending) {
            self.receipt = None;
        }
    }

    pub fn queued(&mut self, draft: &SendDraft, request: u64) {
        if draft.key.source != self.source || request == 0 {
            return;
        }
        if self.key() == draft.key
            && self.policy.is_some_and(|policy| policy.revision == draft.policy_revision)
            && self.draft == draft.text.as_str()
        {
            self.draft.clear();
        }
        self.receipt = Some(SendReceipt {
            key: draft.key,
            request,
            outcome: SendOutcome::Pending,
        });
        self.warning = None;
    }

    pub fn queue_failed(&mut self, error: HostError) {
        self.warning = Some(Warning::Queue(error));
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        snapshot: Option<&HostChatSnapshot>,
        account: &str,
        grid: &str,
    ) -> Option<SendDraft> {
        self.sync(snapshot);
        let mut action = None;
        let frame = widgets::overlay_frame(ui.ctx(), p);
        egui::Panel::top("host_chat_identity").frame(frame).show(ui, |ui| {
            ui.label(RichText::new(aurora_i18n::tr!("host-chat-identity", account = account, grid = grid)).strong());
            ui.label(RichText::new(aurora_i18n::tr!("host-chat-bound-help")).small().color(p.muted));
            let current = snapshot.filter(|snapshot| snapshot.source == self.source && self.available);
            let selected = current
                .and_then(|snapshot| {
                    snapshot
                        .conversations
                        .iter()
                        .take(MAX_CONVERSATIONS)
                        .find(|info| info.policy.key == self.key())
                })
                .map(|info| info.title.as_str())
                .unwrap_or(aurora_i18n::tr!("host-chat-no-conversation"));
            let mut picked = self.conversation;
            let combo = egui::ComboBox::from_id_salt("host_chat_conversation")
                .selected_text(selected)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    if let Some(snapshot) = current {
                        for info in snapshot.conversations.iter().take(MAX_CONVERSATIONS).filter(|info| {
                            info.policy.key.source == self.source
                                && supported(info.policy.key.conversation)
                                && info.policy.validate().is_ok()
                                && info.policy.lease_current
                                && info.policy.ready
                        }) {
                            let response = ui.selectable_value(&mut picked, info.policy.key.conversation, info.title.as_str());
                            #[cfg(test)]
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(Id::unique(("host_chat_pick", info.policy.key.conversation)), response.rect)
                            });
                            #[cfg(not(test))]
                            let _ = response;
                        }
                    }
                });
            #[cfg(test)]
            ui.ctx()
                .data_mut(|data| data.insert_temp(Id::unique("host_chat_combo_rect"), combo.response.rect));
            #[cfg(not(test))]
            let _ = combo;
            self.select(picked);
        });
        // Selecting another conversation discards its predecessor's draft,
        // then obtains that exact conversation's native policy before input.
        self.sync(snapshot);
        egui::Panel::bottom("host_chat_compose").frame(frame).show(ui, |ui| {
            if !self.available {
                ui.label(RichText::new(aurora_i18n::tr!("host-chat-unavailable")).color(p.warn));
            }
            if let Some(warning) = self.warning {
                ui.label(RichText::new(warning_text(warning)).small().color(p.warn));
            }
            if let Some(receipt) = self.receipt {
                ui.label(RichText::new(outcome_text(receipt.outcome)).small().color(p.muted));
            }
            let editable = self.available
                && self
                    .policy
                    .is_some_and(|policy| policy.lease_current && policy.ready && policy.can_send)
                && !self.receipt.is_some_and(|receipt| receipt.outcome == SendOutcome::Pending);
            if self.available && !self.policy.is_some_and(|policy| policy.can_send) {
                ui.label(RichText::new(aurora_i18n::tr!("host-chat-restricted")).small().color(p.warn));
            }
            let previous = self.draft.clone();
            let input = ui.add_enabled(
                editable,
                TextEdit::multiline(&mut self.draft)
                    .id(Id::unique("host_chat_draft"))
                    .hint_text(aurora_i18n::tr!("host-chat-input"))
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .char_limit(MAX_TEXT_BYTES),
            );
            #[cfg(test)]
            ui.ctx()
                .data_mut(|data| data.insert_temp(Id::unique("host_chat_input_rect"), input.rect));
            if input.changed() {
                if self.draft.len() > MAX_TEXT_BYTES {
                    self.draft = previous;
                    self.warning = Some(Warning::TooLong);
                } else if ChatText::new(&self.draft).is_err() {
                    self.warning = Some(Warning::Invalid);
                } else {
                    self.warning = None;
                }
            }
            ui.horizontal_wrapped(|ui| {
                let text = ChatText::new(&self.draft).ok().filter(|text| !text.as_str().trim().is_empty());
                let ready = editable && text.is_some();
                // Empty icon deliberately avoids the process-global icon
                // registry: each CPU popout has its own egui texture space.
                let send = widgets::action_button_if(
                    ui,
                    p,
                    "",
                    aurora_i18n::tr!("host-chat-send"),
                    ActionKind::Primary,
                    vec2(0.0, 30.0),
                    ready,
                );
                #[cfg(test)]
                ui.ctx()
                    .data_mut(|data| data.insert_temp(Id::unique("host_chat_send_rect"), send.rect));
                let shortcut = input.has_focus() && ui.input(|input| input.modifiers.command && input.key_pressed(egui::Key::Enter));
                if ready
                    && (send.clicked() || shortcut)
                    && let (Some(policy), Some(text)) = (self.policy, text)
                {
                    action = Some(SendDraft {
                        key: policy.key,
                        policy_revision: policy.revision,
                        text,
                    });
                }
                ui.label(RichText::new(aurora_i18n::tr!("host-chat-compose-help")).small().color(p.muted));
            });
        });
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            if let Some(snapshot) = snapshot.filter(|snapshot| snapshot.source == self.source && self.available) {
                if snapshot.gap {
                    ui.label(RichText::new(aurora_i18n::tr!("host-chat-history-gap")).small().color(p.muted));
                }
                let readable = self
                    .policy
                    .is_some_and(|policy| policy.ready && policy.lease_current && policy.can_read);
                let mut shown = 0;
                egui::ScrollArea::vertical()
                    .id_salt(("host_chat_lines", self.conversation))
                    .stick_to_bottom(true)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        if readable && let Some(policy) = self.policy {
                            for line in snapshot
                                .lines
                                .iter()
                                .take(MAX_RETAINED_LINES)
                                .filter(|line| line.key == policy.key && line.policy_revision == policy.revision)
                            {
                                shown += 1;
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new(line.sender.as_str()).strong().color(p.violet_pale));
                                    ui.add(egui::Label::new(line.text.as_str()).selectable(true));
                                });
                            }
                        }
                        if shown == 0 {
                            let text = if readable {
                                aurora_i18n::tr!("host-chat-empty")
                            } else {
                                aurora_i18n::tr!("host-chat-restricted")
                            };
                            ui.label(RichText::new(text).color(p.muted));
                        }
                    });
            }
        });
        action
    }
}

fn outcome_text(outcome: SendOutcome) -> &'static str {
    match outcome {
        SendOutcome::Pending => aurora_i18n::tr!("host-chat-queued"),
        SendOutcome::AcceptedLocally => aurora_i18n::tr!("host-chat-accepted"),
        SendOutcome::RejectedNative => aurora_i18n::tr!("host-chat-rejected"),
        SendOutcome::Uncertain => aurora_i18n::tr!("host-chat-uncertain"),
    }
}

fn warning_text(warning: Warning) -> &'static str {
    match warning {
        Warning::Changed => aurora_i18n::tr!("host-chat-draft-cleared"),
        Warning::TooLong => aurora_i18n::tr!("host-chat-too-long"),
        Warning::Invalid => aurora_i18n::tr!("host-chat-invalid"),
        Warning::Queue(HostError::Busy) => aurora_i18n::tr!("host-chat-queue-busy"),
        Warning::Queue(HostError::Unavailable | HostError::Unsupported | HostError::Stopped) => {
            aurora_i18n::tr!("host-chat-queue-unavailable")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::chat::{ChatLine, ConversationInfo, LineKind};
    use aurora_session::{
        AccountKey, ConversationId, GridKey, HostId, InstallationId, Label, SessionId, SessionIdentity, WorkerId, WorkerIdentity,
    };
    use egui::{Event, PointerButton, Pos2, Rect};

    fn source() -> ChatSource {
        ChatSource {
            worker: WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).unwrap(),
                host: HostId::from_bytes([2; 16]).unwrap(),
                worker: WorkerId::from_bytes([3; 16]).unwrap(),
            },
            session: SessionIdentity {
                session: SessionId::from_bytes([4; 16]).unwrap(),
                account: AccountKey::from_bytes([5; 32]).unwrap(),
                grid: GridKey::from_bytes([6; 32]).unwrap(),
                generation: 7,
            },
            lease_epoch: 8,
        }
    }
    fn direct() -> Conversation {
        Conversation::Direct(ConversationId::from_bytes([11; 16]).unwrap())
    }
    fn snapshot(source: ChatSource) -> HostChatSnapshot {
        let conversations = [(Conversation::Nearby, "Synthetic nearby"), (direct(), "Synthetic resident")]
            .into_iter()
            .enumerate()
            .map(|(index, (conversation, title))| ConversationInfo {
                policy: NativeChatPolicy {
                    key: ChatKey { source, conversation },
                    revision: index as u64 + 1,
                    lease_current: true,
                    ready: true,
                    can_send: true,
                    can_read: true,
                },
                title: Label::new(title).unwrap(),
            })
            .collect();
        HostChatSnapshot {
            source,
            authenticated: true,
            live: true,
            conversations,
            lines: vec![ChatLine {
                key: ChatKey {
                    source,
                    conversation: Conversation::Nearby,
                },
                event: 1,
                policy_revision: 1,
                sender: Label::new("Synthetic sender").unwrap(),
                text: ChatText::new("Synthetic live line").unwrap(),
                kind: LineKind::Incoming,
                truncated: false,
            }],
            receipts: Vec::new(),
            gap: false,
        }
    }

    struct Frames {
        context: egui::Context,
        ui: ChatUi,
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
            Self {
                context,
                ui: ChatUi::new(source()),
                frame: 0,
            }
        }
        fn show(&mut self, snapshot: Option<&HostChatSnapshot>, events: Vec<Event>) -> Option<SendDraft> {
            self.frame += 1;
            let palette = crate::theme::Theme {
                panel_alpha: 255,
                ..Default::default()
            }
            .palette();
            let modifiers = events
                .iter()
                .rev()
                .find_map(|event| match event {
                    Event::Key { modifiers, .. } | Event::PointerButton { modifiers, .. } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or_default();
            let mut action = None;
            let mut output = self.context.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 640.0))),
                    time: Some(self.frame as f64 / 60.0),
                    modifiers,
                    events,
                    ..Default::default()
                },
                |ui| {
                    let shown = self.ui.show(ui, &palette, snapshot, "Synthetic Alpha", "Synthetic Grid");
                    action = action.take().or(shown);
                },
            );
            assert!(
                output.shapes.iter().all(|shape| !matches!(&shape.shape, egui::Shape::Callback(_))),
                "chat popout stays CPU mesh-only"
            );
            output.textures_delta.clear();
            action
        }
        fn rect(&mut self, snapshot: &HostChatSnapshot, id: Id) -> Rect {
            for _ in 0..3 {
                self.show(Some(snapshot), Vec::new());
            }
            self.context
                .data(|data| data.get_temp::<Rect>(id))
                .expect("real UI action was laid out")
        }
        fn click(&mut self, snapshot: &HostChatSnapshot, id: Id) -> Option<SendDraft> {
            let rect = self.rect(snapshot, id);
            self.show(Some(snapshot), vec![Event::PointerMoved(rect.center())]);
            let mut action = None;
            for pressed in [true, false] {
                let shown = self.show(
                    Some(snapshot),
                    vec![Event::PointerButton {
                        pos: rect.center(),
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                action = action.or(shown);
            }
            action
        }
        fn type_text(&mut self, snapshot: &HostChatSnapshot, text: &str) {
            assert!(self.click(snapshot, Id::unique("host_chat_input_rect")).is_none());
            assert!(self.show(Some(snapshot), vec![Event::Text(text.into())]).is_none());
        }
    }

    #[test]
    fn host_chat_popout_real_egui_send_keeps_exact_inactive_character_and_conversation() {
        let snapshot = snapshot(source());
        let mut frames = Frames::new();
        frames.type_text(&snapshot, "/ao off");
        let nearby = frames.click(&snapshot, Id::unique("host_chat_send_rect")).unwrap();
        assert_eq!(
            nearby,
            SendDraft {
                key: ChatKey {
                    source: source(),
                    conversation: Conversation::Nearby
                },
                policy_revision: 1,
                text: ChatText::new("/ao off").unwrap()
            }
        );
        // The producer has no active-world parameter. Picking a different
        // conversation must clear, rather than move, the existing draft.
        frames.click(&snapshot, Id::unique("host_chat_combo_rect"));
        frames.click(&snapshot, Id::unique(("host_chat_pick", direct())));
        assert!(frames.ui.draft.is_empty());
        frames.type_text(&snapshot, "Synthetic literal IM");
        let direct_send = frames.click(&snapshot, Id::unique("host_chat_send_rect")).unwrap();
        assert_eq!(
            direct_send.key,
            ChatKey {
                source: source(),
                conversation: direct()
            }
        );
        assert_eq!(direct_send.policy_revision, 2);
        assert_eq!(direct_send.text.as_str(), "Synthetic literal IM");
        assert_eq!(frames.ui.source(), source());
    }

    #[test]
    fn host_chat_popout_ctrl_enter_emits_one_literal_action_from_focused_composer() {
        let snapshot = snapshot(source());
        let mut frames = Frames::new();
        frames.type_text(&snapshot, "/ao off");
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        let action = frames
            .show(
                Some(&snapshot),
                vec![Event::Key {
                    key: egui::Key::Enter,
                    physical_key: Some(egui::Key::Enter),
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
            )
            .unwrap();
        assert_eq!(
            action.key,
            ChatKey {
                source: source(),
                conversation: Conversation::Nearby
            }
        );
        assert_eq!(action.text.as_str(), "/ao off");
        frames.ui.queued(&action, 1);
        assert!(
            frames
                .show(
                    Some(&snapshot),
                    vec![Event::Key {
                        key: egui::Key::Enter,
                        physical_key: Some(egui::Key::Enter),
                        pressed: true,
                        repeat: true,
                        modifiers
                    }]
                )
                .is_none()
        );
    }

    #[test]
    fn host_chat_popout_source_or_policy_change_purges_draft_before_pending_click_release() {
        let initial = snapshot(source());
        let mut changed_source = source();
        changed_source.session.generation += 1;
        let generation = snapshot(changed_source);
        changed_source = source();
        changed_source.lease_epoch += 1;
        let lease = snapshot(changed_source);
        changed_source = source();
        changed_source.session.grid = GridKey::from_bytes([66; 32]).unwrap();
        let grid = snapshot(changed_source);
        let mut policy = initial.clone();
        policy.conversations[0].policy.revision += 1;
        let mut expired = initial.clone();
        expired.live = false;
        for changed in [generation, lease, grid, policy, expired] {
            let mut frames = Frames::new();
            frames.type_text(&initial, "Never retarget this draft");
            let rect = frames.rect(&initial, Id::unique("host_chat_send_rect"));
            frames.show(
                Some(&initial),
                vec![
                    Event::PointerMoved(rect.center()),
                    Event::PointerButton {
                        pos: rect.center(),
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            let action = frames.show(
                Some(&changed),
                vec![Event::PointerButton {
                    pos: rect.center(),
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert!(action.is_none());
            assert!(frames.ui.draft.is_empty());
            assert_eq!(frames.ui.source(), source());
        }
    }

    #[test]
    fn host_chat_popout_queue_failure_preserves_draft_and_uncertain_receipt_never_retries() {
        let mut snapshot = snapshot(source());
        let mut frames = Frames::new();
        frames.type_text(&snapshot, "Synthetic explicit send");
        let draft = frames.click(&snapshot, Id::unique("host_chat_send_rect")).unwrap();
        frames.ui.queue_failed(HostError::Busy);
        assert!(frames.show(Some(&snapshot), Vec::new()).is_none());
        assert_eq!(frames.ui.draft, draft.text.as_str());
        // Only an explicit second click after failed local admission queues it.
        let retry = frames.click(&snapshot, Id::unique("host_chat_send_rect")).unwrap();
        frames.ui.queued(&retry, 42);
        snapshot.receipts.push(SendReceipt {
            key: retry.key,
            request: 42,
            outcome: SendOutcome::Pending,
        });
        frames.type_text(&snapshot, "Must not enter a disabled composer");
        assert!(frames.ui.draft.is_empty());
        assert!(frames.click(&snapshot, Id::unique("host_chat_send_rect")).is_none());
        snapshot.receipts[0].outcome = SendOutcome::Uncertain;
        for _ in 0..4 {
            assert!(frames.show(Some(&snapshot), Vec::new()).is_none());
        }
        assert_eq!(frames.ui.receipt.unwrap().outcome, SendOutcome::Uncertain);
        // A stale pending snapshot cannot turn a final uncertain result back
        // into an in-flight request or automatically send its payload again.
        snapshot.receipts[0].outcome = SendOutcome::Pending;
        assert!(frames.show(Some(&snapshot), Vec::new()).is_none());
        assert_eq!(frames.ui.receipt.unwrap().outcome, SendOutcome::Uncertain);
        assert!(frames.ui.draft.is_empty());
    }

    #[test]
    fn host_chat_popout_unicode_drafts_stay_byte_bounded_and_unsupported_backends_are_ineligible() {
        let mut snapshot = snapshot(source());
        let mut frames = Frames::new();
        frames.type_text(&snapshot, "Safe draft");
        frames.show(Some(&snapshot), vec![Event::Paste("🌕".repeat(400))]);
        assert_eq!(frames.ui.draft, "Safe draft");
        assert!(matches!(frames.ui.warning, Some(Warning::TooLong)));
        assert!(frames.ui.draft.len() <= MAX_TEXT_BYTES);
        assert!(eligible(&snapshot));
        snapshot.authenticated = false;
        assert!(!eligible(&snapshot));
        snapshot.authenticated = true;
        snapshot.live = false;
        assert!(!eligible(&snapshot));
        snapshot.live = true;
        for info in &mut snapshot.conversations {
            info.policy.key.conversation = Conversation::Group(ConversationId::from_bytes([11; 16]).unwrap());
        }
        assert!(!eligible(&snapshot));
    }
}
