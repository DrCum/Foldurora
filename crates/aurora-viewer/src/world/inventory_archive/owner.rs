//! Actual World ownership boundaries. The explicit take/restore scopes contain
//! only concrete controller calls, never a UI/event/user callback or a rebind.
use super::{World, controller::Controller, visibility::CpuTurn};
use crate::world::account_metadata_origin::GridDigest;

impl World {
    fn inventory_archive_profile_boundary(&mut self) {
        let origin = self
            .inventory_archive
            .as_ref()
            .filter(|archive| archive.profile_changed())
            .map(|archive| archive.original_for_profile_rebind(self));
        if let Some(origin) = origin {
            // Retire the old stored owner BEFORE constructing a fresh model;
            // no profile rebind occurs inside a controller take/restore gap.
            self.bind_inventory_archive(origin);
        }
    }
    pub(crate) fn retire_inventory_archive(&mut self) {
        if let Some(mut archive) = self.inventory_archive.take() {
            archive.retire();
        }
    }
    /// The caller supplies the one consumed actual submitted-login digest.
    /// This happens after actual Inventory owner/root/login are installed.
    pub(crate) fn bind_inventory_archive(&mut self, origin: Option<GridDigest>) {
        self.retire_inventory_archive();
        self.inventory_archive = Some(Controller::bind(self, origin));
    }
    pub(crate) fn observe_inventory_archive(&mut self) {
        self.inventory_archive_profile_boundary();
        if let Some(mut archive) = self.inventory_archive.take() {
            archive.observe(self);
            self.inventory_archive = Some(archive);
        }
    }
    pub(crate) fn poll_inventory_archive(&mut self) {
        self.inventory_archive_profile_boundary();
        if let Some(mut archive) = self.inventory_archive.take() {
            archive.poll(self);
            self.inventory_archive = Some(archive);
        }
    }
    pub(crate) fn pump_inventory_archive(&mut self, turn: &mut CpuTurn) {
        self.inventory_archive_profile_boundary();
        if let Some(mut archive) = self.inventory_archive.take() {
            archive.pump(self, turn);
            self.inventory_archive = Some(archive);
        }
    }
}
