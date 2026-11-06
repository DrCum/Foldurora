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
        if !self.states.contains_key(&conversation) && self.states.len() == MAX_CONVERSATIONS {
            return Err(ChatError::Capacity);
        }
        self.next_policy = self.next_policy.checked_add(1).ok_or(ChatError::Sequence)?;
        let key = ChatKey { source, conversation };
        let previous = self.lines.len();
        self.lines.retain(|line| line.key != key);
        if previous != self.lines.len() {
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
        self.states.retain(|conversation, _| known.contains(conversation));
        let before = self.lines.len();
        self.lines.retain(|line| known.contains(&line.key.conversation));
        if before != self.lines.len() {
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
                self.gap()?;
            }
            for line in page.lines.into_iter().take(SCAN_LINES) {
                let event = self.event()?;
                if self.lines.len() == MAX_RETAINED_LINES {
                    if let Some(old) = self.lines.pop_front() {
                        self.gap_through = self.gap_through.max(old.event);
                    }
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
                let mut result = Vec::new();
                let mut advertised = BTreeSet::new();
                let keys: Vec<_> = self.states.keys().copied().collect();
                if !keys.is_empty() {
                    let key = keys[self.metadata % keys.len()];
                    self.metadata = self.metadata.wrapping_add(1);
                    self.refresh(native, key)?;
                    let state = self.states.get(&key).ok_or(ChatError::Unavailable)?;
                    let info = ConversationInfo {
                        policy: state.policy,
                        title: state.snapshot.title.clone(),
                    };
                    result.push(self.packet(packet.sequence, ChatEvent::Conversation(info))?);
                    advertised.insert(key);
                }
                // Recheck current native restrictions on every exported line,
                // including conversations outside this tick's scan rotation.
                if *after_event < self.gap_through {
                    result.push(self.packet(
                        packet.sequence,
                        ChatEvent::Gap {
                            through_event: self.gap_through,
                        },
                    )?);
                }
                let after = (*after_event).max(self.gap_through);
                let selected: Vec<_> = self
                    .lines
                    .iter()
                    .filter(|line| line.event > after)
                    .take(POLL_LINES)
                    .cloned()
                    .collect();
                for line in selected {
                    self.refresh(native, line.key.conversation)?;
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
                            result.push(self.packet(packet.sequence, ChatEvent::Conversation(info))?);
                        }
                        result.push(self.packet(packet.sequence, ChatEvent::Line(line))?);
                    }
                }
                if let Some(receipt) = self.receipt {
                    result.push(self.packet(packet.sequence, ChatEvent::Receipt(receipt))?);
                }
                if self.context(native)? != source {
                    return Err(ChatError::Binding);
                }
                self.received = packet.sequence;
                Ok(result)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::chat::{LineKind, SendRequest};
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
        initial: u64,
        largest_scan: Cell<usize>,
        submits: usize,
        direct: bool,
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
                initial: 0,
                largest_scan: Cell::new(0),
                submits: 0,
                direct: false,
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
        fn snapshot(&self, _: Conversation) -> Option<NativeSnapshot> {
            Some(NativeSnapshot {
                title: Label::new("Synthetic nearby").unwrap(),
                stamp: [self.revision, 0, 0, 0],
                ready: true,
                can_send: true,
                can_read: self.readable,
                initial: NativeCursor {
                    token: 1,
                    next: self.initial,
                },
            })
        }
        fn lines(&self, _: Conversation, cursor: NativeCursor, limit: usize) -> NativePage {
            self.largest_scan.set(self.largest_scan.get().max(limit));
            let end = cursor.next.saturating_add(limit as u64).min(self.entries);
            let lines = (cursor.next..end)
                .map(|_| NativeLine {
                    sender: Label::new("Synthetic sender").unwrap(),
                    text: ChatText::new("Synthetic live text").unwrap(),
                    kind: LineKind::Incoming,
                    truncated: false,
                })
                .collect();
            NativePage {
                cursor: NativeCursor { token: 1, next: end },
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
}
