//! Runtime-bound native additional Inventory lifecycle. Effects occur only after a
//! committed workspace; the saved schema never supplies arbitrary UI IDs.
use super::{App, Renderer, Window, ui};
use crate::workspace::{
    controller::{InventoryPending, Status},
    model::InventoryWindow,
};
use crate::world::inventory_archive::visibility::VisibilityStatus;
use std::time::{Duration, Instant};
use ui::inventory::workspace_state as native;

impl App {
    fn workspace_inventory_permit(&self, renderer: &Renderer, window: &Window) -> bool {
        self.workspace_permit(renderer, window)
            && self.workspace_expanded_source()
            && !self.panels.settings
            && self.workspaces.preview.is_none()
            && !self.world.rlva.has("showinv")
            && self.world.inventory.owner == self.world.agent_id
            && !renderer.failed()
            && !renderer.device_lost()
            && renderer.stalled_for().is_none()
    }
    pub(super) fn workspace_inventory_capture(&mut self, folders: bool) -> Result<InventoryWindow, Status> {
        if !self.workspace_expanded_source() || self.world.rlva.has("showinv") || !self.inventory_ui.workspace_select_open_owner() {
            return Err(Status::InventoryWindowUnavailable);
        }
        let visible = self
            .inventory_ui
            .workspace_additional_open()
            .ok_or(Status::InventoryWindowUnavailable)?;
        let owner = self.inventory_ui.workspace_additional_owner();
        let name = self.inventory_ui.workspace_additional_name();
        let minimized = visible
            && name
                .as_ref()
                .is_some_and(|name| ui::widgets::floater_minimized(&self.egui_ctx, name));
        let mut snapshot = InventoryWindow {
            visible,
            minimized,
            geometry: name
                .as_ref()
                .and_then(|name| ui::layout::workspace_named_capture(&self.egui_ctx, name)),
            folder: None,
            expanded: None,
        };
        if !visible || minimized || !folders {
            return Ok(snapshot);
        }
        let owner = owner.ok_or(Status::InventoryWindowUnavailable)?;
        if !self.inventory_ui.jump_owner_visible(&self.egui_ctx, self.panels.inventory, owner) {
            return Err(Status::InventoryWindowUnavailable);
        }
        let folder = self
            .inventory_ui
            .workspace_additional_state()
            .and_then(|state| state.workspace_folder(&self.world.inventory));
        let refs = self.inventory_ui.workspace_additional_expanded_refs();
        let (_, read) = super::folder_jump::archive_read(&mut self.inventory_ui, &self.world, owner);
        let read = read
            .filter(|r| r.status() == VisibilityStatus::Ready && r.revision().is_some())
            .ok_or(Status::InventoryWindowUnavailable)?;
        if let Some(folder) = folder {
            if !native::fetched_path(&self.world.inventory, folder, read.mask()) {
                return Err(Status::InventoryWindowUnavailable);
            }
            snapshot.folder = Some(folder);
        }
        if let Some(refs) = refs {
            snapshot.expanded =
                Some(native::capture(&self.world.inventory, &refs, read.mask()).map_err(|_| Status::InventoryWindowUnavailable)?);
        }
        Ok(snapshot)
    }
    pub(super) fn workspace_inventory_restore(&mut self, wanted: InventoryWindow, renderer: &Renderer, window: &Window) {
        self.workspaces.inventory_pending = None;
        if wanted.validate().is_err()
            || !self.workspace_inventory_permit(renderer, window)
            || !self.inventory_ui.workspace_select_open_owner()
        {
            self.workspaces.status = Status::InventoryWindowUnavailable;
            return;
        }
        let previous_owner = self.inventory_ui.workspace_additional_owner();
        if !self
            .inventory_ui
            .workspace_additional_visibility(&self.egui_ctx, wanted.visible, wanted.minimized)
        {
            self.workspaces.status = Status::InventoryWindowUnavailable;
            return;
        }
        if let Some(geometry) = wanted.geometry
            && let Some(name) = self.inventory_ui.workspace_additional_name()
        {
            ui::layout::workspace_named_apply(&self.egui_ctx, &name, geometry);
        }
        if !wanted.has_folders() {
            return;
        }
        let Some(owner) = self.inventory_ui.workspace_additional_owner() else {
            return;
        };
        let fresh = previous_owner != Some(owner);
        let Some(source) = self.workspaces.source.clone() else { return };
        let Some(metadata) = self.world.inventory_archive.as_ref().and_then(|a| a.metadata_revision(&self.world)) else {
            self.workspaces.status = Status::InventoryWindowUnavailable;
            return;
        };
        self.workspaces.inventory_pending = Some(InventoryPending {
            owner,
            source,
            binding: self.workspaces.binding,
            rules: (self.world.rlva.revision, self.world.rlva.enabled()),
            metadata,
            wanted,
            passes: 0,
            fresh,
            deadline: Instant::now() + Duration::from_secs(2),
        });
    }
    pub(super) fn workspace_inventory_tick(&mut self, renderer: &Renderer, window: &Window) {
        let Some(mut pending) = self.workspaces.inventory_pending.take() else {
            return;
        };
        let input = self.egui_ctx.input(|i| {
            i.events
                .iter()
                .any(|e| !matches!(e, egui::Event::PointerMoved(_) | egui::Event::PointerGone))
        });
        if !self.workspace_inventory_permit(renderer, window)
            || !pending.same_owner(self.inventory_ui.workspace_additional_owner())
            || !pending.current(
                self.workspaces.source.as_ref(),
                self.workspaces.binding,
                (self.world.rlva.revision, self.world.rlva.enabled()),
                self.world.inventory_archive.as_ref().and_then(|a| a.metadata_revision(&self.world)),
                Instant::now(),
            )
            || input
        {
            self.workspaces.status = Status::InventoryWindowUnavailable;
            return;
        }
        pending.passes += 1;
        if !self
            .inventory_ui
            .jump_owner_visible(&self.egui_ctx, self.panels.inventory, pending.owner)
        {
            self.workspaces.inventory_pending = Some(pending);
            self.egui_ctx.request_repaint();
            return;
        }
        if !self.inventory_ui.workspace_additional_idle() || (pending.fresh && !self.inventory_ui.workspace_additional_untouched()) {
            self.workspaces.status = Status::InventoryWindowUnavailable;
            return;
        }
        match self.workspace_inventory_context(pending.owner, &pending.wanted) {
            Some(true) => {}
            Some(false) => self.workspaces.status = Status::InventoryWindowUnavailable,
            None => {
                self.workspaces.inventory_pending = Some(pending);
                self.egui_ctx.request_repaint();
            }
        }
    }
    fn workspace_inventory_context(&mut self, owner: uuid::Uuid, wanted: &InventoryWindow) -> Option<bool> {
        if self.inventory_ui.workspace_additional_owner() != Some(owner) {
            return Some(false);
        }
        // All borrowed policy/path qualification finishes before mutable UI
        // consumption, on this same App turn with no reentrant callback.
        let refs = self.inventory_ui.workspace_additional_expanded_refs();
        let current = {
            let (_, read) = super::folder_jump::archive_read(&mut self.inventory_ui, &self.world, owner);
            let read = read.filter(|r| r.status() == VisibilityStatus::Ready && r.revision().is_some())?;
            if wanted
                .folder
                .is_some_and(|folder| !native::fetched_path(&self.world.inventory, folder, read.mask()))
            {
                return Some(false);
            }
            if let Some(expanded) = &wanted.expanded {
                if !native::restorable(&self.world.inventory, expanded, read.mask()) {
                    return Some(false);
                }
                let Some(refs) = refs else { return Some(false) };
                let Ok(current) = native::capture(&self.world.inventory, &refs, read.mask()) else {
                    return Some(false);
                };
                Some(current)
            } else {
                None
            }
        };
        if let Some(folder) = wanted.folder {
            let Some(state) = self.inventory_ui.workspace_additional_state_mut() else {
                return Some(false);
            };
            state.restore_workspace_folder(&self.world.inventory, folder);
        }
        if let Some(expanded) = &wanted.expanded {
            let Some(current) = current else { return Some(false) };
            return Some(self.inventory_ui.workspace_restore_additional_expanded(expanded, &current));
        }
        Some(true)
    }
}
