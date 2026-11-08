//! Shared loaded-folder projection for LLInventoryFilter::passesArchive
//! (indra/newview/llinventoryfilter.cpp). No fetch, names, paths or item mirror.
//! The App adapter must advance ONE CpuTurn before its CPU event/draw turn and
//! lend that same turn to every caller. Observers only call `observe`.

use super::{MAX_DEPTH, Model, Revision, Source};
use crate::world::{World, inventory::Inventory};
use std::{
    collections::hash_map::RandomState,
    hash::BuildHasher,
    mem::size_of,
    ops::{Deref, DerefMut},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

const SLOTS: usize = 32_768;
const RECORDS: usize = 16_384;
const PROBES: usize = 64;
const BUNDLE_BYTES: usize = 2 * 1024 * 1024;
const PROCESS_BYTES: usize = 2 * BUNDLE_BYTES;
const MARKED: u8 = 1;
const TRASH: u8 = 2;
const LIBRARY: u8 = 4;
static POOL: Pool = Pool::new();
static NEXT_PUBLICATION: AtomicU64 = AtomicU64::new(1);

struct Pool {
    bytes: AtomicUsize,
}
impl Pool {
    const fn new() -> Self {
        Self {
            bytes: AtomicUsize::new(0),
        }
    }
    fn acquire(&'static self) -> Option<Lease> {
        self.bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(BUNDLE_BYTES).filter(|next| *next <= PROCESS_BYTES)
            })
            .ok()?;
        Some(Lease(self))
    }
}
struct Lease(&'static Pool);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.bytes.fetch_sub(BUNDLE_BYTES, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    source: Source,
    revision: Revision,
    generation: u64,
    privacy: u64,
    privacy_enabled: bool,
    known: bool,
}
impl Key {
    fn observe(world: &World, model: &Model) -> Option<Self> {
        model.include_allowed(world).then(|| Self {
            source: model.source,
            revision: Revision {
                instance: model.instance,
                publication: model.published.publication,
            },
            generation: world.inventory.generation,
            privacy: world.rlva.revision,
            privacy_enabled: world.rlva.enabled(),
            known: model.read(world).is_some(),
        })
    }
}

/// Owner-local visibility choice. Its checked revision also fences Folder Jump.
#[derive(Default)]
pub(crate) struct OwnerPolicy {
    key: Option<Key>,
    binding: Option<(Source, u64)>,
    include: bool,
    revision: u64,
    exhausted: bool,
    presentation: Option<(VisibilityStatus, Option<u64>)>,
}
impl OwnerPolicy {
    fn update(&mut self, world: &World, model: &Model, key: Option<Key>) {
        let binding = model.matches(world).then_some((model.source, model.instance));
        if self.key == key && self.binding == binding {
            return;
        }
        if self.binding != binding {
            self.include = false;
        }
        self.binding = binding;
        self.key = key;
        self.bump();
    }
    fn bump(&mut self) {
        match self.revision.checked_add(1) {
            Some(next) => self.revision = next,
            None => {
                self.exhausted = true;
                self.include = false;
            }
        }
    }
    fn present(&mut self, status: VisibilityStatus, publication: Option<u64>) {
        let presentation = Some((status, publication));
        if self.presentation != presentation {
            self.presentation = presentation;
            self.bump();
        }
    }
    pub(crate) fn set_include(&mut self, world: &World, model: &Model, include: bool) -> bool {
        self.update(world, model, Key::observe(world, model));
        if self.key.is_none() || self.exhausted {
            return false;
        }
        if self.include != include {
            self.include = include;
            self.bump();
        }
        !self.exhausted
    }
    pub(crate) fn revision(&self) -> Option<u64> {
        (!self.exhausted).then_some(self.revision)
    }
}

/// Reusable App-owned CPU-turn budget. `advance` is called only by the outer
/// CPU loop, never by a view/framework or an invalidation observer.
pub(crate) struct CpuTurn {
    stamp: u64,
    started: Instant,
    used: Work,
    limits: Work,
    exhausted: bool,
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct Work {
    pub(super) nodes: usize,
    pub(super) edges: usize,
    pub(super) probes: usize,
}
impl Default for CpuTurn {
    fn default() -> Self {
        Self {
            stamp: 0,
            started: Instant::now(),
            used: Work::default(),
            limits: Work {
                nodes: 256,
                edges: 512,
                probes: 1024,
            },
            exhausted: true,
        }
    }
}
impl CpuTurn {
    pub(crate) fn advance(&mut self) -> bool {
        let Some(next) = self.stamp.checked_add(1) else {
            self.exhausted = true;
            return false;
        };
        self.stamp = next;
        self.started = Instant::now();
        self.used = Work::default();
        self.exhausted = false;
        true
    }
    fn available(&self) -> bool {
        !self.exhausted
            && self.started.elapsed() < Duration::from_millis(2)
            && self.used.nodes < self.limits.nodes
            && self.used.edges < self.limits.edges
            && self.used.probes < self.limits.probes
    }
}

#[derive(Clone, Copy, Default)]
struct Slot {
    id: Uuid,
    bits: u8,
}
#[derive(Clone, Copy, Default)]
struct Frame {
    id: Uuid,
    next: usize,
    bits: u8,
}
#[derive(Clone, Copy)]
struct Candidate {
    id: Uuid,
    bits: u8,
    depth: usize,
    index: usize,
    probes: usize,
}
struct Bundle {
    slots: Vec<Slot>,
    stack: [Frame; MAX_DEPTH + 1],
    stack_len: usize,
    hash: RandomState,
    records: usize,
}
/// No bare storage Arc/Weak escapes. Each clone keeps this exact pair; dropping
/// the last pair destroys AND deallocates storage before refunding the lease.
#[derive(Clone)]
struct Owned {
    storage: Arc<Bundle>,
    _lease: Arc<Lease>,
}
impl Deref for Owned {
    type Target = Bundle;
    fn deref(&self) -> &Bundle {
        &self.storage
    }
}
impl DerefMut for Owned {
    fn deref_mut(&mut self) -> &mut Bundle {
        // Only the unpublished builder mutates; snapshot pairs are read-only.
        Arc::get_mut(&mut self.storage).expect("unique unpublished projection storage")
    }
}
impl Bundle {
    fn bytes(capacity: usize) -> Option<usize> {
        // Includes fixed DFS stack, Vec/hasher metadata, paired ownership and
        // both Arc headers. Conservative extra Bundle covers construction moves.
        capacity
            .checked_mul(size_of::<Slot>())?
            .checked_add(2 * size_of::<Self>())?
            .checked_add(size_of::<Builder>() + size_of::<Snapshot>() + size_of::<Lease>() + 4 * size_of::<usize>())
    }
    fn allocate(pool: &'static Pool, config: Config) -> Result<Owned, Allocation> {
        let lease = pool.acquire().ok_or(Allocation::Busy)?;
        if Self::bytes(config.slots).is_none_or(|n| n > BUNDLE_BYTES) {
            return Err(Allocation::Limited);
        }
        let mut slots = Vec::new();
        slots.try_reserve_exact(config.slots).map_err(|_| Allocation::Limited)?;
        if Self::bytes(slots.capacity()).is_none_or(|n| n > BUNDLE_BYTES) {
            return Err(Allocation::Limited);
        }
        // Stable Arc has standard global-allocator OOM behavior. These bounded,
        // accounted metadata allocations happen up front under the lease, not
        // at publication. The variable table reserve above is fallible.
        Ok(Owned {
            storage: Arc::new(Self {
                slots,
                stack: [Frame::default(); MAX_DEPTH + 1],
                stack_len: 0,
                hash: RandomState::new(),
                records: 0,
            }),
            _lease: Arc::new(lease),
        })
    }
    fn index(&self, id: Uuid, config: Config) -> usize {
        #[cfg(test)]
        if config.collide {
            return 0;
        }
        self.hash.hash_one(id) as usize % config.slots
    }
    fn lookup(&self, id: Uuid, config: Config) -> Option<u8> {
        let start = self.index(id, config);
        for offset in 0..config.probes {
            let slot = self.slots.get((start + offset) % config.slots)?;
            if slot.id == id && !id.is_nil() {
                return Some(slot.bits);
            }
            if slot.id.is_nil() {
                return None;
            }
        }
        None
    }
}
#[derive(Clone, Copy)]
struct Config {
    slots: usize,
    records: usize,
    probes: usize,
    depth: usize,
    #[cfg(test)]
    collide: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            slots: SLOTS,
            records: RECORDS,
            probes: PROBES,
            depth: MAX_DEPTH,
            #[cfg(test)]
            collide: false,
        }
    }
}
enum Allocation {
    Busy,
    Limited,
}
struct Builder {
    bundle: Owned,
    pending: Option<Candidate>,
    root: u8,
}
impl Builder {
    fn candidate(&self, inv: &Inventory, model: &Model, id: Uuid, parent: Option<Frame>, config: Config) -> Option<Candidate> {
        let folder = inv.folders.get(&id)?;
        let depth = parent.map_or(0, |_| self.bundle.stack_len);
        let library = parent.map_or(id == inv.lib_root, |p| p.bits & LIBRARY != 0);
        if id.is_nil()
            || folder.info.id != id
            || depth > config.depth
            || folder.library != library
            || (!library && id == inv.lib_root)
            || (library && id == inv.root)
            || parent.is_some_and(|p| folder.info.parent != p.id || id == inv.root || id == inv.lib_root)
        {
            return None;
        }
        let mut bits = parent.map_or(if library { LIBRARY } else { 0 }, |p| p.bits);
        if folder.info.type_default == 14 {
            bits |= TRASH;
        }
        if model.published.folders.contains(&id) {
            bits |= MARKED;
        }
        Some(Candidate {
            id,
            bits,
            depth,
            index: self.bundle.index(id, config),
            probes: 0,
        })
    }
    /// One resumable initialization slot, hash probe, root or child edge.
    fn step(&mut self, world: &World, model: &Model, turn: &mut CpuTurn, config: Config) -> Step {
        if self.bundle.slots.len() < config.slots {
            self.bundle.slots.push(Slot::default());
            turn.used.probes += 1;
            return Step::More;
        }
        if let Some(mut candidate) = self.pending.take() {
            turn.used.probes += 1;
            let bundle: &mut Bundle = &mut self.bundle;
            let slot = &mut bundle.slots[candidate.index];
            if slot.id == candidate.id {
                return Step::Limited;
            }
            if slot.id.is_nil() {
                if bundle.records >= config.records {
                    return Step::Limited;
                }
                *slot = Slot {
                    id: candidate.id,
                    bits: candidate.bits,
                };
                bundle.records += 1;
                turn.used.nodes += 1;
                bundle.stack[candidate.depth] = Frame {
                    id: candidate.id,
                    next: 0,
                    bits: candidate.bits,
                };
                bundle.stack_len = candidate.depth + 1;
                return Step::More;
            }
            candidate.probes += 1;
            if candidate.probes >= config.probes {
                return Step::Limited;
            }
            candidate.index = (candidate.index + 1) % config.slots;
            self.pending = Some(candidate);
            return Step::More;
        }
        if self.bundle.stack_len == 0 {
            let id = match self.root {
                0 => world.inventory.root,
                1 if !world.inventory.lib_root.is_nil() => world.inventory.lib_root,
                _ => return Step::Complete,
            };
            self.root += 1;
            self.pending = self.candidate(&world.inventory, model, id, None, config);
            return if self.pending.is_some() { Step::More } else { Step::Limited };
        }
        let last = self.bundle.stack_len - 1;
        let frame = self.bundle.stack[last];
        let Some(folder) = world.inventory.folders.get(&frame.id) else {
            return Step::Limited;
        };
        let Some(&child) = folder.children.get(frame.next) else {
            self.bundle.stack_len -= 1;
            return Step::More;
        };
        self.bundle.stack[last].next += 1;
        turn.used.edges += 1;
        self.pending = self.candidate(&world.inventory, model, child, Some(frame), config);
        if self.pending.is_some() { Step::More } else { Step::Limited }
    }
}
enum Step {
    More,
    Complete,
    Limited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VisibilityStatus {
    Unavailable,
    Loading,
    Ready,
    Limited,
}
struct Snapshot {
    key: Key,
    publication: u64,
    bundle: Owned,
    status: VisibilityStatus,
}

/// One World-owned owner. A source replacement never replaces the static pool.
pub(crate) struct Projection {
    key: Option<Key>,
    builder: Option<Builder>,
    snapshot: Option<Snapshot>,
    stopped: bool,
    pool: &'static Pool,
    config: Config,
}
impl Default for Projection {
    fn default() -> Self {
        Self {
            key: None,
            builder: None,
            snapshot: None,
            stopped: false,
            pool: &POOL,
            config: Config::default(),
        }
    }
}
impl Projection {
    /// O(1) revocation; no observer performs projection work or grants a budget.
    pub(crate) fn observe(&mut self, world: &World, model: &Model) {
        let key = Key::observe(world, model);
        if key != self.key {
            self.key = key;
            self.builder = None;
            self.snapshot = None;
            self.stopped = false;
        }
    }
    pub(crate) fn pump(&mut self, world: &World, model: &Model, turn: &mut CpuTurn) {
        self.observe(world, model);
        let Some(key) = self.key else {
            return;
        };
        if !key.known || model.published.folders.is_empty() || self.snapshot.is_some() || self.stopped || !turn.available() {
            return;
        }
        if self.builder.is_none() {
            let bundle = match Bundle::allocate(self.pool, self.config) {
                Ok(bundle) => bundle,
                Err(Allocation::Busy) => return,
                Err(Allocation::Limited) => {
                    self.stopped = true;
                    return;
                }
            };
            self.builder = Some(Builder {
                bundle,
                pending: None,
                root: 0,
            });
        }
        while turn.available() {
            let step = self
                .builder
                .as_mut()
                .expect("admitted builder")
                .step(world, model, turn, self.config);
            let status = match step {
                Step::More => continue,
                Step::Complete => VisibilityStatus::Ready,
                Step::Limited => VisibilityStatus::Limited,
            };
            let builder = self.builder.take().expect("finished builder");
            let Ok(publication) = NEXT_PUBLICATION.try_update(Ordering::AcqRel, Ordering::Acquire, |next| next.checked_add(1)) else {
                self.stopped = true;
                break;
            };
            self.snapshot = Some(Snapshot {
                key,
                publication,
                bundle: builder.bundle,
                status,
            });
            break;
        }
    }
    pub(crate) fn read<'a>(&'a self, world: &'a World, model: &'a Model, policy: &'a mut OwnerPolicy) -> VisibilityRead<'a> {
        let key = Key::observe(world, model);
        policy.update(world, model, key);
        let current = key.is_some() && !policy.exhausted;
        let bypass = current && (policy.include || (key.is_some_and(|k| k.known) && model.published.folders.is_empty()));
        let snapshot = self.snapshot.as_ref().filter(|s| current && self.key == key && Some(s.key) == key);
        let mut status = if !current {
            VisibilityStatus::Unavailable
        } else if bypass {
            VisibilityStatus::Ready
        } else if let Some(snapshot) = snapshot {
            snapshot.status
        } else if self.key == key && self.stopped {
            VisibilityStatus::Limited
        } else {
            VisibilityStatus::Loading
        };
        // Loading -> published (including Limited) must invalidate caches even
        // when Inventory and archive metadata did not change in between.
        policy.present(status, snapshot.map(|s| s.publication));
        if policy.exhausted {
            status = VisibilityStatus::Unavailable;
        }
        VisibilityRead {
            world,
            model,
            key,
            snapshot,
            bypass,
            status,
            config: self.config,
            revision: policy.revision(),
            _policy: policy,
        }
    }
}

/// Borrowed actual source/privacy proof plus immutable projection. Snapshots
/// alone cannot authorize anything; every consumer obtains a fresh read.
pub(crate) struct VisibilityRead<'a> {
    world: &'a World,
    model: &'a Model,
    key: Option<Key>,
    snapshot: Option<&'a Snapshot>,
    bypass: bool,
    status: VisibilityStatus,
    config: Config,
    revision: Option<u64>,
    // A held read also prevents Include/policy mutation until its paint ends.
    _policy: &'a OwnerPolicy,
}
impl VisibilityRead<'_> {
    pub(crate) fn status(&self) -> VisibilityStatus {
        self.status
    }
    pub(crate) fn revision(&self) -> Option<u64> {
        self.revision
    }
    pub(crate) fn passes(&self, id: Uuid) -> bool {
        if self.revision.is_none() || self.key.is_none() || Key::observe(self.world, self.model) != self.key || id.is_nil() {
            return false;
        }
        let inv = &self.world.inventory;
        let folder = if let Some(folder) = inv.folders.get(&id) {
            if folder.info.id != id {
                return false;
            }
            id
        } else if let Some(item) = inv.items.get(&id) {
            if item.id != id {
                return false;
            }
            item.parent // Links use their own actual parent, never asset_id.
        } else {
            return false;
        };
        if self.bypass {
            return inv.folders.get(&folder).is_some_and(|f| f.info.id == folder);
        }
        self.snapshot
            .and_then(|s| s.bundle.lookup(folder, self.config))
            .is_some_and(|bits| bits & MARKED == 0 || bits & (TRASH | LIBRARY) != 0)
    }
}

#[cfg(test)]
pub(super) mod testing {
    use super::*;
    pub(in crate::world::inventory_archive) fn projection(
        slots: usize,
        records: usize,
        probes: usize,
        depth: usize,
        collide: bool,
    ) -> Projection {
        assert!(slots > 0 && slots <= SLOTS && records <= slots / 2 && probes > 0 && probes <= PROBES && depth <= MAX_DEPTH);
        Projection {
            pool: Box::leak(Box::new(Pool::new())),
            config: Config {
                slots,
                records,
                probes,
                depth,
                collide,
            },
            ..Projection::default()
        }
    }
    pub(in crate::world::inventory_archive) fn replacement(projection: &Projection) -> Projection {
        Projection {
            pool: projection.pool,
            config: projection.config,
            ..Projection::default()
        }
    }
    pub(in crate::world::inventory_archive) fn limits(turn: &mut CpuTurn, nodes: usize, edges: usize, probes: usize) {
        turn.limits = Work { nodes, edges, probes };
    }
    pub(in crate::world::inventory_archive) fn used(turn: &CpuTurn) -> Work {
        turn.used
    }
    pub(in crate::world::inventory_archive) fn retained(projection: &Projection) -> Option<impl Clone + use<>> {
        projection.snapshot.as_ref().map(|s| s.bundle.clone())
    }
    pub(in crate::world::inventory_archive) fn bytes(projection: &Projection) -> usize {
        projection.pool.bytes.load(Ordering::Acquire)
    }
    pub(in crate::world::inventory_archive) fn initialized(projection: &Projection) -> usize {
        projection.builder.as_ref().map_or(0, |b| b.bundle.slots.len())
    }
    pub(in crate::world::inventory_archive) fn payload(projection: &Projection) -> Option<usize> {
        let bundle = projection
            .builder
            .as_ref()
            .map(|b| &b.bundle)
            .or_else(|| projection.snapshot.as_ref().map(|s| &s.bundle))?;
        Bundle::bytes(bundle.slots.capacity())
    }
    pub(in crate::world::inventory_archive) fn owner_exhaust(policy: &mut OwnerPolicy) {
        policy.revision = u64::MAX;
    }
}
