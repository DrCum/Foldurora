//! FSGestureBoardController::resolve / Inventory sound Play and gesture loading
//! (fsgestureboardcontroller.cpp, llinventorybridge.cpp, abbe3cb0ca66ee2484f277219f6a27706fab7fc6).
//! Playback does not require copy/modify/transfer; asset IDs remain internal to
//! native playback, never serialized or exposed through the board's saved data.
use super::{Inventory, in_type, original, under};
use uuid::Uuid;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaybackKind {
    Gesture,
    Sound,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaybackError {
    InventoryNotReady,
    Missing,
}
/// Current inventory observation, not a session or native action capability.
/// Only this resolver can construct it; no asset UUID is accepted from a caller.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlaybackItem {
    reference: Uuid,
    item: Uuid,
    asset: Uuid,
    kind: PlaybackKind,
}
impl PlaybackItem {
    pub fn reference(self) -> Uuid {
        self.reference
    }
    pub fn item(self) -> Uuid {
        self.item
    }
    pub fn asset(self) -> Uuid {
        self.asset
    }
    pub fn kind(self) -> PlaybackKind {
        self.kind
    }
}
pub(crate) fn resolve_playback_item(inv: &Inventory, reference: Uuid, kind: PlaybackKind) -> Result<PlaybackItem, PlaybackError> {
    if inv.owner.is_nil() || inv.root.is_nil() || !inv.folders.contains_key(&inv.root) {
        return Err(PlaybackError::InventoryNotReady);
    }
    let original_item = inv.items.get(&reference).ok_or(PlaybackError::Missing)?;
    if original_item.asset_type == 25 {
        return Err(PlaybackError::Missing);
    }
    let item = original(inv, reference).ok_or(PlaybackError::Missing)?;
    let resolved = inv.items.get(&item).ok_or(PlaybackError::Missing)?;
    let expected = match kind {
        PlaybackKind::Gesture => 21,
        PlaybackKind::Sound => 1,
    };
    let known_root = |id| under(inv, id, inv.root) || (!inv.lib_root.is_nil() && under(inv, id, inv.lib_root));
    // Stricter than a bare cached UUID: both original and reference must still
    // belong to current personal/Library inventory, with known ancestry.
    if resolved.asset_type != expected
        || resolved.asset_id.is_nil()
        || !known_root(reference)
        || !known_root(item)
        || in_type(inv, reference, 14)
        || in_type(inv, item, 14)
    {
        return Err(PlaybackError::Missing);
    }
    Ok(PlaybackItem {
        reference,
        item,
        asset: resolved.asset_id,
        kind,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};
    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    pub(super) fn fixture() -> Inventory {
        let mut inv = Inventory {
            owner: id(1),
            root: id(2),
            lib_root: id(3),
            ..Default::default()
        };
        for (n, parent, kind, library) in [(2, 0, 8, false), (3, 0, 8, true), (4, 2, 14, false)] {
            inv.folders.insert(
                id(n),
                Folder {
                    info: InvFolder {
                        id: id(n),
                        parent: id(parent),
                        type_default: kind,
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: FetchState::Fetched,
                    library,
                },
            );
        }
        inv.items.insert(
            id(10),
            InvItem {
                id: id(10),
                parent: inv.root,
                owner: inv.owner,
                asset_id: id(100),
                asset_type: 21,
                ..Default::default()
            },
        );
        inv.items.insert(
            id(11),
            InvItem {
                id: id(11),
                parent: inv.root,
                owner: inv.owner,
                asset_id: id(101),
                asset_type: 1,
                ..Default::default()
            },
        );
        inv
    }
    #[test]
    fn board_playback_resolver_preserves_native_owner_group_library_and_restricted_permission_play_rules() {
        for mask in [
            super::super::FULL,
            0,
            super::super::COPY,
            super::super::MODIFY,
            super::super::TRANSFER,
        ] {
            for (owner, group_owned, parent) in [
                (id(1), false, id(2)),
                (id(99), true, id(2)),
                (id(99), false, id(2)),
                (id(99), false, id(3)),
            ] {
                let mut inv = fixture();
                for (item, kind) in [(id(10), PlaybackKind::Gesture), (id(11), PlaybackKind::Sound)] {
                    let value = inv.items.get_mut(&item).unwrap();
                    value.owner_mask = mask;
                    value.owner = owner;
                    value.group_owned = group_owned;
                    value.parent = parent;
                    let observed = resolve_playback_item(&inv, item, kind).unwrap();
                    assert_eq!(observed.reference(), item);
                    assert_eq!(observed.item(), item);
                    assert_eq!(observed.kind(), kind);
                    assert_eq!(observed.asset(), inv.items[&item].asset_id);
                }
            }
        }
    }
    #[test]
    fn board_playback_resolver_rechecks_links_type_asset_trash_and_unresolved_cycles() {
        let mut inv = fixture();
        inv.items.insert(
            id(20),
            InvItem {
                id: id(20),
                parent: inv.root,
                asset_id: id(10),
                asset_type: 24,
                ..Default::default()
            },
        );
        let first = resolve_playback_item(&inv, id(20), PlaybackKind::Gesture).unwrap();
        assert_eq!(first.item(), id(10));
        assert_eq!(first.reference(), id(20));
        inv.items.get_mut(&id(20)).unwrap().asset_id = id(11);
        assert!(resolve_playback_item(&inv, id(20), PlaybackKind::Gesture).is_err());
        assert_eq!(resolve_playback_item(&inv, id(20), PlaybackKind::Sound).unwrap().item(), id(11));
        inv.items.get_mut(&id(20)).unwrap().parent = id(4);
        assert!(resolve_playback_item(&inv, id(20), PlaybackKind::Sound).is_err());
        inv.items.get_mut(&id(20)).unwrap().parent = inv.root;
        inv.items.get_mut(&id(11)).unwrap().parent = id(4);
        assert!(resolve_playback_item(&inv, id(20), PlaybackKind::Sound).is_err());
        inv.items.get_mut(&id(20)).unwrap().asset_id = id(20);
        assert!(resolve_playback_item(&inv, id(20), PlaybackKind::Gesture).is_err());
        inv.items.get_mut(&id(10)).unwrap().asset_id = Uuid::nil();
        assert!(resolve_playback_item(&inv, id(10), PlaybackKind::Gesture).is_err());
    }
    #[test]
    fn board_playback_resolver_never_treats_raw_asset_or_foreign_unknown_ancestry_as_inventory_reference() {
        let mut inv = fixture();
        assert!(resolve_playback_item(&inv, id(100), PlaybackKind::Gesture).is_err());
        inv.items.get_mut(&id(10)).unwrap().parent = id(999);
        assert!(resolve_playback_item(&inv, id(10), PlaybackKind::Gesture).is_err());
        inv.owner = Uuid::nil();
        assert!(matches!(
            resolve_playback_item(&inv, id(11), PlaybackKind::Sound),
            Err(PlaybackError::InventoryNotReady)
        ));
    }
}
