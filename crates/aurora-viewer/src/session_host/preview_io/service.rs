//! Try-only small caller endpoints. Socket setup and every codec/MAC operation
//! begin only inside the spawned owners, never in these caller methods.
use super::state::*;
use super::*;
use aurora_render::viewport_capture::CapturedViewport;
use std::io;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering},
};

static HOST_ACTIVE: AtomicBool = AtomicBool::new(false);
static WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);
struct Owner(&'static AtomicBool);
impl Owner {
    fn acquire(flag: &'static AtomicBool) -> io::Result<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| io::Error::from(io::ErrorKind::AlreadyExists))?;
        Ok(Self(flag))
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
fn host_budget() -> ImageBudget {
    static BUDGET: std::sync::OnceLock<ImageBudget> = std::sync::OnceLock::new();
    BUDGET
        .get_or_init(|| ImageBudget::new(RAW_BYTES).expect("32 MiB is a positive constant below the model's 64 MiB bound"))
        .clone()
}
pub(super) struct Shared<T> {
    pub mailbox: Mutex<T>,
    pub wake: Condvar,
    pub emergency: Arc<AtomicBool>,
    pub capture_stop: AtomicBool,
    pub frame_busy: AtomicBool,
    #[cfg(test)]
    pub stall_pixels: AtomicBool,
    #[cfg(test)]
    pub break_before_ack: AtomicBool,
    pub publication_failed: AtomicBool,
    pub stop: AtomicBool,
    pub ended: AtomicBool,
    pub port: AtomicU16,
    pub clock: Instant,
    pub incarnation: u64,
}
impl<T> Shared<T> {
    fn new(mailbox: T) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let incarnation = NEXT
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .unwrap_or(0);
        Self {
            incarnation,
            mailbox: Mutex::new(mailbox),
            wake: Condvar::new(),
            emergency: Arc::new(AtomicBool::new(false)),
            capture_stop: AtomicBool::new(false),
            frame_busy: AtomicBool::new(false),
            #[cfg(test)]
            stall_pixels: AtomicBool::new(false),
            #[cfg(test)]
            break_before_ack: AtomicBool::new(false),
            publication_failed: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            port: AtomicU16::new(0),
            clock: Instant::now(),
        }
    }
    pub fn fail(&self) {
        self.publication_failed.store(true, Ordering::Release);
        self.emergency.store(true, Ordering::Release);
        self.wake.notify_one();
    }
    pub fn clock(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.clock).as_millis().min(u128::from(u64::MAX)) as u64
    }
    pub fn wait(&self) {
        let guard = self.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.stop.load(Ordering::Acquire) {
            drop(
                self.wake
                    .wait_timeout(guard, PERIOD)
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
        }
    }
    fn stop(&self) {
        self.emergency.store(true, Ordering::Release);
        self.stop.store(true, Ordering::Release);
        self.wake.notify_all();
    }
}
struct ThreadEnd<T> {
    owner: Option<Owner>,
    shared: Arc<Shared<T>>,
}
impl<T> Drop for ThreadEnd<T> {
    fn drop(&mut self) {
        drop(self.owner.take());
        self.shared.ended.store(true, Ordering::Release);
        self.shared.wake.notify_all();
    }
}
pub(crate) struct HostChannel {
    pub(super) shared: Arc<Shared<HostMailbox>>,
    installation: InstallationId,
    host: HostId,
    budget: ImageBudget,
}
impl HostChannel {
    pub fn start(installation: InstallationId, host: HostId) -> io::Result<Self> {
        let owner = Owner::acquire(&HOST_ACTIVE)?;
        let budget = host_budget();
        let receive_budget = budget.clone();
        let shared = Arc::new(Shared::new(HostMailbox::new()));
        let io = shared.clone();
        if shared.incarnation == 0 {
            return Err(io::ErrorKind::Other.into());
        }
        std::thread::Builder::new()
            .name("session-preview-host".into())
            .stack_size(THREAD_STACK)
            .spawn(move || {
                let _end = ThreadEnd {
                    owner: Some(owner),
                    shared: io.clone(),
                };
                super::host::run(installation, host, io, receive_budget)
            })?;
        Ok(Self {
            shared,
            installation,
            host,
            budget,
        })
    }
    pub fn image_budget(&self) -> ImageBudget {
        self.budget.clone()
    }
    pub fn port(&self) -> Option<u16> {
        let p = self.shared.port.load(Ordering::Acquire);
        (p != 0 && !self.shared.stop.load(Ordering::Acquire)).then_some(p)
    }
    pub fn publish_permits(&self, permits: &[HostPermit]) -> Result<(), Error> {
        let now = Instant::now();
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        if permits.len() > MAX_WORKERS
            || permits.iter().enumerate().any(|(i, p)| {
                p.registration == 0
                    || !p.current(now)
                    || p.source.worker().installation != self.installation
                    || p.source.worker().host != self.host
                    || permits[..i].iter().any(|q| q.source.worker().worker == p.source.worker().worker)
            })
        {
            self.shared.fail();
            return Err(Error::Binding);
        }
        let Ok(mut mailbox) = self.shared.mailbox.try_lock() else {
            self.shared.fail();
            return Err(Error::Busy);
        };
        for update in &mut mailbox.updates {
            if !permits
                .iter()
                .any(|p| p.source == update.observation.token.source && p.current(now))
            {
                update.observation.retention.revoke();
                update.frame = None;
            }
        }
        mailbox.revision = mailbox.revision.checked_add(1).ok_or_else(|| {
            self.shared.fail();
            Error::Sequence
        })?;
        mailbox.permits.clear();
        mailbox.permits.extend_from_slice(permits);
        self.shared.publication_failed.store(false, Ordering::Release);
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn try_command(&self, token: ConnectionToken, command: PreviewCommand, capture: Option<CaptureControlGuard>) -> Result<(), Error> {
        if self.shared.stop.load(Ordering::Acquire) || self.shared.emergency.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        let Ok(mut mailbox) = self.shared.mailbox.try_lock() else {
            return Err(Error::Busy);
        };
        if !mailbox
            .permits
            .iter()
            .any(|p| p.source == token.source && p.current(Instant::now()))
            || !mailbox
                .updates
                .iter()
                .any(|u| u.observation.token == token && u.observation.current())
        {
            return Err(Error::Binding);
        }
        command.metadata(token).validate()?;
        if let PreviewCommand::Capture(claim) = &command {
            if !mailbox.updates.iter().any(|u| {
                u.observation.token == token
                    && u.observation.capture.current()
                    && u.observation.policy.is_some_and(|p| claim.matches_policy(p))
            }) || token.pixel_epoch == 0
                || claim.source() != token.source
                || claim.connection() != token.connection
                || capture.as_ref().is_none_or(|g| !g.current(**claim, Instant::now()))
            {
                return Err(Error::Binding);
            }
        } else if capture.is_some() {
            return Err(Error::Binding);
        }
        if matches!(command, PreviewCommand::Stop { .. }) {
            if let Some(update) = mailbox.updates.iter_mut().find(|u| u.observation.token == token) {
                update.observation.capture.revoke();
                update.frame = None;
            }
        }
        if mailbox.commands.len() >= QUEUE * MAX_WORKERS
            || mailbox.commands.iter().filter(|c| c.token.source == token.source).count() >= QUEUE
        {
            if matches!(command, PreviewCommand::Stop { .. }) {
                self.shared.fail();
            }
            return Err(Error::Busy);
        }
        mailbox.commands.push_back(CommandOffer { token, command, capture });
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn take_update(&self) -> Option<HostUpdate> {
        if self.shared.emergency.load(Ordering::Acquire) || self.shared.stop.load(Ordering::Acquire) {
            return None;
        }
        let mut m = self.shared.mailbox.try_lock().ok()?;
        let diagnostics = m.diagnostics;
        Some(HostUpdate {
            pending_roles: diagnostics[0],
            connected_roles: diagnostics[1],
            raw_bytes: diagnostics[2],
            sources: m
                .updates
                .iter_mut()
                .map(|u| SourceUpdate {
                    observation: u.observation.clone(),
                    frame: u.frame.take(),
                })
                .collect(),
        })
    }
    pub fn stop(&self) {
        self.shared.stop();
    }
    pub fn ended(&self) -> bool {
        self.shared.ended.load(Ordering::Acquire)
    }
}
impl Drop for HostChannel {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) struct WorkerChannel {
    pub(super) shared: Arc<Shared<WorkerMailbox>>,
}
impl WorkerChannel {
    pub fn start(bootstrap: PreviewBootstrap, identity: WorkerIdentity, lease: u64) -> io::Result<Self> {
        if bootstrap.port == 0 || lease == 0 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let owner = Owner::acquire(&WORKER_ACTIVE)?;
        let shared = Arc::new(Shared::new(WorkerMailbox::new(identity, lease)));
        let io = shared.clone();
        if shared.incarnation == 0 {
            return Err(io::ErrorKind::Other.into());
        }
        std::thread::Builder::new()
            .name("session-preview-worker".into())
            .stack_size(THREAD_STACK)
            .spawn(move || {
                let _end = ThreadEnd {
                    owner: Some(owner),
                    shared: io.clone(),
                };
                super::worker::run(bootstrap, io)
            })?;
        Ok(Self { shared })
    }
    pub fn set_source(&self, source: Option<SourceLease>) -> Result<(), Error> {
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        let Ok(mut m) = self.shared.mailbox.try_lock() else {
            self.shared.fail();
            return Err(Error::Busy);
        };
        let source = source.filter(|s| m.accepts(s.control.source) && s.control.current(Instant::now()));
        if m.source.as_ref().map(|s| s.control.source) != source.as_ref().map(|s| s.control.source) {
            self.shared.emergency.store(true, Ordering::Release);
            m.reset();
        }
        m.source = source;
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn publish_state(&self, state: WorkerState) -> Result<(), Error> {
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        let Ok(mut m) = self.shared.mailbox.try_lock() else {
            self.shared.fail();
            return Err(Error::Busy);
        };
        if m.source.as_ref().is_none_or(|s| {
            s.control.source != state.policy.source()
                || !s.control.current(Instant::now())
                || !s.control.witness.same(&state.retention.control.witness)
                || state.retention.control.expires_at > s.control.expires_at
        }) || !state.current(Instant::now())
        {
            m.state = None;
            self.shared.fail();
            return Err(Error::Expired);
        }
        if let Some(old) = &m.state {
            if old.policy.source() != state.policy.source() || old.policy.epochs() != state.policy.epochs() {
                self.shared.emergency.store(true, Ordering::Release);
            } else if old.policy.access().activation != state.policy.access().activation
                || (old.policy.access().capture_active && !state.policy.access().capture_active)
                || old
                    .capture
                    .as_ref()
                    .is_some_and(|c| state.capture.as_ref().is_none_or(|n| !c.same(n)))
            {
                self.shared.capture_stop.store(true, Ordering::Release);
                m.frame = None;
            }
        }
        m.state = Some(state);
        self.shared.publication_failed.store(false, Ordering::Release);
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn frame_available(&self) -> bool {
        !self.shared.frame_busy.load(Ordering::Acquire) && self.connection().is_some_and(|t| t.pixel_epoch > 0)
    }
    pub fn connection(&self) -> Option<ConnectionToken> {
        if self.shared.emergency.load(Ordering::Acquire) {
            return None;
        }
        self.shared.mailbox.try_lock().ok()?.token
    }
    pub fn take_command(&self) -> Option<ReceivedCommand> {
        if self.shared.emergency.load(Ordering::Acquire) || self.shared.stop.load(Ordering::Acquire) {
            return None;
        }
        let mut m = self.shared.mailbox.try_lock().ok()?;
        while let Some(c) = m.commands.pop_front() {
            let current = match &c.command {
                PreviewCommand::Capture(claim) => {
                    !self.shared.capture_stop.load(Ordering::Acquire)
                        && m.state
                            .as_ref()
                            .is_some_and(|s| s.capture_current(Instant::now()) && claim.matches_policy(s.policy))
                }
                _ => true,
            };
            if current
                && Some(c.token) == m.token
                && c.received_at.elapsed() < DEADLINE
                && m.source.as_ref().is_some_and(|s| s.control.current(Instant::now()))
            {
                return Some(c);
            }
        }
        None
    }
    pub fn try_frame(&self, guard: CapturePermitGuard, stamp: PreviewStamp, frame: CapturedViewport) -> Result<(), Error> {
        if self.shared.emergency.load(Ordering::Acquire) || self.shared.stop.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        let Ok(mut m) = self.shared.mailbox.try_lock() else {
            return Err(Error::Busy);
        };
        if m.token != Some(guard.token)
            || !guard.accepts(stamp, &frame, Instant::now())
            || m.state
                .as_ref()
                .is_none_or(|s| !s.current(Instant::now()) || s.capture.as_ref().is_none_or(|c| !c.same(&guard.native)))
        {
            return Err(Error::Expired);
        }
        if m.frame.is_some() || self.shared.frame_busy.swap(true, Ordering::AcqRel) {
            return Err(Error::Busy);
        }
        m.frame = Some(FrameOffer::Native {
            guard: Box::new(guard),
            stamp,
            frame,
        });
        self.shared.wake.notify_one();
        Ok(())
    }
    /// Private fixture framing/resource seam; no actual native permit is minted.
    #[cfg(test)]
    pub(super) fn fixture_frame(
        &self,
        token: ConnectionToken,
        stamp: PreviewStamp,
        frame: CapturedViewport,
        current: Arc<AtomicBool>,
    ) -> Result<(), Error> {
        if self.shared.emergency.load(Ordering::Acquire) || self.shared.stop.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        let Ok(mut m) = self.shared.mailbox.try_lock() else {
            return Err(Error::Busy);
        };
        if m.token != Some(token)
            || stamp.claim().source() != token.source
            || stamp.claim().connection() != token.connection
            || m.state
                .as_ref()
                .is_none_or(|s| !s.capture_current(Instant::now()) || !stamp.claim().matches_policy(s.policy))
        {
            return Err(Error::Binding);
        }
        if m.frame.is_some() || self.shared.frame_busy.swap(true, Ordering::AcqRel) {
            return Err(Error::Busy);
        }
        m.frame = Some(FrameOffer::Fixture {
            token,
            stamp,
            frame,
            current,
        });
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn try_unavailable(&self, token: ConnectionToken, claim: CaptureClaim, reason: Unavailable) -> Result<(), Error> {
        if claim.source() != token.source || claim.connection() != token.connection {
            return Err(Error::Binding);
        }
        self.outcome(
            Outcome::Unavailable {
                token,
                claim: Box::new(claim),
                reason,
            },
            token,
        )
    }
    pub fn acknowledge_stop(&self, token: ConnectionToken, demand: u64) -> Result<(), Error> {
        if demand == 0 {
            return Err(Error::Sequence);
        }
        self.outcome(Outcome::Stopped { token, demand }, token)
    }
    fn outcome(&self, outcome: Outcome, token: ConnectionToken) -> Result<(), Error> {
        if self.shared.emergency.load(Ordering::Acquire) || self.shared.stop.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        let Ok(mut m) = self.shared.mailbox.try_lock() else {
            return Err(Error::Busy);
        };
        if m.token != Some(token) {
            return Err(Error::Binding);
        }
        if m.outcomes.len() >= QUEUE {
            return Err(Error::Busy);
        }
        m.outcomes.push_back(outcome);
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn stop(&self) {
        self.shared.stop();
    }
    pub fn ended(&self) -> bool {
        self.shared.ended.load(Ordering::Acquire)
    }
}
impl Drop for WorkerChannel {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{control, source, state};
    use super::*;
    fn claim() -> CaptureClaim {
        let c = control(source(1));
        let state = state(c, Some(EpochFence::new()));
        let mut images = aurora_session::preview_image::SourceImages::new(source(1), ImageBudget::new(1024 * 1024).unwrap()).unwrap();
        images.sync(state.policy, 0).unwrap();
        images
            .request(
                1,
                1,
                aurora_session::preview_image::PreviewGeometry::new([640, 360], [0, 0, 640, 360], PreviewSize::Small).unwrap(),
                1,
            )
            .unwrap()
    }
    fn worker() -> WorkerChannel {
        let s = source(1);
        let c = control(s);
        let shared = Arc::new(Shared::new(WorkerMailbox::new(s.worker(), s.lease_epoch())));
        {
            let mut m = shared.mailbox.lock().unwrap();
            m.source = Some(SourceLease { control: c.clone() });
            m.state = Some(state(c, Some(EpochFence::new())));
            m.token = Some(ConnectionToken {
                source: s,
                connection: 1,
                pixel_epoch: 1,
                incarnation: shared.incarnation,
            });
        }
        WorkerChannel { shared }
    }
    #[test]
    fn preview_io_mailbox_contention_fails_closed_and_stop_never_waits_for_held_state() {
        let channel = worker();
        let shared = channel.shared.clone();
        let held = shared.mailbox.lock().unwrap();
        let c = control(source(1));
        assert!(matches!(channel.publish_state(state(c, Some(EpochFence::new()))), Err(Error::Busy)));
        assert!(shared.emergency.load(Ordering::Acquire));
        assert!(channel.connection().is_none());
        channel.stop();
        assert!(shared.stop.load(Ordering::Acquire));
        drop(held);
    }
    #[test]
    fn preview_io_capture_loss_and_privacy_denied_allowed_coalescing_keep_independent_barriers() {
        let channel = worker();
        let c = channel.shared.mailbox.lock().unwrap().source.as_ref().unwrap().control.clone();
        channel.publish_state(state(c.clone(), None)).unwrap();
        channel.publish_state(state(c.clone(), Some(EpochFence::new()))).unwrap();
        assert!(channel.shared.capture_stop.load(Ordering::Acquire));
        assert!(!channel.shared.emergency.load(Ordering::Acquire));
        let mut denied = state(c.clone(), None);
        denied.policy = PreviewPolicy::new(
            source(1),
            denied.policy.epochs(),
            aurora_session::preview_image::PreviewAccess {
                ready: false,
                active: false,
                capture_active: false,
                retain_last_active: false,
                activation: 1,
            },
        )
        .unwrap();
        denied.retention = SourceRetentionGuard::fixture(c.clone(), denied.policy, c.expires_at);
        assert!(matches!(channel.publish_state(denied), Err(Error::Expired)));
        channel.publish_state(state(c, Some(EpochFence::new()))).unwrap();
        assert!(
            channel.shared.emergency.load(Ordering::Acquire),
            "later Allowed snapshot cannot erase a source retirement barrier"
        );
        assert!(channel.connection().is_none());
    }
    #[test]
    fn preview_io_commands_preserve_arrival_age_bound_capacity_and_drop_retired_tokens() {
        let channel = worker();
        let token = channel.connection().unwrap();
        let arrival = Instant::now() - Duration::from_millis(20);
        {
            let mut m = channel.shared.mailbox.lock().unwrap();
            for demand in 1..=QUEUE as u64 {
                m.commands.push_back(ReceivedCommand {
                    token,
                    received_at: arrival,
                    command: PreviewCommand::Stop { demand },
                });
            }
            assert_eq!(m.commands.capacity(), QUEUE);
        }
        let first = channel.take_command().unwrap();
        assert_eq!(first.received_at, arrival);
        assert_eq!(channel.shared.mailbox.lock().unwrap().commands.len(), QUEUE - 1);
        channel.shared.mailbox.lock().unwrap().token = Some(ConnectionToken { connection: 2, ..token });
        assert!(channel.take_command().is_none());
        channel.try_unavailable(token, claim(), Unavailable::Busy).unwrap_err();
        assert!(channel.acknowledge_stop(token, 1).is_err());
        let fresh = channel.connection().unwrap();
        for _ in 1..=QUEUE as u64 {
            channel.try_unavailable(fresh, claim(), Unavailable::Busy).unwrap();
        }
        assert!(matches!(channel.acknowledge_stop(fresh, 1), Err(Error::Busy)));
    }
    #[test]
    fn preview_io_original_control_native_deadlines_and_missing_source_cannot_be_rejuvenated() {
        let channel = worker();
        let mut s = channel.shared.mailbox.lock().unwrap().state.clone().unwrap();
        let original = s.native_observed_at;
        assert!(s.current(original + Duration::from_millis(999)));
        assert!(!s.current(original + DEADLINE));
        s.native_observed_at = Instant::now() - Duration::from_secs(2);
        assert!(matches!(channel.publish_state(s), Err(Error::Expired)));
        channel.set_source(None).unwrap();
        assert!(channel.shared.mailbox.lock().unwrap().state.is_none());
        assert!(channel.take_command().is_none());
        assert!(
            channel
                .try_unavailable(
                    ConnectionToken {
                        source: source(1),
                        connection: 1,
                        pixel_epoch: 1,
                        incarnation: 1
                    },
                    claim(),
                    Unavailable::Busy
                )
                .is_err()
        );
    }
    #[test]
    fn preview_io_host_source_removal_revokes_held_observation_before_io_consumes_update() {
        let s = source(1);
        let shared = Arc::new(Shared::new(HostMailbox::new()));
        let channel = HostChannel {
            shared: shared.clone(),
            installation: s.worker().installation,
            host: s.worker().host,
            budget: host_budget(),
        };
        let c = control(s);
        channel
            .publish_permits(&[HostPermit {
                source: s,
                secret: PreviewSecret::synthetic([7; 32]).unwrap(),
                control: c,
                registration: 1,
            }])
            .unwrap();
        let observed = Observation {
            token: ConnectionToken {
                source: s,
                connection: 1,
                pixel_epoch: 1,
                incarnation: shared.incarnation,
            },
            policy: None,
            policy_received_at: None,
            layout: None,
            status: Status::Metadata,
            stopped_demand: None,
            unavailable: None,
            retention: EpochFence::new(),
            capture: EpochFence::new(),
            emergency: shared.emergency.clone(),
        };
        shared.mailbox.lock().unwrap().updates.push(SourceUpdate {
            observation: observed.clone(),
            frame: None,
        });
        assert!(observed.current());
        channel.publish_permits(&[]).unwrap();
        assert!(!observed.current());
        assert!(
            channel
                .try_command(observed.token, PreviewCommand::Stop { demand: 1 }, None)
                .is_err()
        );
    }
    #[test]
    fn preview_io_host_raw_pool_survives_service_generation_and_preserves_ui_reserve() {
        let _serial = super::super::tests::OWNED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let a = host_budget();
        let baseline = a.used();
        let g = aurora_session::preview_image::PreviewGeometry::new([640, 360], [0, 0, 640, 360], PreviewSize::Large).unwrap();
        let held = a.reserve(g).unwrap();
        let b = host_budget();
        assert_eq!(b.used(), baseline + g.payload_bytes());
        assert_eq!(
            RAW_BYTES + UI_BYTES + SERVICE_BYTES,
            aurora_session::preview_image::HOST_IMAGE_BYTES
        );
        drop(held);
        assert_eq!(b.used(), baseline);
    }
}

#[cfg(test)]
mod resource_tests {
    use super::*;
    #[test]
    fn preview_io_worker_full_typed_mailboxes_and_held_capture_generations_fit_seven_plus_one_partition() {
        let source = super::super::tests::source(1);
        let control = super::super::tests::control(source);
        let shared = Arc::new(Shared::new(WorkerMailbox::new(source.worker(), source.lease_epoch())));
        let token = ConnectionToken {
            source,
            connection: 1,
            pixel_epoch: 1,
            incarnation: shared.incarnation,
        };
        {
            let mut m = shared.mailbox.lock().unwrap();
            m.source = Some(SourceLease { control: control.clone() });
            m.state = Some(super::super::tests::state(control, Some(EpochFence::new())));
            m.token = Some(token);
            for demand in 1..=QUEUE as u64 {
                m.commands.push_back(ReceivedCommand {
                    token,
                    received_at: Instant::now(),
                    command: PreviewCommand::Stop { demand },
                });
            }
        }
        let channel = WorkerChannel { shared };
        let geometry = aurora_session::preview_image::PreviewGeometry::new([640, 360], [0, 0, 640, 360], PreviewSize::Large).unwrap();
        let mut images = aurora_session::preview_image::SourceImages::new(source, ImageBudget::new(1024 * 1024).unwrap()).unwrap();
        images
            .sync(channel.shared.mailbox.lock().unwrap().state.as_ref().unwrap().policy, 0)
            .unwrap();
        let claim = images.request(1, 1, geometry, 1).unwrap();
        let stamp = PreviewStamp::new(claim, 1, 1, 1, geometry).unwrap();
        let (frame, budget) = super::super::tests::frame(PreviewSize::Large);
        let duplicate = frame.clone();
        channel.fixture_frame(token, stamp, frame, Arc::new(AtomicBool::new(true))).unwrap();
        assert!(matches!(
            channel.fixture_frame(token, stamp, duplicate, Arc::new(AtomicBool::new(true))),
            Err(Error::Busy)
        ));
        for _ in 1..=QUEUE as u64 {
            channel.try_unavailable(token, claim, Unavailable::Busy).unwrap();
        }
        let layout = aurora_render::viewport_capture::CaptureLayout::new(640, 360).unwrap();
        let mut held = Vec::with_capacity(8);
        while let Ok(mut reservation) = budget.pixels(layout) {
            reservation.bytes_mut().fill(255);
            held.push(reservation.finish().unwrap());
        }
        let m = channel.shared.mailbox.lock().unwrap();
        assert_eq!((m.commands.len(), m.outcomes.len()), (QUEUE, QUEUE));
        assert!(m.frame.is_some());
        let storage = lane::worker_storage_ceiling()
            + std::mem::size_of::<Shared<WorkerMailbox>>()
            + std::mem::size_of::<WorkerMailbox>()
            + m.commands.capacity() * std::mem::size_of::<ReceivedCommand>()
            + m.outcomes.capacity() * (std::mem::size_of::<Outcome>() + std::mem::size_of::<CaptureClaim>())
            + 2 * std::mem::size_of::<super::super::handshake::WorkerPending>()
            + 3 * std::mem::size_of::<WorkerState>()
            + 2 * std::mem::size_of::<CapturePermitGuard>()
            + std::mem::size_of::<FrameOffer>();
        assert!(storage <= 512 * 1024);
        assert!(budget.used() + storage + THREAD_STACK + 256 * 1024 <= 8 * 1024 * 1024);
        drop(m);
        drop(channel);
        drop(held);
        assert_eq!(budget.used(), 0);
    }
}
