//! Small caller/IO mailboxes. No codec, socket, MAC or image-size allocation
//! occurs under these locks. Monotonic connection gates outlive moved handles.
use super::*;
use aurora_render::viewport_capture::CapturedViewport;
use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
pub(crate) struct SourceLease {
    pub control: ControlGuard,
}
#[derive(Clone)]
pub(crate) struct WorkerState {
    pub policy: PreviewPolicy,
    pub layout: Option<PreviewLayout>,
    pub retention: SourceRetentionGuard,
    pub capture: Option<EpochFence>,
    pub native_observed_at: Instant,
}
impl WorkerState {
    pub(super) fn capture_current(&self, now: Instant) -> bool {
        self.current(now) && self.policy.access().capture_active && self.capture.as_ref().is_some_and(|c| c.current())
    }
    pub(super) fn current(&self, now: Instant) -> bool {
        self.native_observed_at
            .checked_add(DEADLINE)
            .is_some_and(|end| self.retention.native_deadline() <= end)
            && self.native_observed_at <= now
            && now.duration_since(self.native_observed_at) < DEADLINE
            && self.retention.current(now)
            && self.policy == self.retention.policy
            && self.layout.is_none_or(|layout| layout.matches_policy(self.policy))
    }
}
pub(super) enum FrameOffer {
    Native {
        guard: Box<CapturePermitGuard>,
        stamp: PreviewStamp,
        frame: CapturedViewport,
    },
    // Private codec/resource fixture only; not a renderer/native authority.
    #[cfg(test)]
    Fixture {
        token: ConnectionToken,
        stamp: PreviewStamp,
        frame: CapturedViewport,
        current: Arc<AtomicBool>,
    },
}
#[derive(Clone)]
pub(crate) struct ReceivedCommand {
    pub token: ConnectionToken,
    pub received_at: Instant,
    pub command: PreviewCommand,
}
pub(super) struct CommandOffer {
    pub token: ConnectionToken,
    pub command: PreviewCommand,
    pub capture: Option<CaptureControlGuard>,
}
pub(super) enum Outcome {
    Demand {
        token: ConnectionToken,
        receipt: DemandReceipt,
    },
    Unavailable {
        token: ConnectionToken,
        claim: Box<CaptureClaim>,
        reason: Unavailable,
    },
    Stopped {
        token: ConnectionToken,
        demand: u64,
    },
}
pub(super) struct WorkerMailbox {
    pub identity: WorkerIdentity,
    pub lease: u64,
    pub source: Option<SourceLease>,
    pub state: Option<WorkerState>,
    pub frame: Option<FrameOffer>,
    pub commands: VecDeque<ReceivedCommand>,
    pub outcomes: VecDeque<Outcome>,
    pub token: Option<ConnectionToken>,
    pub capture_gate: Option<EpochFence>,
    pub demands: super::demand::DemandBook,
    pub revision: u64,
}
impl WorkerMailbox {
    pub fn new(identity: WorkerIdentity, lease: u64) -> Self {
        Self {
            identity,
            lease,
            source: None,
            state: None,
            frame: None,
            commands: VecDeque::with_capacity(QUEUE),
            outcomes: VecDeque::with_capacity(QUEUE),
            token: None,
            capture_gate: None,
            demands: super::demand::DemandBook::default(),
            revision: 1,
        }
    }
    pub fn accepts(&self, source: PreviewSource) -> bool {
        source.worker() == self.identity && source.lease_epoch() == self.lease
    }
    pub fn reset(&mut self) {
        self.revision = self.revision.checked_add(1).unwrap_or(0);
        self.state = None;
        self.demands = super::demand::DemandBook::default();
        self.frame = None;
        self.commands.clear();
        self.outcomes.clear();
        self.token = None;
        if let Some(gate) = self.capture_gate.take() {
            gate.revoke();
        }
        if self.revision == 0 {
            self.source = None;
        }
    }
}
#[derive(Clone)]
pub(crate) struct HostPermit {
    pub(super) source: PreviewSource,
    pub(super) secret: PreviewSecret,
    pub(super) control: ControlGuard,
    pub(super) registration: u64,
}
impl HostPermit {
    pub fn from_host(
        token: &crate::session_host::transport::preview_control::AcceptedPreviewSource,
        secret: PreviewSecret,
    ) -> Result<Self, Error> {
        if token.registration() == 0 {
            return Err(Error::Binding);
        }
        Ok(Self {
            source: token.source(),
            secret,
            control: ControlGuard::from_host(token)?,
            registration: token.registration(),
        })
    }

    pub(super) fn current(&self, now: Instant) -> bool {
        self.source == self.control.source && self.control.current(now)
    }
}
/// Body-free terminal/availability state. Source identity is explicit; labels
/// never authorize a role or select a replacement worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Connecting,
    Metadata,
    Pixels,
    CaptureStopped,
    Fault,
    Stopped,
}
#[derive(Clone)]
pub(crate) struct Observation {
    pub(super) domain: ObservationDomain,
    pub(super) policy_receipt: Option<PolicyReceipt>,
    pub(super) layout_receipt: Option<LayoutReceipt>,
    pub demand_receipt: Option<DemandReceipt>,
    pub accepted_demand: Option<DemandReceipt>,
    pub token: ConnectionToken,
    pub policy: Option<PreviewPolicy>,
    pub policy_received_at: Option<Instant>,
    pub layout: Option<PreviewLayout>,
    pub status: Status,
    pub stopped_demand: Option<u64>,
    pub unavailable: Option<(CaptureClaim, Unavailable)>,
    pub(super) retention: EpochFence,
    pub(super) capture: EpochFence,
    pub(super) emergency: Arc<AtomicBool>,
}
impl Observation {
    pub(super) fn new(token: ConnectionToken, permit: &HostPermit, capture: EpochFence, emergency: Arc<AtomicBool>) -> Self {
        let retention = EpochFence::new();
        Self {
            domain: ObservationDomain::new(token.incarnation, permit, token.connection, retention.clone()),
            policy_receipt: None,
            layout_receipt: None,
            demand_receipt: None,
            accepted_demand: None,
            token,
            policy: None,
            policy_received_at: None,
            layout: None,
            status: Status::Metadata,
            stopped_demand: None,
            unavailable: None,
            retention,
            capture,
            emergency,
        }
    }

    pub(super) fn install_policy(&mut self, policy: PreviewPolicy, now: Instant) -> Result<(), Error> {
        let origin = ReceiptOrigin::next(
            &self.domain,
            self.policy_receipt.map(|p| p.origin()),
            super::publication::ReceiptKind::Policy,
        )?;
        self.policy = Some(policy);
        self.policy_received_at = Some(now);
        self.policy_receipt = Some(PolicyReceipt::new(origin, now, policy));
        Ok(())
    }
    pub(super) fn install_layout(&mut self, layout: PreviewLayout, now: Instant) -> Result<(), Error> {
        let origin = ReceiptOrigin::next(
            &self.domain,
            self.layout_receipt.map(|p| p.origin()),
            super::publication::ReceiptKind::Layout,
        )?;
        self.layout = Some(layout);
        self.layout_receipt = Some(LayoutReceipt::new(origin, now, layout));
        Ok(())
    }

    pub fn current(&self) -> bool {
        !self.emergency.load(Ordering::Acquire)
            && self.retention.current()
            && self.policy_received_at.is_none_or(|at| at.elapsed() < DEADLINE)
    }
}
/// Transport output remains unadmitted. A later Broker repeats current control,
/// sync_observed and SourceImages::admit before any cache/UI publication.
pub(crate) struct FrameHandoff {
    pub token: ConnectionToken,
    pub received_at: Instant,
    pub(super) pixels: UnadmittedPixels,
    pub(super) original_capture: CaptureControlGuard,
    pub(super) retention: EpochFence,
    pub(super) capture: EpochFence,
    pub(super) emergency: Arc<AtomicBool>,
}
impl FrameHandoff {
    /// Exact unadmitted target for cancellation/bookkeeping only. No pixel or
    /// credit escapes, and reading this claim grants no admission authority.
    pub fn claim(&self) -> CaptureClaim {
        self.pixels.claim()
    }

    pub fn admit(
        self,
        owner: &mut aurora_session::preview_image::SourceImages,
        control: &ControlGuard,
        now: u64,
    ) -> Result<RetainedFrame, aurora_session::preview_image::ImageError> {
        self.try_admit(owner, control, now).map_err(|(error, rejected)| {
            drop(rejected);
            error
        })
    }

    /// Same admission, returning exact rejected ownership so the caller can
    /// retire raw storage after releasing its Store lock. No new validity or
    /// replay permission is attached to the recovered handoff.
    #[expect(
        clippy::result_large_err,
        reason = "Return the bounded original handoff intact without boxing or copying its owned pixel payload under the Store lock."
    )]
    pub fn try_admit(
        self,
        owner: &mut aurora_session::preview_image::SourceImages,
        control: &ControlGuard,
        now: u64,
    ) -> Result<RetainedFrame, (aurora_session::preview_image::ImageError, Self)> {
        if self.emergency.load(Ordering::Acquire)
            || !self.retention.current()
            || !self.capture.current()
            || !self.original_capture.current(self.claim(), Instant::now())
            || !control.witness.same(&self.original_capture.control.witness)
            || control.source != self.token.source
            || !control.current(Instant::now())
        {
            return Err((aurora_session::preview_image::ImageError::Stale, self));
        }
        let Self {
            token,
            received_at,
            pixels,
            original_capture,
            retention,
            capture,
            emergency,
        } = self;
        match pixels.try_admit(owner, now) {
            Ok(pixels) => Ok(RetainedFrame {
                token,
                pixels,
                retention,
                emergency,
                clock_anchor: (now, Instant::now()),
                source_control: original_capture.control.witness.clone(),
            }),
            Err((error, pixels)) => Err((
                error,
                Self {
                    token,
                    received_at,
                    pixels,
                    original_capture,
                    retention,
                    capture,
                    emergency,
                },
            )),
        }
    }
}
/// No current Active/capture fence is required after final admission. The later
/// owner supplies fresh exact source-control proof on every draw/copy, and the
/// admitted model handle enforces original authenticated metadata expiry.
pub(crate) struct RetainedFrame {
    pub token: ConnectionToken,
    pixels: lane::AdmittedPixels,
    retention: EpochFence,
    emergency: Arc<AtomicBool>,
    clock_anchor: (u64, Instant),
    source_control: EpochFence,
}
impl RetainedFrame {
    fn current(&self, control: &ControlGuard, now: u64) -> Option<u64> {
        if self.emergency.load(Ordering::Acquire)
            || !self.retention.current()
            || !control.witness.same(&self.source_control)
            || control.source != self.token.source
            || !control.current(Instant::now())
            || now < self.clock_anchor.0
        {
            return None;
        }
        // Anchor to actual local elapsed time: an old caller clock/read cannot
        // freeze authority. Round up conservatively at millisecond boundaries.
        let elapsed = u64::try_from(self.clock_anchor.1.elapsed().as_millis()).ok()?.checked_add(1)?;
        let now = now.max(self.clock_anchor.0.checked_add(elapsed)?);
        self.pixels.bytes(now).is_some().then_some(now)
    }
    /// Only prepare a private result inside this callback. The later UI owner
    /// must reserve UI-copy credit and recheck view/binding fences before public
    /// publication. A result is discarded if source/retention expires mid-copy.
    pub fn with_pixels<R>(&self, control: &ControlGuard, now: u64, prepare: impl FnOnce(&[u8]) -> R) -> Option<R> {
        let checked = self.current(control, now)?;
        let result = prepare(self.pixels.bytes(checked)?);
        self.current(control, now)?;
        Some(result)
    }
}

#[cfg(test)]
mod owned_admission_tests {
    use super::*;
    use aurora_session::preview_image::{ImageError, PreviewGeometry, SourceImages};
    use aurora_session::{HostedViewport, PhysicalRect};

    // Pure existing metadata/control fixtures only; no channel, socket, native
    // window or Renderer authority is created by this ownership regression.
    fn fixture() -> (SourceImages, FrameHandoff, ImageBudget, ControlGuard, PreviewGeometry) {
        fixture_with_slots(1)
    }
    fn fixture_with_slots(slots: usize) -> (SourceImages, FrameHandoff, ImageBudget, ControlGuard, PreviewGeometry) {
        let source = super::super::tests::source(1);
        let control = super::super::tests::control(source);
        let state = super::super::tests::state(control.clone(), Some(EpochFence::new()));
        let geometry = PreviewGeometry::new([640, 360], [0, 0, 640, 360], PreviewSize::Small).unwrap();
        let budget = ImageBudget::new(geometry.payload_bytes() * slots).unwrap();
        let mut owner = SourceImages::new(source, budget.clone()).unwrap();
        owner.sync(state.policy, 0).unwrap();
        let claim = owner.request(8, 1, geometry, 1).unwrap();
        let stamp = PreviewStamp::new(claim, 1, 1, 1, geometry).unwrap();
        let pixels = UnadmittedPixels::copy_from(stamp, &vec![255; geometry.payload_bytes()], &budget).unwrap();
        let frame = FrameHandoff {
            token: ConnectionToken::transport_fixture(source, 8, 1, 1).unwrap(),
            received_at: Instant::now(),
            pixels,
            original_capture: CaptureControlGuard {
                control: control.clone(),
                activation: 1,
                viewport: HostedViewport {
                    host_window: 1,
                    rect: PhysicalRect {
                        x: 0,
                        y: 0,
                        width: 640,
                        height: 360,
                    },
                },
                witness: EpochFence::new(),
            },
            retention: EpochFence::new(),
            capture: EpochFence::new(),
            emergency: Arc::new(AtomicBool::new(false)),
        };
        (owner, frame, budget, control, geometry)
    }
    fn rejected(result: Result<RetainedFrame, (ImageError, FrameHandoff)>) -> (ImageError, FrameHandoff) {
        match result {
            Err(value) => value,
            Ok(_) => panic!("expected owned handoff rejection"),
        }
    }
    #[test]
    fn preview_handoff_owned_guard_rejection_keeps_exact_capture_and_credit_until_outside_store_drop() {
        for case in 0..8 {
            let (mut owner, mut frame, budget, control, geometry) = fixture();
            let mut supplied = control.clone();
            match case {
                0 => frame.emergency.store(true, Ordering::Release),
                1 => frame.retention.revoke(),
                2 => frame.capture.revoke(),
                3 => frame.original_capture.witness.revoke(),
                4 => supplied.witness = EpochFence::new(),
                5 => supplied.source = super::super::tests::source(2),
                6 => supplied.expires_at = Instant::now() - Duration::from_millis(1),
                7 => frame.original_capture.control.expires_at = Instant::now() - Duration::from_millis(1),
                _ => unreachable!(),
            }
            let token = frame.token;
            let received = frame.received_at;
            let claim = frame.claim();
            let deadline = frame.original_capture.control.expires_at;
            let retention = frame.retention.clone();
            let capture = frame.capture.clone();
            let original = frame.original_capture.witness.clone();
            let source_control = frame.original_capture.control.witness.clone();
            let emergency = frame.emergency.clone();
            let store = std::sync::Mutex::new(());
            let lock = store.lock().unwrap();
            let (error, frame) = rejected(frame.try_admit(&mut owner, &supplied, 2));
            assert_eq!(error, ImageError::Stale);
            assert_eq!(frame.token, token);
            assert_eq!(frame.received_at, received);
            assert_eq!(frame.claim(), claim);
            assert_eq!(frame.original_capture.control.expires_at, deadline);
            assert!(frame.retention.same(&retention));
            assert!(frame.capture.same(&capture));
            assert!(frame.original_capture.witness.same(&original));
            assert!(frame.original_capture.control.witness.same(&source_control));
            assert!(Arc::ptr_eq(&frame.emergency, &emergency));
            assert!(owner.cancel_capture(claim), "outer rejection never called/mutated the image model");
            assert_eq!(budget.used(), geometry.payload_bytes());
            assert!(
                budget.reserve(geometry).is_err(),
                "the rejected allocation has not been destroyed/refunded under Store lock"
            );
            let garbage = Some(frame);
            drop(lock);
            assert!(store.try_lock().is_ok());
            drop(garbage);
            assert_eq!(budget.used(), 0);
            assert!(budget.reserve(geometry).is_ok());
        }
    }
    #[test]
    fn preview_handoff_owned_model_rejection_returns_complete_handoff_for_exact_original_retry() {
        let (mut owner, frame, budget, control, geometry) = fixture_with_slots(2);
        let token = frame.token;
        let received = frame.received_at;
        let claim = frame.claim();
        let deadline = frame.original_capture.control.expires_at;
        let capture = frame.capture.clone();
        let retention = frame.retention.clone();
        let mut wrong = SourceImages::new(control.source, budget.clone()).unwrap();
        let policy = super::super::tests::state(control.clone(), Some(EpochFence::new())).policy;
        wrong.sync(policy, 0).unwrap();
        let wrong_claim = wrong.request(8, 1, geometry, 1).unwrap();
        let (error, frame) = rejected(frame.try_admit(&mut wrong, &control, 2));
        assert_eq!(error, ImageError::Stale);
        assert!(wrong.cancel_capture(wrong_claim));
        assert_eq!(frame.token, token);
        assert_eq!(frame.received_at, received);
        assert_eq!(frame.claim(), claim);
        assert_eq!(frame.original_capture.control.expires_at, deadline);
        assert!(frame.capture.same(&capture));
        assert!(frame.retention.same(&retention));
        assert_eq!(budget.used(), geometry.payload_bytes());
        let retained = match frame.try_admit(&mut owner, &control, 2) {
            Ok(frame) => frame,
            Err((error, _)) => panic!("original pending rejected: {error:?}"),
        };
        assert_eq!(retained.token, token);
        assert_eq!(
            retained.with_pixels(&control, 2, |bytes| bytes.len()),
            Some(geometry.payload_bytes())
        );
        drop(retained);
        assert_eq!(
            budget.used(),
            geometry.payload_bytes(),
            "model metadata still owns its conservative credit"
        );
        owner.retire();
        assert_eq!(budget.used(), 0);
    }
    #[test]
    fn preview_handoff_owned_recovery_does_not_refresh_original_expiry_and_legacy_wrapper_still_drops() {
        let (mut owner, mut frame, budget, control, geometry) = fixture();
        let expired = Instant::now() - Duration::from_millis(1);
        frame.original_capture.control.expires_at = expired;
        let (error, frame) = rejected(frame.try_admit(&mut owner, &control, 2));
        assert_eq!(error, ImageError::Stale);
        assert_eq!(frame.original_capture.control.expires_at, expired);
        assert_eq!(budget.used(), geometry.payload_bytes());
        let (error, frame) = rejected(frame.try_admit(&mut owner, &control, 2));
        assert_eq!(error, ImageError::Stale);
        assert_eq!(frame.original_capture.control.expires_at, expired);
        drop(frame);
        assert_eq!(budget.used(), 0);
        let (mut owner, frame, budget, control, _) = fixture();
        assert!(owner.cancel_capture(frame.claim()));
        let error = match frame.admit(&mut owner, &control, 2) {
            Err(error) => error,
            Ok(_) => panic!("consuming wrapper accepted cancelled claim"),
        };
        assert_eq!(error, ImageError::Stale);
        assert_eq!(budget.used(), 0);
    }
}
pub(crate) struct SourceUpdate {
    pub observation: Observation,
    pub frame: Option<FrameHandoff>,
}
pub(crate) struct HostUpdate {
    pub sources: Vec<SourceUpdate>,
    pub pending_roles: usize,
    pub connected_roles: usize,
    pub raw_bytes: usize,
}
pub(super) struct HostMailbox {
    pub permits: Vec<HostPermit>,
    pub revision: u64,
    pub commands: VecDeque<CommandOffer>,
    pub updates: Vec<SourceUpdate>,
    pub observations: Option<VersionedObservationSet>,
    pub diagnostics: [usize; 3],
}
impl HostMailbox {
    pub fn new() -> Self {
        Self {
            permits: Vec::with_capacity(MAX_WORKERS),
            revision: 1,
            commands: VecDeque::with_capacity(QUEUE * MAX_WORKERS),
            updates: Vec::with_capacity(MAX_WORKERS),
            observations: None,
            diagnostics: [0; 3],
        }
    }
}
