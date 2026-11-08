//! One source-qualified ordinary browse mask and bounded existing fetch demand.
//! Exemptions are explicit; unavailable ordinary metadata never means exempt.
mod input;
mod structure;
pub(crate) use input::{InputRead, NativeInput};

use super::*;
use crate::world::inventory_archive::{
    Status,
    visibility::{MaskRead, OwnerPolicy, VisibilityBinding, VisibilityRead, VisibilityStatus},
};
use std::{
    ops::Deref,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Exemption {
    Worn,
    Favorites,
    Partial,
    Picker,
    Marketplace,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OwnerRevision {
    owner: Option<u64>,
    revision: Option<u64>,
    inventory: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Ordinary(OwnerRevision),
    Exempt(Exemption),
    ExemptView(Exemption, OwnerRevision),
}
#[derive(Clone, Copy)]
pub(crate) enum Gate<'a> {
    Ordinary {
        mask: Option<MaskRead<'a>>,
        revision: OwnerRevision,
    },
    Exempt(Exemption),
    ExemptView {
        reason: Exemption,
        revision: OwnerRevision,
    },
}
impl<'a> Gate<'a> {
    pub(super) fn for_inventory(self, inventory: &Inventory) -> bool {
        match self {
            Self::Ordinary { mask, .. } => mask.is_some_and(|mask| mask.for_inventory(inventory)),
            _ => true,
        }
    }
    pub(crate) fn passes(self, id: Uuid) -> bool {
        match self {
            Self::Ordinary { mask, revision } => {
                revision.owner.is_some() && revision.inventory.is_some() && mask.is_some_and(|mask| mask.passes(id))
            }
            Self::Exempt(_) | Self::ExemptView { .. } => true,
        }
    }
    pub(crate) fn key(self) -> Key {
        match self {
            Self::Ordinary { revision, .. } => Key::Ordinary(revision),
            Self::Exempt(reason) => Key::Exempt(reason),
            Self::ExemptView { reason, revision } => Key::ExemptView(reason, revision),
        }
    }
    pub(crate) fn mask(self) -> Option<MaskRead<'a>> {
        match self {
            Self::Ordinary { mask, .. } => mask,
            Self::Exempt(_) | Self::ExemptView { .. } => None,
        }
    }
}
pub(crate) struct State {
    instance: Option<u64>,
    policy: OwnerPolicy,
}
impl Default for State {
    fn default() -> Self {
        Self {
            instance: NEXT_OWNER
                .try_update(Ordering::AcqRel, Ordering::Acquire, |next| next.checked_add(1))
                .ok(),
            policy: OwnerPolicy::default(),
        }
    }
}
impl State {
    pub(crate) fn instance(&self) -> Option<u64> {
        self.instance
    }
    pub(crate) fn read<'a>(&'a mut self, world: &'a World) -> Option<VisibilityRead<'a>> {
        if self.instance.is_none() || world.inventory_archive_instance.is_none() {
            self.policy.unavailable();
            return None;
        }
        match world.inventory_archive.as_ref() {
            Some(archive) => archive.read(world, &mut self.policy),
            None => {
                self.policy.unavailable();
                None
            }
        }
    }
    pub(crate) fn ordinary<'a>(instance: Option<u64>, read: Option<&'a VisibilityRead<'a>>, inventory: Option<u64>) -> Gate<'a> {
        let mask = read.map(VisibilityRead::mask);
        let revision = mask.and_then(MaskRead::revision);
        Gate::Ordinary {
            mask,
            revision: OwnerRevision {
                owner: instance,
                revision,
                inventory,
            },
        }
    }
    pub(crate) fn assign_include(&mut self, world: &World, desired: bool) -> bool {
        world
            .inventory_archive
            .as_ref()
            .is_some_and(|archive| archive.set_include(world, &mut self.policy, desired))
    }
    fn controls(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut InventoryUi) {
        let (visibility, binding) = self
            .read(world)
            .map(|read| (read.status(), read.binding()))
            .unwrap_or((VisibilityStatus::Unavailable, None));
        st.archive_controls = Some(Controls {
            visibility,
            binding,
            included: self.policy.included(),
        });
        let status = world.inventory_archive.as_ref().and_then(|archive| archive.status());
        let text = match (status, visibility) {
            (_, VisibilityStatus::Unavailable) => Some(aurora_i18n::tr!("inventory-archive-unavailable")),
            (_, VisibilityStatus::OfflineReadOnly) => Some(aurora_i18n::tr!("inventory-archive-offline-readonly")),
            (Some(Status::Loading), _) => Some(aurora_i18n::tr!("inventory-archive-loading")),
            (Some(Status::ReadOnly), _) => Some(aurora_i18n::tr!("inventory-archive-readonly")),
            (Some(Status::Saving), _) => Some(aurora_i18n::tr!("inventory-archive-saving")),
            (Some(Status::SaveFailed), _) => Some(aurora_i18n::tr!("inventory-archive-save-failed")),
            (Some(Status::LoadFailed), _) => Some(aurora_i18n::tr!("inventory-archive-load-failed")),
            (Some(Status::Retired), _) => Some(aurora_i18n::tr!("inventory-archive-unavailable")),
            (_, VisibilityStatus::Loading) => Some(aurora_i18n::tr!("inventory-archive-loading")),
            (_, VisibilityStatus::Limited) => Some(aurora_i18n::tr!("inventory-archive-limited")),
            _ => None,
        };
        if let Some(text) = text {
            ui.label(RichText::new(text).small().color(p.muted));
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Controls {
    visibility: VisibilityStatus,
    binding: Option<VisibilityBinding>,
    included: bool,
}
pub(super) fn visibility_menu(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi) {
    let Some(controls) = st.archive_controls else { return };
    let Some(revision) = st.archive_key else { return };
    super::super::menu::submenu(
        ui,
        p,
        "archive-filled",
        aurora_i18n::tr!("inventory-archive-visibility"),
        true,
        |ui| {
            let enabled = !matches!(
                controls.visibility,
                VisibilityStatus::Unavailable | VisibilityStatus::OfflineReadOnly
            );
            if let Some(proposal) = st
                .archive_include_context
                .paint(ui, p, st.ui_id, revision, controls, enabled, st.archive_input)
            {
                st.archive_include_proposal = Some(proposal);
            }
        },
    );
}

#[derive(Clone, Copy)]
pub(super) struct Summary {
    key: Key,
    generation: u64,
    pub(super) count: usize,
    pub(super) fetching: bool,
}
fn summarize(st: &mut InventoryUi, inventory: &Inventory, gate: Gate<'_>) {
    use rayon::prelude::*;
    let count = st
        .archive_summary
        .filter(|summary| summary.key == gate.key() && summary.generation == inventory.generation)
        .map_or_else(
            || inventory.items.par_iter().filter(|(id, _)| gate.passes(**id)).count(),
            |summary| summary.count,
        );
    let fetching = inventory.queue.iter().any(|(id, _)| gate.passes(*id))
        || inventory
            .folders
            .par_iter()
            .any(|(id, folder)| folder.state == FetchState::Fetching && gate.passes(*id));
    st.archive_summary = Some(Summary {
        key: gate.key(),
        generation: inventory.generation,
        count,
        fetching,
    });
}

pub(super) fn preflight(ctx: &egui::Context, world: &World, st: &mut InventoryUi, root: Uuid) {
    let mut state = st.archive.take().unwrap_or_default();
    let instance = state.instance;
    let read = state.read(world);
    let revision = read.as_ref().and_then(VisibilityRead::revision);
    let owner = OwnerRevision {
        owner: instance,
        revision,
        inventory: world.inventory_archive_instance,
    };
    let gate = if !root.is_nil() {
        Gate::ExemptView {
            reason: Exemption::Partial,
            revision: owner,
        }
    } else if st.ui_id == super::listings::PANEL {
        Gate::ExemptView {
            reason: Exemption::Marketplace,
            revision: owner,
        }
    } else if st.tab == 3 {
        Gate::ExemptView {
            reason: Exemption::Worn,
            revision: owner,
        }
    } else if st.tab == 4 {
        Gate::ExemptView {
            reason: Exemption::Favorites,
            revision: owner,
        }
    } else {
        State::ordinary(instance, read.as_ref(), world.inventory_archive_instance)
    };
    prune(ctx, st, gate);
    summarize(st, &world.inventory, gate);
    st.archive = Some(state);
}

#[derive(Clone, Copy)]
pub(super) struct SourceOrder {
    generation: u64,
    root: Uuid,
    agent: Uuid,
    inventory: Option<u64>,
    login: Option<(Uuid, Uuid, Uuid)>,
    profile: Option<&'static std::path::Path>,
    binding: Option<VisibilityBinding>,
    exempt: bool,
}
pub(super) struct Demands {
    left: usize,
    seen: HashSet<Uuid>,
    pending: Vec<Uuid>,
    ordinary: bool,
}
impl Demands {
    #[cfg(test)]
    pub(in crate::ui::inventory) fn fixture_requests(&mut self) -> impl Iterator<Item = Uuid> + '_ {
        self.pending.drain(..)
    }
    pub(super) fn new(inv: &Inventory) -> Self {
        Self {
            left: 16usize.saturating_sub(inv.queue.len()),
            seen: HashSet::new(),
            pending: Vec::new(),
            ordinary: false,
        }
    }
    fn request(&mut self, id: Uuid, ordinary: bool) {
        self.ordinary |= ordinary;
        if self.left != 0 && self.seen.insert(id) {
            self.left -= 1;
            self.pending.push(id);
        }
    }
    fn apply(&mut self, world: &mut World, owner: &mut State, source: SourceOrder) {
        let current = source.generation == world.inventory.generation
            && source.root == world.inventory.root
            && source.agent == world.agent_id
            && source.inventory == world.inventory_archive_instance
            && source.profile == crate::session_host::profile::root()
            && source.login
                == world
                    .login
                    .as_ref()
                    .map(|login| (login.agent_id, login.session_id, login.inventory_root));
        let qualified = source.exempt || source.binding.is_some() && owner.read(world).and_then(|read| read.binding()) == source.binding;
        if current && qualified {
            for id in self.pending.drain(..) {
                world.inventory.request(id);
            }
        } else {
            self.pending.clear();
        }
        self.ordinary = false;
    }
}
pub(super) struct Browse<'a, 'd> {
    inventory: &'a Inventory,
    pub(super) gate: Gate<'a>,
    ordinary: Gate<'a>,
    demands: &'d mut Demands,
}
impl Deref for Browse<'_, '_> {
    type Target = Inventory;
    fn deref(&self) -> &Inventory {
        self.inventory
    }
}
impl Browse<'_, '_> {
    /// The original qualified ordinary gate, even when the active tab uses
    /// an explicit exemption. This copy still borrows the same actual read.
    pub(super) fn ordinary(&self) -> Gate<'_> {
        self.ordinary
    }
    pub(super) fn request(&mut self, id: Uuid) {
        if self.gate.passes(id) {
            self.demands.request(id, matches!(self.gate, Gate::Ordinary { .. }));
        }
    }
    pub(super) fn request_tree(&mut self, root: Uuid) -> bool {
        let mut todo = vec![root];
        let mut seen = HashSet::new();
        let mut complete = true;
        while let Some(id) = todo.pop() {
            if !self.gate.passes(id) || !seen.insert(id) {
                continue;
            }
            let Some(folder) = self.inventory.folders.get(&id) else { continue };
            if matches!(folder.info.type_default, 14 | 50) {
                continue;
            }
            complete &= folder.state == FetchState::Fetched;
            if folder.state == FetchState::Unknown {
                self.demands.request(id, matches!(self.gate, Gate::Ordinary { .. }));
            }
            todo.extend(folder.children.iter().copied());
        }
        complete
    }
}
#[cfg(test)]
impl<'a, 'd> Browse<'a, 'd> {
    pub(in crate::ui::inventory) fn fixture(inventory: &'a Inventory, gate: Gate<'a>, demands: &'d mut Demands) -> Self {
        Self {
            inventory,
            gate,
            ordinary: gate,
            demands,
        }
    }
}

pub(super) fn tab(ctx: &egui::Context, st: &mut InventoryUi, browse: &mut Browse<'_, '_>) {
    browse.gate = match browse.ordinary {
        Gate::Ordinary { revision, .. } => match st.tab {
            3 => Gate::ExemptView {
                reason: Exemption::Worn,
                revision,
            },
            4 => Gate::ExemptView {
                reason: Exemption::Favorites,
                revision,
            },
            _ => browse.ordinary,
        },
        _ => browse.ordinary,
    };
    prune(ctx, st, browse.gate);
    st.archive_drawing = matches!(browse.gate, Gate::Ordinary { .. });
}

fn prune(ctx: &egui::Context, st: &mut InventoryUi, gate: Gate<'_>) {
    let changed = st.archive_key != Some(gate.key());
    if changed {
        st.archive_change_frame = Some(ctx.cumulative_frame_nr());
        st.archive_key = Some(gate.key());
        st.view = Default::default();
        st.plain = Default::default();
        st.flat = Default::default();
        st.result_tree = Default::default();
        st.side = Default::default();
        st.counts = Default::default();
        st.fetch_generation = None;
        st.fetch_poll = None;
        st.visible.clear();
        st.hover = Default::default();
        st.wanted_hover_images.clear();
        st.wanted_images.clear();
        st.row_menu = None;
        st.archive_context.close(ctx);
        st.archive_include_context = Default::default();
        st.archive_include_proposal = None;
    }
    st.selection.retain(|id| gate.passes(*id));
    st.selected = st.selected.filter(|id| gate.passes(*id));
    st.anchor = st.anchor.filter(|id| gate.passes(*id));
    if st.rename.as_ref().is_some_and(|edit| !gate.passes(edit.id)) {
        st.rename = None;
    }
    if st.reveal.is_some_and(|id| !gate.passes(id)) {
        st.message = aurora_i18n::tr!("inventory-archive-show-original").into();
    }
    st.reveal = st.reveal.filter(|id| gate.passes(*id));
    st.reveal_path.retain(|id| gate.passes(*id));
    st.navigate_to = st.navigate_to.filter(|id| gate.passes(*id));
    if !st.browse.is_nil() && !gate.passes(st.browse) {
        st.browse = Uuid::nil();
    }
    super::drag::prune_archive(ctx, st.ui_id, gate);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn owner(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    world: &World,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<InvAction>,
    classic: bool,
    root: Uuid,
    demands: &mut Demands,
) -> (State, SourceOrder) {
    // None exists only inside this concrete synchronous owner draw. No event,
    // source lookup/chooser callback or rebind resolves it during this scope.
    let mut owner = st.archive.take().unwrap_or_default();
    let supported = root.is_nil() && st.ui_id != super::listings::PANEL;
    if supported {
        owner.controls(ui, p, world, st);
    }
    if !supported {
        st.archive_controls = None;
        st.archive_include_proposal = None;
    }
    let instance = owner.instance;
    let read = owner.read(world);
    let revision = read.as_ref().and_then(VisibilityRead::revision);
    let ordinary = State::ordinary(instance, read.as_ref(), world.inventory_archive_instance);
    let owner_revision = OwnerRevision {
        owner: instance,
        revision,
        inventory: world.inventory_archive_instance,
    };
    let gate = if !supported {
        Gate::ExemptView {
            reason: if st.ui_id == super::listings::PANEL {
                Exemption::Marketplace
            } else {
                Exemption::Partial
            },
            revision: owner_revision,
        }
    } else if st.tab == 3 {
        Gate::ExemptView {
            reason: Exemption::Worn,
            revision: owner_revision,
        }
    } else if st.tab == 4 {
        Gate::ExemptView {
            reason: Exemption::Favorites,
            revision: owner_revision,
        }
    } else {
        ordinary
    };
    prune(ui.ctx(), st, gate);
    st.archive_drawing = matches!(gate, Gate::Ordinary { .. });
    {
        let mut browse = Browse {
            inventory: &world.inventory,
            gate,
            ordinary: if supported { ordinary } else { gate },
            demands,
        };
        if classic {
            if supported {
                super::inventory_contents_visible(ui, p, icons, &mut browse, st, prefs, facts, actions);
            } else {
                super::folder_window_contents_visible(ui, p, icons, &mut browse, root, st, prefs, facts, actions);
            }
        } else {
            super::aurora::contents_visible(ui, p, icons, &mut browse, st, prefs, facts, images, actions);
        }
    }
    if supported {
        st.archive_context.capture(read.as_ref());
    }
    st.archive_context.finish_owner(ui.ctx());
    st.archive_include_context.finish_owner(ui.ctx());
    if let Some(proposal) = st.archive_include_proposal.take() {
        actions.push(InvAction::Archive(Box::new(proposal)));
    }
    st.archive_drawing = false;
    let final_gate = match ordinary {
        Gate::Ordinary { revision, .. } if supported && matches!(st.tab, 3 | 4) => Gate::ExemptView {
            reason: if st.tab == 3 { Exemption::Worn } else { Exemption::Favorites },
            revision,
        },
        _ if supported => ordinary,
        _ => gate,
    };
    summarize(st, &world.inventory, final_gate);
    let source = SourceOrder {
        generation: world.inventory.generation,
        root: world.inventory.root,
        agent: world.agent_id,
        inventory: world.inventory_archive_instance,
        profile: crate::session_host::profile::root(),
        login: world
            .login
            .as_ref()
            .map(|login| (login.agent_id, login.session_id, login.inventory_root)),
        binding: read.as_ref().and_then(VisibilityRead::binding),
        exempt: !demands.ordinary,
    };
    (owner, source)
}

pub(super) fn restore(world: &mut World, st: &mut InventoryUi, owner: State, source: SourceOrder, demands: &mut Demands) {
    st.archive = Some(owner);
    demands.apply(world, st.archive.as_mut().expect("restored actual owner"), source);
}

// Metadata proposals carry the original context and painted control. They are
// consumed only by App after the whole egui batch has completed.
static NEXT_CONTEXT: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, PartialEq)]
struct Control {
    widget: egui::Id,
    rect: egui::Rect,
    layer: egui::LayerId,
    frame: u64,
    pass: u64,
    registered: egui::WidgetRect,
    paint_clip: egui::Rect,
    transform: Option<egui::emath::TSTransform>,
    input_epoch: u64,
    structure_origin: structure::Origin,
    structure: Option<structure::Witness>,
}
fn finite_positive(rect: egui::Rect) -> bool {
    rect.is_finite() && rect.size().is_finite() && rect.is_positive()
}
fn finite_transform(transform: Option<egui::emath::TSTransform>) -> bool {
    let value = transform.unwrap_or(egui::emath::TSTransform::IDENTITY);
    let finite = |value: egui::emath::TSTransform| {
        value.scaling.is_finite() && value.scaling > 0.0 && value.translation.x.is_finite() && value.translation.y.is_finite()
    };
    finite(value) && finite(value.inverse())
}
fn transformed_rect(rect: egui::Rect, transform: Option<egui::emath::TSTransform>) -> Option<egui::Rect> {
    if !finite_positive(rect) || !finite_transform(transform) {
        return None;
    }
    let global = transform.unwrap_or(egui::emath::TSTransform::IDENTITY) * rect;
    finite_positive(global).then_some(global)
}
impl Control {
    fn capture(ui: &egui::Ui, response: &egui::Response, input: InputRead) -> Option<Self> {
        let registered = ui.ctx().viewport(|viewport| viewport.this_pass.widgets.get(response.id).copied())?;
        if !response.enabled()
            || response.rect != registered.rect
            || response.interact_rect != registered.interact_rect
            || response.sense != registered.sense
        {
            return None;
        }
        let transform = ui.ctx().layer_transform_to_global(response.layer_id);
        let paint_clip = ui.clip_rect();
        let viewport = ui.ctx().content_rect();
        if !finite_positive(viewport) {
            return None;
        }
        let rect = transformed_rect(registered.rect, transform)?
            .intersect(transformed_rect(registered.interact_rect, transform)?)
            .intersect(transformed_rect(paint_clip, transform)?)
            .intersect(viewport);
        let control = Self {
            widget: response.id,
            rect,
            layer: response.layer_id,
            frame: ui.ctx().cumulative_frame_nr(),
            pass: ui.ctx().cumulative_pass_nr(),
            registered,
            paint_clip,
            transform,
            input_epoch: input.epoch(),
            structure_origin: structure::Origin::capture(ui, registered)?,
            structure: None,
        };
        control.registry(ui.ctx(), false).then_some(control)
    }
    fn visible(self) -> bool {
        finite_positive(self.rect)
    }
    fn same_widget(self, other: Self) -> bool {
        finite_transform(self.transform)
            && finite_transform(other.transform)
            && self.widget == other.widget
            && self.rect == other.rect
            && self.layer == other.layer
            && self.registered == other.registered
            && self.paint_clip == other.paint_clip
            && self.transform == other.transform
            && self.input_epoch == other.input_epoch
            && self.structure_origin == other.structure_origin
    }
    fn geometry(self, ctx: &egui::Context) -> bool {
        let viewport = ctx.content_rect();
        let current = ctx.layer_transform_to_global(self.layer);
        // Pinned TSTransform PartialEq debug-panics for NaN vectors. Validate
        // BOTH operands, including both translations/inverse, before equality.
        if !finite_transform(self.transform) || !finite_transform(current) {
            return false;
        }
        let Some(rect) = transformed_rect(self.registered.rect, self.transform) else {
            return false;
        };
        let Some(interact) = transformed_rect(self.registered.interact_rect, self.transform) else {
            return false;
        };
        let Some(clip) = transformed_rect(self.paint_clip, self.transform) else {
            return false;
        };
        let visible = rect.intersect(interact).intersect(clip).intersect(viewport);
        finite_positive(viewport) && finite_positive(visible) && visible == self.rect && current == self.transform
    }
    fn registry(self, ctx: &egui::Context, committed: bool) -> bool {
        if !self.geometry(ctx) || !ctx.memory(|memory| memory.allows_interaction(self.layer)) {
            return false;
        }
        ctx.viewport(|viewport| {
            let widgets = if committed {
                &viewport.prev_pass.widgets
            } else {
                &viewport.this_pass.widgets
            };
            widgets.get(self.widget).is_some_and(|widget| {
                widget.enabled
                    && widget.visible
                    && widget.sense.senses_click()
                    && *widget == self.registered
                    && widget.id == self.widget
                    && widget.layer_id == self.layer
            })
        })
    }
    /// Resolve egui's actual final registry AFTER the entire run, rather than
    /// treating our saved control as proof that a later widget kept it live.
    fn committed(self, ctx: &egui::Context) -> bool {
        if !self.registry(ctx, true) {
            return false;
        }
        let Some(structure) = self.structure else { return false };
        if ctx.viewport(|viewport| structure::Witness::capture(&viewport.prev_pass.widgets, &self)) != Some(structure) {
            return false;
        }
        let local = self
            .registered
            .rect
            .intersect(self.registered.interact_rect)
            .intersect(self.paint_clip);
        let unoccluded = ctx.viewport(|viewport| {
            let widgets = &viewport.prev_pass.widgets;
            let Some((layer, index)) = widgets.order(self.widget) else {
                return false;
            };
            layer == self.layer
                && !widgets
                    .get_layer(layer)
                    .skip(index + 1)
                    .any(|widget| widget.visible && !structure.contains(widget.id) && widget.interact_rect.intersect(local).is_positive())
        });
        if !unoccluded {
            return false;
        }
        // No Context accessors are nested while memory/viewport locks are held.
        let above = ctx.memory(|memory| {
            memory
                .layer_ids()
                .skip_while(|layer| *layer != self.layer)
                .skip(1)
                .filter(|layer| memory.areas().is_visible(layer))
                .collect::<Vec<_>>()
        });
        for layer in above {
            if let Some(area) = egui::AreaState::load(ctx, layer.id) {
                let Some(rect) = transformed_rect(area.rect(), ctx.layer_transform_to_global(layer)) else {
                    return false;
                };
                if rect.intersect(self.rect).is_positive() {
                    return false;
                }
            }
        }
        ctx.layer_id_at(self.rect.center()) == Some(self.layer)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Release {
    Pointer(egui::Pos2),
    Key(egui::Key),
    Accessibility,
}
#[derive(Clone, Copy)]
struct Gesture {
    control: Control,
    key: Option<egui::Key>,
}
#[derive(Clone)]
pub(crate) struct Proposal {
    pub(crate) owner: Uuid,
    revision: Key,
    nonce: u64,
    intent: Intent,
    control: Control,
    release: Release,
}
#[derive(Clone, PartialEq, Eq)]
enum Intent {
    Edit {
        ticket: crate::world::inventory_archive::EditTicket,
        profile: String,
    },
    Include {
        binding: VisibilityBinding,
        desired: bool,
    },
}
impl Proposal {
    pub(crate) fn current_gate(&self, gate: Gate<'_>) -> bool {
        gate.key() == self.revision
            && match &self.intent {
                Intent::Edit { ticket, .. } => gate.passes(ticket.namespace().2),
                Intent::Include { .. } => false,
            }
    }
    pub(crate) fn include_desired(&self, read: Option<&VisibilityRead<'_>>, gate: Gate<'_>) -> Option<bool> {
        match self.intent {
            Intent::Include { binding, desired }
                if match self.revision {
                    Key::Ordinary(revision) | Key::ExemptView(_, revision) => gate.key() == Key::Ordinary(revision),
                    Key::Exempt(_) => false,
                } && read.and_then(VisibilityRead::binding) == Some(binding) =>
            {
                Some(desired)
            }
            _ => None,
        }
    }
    pub(crate) fn input_current(&self, input: InputRead) -> bool {
        input.plain()
            && self.control.input_epoch == input.epoch()
            && match self.release {
                Release::Key(key) => input.key_allowed(key),
                _ => true,
            }
    }
    pub(crate) fn same(&self, other: &Self) -> bool {
        self.owner == other.owner
            && self.revision == other.revision
            && self.nonce == other.nonce
            && self.intent == other.intent
            && self.control == other.control
            && self.release == other.release
    }
    pub(crate) fn dispatch(self, world: &mut World) -> Result<(), crate::world::inventory_archive::Error> {
        let Intent::Edit { ticket, .. } = self.intent else {
            return Err(crate::world::inventory_archive::Error::Unavailable);
        };
        let Some(mut archive) = world.inventory_archive.take() else {
            return Err(crate::world::inventory_archive::Error::Unavailable);
        };
        let desired = !ticket.previous();
        let result = archive.assign(world, ticket, desired);
        world.inventory_archive = Some(archive);
        result
    }
}
#[derive(Clone, Copy)]
enum ActivationEdge {
    Pointer {
        pos: egui::Pos2,
        pressed: bool,
    },
    Key {
        key: egui::Key,
        pressed: bool,
        repeat: bool,
        plain: bool,
    },
    Cancel,
}
#[derive(Default)]
struct Activation {
    frame: Option<u64>,
    control: Option<Control>,
    previous: Option<Control>,
    gesture: Option<Gesture>,
    released: Option<(Gesture, Release)>,
    settled_focus: Option<Option<egui::Id>>,
    input_frame: Option<u64>,
    blocked: bool,
}
impl Activation {
    fn paint(&mut self, ui: &egui::Ui, response: &egui::Response, input: InputRead) -> Option<(Control, Release)> {
        let frame = ui.ctx().cumulative_frame_nr();
        if self.frame != Some(frame) {
            self.previous = self.control.filter(|control| control.frame.checked_add(1) == Some(frame));
            self.control = None;
            self.released = None;
            self.settled_focus = None;
            self.frame = Some(frame);
            self.blocked = false;
        }
        if self
            .gesture
            .and_then(|gesture| gesture.key)
            .is_some_and(|key| !input.key_allowed(key))
        {
            self.gesture = None;
            self.released = None;
        }
        let Some(control) = Control::capture(ui, response, input) else {
            self.gesture = None;
            self.released = None;
            self.control = None;
            self.blocked = true;
            return None;
        };
        if self.input_frame != Some(frame) {
            self.input_frame = Some(frame);
            let focused = ui.ctx().memory(|memory| memory.focused());
            // Copy only fixed, bounded control metadata; never clone Text/IME
            // strings or an unbounded raw event list from a menu paint.
            let (edges, blocked) = ui.input(|input| {
                let mut edges = [None; 128];
                let mut blocked = !input.focused || !input.modifiers.is_none() || input.events.len() > edges.len();
                for (index, event) in input.events.iter().take(128).enumerate() {
                    edges[index] = match event {
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            ..
                        } => Some(ActivationEdge::Pointer {
                            pos: *pos,
                            pressed: *pressed,
                        }),
                        egui::Event::Key {
                            key: key @ (egui::Key::Enter | egui::Key::Space),
                            pressed,
                            repeat,
                            modifiers,
                            ..
                        } => Some(ActivationEdge::Key {
                            key: *key,
                            pressed: *pressed,
                            repeat: *repeat,
                            plain: modifiers.is_none(),
                        }),
                        egui::Event::Ime(_) | egui::Event::WindowFocused(false) | egui::Event::ModifiersChanged(_) => {
                            blocked = true;
                            Some(ActivationEdge::Cancel)
                        }
                        _ => None,
                    };
                }
                (edges, blocked)
            });
            self.blocked |= blocked || !ui.is_enabled() || !input.plain();
            if self.blocked {
                self.gesture = None;
                self.released = None;
                self.control = Some(control);
                return None;
            }
            for event in edges.into_iter().flatten() {
                match event {
                    ActivationEdge::Pointer { pos, pressed } => {
                        if pressed {
                            self.gesture = self
                                .previous
                                .filter(|previous| {
                                    previous.committed(ui.ctx())
                                        && previous.rect.contains(pos)
                                        && previous.same_widget(control)
                                        && ui.ctx().layer_id_at(pos) == Some(previous.layer)
                                })
                                .map(|control| Gesture { control, key: None });
                        } else if let Some(gesture) = self.gesture.take().filter(|gesture| gesture.key.is_none()) {
                            self.released = Some((gesture, Release::Pointer(pos)));
                        }
                    }
                    ActivationEdge::Key {
                        key,
                        pressed,
                        repeat,
                        plain,
                    } => {
                        if !plain || !input.key_allowed(key) || pressed && repeat {
                            self.gesture = None;
                            self.released = None;
                        } else if pressed {
                            self.gesture = self
                                .previous
                                .filter(|previous| {
                                    input.key_allowed(key)
                                        && previous.committed(ui.ctx())
                                        && previous.same_widget(control)
                                        && focused == Some(previous.widget)
                                })
                                .map(|control| Gesture { control, key: Some(key) });
                        } else if !pressed && let Some(gesture) = self.gesture.take().filter(|gesture| gesture.key == Some(key)) {
                            self.released = Some((gesture, Release::Key(key)));
                        }
                    }
                    ActivationEdge::Cancel => {
                        self.gesture = None;
                        self.released = None;
                    }
                }
            }
        }
        if self.blocked || !ui.is_enabled() || !input.plain() {
            self.gesture = None;
            self.released = None;
            return None;
        }
        if self.released.is_none()
            && self.gesture.is_none()
            && let Some(previous) = self.previous.filter(|previous| {
                previous.committed(ui.ctx())
                    && previous.same_widget(control)
                    && ui.ctx().memory(|memory| memory.focused()) == Some(previous.widget)
            })
            && ui.input(|input| input.has_accesskit_action_request(control.widget, egui::accesskit::Action::Click))
        {
            self.released = Some((
                Gesture {
                    control: previous,
                    key: None,
                },
                Release::Accessibility,
            ));
        }
        self.control = Some(control);
        let (gesture, release) = self.released?;
        let relevant = control.visible()
            && gesture.control.same_widget(control)
            && ui.ctx().memory(|memory| memory.is_above_modal_layer(control.layer))
            && match release {
                Release::Pointer(pos) => {
                    gesture.control.rect.contains(pos) && control.rect.contains(pos) && ui.ctx().layer_id_at(pos) == Some(control.layer)
                }
                Release::Key(_) | Release::Accessibility => ui.ctx().memory(|memory| memory.focused()) == Some(control.widget),
            };
        if !relevant {
            return None;
        }
        Some((gesture.control, release))
    }
    fn settle(&mut self, ctx: &egui::Context) {
        // Pointer activation need not focus the row. Retain the actual focus
        // after its owner finishes, so a later editor cannot take authority
        // between this original release and the whole-run consumer.
        self.settled_focus = self.released.map(|_| ctx.memory(|memory| memory.focused()));
        if let Some(control) = &mut self.control {
            if control.frame == ctx.cumulative_frame_nr() && control.pass == ctx.cumulative_pass_nr() {
                control.structure = ctx.viewport(|viewport| structure::Witness::capture(&viewport.this_pass.widgets, control));
            } else {
                control.structure = None;
            }
        }
    }
    fn validates(&self, ctx: &egui::Context, original: Control, release: Release) -> bool {
        let Some(control) = self.control else { return false };
        self.released
            .is_some_and(|(gesture, actual)| gesture.control == original && actual == release)
            && control.same_widget(original)
            && control.structure.is_some()
            && control.structure == original.structure
            && control.committed(ctx)
            && control.visible()
            && original.visible()
            && control.frame.checked_add(1) == Some(ctx.cumulative_frame_nr())
            && control.pass.checked_add(1) == Some(ctx.cumulative_pass_nr())
            && !ctx.will_discard()
            && ctx.input(|input| input.focused && input.modifiers.is_none())
            && self.settled_focus == Some(ctx.memory(|memory| memory.focused()))
            && !egui::Popup::is_any_open(ctx)
            && ctx.memory(|memory| memory.is_above_modal_layer(control.layer))
            && match release {
                Release::Pointer(pos) => {
                    control.rect.contains(pos) && original.rect.contains(pos) && ctx.layer_id_at(pos) == Some(control.layer)
                }
                Release::Key(_) | Release::Accessibility => ctx.memory(|memory| memory.focused()) == Some(control.widget),
            }
    }
}

fn archive_row(ui: &mut egui::Ui, p: &Palette, label: &str, enabled: bool, checked: Option<bool>) -> egui::Response {
    #[cfg(test)]
    tests::presentation(ui);
    let response = super::super::menu::editable_row(ui, p, "archive-filled", label, enabled, checked, None, false);
    #[cfg(test)]
    tests::record_presentation(ui, &response);
    response
}

#[derive(Default)]
pub(super) struct Context {
    target: Option<Uuid>,
    popup: Option<egui::Id>,
    ticket: Option<crate::world::inventory_archive::EditTicket>,
    captured: bool,
    nonce: Option<u64>,
    seen: Option<u64>,
    activation: Activation,
}
impl Context {
    fn close(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.popup {
            egui::Popup::close_id(ctx, id);
        }
        *self = Self::default();
    }
    fn begin(&mut self, ui: &egui::Ui, id: Uuid, allowed: bool, opened: bool) {
        let frame = ui.ctx().cumulative_frame_nr();
        let popup = ui.layer_id().id;
        if (opened && self.seen != Some(frame))
            || self.target != Some(id)
            || self.popup != Some(popup)
            || self.seen.is_some_and(|seen| seen.checked_add(1).is_none_or(|next| next < frame))
        {
            *self = Self::default();
            self.target = Some(id);
            self.popup = Some(popup);
            self.nonce = NEXT_CONTEXT
                .try_update(Ordering::AcqRel, Ordering::Acquire, |next| next.checked_add(1))
                .ok();
            self.captured = !allowed;
        }
        self.seen = Some(frame);
    }
    fn finish_owner(&mut self, ctx: &egui::Context) {
        self.activation.settle(ctx);
        if self.seen != Some(ctx.cumulative_frame_nr()) {
            *self = Self::default();
        }
    }
    fn capture(&mut self, read: Option<&VisibilityRead<'_>>) {
        if !self.captured && self.target.is_some() {
            self.ticket = self.target.and_then(|id| read.and_then(|read| read.prepare_edit(id)));
            self.captured = true;
        }
    }
    fn paint(&mut self, ui: &mut egui::Ui, p: &Palette, owner: Uuid, revision: Key, profile: &str, input: InputRead) -> Option<Proposal> {
        let ticket = self.ticket.as_ref()?;
        let nonce = self.nonce?;
        let label = if ticket.previous() {
            aurora_i18n::tr!("inventory-archive-unarchive")
        } else {
            aurora_i18n::tr!("inventory-archive-archive")
        };
        let response = ui
            .push_id(("inventory_archive_original_context", nonce, ticket.namespace()), |ui| {
                archive_row(ui, p, label, true, None)
            })
            .inner;
        let (original, release) = self.activation.paint(ui, &response, input)?;
        ui.close();
        Some(Proposal {
            owner,
            revision,
            nonce,
            intent: Intent::Edit {
                ticket: ticket.clone(),
                profile: profile.into(),
            },
            control: original,
            release,
        })
    }
    fn validates(&self, ctx: &egui::Context, proposal: &Proposal) -> bool {
        self.nonce == Some(proposal.nonce)
            && matches!(&proposal.intent, Intent::Edit { ticket, .. } if self.ticket.as_ref() == Some(ticket))
            && self.activation.validates(ctx, proposal.control, proposal.release)
    }
}

#[derive(Default)]
pub(super) struct IncludeContext {
    binding: Option<(VisibilityBinding, bool)>,
    nonce: Option<u64>,
    seen: Option<u64>,
    activation: Activation,
}
impl IncludeContext {
    fn finish_owner(&mut self, ctx: &egui::Context) {
        self.activation.settle(ctx);
        if self.seen != Some(ctx.cumulative_frame_nr()) {
            *self = Self::default();
        }
    }
    fn paint(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        owner: Uuid,
        revision: Key,
        controls: Controls,
        enabled: bool,
        input: InputRead,
    ) -> Option<Proposal> {
        let frame = ui.ctx().cumulative_frame_nr();
        let binding = controls.binding.filter(|_| enabled).map(|binding| (binding, !controls.included));
        if self.binding != binding || self.seen.is_some_and(|seen| seen.checked_add(1).is_none_or(|next| next < frame)) {
            *self = Self::default();
            self.binding = binding;
            self.nonce = NEXT_CONTEXT
                .try_update(Ordering::AcqRel, Ordering::Acquire, |next| next.checked_add(1))
                .ok();
        }
        self.seen = Some(frame);
        let response = ui
            .push_id(("inventory_archive_include", self.nonce), |ui| {
                archive_row(
                    ui,
                    p,
                    aurora_i18n::tr!("inventory-archive-include"),
                    enabled,
                    Some(controls.included),
                )
            })
            .inner;
        let (binding, desired) = self.binding?;
        let nonce = self.nonce?;
        let (control, release) = self.activation.paint(ui, &response, input)?;
        ui.close();
        Some(Proposal {
            owner,
            revision,
            nonce,
            intent: Intent::Include { binding, desired },
            control,
            release,
        })
    }
    fn validates(&self, ctx: &egui::Context, proposal: &Proposal) -> bool {
        self.nonce == Some(proposal.nonce)
            && matches!(proposal.intent, Intent::Include { binding, desired } if self.binding == Some((binding, desired)))
            && self.activation.validates(ctx, proposal.control, proposal.release)
    }
}

pub(super) fn context(ui: &mut egui::Ui, p: &Palette, id: Uuid, profile: &str, st: &mut InventoryUi, actions: &mut Vec<InvAction>) {
    if !st.archive_drawing {
        return;
    }
    let Some(revision @ Key::Ordinary(_)) = st.archive_key else {
        return;
    };
    let allowed = st.archive_change_frame != Some(ui.ctx().cumulative_frame_nr());
    let opened = ui.input(|input| input.pointer.secondary_clicked()) || st.row_menu_opened_frame == Some(ui.ctx().cumulative_frame_nr());
    st.archive_context.begin(ui, id, allowed, opened);
    if let Some(proposal) = st.archive_context.paint(ui, p, st.ui_id, revision, profile, st.archive_input) {
        actions.push(InvAction::Archive(Box::new(proposal)));
    }
}

impl InventoryUi {
    pub(crate) fn archive_failure(&mut self, message: &str) {
        self.message = message.into();
    }
    pub(crate) fn archive_owner_mut(&mut self, owner: Uuid) -> Option<&mut State> {
        if owner.is_nil() {
            self.archive.as_mut()
        } else {
            self.windows
                .iter_mut()
                .find(|window| window.id == owner && window.additional && window.open && window.root.is_nil())?
                .state
                .archive
                .as_mut()
        }
    }
    pub(crate) fn archive_proposal_current(
        &self,
        ctx: &egui::Context,
        main_open: bool,
        prefs: &InventoryPreferences,
        proposal: &Proposal,
    ) -> bool {
        let state = if proposal.owner.is_nil() {
            if !main_open || !self.was_open {
                return false;
            }
            self
        } else {
            let Some(window) = self.windows.iter().find(|window| {
                window.id == proposal.owner && window.additional && window.open && window.root.is_nil() && window.state.was_open
            }) else {
                return false;
            };
            &window.state
        };
        state.archive.is_some()
            && state.archive_key == Some(proposal.revision)
            && match &proposal.intent {
                Intent::Edit { profile, .. } => {
                    prefs.context_menus.profiles.get(profile).is_some_and(|profile| profile.locked)
                        && state.archive_context.validates(ctx, proposal)
                }
                Intent::Include { .. } => state.archive_include_context.validates(ctx, proposal),
            }
    }
    /// Modifier/history revocation touches Archive proof only. Keep popup,
    /// context ticket, editor text/Undo and App-owned physical release guards.
    pub(crate) fn revoke_archive_gestures(&mut self) {
        self.archive_context.activation = Default::default();
        self.archive_include_context.activation = Default::default();
        self.archive_include_proposal = None;
        for window in &mut self.windows {
            window.state.revoke_archive_gestures();
        }
    }
    pub(crate) fn archive_input(&mut self, input: InputRead) {
        self.archive_input = input;
        for window in &mut self.windows {
            window.state.archive_input(input);
        }
    }
    pub(crate) fn retire_archive_input(&mut self, ctx: &egui::Context) {
        self.archive_context.close(ctx);
        self.archive_include_context = Default::default();
        self.archive_include_proposal = None;
        self.archive_change_frame = Some(ctx.cumulative_frame_nr());
        for window in &mut self.windows {
            window.state.retire_archive_input(ctx);
        }
    }
}

#[cfg(test)]
mod tests;
