//! One worker socket owner. Retention survives an authenticated normal capture
//! stop; all unknown role loss faults the complete image source. No App/native
//! API or renderer operation is called here.
use super::handshake::WorkerPending;
use super::service::Shared;
use super::state::*;
use super::*;
use std::io::ErrorKind;
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, atomic::Ordering};

struct Stopping {
    stop: u64,
    pixel_epoch: u64,
    began: Instant,
    barrier: u64,
}
struct Connected {
    pair: Pair,
    metadata: TcpStream,
    pixels: Option<TcpStream>,
    pending: Option<WorkerPending>,
    pending_capture: Option<EpochFence>,
    source_control: EpochFence,
    token: ConnectionToken,
    stopping: Option<Stopping>,
    next_stop: u64,
    last_ack: Option<(u64, u64)>,
    heartbeat: Instant,
    sent_policy: Option<PreviewPolicy>,
    sent_layout: Option<PreviewLayout>,
    capture_scope: Option<EpochFence>,
    fenced_capture: Option<EpochFence>,
    capture_gate: EpochFence,
    retention_gate: EpochFence,
}
impl Drop for Connected {
    fn drop(&mut self) {
        self.capture_gate.revoke();
        self.retention_gate.revoke();
    }
}
impl Connected {
    fn begin_stop(&mut self, policy: PreviewPolicy, now: Instant, barrier: u64) -> Result<(), Error> {
        self.capture_gate.revoke();
        self.pair.suspend_pixels();
        if let Some(scope) = self.capture_scope.take() {
            self.fenced_capture = Some(scope);
        }
        if self.stopping.is_some() || self.pixels.is_none() {
            return Ok(());
        }
        let stop = self.next_stop;
        self.next_stop = stop.checked_add(1).ok_or(Error::Sequence)?;
        let stopped = PreviewPolicy::new(
            policy.source(),
            policy.epochs(),
            aurora_session::preview_image::PreviewAccess {
                capture_active: false,
                ..policy.access()
            },
        )
        .map_err(|_| Error::Binding)?;
        self.pair.queue_metadata(Metadata::CaptureStopped {
            source: self.token.source,
            connection: self.token.connection,
            pixel_epoch: self.token.pixel_epoch,
            stop,
            policy: stopped,
        })?;
        self.sent_policy = Some(stopped);
        self.stopping = Some(Stopping {
            stop,
            pixel_epoch: self.token.pixel_epoch,
            began: now,
            barrier,
        });
        Ok(())
    }
    fn poll(
        &mut self,
        state: &WorkerState,
        port: u16,
        key: &PreviewSecret,
        now: Instant,
        clock: u64,
        shared: &Shared<WorkerMailbox>,
    ) -> Result<(), Error> {
        if !state.current(now) {
            return Err(Error::Expired);
        }
        let capture_barrier = shared.capture_epoch.load(Ordering::Acquire);
        let capture = state.capture_current(now)
            && !shared.capture_stop.load(Ordering::Acquire)
            && state
                .capture
                .as_ref()
                .is_some_and(|scope| self.fenced_capture.as_ref().is_none_or(|old| !old.same(scope)));
        if !capture {
            self.begin_stop(state.policy, now, capture_barrier)?;
        }
        if self.pixels.is_none() && self.stopping.is_none() {
            shared.capture_consumed(capture_barrier);
        }
        if capture && self.pending.is_none() {
            self.capture_scope = state.capture.clone();
        }
        if self
            .stopping
            .as_ref()
            .is_some_and(|s| Instant::now().saturating_duration_since(s.began) >= DEADLINE)
        {
            return Err(Error::Expired);
        }
        let policy = if self.stopping.is_some() || !capture {
            PreviewPolicy::new(
                state.policy.source(),
                state.policy.epochs(),
                aurora_session::preview_image::PreviewAccess {
                    capture_active: false,
                    ..state.policy.access()
                },
            )
            .map_err(|_| Error::Binding)?
        } else {
            state.policy
        };
        if self.sent_policy != Some(policy) || now.saturating_duration_since(self.heartbeat) >= HEARTBEAT {
            self.pair.queue_metadata(Metadata::Policy {
                connection: self.token.connection,
                policy,
            })?;
            self.sent_policy = Some(policy);
            self.heartbeat = now;
        }
        if capture && self.stopping.is_none() && self.sent_layout != state.layout {
            if let Some(layout) = state.layout {
                self.pair.queue_metadata(Metadata::Layout {
                    connection: self.token.connection,
                    layout,
                })?;
            }
            self.sent_layout = state.layout;
        }
        if capture && self.stopping.is_none() && self.pixels.is_none() && self.pending.is_none() {
            self.pending_capture = state.capture.clone();
            self.pending = Some(WorkerPending::new(
                port,
                self.token.source,
                key.clone(),
                Lane::Pixels,
                self.token.connection,
                self.token.pixel_epoch,
                now,
            )?);
        }
        if let Some(pending) = &mut self.pending {
            if !pending.fresh() {
                return Err(Error::Expired);
            }
            #[cfg(test)]
            let held = shared.hold_pixel_accept.load(Ordering::Acquire) && pending.waiting_accept();
            #[cfg(not(test))]
            let held = false;
            if !held && let Some(auth) = pending.poll(now, clock)? {
                if !pending.fresh() {
                    return Err(Error::Expired);
                }
                let scope = auth.scope();
                let mut mailbox = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let fresh = Instant::now();
                if shared.emergency.load(Ordering::Acquire)
                    || shared.stop.load(Ordering::Acquire)
                    || mailbox.token != Some(self.token)
                    || !domain_current(&mailbox, self.token.source, &self.source_control, fresh)
                {
                    return Err(Error::Expired);
                }
                let current = mailbox.state.as_ref().ok_or(Error::Expired)?;
                let retiring = !current.capture_current(fresh)
                    || shared.capture_stop.load(Ordering::Acquire)
                    || self
                        .pending_capture
                        .as_ref()
                        .is_none_or(|old| current.capture.as_ref().is_none_or(|new| !old.same(new)));
                let current_policy = current.policy;
                if retiring {
                    self.pair.attach_retiring(auth)?;
                } else {
                    self.pair.attach_pixels(auth)?;
                }
                let pending = self.pending.take().ok_or(Error::Sequence)?;
                self.pending_capture = None;
                self.pixels = Some(pending.stream);
                self.token.pixel_epoch = scope.role_epoch;
                self.capture_gate = EpochFence::new();
                if retiring {
                    self.capture_gate.revoke();
                    self.pair.suspend_pixels();
                }
                mailbox.token = Some(self.token);
                mailbox.capture_gate = Some(self.capture_gate.clone());
                drop(mailbox);
                if retiring {
                    self.begin_stop(current_policy, Instant::now(), shared.capture_epoch.load(Ordering::Acquire))?;
                }
            }
        }
        let (frame, outcomes) = {
            let mut m = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            (m.frame.take(), m.outcomes.drain(..).collect::<Vec<_>>())
        };
        if let Some(frame) = frame {
            let sent = match frame {
                FrameOffer::Native { guard, stamp, frame }
                    if capture && self.stopping.is_none() && guard.token == self.token && guard.accepts(stamp, &frame, now) =>
                {
                    self.pair
                        .queue_guarded_capture(stamp, frame, guard.bind_io(self.capture_gate.clone(), shared.emergency.clone()))
                }
                #[cfg(test)]
                FrameOffer::Fixture {
                    token,
                    stamp,
                    frame,
                    current,
                } if capture && self.stopping.is_none() && token == self.token => {
                    self.pair
                        .queue_captured_fixture_gate(stamp, frame, current, self.capture_gate.clone())
                }
                _ => Ok(()),
            };
            match sent {
                Ok(()) | Err(Error::Busy) => {}
                Err(Error::CaptureRevoked) => self.begin_stop(policy, now, shared.capture_epoch.load(Ordering::Acquire))?,
                Err(e) => return Err(e),
            }
        }
        for outcome in outcomes {
            let metadata = match outcome {
                Outcome::Unavailable { token, claim, reason } if token == self.token => {
                    Some(Metadata::Unavailable { claim: *claim, reason })
                }
                Outcome::Stopped { token, demand } if token == self.token => Some(Metadata::Stopped {
                    source: token.source,
                    connection: token.connection,
                    demand,
                }),
                _ => None,
            };
            if let Some(m) = metadata {
                self.pair.queue_metadata(m)?;
            }
        }
        self.pair.set_source_guard(super::guards::IoSourceGuard {
            control: state.retention.control.clone(),
            retention: self.retention_gate.clone(),
            emergency: shared.emergency.clone(),
            native: Some(state.retention.clone()),
            policy_expires: Some(state.retention.native_deadline()),
        })?;
        let mut tick = TickBudget::new();
        let _auth = tick.external(1024, 8);
        #[cfg(test)]
        if self.stopping.is_some()
            && shared.break_before_ack.swap(false, Ordering::AcqRel)
            && let Some(stream) = self.pixels.take()
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
        let pixels = if shared.stall_pixels.load(Ordering::Acquire) {
            None
        } else {
            self.pixels.as_mut()
        };
        let packets = match self.pair.poll_roles(&mut self.metadata, pixels, clock, &mut tick) {
            Ok(p) => p,
            Err(Error::CaptureRevoked) => {
                self.begin_stop(policy, now, shared.capture_epoch.load(Ordering::Acquire))?;
                Vec::new()
            }
            Err(e) => return Err(e),
        };
        for packet in packets {
            let Received::Metadata(metadata) = packet else {
                return Err(Error::Direction);
            };
            match metadata {
                Metadata::CaptureStopAck {
                    source,
                    connection,
                    pixel_epoch,
                    stop,
                } => {
                    if source != self.token.source || connection != self.token.connection {
                        return Err(Error::Binding);
                    }
                    let Some(pending) = &self.stopping else {
                        if self.last_ack == Some((pixel_epoch, stop)) {
                            continue;
                        }
                        if pixel_epoch < self.token.pixel_epoch {
                            continue;
                        }
                        return Err(Error::Sequence);
                    };
                    if pixel_epoch < pending.pixel_epoch {
                        continue;
                    }
                    if pixel_epoch != pending.pixel_epoch || stop != pending.stop {
                        return Err(Error::Binding);
                    }
                    if pending.began.elapsed() >= DEADLINE {
                        return Err(Error::Expired);
                    }
                    // Single closer: no physical pixel close precedes this exact
                    // authenticated ACK. Native handoff never waited for it.
                    if let Some(stream) = self.pixels.take() {
                        let _ = stream.shutdown(Shutdown::Both);
                    }
                    let consumed = pending.barrier;
                    self.last_ack = Some((pixel_epoch, stop));
                    self.stopping = None;
                    shared.capture_consumed(consumed);
                }
                Metadata::Demand { demand, size, rate, .. } => {
                    self.command(PreviewCommand::RememberActive { demand, size, rate }, now, shared)?
                }
                Metadata::Capture(claim) => {
                    if !capture || self.stopping.is_some() || !self.pair.has_pixels() || !claim.matches_policy(state.policy) {
                        continue;
                    }
                    self.command(PreviewCommand::Capture(Box::new(claim)), now, shared)?;
                }
                Metadata::Stop { demand, .. } => {
                    shared.capture_revoked();
                    self.begin_stop(policy, now, shared.capture_epoch.load(Ordering::Acquire))?;
                    self.command(PreviewCommand::Stop { demand }, now, shared)?;
                }
                _ => return Err(Error::Direction),
            }
        }
        // While waiting for ACK, the peer must keep its descriptor open. A
        // reverse cross-socket EOF before ACK is an unexpected full fault.
        if self.stopping.is_some()
            && let Some(stream) = &self.pixels
        {
            let mut byte = [0; 1];
            match stream.peek(&mut byte) {
                Ok(0) => return Err(Error::Closed),
                Ok(_) => return Err(Error::Direction),
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(Error::Io(e.kind())),
            }
        }
        if !self.pair.pixel_pending() {
            let m = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if m.frame.is_none() {
                shared.frame_busy.store(false, Ordering::Release);
            }
        }
        Ok(())
    }
    fn command(&self, command: PreviewCommand, now: Instant, shared: &Shared<WorkerMailbox>) -> Result<(), Error> {
        let mut m = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if m.token != Some(self.token) {
            return Err(Error::Binding);
        }
        if m.commands.len() >= QUEUE {
            return Err(Error::Busy);
        }
        m.commands.push_back(ReceivedCommand {
            token: self.token,
            received_at: now,
            command,
        });
        Ok(())
    }
}

fn domain_current(mailbox: &WorkerMailbox, source: PreviewSource, witness: &EpochFence, now: Instant) -> bool {
    mailbox
        .source
        .as_ref()
        .is_some_and(|lease| lease.control.source == source && lease.control.witness.same(witness) && lease.control.current(now))
        && mailbox
            .state
            .as_ref()
            .is_some_and(|state| state.policy.source() == source && state.retention.control.witness.same(witness) && state.current(now))
}

pub(super) fn run(bootstrap: PreviewBootstrap, shared: Arc<Shared<WorkerMailbox>>) {
    struct End(Arc<Shared<WorkerMailbox>>);
    impl Drop for End {
        fn drop(&mut self) {
            self.0.emergency.store(true, Ordering::Release);
            self.0.stop.store(true, Ordering::Release);
            let mut m = self.0.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            m.reset();
            m.source = None;
            self.0.wake.notify_all();
        }
    }
    let _end = End(shared.clone());
    let mut connected: Option<Connected> = None;
    let mut pending: Option<WorkerPending> = None;
    let mut revision = 0;
    let mut retry = Instant::now();
    while !shared.stop.load(Ordering::Acquire) {
        let now = Instant::now();
        let clock = shared.clock(now);
        if shared.emergency.load(Ordering::Acquire) {
            let consumed = shared.retirement_epoch.load(Ordering::Acquire);
            let capture_consumed = shared.capture_epoch.load(Ordering::Acquire);
            connected = None;
            pending = None;
            shared.frame_busy.store(false, Ordering::Release);
            let mut m = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            m.frame = None;
            m.commands.clear();
            m.outcomes.clear();
            m.token = None;
            if let Some(gate) = m.capture_gate.take() {
                gate.revoke();
            }
            if shared.publication_failed.swap(false, Ordering::AcqRel) {
                m.state = None;
            }
            shared.retired(consumed);
            shared.capture_consumed(capture_consumed);
        }
        let (source, state, current) = {
            let m = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            (m.source.clone(), m.state.clone(), m.revision)
        };
        if current != revision {
            connected = None;
            pending = None;
            revision = current;
        }
        let valid =
            source.as_ref().is_some_and(|s| s.control.current(now)) && state.as_ref().is_some_and(|s| s.current(now)) && current != 0;
        if !valid {
            connected = None;
            pending = None;
            shared.frame_busy.store(false, Ordering::Release);
            let mut m = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            m.frame = None;
            m.commands.clear();
            m.outcomes.clear();
            m.token = None;
            if let Some(gate) = m.capture_gate.take() {
                gate.revoke();
            }
            drop(m);
            shared.wait();
            continue;
        }
        let Some(source) = source else { continue };
        if connected.is_none() && pending.is_none() && now >= retry {
            retry = now + DEADLINE;
            pending = WorkerPending::new(
                bootstrap.port,
                source.control.source,
                bootstrap.key.clone(),
                Lane::Metadata,
                0,
                0,
                now,
            )
            .ok();
        }
        let mut failed = false;
        if let Some(auth) = &mut pending {
            match auth.poll(now, clock) {
                Ok(Some(accepted)) => {
                    let scope = accepted.scope();
                    let Some(auth) = pending.take() else { continue };
                    if !auth.fresh() {
                        failed = true;
                    } else {
                        match Pair::metadata(accepted, None) {
                            Ok(pair) => {
                                let mut mailbox = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                                let current_now = Instant::now();
                                if shared.emergency.load(Ordering::Acquire)
                                    || shared.stop.load(Ordering::Acquire)
                                    || mailbox.revision != current
                                    || !domain_current(&mailbox, scope.source, &source.control.witness, current_now)
                                    || !auth.fresh()
                                {
                                    failed = true;
                                } else {
                                    let token = ConnectionToken {
                                        source: scope.source,
                                        connection: scope.connection,
                                        pixel_epoch: 0,
                                        incarnation: shared.incarnation,
                                    };
                                    connected = Some(Connected {
                                        pair,
                                        metadata: auth.stream,
                                        pixels: None,
                                        pending: None,
                                        pending_capture: None,
                                        source_control: source.control.witness.clone(),
                                        token,
                                        stopping: None,
                                        next_stop: 1,
                                        last_ack: None,
                                        heartbeat: now - HEARTBEAT,
                                        sent_policy: None,
                                        sent_layout: None,
                                        capture_scope: None,
                                        fenced_capture: None,
                                        capture_gate: EpochFence::new(),
                                        retention_gate: EpochFence::new(),
                                    });
                                    mailbox.token = Some(token);
                                    mailbox.capture_gate = connected.as_ref().map(|c| c.capture_gate.clone());
                                }
                            }
                            Err(_) => failed = true,
                        }
                    }
                }
                Ok(None) => {}
                Err(_) => failed = true,
            }
        }
        if let Some(connection) = &mut connected {
            let latest = {
                let m = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if !shared.emergency.load(Ordering::Acquire)
                    && domain_current(&m, connection.token.source, &connection.source_control, Instant::now())
                {
                    m.state.clone()
                } else {
                    None
                }
            };
            if let Some(latest) = latest {
                if !failed
                    && connection
                        .poll(
                            &latest,
                            bootstrap.port,
                            &bootstrap.key,
                            Instant::now(),
                            shared.clock(Instant::now()),
                            &shared,
                        )
                        .is_err()
                {
                    failed = true;
                }
            } else {
                failed = true;
            }
        }
        if failed {
            connected = None;
            pending = None;
            shared.frame_busy.store(false, Ordering::Release);
            retry = now + DEADLINE;
            let mut m = shared.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            m.frame = None;
            m.commands.clear();
            m.outcomes.clear();
            m.token = None;
            if let Some(gate) = m.capture_gate.take() {
                gate.revoke();
            }
        }
        shared.wait();
    }
}
