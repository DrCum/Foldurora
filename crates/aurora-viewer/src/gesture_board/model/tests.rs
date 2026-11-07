use super::*;
fn grid(n: u8) -> GridKey {
    GridKey::from_bytes([n; 32]).unwrap()
}
fn board(n: u128) -> BoardId {
    BoardId::try_from(Uuid::from_u128(n)).unwrap()
}
fn tile(n: u128) -> Tile {
    Tile {
        id: TileId::try_from(Uuid::from_u128(n)).unwrap(),
        item: Reference {
            reference: Uuid::from_u128(n + 100_000),
            kind: Kind::Gesture,
        },
    }
}
fn empty() -> Catalogue {
    Catalogue::empty(Uuid::from_u128(1), grid(1)).unwrap()
}
#[test]
fn board_model_explicit_edits_roundtrip_exact_ids_and_only_changed_board_revision() {
    let initial = empty();
    let id = board(1);
    let other = board(2);
    let item = tile(3);
    let created = initial.apply(Change::Create { id, name: "First".into() }).unwrap();
    assert!(initial.boards.is_empty());
    assert_eq!(created.revision, 1);
    let two = created
        .apply(Change::Create {
            id: other,
            name: "Second".into(),
        })
        .unwrap();
    let added = two.apply(Change::Add { board: id, tile: item }).unwrap();
    assert_eq!(added.board(other).unwrap().revision, 1);
    assert_eq!(added.board(id).unwrap().revision, 2);
    let renamed = added
        .apply(Change::Rename {
            board: id,
            name: "Renamed".into(),
        })
        .unwrap();
    assert_eq!(renamed.tile(id, item.id), Some(&item));
    assert_eq!(renamed.board(id).unwrap().id, id);
    let bytes = renamed.encode().unwrap();
    let loaded = Catalogue::decode(&bytes, initial.account, initial.grid).unwrap();
    assert_eq!(loaded, renamed);
    let removed = loaded.apply(Change::Remove { board: id, tile: item.id }).unwrap();
    assert!(removed.tile(id, item.id).is_none());
    let deleted = removed.apply(Change::Delete { board: other }).unwrap();
    assert!(deleted.board(other).is_none());
    assert_eq!(deleted.boards.len(), 1);
    assert!(BoardId::new() != BoardId::new());
    assert!(TileId::new() != TileId::new());
}
#[test]
fn board_model_maximum_sixteen_by_1024_escaped_records_fit_closed_four_mebibyte_file() {
    let mut data = empty();
    for b in 0..MAX_BOARDS {
        let tiles = (0..MAX_TILES).map(|t| tile(((b as u128 + 1) << 64) + t as u128 + 1)).collect();
        data.boards.push(Board {
            id: board(b as u128 + 1),
            name: format!("{}{}", "\"".repeat(62), b),
            revision: 1,
            tiles,
        });
    }
    let bytes = data.encode().unwrap();
    assert!(bytes.len() <= MAX_FILE);
    assert_eq!(Catalogue::decode(&bytes, data.account, data.grid).unwrap(), data);
    assert_eq!(
        data.apply(Change::Create {
            id: board(999),
            name: "extra".into()
        }),
        Err(Error::Limit)
    );
    assert_eq!(
        data.apply(Change::Add {
            board: board(1),
            tile: tile(888)
        }),
        Err(Error::Limit)
    );
    let mut over = serde_json::to_value(&data).unwrap();
    over["boards"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(&data.boards[0]).unwrap());
    assert!(
        serde_json::from_value::<Catalogue>(over).is_err(),
        "visitor rejects before growing a seventeenth board"
    );
    let mut over = serde_json::to_value(&data).unwrap();
    over["boards"][0]["tiles"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(tile(888)).unwrap());
    assert!(
        serde_json::from_value::<Catalogue>(over).is_err(),
        "visitor rejects before growing tile1025"
    );
    assert_eq!(
        Catalogue::decode(&vec![b' '; MAX_FILE + 1], data.account, data.grid),
        Err(Error::Limit)
    );
}
#[test]
fn board_model_unknown_fields_kinds_versions_and_raw_asset_side_channels_fail_closed() {
    let base = empty()
        .apply(Change::Create {
            id: board(1),
            name: "Board".into(),
        })
        .unwrap()
        .apply(Change::Add {
            board: board(1),
            tile: tile(2),
        })
        .unwrap();
    let value = serde_json::to_value(&base).unwrap();
    for depth in 0..4 {
        let mut unknown = value.clone();
        let at = match depth {
            0 => &mut unknown,
            1 => &mut unknown["boards"][0],
            2 => &mut unknown["boards"][0]["tiles"][0],
            _ => &mut unknown["boards"][0]["tiles"][0]["item"],
        };
        at["asset"] = serde_json::json!(Uuid::from_u128(9));
        let bytes = serde_json::to_vec(&unknown).unwrap();
        assert_eq!(Catalogue::decode(&bytes, base.account, base.grid), Err(Error::Invalid));
    }
    let mut unknown = value.clone();
    unknown["version"] = serde_json::json!(999);
    assert_eq!(
        Catalogue::decode(&serde_json::to_vec(&unknown).unwrap(), base.account, base.grid),
        Err(Error::Unsupported)
    );
    let mut unknown = value;
    unknown["boards"][0]["tiles"][0]["item"]["kind"] = serde_json::json!("raw_sound_file");
    assert_eq!(
        Catalogue::decode(&serde_json::to_vec(&unknown).unwrap(), base.account, base.grid),
        Err(Error::Invalid)
    );
    assert_eq!(
        base.tile(board(1), tile(2).id),
        Some(&tile(2)),
        "failed proposals never edit the original"
    );
}
#[test]
fn board_model_identity_names_duplicates_and_nonzero_ids_are_checked_without_fallback() {
    assert!(Catalogue::empty(Uuid::nil(), grid(1)).is_err());
    assert!(BoardId::try_from(Uuid::nil()).is_err());
    assert!(TileId::try_from(Uuid::nil()).is_err());
    for name in ["", " ", " x", "x ", "x\ny", &"x".repeat(65), &"界".repeat(33)] {
        assert!(!valid_name(name));
    }
    assert!(valid_name("界面"));
    let data = empty()
        .apply(Change::Create {
            id: board(1),
            name: "Example".into(),
        })
        .unwrap();
    assert_eq!(
        data.apply(Change::Create {
            id: board(2),
            name: "example".into()
        }),
        Err(Error::Duplicate)
    );
    assert_eq!(
        data.apply(Change::Create {
            id: board(1),
            name: "other".into()
        }),
        Err(Error::Duplicate)
    );
    let bytes = data.encode().unwrap();
    assert_eq!(Catalogue::decode(&bytes, Uuid::from_u128(2), data.grid), Err(Error::Identity));
    assert_eq!(Catalogue::decode(&bytes, data.account, grid(2)), Err(Error::Identity));
    assert_eq!(
        data.apply(Change::Rename {
            board: board(999),
            name: "other".into()
        }),
        Err(Error::Missing)
    );
    let with = data
        .apply(Change::Add {
            board: board(1),
            tile: tile(5),
        })
        .unwrap();
    assert_eq!(
        with.apply(Change::Add {
            board: board(1),
            tile: Tile {
                id: tile(6).id,
                item: tile(5).item
            }
        }),
        Err(Error::Duplicate)
    );
    let two = with
        .apply(Change::Create {
            id: board(2),
            name: "other".into(),
        })
        .unwrap();
    assert_eq!(
        two.apply(Change::Add {
            board: board(2),
            tile: tile(5)
        }),
        Err(Error::Duplicate)
    );
    assert!(
        two.apply(Change::Add {
            board: board(2),
            tile: Tile {
                id: tile(6).id,
                item: tile(5).item
            }
        })
        .is_ok(),
        "separate boards may refer to one shared native gesture"
    );
}
#[test]
fn board_model_revision_exhaustion_preserves_all_original_records_and_never_wraps() {
    let mut data = empty()
        .apply(Change::Create {
            id: board(1),
            name: "First".into(),
        })
        .unwrap();
    data.revision = u64::MAX;
    assert_eq!(
        data.apply(Change::Rename {
            board: board(1),
            name: "next".into()
        }),
        Err(Error::Exhausted)
    );
    assert_eq!(data.board(board(1)).unwrap().name, "First");
    data.revision = 1;
    data.boards[0].revision = u64::MAX;
    assert_eq!(
        data.apply(Change::Add {
            board: board(1),
            tile: tile(2)
        }),
        Err(Error::Exhausted)
    );
    assert!(data.boards[0].tiles.is_empty());
}
