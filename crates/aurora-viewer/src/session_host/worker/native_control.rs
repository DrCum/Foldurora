//! A bounded native observation lane survives ordinary detach long enough to
//! report the user's exit choice. IO never runs on the App/window thread.
use aurora_session::{Binding, native::*};
use std::time::{Duration, Instant};
const PREPARATION_TIMEOUT: Duration = Duration::from_secs(10);
const COMMIT_TIMEOUT: Duration = Duration::from_secs(2);
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct Control {
    snapshot: Arc<Mutex<Option<Observation>>>,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    revision: u64,
    last_request: u64,
    pending: Option<(Binding, NativeReceipt)>,
    exit_started: bool,
    deadline: Option<Instant>,
    background: BackgroundMode,
    trim: Option<(u64, bool)>,
}
impl Control {
    pub fn start(config: &super::WorkerConfig, directory: &std::path::Path) -> std::io::Result<Self> {
        let snapshot = Arc::new(Mutex::new(None::<Observation>));
        let latest = snapshot.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let ending = stop.clone();
        let failed = Arc::new(AtomicBool::new(false));
        let failure = failed.clone();
        let path = directory.join("native.json");
        std::thread::Builder::new()
            .name("session-native-observation".into())
            .spawn(move || {
                let mut published = 0;
                loop {
                    let observation = latest.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
                    if let Some(observation) = observation.filter(|observation| observation.revision > published) {
                        let result = serde_json::to_vec(&observation).map_err(std::io::Error::other).and_then(|bytes| {
                            if bytes.len() > 4096 {
                                return Err(std::io::Error::other("native observation exceeds bound"));
                            }
                            super::super::transport::publish(&path, &bytes)
                        });
                        if result.is_err() {
                            failure.store(true, Ordering::Release);
                            break;
                        }
                        published = observation.revision;
                    }
                    if ending.load(Ordering::Acquire) {
                        break;
                    }
                    std::thread::park_timeout(std::time::Duration::from_millis(50));
                }
            })?;
        let _ = config;
        Ok(Self {
            snapshot,
            stop,
            failed,
            revision: 0,
            last_request: 0,
            pending: None,
            exit_started: false,
            deadline: None,
            background: BackgroundMode::Warm,
            trim: None,
        })
    }

    pub fn observe(&mut self, binding: Binding, lease_epoch: u64, supported: bool, display: Option<PermittedDisplay>) {
        if self.failed.load(Ordering::Acquire) || binding.session.is_none() {
            return;
        }
        let Some(revision) = self.revision.checked_add(1) else {
            self.failed.store(true, Ordering::Release);
            return;
        };
        self.revision = revision;
        if self.pending.as_ref().is_some_and(|(source, _)| *source != binding) {
            self.pending = None;
            self.deadline = None;
            // Keep the ticket until pump_background can cancel it on the
            // owning renderer. Forgetting it here would strand its one slot.
        }
        let observation = Observation {
            version: VERSION,
            binding,
            lease_epoch,
            revision,
            capabilities: Capabilities {
                exit: supported,
                background: supported,
                display: true,
            },
            display,
            receipt: self.pending.as_ref().map(|(_, receipt)| *receipt),
        };
        *self.snapshot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(observation);
    }

    pub fn accept(&mut self, binding: Binding, request: NativeRequest, inactive: bool) -> bool {
        self.expire(Instant::now());
        let permitted = self
            .snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|observation| {
                observation.binding == binding
                    && match request.action {
                        NativeAction::RequestExit => observation.capabilities.exit,
                        NativeAction::Background(_) => observation.capabilities.background,
                        _ => false,
                    }
            });
        if !permitted
            || request.id <= self.last_request
            || self.failed.load(Ordering::Acquire)
            || binding.session.is_none()
            || self.pending.as_ref().is_some_and(|(_, receipt)| {
                matches!(
                    receipt.outcome,
                    NativeOutcome::Accepted
                        | NativeOutcome::WaitingForRestore
                        | NativeOutcome::ReadyForExit
                        | NativeOutcome::Committed
                        | NativeOutcome::PromptOpen
                )
            })
        {
            return false;
        }
        self.last_request = request.id;
        self.exit_started = false;
        let outcome = if matches!(request.action, NativeAction::Background(_)) && !inactive {
            NativeOutcome::Rejected
        } else {
            NativeOutcome::Accepted
        };
        self.pending = Some((binding, NativeReceipt { request, outcome }));
        self.deadline = (request.action == NativeAction::RequestExit).then(|| Instant::now() + PREPARATION_TIMEOUT);
        outcome == NativeOutcome::Accepted
    }

    pub fn expire(&mut self, now: Instant) {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.cancel_prepared();
        }
    }
    pub fn cancel_prepared(&mut self) {
        if let Some((_, receipt)) = &mut self.pending
            && receipt.request.action == NativeAction::RequestExit
            && matches!(
                receipt.outcome,
                NativeOutcome::Accepted | NativeOutcome::WaitingForRestore | NativeOutcome::ReadyForExit | NativeOutcome::Committed
            )
        {
            receipt.outcome = NativeOutcome::Cancelled;
            self.deadline = None;
        }
    }
    pub fn source_changed(&mut self) {
        self.cancel_prepared();
    }
    pub fn requires_io(&self) -> bool {
        self.pending.as_ref().is_some_and(|(_, receipt)| {
            receipt.request.action == NativeAction::RequestExit
                && matches!(
                    receipt.outcome,
                    NativeOutcome::Accepted | NativeOutcome::WaitingForRestore | NativeOutcome::ReadyForExit | NativeOutcome::Committed
                )
        })
    }
    pub fn restoring(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|(_, receipt)| receipt.outcome == NativeOutcome::WaitingForRestore)
    }
    pub fn can_restore(&mut self, binding: Binding, request: NativeRequest) -> bool {
        self.expire(Instant::now());
        request.action == NativeAction::RestoreExit
            && self.pending.as_ref().is_some_and(|(source, receipt)| {
                *source == binding
                    && receipt.request.id == request.id
                    && receipt.request.action == NativeAction::RequestExit
                    && receipt.outcome == NativeOutcome::Accepted
            })
    }
    pub fn restore_requested(&mut self) {
        self.outcome(NativeOutcome::WaitingForRestore);
    }
    pub fn restored(&mut self) {
        self.expire(Instant::now());
        if self.restoring() {
            self.outcome(NativeOutcome::ReadyForExit);
        } else {
            self.cancel_prepared();
        }
    }
    pub fn finalize(&mut self, binding: Binding, request: NativeRequest, restored: bool) -> bool {
        self.expire(Instant::now());
        let exact = self.pending.as_ref().is_some_and(|(source, receipt)| {
            *source == binding && receipt.request.id == request.id && receipt.request.action == NativeAction::RequestExit
        });
        if !exact {
            return false;
        }
        if request.action == NativeAction::CancelExit {
            self.cancel_prepared();
            return true;
        }
        if request.action != NativeAction::CommitExit
            || !restored
            || self
                .pending
                .as_ref()
                .is_none_or(|(_, receipt)| receipt.outcome != NativeOutcome::ReadyForExit)
        {
            return false;
        }
        self.outcome(NativeOutcome::Committed);
        self.deadline = Some(Instant::now() + COMMIT_TIMEOUT);
        true
    }
    pub fn take_exit(&mut self, restored: bool) -> bool {
        self.expire(Instant::now());
        if !restored
            || self.exit_started
            || self.pending.as_ref().is_none_or(|(_, receipt)| {
                receipt.request.action != NativeAction::RequestExit || receipt.outcome != NativeOutcome::Committed
            })
        {
            return false;
        }
        self.exit_started = true;
        self.outcome(NativeOutcome::PromptOpen);
        self.deadline = None;
        true
    }
    pub fn outcome(&mut self, outcome: NativeOutcome) {
        if let Some((_, receipt)) = &mut self.pending {
            receipt.outcome = outcome;
        }
    }
    pub fn logged_out(&mut self) {
        if self
            .pending
            .as_ref()
            .is_some_and(|(_, receipt)| receipt.outcome == NativeOutcome::LogoutChosen)
        {
            self.outcome(NativeOutcome::LoggedOut);
        }
    }
    pub fn prompt_open(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|(_, receipt)| receipt.outcome == NativeOutcome::PromptOpen)
    }
    pub fn trimming(&self) -> bool {
        self.trim.is_some()
            || self.pending.as_ref().is_some_and(|(_, receipt)| {
                receipt.request.action == NativeAction::Background(BackgroundMode::Economy) && receipt.outcome == NativeOutcome::Accepted
            })
    }

    fn retired_trim(&mut self, inactive: bool) -> Option<(u64, bool)> {
        let continuing = inactive
            && self.pending.as_ref().is_some_and(|(_, receipt)| {
                receipt.request.action == NativeAction::Background(BackgroundMode::Economy) && receipt.outcome == NativeOutcome::Accepted
            });
        if continuing { None } else { self.trim.take() }
    }

    pub fn pump_background(&mut self, renderer: &mut aurora_render::Renderer, ticket: &mut u64, inactive: bool) {
        if let Some((current, _)) = self.retired_trim(inactive) {
            renderer.cancel_session_fence(current);
            renderer.cancel_disposable_trim(current);
        }
        let Some((_, receipt)) = self.pending else {
            return;
        };
        if receipt.outcome != NativeOutcome::Accepted {
            return;
        }
        let NativeAction::Background(mode) = receipt.request.action else {
            return;
        };
        if !inactive {
            self.outcome(NativeOutcome::Rejected);
            return;
        }
        if mode == BackgroundMode::Warm {
            self.background = mode;
            self.outcome(NativeOutcome::Applied);
            return;
        }
        if self.trim.is_none() {
            let Some(next) = ticket.checked_add(1) else {
                self.outcome(NativeOutcome::Rejected);
                return;
            };
            *ticket = next;
            match renderer.request_session_fence(next) {
                Ok(()) => self.trim = Some((next, false)),
                Err(_) => self.outcome(NativeOutcome::Rejected),
            }
            return;
        }
        let Some((current, trimmed)) = self.trim else {
            return;
        };
        if !trimmed {
            match renderer.poll_session_fence(current) {
                Ok(Some(proof)) => match renderer.request_disposable_trim(proof) {
                    Ok(()) => self.trim = Some((current, true)),
                    Err(_) => self.outcome(NativeOutcome::Rejected),
                },
                Ok(None) => {}
                Err(_) => self.outcome(NativeOutcome::Rejected),
            }
        } else {
            match renderer.poll_disposable_trim(current) {
                Ok(true) => {
                    self.background = mode;
                    self.outcome(NativeOutcome::Applied);
                }
                Ok(false) => {}
                Err(_) => self.outcome(NativeOutcome::Rejected),
            }
        }
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::{AccountKey, GridKey, HostId, InstallationId, Label, SessionId, SessionIdentity, WorkerId, WorkerIdentity};
    fn fixture() -> (Control, Binding) {
        let binding = Binding {
            worker: WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).unwrap(),
                host: HostId::from_bytes([2; 16]).unwrap(),
                worker: WorkerId::from_bytes([3; 16]).unwrap(),
            },
            session: Some(SessionIdentity {
                session: SessionId::from_bytes([4; 16]).unwrap(),
                account: AccountKey::from_bytes([5; 32]).unwrap(),
                grid: GridKey::from_bytes([6; 32]).unwrap(),
                generation: 1,
            }),
        };
        let control = Control {
            snapshot: Arc::new(Mutex::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
            failed: Arc::new(AtomicBool::new(false)),
            revision: 0,
            last_request: 0,
            pending: None,
            exit_started: false,
            deadline: None,
            background: BackgroundMode::Warm,
            trim: None,
        };
        (control, binding)
    }
    #[test]
    fn native_exit_waits_for_actual_restore_and_keeps_cancel_authoritative() {
        let (mut control, binding) = fixture();
        let request = NativeRequest {
            id: 1,
            action: NativeAction::RequestExit,
        };
        assert!(!control.accept(binding, request, true), "no positive capability observation");
        control.observe(binding, 1, true, None);
        let mut stale = binding;
        stale.session.as_mut().unwrap().generation += 1;
        assert!(!control.accept(stale, request, true));
        assert!(control.accept(binding, request, true));
        assert!(
            !control.take_exit(false),
            "transport acknowledgment cannot open an invisible prompt"
        );
        assert!(
            !control.take_exit(true),
            "accepted preparation never opens native UI, even after ordinary restoration"
        );
        assert!(!control.finalize(
            binding,
            NativeRequest {
                id: 1,
                action: NativeAction::CommitExit
            },
            true
        ));
        assert!(control.can_restore(
            binding,
            NativeRequest {
                id: 1,
                action: NativeAction::RestoreExit
            }
        ));
        control.restore_requested();
        assert!(!control.take_exit(true));
        control.restored();
        assert!(control.requires_io());
        assert!(control.finalize(
            binding,
            NativeRequest {
                id: 1,
                action: NativeAction::CommitExit
            },
            true
        ));
        assert!(
            !control.finalize(
                binding,
                NativeRequest {
                    id: 1,
                    action: NativeAction::CommitExit
                },
                true
            ),
            "commit cannot replay"
        );
        assert!(control.take_exit(true));
        assert!(!control.take_exit(true));
        assert!(control.prompt_open());
        control.outcome(NativeOutcome::Cancelled);
        assert!(!control.prompt_open());
        assert!(!control.take_exit(true));
        assert!(!control.accept(binding, request, true), "replay cannot restore an approval");
    }
    #[test]
    fn native_display_withdrawal_replaces_same_binding_and_economy_is_not_receipt_only() {
        let (mut control, binding) = fixture();
        control.observe(
            binding,
            1,
            true,
            Some(PermittedDisplay {
                account: Label::new("Synthetic name").unwrap(),
                grid: Label::new("Synthetic grid").unwrap(),
            }),
        );
        let first = control.snapshot.lock().unwrap().clone().unwrap();
        control.observe(binding, 1, true, None);
        let second = control.snapshot.lock().unwrap().clone().unwrap();
        assert_eq!(first.binding, second.binding);
        assert!(second.revision > first.revision && second.display.is_none());
        assert!(!control.accept(
            binding,
            NativeRequest {
                id: 1,
                action: NativeAction::Background(BackgroundMode::Economy)
            },
            false
        ));
        assert_eq!(control.pending.unwrap().1.outcome, NativeOutcome::Rejected);
        assert!(control.accept(
            binding,
            NativeRequest {
                id: 2,
                action: NativeAction::Background(BackgroundMode::Economy)
            },
            true
        ));
        assert!(control.trimming());
        assert_eq!(
            control.pending.unwrap().1.outcome,
            NativeOutcome::Accepted,
            "only real fence plus render trim completion can produce Applied"
        );
    }
    #[test]
    fn interrupted_economy_retires_exact_old_trim_before_another_source_uses_the_slot() {
        let (mut control, binding) = fixture();
        control.observe(binding, 1, true, None);
        assert!(control.accept(
            binding,
            NativeRequest {
                id: 1,
                action: NativeAction::Background(BackgroundMode::Economy)
            },
            true
        ));
        control.trim = Some((41, true));
        assert_eq!(
            control.retired_trim(true),
            None,
            "an inactive, accepted request still owns its trim"
        );
        assert_eq!(
            control.retired_trim(false),
            Some((41, true)),
            "promotion returns the exact in-flight ticket for cancellation"
        );
        assert_eq!(control.retired_trim(false), None);
        control.trim = Some((42, false));
        let mut replacement = binding;
        replacement.session.as_mut().unwrap().generation += 1;
        control.observe(replacement, 1, true, None);
        assert!(control.pending.is_none());
        assert!(control.trimming(), "orphaned fence stays owned until renderer-thread cancellation");
        assert_eq!(control.retired_trim(true), Some((42, false)));
        assert!(!control.trimming());
    }
    fn ready_exit() -> (Control, Binding) {
        let (mut control, binding) = fixture();
        control.observe(binding, 1, true, None);
        assert!(control.accept(
            binding,
            NativeRequest {
                id: 1,
                action: NativeAction::RequestExit
            },
            true
        ));
        assert!(control.can_restore(
            binding,
            NativeRequest {
                id: 1,
                action: NativeAction::RestoreExit
            }
        ));
        control.restore_requested();
        control.restored();
        assert_eq!(control.pending.unwrap().1.outcome, NativeOutcome::ReadyForExit);
        (control, binding)
    }
    #[test]
    fn cancelled_or_expired_preparation_never_opens_on_later_detach_or_shutdown() {
        for stage in ["accepted", "restoring", "ready", "expired", "source_changed"] {
            let (mut control, binding) = fixture();
            control.observe(binding, 1, true, None);
            assert!(control.accept(
                binding,
                NativeRequest {
                    id: 1,
                    action: NativeAction::RequestExit
                },
                true
            ));
            if matches!(stage, "restoring" | "ready" | "expired" | "source_changed") {
                control.restore_requested();
            }
            if matches!(stage, "ready" | "expired" | "source_changed") {
                control.restored();
            }
            match stage {
                "expired" => control.expire(control.deadline.unwrap() + Duration::from_nanos(1)),
                "source_changed" => control.source_changed(),
                _ => {
                    assert!(control.finalize(
                        binding,
                        NativeRequest {
                            id: 1,
                            action: NativeAction::CancelExit
                        },
                        false
                    ));
                }
            }
            control.restored(); // later ordinary detach / host shutdown
            assert!(!control.take_exit(true), "{stage}");
            assert!(!control.requires_io(), "{stage}");
            assert!(
                !control.finalize(
                    binding,
                    NativeRequest {
                        id: 1,
                        action: NativeAction::CommitExit
                    },
                    true
                ),
                "late {stage}"
            );
        }
        let (mut control, binding) = fixture();
        control.observe(binding, 1, true, None);
        assert!(control.accept(
            binding,
            NativeRequest {
                id: 1,
                action: NativeAction::RequestExit
            },
            true
        ));
        control.restored(); // an ordinary Detach is not native RestoreExit
        assert!(!control.take_exit(true));
        assert!(!control.requires_io());
    }
    #[test]
    fn final_commit_requires_exact_restored_source_and_expires_before_delayed_consumption() {
        let (mut control, binding) = ready_exit();
        let commit = NativeRequest {
            id: 1,
            action: NativeAction::CommitExit,
        };
        let mut replacement = binding;
        replacement.session.as_mut().unwrap().generation += 1;
        assert!(!control.finalize(replacement, commit, true));
        assert!(!control.finalize(binding, NativeRequest { id: 2, ..commit }, true));
        assert!(!control.finalize(binding, commit, false));
        assert!(control.finalize(binding, commit, true));
        control.expire(control.deadline.unwrap() + Duration::from_nanos(1));
        assert!(!control.take_exit(true));
        assert!(!control.finalize(binding, commit, true));
    }
}
