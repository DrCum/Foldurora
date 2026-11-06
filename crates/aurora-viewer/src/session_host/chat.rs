//! Main-thread chat backend. The socket lane owns IO; this module retains only
//! bounded live data and never interprets text as GUI input or a viewer command.

use aurora_session::Label;
use aurora_session::chat::{
    ChatCommand, ChatError, ChatEvent, ChatKey, ChatLine, ChatMessage, ChatPacket, ChatSource, ChatText, Conversation, ConversationInfo,
    MAX_CONVERSATIONS, MAX_RETAINED_LINES, NativeChatPolicy, SendAdmission, SendLedger, SendOutcome, SendReceipt, VERSION,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const SCAN_CONVERSATIONS: usize = 4;
pub(crate) const SCAN_LINES: usize = 8;
const POLL_LINES: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NativeCursor {
    pub token: u64,
    pub next: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeSnapshot {
    pub title: Label,
    /// Native rule/mute revisions and privacy options, not message contents.
    pub stamp: [u64; 4],
    pub ready: bool,
    pub can_send: bool,
    pub can_read: bool,
    pub initial: NativeCursor,
}

pub(crate) struct NativeLine {
    pub sender: Label,
    pub text: ChatText,
    pub kind: aurora_session::chat::LineKind,
    pub truncated: bool,
}
pub(crate) struct NativePage {
    pub cursor: NativeCursor,
    pub gap: bool,
    pub lines: Vec<NativeLine>,
}

pub(crate) trait NativeChat {
    /// Called again at operation boundaries; must evaluate the current control
    /// lease/account, not return a cached source captured before a long pause.
    fn source(&self) -> Option<ChatSource>;
    fn catalogue(&self) -> Vec<Conversation>;
    fn snapshot(&self, conversation: Conversation) -> Option<NativeSnapshot>;
    fn lines(&self, conversation: Conversation, cursor: NativeCursor, limit: usize) -> NativePage;
    fn submit_literal(&mut self, key: ChatKey, text: &str) -> SendOutcome;
}

struct ConversationState {
    snapshot: NativeSnapshot,
    policy: NativeChatPolicy,
    cursor: NativeCursor,
}

#[derive(Default)]
pub(crate) struct WorkerChat {
    source: Option<ChatSource>,
    retired: Option<ChatSource>,
    ledger: Option<SendLedger>,
    states: BTreeMap<Conversation, ConversationState>,
    lines: VecDeque<ChatLine>,
    receipt: Option<SendReceipt>,
    next_event: u64,
    next_policy: u64,
    gap_through: u64,
    scan: usize,
    metadata: usize,
    received: u64,
    outgoing: u64,
}

impl WorkerChat {
    pub fn current_source(&self) -> Option<ChatSource> {
        self.source
    }
    pub fn outgoing_sequence(&self) -> u64 {
        self.outgoing
    }

    /// Only for lost authority/logout, not a transient socket disconnect.
    /// Producer/receiver sequences survive; the same retired lease cannot
    /// silently recreate a ledger and replay an uncertain send.
    pub fn clear(&mut self) {
        if let Some(source) = self.source.take() {
            self.retired = Some(source);
        }
        self.ledger = None;
        self.states.clear();
        self.lines.clear();
        self.receipt = None;
        self.next_event = 0;
        self.next_policy = 0;
        self.gap_through = 0;
        self.scan = 0;
        self.metadata = 0;
    }

    fn context(&mut self, native: &impl NativeChat) -> Result<ChatSource, ChatError> {
        let Some(source) = native.source() else {
            self.clear();
            return Err(ChatError::Unavailable);
        };
        source.validate()?;
        if self.source == Some(source) {
            return Ok(source);
        }
        if let Some(old) = self.source.or(self.retired) {
            let mut check = SendLedger::new(old)?;
            if !check.reset(source)? {
                return Err(ChatError::Binding);
            }
        }
        self.clear();
        self.source = Some(source);
        self.ledger = Some(SendLedger::new(source)?);
        Ok(source)
    }

    fn event(&mut self) -> Result<u64, ChatError> {
        self.next_event = self.next_event.checked_add(1).ok_or(ChatError::Sequence)?;
        Ok(self.next_event)
    }
    fn gap(&mut self) -> Result<(), ChatError> {
        self.gap_through = self.event()?;
        Ok(())
    }

    fn refresh(&mut self, native: &impl NativeChat, conversation: Conversation) -> Result<(), ChatError> {
        let source = self.context(native)?;
        let snapshot = native.snapshot(conversation).ok_or(ChatError::Unavailable)?;
        let existing = self.states.contains_key(&conversation);
        let changed = self.states.get(&conversation).is_none_or(|state| {
            state.snapshot.stamp != snapshot.stamp
                || state.snapshot.ready != snapshot.ready
                || state.snapshot.can_send != snapshot.can_send
                || state.snapshot.can_read != snapshot.can_read
        });
        if !changed {
            if let Some(state) = self.states.get_mut(&conversation) {
                state.snapshot.title = snapshot.title;
            }
            return Ok(());
        }
        if !existing && self.states.len() == MAX_CONVERSATIONS {
            return Err(ChatError::Capacity);
        }
        self.next_policy = self.next_policy.checked_add(1).ok_or(ChatError::Sequence)?;
        let key = ChatKey { source, conversation };
        let previous = self.lines.len();
        self.lines.retain(|line| line.key != key);
        // The host may still retain a title or text after this worker has
        // evicted its own duplicate. Invalidation cannot depend on retained
        // native lines, nor wait for this conversation's metadata rotation.
        if existing || previous != self.lines.len() {
            self.gap()?;
        }
        let policy = NativeChatPolicy {
            key,
            revision: self.next_policy,
            lease_current: true,
            ready: snapshot.ready,
            can_send: snapshot.can_send,
            can_read: snapshot.can_read,
        };
        let cursor = snapshot.initial;
        self.states.insert(conversation, ConversationState { snapshot, policy, cursor });
        Ok(())
    }

    /// Scans at most four conversations/eight native entries each. Network,
    /// transcript and inventory servicing remain owned by the ordinary viewer.
    pub fn sync(&mut self, native: &impl NativeChat) -> Result<(), ChatError> {
        let source = self.context(native)?;
        let catalogue: Vec<_> = native.catalogue().into_iter().take(MAX_CONVERSATIONS).collect();
        let known: BTreeSet<_> = catalogue.iter().copied().collect();
        let previous_states = self.states.len();
        self.states.retain(|conversation, _| known.contains(conversation));
        let before = self.lines.len();
        self.lines.retain(|line| known.contains(&line.key.conversation));
        if previous_states != self.states.len() || before != self.lines.len() {
            self.gap()?;
        }
        for _ in 0..SCAN_CONVERSATIONS.min(catalogue.len()) {
            let conversation = catalogue[self.scan % catalogue.len()];
            self.scan = self.scan.wrapping_add(1);
            self.refresh(native, conversation)?;
            let state = self.states.get(&conversation).ok_or(ChatError::Unavailable)?;
            if !state.policy.ready || !state.policy.can_read {
                continue;
            }
            let policy = state.policy;
            let page = native.lines(conversation, state.cursor, SCAN_LINES);
            if let Some(state) = self.states.get_mut(&conversation) {
                state.cursor = page.cursor;
            }
            if page.gap {
                // Missing/cleared native history is ordinary loss, not a
                // privacy revocation. A skipped event marks consumer history
                // incomplete without purging other conversations or drafts.
                self.event()?;
            }
            for line in page.lines.into_iter().take(SCAN_LINES) {
                let event = self.event()?;
                if self.lines.len() == MAX_RETAINED_LINES {
                    self.lines.pop_front();
                }
                self.lines.push_back(ChatLine {
                    key: policy.key,
                    event,
                    policy_revision: policy.revision,
                    sender: line.sender,
                    text: line.text,
                    kind: line.kind,
                    truncated: line.truncated,
                });
            }
        }
        if self.context(native)? != source {
            return Err(ChatError::Binding);
        }
        Ok(())
    }

    fn packet(&mut self, reply_to: u64, event: ChatEvent) -> Result<ChatPacket, ChatError> {
        self.outgoing = self.outgoing.checked_add(1).ok_or(ChatError::Sequence)?;
        Ok(ChatPacket {
            version: VERSION,
            source: self.source.ok_or(ChatError::Unavailable)?,
            sequence: self.outgoing,
            reply_to,
            message: ChatMessage::Event(event),
        })
    }

    pub fn handle(&mut self, native: &mut impl NativeChat, packet: &ChatPacket) -> Result<Vec<ChatPacket>, ChatError> {
        packet.validate()?;
        let source = self.context(native)?;
        if packet.source != source {
            return Err(ChatError::Binding);
        }
        let ChatMessage::Command(command) = &packet.message else {
            return Err(ChatError::Invalid);
        };
        if packet.sequence <= self.received {
            return Err(ChatError::Sequence);
        }
        match command {
            ChatCommand::Send(request) => {
                // Only a currently advertised native conversation can send.
                if !native
                    .catalogue()
                    .into_iter()
                    .take(MAX_CONVERSATIONS)
                    .any(|key| key == request.key.conversation)
                {
                    return Err(ChatError::Unavailable);
                }
                self.refresh(native, request.key.conversation)?;
                let policy = self.states.get(&request.key.conversation).ok_or(ChatError::Unavailable)?.policy;
                let admission = self.ledger.as_mut().ok_or(ChatError::Unavailable)?.admit(request, policy);
                let receipt = match admission {
                    Ok(SendAdmission::Duplicate(receipt)) => receipt,
                    Ok(SendAdmission::SubmitOnce) => {
                        if self.context(native)? != source {
                            return Err(ChatError::Binding);
                        }
                        let outcome = native.submit_literal(request.key, request.text.as_str());
                        self.ledger
                            .as_mut()
                            .ok_or(ChatError::Unavailable)?
                            .finish(request.key, request.request, outcome)?
                    }
                    Err(ChatError::Denied | ChatError::Unavailable | ChatError::StalePolicy | ChatError::Busy) => SendReceipt {
                        key: request.key,
                        request: request.request,
                        outcome: SendOutcome::RejectedNative,
                    },
                    Err(error) => return Err(error),
                };
                if self.context(native)? != source {
                    return Err(ChatError::Binding);
                }
                self.received = packet.sequence;
                self.receipt = Some(receipt);
                Ok(vec![self.packet(packet.sequence, ChatEvent::Receipt(receipt))?])
            }
            ChatCommand::Poll { after_event } => {
                self.sync(native)?;
                let keys: Vec<_> = self.states.keys().copied().collect();
                let metadata = if !keys.is_empty() {
                    let key = keys[self.metadata % keys.len()];
                    self.metadata = self.metadata.wrapping_add(1);
                    self.refresh(native, key)?;
                    Some(key)
                } else {
                    None
                };
                // Recheck current native restrictions on every exported line,
                // including conversations outside this tick's scan rotation.
                // Stage all native refreshes first: any can invalidate host
                // caches, and that reset must precede every new metadata line.
                let after = (*after_event).max(self.gap_through);
                let selected: Vec<_> = self
                    .lines
                    .iter()
                    .filter(|line| line.event > after)
                    .take(POLL_LINES)
                    .cloned()
                    .collect();
                for line in &selected {
                    self.refresh(native, line.key.conversation)?;
                }
                let mut events = Vec::new();
                let mut advertised = BTreeSet::new();
                if *after_event < self.gap_through {
                    events.push(ChatEvent::Gap {
                        through_event: self.gap_through,
                    });
                }
                if let Some(key) = metadata {
                    let state = self.states.get(&key).ok_or(ChatError::Unavailable)?;
                    events.push(ChatEvent::Conversation(ConversationInfo {
                        policy: state.policy,
                        title: state.snapshot.title.clone(),
                    }));
                    advertised.insert(key);
                }
                let after = (*after_event).max(self.gap_through);
                for line in selected.into_iter().filter(|line| line.event > after) {
                    let current = self.states.get(&line.key.conversation).ok_or(ChatError::Unavailable)?;
                    if current.policy.can_read && current.policy.ready && current.policy.revision == line.policy_revision {
                        // The host must never invent permissions for a line
                        // outside the rotating metadata conversation. Send
                        // its trusted current policy first in the same batch.
                        if advertised.insert(line.key.conversation) {
                            let info = ConversationInfo {
                                policy: current.policy,
                                title: current.snapshot.title.clone(),
                            };
                            events.push(ChatEvent::Conversation(info));
                        }
                        events.push(ChatEvent::Line(line));
                    }
                }
                if let Some(receipt) = self.receipt {
                    events.push(ChatEvent::Receipt(receipt));
                }
                if self.context(native)? != source {
                    return Err(ChatError::Binding);
                }
                let result = events
                    .into_iter()
                    .map(|event| self.packet(packet.sequence, event))
                    .collect::<Result<Vec<_>, _>>()?;
                self.received = packet.sequence;
                Ok(result)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::chat::{ChatDrafts, ChatHistory, LineKind, SendRequest};
    use aurora_session::{
        AccountKey, ConversationId, GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerId, WorkerIdentity,
    };
    use std::cell::Cell;

    fn source(generation: u64) -> ChatSource {
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
                generation,
            },
            lease_epoch: 1,
        }
    }

    struct Native {
        source: ChatSource,
        calls: Cell<usize>,
        change: Option<(usize, Option<ChatSource>)>,
        revision: u64,
        readable: bool,
        entries: u64,
        direct_entries: Option<u64>,
        token: u64,
        initial: u64,
        largest_scan: Cell<usize>,
        submits: usize,
        direct: bool,
        snapshots: Cell<usize>,
        change_policy: Option<(usize, u64)>,
    }
    impl Native {
        fn new() -> Self {
            Self {
                source: source(1),
                calls: Cell::new(0),
                change: None,
                revision: 0,
                readable: true,
                entries: 0,
                direct_entries: None,
                token: 1,
                initial: 0,
                largest_scan: Cell::new(0),
                submits: 0,
                direct: false,
                snapshots: Cell::new(0),
                change_policy: None,
            }
        }
    }
    impl NativeChat for Native {
        fn source(&self) -> Option<ChatSource> {
            let call = self.calls.get() + 1;
            self.calls.set(call);
            if let Some((at, replacement)) = self.change
                && call >= at
            {
                return replacement;
            }
            Some(self.source)
        }
        fn catalogue(&self) -> Vec<Conversation> {
            let mut conversations = vec![Conversation::Nearby];
            if self.direct {
                conversations.push(Conversation::Direct(ConversationId::from_bytes([7; 16]).unwrap()));
            }
            conversations
        }
        fn snapshot(&self, conversation: Conversation) -> Option<NativeSnapshot> {
            let call = self.snapshots.get() + 1;
            self.snapshots.set(call);
            let revision = self
                .change_policy
                .filter(|_| matches!(conversation, Conversation::Direct(_)))
                .filter(|(at, _)| call >= *at)
                .map_or(self.revision, |(_, revision)| revision);
            Some(NativeSnapshot {
                title: Label::new("Synthetic nearby").unwrap(),
                stamp: [revision, 0, 0, 0],
                ready: true,
                can_send: true,
                can_read: self.readable,
                initial: NativeCursor {
                    token: self.token,
                    next: self.initial,
                },
            })
        }
        fn lines(&self, conversation: Conversation, cursor: NativeCursor, limit: usize) -> NativePage {
            self.largest_scan.set(self.largest_scan.get().max(limit));
            let entries = if matches!(conversation, Conversation::Direct(_)) {
                self.direct_entries.unwrap_or(self.entries)
            } else {
                self.entries
            };
            if cursor.token != self.token {
                return NativePage {
                    cursor: NativeCursor {
                        token: self.token,
                        next: entries,
                    },
                    gap: true,
                    lines: Vec::new(),
                };
            }
            let end = cursor.next.saturating_add(limit as u64).min(entries);
            let lines = (cursor.next..end)
                .map(|_| NativeLine {
                    sender: Label::new("Synthetic sender").unwrap(),
                    text: ChatText::new("Synthetic live text").unwrap(),
                    kind: LineKind::Incoming,
                    truncated: false,
                })
                .collect();
            NativePage {
                cursor: NativeCursor {
                    token: self.token,
                    next: end,
                },
                gap: false,
                lines,
            }
        }
        fn submit_literal(&mut self, key: ChatKey, _: &str) -> SendOutcome {
            assert_eq!(key.source, self.source);
            self.submits += 1;
            // Models the actual App API: the enqueue result is unavailable.
            SendOutcome::Uncertain
        }
    }

    fn send(source: ChatSource, revision: u64) -> ChatPacket {
        ChatPacket {
            version: VERSION,
            source,
            sequence: 1,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::Send(SendRequest {
                key: ChatKey {
                    source,
                    conversation: Conversation::Nearby,
                },
                request: 1,
                policy_revision: revision,
                text: ChatText::new("Synthetic literal text").unwrap(),
            })),
        }
    }
    fn poll(source: ChatSource, sequence: u64) -> ChatPacket {
        ChatPacket {
            version: VERSION,
            source,
            sequence,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::Poll { after_event: 0 }),
        }
    }

    fn retain_without_reset(history: &mut ChatHistory, policies: &mut BTreeMap<Conversation, NativeChatPolicy>, packets: &[ChatPacket]) {
        for packet in packets {
            match &packet.message {
                ChatMessage::Event(ChatEvent::Conversation(info)) => {
                    history.apply_policy(info.policy).unwrap();
                    policies.insert(info.policy.key.conversation, info.policy);
                }
                ChatMessage::Event(ChatEvent::Line(line)) => {
                    history.push(line.clone(), policies[&line.key.conversation]).unwrap();
                }
                ChatMessage::Event(ChatEvent::Gap { .. }) => panic!("ordinary history loss must not reset consumer policy or drafts"),
                _ => {}
            }
        }
    }

    #[test]
    fn host_chat_backend_rechecks_source_after_admission_before_any_native_mutation() {
        for replacement in [Some(source(2)), None] {
            let mut native = Native::new();
            let mut backend = WorkerChat::default();
            backend.sync(&native).unwrap();
            let revision = backend.states[&Conversation::Nearby].policy.revision;
            native.calls.set(0);
            // initial context, policy refresh, then the pre-submit boundary.
            native.change = Some((3, replacement));
            let error = backend.handle(&mut native, &send(source(1), revision)).unwrap_err();
            assert_eq!(
                error,
                if replacement.is_some() {
                    ChatError::Binding
                } else {
                    ChatError::Unavailable
                }
            );
            assert_eq!(native.submits, 0);
            assert_eq!(backend.received, 0);
            assert_eq!(backend.outgoing_sequence(), 0);
            assert!(backend.lines.is_empty() && backend.receipt.is_none());
            assert_eq!(backend.current_source(), replacement);
        }
    }

    #[test]
    fn host_chat_backend_lost_receipt_and_reconnect_never_resubmit_uncertain_send() {
        let mut native = Native::new();
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        let revision = backend.states[&Conversation::Nearby].policy.revision;
        let mut request = send(source(1), revision);
        let first = backend.handle(&mut native, &request).unwrap();
        request.sequence = 2; // Same request after a lost reply / socket reconnect.
        let duplicate = backend.handle(&mut native, &request).unwrap();
        assert_eq!(native.submits, 1);
        for reply in [&first[0], &duplicate[0]] {
            assert!(matches!(
                reply.message,
                ChatMessage::Event(ChatEvent::Receipt(SendReceipt {
                    outcome: SendOutcome::Uncertain,
                    ..
                }))
            ));
        }
        assert!(duplicate[0].sequence > first[0].sequence);
        backend.clear(); // Authority loss is different from a socket reconnect.
        assert_eq!(backend.sync(&native), Err(ChatError::Binding));
        assert_eq!(native.submits, 1);
        native.source = source(2);
        backend.sync(&native).unwrap();
        let next = backend.handle(&mut native, &poll(source(2), 3)).unwrap();
        assert!(
            next.iter()
                .all(|packet| packet.sequence > duplicate[0].sequence && packet.source == source(2))
        );
    }

    #[test]
    fn host_chat_backend_invalid_source_and_direction_cannot_poison_receiver_sequence() {
        let mut native = Native::new();
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        assert_eq!(backend.handle(&mut native, &poll(source(2), 9000)), Err(ChatError::Binding));
        let invalid = ChatPacket {
            version: VERSION,
            source: source(1),
            sequence: 9000,
            reply_to: 0,
            message: ChatMessage::Event(ChatEvent::Unsupported),
        };
        assert_eq!(backend.handle(&mut native, &invalid), Err(ChatError::Invalid));
        assert_eq!(backend.received, 0);
        let valid = backend.handle(&mut native, &poll(source(1), 1)).unwrap();
        assert!(!valid.is_empty());
        assert_eq!(backend.received, 1);
    }

    #[test]
    fn host_chat_backend_retention_scan_and_batches_are_bounded_and_restrictions_purge_text() {
        let mut native = Native::new();
        native.entries = 1024;
        let mut backend = WorkerChat::default();
        for _ in 0..40 {
            backend.sync(&native).unwrap();
        }
        assert_eq!(backend.lines.len(), MAX_RETAINED_LINES);
        assert_eq!(native.largest_scan.get(), SCAN_LINES);
        let replies = backend.handle(&mut native, &poll(source(1), 1)).unwrap();
        assert!(replies.len() <= 11);
        assert!(
            replies
                .iter()
                .filter(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Line(_))))
                .count()
                <= POLL_LINES
        );
        assert!(replies.iter().all(|packet| aurora_session::chat::encode(packet).is_ok()));
        native.revision += 1;
        native.readable = false;
        native.initial = native.entries;
        let restricted = backend.handle(&mut native, &poll(source(1), 2)).unwrap();
        assert!(backend.lines.is_empty());
        assert!(
            !restricted
                .iter()
                .any(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Line(_))))
        );
        assert!(
            restricted
                .iter()
                .any(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Conversation(ref info)) if !info.policy.can_read))
        );
    }

    #[test]
    fn host_chat_backend_nearby_overflow_preserves_direct_policy_history_and_draft() {
        let mut native = Native::new();
        native.direct = true;
        native.direct_entries = Some(1);
        let mut backend = WorkerChat::default();
        let mut history = ChatHistory::new(source(1)).unwrap();
        let mut policies = BTreeMap::new();
        let first = backend.handle(&mut native, &poll(source(1), 1)).unwrap();
        retain_without_reset(&mut history, &mut policies, &first);
        let direct = Conversation::Direct(ConversationId::from_bytes([7; 16]).unwrap());
        let direct_policy = policies[&direct];
        assert_eq!(history.lines(direct_policy).count(), 1);
        let mut drafts = ChatDrafts::new(source(1)).unwrap();
        let draft = ChatText::new("Synthetic inactive IM draft").unwrap();
        drafts.set(direct_policy.key, draft.clone()).unwrap();

        native.entries = 1024;
        for _ in 0..40 {
            backend.sync(&native).unwrap();
        }
        assert_eq!(backend.lines.len(), MAX_RETAINED_LINES);
        assert!(backend.lines.iter().all(|line| line.key.conversation == Conversation::Nearby));
        assert_eq!(backend.gap_through, 0);
        let mut request = poll(source(1), 2);
        request.message = ChatMessage::Command(ChatCommand::Poll { after_event: 1 });
        let replies = backend.handle(&mut native, &request).unwrap();
        assert!(replies.len() <= 11);
        retain_without_reset(&mut history, &mut policies, &replies);
        assert!(history.has_gap(), "skipped Nearby events still mark incomplete history");
        assert_eq!(policies[&direct], direct_policy);
        assert_eq!(backend.states[&direct].policy, direct_policy);
        assert_eq!(history.lines(direct_policy).count(), 1);
        assert_eq!(drafts.get(direct_policy.key), Some(&draft));
    }

    #[test]
    fn host_chat_backend_native_cursor_reset_marks_loss_without_policy_reset() {
        let mut native = Native::new();
        native.entries = 1;
        let mut backend = WorkerChat::default();
        let mut history = ChatHistory::new(source(1)).unwrap();
        let mut policies = BTreeMap::new();
        let first = backend.handle(&mut native, &poll(source(1), 1)).unwrap();
        retain_without_reset(&mut history, &mut policies, &first);
        let policy = policies[&Conversation::Nearby];
        assert!(!history.has_gap());

        // Mirrors the native adapter's cleared/replaced LineClock token:
        // the page skips to the current end and reports ordinary history loss.
        native.token += 1;
        backend.sync(&native).unwrap();
        assert_eq!(backend.states[&Conversation::Nearby].cursor.token, native.token);
        assert_eq!(backend.gap_through, 0);
        native.entries += 1;
        let mut request = poll(source(1), 2);
        request.message = ChatMessage::Command(ChatCommand::Poll { after_event: 1 });
        let replies = backend.handle(&mut native, &request).unwrap();
        retain_without_reset(&mut history, &mut policies, &replies);
        assert!(history.has_gap());
        assert_eq!(policies[&Conversation::Nearby], policy);
        assert_eq!(history.lines(policy).count(), 2);
    }

    #[test]
    fn host_chat_backend_each_exported_conversation_supplies_native_policy_before_its_lines() {
        let mut native = Native::new();
        native.direct = true;
        native.entries = 8;
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        let mut request = poll(source(1), 1);
        // The rotating metadata begins with Nearby, but the selected retained
        // lines belong to a Direct conversation the host has not seen yet.
        request.message = ChatMessage::Command(ChatCommand::Poll { after_event: 8 });
        let replies = backend.handle(&mut native, &request).unwrap();
        let mut policies = BTreeMap::new();
        let mut lines = 0;
        for reply in replies {
            match reply.message {
                ChatMessage::Event(ChatEvent::Conversation(info)) => {
                    policies.insert(info.policy.key.conversation, info.policy);
                }
                ChatMessage::Event(ChatEvent::Line(line)) => {
                    let policy = policies.get(&line.key.conversation).expect("same-batch native policy before line");
                    assert_eq!(line.key, policy.key);
                    assert_eq!(line.policy_revision, policy.revision);
                    assert!(policy.can_read && policy.ready && policy.lease_current);
                    assert!(matches!(line.key.conversation, Conversation::Direct(_)));
                    lines += 1;
                }
                _ => {}
            }
        }
        assert_eq!(lines, POLL_LINES);
    }

    #[test]
    fn host_chat_backend_policy_change_resets_host_cache_even_after_native_history_is_empty() {
        let mut native = Native::new();
        native.entries = 8;
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        let first = backend.handle(&mut native, &poll(source(1), 1)).unwrap();
        let cached = first
            .iter()
            .find_map(|packet| match &packet.message {
                ChatMessage::Event(ChatEvent::Line(line)) => Some(line.clone()),
                _ => None,
            })
            .expect("host retained a real exported line");
        let mut catch_up = poll(source(1), 2);
        catch_up.message = ChatMessage::Command(ChatCommand::Poll { after_event: 4 });
        let second = backend.handle(&mut native, &catch_up).unwrap();
        let after_event = second
            .iter()
            .filter_map(|packet| match &packet.message {
                ChatMessage::Event(ChatEvent::Line(line)) => Some(line.event),
                _ => None,
            })
            .max()
            .unwrap();
        assert_eq!(after_event, 8);
        // Model the consumer retaining a line after its producer no longer
        // retains a duplicate. A privacy reset must not depend on that copy.
        backend.lines.clear();
        native.entries = 0;
        native.revision += 1;
        native.readable = false;
        let mut request = poll(source(1), 3);
        request.message = ChatMessage::Command(ChatCommand::Poll { after_event });
        let replies = backend.handle(&mut native, &request).unwrap();
        assert!(backend.lines.is_empty());
        assert!(matches!(replies[0].message, ChatMessage::Event(ChatEvent::Gap { through_event }) if through_event > after_event));
        assert!(matches!(&replies[1].message, ChatMessage::Event(ChatEvent::Conversation(info))
            if info.policy.key == cached.key && info.policy.revision > cached.policy_revision && !info.policy.can_read));
        assert!(
            !replies
                .iter()
                .any(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Line(_))))
        );
        assert!(replies.len() <= 11);
        assert!(replies.iter().all(|packet| aurora_session::chat::encode(packet).is_ok()));
    }

    #[test]
    fn host_chat_backend_closed_conversation_resets_cached_titles_without_native_lines() {
        let mut native = Native::new();
        native.direct = true;
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        assert!(backend.lines.is_empty());
        let first = backend.handle(&mut native, &poll(source(1), 1)).unwrap();
        let second = backend.handle(&mut native, &poll(source(1), 2)).unwrap();
        assert!(
            !first
                .iter()
                .chain(&second)
                .any(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Gap { .. }))),
            "first policy discovery does not invalidate an existing cache"
        );
        assert!(
            second
                .iter()
                .any(|packet| matches!(&packet.message, ChatMessage::Event(ChatEvent::Conversation(info))
            if matches!(info.policy.key.conversation, Conversation::Direct(_))))
        );
        native.direct = false;
        let closed = backend.handle(&mut native, &poll(source(1), 3)).unwrap();
        assert!(matches!(closed[0].message, ChatMessage::Event(ChatEvent::Gap { .. })));
        assert!(closed.iter().all(
            |packet| !matches!(&packet.message, ChatMessage::Event(ChatEvent::Conversation(info))
            if matches!(info.policy.key.conversation, Conversation::Direct(_)))
        ));
    }

    #[test]
    fn host_chat_backend_late_line_preflight_reset_still_precedes_all_metadata() {
        let mut native = Native::new();
        native.direct = true;
        native.entries = 8;
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        native.snapshots.set(0);
        // Two sync snapshots, rotating Nearby metadata, then the selected
        // Direct line's preflight discovers a changed existing policy.
        native.change_policy = Some((4, 1));
        let mut request = poll(source(1), 1);
        request.message = ChatMessage::Command(ChatCommand::Poll { after_event: 8 });
        let replies = backend.handle(&mut native, &request).unwrap();
        assert!(matches!(replies[0].message, ChatMessage::Event(ChatEvent::Gap { through_event }) if through_event > 16));
        assert!(
            replies
                .iter()
                .skip(1)
                .any(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Conversation(_))))
        );
        assert!(
            !replies
                .iter()
                .any(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Line(_))))
        );
        assert!(replies.len() <= 11);
    }
}
