//! Actual login/Inventory owner admission and passive rooted-window callback.
//! FSFloaterFolderJump::owner/openFolder (Legacy Viewer abbe3cb0). No Inventory
//! fetch, move, deletion, asset disclosure or network command belongs here.

use super::{App, Screen};
use crate::keybinds::Input;
use crate::theme::Palette;
use crate::ui::inventory::{InventoryUi, archive::State, folder_jump};
use crate::world::{World, inventory_archive::visibility::VisibilityRead};
use winit::event::{ElementState, WindowEvent};
use winit::keyboard::{Key, PhysicalKey};

fn archive_read<'a>(ui: &'a mut InventoryUi, world: &'a World, owner: uuid::Uuid) -> (Option<u64>, Option<VisibilityRead<'a>>) {
    let Some(state) = ui.archive_owner_mut(owner) else {
        return (None, None);
    };
    let instance = state.instance();
    (instance, state.read(world))
}

impl App {
    pub(super) fn observe_folder_jump(&mut self) {
        let visible = self
            .folder_jump
            .owner()
            .is_some_and(|owner| self.inventory_ui.jump_owner_visible(&self.egui_ctx, self.panels.inventory, owner));
        let live = matches!(self.screen, Screen::World) && !self.login_form.busy && self.host_input_allowed();
        let owner = self.folder_jump.owner().unwrap_or(uuid::Uuid::nil());
        let (instance, read) = archive_read(&mut self.inventory_ui, &self.world, owner);
        let gate = State::ordinary(instance, read.as_ref(), self.world.inventory_archive_instance);
        self.folder_jump.observe(&self.world, live, visible, gate);
    }

    /// Called before egui and viewer shortcuts/movement. Both owner selection
    /// and actual UI admission use the same last-painted claim; an admitted
    /// native chord is consumed here rather than replayed into RawInput.
    pub(super) fn folder_jump_event(&mut self, event: &WindowEvent) -> bool {
        self.observe_folder_jump();
        let WindowEvent::KeyboardInput { event, is_synthetic, .. } = event else {
            return false;
        };
        if *is_synthetic
            || event.repeat
            || !self.folder_jump.jump_allowed()
            || event.state != ElementState::Pressed
            || !matches!(&event.logical_key, Key::Character(key) if key.eq_ignore_ascii_case("j"))
            || !matches!(self.screen, Screen::World)
            || self.login_form.busy
            || !self.host_input_allowed()
            || !self.focused
            || self.folder_jump.composing()
            || self.exit_prompt.open
            || self.options_ui.capture.is_some()
            || self.options_ui.joystick_capturing()
            || self.web_has_focus()
            || self.media.focus.is_some()
        {
            return false;
        }
        let PhysicalKey::Code(code) = event.physical_key else {
            return false;
        };
        let input = Input::key(crate::keybinds::binding_code(code, &event.logical_key));
        let Some(owner) = folder_jump::shortcut_owner(&self.egui_ctx, &self.settings.keybinds, &input, self.key_mods(event)) else {
            return false;
        };
        // Recheck actual UI ownership at consumption, never a recent hidden
        // window or a current-selection fallback after the press.
        let visible = self.inventory_ui.jump_owner_visible(&self.egui_ctx, self.panels.inventory, owner);
        if !visible {
            return false;
        }
        let (instance, read) = archive_read(&mut self.inventory_ui, &self.world, owner);
        let gate = State::ordinary(instance, read.as_ref(), self.world.inventory_archive_instance);
        self.folder_jump.open(&self.world, true, owner, visible, gate)
    }

    pub(super) fn show_folder_jump(&mut self, ctx: &egui::Context, palette: &Palette) {
        if let Some(owner) = self.inventory_ui.take_jump_request() {
            let visible = self.inventory_ui.jump_owner_visible(ctx, self.panels.inventory, owner);
            let live = matches!(self.screen, Screen::World) && !self.login_form.busy && self.host_input_allowed();
            let (instance, read) = archive_read(&mut self.inventory_ui, &self.world, owner);
            let gate = State::ordinary(instance, read.as_ref(), self.world.inventory_archive_instance);
            self.folder_jump.open(&self.world, live, owner, visible, gate);
        }
        let visible = self
            .folder_jump
            .owner()
            .is_some_and(|owner| self.inventory_ui.jump_owner_visible(ctx, self.panels.inventory, owner));
        let live = matches!(self.screen, Screen::World) && !self.login_form.busy && self.host_input_allowed();
        let blocked = !self.focused
            || self.exit_prompt.open
            || self.options_ui.capture.is_some()
            || self.options_ui.joystick_capturing()
            || self.web_has_focus()
            || self.media.focus.is_some();
        let owner = self.folder_jump.owner().unwrap_or(uuid::Uuid::nil());
        let (instance, read) = archive_read(&mut self.inventory_ui, &self.world, owner);
        let gate = State::ordinary(instance, read.as_ref(), self.world.inventory_archive_instance);
        self.folder_jump.show(ctx, palette, &self.world, live, visible, blocked, gate);
    }

    /// The actual whole-context run has completed, including any later widget
    /// requesting another layout pass. Navigation can now consume its one
    /// exact original proposal against current owner/source/Inventory facts.
    pub(super) fn finish_folder_jump_input(&mut self, ctx: &egui::Context) {
        let visible = self
            .folder_jump
            .owner()
            .is_some_and(|owner| self.inventory_ui.jump_owner_visible(ctx, self.panels.inventory, owner));
        let live = matches!(self.screen, Screen::World) && !self.login_form.busy && self.host_input_allowed();
        let owner = self.folder_jump.owner().unwrap_or(uuid::Uuid::nil());
        let (instance, read) = archive_read(&mut self.inventory_ui, &self.world, owner);
        let gate = State::ordinary(instance, read.as_ref(), self.world.inventory_archive_instance);
        let folder = self.folder_jump.finish_input(ctx, &self.world, live, visible, gate);
        drop(read);
        if let Some(folder) = folder {
            // The whole-context proposal was consumed. Reacquire the SAME
            // original owner's actual gate immediately before the callback;
            // never reuse an earlier layout pass or infer Include.
            let visible = self.inventory_ui.jump_owner_visible(ctx, self.panels.inventory, owner);
            let (instance, read) = archive_read(&mut self.inventory_ui, &self.world, owner);
            let gate = State::ordinary(instance, read.as_ref(), self.world.inventory_archive_instance);
            let current = live
                && visible
                && read.as_ref().is_some_and(|read| read.passes(folder))
                && folder_jump::current_folder(&self.world.inventory, folder, gate.mask());
            drop(read);
            if current {
                self.inventory_ui.open_folder_window(folder);
            }
        }
    }
}
