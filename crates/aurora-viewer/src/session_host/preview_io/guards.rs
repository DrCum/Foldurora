//! Readonly capability consumers. Actual native/Broker producers must mint
//! witnesses only after their exact accepted-source/grant/physical checks.
use super::*;
use aurora_render::viewport_capture::{CapturePublicationGuard, CaptureRetentionGuard, CapturedViewport};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone)]
pub(crate) struct EpochFence(Arc<AtomicU64>);
impl EpochFence {
    pub fn new() -> Self {
        Self(Arc::new(AtomicU64::new(1)))
    }
    pub(super) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn revoke(&self) {
        self.0.store(0, Ordering::Release);
    }
    pub(crate) fn current(&self) -> bool {
        self.0.load(Ordering::Acquire) == 1
    }
}
/// Original control receipt deadline plus exact full source. This is not a
/// roster lookup: only the later accepted-control owner may supply that proof.
#[derive(Clone)]
pub(crate) struct ControlGuard {
    pub source: PreviewSource,
    pub expires_at: Instant,
    pub witness: EpochFence,
}
impl ControlGuard {
    pub fn current(&self, now: Instant) -> bool {
        self.expires_at > now && self.witness.current()
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

/// The later Broker mints this only from exact accepted ActiveViewport+Granted
/// control state. Its capture-only fence never gates already-admitted history.
#[derive(Clone)]
pub(crate) struct CaptureControlGuard {
    pub control: ControlGuard,
    pub activation: u64,
    pub viewport: aurora_session::HostedViewport,
    pub witness: EpochFence,
}
impl CaptureControlGuard {
    pub fn current(&self, claim: CaptureClaim, now: Instant) -> bool {
        self.control.current(now)
            && self.witness.current()
            && self.activation != 0
            && claim.source() == self.control.source
            && claim.activation() == self.activation
            && self.viewport.validate().is_ok()
            && claim.geometry().source_size() == [self.viewport.rect.width, self.viewport.rect.height]
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
        assert!(active.current(claim, Instant::now()));
        active.activation = 2;
        assert!(!active.current(claim, Instant::now()));
        active.activation = 1;
        active.control.source = super::super::tests::source(2);
        assert!(!active.current(claim, Instant::now()));
        active.control = c;
        active.witness.revoke();
        assert!(!active.current(claim, Instant::now()));
    }
}
