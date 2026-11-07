//! Actual Broker consumer: accepted proofs -> ordered store -> owned admission.
//! Endpoint startup remains an explicit reviewed capability join, never a UI
//! side effect. Failure has no ordinary hosting/Review/Chat error channel.
use super::*;
use crate::session_host::preview_bootstrap::{PreviewBootstrap, PreviewSecret};
use crate::session_host::preview_io::{PreviewCommand, SourceUpdate};
use crate::session_host::transport::preview_control::AcceptedPreviewCapture;
use aurora_session::{HostId, InstallationId, WorkerIdentity};

pub(in crate::session_host) enum RequestKind {
    Views {
        generation: u64,
        views: Vec<ViewDescriptor>,
    },
    Settings {
        view: View,
        namespace: ViewNamespace,
        ticket: Box<Ticket>,
        base: State,
        desired: Options,
    },
    Exchange {
        namespace: ViewNamespace,
        exchange: Box<Exchange>,
        viewport: HostedViewport,
    },
}
struct Request {
    ticket: u64,
    kind: RequestKind,
    slot: OperationSlot,
    cancelled: bool,
}
struct ExchangeRecord {
    request: u64,
    namespace: ViewNamespace,
    exchange: Exchange,
    viewport: HostedViewport,
    expected: Option<u64>,
    rolling_back: bool,
    began: Instant,
    outcome: Option<ExchangeOutcome>,
    _slot: OperationSlot,
}
enum Sent {
    Demand(demand::Offer, bool),
    Capture(Box<CaptureClaim>, bool),
}
/// Pending is an actor state; None is not silently substituted while port
/// initialization races a worker launch. A terminal image failure can fall
/// back to an ordinary launch without relaunching or terminating a worker.
pub(in crate::session_host) enum BootstrapAdmission {
    Pending,
    Ready(PreviewBootstrap),
    Absent,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartupState {
    Pending,
    Ready(u16),
    Failed,
}
fn startup_state(port: Option<u16>, ended: bool, elapsed: Duration) -> StartupState {
    if ended {
        StartupState::Failed
    } else if let Some(port) = port {
        StartupState::Ready(port)
    } else if elapsed >= Duration::from_secs(2) {
        StartupState::Failed
    } else {
        StartupState::Pending
    }
}
pub(in crate::session_host) struct BrokerProjection {
    reader: PreviewReader,
    channel: Option<Arc<HostChannel>>,
    startup: Option<Instant>,
    failed: bool,
    requests: VecDeque<Request>,
    exchange: Option<ExchangeRecord>,
    sent: VecDeque<Sent>,
    frames: VecDeque<FrameHandoff>,
    completions: Vec<prepare::Completion>,
    preparer: Option<prepare::Preparer>,
    exits: Vec<WorkerIdentity>,
}
impl BrokerProjection {
    pub(in crate::session_host) fn new(reader: PreviewReader, installation: InstallationId, host: HostId, joined: bool) -> Self {
        // This branch is reachable only at the root-reviewed actual join.
        let channel = joined.then(|| HostChannel::start(installation, host).ok()).flatten().map(Arc::new);
        Self {
            startup: channel.as_ref().map(|_| Instant::now()),
            failed: joined && channel.is_none(),
            reader,
            channel,
            requests: VecDeque::new(),
            exchange: None,
            sent: VecDeque::new(),
            frames: VecDeque::new(),
            completions: Vec::new(),
            preparer: None,
            exits: Vec::new(),
        }
    }
    pub(in crate::session_host) fn queue(&mut self, ticket: u64, kind: RequestKind, slot: OperationSlot) {
        // Every packet owns one of the shared sixteen operation slots from
        // BEFORE FIFO admission, through its eventual immutable receipt.
        self.requests.push_back(Request {
            ticket,
            kind,
            slot,
            cancelled: false,
        });
    }
    pub(in crate::session_host) fn bootstrap(&mut self) -> BootstrapAdmission {
        let Some(channel) = &self.channel else {
            return BootstrapAdmission::Absent;
        };
        let port = match startup_state(
            channel.port(),
            channel.ended(),
            self.startup.map_or(Duration::ZERO, |at| at.elapsed()),
        ) {
            StartupState::Pending => return BootstrapAdmission::Pending,
            StartupState::Failed => {
                self.fail_images();
                return BootstrapAdmission::Absent;
            }
            StartupState::Ready(port) => port,
        };
        match PreviewSecret::random() {
            Ok(key) => BootstrapAdmission::Ready(PreviewBootstrap { port, key }),
            Err(_) => {
                self.fail_images();
                BootstrapAdmission::Absent
            }
        }
    }
    fn install_io(&self) -> Result<(), PreviewError> {
        let Some(channel) = &self.channel else { return Ok(()) };
        let mut io = self.reader.0.io.try_lock().map_err(|_| PreviewError::Busy)?;
        if io.as_ref().is_some_and(|(_, old)| Arc::ptr_eq(old, channel)) {
            return Ok(());
        }
        let generation = self
            .reader
            .0
            .io_generation
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| PreviewError::Exhausted)?
            + 1;
        *io = Some((generation, channel.clone()));
        Ok(())
    }
    fn fail_images(&mut self) {
        self.failed = true;
        self.reader.0.io_failed.store(true, Ordering::Release);
        let _ = self.reader.0.frontdoor.capture_intent();
        if let Some(channel) = self.channel.take() {
            channel.stop();
        }
        let _ = self
            .reader
            .0
            .io_generation
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1));
        if let Ok(mut io) = self.reader.0.io.try_lock() {
            *io = None;
        }
    }
    pub(in crate::session_host) fn publish_control(
        &self,
        sources: Vec<(ViewSource, AcceptedViewSource)>,
        images: Vec<AcceptedPreviewSource>,
        active: Option<PreviewSource>,
    ) -> Result<(), PreviewError> {
        let shared = &self.reader.0;
        let images = if self.failed { Vec::new() } else { images };
        let mut published = shared.try_control()?;
        let same = published.active == active
            && published.sources.len() == sources.len()
            && published.sources.iter().zip(&sources).all(|((a, x), (b, y))| {
                a == b
                    && x.source() == y.source()
                    && x.observed_at() == y.observed_at()
                    && x.expires_at() == y.expires_at()
                    && x.witness().same(&y.witness())
            })
            && published.images.len() == images.len()
            && published.images.iter().zip(&images).all(|(a, b)| {
                a.source() == b.source()
                    && a.observed_at() == b.observed_at()
                    && a.expires_at() == b.expires_at()
                    && a.registration() == b.registration()
                    && a.witness().same(&b.witness())
            });
        if same && published.current() {
            return Ok(());
        }
        let serial = published.serial.checked_add(1).ok_or_else(|| {
            shared.frontdoor.retire();
            PreviewError::Exhausted
        })?;
        *published = Arc::new(ControlSnapshot {
            serial,
            sources,
            images,
            active,
            guard: EpochFence::generation(shared.control_head.clone(), serial),
        });
        shared.control_head.store(serial, Ordering::Release);
        Ok(())
    }
    pub(in crate::session_host) fn publish_permits(&mut self, permits: &[HostPermit]) {
        if self.install_io().is_err() {
            return;
        }
        if let Some(channel) = &self.channel
            && channel.publish_permits(permits).is_err()
        {
            // IO already synchronously revokes ambiguous source/auth gates.
            // The next poll reconciles image state; ordinary control continues.
            let _ = self.reader.0.frontdoor.capture_intent();
        }
    }
    pub(in crate::session_host) fn capture_admitted(&self, source: PreviewSource) -> bool {
        let Ok(store) = self.reader.0.try_store() else { return false };
        store.generation == self.reader.0.frontdoor.view_generation()
            && store.visible(source)
            && store
                .entries
                .iter()
                .find(|entry| entry.source == source)
                .is_some_and(|entry| entry.model.is_some() && entry.token.and_then(|token| store.ledger.accepted(token)).is_some())
    }
    pub(in crate::session_host) fn exited(&mut self, worker: WorkerIdentity) {
        if !self.exits.contains(&worker) {
            if self.exits.len() < MAX_WORKERS {
                self.exits.push(worker);
            } else {
                // Refuse further images instead of fabricating a reservation
                // release when the bounded exit bookkeeping is saturated.
                self.fail_images();
            }
        }
    }
    pub(in crate::session_host) fn stop(&mut self) {
        self.reader.0.frontdoor.retire();
        self.external_intent();
        if let Some(channel) = &self.channel {
            channel.stop();
        }
    }
}

impl Drop for BrokerProjection {
    fn drop(&mut self) {
        self.stop();
    }
}

impl BrokerProjection {
    fn apply_requests(&mut self, store: &mut Store, control: &ControlSnapshot) {
        while let Some(request) = self.requests.front() {
            if matches!(request.kind, RequestKind::Exchange { .. }) {
                break;
            }
            let request = self.requests.pop_front().expect("bounded request front exists");
            match request.kind {
                RequestKind::Views { generation, views } => {
                    if generation != self.reader.0.frontdoor.view_generation() {
                        continue;
                    }
                    let valid = views.len() <= MAX_WORKERS
                        && views.iter().enumerate().all(|(at, view)| {
                            view.valid()
                                && !views[..at]
                                    .iter()
                                    .any(|old| old.view == view.view || old.namespace == view.namespace)
                                && control
                                    .sources
                                    .iter()
                                    .any(|(_, source)| source.source() == view.source && source.current(Instant::now()))
                                && (view.namespace.get() > store.namespace_floor
                                    || store.views.iter().any(|old| {
                                        old.namespace == view.namespace
                                            && old.view == view.view
                                            && old.source == view.source
                                            && old.ticket.same_binding(view.ticket)
                                    }))
                        });
                    if !valid {
                        self.reader.0.frontdoor.view_failed(generation);
                        store.reconcile_frontdoor(&self.reader.0);
                    } else {
                        for entry in &mut store.entries {
                            if !views.iter().any(|view| view.source == entry.source && view.visible) {
                                entry.retire(&mut store.garbage);
                                store.ledger.visibility_lost(entry.source.worker());
                            }
                        }
                        store.views = views;
                        store.namespace_floor = store
                            .views
                            .iter()
                            .map(|view| view.namespace.get())
                            .max()
                            .unwrap_or(0)
                            .max(store.namespace_floor);
                        store.generation = generation;
                        store.request = request.ticket;
                        let mut at = 0;
                        while at < store.settings.len() {
                            let pending = &store.settings[at].receipt;
                            if !store.views.iter().any(|view| {
                                view.view == pending.view
                                    && view.namespace == pending.namespace
                                    && view.source == pending.ticket.source().preview()
                            }) {
                                let mut pending = store.settings.swap_remove(at);
                                pending.receipt.outcome = SettingsOutcome::Cancelled;
                                bounded_push(&mut store.settings_done, pending.receipt);
                            } else {
                                at += 1;
                            }
                        }
                    }
                }
                RequestKind::Settings {
                    view,
                    namespace,
                    ticket,
                    base,
                    desired,
                } => {
                    let ticket = *ticket;
                    let receipt = SettingsReceipt {
                        request: request.ticket,
                        view,
                        namespace,
                        ticket,
                        base,
                        desired,
                        outcome: SettingsOutcome::Rejected,
                    };
                    // Input nonce is manager-local: focus/Chat/Pin may change it
                    // after capture without replacing this exact view binding.
                    let valid = store.generation == self.reader.0.frontdoor.view_generation()
                        && ticket.view() == view
                        && base.request.is_none()
                        && store.views.iter().any(|descriptor| {
                            descriptor.view == view
                                && descriptor.namespace == namespace
                                && descriptor.source == ticket.source().preview()
                                && descriptor.ticket.same_binding(ticket)
                                && descriptor.options == base.accepted
                        })
                        && control
                            .sources
                            .iter()
                            .any(|(_, source)| source.source() == ticket.source().preview() && source.current(Instant::now()))
                        && !store
                            .settings
                            .iter()
                            .any(|pending| pending.receipt.view == view && pending.receipt.namespace == namespace);
                    if valid {
                        store.settings.push(PendingSetting {
                            receipt,
                            _slot: request.slot,
                            began: Instant::now(),
                        });
                    } else {
                        bounded_push(&mut store.settings_done, receipt);
                    }
                }
                RequestKind::Exchange { .. } => unreachable!("handled before dequeue"),
            }
        }
    }
    pub(in crate::session_host) fn take_exchange_start(&mut self) -> Option<(u64, ViewNamespace, Exchange, HostedViewport)> {
        if self.exchange.is_some() {
            return None;
        }
        if !self
            .requests
            .front()
            .is_some_and(|request| matches!(request.kind, RequestKind::Exchange { .. }))
        {
            return None;
        }
        let control = self.reader.0.try_control().ok()?.clone();
        let mut locked = self.reader.0.try_store().ok()?;
        let store = &mut *locked;
        if store.apply_control(&control).is_err() {
            return None;
        }
        store.reconcile_frontdoor(&self.reader.0);
        let request = self.requests.pop_front()?;
        let RequestKind::Exchange {
            namespace,
            exchange,
            viewport,
        } = request.kind
        else {
            unreachable!()
        };
        let exchange = *exchange;
        let valid = !request.cancelled
            && store.generation == self.reader.0.frontdoor.view_generation()
            && store.views.iter().any(|view| {
                view.view == exchange.view() && view.namespace == namespace && view.source == exchange.target().preview() && view.visible
            })
            && control
                .sources
                .iter()
                .any(|(_, source)| source.source() == exchange.target().preview() && source.current(Instant::now()));
        let garbage = store.drain_garbage();
        drop(locked);
        drop(garbage);
        self.exchange = Some(ExchangeRecord {
            request: request.ticket,
            namespace,
            exchange,
            viewport,
            expected: None,
            rolling_back: false,
            began: Instant::now(),
            outcome: (!valid).then_some(if request.cancelled {
                ExchangeOutcome::Cancelled
            } else {
                ExchangeOutcome::Rejected
            }),
            _slot: request.slot,
        });
        valid.then_some((request.ticket, namespace, exchange, viewport))
    }
    pub(in crate::session_host) fn exchange_started(&mut self, activation: u64) {
        if let Some(record) = &mut self.exchange
            && record.outcome.is_none()
            && activation != 0
        {
            record.expected = Some(activation);
        }
    }
    pub(in crate::session_host) fn exchange_rejected(&mut self) {
        self.exchange_finish(ExchangeOutcome::Rejected);
    }
    fn exchange_finish(&mut self, outcome: ExchangeOutcome) {
        if let Some(record) = &mut self.exchange
            && record.outcome.is_none()
        {
            record.outcome = Some(outcome);
        }
    }
    pub(in crate::session_host) fn external_intent(&mut self) {
        self.exchange_finish(ExchangeOutcome::Cancelled);
        for request in &mut self.requests {
            if matches!(request.kind, RequestKind::Exchange { .. }) {
                request.cancelled = true;
            }
        }
    }
    pub(in crate::session_host) fn exchange_waiting(&self) -> bool {
        self.reader.0.frontdoor.current()
            && (self.exchange.as_ref().is_some_and(|record| record.outcome.is_none())
                || self
                    .requests
                    .iter()
                    .any(|request| !request.cancelled && matches!(request.kind, RequestKind::Exchange { .. })))
    }
    pub(in crate::session_host) fn control_activation(&mut self, activation: u64) {
        if let Some(record) = &mut self.exchange
            && record.outcome.is_none()
            && record.expected.is_some_and(|old| old != activation)
        {
            // External intents cancel BEFORE touching Coordinator. A new
            // activation emitted during this transaction is its actual rollback.
            record.rolling_back = true;
            record.expected = Some(activation);
        }
    }
    pub(in crate::session_host) fn exchange_granted(
        &mut self,
        source: &AcceptedViewSource,
        activation: u64,
        viewport: HostedViewport,
        rollback: bool,
    ) {
        let Some(record) = &self.exchange else { return };
        if record.outcome.is_some() || record.expected != Some(activation) || !source.current(Instant::now()) {
            return;
        }
        if rollback
            && record.rolling_back
            && record
                .exchange
                .previous()
                .is_some_and(|previous| previous.preview() == source.source())
        {
            self.exchange_finish(ExchangeOutcome::RolledBack);
        } else if !rollback && !record.rolling_back && record.exchange.target().preview() == source.source() && record.viewport == viewport
        {
            self.exchange_finish(ExchangeOutcome::Succeeded { activation, viewport });
        }
    }
    pub(in crate::session_host) fn exchange_lost(&mut self) {
        self.exchange_finish(ExchangeOutcome::Uncertain);
    }
    fn publish_exchange(&mut self, store: &mut Store) {
        if let Some(record) = &mut self.exchange {
            if record.outcome.is_none() && record.began.elapsed() >= Duration::from_secs(30) {
                record.outcome = Some(ExchangeOutcome::Uncertain);
            }
            if !store.views.iter().any(|view| {
                view.namespace == record.namespace
                    && view.view == record.exchange.view()
                    && view.source == record.exchange.target().preview()
            }) && record.outcome.is_none()
            {
                record.outcome = Some(ExchangeOutcome::Cancelled);
            }
        }
        if self.exchange.as_ref().is_some_and(|record| record.outcome.is_some()) {
            let record = self.exchange.take().expect("terminal record checked");
            bounded_push(
                &mut store.exchanges,
                ExchangeReceipt {
                    request: record.request,
                    namespace: record.namespace,
                    exchange: record.exchange,
                    outcome: record.outcome.expect("terminal record checked"),
                },
            );
        }
    }
}

impl BrokerProjection {
    pub(in crate::session_host) fn poll(&mut self, captures: &[AcceptedPreviewCapture]) {
        if self.channel.as_ref().is_some_and(|channel| channel.ended()) {
            self.fail_images();
        }
        let shared = self.reader.0.clone();
        let Ok(control) = shared.try_control().map(|control| control.clone()) else {
            return;
        };
        let metadata = self.channel.as_ref().and_then(|channel| channel.try_observations().ok());
        let io_service = shared.io_generation.load(Ordering::Acquire);
        if self.frames.is_empty()
            && let Some(update) = self.channel.as_ref().and_then(|channel| channel.take_update())
        {
            self.frames
                .extend(update.sources.into_iter().filter_map(|SourceUpdate { frame, .. }| frame));
        }
        if self.completions.is_empty()
            && let Some(preparer) = &self.preparer
            && let Ok(done) = preparer.take()
        {
            self.completions = done;
        }
        let mut offers = Vec::new();
        let mut capture_offer = None;
        let mut jobs = Vec::new();
        let planned_generation;
        let garbage = {
            let Ok(mut locked) = shared.try_store() else { return };
            let mut store = &mut *locked;
            let now = shared.now(); // Every model time is sampled inside lock.
            if store.apply_control(&control).is_err() {
                return;
            }
            store.reconcile_frontdoor(&shared);
            self.apply_requests(&mut store, &control);
            planned_generation = store.generation;
            self.publish_exchange(&mut store);
            for worker in self.exits.drain(..) {
                store.ledger.exited(worker);
            }
            while let Some(sent) = self.sent.pop_front() {
                match sent {
                    Sent::Demand(offer, queued) => store.ledger.admission(offer, queued),
                    Sent::Capture(claim, false) => {
                        let claim = *claim;
                        let source = claim.source();
                        if let Some(entry) = store
                            .entries
                            .iter_mut()
                            .find(|entry| entry.source == source && entry.pending == Some(claim))
                        {
                            entry.cancel();
                        }
                    }
                    Sent::Capture(_, true) => {}
                }
            }
            let metadata_current = metadata.as_ref().zip(self.channel.as_ref()).is_some_and(|(metadata, channel)| {
                store
                    .apply_metadata(&shared, &control, metadata, io_service, &channel.image_budget(), now)
                    .is_ok()
            });
            store.ledger.tick(Instant::now());
            if self.channel.is_none() {
                for entry in &mut store.entries {
                    entry.retire(&mut store.garbage);
                    store.ledger.lost(entry.source.worker());
                    entry.status = if self.failed {
                        ImageStatus::NoImage
                    } else {
                        ImageStatus::Unsupported
                    };
                }
            }
            if let Some(metadata) = metadata.as_ref().filter(|_| metadata_current) {
                for observed in metadata.sources() {
                    let token = observed.observation().token;
                    if let Some(receipt) = observed.demand_receipt().copied()
                        && store.ledger.receipt(token, receipt)
                    {
                        let _ = shared.frontdoor.capture_intent();
                        store.reconcile_frontdoor(&shared);
                        if matches!(receipt.outcome, aurora_session::preview_wire::DemandOutcome::Rejected { .. }) {
                            finish_settings(&mut store, token.source(), SettingsOutcome::Rejected);
                        }
                    }
                    if let Some(demand) = observed.observation().stopped_demand {
                        store.ledger.stopped(token, demand);
                    }
                    if let Some((claim, _reason)) = observed.observation().unavailable
                        && let Some(entry) = store
                            .entries
                            .iter_mut()
                            .find(|entry| entry.source == claim.source() && entry.pending == Some(claim))
                    {
                        entry.cancel();
                    }
                }
            }
            resolve_settings(&mut store, metadata.as_ref().filter(|_| metadata_current));
            let mut at = 0;
            while at < store.settings.len() {
                if store.settings[at].began.elapsed() >= Duration::from_secs(2) {
                    let mut pending = store.settings.swap_remove(at);
                    let source = pending.receipt.ticket.source().preview();
                    if store.ledger.pending_native(source) {
                        store.ledger.lost(source.worker());
                    }
                    pending.receipt.outcome = if store.ledger.uncertain(pending.receipt.ticket.source().preview()) {
                        SettingsOutcome::Uncertain
                    } else {
                        SettingsOutcome::Rejected
                    };
                    bounded_push(&mut store.settings_done, pending.receipt);
                } else {
                    at += 1;
                }
            }
            let uncertain: Vec<_> = store
                .entries
                .iter()
                .filter(|entry| store.ledger.uncertain(entry.source))
                .map(|entry| entry.source)
                .collect();
            for source in uncertain {
                finish_settings(&mut store, source, SettingsOutcome::Uncertain);
                if let Some(entry) = store.entries.iter_mut().find(|entry| entry.source == source) {
                    entry.retire(&mut store.garbage);
                }
            }
            if self.channel.is_none() {
                while let Some(mut pending) = store.settings.pop() {
                    pending.receipt.outcome = SettingsOutcome::Rejected;
                    bounded_push(&mut store.settings_done, pending.receipt);
                }
            }
            while let Some(frame) = self.frames.pop_front() {
                let frame_claim = frame.claim();
                if !metadata_current || !control.current() || store.generation != shared.frontdoor.view_generation() {
                    if let Some(entry) = store
                        .entries
                        .iter_mut()
                        .find(|entry| entry.source == frame_claim.source() && entry.pending == Some(frame_claim))
                    {
                        entry.cancel();
                    }
                    // Unadmitted frames can be discarded; never replay them
                    // against another model/request when the lock becomes free.
                    store.garbage.push(Garbage::handoff(frame));
                    continue;
                }
                let source = frame.token.source();
                let Some(proof) = control.images.iter().find(|proof| proof.source() == source) else {
                    store.garbage.push(Garbage::handoff(frame));
                    continue;
                };
                let Ok(guard) = ControlGuard::from_host(proof) else {
                    store.garbage.push(Garbage::handoff(frame));
                    continue;
                };
                let Some(entry) = store.entries.iter_mut().find(|entry| entry.source == source) else {
                    store.garbage.push(Garbage::handoff(frame));
                    continue;
                };
                if entry.pending != Some(frame_claim) || !entry.layout_matches(frame_claim) {
                    if entry.pending == Some(frame_claim) {
                        entry.cancel();
                    }
                    store.garbage.push(Garbage::handoff(frame));
                    continue;
                }
                let Some(model) = &mut entry.model else {
                    store.garbage.push(Garbage::handoff(frame));
                    continue;
                };
                let frame = match frame.try_admit(model, &guard, now) {
                    Ok(frame) => frame,
                    Err((_error, rejected)) => {
                        if entry.pending == Some(frame_claim) {
                            entry.cancel();
                        }
                        store.garbage.push(Garbage::handoff(rejected));
                        continue;
                    }
                };
                let raw = Arc::new(frame);
                let Ok(Some(admitted)) = model.retained(now).map(|image| image.cloned()) else {
                    store.garbage.push(Garbage::raw(raw));
                    continue;
                };
                let Ok(revision) = fresh() else {
                    store.garbage.push(Garbage::raw(raw));
                    entry.retire(&mut store.garbage);
                    continue;
                };
                entry.pending = None;
                entry.pending_layout = None;
                if let Some(old) = entry.raw.replace(raw) {
                    store.garbage.push(Garbage::raw(old));
                }
                entry.candidate.store(revision, Ordering::Release);
                entry.prepare_pending = false;
                // The actual admitted handle belongs to this exact payload,
                // not a reconstructed stamp from the receiver's metadata.
                entry.admitted_candidate = Some(admitted);
            }
            for mut completion in self.completions.drain(..) {
                if let Some(entry) = store.entries.iter_mut().find(|entry| entry.source == completion.source)
                    && entry.lifetime.same(&completion.lifetime)
                    && entry.lifetime.current()
                    && entry.candidate.load(Ordering::Acquire) == completion.revision
                {
                    entry.prepare_pending = false;
                    if let Some(payload) = completion.payload.take() {
                        if payload.admitted.current(now) {
                            if let Some(old) = entry.prepared.replace(payload) {
                                store.garbage.push(Garbage::prepared(old));
                            }
                            entry.status = ImageStatus::LastActive;
                        } else {
                            store.garbage.push(Garbage::prepared(payload));
                        }
                    }
                }
                if let Some(payload) = completion.payload {
                    store.garbage.push(Garbage::prepared(payload));
                }
            }
            if let Some(metadata) = metadata.as_ref().filter(|_| metadata_current) {
                let pending: Vec<_> = store.settings.iter().map(|pending| pending.receipt).collect();
                let desired = demand::aggregate(&store.views, &pending);
                for observed in metadata.sources() {
                    let token = observed.observation().token;
                    if control
                        .images
                        .iter()
                        .any(|proof| proof.source() == token.source() && proof.current(Instant::now()))
                    {
                        let options = desired
                            .iter()
                            .find(|(source, _)| *source == token.source())
                            .map(|(_, options)| *options);
                        if let Ok(Some(offer)) = store.ledger.offer(token, options, Instant::now()) {
                            offers.push(offer);
                        }
                    }
                }
                // Build from authenticated actual Layout only AFTER exact
                // native config ACK, and only for the real accepted Active.
                let eligible: Vec<_> = store
                    .entries
                    .iter()
                    .filter(|entry| {
                        store.visible(entry.source)
                            && entry.pending.is_none()
                            && entry.model.is_some()
                            && captures
                                .iter()
                                .any(|capture| capture.source().source() == entry.source && capture.current(Instant::now()))
                            && entry.policy.is_some_and(|policy| policy.policy().access().capture_active)
                            && entry.layout_current
                            && entry.layout.is_some()
                    })
                    .filter_map(|entry| entry.token.filter(|token| token.pixel_epoch() != 0))
                    .collect();
                if let Some(accepted) = store.ledger.due(&eligible, Instant::now()) {
                    let source = accepted.token.source();
                    if let Some(entry) = store.entries.iter_mut().find(|entry| entry.source == source)
                        && let Some(layout) = entry.layout.map(|receipt| receipt.layout())
                        && layout.demand() == accepted.demand
                        && entry.policy.is_some_and(|policy| layout.matches_policy(policy.policy()))
                        && let Ok(geometry) = layout.geometry(accepted.options.size)
                        && let Some(model) = &mut entry.model
                        && let Ok(claim) = model.request(accepted.token.connection(), accepted.demand, geometry, now)
                    {
                        entry.pending = Some(claim);
                        entry.pending_at = now;
                        entry.pending_layout = Some(layout);
                        let proof = captures.iter().find(|proof| proof.source().source() == source);
                        if let Some(guard) = proof
                            .and_then(|proof| CaptureControlGuard::from_host(proof).ok())
                            .filter(|guard| guard.current(claim, Instant::now()))
                        {
                            capture_offer = Some((accepted.token, claim, guard));
                        } else {
                            entry.cancel();
                        }
                    }
                }
            }
            for entry in &mut store.entries {
                if entry.prepare_pending || !entry.lifetime.current() {
                    continue;
                }
                let revision = entry.candidate.load(Ordering::Acquire);
                if revision == 0 || entry.prepared.as_ref().is_some_and(|prepared| prepared.revision == revision) {
                    continue;
                }
                if let (Some(raw), Some(admitted), Some(proof)) = (
                    entry.raw.as_ref(),
                    entry.admitted_candidate.as_ref(),
                    control.images.iter().find(|proof| proof.source() == entry.source),
                ) && let Ok(guard) = ControlGuard::from_host(proof)
                {
                    entry.prepare_pending = true;
                    jobs.push(prepare::Job {
                        source: entry.source,
                        revision,
                        raw: raw.clone(),
                        admitted: admitted.clone(),
                        control: guard,
                        candidate: EpochFence::generation(entry.candidate.clone(), revision),
                        lifetime: entry.lifetime.clone(),
                        clock: shared.clock,
                        budget: shared.ui_budget.clone(),
                        owner: shared.frontdoor.clone(),
                        view_generation: shared.frontdoor.view_fence(store.generation),
                    });
                }
            }
            store.drain_garbage()
        };
        drop(garbage);
        // No store/control lock is held for IO mailbox work or conversion.
        if let Some(channel) = &self.channel {
            for offer in offers {
                let current = shared.frontdoor.current()
                    && shared.frontdoor.view_generation() == planned_generation
                    && control.current()
                    && metadata.as_ref().is_some_and(|set| set.current());
                let queued = current && channel.try_command(offer.token, offer.command(), None).is_ok();
                self.sent.push_back(Sent::Demand(offer, queued));
            }
            if let Some((token, claim, guard)) = capture_offer {
                let queued = channel
                    .try_command(token, PreviewCommand::Capture(Box::new(claim)), Some(guard))
                    .is_ok();
                self.sent.push_back(Sent::Capture(Box::new(claim), queued));
            }
        }
        if !jobs.is_empty() && self.preparer.is_none() {
            self.preparer = prepare::Preparer::shared().ok();
        }
        for job in jobs {
            let rejected = if let Some(preparer) = &self.preparer {
                preparer.submit(job).err()
            } else {
                Some(job)
            };
            if let Some(job) = rejected {
                self.completions.push(prepare::Completion {
                    source: job.source,
                    revision: job.revision,
                    lifetime: job.lifetime,
                    payload: None,
                });
            }
        }
    }
}

fn finish_settings(store: &mut Store, source: PreviewSource, outcome: SettingsOutcome) {
    let mut at = 0;
    while at < store.settings.len() {
        if store.settings[at].receipt.ticket.source().preview() == source {
            let mut pending = store.settings.swap_remove(at);
            pending.receipt.outcome = outcome;
            bounded_push(&mut store.settings_done, pending.receipt);
        } else {
            at += 1;
        }
    }
}
fn resolve_settings(store: &mut Store, metadata: Option<&VersionedObservationSet>) {
    let pending: Vec<_> = store.settings.iter().map(|pending| pending.receipt).collect();
    let desired_aggregate = demand::aggregate(&store.views, &pending);
    let mut at = 0;
    while at < store.settings.len() {
        let request = store.settings[at].receipt;
        let source = request.ticket.source().preview();
        let proof = metadata
            .and_then(|set| set.sources().iter().find(|source_meta| source_meta.domain().source() == source))
            .and_then(|source_meta| {
                source_meta
                    .accepted_demand()
                    .zip(store.ledger.accepted(source_meta.observation().token))
            });
        if let Some((receipt, accepted)) = proof
            && desired_aggregate
                .iter()
                .find(|(candidate, _)| *candidate == source)
                .is_some_and(|(_, desired)| *desired == accepted.options)
            && receipt.demand == accepted.demand
            && matches!(receipt.outcome, aurora_session::preview_wire::DemandOutcome::Accepted { native_revision, .. } if native_revision == accepted.native_revision)
            && accepted.options.size.bytes() >= request.desired.size.bytes()
            && accepted.options.rate >= request.desired.rate
            && store
                .views
                .iter()
                .any(|view| view.view == request.view && view.namespace == request.namespace && view.source == source)
        {
            let mut pending = store.settings.swap_remove(at);
            pending.receipt.outcome = SettingsOutcome::Accepted {
                options: request.desired,
                effective: accepted.options,
                demand: accepted.demand,
                native_revision: accepted.native_revision,
            };
            if let Some(view) = store
                .views
                .iter_mut()
                .find(|view| view.view == request.view && view.namespace == request.namespace)
            {
                view.options = request.desired;
            }
            bounded_push(&mut store.settings_done, pending.receipt);
        } else {
            at += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_host_starting_listener_defers_launch_until_actual_ready_or_bounded_image_only_failure() {
        assert_eq!(startup_state(None, false, Duration::ZERO), StartupState::Pending);
        assert_eq!(startup_state(None, false, Duration::from_millis(1999)), StartupState::Pending);
        assert_eq!(
            startup_state(Some(32123), false, Duration::from_millis(1999)),
            StartupState::Ready(32123)
        );
        assert_eq!(
            startup_state(Some(32123), false, Duration::from_secs(3)),
            StartupState::Ready(32123)
        );
        assert_eq!(startup_state(None, false, Duration::from_secs(2)), StartupState::Failed);
        assert_eq!(startup_state(Some(32123), true, Duration::ZERO), StartupState::Failed);
        assert_eq!(startup_state(None, true, Duration::ZERO), StartupState::Failed);
    }
}
