//! Disabled generic factory recorder. Synthetic retirement below is an
//! ownership model, not queue/GPU/native completion authority. Promotion needs
//! an actual last-submission callback adapter that cannot be caller-forged.

use super::descriptor::{Bounded, ComponentRole, DeviceCaps, Resource};
use super::inventory::{ComponentSpec, Entry, Holding, MAX_PLAN_RECORDS, Payloads, Record};
use super::{Error, next_owner};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct DeviceId(u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
    state: Option<(Arc<CreationState>, usize)>,
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
            state: None,
        })
    }
}
impl Drop for Credit {
    fn drop(&mut self) {
        // In Allocation this runs only AFTER owned physical storage drops.
        // A reserved component that never existed also becomes terminal Gone.
        if let Some((state, slot)) = &self.state {
            state.live[*slot].store(2, Ordering::Release);
        }
        self.ledger.0.used[self.class].fetch_sub(self.bytes, Ordering::AcqRel);
        self.ledger.0.preparation.fetch_sub(self.metadata, Ordering::AcqRel);
    }
}

// Account the two Arc counters and alignment before allocating its data.
// Allocator bookkeeping outside the requested block remains separate.
fn arc_bytes<T>() -> Result<u64, Error> {
    let alignment = std::mem::align_of::<T>().max(std::mem::align_of::<usize>());
    let counters = 2usize.checked_mul(std::mem::size_of::<usize>()).ok_or(Error::Overflow)?;
    let prefix = counters.checked_add(alignment - 1).ok_or(Error::Overflow)? / alignment;
    let prefix = prefix.checked_mul(alignment).ok_or(Error::Overflow)?;
    let bytes = prefix.checked_add(std::mem::size_of::<T>()).ok_or(Error::Overflow)?;
    u64::try_from(bytes).map_err(|_| Error::Overflow)
}
/// Private fixed creation-group metadata, separately credited through its last
/// internal Arc. States are Reserved=0, Live=1, Gone=2; no public setter exists.
struct CreationState {
    ids: [Option<AllocationId>; 2],
    live: [AtomicU8; 2],
    _metadata: Credit,
}
impl CreationState {
    fn required(&self) -> [Option<AllocationId>; 2] {
        std::array::from_fn(|i| self.ids[i].filter(|_| self.live[i].load(Ordering::Acquire) != 2))
    }
}
struct Allocation<T> {
    // Physical storage drops BEFORE the component publishes Gone/refunds credit.
    _resource: T,
    _credit: Credit,
}
/// No resource getter, raw Arc/Weak/Deref/into_inner. Every derived holder owns
/// its component's same opaque lease and drops its storage before that lease.
pub(super) struct Lease<T> {
    id: AllocationId,
    component: ComponentSpec,
    allocation: Arc<Allocation<T>>,
}
impl<T> Clone for Lease<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            component: self.component,
            allocation: self.allocation.clone(),
        }
    }
}
impl<T> Lease<T> {
    pub fn id(&self) -> AllocationId {
        self.id
    }
    pub fn entry(&self) -> Entry {
        self.component.origin()
    }
    pub fn component(&self) -> ComponentSpec {
        self.component
    }
    pub fn required_ids(&self) -> [Option<AllocationId>; 2] {
        self.allocation
            ._credit
            .state
            .as_ref()
            .map_or([Some(self.id), None], |(state, _)| state.required())
    }
    pub fn hold<U>(self, derived: U) -> Holder<T, U> {
        Holder {
            _derived: derived,
            _parent: self,
        }
    }
}
pub(super) struct Holder<T, U> {
    _derived: U,
    _parent: Lease<T>,
}
impl<T, U> Holder<T, U> {
    pub fn record(&self, holding: Holding) -> Record {
        Record::held(&self._parent, holding)
    }
}

struct Reserved {
    logical: usize,
    component: ComponentSpec,
    id: AllocationId,
    credit: Option<Credit>,
}
pub(super) struct Transaction<T> {
    reserved: Bounded<Reserved>,
    made: Bounded<Lease<T>>,
    _storage: Credit,
}
/// Factory inputs only: owned storage enters this fixed pair and has no getter.
/// The caller cannot obtain raw storage from a created/charged owner.
pub(super) struct BufferParts<T> {
    primary: T,
    staging: Option<T>,
}
impl<T> BufferParts<T> {
    pub fn new(primary: T, staging: Option<T>) -> Self {
        Self { primary, staging }
    }
}
/// No primary lease can escape the Init owner before modeled unmap.
pub(super) struct CreatedBuffer<T> {
    primary: Lease<T>,
    staging: Option<Lease<T>>,
}
impl<T> CreatedBuffer<T> {
    pub fn records(&self) -> [Option<Record>; 2] {
        [
            Some(Record::held(&self.primary, Holding::Resident)),
            self.staging.as_ref().map(|s| Record::held(s, Holding::Resident)),
        ]
    }
    pub fn synthetic_unmap(self) -> (Lease<T>, Option<SyntheticPendingUpload<T>>) {
        let Self { primary, staging } = self;
        let pending = staging.map(|staging| SyntheticPendingUpload {
            staging,
            primary: primary.clone(),
        });
        (primary, pending)
    }
}
/// MODEL ONLY. This is not a real queue adapter, submission/fence or completion
/// authority. It retains both physical components independently of app owners.
pub(super) struct SyntheticPendingUpload<T> {
    staging: Lease<T>,
    primary: Lease<T>,
}
impl<T> SyntheticPendingUpload<T> {
    pub fn records(&self) -> [Record; 2] {
        [
            Record::held(&self.primary, Holding::CopySource),
            Record::held(&self.staging, Holding::CopySource),
        ]
    }
    pub fn hold_staging<U>(&self, derived: U) -> Holder<T, U> {
        self.staging.clone().hold(derived)
    }
    pub fn synthetic_submit(self) -> SyntheticSubmittedUpload<T> {
        SyntheticSubmittedUpload(self)
    }
}
/// Dropping this models storage-owner destruction only, never actual GPU done.
pub(super) struct SyntheticSubmittedUpload<T>(SyntheticPendingUpload<T>);
impl<T> SyntheticSubmittedUpload<T> {
    pub fn records(&self) -> [Record; 2] {
        self.0.records()
    }
    pub fn hold_staging<U>(&self, derived: U) -> Holder<T, U> {
        self.0.hold_staging(derived)
    }
}
impl<T> Transaction<T> {
    pub fn begin(ids: Arc<DeviceIds>, ledger: &Ledger, entries: &[Entry], caps: DeviceCaps<'_>) -> Result<Self, Error> {
        if entries.is_empty() || entries.len() > MAX_PLAN_RECORDS {
            return Err(Error::Capacity);
        }
        let mut count = 0usize;
        for (index, entry) in entries.iter().enumerate() {
            let components = entry.components(caps)?;
            count = count.checked_add(components.into_iter().flatten().count()).ok_or(Error::Overflow)?;
            if count > MAX_PLAN_RECORDS {
                return Err(Error::Capacity);
            }
            if entries[..index]
                .iter()
                .any(|old| old.factory == entry.factory && old.index == entry.index)
            {
                return Err(Error::Invalid);
            }
        }
        let header = u64::try_from(std::mem::size_of::<Self>()).map_err(|_| Error::Overflow)?;
        let vector_bound = std::mem::size_of::<Reserved>()
            .checked_add(std::mem::size_of::<Lease<T>>())
            .and_then(|n| n.checked_mul(MAX_PLAN_RECORDS))
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(Error::Overflow)?;
        let storage_bound = vector_bound.checked_add(header).ok_or(Error::Overflow)?;
        let storage_credit = ledger.reserve(0, 0, storage_bound)?;
        let reserved = Bounded::new(count, MAX_PLAN_RECORDS, vector_bound)?;
        let made = Bounded::new(
            count,
            MAX_PLAN_RECORDS,
            vector_bound.checked_sub(reserved.storage()).ok_or(Error::Capacity)?,
        )?;
        if reserved.storage().checked_add(made.storage()).ok_or(Error::Overflow)? > vector_bound {
            return Err(Error::Capacity);
        }
        let mut transaction = Self {
            reserved,
            made,
            _storage: storage_credit,
        };
        for (logical, entry) in entries.iter().copied().enumerate() {
            let components = entry.components(caps)?;
            let component_ids = [
                Some(ids.allocate()?),
                if components[1].is_some() { Some(ids.allocate()?) } else { None },
            ];
            let metadata = arc_bytes::<CreationState>()?;
            let metadata_credit = ledger.reserve(0, 0, metadata)?;
            let state = Arc::new(CreationState {
                ids: component_ids,
                live: std::array::from_fn(|_| AtomicU8::new(0)),
                _metadata: metadata_credit,
            });
            for (slot, component) in components.into_iter().enumerate() {
                let Some(component) = component else {
                    continue;
                };
                let metadata = arc_bytes::<Allocation<T>>()?;
                let mut credit = ledger.reserve(entry.class as usize, component.payload(caps)?, metadata)?;
                credit.state = Some((state.clone(), slot));
                transaction.reserved.push(Reserved {
                    logical,
                    component,
                    id: component_ids[slot].ok_or(Error::Invalid)?,
                    credit: Some(credit),
                })?;
            }
        }
        Ok(transaction)
    }
    fn slot(&self, logical: usize, role: ComponentRole) -> Result<usize, Error> {
        self.reserved
            .slice()
            .iter()
            .position(|r| r.logical == logical && r.component.role() == role)
            .ok_or(Error::Invalid)
    }
    fn verify(&self, slot: usize, actual: Entry) -> Result<(), Error> {
        let reserved = self.reserved.slice().get(slot).ok_or(Error::Invalid)?;
        if reserved.component.origin() != actual || reserved.credit.is_none() {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    fn wrap(&mut self, slot: usize, credit: Credit, resource: T) -> Result<Lease<T>, Error> {
        let reserved = &self.reserved.slice()[slot];
        if let Some((state, component)) = &credit.state {
            state.live[*component].store(1, Ordering::Release);
        }
        let lease = Lease {
            id: reserved.id,
            component: reserved.component,
            allocation: Arc::new(Allocation {
                _resource: resource,
                _credit: credit,
            }),
        };
        self.made.push(lease.clone())?;
        Ok(lease)
    }
    pub fn create(&mut self, logical: usize, actual: Entry, create: impl FnOnce() -> Result<T, Error>) -> Result<Lease<T>, Error> {
        let slot = self.slot(logical, ComponentRole::Primary)?;
        self.verify(slot, actual)?;
        if self.slot(logical, ComponentRole::InitStaging).is_ok() {
            return Err(Error::MissingDependencies);
        }
        let credit = self.reserved.items_mut()[slot].credit.take().ok_or(Error::Invalid)?;
        let resource = create()?;
        self.wrap(slot, credit, resource)
    }
    pub fn create_buffer(
        &mut self,
        logical: usize,
        actual: Entry,
        create: impl FnOnce() -> Result<BufferParts<T>, Error>,
    ) -> Result<CreatedBuffer<T>, Error> {
        if !matches!(actual.resource, Resource::Buffer(_)) {
            return Err(Error::Invalid);
        }
        let primary_slot = self.slot(logical, ComponentRole::Primary)?;
        self.verify(primary_slot, actual)?;
        let stage_slot = self.slot(logical, ComponentRole::InitStaging).ok();
        if let Some(slot) = stage_slot {
            self.verify(slot, actual)?;
        }
        let primary_credit = self.reserved.items_mut()[primary_slot].credit.take().ok_or(Error::Invalid)?;
        let stage_credit = match stage_slot {
            Some(slot) => Some(self.reserved.items_mut()[slot].credit.take().ok_or(Error::Invalid)?),
            None => None,
        };
        let parts = create()?;
        if parts.staging.is_some() != stage_slot.is_some() {
            drop(parts); // actual storage before either reserved credit refunds
            return Err(Error::MissingDependencies);
        }
        let BufferParts { primary, staging } = parts;
        let primary = self.wrap(primary_slot, primary_credit, primary)?;
        let staging = match (stage_slot, stage_credit, staging) {
            (Some(slot), Some(credit), Some(resource)) => Some(self.wrap(slot, credit, resource)?),
            (None, None, None) => None,
            _ => return Err(Error::Invalid),
        };
        Ok(CreatedBuffer { primary, staging })
    }
    pub fn finish(self) -> Result<Bundle<T>, Error> {
        if self.reserved.slice().iter().any(|r| r.credit.is_some()) || self.made.slice().len() != self.reserved.slice().len() {
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
/// Storage ownership model, never a real completion flag or queue callback.
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
    #[test]
    fn initialized_pair_id_exhaustion_burns_first_id_and_rolls_back_all_credit() {
        let ids = DeviceIds::new().unwrap();
        ids.next.store(u64::MAX - 1, Ordering::Relaxed);
        let limits = wgpu::Limits::default();
        let formats = [];
        let caps = DeviceCaps {
            limits: &limits,
            formats: &formats,
            mappable_primary_buffers: false,
        };
        let ledger = Ledger::new(Limits {
            payloads: Payloads([100; 4]),
            preparation: 1_000_000,
            driver_headroom: 1,
        })
        .unwrap();
        let entry = Entry {
            factory: super::super::inventory::Factory::Uniforms,
            index: 0,
            class: super::super::inventory::Class::Buffer,
            resource: Resource::Buffer(super::super::descriptor::Buffer {
                size: 4,
                usage: wgpu::BufferUsages::UNIFORM,
                mapped: true,
            }),
        };
        assert!(matches!(
            Transaction::<[u8; 4]>::begin(ids.clone(), &ledger, &[entry], caps),
            Err(Error::Exhausted)
        ));
        assert_eq!(ids.allocate(), Err(Error::Exhausted));
        assert_eq!(ledger.used(), Payloads::default());
        assert_eq!(ledger.preparation(), 0);
    }
}
