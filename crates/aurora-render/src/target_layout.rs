//! Dormant source-accounting prerequisite. Descriptors cover only the direct
//! textures in Backend::make_targets (renderer.rs), not a complete view/world.
//! Requested payload excludes driver tiling, heaps and allocation metadata.
//! No production allocator, render permission or numeric budget is changed.

mod generations;
mod layout;
mod tests;

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Invalid,
    Format,
    Limits,
    Overflow,
    MissingDependencies,
    Capacity,
    Busy,
    NoRequest,
    Exhausted,
    Allocation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PayloadCost(u64);

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
fn next_owner() -> Result<u64, Error> {
    NEXT_OWNER
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| Error::Exhausted)
}
