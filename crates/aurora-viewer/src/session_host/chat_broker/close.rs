//! Session-local host hiding, following Folderstorm ChatView::close/accept
//! and FSSessionWorker::CloseChat (abbe3cb0, LGPL-2.1-or-later). Neither the
//! native conversation nor unread is changed. Only native live incoming
//! provenance newer than the original acknowledged fence can reopen a row.
use super::*;
use aurora_session::chat::{CloseOutcome, CloseReceiptReply, CloseReceiptState, CloseRequest, NativeIncoming, ReadStateRow};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseStatus {
    Pending,
    Uncertain,
    Completed(CloseOutcome),
    Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CloseOperation {
    pub request: CloseRequest,
    pub status: CloseStatus,
}
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct CloseSnapshot {
    pub hidden: BTreeSet<Conversation>,
    pub targets: BTreeMap<Conversation, u64>,
    pub operations: Vec<CloseOperation>,
}
struct Hidden {
    request: CloseRequest,
    fence: Option<NativeIncoming>,
}
struct PendingClose {
    request: CloseRequest,
    since: Instant,
    queried_at: Option<Instant>,
}
#[derive(Default)]
pub(super) struct CloseBook {
    hidden: BTreeMap<Conversation, Hidden>,
    targets: BTreeMap<Conversation, u64>,
    incoming: BTreeMap<Conversation, NativeIncoming>,
    operations: VecDeque<CloseOperation>,
    pending: Option<PendingClose>,
    highwater: u64,
}
impl CloseBook {
    pub fn hidden(&self, conversation: Conversation) -> bool {
        self.hidden.contains_key(&conversation)
    }
    pub fn revoke(&mut self) {
        self.targets.clear();
        self.incoming.clear();
        if let Some(pending) = &self.pending {
            self.status(pending.request, CloseStatus::Uncertain);
        }
    }
    pub fn policy_changed(&mut self, conversation: Conversation) {
        self.targets.remove(&conversation);
        self.incoming.remove(&conversation);
    }
    pub fn row(&mut self, row: &ReadStateRow) {
        self.policy_changed(row.conversation);
        if let Some(stream) = row.incoming_stream {
            self.targets.insert(row.conversation, stream);
        }
        if let Some(incoming) = row.incoming {
            self.incoming.insert(row.conversation, incoming);
        }
        self.reopen(row.conversation);
    }
    fn reopen(&mut self, conversation: Conversation) {
        let newer = self.hidden.get(&conversation).and_then(|hidden| {
            let fence = hidden.fence?;
            let incoming = *self.incoming.get(&conversation)?;
            (self.targets.get(&conversation) == Some(&fence.stream))
                .then(|| incoming.newer_than(fence))
                .flatten()
        });
        if newer == Some(true) {
            self.hidden.remove(&conversation);
        }
    }
    fn status(&mut self, request: CloseRequest, status: CloseStatus) {
        if let Some(operation) = self.operations.iter_mut().find(|operation| operation.request == request) {
            operation.status = status;
        }
    }
    fn record(&mut self, request: CloseRequest, status: CloseStatus) -> Result<(), HostError> {
        if request.request <= self.highwater {
            return Err(HostError::Unavailable);
        }
        if self.operations.len() == aurora_session::chat::MAX_CLOSE_RECEIPTS {
            let active = self.pending.as_ref().map(|pending| pending.request);
            let index = self
                .operations
                .iter()
                .position(|operation| Some(operation.request) != active)
                .ok_or(HostError::Busy)?;
            self.operations.remove(index);
        }
        self.highwater = request.request;
        self.operations.push_back(CloseOperation { request, status });
        Ok(())
    }
    pub fn snapshot(&self) -> CloseSnapshot {
        CloseSnapshot {
            hidden: self.hidden.keys().copied().collect(),
            targets: self.targets.clone(),
            operations: self.operations.iter().copied().collect(),
        }
    }
    pub fn receipt(&mut self, reply: CloseReceiptReply) -> Result<(), ChatError> {
        reply.validate()?;
        let request = reply.request;
        let operation = self
            .operations
            .iter()
            .find(|operation| operation.request == request)
            .ok_or(ChatError::UnknownReceipt)?;
        let status = match reply.result {
            CloseReceiptState::Pending => CloseStatus::Pending,
            CloseReceiptState::Unknown => CloseStatus::Uncertain,
            CloseReceiptState::Terminal(outcome) => CloseStatus::Completed(outcome),
        };
        if !matches!(operation.status, CloseStatus::Pending | CloseStatus::Uncertain) {
            return if operation.status == status {
                Ok(())
            } else {
                Err(ChatError::FinalReceipt)
            };
        }
        // A delayed Pending cannot regress an uncertain operation or acquire
        // a fresh query budget. An unknown lookup never supplies a new fence.
        if status == CloseStatus::Pending {
            return Ok(());
        }
        self.status(request, status);
        if self.pending.as_ref().is_some_and(|pending| pending.request == request) {
            self.pending = None;
        }
        if let Some(hidden) = self
            .hidden
            .get_mut(&request.key.conversation)
            .filter(|hidden| hidden.request == request)
        {
            match reply.result {
                CloseReceiptState::Terminal(CloseOutcome::Closed(fence)) => hidden.fence = Some(fence),
                CloseReceiptState::Terminal(_) => {
                    self.hidden.remove(&request.key.conversation);
                }
                _ => {}
            }
            self.reopen(request.key.conversation);
        }
        Ok(())
    }
}

impl Peer {
    fn close_allowed(&self, request: CloseRequest) -> bool {
        request.validate().is_ok()
            && self.live
            && self.io.is_some()
            && self.source == Some(request.key.source)
            && self.closes.targets.get(&request.key.conversation) == Some(&request.stream)
            && self.conversations.get(&request.key.conversation).is_some_and(|info| {
                info.policy.key == request.key
                    && info.policy.revision == request.policy_revision
                    && info.policy.lease_current
                    && info.policy.ready
                    && info.policy.can_read
            })
    }
    fn close_query_allowed(&self, request: CloseRequest) -> bool {
        // Read-only recovery keeps the original request/stream. A changed
        // current policy can authorize lookup, never a replacement capture.
        self.live
            && self.io.is_some()
            && self.source == Some(request.key.source)
            && self.conversations.get(&request.key.conversation).is_some_and(|info| {
                info.policy.key == request.key && info.policy.lease_current && info.policy.ready && info.policy.can_read
            })
    }
    pub fn admit_close(&mut self, request: CloseRequest, now: Instant) -> Result<(), HostError> {
        let fresh = request.request > self.closes.highwater;
        let result = self.queue_close(request, now);
        if result.is_err() && fresh && self.source == Some(request.key.source) {
            if self.closes.record(request, CloseStatus::Unavailable).is_err() {
                self.closes.status(request, CloseStatus::Unavailable);
            }
            self.publish();
        }
        result
    }
    fn queue_close(&mut self, request: CloseRequest, now: Instant) -> Result<(), HostError> {
        if self.pending.is_some() || self.read_pending.is_some() || self.review_pending.is_some() || self.closes.pending.is_some() {
            return Err(HostError::Busy);
        }
        if !self.close_allowed(request)
            || self.closes.hidden(request.key.conversation)
            || self.closes.hidden.len() == aurora_session::chat::MAX_CONVERSATIONS
            || request.request <= self.closes.highwater
        {
            return Err(HostError::Unavailable);
        }
        let sequence = self.sent.checked_add(1).ok_or(HostError::Unavailable)?;
        let packet = ChatPacket {
            version: VERSION,
            source: request.key.source,
            sequence,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::CloseConversation(request)),
        };
        // Reserve bounded operation history before any native queue effect.
        self.closes.record(request, CloseStatus::Pending)?;
        self.io.as_mut().ok_or(HostError::Unavailable)?.queue(&packet).map_err(|error| {
            if error == LaneError::Busy {
                HostError::Busy
            } else {
                HostError::Unavailable
            }
        })?;
        self.sent = sequence;
        self.closes.hidden.insert(request.key.conversation, Hidden { request, fence: None });
        self.closes.pending = Some(PendingClose {
            request,
            since: now,
            queried_at: None,
        });
        self.publish();
        Ok(())
    }
    pub(super) fn poll_close_receipt(&mut self, now: Instant) {
        let Some(pending) = &self.closes.pending else { return };
        let request = pending.request;
        if now.saturating_duration_since(pending.since) < RECEIPT_WAIT {
            return;
        }
        let queried_at = pending.queried_at;
        let age = now.saturating_duration_since(pending.since);
        self.closes.status(request, CloseStatus::Uncertain);
        if let Some(queried_at) = queried_at {
            if now.saturating_duration_since(queried_at) >= RECEIPT_WAIT {
                self.closes.pending = None;
            }
        } else if self.close_query_allowed(request) {
            let Some(sequence) = self.sent.checked_add(1) else { return };
            let packet = ChatPacket {
                version: VERSION,
                source: request.key.source,
                sequence,
                reply_to: 0,
                message: ChatMessage::Command(ChatCommand::QueryCloseReceipt(request)),
            };
            if self.io.as_mut().is_some_and(|io| io.queue(&packet).is_ok()) {
                self.sent = sequence;
                if let Some(pending) = &mut self.closes.pending {
                    pending.queried_at = Some(now);
                }
            }
        }
        if age >= RECEIPT_WAIT + RECEIPT_WAIT && self.closes.pending.as_ref().is_some_and(|pending| pending.queried_at.is_none()) {
            // Authority/queue loss cannot block every other conversation.
            // Keep the unknown hide; this is not proof the native close failed.
            self.closes.pending = None;
        }
        // Exactly one successfully queued lookup; never resend Close or
        // replace its stream/fence from a newer catalogue or native counter.
        self.publish();
    }
}

#[cfg(test)]
mod tests;
