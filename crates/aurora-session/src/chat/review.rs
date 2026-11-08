//! Typed prerequisite for explicit native conversation review, following
//! ReviewChat in indra/newview/fssessionworker.cpp. These chat V3 DTOs and
//! receipts do not themselves activate a world, open UI or authorize sending.
//! The eventual owning App adapter must still verify an existing conversation,
//! current read/privacy policy and the exact acknowledged Active input grant.

use super::{ChatError, ChatKey, ChatSource, Conversation, ObservedIncoming, changed_source};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Review receipts contain no body or title. Thirty-two retained operations
/// bound this separate UI-operation history below the existing 64 read receipts;
/// evicted operation numbers remain fenced by the source's high-water mark.
pub const MAX_REVIEW_RECEIPTS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewCapture {
    pub key: ChatKey,
    pub policy_revision: u64,
    pub observed: ObservedIncoming,
}

impl ReviewCapture {
    pub fn validate(self) -> Result<(), ChatError> {
        validate_key(self.key)?;
        if self.policy_revision == 0 || self.observed.epoch == 0 {
            return Err(ChatError::Sequence);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewConversationRequest {
    pub target: ReviewCapture,
    pub request: u64,
    pub activation: u64,
}

impl ReviewConversationRequest {
    pub fn validate(self) -> Result<(), ChatError> {
        self.target.validate()?;
        if self.request == 0 || self.activation == 0 {
            return Err(ChatError::Sequence);
        }
        Ok(())
    }
}

/// Terminal local outcomes only. Pending and uncertainty belong to the caller's
/// orchestration state and cannot turn a terminal receipt into a new UI action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewOutcome {
    OpenedLocally,
    RejectedNative,
    StaleTarget,
    NotForeground,
    Expired,
}

/// Full original capture and grant identify the operation without granting any
/// fresh authority. The caller must match them before attributing an outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewReceipt {
    pub target: ReviewCapture,
    pub request: u64,
    pub activation: u64,
    pub outcome: ReviewOutcome,
}

impl ReviewReceipt {
    pub fn validate(self) -> Result<(), ChatError> {
        ReviewConversationRequest {
            target: self.target,
            request: self.request,
            activation: self.activation,
        }
        .validate()
    }

    pub fn matches(self, request: ReviewConversationRequest) -> bool {
        self.target == request.target && self.request == request.request && self.activation == request.activation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewReceiptQuery {
    pub key: ChatKey,
    pub request: u64,
}

impl ReviewReceiptQuery {
    pub fn validate(self) -> Result<(), ChatError> {
        validate_key(self.key)?;
        if self.request == 0 {
            return Err(ChatError::Sequence);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewReceiptReply {
    pub query: ReviewReceiptQuery,
    /// A lookup miss never permits replay, including an unfinished reservation
    /// whose local operation outcome is not known.
    pub receipt: Option<ReviewReceipt>,
}

impl ReviewReceiptReply {
    pub fn validate(self) -> Result<(), ChatError> {
        self.query.validate()?;
        if let Some(receipt) = self.receipt {
            receipt.validate()?;
            if receipt.target.key != self.query.key || receipt.request != self.query.request {
                return Err(ChatError::Binding);
            }
        }
        Ok(())
    }

    /// Validate against the captured original operation, not only the query's
    /// key/number. A receipt for another policy, observation or grant is stale.
    pub fn validate_for(self, request: ReviewConversationRequest) -> Result<(), ChatError> {
        self.validate()?;
        request.validate()?;
        if self.query.key != request.target.key || self.query.request != request.request {
            return Err(ChatError::Binding);
        }
        if self.receipt.is_some_and(|receipt| !receipt.matches(request)) {
            return Err(ChatError::RequestChanged);
        }
        Ok(())
    }
}

fn validate_key(key: ChatKey) -> Result<(), ChatError> {
    key.validate()?;
    // A typed Direct/Group key is not proof that the native session exists. That
    // lookup remains mandatory at the owning App's eventual review boundary.
    if !matches!(
        key.conversation,
        Conversation::Nearby | Conversation::Direct(_) | Conversation::Group(_)
    ) {
        return Err(ChatError::Invalid);
    }
    Ok(())
}

struct ReviewRecord {
    request: ReviewConversationRequest,
    receipt: Option<ReviewReceipt>,
}

/// Source-bound, single-flight reservation before a native UI operation. The
/// ledger survives socket reconnects; only a fresh accepted source retires it.
pub struct ReviewReceiptLedger {
    source: ChatSource,
    high_water: u64,
    records: VecDeque<ReviewRecord>,
}

impl ReviewReceiptLedger {
    pub fn new(source: ChatSource) -> Result<Self, ChatError> {
        source.validate()?;
        Ok(Self {
            source,
            high_water: 0,
            records: VecDeque::new(),
        })
    }

    /// Same-source reconnect retains reservations and high-water. A changed
    /// source follows the common monotonic session-generation/lease rules.
    pub fn reset(&mut self, source: ChatSource) -> Result<bool, ChatError> {
        if !changed_source(self.source, source)? {
            return Ok(false);
        }
        self.source = source;
        self.high_water = 0;
        self.records.clear();
        Ok(true)
    }

    /// None records the first admission of a separately authorized native
    /// operation. An exact duplicate returns its immutable receipt; an
    /// unfinished reservation returns Busy and can never be replayed/evicted.
    pub fn reserve(&mut self, request: ReviewConversationRequest) -> Result<Option<ReviewReceipt>, ChatError> {
        request.validate()?;
        if request.target.key.source != self.source {
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
        if self.records.len() == MAX_REVIEW_RECEIPTS {
            self.records.pop_front();
        }
        self.high_water = request.request;
        self.records.push_back(ReviewRecord { request, receipt: None });
        Ok(None)
    }

    pub fn finish(&mut self, request: ReviewConversationRequest, outcome: ReviewOutcome) -> Result<ReviewReceipt, ChatError> {
        request.validate()?;
        if request.target.key.source != self.source {
            return Err(ChatError::Binding);
        }
        let record = self
            .records
            .iter_mut()
            .find(|record| record.request.request == request.request)
            .ok_or(ChatError::UnknownReceipt)?;
        if record.request != request {
            return Err(ChatError::RequestChanged);
        }
        if record.receipt.is_some() {
            return Err(ChatError::FinalReceipt);
        }
        let receipt = ReviewReceipt {
            target: request.target,
            request: request.request,
            activation: request.activation,
            outcome,
        };
        record.receipt = Some(receipt);
        Ok(receipt)
    }

    pub fn query(&self, query: ReviewReceiptQuery) -> Result<ReviewReceiptReply, ChatError> {
        query.validate()?;
        if query.key.source != self.source {
            return Err(ChatError::Binding);
        }
        let record = self.records.iter().find(|record| record.request.request == query.request);
        if record.is_some_and(|record| record.request.target.key != query.key) {
            return Err(ChatError::RequestChanged);
        }
        Ok(ReviewReceiptReply {
            query,
            receipt: record.and_then(|record| record.receipt),
        })
    }

    pub fn retained_count(&self) -> usize {
        self.records.len()
    }
}
