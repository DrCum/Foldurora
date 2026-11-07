//! Bounded generic ownership model only. Future GPU wiring must register each
//! returned callback after the last queue submission using that exact bundle.
//! These callbacks are not native render permission or presentation proofs.

use super::layout::CoreTargetLayout;
use super::{Error, PayloadCost, next_owner};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

struct Pool {
    limit: u64,
    used: AtomicU64,
}
#[derive(Clone)]
pub(super) struct TargetBudget(Arc<Pool>);
struct Credit {
    budget: TargetBudget,
    bytes: u64,
}
impl TargetBudget {
    /// No default limit: the owner must inject an explicitly approved domain.
    pub fn new(limit: u64) -> Result<Self, Error> {
        if limit == 0 {
            return Err(Error::Invalid);
        }
        Ok(Self(Arc::new(Pool {
            limit,
            used: AtomicU64::new(0),
        })))
    }
    pub fn used(&self) -> u64 {
        self.0.used.load(Ordering::Acquire)
    }
    fn reserve(&self, PayloadCost(bytes): PayloadCost) -> Result<Credit, Error> {
        if bytes == 0 {
            return Err(Error::Invalid);
        }
        self.0
            .used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.0.limit)
            })
            .map_err(|_| Error::Capacity)?;
        Ok(Credit {
            budget: self.clone(),
            bytes,
        })
    }
}
impl Drop for Credit {
    fn drop(&mut self) {
        self.budget.0.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct GenerationId {
    owner: u64,
    generation: u64,
}
struct Allocation<T> {
    // Physical handles/resources must drop before their payload reservation.
    _resource: T,
    _credit: Credit,
}
/// Clones retain the same physical bundle and credit. No bare resource handle is
/// exported; a future backend must retain this lease with every submitted use.
pub(super) struct GenerationLease<T> {
    id: GenerationId,
    allocation: Arc<Allocation<T>>,
}
impl<T> Clone for GenerationLease<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            allocation: self.allocation.clone(),
        }
    }
}
impl<T> GenerationLease<T> {
    pub fn id(&self) -> GenerationId {
        self.id
    }
}
struct Bundle<T> {
    layout: CoreTargetLayout,
    lease: GenerationLease<T>,
}
struct Retiring<T> {
    bundle: Bundle<T>,
    done: Arc<AtomicBool>,
}
/// Clone models independently held callback references; all must drop as well
/// as completion being observed before the retirement slot is reusable.
#[must_use = "Register after the last submission, retaining its allocation until callback completion"]
pub(super) struct RetirementCallback<T> {
    lease: GenerationLease<T>,
    done: Arc<AtomicBool>,
}
impl<T> Clone for RetirementCallback<T> {
    fn clone(&self) -> Self {
        Self {
            lease: self.lease.clone(),
            done: self.done.clone(),
        }
    }
}
impl<T> RetirementCallback<T> {
    pub fn id(&self) -> GenerationId {
        self.lease.id()
    }
    /// The actual owner must call only from its ordered submitted-work callback;
    /// consuming a canceled CPU request cannot manufacture GPU completion.
    pub fn complete(self) {
        self.done.store(true, Ordering::Release);
    }
}
#[must_use = "Keep the accepted lease and register any returned retirement callback"]
pub(super) struct Admission<T> {
    pub lease: GenerationLease<T>,
    pub retirement: Option<RetirementCallback<T>>,
}
pub(super) struct TargetGenerations<T> {
    owner: u64,
    next: u64,
    exhausted: bool,
    budget: TargetBudget,
    active: Option<Bundle<T>>,
    retiring: Option<Retiring<T>>,
    wanted: Option<CoreTargetLayout>,
}
impl<T> TargetGenerations<T> {
    pub fn new(budget: TargetBudget) -> Result<Self, Error> {
        Ok(Self {
            owner: next_owner()?,
            next: 0,
            exhausted: false,
            budget,
            active: None,
            retiring: None,
            wanted: None,
        })
    }
    /// One latest metadata-only request. An invalid replacement cannot silently
    /// fall back to an older pending layout. Existing active ownership survives.
    pub fn request(&mut self, layout: CoreTargetLayout) -> Result<(), Error> {
        self.wanted = None;
        if self.exhausted {
            return Err(Error::Exhausted);
        }
        layout.reservable_core_payload()?;
        self.wanted = Some(layout);
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.wanted = None;
    }
    /// No GPU poll, next frame, queue lock or permission query is needed here.
    pub fn poll(&mut self) {
        if self
            .retiring
            .as_ref()
            .is_some_and(|retiring| retiring.done.load(Ordering::Acquire) && Arc::strong_count(&retiring.bundle.lease.allocation) == 1)
        {
            self.retiring = None;
        }
    }
    pub fn admit(&mut self, create: impl FnOnce(&CoreTargetLayout) -> Result<T, Error>) -> Result<Admission<T>, Error> {
        if self.exhausted {
            return Err(Error::Exhausted);
        }
        let layout = self.wanted.ok_or(Error::NoRequest)?;
        self.poll();
        if let Some(active) = &self.active
            && active.layout == layout
        {
            return Ok(Admission {
                lease: active.lease.clone(),
                retirement: None,
            });
        }
        if self.retiring.is_some() {
            return Err(Error::Busy);
        }
        let generation = self.next.checked_add(1).ok_or_else(|| {
            self.exhausted = true;
            self.wanted = None;
            Error::Exhausted
        })?;
        // Old active bytes remain charged through reserve and construction.
        // A failed factory drops its uncommitted reservation without replacing
        // active state. It must clean partially constructed handles before Err.
        let credit = self.budget.reserve(layout.reservable_core_payload()?)?;
        let resource = create(&layout)?;
        let lease = GenerationLease {
            id: GenerationId {
                owner: self.owner,
                generation,
            },
            allocation: Arc::new(Allocation {
                _resource: resource,
                _credit: credit,
            }),
        };
        self.next = generation;
        let retirement = self.active.take().map(|bundle| {
            let done = Arc::new(AtomicBool::new(false));
            let callback = RetirementCallback {
                lease: bundle.lease.clone(),
                done: done.clone(),
            };
            self.retiring = Some(Retiring { bundle, done });
            callback
        });
        self.active = Some(Bundle {
            layout,
            lease: lease.clone(),
        });
        Ok(Admission { lease, retirement })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target_layout::tests::layout;

    #[test]
    fn target_generation_counters_exhaust_without_reusing_ids_or_wrapping_shared_credit() {
        assert!(matches!(TargetBudget::new(0), Err(Error::Invalid)));
        let extreme = TargetBudget::new(u64::MAX).unwrap();
        let held = extreme.reserve(PayloadCost(u64::MAX)).unwrap();
        assert!(matches!(extreme.reserve(PayloadCost(1)), Err(Error::Capacity)));
        assert_eq!(extreme.used(), u64::MAX);
        drop(held);
        assert_eq!(extreme.used(), 0);
        let budget = TargetBudget::new(1000).unwrap();
        let mut owner = TargetGenerations::<()>::new(budget.clone()).unwrap();
        owner.request(layout([1, 1])).unwrap();
        let active = owner.admit(|_| Ok(())).unwrap();
        owner.next = u64::MAX;
        owner.request(layout([2, 1])).unwrap();
        assert!(matches!(
            owner.admit(|_| panic!("exhausted counter must not allocate")),
            Err(Error::Exhausted)
        ));
        assert_eq!(owner.request(layout([3, 1])), Err(Error::Exhausted));
        assert_eq!(budget.used(), 25);
        drop(active);
        drop(owner);
        assert_eq!(budget.used(), 0);
    }
}
