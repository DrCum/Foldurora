//! Controller-side preview admission and latest image cache. Every source is
//! supplied by accepted control state; socket authentication is necessary but
//! never sufficient to retain an image or renew a GPU budget.

use super::preview_barrier::{Participant, Request as BarrierRequest};
use super::preview_io::{PreviewCache, PreviewConfig, PreviewSnapshot};
use super::preview_protocol::{self as wire, BudgetPermit, CohortReport, GpuSample, Header, Peer as NativePeer, PreviewSource, Probe};
use super::preview_transport::{HostIo, Kind};
use super::transport::HostError;
use aurora_session::WorkerIdentity;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

const PROBE_PERIOD: Duration = Duration::from_millis(250);
const MAX_CHALLENGES: usize = 8;

#[derive(Default)]
struct Evidence {
    issued: VecDeque<(u64, Instant)>,
    sequence: u64,
    metadata: Option<(NativePeer, Instant)>,
    sample: Option<(GpuSample, Instant)>,
}

impl Evidence {
    fn clear(&mut self) {
        self.issued.clear();
        self.metadata = None;
        self.sample = None;
        // Challenges never reset when a process logs out or reconnects.
    }
    fn next(&self) -> Option<u64> {
        self.sequence.checked_add(1)
    }
    fn issued(&mut self, challenge: u64, now: Instant) {
        self.sequence = challenge;
        while self.issued.len() >= MAX_CHALLENGES {
            self.issued.pop_front();
        }
        self.issued.push_back((challenge, now));
    }
    fn issue_time(&self, challenge: u64, now: Instant) -> Option<Instant> {
        self.issued
            .iter()
            .find(|(id, at)| *id == challenge && current(*at, now))
            .map(|(_, at)| *at)
    }
    fn metadata(&mut self, report: CohortReport, source: PreviewSource, pid: u32, now: Instant) -> bool {
        // Native metadata is verified against the real child and accepted
        // source BEFORE any challenge/frame high-water mark is consumed.
        if Header::Peer(report).validate().is_err() || report.peer.cohort.source != source || report.peer.cohort.pid != pid {
            return false;
        }
        let Some(issued) = self.issue_time(report.challenge, now) else {
            return false;
        };
        if self
            .metadata
            .is_some_and(|(old, _)| old.cohort.device_epoch > report.peer.cohort.device_epoch)
        {
            return false;
        }
        if self.metadata.is_some_and(|(old, at)| {
            issued < at || (old.cohort.device_epoch == report.peer.cohort.device_epoch && old.luid != report.peer.luid)
        }) {
            return false;
        }
        if self.metadata.is_some_and(|(old, _)| old != report.peer) {
            self.sample = None;
        }
        self.metadata = Some((report.peer, issued));
        true
    }
    fn sample(&mut self, sample: GpuSample, source: PreviewSource, pid: u32, now: Instant) -> bool {
        if sample.validate().is_err() || sample.peer.cohort.source != source || sample.peer.cohort.pid != pid {
            return false;
        }
        let Some(issued) = self.issue_time(sample.challenge, now) else {
            return false;
        };
        if self.metadata.is_none_or(|(peer, at)| peer != sample.peer || !current(at, now)) {
            return false;
        }
        if self.sample.is_some_and(|(old, _)| {
            sample.peer.cohort.device_epoch < old.peer.cohort.device_epoch
                || (old.peer == sample.peer
                    && (sample.surface_epoch < old.surface_epoch
                        || (sample.surface_epoch == old.surface_epoch && sample.frame <= old.frame)))
        }) {
            return false;
        }
        self.sample = Some((sample, issued));
        true
    }
    fn peer(&self, now: Instant) -> Option<NativePeer> {
        self.metadata.filter(|(_, at)| current(*at, now)).map(|(peer, _)| peer)
    }
    fn peer_evidence(&self, now: Instant) -> Option<(NativePeer, Instant)> {
        self.metadata.filter(|(_, at)| current(*at, now))
    }
    fn known_peer(&self, source: PreviewSource) -> Option<NativePeer> {
        self.metadata.map(|(peer, _)| peer).filter(|peer| peer.cohort.source == source)
    }
    fn completed(&self, now: Instant) -> Option<(GpuSample, Instant)> {
        self.sample
            .filter(|(sample, at)| current(*at, now) && self.peer(now) == Some(sample.peer))
    }
}

fn current(issued: Instant, now: Instant) -> bool {
    now.checked_duration_since(issued).is_some_and(|age| age <= wire::SAMPLE_FRESH)
}

/// Round upward: a 500ms plus one-nanosecond sample is already stale and must
/// never become an apparently valid 500ms budget after integer conversion.
fn age_millis(issued: Instant, now: Instant) -> Option<u16> {
    let age = now.checked_duration_since(issued)?;
    if age > wire::SAMPLE_FRESH {
        return None;
    }
    u16::try_from(age.as_nanos().div_ceil(1_000_000)).ok()
}

pub(crate) struct Peer {
    identity: WorkerIdentity,
    lease_epoch: u64,
    pid: u32,
    source: Option<PreviewSource>,
    live: bool,
    parked: bool,
    control: Option<HostIo>,
    image: Option<HostIo>,
    config: Option<(u64, PreviewConfig)>,
    cache: Option<PreviewCache>,
    evidence: Evidence,
    next_probe: Instant,
    next_subscription: Instant,
    budget_request: Option<Probe>,
    snapshot: Option<Arc<PreviewSnapshot>>,
    ever_authorized: bool,
    suspending: Option<wire::Suspend>,
    suspended: Option<wire::Suspended>,
    resume_after: Option<u64>,
}

impl Peer {
    pub fn new(identity: WorkerIdentity, lease_epoch: u64, pid: u32) -> Self {
        Self {
            identity,
            lease_epoch,
            pid,
            source: None,
            live: false,
            parked: false,
            control: None,
            image: None,
            config: None,
            cache: None,
            evidence: Evidence::default(),
            next_probe: Instant::now(),
            next_subscription: Instant::now(),
            budget_request: None,
            snapshot: None,
            ever_authorized: false,
            suspending: None,
            suspended: None,
            resume_after: None,
        }
    }
    pub fn set_control(&mut self, source: Option<PreviewSource>, live: bool, parked: bool, now: Instant) {
        let source =
            source.filter(|source| source.validate().is_ok() && source.worker == self.identity && source.lease_epoch == self.lease_epoch);
        if source != self.source || !live {
            self.control = None;
            self.image = None;
            self.cache = None;
            self.config = None;
            self.evidence.clear();
            self.budget_request = None;
            self.snapshot = None;
            self.suspending = None;
            self.suspended = None;
            self.resume_after = None;
        }
        if !parked && self.parked {
            self.image = None;
            if let Some(cache) = &mut self.cache {
                cache.purge();
            }
        }
        self.source = source;
        self.live = live && source.is_some();
        self.parked = self.live && parked;
        self.publish(now);
    }
    pub fn accepting(&self, kind: Kind) -> bool {
        self.live
            && self.source.is_some()
            && match kind {
                Kind::Control => self.control.is_none(),
                Kind::Image => {
                    self.suspending.is_none()
                        && self.control.is_some()
                        && self.image.is_none()
                        && self.parked
                        && self.config.is_some_and(|(_, config)| config.visible)
                }
            }
    }
    pub fn attach(&mut self, io: HostIo, kind: Kind) -> Result<(), HostError> {
        if io.identity() != self.identity || io.lease_epoch() != self.lease_epoch || !self.accepting(kind) {
            return Err(HostError::Unavailable);
        }
        match kind {
            Kind::Control => {
                self.control = Some(io);
                self.next_probe = Instant::now();
                self.next_subscription = Instant::now();
            }
            Kind::Image => self.image = Some(io),
        }
        Ok(())
    }
    pub fn configure(
        &mut self,
        source: PreviewSource,
        config: Option<PreviewConfig>,
        revision: u64,
        now: Instant,
    ) -> Result<(), HostError> {
        if !self.live || self.source != Some(source) || (config.is_some() && self.control.is_none()) || revision == 0 {
            return Err(HostError::Unavailable);
        }
        self.image = None;
        self.cache = None;
        self.snapshot = None;
        self.budget_request = None;
        self.config = config.map(|config| (revision, config));
        if let Some((revision, config)) = self.config {
            self.cache = Some(PreviewCache::new(source, revision, config.size, config.rate));
        } else {
            // Retire worker GPU/readback work even if the close occurs before
            // a later host loop can publish the absence of this source.
            let challenge = self.evidence.next().ok_or(HostError::Unavailable)?;
            self.evidence.issued(challenge, now);
            let disabled = Header::Subscribe(wire::Subscription {
                source,
                revision,
                size: aurora_session::PreviewSize::Small,
                rate: aurora_session::PreviewRate::Half,
                visible: false,
                challenge,
                budget: None,
                resume_after: self.resume_after,
            });
            if self.control.as_mut().is_some_and(|control| control.queue(&disabled).is_err()) {
                self.disconnect();
            }
        }
        self.next_subscription = now;
        self.publish(now);
        Ok(())
    }
    /// Removal cannot depend on opening eligibility or fresh metadata. It
    /// stops this exact source's demand immediately but never frees uncertain
    /// policy reservations or supplies a GPU drain acknowledgment.
    pub fn retire_request(&mut self, source: PreviewSource, revision: u64, now: Instant) {
        if self.source != Some(source) {
            return;
        }
        let _ = self.configure(source, None, revision, now);
        self.image = None;
        self.config = None;
        self.cache = None;
        self.snapshot = None;
        self.budget_request = None;
    }

    fn disconnect(&mut self) {
        self.control = None;
        self.image = None;
        self.evidence.clear();
        self.budget_request = None;
        if let Some(cache) = &mut self.cache {
            cache.purge();
        }
    }
    pub fn poll_control(&mut self, now: Instant) {
        if !self.live {
            return;
        }
        let Some(source) = self.source else {
            return;
        };
        let Some(control) = &mut self.control else {
            return;
        };
        if let Some(request) = self.suspending
            && self.suspended.is_none_or(|proof| proof.request != request)
        {
            let _ = control.queue(&Header::Suspend(request));
        }
        if now >= self.next_probe
            && let Some(challenge) = self.evidence.next()
            && control.queue(&Header::Probe(Probe { source, challenge })).is_ok()
        {
            self.evidence.issued(challenge, now);
            self.next_probe = now + PROBE_PERIOD;
        }
        let packets = match control.poll() {
            Ok(packets) => packets,
            Err(_) => {
                self.disconnect();
                self.publish(now);
                return;
            }
        };
        let received = Instant::now();
        for packet in packets {
            if packet.header.source() != source || !self.live {
                continue;
            }
            match packet.header {
                Header::Peer(report) => {
                    self.evidence.metadata(report, source, self.pid, received);
                }
                Header::Sample(sample) => {
                    self.evidence.sample(sample, source, self.pid, received);
                }
                Header::Probe(probe) => self.budget_request = Some(probe),
                Header::State(state) => {
                    if let Some(cache) = &mut self.cache {
                        if self.suspending.is_some() || (!self.parked && state.availability == wire::Availability::Live) {
                            cache.purge();
                        } else {
                            cache.receive_state(state, received);
                        }
                    }
                }
                Header::Suspended(proof) => {
                    if self.suspending == Some(proof.request)
                        && proof.validate().is_ok()
                        && self.evidence.known_peer(source).is_some_and(|peer| {
                            peer.cohort.source == proof.request.source && peer.cohort.device_epoch == proof.request.device_epoch
                        })
                    {
                        self.suspended = Some(proof);
                    }
                }
                _ => {}
            }
        }
        self.publish(received);
    }
    pub fn metadata(&self, now: Instant) -> Option<NativePeer> {
        self.live.then(|| self.evidence.peer(now)).flatten()
    }
    pub fn metadata_evidence(&self, now: Instant) -> Option<(NativePeer, Instant)> {
        self.live.then(|| self.evidence.peer_evidence(now)).flatten()
    }
    pub fn sample(&self, now: Instant) -> Option<(GpuSample, Instant)> {
        self.live.then(|| self.evidence.completed(now)).flatten()
    }
    pub fn take_budget_request(&mut self) -> Option<Probe> {
        self.budget_request.take()
    }
    pub fn has_authority(&self) -> bool {
        self.ever_authorized
    }
    pub fn participant(&self, _now: Instant) -> Result<Option<Participant>, HostError> {
        if !self.ever_authorized {
            return Ok(None);
        }
        let source = self
            .source
            .filter(|_| self.live && self.control.is_some())
            .ok_or(HostError::Unavailable)?;
        let peer = self.evidence.known_peer(source).ok_or(HostError::Unavailable)?;
        Ok(Some(Participant {
            source: peer.cohort.source,
            device_epoch: peer.cohort.device_epoch,
        }))
    }
    pub fn suspend(&mut self, request: BarrierRequest, now: Instant) -> Result<(), HostError> {
        if self.participant(now)? != Some(request.participant) {
            return Err(HostError::Unavailable);
        }
        self.suspending = Some(wire::Suspend {
            source: request.participant.source,
            device_epoch: request.participant.device_epoch,
            challenge: request.challenge,
        });
        self.suspended = None;
        self.resume_after = None;
        self.image = None;
        if let Some(cache) = &mut self.cache {
            cache.purge();
        }
        self.publish(now);
        Ok(())
    }
    pub fn suspension(&self) -> Option<wire::Suspended> {
        self.suspended
    }
    pub fn resume_proven(&mut self) {
        if let Some(proof) = self.suspended.filter(|proof| Some(proof.request) == self.suspending) {
            self.resume_after = Some(proof.request.challenge);
            self.suspending = None;
            self.next_subscription = Instant::now();
        }
    }
    pub fn subscription(&mut self, request: Option<Probe>, budget: Option<BudgetPermit>, now: Instant) {
        let Some((revision, config)) = self.config else {
            return;
        };
        let Some(source) = self.source.filter(|_| self.live) else {
            return;
        };
        if self.suspending.is_some() {
            return;
        }
        if request.is_none() && now < self.next_subscription {
            return;
        }
        if request.is_some_and(|request| request.source != source) {
            return;
        }
        let (Some(cache), Some(control)) = (&mut self.cache, &mut self.control) else {
            return;
        };
        let Some(challenge) = cache.challenge(now) else {
            self.disconnect();
            return;
        };
        let header = Header::Subscribe(wire::Subscription {
            source,
            revision,
            size: config.size,
            rate: config.rate,
            visible: config.visible,
            challenge,
            budget: budget.filter(|budget| request.is_some_and(|request| request.challenge == budget.request)),
            resume_after: self.resume_after,
        });
        if control.queue(&header).is_ok() {
            if matches!(&header,Header::Subscribe(subscription) if subscription.budget.is_some()) {
                self.ever_authorized = true;
            }
            self.next_subscription = now + PROBE_PERIOD;
        }
    }
    pub fn poll_image(&mut self, now: Instant) {
        let Some(source) = self.source.filter(|_| self.live && self.parked && self.suspending.is_none()) else {
            self.image = None;
            self.publish(now);
            return;
        };
        let Some(image) = &mut self.image else {
            self.publish(now);
            return;
        };
        match image.poll() {
            Ok(packets) => {
                for packet in packets {
                    if let Header::Image(header) = packet.header
                        && header.source == source
                        && let Some(cache) = &mut self.cache
                    {
                        cache.receive_image(header, packet.rgba, Instant::now());
                    }
                }
            }
            Err(_) => {
                self.image = None;
                if let Some(cache) = &mut self.cache {
                    cache.purge();
                }
            }
        }
        self.publish(now);
    }
    pub fn snapshot(&self) -> Option<Arc<PreviewSnapshot>> {
        self.snapshot.clone()
    }
    fn publish(&mut self, now: Instant) {
        let (Some(source), Some((revision, config)), Some(cache)) = (self.source, self.config, &mut self.cache) else {
            self.snapshot = None;
            return;
        };
        let image = cache.image(now);
        let state = cache.state(now);
        let valid_until = cache.valid_until();
        let same = self.snapshot.as_ref().is_some_and(|old| {
            old.source == source
                && old.revision == revision
                && old.config == config
                && old.state == state
                && old.valid_until == valid_until
                && match (&old.image, &image) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
        });
        if !same {
            self.snapshot = Some(Arc::new(PreviewSnapshot {
                source,
                revision,
                config,
                state,
                image,
                valid_until,
            }));
        }
    }
}

/// All peer metadata must be freshly authenticated and owned by this host.
/// The receiving worker separately matches the complete OS registry against
/// this set; unknown/unmanaged peers cannot inherit a shared-only exception.
pub(crate) fn budget(
    request: Probe,
    completed: (GpuSample, Instant),
    peers: Vec<(NativePeer, Instant)>,
    now: Instant,
) -> Option<BudgetPermit> {
    let (sample, issued) = completed;
    request.source.validate().ok()?;
    if peers.iter().any(|(peer, issued)| {
        !current(*issued, now)
            || peer.cohort.source == request.source
            || peer.luid != sample.peer.luid
            || peer.cohort.source.worker.host != request.source.worker.host
            || peer.cohort.source.worker.installation != request.source.worker.installation
    }) {
        return None;
    }
    let permit = BudgetPermit {
        request: request.challenge,
        sample,
        sample_age_ms: age_millis(issued, now)?,
        peers: peers.into_iter().map(|(peer, _)| peer).collect(),
    };
    permit.validate().ok()?;
    Some(permit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wire::{Cohort, CohortReport};
    fn source() -> PreviewSource {
        wire::fixture_source(3)
    }
    fn peer() -> NativePeer {
        NativePeer {
            cohort: Cohort {
                source: source(),
                device_epoch: 1,
                pid: 101,
            },
            luid: 303,
        }
    }
    fn sample(challenge: u64, frame: u64) -> GpuSample {
        GpuSample {
            peer: peer(),
            challenge,
            surface_epoch: 2,
            frame,
            gpu_micros: 10_000,
        }
    }
    #[test]
    fn preview_evidence_requires_real_child_source_new_completion_and_own_challenge_age() {
        let now = Instant::now();
        let mut e = Evidence::default();
        e.issued(1, now);
        let report = CohortReport {
            peer: peer(),
            challenge: 1,
        };
        assert!(!e.metadata(report, source(), 102, now));
        let mut other = source();
        other.session.generation += 1;
        assert!(!e.metadata(report, other, 101, now));
        assert!(e.metadata(report, source(), 101, now));
        assert!(e.sample(sample(1, 4), source(), 101, now));
        e.issued(2, now + Duration::from_millis(200));
        assert!(
            !e.sample(sample(2, 4), source(), 101, now + Duration::from_millis(200)),
            "a new challenge cannot freshen an old completed frame"
        );
        assert!(e.sample(sample(2, 5), source(), 101, now + Duration::from_millis(200)));
        assert!(e.completed(now + Duration::from_millis(500)).is_some());
        assert!(e.completed(now + Duration::from_millis(701)).is_none());
        assert!(!e.metadata(report, source(), 101, now - Duration::from_nanos(1)));
    }
    #[test]
    fn preview_budget_ceil_never_turns_an_expired_query_into_a_fresh_permit() {
        let now = Instant::now();
        assert_eq!(age_millis(now, now + Duration::from_nanos(1)), Some(1));
        assert_eq!(age_millis(now, now + wire::SAMPLE_FRESH), Some(500));
        assert_eq!(age_millis(now, now + wire::SAMPLE_FRESH + Duration::from_nanos(1)), None);
        assert_eq!(age_millis(now, now - Duration::from_nanos(1)), None);
        let mut receiver = source();
        receiver.worker.worker = aurora_session::WorkerId::from_bytes([8; 16]).unwrap();
        assert!(
            budget(
                Probe {
                    source: receiver,
                    challenge: 9
                },
                (sample(1, 2), now),
                vec![(peer(), now)],
                now
            )
            .is_some()
        );
        assert!(
            budget(
                Probe {
                    source: source(),
                    challenge: 9
                },
                (sample(1, 2), now),
                vec![],
                now
            )
            .is_none()
        );
        assert!(
            budget(
                Probe {
                    source: receiver,
                    challenge: 10
                },
                (sample(1, 3), now + Duration::from_millis(400)),
                vec![(peer(), now)],
                now + wire::SAMPLE_FRESH + Duration::from_nanos(1)
            )
            .is_none(),
            "post-control work may expire peer metadata even while the active sample is still fresh"
        );
    }

    #[test]
    fn preview_configuration_survives_world_switches_but_never_rebinds_to_a_new_login() {
        let now = Instant::now();
        let source = source();
        let config = PreviewConfig {
            size: aurora_session::PreviewSize::Small,
            rate: aurora_session::PreviewRate::Half,
            visible: true,
        };
        let mut peer = Peer::new(source.worker, source.lease_epoch, 101);
        peer.set_control(Some(source), true, true, now);
        assert_eq!(
            peer.configure(source, Some(config), 1, now),
            Err(HostError::Unavailable),
            "online without authentication is no preview capability"
        );
        peer.config = Some((1, config));
        peer.cache = Some(PreviewCache::new(source, 1, config.size, config.rate));
        peer.evidence.issued(8, now);
        peer.set_control(Some(source), true, false, now);
        assert_eq!(peer.config, Some((1, config)), "world activation only suspends this same popup");
        // Closing remains possible when the optional TCP lane is down.
        peer.configure(source, None, 2, now).unwrap();
        assert!(peer.config.is_none() && peer.snapshot().is_none());
        peer.config = Some((3, config));
        let mut next = source;
        next.session.generation += 1;
        peer.set_control(Some(next), true, true, now);
        assert!(peer.config.is_none() && peer.cache.is_none());
        assert_eq!(peer.configure(source, None, 4, now), Err(HostError::Unavailable));
        assert_eq!(
            peer.evidence.next(),
            Some(9),
            "source changes do not reuse authentication-adjacent challenges"
        );
    }

    #[test]
    fn preview_retirement_does_not_retarget_a_new_source_or_free_uncertain_authority() {
        let now = Instant::now();
        let original = source();
        let mut next = original;
        next.session.generation += 1;
        let config = PreviewConfig {
            size: aurora_session::PreviewSize::Small,
            rate: aurora_session::PreviewRate::Half,
            visible: true,
        };
        let mut peer = Peer::new(original.worker, original.lease_epoch, 101);
        peer.set_control(Some(next), true, true, now);
        peer.config = Some((4, config));
        peer.ever_authorized = true;
        peer.retire_request(original, 5, now);
        assert_eq!(peer.config, Some((4, config)), "the old popup cannot hide the new login's preview");
        peer.retire_request(next, 6, now);
        assert!(peer.config.is_none() && peer.snapshot().is_none());
        assert!(peer.has_authority(), "removal/socket loss is never an actual GPU drain proof");
    }
}
