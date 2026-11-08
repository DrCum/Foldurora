//! Dormant source-accounting prerequisite. The protected core covers direct
//! make_targets textures; the extension describes a prospective complete family
//! graph with mandatory external allocation inventories. Actual current wgpu
//! factories/holders have not been migrated or recorder-verified. Requested
//! payload excludes driver tiling, heaps and allocator metadata. No production
//! allocator, render permission or numeric budget is changed.

mod descriptor;
mod generations;
mod inventory;
mod layout;
mod ownership;
mod tests;
mod world;
mod world_tests;

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
