//! Conservative aggregate reservation ledger for configured preview policies.
//! The prototype cap is five requested preview FPS in total; it is not a GPU
//! latency, memory or performance guarantee. Physical admission remains separate.
//! All configured visible policies count, including an active world's policy.
//!
//! The broker reserves before native work, acknowledges only the exact applied
//! policy, and calls drained only after its captured policy intent has actual
//! cohort GPU completion. Policy/IO acknowledgements never imply a GPU drain.
//! Old acknowledged AND superseded unacknowledged requests remain held until
//! that proof. A suspended worker must not resume an older policy after drain.

use super::preview_protocol::PreviewSource;
use aurora_session::{MAX_WORKERS, PreviewDemand, PreviewRate, admit_previews};

pub(crate) const MAX_HALF_FPS: u32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Policy {
    pub source: PreviewSource,
    pub revision: u64,
    /// None is a hidden/removal request, not evidence that old work stopped.
    pub rate: Option<PreviewRate>,
}

impl Policy {
    fn valid(self) -> bool {
        self.revision != 0 && self.source.validate().is_ok()
    }
    fn half_fps(self) -> u32 {
        self.rate.map_or(0, PreviewRate::half_fps)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    InvalidPolicy,
    Binding,
    StaleRevision,
    RevisionConflict,
    Unrequested,
    UnknownSource,
    Capacity,
    NotHidden,
    NotApplied,
    NotDrained,
    NotRetired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Reservation {
    pub policy: Policy,
    /// High-water of prior acknowledged or merely requested rates, which may
    /// already be in flight. Only an exact current GPU drain releases this.
    pub retained_half_fps: u32,
    pub reserved_half_fps: u32,
    pub applied: bool,
    pub drained: bool,
    pub retired: bool,
}

#[derive(Clone, Copy)]
struct Entry {
    policy: Policy,
    retained: u32,
    applied: bool,
    drained: bool,
    retired: bool,
}

impl Entry {
    fn reserved(self) -> u32 {
        self.retained.max(self.policy.half_fps())
    }

    fn snapshot(self) -> Reservation {
        Reservation {
            policy: self.policy,
            retained_half_fps: self.retained,
            reserved_half_fps: self.reserved(),
            applied: self.applied,
            drained: self.drained,
            retired: self.retired,
        }
    }

    fn replacement_allowed(self, policy: Policy) -> bool {
        self.retired
            && self.applied
            && self.drained
            && self.policy.rate.is_none()
            && self.reserved() == 0
            && policy.source.lease_epoch == self.policy.source.lease_epoch
            && policy.source.session.session == self.policy.source.session.session
            && policy.source.session.generation > self.policy.source.session.generation
            && policy.revision > self.policy.revision
    }
}

/// Entries remain as bounded per-worker tombstones after retirement. No loss,
/// reconnect or device change evicts a worker's source/revision high-water.
/// New generations replace that same slot only after explicit hidden retirement.
/// Verified reclamation can explicitly free a clean slot; unknown or uncertain
/// workers still consume bounded capacity and cannot be silently evicted.
pub(crate) struct Ledger {
    entries: Vec<Entry>,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            entries: Vec::with_capacity(MAX_WORKERS),
        }
    }
}

impl Ledger {
    /// True means a new policy was fully admitted at its exact requested rate.
    /// False is an exact duplicate. Rejection changes neither charge nor policy.
    /// Caller must first verify the fresh live authenticated roster and full
    /// current binding. Reclaimed IDs have no ledger tombstone, so stale wire
    /// replies or GUI rows must never be allowed to submit a request here.
    pub fn request(&mut self, policy: Policy) -> Result<bool, Error> {
        if !policy.valid() {
            return Err(Error::InvalidPolicy);
        }
        if self.entries.first().is_some_and(|entry| {
            (entry.policy.source.worker.installation, entry.policy.source.worker.host)
                != (policy.source.worker.installation, policy.source.worker.host)
        }) {
            return Err(Error::Binding);
        }
        let index = self
            .entries
            .iter()
            .position(|entry| entry.policy.source.worker == policy.source.worker);
        let previous = index.map(|index| self.entries[index]);
        if let Some(previous) = previous {
            if previous.policy.source != policy.source {
                if !previous.replacement_allowed(policy) {
                    return Err(Error::Binding);
                }
            } else if policy.revision == previous.policy.revision {
                return if policy == previous.policy {
                    Ok(false)
                } else {
                    Err(Error::RevisionConflict)
                };
            } else if policy.revision < previous.policy.revision {
                return Err(Error::StaleRevision);
            }
        } else if self.entries.len() == MAX_WORKERS {
            return Err(Error::Capacity);
        }

        // Superseding a request cannot release work that might already have
        // reached native code, even if its applied acknowledgement was lost.
        let retained = previous.map_or(0, Entry::reserved);
        let others: u32 = self
            .entries
            .iter()
            .enumerate()
            .filter(|(other, _)| Some(*other) != index)
            .map(|(_, entry)| entry.reserved())
            .sum();
        if others > MAX_HALF_FPS {
            return Err(Error::Capacity);
        }
        // One eligible proposal avoids fair-share downgrades. All other
        // reservations form one ineligible holding demand, so five worker
        // entries still use only two inputs to the existing pure allocator.
        let demands = [
            PreviewDemand {
                requested: policy.rate,
                visible: policy.rate.is_some(),
                inactive: true,
                session_current: true,
                acknowledged_half_fps: previous.map_or(0, |entry| entry.retained),
                pending_half_fps: previous.map_or(0, |entry| entry.policy.half_fps()),
            },
            PreviewDemand {
                acknowledged_half_fps: others,
                ..PreviewDemand::default()
            },
        ];
        let admitted = admit_previews(&demands, MAX_HALF_FPS).map_err(|_| Error::Capacity)?;
        if admitted.first().copied() != Some(policy.rate) || others + retained.max(policy.half_fps()) > MAX_HALF_FPS {
            return Err(Error::Capacity);
        }
        let entry = Entry {
            policy,
            retained,
            applied: false,
            drained: false,
            retired: false,
        };
        if let Some(index) = index {
            self.entries[index] = entry;
        } else {
            self.entries.push(entry);
        }
        Ok(true)
    }

    fn exact_index(&self, policy: Policy) -> Result<usize, Error> {
        if !policy.valid() {
            return Err(Error::InvalidPolicy);
        }
        let index = self
            .entries
            .iter()
            .position(|entry| entry.policy.source.worker == policy.source.worker)
            .ok_or(Error::UnknownSource)?;
        let current = self.entries[index].policy;
        if current.source != policy.source {
            return Err(Error::Binding);
        }
        if policy.revision < current.revision {
            return Err(Error::StaleRevision);
        }
        if policy.revision > current.revision {
            return Err(Error::Unrequested);
        }
        if current != policy {
            return Err(Error::RevisionConflict);
        }
        Ok(index)
    }

    /// Native policy acknowledgement never releases an older reservation.
    pub fn applied(&mut self, policy: Policy) -> Result<bool, Error> {
        let index = self.exact_index(policy)?;
        let changed = !self.entries[index].applied;
        self.entries[index].applied = true;
        Ok(changed)
    }

    /// Caller must hold the exact captured intent's actual cohort GPU proof.
    /// Desired new work stays reserved even when it has not yet been applied.
    pub fn drained(&mut self, policy: Policy) -> Result<bool, Error> {
        let index = self.exact_index(policy)?;
        let entry = &mut self.entries[index];
        let changed = !entry.drained;
        entry.retained = 0;
        entry.drained = true;
        Ok(changed)
    }

    /// Loss, reconnect or device replacement is uncertainty, not quiescence.
    pub fn uncertain(&mut self, source: PreviewSource) -> Result<(), Error> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.policy.source.worker == source.worker)
            .ok_or(Error::UnknownSource)?;
        if entry.policy.source != source {
            return Err(Error::Binding);
        }
        entry.applied = false;
        entry.drained = false;
        entry.retired = false;
        Ok(())
    }

    /// Keep the source/revision tombstone. Only a confirmed hidden policy whose
    /// actual old GPU work drained can later admit a strictly newer generation.
    pub fn retire_drained(&mut self, policy: Policy) -> Result<bool, Error> {
        let index = self.exact_index(policy)?;
        let entry = &mut self.entries[index];
        if entry.policy.rate.is_some() {
            return Err(Error::NotHidden);
        }
        if !entry.applied {
            return Err(Error::NotApplied);
        }
        if !entry.drained || entry.reserved() != 0 {
            return Err(Error::NotDrained);
        }
        let changed = !entry.retired;
        entry.retired = true;
        Ok(changed)
    }

    /// Explicitly forget only an exact confirmed hidden, retired, GPU-drained
    /// policy with zero desired/pending/history charge. Intervening loss,
    /// reconnect or device changes must first be marked uncertain by the caller.
    /// Only after success may the broker retire this peer's ever-authority flag
    /// or drop its clean exited child. No IO/disconnect event is a drain proof.
    pub fn reclaim_retired(&mut self, policy: Policy) -> Result<bool, Error> {
        let index = self.exact_index(policy)?;
        let entry = self.entries[index];
        if entry.policy.rate.is_some() {
            return Err(Error::NotHidden);
        }
        if !entry.applied {
            return Err(Error::NotApplied);
        }
        if !entry.drained || entry.retained != 0 || entry.reserved() != 0 {
            return Err(Error::NotDrained);
        }
        if !entry.retired {
            return Err(Error::NotRetired);
        }
        self.entries.remove(index);
        Ok(true)
    }

    pub fn reservation(&self, source: PreviewSource) -> Option<Reservation> {
        self.entries
            .iter()
            .find(|entry| entry.policy.source == source)
            .copied()
            .map(Entry::snapshot)
    }

    pub fn total_half_fps(&self) -> u32 {
        self.entries.iter().map(|entry| entry.reserved()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::super::preview_protocol;
    use super::*;
    use aurora_session::{AccountKey, GridKey, HostId, SessionId};

    fn policy(worker: u8, revision: u64, rate: Option<PreviewRate>) -> Policy {
        Policy {
            source: preview_protocol::fixture_source(worker),
            revision,
            rate,
        }
    }

    fn next(previous: Policy, rate: Option<PreviewRate>) -> Policy {
        Policy {
            revision: previous.revision + 1,
            rate,
            ..previous
        }
    }

    #[test]
    fn preview_policy_core_admission_preserves_exact_requests_without_fair_downgrade() {
        let demand = |requested| PreviewDemand {
            requested: Some(requested),
            visible: true,
            inactive: true,
            session_current: true,
            ..PreviewDemand::default()
        };
        let fair = admit_previews(&[demand(PreviewRate::Five), demand(PreviewRate::Half)], MAX_HALF_FPS).expect("existing allocator");
        assert_eq!(fair[0], Some(PreviewRate::Two), "existing equal-share behavior remains unchanged");
        let mut ledger = Ledger::default();
        let full = policy(3, 1, Some(PreviewRate::Five));
        ledger.request(full).expect("full requested rate");
        assert_eq!(ledger.request(policy(4, 1, Some(PreviewRate::Half))), Err(Error::Capacity));
        assert_eq!(ledger.reservation(full.source).expect("unchanged requested policy").policy, full);
        assert_eq!(ledger.total_half_fps(), 10);
        assert!(ledger.reservation(policy(4, 1, None).source).is_none());
    }

    #[test]
    fn preview_policy_five_workers_use_exact_combined_cap_with_only_two_allocator_inputs() {
        let mut ledger = Ledger::default();
        for worker in 1..=MAX_WORKERS as u8 {
            let policy = policy(worker, 1, Some(PreviewRate::One));
            assert_eq!(ledger.request(policy), Ok(true));
            assert_eq!(ledger.reservation(policy.source).expect("reservation").reserved_half_fps, 2);
            assert_eq!(ledger.total_half_fps(), u32::from(worker) * 2);
        }
        assert_eq!(ledger.entries.len(), MAX_WORKERS);
        assert_eq!(
            ledger.request(policy(9, 1, None)),
            Err(Error::Capacity),
            "worker tombstones stay bounded even for zero-rate admission"
        );
    }

    #[test]
    fn preview_policy_lower_and_hidden_applied_ack_do_not_release_old_gpu_work() {
        let mut ledger = Ledger::default();
        let full = policy(3, 1, Some(PreviewRate::Five));
        ledger.request(full).expect("initial request");
        ledger.applied(full).expect("native policy acknowledgement");
        let lower = next(full, Some(PreviewRate::Half));
        ledger.request(lower).expect("lower request");
        assert_eq!(ledger.total_half_fps(), 10);
        ledger.applied(lower).expect("lower native acknowledgement");
        assert_eq!(ledger.total_half_fps(), 10, "policy ack is not GPU completion");
        assert_eq!(ledger.request(policy(4, 1, Some(PreviewRate::Half))), Err(Error::Capacity));
        ledger.drained(lower).expect("actual captured lower-policy GPU barrier");
        assert_eq!(ledger.total_half_fps(), 1, "new requested rate remains reserved");
        let hidden = next(lower, None);
        ledger.request(hidden).expect("hide request");
        ledger.applied(hidden).expect("native hide acknowledgement");
        assert_eq!(ledger.total_half_fps(), 1);
        ledger.drained(hidden).expect("actual hidden-policy GPU barrier");
        assert_eq!(ledger.total_half_fps(), 0);
    }

    #[test]
    fn preview_policy_superseded_unacknowledged_request_stays_reserved_until_exact_drain() {
        let mut ledger = Ledger::default();
        let pending = policy(3, 1, Some(PreviewRate::Five));
        ledger.request(pending).expect("reserved before any native work");
        let hidden = next(pending, None);
        ledger.request(hidden).expect("hide supersedes unacknowledged request");
        assert_eq!(ledger.reservation(hidden.source).expect("history retained").retained_half_fps, 10);
        assert_eq!(ledger.total_half_fps(), 10);
        assert_eq!(ledger.applied(pending), Err(Error::StaleRevision));
        assert_eq!(ledger.drained(pending), Err(Error::StaleRevision));
        assert_eq!(ledger.total_half_fps(), 10);
        ledger.applied(hidden).expect("native hide acknowledgement");
        assert_eq!(ledger.total_half_fps(), 10);
        ledger.drained(hidden).expect("current intent actually drained");
        assert_eq!(ledger.total_half_fps(), 0);
    }

    #[test]
    fn preview_policy_duplicate_demand_and_proofs_never_double_charge_or_free_desired_work() {
        let mut ledger = Ledger::default();
        let desired = policy(3, 1, Some(PreviewRate::Two));
        assert_eq!(ledger.request(desired), Ok(true));
        for _ in 0..5 {
            assert_eq!(ledger.request(desired), Ok(false));
        }
        assert_eq!(ledger.entries.len(), 1);
        assert_eq!(ledger.total_half_fps(), 4);
        assert_eq!(ledger.applied(desired), Ok(true));
        assert_eq!(ledger.applied(desired), Ok(false));
        assert_eq!(ledger.drained(desired), Ok(true));
        assert_eq!(ledger.drained(desired), Ok(false));
        assert_eq!(ledger.total_half_fps(), 4, "configured work stays reserved after suspension");
        assert_eq!(ledger.retire_drained(desired), Err(Error::NotHidden));
    }

    #[test]
    fn preview_policy_rejected_upgrade_is_atomic_and_can_retry_after_other_actual_drains() {
        let mut ledger = Ledger::default();
        let first = policy(3, 1, Some(PreviewRate::Two));
        let other = policy(4, 1, Some(PreviewRate::One));
        ledger.request(first).expect("first request");
        ledger.request(other).expect("other request");
        let upgrade = next(first, Some(PreviewRate::Five));
        let before = ledger.reservation(first.source);
        assert_eq!(ledger.request(upgrade), Err(Error::Capacity));
        assert_eq!(ledger.reservation(first.source), before);
        assert_eq!(ledger.total_half_fps(), 6);
        ledger.applied(first).expect("rejection did not advance revision");
        let hidden = next(other, None);
        ledger.request(hidden).expect("other hide");
        assert_eq!(ledger.request(upgrade), Err(Error::Capacity));
        ledger.drained(hidden).expect("actual other GPU drain");
        assert_eq!(ledger.request(upgrade), Ok(true));
        assert_eq!(ledger.total_half_fps(), 10);
        assert_eq!(
            ledger.reservation(first.source).expect("full upgrade").policy.rate,
            Some(PreviewRate::Five)
        );
    }

    #[test]
    fn preview_policy_loss_reconnect_and_device_uncertainty_preserve_every_reservation() {
        let mut ledger = Ledger::default();
        let desired = policy(3, 1, Some(PreviewRate::Five));
        ledger.request(desired).expect("request");
        ledger.applied(desired).expect("native acknowledgement");
        ledger.drained(desired).expect("completed current suspension");
        for _ in 0..3 {
            ledger.uncertain(desired.source).expect("loss/reconnect/device replacement");
            let held = ledger.reservation(desired.source).expect("reservation retained");
            assert!(!held.applied && !held.drained && !held.retired);
            assert_eq!(held.reserved_half_fps, 10);
            assert_eq!(ledger.request(desired), Ok(false), "request replay is not native confirmation");
            assert_eq!(ledger.total_half_fps(), 10);
        }
        let mut relogin = next(desired, Some(PreviewRate::Half));
        relogin.source.session.generation += 1;
        assert_eq!(ledger.request(relogin), Err(Error::Binding));
        assert_eq!(ledger.total_half_fps(), 10);
    }

    #[test]
    fn preview_policy_stale_future_and_conflicting_policy_proofs_never_release_latest_charge() {
        let mut ledger = Ledger::default();
        let old = policy(3, 1, Some(PreviewRate::Five));
        ledger.request(old).expect("old requested work");
        let lower = next(old, Some(PreviewRate::Two));
        ledger.request(lower).expect("lower request");
        let hidden = next(lower, None);
        ledger.request(hidden).expect("latest hide");
        assert_eq!(ledger.request(lower), Err(Error::StaleRevision));
        assert_eq!(ledger.drained(lower), Err(Error::StaleRevision));
        assert_eq!(ledger.applied(lower), Err(Error::StaleRevision));
        assert_eq!(
            ledger.request(Policy {
                rate: Some(PreviewRate::Half),
                ..hidden
            }),
            Err(Error::RevisionConflict)
        );
        assert_eq!(
            ledger.drained(Policy {
                rate: Some(PreviewRate::Half),
                ..hidden
            }),
            Err(Error::RevisionConflict)
        );
        assert_eq!(ledger.applied(next(hidden, None)), Err(Error::Unrequested));
        assert_eq!(ledger.total_half_fps(), 10);
        ledger.drained(hidden).expect("exact current proof");
        assert_eq!(ledger.total_half_fps(), 0);
        let reopened = next(hidden, Some(PreviewRate::Two));
        ledger.request(reopened).expect("new desired work");
        assert_eq!(ledger.drained(hidden), Err(Error::StaleRevision));
        assert_eq!(ledger.total_half_fps(), 4);
    }

    #[test]
    fn preview_policy_drain_before_apply_keeps_desired_reservation_and_cannot_retire_early() {
        let mut ledger = Ledger::default();
        let full = policy(3, 1, Some(PreviewRate::Five));
        ledger.request(full).expect("request");
        let lower = next(full, Some(PreviewRate::One));
        ledger.request(lower).expect("lower request");
        ledger.drained(lower).expect("actual cohort completion before native policy ack");
        assert_eq!(ledger.total_half_fps(), 2);
        assert!(!ledger.reservation(lower.source).expect("pending policy").applied);
        ledger.applied(lower).expect("native policy confirmation");
        assert_eq!(ledger.total_half_fps(), 2);
        let hidden = next(lower, None);
        ledger.request(hidden).expect("hidden request");
        ledger.drained(hidden).expect("actual drain");
        assert_eq!(ledger.retire_drained(hidden), Err(Error::NotApplied));
        ledger.applied(hidden).expect("hidden native confirmation");
        assert_eq!(ledger.retire_drained(hidden), Ok(true));
        ledger.uncertain(hidden.source).expect("later connection uncertainty");
        assert_eq!(ledger.retire_drained(hidden), Err(Error::NotApplied));
        ledger.applied(hidden).expect("fresh confirmation");
        assert_eq!(ledger.retire_drained(hidden), Err(Error::NotDrained));
        assert_eq!(ledger.total_half_fps(), 0);
    }

    #[test]
    fn preview_policy_retired_worker_tombstone_allows_only_strict_new_generation_in_place() {
        let mut ledger = Ledger::default();
        let full = policy(3, 1, Some(PreviewRate::Five));
        ledger.request(full).expect("old source request");
        let hidden = next(full, None);
        ledger.request(hidden).expect("old source hidden");
        assert_eq!(ledger.retire_drained(hidden), Err(Error::NotApplied));
        ledger.applied(hidden).expect("hidden applied");
        assert_eq!(ledger.retire_drained(hidden), Err(Error::NotDrained));
        ledger.drained(hidden).expect("actual old source GPU drain");
        let mut relogin = next(hidden, Some(PreviewRate::One));
        relogin.source.session.generation += 1;
        relogin.source.session.account = AccountKey::from_bytes([9; 32]).expect("new synthetic account");
        relogin.source.session.grid = GridKey::from_bytes([10; 32]).expect("new synthetic grid");
        assert_eq!(
            ledger.request(relogin),
            Err(Error::Binding),
            "zero charge alone is not explicit retirement"
        );
        assert_eq!(ledger.retire_drained(hidden), Ok(true));
        assert_eq!(ledger.retire_drained(hidden), Ok(false));
        assert_eq!(ledger.request(relogin), Ok(true));
        assert_eq!(ledger.entries.len(), 1);
        assert!(ledger.reservation(full.source).is_none());
        assert_eq!(ledger.applied(hidden), Err(Error::Binding));
        assert_eq!(ledger.drained(hidden), Err(Error::Binding));

        let hidden_new = next(relogin, None);
        ledger.request(hidden_new).expect("new source hide");
        ledger.applied(hidden_new).expect("new hide applied");
        ledger.drained(hidden_new).expect("new source actual drain");
        ledger.retire_drained(hidden_new).expect("new source explicitly retired");
        assert_eq!(
            ledger.request(Policy {
                revision: u64::MAX,
                ..full
            }),
            Err(Error::Binding),
            "old generation cannot reopen through a higher revision"
        );
        let mut valid = next(hidden_new, Some(PreviewRate::Half));
        valid.source.session.generation += 1;
        for invalid in [
            Policy {
                source: PreviewSource {
                    lease_epoch: valid.source.lease_epoch + 1,
                    ..valid.source
                },
                ..valid
            },
            Policy {
                source: PreviewSource {
                    session: aurora_session::SessionIdentity {
                        session: SessionId::from_bytes([11; 16]).expect("other bootstrap session"),
                        ..valid.source.session
                    },
                    ..valid.source
                },
                ..valid
            },
            Policy {
                source: PreviewSource {
                    session: aurora_session::SessionIdentity {
                        generation: hidden_new.source.session.generation,
                        ..valid.source.session
                    },
                    ..valid.source
                },
                ..valid
            },
            Policy {
                revision: hidden_new.revision,
                ..valid
            },
        ] {
            assert_eq!(ledger.request(invalid), Err(Error::Binding));
        }
        assert_eq!(ledger.request(valid), Ok(true));
        assert_eq!(ledger.entries.len(), 1, "generation changes reuse the bounded worker slot");
    }

    #[test]
    fn preview_policy_same_source_can_reopen_after_retirement_without_erasing_revision_floor() {
        let mut ledger = Ledger::default();
        let hidden = policy(3, 5, None);
        ledger.request(hidden).expect("hidden policy");
        ledger.applied(hidden).expect("hidden applied");
        ledger.drained(hidden).expect("known empty or actual GPU cohort drain");
        ledger.retire_drained(hidden).expect("explicit retirement");
        assert_eq!(ledger.request(hidden), Ok(false));
        assert!(ledger.reservation(hidden.source).expect("tombstone").retired);
        assert_eq!(
            ledger.request(Policy {
                revision: 4,
                rate: Some(PreviewRate::Five),
                ..hidden
            }),
            Err(Error::StaleRevision)
        );
        assert_eq!(
            ledger.request(Policy {
                rate: Some(PreviewRate::Five),
                ..hidden
            }),
            Err(Error::RevisionConflict)
        );
        let reopened = next(hidden, Some(PreviewRate::Half));
        assert_eq!(ledger.request(reopened), Ok(true));
        assert!(!ledger.reservation(hidden.source).expect("reopened policy").retired);
        assert_eq!(ledger.total_half_fps(), 1);
    }

    #[test]
    fn preview_policy_invalid_source_revision_and_foreign_host_do_not_create_reservations() {
        let mut ledger = Ledger::default();
        let valid = policy(3, 1, Some(PreviewRate::Half));
        assert_eq!(ledger.request(Policy { revision: 0, ..valid }), Err(Error::InvalidPolicy));
        let mut invalid = valid;
        invalid.source.lease_epoch = 0;
        assert_eq!(ledger.request(invalid), Err(Error::InvalidPolicy));
        invalid = valid;
        invalid.source.session.generation = 0;
        assert_eq!(ledger.request(invalid), Err(Error::InvalidPolicy));
        assert_eq!(ledger.total_half_fps(), 0);
        assert!(ledger.entries.is_empty());
        ledger.request(valid).expect("valid policy");
        let mut foreign = policy(4, 1, Some(PreviewRate::Half));
        foreign.source.worker.host = HostId::from_bytes([9; 16]).expect("foreign host");
        assert_eq!(ledger.request(foreign), Err(Error::Binding));
        assert_eq!(ledger.applied(policy(4, 1, None)), Err(Error::UnknownSource));
        assert_eq!(ledger.total_half_fps(), 1);
        assert_eq!(ledger.entries.len(), 1);
    }

    #[test]
    fn preview_policy_explicitly_reclaims_clean_retired_slot_for_a_sixth_worker() {
        let mut ledger = Ledger::default();
        let configured: Vec<_> = (1..=MAX_WORKERS as u8)
            .map(|worker| policy(worker, 1, Some(PreviewRate::One)))
            .collect();
        for configured in &configured {
            ledger.request(*configured).expect("full five-worker cohort");
            ledger.applied(*configured).expect("native policy applied");
        }
        let mut hidden = Vec::new();
        for configured in configured {
            let close = next(configured, None);
            ledger.request(close).expect("hidden requested");
            ledger.applied(close).expect("native hidden applied");
            ledger.drained(close).expect("actual captured GPU drain");
            ledger.retire_drained(close).expect("explicit retirement");
            hidden.push(close);
        }
        assert_eq!(ledger.entries.len(), MAX_WORKERS);
        assert_eq!(ledger.total_half_fps(), 0);
        let sixth = policy(9, 1, Some(PreviewRate::Five));
        assert_eq!(ledger.request(sixth), Err(Error::Capacity), "no automatic tombstone eviction");
        assert_eq!(ledger.reclaim_retired(hidden[0]), Ok(true));
        assert_eq!(ledger.entries.len(), MAX_WORKERS - 1);
        assert_eq!(ledger.request(sixth), Ok(true));
        assert_eq!(ledger.entries.len(), MAX_WORKERS);
        assert_eq!(ledger.total_half_fps(), 10);
        let current = ledger.reservation(sixth.source);
        assert_eq!(ledger.applied(hidden[0]), Err(Error::UnknownSource));
        assert_eq!(ledger.drained(hidden[0]), Err(Error::UnknownSource));
        assert_eq!(ledger.reservation(sixth.source), current);
        assert_eq!(ledger.total_half_fps(), 10);
    }

    #[test]
    fn preview_policy_reclamation_rejects_unknown_uncertain_and_unconfirmed_work() {
        let mut ledger = Ledger::default();
        let visible = policy(3, 1, Some(PreviewRate::One));
        let hidden = next(visible, None);
        assert_eq!(ledger.reclaim_retired(hidden), Err(Error::UnknownSource));
        ledger.request(visible).expect("visible reservation");
        assert_eq!(ledger.reclaim_retired(visible), Err(Error::NotHidden));
        ledger.request(hidden).expect("hidden request");
        assert_eq!(ledger.reclaim_retired(hidden), Err(Error::NotApplied));
        ledger.applied(hidden).expect("native hidden policy");
        assert_eq!(ledger.reclaim_retired(hidden), Err(Error::NotDrained));
        assert_eq!(ledger.total_half_fps(), 2);
        ledger.drained(hidden).expect("actual captured GPU drain");
        assert_eq!(ledger.reclaim_retired(hidden), Err(Error::NotRetired));
        ledger.retire_drained(hidden).expect("explicit retirement");
        ledger.uncertain(hidden.source).expect("disconnect or device uncertainty");
        assert_eq!(ledger.reclaim_retired(hidden), Err(Error::NotApplied));
        assert_eq!(ledger.entries.len(), 1);
        ledger.applied(hidden).expect("fresh native confirmation");
        assert_eq!(ledger.reclaim_retired(hidden), Err(Error::NotDrained));
        ledger.drained(hidden).expect("new actual GPU drain");
        assert_eq!(ledger.reclaim_retired(hidden), Err(Error::NotRetired));
        ledger.retire_drained(hidden).expect("fresh explicit retirement");
        assert_eq!(ledger.reclaim_retired(hidden), Ok(true));
        assert!(ledger.entries.is_empty());
    }

    #[test]
    fn preview_policy_forgotten_source_replies_cannot_release_a_fresh_live_binding() {
        let mut ledger = Ledger::default();
        let old = policy(3, 1, Some(PreviewRate::Five));
        ledger.request(old).expect("old reservation");
        let hidden = next(old, None);
        ledger.request(hidden).expect("old hidden policy");
        ledger.applied(hidden).expect("hidden native acknowledgement");
        ledger.drained(hidden).expect("actual old GPU drain");
        ledger.retire_drained(hidden).expect("explicit retirement");
        ledger.reclaim_retired(hidden).expect("verified clean reclamation");
        assert_eq!(ledger.applied(old), Err(Error::UnknownSource));
        assert_eq!(ledger.drained(hidden), Err(Error::UnknownSource));
        assert_eq!(ledger.reclaim_retired(hidden), Err(Error::UnknownSource));

        // The broker, not an old wire reply or GUI row, verified this fresh
        // authenticated live binding before submitting its new policy request.
        let mut live = next(hidden, Some(PreviewRate::One));
        live.source.session.generation += 1;
        ledger.request(live).expect("fresh roster-validated binding");
        let current = ledger.reservation(live.source);
        assert_eq!(ledger.applied(old), Err(Error::Binding));
        assert_eq!(ledger.drained(hidden), Err(Error::Binding));
        assert_eq!(ledger.retire_drained(hidden), Err(Error::Binding));
        assert_eq!(ledger.reclaim_retired(hidden), Err(Error::Binding));
        assert_eq!(ledger.reservation(live.source), current);
        assert_eq!(ledger.total_half_fps(), 2);
    }
}
