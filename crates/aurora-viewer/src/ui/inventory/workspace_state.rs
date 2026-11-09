//! Bounded native plain-tree expansion, Folderstorm fsworkspacelayout.h.
//! Caller owns native/source admission. No hidden windows or fetch calls.
use super::{FetchState, Inventory, InventoryUi, folder_jump};
use crate::workspace::model::{ExpandedFolders, MAX_EXPANDED};
use crate::world::inventory_archive::visibility::MaskRead;
use uuid::Uuid;

/// A folder and every ancestor must already be fetched. A freshly expanded
/// unknown branch would otherwise make PlainTree's normal lazy loader fetch it.
fn fetched_path(inv: &Inventory, id: Uuid, mask: MaskRead<'_>) -> bool {
    if !folder_jump::current_folder(inv, id, Some(mask)) {
        return false;
    }
    let mut at = id;
    for _ in 0..=128 {
        let Some(folder) = inv.folders.get(&at).filter(|f| f.state == FetchState::Fetched) else {
            return false;
        };
        if at == inv.root {
            return true;
        }
        at = folder.info.parent;
    }
    false
}
/// Qualify only current ordinary folder paths. Hidden/foreign paths are never
/// captured; incomplete visible paths refuse capture rather than erase state.
pub(crate) fn capture(inv: &Inventory, refs: &[Uuid], mask: MaskRead<'_>) -> Result<ExpandedFolders, ()> {
    if refs.len() > MAX_EXPANDED || !mask.for_inventory(inv) || mask.revision().is_none() {
        return Err(());
    }
    let mut wanted = Vec::new();
    for id in refs {
        if folder_jump::current_folder(inv, *id, Some(mask)) {
            if !fetched_path(inv, *id, mask) {
                return Err(());
            }
            wanted.push(*id);
        }
    }
    ExpandedFolders::try_from(wanted).map_err(|_| ())
}
pub(crate) fn restorable(inv: &Inventory, wanted: &ExpandedFolders, mask: MaskRead<'_>) -> bool {
    mask.for_inventory(inv) && mask.revision().is_some() && wanted.refs().iter().all(|id| fetched_path(inv, *id, mask))
}
impl InventoryUi {
    fn workspace_plain(&self) -> bool {
        self.is_main()
            && self.scope.is_nil()
            && self.tab == 0
            && !self.result_view()
            && !self.aurora.is_some_and(|frame| frame.explorer)
            && self.rename.is_none()
            && self.dialog.is_none()
            && self.progress.is_none()
            && self.expand_all.is_none()
    }
    /// None means another view, not a valid empty expansion snapshot.
    pub(crate) fn workspace_expanded_refs(&self) -> Option<Vec<Uuid>> {
        self.workspace_plain().then(|| self.plain.workspace_open())
    }
    /// Recheck compatibility at consumption. Only the current qualified scope
    /// is closed; hidden/unrelated state and other Inventory owners stay intact.
    pub(crate) fn workspace_restore_expanded(&mut self, wanted: &ExpandedFolders, current: &ExpandedFolders) -> bool {
        if !self.workspace_plain() {
            return false;
        }
        for id in current.refs() {
            self.plain.set_open(*id, wanted.refs().contains(id));
        }
        self.plain.open_all(wanted.refs().iter().copied());
        // Stop a preceding reveal from reopening folders excluded explicitly by
        // this snapshot. Selection/edit/search data and keyboard focus stay put.
        self.reveal_path.retain(|id| wanted.refs().contains(id));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::inventory::archive::State;
    use crate::world::{
        World,
        inventory_archive::{
            controller::testing,
            visibility::{CpuTurn, VisibilityStatus},
        },
    };
    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    #[test]
    fn workspace_expanded_native_owner_refuses_search_other_windows_and_overflow() {
        let mut st = InventoryUi::default();
        st.plain.open_all([id(1), id(2)]);
        assert_eq!(st.workspace_expanded_refs().unwrap().len(), 2);
        st.search = "query".into();
        assert!(st.workspace_expanded_refs().is_none());
        st.search.clear();
        st.ui_id = id(99);
        assert!(st.workspace_expanded_refs().is_none());
        st.ui_id = Uuid::nil();
        st.tab = 1;
        assert!(st.workspace_expanded_refs().is_none());
        st.tab = 0;
        st.scope = id(2);
        assert!(st.workspace_expanded_refs().is_none());
        st.scope = Uuid::nil();
        st.plain.open_all((1..=300).map(id));
        assert_eq!(
            st.workspace_expanded_refs().unwrap().len(),
            MAX_EXPANDED + 1,
            "bounded overflow evidence, not a truncated valid set"
        );
    }
    #[test]
    fn workspace_expanded_native_apply_touches_only_qualified_scope_and_preserves_edit_selection() {
        let mut st = InventoryUi::default();
        st.plain.open_all([id(1), id(2), id(99)]);
        st.selected = Some(id(7));
        st.reveal_path.extend([id(1), id(2)]);
        let current = ExpandedFolders::try_from(vec![id(1), id(2)]).unwrap();
        let wanted = ExpandedFolders::try_from(vec![id(1), id(3)]).unwrap();
        assert!(st.workspace_restore_expanded(&wanted, &current));
        assert!(st.plain.is_open(id(1)) && st.plain.is_open(id(3)) && st.plain.is_open(id(99)));
        assert!(!st.plain.is_open(id(2)));
        assert_eq!(st.selected, Some(id(7)));
        assert!(!st.reveal_path.contains(&id(2)));
        st.search = "unchanged".into();
        assert!(!st.workspace_restore_expanded(&ExpandedFolders::default(), &wanted));
        assert!(st.plain.is_open(id(3)));
        assert_eq!(st.search, "unchanged");
    }
    fn world() -> World {
        let mut world = crate::world::inventory_archive::tests::native_fixture_world();
        world.inventory.folders.get_mut(&id(8)).unwrap().library = true;
        for (parent, children) in [(1, vec![2, 4, 5]), (2, vec![3]), (5, vec![6]), (7, vec![8])] {
            world.inventory.folders.get_mut(&id(parent)).unwrap().children = children.into_iter().map(id).collect();
        }
        for folder in world.inventory.folders.values_mut() {
            folder.state = FetchState::Fetched;
        }
        world.inventory_archive = Some(testing::native_loaded(&world, &[id(2)]));
        pump(&mut world);
        world
    }
    fn pump(world: &mut World) {
        let mut turn = CpuTurn::default();
        let mut state = State::default();
        for _ in 0..256 {
            turn.advance();
            world.pump_inventory_archive(&mut turn);
            if state.read(world).is_some_and(|r| r.status() == VisibilityStatus::Ready) {
                return;
            }
        }
        panic!("native ordinary expanded-folder projection did not settle");
    }
    #[test]
    fn workspace_expanded_actual_archive_mask_and_fetched_ancestry_gate_capture_and_restore() {
        let mut world = world();
        let mut policy = State::default();
        let visible = ExpandedFolders::try_from(vec![id(4)]).unwrap();
        {
            let read = policy.read(&world).unwrap();
            assert_eq!(read.status(), VisibilityStatus::Ready);
            let captured = capture(&world.inventory, &[id(2), id(3), id(4), id(6), id(8)], read.mask()).unwrap();
            assert_eq!(captured, visible);
            assert!(restorable(&world.inventory, &visible, read.mask()));
            for denied in [id(2), id(3), id(6), id(8), id(900)] {
                assert!(!restorable(
                    &world.inventory,
                    &ExpandedFolders::try_from(vec![denied]).unwrap(),
                    read.mask()
                ));
            }
            assert!(capture(&world.inventory, &vec![id(4); MAX_EXPANDED + 1], read.mask()).is_err());
        }
        world.inventory.folders.get_mut(&id(1)).unwrap().state = FetchState::Unknown;
        world.inventory.generation += 1;
        pump(&mut world);
        let read = policy.read(&world).unwrap();
        assert!(capture(&world.inventory, &[id(4)], read.mask()).is_err());
        assert!(
            !restorable(&world.inventory, &visible, read.mask()),
            "no expansion may initiate an ancestor fetch"
        );
    }
}
