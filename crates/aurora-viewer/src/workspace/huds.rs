//! Source-qualified ordinary HUD references, never an attach capability.
//! Caller still needs the current native Workspace gate before capture/action.
use super::{
    controller::Source,
    model::{HudRef, HudSet, MAX_HUDS},
};
use crate::{
    ui::inventory::archive::State,
    world::{
        World,
        inventory::actions,
        inventory_archive::visibility::{VisibilityRead, VisibilityStatus},
    },
};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct HudRow {
    pub reference: HudRef,
    pub name: String,
    pub worn: bool,
}
fn source_matches(world: &World, source: &Source) -> bool {
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
            .and_then(|archive| archive.native_grid(world))
            .is_some_and(|grid| grid.namespace() == source.grid)
        && !world.rlva.has("showinv")
        && world
            .objects
            .index_of_uuid(&source.agent)
            .and_then(|i| world.objects.get(i))
            .is_some_and(|o| o.is_avatar())
}
fn row(world: &World, read: &VisibilityRead<'_>, reference: HudRef, worn: &HashMap<Uuid, u8>, require_worn: bool) -> Option<HudRow> {
    if !reference.valid()
        || !world.avatar_lib.attach_points.get(&reference.point).is_some_and(|point| point.hud)
        || !read.passes(reference.item)
        || actions::library(&world.inventory, reference.item)
        || actions::in_type(&world.inventory, reference.item, 14)
        || actions::original(&world.inventory, reference.item) != Some(reference.item)
    {
        return None;
    }
    let item = world.inventory.items.get(&reference.item)?;
    if item.id != reference.item || item.owner != world.agent_id || item.asset_type != 6 {
        return None;
    }
    let current = worn.get(&reference.item).copied();
    if current.is_some_and(|point| point != reference.point) || (require_worn && current.is_none()) {
        return None;
    }
    if current.is_some()
        && world.rlva.enabled()
        && world.rlva.preferences.hide_locked_attachments
        && !world.rlva_query_removable(reference.item, Uuid::nil(), worn)
    {
        return None;
    }
    if current.is_none() && !world.rlva_can_add(reference.item, reference.point, Uuid::nil()) {
        return None;
    }
    Some(HudRow {
        reference,
        name: item.name.chars().take(128).collect(),
        worn: current.is_some(),
    })
}
/// Current worn items only. Unknown/partial metadata is distinct from a valid
/// empty set; no caller may turn None into an empty successful capture.
pub(crate) fn worn(world: &World, source: &Source, policy: &mut State) -> Option<Vec<HudRow>> {
    if !source_matches(world, source) {
        return None;
    }
    let read = policy.read(world)?;
    if read.status() != VisibilityStatus::Ready || read.revision().is_none() {
        return None;
    }
    let current = world.worn_attachment_points();
    if current.len() > MAX_HUDS {
        return None;
    }
    let mut rows: Vec<_> = current
        .iter()
        .filter_map(|(item, point)| {
            row(
                world,
                &read,
                HudRef {
                    item: *item,
                    point: *point,
                },
                &current,
                true,
            )
        })
        .collect();
    rows.sort_by_key(|row| (row.reference.point, row.reference.item));
    Some(rows)
}
/// Resolves a captured set in its original order. The returned subset has fresh
/// rights and names; unresolved entries are omitted, never fetched or moved.
/// This alone neither approves nor dispatches an appearance transaction.
pub(crate) fn resolve(world: &World, source: &Source, policy: &mut State, references: &HudSet) -> Option<Vec<HudRow>> {
    if !source_matches(world, source) {
        return None;
    }
    let read = policy.read(world)?;
    if read.status() != VisibilityStatus::Ready || read.revision().is_none() {
        return None;
    }
    let current = world.worn_attachment_points();
    if current.len() > MAX_HUDS {
        return None;
    }
    Some(
        references
            .refs()
            .iter()
            .filter_map(|reference| row(world, &read, *reference, &current, false))
            .collect(),
    )
}

#[derive(Clone, PartialEq, Eq)]
struct Scope {
    source: Source,
    binding: u64,
    inventory: u64,
    archive: crate::world::inventory_archive::Revision,
    rules: u64,
    enabled: bool,
    hide_locked: bool,
}
#[derive(Clone, Default)]
pub(crate) struct View {
    pub available: bool,
    pub worn: Vec<HudRow>,
    pub review: Option<(Uuid, Vec<HudRow>)>,
    pub progress: super::hud_lane::Progress,
}
#[derive(Default)]
pub(crate) struct Runtime {
    pub policy: State,
    lane: super::hud_lane::Lane<Scope>,
    pub view: View,
}
impl Runtime {
    /// Archive publication and rule changes revoke consent; ordinary inventory
    /// generation changes are requalified per item (native outfit writes change
    /// that generation themselves). Never borrow another consumer's exemption.
    pub(crate) fn sync(&mut self, world: &World, source: Option<&Source>, binding: u64, authorized: bool) {
        let scope = source.filter(|s| authorized && source_matches(world, s)).and_then(|source| {
            Some(Scope {
                source: source.clone(),
                binding,
                inventory: world.inventory_archive_instance?,
                archive: world.inventory_archive.as_ref()?.metadata_revision(world)?,
                rules: world.rlva.revision,
                enabled: world.rlva.enabled(),
                hide_locked: world.rlva.preferences.hide_locked_attachments,
            })
        });
        self.lane.bind(scope);
        self.view = View::default();
        if let Some(source) = source.filter(|_| authorized) {
            if let Some(rows) = worn(world, source, &mut self.policy) {
                self.view.available = true;
                self.view.worn = rows;
            }
            if let Some(review) = &self.lane.review {
                if let Some(rows) =
                    resolve(world, source, &mut self.policy, &review.wanted).filter(|rows| rows.len() == review.wanted.refs().len())
                {
                    self.view.review = Some((review.nonce, rows));
                } else {
                    // Retire rather than leave an invisible approval that might
                    // reappear after inventory/privacy qualification changes.
                    self.lane.refuse();
                }
            }
            self.view.progress = self.lane.progress;
        }
    }
    pub(crate) fn retire(&mut self) {
        self.lane.retire();
        self.view = View::default();
    }
    pub(crate) fn offer(&mut self, world: &World, source: &Source, wanted: &HudSet) {
        match resolve(world, source, &mut self.policy, wanted).filter(|rows| rows.len() == wanted.refs().len()) {
            Some(rows) => {
                let missing = HudSet::try_from(rows.into_iter().filter(|r| !r.worn).map(|r| r.reference).collect::<Vec<_>>())
                    .expect("subset of validated HUD references");
                self.lane.offer(missing);
            }
            None => self.lane.refuse(),
        }
    }
    pub(crate) fn decide(&mut self, world: &World, source: &Source, nonce: Uuid, accepted: bool) {
        if accepted {
            let Some(review) = &self.lane.review else { return };
            if review.nonce != nonce {
                return;
            }
            if resolve(world, source, &mut self.policy, &review.wanted).is_none_or(|rows| rows.len() != review.wanted.refs().len()) {
                self.lane.refuse();
                return;
            }
        }
        self.lane.decide(nonce, accepted, std::time::Instant::now());
    }
    pub(crate) fn next(&mut self, world: &World, native_busy: bool) -> Option<Vec<HudRef>> {
        let current = world.worn_attachment_points();
        self.lane
            .next(std::time::Instant::now(), native_busy, |r| current.get(&r.item) == Some(&r.point))
    }
    pub(crate) fn busy(&self) -> bool {
        self.lane.busy()
    }
    pub(crate) fn dispatch_failed(&mut self) {
        self.lane.dispatch_failed();
    }
    pub(crate) fn refuse(&mut self) {
        self.lane.refuse();
    }
}

#[cfg(test)]
mod tests;
