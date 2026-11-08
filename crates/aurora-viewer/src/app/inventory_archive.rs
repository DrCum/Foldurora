//! Actual App event/frame integration, still test-gated until every ordinary
//! Inventory and Folder Jump consumer uses the same borrowed visibility gate.
use super::App;

impl App {
    pub(super) fn observe_inventory_archive(&mut self) {
        self.world.observe_inventory_archive();
    }
    /// The only App admission site is immediately before the whole UI run,
    /// outside all retryable paint closures. World reset cannot replace this
    /// App-owned budget. No observer/native/view helper advances it.
    pub(super) fn pump_inventory_archive(&mut self) {
        self.world.poll_inventory_archive();
        if self.inventory_archive_turn.advance() {
            self.world.pump_inventory_archive(&mut self.inventory_archive_turn);
        }
    }
}
