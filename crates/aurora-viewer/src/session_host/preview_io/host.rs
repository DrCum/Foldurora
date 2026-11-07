//! One host listener/poller, at most five exact-source pairs. No scheduling,
//! native authority derivation or final SourceImages admission occurs here.
use super::handshake::HostPending;
use super::service::Shared;
use super::state::*;
use super::*;
use std::io::{ErrorKind, Read};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::{Arc, atomic::Ordering};

struct Pending {
    io: HostPending,
    scope: Option<Scope>,
    registration: u64,
    control_witness: Option<EpochFence>,
    policy: Option<PreviewPolicy>,
}
enum Suspension {
    AwaitStop(Instant),
    AwaitEof { began: Instant, stop: u64, ack_sequence: u64 },
}
struct Source {
    permit: HostPermit,
    pair: Pair,
    metadata: TcpStream,
    pixels: Option<TcpStream>,
    observation: Observation,
    capture: EpochFence,
    frame: Option<FrameHandoff>,
    suspension: Option<Suspension>,
    last_pixel: u64,
    last_stop: u64,
    last_stop_policy: Option<PreviewPolicy>,
    pending_capture: Option<(CaptureClaim, CaptureControlGuard, Instant)>,
}
impl Drop for Source {
    fn drop(&mut self) {
        self.observation.retention.revoke();
        self.capture.revoke();
    }
}
impl Source {
    fn metadata(&mut self, m: Metadata, now: Instant) -> Result<(), Error> {
        match m {
            Metadata::Policy { policy, .. } => {
                if !policy.access().retain_last_active {
                    return Err(Error::Expired);
                }
                if self.observation.policy.is_some_and(|p| p.epochs() != policy.epochs()) {
                    return Err(Error::Expired);
                }
                if self.pixels.is_some() && (!policy.access().capture_active || !self.pair.has_pixels()) && self.suspension.is_none() {
                    self.capture.revoke();
                    self.pending_capture = None;
                    self.frame = None;
                    self.suspension = Some(Suspension::AwaitStop(now));
                }
                if self.observation.layout.is_some_and(|layout| !layout.matches_policy(policy)) {
                    self.observation.layout = None;
                }
                self.observation.policy = Some(policy);
                self.observation.policy_received_at = Some(now);
            }
            Metadata::Layout { layout, .. } => {
                if self.observation.policy.is_none_or(|p| !layout.matches_policy(p)) {
                    return Err(Error::Binding);
                }
                self.observation.layout = Some(layout);
            }
            Metadata::Invalidate { .. } => return Err(Error::Expired),
            Metadata::CaptureStopped {
                pixel_epoch, stop, policy, ..
            } => {
                if pixel_epoch < self.last_pixel {
                    return Ok(());
                }
                if pixel_epoch != self.last_pixel || stop < self.last_stop {
                    return Err(Error::Binding);
                }
                if stop == self.last_stop {
                    return if self.last_stop_policy == Some(policy) {
                        Ok(())
                    } else {
                        Err(Error::Sequence)
                    };
                }
                if self.pixels.is_none() {
                    return Err(Error::Binding);
                }
                if !policy.access().retain_last_active
                    || policy.access().capture_active
                    || self.observation.policy.is_some_and(|p| p.epochs() != policy.epochs())
                {
                    return Err(Error::Binding);
                }
                self.capture.revoke();
                self.pending_capture = None;
                self.frame = None;
                self.pair.suspend_pixels();
                self.last_stop = stop;
                self.last_stop_policy = Some(policy);
                self.observation.policy = Some(policy);
                self.observation.policy_received_at = Some(now);
                self.pair.queue_metadata(Metadata::CaptureStopAck {
                    source: self.permit.source,
                    connection: self.observation.token.connection,
                    pixel_epoch,
                    stop,
                })?;
                self.suspension = Some(Suspension::AwaitEof {
                    began: now,
                    stop,
                    ack_sequence: self.pair.metadata_queued(),
                });
                self.observation.status = Status::CaptureStopped;
            }
            Metadata::Stopped { demand, .. } => self.observation.stopped_demand = Some(demand),
            Metadata::Unavailable { claim: outcome, reason } => {
                if self.pending_capture.as_ref().is_some_and(|(claim, _, _)| *claim == outcome) {
                    self.pending_capture = None;
                }
                self.observation.unavailable = Some((outcome, reason));
            }
            _ => return Err(Error::Direction),
        }
        Ok(())
    }
    fn suspended(&mut self, _now: Instant, tick: &mut TickBudget, max_bytes: usize) -> Result<(), Error> {
        let Some(suspension) = &self.suspension else { return Ok(()) };
        let began = match suspension {
            Suspension::AwaitStop(t) => *t,
            Suspension::AwaitEof { began, .. } => *began,
        };
        if Instant::now().saturating_duration_since(began) >= DEADLINE {
            return Err(Error::Expired);
        }
        let Some(stream) = self.pixels.as_mut() else {
            return Err(Error::Binding);
        };
        let (mut bytes, mut operations) = tick.external(max_bytes.min(64 * 1024), 16);
        let mut buffer = [0; 4096];
        while bytes > 0 && operations > 0 {
            if Instant::now().saturating_duration_since(began) >= DEADLINE {
                return Err(Error::Expired);
            }
            operations -= 1;
            // Before the authenticated Stop, consuming old pixel bytes could
            // hide an ambiguous EOF. Peek only; zero is always an early fault.
            let result = if matches!(suspension, Suspension::AwaitStop(_)) {
                stream.peek(&mut buffer[..1])
            } else {
                {
                    let length = bytes.min(buffer.len());
                    stream.read(&mut buffer[..length])
                }
            };
            if Instant::now().saturating_duration_since(began) >= DEADLINE {
                return Err(Error::Expired);
            }
            match result {
                Ok(0) => {
                    if let Suspension::AwaitEof { ack_sequence, .. } = suspension {
                        if self.pair.metadata_flushed() < *ack_sequence {
                            return Err(Error::Closed);
                        }
                        self.pixels = None;
                        self.suspension = None;
                        return Ok(());
                    }
                    return Err(Error::Closed);
                }
                Ok(n) => {
                    bytes -= n;
                    if matches!(suspension, Suspension::AwaitStop(_)) {
                        break;
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(Error::Io(e.kind())),
            }
        }
        Ok(())
    }
    fn metadata_poll(&mut self, now: Instant, clock: u64, tick: &mut TickBudget) -> Result<(), Error> {
        if !self.permit.current(now) || !self.observation.retention.current() {
            return Err(Error::Expired);
        }
        if self
            .pending_capture
            .as_ref()
            .is_some_and(|(claim, guard, sent)| sent.elapsed() >= DEADLINE || !guard.current(*claim, Instant::now()))
        {
            if let Some((claim, _, _)) = self.pending_capture.take() {
                self.observation.unavailable = Some((claim, Unavailable::Expired));
            }
        }
        self.pair.set_source_guard(super::guards::IoSourceGuard {
            control: self.permit.control.clone(),
            retention: self.observation.retention.clone(),
            emergency: self.observation.emergency.clone(),
            native: None,
            policy_expires: self.observation.policy_received_at.and_then(|at| at.checked_add(DEADLINE)),
        })?;
        for packet in self.pair.poll_roles::<TcpStream>(&mut self.metadata, None, clock, tick)? {
            if let Received::Metadata(m) = packet {
                self.metadata(m, now)?;
            } else {
                return Err(Error::Direction);
            }
        }
        Ok(())
    }
    fn pixel_poll(&mut self, now: Instant, clock: u64, tick: &mut TickBudget, max_bytes: usize) -> Result<(), Error> {
        self.pair.set_source_guard(super::guards::IoSourceGuard {
            control: self.permit.control.clone(),
            retention: self.observation.retention.clone(),
            emergency: self.observation.emergency.clone(),
            native: None,
            policy_expires: self.observation.policy_received_at.and_then(|at| at.checked_add(DEADLINE)),
        })?;
        if self.suspension.is_some() {
            return self.suspended(now, tick, max_bytes);
        }
        let Some(stream) = &mut self.pixels else { return Ok(()) };
        for packet in self.pair.poll_pixels_only(stream, clock, tick, max_bytes)? {
            if let Received::UnadmittedPixels(pixels) = packet {
                if !self.capture.current() {
                    return Err(Error::Expired);
                }
                let claim = pixels.claim();
                if self.pending_capture.as_ref().is_none_or(|(expected, guard, sent)| {
                    *expected != claim || !guard.current(claim, Instant::now()) || sent.elapsed() >= DEADLINE
                }) {
                    continue;
                }
                let (_, original_capture, _) = self.pending_capture.take().ok_or(Error::Sequence)?;
                self.frame = Some(FrameHandoff {
                    original_capture,
                    token: self.observation.token,
                    received_at: now,
                    pixels,
                    retention: self.observation.retention.clone(),
                    capture: self.capture.clone(),
                    emergency: self.observation.emergency.clone(),
                });
            } else {
                return Err(Error::Direction);
            }
        }
        Ok(())
    }
}
fn publish(source: &mut Source, shared: &Shared<HostMailbox>) {
    let mut m = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(old) = m.updates.iter_mut().find(|u| u.observation.token.source == source.permit.source) {
        if old.observation.token != source.observation.token || !source.capture.current() {
            old.frame = None;
        }
        old.observation = source.observation.clone();
        if source.frame.is_some() {
            old.frame = source.frame.take();
        }
    } else if m.updates.len() < MAX_WORKERS {
        m.updates.push(SourceUpdate {
            observation: source.observation.clone(),
            frame: source.frame.take(),
        });
    }
}
fn fail(source: &mut Source, shared: &Shared<HostMailbox>) {
    source.observation.retention.revoke();
    source.capture.revoke();
    source.frame = None;
    source.pending_capture = None;
    source.pair.close();
    source.observation.status = Status::Fault;
    publish(source, shared);
}

pub(super) fn run(installation: InstallationId, host: HostId, shared: Arc<Shared<HostMailbox>>, budget: ImageBudget) {
    struct End(Arc<Shared<HostMailbox>>);
    impl Drop for End {
        fn drop(&mut self) {
            self.0.emergency.store(true, Ordering::Release);
            self.0.stop.store(true, Ordering::Release);
            self.0.wake.notify_all();
        }
    }
    let _end = End(shared.clone());
    let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).and_then(|l| {
        l.set_nonblocking(true)?;
        Ok(l)
    }) {
        Ok(l) => l,
        Err(_) => return,
    };
    let port = match listener.local_addr() {
        Ok(a) => a.port(),
        Err(_) => return,
    };
    shared.port.store(port, Ordering::Release);
    let mut sources: Vec<Source> = Vec::with_capacity(MAX_WORKERS);
    let mut pending: Vec<Pending> = Vec::with_capacity(2 * MAX_WORKERS);
    let mut next_connection = 1u64;
    let mut rotation = 0usize;
    while !shared.stop.load(Ordering::Acquire) {
        let now = Instant::now();
        let clock = shared.clock(now);
        if shared.emergency.load(Ordering::Acquire) {
            let consumed = shared.retirement_epoch.load(Ordering::Acquire);
            for s in &mut sources {
                fail(s, &shared);
            }
            sources.clear();
            pending.clear();
            let mut m = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            m.commands.clear();
            m.updates.clear();
            if shared.publication_failed.swap(false, Ordering::AcqRel) {
                m.permits.clear();
            }
            shared.retired(consumed);
        }
        let (permits, commands) = {
            let mut m = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            (m.permits.clone(), m.commands.drain(..).collect::<Vec<_>>())
        };
        sources.retain_mut(|s| {
            let current = permits.iter().find(|p| {
                p.source == s.permit.source
                    && p.registration == s.permit.registration
                    && p.control.witness.same(&s.permit.control.witness)
                    && p.current(now)
            });
            if let Some(current) = current.filter(|_| s.permit.current(now)) {
                s.permit = current.clone();
                true
            } else {
                fail(s, &shared);
                false
            }
        });
        {
            let mut m = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            m.updates
                .retain(|u| permits.iter().any(|p| p.source == u.observation.token.source && p.current(now)));
        }
        for command in commands {
            if let Some(s) = sources.iter_mut().find(|s| s.observation.token == command.token) {
                let valid = match &command.command {
                    PreviewCommand::Capture(c) => {
                        s.capture.current()
                            && s.pending_capture.is_none()
                            && command.capture.as_ref().is_some_and(|g| g.current(**c, Instant::now()))
                            && s.observation.policy.is_some_and(|p| c.matches_policy(p))
                            && s.pair.has_pixels()
                    }
                    _ => true,
                };
                if valid {
                    if matches!(command.command, PreviewCommand::Stop { .. }) {
                        s.capture.revoke();
                        s.pending_capture = None;
                        s.frame = None;
                        s.pair.suspend_pixels();
                        if s.pixels.is_some() && s.suspension.is_none() {
                            s.suspension = Some(Suspension::AwaitStop(now));
                        }
                    }
                    if s.pair.queue_metadata(command.command.metadata(command.token)).is_err() {
                        fail(s, &shared);
                    } else if let (PreviewCommand::Capture(claim), Some(guard)) = (command.command, command.capture) {
                        s.pending_capture = Some((*claim, guard, Instant::now()));
                    }
                }
            }
        }
        let mut tick = TickBudget::new();
        // Reserve a fixed4KiB/44op slice for listener/auth progress. Role data
        // still shares the remaining global256KiB budget and rotating order.
        let _auth = tick.external(4096, 44);
        let count = sources.len();
        let mut metadata_bytes = [0usize; MAX_WORKERS];
        // Every source's metadata/revocation is consumed before any pixels.
        for offset in 0..count {
            let index = (rotation + offset) % count;
            let before = tick.remaining_bytes();
            if sources[index].observation.status == Status::Fault {
                continue;
            }
            if sources[index].metadata_poll(now, clock, &mut tick).is_err() {
                fail(&mut sources[index], &shared);
            } else {
                publish(&mut sources[index], &shared);
            }
            metadata_bytes[index] = before.saturating_sub(tick.remaining_bytes());
        }
        for offset in 0..count {
            let index = (rotation + offset) % count;
            if sources[index].observation.status == Status::Fault {
                continue;
            }
            let remaining = (64 * 1024usize).saturating_sub(metadata_bytes[index]);
            if sources[index].pixel_poll(now, clock, &mut tick, remaining).is_err() {
                fail(&mut sources[index], &shared);
            } else {
                publish(&mut sources[index], &shared);
            }
        }
        sources.retain(|s| s.observation.status != Status::Fault);
        if count > 0 {
            rotation = (rotation + 1) % count;
        }
        for _ in 0..4 {
            match listener.accept() {
                Ok((stream, address)) => {
                    if pending.len() >= 2 * MAX_WORKERS || !address.ip().is_loopback() {
                        continue;
                    }
                    if stream.set_nonblocking(true).and_then(|_| stream.set_nodelay(true)).is_err() {
                        continue;
                    }
                    pending.push(Pending {
                        io: HostPending::new(stream, now),
                        scope: None,
                        registration: 0,
                        control_witness: None,
                        policy: None,
                    });
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => return,
            }
        }
        let mut index = 0;
        while index < pending.len() {
            let result = pending[index].io.poll(now, clock);
            if result.is_err() {
                pending.swap_remove(index);
                continue;
            }
            if let Some(auth) = result.ok().flatten() {
                let p = pending.swap_remove(index);
                let scope = auth.scope();
                let permit = {
                    let m = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if shared.emergency.load(Ordering::Acquire) || !p.io.fresh() {
                        None
                    } else {
                        m.permits
                            .iter()
                            .find(|permit| {
                                permit.source == scope.source
                                    && permit.registration == p.registration
                                    && p.control_witness.as_ref().is_some_and(|old| old.same(&permit.control.witness))
                                    && permit.current(Instant::now())
                            })
                            .cloned()
                    }
                };
                let Some(permit) = permit else { continue };
                if p.io.permit_revision != permit.registration {
                    continue;
                }
                if scope.lane == Lane::Metadata {
                    if sources.iter().any(|s| s.permit.source.worker() == scope.source.worker()) || sources.len() >= MAX_WORKERS {
                        continue;
                    }
                    let pair = match Pair::metadata(auth, Some(budget.clone())) {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    let token = ConnectionToken {
                        source: scope.source,
                        connection: scope.connection,
                        pixel_epoch: 0,
                        incarnation: shared.incarnation,
                    };
                    let capture = EpochFence::new();
                    sources.push(Source {
                        permit,
                        pair,
                        metadata: p.io.stream,
                        pixels: None,
                        observation: Observation {
                            token,
                            policy: None,
                            policy_received_at: None,
                            layout: None,
                            status: Status::Metadata,
                            stopped_demand: None,
                            unavailable: None,
                            retention: EpochFence::new(),
                            capture: capture.clone(),
                            emergency: shared.emergency.clone(),
                        },
                        capture,
                        frame: None,
                        suspension: None,
                        last_pixel: 0,
                        last_stop: 0,
                        last_stop_policy: None,
                        pending_capture: None,
                    });
                } else if let Some(s) = sources
                    .iter_mut()
                    .find(|s| s.permit.source == scope.source && s.observation.token.connection == scope.connection)
                {
                    let current = s.observation.policy;
                    let continuous = p
                        .policy
                        .zip(current)
                        .is_some_and(|(old, new)| old.epochs() == new.epochs() && new.access().retain_last_active);
                    let retiring = p.policy != current || current.is_none_or(|p| !p.access().capture_active);
                    if continuous && s.pixels.is_none() && s.suspension.is_none() {
                        let attached = if retiring {
                            s.pair.attach_retiring(auth)
                        } else {
                            s.pair.attach_pixels(auth)
                        };
                        if attached.is_ok() {
                            s.pixels = Some(p.io.stream);
                            s.last_pixel = scope.role_epoch;
                            s.capture = EpochFence::new();
                            s.observation.capture = s.capture.clone();
                            s.observation.token.pixel_epoch = scope.role_epoch;
                            if retiring {
                                s.capture.revoke();
                                s.pair.suspend_pixels();
                                s.suspension = Some(Suspension::AwaitStop(Instant::now()));
                                s.observation.status = Status::CaptureStopped;
                            } else {
                                s.observation.status = Status::Pixels;
                            }
                        }
                    }
                }
                continue;
            }
            if let Some(hello) = pending[index].io.hello() {
                #[cfg(test)]
                if hello.lane() == Lane::Pixels && shared.hold_pixel_challenge.load(Ordering::Acquire) {
                    shared.pixel_hello_waiting.store(true, Ordering::Release);
                    index += 1;
                    continue;
                }
                let permit = {
                    let m = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if shared.emergency.load(Ordering::Acquire) || !pending[index].io.fresh() {
                        None
                    } else {
                        m.permits
                            .iter()
                            .find(|p| {
                                p.source.worker() == hello.identity()
                                    && p.source.lease_epoch() == hello.lease_epoch()
                                    && p.current(Instant::now())
                            })
                            .cloned()
                    }
                };
                let Some(permit) = permit else {
                    pending.swap_remove(index);
                    continue;
                };
                if hello.identity().installation != installation || hello.identity().host != host {
                    pending.swap_remove(index);
                    continue;
                }
                let duplicate = pending.iter().enumerate().any(|(other, p)| {
                    other != index
                        && p.scope
                            .is_some_and(|s| s.source.worker() == hello.identity() && s.lane == hello.lane())
                });
                let scope = if hello.lane() == Lane::Metadata {
                    if duplicate || sources.iter().any(|s| s.permit.source.worker() == hello.identity()) {
                        pending.swap_remove(index);
                        continue;
                    }
                    let connection = next_connection;
                    let Some(next) = connection.checked_add(1) else { return };
                    next_connection = next;
                    Scope {
                        source: permit.source,
                        connection,
                        role_epoch: 1,
                        lane: Lane::Metadata,
                    }
                } else {
                    let Some(s) = sources.iter_mut().find(|s| {
                        s.permit.source == permit.source
                            && s.permit.registration == permit.registration
                            && s.permit.control.witness.same(&permit.control.witness)
                            && s.observation.retention.current()
                            && s.observation.token.connection == hello.connection()
                    }) else {
                        pending.swap_remove(index);
                        continue;
                    };
                    // A role started just before capture loss may reach Hello
                    // after the retained/capture=false metadata. Authenticate
                    // it only to finish exact retirement, never to publish pixels.
                    if duplicate
                        || s.pixels.is_some()
                        || s.suspension.is_some()
                        || s.observation.policy.is_none_or(|p| !p.access().retain_last_active)
                    {
                        pending.swap_remove(index);
                        continue;
                    }
                    let Some(epoch) = s.last_pixel.checked_add(1) else { return };
                    s.last_pixel = epoch;
                    Scope {
                        source: permit.source,
                        connection: hello.connection(),
                        role_epoch: epoch,
                        lane: Lane::Pixels,
                    }
                };
                let p = &mut pending[index];
                p.scope = Some(scope);
                p.registration = permit.registration;
                p.control_witness = Some(permit.control.witness.clone());
                p.policy = if scope.lane == Lane::Pixels {
                    sources
                        .iter()
                        .find(|s| s.permit.source == scope.source && s.observation.token.connection == scope.connection)
                        .and_then(|s| s.observation.policy)
                } else {
                    None
                };
                if p.io.challenge(scope, permit.secret.clone(), permit.registration, clock).is_err() {
                    pending.swap_remove(index);
                    continue;
                }
            }
            index += 1;
        }
        {
            let mut m = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            m.diagnostics = [
                pending.len(),
                sources.iter().map(|s| 1 + usize::from(s.pixels.is_some())).sum(),
                budget.used(),
            ];
        }
        shared.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::preview_image::{PreviewAccess, PreviewEpochs, SourceImages};
    use lane::{HostHandshake, WorkerHandshake};
    #[test]
    fn preview_io_unavailable_old_request_one_context_never_clears_fresh_owner_only_exact_claim_may() {
        let source = super::super::tests::source(1);
        let control = super::super::tests::control(source);
        let base = super::super::tests::state(control.clone(), Some(EpochFence::new())).policy;
        let geometry = aurora_session::preview_image::PreviewGeometry::new([640, 360], [0, 0, 640, 360], PreviewSize::Small).unwrap();
        let make = |policy: PreviewPolicy, demand| {
            let mut model = SourceImages::new(source, ImageBudget::new(1024 * 1024).unwrap()).unwrap();
            model.sync(policy, 0).unwrap();
            model.request(8, demand, geometry, 1).unwrap()
        };
        let fresh = make(base, 2);
        let old_instance = make(base, 2);
        let old_demand = make(base, 1);
        let old_policy = make(
            PreviewPolicy::new(source, PreviewEpochs::new(2, 1, 1, 1, 1).unwrap(), base.access()).unwrap(),
            2,
        );
        let old_activation = make(
            PreviewPolicy::new(
                source,
                base.epochs(),
                PreviewAccess {
                    activation: 2,
                    ..base.access()
                },
            )
            .unwrap(),
            2,
        );
        let key = PreviewSecret::synthetic([7; 32]).unwrap();
        let scope = Scope {
            source,
            connection: 8,
            role_epoch: 1,
            lane: Lane::Metadata,
        };
        let mut host = HostHandshake::new(scope, key.clone(), 0).unwrap();
        assert_eq!(
            &host.challenge().as_slice()[4..6],
            &2u16.to_be_bytes(),
            "auth schema remains V2 while image data is V3"
        );
        let mut worker = WorkerHandshake::new(scope, key.clone(), 0).unwrap();
        let proof = worker.answer(host.challenge().as_slice(), 1).unwrap();
        let (_, auth) = host.accept(proof.as_slice(), 2).unwrap();
        let pair = Pair::metadata(auth, Some(ImageBudget::new(1024 * 1024).unwrap())).unwrap();
        // Owned loopback descriptors are fixture plumbing; the state handler
        // itself is pure and neither reads nor writes either socket here.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (metadata, _) = listener.accept().unwrap();
        let emergency = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let capture = EpochFence::new();
        let now = Instant::now();
        let token = ConnectionToken {
            source,
            connection: 8,
            pixel_epoch: 1,
            incarnation: 1,
        };
        let proof = CaptureControlGuard {
            control: control.clone(),
            activation: 1,
            viewport: aurora_session::HostedViewport {
                host_window: 1,
                rect: aurora_session::PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 640,
                    height: 360,
                },
            },
            witness: EpochFence::new(),
        };
        let mut state = Source {
            permit: HostPermit {
                source,
                secret: key,
                control,
                registration: 1,
            },
            pair,
            metadata,
            pixels: None,
            observation: Observation {
                token,
                policy: Some(base),
                policy_received_at: Some(now),
                layout: None,
                status: Status::Metadata,
                stopped_demand: None,
                unavailable: None,
                retention: EpochFence::new(),
                capture: capture.clone(),
                emergency,
            },
            capture,
            frame: None,
            suspension: None,
            last_pixel: 1,
            last_stop: 0,
            last_stop_policy: None,
            pending_capture: Some((fresh, proof, now)),
        };
        for old in [old_instance, old_demand, old_policy, old_activation] {
            assert_eq!(old.request(), fresh.request());
            assert_ne!(old, fresh);
            let message = Metadata::Unavailable {
                claim: old,
                reason: Unavailable::Busy,
            };
            let encoded = aurora_session::preview_wire::encode_metadata(message).unwrap();
            let header = aurora_session::preview_wire::Header::new(
                Lane::Metadata,
                aurora_session::preview_wire::Direction::WorkerToHost,
                8,
                1,
                1,
                encoded.as_slice().len(),
                0,
            )
            .unwrap();
            let decoded = aurora_session::preview_wire::decode_metadata(header, encoded.as_slice()).unwrap();
            state.metadata(decoded, now).unwrap();
            assert_eq!(state.pending_capture.as_ref().unwrap().0, fresh);
            assert_eq!(state.observation.unavailable, Some((old, Unavailable::Busy)));
        }
        state
            .metadata(
                Metadata::Unavailable {
                    claim: fresh,
                    reason: Unavailable::Busy,
                },
                now,
            )
            .unwrap();
        assert!(state.pending_capture.is_none());
        drop(client);
    }
}
