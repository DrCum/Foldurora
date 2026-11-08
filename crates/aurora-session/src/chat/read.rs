//! V2 native read observations and immutable, bounded operation receipts.

use super::*;

pub const MAX_READ_ROWS: usize = 4;
pub const MAX_READ_RECEIPTS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedIncoming {
    pub epoch: u64,
    pub through: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadMetadata {
    pub unread: u64,
    pub unread_revision: u64,
    pub observed: ObservedIncoming,
    pub can_mark_read: bool,
}

impl ReadMetadata {
    pub fn validate(self) -> Result<(), ChatError> {
        if self.observed.epoch == 0 || self.unread_revision == 0 {
            Err(ChatError::Sequence)
        } else {
            Ok(())
        }
    }
}

/// Source comes only from the authenticated packet, avoiding four full source
/// copies. Policy authority precedes the optional privacy-checked read data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadStateRow {
    pub conversation: Conversation,
    pub title: Label,
    pub policy_revision: u64,
    pub lease_current: bool,
    pub ready: bool,
    pub can_send: bool,
    pub can_read: bool,
    pub read: Option<ReadMetadata>,
    /// Independently produced live incoming activity; no read permission.
    pub incoming: Option<NativeIncoming>,
    /// Current native incarnation only; does not imply per-session activity.
    pub incoming_stream: Option<u64>,
}

impl ReadStateRow {
    pub fn info(&self, source: ChatSource) -> ConversationInfo {
        ConversationInfo {
            title: self.title.clone(),
            policy: NativeChatPolicy {
                key: ChatKey {
                    source,
                    conversation: self.conversation,
                },
                revision: self.policy_revision,
                lease_current: self.lease_current,
                ready: self.ready,
                can_send: self.can_send,
                can_read: self.can_read,
            },
        }
    }
    pub fn validate(&self, source: ChatSource) -> Result<(), ChatError> {
        self.info(source).policy.validate()?;
        if !matches!(self.conversation, Conversation::Nearby | Conversation::Direct(_)) {
            return Err(ChatError::Invalid);
        }
        if let Some(read) = self.read {
            read.validate()?;
            if !self.lease_current || !self.ready || !self.can_read {
                return Err(ChatError::Denied);
            }
        }
        if self.incoming.is_some() && self.incoming.map(|incoming| incoming.stream) != self.incoming_stream {
            return Err(ChatError::Binding);
        }
        if self.incoming_stream == Some(0) {
            return Err(ChatError::Sequence);
        }
        if self.incoming_stream.is_some()
            && (!matches!(self.conversation, Conversation::Direct(_)) || !self.lease_current || !self.ready || !self.can_read)
        {
            return Err(ChatError::Denied);
        }
        if let Some(incoming) = self.incoming {
            incoming.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkReadRequest {
    pub key: ChatKey,
    pub request: u64,
    pub policy_revision: u64,
    pub observed: ObservedIncoming,
    pub unread_revision: u64,
}
impl MarkReadRequest {
    pub fn validate(self) -> Result<(), ChatError> {
        self.key.validate()?;
        if self.request == 0 || self.policy_revision == 0 || self.observed.epoch == 0 || self.unread_revision == 0 {
            return Err(ChatError::Sequence);
        }
        if !matches!(self.key.conversation, Conversation::Nearby | Conversation::Direct(_)) {
            return Err(ChatError::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadOutcome {
    Applied,
    AlreadyRead,
    StaleObservation,
    RejectedNative,
}

/// Never contains text, counts, titles or fresh authority. Repeating it cannot
/// restore old private metadata or authorize another counter mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadReceipt {
    pub key: ChatKey,
    pub request: u64,
    pub observed: ObservedIncoming,
    pub unread_revision: u64,
    pub outcome: ReadOutcome,
}
impl ReadReceipt {
    pub fn matches(self, request: MarkReadRequest) -> bool {
        self.key == request.key
            && self.request == request.request
            && self.observed == request.observed
            && self.unread_revision == request.unread_revision
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadReceiptQuery {
    pub key: ChatKey,
    pub request: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadReceiptReply {
    pub query: ReadReceiptQuery,
    /// None is a final lookup miss. It never means a mutation may be retried.
    pub receipt: Option<ReadReceipt>,
}

struct ReadRecord {
    request: MarkReadRequest,
    receipt: Option<ReadReceipt>,
}

pub struct ReadReceiptLedger {
    source: ChatSource,
    high_water: u64,
    records: VecDeque<ReadRecord>,
}
impl ReadReceiptLedger {
    pub fn new(source: ChatSource) -> Self {
        Self {
            source,
            high_water: 0,
            records: VecDeque::new(),
        }
    }
    /// Reserve before invoking native code. An unfinished reservation cannot
    /// be replayed or evicted. Old evicted IDs remain below the high water.
    pub fn reserve(&mut self, request: MarkReadRequest) -> Result<Option<ReadReceipt>, ChatError> {
        request.validate()?;
        if request.key.source != self.source {
            return Err(ChatError::Binding);
        }
        if let Some(record) = self.records.iter().find(|record| record.request.request == request.request) {
            if record.request != request {
                return Err(ChatError::RequestChanged);
            }
            return record.receipt.map(Some).ok_or(ChatError::Busy);
        }
        if request.request <= self.high_water {
            return Err(ChatError::StaleRequest);
        }
        if self.records.iter().any(|record| record.receipt.is_none()) {
            return Err(ChatError::Busy);
        }
        if self.records.len() == MAX_READ_RECEIPTS {
            self.records.pop_front();
        }
        self.high_water = request.request;
        self.records.push_back(ReadRecord { request, receipt: None });
        Ok(None)
    }
    pub fn finish(&mut self, request: MarkReadRequest, outcome: ReadOutcome) -> Result<ReadReceipt, ChatError> {
        let record = self
            .records
            .iter_mut()
            .find(|record| record.request == request)
            .ok_or(ChatError::UnknownReceipt)?;
        if record.receipt.is_some() {
            return Err(ChatError::FinalReceipt);
        }
        let receipt = ReadReceipt {
            key: request.key,
            request: request.request,
            observed: request.observed,
            unread_revision: request.unread_revision,
            outcome,
        };
        record.receipt = Some(receipt);
        Ok(receipt)
    }
    pub fn query(&self, query: ReadReceiptQuery) -> Result<ReadReceiptReply, ChatError> {
        if query.key.source != self.source {
            return Err(ChatError::Binding);
        }
        Ok(ReadReceiptReply {
            query,
            receipt: self
                .records
                .iter()
                .find(|record| record.request.key == query.key && record.request.request == query.request)
                .and_then(|record| record.receipt),
        })
    }
}
