//! Actual World ownership boundaries. The explicit take/restore scopes contain
//! only concrete controller calls, never a UI/event/user callback or a rebind.
use super::{World, controller::Controller, visibility::CpuTurn};
use crate::world::account_metadata_origin::GridDigest;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_INVENTORY: AtomicU64 = AtomicU64::new(1);
pub(crate) fn fresh_inventory_instance() -> Option<u64> {
    NEXT_INVENTORY
        .try_update(Ordering::AcqRel, Ordering::Acquire, |next| next.checked_add(1))
        .ok()
}

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
            if self.inventory_archive.as_ref().is_some_and(Controller::memory_only) {
                self.retire_inventory_archive();
            } else {
                self.bind_inventory_archive(origin);
            }
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
    pub(crate) fn bind_demo_inventory_archive(&mut self, witness: crate::app::inventory_archive::DemoWitness) {
        self.retire_inventory_archive();
        self.inventory_archive = Controller::demo(self, witness);
    }
    #[cfg(test)]
    pub(crate) fn bind_archive_offline_fixture(&mut self) {
        self.retire_inventory_archive();
        self.inventory_archive = Controller::offline_fixture(self);
    }
    #[cfg(test)]
    pub(crate) fn replace_archive_inventory_fixture(&mut self, inventory: crate::world::inventory::Inventory) {
        self.replace_inventory(inventory);
        self.bind_archive_offline_fixture();
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
