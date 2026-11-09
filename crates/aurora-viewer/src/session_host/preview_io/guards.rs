//! Readonly capability consumers. Actual native/Broker producers must mint
//! witnesses only after their exact accepted-source/grant/physical checks.
use super::*;
use aurora_render::viewport_capture::{CapturePublicationGuard, CaptureRetentionGuard, CapturedViewport};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone)]
pub(crate) struct EpochFence {
    local: Arc<std::sync::atomic::AtomicBool>,
    generation: Option<(Arc<AtomicU64>, u64)>,
}
impl EpochFence {
    pub fn new() -> Self {
        Self {
            local: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            generation: None,
        }
    }
    /// The counter belongs to the synchronous front door. IO may revoke its
    /// own clone permanently, but must never reset or write that owner's counter.
    pub fn generation(counter: Arc<AtomicU64>, expected: u64) -> Self {
        Self {
            local: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            generation: Some((counter, expected)),
        }
    }
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.local, &other.local)
            && match (&self.generation, &other.generation) {
                (None, None) => true,
                (Some((a, x)), Some((b, y))) => x == y && Arc::ptr_eq(a, b),
                _ => false,
            }
    }
    pub fn revoke(&self) {
        self.local.store(false, Ordering::Release);
    }
    pub(crate) fn current(&self) -> bool {
        self.local.load(Ordering::Acquire)
            && self
                .generation
                .as_ref()
                .is_none_or(|(counter, expected)| *expected != 0 && counter.load(Ordering::Acquire) == *expected)
    }
}
impl PartialEq for EpochFence {
    fn eq(&self, other: &Self) -> bool {
        self.same(other)
    }
}
impl Eq for EpochFence {}
/// Original control receipt deadline plus exact full source. This is not a
/// roster lookup: only the later accepted-control owner may supply that proof.
#[derive(Clone)]
pub(crate) struct ControlGuard {
    pub(super) source: PreviewSource,
    pub(super) expires_at: Instant,
    pub(super) witness: EpochFence,
}
impl ControlGuard {
    pub fn from_host(token: &crate::session_host::transport::preview_control::AcceptedPreviewSource) -> Result<Self, Error> {
        if !token.current(Instant::now()) {
            return Err(Error::Expired);
        }
        Ok(Self {
            source: token.source(),
            expires_at: token.expires_at(),
            witness: token.witness(),
        })
    }
    pub fn from_native(token: &crate::session_host::worker::AcceptedNativePreviewSource) -> Result<Self, Error> {
        let now = Instant::now();
        if !token.current() || token.received_at() > now || token.expires_at() <= token.received_at() {
            return Err(Error::Expired);
        }
        Ok(Self {
            source: token.source(),
            expires_at: token.expires_at(),
            witness: token.witness(),
        })
    }
    pub fn source(&self) -> PreviewSource {
        self.source
    }
    pub fn expires_at(&self) -> Instant {
        self.expires_at
    }
    pub fn same_domain(&self, other: &Self) -> bool {
        self.source() == other.source() && self.witness.same(&other.witness)
    }

    pub fn current(&self, now: Instant) -> bool {
        self.expires_at() > now && self.witness.current()
    }
}
#[derive(Clone)]
enum Physical {
    Renderer(CaptureRetentionGuard),
    // Source-only fixture, never a real native/render permit.
    #[cfg(test)]
    Fixture(EpochFence),
}
#[derive(Clone)]
pub(crate) struct SourceRetentionGuard {
    pub(super) control: ControlGuard,
    pub(super) policy: PreviewPolicy,
    expires_at: Instant,
    native: EpochFence,
    physical: Physical,
}
impl SourceRetentionGuard {
    pub fn new(
        control: ControlGuard,
        policy: PreviewPolicy,
        expires_at: Instant,
        native: EpochFence,
        physical: CaptureRetentionGuard,
    ) -> Result<Self, Error> {
        if control.source != policy.source() {
            return Err(Error::Binding);
        }
        Ok(Self {
            control,
            policy,
            expires_at,
            native,
            physical: Physical::Renderer(physical),
        })
    }
    pub(super) fn native_deadline(&self) -> Instant {
        self.expires_at
    }
    pub fn current(&self, now: Instant) -> bool {
        self.control.current(now)
            && self.expires_at > now
            && self.native.current()
            && self.policy.access().retain_last_active
            && match &self.physical {
                Physical::Renderer(p) => p.current(),
                #[cfg(test)]
                Physical::Fixture(p) => p.current(),
            }
    }
    #[cfg(test)]
    pub(super) fn fixture(control: ControlGuard, policy: PreviewPolicy, expires_at: Instant) -> Self {
        Self {
            control,
            policy,
            expires_at,
            native: EpochFence::new(),
            physical: Physical::Fixture(EpochFence::new()),
        }
    }
}
#[derive(Clone)]
pub(crate) struct CapturePermitGuard {
    pub(super) retention: SourceRetentionGuard,
    pub(super) token: ConnectionToken,
    pub(super) claim: CaptureClaim,
    pub(super) publication: CapturePublicationGuard,
    pub(super) native: EpochFence,
    expires_at: Instant,
    io: Option<(EpochFence, Arc<std::sync::atomic::AtomicBool>)>,
}
impl CapturePermitGuard {
    pub fn new(
        retention: SourceRetentionGuard,
        token: ConnectionToken,
        claim: CaptureClaim,
        publication: CapturePublicationGuard,
        native: EpochFence,
        expires_at: Instant,
    ) -> Result<Self, Error> {
        if token.source != claim.source()
            || token.connection != claim.connection()
            || !claim.matches_policy(retention.policy)
            || token.pixel_epoch == 0
        {
            return Err(Error::Binding);
        }
        Ok(Self {
            retention,
            token,
            claim,
            publication,
            native,
            expires_at,
            io: None,
        })
    }
    pub fn current(&self, now: Instant) -> bool {
        self.retention.current(now) && self.expires_at > now && self.native.current() && self.publication.current()
    }
    pub(super) fn bind_io(mut self: Box<Self>, gate: EpochFence, emergency: Arc<std::sync::atomic::AtomicBool>) -> Box<Self> {
        self.io = Some((gate, emergency));
        self
    }
    pub fn transport_check(&self) -> Result<(), Error> {
        let now = Instant::now();
        if !self.retention.current(now)
            || self.expires_at <= now
            || self.publication.deadline() <= now
            || self.io.as_ref().is_some_and(|(_, emergency)| emergency.load(Ordering::Acquire))
        {
            Err(Error::Expired)
        } else if !self.current(now) || self.io.as_ref().is_some_and(|(gate, _)| !gate.current()) {
            Err(Error::CaptureRevoked)
        } else {
            Ok(())
        }
    }
    pub(crate) fn accepts(&self, stamp: PreviewStamp, frame: &CapturedViewport, now: Instant) -> bool {
        self.current(now) && stamp.claim() == self.claim && self.publication.token() == frame.request().token()
    }
}

/// Broker-owned active grant or independently eligible inactive source. The
/// explicit kind cannot cross lanes; its fence never gates admitted history.
#[derive(Clone)]
pub(crate) struct CaptureControlGuard {
    pub(super) control: ControlGuard,
    pub(super) mode: crate::session_host::transport::preview_control::CaptureMode,
    pub(super) witness: EpochFence,
}
impl CaptureControlGuard {
    pub fn from_host(token: &crate::session_host::transport::preview_control::AcceptedPreviewCapture) -> Result<Self, Error> {
        if !token.current(Instant::now()) {
            return Err(Error::Expired);
        }
        Ok(Self {
            control: ControlGuard::from_host(token.source())?,
            mode: token.mode(),
            witness: token.witness(),
        })
    }
    pub fn control(&self) -> &ControlGuard {
        &self.control
    }
    pub fn source(&self) -> PreviewSource {
        self.control.source
    }

    pub fn current(&self, claim: CaptureClaim, now: Instant) -> bool {
        self.control().current(now)
            && self.witness.current()
            && claim.source() == self.source()
            && claim.kind() == self.mode.kind()
            && match self.mode {
                crate::session_host::transport::preview_control::CaptureMode::Active { activation, viewport } => {
                    activation != 0
                        && claim.activation() == activation
                        && viewport.validate().is_ok()
                        && claim.geometry().source_size() == [viewport.rect.width, viewport.rect.height]
                }
                crate::session_host::transport::preview_control::CaptureMode::Inactive => true,
            }
    }
}

/// Per-operation IO fence. Revocation is visible while a packet is fragmented,
/// before the next header/payload allocation/read/write, not just at handoff.
#[derive(Clone)]
pub(crate) struct IoSourceGuard {
    pub control: ControlGuard,
    pub retention: EpochFence,
    pub emergency: Arc<std::sync::atomic::AtomicBool>,
    pub native: Option<SourceRetentionGuard>,
    pub policy_expires: Option<Instant>,
}
impl IoSourceGuard {
    pub fn current(&self) -> bool {
        let now = Instant::now();
        self.control.current(now)
            && self.retention.current()
            && !self.emergency.load(Ordering::Acquire)
            && self.policy_expires.is_none_or(|end| end > now)
            && self.native.as_ref().is_none_or(|guard| guard.current(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_io_physical_retention_loss_without_any_pending_frame_is_permanent() {
        let _retention: fn(
            ControlGuard,
            PreviewPolicy,
            Instant,
            EpochFence,
            aurora_render::viewport_capture::CaptureRetentionGuard,
        ) -> Result<SourceRetentionGuard, Error> = SourceRetentionGuard::new;
        let _capture: fn(
            SourceRetentionGuard,
            ConnectionToken,
            CaptureClaim,
            aurora_render::viewport_capture::CapturePublicationGuard,
            EpochFence,
            Instant,
        ) -> Result<CapturePermitGuard, Error> = CapturePermitGuard::new;
        let _enqueue: fn(&WorkerChannel, CapturePermitGuard, PreviewStamp, CapturedViewport) -> Result<(), Error> =
            WorkerChannel::try_frame;
        let c = super::super::tests::control(super::super::tests::source(1));
        let state = super::super::tests::state(c.clone(), None);
        let guard = state.retention;
        assert!(guard.current(Instant::now()));
        let Physical::Fixture(physical) = &guard.physical else {
            panic!("private fixture")
        };
        physical.revoke();
        assert!(!guard.current(Instant::now()));
        assert!(!guard.clone().current(Instant::now()));
        let replacement = SourceRetentionGuard::fixture(c, guard.policy, Instant::now() + DEADLINE);
        assert!(replacement.current(Instant::now()));
        assert!(!guard.current(Instant::now()));
    }
    #[test]
    fn preview_io_source_and_capture_control_proofs_never_use_labels_or_an_active_worker_fallback() {
        let s = super::super::tests::source(1);
        let c = super::super::tests::control(s);
        let policy = super::super::tests::state(c.clone(), Some(EpochFence::new())).policy;
        let budget = ImageBudget::new(1024 * 1024).unwrap();
        let mut images = aurora_session::preview_image::SourceImages::new(s, budget).unwrap();
        images.sync(policy, 0).unwrap();
        let geometry = aurora_session::preview_image::PreviewGeometry::new([640, 360], [0, 0, 640, 360], PreviewSize::Small).unwrap();
        let claim = images.request(1, 1, geometry, 1).unwrap();
        let mut active = CaptureControlGuard {
            control: c.clone(),
            mode: crate::session_host::transport::preview_control::CaptureMode::Active {
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
            },
            witness: EpochFence::new(),
        };
        assert!(active.current(claim, Instant::now()));
        let original = active.mode;
        if let crate::session_host::transport::preview_control::CaptureMode::Active { activation, .. } = &mut active.mode {
            *activation = 2;
        }
        assert!(!active.current(claim, Instant::now()));
        active.mode = original;
        active.control.source = super::super::tests::source(2);
        assert!(!active.current(claim, Instant::now()));
        active.control = c;
        active.witness.revoke();
        assert!(!active.current(claim, Instant::now()));
    }
    #[test]
    fn live_preview_io_inactive_guard_cannot_borrow_active_authority_or_revive_after_retirement() {
        use aurora_session::preview_image::{ImageBudget, PreviewAccess, PreviewGeometry, PreviewPolicy, SourceImages};
        let source = super::super::tests::source(3);
        let control = super::super::tests::control(source);
        let active = super::super::tests::state(control.clone(), Some(EpochFence::new())).policy;
        let mut model = SourceImages::new(source, ImageBudget::new(1024 * 1024).unwrap()).unwrap();
        model.sync(active, 0).unwrap();
        let geometry = PreviewGeometry::new([640, 360], [0, 0, 640, 360], aurora_session::PreviewSize::Small).unwrap();
        let active_claim = model.request(1, 1, geometry, 1).unwrap();
        let guard = CaptureControlGuard {
            control,
            mode: crate::session_host::transport::preview_control::CaptureMode::Inactive,
            witness: EpochFence::new(),
        };
        assert!(!guard.current(active_claim, Instant::now()));
        let inactive = PreviewPolicy::new(
            source,
            active.epochs(),
            PreviewAccess {
                active: false,
                activation: 0,
                capture_active: false,
                capture_inactive: true,
                ..active.access()
            },
        )
        .unwrap();
        // A fresh never-activated domain keeps the active watermark monotonic.
        let mut model = SourceImages::new(source, ImageBudget::new(1024 * 1024).unwrap()).unwrap();
        model.sync(inactive, 0).unwrap();
        let claim = model.request_inactive(1, 1, geometry, 1).unwrap();
        assert!(guard.current(claim, Instant::now()));
        guard.witness.revoke();
        assert!(!guard.current(claim, Instant::now()));
        assert!(!guard.current(active_claim, Instant::now()));
    }
}

#[cfg(test)]
mod generation_tests {
    use super::*;
    #[test]
    fn preview_io_generation_fence_local_revoke_never_resets_owner_or_revives_earlier_generation() {
        let counter = Arc::new(AtomicU64::new(1));
        let old = EpochFence::generation(counter.clone(), 1);
        let held = old.clone();
        assert!(old.current());
        assert!(old.same(&held));
        counter.store(2, Ordering::Release);
        let new = EpochFence::generation(counter.clone(), 2);
        assert!(!old.current());
        assert!(new.current());
        assert!(!old.same(&new));
        old.revoke();
        assert_eq!(counter.load(Ordering::Acquire), 2);
        assert!(new.current());
        new.revoke();
        assert_eq!(counter.load(Ordering::Acquire), 2);
        assert!(!new.current());
        counter.store(3, Ordering::Release);
        assert!(!old.current());
        assert!(!held.current());
        assert!(!new.current());
        assert!(!EpochFence::generation(counter.clone(), 0).current());
        counter.store(0, Ordering::Release);
        assert!(!EpochFence::generation(counter, 3).current());
        let c = super::super::tests::control(super::super::tests::source(1));
        assert_eq!(c.source(), super::super::tests::source(1));
        assert_eq!(c.expires_at(), c.expires_at);
        assert!(c.same_domain(&c.clone()));
        assert!(!c.same_domain(&super::super::tests::control(c.source())));
    }
    #[test]
    fn preview_io_transport_fixture_is_only_exact_metadata_correlation_not_pixel_or_source_permission() {
        let source = super::super::tests::source(1);
        let metadata = ConnectionToken::transport_fixture(source, 1, 0, 1).unwrap();
        let pixels = ConnectionToken::transport_fixture(source, 1, 2, 1).unwrap();
        assert!(metadata.same_metadata(pixels));
        assert!(metadata != pixels);
        assert!(!metadata.same_metadata(ConnectionToken::transport_fixture(source, 2, 0, 1).unwrap()));
        assert!(!metadata.same_metadata(ConnectionToken::transport_fixture(source, 1, 0, 2).unwrap()));
        assert!(!metadata.same_metadata(ConnectionToken::transport_fixture(super::super::tests::source(2), 1, 0, 1).unwrap()));
        assert!(ConnectionToken::transport_fixture(source, 0, 0, 1).is_none());
        assert!(ConnectionToken::transport_fixture(source, 1, 0, 0).is_none());
    }
}
