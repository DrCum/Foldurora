//! One exclusive model/store boundary for Broker admission and GUI metadata.
//! Comparing versions happens before ANY model call (including tick/retire).
use super::*;
use crate::session_host::preview_io::{
    CaptureControlGuard, ConnectionToken, EpochFence, FrameHandoff, HostPermit, LayoutReceipt, ObservationDomain, PolicyReceipt,
    PublicationStamp, RetainedFrame, VersionedObservation, VersionedObservationSet,
};
use aurora_session::preview_image::{CaptureClaim, ImageBudget, PreviewPolicy, SourceImages};
use std::collections::VecDeque;
use std::time::Duration;

struct Garbage {
    _raw: Option<Arc<RetainedFrame>>,
    _prepared: Option<Arc<prepare::Payload>>,
    _handoff: Option<FrameHandoff>,
}
impl Garbage {
    fn raw(raw: Arc<RetainedFrame>) -> Self {
        Self {
            _raw: Some(raw),
            _prepared: None,
            _handoff: None,
        }
    }
    fn prepared(prepared: Arc<prepare::Payload>) -> Self {
        Self {
            _raw: None,
            _prepared: Some(prepared),
            _handoff: None,
        }
    }
    fn handoff(handoff: FrameHandoff) -> Self {
        Self {
            _raw: None,
            _prepared: None,
            _handoff: Some(handoff),
        }
    }
}
struct Entry {
    source: PreviewSource,
    domain: Option<ObservationDomain>,
    policy: Option<PolicyReceipt>,
    policy_floor: Option<PreviewPolicy>,
    layout: Option<LayoutReceipt>,
    layout_current: bool,
    model: Option<SourceImages>,
    pending: Option<CaptureClaim>,
    pending_at: u64,
    pending_layout: Option<aurora_session::preview_image::PreviewLayout>,
    token: Option<ConnectionToken>,
    raw: Option<Arc<RetainedFrame>>,
    prepared: Option<Arc<prepare::Payload>>,
    candidate: Arc<AtomicU64>,
    prepare_pending: bool,
    admitted_candidate: Option<AdmittedImage>,
    lifetime: EpochFence,
    status: ImageStatus,
    broken_domain: bool,
}
impl Entry {
    fn new(source: PreviewSource) -> Self {
        Self {
            source,
            domain: None,
            policy: None,
            policy_floor: None,
            layout: None,
            layout_current: false,
            model: None,
            pending: None,
            pending_at: 0,
            pending_layout: None,
            token: None,
            raw: None,
            prepared: None,
            candidate: Arc::new(AtomicU64::new(0)),
            prepare_pending: false,
            admitted_candidate: None,
            lifetime: EpochFence::new(),
            status: ImageStatus::NoImage,
            broken_domain: false,
        }
    }
    fn retire(&mut self, garbage: &mut Vec<Garbage>) {
        self.lifetime.revoke();
        self.candidate.store(0, Ordering::Release);
        self.prepare_pending = false;
        self.admitted_candidate = None;
        if let Some(mut model) = self.model.take() {
            model.retire();
        }
        self.pending = None;
        self.pending_layout = None;
        if let Some(raw) = self.raw.take() {
            garbage.push(Garbage::raw(raw));
        }
        if let Some(prepared) = self.prepared.take() {
            garbage.push(Garbage::prepared(prepared));
        }
        self.status = ImageStatus::NoImage;
    }
    fn cancel(&mut self) {
        self.pending_layout = None;
        if let Some(claim) = self.pending.take()
            && let Some(model) = &mut self.model
        {
            model.cancel_capture(claim);
        }
    }
    fn layout_matches(&self, claim: CaptureClaim) -> bool {
        self.layout_current
            && self.layout.is_some_and(|receipt| {
                let layout = receipt.layout();
                layout.source() == claim.source()
                    && layout.epochs() == claim.epochs()
                    && layout.activation() == claim.activation()
                    && layout.demand() == claim.demand()
                    && layout.geometry(claim.cap()).ok() == Some(claim.geometry())
                    && self.pending_layout == Some(layout)
            })
    }
    fn break_domain(&mut self, garbage: &mut Vec<Garbage>) {
        self.retire(garbage);
        self.broken_domain = true;
        self.status = ImageStatus::Denied;
    }
}

pub(super) struct Store {
    control_applied: u64,
    io_applied: Option<PublicationStamp>,
    io_service: u64,
    entries: Vec<Entry>,
    views: Vec<ViewDescriptor>,
    generation: u64,
    request: u64,
    failed_views: u64,
    capture_generation: u64,
    namespace_floor: u64,
    ledger: demand::Ledger,
    settings: Vec<PendingSetting>,
    settings_done: VecDeque<SettingsReceipt>,
    exchanges: VecDeque<ExchangeReceipt>,
    garbage: Vec<Garbage>,
}
impl Store {
    pub fn new() -> Self {
        Self {
            control_applied: 0,
            io_applied: None,
            io_service: 0,
            entries: Vec::new(),
            views: Vec::new(),
            generation: 1,
            request: 0,
            failed_views: 0,
            capture_generation: 1,
            namespace_floor: 0,
            ledger: demand::Ledger::new(),
            settings: Vec::new(),
            settings_done: VecDeque::with_capacity(32),
            exchanges: VecDeque::with_capacity(32),
            garbage: Vec::new(),
        }
    }
    fn drain_garbage(&mut self) -> Vec<Garbage> {
        std::mem::take(&mut self.garbage)
    }
    fn apply_control(&mut self, control: &ControlSnapshot) -> Result<(), PreviewError> {
        if !control.current() || control.serial < self.control_applied {
            return Err(PreviewError::Stale);
        }
        if control.serial == self.control_applied {
            return Ok(());
        }
        // The complete newer roster is the only authority for absence. Old
        // copies cannot retire a model after another writer advanced this mark.
        for entry in &mut self.entries {
            if !control
                .sources
                .iter()
                .any(|(_, source)| source.source() == entry.source && source.current(Instant::now()))
            {
                entry.retire(&mut self.garbage);
                self.ledger.lost(entry.source.worker());
            }
        }
        self.entries
            .retain(|entry| control.sources.iter().any(|(_, source)| source.source() == entry.source));
        self.views.retain(|view| {
            control
                .sources
                .iter()
                .any(|(_, source)| source.source() == view.source && source.current(Instant::now()))
        });
        let mut at = 0;
        while at < self.settings.len() {
            let pending = self.settings[at].receipt;
            if !self.views.iter().any(|view| {
                view.view == pending.view && view.namespace == pending.namespace && view.source == pending.ticket.source().preview()
            }) {
                let mut cancelled = self.settings.swap_remove(at);
                cancelled.receipt.outcome = SettingsOutcome::Cancelled;
                bounded_push(&mut self.settings_done, cancelled.receipt);
            } else {
                at += 1;
            }
        }
        for (_, source) in &control.sources {
            if !self.entries.iter().any(|entry| entry.source == source.source()) {
                self.entries.push(Entry::new(source.source()));
            }
        }
        self.control_applied = control.serial;
        Ok(())
    }
    fn reconcile_frontdoor(&mut self, shared: &Shared) {
        let capture = shared.frontdoor.capture_generation();
        if capture != self.capture_generation {
            for entry in &mut self.entries {
                entry.cancel();
            }
            self.capture_generation = capture;
        }
        let generation = shared.frontdoor.view_generation();
        let failed = shared.frontdoor.failed_views();
        if failed > self.failed_views || !shared.frontdoor.current() {
            // A failed/missing newer FIFO replacement means EMPTY, never old
            // subscriptions. Admission of its exact packet can repopulate later.
            self.views.clear();
            self.generation = generation;
            self.request = 0;
            self.failed_views = failed;
            for entry in &mut self.entries {
                entry.retire(&mut self.garbage);
                self.ledger.visibility_lost(entry.source.worker());
            }
            for mut pending in self.settings.drain(..) {
                pending.receipt.outcome = SettingsOutcome::Cancelled;
                bounded_push(&mut self.settings_done, pending.receipt);
            }
        }
    }
    fn visible(&self, source: PreviewSource) -> bool {
        self.views.iter().any(|view| view.source == source && view.visible)
    }
    fn apply_metadata(
        &mut self,
        shared: &Shared,
        control: &ControlSnapshot,
        set: &VersionedObservationSet,
        io_service: u64,
        budget: &ImageBudget,
        now: u64,
    ) -> Result<(), PreviewError> {
        let stamp = set.stamp();
        if !set.current() || io_service != shared.io_generation.load(Ordering::Acquire) || io_service < self.io_service {
            return Err(PreviewError::Stale);
        }
        if io_service == self.io_service
            && let Some(old) = self.io_applied
        {
            if stamp.publisher() != old.publisher() || stamp.serial() < old.serial() {
                return Err(PreviewError::Stale);
            }
        }
        // Check every join and origin before ANY model mutation, including
        // absence and expiry handling. One stale source cannot partially apply.
        let mut broken = Vec::new();
        for observed in set.sources() {
            let Some(source) = control.images.iter().find(|source| source.source() == observed.domain().source()) else {
                continue;
            };
            let guard = ControlGuard::from_host(source).map_err(|_| PreviewError::Stale)?;
            if observed.domain().registration() != source.registration() || !observed.domain().matches_control(&guard) {
                return Err(PreviewError::Stale);
            }
            if let Some(entry) = self.entries.iter().find(|entry| entry.source == source.source())
                && entry.domain.as_ref().is_some_and(|domain| domain.same_domain(observed.domain()))
            {
                match validate_receipts(entry, observed) {
                    Ok(()) => {}
                    Err(ReceiptFailure::Stale) => return Err(PreviewError::Stale),
                    Err(ReceiptFailure::Invariant) => broken.push(entry.source),
                }
            }
            if let Some(receipt) = observed.policy() {
                let _ = shared.observed(receipt.received_at())?;
                if receipt.received_at() > Instant::now() {
                    return Err(PreviewError::Stale);
                }
            }
        }
        // The authenticated complete snapshot is accepted before fallible model
        // work. Even an allocation/model failure cannot let an older absence
        // list roll back entries already synchronized from this publication.
        self.io_applied = Some(stamp);
        self.io_service = io_service;
        for entry in &mut self.entries {
            if broken.contains(&entry.source) {
                entry.break_domain(&mut self.garbage);
                self.ledger.lost(entry.source.worker());
            }
            let observed = set.sources().iter().find(|observed| observed.domain().source() == entry.source);
            let source = control.images.iter().find(|source| source.source() == entry.source);
            let visible = self.views.iter().any(|view| view.source == entry.source && view.visible);
            let Some((observed, _source)) = observed.zip(source) else {
                entry.retire(&mut self.garbage);
                entry.status = if source.is_none() {
                    ImageStatus::Unsupported
                } else {
                    ImageStatus::NoImage
                };
                self.ledger.lost(entry.source.worker());
                continue;
            };
            let observation = observed.observation();
            if !observation.current() {
                entry.retire(&mut self.garbage);
                entry.status = ImageStatus::Denied;
                self.ledger.lost(entry.source.worker());
                continue;
            }
            let changed_domain = entry.domain.as_ref().is_none_or(|old| !old.same_domain(observed.domain()));
            if changed_domain {
                entry.retire(&mut self.garbage);
                entry.domain = Some(observed.domain().clone());
                // Native policy floor is kept below across connection/model
                // replacement; origin comparison itself is domain-scoped.
                entry.layout = None;
                entry.layout_current = false;
                entry.policy = None;
                entry.broken_domain = false;
            }
            if entry.broken_domain {
                entry.status = ImageStatus::Denied;
                continue;
            }
            entry.token = Some(observation.token);
            if entry.pending.is_some() && now.saturating_sub(entry.pending_at) >= 1000 {
                entry.cancel();
            }
            if let Some(receipt) = observed.policy().copied() {
                let newer = changed_domain || entry.policy.is_none_or(|old| receipt.origin().serial() > old.origin().serial());
                if newer {
                    if entry
                        .policy_floor
                        .is_some_and(|old| receipt.policy().validate_successor(old).is_err())
                    {
                        entry.retire(&mut self.garbage);
                        entry.status = ImageStatus::Denied;
                        continue;
                    }
                    if entry.pending.is_some_and(|claim| !claim.matches_policy(receipt.policy())) {
                        entry.cancel();
                    }
                    if let Some(model) = &mut entry.model {
                        let observed_at = shared.observed(receipt.received_at())?;
                        if model.sync_observed(receipt.policy(), observed_at, now).is_err() {
                            entry.retire(&mut self.garbage);
                        }
                    }
                    entry.policy = Some(receipt);
                    entry.policy_floor = Some(receipt.policy());
                }
            }
            if let Some(receipt) = observed.layout().copied()
                && (changed_domain || entry.layout.is_none_or(|old| receipt.origin().serial() > old.origin().serial()))
            {
                entry.layout = Some(receipt);
            }
            entry.layout_current = observed.layout().is_some();
            if entry.pending.is_some_and(|claim| !entry.layout_matches(claim)) {
                entry.cancel();
            }
            if !visible {
                entry.retire(&mut self.garbage);
                continue;
            }
            // Explicit same-domain new-owner installation uses ONLY accepted
            // Store facts, their original timestamp and current domain/heads.
            if entry.model.is_none()
                && let Some(receipt) = entry.policy
            {
                let observed_at = shared.observed(receipt.received_at())?;
                let mut model = SourceImages::new(entry.source, budget.clone()).map_err(|_| PreviewError::Unavailable)?;
                if model.sync_observed(receipt.policy(), observed_at, now).is_ok() {
                    entry.lifetime = EpochFence::new();
                    entry.model = Some(model);
                }
            }
            if let Some(model) = &mut entry.model {
                if model.retained(now).is_err() {
                    entry.retire(&mut self.garbage);
                }
            }
            entry.status = if entry.prepared.is_some() {
                ImageStatus::LastActive
            } else {
                ImageStatus::Pending
            };
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReceiptFailure {
    Stale,
    Invariant,
}
fn receipt_order(
    old: super::super::preview_io::ReceiptOrigin,
    new: super::super::preview_io::ReceiptOrigin,
    equal: bool,
) -> Result<(), ReceiptFailure> {
    if old.publisher() != new.publisher() || old.connection() != new.connection() {
        return Err(ReceiptFailure::Invariant);
    }
    if new.serial() < old.serial() {
        return Err(ReceiptFailure::Stale);
    }
    if new.serial() == old.serial() && !equal {
        return Err(ReceiptFailure::Invariant);
    }
    Ok(())
}
fn validate_receipts(entry: &Entry, observed: &VersionedObservation) -> Result<(), ReceiptFailure> {
    if entry.policy.is_some() && observed.policy().is_none() {
        return Err(ReceiptFailure::Stale);
    }
    if let (Some(old), Some(new)) = (entry.policy, observed.policy().copied()) {
        receipt_order(old.origin(), new.origin(), new == old)?;
    }
    if let (Some(old), Some(new)) = (entry.layout, observed.layout().copied()) {
        receipt_order(old.origin(), new.origin(), new == old)?;
    }
    Ok(())
}
fn bounded_push<T>(receipts: &mut VecDeque<T>, value: T) {
    if receipts.len() == 32 {
        receipts.pop_front();
    }
    receipts.push_back(value);
}

pub(super) fn read(shared: &Arc<Shared>, view: View, namespace: ViewNamespace) -> Result<PreviewRead, PreviewError> {
    let control = shared.try_control()?.clone();
    let io = shared.io.try_lock().ok().and_then(|io| io.clone());
    let io_service = io.as_ref().map_or(0, |(generation, _)| *generation);
    let metadata = io.as_ref().and_then(|(_, io)| io.try_observations().ok());
    let (result, garbage) = {
        let mut store = shared.try_store()?;
        let now = shared.now(); // Sample AFTER the exclusive store lock.
        let result = (|| {
            store.apply_control(&control)?;
            store.reconcile_frontdoor(shared);
            if store.generation != shared.frontdoor.view_generation() {
                return Err(PreviewError::Stale);
            }
            let descriptor = *store
                .views
                .iter()
                .find(|descriptor| descriptor.view == view && descriptor.namespace == namespace)
                .filter(|descriptor| descriptor.visible)
                .ok_or(PreviewError::Stale)?;
            let source = control
                .sources
                .iter()
                .find(|(_, source)| source.source() == descriptor.source)
                .map(|(_, source)| source.clone())
                .filter(|source| source.current(Instant::now()))
                .ok_or(PreviewError::Unavailable)?;
            let mut image_status = if shared.io_failed.load(Ordering::Acquire) {
                ImageStatus::NoImage
            } else if control.images.iter().any(|image| image.source() == descriptor.source) {
                ImageStatus::Busy
            } else {
                ImageStatus::Unsupported
            };
            let mut metadata_current = false;
            if let Some((metadata, (_, io))) = metadata.as_ref().zip(io.as_ref()) {
                metadata_current = store
                    .apply_metadata(shared, &control, metadata, io_service, &io.image_budget(), now)
                    .is_ok();
            }
            let view_guard = shared.frontdoor.view_fence(store.generation);
            let image_guard = if metadata_current {
                control
                    .images
                    .iter()
                    .find(|source| source.source() == descriptor.source)
                    .and_then(|source| ControlGuard::from_host(source).ok())
                    .zip(metadata.as_ref())
                    .map(|(control, set)| ImageReadGuard {
                        control,
                        metadata_head: set.head_guard(),
                        service: io_service,
                    })
            } else {
                None
            };
            let mut image = None;
            if metadata_current {
                if let Some(entry) = store.entries.iter().find(|entry| entry.source == descriptor.source) {
                    image_status = entry.status;
                    if let (Some(payload), Some(source), Some(metadata)) = (
                        entry.prepared.clone(),
                        control.images.iter().find(|source| source.source() == descriptor.source),
                        metadata.as_ref(),
                    ) && let Ok(control_guard) = ControlGuard::from_host(source)
                    {
                        let prepared = Arc::new(PreparedImage::for_read(
                            payload,
                            prepare::ReadScope {
                                control: control_guard,
                                control_head: control.guard.clone(),
                                metadata_head: metadata.head_guard(),
                                view: view_guard.clone(),
                                shared: shared.clone(),
                                binding: (descriptor.view, descriptor.namespace),
                                service: io_service,
                            },
                        ));
                        if prepared.current() {
                            image_status = ImageStatus::LastActive;
                            image = Some(prepared);
                        }
                    }
                }
            }
            if !control.current() || !source.current(Instant::now()) || !view_guard.current() {
                return Err(PreviewError::Stale);
            }
            if metadata.as_ref().is_some_and(|set| !set.current()) {
                image = None;
                image_status = ImageStatus::Busy;
            }
            if image.is_none() && image_status == ImageStatus::LastActive {
                image_status = ImageStatus::NoImage;
            }
            Ok(PreviewRead {
                view: descriptor,
                generation: store.generation,
                request: store.request,
                view_guard,
                source,
                control_head: control.guard.clone(),
                shared: shared.clone(),
                now,
                status: image_status,
                image,
                image_guard,
                commands: true,
                settings: store.settings_done.iter().copied().collect(),
                exchanges: store.exchanges.iter().copied().collect(),
            })
        })();
        (result, store.drain_garbage())
    };
    drop(garbage);
    result
}
pub(super) fn status(shared: &Arc<Shared>) -> Result<PreviewStatus, PreviewError> {
    let control = shared.try_control()?.clone();
    let (result, garbage) = {
        let mut store = shared.try_store()?;
        let _now = shared.now();
        let result = (|| {
            store.apply_control(&control)?;
            store.reconcile_frontdoor(shared);
            let result = PreviewStatus {
                sources: control.sources.iter().map(|(source, _)| *source).collect(),
                control: control.clone(),
                generation: store.generation,
                request: store.request,
                view_guard: shared.frontdoor.view_fence(store.generation),
                shared: shared.clone(),
                settings: store.settings_done.iter().copied().collect(),
                exchanges: store.exchanges.iter().copied().collect(),
            };
            result.current().then_some(result).ok_or(PreviewError::Stale)
        })();
        (result, store.drain_garbage())
    };
    drop(garbage);
    result
}

// Actual Broker lifecycle/IO glue follows here; it uses these same store paths.
struct PendingSetting {
    receipt: SettingsReceipt,
    _slot: OperationSlot,
    began: Instant,
}
mod runtime;
pub(in crate::session_host) use runtime::{BootstrapAdmission, BrokerProjection, RequestKind};
#[cfg(test)]
mod tests;
