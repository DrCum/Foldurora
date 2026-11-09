//! Bounded literal-chat data, independent of world activation and the control
//! V1 envelope. No GUI input, offers, scripts, arbitrary channels, credential
//! access, transcript files or network submission are implemented here.
//!
//! Native adapter boundaries: `App::send` / `send_from` retain RLVa command
//! filtering; `World::send_session_im` and `own_im` retain conversation/echo
//! restrictions. Do not invoke `App::send_local_chat`: it executes chatbar
//! commands and gestures. Export only already-filtered plain `ChatLine` text;
//! interactive Invite/InventoryOffer/Payment cards have no representation.

use crate::{Binding, ConversationId, Label, SessionIdentity, WorkerIdentity};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::fmt;

mod read;
pub use read::*;
mod review;
pub use review::*;
mod close;
pub use close::*;
mod contacts;
pub use contacts::*;
#[cfg(test)]
mod conference_tests;
#[cfg(test)]
mod group_tests;

/// V7 also admits explicit initialized-Group Review; older consumers fail closed.
/// V8 admits source-qualified Conference read/close/review rows; native roster
/// completeness and current participant permission stay with the owning worker.
pub const VERSION: u16 = 8;
pub const MAX_PACKET_BYTES: usize = 4096;
/// One native UDP chat/IM part (`aurora-net::message_split`), in UTF-8 bytes.
pub const MAX_TEXT_BYTES: usize = 1023;
pub const MAX_CONVERSATIONS: usize = 64;
pub const MAX_RETAINED_LINES: usize = 256;
pub const MAX_RECEIPTS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatError {
    Binding,
    Invalid,
    TooLong,
    Oversized,
    Malformed,
    Version,
    Sequence,
    StalePolicy,
    Unavailable,
    Denied,
    Busy,
    StaleRequest,
    RequestChanged,
    UnknownReceipt,
    FinalReceipt,
    Capacity,
}
impl fmt::Display for ChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session chat: {self:?}")
    }
}
impl std::error::Error for ChatError {}

#[derive(Clone, Debug, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatSource {
    pub worker: WorkerIdentity,
    pub session: SessionIdentity,
    pub lease_epoch: u64,
}
impl ChatSource {
    pub fn validate(self) -> Result<(), ChatError> {
        if self.lease_epoch == 0 {
            return Err(ChatError::Binding);
        }
        self.binding().validate().map_err(|_| ChatError::Binding)
    }
    pub fn binding(self) -> Binding {
        Binding {
            worker: self.worker,
            session: Some(self.session),
        }
    }
}
impl TryFrom<(Binding, u64)> for ChatSource {
    type Error = ChatError;
    fn try_from((binding, lease_epoch): (Binding, u64)) -> Result<Self, Self::Error> {
        binding.validate().map_err(|_| ChatError::Binding)?;
        let source = Self {
            worker: binding.worker,
            session: binding.session.ok_or(ChatError::Binding)?,
            lease_epoch,
        };
        source.validate()?;
        Ok(source)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case", deny_unknown_fields)]
pub enum Conversation {
    /// Always literal normal public-channel text, never chatbar interpretation.
    Nearby,
    /// Aurora's 1:1 UI key is its peer UUID; group/conference keys are native
    /// conversation UUIDs. Only existing initialized conversations may send.
    Direct(ConversationId),
    Group(ConversationId),
    Conference(ConversationId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatKey {
    pub source: ChatSource,
    pub conversation: Conversation,
}
impl ChatKey {
    pub fn validate(self) -> Result<(), ChatError> {
        self.source.validate()
    }
}

#[derive(Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ChatText(String);
impl ChatText {
    pub fn new(text: &str) -> Result<Self, ChatError> {
        if text.len() > MAX_TEXT_BYTES {
            return Err(ChatError::TooLong);
        }
        if text.chars().any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')) {
            return Err(ChatError::Invalid);
        }
        Ok(Self(text.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
impl fmt::Debug for ChatText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChatText([redacted], {} UTF-8 bytes)", self.0.len())
    }
}
impl<'de> Deserialize<'de> for ChatText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::new(&text).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendRequest {
    pub key: ChatKey,
    pub request: u64,
    pub policy_revision: u64,
    pub text: ChatText,
}
impl SendRequest {
    pub fn validate(&self) -> Result<(), ChatError> {
        self.key.validate()?;
        if self.request == 0 || self.policy_revision == 0 {
            return Err(ChatError::Sequence);
        }
        if self.text.as_str().trim().is_empty() {
            return Err(ChatError::Invalid);
        }
        Ok(())
    }
}

/// These facts are computed freshly by the owning viewer, never supplied by
/// a Send request. Admission does not replace native restriction checks at
/// the actual send boundary, join a session, unblock a resident or promote
/// a world. Bump revision when permissions or native conversation state change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeChatPolicy {
    pub key: ChatKey,
    pub revision: u64,
    pub lease_current: bool,
    pub ready: bool,
    pub can_send: bool,
    pub can_read: bool,
}
impl NativeChatPolicy {
    pub fn validate(self) -> Result<(), ChatError> {
        self.key.validate()?;
        if self.revision == 0 { Err(ChatError::Sequence) } else { Ok(()) }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationInfo {
    pub policy: NativeChatPolicy,
    pub title: Label,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendOutcome {
    Pending,
    /// Enqueued by native transport; this is not a remote delivery receipt.
    AcceptedLocally,
    RejectedNative,
    /// May already have been submitted. Never replay it as a new send.
    Uncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendReceipt {
    pub key: ChatKey,
    pub request: u64,
    pub outcome: SendOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Receipts are bounded small control data; no per-admission heap allocation.
#[allow(clippy::large_enum_variant)]
pub enum SendAdmission {
    SubmitOnce,
    Duplicate(SendReceipt),
}

struct Record {
    request: SendRequest,
    outcome: SendOutcome,
}

/// Per-source single-flight native submission, with bounded retained receipts.
/// Even evicted request numbers stay below the high-water mark and cannot be
/// replayed. Record Pending BEFORE calling native code; resolve uncertainty
/// by a receipt only, never by resubmitting the mutation.
pub struct SendLedger {
    source: ChatSource,
    high_water: u64,
    records: VecDeque<Record>,
    policies: BTreeMap<Conversation, NativeChatPolicy>,
}
impl SendLedger {
    pub fn new(source: ChatSource) -> Result<Self, ChatError> {
        source.validate()?;
        Ok(Self {
            source,
            high_water: 0,
            records: VecDeque::new(),
            policies: BTreeMap::new(),
        })
    }
    pub fn reset(&mut self, source: ChatSource) -> Result<bool, ChatError> {
        if !changed_source(self.source, source)? {
            return Ok(false);
        }
        self.source = source;
        self.high_water = 0;
        self.records.clear();
        self.policies.clear();
        Ok(true)
    }
    pub fn admit(&mut self, request: &SendRequest, policy: NativeChatPolicy) -> Result<SendAdmission, ChatError> {
        request.validate()?;
        policy.validate()?;
        if request.key.source != self.source || request.key != policy.key {
            return Err(ChatError::Binding);
        }
        update_policy(&mut self.policies, self.source, policy)?;
        if !policy.lease_current {
            return Err(ChatError::Unavailable);
        }
        if let Some(record) = self.records.iter().find(|record| record.request.request == request.request) {
            if record.request != *request {
                return Err(ChatError::RequestChanged);
            }
            return Ok(SendAdmission::Duplicate(SendReceipt {
                key: request.key,
                request: request.request,
                outcome: record.outcome,
            }));
        }
        if request.request <= self.high_water {
            return Err(ChatError::StaleRequest);
        }
        if request.policy_revision != policy.revision {
            return Err(ChatError::StalePolicy);
        }
        if !policy.ready {
            return Err(ChatError::Unavailable);
        }
        if !policy.can_send {
            return Err(ChatError::Denied);
        }
        if self.records.iter().any(|record| record.outcome == SendOutcome::Pending) {
            return Err(ChatError::Busy);
        }
        if self.records.len() == MAX_RECEIPTS {
            self.records.pop_front();
        }
        self.high_water = request.request;
        self.records.push_back(Record {
            request: request.clone(),
            outcome: SendOutcome::Pending,
        });
        Ok(SendAdmission::SubmitOnce)
    }
    pub fn finish(&mut self, key: ChatKey, request: u64, outcome: SendOutcome) -> Result<SendReceipt, ChatError> {
        if key.source != self.source {
            return Err(ChatError::Binding);
        }
        if outcome == SendOutcome::Pending {
            return Err(ChatError::Invalid);
        }
        let record = self
            .records
            .iter_mut()
            .find(|record| record.request.request == request)
            .ok_or(ChatError::UnknownReceipt)?;
        if record.request.key != key {
            return Err(ChatError::Binding);
        }
        if !matches!(record.outcome, SendOutcome::Pending | SendOutcome::Uncertain) && record.outcome != outcome {
            return Err(ChatError::FinalReceipt);
        }
        record.outcome = outcome;
        Ok(SendReceipt { key, request, outcome })
    }
    pub fn retained_count(&self) -> usize {
        self.records.len()
    }
}

fn changed_source(old: ChatSource, new: ChatSource) -> Result<bool, ChatError> {
    new.validate()?;
    if old == new {
        return Ok(false);
    }
    if old.worker == new.worker
        && (new.lease_epoch < old.lease_epoch
            || (new.session != old.session && new.session.generation <= old.session.generation)
            || (new.lease_epoch == old.lease_epoch && new.session.generation <= old.session.generation))
    {
        return Err(ChatError::Binding);
    }
    Ok(true)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    Incoming,
    Own,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatLine {
    pub key: ChatKey,
    pub event: u64,
    pub policy_revision: u64,
    pub sender: Label,
    pub text: ChatText,
    pub kind: LineKind,
    pub truncated: bool,
}

/// One bounded in-memory live window. It never reads/writes transcript files.
/// Fresh native policy is required both when retaining and when viewing lines.
pub struct ChatHistory {
    source: ChatSource,
    lines: VecDeque<ChatLine>,
    last_event: u64,
    gap: bool,
    policies: BTreeMap<Conversation, NativeChatPolicy>,
}
impl ChatHistory {
    pub fn new(source: ChatSource) -> Result<Self, ChatError> {
        source.validate()?;
        Ok(Self {
            source,
            lines: VecDeque::new(),
            last_event: 0,
            gap: false,
            policies: BTreeMap::new(),
        })
    }
    pub fn reset(&mut self, source: ChatSource) -> Result<bool, ChatError> {
        if !changed_source(self.source, source)? {
            return Ok(false);
        }
        self.source = source;
        self.lines.clear();
        self.last_event = 0;
        self.gap = false;
        self.policies.clear();
        Ok(true)
    }
    pub fn apply_policy(&mut self, policy: NativeChatPolicy) -> Result<(), ChatError> {
        let changed = self
            .policies
            .get(&policy.key.conversation)
            .is_some_and(|old| old.revision != policy.revision);
        update_policy(&mut self.policies, self.source, policy)?;
        if changed || !policy.lease_current || !policy.ready || !policy.can_read {
            let previous = self.lines.len();
            self.lines.retain(|line| line.key != policy.key);
            self.gap |= previous != self.lines.len();
        }
        Ok(())
    }
    pub fn push(&mut self, line: ChatLine, policy: NativeChatPolicy) -> Result<(), ChatError> {
        line.key.validate()?;
        if line.key != policy.key {
            return Err(ChatError::Binding);
        }
        // A trusted current policy revokes retained text even when this
        // particular queued line predates that policy or was duplicated.
        self.apply_policy(policy)?;
        if line.event == 0 || line.event <= self.last_event {
            return Err(ChatError::Sequence);
        }
        if line.policy_revision != policy.revision {
            return Err(ChatError::StalePolicy);
        }
        if !policy.lease_current || !policy.ready || !policy.can_read {
            return Err(ChatError::Denied);
        }
        if line.event != self.last_event.saturating_add(1) {
            self.gap = true;
        }
        self.last_event = line.event;
        if self.lines.len() == MAX_RETAINED_LINES {
            self.lines.pop_front();
            self.gap = true;
        }
        self.lines.push_back(line);
        Ok(())
    }
    pub fn note_gap(&mut self, source: ChatSource, through_event: u64) -> Result<(), ChatError> {
        if source != self.source {
            return Err(ChatError::Binding);
        }
        if through_event == 0 || through_event <= self.last_event {
            return Err(ChatError::Sequence);
        }
        self.last_event = through_event;
        self.gap = true;
        Ok(())
    }
    pub fn lines(&self, policy: NativeChatPolicy) -> impl Iterator<Item = &ChatLine> {
        let allowed = policy.validate().is_ok()
            && policy.key.source == self.source
            && policy.lease_current
            && policy.ready
            && policy.can_read
            && self.policies.get(&policy.key.conversation) == Some(&policy);
        self.lines.iter().filter(move |line| allowed && line.key == policy.key)
    }
    pub fn retained_count(&self) -> usize {
        self.lines.len()
    }
    pub fn has_gap(&self) -> bool {
        self.gap
    }
}

fn update_policy(
    policies: &mut BTreeMap<Conversation, NativeChatPolicy>,
    source: ChatSource,
    policy: NativeChatPolicy,
) -> Result<(), ChatError> {
    policy.validate()?;
    if policy.key.source != source {
        return Err(ChatError::Binding);
    }
    if let Some(old) = policies.get(&policy.key.conversation) {
        if old.revision > policy.revision || (old.revision == policy.revision && *old != policy) {
            return Err(ChatError::StalePolicy);
        }
    } else if policies.len() == MAX_CONVERSATIONS {
        return Err(ChatError::Capacity);
    }
    policies.insert(policy.key.conversation, policy);
    Ok(())
}

pub struct ChatDrafts {
    source: ChatSource,
    drafts: BTreeMap<Conversation, ChatText>,
}
impl ChatDrafts {
    pub fn new(source: ChatSource) -> Result<Self, ChatError> {
        source.validate()?;
        Ok(Self {
            source,
            drafts: BTreeMap::new(),
        })
    }
    pub fn reset(&mut self, source: ChatSource) -> Result<bool, ChatError> {
        if !changed_source(self.source, source)? {
            return Ok(false);
        }
        self.source = source;
        self.drafts.clear();
        Ok(true)
    }
    pub fn set(&mut self, key: ChatKey, text: ChatText) -> Result<(), ChatError> {
        if key.source != self.source {
            return Err(ChatError::Binding);
        }
        if text.is_empty() {
            self.drafts.remove(&key.conversation);
            return Ok(());
        }
        if !self.drafts.contains_key(&key.conversation) && self.drafts.len() == MAX_CONVERSATIONS {
            return Err(ChatError::Capacity);
        }
        self.drafts.insert(key.conversation, text);
        Ok(())
    }
    pub fn get(&self, key: ChatKey) -> Option<&ChatText> {
        if key.source != self.source {
            return None;
        }
        self.drafts.get(&key.conversation)
    }
    pub fn retained_count(&self) -> usize {
        self.drafts.len()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)] // one bounded DTO per private packet
pub enum ChatCommand {
    Send(SendRequest),
    CloseConversation(CloseRequest),
    QueryCloseReceipt(CloseRequest),
    MarkRead(MarkReadRequest),
    QueryReadReceipt(ReadReceiptQuery),
    ReviewConversation(ReviewConversationRequest),
    QueryReviewReceipt(ReviewReceiptQuery),
    Poll { after_event: u64 },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)] // one bounded DTO per private packet
pub enum ChatEvent {
    Conversation(ConversationInfo),
    Line(ChatLine),
    Receipt(SendReceipt),
    ReadBatch(Vec<ReadStateRow>),
    ReadReceipt(ReadReceiptReply),
    ReviewReceipt(ReviewReceiptReply),
    CloseReceipt(CloseReceiptReply),
    Gap { through_event: u64 },
    Unsupported,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "direction", content = "body", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatMessage {
    Command(ChatCommand),
    Event(ChatEvent),
}

/// Separate channel DTO: adding this module does not enable chat in the
/// control broker, advertise support or expose a composer prematurely.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatPacket {
    pub version: u16,
    pub source: ChatSource,
    pub sequence: u64,
    pub reply_to: u64,
    pub message: ChatMessage,
}
impl fmt::Debug for ChatPacket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match &self.message {
            ChatMessage::Command(ChatCommand::CloseConversation(_)) => "close_conversation",
            ChatMessage::Command(ChatCommand::QueryCloseReceipt(_)) => "query_close_receipt",
            ChatMessage::Command(ChatCommand::Send(_)) => "send",
            ChatMessage::Command(ChatCommand::MarkRead(_)) => "mark_read",
            ChatMessage::Command(ChatCommand::QueryReadReceipt(_)) => "query_read_receipt",
            ChatMessage::Command(ChatCommand::ReviewConversation(_)) => "review_conversation",
            ChatMessage::Command(ChatCommand::QueryReviewReceipt(_)) => "query_review_receipt",
            ChatMessage::Command(ChatCommand::Poll { .. }) => "poll",
            ChatMessage::Event(ChatEvent::Conversation(_)) => "conversation",
            ChatMessage::Event(ChatEvent::Line(_)) => "line",
            ChatMessage::Event(ChatEvent::Receipt(_)) => "receipt",
            ChatMessage::Event(ChatEvent::ReadBatch(_)) => "read_batch",
            ChatMessage::Event(ChatEvent::ReadReceipt(_)) => "read_receipt",
            ChatMessage::Event(ChatEvent::ReviewReceipt(_)) => "review_receipt",
            ChatMessage::Event(ChatEvent::CloseReceipt(_)) => "close_receipt",
            ChatMessage::Event(ChatEvent::Gap { .. }) => "gap",
            ChatMessage::Event(ChatEvent::Unsupported) => "unsupported",
        };
        f.debug_struct("ChatPacket")
            .field("version", &self.version)
            .field("sequence", &self.sequence)
            .field("reply_to", &self.reply_to)
            .field("kind", &kind)
            .finish_non_exhaustive()
    }
}
impl ChatPacket {
    pub fn validate(&self) -> Result<(), ChatError> {
        if self.version != VERSION {
            return Err(ChatError::Version);
        }
        self.source.validate()?;
        if self.sequence == 0 {
            return Err(ChatError::Sequence);
        }
        let mut source = self.source;
        match &self.message {
            ChatMessage::Command(command) => {
                if self.reply_to != 0 {
                    return Err(ChatError::Sequence);
                }
                if let ChatCommand::CloseConversation(request) | ChatCommand::QueryCloseReceipt(request) = command {
                    request.validate()?;
                    source = request.key.source;
                }
                if let ChatCommand::MarkRead(request) = command {
                    request.validate()?;
                    source = request.key.source;
                }
                if let ChatCommand::QueryReadReceipt(query) = command {
                    query.key.validate()?;
                    if query.request == 0 {
                        return Err(ChatError::Sequence);
                    }
                    source = query.key.source;
                }
                if let ChatCommand::Send(request) = command {
                    request.validate()?;
                    source = request.key.source;
                }
                if let ChatCommand::ReviewConversation(request) = command {
                    request.validate()?;
                    source = request.target.key.source;
                }
                if let ChatCommand::QueryReviewReceipt(query) = command {
                    query.validate()?;
                    source = query.key.source;
                }
            }
            ChatMessage::Event(event) => match event {
                ChatEvent::CloseReceipt(reply) => {
                    reply.validate()?;
                    if self.reply_to == 0 {
                        return Err(ChatError::Sequence);
                    }
                    source = reply.request.key.source;
                }
                ChatEvent::Conversation(info) => {
                    info.policy.validate()?;
                    source = info.policy.key.source;
                }
                ChatEvent::Line(line) => {
                    line.key.validate()?;
                    if line.event == 0 || line.policy_revision == 0 {
                        return Err(ChatError::Sequence);
                    }
                    source = line.key.source;
                }
                ChatEvent::Receipt(receipt) => {
                    receipt.key.validate()?;
                    if receipt.request == 0 {
                        return Err(ChatError::Sequence);
                    }
                    source = receipt.key.source;
                }
                ChatEvent::ReadBatch(rows) => {
                    if rows.len() > MAX_READ_ROWS {
                        return Err(ChatError::Capacity);
                    }
                    let mut seen = std::collections::BTreeSet::new();
                    for row in rows {
                        row.validate(self.source)?;
                        if !seen.insert(row.conversation) {
                            return Err(ChatError::Invalid);
                        }
                    }
                }
                ChatEvent::ReadReceipt(reply) => {
                    reply.query.key.validate()?;
                    if reply.query.request == 0 {
                        return Err(ChatError::Sequence);
                    }
                    source = reply.query.key.source;
                    if let Some(receipt) = reply.receipt
                        && (receipt.key != reply.query.key
                            || receipt.request != reply.query.request
                            || receipt.observed.epoch == 0
                            || receipt.unread_revision == 0)
                    {
                        return Err(ChatError::Invalid);
                    }
                }
                ChatEvent::ReviewReceipt(reply) => {
                    reply.validate()?;
                    if self.reply_to == 0 {
                        return Err(ChatError::Sequence);
                    }
                    source = reply.query.key.source;
                }
                ChatEvent::Gap { through_event } if *through_event == 0 => return Err(ChatError::Sequence),
                _ => {}
            },
        }
        if source != self.source {
            return Err(ChatError::Binding);
        }
        Ok(())
    }
}
pub fn encode(packet: &ChatPacket) -> Result<Vec<u8>, ChatError> {
    packet.validate()?;
    let bytes = serde_json::to_vec(packet).map_err(|_| ChatError::Malformed)?;
    if bytes.len() > MAX_PACKET_BYTES {
        Err(ChatError::Oversized)
    } else {
        Ok(bytes)
    }
}
pub fn decode(bytes: &[u8]) -> Result<ChatPacket, ChatError> {
    if bytes.len() > MAX_PACKET_BYTES {
        return Err(ChatError::Oversized);
    }
    let packet: ChatPacket = serde_json::from_slice(bytes).map_err(|_| ChatError::Malformed)?;
    packet.validate()?;
    Ok(packet)
}
