//! Explicit read-only local provenance, type-distinct from native Source.
//! Nil/None login and existing rights remain untouched. No digest/Document/IO.
use super::World;
use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};
use uuid::Uuid;

static NEXT_SCOPE: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Identity {
    inventory: u64,
    agent: Uuid,
    owner: Uuid,
    root: Uuid,
    library: Uuid,
    root_info: Option<(Uuid, Uuid, i32, bool)>,
    login: Option<(Uuid, Uuid, Uuid)>,
    profile: Option<&'static Path>,
}
impl Identity {
    pub(crate) fn observe(world: &World) -> Option<Self> {
        Some(Self {
            inventory: world.inventory_archive_instance?,
            agent: world.agent_id,
            owner: world.inventory.owner,
            root: world.inventory.root,
            library: world.inventory.lib_root,
            root_info: world
                .inventory
                .folders
                .get(&world.inventory.root)
                .map(|folder| (folder.info.id, folder.info.parent, folder.info.type_default, folder.library)),
            login: world
                .login
                .as_ref()
                .map(|login| (login.agent_id, login.session_id, login.inventory_root)),
            profile: crate::session_host::profile::root(),
        })
    }
    pub(crate) fn matches(self, world: &World) -> bool {
        Self::observe(world) == Some(self)
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Key {
    instance: u64,
    identity: Identity,
}
pub(super) struct Scope {
    key: Key,
    retired: bool,
}
impl Scope {
    fn capture(world: &World) -> Option<Self> {
        Some(Self {
            key: Key {
                instance: NEXT_SCOPE
                    .try_update(Ordering::AcqRel, Ordering::Acquire, |next| next.checked_add(1))
                    .ok()?,
                identity: Identity::observe(world)?,
            },
            retired: false,
        })
    }
    pub(super) fn demo(world: &World, witness: crate::app::inventory_archive::DemoWitness) -> Option<Self> {
        witness.matches(world).then(|| Self::capture(world)).flatten()
    }
    #[cfg(test)]
    pub(super) fn fixture(world: &World) -> Option<Self> {
        Self::capture(world)
    }
    pub(super) fn matches(&self, world: &World) -> bool {
        !self.retired && self.key.identity.matches(world)
    }
    pub(super) fn available(&self, world: &World) -> bool {
        self.matches(world) && !world.rlva.has("showinv")
    }
    pub(super) fn key(&self) -> Key {
        self.key
    }
    pub(super) fn retire(&mut self) {
        self.retired = true;
    }
}
