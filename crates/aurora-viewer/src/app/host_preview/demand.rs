//! Bounded native apply receipts, not source/render authority. The App supplies
//! a genuine current channel/native scope before using this settings ledger.
use aurora_session::preview_image::PreviewSource;
use aurora_session::preview_wire::{DemandConfiguration, DemandOutcome, DemandReceipt, DemandRejection};
use std::collections::VecDeque;

const LIMIT: usize = 32;
#[derive(Clone, Copy)]
struct Record {
    receipt: DemandReceipt,
    stopped: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Applied {
    pub demand: u64,
    pub config: DemandConfiguration,
    pub revision: u64,
}
#[derive(Clone, Copy)]
pub(super) enum Decision {
    Known(DemandReceipt),
    Reject(DemandRejection),
    Apply,
}
pub(super) struct Ledger {
    records: VecDeque<Record>,
    high: u64,
    revision: u64,
    current: Option<Applied>,
    exhausted: bool,
}
impl Default for Ledger {
    fn default() -> Self {
        Self {
            records: VecDeque::with_capacity(LIMIT),
            high: 0,
            revision: 0,
            current: None,
            exhausted: false,
        }
    }
}
impl Ledger {
    pub fn current(&self) -> Option<Applied> {
        self.current
    }
    /// A new genuine metadata domain retires configuration; do not replay it.
    /// The local native revision remains monotonic across that replacement.
    pub fn reset_domain(&mut self) {
        self.records.clear();
        self.high = 0;
        self.current = None;
    }
    pub fn classify(&self, demand: u64, config: DemandConfiguration) -> Decision {
        if let Some(record) = self.records.iter().find(|r| r.receipt.demand == demand) {
            return if record.receipt.requested == config {
                Decision::Known(record.receipt)
            } else {
                Decision::Reject(DemandRejection::Stale)
            };
        }
        if self.exhausted {
            return Decision::Reject(DemandRejection::Denied);
        }
        if demand == 0 || demand <= self.high {
            return Decision::Reject(DemandRejection::Stale);
        }
        Decision::Apply
    }
    /// Only called AFTER the owner revokes/stops the old capture configuration.
    pub fn apply(
        &mut self,
        source: PreviewSource,
        connection: u64,
        demand: u64,
        config: DemandConfiguration,
        denial: Option<DemandRejection>,
    ) -> Option<DemandReceipt> {
        if connection == 0 || demand == 0 {
            return None;
        }
        match self.classify(demand, config) {
            Decision::Known(receipt) => {
                return if receipt.source == source && receipt.metadata_connection == connection {
                    Some(receipt)
                } else {
                    Self::rejection(source, connection, demand, config, DemandRejection::Stale)
                };
            }
            Decision::Reject(reason) => return Self::rejection(source, connection, demand, config, reason),
            Decision::Apply => {}
        }
        let outcome = if let Some(reason) = denial {
            DemandOutcome::Rejected { reason }
        } else if let Some(revision) = self.revision.checked_add(1) {
            self.revision = revision;
            self.current = Some(Applied { demand, config, revision });
            DemandOutcome::Accepted {
                effective: config,
                native_revision: revision,
            }
        } else {
            self.exhausted = true;
            self.current = None;
            DemandOutcome::Rejected {
                reason: DemandRejection::Denied,
            }
        };
        let receipt = DemandReceipt {
            source,
            metadata_connection: connection,
            demand,
            requested: config,
            outcome,
        };
        if receipt.validate().is_err() {
            return None;
        }
        self.high = demand;
        if self.records.len() == LIMIT {
            let current = self.current.map(|current| current.demand);
            if let Some(index) = self.records.iter().position(|record| Some(record.receipt.demand) != current) {
                self.records.remove(index);
            } else {
                self.exhausted = true;
                self.current = None;
                return None; // Never grow beyond the fixed metadata cap.
            }
        }
        self.records.push_back(Record { receipt, stopped: false });
        Some(receipt)
    }
    pub fn rejection(
        source: PreviewSource,
        connection: u64,
        demand: u64,
        config: DemandConfiguration,
        reason: DemandRejection,
    ) -> Option<DemandReceipt> {
        let receipt = DemandReceipt {
            source,
            metadata_connection: connection,
            demand,
            requested: config,
            outcome: DemandOutcome::Rejected { reason },
        };
        receipt.validate().ok().map(|()| receipt)
    }
    pub fn can_stop(&self, demand: u64) -> bool {
        self.current.is_some_and(|current| current.demand == demand) || self.records.iter().any(|r| r.receipt.demand == demand && r.stopped)
    }
    /// This is native application, not pixel ACK or GPU-drain proof.
    pub fn stop(&mut self, demand: u64) -> bool {
        if !self.can_stop(demand) {
            return false;
        }
        if self.current.is_some_and(|current| current.demand == demand) {
            self.current = None;
        }
        if let Some(record) = self.records.iter_mut().find(|r| r.receipt.demand == demand) {
            record.stopped = true;
        }
        true
    }
    #[cfg(test)]
    pub fn allocated_bytes(&self) -> usize {
        self.records.capacity() * std::mem::size_of::<Record>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::{
        AccountKey, GridKey, HostId, InstallationId, PreviewRate, PreviewSize, SessionId, SessionIdentity, WorkerId, WorkerIdentity,
    };
    fn source() -> PreviewSource {
        PreviewSource::new(
            WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).unwrap(),
                host: HostId::from_bytes([2; 16]).unwrap(),
                worker: WorkerId::from_bytes([3; 16]).unwrap(),
            },
            SessionIdentity {
                session: SessionId::from_bytes([4; 16]).unwrap(),
                account: AccountKey::from_bytes([5; 32]).unwrap(),
                grid: GridKey::from_bytes([6; 32]).unwrap(),
                generation: 1,
            },
            7,
        )
        .unwrap()
    }
    fn config() -> DemandConfiguration {
        DemandConfiguration {
            size: PreviewSize::Small,
            rate: PreviewRate::One,
        }
    }
    #[test]
    fn native_demand_exact_duplicate_keeps_original_receipt_and_never_reapplies_mutation() {
        let mut ledger = Ledger::default();
        let first = ledger.apply(source(), 1, 1, config(), None).unwrap();
        let applied = ledger.current();
        assert_eq!(ledger.apply(source(), 1, 1, config(), None), Some(first));
        let changed = DemandConfiguration {
            rate: PreviewRate::Five,
            ..config()
        };
        assert!(matches!(ledger.classify(1, changed), Decision::Reject(DemandRejection::Stale)));
        let wrong = ledger.apply(source(), 1, 1, changed, None).unwrap();
        assert!(matches!(
            wrong.outcome,
            DemandOutcome::Rejected {
                reason: DemandRejection::Stale
            }
        ));
        assert_eq!(ledger.current(), applied);
        assert_eq!(ledger.apply(source(), 1, 1, config(), None), Some(first));
    }
    #[test]
    fn native_demand_stop_preserves_receipt_and_cannot_clear_a_newer_accepted_config() {
        let mut ledger = Ledger::default();
        let old = ledger.apply(source(), 1, 1, config(), None).unwrap();
        assert!(ledger.stop(1));
        assert!(ledger.stop(1));
        let newer = DemandConfiguration {
            size: PreviewSize::Large,
            rate: PreviewRate::Two,
        };
        ledger.apply(source(), 1, 2, newer, None).unwrap();
        assert!(ledger.stop(1));
        assert_eq!(ledger.current().unwrap().demand, 2);
        assert_eq!(ledger.apply(source(), 1, 1, config(), None), Some(old));
        assert_eq!(ledger.current().unwrap().demand, 2, "old Accepted receipt cannot reapply settings");
        assert!(!ledger.stop(99));
    }
    #[test]
    fn native_demand_evicted_ids_stay_retired_and_metadata_reset_does_not_reset_native_revision() {
        let mut ledger = Ledger::default();
        for id in 1..=80 {
            ledger.apply(source(), 1, id, config(), None).unwrap();
        }
        assert_eq!(ledger.records.len(), LIMIT);
        assert_eq!(ledger.records.capacity(), LIMIT);
        assert!(ledger.allocated_bytes() < 64 * 1024);
        assert!(matches!(ledger.classify(1, config()), Decision::Reject(DemandRejection::Stale)));
        ledger.reset_domain();
        assert!(ledger.current().is_none());
        let receipt = ledger.apply(source(), 2, 1, config(), None).unwrap();
        assert!(matches!(receipt.outcome, DemandOutcome::Accepted { native_revision: 81, .. }));
    }
    #[test]
    fn native_demand_rejected_and_exhausted_application_never_reports_effective_success() {
        let mut ledger = Ledger::default();
        let denied = ledger.apply(source(), 1, 1, config(), Some(DemandRejection::Denied)).unwrap();
        assert!(ledger.current().is_none());
        assert_eq!(ledger.apply(source(), 1, 1, config(), None), Some(denied));
        assert!(!ledger.can_stop(1));
        ledger.revision = u64::MAX;
        let exhausted = ledger.apply(source(), 1, 2, config(), None).unwrap();
        assert!(matches!(
            exhausted.outcome,
            DemandOutcome::Rejected {
                reason: DemandRejection::Denied
            }
        ));
        assert!(ledger.current().is_none());
        assert!(matches!(ledger.classify(3, config()), Decision::Reject(DemandRejection::Denied)));
    }
    #[test]
    fn native_demand_rejection_flood_keeps_current_receipt_and_exact_stop_available() {
        let mut ledger = Ledger::default();
        let first = ledger.apply(source(), 1, 1, config(), None).unwrap();
        for id in 2..=96 {
            ledger.apply(source(), 1, id, config(), Some(DemandRejection::Expired)).unwrap();
        }
        assert_eq!(ledger.records.len(), LIMIT);
        assert_eq!(ledger.records.capacity(), LIMIT);
        assert_eq!(ledger.current().unwrap().demand, 1);
        assert_eq!(ledger.apply(source(), 1, 1, config(), None), Some(first));
        assert!(ledger.stop(1));
        assert!(ledger.current().is_none());
        assert!(ledger.stop(1));
    }
    #[test]
    fn native_demand_invalid_connection_does_not_mutate_current_or_receipt_highwater() {
        let mut ledger = Ledger::default();
        ledger.apply(source(), 1, 1, config(), None).unwrap();
        let before = ledger.current();
        assert!(ledger.apply(source(), 0, 2, config(), None).is_none());
        assert!(ledger.apply(source(), 1, 0, config(), None).is_none());
        assert_eq!(ledger.current(), before);
        assert!(matches!(ledger.classify(2, config()), Decision::Apply));
        let wrong = ledger.apply(source(), 2, 1, config(), None).unwrap();
        assert!(matches!(
            wrong.outcome,
            DemandOutcome::Rejected {
                reason: DemandRejection::Stale
            }
        ));
        assert_eq!(ledger.current(), before);
    }
}
