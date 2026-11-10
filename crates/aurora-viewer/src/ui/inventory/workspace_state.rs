//! Bounded native plain-tree expansion, Folderstorm fsworkspacelayout.h.
//! Caller owns native/source admission. No hidden windows or fetch calls.
use super::{FetchState, Inventory, InventoryUi, folder_jump};
use crate::workspace::model::{ExpandedFolders, MAX_EXPANDED};
use crate::world::inventory_archive::visibility::MaskRead;
use uuid::Uuid;

/// A folder and every ancestor must already be fetched. A freshly expanded
/// unknown branch would otherwise make PlainTree's normal lazy loader fetch it.
pub(crate) fn fetched_path(inv: &Inventory, id: Uuid, mask: MaskRead<'_>) -> bool {
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
    fn workspace_plain_owner(&self) -> bool {
        self.scope.is_nil()
            && self.tab == 0
            && !self.result_view()
            && !self.aurora.is_some_and(|frame| frame.explorer)
            && self.rename.is_none()
            && self.dialog.is_none()
            && self.progress.is_none()
            && self.expand_all.is_none()
    }
    fn workspace_plain(&self) -> bool {
        self.is_main() && self.workspace_plain_owner()
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
        self.workspace_apply_expanded(wanted, current);
        true
    }
    fn workspace_apply_expanded(&mut self, wanted: &ExpandedFolders, current: &ExpandedFolders) {
        for id in current.refs() {
            self.plain.set_open(*id, wanted.refs().contains(id));
        }
        self.plain.open_all(wanted.refs().iter().copied());
        // Stop a preceding reveal from reopening folders excluded explicitly by
        // this snapshot. Selection/edit/search data and keyboard focus stay put.
        self.reveal_path.retain(|id| wanted.refs().contains(id));
    }
}

impl InventoryUi {
    // The first created full owner is the workspace role. Never choose an
    // arbitrary surviving window when that original owner disappears.
    fn workspace_additional_unambiguous(&self) -> bool {
        let Some(owner) = self.workspace_owner else { return true };
        let mut windows = self.windows.iter().filter(|w| w.id == owner);
        match (windows.next(), windows.next()) {
            (None, None) => true,
            (Some(w), None) => w.additional && w.root.is_nil() && w.state.ui_id == owner && w.state.full_inventory,
            _ => false,
        }
    }
    /// Called only at a new capture/restore boundary, never by a delayed
    /// continuation. Prefer the original visible role, otherwise the earliest
    /// genuine open full owner. Pending operations retain their original UUID.
    pub(crate) fn workspace_select_open_owner(&mut self) -> bool {
        if !self.workspace_additional_unambiguous() {
            return false;
        }
        if self
            .workspace_additional_owner()
            .is_some_and(|owner| self.windows.iter().any(|w| w.id == owner && w.open))
        {
            return true;
        }
        if let Some(owner) = self
            .windows
            .iter()
            .find(|w| w.additional && w.open && w.root.is_nil() && w.state.ui_id == w.id && w.state.full_inventory)
            .map(|w| w.id)
        {
            // A duplicated UUID is not a selectable owner, even if only one
            // copy otherwise appears well formed.
            if self.windows.iter().filter(|w| w.id == owner).count() != 1 {
                return false;
            }
            self.workspace_owner = Some(owner);
        }
        true
    }

    pub(crate) fn workspace_additional_owner(&self) -> Option<Uuid> {
        self.workspace_additional_unambiguous()
            .then_some(self.workspace_owner)
            .flatten()
            .filter(|owner| self.windows.iter().any(|w| w.id == *owner))
    }
    pub(crate) fn workspace_additional_name(&self) -> Option<String> {
        self.workspace_additional_owner().map(super::floater_id)
    }
    pub(crate) fn workspace_additional_state(&self) -> Option<&InventoryUi> {
        let owner = self.workspace_additional_owner()?;
        self.windows.iter().find(|w| w.id == owner).map(|w| w.state.as_ref())
    }
    pub(crate) fn workspace_additional_state_mut(&mut self) -> Option<&mut InventoryUi> {
        let owner = self.workspace_additional_owner()?;
        self.windows.iter_mut().find(|w| w.id == owner).map(|w| w.state.as_mut())
    }
    pub(crate) fn workspace_additional_open(&self) -> Option<bool> {
        self.workspace_additional_unambiguous().then(|| {
            self.workspace_additional_owner()
                .is_some_and(|owner| self.windows.iter().any(|w| w.id == owner && w.open))
        })
    }
    fn workspace_dependencies(&self) -> bool {
        self.rename.is_some()
            || self.preview.is_some()
            || self.dialog.is_some()
            || self.move_to.is_some()
            || self.thumbnail.item.is_some()
            || self.bulk_permissions.active()
            || self.share_picker.open
            || self.property_edit.is_some()
            || self.attachment_preview.is_some()
            || self.save_pending
            || self.content_save.is_some()
            || self.progress.is_some()
            || self.pending.is_some()
            || self.filters_open
            || self.preferences_open
            || self.default_permissions_open
            || self.folder_color.is_some()
            || !self.windows.is_empty()
    }
    /// Caller independently checks actual source/native/RLVa permission. This
    /// wrapper preserves dependent state instead of silently destroying it.
    pub(crate) fn workspace_additional_visibility(&mut self, ctx: &egui::Context, visible: bool, minimized: bool) -> bool {
        if !self.workspace_additional_unambiguous() || self.workspace_additional_state().is_some_and(Self::workspace_dependencies) {
            return false;
        }
        if visible {
            if let Some(owner) = self.workspace_additional_owner() {
                let Some(window) = self.windows.iter_mut().find(|w| w.id == owner) else {
                    return false;
                };
                crate::ui::widgets::open_floater(ctx, &super::floater_id(owner), &mut window.open);
            } else {
                self.open_additional_window(ctx);
            }
            let Some(name) = self.workspace_additional_name() else {
                return false;
            };
            if minimized {
                ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(&name).with(crate::ui::widgets::EXPAND)));
                crate::ui::widgets::minimize_floater(ctx, &name);
            } else if crate::ui::widgets::floater_minimized(ctx, &name) {
                crate::ui::widgets::expand_floater(ctx, &name);
            }
        } else if let Some(owner) = self.workspace_additional_owner()
            && let Some(window) = self.windows.iter_mut().find(|w| w.id == owner)
        {
            window.open = false;
        }
        true
    }
    pub(crate) fn workspace_additional_expanded_refs(&self) -> Option<Vec<Uuid>> {
        let state = self.workspace_additional_state()?;
        state.workspace_plain_owner().then(|| state.plain.workspace_open())
    }
    pub(crate) fn workspace_restore_additional_expanded(&mut self, wanted: &ExpandedFolders, current: &ExpandedFolders) -> bool {
        let Some(state) = self.workspace_additional_state_mut() else {
            return false;
        };
        if !state.workspace_plain_owner() {
            return false;
        }
        state.workspace_apply_expanded(wanted, current);
        true
    }
}

impl InventoryUi {
    pub(crate) fn workspace_additional_idle(&self) -> bool {
        self.workspace_additional_state()
            .is_some_and(|state| !state.workspace_dependencies())
    }
    pub(crate) fn workspace_additional_untouched(&self) -> bool {
        self.workspace_additional_state()
            .is_some_and(|state| !state.workspace_dependencies() && state.selected.is_none() && state.search.is_empty() && state.tab == 0)
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
    #[test]
    fn workspace_additional_window_uses_one_native_role_and_never_drops_dependents_or_partial_windows() {
        let ctx = egui::Context::default();
        let mut st = InventoryUi::default();
        st.open_folder_window(id(2));
        assert!(st.workspace_additional_visibility(&ctx, true, false));
        assert_eq!(st.windows.len(), 2);
        assert_eq!(st.workspace_additional_open(), Some(true));
        st.workspace_additional_state_mut().unwrap().preview = Some(id(50));
        assert!(!st.workspace_additional_visibility(&ctx, false, false));
        assert!(st.workspace_additional_open().unwrap());
        st.workspace_additional_state_mut().unwrap().preview = None;
        assert!(st.workspace_additional_visibility(&ctx, false, false));
        assert!(st.windows.iter().find(|w| !w.additional).unwrap().open);
        st.windows.iter_mut().find(|w| w.additional).unwrap().state.ui_id = id(900);
        assert!(st.workspace_additional_open().is_none());
        assert!(!st.workspace_additional_visibility(&ctx, true, false));
        assert_eq!(st.windows.len(), 2, "never create another owner as an ambiguity fallback");
    }
    #[test]
    fn workspace_additional_role_coexists_with_fresh_full_windows_without_retargeting_or_closing_them() {
        let ctx = egui::Context::default();
        let mut st = InventoryUi::default();
        for _ in 0..4 {
            st.open_additional_window(&ctx);
        }
        let original = st.windows[0].id;
        let ordinary: Vec<_> = st.windows.iter().skip(1).map(|w| w.id).collect();
        assert_eq!(st.workspace_additional_owner(), Some(original));
        assert_eq!(st.workspace_additional_open(), Some(true), "capture sees the first ordinary extra");
        st.workspace_additional_state_mut().unwrap().search = "workspace only".into();
        let other = &mut st.windows[2];
        other.state.search = "other".into();
        other.state.selected = Some(id(901));
        other.state.preview = Some(id(902));
        let other_id = other.id;
        crate::ui::widgets::minimize_floater(&ctx, &super::super::floater_id(other_id));
        assert!(st.workspace_additional_visibility(&ctx, false, false));
        for id in &ordinary {
            assert!(st.windows.iter().find(|w| w.id == *id).unwrap().open);
        }
        assert!(st.workspace_additional_visibility(&ctx, true, true));
        assert!(st.workspace_additional_visibility(&ctx, true, false));
        assert_eq!(st.workspace_additional_owner(), Some(original));
        let other = st.windows.iter().find(|w| w.id == other_id).unwrap();
        assert_eq!(other.state.search, "other");
        assert_eq!(other.state.selected, Some(id(901)));
        assert_eq!(other.state.preview, Some(id(902)));
        assert!(crate::ui::widgets::floater_minimized(&ctx, &super::super::floater_id(other_id)));
        st.windows.retain(|w| w.id != original);
        assert_eq!(st.workspace_additional_owner(), None, "never substitute a surviving full owner");
        assert_eq!(st.workspace_additional_open(), Some(false));
        assert!(st.workspace_additional_visibility(&ctx, true, false));
        assert_ne!(st.workspace_additional_owner(), Some(original));
        assert!(
            !ordinary.contains(&st.workspace_additional_owner().unwrap()),
            "new restore uses a fresh owner"
        );
        st.close_additional_windows();
        assert!(st.windows.iter().all(|w| !w.open));
    }

    #[test]
    fn workspace_explicit_capture_reselects_first_open_full_owner_but_preserves_retained_closed_documents() {
        let ctx = egui::Context::default();
        let mut st = InventoryUi::default();
        for _ in 0..3 {
            st.open_additional_window(&ctx);
        }
        let original = st.windows[0].id;
        let next = st.windows[1].id;
        st.windows[0].state.preview = Some(id(920));
        st.windows[0].open = false;
        assert_eq!(
            st.workspace_additional_owner(),
            Some(original),
            "queries do not retarget a continuation"
        );
        assert!(st.workspace_select_open_owner());
        assert_eq!(st.workspace_additional_owner(), Some(next));
        assert_eq!(st.windows[0].state.preview, Some(id(920)));
        assert!(!st.windows[0].open);
        st.windows[1].state.ui_id = id(921);
        assert!(
            !st.workspace_select_open_owner(),
            "a malformed bound owner cannot be replaced silently"
        );
        assert_eq!(st.windows.len(), 3);
    }

    #[test]
    fn workspace_additional_expansion_requires_its_original_painted_owner_and_keeps_main_state() {
        let ctx = egui::Context::default();
        let mut st = InventoryUi::default();
        st.plain.open_all([id(1), id(99)]);
        assert!(st.workspace_additional_visibility(&ctx, true, false));
        assert!(
            st.workspace_additional_expanded_refs() == Some(Vec::new()),
            "new explicit full owner starts empty without borrowing main state"
        );
        let state = st.workspace_additional_state_mut().unwrap();
        // The runtime constructor already captured the exact full owner.
        state.plain.open_all([id(1), id(2)]);
        let wanted = ExpandedFolders::try_from(vec![id(1), id(3)]).unwrap();
        let current = ExpandedFolders::try_from(vec![id(1), id(2)]).unwrap();
        assert!(st.workspace_restore_additional_expanded(&wanted, &current));
        assert!(st.plain.is_open(id(99)) && !st.plain.is_open(id(3)));
        let state = st.workspace_additional_state().unwrap();
        assert!(state.plain.is_open(id(3)) && !state.plain.is_open(id(2)));
    }
    #[test]
    fn workspace_additional_minimized_state_survives_native_next_paint_without_latent_close() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let mut st = InventoryUi::default();
        let paint = |st: &mut InventoryUi| {
            let name = st.workspace_additional_name().unwrap();
            let mut open = st.workspace_additional_open().unwrap();
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
                    ..Default::default()
                },
                |ui| {
                    crate::ui::widgets::Floater::new(&name, "Inventory", egui::pos2(50.0, 60.0), egui::vec2(320.0, 240.0))
                        .silent()
                        .show(ui.ctx(), &p, &mut open, |ui| {
                            ui.label("Owned native role");
                        });
                },
            );
            out.textures_delta.clear();
            st.windows.iter_mut().find(|w| w.additional).unwrap().open = open;
        };
        assert!(st.workspace_additional_visibility(&ctx, true, true));
        paint(&mut st);
        paint(&mut st);
        assert!(crate::ui::widgets::floater_minimized(
            &ctx,
            &st.workspace_additional_name().unwrap()
        ));
        assert!(st.workspace_additional_visibility(&ctx, false, false));
        assert!(st.workspace_additional_visibility(&ctx, true, false));
        paint(&mut st);
        paint(&mut st);
        assert!(st.workspace_additional_open().unwrap());
        assert!(!crate::ui::widgets::floater_minimized(
            &ctx,
            &st.workspace_additional_name().unwrap()
        ));
    }
}
