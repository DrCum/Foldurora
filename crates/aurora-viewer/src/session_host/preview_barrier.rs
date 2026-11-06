//! Pure cohort suspension barrier before the broker releases a held M1 intent.
//! The broker supplies every ever-authorized live participant and reconciles
//! that exact cohort while waiting. Only an authenticated Suspended message
//! backed by the owning bridge's actual ordered GPU completion may become Ack.
//! IO cancellation, image/frame progress, grants and timeouts are not proofs.

use super::preview_protocol::{MAX_SUSPEND_CHALLENGE, PreviewSource};
use aurora_session::MAX_WORKERS;
use std::time::{Duration, Instant};

pub(crate) const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Participant {
    pub source: PreviewSource,
    pub device_epoch: u64,
}

impl Participant {
    fn valid(self) -> bool {
        self.device_epoch != 0 && self.source.validate().is_ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    pub participant: Participant,
    pub challenge: u64,
}

/// This is a caller-supplied GPU suspension proof, never synthesized by IO.
/// None is a completed queue fence before any native frame was presented;
/// completed_frame itself neither implies a new frame nor grants readiness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ack {
    pub intent_id: u64,
    pub request: Request,
    pub surface_epoch: u64,
    pub completed_frame: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    InvalidIntent,
    InvalidParticipant,
    TooManyParticipants,
    DuplicateWorker,
    Exhausted,
    Timeout,
    ParticipantLost,
    ParticipantChanged,
    CohortChanged,
    StaleIntent,
    StaleAck,
    InvalidProof,
    ConflictingAck,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Idle,
    Pending { intent_id: u64, remaining: usize },
    Ready { intent_id: u64 },
    Failed { intent_id: u64, error: Error },
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Proof {
    surface_epoch: u64,
    completed_frame: Option<u64>,
}

struct Pending {
    request: Request,
    proof: Option<Proof>,
}

struct Active {
    intent_id: u64,
    deadline: Instant,
    participants: Vec<Pending>,
}

enum Phase {
    Idle,
    Active(Active),
    Failed { intent_id: u64, error: Error },
}

pub(crate) struct Barrier {
    phase: Phase,
    // Next unused challenge. Zero is terminal exhaustion, never an initial
    // value; cancel, failure, source replacement and retry cannot reset it.
    next_challenge: u64,
}

impl Default for Barrier {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            next_challenge: 1,
        }
    }
}

fn validate(participants: &[Participant]) -> Result<(), Error> {
    if participants.len() > MAX_WORKERS {
        return Err(Error::TooManyParticipants);
    }
    for (index, participant) in participants.iter().enumerate() {
        if !participant.valid()
            || participants.first().is_some_and(|first| {
                (first.source.worker.installation, first.source.worker.host)
                    != (participant.source.worker.installation, participant.source.worker.host)
            })
        {
            return Err(Error::InvalidParticipant);
        }
        if participants[..index]
            .iter()
            .any(|previous| previous.source.worker == participant.source.worker)
        {
            return Err(Error::DuplicateWorker);
        }
    }
    Ok(())
}

impl Barrier {
    pub fn begin(&mut self, intent_id: u64, participants: &[Participant], now: Instant) -> Result<Vec<Request>, Error> {
        // A rejected replacement must not leave the previous target ready.
        self.phase = Phase::Idle;
        let result = self.begin_inner(intent_id, participants, now);
        if let Err(error) = result {
            self.phase = Phase::Failed { intent_id, error };
        }
        result
    }

    fn begin_inner(&mut self, intent_id: u64, participants: &[Participant], now: Instant) -> Result<Vec<Request>, Error> {
        if intent_id == 0 {
            return Err(Error::InvalidIntent);
        }
        validate(participants)?;
        if self.next_challenge == 0 || self.next_challenge > MAX_SUSPEND_CHALLENGE {
            self.next_challenge = 0;
            return Err(Error::Exhausted);
        }
        let deadline = now.checked_add(TIMEOUT).ok_or(Error::Timeout)?;
        if !participants.is_empty()
            && self
                .next_challenge
                .checked_add(participants.len() as u64 - 1)
                .is_none_or(|last| last > MAX_SUSPEND_CHALLENGE)
        {
            self.next_challenge = 0;
            return Err(Error::Exhausted);
        }
        let mut pending = Vec::with_capacity(participants.len());
        let mut requests = Vec::with_capacity(participants.len());
        for participant in participants {
            let request = Request {
                participant: *participant,
                challenge: self.next_challenge,
            };
            self.next_challenge = self
                .next_challenge
                .checked_add(1)
                .filter(|next| *next <= MAX_SUSPEND_CHALLENGE)
                .unwrap_or(0);
            pending.push(Pending { request, proof: None });
            requests.push(request);
        }
        self.phase = Phase::Active(Active {
            intent_id,
            deadline,
            participants: pending,
        });
        Ok(requests)
    }

    pub fn tick(&mut self, now: Instant) -> Status {
        if let Phase::Active(active) = &self.phase {
            if now >= active.deadline {
                self.phase = Phase::Failed {
                    intent_id: active.intent_id,
                    error: Error::Timeout,
                };
            }
        }
        match &self.phase {
            Phase::Idle => Status::Idle,
            Phase::Failed { intent_id, error } => Status::Failed {
                intent_id: *intent_id,
                error: *error,
            },
            Phase::Active(active) => {
                let remaining = active.participants.iter().filter(|participant| participant.proof.is_none()).count();
                if remaining == 0 {
                    Status::Ready {
                        intent_id: active.intent_id,
                    }
                } else {
                    Status::Pending {
                        intent_id: active.intent_id,
                        remaining,
                    }
                }
            }
        }
    }

    fn active(&mut self, intent_id: u64, now: Instant) -> Result<&mut Active, Error> {
        self.tick(now);
        match &mut self.phase {
            Phase::Active(active) if active.intent_id == intent_id => Ok(active),
            Phase::Failed { intent_id: failed, error } if *failed == intent_id => Err(*error),
            _ => Err(Error::StaleIntent),
        }
    }

    /// True only for the first exact acknowledgement. Duplicate identical
    /// proofs are harmless, while changed or stale proofs never advance state.
    pub fn observe(&mut self, ack: Ack, now: Instant) -> Result<bool, Error> {
        let active = self.active(ack.intent_id, now)?;
        if ack.surface_epoch == 0 {
            return Err(Error::InvalidProof);
        }
        let participant = active
            .participants
            .iter_mut()
            .find(|pending| pending.request == ack.request)
            .ok_or(Error::StaleAck)?;
        let proof = Proof {
            surface_epoch: ack.surface_epoch,
            completed_frame: ack.completed_frame,
        };
        match participant.proof {
            Some(previous) if previous == proof => Ok(false),
            Some(_) => Err(Error::ConflictingAck),
            None => {
                participant.proof = Some(proof);
                Ok(true)
            }
        }
    }

    /// Reconcile all live members, including after all acks but before take.
    /// Order is irrelevant; disappearance never counts as GPU completion.
    pub fn reconcile(&mut self, intent_id: u64, participants: &[Participant], now: Instant) -> Result<(), Error> {
        let active = self.active(intent_id, now)?;
        let result = validate(participants).and_then(|()| {
            for pending in &active.participants {
                let current = participants
                    .iter()
                    .find(|participant| participant.source.worker == pending.request.participant.source.worker)
                    .ok_or(Error::ParticipantLost)?;
                if *current != pending.request.participant {
                    return Err(Error::ParticipantChanged);
                }
            }
            if participants.len() != active.participants.len() {
                return Err(Error::CohortChanged);
            }
            Ok(())
        });
        if let Err(error) = result {
            self.phase = Phase::Failed { intent_id, error };
        }
        result
    }

    pub fn ready(&mut self, intent_id: u64, now: Instant) -> bool {
        self.tick(now) == Status::Ready { intent_id }
    }

    pub fn take_ready(&mut self, intent_id: u64, now: Instant) -> bool {
        if !self.ready(intent_id, now) {
            return false;
        }
        self.phase = Phase::Idle;
        true
    }

    pub fn cancel(&mut self, intent_id: u64) -> bool {
        if !matches!(&self.phase, Phase::Active(active) if active.intent_id == intent_id) {
            return false;
        }
        self.phase = Phase::Failed {
            intent_id,
            error: Error::Cancelled,
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::preview_protocol;
    use super::*;
    use aurora_session::{AccountKey, GridKey, HostId, SessionId};

    fn participant(worker: u8) -> Participant {
        Participant {
            source: preview_protocol::fixture_source(worker),
            device_epoch: 1,
        }
    }

    fn ack(intent_id: u64, request: Request) -> Ack {
        Ack {
            intent_id,
            request,
            surface_epoch: 4,
            completed_frame: Some(8),
        }
    }

    #[test]
    fn preview_barrier_empty_cohort_is_immediate_exact_intent_and_consumed_once() {
        let now = Instant::now();
        let mut barrier = Barrier::default();
        assert!(barrier.begin(1, &[], now).expect("no preview authority ever issued").is_empty());
        assert_eq!(barrier.tick(now), Status::Ready { intent_id: 1 });
        assert!(!barrier.ready(2, now));
        assert!(!barrier.take_ready(2, now));
        assert!(barrier.take_ready(1, now));
        assert!(!barrier.take_ready(1, now));
        assert_eq!(barrier.tick(now), Status::Idle);
        assert_eq!(barrier.next_challenge, 1, "empty M1 path consumes no GPU challenge");
    }

    #[test]
    fn preview_barrier_requires_every_exact_gpu_ack_in_any_order() {
        let now = Instant::now();
        let mut barrier = Barrier::default();
        let requests = barrier
            .begin(7, &[participant(3), participant(4), participant(5)], now)
            .expect("cohort");
        assert_eq!(
            barrier.tick(now),
            Status::Pending {
                intent_id: 7,
                remaining: 3
            }
        );
        assert_eq!(barrier.observe(ack(7, requests[2]), now), Ok(true));
        assert_eq!(barrier.observe(ack(7, requests[0]), now), Ok(true));
        assert!(!barrier.ready(7, now));
        assert_eq!(
            barrier.tick(now),
            Status::Pending {
                intent_id: 7,
                remaining: 1
            }
        );
        // An actual completed queue with no prior native presentation is valid.
        assert_eq!(
            barrier.observe(
                Ack {
                    completed_frame: None,
                    ..ack(7, requests[1])
                },
                now
            ),
            Ok(true)
        );
        assert!(barrier.ready(7, now));
        assert!(barrier.take_ready(7, now));
    }

    #[test]
    fn preview_barrier_slow_fence_times_out_without_counting_the_missing_worker() {
        let now = Instant::now();
        let mut barrier = Barrier::default();
        let requests = barrier.begin(8, &[participant(3), participant(4)], now).expect("cohort");
        barrier.observe(ack(8, requests[0]), now).expect("first physical completion");
        assert!(!barrier.ready(8, now + TIMEOUT - Duration::from_millis(1)));
        assert_eq!(
            barrier.tick(now + TIMEOUT),
            Status::Failed {
                intent_id: 8,
                error: Error::Timeout
            }
        );
        assert_eq!(barrier.observe(ack(8, requests[1]), now + TIMEOUT), Err(Error::Timeout));
        assert!(!barrier.ready(8, now + TIMEOUT));
        assert!(!barrier.take_ready(8, now + TIMEOUT));
    }

    #[test]
    fn preview_barrier_late_grants_frame_counts_and_acks_cannot_release_a_new_target() {
        let now = Instant::now();
        let mut barrier = Barrier::default();
        let old = barrier.begin(1, &[participant(3)], now).expect("old target")[0];
        let current = barrier.begin(2, &[participant(3)], now).expect("changed target")[0];
        assert!(current.challenge > old.challenge);
        // Grants and ordinary frame notifications have no admission method.
        // Even a high frame count cannot rescue an old request or invalid fence.
        assert_eq!(
            barrier.observe(
                Ack {
                    completed_frame: Some(u64::MAX),
                    ..ack(1, old)
                },
                now
            ),
            Err(Error::StaleIntent)
        );
        assert_eq!(
            barrier.observe(
                Ack {
                    completed_frame: Some(u64::MAX),
                    ..ack(2, old)
                },
                now
            ),
            Err(Error::StaleAck)
        );
        assert_eq!(
            barrier.observe(
                Ack {
                    surface_epoch: 0,
                    completed_frame: Some(u64::MAX),
                    ..ack(2, current)
                },
                now
            ),
            Err(Error::InvalidProof)
        );
        assert!(!barrier.ready(1, now) && !barrier.ready(2, now));
        assert_eq!(barrier.observe(ack(2, current), now), Ok(true));
        assert!(!barrier.take_ready(1, now));
        assert!(barrier.take_ready(2, now));
    }

    #[test]
    fn preview_barrier_participant_loss_invalidates_even_an_unconsumed_ready_intent() {
        let now = Instant::now();
        let mut barrier = Barrier::default();
        let cohort = [participant(3), participant(4)];
        let requests = barrier.begin(3, &cohort, now).expect("cohort");
        for request in requests {
            barrier.observe(ack(3, request), now).expect("physical completion");
        }
        assert!(barrier.ready(3, now));
        assert_eq!(barrier.reconcile(3, &cohort[..1], now), Err(Error::ParticipantLost));
        assert_eq!(
            barrier.tick(now),
            Status::Failed {
                intent_id: 3,
                error: Error::ParticipantLost
            }
        );
        assert!(!barrier.take_ready(3, now));
    }

    #[test]
    fn preview_barrier_full_source_and_device_changes_invalidate_the_cohort() {
        let now = Instant::now();
        let original = participant(3);
        let mut changes = [original; 6];
        changes[0].device_epoch += 1;
        changes[1].source.lease_epoch += 1;
        changes[2].source.session.generation += 1;
        changes[3].source.session.session = SessionId::from_bytes([9; 16]).expect("new synthetic session");
        changes[4].source.session.account = AccountKey::from_bytes([9; 32]).expect("new synthetic account");
        changes[5].source.session.grid = GridKey::from_bytes([9; 32]).expect("new synthetic grid");
        for changed in changes {
            let mut barrier = Barrier::default();
            let request = barrier.begin(4, &[original], now).expect("cohort")[0];
            assert_eq!(barrier.reconcile(4, &[changed], now), Err(Error::ParticipantChanged));
            assert_eq!(barrier.observe(ack(4, request), now), Err(Error::ParticipantChanged));
            assert!(!barrier.ready(4, now));
        }
    }

    #[test]
    fn preview_barrier_reconcile_is_order_independent_but_additions_never_count_as_done() {
        let now = Instant::now();
        let mut barrier = Barrier::default();
        let cohort = [participant(3), participant(4)];
        barrier.begin(5, &cohort, now).expect("cohort");
        assert_eq!(barrier.reconcile(5, &[cohort[1], cohort[0]], now), Ok(()));
        assert_eq!(
            barrier.tick(now),
            Status::Pending {
                intent_id: 5,
                remaining: 2
            }
        );
        assert_eq!(
            barrier.reconcile(5, &[cohort[0], cohort[1], participant(5)], now),
            Err(Error::CohortChanged)
        );
        assert!(!barrier.take_ready(5, now));
        barrier.begin(6, &[], now).expect("new empty intent");
        assert_eq!(barrier.reconcile(6, &[participant(5)], now), Err(Error::CohortChanged));
        assert!(!barrier.ready(6, now));
    }

    #[test]
    fn preview_barrier_cancel_retry_and_source_change_keep_challenges_fresh() {
        let now = Instant::now();
        let mut barrier = Barrier::default();
        let original = participant(3);
        let first = barrier.begin(9, &[original], now).expect("first attempt")[0];
        assert!(!barrier.cancel(8));
        assert!(barrier.cancel(9));
        assert!(!barrier.take_ready(9, now));
        assert_eq!(barrier.observe(ack(9, first), now), Err(Error::Cancelled));
        let second = barrier.begin(9, &[original], now).expect("same intent retry")[0];
        assert!(second.challenge > first.challenge);
        assert_eq!(barrier.observe(ack(9, first), now), Err(Error::StaleAck));
        let mut changed = original;
        changed.source.session.generation += 1;
        let third = barrier.begin(10, &[changed], now).expect("new source intent")[0];
        assert!(third.challenge > second.challenge);
        assert_eq!(barrier.observe(ack(10, second), now), Err(Error::StaleAck));
        assert!(!barrier.ready(10, now));
        barrier.observe(ack(10, third), now).expect("new physical completion");
        assert!(barrier.take_ready(10, now));
    }

    #[test]
    fn preview_barrier_duplicate_ack_is_idempotent_and_changed_proof_cannot_replace_it() {
        let now = Instant::now();
        let mut barrier = Barrier::default();
        let requests = barrier.begin(11, &[participant(3), participant(4)], now).expect("cohort");
        let first = ack(11, requests[0]);
        assert_eq!(barrier.observe(first, now), Ok(true));
        assert_eq!(barrier.observe(first, now), Ok(false));
        assert_eq!(
            barrier.observe(
                Ack {
                    surface_epoch: first.surface_epoch + 1,
                    ..first
                },
                now
            ),
            Err(Error::ConflictingAck)
        );
        let mut other_device = requests[1];
        other_device.participant.device_epoch += 1;
        assert_eq!(barrier.observe(ack(11, other_device), now), Err(Error::StaleAck));
        assert_eq!(
            barrier.tick(now),
            Status::Pending {
                intent_id: 11,
                remaining: 1
            }
        );
        barrier.observe(ack(11, requests[1]), now).expect("remaining exact completion");
        assert!(barrier.ready(11, now));
    }

    #[test]
    fn preview_barrier_invalid_new_begin_retires_old_ready_and_bounds_admission() {
        let now = Instant::now();
        let mut barrier = Barrier::default();
        let p = participant(3);
        let request = barrier.begin(12, &[p], now).expect("old cohort")[0];
        barrier.observe(ack(12, request), now).expect("old completion");
        assert!(barrier.ready(12, now));
        let mut same_worker_other_device = p;
        same_worker_other_device.device_epoch += 1;
        assert_eq!(barrier.begin(13, &[p, same_worker_other_device], now), Err(Error::DuplicateWorker));
        assert!(!barrier.ready(12, now) && !barrier.ready(13, now));
        assert_eq!(barrier.begin(0, &[], now), Err(Error::InvalidIntent));
        let oversized: Vec<_> = (1..=MAX_WORKERS as u8 + 1).map(participant).collect();
        assert_eq!(barrier.begin(14, &oversized, now), Err(Error::TooManyParticipants));
        let mut invalid = p;
        invalid.device_epoch = 0;
        assert_eq!(barrier.begin(15, &[invalid], now), Err(Error::InvalidParticipant));
        invalid = p;
        invalid.source.session.generation = 0;
        assert_eq!(barrier.begin(16, &[invalid], now), Err(Error::InvalidParticipant));
        let mut foreign = participant(4);
        foreign.source.worker.host = HostId::from_bytes([9; 16]).expect("foreign synthetic host");
        assert_eq!(barrier.begin(17, &[p, foreign], now), Err(Error::InvalidParticipant));
    }

    #[test]
    fn preview_barrier_challenge_exhaustion_is_atomic_and_sticky_across_cancel_retry() {
        let now = Instant::now();
        let mut barrier = Barrier {
            next_challenge: MAX_SUSPEND_CHALLENGE - 1,
            ..Barrier::default()
        };
        let requests = barrier
            .begin(18, &[participant(3), participant(4)], now)
            .expect("last two unique challenges");
        assert_eq!(
            requests.iter().map(|request| request.challenge).collect::<Vec<_>>(),
            [MAX_SUSPEND_CHALLENGE - 1, MAX_SUSPEND_CHALLENGE]
        );
        assert!(barrier.cancel(18));
        assert_eq!(barrier.begin(18, &[participant(3)], now), Err(Error::Exhausted));
        assert_eq!(barrier.begin(19, &[], now), Err(Error::Exhausted));
        assert_eq!(barrier.next_challenge, 0);
        assert!(!barrier.ready(18, now) && !barrier.ready(19, now));

        let mut barrier = Barrier {
            next_challenge: MAX_SUSPEND_CHALLENGE,
            ..Barrier::default()
        };
        assert_eq!(barrier.begin(20, &[participant(3), participant(4)], now), Err(Error::Exhausted));
        assert_eq!(barrier.next_challenge, 0, "no partial request allocation or wrap to one");
        assert_eq!(barrier.begin(20, &[participant(3)], now), Err(Error::Exhausted));
    }

    #[test]
    fn preview_barrier_ready_and_take_recheck_deadline_without_an_external_tick() {
        let now = Instant::now();
        let mut barrier = Barrier::default();
        let request = barrier.begin(21, &[participant(3)], now).expect("cohort")[0];
        barrier
            .observe(ack(21, request), now + RETRY_FOR_TEST)
            .expect("completion before timeout");
        assert!(barrier.ready(21, now + TIMEOUT - Duration::from_millis(1)));
        assert!(!barrier.take_ready(21, now + TIMEOUT));
        assert_eq!(
            barrier.tick(now + TIMEOUT),
            Status::Failed {
                intent_id: 21,
                error: Error::Timeout
            }
        );
    }

    const RETRY_FOR_TEST: Duration = Duration::from_secs(1);
}
