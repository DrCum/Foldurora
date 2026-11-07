//! RememberActive reservations. No inactive/source policy is fabricated to use
//! the legacy inactive-only allocator. Native ACK, never pixels, releases rate.
use super::*;
use crate::session_host::preview_io::{ConnectionToken, PreviewCommand};
use aurora_session::WorkerIdentity;
use aurora_session::preview_wire::{DemandConfiguration, DemandOutcome, DemandReceipt};
use std::time::Duration;

const RECEIPT_WAIT: Duration = Duration::from_secs(2);
const HALF_FPS: u32 = 20;

fn configuration(options: Options) -> DemandConfiguration {
    DemandConfiguration {
        size: options.size,
        rate: options.rate,
    }
}
pub(super) fn options(config: DemandConfiguration) -> Options {
    Options {
        size: config.size,
        rate: config.rate,
    }
}
fn maximum(left: Options, right: Options) -> Options {
    Options {
        size: if left.size.bytes() >= right.size.bytes() {
            left.size
        } else {
            right.size
        },
        rate: left.rate.max(right.rate),
    }
}
pub(super) fn aggregate(views: &[ViewDescriptor], pending: &[SettingsReceipt]) -> Vec<(PreviewSource, Options)> {
    let mut sources: Vec<(PreviewSource, Options)> = Vec::with_capacity(MAX_WORKERS);
    for view in views.iter().filter(|view| view.visible) {
        let requested = pending
            .iter()
            .find(|request| {
                request.namespace == view.namespace && request.view == view.view && request.ticket.source().preview() == view.source
            })
            .map_or(view.options, |request| request.desired);
        if let Some((_, options)) = sources.iter_mut().find(|(source, _)| *source == view.source) {
            *options = maximum(*options, requested);
        } else {
            sources.push((view.source, requested));
        }
    }
    sources
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Accepted {
    pub token: ConnectionToken,
    pub demand: u64,
    pub options: Options,
    pub native_revision: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Offer {
    pub token: ConnectionToken,
    pub demand: u64,
    pub desired: Option<Options>,
}
impl Offer {
    pub fn command(self) -> PreviewCommand {
        match self.desired {
            Some(options) => PreviewCommand::RememberActive {
                demand: self.demand,
                size: options.size,
                rate: options.rate,
            },
            None => PreviewCommand::Stop { demand: self.demand },
        }
    }
}
struct Pending {
    offer: Offer,
    began: Instant,
    queued: bool,
}
struct Entry {
    worker: WorkerIdentity,
    accepted: Option<Accepted>,
    pending: Option<Pending>,
    uncertain: bool,
    next: Instant,
    fresh_required: bool,
}
impl Entry {
    fn held(&self) -> u32 {
        self.accepted.map_or(0, |a| a.options.rate.half_fps()).max(
            self.pending
                .as_ref()
                .and_then(|pending| pending.offer.desired)
                .map_or(0, |o| o.rate.half_fps()),
        )
    }
}
pub(super) struct Ledger {
    entries: Vec<Entry>,
    next_demand: u64,
    next_global: Option<Instant>,
    cursor: usize,
}
impl Ledger {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_demand: 1,
            next_global: None,
            cursor: 0,
        }
    }
    pub fn held(&self) -> u32 {
        self.entries.iter().map(Entry::held).sum()
    }
    pub fn accepted(&self, token: ConnectionToken) -> Option<Accepted> {
        self.entries
            .iter()
            .filter(|entry| !entry.uncertain && !entry.fresh_required && entry.pending.is_none())
            .filter_map(|entry| entry.accepted)
            .find(|accepted| accepted.token.same_metadata(token))
            .map(|mut accepted| {
                accepted.token = token;
                accepted
            })
    }
    /// Reserve BEFORE enqueue, including the old acknowledged charge. Failed
    /// non-admission returns that reservation; accepted/uncertain never does.
    pub fn offer(&mut self, token: ConnectionToken, desired: Option<Options>, now: Instant) -> Result<Option<Offer>, PreviewError> {
        let worker = token.source().worker();
        let index = match self.entries.iter().position(|entry| entry.worker == worker) {
            Some(index) => index,
            None if desired.is_none() => return Ok(None),
            None => {
                if self.entries.len() == MAX_WORKERS {
                    return Err(PreviewError::Busy);
                }
                self.entries.push(Entry {
                    worker,
                    accepted: None,
                    pending: None,
                    uncertain: false,
                    next: now,
                    fresh_required: true,
                });
                self.entries.len() - 1
            }
        };
        let entry = &mut self.entries[index];
        if entry.uncertain || entry.pending.is_some() {
            return Err(PreviewError::Busy);
        }
        if let Some(accepted) = entry.accepted {
            if !accepted.token.same_metadata(token) {
                entry.uncertain = true;
                return Err(PreviewError::Unavailable);
            }
            if desired == Some(accepted.options) && !entry.fresh_required {
                return Ok(None);
            }
        } else if desired.is_none() {
            return Ok(None);
        }
        let old = entry.held();
        let requested = desired.map_or(0, |o| o.rate.half_fps());
        if self.held().saturating_sub(old).saturating_add(old.max(requested)) > HALF_FPS {
            return Err(PreviewError::Busy);
        }
        let demand = if desired.is_none() {
            self.entries[index].accepted.ok_or(PreviewError::Stale)?.demand
        } else {
            let demand = self.next_demand;
            self.next_demand = demand.checked_add(1).ok_or(PreviewError::Exhausted)?;
            demand
        };
        let offer = Offer { token, demand, desired };
        self.entries[index].pending = Some(Pending {
            offer,
            began: now,
            queued: false,
        });
        Ok(Some(offer))
    }
    pub fn admission(&mut self, offer: Offer, queued: bool) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.worker == offer.token.source().worker())
            && let Some(pending) = entry.pending.as_mut().filter(|pending| pending.offer == offer)
        {
            if queued {
                pending.queued = true;
            } else {
                entry.pending = None;
            }
        }
    }
    pub fn receipt(&mut self, token: ConnectionToken, receipt: DemandReceipt) -> bool {
        if receipt.validate().is_err() || receipt.source != token.source() || receipt.metadata_connection != token.connection() {
            return false;
        }
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.worker == token.source().worker()) else {
            return false;
        };
        let Some(pending) = entry.pending.as_ref().filter(|pending| {
            pending.queued
                && pending.offer.token.same_metadata(token)
                && pending.offer.demand == receipt.demand
                && pending
                    .offer
                    .desired
                    .is_some_and(|desired| configuration(desired) == receipt.requested)
        }) else {
            return false;
        };
        if entry.uncertain {
            return false;
        }
        match receipt.outcome {
            DemandOutcome::Accepted {
                effective,
                native_revision,
            } => {
                entry.accepted = Some(Accepted {
                    token,
                    demand: pending.offer.demand,
                    options: options(effective),
                    native_revision,
                });
                entry.fresh_required = false;
            }
            DemandOutcome::Rejected { .. } => {}
        }
        entry.pending = None;
        true
    }
    pub fn stopped(&mut self, token: ConnectionToken, demand: u64) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.worker == token.source().worker()) else {
            return false;
        };
        if entry.uncertain
            || entry
                .accepted
                .is_none_or(|accepted| accepted.demand != demand || !accepted.token.same_metadata(token))
            || entry.pending.as_ref().is_none_or(|pending| {
                !pending.queued
                    || pending.offer.desired.is_some()
                    || pending.offer.demand != demand
                    || !pending.offer.token.same_metadata(token)
            })
        {
            return false;
        }
        entry.pending = None;
        entry.accepted = None;
        entry.fresh_required = true;
        true
    }
    /// Actual owned process exit ONLY. EOF, source relog and detach cannot
    /// discharge an uncertain native configuration reservation.
    pub fn exited(&mut self, worker: WorkerIdentity) {
        self.entries.retain(|entry| entry.worker != worker);
    }
    pub fn lost(&mut self, worker: WorkerIdentity) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.worker == worker)
            && (entry.accepted.is_some() || entry.pending.as_ref().is_some_and(|pending| pending.queued))
        {
            entry.uncertain = true;
        }
    }
    pub fn visibility_lost(&mut self, worker: WorkerIdentity) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.worker == worker) {
            entry.fresh_required = true;
        }
    }
    pub fn tick(&mut self, now: Instant) {
        for entry in &mut self.entries {
            if entry
                .pending
                .as_ref()
                .is_some_and(|pending| pending.queued && now.saturating_duration_since(pending.began) >= RECEIPT_WAIT)
            {
                entry.uncertain = true;
            }
        }
    }
    pub fn uncertain(&self, source: PreviewSource) -> bool {
        self.entries.iter().any(|entry| entry.worker == source.worker() && entry.uncertain)
    }
    pub fn pending_native(&self, source: PreviewSource) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.worker == source.worker() && entry.pending.as_ref().is_some_and(|pending| pending.queued))
    }
    /// At most one globally paced source; no accumulated tokens or catch-up.
    pub fn due(&mut self, eligible: &[ConnectionToken], now: Instant) -> Option<Accepted> {
        if self.next_global.is_some_and(|next| now < next) || eligible.is_empty() {
            return None;
        }
        for offset in 0..eligible.len() {
            let at = (self.cursor + offset) % eligible.len();
            let token = eligible[at];
            let Some(accepted) = self.accepted(token) else { continue };
            let entry = self.entries.iter_mut().find(|entry| entry.worker == token.source().worker())?;
            if now < entry.next {
                continue;
            }
            entry.next = now.checked_add(Duration::from_millis(accepted.options.rate.interval_millis()))?;
            self.next_global = now.checked_add(Duration::from_millis(100));
            self.cursor = (at + 1) % eligible.len();
            return Some(accepted);
        }
        None
    }
}

#[cfg(test)]
mod tests;
