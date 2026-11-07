//! Native owner of accepted image-source control proof. Capture authority stays
//! separate from this source lease and from ordinary hosting/chat fallbacks.
//! Combined-source dependency: reviewed image V4/sealed IO APIs and root-owned
//! production registration; this module supplies real callers, never IO stubs.

use super::{NativeAuthority, WorkerBridge};
use crate::session_host::preview_io::{ControlGuard, EpochFence, SourceLease, WorkerChannel};
use crate::session_host::transport::WorkerConfig;
use aurora_session::Command;
use aurora_session::preview_image::PreviewSource;
use std::time::Instant;

/// Only this WorkerBridge child can construct a token, from its actual accepted
/// managed Online authority. No UI/source/fence/guessed-expiry constructor.
#[derive(Clone)]
pub(crate) struct AcceptedNativePreviewSource {
    source: PreviewSource,
    received_at: Instant,
    expires_at: Instant,
    witness: EpochFence,
}
impl AcceptedNativePreviewSource {
    pub fn source(&self) -> PreviewSource {
        self.source
    }
    pub fn received_at(&self) -> Instant {
        self.received_at
    }
    pub fn expires_at(&self) -> Instant {
        self.expires_at
    }
    pub fn witness(&self) -> EpochFence {
        self.witness.clone()
    }
    pub fn current(&self) -> bool {
        self.current_at(Instant::now())
    }
    fn current_at(&self, now: Instant) -> bool {
        self.received_at <= now && now < self.expires_at && self.witness.current()
    }
}

#[derive(Default)]
struct SourceDomain {
    accepted: Option<AcceptedNativePreviewSource>,
}
impl SourceDomain {
    fn retire(&mut self) {
        if let Some(old) = self.accepted.take() {
            old.witness.revoke();
        }
    }
    fn observe(&mut self, authority: &NativeAuthority, now: Instant) -> Option<AcceptedNativePreviewSource> {
        let Some(scope) = authority.scope(now) else {
            self.retire();
            return None;
        };
        let received_at = authority.last_receipt?;
        let witness = if let Some(old) = self
            .accepted
            .as_ref()
            .filter(|old| old.source == scope.source && old.current_at(now))
        {
            old.witness.clone()
        } else {
            self.retire();
            EpochFence::new()
        };
        let accepted = AcceptedNativePreviewSource {
            source: scope.source,
            received_at,
            expires_at: scope.expires_at,
            witness,
        };
        self.accepted = Some(accepted.clone());
        Some(accepted)
    }
}

pub(super) struct Runtime {
    channel: WorkerChannel,
    source: SourceDomain,
    capture: CaptureDomain,
}

#[derive(Default)]
struct CaptureDomain {
    fence: Option<EpochFence>,
}
impl CaptureDomain {
    fn revoke(&self) {
        if let Some(fence) = &self.fence {
            fence.revoke();
        }
    }
    fn accepted_command(&self, command: &Command) {
        if !matches!(command, Command::Heartbeat { .. }) {
            self.revoke();
        }
    }
    fn install(&mut self, fence: Option<EpochFence>) -> bool {
        if fence.as_ref().is_some_and(|fence| !fence.current()) {
            self.revoke();
            return false;
        }
        if let Some(old) = &self.fence
            && fence.as_ref().is_none_or(|next| !old.same(next))
        {
            old.revoke();
        }
        self.fence = fence;
        true
    }
}
impl Runtime {
    pub(super) fn from_config(config: &WorkerConfig) -> Option<Self> {
        let bootstrap = config.preview.as_ref()?;
        match WorkerChannel::start(bootstrap.clone(), config.identity, config.lease_epoch) {
            Ok(channel) => Some(Self {
                channel,
                source: SourceDomain::default(),
                capture: CaptureDomain::default(),
            }),
            Err(_) => {
                log::warn!("worker image service could not start");
                None
            }
        }
    }
    fn revoke_capture(&self) {
        self.capture.revoke();
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.revoke_capture();
        self.source.retire();
        self.channel.stop(); // Nonblocking; the IO owner drops its own sockets.
    }
}

impl WorkerBridge {
    pub(super) fn preview_accepted_command(&mut self, command: &Command) {
        if let Some(runtime) = &self.preview {
            runtime.capture.accepted_command(command);
        }
        if matches!(command, Command::Detach) {
            self.retire_preview_source();
        }
    }
    pub(super) fn retire_preview_source(&mut self) {
        if let Some(runtime) = &mut self.preview {
            runtime.revoke_capture();
            runtime.source.retire();
            let _ = runtime.channel.set_source(None);
        }
    }
    /// Source renewals use the original accepted control receipt. This is also
    /// called after native pumping: temporary App bridge.take is never a loss.
    pub(crate) fn refresh_preview_source(&mut self) {
        if self.preview.is_none() {
            return;
        }
        let authority = self.native_authority(self.window != 0 && self.status.native_window == self.window);
        let runtime = self.preview.as_mut().expect("checked image runtime");
        let proof = runtime.source.observe(&authority, Instant::now());
        let lease = proof
            .as_ref()
            .and_then(|proof| ControlGuard::from_native(proof).ok())
            .map(|control| SourceLease { control });
        if lease.is_none() {
            runtime.revoke_capture();
        }
        // Failure is confined to this image lane. No hosting detach/chat loss.
        let _ = runtime.channel.set_source(lease);
    }
    pub(crate) fn preview_source_proof(&self) -> Option<AcceptedNativePreviewSource> {
        self.preview
            .as_ref()?
            .source
            .accepted
            .as_ref()
            .filter(|proof| proof.current())
            .cloned()
    }
    pub(crate) fn preview_channel(&self) -> Option<&WorkerChannel> {
        Some(&self.preview.as_ref()?.channel)
    }
    pub(crate) fn preview_capture_barrier(&self) {
        if let Some(runtime) = &self.preview {
            runtime.revoke_capture();
        }
    }
    pub(crate) fn register_preview_capture_fence(&mut self, capture: Option<EpochFence>) -> bool {
        let Some(runtime) = &mut self.preview else {
            return false;
        };
        if runtime.source.accepted.as_ref().is_none_or(|proof| !proof.current()) {
            runtime.revoke_capture();
            return false;
        }
        runtime.capture.install(capture)
    }
    pub(crate) fn disable_preview(&mut self) {
        drop(self.preview.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::{Command, Message};
    use std::time::Duration;

    fn accepted(now: Instant) -> NativeAuthority {
        let (mut guard, mut envelope) = super::super::tests::guard_and_command();
        envelope.message = Message::Command(Command::Heartbeat {
            workspace_foreground: false,
        });
        guard.on_command(&envelope, 1).unwrap();
        NativeAuthority {
            current: Some(aurora_session::chat::ChatSource::try_from((guard.binding(), 1)).unwrap()),
            last_receipt: Some(now),
            activation: guard.activation(),
            managed: true,
            active: false,
            display_active: false,
            render_owned: false,
            input_owned: false,
            blocked: false,
            native_matches: true,
        }
    }
    #[test]
    fn native_preview_source_proof_uses_original_accepted_receipt_and_retains_parked_domain() {
        let now = Instant::now();
        let mut facts = accepted(now);
        let mut domain = SourceDomain::default();
        let first = domain.observe(&facts, now).unwrap();
        let deadline = now + Duration::from_millis(aurora_session::LEASE_MILLIS);
        assert_eq!(first.received_at(), now);
        assert_eq!(first.expires_at(), deadline);
        assert!(first.current_at(now));
        facts.last_receipt = Some(now + Duration::from_millis(10));
        let renewed = domain.observe(&facts, now + Duration::from_millis(10)).unwrap();
        assert!(first.witness.same(&renewed.witness));
        assert_eq!(first.expires_at(), deadline, "held proof deadline never rejuvenates");
        assert_eq!(renewed.expires_at(), deadline + Duration::from_millis(10));
        assert!(!facts.active && !facts.display_active, "source proof does not grant capture");
        assert!(!first.current_at(deadline));
    }
    #[test]
    fn native_preview_source_loss_retires_held_proof_before_same_source_restoration() {
        let now = Instant::now();
        for loss in 0..4 {
            let mut facts = accepted(now);
            let mut domain = SourceDomain::default();
            let held = domain.observe(&facts, now).unwrap();
            match loss {
                0 => facts.managed = false,
                1 => facts.native_matches = false,
                2 => facts.current = None,
                _ => facts.last_receipt = None,
            }
            assert!(domain.observe(&facts, now).is_none());
            assert!(!held.current_at(now));
            let replacement = domain.observe(&accepted(now), now).unwrap();
            assert!(replacement.current_at(now));
            assert!(!held.current_at(now));
            assert!(!held.witness.same(&replacement.witness));
        }
    }
    #[test]
    fn native_preview_receipt_gap_or_future_clock_cannot_reuse_an_old_control_witness() {
        let now = Instant::now();
        let mut domain = SourceDomain::default();
        let held = domain.observe(&accepted(now), now).unwrap();
        let later = held.expires_at();
        let renewed = domain.observe(&accepted(later), later).unwrap();
        assert!(!held.witness.same(&renewed.witness));
        assert!(!held.current_at(now), "expired domain was permanently retired");
        assert!(domain.observe(&accepted(later + Duration::from_millis(1)), later).is_none());
        assert!(!renewed.current_at(later));
    }
    #[test]
    fn native_preview_control_guard_consumes_real_owner_token_without_renewing_its_expiry() {
        let now = Instant::now();
        let mut domain = SourceDomain::default();
        let proof = domain.observe(&accepted(now), now).unwrap();
        let guard = ControlGuard::from_native(&proof).unwrap();
        assert_eq!(guard.source(), proof.source());
        assert_eq!(guard.expires_at(), proof.expires_at());
        assert!(guard.current(Instant::now()));
        domain.retire();
        assert!(!guard.current(Instant::now()));
        assert!(ControlGuard::from_native(&proof).is_err());
    }
    #[test]
    fn native_preview_absent_bootstrap_never_constructs_a_channel_or_source_domain() {
        let (guard, _) = super::super::tests::guard_and_command();
        let binding = guard.binding();
        let config = WorkerConfig {
            version: aurora_session::PROTOCOL_VERSION,
            identity: binding.worker,
            session: binding.session.unwrap().session,
            generation: 1,
            slot: None,
            expected_account: None,
            expected_grid: None,
            lease_epoch: 1,
            host_pid: 1,
            chat: None,
            preview: None,
        };
        assert!(Runtime::from_config(&config).is_none());
    }
    #[test]
    fn native_preview_actual_accepted_prepare_retires_shared_capture_before_effects_but_keeps_source() {
        let now = Instant::now();
        let mut source = SourceDomain::default();
        let proof = source.observe(&accepted(now), now).unwrap();
        let mut capture = CaptureDomain::default();
        let shared = EpochFence::new();
        assert!(capture.install(Some(shared.clone())));
        let (mut guard, command) = super::super::tests::guard_and_command();
        let transition = guard.on_command(&command, 1).unwrap();
        let Message::Command(command) = &command.message else {
            unreachable!()
        };
        capture.accepted_command(command);
        assert!(transition.effects.prepare);
        assert!(
            !shared.current(),
            "App and IO's original Arc is retired before applying effects/replies"
        );
        assert!(proof.current_at(now), "normal preparation changes capture, not the source lease");
        assert!(!capture.install(Some(shared)), "a previously revoked capture cannot revive");
    }
    #[test]
    fn native_preview_capture_fence_renewal_is_exact_and_heartbeat_does_not_revoke() {
        let mut capture = CaptureDomain::default();
        let first = EpochFence::new();
        assert!(capture.install(Some(first.clone())));
        assert!(capture.install(Some(first.clone())));
        capture.accepted_command(&Command::Heartbeat {
            workspace_foreground: false,
        });
        assert!(first.current());
        let replacement = EpochFence::new();
        assert!(capture.install(Some(replacement.clone())));
        assert!(!first.current());
        assert!(replacement.current());
        capture.revoke();
        assert!(!replacement.current());
        assert!(capture.install(None));
    }
}
