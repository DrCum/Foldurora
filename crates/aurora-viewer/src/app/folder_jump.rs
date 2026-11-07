//! Actual login/Inventory owner admission and passive rooted-window callback.
//! FSFloaterFolderJump::owner/openFolder (Legacy Viewer abbe3cb0). No Inventory
//! fetch, move, deletion, asset disclosure or network command belongs here.

use super::{App, Screen};
use crate::keybinds::Input;
use crate::theme::Palette;
use crate::ui::inventory::folder_jump;
use winit::event::{ElementState, WindowEvent};
use winit::keyboard::{Key, PhysicalKey};

impl App {
    pub(super) fn observe_folder_jump(&mut self) {
        let visible = self
            .folder_jump
            .owner()
            .is_some_and(|owner| self.inventory_ui.jump_owner_visible(&self.egui_ctx, self.panels.inventory, owner));
        let live = matches!(self.screen, Screen::World) && !self.login_form.busy && self.host_input_allowed();
        self.folder_jump.observe(&self.world, live, visible);
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
        visible && self.folder_jump.open(&self.world, true, owner, visible)
    }

    pub(super) fn show_folder_jump(&mut self, ctx: &egui::Context, palette: &Palette) {
        if let Some(owner) = self.inventory_ui.take_jump_request() {
            let visible = self.inventory_ui.jump_owner_visible(ctx, self.panels.inventory, owner);
            let live = matches!(self.screen, Screen::World) && !self.login_form.busy && self.host_input_allowed();
            self.folder_jump.open(&self.world, live, owner, visible);
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
        if let Some(folder) = self.folder_jump.show(ctx, palette, &self.world, live, visible, blocked) {
            self.inventory_ui.open_folder_window(folder);
        }
    }
}
