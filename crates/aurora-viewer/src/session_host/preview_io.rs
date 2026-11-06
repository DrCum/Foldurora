//! Bounded in-memory handoff for the authenticated preview transport. This
//! module opens no socket and owns no key. Control must have its own channel:
//! an old partially written image must never delay a privacy invalidation.

use super::preview_protocol::{self as wire, Availability, GpuSample, ImageHeader, Probe, State, Subscription};
use aurora_render::session_preview::CompletedGpuSample;
use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

const MAX_CHALLENGES: usize = 8;
const PROBE_PERIOD: Duration = Duration::from_millis(100);

pub(crate) fn image_ttl(rate: aurora_session::PreviewRate) -> Duration {
    Duration::from_millis(rate.interval_millis()) + wire::CONTROL_FRESH
}

pub(crate) struct Image {
    pub header: ImageHeader,
    pub rgba: Vec<u8>,
    /// Receiver-local expiry; never serialized or accepted from the sender.
    pub valid_until: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PreviewConfig {
    pub size: aurora_session::PreviewSize,
    pub rate: aurora_session::PreviewRate,
    pub visible: bool,
}

#[derive(Clone)]
pub(crate) struct PreviewSnapshot {
    pub source: wire::PreviewSource,
    pub revision: u64,
    pub config: PreviewConfig,
    pub state: Option<State>,
    pub image: Option<Arc<Image>>,
    /// Controller-local state deadline, also checked by the CPU popup when
    /// an IO thread stops publishing snapshots. Unknown must blank the image.
    pub valid_until: Option<Instant>,
}

#[derive(Clone)]
pub(crate) struct Control {
    pub subscription: Arc<Subscription>,
    pub received_at: Instant,
}

#[derive(Default)]
struct Pending {
    generation: u64,
    control: Option<Control>,
    last_control: Option<(wire::PreviewSource, u64, u64)>,
    probes: VecDeque<(Probe, Instant)>,
    last_probe: Option<(Probe, Instant)>,
    budget_probes: VecDeque<(Probe, Instant)>,
    outgoing_probe: Option<Probe>,
    sample: Option<GpuSample>,
    last_sample: Option<GpuSample>,
    peer: Option<wire::CohortReport>,
    state: Option<State>,
    image: Option<Arc<Image>>,
    sequence: u64,
}

pub(crate) struct PreviewMailbox {
    pending: Mutex<Pending>,
    generation: AtomicU64,
}
impl Default for PreviewMailbox {
    fn default() -> Self {
        Self {
            pending: Mutex::new(Pending {
                generation: 1,
                ..Pending::default()
            }),
            generation: AtomicU64::new(1),
        }
    }
}

impl PreviewMailbox {
    fn advance(&self) -> u64 {
        if self
            .generation
            .fetch_update(Ordering::Release, Ordering::Relaxed, |n| {
                (n != 0).then(|| n.checked_add(1)).flatten()
            })
            .is_err()
        {
            self.generation.store(0, Ordering::Release);
        }
        self.generation()
    }
    fn invalidate_pixels(&self, pending: &mut Pending) -> bool {
        let previous = pending.generation;
        let next = previous.checked_add(1).filter(|_| previous != 0);
        let changed = next.is_some_and(|next| {
            self.generation
                .compare_exchange(previous, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        });
        if !changed {
            pending.generation = 0;
            pending.control = None;
            pending.state = None;
            pending.image = None;
            return false;
        }
        pending.generation = next.unwrap_or(0);
        pending.image = None;
        if pending.generation == 0 {
            pending.control = None;
            pending.state = None;
        }
        true
    }
    fn current_pending(&self) -> Option<MutexGuard<'_, Pending>> {
        let pending = self.pending.try_lock().ok()?;
        (pending.generation != 0 && pending.generation == self.generation()).then_some(pending)
    }
    fn renew(&self, p: &mut Pending) {
        let generation = self.generation();
        if p.generation != generation {
            let sequence = p.sequence;
            let last_sample = p.last_sample;
            let last_control = p.last_control;
            *p = Pending {
                generation,
                sequence,
                last_sample,
                last_control,
                ..Pending::default()
            };
        }
    }

    /// A writer snapshots this generation before encoding and rechecks between
    /// bounded writes. On change it closes the image connection, discarding the
    /// partial frame. The separate control connection remains able to purge.
    pub fn generation(&self) -> u64 {
        let generation = self.generation.load(Ordering::Acquire);
        if generation == u64::MAX { 0 } else { generation }
    }

    pub fn receive_subscription(&self, subscription: Subscription, received_at: Instant) -> bool {
        if self.generation() == 0 || subscription.validate().is_err() {
            return false;
        }
        let Ok(mut p) = self.pending.try_lock() else {
            return false;
        };
        self.renew(&mut p);
        if p.last_control.is_some_and(|(source, revision, challenge)| {
            source == subscription.source
                && (revision > subscription.revision || (revision == subscription.revision && challenge >= subscription.challenge))
        }) {
            return false;
        }
        let changed = p.control.as_ref().is_none_or(|old| {
            old.subscription.source != subscription.source
                || old.subscription.revision != subscription.revision
                || old.subscription.visible != subscription.visible
                || old.subscription.size != subscription.size
                || old.subscription.rate != subscription.rate
        });
        if changed {
            if !self.invalidate_pixels(&mut p) {
                return false;
            }
            p.state = None;
            p.probes.clear();
            p.last_probe = None;
            p.budget_probes.clear();
        }
        p.control = Some(Control {
            subscription: Arc::new(subscription),
            received_at,
        });
        p.last_control = p
            .control
            .as_ref()
            .map(|c| (c.subscription.source, c.subscription.revision, c.subscription.challenge));
        true
    }

    pub fn receive_probe(&self, probe: Probe, received_at: Instant) -> bool {
        if self.generation() == 0 || wire::Header::Probe(probe).validate().is_err() {
            return false;
        }
        let Ok(mut p) = self.pending.try_lock() else {
            return false;
        };
        self.renew(&mut p);
        if p.probes
            .back()
            .is_some_and(|(old, _)| old.source == probe.source && old.challenge >= probe.challenge)
        {
            return false;
        }
        while p.probes.len() >= MAX_CHALLENGES {
            p.probes.pop_front();
        }
        p.probes.push_back((probe, received_at));
        true
    }

    pub fn control(&self, now: Instant) -> Option<Control> {
        let p = self.current_pending()?;
        p.control.as_ref().filter(|c| wire::current(c.received_at, now)).cloned()
    }

    /// The standby's own challenge bounds the permit reply's full transit time.
    /// Hosts may not refresh this timestamp by resending a cached permit.
    pub fn request_budget(&self, source: wire::PreviewSource, now: Instant) {
        let Some(mut p) = self.current_pending() else {
            return;
        };
        if p.last_probe
            .is_some_and(|(probe, at)| probe.source == source && now.checked_duration_since(at).is_some_and(|age| age < PROBE_PERIOD))
        {
            return;
        }
        let Some(challenge) = p.sequence.checked_add(1) else {
            self.invalidate_pixels(&mut p);
            p.control = None;
            return;
        };
        p.sequence = challenge;
        let probe = Probe { source, challenge };
        p.last_probe = Some((probe, now));
        while p.budget_probes.len() >= MAX_CHALLENGES {
            p.budget_probes.pop_front();
        }
        p.budget_probes.push_back((probe, now));
        p.outgoing_probe = Some(probe);
    }

    pub fn budget_current(&self, control: &Control, now: Instant) -> bool {
        if !wire::current(control.received_at, now) {
            return false;
        }
        let Some(budget) = &control.subscription.budget else {
            return false;
        };
        if budget.validate().is_err() {
            return false;
        }
        let Some(p) = self.current_pending() else {
            return false;
        };
        p.budget_probes.iter().any(|(probe, issued)| {
            probe.source == control.subscription.source
                && probe.challenge == budget.request
                && now
                    .checked_duration_since(*issued)
                    .is_some_and(|age| age <= wire::BUDGET_ROUND_TRIP)
        })
    }

    pub fn take_budget_request(&self) -> Option<Probe> {
        self.current_pending()?.outgoing_probe.take()
    }

    /// Active workers answer only with a new actual completion whose native
    /// encoding began after receipt of this challenge. Older probes remain
    /// bounded while an asynchronous timestamp readback is still in flight.
    pub fn probe_for_sample(&self, source: wire::PreviewSource, sample: CompletedGpuSample, now: Instant) -> Option<Probe> {
        let mut p = self.current_pending()?;
        p.probes
            .retain(|(_, at)| now.checked_duration_since(*at).is_some_and(|age| age <= wire::SAMPLE_FRESH));
        p.probes
            .iter()
            .rev()
            .find(|(probe, at)| probe.source == source && sample.fresh_after(*at, now))
            .map(|(probe, _)| *probe)
    }

    pub fn latest_probe(&self, source: wire::PreviewSource, now: Instant) -> Option<Probe> {
        self.current_pending()?
            .probes
            .iter()
            .rev()
            .find(|(p, at)| p.source == source && now.checked_duration_since(*at).is_some_and(|age| age <= wire::SAMPLE_FRESH))
            .map(|(p, _)| *p)
    }

    pub fn publish_peer(&self, peer: wire::CohortReport) {
        if peer.validate().is_ok()
            && let Some(mut p) = self.current_pending()
        {
            p.peer = Some(peer);
        }
    }
    pub fn take_peer(&self, now: Instant) -> Option<wire::CohortReport> {
        let mut p = self.current_pending()?;
        let peer = p.peer.take()?;
        p.probes
            .iter()
            .any(|(probe, at)| {
                probe.source == peer.peer.cohort.source
                    && probe.challenge == peer.challenge
                    && now.checked_duration_since(*at).is_some_and(|age| age <= wire::SAMPLE_FRESH)
            })
            .then_some(peer)
    }

    pub fn publish_sample(&self, sample: GpuSample) {
        if sample.validate().is_err() {
            return;
        }
        if let Some(mut p) = self.current_pending() {
            if p.last_sample
                .is_some_and(|old| old.peer == sample.peer && old.surface_epoch == sample.surface_epoch && old.frame >= sample.frame)
            {
                return;
            }
            p.last_sample = Some(sample);
            p.sample = Some(sample);
        }
    }
    pub fn take_sample(&self, now: Instant) -> Option<GpuSample> {
        let mut p = self.current_pending()?;
        let sample = p.sample.take()?;
        p.probes
            .iter()
            .any(|(probe, at)| {
                probe.source == sample.peer.cohort.source
                    && probe.challenge == sample.challenge
                    && now.checked_duration_since(*at).is_some_and(|age| age <= wire::SAMPLE_FRESH)
            })
            .then_some(sample)
    }

    pub fn publish_state(&self, state: State) -> bool {
        if wire::Header::State(state).validate().is_err() {
            return false;
        }
        let Some(mut p) = self.current_pending() else {
            return false;
        };
        if p.control.as_ref().is_none_or(|c| {
            c.subscription.source != state.source
                || c.subscription.revision != state.subscription_revision
                || c.subscription.challenge != state.challenge
                || (!c.subscription.visible && state.availability == Availability::Live)
        }) {
            return false;
        }
        if p.state.is_some_and(|old| {
            old.source == state.source
                && old.subscription_revision == state.subscription_revision
                && (state.epoch < old.epoch || (state.epoch == old.epoch && state.challenge < old.challenge))
        }) {
            return false;
        }
        let changed = p.state.is_none_or(|old| {
            old.source != state.source
                || old.subscription_revision != state.subscription_revision
                || old.epoch != state.epoch
                || state.availability != Availability::Live
        });
        if changed && !self.invalidate_pixels(&mut p) {
            return false;
        }
        p.state = Some(state);
        true
    }

    pub fn publish_image(&self, header: ImageHeader, rgba: Vec<u8>) -> bool {
        if wire::Header::Image(header).validate().is_err() || rgba.len() != header.size.bytes() as usize {
            return false;
        }
        let Some(mut p) = self.current_pending() else {
            return false;
        };
        if p.control.as_ref().is_none_or(|c| {
            c.subscription.source != header.source
                || c.subscription.revision != header.subscription_revision
                || c.subscription.size != header.size
                || !c.subscription.visible
        }) {
            return false;
        }
        let allowed = p.state.is_some_and(|s| {
            s.source == header.source
                && s.subscription_revision == header.subscription_revision
                && s.epoch == header.epoch
                && s.availability == Availability::Live
                && s.challenge >= header.challenge
        });
        if !allowed || p.image.as_ref().is_some_and(|old| old.header.sequence >= header.sequence) {
            return false;
        }
        let rate = p.control.as_ref().map(|c| c.subscription.rate);
        let Some(rate) = rate else {
            return false;
        };
        let image = Arc::new(Image {
            header,
            rgba,
            valid_until: Instant::now() + image_ttl(rate),
        });
        p.image = Some(image);
        true
    }

    pub fn state(&self) -> Option<State> {
        let p = self.current_pending()?;
        let state = p.state;
        let generation = p.generation;
        drop(p);
        (self.generation() == generation).then_some(state).flatten()
    }
    pub fn image(&self) -> Option<Arc<Image>> {
        let p = self.current_pending()?;
        let image = p.image.clone();
        let generation = p.generation;
        drop(p);
        (self.generation() == generation).then_some(image).flatten()
    }

    pub fn purge_pixels(&self) {
        match self.pending.try_lock() {
            Ok(mut p) => {
                if p.image.is_some() || p.state.is_some() || p.control.is_some() {
                    self.invalidate_pixels(&mut p);
                    p.state = None;
                    p.control = None;
                    p.budget_probes.clear();
                    p.last_probe = None;
                }
            }
            // Never discard an invalidation because the producer has the lock.
            // Its data remain tagged with the obsolete generation.
            Err(_) => {
                self.advance();
            }
        }
    }

    pub fn disconnect(&self) {
        let generation = self.advance();
        if let Ok(mut p) = self.pending.try_lock() {
            let sequence = p.sequence;
            let last_sample = p.last_sample;
            let last_control = p.last_control;
            *p = Pending {
                generation,
                sequence,
                last_sample,
                last_control,
                ..Pending::default()
            };
        }
    }
}

/// Controller-side image cache. A heartbeat's repeated arrival never extends
/// freshness: expiry is measured from the controller's own challenge issuance.
pub(crate) struct PreviewCache {
    source: wire::PreviewSource,
    revision: u64,
    size: aurora_session::PreviewSize,
    rate: aurora_session::PreviewRate,
    challenges: VecDeque<(u64, Instant)>,
    next: u64,
    state: Option<(State, Instant)>,
    image: Option<Arc<Image>>,
    image_issued: Option<Instant>,
    epoch_floor: u64,
    sequence_floor: u64,
}
impl PreviewCache {
    pub fn new(source: wire::PreviewSource, revision: u64, size: aurora_session::PreviewSize, rate: aurora_session::PreviewRate) -> Self {
        Self {
            source,
            revision,
            size,
            rate,
            challenges: VecDeque::new(),
            next: 0,
            state: None,
            image: None,
            image_issued: None,
            epoch_floor: 0,
            sequence_floor: 0,
        }
    }
    pub fn challenge(&mut self, now: Instant) -> Option<u64> {
        self.next = self.next.checked_add(1)?;
        while self.challenges.len() >= MAX_CHALLENGES {
            self.challenges.pop_front();
        }
        self.challenges.push_back((self.next, now));
        Some(self.next)
    }
    fn issued(&self, challenge: u64, now: Instant) -> Option<Instant> {
        self.challenges
            .iter()
            .find(|(id, at)| *id == challenge && wire::current(*at, now))
            .map(|(_, at)| *at)
    }
    pub fn receive_state(&mut self, state: State, now: Instant) -> bool {
        if wire::Header::State(state).validate().is_err() || state.source != self.source || state.subscription_revision != self.revision {
            return false;
        }
        let Some(issued) = self.issued(state.challenge, now) else {
            self.purge();
            return false;
        };
        if state.epoch < self.epoch_floor
            || self
                .state
                .is_some_and(|(old, _)| state.epoch == old.epoch && state.challenge < old.challenge)
        {
            return false;
        }
        if state.epoch > self.epoch_floor {
            self.sequence_floor = 0;
        }
        self.epoch_floor = state.epoch;
        if state.availability != Availability::Live || self.state.is_none_or(|(old, _)| old.epoch != state.epoch) {
            self.image = None;
            self.image_issued = None;
        }
        self.state = Some((state, issued));
        true
    }
    pub fn receive_image(&mut self, header: ImageHeader, rgba: Vec<u8>, now: Instant) -> bool {
        if wire::Header::Image(header).validate().is_err()
            || header.source != self.source
            || header.subscription_revision != self.revision
            || header.size != self.size
            || rgba.len() != header.size.bytes() as usize
            || self.issued(header.challenge, now).is_none()
        {
            return false;
        }
        let ready = self.state.is_some_and(|(state, issued)| {
            state.epoch == header.epoch && state.availability == Availability::Live && wire::current(issued, now)
        });
        if !ready || header.epoch != self.epoch_floor || header.sequence <= self.sequence_floor {
            return false;
        }
        self.sequence_floor = header.sequence;
        let Some(issued) = self.issued(header.challenge, now) else {
            return false;
        };
        self.image_issued = Some(issued);
        self.image = Some(Arc::new(Image {
            header,
            rgba,
            valid_until: issued + image_ttl(self.rate),
        }));
        true
    }
    pub fn image(&mut self, now: Instant) -> Option<Arc<Image>> {
        if self.state.is_none_or(|(_, issued)| !wire::current(issued, now)) {
            self.purge();
        }
        if self
            .image_issued
            .is_none_or(|issued| now.checked_duration_since(issued).is_none_or(|age| age > image_ttl(self.rate)))
        {
            self.image = None;
            self.image_issued = None;
        }
        self.image.clone()
    }
    pub fn state(&mut self, now: Instant) -> Option<State> {
        let _ = self.image(now);
        self.state.map(|(mut s, _)| {
            if self.image.is_none() && s.availability == Availability::Live {
                s.availability = Availability::Initializing;
            }
            s
        })
    }
    pub fn valid_until(&self) -> Option<Instant> {
        let state = self.state.map(|(_, at)| at + wire::CONTROL_FRESH)?;
        Some(self.image_issued.map_or(state, |at| state.min(at + image_ttl(self.rate))))
    }
    pub fn purge(&mut self) {
        self.image = None;
        self.image_issued = None;
        self.state = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::{PreviewRate, PreviewSize};
    use wire::{BudgetPermit, Cohort, Peer, PreviewSource};

    fn source() -> PreviewSource {
        wire::fixture_source(3)
    }
    fn control(challenge: u64) -> Subscription {
        Subscription {
            source: source(),
            revision: 1,
            size: PreviewSize::Small,
            rate: PreviewRate::Half,
            visible: true,
            challenge,
            budget: None,
        }
    }
    fn state(challenge: u64, epoch: u64, availability: Availability) -> State {
        State {
            source: source(),
            subscription_revision: 1,
            epoch,
            availability,
            challenge,
        }
    }
    fn header(challenge: u64, epoch: u64, sequence: u64) -> ImageHeader {
        ImageHeader {
            source: source(),
            subscription_revision: 1,
            epoch,
            sequence,
            size: PreviewSize::Small,
            challenge,
        }
    }
    fn pixels() -> Vec<u8> {
        vec![12; PreviewSize::Small.bytes() as usize]
    }

    #[test]
    fn preview_privacy_and_disconnect_invalidation_survive_a_held_producer_lock() {
        let mailbox = PreviewMailbox::default();
        let now = Instant::now();
        mailbox.receive_subscription(control(1), now);
        mailbox.publish_state(state(1, 1, Availability::Live));
        mailbox.publish_image(header(1, 1, 1), pixels());
        let before = mailbox.generation();
        let mut held = mailbox.pending.lock().unwrap();
        mailbox.purge_pixels();
        assert_ne!(mailbox.generation(), before);
        assert!(
            !mailbox.invalidate_pixels(&mut held),
            "obsolete producer cannot retag its old control/pixels"
        );
        drop(held);
        assert!(mailbox.image().is_none());
        assert!(mailbox.state().is_none());
        assert!(!mailbox.publish_state(state(1, 1, Availability::Live)));
        assert!(mailbox.receive_subscription(control(2), now));
        mailbox.publish_state(state(2, 2, Availability::Live));
        mailbox.publish_image(header(2, 2, 2), pixels());
        let held = mailbox.pending.lock().unwrap();
        mailbox.disconnect();
        drop(held);
        assert!(mailbox.image().is_none());
        assert!(mailbox.control(now).is_none());
        assert!(!mailbox.publish_state(state(2, 2, Availability::Live)));
    }

    #[test]
    fn ordinary_preview_purge_requires_a_new_subscription_before_obsolete_callbacks_can_publish() {
        let m = PreviewMailbox::default();
        let now = Instant::now();
        assert!(m.receive_subscription(control(1), now));
        assert!(m.publish_state(state(1, 1, Availability::Live)));
        assert!(m.publish_image(header(1, 1, 1), pixels()));
        m.purge_pixels();
        assert!(m.control(now).is_none());
        assert!(m.image().is_none());
        assert!(!m.publish_state(state(1, 1, Availability::Live)));
        assert!(!m.publish_image(header(1, 1, 2), pixels()));
        assert!(
            !m.receive_subscription(control(1), now),
            "old buffered subscription cannot reauthorize a purged producer"
        );
        assert!(m.receive_subscription(control(2), now));
        assert!(!m.publish_state(state(1, 1, Availability::Live)));
        assert!(m.publish_state(state(2, 2, Availability::Restricted)));
        assert!(!m.publish_image(header(1, 1, 2), pixels()));
    }

    #[test]
    fn preview_live_heartbeats_do_not_extend_an_images_own_challenge_deadline() {
        let now = Instant::now();
        let mut cache = PreviewCache::new(source(), 1, PreviewSize::Small, PreviewRate::Five);
        let first = cache.challenge(now).unwrap();
        cache.receive_state(state(first, 1, Availability::Live), now);
        cache.receive_image(header(first, 1, 1), pixels(), now);
        let next_at = now + Duration::from_millis(500);
        let next = cache.challenge(next_at).unwrap();
        assert!(cache.receive_state(state(next, 1, Availability::Live), next_at));
        assert!(cache.image(now + Duration::from_millis(949)).is_some());
        assert!(cache.image(now + Duration::from_millis(951)).is_none());
        assert_eq!(
            cache.state(now + Duration::from_millis(951)).unwrap().availability,
            Availability::Initializing
        );
        assert!(!cache.receive_image(header(first, 1, 2), pixels(), now + Duration::from_millis(951)));
        assert!(cache.receive_image(header(next, 1, 2), pixels(), now + Duration::from_millis(951)));
        let image = cache.image(now + Duration::from_millis(951)).unwrap();
        assert_eq!(image.valid_until, next_at + image_ttl(PreviewRate::Five));
    }

    #[test]
    fn half_fps_snapshot_holds_to_its_immutable_period_plus_slack_but_privacy_or_authority_can_blank_earlier() {
        let now = Instant::now();
        let mut cache = PreviewCache::new(source(), 1, PreviewSize::Small, PreviewRate::Half);
        let first = cache.challenge(now).unwrap();
        cache.receive_state(state(first, 1, Availability::Live), now);
        cache.receive_image(header(first, 1, 1), pixels(), now);
        let original = cache.image(now).unwrap();
        assert_eq!(original.valid_until, now + Duration::from_millis(2750));
        for ms in [500, 1000, 1500, 2000, 2500] {
            let at = now + Duration::from_millis(ms);
            let challenge = cache.challenge(at).unwrap();
            assert!(cache.receive_state(state(challenge, 1, Availability::Live), at));
            assert!(cache.image(at).is_some());
            assert_eq!(cache.image(at).unwrap().valid_until, original.valid_until);
        }
        assert_eq!(cache.valid_until(), Some(original.valid_until));
        assert!(cache.image(now + Duration::from_millis(2751)).is_none());
        let at = now + Duration::from_secs(3);
        let challenge = cache.challenge(at).unwrap();
        cache.receive_state(state(challenge, 2, Availability::Live), at);
        cache.receive_image(header(challenge, 2, 2), pixels(), at);
        assert!(cache.image(at).is_some());
        cache.receive_state(state(challenge, 3, Availability::Restricted), at);
        assert!(cache.image(at).is_none());
        assert!(!cache.receive_image(header(challenge, 2, 3), pixels(), at));
        cache.receive_state(state(challenge, 4, Availability::Live), at);
        cache.receive_image(header(challenge, 4, 4), pixels(), at);
        assert!(
            cache.image(at + wire::CONTROL_FRESH + Duration::from_nanos(1)).is_none(),
            "a longer image deadline never extends Live authority"
        );
    }

    #[test]
    fn preview_mailbox_keeps_one_latest_frame_and_purges_before_old_callbacks_can_resurrect_pixels() {
        let m = PreviewMailbox::default();
        let now = Instant::now();
        assert!(m.receive_subscription(control(1), now));
        assert!(m.publish_state(state(1, 1, Availability::Live)));
        assert!(m.publish_image(header(1, 1, 1), pixels()));
        assert!(m.publish_image(header(1, 1, 2), pixels()));
        assert_eq!(m.image().unwrap().header.sequence, 2);
        assert!(!m.publish_image(header(1, 1, 1), pixels()));
        let generation = m.generation();
        assert!(m.receive_subscription(
            Subscription {
                visible: false,
                ..control(2)
            },
            now
        ));
        assert!(m.image().is_none());
        assert!(m.generation() > generation);
        assert!(!m.publish_state(state(1, 1, Availability::Live)));
        assert!(!m.publish_image(header(1, 1, 3), pixels()));
        assert!(m.receive_subscription(control(3), now));
        assert!(m.publish_state(state(3, 2, Availability::Restricted)));
        assert!(!m.publish_image(header(3, 1, 4), pixels()));
        assert!(m.image().is_none());
        assert!(m.publish_state(state(3, 3, Availability::Live)));
        assert!(m.publish_image(header(3, 3, 4), pixels()));
        m.disconnect();
        assert!(m.image().is_none());
        assert!(m.control(now).is_none());
        assert!(!m.publish_state(state(3, 3, Availability::Live)));
    }

    #[test]
    fn preview_budget_requires_own_recent_request_and_bounded_active_sample_age() {
        let m = PreviewMailbox::default();
        let now = Instant::now();
        m.request_budget(source(), now);
        let request = m.take_budget_request().unwrap();
        let peer = Peer {
            cohort: Cohort {
                source: wire::fixture_source(9),
                device_epoch: 1,
                pid: 200,
            },
            luid: 42,
        };
        let sample = GpuSample {
            peer,
            challenge: 77,
            surface_epoch: 2,
            frame: 5,
            gpu_micros: 99_000,
        };
        let budget = BudgetPermit {
            request: request.challenge,
            sample,
            sample_age_ms: 500,
            peers: vec![peer],
        };
        assert!(m.receive_subscription(
            Subscription {
                budget: Some(budget.clone()),
                ..control(1)
            },
            now
        ));
        // A policy change clears any old request; a new request is required.
        assert!(!m.budget_current(&m.control(now).unwrap(), now));
        m.request_budget(source(), now);
        let request = m.take_budget_request().unwrap();
        assert!(m.receive_subscription(
            Subscription {
                budget: Some(BudgetPermit {
                    request: request.challenge,
                    ..budget.clone()
                }),
                ..control(2)
            },
            now
        ));
        let received = m.control(now).unwrap();
        assert!(m.budget_current(&received, now + wire::BUDGET_ROUND_TRIP));
        assert!(!m.budget_current(&received, now + wire::BUDGET_ROUND_TRIP + Duration::from_nanos(1)));
        assert!(!m.budget_current(&received, now - Duration::from_nanos(1)));
        assert!(
            BudgetPermit {
                sample_age_ms: 501,
                ..budget.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            BudgetPermit {
                sample: GpuSample {
                    gpu_micros: 100_001,
                    ..sample
                },
                ..budget
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn cached_or_queued_gpu_samples_cannot_answer_a_new_probe_or_refresh_a_completion_id() {
        let m = PreviewMailbox::default();
        let now = Instant::now();
        let probe = Probe {
            source: source(),
            challenge: 1,
        };
        m.receive_probe(probe, now);
        let sample = CompletedGpuSample {
            frame: 2,
            surface_epoch: 3,
            submitted_at: now,
            completed_at: now + Duration::from_millis(25),
            gpu_ms: 20.0,
        };
        assert_eq!(m.probe_for_sample(source(), sample, now + Duration::from_millis(30)), Some(probe));
        m.receive_probe(Probe { challenge: 2, ..probe }, now + Duration::from_millis(50));
        assert_eq!(m.probe_for_sample(source(), sample, now + Duration::from_millis(60)), Some(probe));
        let peer = Peer {
            cohort: Cohort {
                source: source(),
                device_epoch: 1,
                pid: 200,
            },
            luid: 42,
        };
        let sample = GpuSample {
            peer,
            challenge: 1,
            surface_epoch: 3,
            frame: 2,
            gpu_micros: 20_000,
        };
        m.publish_sample(sample);
        assert_eq!(m.take_sample(now + Duration::from_millis(60)), Some(sample));
        m.publish_sample(GpuSample { challenge: 2, ..sample });
        assert!(m.take_sample(now + Duration::from_millis(60)).is_none());
        m.publish_sample(GpuSample {
            frame: 3,
            challenge: 2,
            ..sample
        });
        assert!(
            m.take_sample(now + Duration::from_millis(551)).is_none(),
            "connection delay must not release a stale queued sample"
        );
        let report = wire::CohortReport { peer, challenge: 2 };
        m.publish_peer(report);
        assert_eq!(m.take_peer(now + Duration::from_millis(550)), Some(report));
        m.publish_peer(report);
        assert!(m.take_peer(now + Duration::from_millis(551)).is_none());
    }

    #[test]
    fn controller_preview_cache_uses_challenge_issue_age_and_cannot_revive_previous_privacy_epoch() {
        let now = Instant::now();
        let mut cache = PreviewCache::new(source(), 1, PreviewSize::Small, PreviewRate::Five);
        let a = cache.challenge(now).unwrap();
        assert!(cache.receive_state(state(a, 1, Availability::Live), now));
        assert!(cache.receive_image(header(a, 1, 1), pixels(), now));
        assert!(cache.image(now + Duration::from_millis(700)).is_some());
        assert!(cache.receive_state(state(a, 1, Availability::Live), now + Duration::from_millis(700)));
        assert!(
            cache.image(now + wire::CONTROL_FRESH + Duration::from_nanos(1)).is_none(),
            "repeated receipt must not renew issue time"
        );
        let b = cache.challenge(now + Duration::from_secs(1)).unwrap();
        assert!(cache.receive_state(state(b, 2, Availability::Restricted), now + Duration::from_secs(1)));
        assert!(!cache.receive_image(header(b, 1, 2), pixels(), now + Duration::from_secs(1)));
        assert!(!cache.receive_state(state(b, 1, Availability::Live), now + Duration::from_secs(1)));
        assert!(cache.image(now + Duration::from_secs(1)).is_none());
        assert!(cache.receive_state(state(b, 3, Availability::Live), now + Duration::from_secs(1)));
        assert!(cache.receive_image(header(b, 3, 3), pixels(), now + Duration::from_secs(1)));
        assert!(!cache.receive_image(header(b, 3, 2), pixels(), now + Duration::from_secs(1)));
        assert!(cache.image(now + Duration::from_secs(1)).is_some());
    }
}
