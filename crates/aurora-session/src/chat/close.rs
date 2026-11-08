//! Close-in-host native fences. Data only: neither a native IM close nor Read.
//! The real producer advances only on eligible live nonself incoming messages.
use super::*;

pub const MAX_CLOSE_RECEIPTS: usize = 64;

/// Known zero means no qualifying incoming yet. Absence is unknown, not zero.
/// Always compare within the same exact ChatSource; source replacement retires
/// the stream. This is not an export cursor, read watermark or native permit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeIncoming {
    pub stream: u64,
    pub serial: u64,
}
impl NativeIncoming {
    pub fn validate(self) -> Result<(), ChatError> {
        if self.stream == 0 { Err(ChatError::Sequence) } else { Ok(()) }
    }
    pub fn checked_next(self) -> Option<Self> {
        self.validate().ok()?;
        self.serial.checked_add(1).map(|serial| Self {
            stream: self.stream,
            serial,
        })
    }
    /// A different or unknown stream is not evidence of a newer message.
    pub fn newer_than(self, fence: Self) -> Option<bool> {
        self.validate().ok()?;
        fence.validate().ok()?;
        (self.stream == fence.stream).then_some(self.serial > fence.serial)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloseRequest {
    pub key: ChatKey,
    pub request: u64,
    pub policy_revision: u64,
    /// Original native incarnation; never replace this from a later query.
    pub stream: u64,
}
impl CloseRequest {
    pub fn validate(self) -> Result<(), ChatError> {
        self.key.validate()?;
        if self.request == 0 || self.policy_revision == 0 || self.stream == 0 {
            return Err(ChatError::Sequence);
        }
        if !matches!(
            self.key.conversation,
            Conversation::Direct(_) | Conversation::Group(_) | Conversation::Conference(_)
        ) {
            return Err(ChatError::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "through", rename_all = "snake_case", deny_unknown_fields)]
pub enum CloseOutcome {
    Closed(NativeIncoming),
    RejectedNative,
    Expired,
    StaleTarget,
}

/// Unknown is an unavailable lookup, never proof of failure or a fresh fence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum CloseReceiptState {
    Unknown,
    Pending,
    Terminal(CloseOutcome),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloseReceiptReply {
    pub request: CloseRequest,
    pub result: CloseReceiptState,
}
impl CloseReceiptReply {
    pub fn validate(self) -> Result<(), ChatError> {
        self.request.validate()?;
        if let CloseReceiptState::Terminal(CloseOutcome::Closed(through)) = self.result {
            through.validate()?;
            if through.stream != self.request.stream {
                return Err(ChatError::Binding);
            }
        }
        Ok(())
    }
    pub fn matches(self, request: CloseRequest) -> bool {
        self.validate().is_ok() && self.request == request
    }
}

struct Record {
    request: CloseRequest,
    outcome: Option<CloseOutcome>,
}
/// Exact source/operation history, independent of socket reconnect. Reserve
/// before native capture and reply construction; never evict pending work or
/// recapture a duplicate at a newer frontier. Host query-once ownership joins
/// later; this native lookup remains read-only even if called repeatedly.
pub struct CloseReceiptLedger {
    source: ChatSource,
    highwater: u64,
    records: VecDeque<Record>,
}
impl CloseReceiptLedger {
    pub fn new(source: ChatSource) -> Self {
        Self {
            source,
            highwater: 0,
            records: VecDeque::new(),
        }
    }
    pub fn reserve(&mut self, request: CloseRequest) -> Result<Option<CloseReceiptReply>, ChatError> {
        request.validate()?;
        if request.key.source != self.source {
            return Err(ChatError::Binding);
        }
        if let Some(record) = self.records.iter().find(|record| record.request.request == request.request) {
            if record.request != request {
                return Err(ChatError::RequestChanged);
            }
            return record
                .outcome
                .map(|outcome| {
                    Some(CloseReceiptReply {
                        request,
                        result: CloseReceiptState::Terminal(outcome),
                    })
                })
                .ok_or(ChatError::Busy);
        }
        if request.request <= self.highwater {
            return Err(ChatError::StaleRequest);
        }
        if self.records.iter().any(|record| record.outcome.is_none()) {
            return Err(ChatError::Busy);
        }
        if self.records.len() == MAX_CLOSE_RECEIPTS {
            self.records.pop_front();
        }
        self.records.push_back(Record { request, outcome: None });
        self.highwater = request.request;
        Ok(None)
    }
    pub fn finish(&mut self, request: CloseRequest, outcome: CloseOutcome) -> Result<CloseReceiptReply, ChatError> {
        let receipt = CloseReceiptReply {
            request,
            result: CloseReceiptState::Terminal(outcome),
        };
        receipt.validate()?;
        let record = self
            .records
            .iter_mut()
            .find(|record| record.request == request)
            .ok_or(ChatError::UnknownReceipt)?;
        if record.outcome.is_some() {
            return Err(ChatError::FinalReceipt);
        }
        record.outcome = Some(outcome);
        Ok(receipt)
    }
    pub fn query(&self, request: CloseRequest) -> Result<CloseReceiptReply, ChatError> {
        request.validate()?;
        if request.key.source != self.source {
            return Err(ChatError::Binding);
        }
        let result = if let Some(record) = self.records.iter().find(|record| record.request.request == request.request) {
            if record.request != request {
                return Err(ChatError::RequestChanged);
            }
            record.outcome.map_or(CloseReceiptState::Pending, CloseReceiptState::Terminal)
        } else {
            CloseReceiptState::Unknown
        };
        Ok(CloseReceiptReply { request, result })
    }
}

#[cfg(test)]
mod tests;
