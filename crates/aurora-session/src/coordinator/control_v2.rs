//! V2 entry points on the actual Coordinator/Entry, sharing activation state
//! and sent/received counters. These do not select the production transport.
use super::*;
use crate::control_v2::{
    CONTROL_V2_VERSION, ControlV2Command, ControlV2Effect, ControlV2Envelope, ControlV2Event, ControlV2Message, ControlV2Observation,
    ShowViewerQueryState, matches_reply,
};
use crate::show_viewer::{MAX_SHOW_RECEIPTS, ShowViewerReceipt, ShowViewerRequest};
use std::collections::VecDeque;

struct ShowOperation {
    request: ShowViewerRequest,
    sequence: u64,
    query: Option<u64>,
    receipt: Option<ShowViewerReceipt>,
    unavailable: bool,
}
#[derive(Default)]
pub(super) struct ShowOperations {
    highwater: u64,
    entries: VecDeque<ShowOperation>,
}

fn effects_v2(effects: Vec<Effect>) -> Vec<ControlV2Effect> {
    effects
        .into_iter()
        .map(|effect| match effect {
            Effect::Send(envelope) => ControlV2Effect::Send(ControlV2Envelope::from_control(envelope)),
            Effect::Notice(notice) => ControlV2Effect::Notice(notice),
        })
        .collect()
}
fn show_command(entry: &Entry, request: ShowViewerRequest, sequence: u64, query: bool) -> ControlV2Envelope {
    ControlV2Envelope {
        version: CONTROL_V2_VERSION,
        binding: request.target.binding,
        lease_epoch: entry.lease_epoch,
        sequence,
        reply_to: 0,
        message: ControlV2Message::Command(if query {
            ControlV2Command::QueryShowViewer(request)
        } else {
            ControlV2Command::ShowViewer(request)
        }),
    }
}

impl Coordinator {
    pub fn activate_control_v2(
        &mut self,
        worker: WorkerId,
        viewport: Option<HostedViewport>,
        now_ms: u64,
    ) -> Result<Vec<ControlV2Effect>, Error> {
        self.activate(worker, viewport, now_ms).map(effects_v2)
    }
    pub fn refresh_viewport_control_v2(
        &mut self,
        worker: WorkerId,
        viewport: HostedViewport,
        now_ms: u64,
    ) -> Result<Vec<ControlV2Effect>, Error> {
        self.refresh_viewport(worker, viewport, now_ms).map(effects_v2)
    }
    pub fn park_active_control_v2(&mut self, now_ms: u64) -> Result<Vec<ControlV2Effect>, Error> {
        self.park_active(now_ms).map(effects_v2)
    }
    pub fn detach_control_v2(&mut self, worker: WorkerId) -> Result<Vec<ControlV2Effect>, Error> {
        self.detach(worker).map(effects_v2)
    }
    pub fn detach_all_control_v2(&mut self) -> Vec<ControlV2Effect> {
        effects_v2(self.detach_all())
    }
    pub fn remove_control_v2(&mut self, worker: WorkerId) -> Vec<ControlV2Effect> {
        effects_v2(self.remove(worker))
    }
    pub fn tick_control_v2(&mut self, now_ms: u64) -> Vec<ControlV2Effect> {
        effects_v2(self.tick(now_ms))
    }

    /// Advertised correlation metadata only. The actual owned App/window and
    /// original capture deadline must still pass at the native consumer.
    pub fn ordinary_target_control_v2(&self, worker: WorkerId) -> Option<crate::show_viewer::ShowViewerTarget> {
        self.entries
            .get(&worker)
            .filter(|entry| entry.roster.live)
            .and_then(|entry| entry.ordinary)
    }

    pub fn show_viewer_control_v2(&mut self, request: ShowViewerRequest, now_ms: u64) -> Result<ControlV2Envelope, Error> {
        request.validate()?;
        self.now = self.now.max(now_ms);
        if self.pending.is_some() || self.active() == Some(request.target.binding.worker.worker) {
            return Err(Error::Busy);
        }
        let entry = self
            .entries
            .get_mut(&request.target.binding.worker.worker)
            .ok_or(Error::Unavailable)?;
        if request.target.binding != entry.roster.binding || request.target.lease_epoch != entry.lease_epoch {
            return Err(Error::Binding);
        }
        if !entry.roster.live
            || self.now.saturating_sub(entry.last_seen) >= entry.receipt_timeout()
            || entry.ordinary != Some(request.target)
        {
            return Err(Error::Unavailable);
        }
        // A host operation never resends Show, even after a terminal outcome.
        // Only a read-only query may recover the same original operation.
        if request.request <= entry.shows.highwater {
            return Err(Error::Stale);
        }
        let eviction = if entry.shows.entries.len() == MAX_SHOW_RECEIPTS {
            Some(
                entry
                    .shows
                    .entries
                    .iter()
                    .position(|op| (op.receipt.is_some() || op.unavailable) && op.query.is_none())
                    .ok_or(Error::Capacity)?,
            )
        } else {
            None
        };
        let sequence = entry.next_sequence()?;
        let envelope = show_command(entry, request, sequence, false);
        envelope.validate()?;
        if let Some(index) = eviction {
            let _ = entry.shows.entries.remove(index);
        }
        entry.shows.entries.push_back(ShowOperation {
            request,
            sequence,
            query: None,
            receipt: None,
            unavailable: false,
        });
        entry.shows.highwater = request.request;
        entry.sent = sequence;
        Ok(envelope)
    }

    pub fn query_show_viewer_control_v2(&mut self, request: ShowViewerRequest, now_ms: u64) -> Result<ControlV2Envelope, Error> {
        request.validate()?;
        self.now = self.now.max(now_ms);
        let entry = self
            .entries
            .get_mut(&request.target.binding.worker.worker)
            .ok_or(Error::Unavailable)?;
        if request.target.binding != entry.roster.binding || request.target.lease_epoch != entry.lease_epoch {
            return Err(Error::Binding);
        }
        if !entry.roster.live || self.now.saturating_sub(entry.last_seen) >= entry.receipt_timeout() {
            return Err(Error::Unavailable);
        }
        let operation = entry
            .shows
            .entries
            .iter()
            .find(|op| op.request.request == request.request)
            .ok_or(Error::Stale)?;
        if operation.request != request {
            return Err(Error::Binding);
        }
        if operation.unavailable {
            return Err(Error::Stale);
        }
        if operation.query.is_some() {
            return Err(Error::Busy);
        }
        let sequence = entry.next_sequence()?;
        let envelope = show_command(entry, request, sequence, true);
        envelope.validate()?;
        let operation = entry
            .shows
            .entries
            .iter_mut()
            .find(|op| op.request == request)
            .ok_or(Error::Stale)?;
        operation.query = Some(sequence);
        entry.sent = sequence;
        Ok(envelope)
    }

    pub fn observe_control_v2(&mut self, envelope: &ControlV2Envelope, now_ms: u64) -> Result<ControlV2Observation, Error> {
        envelope.validate()?;
        let ControlV2Message::Event(event) = &envelope.message else {
            return Err(Error::Invalid);
        };
        if let ControlV2Event::ControlUnavailable { command } = event {
            // Exact operation metadata only: no receive/liveness/activation
            // counter changes and no fabricated native ledger outcome.
            let (request, query) = match command {
                ControlV2Command::ShowViewer(request) => (*request, false),
                ControlV2Command::QueryShowViewer(request) => (*request, true),
                _ => return Err(Error::Unavailable),
            };
            let entry = self.entries.get_mut(&envelope.binding.worker.worker).ok_or(Error::Unavailable)?;
            if envelope.binding.worker != entry.roster.binding.worker || envelope.lease_epoch != entry.lease_epoch {
                return Err(Error::Binding);
            }
            let operation = entry
                .shows
                .entries
                .iter_mut()
                .find(|op| op.request == request)
                .ok_or(Error::Stale)?;
            let expected = if query {
                operation.query.ok_or(Error::Stale)?
            } else {
                operation.sequence
            };
            if envelope.reply_to != expected || operation.unavailable || !query && operation.receipt.is_some() {
                return Err(Error::Stale);
            }
            if query {
                operation.query = None;
            }
            operation.unavailable = operation.receipt.is_none();
            return Ok(ControlV2Observation {
                show_unavailable: Some(request),
                ..Default::default()
            });
        }
        if let Some(control) = event.control() {
            // The real V1 state machine handles every normal activation/status
            // result; the adapter is typed and does not decode another wire.
            let effects = self.observe(&envelope.control_envelope(Message::Event(control)), now_ms)?;
            if let ControlV2Event::Status { ordinary, .. } = event
                && let Some(entry) = self.entries.get_mut(&envelope.binding.worker.worker)
            {
                entry.ordinary = ordinary.filter(|_| entry.roster.live && entry.roster.binding == envelope.binding);
            }
            return Ok(ControlV2Observation {
                effects: effects_v2(effects),
                ..Default::default()
            });
        }
        self.now = self.now.max(now_ms);
        let entry = self.entries.get_mut(&envelope.binding.worker.worker).ok_or(Error::Unavailable)?;
        if envelope.binding.worker != entry.roster.binding.worker || envelope.lease_epoch != entry.lease_epoch {
            return Err(Error::Binding);
        }
        if !entry.roster.live || envelope.sequence <= entry.received || self.now.saturating_sub(entry.last_seen) >= entry.receipt_timeout()
        {
            return Err(Error::Stale);
        }
        if envelope.reply_to > entry.sent {
            return Err(Error::Sequence);
        }
        let request = match event {
            ControlV2Event::ShowViewerReceipt(receipt) => receipt.request,
            ControlV2Event::ShowViewerQuery(result) => result.request,
            _ => return Err(Error::Invalid),
        };
        let operation = entry.shows.entries.iter().find(|op| op.request == request).ok_or(Error::Stale)?;
        let is_query = matches!(event, ControlV2Event::ShowViewerQuery(_));
        let sequence = if is_query {
            operation.query.ok_or(Error::Stale)?
        } else {
            operation.sequence
        };
        if !matches_reply(&show_command(entry, request, sequence, is_query), envelope) {
            return Err(Error::Stale);
        }
        let terminal = match event {
            ControlV2Event::ShowViewerReceipt(receipt) => Some(*receipt),
            ControlV2Event::ShowViewerQuery(result) => match result.result {
                ShowViewerQueryState::Terminal(receipt) => Some(receipt),
                _ => None,
            },
            _ => None,
        };
        if terminal.is_some_and(|receipt| operation.receipt.is_some_and(|previous| previous != receipt)) {
            return Err(Error::Stale);
        }
        // Query/Show results update only their exact immutable operation and the
        // same Entry receive counter. They cannot complete/advance a handoff,
        // change roster source/ordinary metadata, Active, viewport or ownership.
        let operation = entry
            .shows
            .entries
            .iter_mut()
            .find(|op| op.request == request)
            .ok_or(Error::Stale)?;
        if operation.unavailable {
            return Err(Error::Stale);
        }
        if is_query {
            operation.query = None;
        }
        if terminal.is_some() {
            operation.receipt = terminal;
        }
        entry.received = envelope.sequence;
        // The sender counter spans this boot, but an old-source operation
        // receipt is not a fresh receipt from the current native source.
        if envelope.binding == entry.roster.binding {
            entry.last_seen = self.now;
        }
        Ok(ControlV2Observation {
            show_receipt: if let ControlV2Event::ShowViewerReceipt(receipt) = event {
                Some(*receipt)
            } else {
                None
            },
            show_query: if let ControlV2Event::ShowViewerQuery(result) = event {
                Some(*result)
            } else {
                None
            },
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests;
