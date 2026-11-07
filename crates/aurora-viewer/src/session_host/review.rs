//! Body-free orchestration for explicit native conversation review. Port of
//! reviewChat/attentionAction and the captured pendingReview handoff
//! (tools/session-host/main.cpp, abbe3cb0ca66ee2484f277219f6a27706fab7fc6).
//! Only an exact acknowledged grant can release one captured request; no
//! timeout, cancellation, later world or receipt lookup can replay that open.

use aurora_session::chat::{ChatSource, NativeChatPolicy, ReadMetadata, ReviewCapture, ReviewConversationRequest, ReviewOutcome};
use aurora_session::{Binding, HostedViewport};
use std::collections::VecDeque;

pub(super) const MAX_OPERATIONS: usize = 32;
const DEADLINE_MILLIS: u64 = 15_000;

/// The captured incarnation may gain arrivals or be read normally during a
/// handoff. Counts, unread revisions, Send and MarkRead capacity are unrelated
/// to this UI-only operation; replacement epoch/policy/source never is.
pub(super) fn current_target(
    target: ReviewCapture,
    source: Option<ChatSource>,
    live: bool,
    authenticated: bool,
    policy: Option<NativeChatPolicy>,
    read: Option<ReadMetadata>,
) -> bool {
    target.validate().is_ok()
        && source == Some(target.key.source)
        && live
        && authenticated
        && policy.is_some_and(|policy| {
            policy.validate().is_ok()
                && policy.key == target.key
                && policy.revision == target.policy_revision
                && policy.lease_current
                && policy.ready
                && policy.can_read
        })
        && read.is_some_and(|read| {
            read.validate().is_ok() && read.observed.epoch == target.observed.epoch && read.observed.through >= target.observed.through
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    WaitingForGrant,
    Dispatched,
    Uncertain,
    Completed(ReviewOutcome),
    Cancelled,
    Unavailable,
    Expired,
    Busy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Operation {
    pub target: ReviewCapture,
    pub request: u64,
    pub activation: Option<u64>,
    pub status: Status,
}

/// Produced only after the broker's ordinary accepted Switched/grant path, or
/// its fully validated existing-active shortcut. Active WorkerId alone is not
/// this proof. No coordinator or control-wire internals leave the broker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Grant {
    pub binding: Binding,
    pub activation: u64,
    pub viewport: HostedViewport,
}

#[derive(Clone, Copy)]
pub(super) struct Delivery {
    pub request: ReviewConversationRequest,
    pub status: Status,
    pub recovering: bool,
}

struct Pending {
    request: u64,
    viewport: HostedViewport,
    deadline: u64,
    granted: bool,
    dispatched: bool,
}

#[derive(Default)]
pub(super) struct Transactions {
    operations: VecDeque<Operation>,
    pending: Option<Pending>,
    high_water: u64,
}

impl Transactions {
    pub fn operations(&self) -> Vec<Operation> {
        self.operations.iter().copied().collect()
    }

    fn retain(&mut self, operation: Operation) {
        if self.operations.iter().any(|old| old.request == operation.request) {
            return;
        }
        if self.operations.len() == MAX_OPERATIONS {
            // A run of local Busy outcomes cannot evict the unresolved action.
            let active = self.pending.as_ref().map(|pending| pending.request);
            if let Some(index) = self.operations.iter().position(|old| Some(old.request) != active) {
                self.operations.remove(index);
            } else {
                return;
            }
        }
        self.operations.push_back(operation);
    }

    pub fn reject(&mut self, target: ReviewCapture, request: u64, status: Status) {
        self.retain(Operation {
            target,
            request,
            activation: None,
            status,
        });
    }

    pub fn begin(&mut self, target: ReviewCapture, request: u64, viewport: HostedViewport, now: u64) -> bool {
        if request == 0 || request <= self.high_water {
            return false;
        }
        self.high_water = request;
        if self.pending.is_some() {
            self.reject(target, request, Status::Busy);
            return false;
        }
        if target.validate().is_err() || viewport.validate().is_err() {
            self.reject(target, request, Status::Unavailable);
            return false;
        }
        if self.operations.iter().any(|old| old.request == request) {
            return false;
        }
        self.retain(Operation {
            target,
            request,
            activation: None,
            status: Status::WaitingForGrant,
        });
        self.pending = Some(Pending {
            request,
            viewport,
            deadline: now.saturating_add(DEADLINE_MILLIS),
            granted: false,
            dispatched: false,
        });
        true
    }

    fn current(&self) -> Option<Operation> {
        let pending = self.pending.as_ref()?;
        self.operations
            .iter()
            .find(|operation| operation.request == pending.request)
            .copied()
    }

    fn status(&mut self, status: Status) {
        if let Some(pending) = &self.pending
            && let Some(operation) = self.operations.iter_mut().find(|operation| operation.request == pending.request)
        {
            operation.status = status;
        }
    }

    pub fn target(&self) -> Option<ReviewCapture> {
        self.current().map(|operation| operation.target)
    }

    pub fn waiting(&self) -> bool {
        self.pending.as_ref().is_some_and(|pending| !pending.dispatched)
    }

    pub fn expect_activation(&mut self, activation: u64) -> bool {
        let Some(pending) = &self.pending else { return false };
        if activation == 0 || pending.dispatched {
            return false;
        }
        if self
            .current()
            .is_some_and(|operation| operation.activation.is_some_and(|expected| expected != activation))
        {
            self.interrupt(Status::Cancelled);
            return false;
        }
        let request = self.pending.as_ref().map(|pending| pending.request);
        if let Some(operation) = self.operations.iter_mut().find(|operation| Some(operation.request) == request) {
            operation.activation = Some(activation);
            true
        } else {
            false
        }
    }

    /// An unrelated activation/geometry transaction cannot supply this grant.
    pub fn control_activation(&mut self, activation: u64) {
        if self.waiting() && self.current().is_some_and(|operation| operation.activation != Some(activation)) {
            self.interrupt(Status::Cancelled);
        }
    }

    pub fn granted(&mut self, grant: Grant) {
        let Some(operation) = self.current() else { return };
        let Some(pending) = &mut self.pending else { return };
        if pending.dispatched {
            return;
        }
        if grant.binding == operation.target.key.source.binding()
            && operation.activation == Some(grant.activation)
            && pending.viewport == grant.viewport
        {
            pending.granted = true;
        } else {
            self.interrupt(Status::Cancelled);
        }
    }

    pub fn full_request(&self) -> Option<ReviewConversationRequest> {
        let operation = self.current()?;
        Some(ReviewConversationRequest {
            target: operation.target,
            request: operation.request,
            activation: operation.activation?,
        })
    }

    pub fn dispatch(&self, grant: Grant) -> Option<ReviewConversationRequest> {
        let pending = self.pending.as_ref()?;
        let request = self.full_request()?;
        (pending.granted
            && !pending.dispatched
            && grant.binding == request.target.key.source.binding()
            && grant.activation == request.activation
            && grant.viewport == pending.viewport)
            .then_some(request)
    }

    pub fn queued(&mut self, request: ReviewConversationRequest) {
        if self.full_request() == Some(request)
            && let Some(pending) = &mut self.pending
            && pending.granted
            && !pending.dispatched
        {
            pending.dispatched = true;
            self.status(Status::Dispatched);
        }
    }

    /// Before dispatch cancellation/expiry guarantees no open was queued.
    /// Afterwards only the immutable native receipt can say what happened.
    pub fn interrupt(&mut self, before_dispatch: Status) {
        if let Some(pending) = &self.pending {
            if pending.dispatched {
                self.status(Status::Uncertain);
            } else {
                self.status(before_dispatch);
                self.pending = None;
            }
        }
    }

    pub fn cancel(&mut self, request: u64) {
        if self.pending.as_ref().is_some_and(|pending| pending.request == request) {
            self.interrupt(Status::Cancelled);
        }
    }

    pub fn tick(&mut self, now: u64, target_current: bool) {
        if self.pending.as_ref().is_some_and(|pending| now >= pending.deadline) {
            self.interrupt(Status::Expired);
            self.pending = None;
        } else if !target_current && self.waiting() {
            self.interrupt(Status::Unavailable);
        }
    }

    pub fn delivery(&mut self, delivery: Option<Delivery>) {
        if self.pending.as_ref().is_none_or(|pending| !pending.dispatched) {
            return;
        }
        let Some(delivery) = delivery.filter(|delivery| self.full_request() == Some(delivery.request)) else {
            self.status(Status::Uncertain);
            self.pending = None;
            return;
        };
        if !(self.current().is_some_and(|operation| operation.status == Status::Uncertain) && delivery.status == Status::Dispatched) {
            self.status(delivery.status);
        }
        if !delivery.recovering {
            self.pending = None;
        }
    }
}

#[cfg(test)]
mod tests;
