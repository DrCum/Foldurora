//! Ordinary native texture qualification. Saved UUIDs are metadata, never a
//! permission to display/fetch, and never a picker or archive exemption.
use super::{controller::Source, model::TextureRef};
use crate::{
    ui::inventory::{archive::State, workspace_state::fetched_path},
    world::{
        World,
        inventory::actions,
        inventory_archive::visibility::{VisibilityBinding, VisibilityRead, VisibilityStatus},
    },
};
use std::collections::HashSet;
use uuid::Uuid;

fn source_current(world: &World, source: &Source) -> bool {
    !source.agent.is_nil()
        && source.agent == world.agent_id
        && world.inventory.owner == source.agent
        && world
            .login
            .as_ref()
            .is_some_and(|login| !login.session_id.is_nil() && login.session_id == source.login)
        && world
            .inventory_archive
            .as_ref()
            .and_then(|a| a.native_grid(world))
            .is_some_and(|g| g.namespace() == source.grid)
        && !world.rlva.has("showinv")
        && !world.rlva.has("viewtexture")
}
fn permitted(world: &World, read: &VisibilityRead<'_>, source: &Source, reference: &TextureRef) -> bool {
    let Some(item) = world.inventory.items.get(&reference.item) else {
        return false;
    };
    reference.grid == source.grid
        && !reference.asset.is_nil()
        && !reference.item.is_nil()
        && item.id == reference.item
        && item.asset_id == reference.asset
        && item.owner == source.agent
        && matches!(item.asset_type, 0 | 15)
        && matches!(item.inv_type, 0 | 15)
        && item.owner_mask & actions::FULL == actions::FULL
        && read.passes(item.id)
        && fetched_path(&world.inventory, item.parent, read.mask())
}
fn ordinary(read: &VisibilityRead<'_>) -> bool {
    read.status() == VisibilityStatus::Ready && read.revision().is_some() && read.binding().is_some()
}
#[derive(Clone, PartialEq, Eq)]
struct Scope {
    source: Source,
    binding: u64,
    visibility: VisibilityBinding,
}
#[derive(Default)]
pub(crate) struct Policy {
    state: State,
    scope: Option<Scope>,
    approved: HashSet<TextureRef>,
    requested: Vec<TextureRef>,
}
impl Policy {
    pub(crate) fn refresh(&mut self, world: &World, source: Option<&Source>, binding: u64, references: &[TextureRef]) {
        let Some(source) = source.filter(|s| binding > 0 && source_current(world, s)) else {
            self.scope = None;
            self.approved.clear();
            self.requested.clear();
            return;
        };
        if references.len() > super::model::MAX_DISPLAYS + 2 {
            self.scope = None;
            self.approved.clear();
            self.requested.clear();
            return;
        }
        let Some(read) = self.state.read(world).filter(ordinary) else {
            self.scope = None;
            self.approved.clear();
            self.requested.clear();
            return;
        };
        let scope = Scope {
            source: source.clone(),
            binding,
            visibility: read.binding().expect("ordinary checked"),
        };
        if self.scope.as_ref() == Some(&scope) && self.requested == references {
            return;
        }
        self.scope = Some(scope);
        self.requested = references.to_vec();
        self.approved.clear();
        self.approved
            .extend(references.iter().filter(|r| permitted(world, &read, source, r)).cloned());
    }
    /// Reacquire the actual owner proof immediately before painting. A copied
    /// approved set cannot survive inventory/privacy/publication/source changes.
    pub(crate) fn current(&mut self, world: &World, source: Option<&Source>, binding: u64) -> bool {
        let Some(source) = source.filter(|s| source_current(world, s)) else {
            return false;
        };
        let Some(read) = self.state.read(world).filter(ordinary) else {
            return false;
        };
        self.scope
            .as_ref()
            .is_some_and(|s| s.source == *source && s.binding == binding && Some(s.visibility) == read.binding())
    }
    pub(crate) fn allows(&self, reference: &TextureRef) -> bool {
        self.approved.contains(reference)
    }
    pub(crate) fn accepts(&mut self, world: &World, source: &Source, reference: &TextureRef) -> bool {
        source_current(world, source)
            && self
                .state
                .read(world)
                .filter(ordinary)
                .is_some_and(|r| permitted(world, &r, source, reference))
    }
    pub(crate) fn visibility(&mut self, world: &World, source: &Source) -> Option<VisibilityBinding> {
        if !source_current(world, source) {
            return None;
        }
        self.state.read(world).filter(ordinary)?.binding()
    }
    pub(crate) fn choose(&mut self, world: &World, source: &Source, asset: Uuid) -> Option<TextureRef> {
        if asset.is_nil() || !source_current(world, source) {
            return None;
        }
        let read = self.state.read(world).filter(ordinary)?;
        world
            .inventory
            .items
            .values()
            .filter(|item| item.asset_id == asset)
            .filter_map(|item| TextureRef::new(item.id, asset, source.grid.clone()).ok())
            .filter(|r| permitted(world, &read, source, r))
            .min_by_key(|r| r.item)
    }
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests;
