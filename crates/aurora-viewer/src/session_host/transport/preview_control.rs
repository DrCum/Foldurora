//! Broker-owned preview authority, independent of Chat and foreground Review.
//! DTOs, stable slots and HostSnapshot labels cannot construct these tokens.
use super::*;
use crate::session_host::preview_io::EpochFence;
use aurora_session::preview_image::PreviewSource;
use std::sync::atomic::{AtomicU64, AtomicUsize};

static NEXT_REGISTRATION: AtomicU64 = AtomicU64::new(1);

/// Shared front-door counters are advanced before even trying the FIFO lock.
/// A failed ordinary intent still retires its old capture publication; it does
/// not alter the ordinary intent's result or admitted parked history.
pub(crate) struct Frontdoor {
    capture: Arc<AtomicU64>,
    views: Arc<AtomicU64>,
    failed_views: AtomicU64,
    retired: AtomicBool,
    attempts: Arc<AtomicUsize>,
    queued: Arc<AtomicUsize>,
}
impl Default for Frontdoor {
    fn default() -> Self {
        Self {
            capture: Arc::new(AtomicU64::new(1)),
            views: Arc::new(AtomicU64::new(1)),
            failed_views: AtomicU64::new(0),
            retired: AtomicBool::new(false),
            attempts: Arc::new(AtomicUsize::new(0)),
            queued: Arc::new(AtomicUsize::new(0)),
        }
    }
}
impl Frontdoor {
    fn advance(&self, value: &AtomicU64) -> Result<u64, HostError> {
        if self.retired.load(Ordering::Acquire) {
            return Err(HostError::Unavailable);
        }
        value
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map(|old| old + 1)
            .map_err(|_| {
                self.retired.store(true, Ordering::Release);
                self.capture.store(0, Ordering::Release);
                self.views.store(0, Ordering::Release);
                HostError::Unavailable
            })
    }
    pub(crate) fn capture_intent(&self) -> Result<u64, HostError> {
        self.advance(&self.capture)
    }
    pub(super) fn begin_capture(&self) -> Result<CaptureAttempt, HostError> {
        self.attempts
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| {
                self.retire();
                HostError::Unavailable
            })?;
        let attempt = CaptureAttempt(self.attempts.clone());
        self.capture_intent()?;
        Ok(attempt)
    }
    pub(super) fn queue_slot(&self) -> Result<CaptureQueueSlot, HostError> {
        self.queued
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| {
                self.retire();
                HostError::Unavailable
            })?;
        Ok(CaptureQueueSlot(self.queued.clone()))
    }
    pub(crate) fn capture_ready(&self) -> bool {
        self.current() && self.attempts.load(Ordering::Acquire) == 0 && self.queued.load(Ordering::Acquire) == 0
    }
    pub(crate) fn views_intent(&self) -> Result<u64, HostError> {
        // View retirement closes capture even if the replacement is refused.
        let _ = self.capture_intent();
        self.advance(&self.views)
    }
    pub(crate) fn capture_generation(&self) -> u64 {
        self.capture.load(Ordering::Acquire)
    }
    pub(crate) fn view_generation(&self) -> u64 {
        self.views.load(Ordering::Acquire)
    }
    pub(crate) fn view_failed(&self, generation: u64) {
        self.failed_views.fetch_max(generation, Ordering::AcqRel);
    }
    pub(crate) fn failed_views(&self) -> u64 {
        self.failed_views.load(Ordering::Acquire)
    }
    pub(crate) fn current(&self) -> bool {
        !self.retired.load(Ordering::Acquire)
    }
    pub(crate) fn capture_fence(&self, generation: u64) -> EpochFence {
        EpochFence::generation(self.capture.clone(), generation)
    }
    pub(crate) fn view_fence(&self, generation: u64) -> EpochFence {
        EpochFence::generation(self.views.clone(), generation)
    }
    pub(crate) fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        self.capture.store(0, Ordering::Release);
        self.views.store(0, Ordering::Release);
    }
}
pub(super) struct CaptureAttempt(Arc<AtomicUsize>);
impl Drop for CaptureAttempt {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
pub(super) struct CaptureQueueSlot(Arc<AtomicUsize>);
impl Drop for CaptureQueueSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone)]
pub(crate) struct AcceptedPreviewSource {
    view: AcceptedViewSource,
    witness: EpochFence,
}
impl AcceptedPreviewSource {
    pub(crate) fn source(&self) -> PreviewSource {
        self.view.source()
    }
    pub(crate) fn observed_at(&self) -> Instant {
        self.view.observed_at()
    }
    pub(crate) fn expires_at(&self) -> Instant {
        self.view.expires_at()
    }
    pub(crate) fn registration(&self) -> u64 {
        self.view.registration
    }
    pub(crate) fn witness(&self) -> EpochFence {
        self.witness.clone()
    }
    pub(crate) fn current(&self, now: Instant) -> bool {
        self.view.current(now) && self.witness.current()
    }
}

/// Canonical view identity is provable for old/None workers too. This token
/// cannot construct an IO permit: only a real optional bootstrap can promote
/// the private representation to AcceptedPreviewSource in the Broker.
#[derive(Clone)]
pub(crate) struct AcceptedViewSource {
    source: PreviewSource,
    observed_at: Instant,
    expires_at: Instant,
    registration: u64,
    witness: EpochFence,
}
impl AcceptedViewSource {
    pub(crate) fn source(&self) -> PreviewSource {
        self.source
    }
    pub(crate) fn observed_at(&self) -> Instant {
        self.observed_at
    }
    pub(crate) fn expires_at(&self) -> Instant {
        self.expires_at
    }
    pub(crate) fn witness(&self) -> EpochFence {
        self.witness.clone()
    }
    pub(crate) fn current(&self, now: Instant) -> bool {
        self.registration != 0 && self.observed_at <= now && now < self.expires_at && self.witness.current()
    }
}

#[derive(Clone)]
pub(crate) struct AcceptedPreviewCapture {
    source: AcceptedPreviewSource,
    activation: u64,
    viewport: HostedViewport,
    witness: EpochFence,
    frontdoor: Arc<Frontdoor>,
}
impl AcceptedPreviewCapture {
    pub(crate) fn source(&self) -> &AcceptedPreviewSource {
        &self.source
    }
    pub(crate) fn activation(&self) -> u64 {
        self.activation
    }
    pub(crate) fn viewport(&self) -> HostedViewport {
        self.viewport
    }
    pub(crate) fn witness(&self) -> EpochFence {
        self.witness.clone()
    }
    pub(crate) fn current(&self, now: Instant) -> bool {
        self.frontdoor.current() && self.source.current(now) && self.witness.current() && self.activation != 0
    }
}

/// One launch/source lifetime, including its permanent old witness revocation.
/// A numeric same-source renewal cannot revive a retired witness.
pub(super) struct ChildPreview {
    registration: u64,
    source: Option<PreviewSource>,
    witness: EpochFence,
    image_witness: EpochFence,
}
/// The actual owned Child receipt joined to its accepted Coordinator entry.
/// Kept private so a UI DTO cannot be promoted to a source proof.
struct ChildReceipt<'a> {
    config: &'a WorkerConfig,
    binding: Binding,
    received: u64,
    last_seen: Option<Instant>,
    entry: Option<&'a aurora_session::RosterEntry>,
    owned: bool,
}
impl ChildPreview {
    pub(super) fn new() -> Self {
        Self {
            registration: NEXT_REGISTRATION
                .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
                .unwrap_or(0),
            source: None,
            witness: EpochFence::new(),
            image_witness: EpochFence::new(),
        }
    }
    pub(super) fn retire(&mut self) {
        self.witness.revoke();
        self.image_witness.revoke();
        self.source = None;
    }
    /// Called only after the real Coordinator has accepted each envelope. The
    /// same helper is exercised with actual Coordinator receipts in tests.
    fn observe(&mut self, receipt: ChildReceipt<'_>, now: Instant) -> Option<AcceptedPreviewSource> {
        let config = receipt.config;
        let view = self.observe_view(receipt, now)?;
        if config.preview.as_ref().is_none_or(|bootstrap| bootstrap.port == 0) {
            self.image_witness.revoke();
            return None;
        }
        if !self.image_witness.current() {
            self.image_witness = EpochFence::new();
        }
        Some(AcceptedPreviewSource {
            view,
            witness: self.image_witness.clone(),
        })
    }
    fn observe_view(&mut self, receipt: ChildReceipt<'_>, now: Instant) -> Option<AcceptedViewSource> {
        let ChildReceipt {
            config,
            binding,
            received,
            last_seen,
            entry,
            owned,
        } = receipt;
        let accepted = (|| {
            let entry = entry?;
            let session = binding.session?;
            let observed_at = last_seen?;
            let expires_at = observed_at.checked_add(REACHABLE.min(Duration::from_millis(aurora_session::LEASE_MILLIS)))?;
            if !owned
                || self.registration == 0
                || received == 0
                || !entry.live
                || entry.status.phase != Phase::Online
                || entry.binding != binding
                || binding.worker != config.identity
                || session.session != config.session
                || session.generation < config.generation
                || config.expected_account.is_some_and(|account| account != session.account)
                || config.expected_grid.is_some_and(|grid| grid != session.grid)
                || now < observed_at
                || now >= expires_at
            {
                return None;
            }
            let source = PreviewSource::new(config.identity, session, config.lease_epoch).ok()?;
            Some((source, observed_at, expires_at))
        })();
        let Some((source, observed_at, expires_at)) = accepted else {
            self.retire();
            return None;
        };
        if self.source != Some(source) || !self.witness.current() {
            self.retire();
            self.witness = EpochFence::new();
            self.source = Some(source);
        }
        Some(AcceptedViewSource {
            source,
            observed_at,
            expires_at,
            registration: self.registration,
            witness: self.witness.clone(),
        })
    }
}
impl Drop for ChildPreview {
    fn drop(&mut self) {
        self.retire();
    }
}

#[cfg(any(windows, test))]
impl Broker {
    pub(super) fn preview_view_sources(&mut self, now: Instant) -> Vec<AcceptedViewSource> {
        self.children
            .iter_mut()
            .filter_map(|child| {
                let entry = self
                    .coordinator
                    .workers()
                    .find(|entry| entry.binding.worker == child.config.identity);
                child.preview.observe_view(
                    ChildReceipt {
                        config: &child.config,
                        binding: child.binding,
                        received: child.received,
                        last_seen: child.last_seen,
                        entry,
                        owned: !child.detached && !child.exited,
                    },
                    now,
                )
            })
            .collect()
    }
    pub(super) fn preview_sources(&mut self, now: Instant) -> Vec<AcceptedPreviewSource> {
        self.children
            .iter_mut()
            .filter_map(|child| {
                let entry = self
                    .coordinator
                    .workers()
                    .find(|entry| entry.binding.worker == child.config.identity);
                child.preview.observe(
                    ChildReceipt {
                        config: &child.config,
                        binding: child.binding,
                        received: child.received,
                        last_seen: child.last_seen,
                        entry,
                        owned: !child.detached && !child.exited,
                    },
                    now,
                )
            })
            .collect()
    }

    pub(super) fn preview_capture(&self, source: &AcceptedPreviewSource) -> Option<AcceptedPreviewCapture> {
        let binding = Binding {
            worker: source.source().worker(),
            session: Some(source.source().session()),
        };
        let child = self.children.iter().find(|child| child.binding == binding)?;
        if !self.preview_projection.capture_admitted(source.source())
            || self.preview_projection.exchange_waiting()
            || !self.preview_frontdoor.capture_ready()
            || !self.preview_frontdoor.current()
            || !self.opens.is_empty()
            || self.park_requested
            || self.viewport_dirty
            || self.reviews.target().is_some()
            || child.detached
            || child.exited
        {
            return None;
        }
        capture_from_accepted(
            source,
            self.preview_frontdoor.clone(),
            CaptureFacts {
                coordinator: &self.coordinator,
                active: self.active_viewport,
                viewport: self.viewport,
                prepared: child.prepared,
                granted: child.granted,
                status: child.status.as_ref(),
            },
        )
    }
}

#[cfg(any(windows, test))]
struct CaptureFacts<'a> {
    coordinator: &'a Coordinator,
    active: Option<ActiveViewport>,
    viewport: Option<HostedViewport>,
    prepared: Option<(Binding, u64, Option<HostedViewport>)>,
    granted: Option<(Binding, u64)>,
    status: Option<&'a WorkerStatus>,
}
/// Private gate over the owner's retained accepted receipts. The production
/// caller above supplies real Child fields; tests drive those same protocol
/// receipts through Coordinator/WorkerGuard, never a public grant constructor.
#[cfg(any(windows, test))]
fn capture_from_accepted(
    source: &AcceptedPreviewSource,
    frontdoor: Arc<Frontdoor>,
    facts: CaptureFacts<'_>,
) -> Option<AcceptedPreviewCapture> {
    let generation = frontdoor.capture_generation();
    let active = facts.active?;
    let binding = Binding {
        worker: source.source().worker(),
        session: Some(source.source().session()),
    };
    if !source.current(Instant::now())
        || !frontdoor.capture_ready()
        || facts.coordinator.busy()
        || facts.coordinator.active() != Some(active.worker)
        || active.worker != binding.worker.worker
        || active.binding != binding
        || facts.viewport != Some(active.viewport)
        || active.viewport.validate().is_err()
        || facts.prepared != Some((binding, active.activation, Some(active.viewport)))
        || facts.granted != Some((binding, active.activation))
        || facts
            .status
            .is_none_or(|status| status.phase != Phase::Online || status.blocked || status.native_window == 0)
    {
        return None;
    }
    let result = AcceptedPreviewCapture {
        source: source.clone(),
        activation: active.activation,
        viewport: active.viewport,
        witness: frontdoor.capture_fence(generation),
        frontdoor,
    };
    result.current(Instant::now()).then_some(result)
}

#[cfg(any(windows, test))]
impl HostController {
    pub(in crate::session_host) fn preview_reader(&self) -> PreviewReader {
        self.preview_reader.clone()
    }
    pub(in crate::session_host) fn set_preview_views(&self, views: Vec<ViewDescriptor>) -> Result<ViewSetRequest, PreviewError> {
        let _attempt = self.preview_frontdoor.begin_capture().map_err(PreviewError::from)?;
        let generation = self.preview_frontdoor.views_intent().map_err(PreviewError::from)?;
        let result = (|| {
            if views.len() > MAX_WORKERS
                || views.iter().enumerate().any(|(at, view)| {
                    !view.valid()
                        || views[..at]
                            .iter()
                            .any(|old| old.view == view.view || old.namespace == view.namespace)
                })
            {
                return Err(PreviewError::Invalid);
            }
            let slot = self.preview_reader.reserve()?;
            let request = self
                .send(HostIntent::Preview(RequestKind::Views { generation, views }, slot))
                .map_err(PreviewError::from)?;
            Ok(ViewSetRequest::admitted(generation, request))
        })();
        if result.is_err() {
            self.preview_frontdoor.view_failed(generation);
        }
        result
    }
    pub(in crate::session_host) fn set_preview_options(
        &self,
        view: crate::session_host::monitor_model::View,
        namespace: ViewNamespace,
        ticket: crate::session_host::monitor_model::Ticket,
        base: crate::session_host::monitor_ui::State,
        desired: crate::session_host::monitor_model::Options,
    ) -> Result<u64, PreviewError> {
        let _attempt = self.preview_frontdoor.begin_capture().map_err(PreviewError::from)?;
        let read = self.preview_reader.try_read(view, namespace)?;
        if !read.admits_settings(ticket, base) || read.source() != ticket.source().preview() || ticket.view() != view {
            return Err(PreviewError::Stale);
        }
        let slot = self.preview_reader.reserve()?;
        self.send(HostIntent::Preview(
            RequestKind::Settings {
                view,
                namespace,
                ticket: Box::new(ticket),
                base,
                desired,
            },
            slot,
        ))
        .map_err(PreviewError::from)
    }
    pub(in crate::session_host) fn exchange_preview(
        &self,
        namespace: ViewNamespace,
        exchange: crate::session_host::monitor_model::Exchange,
        viewport: HostedViewport,
    ) -> Result<u64, PreviewError> {
        let _attempt = self.preview_frontdoor.begin_capture().map_err(PreviewError::from)?;
        if !cfg!(windows) || viewport.validate().is_err() {
            return Err(PreviewError::Unsupported);
        }
        let read = self.preview_reader.try_read(exchange.view(), namespace)?;
        if !read.commands_current() || read.source() != exchange.target().preview() {
            return Err(PreviewError::Stale);
        }
        let slot = self.preview_reader.reserve()?;
        self.send(HostIntent::Preview(
            RequestKind::Exchange {
                namespace,
                exchange: Box::new(exchange),
                viewport,
            },
            slot,
        ))
        .map_err(PreviewError::from)
    }
}

#[cfg(any(windows, test))]
impl Broker {
    pub(super) fn poll_previews(&mut self, control_now: u64) {
        let now = Instant::now();
        let view_sources = self.preview_view_sources(now);
        let sources: Vec<_> = view_sources
            .into_iter()
            .filter_map(|source| {
                let slot = self
                    .children
                    .iter()
                    .find(|child| child.config.identity == source.source().worker())?
                    .config
                    .slot?;
                Some((
                    preview_projection::ViewSource {
                        slot,
                        source: source.source(),
                        available: true,
                    },
                    source,
                ))
            })
            .collect();
        let images = self.preview_sources(now);
        let active = self
            .active_viewport
            .filter(|active| {
                !self.coordinator.busy()
                    && self.coordinator.active() == Some(active.worker)
                    && self.viewport == Some(active.viewport)
                    && self
                        .children
                        .iter()
                        .any(|child| child.binding == active.binding && child.granted == Some((active.binding, active.activation)))
            })
            .and_then(|active| {
                sources
                    .iter()
                    .find(|(_, source)| {
                        source.source().worker() == active.binding.worker
                            && Some(source.source().session()) == active.binding.session
                            && source.current(now)
                    })
                    .map(|(_, source)| source.source())
            });
        let permits: Vec<_> = images
            .iter()
            .filter_map(|source| {
                let bootstrap = self
                    .children
                    .iter()
                    .find(|child| child.config.identity == source.source().worker())?
                    .config
                    .preview
                    .as_ref()?;
                crate::session_host::preview_io::HostPermit::from_host(source, bootstrap.key.clone()).ok()
            })
            .collect();
        self.preview_projection.publish_permits(&permits);
        let captures: Vec<_> = images.iter().filter_map(|source| self.preview_capture(source)).collect();
        let _ = self.preview_projection.publish_control(sources, images, active);
        self.preview_projection.poll(&captures);
        if let Some((_, _, exchange, viewport)) = self.preview_projection.take_exchange_start() {
            self.begin_preview_exchange(exchange, viewport, control_now);
        }
    }
    fn begin_preview_exchange(&mut self, exchange: crate::session_host::monitor_model::Exchange, viewport: HostedViewport, now: u64) {
        let sources = self.preview_view_sources(Instant::now());
        let target = exchange.target().preview();
        let previous = exchange.previous().map(|source| source.preview());
        let current = self
            .active_viewport
            .filter(|active| {
                !self.coordinator.busy() && self.coordinator.active() == Some(active.worker) && self.viewport == Some(active.viewport)
            })
            .and_then(|active| {
                sources
                    .iter()
                    .find(|source| {
                        source.source().worker() == active.binding.worker && Some(source.source().session()) == active.binding.session
                    })
                    .map(|source| source.source())
            });
        let allowed = previous == current
            && !self.coordinator.busy()
            && self.opens.is_empty()
            && !self.park_requested
            && !self.viewport_dirty
            && !self.reviews.waiting()
            && sources
                .iter()
                .any(|source| source.source() == target && source.current(Instant::now()));
        if !allowed {
            self.preview_projection.exchange_rejected();
            return;
        }
        let effects = match self.coordinator.activate(target.worker().worker, Some(viewport), now) {
            Ok(effects) => effects,
            Err(_) => {
                self.preview_projection.exchange_rejected();
                return;
            }
        };
        if effects.is_empty() {
            // A shortcut still needs the exact retained Granted/binding/viewport.
            if let Some(active) = self
                .accepted_viewport(target.worker().worker)
                .filter(|active| active.viewport == viewport)
                && let Some(source) = sources.iter().find(|source| source.source() == target)
            {
                self.preview_projection.exchange_started(active.activation);
                self.preview_projection
                    .exchange_granted(source, active.activation, active.viewport, false);
            } else {
                self.preview_projection.exchange_rejected();
            }
        } else if let Some(activation) = review_activation(&effects) {
            self.preview_projection.exchange_started(activation);
        } else {
            self.preview_projection.exchange_rejected();
        }
        self.viewport = Some(viewport);
        self.viewport_dirty = false;
        if let Err(error) = self.effects(effects) {
            self.preview_projection.exchange_lost();
            self.fail_closed(&error);
        }
    }
}

#[cfg(test)]
pub(in crate::session_host) mod fixtures;
#[cfg(test)]
mod tests;
