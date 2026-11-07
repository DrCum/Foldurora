//! A chat window is anchored to one exact worker/account/grid/session lease.
//! The active world is deliberately absent from this interface. Only typed
//! literal sends leave this view; changing conversations never retargets text.

use super::transport::{HostChatSnapshot, HostError};
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind};
use aurora_session::chat::{
    ChatKey, ChatSource, ChatText, Conversation, ConversationInfo, MAX_CONVERSATIONS, MAX_RECEIPTS, MAX_RETAINED_LINES, MAX_TEXT_BYTES,
    NativeChatPolicy, SendOutcome, SendReceipt,
};
use egui::{Id, RichText, TextEdit, vec2};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SendDraft {
    pub key: ChatKey,
    pub policy_revision: u64,
    pub text: ChatText,
    native_policy: NativeChatPolicy,
}

struct RetainedDraft {
    policy: NativeChatPolicy,
    // Strings also retain a bounded invalid draft so navigation cannot silently
    // discard an edit merely because it is not yet valid literal chat text.
    text: String,
    editor: egui::text_edit::TextEditState,
    incarnation: u64,
}

#[derive(Clone, Copy)]
enum Warning {
    Changed,
    TooLong,
    Invalid,
    Queue(HostError),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum InputMode {
    Live,
    /// A selection changed this pass: show its state without interpreting
    /// input that began in the preceding conversation or character.
    Navigating,
    /// Drain queued edits before disposing a native surface. Navigation and
    /// sends are impossible even if Enter or a button release was queued.
    Closing,
}

pub(super) fn source_id(source: ChatSource) -> Id {
    Id::unique((
        "host_chat_source",
        (source.worker.installation, source.worker.host, source.worker.worker),
        (
            source.session.session,
            source.session.account,
            source.session.grid,
            source.session.generation,
            source.lease_epoch,
        ),
    ))
}

fn policy_id(policy: NativeChatPolicy) -> Id {
    source_id(policy.key.source).with((
        policy.key.conversation,
        policy.revision,
        policy.lease_current,
        policy.ready,
        policy.can_send,
        policy.can_read,
    ))
}

/// A held pointer/key is never permission to act on a newly selected target.
/// This is context input only; it sends no worker or world command.
pub(super) fn cancel_input(ctx: &egui::Context) {
    egui::Popup::close_all(ctx);
    ctx.stop_dragging();
    ctx.input_mut(|input| {
        input.pointer = Default::default();
        input
            .events
            .retain(|event| !matches!(event, egui::Event::Key { key: egui::Key::Enter, .. }));
    });
}

pub(super) struct ChatUi {
    source: ChatSource,
    /// Shell's monotonic composer allocation. Re-authentication of the same
    /// source must not reuse a revoked editor's still-focused widget ID.
    instance: u64,
    conversation: Conversation,
    policy: Option<NativeChatPolicy>,
    // At most MAX_CONVERSATIONS entries, each with MAX_TEXT_BYTES of text.
    // Undo state is bounded by egui and owned here, never left in a surface's
    // context after a pass: sync can purge it even while closed or minimized.
    drafts: BTreeMap<Conversation, RetainedDraft>,
    next_incarnation: u64,
    available: bool,
    warning: Option<Warning>,
    receipt: Option<SendReceipt>,
    catalogue: Vec<ConversationInfo>,
    interaction: u64,
    changed: bool,
    ime_composing: bool,
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

fn native_policy(snapshot: &HostChatSnapshot, key: ChatKey) -> Option<NativeChatPolicy> {
    snapshot.conversations.iter().take(MAX_CONVERSATIONS).find_map(|info| {
        (info.policy.key == key && info.policy.validate().is_ok() && info.policy.lease_current && info.policy.ready).then_some(info.policy)
    })
}

impl ChatUi {
    #[cfg(test)]
    pub fn new(source: ChatSource) -> Self {
        Self::for_instance(source, 0)
    }

    pub fn for_instance(source: ChatSource, instance: u64) -> Self {
        Self {
            source,
            instance,
            conversation: Conversation::Nearby,
            policy: None,
            drafts: BTreeMap::new(),
            next_incarnation: 0,
            available: false,
            warning: None,
            receipt: None,
            catalogue: Vec::new(),
            interaction: 0,
            changed: false,
            ime_composing: false,
        }
    }

    pub fn source(&self) -> ChatSource {
        self.source
    }

    /// Called on every snapshot update, including while minimized. Lost
    /// authority purges text immediately, rather than waiting for a repaint.
    pub fn sync(&mut self, snapshot: Option<&HostChatSnapshot>) {
        let current = snapshot.filter(|snapshot| snapshot.source == self.source && eligible(snapshot));
        let catalogue = current.map_or_else(Vec::new, |snapshot| {
            snapshot
                .conversations
                .iter()
                .take(MAX_CONVERSATIONS)
                .filter(|info| info.policy.key.source == self.source && supported(info.policy.key.conversation))
                .cloned()
                .collect()
        });
        if self.catalogue != catalogue {
            self.catalogue = catalogue;
            self.cancel_gesture();
        }
        self.available = current.is_some();
        let mut cleared_text = false;
        self.drafts.retain(|conversation, draft| {
            let key = ChatKey {
                source: self.source,
                conversation: *conversation,
            };
            let keep = current.and_then(|snapshot| native_policy(snapshot, key)) == Some(draft.policy);
            cleared_text |= !keep && !draft.text.is_empty();
            keep
        });
        if cleared_text {
            self.warning = Some(Warning::Changed);
        }
        self.policy = current.and_then(|snapshot| native_policy(snapshot, self.key()));
        if !self.available {
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

    pub fn cancel_gesture(&mut self) {
        self.interaction = self.interaction.saturating_add(1);
        self.changed = true;
        self.ime_composing = false;
    }

    fn draft(&self) -> &str {
        self.drafts.get(&self.conversation).map_or("", |draft| draft.text.as_str())
    }

    fn ensure_draft(&mut self, policy: NativeChatPolicy) -> Option<&mut RetainedDraft> {
        if !self.drafts.contains_key(&policy.key.conversation) {
            if self.drafts.len() == MAX_CONVERSATIONS {
                return None;
            }
            self.next_incarnation = self.next_incarnation.checked_add(1)?;
            self.drafts.insert(
                policy.key.conversation,
                RetainedDraft {
                    policy,
                    text: String::new(),
                    editor: Default::default(),
                    incarnation: self.next_incarnation,
                },
            );
        }
        self.drafts.get_mut(&policy.key.conversation).filter(|draft| draft.policy == policy)
    }

    fn editor_id(&self) -> Id {
        let source = self.source;
        let authority = self.policy.map(|policy| {
            (
                policy.revision,
                policy.lease_current,
                policy.ready,
                policy.can_send,
                policy.can_read,
            )
        });
        Id::unique((
            "host_chat_draft",
            (source.worker.installation, source.worker.host, source.worker.worker),
            (
                source.session.session,
                source.session.account,
                source.session.grid,
                source.session.generation,
                source.lease_epoch,
            ),
            self.conversation,
            authority,
            self.instance,
            self.drafts.get(&self.conversation).map(|draft| draft.incarnation),
        ))
    }

    pub fn composer_rect(&self, ctx: &egui::Context) -> Option<egui::Rect> {
        ctx.read_response(self.editor_id())
            .filter(|response| response.has_focus())
            .map(|response| response.rect)
    }

    fn select(&mut self, conversation: Conversation) {
        if self.conversation == conversation || !supported(conversation) {
            return;
        }
        self.policy = None;
        self.conversation = conversation;
        self.cancel_gesture();
        self.warning = ChatText::new(self.draft()).is_err().then_some(Warning::Invalid);
        if self.receipt.is_some_and(|receipt| receipt.outcome != SendOutcome::Pending) {
            self.receipt = None;
        }
    }

    pub fn queued(&mut self, draft: &SendDraft, request: u64) {
        if draft.key.source != self.source || request == 0 {
            return;
        }
        if self.drafts.get(&draft.key.conversation).is_some_and(|retained| {
            retained.policy == draft.native_policy
                && draft.native_policy.key == draft.key
                && draft.native_policy.revision == draft.policy_revision
                && retained.text == draft.text.as_str()
        }) {
            self.drafts.remove(&draft.key.conversation);
        }
        self.receipt = Some(SendReceipt {
            key: draft.key,
            request,
            outcome: SendOutcome::Pending,
        });
        if self.key() == draft.key {
            self.warning = None;
        }
    }

    pub fn queue_failed(&mut self, error: HostError) {
        self.warning = Some(Warning::Queue(error));
    }

    #[cfg(test)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        snapshot: Option<&HostChatSnapshot>,
        account: &str,
        grid: &str,
    ) -> Option<SendDraft> {
        self.show_in_mode(ui, p, snapshot, account, grid, InputMode::Live)
    }

    #[cfg(test)]
    pub fn show_in_mode(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        snapshot: Option<&HostChatSnapshot>,
        account: &str,
        grid: &str,
        mode: InputMode,
    ) -> Option<SendDraft> {
        self.show_with_selector(ui, p, snapshot, account, grid, mode, |_| false)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show_with_selector(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        snapshot: Option<&HostChatSnapshot>,
        account: &str,
        grid: &str,
        mode: InputMode,
        selector: impl FnOnce(&mut egui::Ui) -> bool,
    ) -> Option<SendDraft> {
        self.sync(snapshot);
        let popup_open = egui::Popup::is_any_open(ui.ctx());
        let mut navigating = std::mem::take(&mut self.changed) || mode == InputMode::Navigating;
        if navigating {
            cancel_input(ui.ctx());
        }
        let focused = ui.input(|input| input.focused);
        let focus_changed = ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(_))));
        if (!focused || focus_changed) && mode != InputMode::Closing {
            cancel_input(ui.ctx());
            self.ime_composing = false;
        }
        let source_id = source_id(self.source);
        let mut action = None;
        let frame = widgets::overlay_frame(ui.ctx(), p);
        egui::Panel::bottom(source_id.with("compose")).frame(frame).show(ui, |ui| {
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
            if let Some(policy) = self.policy {
                let _ = self.ensure_draft(policy);
            }
            let previous = self.draft().to_owned();
            let mut text = previous.clone();
            let editor_id = self.editor_id();
            let previous_editor = self
                .drafts
                .get(&self.conversation)
                .map(|draft| draft.editor.clone())
                .unwrap_or_default();
            let previous_undo = previous_editor.undoer();
            TextEdit::store_state(ui.ctx(), editor_id, previous_editor.clone());
            let was_composing = self.ime_composing;
            let mut ime_event = false;
            ui.input(|input| {
                for event in &input.events {
                    if let egui::Event::Ime(event) = event {
                        ime_event = true;
                        match event {
                            egui::ImeEvent::Preedit { text, .. } => self.ime_composing = !text.is_empty(),
                            egui::ImeEvent::Commit(_) => self.ime_composing = false,
                            _ => {}
                        }
                    }
                }
            });
            let can_shortcut = mode == InputMode::Live
                && focused
                && !focus_changed
                && !navigating
                && !popup_open
                && !egui::Popup::is_any_open(ui.ctx())
                && !was_composing
                && !self.ime_composing
                && !ime_event
                && ui.ctx().memory(|memory| memory.has_focus(editor_id));
            let shortcut = can_shortcut
                && ui.input(|input| input.events.iter().any(|event| {
                    matches!(event, egui::Event::Key {
                        key: egui::Key::Enter, pressed: true, repeat: false, modifiers, ..
                    } if !modifiers.shift && !modifiers.alt)
                }));
            // Freeze the Enter payload before processing any later text in
            // this OS event batch. Later edits keep their own retained state.
            let shortcut_text = shortcut
                .then(|| ChatText::new(self.draft()).ok().filter(|text| !text.as_str().trim().is_empty()))
                .flatten();
            // Enter sends, Shift+Enter inserts a newline. Remove repeated or
            // suppressed send keys too, so they cannot mutate a later draft.
            if mode == InputMode::Closing || (!popup_open && ui.ctx().memory(|memory| memory.has_focus(editor_id)) && !was_composing && !self.ime_composing && !ime_event) {
                ui.ctx().input_mut(|input| input.events.retain(|event| {
                    !matches!(event, egui::Event::Key { key: egui::Key::Enter, modifiers, .. } if !modifiers.shift || mode == InputMode::Closing)
                }));
            }
            let output = ui
                .add_enabled_ui(editable && self.drafts.contains_key(&self.conversation), |ui| {
                    TextEdit::multiline(&mut text)
                        .id(editor_id)
                        .return_key(Some(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)))
                        .hint_text(aurora_i18n::tr!("host-chat-input"))
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .char_limit(MAX_TEXT_BYTES)
                        .show(ui)
                })
                .inner;
            // Keeping text/undo only in the bounded source-bound model makes
            // minimized/closed policy pruning immediate and context-independent.
            ui.ctx().data_mut(|data| data.remove::<egui::text_edit::TextEditState>(editor_id));
            let input = output.response;
            #[cfg(not(test))]
            let _ = input;
            let mut editor = output.state;
            #[cfg(test)]
            ui.ctx()
                .data_mut(|data| data.insert_temp(Id::unique("host_chat_input_rect"), input.rect));
            if input.changed() {
                if text.len() > MAX_TEXT_BYTES {
                    text = previous;
                    editor = previous_editor;
                    editor.set_undoer(previous_undo);
                    self.warning = Some(Warning::TooLong);
                } else if ChatText::new(&text).is_err() {
                    self.warning = Some(Warning::Invalid);
                } else {
                    self.warning = None;
                }
            }
            if let Some(draft) = self.drafts.get_mut(&self.conversation) {
                draft.text = text;
                draft.editor = editor;
            }
            ui.horizontal_wrapped(|ui| {
                let text = ChatText::new(self.draft()).ok().filter(|text| !text.as_str().trim().is_empty());
                let ready = editable && text.is_some();
                // Empty icon deliberately avoids the process-global icon
                // registry: each CPU popout has its own egui texture space.
                let send = ui.push_id((editor_id, self.interaction, "send"), |ui| widgets::action_button_if(
                    ui,
                    p,
                    "",
                    aurora_i18n::tr!("host-chat-send"),
                    ActionKind::Primary,
                    vec2(0.0, 30.0),
                    ready && mode == InputMode::Live && focused && !focus_changed && !navigating && !popup_open && !self.ime_composing && !ime_event,
                )).inner;
                #[cfg(test)]
                ui.ctx()
                    .data_mut(|data| data.insert_temp(Id::unique("host_chat_send_rect"), send.rect));
                let text = if shortcut { shortcut_text } else { text };
                if editable
                    && (send.clicked() || shortcut)
                    && let (Some(policy), Some(text)) = (self.policy, text)
                {
                    action = Some(SendDraft {
                        key: policy.key,
                        policy_revision: policy.revision,
                        text,
                        native_policy: policy,
                    });
                }
                ui.label(RichText::new(aurora_i18n::tr!("host-chat-compose-help")).small().color(p.muted));
            });
        });
        if mode == InputMode::Closing {
            return None;
        }
        // Apply delivered edits to the old focused composer before drawing
        // either navigation control. A navigation discards this visual pass;
        // the next pass paints the new target without replaying input.
        let source_changed = selector(ui);
        egui::Panel::top(source_id.with("identity")).frame(frame).show(ui, |ui| {
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
            let combo_width = ui.available_width();
            let combo = egui::ComboBox::from_id_salt((source_id, self.instance, "conversation", self.interaction))
                .selected_text(selected)
                .width(combo_width)
                .wrap_mode(egui::TextWrapMode::Truncate)
                .show_ui(ui, |ui| {
                    ui.set_width((combo_width - ui.spacing().menu_margin.sum().x).max(1.0));
                    if let Some(snapshot) = current {
                        for info in snapshot.conversations.iter().take(MAX_CONVERSATIONS).filter(|info| {
                            info.policy.key.source == self.source
                                && supported(info.policy.key.conversation)
                                && info.policy.validate().is_ok()
                                && info.policy.lease_current
                                && info.policy.ready
                        }) {
                            let response = ui
                                .add_enabled_ui(mode == InputMode::Live, |ui| {
                                    ui.push_id(policy_id(info.policy), |ui| {
                                        ui.selectable_value(&mut picked, info.policy.key.conversation, info.title.as_str())
                                    })
                                    .inner
                                })
                                .inner;
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
        self.sync(snapshot);
        let conversation_changed = std::mem::take(&mut self.changed);
        navigating |= source_changed || conversation_changed;
        if source_changed || conversation_changed {
            cancel_input(ui.ctx());
            ui.ctx().request_discard("chat target changed after retaining delivered edits");
        }
        if navigating || egui::Popup::is_any_open(ui.ctx()) {
            action = None;
        }
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
                    .id_salt((source_id, self.instance, "lines", self.conversation))
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
mod visual;

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
            Self::for_source(source())
        }
        fn for_source(source: ChatSource) -> Self {
            let context = egui::Context::default();
            crate::theme::Theme {
                panel_alpha: 255,
                ..Default::default()
            }
            .apply(&context, 1.0);
            Self {
                context,
                ui: ChatUi::new(source),
                frame: 0,
            }
        }
        fn show(&mut self, snapshot: Option<&HostChatSnapshot>, mut events: Vec<Event>) -> Option<SendDraft> {
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
            events.insert(0, Event::ModifiersChanged(modifiers));
            let mut action = None;
            let mut output = self.context.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 640.0))),
                    time: Some(self.frame as f64 / 60.0),
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
        fn navigate(&mut self, snapshot: &HostChatSnapshot, conversation: Conversation) {
            assert!(self.click(snapshot, Id::unique("host_chat_combo_rect")).is_none());
            assert!(self.click(snapshot, Id::unique(("host_chat_pick", conversation))).is_none());
            assert_eq!(self.ui.conversation, conversation);
        }
        fn undo(&mut self, snapshot: &HostChatSnapshot) {
            if !self.context.memory(|memory| memory.has_focus(self.ui.editor_id())) {
                assert!(self.click(snapshot, Id::unique("host_chat_input_rect")).is_none());
            }
            let modifiers = egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            };
            assert!(
                self.show(
                    Some(snapshot),
                    vec![Event::Key {
                        key: egui::Key::Z,
                        physical_key: Some(egui::Key::Z),
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }]
                )
                .is_none()
            );
            self.show(
                Some(snapshot),
                vec![Event::Key {
                    key: egui::Key::Z,
                    physical_key: Some(egui::Key::Z),
                    pressed: false,
                    repeat: false,
                    modifiers,
                }],
            );
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
                text: ChatText::new("/ao off").unwrap(),
                native_policy: snapshot.conversations[0].policy,
            }
        );
        // The producer has no active-world parameter. Picking a different
        // conversation restores its own draft without retargeting Nearby text.
        frames.click(&snapshot, Id::unique("host_chat_combo_rect"));
        frames.click(&snapshot, Id::unique(("host_chat_pick", direct())));
        assert!(frames.ui.draft().is_empty());
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
    fn host_chat_popout_roundtrip_retains_independent_conversation_and_window_drafts() {
        let snapshot = snapshot(source());
        let mut frames = Frames::new();
        frames.type_text(&snapshot, "Nearby draft · こんにちは");
        frames.navigate(&snapshot, direct());
        assert!(frames.ui.draft().is_empty());
        assert!(frames.ui.warning.is_none());
        frames.type_text(&snapshot, "Resident IM draft · Bonjour");
        frames.navigate(&snapshot, Conversation::Nearby);
        assert_eq!(frames.ui.draft(), "Nearby draft · こんにちは");
        assert!(frames.ui.warning.is_none());
        frames.navigate(&snapshot, direct());
        assert_eq!(frames.ui.draft(), "Resident IM draft · Bonjour");

        // Independent popouts must not share either text or editor state,
        // including another popout anchored to the same immutable source.
        let mut same_source = Frames::new();
        same_source.type_text(&snapshot, "Another window's draft");
        for _ in 0..65 {
            same_source.show(Some(&snapshot), Vec::new());
        }
        same_source.navigate(&snapshot, direct());
        assert!(same_source.ui.draft().is_empty());
        same_source.undo(&snapshot);
        assert!(same_source.ui.draft().is_empty());
        assert_eq!(frames.ui.draft(), "Resident IM draft · Bonjour");
        let mut other_source = source();
        other_source.session.account = AccountKey::from_bytes([55; 32]).unwrap();
        let other_snapshot = self::snapshot(other_source);
        let mut other = Frames::for_source(other_source);
        other.type_text(&other_snapshot, "Another character's draft");
        assert_eq!(other.ui.source(), other_source);
        assert_eq!(frames.ui.source(), source());
        assert_eq!(frames.ui.draft(), "Resident IM draft · Bonjour");
    }

    #[test]
    fn host_chat_popout_real_undo_checkpoints_stay_with_their_exact_conversation() {
        let snapshot = snapshot(source());
        let mut frames = Frames::new();
        frames.type_text(&snapshot, "Nearby first edit");
        for _ in 0..65 {
            frames.show(Some(&snapshot), Vec::new());
        }
        frames.show(Some(&snapshot), vec![Event::Text(" plus second edit".into())]);
        for _ in 0..65 {
            frames.show(Some(&snapshot), Vec::new());
        }
        let latest = frames.ui.draft().to_owned();
        assert_ne!(latest, "Nearby first edit");
        frames.navigate(&snapshot, direct());
        frames.undo(&snapshot);
        assert!(
            frames.ui.draft().is_empty(),
            "a real Nearby checkpoint cannot enter the resident composer"
        );
        frames.type_text(&snapshot, "Resident-only draft");
        frames.navigate(&snapshot, Conversation::Nearby);
        assert_eq!(frames.ui.draft(), latest);
        for _ in 0..3 {
            frames.undo(&snapshot);
            if frames.ui.draft() != latest {
                break;
            }
        }
        assert_eq!(
            frames.ui.draft(),
            "Nearby first edit",
            "the original conversation's real undo history remains usable"
        );
        frames.navigate(&snapshot, direct());
        assert_eq!(frames.ui.draft(), "Resident-only draft");
    }

    #[test]
    fn host_chat_popout_sync_prunes_parked_policy_changes_without_touching_other_drafts() {
        let initial = snapshot(source());
        for remove in [false, true] {
            let mut frames = Frames::new();
            frames.type_text(&initial, "Private Nearby draft");
            for _ in 0..65 {
                frames.show(Some(&initial), Vec::new());
            }
            frames.navigate(&initial, direct());
            frames.type_text(&initial, "Unaffected resident draft");
            let mut changed = initial.clone();
            if remove {
                changed.conversations.remove(0);
            } else {
                changed.conversations[0].policy.revision += 1;
                changed.conversations[0].policy.can_read = false;
            }
            changed.gap = true;
            // No UI frame: this is the minimized/closed snapshot path.
            frames.ui.sync(Some(&changed));
            assert!(!frames.ui.drafts.contains_key(&Conversation::Nearby));
            assert_eq!(frames.ui.draft(), "Unaffected resident draft");
            // Restoring the original metadata cannot restore a purged draft
            // through either the text store or the editor's previous undoer.
            frames.ui.sync(Some(&initial));
            frames.navigate(&initial, Conversation::Nearby);
            assert!(frames.ui.draft().is_empty());
            frames.undo(&initial);
            assert!(frames.ui.draft().is_empty());
            frames.navigate(&initial, direct());
            assert_eq!(frames.ui.draft(), "Unaffected resident draft");
        }
    }

    #[test]
    fn host_chat_popout_auth_live_and_source_loss_purge_all_parked_drafts_without_a_frame() {
        let initial = snapshot(source());
        let mut unauthorized = initial.clone();
        unauthorized.authenticated = false;
        let mut expired = initial.clone();
        expired.live = false;
        let mut replacement = source();
        replacement.session.generation += 1;
        let replaced = snapshot(replacement);
        for unavailable in [None, Some(&unauthorized), Some(&expired), Some(&replaced)] {
            let mut frames = Frames::new();
            frames.type_text(&initial, "Nearby private draft");
            frames.navigate(&initial, direct());
            frames.type_text(&initial, "IM private draft");
            frames.ui.sync(unavailable);
            assert!(frames.ui.drafts.is_empty());
            assert!(!frames.ui.available);
            frames.ui.sync(Some(&initial));
            frames.undo(&initial);
            assert!(frames.ui.draft().is_empty());
            frames.navigate(&initial, Conversation::Nearby);
            frames.undo(&initial);
            assert!(frames.ui.draft().is_empty());
        }
    }

    #[test]
    fn host_chat_popout_queue_admission_clears_only_its_frozen_policy_text_and_conversation() {
        let mut snapshot = snapshot(source());
        let mut frames = Frames::new();
        frames.type_text(&snapshot, "Nearby explicit send");
        let sent = frames.click(&snapshot, Id::unique("host_chat_send_rect")).unwrap();
        frames.navigate(&snapshot, direct());
        frames.type_text(&snapshot, "Keep this resident draft");
        frames.ui.queue_failed(HostError::Busy);
        assert_eq!(frames.ui.drafts[&Conversation::Nearby].text, sent.text.as_str());
        frames.ui.queued(&sent, 42);
        assert!(!frames.ui.drafts.contains_key(&Conversation::Nearby));
        assert_eq!(frames.ui.draft(), "Keep this resident draft");
        snapshot.receipts.push(SendReceipt {
            key: ChatKey {
                source: source(),
                conversation: direct(),
            },
            request: 42,
            outcome: SendOutcome::AcceptedLocally,
        });
        frames.ui.sync(Some(&snapshot));
        assert_eq!(frames.ui.receipt.unwrap().outcome, SendOutcome::Pending);
        snapshot.receipts.push(SendReceipt {
            key: sent.key,
            request: 42,
            outcome: SendOutcome::Uncertain,
        });
        assert!(frames.show(Some(&snapshot), Vec::new()).is_none());
        assert_eq!(frames.ui.receipt.unwrap().outcome, SendOutcome::Uncertain);
        assert_eq!(frames.ui.draft(), "Keep this resident draft");
        frames.navigate(&snapshot, Conversation::Nearby);
        frames.undo(&snapshot);
        assert!(
            frames.ui.draft().is_empty(),
            "admission also discards that exact draft's undo history"
        );

        let mut frames = Frames::new();
        frames.type_text(&snapshot, "Earlier version");
        let earlier = frames.click(&snapshot, Id::unique("host_chat_send_rect")).unwrap();
        frames.type_text(&snapshot, " plus a newer edit");
        let newer = frames.ui.draft().to_owned();
        frames.ui.queued(&earlier, 43);
        assert_eq!(frames.ui.draft(), newer);

        let mut frames = Frames::new();
        frames.type_text(&snapshot, "Same text under a changed policy");
        let earlier = frames.click(&snapshot, Id::unique("host_chat_send_rect")).unwrap();
        snapshot.conversations[0].policy.can_read = false;
        frames.ui.sync(Some(&snapshot));
        frames.type_text(&snapshot, earlier.text.as_str());
        frames.ui.queued(&earlier, 44);
        assert_eq!(
            frames.ui.draft(),
            earlier.text.as_str(),
            "full policy identity, beyond its revision, is frozen"
        );
    }

    #[test]
    fn host_chat_popout_unicode_and_invalid_drafts_roundtrip_with_bounded_conversation_storage() {
        let mut snapshot = snapshot(source());
        let mut frames = Frames::new();
        let nearby = "界".repeat(MAX_TEXT_BYTES / 3);
        frames.type_text(&snapshot, &nearby);
        frames.navigate(&snapshot, direct());
        let direct_text = "🌕".repeat(MAX_TEXT_BYTES / 4);
        frames.type_text(&snapshot, &direct_text);
        frames.show(Some(&snapshot), vec![Event::Paste("🌕".into())]);
        assert_eq!(frames.ui.draft(), direct_text);
        frames.navigate(&snapshot, Conversation::Nearby);
        assert_eq!(frames.ui.draft(), nearby);
        assert!(frames.ui.drafts.values().all(|draft| draft.text.len() <= MAX_TEXT_BYTES));
        frames.navigate(&snapshot, direct());
        assert_eq!(frames.ui.draft(), direct_text);

        let mut invalid = Frames::new();
        invalid.type_text(&snapshot, "Keep invalid edit \u{1}");
        assert!(matches!(invalid.ui.warning, Some(Warning::Invalid)));
        invalid.navigate(&snapshot, direct());
        invalid.navigate(&snapshot, Conversation::Nearby);
        assert_eq!(invalid.ui.draft(), "Keep invalid edit \u{1}");
        assert!(matches!(invalid.ui.warning, Some(Warning::Invalid)));

        // The native catalogue is externally supplied. Its bounded prefix
        // limits all retained entries, even when more metadata is offered.
        let template = snapshot.conversations[1].clone();
        snapshot.conversations = (0..=MAX_CONVERSATIONS)
            .map(|index| {
                let mut info = template.clone();
                info.policy.key.conversation = Conversation::Direct(ConversationId::from_bytes([(index + 1) as u8; 16]).unwrap());
                info
            })
            .collect();
        let mut bounded = ChatUi::new(source());
        bounded.sync(Some(&snapshot));
        for info in &snapshot.conversations {
            if let Some(policy) = native_policy(&snapshot, info.policy.key)
                && let Some(draft) = bounded.ensure_draft(policy)
            {
                draft.text = nearby.clone();
            }
        }
        assert_eq!(bounded.drafts.len(), MAX_CONVERSATIONS);
        assert_eq!(
            bounded.drafts.values().map(|draft| draft.text.len()).sum::<usize>(),
            MAX_CONVERSATIONS * MAX_TEXT_BYTES
        );
        assert!(bounded.ensure_draft(snapshot.conversations[MAX_CONVERSATIONS].policy).is_none());
    }

    #[test]
    fn host_chat_retained_model_roundtrip_retains_drafts_selection_and_final_receipt_without_replay() {
        let mut snapshot = snapshot(source());
        let mut frames = Frames::new();
        frames.type_text(&snapshot, "Parked Nearby draft");
        for _ in 0..65 {
            frames.show(Some(&snapshot), Vec::new());
        }
        frames.show(Some(&snapshot), vec![Event::Text(" plus retained edit".into())]);
        for _ in 0..65 {
            frames.show(Some(&snapshot), Vec::new());
        }
        let nearby = frames.ui.draft().to_owned();
        frames.navigate(&snapshot, direct());
        frames.type_text(&snapshot, "Explicit resident send");
        let sent = frames.click(&snapshot, Id::unique("host_chat_send_rect")).unwrap();
        frames.ui.queued(&sent, 42);
        snapshot.receipts.push(SendReceipt {
            key: sent.key,
            request: 42,
            outcome: SendOutcome::Pending,
        });
        let mut retained = frames.ui;

        // Delivery became uncertain while the native surface was closed.
        // Reopening uses a fresh context, not the destroyed window's memory.
        snapshot.receipts[0].outcome = SendOutcome::Uncertain;
        retained.sync(Some(&snapshot));
        let mut reopened = Frames::new();
        reopened.ui = retained;
        assert_eq!(reopened.ui.conversation, direct());
        assert_eq!(reopened.ui.receipt.unwrap().outcome, SendOutcome::Uncertain);
        assert!(reopened.show(Some(&snapshot), Vec::new()).is_none());
        assert!(reopened.ui.draft().is_empty());
        reopened.navigate(&snapshot, Conversation::Nearby);
        assert_eq!(reopened.ui.draft(), nearby);
        for _ in 0..3 {
            reopened.undo(&snapshot);
            if reopened.ui.draft() != nearby {
                break;
            }
        }
        assert_eq!(
            reopened.ui.draft(),
            "Parked Nearby draft",
            "the retained editor also works on the fresh context"
        );

        reopened.navigate(&snapshot, direct());
        reopened.type_text(&snapshot, "Keep this resident draft");
        let mut retained = reopened.ui;
        snapshot.conversations[0].policy.revision += 1;
        retained.sync(Some(&snapshot));
        let mut reopened = Frames::new();
        reopened.ui = retained;
        assert_eq!(reopened.ui.draft(), "Keep this resident draft");
        reopened.navigate(&snapshot, Conversation::Nearby);
        reopened.undo(&snapshot);
        assert!(reopened.ui.draft().is_empty());
        let mut retained = reopened.ui;
        snapshot.authenticated = false;
        retained.sync(Some(&snapshot));
        assert!(retained.drafts.is_empty());
    }

    #[test]
    fn host_chat_popout_nearby_retention_gap_preserves_an_inactive_im_draft() {
        let initial = snapshot(source());
        let mut frames = Frames::new();
        frames.show(Some(&initial), Vec::new());
        frames.click(&initial, Id::unique("host_chat_combo_rect"));
        frames.click(&initial, Id::unique(("host_chat_pick", direct())));
        frames.type_text(&initial, "Keep this inactive IM draft");
        let policy = frames.ui.policy;
        let mut overflow = initial.clone();
        overflow.gap = true;
        overflow.lines[0].event = 300;
        overflow.lines[0].text = ChatText::new("Latest synthetic nearby line").unwrap();
        for _ in 0..3 {
            assert!(frames.show(Some(&overflow), Vec::new()).is_none());
        }
        assert_eq!(frames.ui.draft(), "Keep this inactive IM draft");
        assert_eq!(frames.ui.policy, policy);
        let send = frames.click(&overflow, Id::unique("host_chat_send_rect")).unwrap();
        assert_eq!(
            send.key,
            ChatKey {
                source: source(),
                conversation: direct()
            }
        );
        assert_eq!(send.text.as_str(), "Keep this inactive IM draft");
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
            assert!(frames.ui.draft().is_empty());
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
        assert_eq!(frames.ui.draft(), draft.text.as_str());
        // Only an explicit second click after failed local admission queues it.
        let retry = frames.click(&snapshot, Id::unique("host_chat_send_rect")).unwrap();
        frames.ui.queued(&retry, 42);
        snapshot.receipts.push(SendReceipt {
            key: retry.key,
            request: 42,
            outcome: SendOutcome::Pending,
        });
        frames.type_text(&snapshot, "Must not enter a disabled composer");
        assert!(frames.ui.draft().is_empty());
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
        assert!(frames.ui.draft().is_empty());
    }

    #[test]
    fn host_chat_valid_conversation_switch_cancels_a_held_send_without_discarding_either_draft() {
        let snapshot = snapshot(source());
        let mut frames = Frames::new();
        frames.type_text(&snapshot, "Nearby retained");
        frames.navigate(&snapshot, direct());
        frames.type_text(&snapshot, "Resident retained");
        frames.navigate(&snapshot, Conversation::Nearby);
        let at = frames.rect(&snapshot, Id::unique("host_chat_send_rect")).center();
        frames.show(
            Some(&snapshot),
            vec![
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frames.ui.select(direct());
        assert!(
            frames
                .show(
                    Some(&snapshot),
                    vec![Event::PointerButton {
                        pos: at,
                        button: PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    }]
                )
                .is_none()
        );
        assert_eq!(
            frames.click(&snapshot, Id::unique("host_chat_send_rect")).unwrap().text.as_str(),
            "Resident retained"
        );
        frames.navigate(&snapshot, Conversation::Nearby);
        assert_eq!(frames.ui.draft(), "Nearby retained");
    }

    #[test]
    fn host_chat_catalogue_reorder_cancels_a_held_send_without_purging_unchanged_policy() {
        let mut snapshot = snapshot(source());
        let mut frames = Frames::new();
        frames.type_text(&snapshot, "Current policy retained");
        let at = frames.rect(&snapshot, Id::unique("host_chat_send_rect")).center();
        frames.show(
            Some(&snapshot),
            vec![
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        snapshot.conversations.reverse();
        assert!(
            frames
                .show(
                    Some(&snapshot),
                    vec![Event::PointerButton {
                        pos: at,
                        button: PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    }]
                )
                .is_none()
        );
        assert_eq!(frames.ui.draft(), "Current policy retained");
        assert_eq!(
            frames.click(&snapshot, Id::unique("host_chat_send_rect")).unwrap().key.conversation,
            Conversation::Nearby
        );
    }

    #[test]
    fn host_chat_popout_unicode_drafts_stay_byte_bounded_and_unsupported_backends_are_ineligible() {
        let mut snapshot = snapshot(source());
        let mut frames = Frames::new();
        frames.type_text(&snapshot, "Safe draft");
        frames.show(Some(&snapshot), vec![Event::Paste("🌕".repeat(400))]);
        assert_eq!(frames.ui.draft(), "Safe draft");
        assert!(matches!(frames.ui.warning, Some(Warning::TooLong)));
        assert!(frames.ui.draft().len() <= MAX_TEXT_BYTES);
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
