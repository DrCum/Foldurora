//! Controller-side live chat state. Authentication alone grants no chat source:
//! exact current control bindings/leases precede every receive high-water update.

use super::chat_transport::{HostIo, LaneError};
use super::transport::HostError;
use aurora_session::WorkerIdentity;
use aurora_session::chat::{
    ChatCommand, ChatError, ChatEvent, ChatHistory, ChatKey, ChatLine, ChatMessage, ChatPacket, ChatSource, ChatText, Conversation,
    ConversationInfo, MAX_RECEIPTS, SendOutcome, SendReceipt, SendRequest, VERSION,
};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(500);
const RECEIPT_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct HostChatSnapshot {
    pub source: ChatSource,
    pub authenticated: bool,
    pub live: bool,
    pub conversations: Vec<ConversationInfo>,
    pub lines: Vec<ChatLine>,
    pub receipts: Vec<SendReceipt>,
    pub gap: bool,
}

struct Pending {
    key: ChatKey,
    request: u64,
    since: Instant,
}

pub(crate) struct Peer {
    identity: WorkerIdentity,
    lease_epoch: u64,
    source: Option<ChatSource>,
    live: bool,
    io: Option<HostIo>,
    history: Option<ChatHistory>,
    conversations: BTreeMap<Conversation, ConversationInfo>,
    receipts: VecDeque<SendReceipt>,
    pending: Option<Pending>,
    received: u64,
    sent: u64,
    after_event: u64,
    poll_at: Instant,
    gap: bool,
    snapshot: Option<Arc<HostChatSnapshot>>,
}

impl Peer {
    pub fn new(identity: WorkerIdentity, lease_epoch: u64) -> Self {
        Self {
            identity,
            lease_epoch,
            source: None,
            live: false,
            io: None,
            history: None,
            conversations: BTreeMap::new(),
            receipts: VecDeque::new(),
            pending: None,
            received: 0,
            sent: 0,
            after_event: 0,
            poll_at: Instant::now(),
            gap: false,
            snapshot: None,
        }
    }

    pub fn accepting(&self) -> bool {
        self.live && self.source.is_some() && self.io.is_none()
    }

    pub fn snapshot(&self) -> Option<Arc<HostChatSnapshot>> {
        self.snapshot.clone()
    }

    /// Only the broker's accepted control roster supplies this source/liveness.
    pub fn set_control(&mut self, source: Option<ChatSource>, live: bool) {
        let source =
            source.filter(|source| source.validate().is_ok() && source.worker == self.identity && source.lease_epoch == self.lease_epoch);
        if (source == self.source && live == self.live) || (source.is_none() && !self.live && self.io.is_none()) {
            return;
        }
        if self
            .source
            .zip(source)
            .is_some_and(|(previous, next)| previous != next && next.session.generation <= previous.session.generation)
        {
            self.disconnect();
            self.live = false;
            self.publish();
            return;
        }
        if source.is_none() || !live {
            if self.live || self.io.is_some() {
                self.disconnect();
            }
            self.live = false;
            self.publish();
            return;
        }
        if self.source != source {
            self.disconnect();
            self.source = source;
            self.history = source.and_then(|source| ChatHistory::new(source).ok());
            self.conversations.clear();
            self.receipts.clear();
            self.after_event = 0;
            self.gap = false;
            // Socket and source changes never reset producer/receiver numbers.
        }
        self.live = true;
        self.publish();
    }

    pub fn attach(&mut self, io: HostIo) -> Result<(), HostError> {
        if !self.live || io.identity() != self.identity || io.lease_epoch() != self.lease_epoch || self.io.is_some() {
            return Err(HostError::Unavailable);
        }
        self.io = Some(io);
        self.poll_at = Instant::now();
        self.publish();
        Ok(())
    }

    fn receipt(&mut self, receipt: SendReceipt) {
        if let Some(existing) = self
            .receipts
            .iter_mut()
            .find(|existing| existing.request == receipt.request && existing.key == receipt.key)
        {
            *existing = receipt;
        } else {
            if self.receipts.len() == MAX_RECEIPTS {
                self.receipts.pop_front();
            }
            self.receipts.push_back(receipt);
        }
    }

    fn uncertain(&mut self) {
        if let Some(pending) = self.pending.take() {
            self.receipt(SendReceipt {
                key: pending.key,
                request: pending.request,
                outcome: SendOutcome::Uncertain,
            });
        }
    }

    fn disconnect(&mut self) {
        self.uncertain();
        self.io = None;
        self.conversations.clear();
        self.history = self.source.and_then(|source| ChatHistory::new(source).ok());
        self.gap = true;
        self.publish();
    }

    pub fn send(&mut self, key: ChatKey, policy_revision: u64, text: ChatText, request: u64, now: Instant) -> Result<(), HostError> {
        if self.source != Some(key.source) {
            return Err(HostError::Unavailable);
        }
        let admitted = SendReceipt {
            key,
            request,
            outcome: SendOutcome::Uncertain,
        };
        if self.pending.is_some() {
            self.receipt(admitted);
            self.publish();
            return Err(HostError::Busy);
        }
        let allowed = self.live
            && self.io.is_some()
            && self.source == Some(key.source)
            && self.conversations.get(&key.conversation).is_some_and(|info| {
                info.policy.key == key
                    && info.policy.revision == policy_revision
                    && info.policy.lease_current
                    && info.policy.ready
                    && info.policy.can_send
            });
        let send = SendRequest {
            key,
            request,
            policy_revision,
            text,
        };
        if !allowed || send.validate().is_err() {
            self.receipt(admitted);
            self.publish();
            return Err(HostError::Unavailable);
        }
        let sequence = self.sent.checked_add(1).ok_or(HostError::Unavailable)?;
        let packet = ChatPacket {
            version: VERSION,
            source: key.source,
            sequence,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::Send(send)),
        };
        let Some(io) = &mut self.io else {
            return Err(HostError::Unavailable);
        };
        if let Err(error) = io.queue(&packet) {
            self.receipt(admitted);
            if error != LaneError::Busy {
                self.disconnect();
            }
            self.publish();
            return Err(if error == LaneError::Busy {
                HostError::Busy
            } else {
                HostError::Unavailable
            });
        }
        self.sent = sequence;
        self.pending = Some(Pending { key, request, since: now });
        self.receipt(SendReceipt {
            outcome: SendOutcome::Pending,
            ..admitted
        });
        self.publish();
        Ok(())
    }

    /// No Send is retried here. Polls recover immutable receipts/history only.
    pub fn poll(&mut self, now: Instant) {
        if !self.live {
            return;
        }
        let Some(io) = &mut self.io else {
            return;
        };
        let incoming = match io.poll() {
            Ok(incoming) => incoming,
            Err(_) => {
                self.disconnect();
                return;
            }
        };
        let mut changed = !incoming.is_empty();
        for packet in incoming {
            let _ = self.observe(&packet);
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| now.saturating_duration_since(pending.since) >= RECEIPT_WAIT)
        {
            self.uncertain();
            changed = true;
        }
        if now >= self.poll_at {
            if let (Some(source), Some(sequence), Some(io)) = (self.source, self.sent.checked_add(1), self.io.as_mut()) {
                let packet = ChatPacket {
                    version: VERSION,
                    source,
                    sequence,
                    reply_to: 0,
                    message: ChatMessage::Command(ChatCommand::Poll {
                        after_event: self.after_event,
                    }),
                };
                match io.queue(&packet) {
                    Ok(()) => {
                        self.sent = sequence;
                        self.poll_at = now + POLL;
                    }
                    Err(LaneError::Busy) => {}
                    Err(_) => self.disconnect(),
                }
            }
        }
        if changed {
            self.publish();
        }
    }

    fn observe(&mut self, packet: &ChatPacket) -> Result<(), ChatError> {
        packet.validate()?;
        if !self.live || self.io.is_none() || self.source != Some(packet.source) {
            return Err(ChatError::Binding);
        }
        if packet.sequence <= self.received || packet.reply_to > self.sent {
            return Err(ChatError::Sequence);
        }
        let ChatMessage::Event(event) = &packet.message else {
            return Err(ChatError::Invalid);
        };
        match event {
            ChatEvent::Conversation(info) => {
                let history = self.history.as_mut().ok_or(ChatError::Unavailable)?;
                history.apply_policy(info.policy)?;
                if !self.conversations.contains_key(&info.policy.key.conversation)
                    && self.conversations.len() == aurora_session::chat::MAX_CONVERSATIONS
                {
                    return Err(ChatError::Capacity);
                }
                self.conversations.insert(info.policy.key.conversation, info.clone());
            }
            ChatEvent::Line(line) => {
                let policy = self.conversations.get(&line.key.conversation).ok_or(ChatError::Unavailable)?.policy;
                self.history.as_mut().ok_or(ChatError::Unavailable)?.push(line.clone(), policy)?;
                self.after_event = self.after_event.max(line.event);
            }
            ChatEvent::Receipt(receipt) => {
                let existing = self
                    .receipts
                    .iter()
                    .find(|existing| existing.request == receipt.request && existing.key == receipt.key)
                    .ok_or(ChatError::UnknownReceipt)?;
                if !matches!(existing.outcome, SendOutcome::Pending | SendOutcome::Uncertain) && existing.outcome != receipt.outcome {
                    return Err(ChatError::FinalReceipt);
                }
                self.receipt(*receipt);
                if receipt.outcome != SendOutcome::Pending
                    && self
                        .pending
                        .as_ref()
                        .is_some_and(|pending| pending.key == receipt.key && pending.request == receipt.request)
                {
                    self.pending = None;
                }
            }
            ChatEvent::Gap { through_event } => {
                if *through_event > self.after_event {
                    // Native rule changes use a gap before fresh metadata.
                    // Purge old text AND titles even when the worker's own
                    // rolling buffer no longer contains our earlier lines.
                    let mut history = ChatHistory::new(packet.source)?;
                    history.note_gap(packet.source, *through_event)?;
                    self.history = Some(history);
                    self.conversations.clear();
                    self.after_event = *through_event;
                    self.gap = true;
                }
            }
            ChatEvent::Unsupported => {
                self.conversations.clear();
                self.history = ChatHistory::new(packet.source).ok();
                self.uncertain();
            }
        }
        // A wrong/stale source, policy or receipt cannot poison this number.
        self.received = packet.sequence;
        Ok(())
    }

    fn publish(&mut self) {
        let Some(source) = self.source else {
            self.snapshot = None;
            return;
        };
        let authenticated = self.io.is_some();
        let conversations = if self.live && authenticated {
            self.conversations.values().cloned().collect()
        } else {
            Vec::new()
        };
        let mut lines = Vec::new();
        if self.live
            && authenticated
            && let Some(history) = &self.history
        {
            for info in self.conversations.values() {
                lines.extend(history.lines(info.policy).cloned());
            }
            lines.sort_by_key(|line| line.event);
        }
        let next = HostChatSnapshot {
            source,
            authenticated,
            live: self.live,
            conversations,
            lines,
            receipts: self.receipts.iter().copied().collect(),
            gap: self.gap || self.history.as_ref().is_some_and(ChatHistory::has_gap),
        };
        if self.snapshot.as_ref().is_none_or(|previous| **previous != next) {
            self.snapshot = Some(Arc::new(next));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_host::chat_transport::{ChatSecret, Listener, Permit, WorkerIo};
    use aurora_session::chat::{LineKind, NativeChatPolicy};
    use aurora_session::{AccountKey, GridKey, HostId, InstallationId, Label, SessionId, SessionIdentity, WorkerId};

    pub(super) fn source() -> ChatSource {
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
                generation: 1,
            },
            lease_epoch: 7,
        }
    }

    fn policy(source: ChatSource, revision: u64, read: bool) -> ConversationInfo {
        ConversationInfo {
            title: Label::new("Synthetic nearby").unwrap(),
            policy: NativeChatPolicy {
                key: ChatKey {
                    source,
                    conversation: Conversation::Nearby,
                },
                revision,
                lease_current: true,
                ready: true,
                can_send: true,
                can_read: read,
            },
        }
    }

    fn event(source: ChatSource, sequence: u64, event: ChatEvent) -> ChatPacket {
        ChatPacket {
            version: VERSION,
            source,
            sequence,
            reply_to: 0,
            message: ChatMessage::Event(event),
        }
    }

    pub(super) fn connected() -> (Peer, WorkerIo) {
        let source = source();
        let secret: ChatSecret = serde_json::from_str(&format!("\"{}\"", "07".repeat(32))).unwrap();
        let permit = Permit {
            identity: source.worker,
            lease_epoch: source.lease_epoch,
            secret,
        };
        let mut listener = Listener::bind().unwrap();
        let mut worker = WorkerIo::connect(listener.port(), permit.clone()).unwrap();
        let mut host = None;
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            worker.poll().unwrap();
            if host.is_none() {
                host = listener.poll(std::slice::from_ref(&permit)).unwrap();
            }
            if let Some(host) = &mut host {
                host.poll().unwrap();
            }
            if worker.ready() {
                break;
            }
            assert!(Instant::now() < deadline, "owned loopback authentication timed out");
            std::thread::yield_now();
        }
        let mut peer = Peer::new(source.worker, source.lease_epoch);
        peer.set_control(Some(source), true);
        peer.attach(host.unwrap()).unwrap();
        (peer, worker)
    }

    #[test]
    fn chat_broker_rejects_wrong_source_before_receive_high_water() {
        let (mut peer, _worker) = connected();
        let source = source();
        let mut wrong = source;
        wrong.session.account = AccountKey::from_bytes([9; 32]).unwrap();
        assert_eq!(
            peer.observe(&event(wrong, 1000, ChatEvent::Conversation(policy(wrong, 1, true)))),
            Err(ChatError::Binding)
        );
        assert_eq!(peer.received, 0);
        peer.observe(&event(source, 1, ChatEvent::Conversation(policy(source, 1, true))))
            .unwrap();
        assert_eq!(peer.received, 1);
        peer.set_control(None, false);
        assert_eq!(
            peer.observe(&event(source, 1001, ChatEvent::Conversation(policy(source, 2, true)))),
            Err(ChatError::Binding)
        );
        assert_eq!(peer.received, 1, "an expired control lease cannot poison a future receive sequence");
    }

    #[test]
    fn chat_broker_retains_only_exact_current_native_policy_lines_and_purges_on_revocation() {
        let (mut peer, _worker) = connected();
        let source = source();
        let first = policy(source, 1, true);
        peer.observe(&event(source, 1, ChatEvent::Conversation(first.clone()))).unwrap();
        let line = ChatLine {
            key: first.policy.key,
            event: 1,
            policy_revision: 1,
            sender: Label::new("Synthetic").unwrap(),
            text: ChatText::new("bounded live text").unwrap(),
            kind: LineKind::Incoming,
            truncated: false,
        };
        peer.observe(&event(source, 2, ChatEvent::Line(line.clone()))).unwrap();
        peer.publish();
        assert_eq!(peer.snapshot().unwrap().lines.len(), 1);
        peer.observe(&event(source, 3, ChatEvent::Conversation(policy(source, 2, false))))
            .unwrap();
        peer.publish();
        assert!(peer.snapshot().unwrap().lines.is_empty());
        assert_eq!(
            peer.observe(&event(source, 1000, ChatEvent::Line(ChatLine { event: 2, ..line }))),
            Err(ChatError::StalePolicy)
        );
        assert_eq!(peer.received, 3, "stale policy data cannot advance the high-water mark");
    }

    #[test]
    fn chat_broker_natural_retention_holes_keep_unrelated_conversation_policy() {
        let (mut peer, _worker) = connected();
        let source = source();
        let nearby = policy(source, 1, true);
        let mut direct = policy(source, 2, true);
        direct.policy.key.conversation = Conversation::Direct(aurora_session::ConversationId::from_bytes([11; 16]).unwrap());
        peer.observe(&event(source, 1, ChatEvent::Conversation(nearby.clone()))).unwrap();
        peer.observe(&event(source, 2, ChatEvent::Conversation(direct.clone()))).unwrap();
        peer.observe(&event(
            source,
            3,
            ChatEvent::Line(ChatLine {
                key: nearby.policy.key,
                event: 300,
                policy_revision: 1,
                sender: Label::new("Synthetic").unwrap(),
                text: ChatText::new("Latest nearby").unwrap(),
                kind: LineKind::Incoming,
                truncated: false,
            }),
        ))
        .unwrap();
        peer.publish();
        let snapshot = peer.snapshot().unwrap();
        assert!(snapshot.gap);
        assert!(snapshot.conversations.iter().any(|info| info == &direct));
        assert_eq!(snapshot.lines.len(), 1);
    }

    #[test]
    fn chat_broker_gap_revokes_all_cached_text_and_titles_before_new_policy() {
        let (mut peer, _worker) = connected();
        let source = source();
        let info = policy(source, 1, true);
        peer.observe(&event(source, 1, ChatEvent::Conversation(info.clone()))).unwrap();
        peer.observe(&event(
            source,
            2,
            ChatEvent::Line(ChatLine {
                key: info.policy.key,
                event: 1,
                policy_revision: 1,
                sender: Label::new("Synthetic").unwrap(),
                text: ChatText::new("old live text").unwrap(),
                kind: LineKind::Incoming,
                truncated: false,
            }),
        ))
        .unwrap();
        peer.observe(&event(source, 3, ChatEvent::Gap { through_event: 2 })).unwrap();
        peer.publish();
        let snapshot = peer.snapshot().unwrap();
        assert!(snapshot.lines.is_empty() && snapshot.conversations.is_empty() && snapshot.gap);
        peer.observe(&event(source, 4, ChatEvent::Conversation(policy(source, 2, false))))
            .unwrap();
        peer.observe(&event(source, 5, ChatEvent::Gap { through_event: 2 })).unwrap();
        assert_eq!(peer.conversations.len(), 1, "a repeated gap is idempotent");
    }

    #[test]
    fn chat_broker_marks_a_queued_mutation_uncertain_on_socket_loss_and_never_requeues_it() {
        let (mut peer, _worker) = connected();
        let source = source();
        let info = policy(source, 1, true);
        peer.observe(&event(source, 1, ChatEvent::Conversation(info.clone()))).unwrap();
        peer.send(info.policy.key, 1, ChatText::new("literal /text").unwrap(), 42, Instant::now())
            .unwrap();
        assert_eq!(peer.sent, 1);
        assert_eq!(peer.snapshot().unwrap().receipts[0].outcome, SendOutcome::Pending);
        peer.disconnect();
        let snapshot = peer.snapshot().unwrap();
        assert!(!snapshot.authenticated);
        assert_eq!(
            snapshot.receipts[0],
            SendReceipt {
                key: info.policy.key,
                request: 42,
                outcome: SendOutcome::Uncertain
            }
        );
        peer.poll(Instant::now() + RECEIPT_WAIT);
        assert_eq!(peer.sent, 1, "socket failure is never permission to replay a Send");
        assert!(peer.pending.is_none());
    }

    #[test]
    fn chat_broker_receipts_bind_exact_key_and_request_and_idle_snapshots_are_immutable() {
        let (mut peer, _worker) = connected();
        let source = source();
        let info = policy(source, 1, true);
        peer.observe(&event(source, 1, ChatEvent::Conversation(info.clone()))).unwrap();
        peer.publish();
        let before = peer.snapshot().unwrap();
        peer.publish();
        assert!(Arc::ptr_eq(&before, &peer.snapshot().unwrap()));
        peer.send(info.policy.key, 1, ChatText::new("literal").unwrap(), 43, Instant::now())
            .unwrap();
        let bad = SendReceipt {
            key: info.policy.key,
            request: 44,
            outcome: SendOutcome::Uncertain,
        };
        assert_eq!(
            peer.observe(&event(source, 1000, ChatEvent::Receipt(bad))),
            Err(ChatError::UnknownReceipt)
        );
        assert_eq!(peer.received, 1);
        let good = SendReceipt { request: 43, ..bad };
        peer.observe(&event(source, 2, ChatEvent::Receipt(good))).unwrap();
        assert!(peer.pending.is_none());
        peer.set_control(None, false);
        let ended = peer.snapshot().unwrap();
        assert!(!ended.live && !ended.authenticated && ended.conversations.is_empty() && ended.lines.is_empty());
    }
}

#[cfg(test)]
mod live_path_tests {
    use super::*;
    use crate::session_host::chat::{NativeChat, NativeCursor, NativePage, NativeSnapshot, WorkerChat};

    struct Native {
        source: ChatSource,
        submissions: usize,
    }

    impl NativeChat for Native {
        fn source(&self) -> Option<ChatSource> {
            Some(self.source)
        }
        fn catalogue(&self) -> Vec<Conversation> {
            vec![Conversation::Nearby]
        }
        fn snapshot(&self, _: Conversation) -> Option<NativeSnapshot> {
            Some(NativeSnapshot {
                title: aurora_session::Label::new("Synthetic nearby").unwrap(),
                stamp: [1; 4],
                ready: true,
                can_send: true,
                can_read: true,
                initial: NativeCursor::default(),
            })
        }
        fn lines(&self, _: Conversation, cursor: NativeCursor, _: usize) -> NativePage {
            NativePage {
                cursor,
                gap: false,
                lines: Vec::new(),
            }
        }
        fn submit_literal(&mut self, key: ChatKey, text: &str) -> SendOutcome {
            assert_eq!(key.source, self.source);
            assert_eq!(key.conversation, Conversation::Nearby);
            assert_eq!(text, "literal /text");
            self.submissions += 1;
            SendOutcome::Uncertain
        }
    }

    #[test]
    fn authenticated_socket_broker_and_native_ledger_produce_only_the_exact_send_receipt() {
        let (mut peer, mut worker) = super::tests::connected();
        let source = super::tests::source();
        let mut backend = WorkerChat::default();
        let mut native = Native { source, submissions: 0 };
        let drive = |peer: &mut Peer,
                     worker: &mut crate::session_host::chat_transport::WorkerIo,
                     backend: &mut WorkerChat,
                     native: &mut Native,
                     receipt: bool| {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                peer.poll(Instant::now());
                for command in worker.poll().unwrap() {
                    for event in backend.handle(native, &command).unwrap() {
                        worker.queue(&event).unwrap();
                    }
                }
                let ready = peer.snapshot().is_some_and(|snapshot| {
                    if receipt {
                        snapshot
                            .receipts
                            .iter()
                            .any(|receipt| receipt.request == 42 && receipt.outcome == SendOutcome::Uncertain)
                    } else {
                        snapshot.authenticated && snapshot.live && !snapshot.conversations.is_empty()
                    }
                });
                if ready {
                    break;
                }
                assert!(Instant::now() < deadline, "owned authenticated chat fixture did not progress");
                std::thread::yield_now();
            }
        };
        drive(&mut peer, &mut worker, &mut backend, &mut native, false);
        let info = peer.snapshot().unwrap().conversations[0].clone();
        peer.send(
            info.policy.key,
            info.policy.revision,
            ChatText::new("literal /text").unwrap(),
            42,
            Instant::now(),
        )
        .unwrap();
        drive(&mut peer, &mut worker, &mut backend, &mut native, true);
        assert_eq!(native.submissions, 1);
        assert_eq!(peer.snapshot().unwrap().receipts[0].key.source, source);
        peer.disconnect();
        assert!(peer.snapshot().unwrap().lines.is_empty());
        assert_eq!(native.submissions, 1, "transport loss cannot resubmit a mutation");
    }
}
