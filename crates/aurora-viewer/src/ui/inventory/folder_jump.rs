//! Loaded-folder navigation from FSFloaterFolderJump (Legacy Viewer
//! indra/newview/fsfloaterfolderjump.cpp, pinned abbe3cb0, originally LGPL 2.1).
//! This module is not an Inventory mutation or a folder-content fetcher.

#[cfg(test)]
mod archive_tests;
mod body;
#[cfg(test)]
pub(super) mod fixtures;
mod model;
mod shortcut;
#[cfg(test)]
mod tests;

use super::{InventoryUi, archive};
use crate::theme::Palette;
use crate::ui::widgets;
use crate::world::World;
use model::{Policy, Source, Ticket};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;
use winit::event::{Ime, WindowEvent};
use winit::keyboard::{Key, ModifiersState, NamedKey};

fn fresh_profile() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| value.checked_add(1))
        .unwrap_or(0)
}
pub(crate) fn current_folder(
    inventory: &crate::world::inventory::Inventory,
    id: Uuid,
    mask: Option<crate::world::inventory_archive::visibility::MaskRead<'_>>,
) -> bool {
    model::current_path(inventory, id, mask)
}

pub(crate) struct Chooser {
    body: body::Body,
    owner: Option<Uuid>,
    source: Option<Source>,
    profile_root: Option<&'static Path>,
    profile: u64,
    enter: crate::session_host::EnterRelease,
    space: crate::session_host::EnterRelease,
    jump: crate::session_host::EnterRelease,
    jump_blocked: bool,
    blocked: [bool; 2],
    composing: bool,
    /// Only an IME preedit actually owned by this query can quarantine input.
    /// Global IME activity in a canonical chat/other field never grants this.
    owned_composition: bool,
    composition_epoch: u64,
    owned_terminal_epoch: Option<u64>,
    quarantine_epoch: Option<u64>,
    quarantine_terminal_seen: bool,
    ime_exhausted: bool,
    ime_quarantined: bool,
    ime_frame: Option<u64>,
    ime_frame_quarantined: bool,
    ime_terminal_consumed: bool,
    pending_open: Option<Ticket>,
    archive_key: Option<(archive::Key, u64)>,
    archive_revision: u64,
    archive_exhausted: bool,
}

impl Default for Chooser {
    fn default() -> Self {
        Self {
            body: Default::default(),
            owner: None,
            source: None,
            profile_root: crate::session_host::profile::root(),
            profile: fresh_profile(),
            enter: Default::default(),
            space: Default::default(),
            jump: Default::default(),
            jump_blocked: false,
            blocked: [false; 2],
            composing: false,
            owned_composition: false,
            composition_epoch: 0,
            owned_terminal_epoch: None,
            quarantine_epoch: None,
            quarantine_terminal_seen: false,
            ime_exhausted: false,
            ime_quarantined: false,
            ime_frame: None,
            ime_frame_quarantined: false,
            ime_terminal_consumed: false,
            pending_open: None,
            archive_key: None,
            archive_revision: 0,
            archive_exhausted: false,
        }
    }
}

impl Chooser {
    pub(crate) fn owner(&self) -> Option<Uuid> {
        self.owner
    }

    pub(crate) fn composing(&self) -> bool {
        self.composing || self.ime_exhausted
    }

    pub(crate) fn jump_allowed(&self) -> bool {
        !self.jump_blocked
    }

    fn context(&mut self, world: &World, live: bool, visible: bool, gate: archive::Gate<'_>) -> (Option<Source>, Policy) {
        let root = crate::session_host::profile::root();
        if root != self.profile_root {
            self.profile_root = root;
            self.profile = fresh_profile();
        }
        let key = Some((gate.key(), world.rlva.revision));
        if self.archive_key != key {
            self.archive_key = key;
            self.pending_open = None;
            if let Some(next) = self.archive_revision.checked_add(1) {
                self.archive_revision = next;
            } else {
                self.archive_exhausted = true;
            }
        }
        let policy = Policy {
            revision: self.archive_revision,
            allowed: !self.archive_exhausted
                && gate.mask().is_some()
                && gate.passes(world.inventory.root)
                && live
                && visible
                && !world.rlva.has("showinv"),
        };
        let source = self.owner.and_then(|window| {
            let login = world.login.as_ref()?;
            (policy.allowed && login.agent_id == world.agent_id && world.main().is_some() && world.movement_complete && !world.teleporting)
                .then_some(Source {
                    agent: world.agent_id,
                    session: login.session_id,
                    owner: world.inventory.owner,
                    root: world.inventory.root,
                    profile: self.profile,
                    window,
                })
        });
        (source, policy)
    }

    pub(crate) fn observe(&mut self, world: &World, live: bool, visible: bool, gate: archive::Gate<'_>) {
        let (source, policy) = self.context(world, live, visible, gate);
        self.body.observe_visible(&world.inventory, source, policy, gate.mask());
        let source = self.body.source();
        if self.source.is_some() && self.source != source {
            self.close();
        } else if self.owner.is_some() {
            self.source = source;
        }
    }

    pub(crate) fn open(&mut self, world: &World, live: bool, owner: Uuid, visible: bool, gate: archive::Gate<'_>) -> bool {
        if self.ime_exhausted || self.composing && !self.owned_composition {
            return false;
        }
        self.close();
        self.owner = Some(owner);
        self.observe(world, live, visible, gate);
        if self.body.source().is_none() {
            self.close();
            return false;
        }
        true
    }

    pub(crate) fn close(&mut self) {
        if self.owned_composition {
            self.ime_quarantined = true;
            self.quarantine_epoch = Some(self.composition_epoch);
            self.quarantine_terminal_seen = self.owned_terminal_epoch == Some(self.composition_epoch);
        }
        let rejected = self.body.rejected_presses();
        self.body = Default::default();
        for (key, rejected) in [(egui::Key::Enter, rejected[0]), (egui::Key::Space, rejected[1])] {
            if rejected {
                self.body.reject_press(key);
            }
        }
        self.body.block_key(egui::Key::Enter, self.blocked[0]);
        self.body.block_key(egui::Key::Space, self.blocked[1]);
        self.owner = None;
        self.source = None;
        self.pending_open = None;
    }

    /// Native physical evidence, independent of egui's derived repeat bit.
    /// Synthetic releases cannot clear an activation-key quarantine.
    pub(crate) fn event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::Focused(focused) => {
                if !focused && self.owned_composition {
                    self.ime_quarantined = true;
                    self.quarantine_epoch = Some(self.composition_epoch);
                    self.quarantine_terminal_seen = self.owned_terminal_epoch == Some(self.composition_epoch);
                }
                self.enter.focused(*focused);
                self.space.focused(*focused);
                self.jump.focused(*focused);
                self.jump_blocked = true;
                self.blocked = [true; 2];
                self.body.cancel_input();
                for key in [egui::Key::Enter, egui::Key::Space] {
                    self.body.block_key(key, true);
                }
            }
            WindowEvent::KeyboardInput { event, is_synthetic, .. } => {
                self.key_edge(&event.logical_key, event.state, event.repeat, *is_synthetic);
            }
            WindowEvent::Ime(Ime::Preedit(text, _)) => self.composing |= !text.is_empty(),
            WindowEvent::Ime(Ime::Commit(_) | Ime::Disabled) => self.composing = false,
            _ => {}
        }
    }

    /// Observe ownership before egui receives the original native event.
    /// No preedit/commit text is copied or forwarded as a new event.
    pub(crate) fn native_event(&mut self, ctx: &egui::Context, event: &WindowEvent) {
        if matches!(event, WindowEvent::Ime(Ime::Preedit(text, _)) if !text.is_empty())
            && !self.composing
            && (!self.ime_quarantined || self.quarantine_terminal_seen)
        {
            let Some(epoch) = self.composition_epoch.checked_add(1) else {
                self.ime_exhausted = true;
                self.close();
                return;
            };
            self.composition_epoch = epoch;
            self.owned_composition = self.body.query_focused(ctx);
        }
        if matches!(event, WindowEvent::Ime(Ime::Commit(_) | Ime::Disabled)) {
            if self.owned_composition {
                self.owned_terminal_epoch = Some(self.composition_epoch);
            }
            if self.ime_quarantined
                && (self.quarantine_epoch == Some(self.composition_epoch) || matches!(event, WindowEvent::Ime(Ime::Disabled)))
            {
                self.quarantine_terminal_seen = true;
            }
        }
        self.event(event);
    }

    /// Runs before ANY field consumes this main-context batch. Quarantine is
    /// solely for a proven old chooser-query composition across retirement.
    pub(crate) fn prepare_input(&mut self, ctx: &egui::Context) {
        let frame = ctx.cumulative_frame_nr();
        if self.ime_frame != Some(frame) {
            self.ime_frame = Some(frame);
            self.ime_frame_quarantined = false;
            self.ime_terminal_consumed = false;
        }
        self.ime_frame_quarantined |= self.ime_quarantined;
        // Empty Preedit may precede a later Commit, so it cannot release an
        // old-source composition fence. Only the actual terminal can do that.
        self.ime_terminal_consumed |= ctx.input(|input| {
            input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Ime(egui::ImeEvent::Commit(_))))
        });
        if self.ime_frame_quarantined {
            self.body
                .quarantine_ime(ctx, self.ime_terminal_consumed || self.quarantine_terminal_seen);
            ctx.input_mut(|input| {
                input.events.retain(|event| {
                    !matches!(
                        event,
                        egui::Event::Ime(_)
                            | egui::Event::Key {
                                key: egui::Key::Enter | egui::Key::Space,
                                ..
                            }
                    )
                })
            });
        }
    }

    /// Only the completed real context batch, never a native Up or a new Body,
    /// can release rejected-press/old-composition quarantine.
    fn finish_consumed_batch(&mut self, ctx: &egui::Context) {
        self.body.finish_batch(ctx);
        if self.owned_terminal_epoch == Some(self.composition_epoch) {
            self.owned_composition = false;
        }
        if self.quarantine_terminal_seen {
            self.ime_quarantined = false;
            self.quarantine_epoch = None;
        }
        self.owned_terminal_epoch = None;
        self.quarantine_terminal_seen = false;
        self.ime_terminal_consumed = false;
        self.ime_frame_quarantined = false;
    }

    fn take_open_visible(
        &mut self,
        ctx: &egui::Context,
        inventory: &crate::world::inventory::Inventory,
        source: Option<Source>,
        policy: Policy,
        gate: archive::Gate<'_>,
    ) -> Option<Uuid> {
        self.pending_open
            .take()
            .filter(|ticket| self.body.take_open_visible(*ticket, ctx, inventory, source, policy, gate.mask()))
            .map(Ticket::folder)
    }

    pub(crate) fn finish_input(
        &mut self,
        ctx: &egui::Context,
        world: &World,
        live: bool,
        visible: bool,
        gate: archive::Gate<'_>,
    ) -> Option<Uuid> {
        self.observe(world, live, visible, gate);
        let (source, policy) = self.context(world, live, visible, gate);
        let folder = self.take_open_visible(ctx, &world.inventory, source, policy, gate);
        self.finish_consumed_batch(ctx);
        if folder.is_some() {
            self.close();
        }
        folder
    }

    fn draw_body_visible(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        inventory: &crate::world::inventory::Inventory,
        source: Option<Source>,
        policy: Policy,
        gate: archive::Gate<'_>,
    ) -> bool {
        match self.body.show_visible(ui, palette, inventory, source, policy, gate.mask()) {
            Some(body::Action::Open(ticket)) => {
                self.pending_open.get_or_insert(ticket);
                false
            }
            Some(body::Action::Close) => true,
            None => false,
        }
    }

    fn key_edge(&mut self, key: &Key, state: winit::event::ElementState, repeat: bool, synthetic: bool) -> bool {
        if matches!(key, Key::Character(value) if value.eq_ignore_ascii_case("j")) {
            self.jump_blocked = self
                .jump
                .suppress(&Key::Named(NamedKey::Enter), state, repeat, synthetic, ModifiersState::empty())
                || synthetic
                || repeat;
            return self.jump_blocked;
        }
        let index = match key {
            Key::Named(NamedKey::Enter) => 0,
            key if matches!(key, Key::Named(NamedKey::Space)) || matches!(key, Key::Character(value) if value.as_str() == " ") => 1,
            _ => return false,
        };
        let fence = if index == 0 { &mut self.enter } else { &mut self.space };
        // Independent observational Space fence; actual native and egui
        // events, including query-space typing, remain untouched.
        self.blocked[index] =
            fence.suppress(&Key::Named(NamedKey::Enter), state, repeat, synthetic, ModifiersState::empty()) || synthetic || repeat;
        self.body
            .block_key(if index == 0 { egui::Key::Enter } else { egui::Key::Space }, self.blocked[index]);
        if state == winit::event::ElementState::Pressed && self.blocked[index] {
            self.body.reject_press(if index == 0 { egui::Key::Enter } else { egui::Key::Space });
        }
        self.blocked[index]
    }

    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        palette: &Palette,
        world: &World,
        live: bool,
        visible: bool,
        blocked: bool,
        gate: archive::Gate<'_>,
    ) {
        self.observe(world, live, visible, gate);
        self.prepare_input(ctx);
        if self.owner.is_none() {
            return;
        }
        let (source, policy) = self.context(world, live, visible, gate);
        if blocked || widgets::floater_minimized(ctx, "inventory_folder_jump") {
            self.body.cancel_input();
        }
        let mut open = true;
        let mut floater = widgets::Floater::new(
            "inventory_folder_jump",
            aurora_i18n::tr!("inventory-jump-title"),
            ctx.content_rect().center() - egui::vec2(260.0, 220.0),
            egui::vec2(520.0, 440.0),
        )
        .steady_size();
        floater.min_size = egui::vec2(300.0, 260.0);
        let result = floater.show(ctx, palette, &mut open, |ui| {
            ui.add_enabled_ui(!blocked, |ui| {
                self.draw_body_visible(ui, palette, &world.inventory, source, policy, gate)
            })
            .inner
        });
        if !open || result == Some(true) {
            self.close();
        }
    }
}

pub(super) fn supported(state: &InventoryUi) -> bool {
    state.scope.is_nil() && state.tab == 0 && (state.ui_id.is_nil() || state.ui_id == super::ADDITIONAL_WINDOW)
}

pub(super) fn claim(ui: &egui::Ui, state: &InventoryUi) {
    if supported(state) {
        shortcut::claim(ui, state.ui_id);
    }
}

pub(crate) fn shortcut_owner(
    ctx: &egui::Context,
    bindings: &crate::keybinds::KeyBindings,
    input: &crate::keybinds::Input,
    mods: crate::keybinds::Mods,
) -> Option<Uuid> {
    shortcut::available(bindings, input, mods).then(|| shortcut::owner(ctx)).flatten()
}

impl InventoryUi {
    pub(crate) fn jump_owner_visible(&self, ctx: &egui::Context, main_open: bool, owner: Uuid) -> bool {
        if widgets::interface_hidden(ctx) {
            return false;
        }
        if owner.is_nil() {
            main_open && self.was_open && supported(self) && !widgets::floater_minimized(ctx, "inventory")
        } else {
            self.windows.iter().any(|window| {
                window.additional
                    && window.id == owner
                    && window.open
                    && window.state.was_open
                    && supported(&window.state)
                    && !widgets::floater_minimized(ctx, &super::floater_id(window.id))
            })
        }
    }

    pub(crate) fn take_jump_request(&mut self) -> Option<Uuid> {
        let main = std::mem::take(&mut self.jump_requested);
        if main && supported(self) {
            return Some(Uuid::nil());
        }
        for window in &mut self.windows {
            let requested = std::mem::take(&mut window.state.jump_requested);
            if requested && window.additional && supported(&window.state) {
                return Some(window.id);
            }
        }
        None
    }
}

#[cfg(test)]
impl Chooser {
    fn draw_body(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        fixture: &fixtures::InventoryFixture,
        source: Option<Source>,
        policy: Policy,
    ) -> bool {
        fixture.with(|inventory, _, gate| self.draw_body_visible(ui, palette, inventory, source, policy, gate))
    }
    fn take_open(
        &mut self,
        ctx: &egui::Context,
        fixture: &fixtures::InventoryFixture,
        source: Option<Source>,
        policy: Policy,
    ) -> Option<Uuid> {
        fixture.with(|inventory, _, gate| self.take_open_visible(ctx, inventory, source, policy, gate))
    }
}
