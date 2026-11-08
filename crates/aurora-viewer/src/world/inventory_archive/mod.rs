//! Local Inventory visibility, not Inventory mutation or access permission.
//! Behavior: fsinventoryarchive.h / LLInventoryFilter::passesArchive
//! (Folderstorm abbe3cb0, indra/newview, LGPL-2.1-or-later).
//! Actual ordinary consumers share one borrowed projection and local metadata owner.

mod codec;
pub(crate) mod controller;
pub(crate) mod offline;
mod owner;
mod store;
pub(crate) mod visibility;
use super::{World, account_metadata_origin::GridDigest, inventory::Inventory};
pub(crate) use codec::Document;
pub(crate) use owner::fresh_inventory_instance;
use std::collections::BTreeSet;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use uuid::Uuid;

pub(crate) const MAX_FLAGS: usize = 2048;
pub(crate) const MAX_BYTES: usize = 128 * 1024;
pub(crate) const MAX_DEPTH: usize = 256;
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Source {
    account: Uuid,
    session: Uuid,
    root: Uuid,
    grid: GridDigest,
}
impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ArchiveSource([redacted])")
    }
}
impl Source {
    fn observe(world: &World, grid: GridDigest) -> Option<Self> {
        let login = world.login.as_ref()?;
        let source = Self {
            account: world.agent_id,
            session: login.session_id,
            root: world.inventory.root,
            grid,
        };
        source.matches(world).then_some(source)
    }
    fn matches(self, world: &World) -> bool {
        !self.account.is_nil()
            && !self.session.is_nil()
            && !self.root.is_nil()
            && self.root != world.inventory.lib_root
            && world.agent_id == self.account
            && world.inventory.owner == self.account
            && world.inventory.root == self.root
            && world.login.as_ref().is_some_and(|login| {
                login.agent_id == self.account && login.session_id == self.session && login.inventory_root == self.root
            })
            && world
                .inventory
                .folders
                .get(&self.root)
                .is_some_and(|folder| folder.info.id == self.root && !folder.library && folder.info.type_default != 14)
    }
    fn available(self, world: &World) -> bool {
        self.matches(world) && !world.rlva.has("showinv")
    }
    pub(crate) fn account(self) -> Uuid {
        self.account
    }
    pub(crate) fn grid(self) -> GridDigest {
        self.grid
    }
}

/// A single explicit target check, never a full-tree visibility projection.
/// Only actual ordinary/outfit categories, below own root and outside Trash.
pub(crate) fn eligible(inv: &Inventory, id: Uuid) -> bool {
    if id.is_nil() || inv.owner.is_nil() || inv.root.is_nil() || id == inv.root || inv.root == inv.lib_root {
        return false;
    }
    let Some(folder) = inv.folders.get(&id) else {
        return false;
    };
    if !matches!(folder.info.type_default, -1 | 47) {
        return false;
    }
    let mut at = id;
    for _ in 0..MAX_DEPTH {
        if at == inv.lib_root {
            return false;
        }
        let Some(folder) = inv.folders.get(&at) else {
            return false;
        };
        if folder.info.id != at || folder.library || folder.info.type_default == 14 {
            return false;
        }
        if at == inv.root {
            return true;
        }
        at = folder.info.parent;
        if at.is_nil() {
            return false;
        }
    }
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Unavailable,
    Invalid,
    Scope,
    Capacity,
    Exhausted,
    Stale,
    Busy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Loading,
    Ready,
    Saving,
    LoadFailed,
    SaveFailed,
    ReadOnly,
    Retired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Revision {
    instance: u64,
    publication: u64,
}
pub(crate) struct Read<'a> {
    source: Source,
    revision: Revision,
    folders: &'a BTreeSet<Uuid>,
}
impl Read<'_> {
    pub(crate) fn source(&self) -> Source {
        self.source
    }
    pub(crate) fn revision(&self) -> Revision {
        self.revision
    }
    pub(crate) fn marked(&self, id: Uuid) -> bool {
        self.folders.contains(&id)
    }
    pub(crate) fn folders(&self) -> impl ExactSizeIterator<Item = Uuid> + '_ {
        self.folders.iter().copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LoadTicket {
    instance: u64,
    source: Source,
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct EditTicket {
    instance: u64,
    source: Source,
    revision: u64,
    item: Uuid,
    previous: bool,
    parent: Uuid,
    kind: i32,
    inventory_generation: u64,
    rlva_revision: u64,
    rlva_enabled: bool,
}
impl std::fmt::Debug for EditTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ArchiveEditTicket([redacted])")
    }
}
impl EditTicket {
    pub(crate) fn previous(&self) -> bool {
        self.previous
    }
    pub(crate) fn namespace(&self) -> (u64, u64, Uuid) {
        (self.instance, self.revision, self.item)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SaveIntent {
    instance: u64,
    source: Source,
    operation: u64,
    expected: Document,
    next: Document,
}
impl SaveIntent {
    pub(crate) fn operation(&self) -> u64 {
        self.operation
    }
    pub(crate) fn source(&self) -> Source {
        self.source
    }
    pub(crate) fn expected(&self) -> &Document {
        &self.expected
    }
    pub(crate) fn next(&self) -> &Document {
        &self.next
    }
}

pub(crate) struct Model {
    instance: u64,
    source: Source,
    status: Status,
    published: Document,
    next_operation: u64,
    pending: Option<SaveIntent>,
}
impl Model {
    pub(crate) fn new(world: &World, grid: GridDigest) -> Result<Self, Error> {
        let source = Source::observe(world, grid).ok_or(Error::Unavailable)?;
        let instance = NEXT_INSTANCE
            .try_update(Ordering::AcqRel, Ordering::Acquire, |next| next.checked_add(1))
            .map_err(|_| Error::Exhausted)?;
        Ok(Self {
            instance,
            source,
            status: Status::Loading,
            published: Document::empty(source),
            next_operation: 1,
            pending: None,
        })
    }
    pub(crate) fn status(&self) -> Status {
        self.status
    }
    pub(crate) fn source(&self) -> Source {
        self.source
    }
    pub(crate) fn matches(&self, world: &World) -> bool {
        self.status != Status::Retired && self.source.matches(world)
    }
    pub(crate) fn include_allowed(&self, world: &World) -> bool {
        self.status != Status::Retired && self.source.available(world)
    }
    pub(crate) fn load_ticket(&self) -> Option<LoadTicket> {
        (self.status == Status::Loading).then_some(LoadTicket {
            instance: self.instance,
            source: self.source,
        })
    }
    pub(crate) fn complete_load(&mut self, ticket: LoadTicket, result: Result<Option<Document>, Error>) -> bool {
        if self.load_ticket() != Some(ticket) {
            return false;
        }
        match result {
            Ok(document) => {
                if document.as_ref().is_some_and(|document| !document.valid_published()) {
                    self.status = Status::LoadFailed;
                    return true;
                }
                let document = document.unwrap_or_else(|| Document::empty(self.source));
                if !document.matches(self.source) {
                    self.status = Status::LoadFailed;
                    return true;
                }
                self.published = document;
                self.status = Status::Ready;
            }
            Err(_) => self.status = Status::LoadFailed,
        }
        true
    }
    pub(crate) fn read(&self, world: &World) -> Option<Read<'_>> {
        if !self.source.available(world) || !matches!(self.status, Status::Ready | Status::Saving | Status::SaveFailed | Status::ReadOnly) {
            return None;
        }
        Some(Read {
            source: self.source,
            revision: Revision {
                instance: self.instance,
                publication: self.published.publication,
            },
            folders: &self.published.folders,
        })
    }
    pub(crate) fn prepare_edit(&self, world: &World, item: Uuid) -> Option<EditTicket> {
        if !matches!(self.status, Status::Ready | Status::SaveFailed) || !self.source.available(world) || !eligible(&world.inventory, item)
        {
            return None;
        }
        let folder = world.inventory.folders.get(&item)?;
        Some(EditTicket {
            instance: self.instance,
            source: self.source,
            revision: self.next_operation,
            item,
            previous: self.published.folders.contains(&item),
            parent: folder.info.parent,
            kind: folder.info.type_default,
            inventory_generation: world.inventory.generation,
            rlva_revision: world.rlva.revision,
            rlva_enabled: world.rlva.enabled(),
        })
    }
    pub(crate) fn begin_save(&mut self, world: &World, ticket: EditTicket, desired: bool) -> Result<SaveIntent, Error> {
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        if self.prepare_edit(world, ticket.item).as_ref() != Some(&ticket) {
            return Err(Error::Stale);
        }
        if desired == ticket.previous {
            return Err(Error::Invalid);
        }
        let mut folders = (*self.published.folders).clone();
        if desired {
            folders.insert(ticket.item);
        } else {
            folders.remove(&ticket.item);
        }
        if folders.len() > MAX_FLAGS {
            return Err(Error::Capacity);
        }
        let Some(next_operation) = self.next_operation.checked_add(1) else {
            self.status = Status::ReadOnly;
            return Err(Error::Exhausted);
        };
        let Some(publication) = self.published.publication.checked_add(1) else {
            self.status = Status::ReadOnly;
            return Err(Error::Exhausted);
        };
        let intent = SaveIntent {
            instance: self.instance,
            source: self.source,
            operation: self.next_operation,
            expected: self.published.clone(),
            next: Document {
                account: self.source.account,
                grid: self.source.grid.bytes(),
                publication,
                folders: Arc::new(folders),
            },
        };
        self.next_operation = next_operation;
        self.pending = Some(intent.clone());
        self.status = Status::Saving;
        Ok(intent)
    }
    /// Only the IO owner's exact publication outcome may change the model.
    /// A failed/unknown old result cannot clear another operation's pending state.
    pub(crate) fn complete_save(&mut self, intent: &SaveIntent, published: bool) -> bool {
        if self.pending.as_ref() != Some(intent) {
            return false;
        }
        let pending = self.pending.take().expect("exact pending intent");
        if published {
            self.published = pending.next;
            self.status = Status::Ready;
        } else {
            self.status = Status::SaveFailed;
        }
        true
    }
    pub(crate) fn retire(&mut self) {
        self.pending = None;
        self.published = Document::empty(self.source);
        self.status = Status::Retired;
    }
}

#[cfg(test)]
pub(crate) mod tests;
