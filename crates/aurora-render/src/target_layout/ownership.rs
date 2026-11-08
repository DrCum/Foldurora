//! Disabled generic factory recorder. Synthetic retirement below is an
//! ownership model, not queue/GPU/native completion authority. Promotion needs
//! an actual last-submission callback adapter that cannot be caller-forged.

use super::descriptor::{Bounded, DeviceCaps};
use super::inventory::{Entry, MAX_PLAN_RECORDS, Payloads};
use super::{Error, next_owner};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DeviceId(u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AllocationId {
    device: DeviceId,
    ordinal: u64,
}

pub(super) struct DeviceIds {
    device: DeviceId,
    next: AtomicU64,
}
impl DeviceIds {
    pub fn new() -> Result<Arc<Self>, Error> {
        Ok(Arc::new(Self {
            device: DeviceId(next_owner()?),
            next: AtomicU64::new(1),
        }))
    }
    fn allocate(&self) -> Result<AllocationId, Error> {
        let ordinal = self
            .next
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| Error::Exhausted)?;
        Ok(AllocationId {
            device: self.device,
            ordinal,
        })
    }
}

/// No numeric defaults and no claim that requested blocks equal physical VRAM.
/// A future approved headroom/allocator policy is a separate mandatory input.
#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub payloads: Payloads,
    pub preparation: u64,
    pub driver_headroom: u64,
}
impl Limits {
    pub fn prospective_gpu_ceiling(self) -> Result<u64, Error> {
        if self.payloads.0.contains(&0) || self.preparation == 0 || self.driver_headroom == 0 {
            return Err(Error::MissingDependencies);
        }
        self.payloads
            .0
            .into_iter()
            .try_fold(self.driver_headroom, |sum, bytes| sum.checked_add(bytes).ok_or(Error::Overflow))
    }
}

struct Pool {
    limits: Limits,
    used: [AtomicU64; 4],
    preparation: AtomicU64,
}
#[derive(Clone)]
pub(super) struct Ledger(Arc<Pool>);
struct Credit {
    ledger: Ledger,
    class: usize,
    bytes: u64,
    metadata: u64,
}
impl Ledger {
    pub fn new(limits: Limits) -> Result<Self, Error> {
        limits.prospective_gpu_ceiling()?;
        Ok(Self(Arc::new(Pool {
            limits,
            used: std::array::from_fn(|_| AtomicU64::new(0)),
            preparation: AtomicU64::new(0),
        })))
    }
    pub fn used(&self) -> Payloads {
        Payloads(std::array::from_fn(|i| self.0.used[i].load(Ordering::Acquire)))
    }
    pub fn preparation(&self) -> u64 {
        self.0.preparation.load(Ordering::Acquire)
    }
    fn reserve(&self, class: usize, bytes: u64, metadata: u64) -> Result<Credit, Error> {
        self.0
            .preparation
            .try_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                old.checked_add(metadata).filter(|new| *new <= self.0.limits.preparation)
            })
            .map_err(|_| Error::Capacity)?;
        if self.0.used[class]
            .try_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                old.checked_add(bytes).filter(|new| *new <= self.0.limits.payloads.0[class])
            })
            .is_err()
        {
            self.0.preparation.fetch_sub(metadata, Ordering::AcqRel);
            return Err(Error::Capacity);
        }
        Ok(Credit {
            ledger: self.clone(),
            class,
            bytes,
            metadata,
        })
    }
}
impl Drop for Credit {
    fn drop(&mut self) {
        self.ledger.0.used[self.class].fetch_sub(self.bytes, Ordering::AcqRel);
        self.ledger.0.preparation.fetch_sub(self.metadata, Ordering::AcqRel);
    }
}

struct Allocation<T> {
    // Field order is deliberate: physical storage drops BEFORE credit refund.
    _resource: T,
    _credit: Credit,
}
/// No resource accessor, raw Arc, Weak, Deref or into_inner. Derived storage is
/// paired using hold(); neither its resource nor its parent's can escape.
pub(super) struct Lease<T> {
    id: AllocationId,
    entry: Entry,
    allocation: Arc<Allocation<T>>,
}
impl<T> Clone for Lease<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            entry: self.entry,
            allocation: self.allocation.clone(),
        }
    }
}
impl<T> Lease<T> {
    pub fn id(&self) -> AllocationId {
        self.id
    }
    pub fn entry(&self) -> Entry {
        self.entry
    }
    pub fn hold<U>(self, derived: U) -> Holder<T, U> {
        Holder {
            _derived: derived,
            _parent: self,
        }
    }
}
pub(super) struct Holder<T, U> {
    // A view/bind group/packet/map owner drops before its parent allocation.
    _derived: U,
    _parent: Lease<T>,
}

struct Reserved {
    entry: Entry,
    credit: Option<Credit>,
}

pub(super) struct Transaction<T> {
    ids: Arc<DeviceIds>,
    reserved: Bounded<Reserved>,
    // Rollback owns all successful allocations until exact finish. A returned
    // consumer lease can hold a failed partial factory's physical allocation;
    // its charge then correctly survives rollback rather than being refunded.
    made: Bounded<Lease<T>>,
    _storage: Credit,
}
impl<T> Transaction<T> {
    /// Reserve the ENTIRE requested bill before invoking any factory, including
    /// checked rollback/receipt slots. No unbounded allocation or HashMap grows.
    pub fn begin(ids: Arc<DeviceIds>, ledger: &Ledger, entries: &[Entry], caps: DeviceCaps<'_>) -> Result<Self, Error> {
        if entries.is_empty() || entries.len() > MAX_PLAN_RECORDS {
            return Err(Error::Capacity);
        }
        for (index, entry) in entries.iter().enumerate() {
            entry.validate(caps)?;
            if entries[..index]
                .iter()
                .any(|old| old.factory == entry.factory && old.index == entry.index)
            {
                return Err(Error::Invalid);
            }
        }
        // Reserve a hard upper bound BEFORE even allocating rollback vectors.
        // Allocator over-reservation is checked by Bounded against this cap.
        let storage_bound = std::mem::size_of::<Reserved>()
            .checked_add(std::mem::size_of::<Lease<T>>())
            .and_then(|size| size.checked_mul(MAX_PLAN_RECORDS))
            .and_then(|size| u64::try_from(size).ok())
            .ok_or(Error::Overflow)?;
        let storage_credit = ledger.reserve(0, 0, storage_bound)?;
        let reserved = Bounded::new(entries.len(), MAX_PLAN_RECORDS, storage_bound)?;
        let remaining = ledger.0.limits.preparation.checked_sub(reserved.storage()).ok_or(Error::Capacity)?;
        let made = Bounded::new(entries.len(), MAX_PLAN_RECORDS, remaining)?;
        let storage = reserved.storage().checked_add(made.storage()).ok_or(Error::Overflow)?;
        if storage > storage_bound {
            return Err(Error::Capacity);
        }
        let mut transaction = Self {
            ids,
            reserved,
            made,
            _storage: storage_credit,
        };
        for entry in entries {
            // Arc header is explicit requested metadata, not an assertion about
            // allocator overhead. Actual resource T's external payload is in its
            // descriptor, while its inline owner lives in this allocation.
            let metadata = std::mem::size_of::<Allocation<T>>()
                .checked_add(2 * std::mem::size_of::<usize>())
                .and_then(|n| u64::try_from(n).ok())
                .ok_or(Error::Overflow)?;
            let credit = ledger.reserve(entry.class as usize, entry.validate(caps)?, metadata)?;
            transaction.reserved.push(Reserved {
                entry: *entry,
                credit: Some(credit),
            })?;
        }
        Ok(transaction)
    }
    pub fn create(&mut self, slot: usize, actual: Entry, create: impl FnOnce() -> Result<T, Error>) -> Result<Lease<T>, Error> {
        let reserved = self.reserved.items_mut().get_mut(slot).ok_or(Error::Invalid)?;
        if reserved.entry != actual || reserved.credit.is_none() {
            return Err(Error::Invalid);
        }
        let id = self.ids.allocate()?;
        let credit = reserved.credit.take().ok_or(Error::Invalid)?;
        let resource = create()?;
        let lease = Lease {
            id,
            entry: actual,
            allocation: Arc::new(Allocation {
                _resource: resource,
                _credit: credit,
            }),
        };
        self.made.push(lease.clone())?;
        Ok(lease)
    }
    /// Omitted factories refuse commit. The resource leases, reservation slots
    /// and storage credit remain one owner with stable bounded capacity.
    pub fn finish(self) -> Result<Bundle<T>, Error> {
        if self.reserved.slice().iter().any(|slot| slot.credit.is_some()) || self.made.slice().len() != self.reserved.slice().len() {
            return Err(Error::MissingDependencies);
        }
        Ok(Bundle(self))
    }
}

pub(super) struct Bundle<T>(Transaction<T>);
impl<T> Bundle<T> {
    pub fn ids(&self) -> impl Iterator<Item = AllocationId> + '_ {
        self.0.made.slice().iter().map(Lease::id)
    }
    pub fn synthetic_retirement(self) -> SyntheticRetirement<T> {
        SyntheticRetirement { bundle: self }
    }
}
/// This token only models an independently retained last-submission owner.
/// There is intentionally no complete(bool)/set_done() API to confuse with a
/// real callback. Dropping it simulates that owner's storage retirement only.
pub(super) struct SyntheticRetirement<T> {
    bundle: Bundle<T>,
}
impl<T> SyntheticRetirement<T> {
    pub fn allocation_count(&self) -> usize {
        self.bundle.0.made.slice().len()
    }
}

#[cfg(test)]
mod exhaustion_tests {
    use super::*;

    #[test]
    fn allocation_ordinals_exhaust_without_reset_or_reuse() {
        let ids = DeviceIds::new().unwrap();
        ids.next.store(u64::MAX - 1, Ordering::Relaxed);
        let last = ids.allocate().unwrap();
        assert_eq!(last.ordinal, u64::MAX - 1);
        assert_eq!(ids.allocate(), Err(Error::Exhausted));
        assert_eq!(ids.allocate(), Err(Error::Exhausted));
        let other = DeviceIds::new().unwrap().allocate().unwrap();
        assert_ne!(last.device, other.device);
    }
}
