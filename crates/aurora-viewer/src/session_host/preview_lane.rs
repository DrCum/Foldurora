//! Worker-side preview IO, separate from control acknowledgments and App ticks.
//! One small control socket and one image socket belong exclusively to this
//! thread. The mailbox retains the newest pixels; this owner queues at most
//! one image and never replays it after an uncertain socket failure.

use super::bridge_auth::{HANDSHAKE_DEADLINE, Permit};
use super::preview_io::{Image, PreviewMailbox};
use super::preview_protocol::{Availability, Header, ImageHeader, PreviewSource, State};
use super::preview_transport::{Error, Kind, Packet, WorkerIo};
use aurora_session::WorkerIdentity;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const PERIOD: Duration = Duration::from_millis(20);
const RETRY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
struct Scope {
    identity: WorkerIdentity,
    lease_epoch: u64,
}

impl Scope {
    fn accepts(self, source: PreviewSource) -> bool {
        source.worker == self.identity && source.lease_epoch == self.lease_epoch && source.validate().is_ok()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Authority {
    revision: u64,
    source: PreviewSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceUpdate {
    Unchanged,
    Renewed,
    Changed,
}

struct Source {
    scope: Scope,
    source: Option<PreviewSource>,
    expires: Option<Instant>,
    revision: u64,
}

impl Source {
    fn new(scope: Scope) -> Self {
        Self {
            scope,
            source: None,
            expires: None,
            revision: 1,
        }
    }

    fn replace(&mut self, source: Option<PreviewSource>, expires: Option<Instant>) {
        self.revision = if self.revision == 0 {
            0
        } else {
            self.revision.checked_add(1).unwrap_or(0)
        };
        self.source = (self.revision != 0).then_some(source).flatten();
        self.expires = self.source.and(expires);
    }

    fn expire(&mut self, now: Instant) -> bool {
        if self.source.is_some() && self.expires.is_none_or(|expires| expires <= now) {
            self.replace(None, None);
            true
        } else {
            false
        }
    }

    fn set(&mut self, source: Option<PreviewSource>, expires: Option<Instant>, now: Instant) -> SourceUpdate {
        let expired = self.expire(now);
        let source = source.filter(|source| self.scope.accepts(*source) && expires.is_some_and(|expires| expires > now));
        let expires = source.and(expires);
        if self.source == source && self.expires == expires {
            return if expired { SourceUpdate::Changed } else { SourceUpdate::Unchanged };
        }
        if self.source == source {
            self.expires = expires;
            return if expired { SourceUpdate::Changed } else { SourceUpdate::Renewed };
        }
        self.replace(source, expires);
        SourceUpdate::Changed
    }

    fn authority(&self, now: Instant) -> Option<Authority> {
        (self.revision != 0 && self.expires.is_some_and(|expires| expires > now))
            .then_some(self.source)
            .flatten()
            .map(|source| Authority {
                revision: self.revision,
                source,
            })
    }
}

struct Shared {
    source: Mutex<Source>,
    mailbox: Arc<PreviewMailbox>,
    wake: Condvar,
    stop: AtomicBool,
}

impl Shared {
    fn source(&self) -> MutexGuard<'_, Source> {
        self.source.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn expire(&self, source: &mut Source, now: Instant) {
        if source.expire(now) {
            // Atomic mailbox invalidation survives a contended producer lock.
            self.mailbox.disconnect();
        }
        if source.revision == 0 || self.mailbox.generation() == 0 {
            self.stop.store(true, Ordering::Release);
        }
    }

    fn authority(&self) -> Option<Authority> {
        let mut source = self.source();
        let now = Instant::now();
        self.expire(&mut source, now);
        (!self.stop.load(Ordering::Acquire)).then(|| source.authority(now)).flatten()
    }

    fn current(&self, authority: Authority) -> bool {
        self.authority() == Some(authority)
    }

    fn can_connect(&self, authority: Authority, now: Instant) -> bool {
        let source = self.source();
        !self.stop.load(Ordering::Acquire)
            && source.authority(now) == Some(authority)
            && source
                .expires
                .is_some_and(|expires| expires.saturating_duration_since(now) > HANDSHAKE_DEADLINE)
    }

    fn incoming(&self, authority: Authority, packet: Packet, now: Instant) {
        // Keep this check and the mailbox counter mutation under the same
        // small source lock, so a rebind cannot admit an old source afterward.
        let mut source = self.source();
        self.expire(&mut source, now);
        if self.stop.load(Ordering::Acquire)
            || source.authority(now) != Some(authority)
            || packet.header.source() != authority.source
            || !packet.rgba.is_empty()
        {
            return;
        }
        match packet.header {
            Header::Subscribe(subscription) => {
                self.mailbox.receive_subscription(subscription, now);
            }
            Header::Probe(probe) => {
                self.mailbox.receive_probe(probe, now);
            }
            _ => {}
        }
    }

    fn reports(&self, authority: Authority, now: Instant) -> Vec<Header> {
        let mut source = self.source();
        self.expire(&mut source, now);
        if self.stop.load(Ordering::Acquire) || source.authority(now) != Some(authority) {
            return Vec::new();
        }
        // Latest optional reports are expendable on Busy; no response FIFO or
        // retransmission history can accumulate behind image work.
        let mut reports = Vec::with_capacity(3);
        if let Some(peer) = self.mailbox.take_peer(now) {
            reports.push(Header::Peer(peer));
        }
        if let Some(sample) = self.mailbox.take_sample(now) {
            reports.push(Header::Sample(sample));
        }
        if let Some(probe) = self.mailbox.take_budget_request() {
            reports.push(Header::Probe(probe));
        }
        reports.retain(|header| header.source() == authority.source);
        reports
    }

    fn wait(&self) {
        let source = self.source();
        if !self.stop.load(Ordering::Acquire) {
            let duration = source
                .expires
                .map_or(PERIOD, |expires| PERIOD.min(expires.saturating_duration_since(Instant::now())));
            drop(
                self.wake
                    .wait_timeout(source, duration)
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
        }
    }
}

pub(crate) struct WorkerChannel {
    shared: Arc<Shared>,
}

impl WorkerChannel {
    pub fn start(control_port: u16, image_port: u16, permit: Permit) -> io::Result<Self> {
        if control_port == 0 || image_port == 0 || control_port == image_port || !permit.valid() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid worker preview endpoints"));
        }
        let shared = Arc::new(Shared {
            source: Mutex::new(Source::new(Scope {
                identity: permit.identity,
                lease_epoch: permit.lease_epoch,
            })),
            mailbox: Arc::new(PreviewMailbox::default()),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("session-worker-preview".into())
            .spawn(move || run(control_port, image_port, permit, worker))?;
        Ok(Self { shared })
    }

    pub fn mailbox(&self) -> Arc<PreviewMailbox> {
        self.shared.mailbox.clone()
    }

    /// Supply the control bridge's exact local receipt deadline. A same-source
    /// renewal keeps sockets; logout, a source change or expiry invalidates all
    /// mailbox publications before any old image may make further progress.
    pub fn set_source(&self, source: Option<PreviewSource>, expires: Option<Instant>) {
        if self.shared.stop.load(Ordering::Acquire) {
            return;
        }
        let mut current = self.shared.source();
        let update = current.set(source, expires, Instant::now());
        if update == SourceUpdate::Changed {
            self.shared.mailbox.disconnect();
        }
        if current.revision == 0 {
            self.shared.stop.store(true, Ordering::Release);
        }
        if update != SourceUpdate::Unchanged {
            self.shared.wake.notify_one();
        }
    }
}

impl Drop for WorkerChannel {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.mailbox.disconnect();
        self.shared.wake.notify_one();
        // A connect can still be in its one-second timeout: never join here.
    }
}

#[derive(Clone, Copy)]
struct ImageGuard {
    authority: Authority,
    generation: u64,
    header: ImageHeader,
    valid_until: Instant,
}

impl ImageGuard {
    fn current(self, shared: &Shared, now: Instant) -> bool {
        if now >= self.valid_until || !connection_current(shared, self.authority, self.generation, now) {
            return false;
        }
        let ready = shared.mailbox.state().is_some_and(|state| {
            state.source == self.header.source
                && state.subscription_revision == self.header.subscription_revision
                && state.epoch == self.header.epoch
                && state.challenge >= self.header.challenge
                && state.availability == Availability::Live
        });
        let subscribed = shared.mailbox.control(now).is_some_and(|control| {
            control.subscription.source == self.header.source
                && control.subscription.revision == self.header.subscription_revision
                && control.subscription.size == self.header.size
                && control.subscription.visible
        });
        ready && subscribed && shared.mailbox.generation() == self.generation && shared.current(self.authority)
    }
}

fn connection_current(shared: &Shared, authority: Authority, generation: u64, now: Instant) -> bool {
    generation != 0
        && shared.mailbox.generation() == generation
        && shared.current(authority)
        && shared
            .mailbox
            .control(now)
            .is_some_and(|control| control.subscription.source == authority.source && control.subscription.visible)
        && shared.mailbox.generation() == generation
}

fn candidate(shared: &Shared, authority: Authority) -> Option<(Arc<Image>, ImageGuard)> {
    let generation = shared.mailbox.generation();
    let image = shared.mailbox.image()?;
    let guard = ImageGuard {
        authority,
        generation,
        header: image.header,
        valid_until: image.valid_until,
    };
    guard.current(shared, Instant::now()).then_some((image, guard))
}

#[derive(Default)]
struct Images {
    last: Option<(u64, u64)>,
    in_flight: Option<ImageGuard>,
}

impl Images {
    fn pending(&self, guard: ImageGuard) -> bool {
        self.last
            .is_none_or(|(generation, sequence)| generation != guard.generation || sequence < guard.header.sequence)
    }
    fn queued(&mut self, guard: ImageGuard) {
        self.last = Some((guard.generation, guard.header.sequence));
        self.in_flight = Some(guard);
    }
    fn disconnected(&mut self) {
        // Preserve last: an uncertain image is not replayed after reconnect.
        self.in_flight = None;
    }
}

struct ImageSocket {
    io: WorkerIo,
    generation: u64,
}

struct Retry {
    next: Instant,
}
impl Retry {
    fn new(now: Instant) -> Self {
        Self { next: now }
    }
    fn due(&self, now: Instant) -> bool {
        now >= self.next
    }
    fn attempted(&mut self, now: Instant) {
        self.next = now + RETRY;
    }
    fn failed(&mut self, now: Instant) {
        self.next = self.next.max(now + RETRY);
    }
}

struct Ended(Arc<Shared>);
impl Drop for Ended {
    fn drop(&mut self) {
        self.0.stop.store(true, Ordering::Release);
        self.0.mailbox.disconnect();
        self.0.wake.notify_all();
    }
}

#[derive(Default)]
struct ControlState {
    last: Option<State>,
    generation: Option<u64>,
}

impl ControlState {
    fn accepts_generation(&self, generation: u64) -> bool {
        generation != 0 && self.generation.is_none_or(|queued| queued == generation)
    }
    fn queued(&mut self, state: State, generation: u64) {
        self.last = Some(state);
        self.generation = Some(generation);
    }
    fn drained(&mut self) {
        self.generation = None;
    }
}

fn control_current(shared: &Shared, authority: Authority, generation: Option<u64>) -> bool {
    shared.current(authority) && generation.is_none_or(|generation| generation != 0 && shared.mailbox.generation() == generation)
}

fn queue_control(socket: &mut WorkerIo, shared: &Shared, authority: Authority, state_cursor: &mut ControlState) -> Result<(), Error> {
    if !socket.has_pending_writes() {
        state_cursor.drained();
    }
    if !control_current(shared, authority, state_cursor.generation) {
        return Err(Error::Closed);
    }
    let generation = shared.mailbox.generation();
    if !state_cursor.accepts_generation(generation) {
        return Err(Error::Closed);
    }
    if let Some(state) = shared
        .mailbox
        .state()
        .filter(|state| state.source == authority.source && Some(*state) != state_cursor.last)
    {
        if !shared.current(authority) || shared.mailbox.generation() != generation {
            return Err(Error::Closed);
        }
        match socket.queue(&Header::State(state), &[]) {
            Ok(()) => {
                // Encoding is off-thread, but a concurrent privacy change
                // must still discard this queued state before any write.
                if !shared.current(authority) || shared.mailbox.generation() != generation {
                    return Err(Error::Closed);
                }
                state_cursor.queued(state, generation);
            }
            Err(Error::Busy) => {}
            Err(error) => return Err(error),
        }
    }
    for report in shared.reports(authority, Instant::now()) {
        if !shared.current(authority) {
            return Err(Error::Closed);
        }
        match socket.queue(&report, &[]) {
            Ok(()) | Err(Error::Busy) => {}
            Err(error) => return Err(error),
        }
        if !shared.current(authority) {
            return Err(Error::Closed);
        }
    }
    Ok(())
}

fn queue_image(
    shared: &Shared,
    images: &mut Images,
    image: &Image,
    guard: ImageGuard,
    connection_generation: u64,
    queue: impl FnOnce(&Header, &[u8]) -> Result<(), Error>,
) -> Result<(), Error> {
    if guard.generation != connection_generation || !guard.current(shared, Instant::now()) {
        return Err(Error::Closed);
    }
    match queue(&Header::Image(image.header), &image.rgba) {
        Ok(()) => {
            images.queued(guard);
            if !guard.current(shared, Instant::now()) {
                return Err(Error::Closed);
            }
            Ok(())
        }
        Err(Error::Busy) => Ok(()),
        Err(error) => Err(error),
    }
}

fn run(control_port: u16, image_port: u16, permit: Permit, shared: Arc<Shared>) {
    let _ended = Ended(shared.clone());
    let mut authority = None;
    let mut control: Option<WorkerIo> = None;
    let mut image: Option<ImageSocket> = None;
    let mut control_retry = Retry::new(Instant::now());
    let mut image_retry = Retry::new(Instant::now());
    let mut images = Images::default();
    let mut state_cursor = ControlState::default();
    while !shared.stop.load(Ordering::Acquire) {
        let current = shared.authority();
        if current != authority {
            control = None;
            image = None;
            images = Images::default();
            state_cursor = ControlState::default();
            authority = current;
        }
        let Some(authority) = authority else {
            shared.wait();
            continue;
        };
        if control.is_none() && control_retry.due(Instant::now()) && shared.can_connect(authority, Instant::now()) {
            control_retry.attempted(Instant::now());
            match WorkerIo::connect(control_port, permit.clone(), Kind::Control) {
                Ok(socket) if shared.current(authority) => control = Some(socket),
                _ => control_retry.failed(Instant::now()),
            }
            continue;
        }
        let mut control_failed = false;
        if let Some(socket) = &mut control {
            if socket.ready() && queue_control(socket, &shared, authority, &mut state_cursor).is_err() {
                control_failed = true;
            }
            if !control_failed {
                let generation = state_cursor.generation;
                match socket.poll_current(|| control_current(&shared, authority, generation)) {
                    Ok(packets) => {
                        if !socket.has_pending_writes() {
                            state_cursor.drained();
                        }
                        for packet in packets {
                            shared.incoming(authority, packet, Instant::now());
                        }
                    }
                    Err(_) => control_failed = true,
                }
            }
        }
        if control_failed {
            control = None;
            image = None;
            images = Images::default();
            state_cursor = ControlState::default();
            shared.mailbox.disconnect();
            control_retry.failed(Instant::now());
            shared.wait();
            continue;
        }

        if image
            .as_ref()
            .is_some_and(|socket| !connection_current(&shared, authority, socket.generation, Instant::now()))
        {
            image = None;
            images.disconnected();
            image_retry.failed(Instant::now());
        }
        if !control.as_ref().is_some_and(WorkerIo::ready) {
            shared.wait();
            continue;
        }
        if image.is_none() && image_retry.due(Instant::now()) && shared.can_connect(authority, Instant::now()) {
            if let Some((_, guard)) = candidate(&shared, authority).filter(|(_, guard)| images.pending(*guard)) {
                image_retry.attempted(Instant::now());
                match WorkerIo::connect(image_port, permit.clone(), Kind::Image) {
                    Ok(socket) if guard.current(&shared, Instant::now()) => {
                        image = Some(ImageSocket {
                            io: socket,
                            generation: guard.generation,
                        })
                    }
                    _ => image_retry.failed(Instant::now()),
                }
                // Poll the priority control lane again after a slow connect.
                continue;
            }
        }
        let mut image_failed = false;
        if let Some(socket) = &mut image {
            if socket.io.ready() && !socket.io.has_pending_writes() {
                images.in_flight = None;
                if let Some((latest, guard)) = candidate(&shared, authority).filter(|(_, guard)| images.pending(*guard)) {
                    image_failed = queue_image(&shared, &mut images, &latest, guard, socket.generation, |header, rgba| {
                        socket.io.queue(header, rgba)
                    })
                    .is_err();
                }
            }
            if !image_failed {
                let generation = socket.generation;
                let in_flight = images.in_flight;
                let current = || {
                    connection_current(&shared, authority, generation, Instant::now())
                        && in_flight.is_none_or(|guard| guard.current(&shared, Instant::now()))
                };
                match socket.io.poll_current(current) {
                    Ok(packets) => {
                        if !packets.is_empty() {
                            image_failed = true;
                        }
                        if !socket.io.has_pending_writes() {
                            images.in_flight = None;
                        }
                    }
                    Err(_) => image_failed = true,
                }
            }
        }
        if image_failed {
            image = None;
            images.disconnected();
            image_retry.failed(Instant::now());
        }
        shared.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::super::preview_protocol::{self as wire, Cohort, CohortReport, GpuSample, Peer, Probe, Subscription};
    use super::*;
    use aurora_session::{PreviewRate, PreviewSize};

    fn source() -> PreviewSource {
        wire::fixture_source(3)
    }

    fn shared() -> (Arc<Shared>, Authority) {
        let source = source();
        let mut current = Source::new(Scope {
            identity: source.worker,
            lease_epoch: source.lease_epoch,
        });
        current.set(Some(source), Some(Instant::now() + Duration::from_secs(3600)), Instant::now());
        let authority = current.authority(Instant::now()).expect("synthetic current authority");
        (
            Arc::new(Shared {
                source: Mutex::new(current),
                mailbox: Arc::new(PreviewMailbox::default()),
                wake: Condvar::new(),
                stop: AtomicBool::new(false),
            }),
            authority,
        )
    }

    fn subscription(source: PreviewSource, challenge: u64) -> Subscription {
        Subscription {
            source,
            revision: 1,
            size: PreviewSize::Small,
            rate: PreviewRate::Half,
            visible: true,
            challenge,
            budget: None,
        }
    }

    fn seed(shared: &Shared, authority: Authority, epoch: u64, sequence: u64) -> (Arc<Image>, ImageGuard) {
        let subscription = subscription(authority.source, epoch);
        assert!(shared.mailbox.receive_subscription(subscription, Instant::now()));
        assert!(shared.mailbox.publish_state(State {
            source: authority.source,
            subscription_revision: 1,
            epoch,
            availability: Availability::Live,
            challenge: epoch
        }));
        let header = ImageHeader {
            source: authority.source,
            subscription_revision: 1,
            epoch,
            sequence,
            size: PreviewSize::Small,
            challenge: epoch,
        };
        assert!(shared.mailbox.publish_image(header, vec![12; PreviewSize::Small.bytes() as usize]));
        candidate(shared, authority).expect("fresh exact source frame")
    }

    #[test]
    fn preview_lane_renewal_keeps_authority_but_rebind_invalidates_it() {
        let (shared, authority) = shared();
        let mut source = shared.source();
        let deadline = source.expires.expect("current deadline");
        assert_eq!(
            source.set(Some(authority.source), Some(deadline), Instant::now()),
            SourceUpdate::Unchanged
        );
        assert_eq!(
            source.set(Some(authority.source), Some(deadline + RETRY), Instant::now()),
            SourceUpdate::Renewed
        );
        assert!(source.authority(Instant::now()) == Some(authority));
        let mut next = authority.source;
        next.session.generation += 1;
        assert_eq!(source.set(Some(next), Some(deadline), Instant::now()), SourceUpdate::Changed);
        assert!(source.authority(Instant::now()) != Some(authority));
        assert_eq!(source.set(Some(next), None, Instant::now()), SourceUpdate::Changed);
        assert!(source.authority(Instant::now()).is_none());
    }

    #[test]
    fn preview_lane_source_change_and_drop_eagerly_purge_mailbox_without_joining() {
        let (shared, authority) = shared();
        let (_, guard) = seed(&shared, authority, 1, 1);
        let channel = WorkerChannel { shared: shared.clone() };
        let mut next = authority.source;
        next.session.generation += 1;
        channel.set_source(Some(next), Some(Instant::now() + Duration::from_secs(3600)));
        assert!(!guard.current(&shared, Instant::now()));
        assert!(shared.mailbox.control(Instant::now()).is_none());
        assert!(shared.mailbox.image().is_none());
        let generation = shared.mailbox.generation();
        drop(channel);
        assert!(shared.stop.load(Ordering::Acquire));
        assert_ne!(shared.mailbox.generation(), generation);
    }

    #[test]
    fn preview_lane_expiry_purges_even_when_no_application_tick_arrives() {
        let (shared, authority) = shared();
        let (_, guard) = seed(&shared, authority, 1, 1);
        let generation = shared.mailbox.generation();
        let mut source = shared.source();
        let deadline = source.expires.expect("receipt deadline");
        shared.expire(&mut source, deadline);
        assert!(source.authority(deadline).is_none());
        assert_ne!(shared.mailbox.generation(), generation);
        assert!(shared.mailbox.control(deadline).is_none());
        drop(source);
        assert!(!guard.current(&shared, Instant::now()));
        assert!(shared.mailbox.image().is_none());
    }

    #[test]
    fn preview_lane_filters_exact_source_before_subscription_or_probe_counters() {
        let (shared, authority) = shared();
        let now = Instant::now();
        let mut foreign = authority.source;
        foreign.session.generation += 1;
        shared.incoming(
            authority,
            Packet {
                header: Header::Probe(Probe {
                    source: foreign,
                    challenge: u64::MAX,
                }),
                rgba: Vec::new(),
            },
            now,
        );
        assert!(shared.mailbox.latest_probe(authority.source, now).is_none());
        shared.incoming(
            authority,
            Packet {
                header: Header::Probe(Probe {
                    source: authority.source,
                    challenge: 1,
                }),
                rgba: Vec::new(),
            },
            now,
        );
        assert_eq!(
            shared
                .mailbox
                .latest_probe(authority.source, now)
                .expect("valid lower challenge")
                .challenge,
            1
        );
        let mut invalid = subscription(foreign, u64::MAX);
        invalid.revision = u64::MAX;
        shared.incoming(
            authority,
            Packet {
                header: Header::Subscribe(invalid),
                rgba: Vec::new(),
            },
            now,
        );
        shared.incoming(
            authority,
            Packet {
                header: Header::Subscribe(subscription(authority.source, 1)),
                rgba: Vec::new(),
            },
            now,
        );
        assert_eq!(
            shared
                .mailbox
                .control(now)
                .expect("valid subscription not poisoned")
                .subscription
                .revision,
            1
        );
    }

    #[test]
    fn preview_lane_image_guards_recheck_generation_deadline_and_connection_after_encoding() {
        let (shared, authority) = shared();
        let (image, guard) = seed(&shared, authority, 1, 1);
        let mut images = Images::default();
        let mut encoded = false;
        let result = queue_image(&shared, &mut images, &image, guard, guard.generation, |_, _| {
            encoded = true;
            shared.mailbox.purge_pixels();
            Ok(())
        });
        assert!(encoded);
        assert_eq!(
            result,
            Err(Error::Closed),
            "privacy invalidation during encoding must drop the socket"
        );
        assert!(!guard.current(&shared, Instant::now()));

        let (image, guard) = seed(&shared, authority, 2, 2);
        let mut encoded = false;
        assert_eq!(
            queue_image(&shared, &mut images, &image, guard, guard.generation - 1, |_, _| {
                encoded = true;
                Ok(())
            }),
            Err(Error::Closed)
        );
        assert!(!encoded, "a new generation requires a new image socket");
        let expired = ImageGuard {
            valid_until: Instant::now(),
            ..guard
        };
        assert_eq!(
            queue_image(&shared, &mut images, &image, expired, expired.generation, |_, _| {
                encoded = true;
                Ok(())
            }),
            Err(Error::Closed)
        );
        assert!(!encoded, "an expired frame is rejected before encoding");
    }

    #[test]
    fn preview_lane_control_state_purge_after_enqueue_rejects_every_later_write() {
        let (shared, authority) = shared();
        let (_, guard) = seed(&shared, authority, 1, 1);
        let mut state = ControlState::default();
        state.queued(shared.mailbox.state().expect("queued Live state"), guard.generation);
        let generation = state.generation;
        assert!(control_current(&shared, authority, generation));
        // Encoding/enqueue completed, then the producer invalidated privacy
        // before poll_current reached its first or any later bounded write.
        shared.mailbox.purge_pixels();
        assert!(!control_current(&shared, authority, generation));
        assert!(!control_current(&shared, authority, generation));
        assert!(
            !state.accepts_generation(shared.mailbox.generation()),
            "a newer state cannot relabel an older partial frame"
        );
        state.drained();
        assert!(
            control_current(&shared, authority, state.generation),
            "metadata-only control remains independent of old pixels"
        );
    }

    #[test]
    fn preview_lane_source_expiry_during_encoding_prevents_any_image_write() {
        let (shared, authority) = shared();
        let (image, guard) = seed(&shared, authority, 1, 1);
        let mut images = Images::default();
        assert_eq!(
            queue_image(&shared, &mut images, &image, guard, guard.generation, |_, _| {
                shared.source().expires = Some(Instant::now());
                Ok(())
            }),
            Err(Error::Closed)
        );
        assert!(shared.mailbox.control(Instant::now()).is_none());
        assert!(shared.mailbox.image().is_none());
    }

    #[test]
    fn preview_lane_busy_and_disconnect_keep_only_latest_image_without_replay() {
        let (shared, authority) = shared();
        let (image, guard) = seed(&shared, authority, 1, 1);
        let mut images = Images::default();
        queue_image(&shared, &mut images, &image, guard, guard.generation, |_, _| Err(Error::Busy)).expect("backpressure");
        assert!(images.last.is_none() && images.in_flight.is_none());
        assert!(images.pending(guard));
        queue_image(&shared, &mut images, &image, guard, guard.generation, |_, _| Ok(())).expect("accepted frame");
        assert!(images.in_flight.is_some());
        images.disconnected();
        assert!(images.in_flight.is_none());
        assert!(!images.pending(guard), "an uncertain frame is not replayed on reconnect");
        let newer = ImageGuard {
            header: ImageHeader {
                sequence: 2,
                ..guard.header
            },
            ..guard
        };
        assert!(images.pending(newer));
        assert!(
            shared.mailbox.control(Instant::now()).is_some(),
            "image failure retains the control subscription"
        );
    }

    #[test]
    fn preview_lane_peer_precedes_sample_and_stale_reports_drop_before_encoding() {
        let (shared, authority) = shared();
        let now = Instant::now();
        let peer = Peer {
            cohort: Cohort {
                source: authority.source,
                device_epoch: 1,
                pid: 200,
            },
            luid: 42,
        };
        let probe = Probe {
            source: authority.source,
            challenge: 9,
        };
        assert!(shared.mailbox.receive_probe(probe, now));
        shared.mailbox.publish_peer(CohortReport {
            peer,
            challenge: probe.challenge,
        });
        shared.mailbox.publish_sample(GpuSample {
            peer,
            challenge: probe.challenge,
            surface_epoch: 2,
            frame: 3,
            gpu_micros: 100,
        });
        shared.mailbox.request_budget(authority.source, now);
        let reports = shared.reports(authority, now);
        assert!(matches!(reports.as_slice(), [Header::Peer(_), Header::Sample(_), Header::Probe(_)]));
        shared.mailbox.publish_peer(CohortReport {
            peer,
            challenge: probe.challenge,
        });
        shared.mailbox.publish_sample(GpuSample {
            peer,
            challenge: probe.challenge,
            surface_epoch: 2,
            frame: 4,
            gpu_micros: 100,
        });
        assert!(
            shared
                .reports(authority, now + wire::SAMPLE_FRESH + Duration::from_millis(1))
                .is_empty()
        );
    }

    #[test]
    fn preview_lane_retry_is_per_endpoint_and_connect_cannot_cross_nearby_receipt_expiry() {
        let (shared, authority) = shared();
        let now = Instant::now();
        let mut control = Retry::new(now);
        let mut image = Retry::new(now);
        control.attempted(now);
        assert!(!control.due(now + RETRY - Duration::from_millis(1)));
        assert!(image.due(now));
        image.failed(now + Duration::from_millis(250));
        assert!(!image.due(now + RETRY));
        assert!(image.due(now + RETRY + Duration::from_millis(250)));
        shared.source().expires = Some(now + HANDSHAKE_DEADLINE);
        assert!(!shared.can_connect(authority, now));
        shared.source().expires = Some(now + HANDSHAKE_DEADLINE + Duration::from_millis(1));
        assert!(shared.can_connect(authority, now));
    }

    #[test]
    fn preview_lane_invalid_scope_and_counter_exhaustion_disable_optional_io() {
        let (shared, authority) = shared();
        let channel = WorkerChannel { shared: shared.clone() };
        let mut foreign = authority.source;
        foreign.lease_epoch += 1;
        channel.set_source(Some(foreign), Some(Instant::now() + RETRY));
        assert!(shared.authority().is_none());
        shared.source().revision = u64::MAX;
        channel.set_source(Some(authority.source), Some(Instant::now() + RETRY));
        assert!(shared.stop.load(Ordering::Acquire));
        assert!(shared.authority().is_none());
    }

    #[test]
    fn preview_lane_same_call_expiry_and_replacement_cannot_reopen_exhausted_authority() {
        let source = source();
        let now = Instant::now();
        let mut current = Source {
            scope: Scope {
                identity: source.worker,
                lease_epoch: source.lease_epoch,
            },
            source: Some(source),
            expires: Some(now),
            revision: u64::MAX,
        };
        assert_eq!(current.set(Some(source), Some(now + RETRY), now), SourceUpdate::Changed);
        assert_eq!(current.revision, 0);
        assert!(current.authority(now).is_none());
        current.set(Some(source), Some(now + RETRY), now);
        assert_eq!(current.revision, 0);
        assert!(current.source.is_none() && current.expires.is_none());
    }
}
