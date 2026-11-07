//! Correlate IO with exact native outcomes; this never applies configuration.
//! Actual native apply owns its separate immutable ledger. Evicted request IDs
//! stay retired behind the highwater; a stream reconnect never replays them.
use super::*;
use std::collections::VecDeque;
const LIMIT: usize = 32;
struct Record {
    id: u64,
    config: DemandConfiguration,
    receipt: Option<DemandReceipt>,
}
#[derive(Default)]
pub(super) struct DemandBook {
    records: VecDeque<Record>,
    high: u64,
    revision: u64,
    accepted: Option<u64>,
}
impl DemandBook {
    pub fn request(&mut self, id: u64, config: DemandConfiguration) -> Result<bool, Error> {
        if id == 0 {
            return Err(Error::Sequence);
        }
        if let Some(r) = self.records.iter().find(|r| r.id == id) {
            return if r.config == config { Ok(false) } else { Err(Error::Binding) };
        }
        if id <= self.high {
            return Err(Error::Sequence);
        }
        if self.records.len() >= LIMIT {
            let old = self
                .records
                .iter()
                .position(|r| r.receipt.is_some() && Some(r.id) != self.accepted)
                .ok_or(Error::Busy)?;
            self.records.remove(old);
        }
        self.records.push_back(Record { id, config, receipt: None });
        self.high = id;
        Ok(true)
    }
    pub fn finish(&mut self, receipt: DemandReceipt) -> Result<bool, Error> {
        receipt.validate()?;
        let r = self
            .records
            .iter_mut()
            .find(|r| r.id == receipt.demand && r.config == receipt.requested)
            .ok_or(Error::Binding)?;
        if let Some(old) = r.receipt {
            return if old == receipt { Ok(false) } else { Err(Error::Sequence) };
        }
        if let DemandOutcome::Accepted { native_revision, .. } = receipt.outcome {
            if native_revision <= self.revision || self.accepted.is_some_and(|id| id >= receipt.demand) {
                return Err(Error::Sequence);
            }
            self.revision = native_revision;
            self.accepted = Some(receipt.demand);
        }
        r.receipt = Some(receipt);
        Ok(true)
    }
    #[cfg(test)]
    pub fn allocated_bytes(&self) -> usize {
        self.records.capacity() * std::mem::size_of::<Record>()
    }
    pub fn can_stop(&self, demand: u64) -> bool {
        demand != 0 && self.accepted == Some(demand)
    }
    pub fn stopped(&mut self, demand: u64) -> Result<(), Error> {
        if !self.can_stop(demand) {
            return Err(Error::Binding);
        }
        self.accepted = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> DemandConfiguration {
        DemandConfiguration {
            size: PreviewSize::Small,
            rate: PreviewRate::One,
        }
    }
    fn receipt(id: u64, revision: u64) -> DemandReceipt {
        DemandReceipt {
            source: super::super::tests::source(1),
            metadata_connection: 1,
            demand: id,
            requested: config(),
            outcome: DemandOutcome::Accepted {
                effective: config(),
                native_revision: revision,
            },
        }
    }
    #[test]
    fn preview_io_demand_receipts_bind_original_config_immutable_duplicates_and_native_revision() {
        let mut book = DemandBook::default();
        assert!(book.request(1, config()).unwrap());
        assert!(!book.request(1, config()).unwrap());
        assert!(
            book.request(
                1,
                DemandConfiguration {
                    size: PreviewSize::Large,
                    ..config()
                }
            )
            .is_err()
        );
        let good = receipt(1, 9);
        assert!(
            book.finish(DemandReceipt {
                requested: DemandConfiguration {
                    size: PreviewSize::Large,
                    ..config()
                },
                ..good
            })
            .is_err()
        );
        assert!(book.finish(good).unwrap());
        assert!(!book.finish(good).unwrap());
        assert_eq!(book.records.iter().find(|r| r.id == 1).unwrap().receipt, Some(good));
        book.request(2, config()).unwrap();
        assert!(book.finish(receipt(2, 9)).is_err());
        assert!(book.finish(receipt(2, 10)).unwrap());
        assert!(!book.finish(good).unwrap());
        assert!(book.can_stop(2));
        assert!(!book.can_stop(1));
        assert!(book.stopped(1).is_err());
        assert!(book.can_stop(2));
        book.stopped(2).unwrap();
        assert!(!book.can_stop(2));
        assert!(book.request(0, config()).is_err());
    }
    #[test]
    fn preview_io_demand_rejection_never_changes_accepted_stop_and_eviction_never_reapplies_old_id() {
        let mut book = DemandBook::default();
        book.request(1, config()).unwrap();
        book.finish(receipt(1, 1)).unwrap();
        for id in 2..=40 {
            book.request(id, config()).unwrap();
            let rejection = DemandReceipt {
                outcome: DemandOutcome::Rejected {
                    reason: aurora_session::preview_wire::DemandRejection::Unsupported,
                },
                ..receipt(id, id)
            };
            book.finish(rejection).unwrap();
            assert!(book.can_stop(1));
        }
        assert_eq!(book.records.len(), LIMIT);
        assert!(book.request(2, config()).is_err());
        assert!(book.finish(receipt(2, 2)).is_err());
        assert!(book.can_stop(1));
    }
    #[test]
    fn preview_io_demand_unfinished_ledger_is_bounded_and_does_not_evict_pending_work() {
        let mut book = DemandBook::default();
        for id in 1..=LIMIT as u64 {
            book.request(id, config()).unwrap();
        }
        assert!(matches!(book.request(33, config()), Err(Error::Busy)));
        assert_eq!(book.records.len(), LIMIT);
        book.finish(receipt(2, 1)).unwrap();
        assert!(
            matches!(book.request(33, config()), Err(Error::Busy)),
            "current accepted Stop proof is retained"
        );
        book.finish(DemandReceipt {
            outcome: DemandOutcome::Rejected {
                reason: aurora_session::preview_wire::DemandRejection::Busy,
            },
            ..receipt(3, 1)
        })
        .unwrap();
        assert!(book.request(33, config()).unwrap());
        assert!(book.records.iter().any(|r| r.id == 1 && r.receipt.is_none()));
    }
}
