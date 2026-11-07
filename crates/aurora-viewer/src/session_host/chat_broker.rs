//! Controller-side live chat state. Authentication alone grants no chat source:
//! exact current control bindings/leases precede every receive high-water update.

use super::chat_transport::{HostIo, LaneError};
use super::review::{self, Delivery, Status};
use super::transport::HostError;
use aurora_session::WorkerIdentity;
use aurora_session::chat::{
    ChatCommand, ChatError, ChatEvent, ChatHistory, ChatKey, ChatLine, ChatMessage, ChatPacket, ChatSource, ChatText, Conversation,
    ConversationInfo, MAX_RECEIPTS, SendOutcome, SendReceipt, SendRequest, VERSION,
};
use aurora_session::chat::{MAX_READ_RECEIPTS, MarkReadRequest, ReadMetadata, ReadOutcome, ReadReceiptQuery};
use aurora_session::chat::{MAX_REVIEW_RECEIPTS, ReviewCapture, ReviewConversationRequest, ReviewReceiptQuery};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadStatus {
    Pending,
    Uncertain,
    Completed(ReadOutcome),
    Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReadOperation {
    pub request: MarkReadRequest,
    pub status: ReadStatus,
}
struct PendingRead {
    request: MarkReadRequest,
    since: Instant,
    query_at: Instant,
}

struct PendingReview {
    request: ReviewConversationRequest,
    since: Instant,
    query_at: Instant,
    queries: u8,
}

struct ReviewRecord {
    request: ReviewConversationRequest,
    status: Status,
}

use std::sync::Arc;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(500);
const RECEIPT_WAIT: Duration = Duration::from_secs(5);
const REVIEW_QUERY_LIMIT: u8 = 10;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct HostChatSnapshot {
    pub source: ChatSource,
    pub authenticated: bool,
    pub live: bool,
    pub conversations: Vec<ConversationInfo>,
    pub lines: Vec<ChatLine>,
    pub receipts: Vec<SendReceipt>,
    pub reads: BTreeMap<Conversation, ReadMetadata>,
    pub read_operations: Vec<ReadOperation>,
    pub gap: bool,
}

impl HostChatSnapshot {
    pub fn permits_review(&self, target: ReviewCapture) -> bool {
        review::current_target(
            target,
            Some(self.source),
            self.live,
            self.authenticated,
            self.conversations
                .iter()
                .find(|info| info.policy.key == target.key)
                .map(|info| info.policy),
            self.reads.get(&target.key.conversation).copied(),
        )
    }
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
    reads: BTreeMap<Conversation, ReadMetadata>,
    read_operations: VecDeque<ReadOperation>,
    read_pending: Option<PendingRead>,
    review_operations: VecDeque<ReviewRecord>,
    review_pending: Option<PendingReview>,
    review_high_water: u64,
    review_watch: Option<(ReviewCapture, bool)>,
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
            reads: BTreeMap::new(),
            read_operations: VecDeque::new(),
            read_pending: None,
            review_operations: VecDeque::new(),
            review_pending: None,
            review_high_water: 0,
            review_watch: None,
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
            self.review_pending = None;
            self.review_operations.clear();
            self.review_watch = None;
            self.live = false;
            self.publish();
            return;
        }
        if source.is_none() || !live {
            self.review_watch = None;
            self.review_pending = None;
            self.review_operations.clear();
            self.read_pending = None;
            self.read_operations.clear();
            self.reads.clear();
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
            self.read_operations.clear();
            self.read_pending = None;
            self.review_pending = None;
            self.review_operations.clear();
            self.review_watch = None;
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
        if let Some((_, valid)) = &mut self.review_watch {
            *valid = false;
        }
        self.uncertain();
        self.reads.clear();
        if let Some(pending) = &self.read_pending {
            let request = pending.request;
            self.update_read_status(request, ReadStatus::Uncertain);
        }
        if let Some(pending) = &self.review_pending {
            self.update_review_status(pending.request, Status::Uncertain);
        }
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

    fn update_read_status(&mut self, request: MarkReadRequest, status: ReadStatus) {
        if let Some(operation) = self.read_operations.iter_mut().find(|operation| operation.request == request) {
            operation.status = status;
        }
    }

    fn retain_read_operation(&mut self, operation: ReadOperation) {
        if self.read_operations.len() == MAX_READ_RECEIPTS {
            // A blocked old uncertain identity may be older than many local
            // rejections. Never evict it to report one of those rejections.
            let active = self.read_pending.as_ref().map(|pending| pending.request);
            if let Some(index) = self.read_operations.iter().position(|old| Some(old.request) != active) {
                self.read_operations.remove(index);
            } else {
                return;
            }
        }
        self.read_operations.push_back(operation);
    }

    pub fn reject_read(&mut self, request: MarkReadRequest) {
        if self.source == Some(request.key.source)
            && !self
                .read_operations
                .iter()
                .any(|operation| operation.request.request == request.request)
        {
            self.retain_read_operation(ReadOperation {
                request,
                status: ReadStatus::Unavailable,
            });
            self.publish();
        }
    }

    pub fn admit_read(&mut self, request: MarkReadRequest, now: Instant) -> Result<(), HostError> {
        let result = self.mark_read(request, now);
        if result.is_err() {
            self.reject_read(request);
        }
        result
    }

    pub fn mark_read(&mut self, request: MarkReadRequest, now: Instant) -> Result<(), HostError> {
        if self.read_pending.is_some() {
            return Err(HostError::Busy);
        }
        let allowed = self.live
            && self.io.is_some()
            && self.source == Some(request.key.source)
            && self.conversations.get(&request.key.conversation).is_some_and(|info| {
                info.policy.key == request.key
                    && info.policy.revision == request.policy_revision
                    && info.policy.lease_current
                    && info.policy.ready
                    && info.policy.can_read
            })
            && self.reads.get(&request.key.conversation).is_some_and(|read| {
                read.can_mark_read && read.observed == request.observed && read.unread_revision == request.unread_revision
            });
        if !allowed || request.validate().is_err() {
            return Err(HostError::Unavailable);
        }
        let sequence = self.sent.checked_add(1).ok_or(HostError::Unavailable)?;
        let packet = ChatPacket {
            version: VERSION,
            source: request.key.source,
            sequence,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::MarkRead(request)),
        };
        self.io.as_mut().ok_or(HostError::Unavailable)?.queue(&packet).map_err(|error| {
            if error == LaneError::Busy {
                HostError::Busy
            } else {
                HostError::Unavailable
            }
        })?;
        self.sent = sequence;
        self.retain_read_operation(ReadOperation {
            request,
            status: ReadStatus::Pending,
        });
        self.read_pending = Some(PendingRead {
            request,
            since: now,
            query_at: now + POLL,
        });
        self.publish();
        Ok(())
    }

    fn poll_read_receipt(&mut self, now: Instant) -> bool {
        let Some(pending) = &self.read_pending else { return false };
        let request = pending.request;
        let expired = now.saturating_duration_since(pending.since) >= RECEIPT_WAIT;
        let due = now >= pending.query_at;
        if expired {
            self.update_read_status(request, ReadStatus::Uncertain);
        }
        if due && let (Some(sequence), Some(io)) = (self.sent.checked_add(1), self.io.as_mut()) {
            let packet = ChatPacket {
                version: VERSION,
                source: request.key.source,
                sequence,
                reply_to: 0,
                message: ChatMessage::Command(ChatCommand::QueryReadReceipt(ReadReceiptQuery {
                    key: request.key,
                    request: request.request,
                })),
            };
            if io.queue(&packet).is_ok() {
                self.sent = sequence;
                if let Some(pending) = &mut self.read_pending {
                    pending.query_at = now + POLL;
                }
            }
        }
        expired
    }

    pub fn review_allowed(&self, target: ReviewCapture) -> bool {
        review::current_target(
            target,
            self.source,
            self.live,
            self.io.is_some(),
            self.conversations.get(&target.key.conversation).map(|info| info.policy),
            self.reads.get(&target.key.conversation).copied(),
        )
    }

    pub fn watch_review(&mut self, target: Option<ReviewCapture>) {
        self.review_watch = target.map(|target| (target, self.review_allowed(target)));
    }

    pub fn watched_review_current(&self, target: ReviewCapture) -> bool {
        self.review_watch == Some((target, true)) && self.review_allowed(target)
    }

    pub(super) fn review_delivery(&self, request: ReviewConversationRequest) -> Option<Delivery> {
        let record = self.review_operations.iter().find(|record| record.request == request)?;
        Some(Delivery {
            request,
            status: record.status,
            recovering: self.review_pending.as_ref().is_some_and(|pending| pending.request == request),
        })
    }

    fn update_review_status(&mut self, request: ReviewConversationRequest, status: Status) {
        if let Some(record) = self.review_operations.iter_mut().find(|record| record.request == request) {
            record.status = status;
        }
    }

    /// One immediate bounded admission only. Busy never creates a delayed open;
    /// reconnects and receipt recovery never call this method on an old ticket.
    pub fn admit_review(&mut self, request: ReviewConversationRequest, now: Instant) -> Result<(), HostError> {
        if request.validate().is_err() || request.request <= self.review_high_water {
            return Err(HostError::Unavailable);
        }
        // Even a refused local admission is terminal for its original ticket.
        // Channel room appearing later cannot turn Busy into a delayed open.
        self.review_high_water = request.request;
        if self.review_pending.is_some() {
            return Err(HostError::Busy);
        }
        if !self.review_allowed(request.target) {
            return Err(HostError::Unavailable);
        }
        let sequence = self.sent.checked_add(1).ok_or(HostError::Unavailable)?;
        let packet = ChatPacket {
            version: VERSION,
            source: request.target.key.source,
            sequence,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::ReviewConversation(request)),
        };
        self.io.as_mut().ok_or(HostError::Unavailable)?.queue(&packet).map_err(|error| {
            if error == LaneError::Busy {
                HostError::Busy
            } else {
                HostError::Unavailable
            }
        })?;
        self.sent = sequence;
        if self.review_operations.len() == MAX_REVIEW_RECEIPTS {
            self.review_operations.pop_front();
        }
        self.review_operations.push_back(ReviewRecord {
            request,
            status: Status::Dispatched,
        });
        self.review_pending = Some(PendingReview {
            request,
            since: now,
            query_at: now + POLL,
            queries: 0,
        });
        self.review_watch = None;
        Ok(())
    }

    fn expire_review(&mut self, now: Instant) {
        if let Some(pending) = &self.review_pending
            && now.saturating_duration_since(pending.since) >= RECEIPT_WAIT
        {
            self.update_review_status(pending.request, Status::Uncertain);
            self.review_pending = None;
        }
    }

    fn poll_review_receipt(&mut self, now: Instant) {
        self.expire_review(now);
        let Some(pending) = &self.review_pending else { return };
        if now < pending.query_at || pending.queries >= REVIEW_QUERY_LIMIT || !self.review_allowed(pending.request.target) {
            return;
        }
        let request = pending.request;
        if let Some(pending) = &mut self.review_pending {
            pending.queries += 1;
            pending.query_at = now + POLL;
        }
        if let (Some(sequence), Some(io)) = (self.sent.checked_add(1), self.io.as_mut()) {
            let packet = ChatPacket {
                version: VERSION,
                source: request.target.key.source,
                sequence,
                reply_to: 0,
                message: ChatMessage::Command(ChatCommand::QueryReviewReceipt(ReviewReceiptQuery {
                    key: request.target.key,
                    request: request.request,
                })),
            };
            if io.queue(&packet).is_ok() {
                self.sent = sequence;
            }
        }
    }

    /// No Send or Review is retried. Polls recover immutable receipts/history.
    pub fn poll(&mut self, now: Instant) {
        self.expire_review(now);
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
        if now >= self.poll_at
            && let (Some(source), Some(sequence), Some(io)) = (self.source, self.sent.checked_add(1), self.io.as_mut())
        {
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
        changed |= self.poll_read_receipt(now);
        self.poll_review_receipt(now);
        if changed {
            self.publish();
        }
    }

    fn conversation(&mut self, info: &ConversationInfo) -> Result<(), ChatError> {
        let history = self.history.as_mut().ok_or(ChatError::Unavailable)?;
        history.apply_policy(info.policy)?;
        if !self.conversations.contains_key(&info.policy.key.conversation)
            && self.conversations.len() == aurora_session::chat::MAX_CONVERSATIONS
        {
            return Err(ChatError::Capacity);
        }
        let changed = self
            .conversations
            .get(&info.policy.key.conversation)
            .is_none_or(|old| old.policy != info.policy);
        if changed || !info.policy.lease_current || !info.policy.ready || !info.policy.can_read {
            self.reads.remove(&info.policy.key.conversation);
        }
        self.conversations.insert(info.policy.key.conversation, info.clone());
        Ok(())
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
                self.conversation(info)?;
            }
            ChatEvent::ReadBatch(rows) => {
                for row in rows {
                    let info = row.info(packet.source);
                    self.conversation(&info)?;
                    if let Some(read) = row.read {
                        self.reads.insert(row.conversation, read);
                    } else {
                        self.reads.remove(&row.conversation);
                    }
                }
            }
            ChatEvent::ReadReceipt(reply) => {
                let operation = self
                    .read_operations
                    .iter()
                    .find(|operation| operation.request.key == reply.query.key && operation.request.request == reply.query.request)
                    .ok_or(ChatError::UnknownReceipt)?;
                let request = operation.request;
                if let Some(receipt) = reply.receipt
                    && !receipt.matches(request)
                {
                    return Err(ChatError::RequestChanged);
                }
                let status = reply
                    .receipt
                    .map_or(ReadStatus::Unavailable, |receipt| ReadStatus::Completed(receipt.outcome));
                if !matches!(operation.status, ReadStatus::Pending | ReadStatus::Uncertain) && operation.status != status {
                    return Err(ChatError::FinalReceipt);
                }
                self.update_read_status(request, status);
                // This receipt can invalidate only its own observed metadata.
                // Never clear a newer arrival's metadata with an old receipt.
                if self
                    .reads
                    .get(&request.key.conversation)
                    .is_some_and(|read| read.observed == request.observed && read.unread_revision == request.unread_revision)
                {
                    self.reads.remove(&request.key.conversation);
                }
                if self.read_pending.as_ref().is_some_and(|pending| pending.request == request) {
                    self.read_pending = None;
                }
            }
            ChatEvent::ReviewReceipt(reply) => {
                let record = self
                    .review_operations
                    .iter()
                    .find(|record| record.request.target.key == reply.query.key && record.request.request == reply.query.request)
                    .ok_or(ChatError::UnknownReceipt)?;
                let request = record.request;
                reply.validate_for(request)?;
                if !self.review_allowed(request.target) {
                    return Err(ChatError::Denied);
                }
                let status = reply
                    .receipt
                    .map_or(Status::Uncertain, |receipt| Status::Completed(receipt.outcome));
                if matches!(record.status, Status::Completed(_)) {
                    if record.status != status {
                        return Err(ChatError::FinalReceipt);
                    }
                } else if !self.review_pending.as_ref().is_some_and(|pending| pending.request == request) {
                    return Err(ChatError::UnknownReceipt);
                }
                self.update_review_status(request, status);
                // An immutable duplicate belongs only to its original full
                // capture/grant/ticket; it cannot retire a newer recovery.
                if self.review_pending.as_ref().is_some_and(|pending| pending.request == request) {
                    self.review_pending = None;
                }
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
                    self.reads.clear();
                    self.after_event = *through_event;
                    self.gap = true;
                }
            }
            ChatEvent::Unsupported => {
                self.conversations.clear();
                self.reads.clear();
                self.history = ChatHistory::new(packet.source).ok();
                self.uncertain();
            }
        }
        // Latch a temporary loss within a batch of incoming packets: a later
        // policy/epoch restoration cannot rescue a pre-dispatch old gesture.
        if let Some((target, true)) = self.review_watch
            && !self.review_allowed(target)
        {
            self.review_watch = Some((target, false));
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
            reads: if self.live && authenticated {
                self.reads.clone()
            } else {
                BTreeMap::new()
            },
            read_operations: self.read_operations.iter().copied().collect(),
            gap: self.gap || self.history.as_ref().is_some_and(ChatHistory::has_gap),
        };
        if self.snapshot.as_ref().is_none_or(|previous| **previous != next) {
            self.snapshot = Some(Arc::new(next));
        }
    }
}

#[cfg(test)]
mod review_tests;

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
    fn read_row(source: ChatSource, revision: u64, through: u64) -> aurora_session::chat::ReadStateRow {
        let info = policy(source, revision, true);
        aurora_session::chat::ReadStateRow {
            conversation: Conversation::Nearby,
            title: info.title,
            policy_revision: revision,
            lease_current: true,
            ready: true,
            can_send: true,
            can_read: true,
            read: Some(ReadMetadata {
                unread: through,
                unread_revision: through + 1,
                observed: aurora_session::chat::ObservedIncoming { epoch: 1, through },
                can_mark_read: true,
            }),
        }
    }
    fn read_request(row: &aurora_session::chat::ReadStateRow, number: u64) -> MarkReadRequest {
        let read = row.read.unwrap();
        MarkReadRequest {
            key: ChatKey {
                source: source(),
                conversation: row.conversation,
            },
            request: number,
            policy_revision: row.policy_revision,
            observed: read.observed,
            unread_revision: read.unread_revision,
        }
    }

    #[test]
    fn chat_broker_read_admission_race_publishes_terminal_exact_ticket_without_native_enqueue() {
        let (mut peer, _worker) = connected();
        let first = read_row(source(), 1, 1);
        peer.observe(&event(source(), 1, ChatEvent::ReadBatch(vec![first.clone()])))
            .unwrap();
        let request = read_request(&first, 40); // host FIFO ticket already returned to the UI
        peer.observe(&event(source(), 2, ChatEvent::ReadBatch(vec![read_row(source(), 1, 2)])))
            .unwrap();
        let sent = peer.sent;
        assert_eq!(peer.admit_read(request, Instant::now()), Err(HostError::Unavailable));
        let snapshot = peer.snapshot().unwrap();
        assert_eq!(
            snapshot.read_operations,
            vec![ReadOperation {
                request,
                status: ReadStatus::Unavailable
            }]
        );
        assert!(peer.read_pending.is_none());
        assert_eq!(peer.sent, sent, "rejected ticket never enters native transport");
    }

    #[test]
    fn chat_broker_read_timeout_queries_only_and_protects_active_identity_from_local_rejections() {
        let (mut peer, mut worker) = connected();
        let row = read_row(source(), 1, 1);
        peer.observe(&event(source(), 1, ChatEvent::ReadBatch(vec![row.clone()]))).unwrap();
        let request = read_request(&row, 40);
        let now = Instant::now();
        peer.admit_read(request, now).unwrap();
        assert!(peer.poll_read_receipt(now + RECEIPT_WAIT));
        assert_eq!(peer.read_operations[0].status, ReadStatus::Uncertain);
        assert_eq!(peer.admit_read(read_request(&row, 41), now), Err(HostError::Busy));
        for number in 42..=120 {
            peer.reject_read(read_request(&row, number));
        }
        assert_eq!(peer.read_operations.len(), MAX_READ_RECEIPTS);
        assert!(peer.read_operations.iter().any(|operation| operation.request == request));
        // Drive only owned in-memory loopback IO. No World, network grid or
        // native mutation is installed in this test.
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut commands = Vec::new();
        while commands.len() < 2 {
            if let Some(io) = &mut peer.io {
                io.poll().unwrap();
            }
            commands.extend(worker.poll().unwrap());
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(
            commands
                .iter()
                .filter(|packet| matches!(packet.message, ChatMessage::Command(ChatCommand::MarkRead(_))))
                .count(),
            1
        );
        assert!(
            commands
                .iter()
                .any(|packet| matches!(packet.message, ChatMessage::Command(ChatCommand::QueryReadReceipt(_))))
        );
        peer.disconnect();
        assert!(peer.reads.is_empty());
        assert_eq!(
            peer.read_pending.as_ref().unwrap().request,
            request,
            "socket reconnect retains lookup identity only"
        );
        peer.set_control(None, false);
        assert!(peer.read_pending.is_none() && peer.read_operations.is_empty());
    }

    #[test]
    fn chat_broker_read_batch_policy_precedes_metadata_and_old_receipt_cannot_restore_it() {
        let (mut peer, _worker) = connected();
        let row = read_row(source(), 1, 1);
        peer.observe(&event(source(), 1, ChatEvent::ReadBatch(vec![row.clone()]))).unwrap();
        let request = read_request(&row, 40);
        peer.admit_read(request, Instant::now()).unwrap();
        peer.observe(&event(source(), 2, ChatEvent::Gap { through_event: 1 })).unwrap();
        assert!(peer.reads.is_empty() && peer.conversations.is_empty());
        let mut denied = read_row(source(), 2, 2);
        denied.can_read = false;
        denied.read = None;
        peer.observe(&event(source(), 3, ChatEvent::ReadBatch(vec![denied]))).unwrap();
        let receipt = aurora_session::chat::ReadReceipt {
            key: request.key,
            request: request.request,
            observed: request.observed,
            unread_revision: request.unread_revision,
            outcome: ReadOutcome::Applied,
        };
        peer.observe(&event(
            source(),
            4,
            ChatEvent::ReadReceipt(aurora_session::chat::ReadReceiptReply {
                query: ReadReceiptQuery {
                    key: request.key,
                    request: request.request,
                },
                receipt: Some(receipt),
            }),
        ))
        .unwrap();
        peer.publish();
        let snapshot = peer.snapshot().unwrap();
        assert!(snapshot.reads.is_empty());
        assert!(!snapshot.conversations[0].policy.can_read);
        let received = peer.received;
        assert_eq!(
            peer.observe(&event(source(), 1000, ChatEvent::ReadBatch(vec![row]))),
            Err(ChatError::StalePolicy)
        );
        assert_eq!(peer.received, received);
        assert!(peer.reads.is_empty());
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
