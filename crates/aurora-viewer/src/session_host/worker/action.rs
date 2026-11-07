//! Exact managed Active scope for native actions. This is independent of image
//! capture, renderer availability and the Review foreground requirement.
use super::{NativeAuthority, WorkerBridge};
use aurora_session::chat::ChatSource;
use std::time::{Duration, Instant};

/// Only this WorkerBridge child can mint a scope from its actually accepted
/// control state. Callers also retain their own App action/session epoch; a
/// later scope never extends or replaces permission on a captured action.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeActionScope {
    source: ChatSource,
    activation: u64,
    generation: u64,
    received_at: Instant,
    deadline: Instant,
}
impl std::fmt::Debug for NativeActionScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeActionScope([redacted])")
    }
}
impl NativeActionScope {
    pub fn source(self) -> ChatSource {
        self.source
    }
    pub fn activation(self) -> u64 {
        self.activation
    }
    pub fn generation(self) -> u64 {
        self.generation
    }
    pub fn deadline(self) -> Instant {
        self.deadline
    }
    fn capture(authority: &NativeAuthority, generation: u64, now: Instant) -> Option<Self> {
        let source = authority.current?;
        if generation == 0 || source.session.generation != generation || !authority.accepts(source, authority.activation, now) {
            return None;
        }
        let received_at = authority.last_receipt?;
        let deadline = received_at.checked_add(Duration::from_millis(aurora_session::LEASE_MILLIS))?;
        Some(Self {
            source,
            activation: authority.activation,
            generation,
            received_at,
            deadline,
        })
    }
    fn accepts(self, authority: &NativeAuthority, generation: u64, now: Instant) -> bool {
        self.received_at <= now
            && now < self.deadline
            && self.generation == generation
            && self.source.session.generation == generation
            && authority.accepts(self.source, self.activation, now)
    }
}

impl WorkerBridge {
    pub(crate) fn native_action_scope(&self) -> Option<NativeActionScope> {
        let authority = self.native_authority(self.window != 0 && self.status.native_window == self.window);
        NativeActionScope::capture(&authority, self.generation, Instant::now())
    }
    /// Recheck the ORIGINAL held scope at the effect boundary. A heartbeat may
    /// keep the current session alive but cannot renew this action's deadline.
    /// Managed None never implies permission in the ordinary standalone domain.
    pub(crate) fn native_action_current(&self, scope: NativeActionScope) -> bool {
        let authority = self.native_authority(self.window != 0 && self.status.native_window == self.window);
        scope.accepts(&authority, self.generation, Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::{Command, Message};

    fn accepted(now: Instant) -> NativeAuthority {
        // Use the real WorkerGuard accepted control envelope/source binding.
        // Native display/input readiness is synthetic; no window/GPU is run.
        let (mut guard, mut envelope) = super::super::tests::guard_and_command();
        envelope.message = Message::Command(Command::Heartbeat {
            workspace_foreground: false,
        });
        guard.on_command(&envelope, 1).unwrap();
        NativeAuthority {
            current: Some(ChatSource::try_from((guard.binding(), 1)).unwrap()),
            last_receipt: Some(now),
            activation: 7,
            managed: true,
            active: true,
            display_active: true,
            render_owned: true,
            input_owned: true,
            blocked: false,
            native_matches: true,
        }
    }

    #[test]
    fn native_action_scope_uses_real_accepted_binding_and_original_receipt_without_renderer_or_foreground() {
        let now = Instant::now();
        let authority = accepted(now);
        let scope = NativeActionScope::capture(&authority, 1, now).unwrap();
        assert_eq!(scope.source(), authority.current.unwrap());
        assert_eq!(scope.activation(), 7);
        assert_eq!(scope.generation(), 1);
        assert_eq!(scope.deadline(), now + Duration::from_millis(aurora_session::LEASE_MILLIS));
        assert!(scope.accepts(&authority, 1, now));
        assert!(NativeActionScope::capture(&authority, 0, now).is_none());
        assert!(NativeActionScope::capture(&authority, 2, now).is_none());
    }

    #[test]
    fn native_action_scope_rejects_every_incomplete_managed_grant_and_standby_fallback() {
        let now = Instant::now();
        let scope = NativeActionScope::capture(&accepted(now), 1, now).unwrap();
        for case in 0..11 {
            let mut authority = accepted(now);
            match case {
                0 => authority.current = None,
                1 => authority.managed = false, // detach / unsupported / not admitted
                2 => authority.active = false,  // Standby or Preparing
                3 => authority.display_active = false,
                4 => authority.render_owned = false,
                5 => authority.input_owned = false,
                6 => authority.blocked = true,
                7 => authority.native_matches = false,
                8 => authority.activation = 0,
                9 => authority.activation += 1,
                10 => authority.last_receipt = None,
                _ => unreachable!(),
            }
            if case == 9 {
                assert_eq!(NativeActionScope::capture(&authority, 1, now).unwrap().activation(), 8);
            } else {
                assert!(NativeActionScope::capture(&authority, 1, now).is_none(), "mint case {case}");
            }
            assert!(!scope.accepts(&authority, 1, now), "held scope case {case}");
        }
    }

    #[test]
    fn native_action_renewal_cannot_extend_held_scope_and_old_generation_or_source_never_substitutes() {
        let now = Instant::now();
        let authority = accepted(now);
        let scope = NativeActionScope::capture(&authority, 1, now).unwrap();
        let mut renewed = accepted(now + Duration::from_millis(100));
        assert!(scope.accepts(&renewed, 1, now + Duration::from_millis(100)));
        assert!(!scope.accepts(&renewed, 1, scope.deadline()));
        assert!(!scope.accepts(&authority, 1, now - Duration::from_nanos(1)));
        assert!(!scope.accepts(&authority, 2, now));
        renewed.current.as_mut().unwrap().lease_epoch += 1;
        assert!(!scope.accepts(&renewed, 1, now + Duration::from_millis(100)));
        let future = accepted(now + Duration::from_secs(1));
        assert!(NativeActionScope::capture(&future, 1, now).is_none());
    }
}
