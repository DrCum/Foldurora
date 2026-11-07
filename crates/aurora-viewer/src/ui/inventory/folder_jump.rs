//! Loaded-folder navigation from FSFloaterFolderJump (Legacy Viewer
//! indra/newview/fsfloaterfolderjump.cpp, pinned abbe3cb0, originally LGPL 2.1).
//! This module is not an Inventory mutation or a folder-content fetcher.

mod body;
mod model;
mod shortcut;
#[cfg(test)]
mod tests;

use super::InventoryUi;
use crate::theme::Palette;
use crate::ui::widgets;
use crate::world::World;
use model::{Policy, Source};
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

pub(crate) struct Chooser {
    body: body::Body,
    owner: Option<Uuid>,
    source: Option<Source>,
    profile_root: Option<&'static Path>,
    profile: u64,
    enter: crate::session_host::EnterRelease,
    space: crate::session_host::EnterRelease,
    blocked: [bool; 2],
    composing: bool,
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
            blocked: [false; 2],
            composing: false,
        }
    }
}

impl Chooser {
    pub(crate) fn owner(&self) -> Option<Uuid> {
        self.owner
    }

    pub(crate) fn composing(&self) -> bool {
        self.composing
    }

    fn context(&mut self, world: &World, live: bool, visible: bool) -> (Option<Source>, Policy) {
        let root = crate::session_host::profile::root();
        if root != self.profile_root {
            self.profile_root = root;
            self.profile = fresh_profile();
        }
        let policy = Policy {
            revision: world.rlva.revision,
            allowed: live && visible && !world.rlva.has("showinv"),
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

    pub(crate) fn observe(&mut self, world: &World, live: bool, visible: bool) {
        let (source, policy) = self.context(world, live, visible);
        self.body.observe(&world.inventory, source, policy);
        let source = self.body.source();
        if self.source.is_some() && self.source != source {
            self.close();
        } else if self.owner.is_some() {
            self.source = source;
        }
    }

    pub(crate) fn open(&mut self, world: &World, live: bool, owner: Uuid, visible: bool) -> bool {
        self.close();
        self.owner = Some(owner);
        self.observe(world, live, visible);
        if self.body.source().is_none() {
            self.close();
            return false;
        }
        true
    }

    pub(crate) fn close(&mut self) {
        self.body = Default::default();
        self.body.block_key(egui::Key::Enter, self.blocked[0]);
        self.body.block_key(egui::Key::Space, self.blocked[1]);
        self.owner = None;
        self.source = None;
    }

    /// Native physical evidence, independent of egui's derived repeat bit.
    /// Synthetic releases cannot clear an activation-key quarantine.
    pub(crate) fn event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::Focused(focused) => {
                self.enter.focused(*focused);
                self.space.focused(*focused);
                self.blocked = [true; 2];
                self.body.cancel_input();
                if !focused {
                    self.composing = false;
                }
                for key in [egui::Key::Enter, egui::Key::Space] {
                    self.body.block_key(key, true);
                }
            }
            WindowEvent::KeyboardInput { event, is_synthetic, .. } => {
                self.key_edge(&event.logical_key, event.state, event.repeat, *is_synthetic);
            }
            WindowEvent::Ime(Ime::Preedit(text, _)) => self.composing = !text.is_empty(),
            WindowEvent::Ime(Ime::Commit(_) | Ime::Disabled) => self.composing = false,
            _ => {}
        }
    }

    fn key_edge(&mut self, key: &Key, state: winit::event::ElementState, repeat: bool, synthetic: bool) -> bool {
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
    ) -> Option<Uuid> {
        self.observe(world, live, visible);
        self.owner?;
        let (source, policy) = self.context(world, live, visible);
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
        let result = floater
            .show(ctx, palette, &mut open, |ui| {
                ui.add_enabled_ui(!blocked, |ui| self.body.show(ui, palette, &world.inventory, source, policy))
                    .inner
            })
            .flatten();
        if !open || matches!(result, Some(body::Action::Close)) {
            self.close();
            return None;
        }
        let Some(body::Action::Open(ticket)) = result else { return None };
        // Final actual source/visibility/parent check before the existing
        // rooted-folder constructor. No selection or sender fallback.
        let (source, policy) = self.context(world, live, visible);
        if self.body.validate(ticket, &world.inventory, source, policy) {
            let folder = ticket.folder();
            self.close();
            Some(folder)
        } else {
            self.body.cancel_input();
            None
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
