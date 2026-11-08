//! Data and immutable receipt ownership for an explicit ordinary-window show.
//!
//! No value here grants native/control/render/input authority. The real worker
//! must validate its current owned App/window, ordinary display, exact source,
//! lease and original local ingress deadline before any synchronous effect.
//! Wire commands/version and the actual native consumer are a separate join.

use crate::{Binding, Error, WorkerIdentity};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const MAX_SHOW_RECEIPTS: usize = 16;

/// Nonreused owner observations, never an HWND or a native permission.
/// `ordinary` also changes for an eligible-mode/source/blocking ABA while the
/// same physical window remains. Real lifecycle owners mint these counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowObservation {
    pub app: u64,
    pub window: u64,
    pub ordinary: u64,
}
impl WindowObservation {
    pub fn validate(self) -> Result<(), Error> {
        if self.app == 0 || self.window == 0 || self.ordinary == 0 {
            Err(Error::Invalid)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowViewerTarget {
    pub binding: Binding,
    pub lease_epoch: u64,
    pub window: WindowObservation,
}
impl ShowViewerTarget {
    pub fn validate(self) -> Result<(), Error> {
        self.binding.validate()?;
        if self.lease_epoch == 0 {
            return Err(Error::Invalid);
        }
        self.window.validate()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowViewerRequest {
    pub target: ShowViewerTarget,
    pub request: u64,
}
impl ShowViewerRequest {
    pub fn validate(self) -> Result<(), Error> {
        self.target.validate()?;
        if self.request == 0 { Err(Error::Sequence) } else { Ok(()) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowViewerOutcome {
    /// The native owner performed the effect and observed its own foreground.
    ShownLocally,
    /// Includes unsupported/modal/managed presentation or invalid native owner.
    RejectedNative,
    StaleTarget,
    /// Measured from ORIGINAL local IO ingress; App dequeue cannot renew it.
    Expired,
    /// Restore/focus was attempted, but native foreground was not confirmed.
    /// This terminal receipt never authorizes retrying the native mutation.
    ForegroundRefused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowViewerReceipt {
    pub request: ShowViewerRequest,
    pub outcome: ShowViewerOutcome,
}
impl ShowViewerReceipt {
    pub fn validate(self) -> Result<(), Error> {
        self.request.validate()
    }
    pub fn matches(self, request: ShowViewerRequest) -> bool {
        self.validate().is_ok() && request.validate().is_ok() && self.request == request
    }
}

struct Entry {
    request: ShowViewerRequest,
    receipt: Option<ShowViewerReceipt>,
}

/// Bounded bookkeeping for one actual WorkerIdentity/lease, not permission.
/// The caller reserves reply ownership AND this ledger before effect, and checks native
/// authorization even for lookup. A source or window replacement cannot borrow
/// this ledger's history as a grant. Pending entries are never evicted; completed
/// entries can retire, but their monotonic highwater prevents reopening them.
pub struct ShowViewerLedger {
    worker: WorkerIdentity,
    lease_epoch: u64,
    highwater: u64,
    entries: VecDeque<Entry>,
}
impl ShowViewerLedger {
    pub fn new(worker: WorkerIdentity, lease_epoch: u64) -> Result<Self, Error> {
        if lease_epoch == 0 {
            return Err(Error::Invalid);
        }
        Ok(Self {
            worker,
            lease_epoch,
            highwater: 0,
            entries: VecDeque::new(),
        })
    }

    fn validate(&self, request: ShowViewerRequest) -> Result<(), Error> {
        request.validate()?;
        if request.target.binding.worker != self.worker || request.target.lease_epoch != self.lease_epoch {
            return Err(Error::Binding);
        }
        Ok(())
    }

    /// None reserves a NEW request. A terminal exact duplicate returns the
    /// immutable receipt; an exact pending duplicate is Busy, never new work.
    pub fn reserve(&mut self, request: ShowViewerRequest) -> Result<Option<ShowViewerReceipt>, Error> {
        self.validate(request)?;
        if let Some(entry) = self.entries.iter().find(|entry| entry.request.request == request.request) {
            if entry.request != request {
                return Err(Error::Invalid);
            }
            return entry.receipt.map(Some).ok_or(Error::Busy);
        }
        if request.request <= self.highwater {
            return Err(Error::Stale);
        }
        if self.entries.len() == MAX_SHOW_RECEIPTS {
            let Some(index) = self.entries.iter().position(|entry| entry.receipt.is_some()) else {
                return Err(Error::Capacity);
            };
            let _ = self.entries.remove(index);
        }
        self.entries.push_back(Entry { request, receipt: None });
        self.highwater = request.request;
        Ok(None)
    }

    /// Every second finish is rejected, even with the same outcome. Only the
    /// first exact completion owns the terminal result.
    pub fn finish(&mut self, request: ShowViewerRequest, outcome: ShowViewerOutcome) -> Result<ShowViewerReceipt, Error> {
        self.validate(request)?;
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.request.request == request.request)
            .ok_or(Error::Stale)?;
        if entry.request != request {
            return Err(Error::Invalid);
        }
        if entry.receipt.is_some() {
            return Err(Error::Stale);
        }
        let receipt = ShowViewerReceipt { request, outcome };
        entry.receipt = Some(receipt);
        Ok(receipt)
    }

    /// Read-only lookup is not a native retry. The caller supplies the ORIGINAL
    /// whole request; an altered target cannot retrieve a receipt by ID alone.
    pub fn receipt(&self, request: ShowViewerRequest) -> Result<Option<ShowViewerReceipt>, Error> {
        self.validate(request)?;
        if let Some(entry) = self.entries.iter().find(|entry| entry.request.request == request.request) {
            if entry.request != request {
                return Err(Error::Invalid);
            }
            return Ok(entry.receipt);
        }
        if request.request <= self.highwater {
            Err(Error::Stale)
        } else {
            Ok(None)
        }
    }

    pub fn retained_count(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests;
