//! Explicit actual-World fixture boundary. Same Inventory is moved, never
//! cloned; existing nil login/agent, owners and rights stay exactly as authored.
use super::archive::{Gate, State};
use crate::world::{World, inventory::Inventory, inventory_archive::visibility::MaskRead};
use std::{
    cell::RefCell,
    ops::{Deref, DerefMut},
    sync::{Arc, OnceLock},
};

pub(in crate::ui::inventory) struct InventoryFixture {
    world: World,
    state: RefCell<State>,
}
impl InventoryFixture {
    pub(in crate::ui::inventory) fn new(inventory: Inventory) -> Self {
        static LIBRARY: OnceLock<Arc<crate::scene::avatar::AvatarLibrary>> = OnceLock::new();
        let mut world = World::new(
            LIBRARY
                .get_or_init(|| Arc::new(crate::scene::avatar::AvatarLibrary::load()))
                .clone(),
        );
        world.replace_archive_inventory_fixture(inventory);
        Self {
            world,
            state: RefCell::new(State::default()),
        }
    }
    pub(in crate::ui::inventory) fn with<T>(&self, f: impl for<'a> FnOnce(&'a Inventory, MaskRead<'a>, Gate<'a>) -> T) -> T {
        let mut state = self.state.borrow_mut();
        let instance = state.instance();
        let read = state.read(&self.world).expect("explicit offline fixture binding");
        let gate = State::ordinary(instance, Some(&read), self.world.inventory_archive_instance);
        f(&self.world.inventory, gate.mask().expect("ordinary actual fixture mask"), gate)
    }
    pub(in crate::ui::inventory) fn browse<T>(&mut self, f: impl for<'a, 'd> FnOnce(&mut super::archive::Browse<'a, 'd>) -> T) -> T {
        let identity = crate::world::inventory_archive::offline::Identity::observe(&self.world);
        let generation = self.world.inventory.generation;
        let mut demands = super::archive::Demands::new(&self.world.inventory);
        let result = self.with(|inventory, _, gate| {
            let mut browse = super::archive::Browse::fixture(inventory, gate, &mut demands);
            f(&mut browse)
        });
        if identity == crate::world::inventory_archive::offline::Identity::observe(&self.world)
            && generation == self.world.inventory.generation
        {
            for id in demands.fixture_requests() {
                self.world.inventory.request(id);
            }
        }
        result
    }
    pub(in crate::ui::inventory) fn into_inventory(mut self) -> Inventory {
        self.world.retire_inventory_archive();
        std::mem::take(&mut self.world.inventory)
    }
}
impl Deref for InventoryFixture {
    type Target = Inventory;
    fn deref(&self) -> &Inventory {
        &self.world.inventory
    }
}
impl DerefMut for InventoryFixture {
    fn deref_mut(&mut self) -> &mut Inventory {
        &mut self.world.inventory
    }
}
