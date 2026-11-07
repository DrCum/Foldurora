use super::*;
use crate::world::inventory::FetchState;
use aurora_net::inventory::InvFolder;

const ALLOWED: Policy = Policy {
    revision: 1,
    allowed: true,
};

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn inventory() -> Inventory {
    let mut inventory = Inventory {
        owner: id(900),
        root: id(1),
        ..Default::default()
    };
    loaded(&mut inventory, vec![folder(1, 0, "Inventory", 8)]);
    inventory
}

fn folder(value: u128, parent: u128, name: &str, kind: i32) -> InvFolder {
    InvFolder {
        id: id(value),
        parent: id(parent),
        name: name.into(),
        type_default: kind,
        ..Default::default()
    }
}

// A real metadata update preserves Unknown content state and updates existing
// names/parents. created_folders is a creation receipt, not this producer.
fn loaded(inventory: &mut Inventory, folders: Vec<InvFolder>) {
    inventory.server_updated(folders, Vec::new());
}

fn source(inventory: &Inventory) -> Source {
    Source {
        agent: inventory.owner,
        owner: inventory.owner,
        session: id(901),
        root: inventory.root,
        profile: 1,
        window: Uuid::nil(),
    }
}

fn ready(model: &mut Model, inventory: &Inventory) {
    for _ in 0..100_000 {
        if !matches!(model.phase(), Phase::Indexing | Phase::Matching) {
            return;
        }
        assert!(model.advance_with(inventory, 17, || false) <= 17);
    }
    panic!("bounded synthetic index did not finish");
}

fn start(inventory: &Inventory) -> Model {
    let mut model = Model::default();
    model.sync(inventory, Some(source(inventory)), ALLOWED);
    ready(&mut model, inventory);
    model
}

fn rows(model: &Model) -> Vec<Uuid> {
    model.rows().map(|row| row.folder).collect()
}

#[test]
fn folder_jump_loaded_inventory_ranks_leaf_before_path_and_disambiguates_duplicate_names() {
    let mut inventory = inventory();
    loaded(
        &mut inventory,
        vec![
            folder(2, 1, "Blue", -1),
            folder(3, 1, "Other", -1),
            folder(4, 2, "Blue Coat", -1),
            folder(5, 2, "Coat", -1),
            folder(6, 3, "Coat", -1),
            folder(7, 1, "Coat storage", -1),
            folder(8, 1, "A coat", -1),
        ],
    );
    let mut model = start(&inventory);
    assert_eq!(
        rows(&model),
        [id(8), id(2), id(7), id(3)],
        "empty query is top-level only, sorted by folded leaf"
    );
    model.set_query("Coat");
    ready(&mut model, &inventory);
    assert_eq!(rows(&model), [id(5), id(6), id(7), id(8), id(4)]);
    let duplicate_paths: Vec<_> = model.rows().take(2).map(|row| (row.name, row.parent_path)).collect();
    assert_eq!(duplicate_paths, [("Coat", "Blue"), ("Coat", "Other")]);
    model.set_query("blue / coat");
    ready(&mut model, &inventory);
    assert_eq!(
        rows(&model),
        [id(4), id(5)],
        "leaf-token match outranks a token found only in parent path"
    );
    model.set_query("/\t /");
    ready(&mut model, &inventory);
    assert_eq!(
        rows(&model),
        [id(8), id(2), id(7), id(3)],
        "delimiter-only query has Legacy empty-token behavior"
    );
}

#[test]
fn folder_jump_huge_siblings_use_bounded_steps_and_keep_only_the_best_sixty_without_fetching() {
    let mut inventory = inventory();
    let folders = (2..=1001)
        .map(|value| folder(value, 1, &format!("Folder {:04}", 1002 - value), -1))
        .collect();
    loaded(&mut inventory, folders);
    inventory.queue.push((id(1), false));
    inventory.held = Some(vec![id(1)]);
    let states: Vec<_> = inventory.folders.iter().map(|(id, folder)| (*id, folder.state)).collect();
    let original_queue = inventory.queue.clone();
    let original_held = inventory.held.clone();
    let revision = inventory.generation;
    let mut model = Model::default();
    model.sync(&inventory, Some(source(&inventory)), ALLOWED);
    assert_eq!(model.advance_with(&inventory, 1, || false), 1);
    assert_eq!(model.nodes.len(), 2, "one child edge cannot consume its thousand siblings");
    assert_eq!(model.phase(), Phase::Indexing);
    assert_eq!(model.rows().len(), 0, "active rebuild is nonactionable");
    let mut clocks = 0;
    assert_eq!(
        model.advance_with(&inventory, 256, || {
            clocks += 1;
            clocks > 1
        }),
        32
    );
    ready(&mut model, &inventory);
    assert_eq!(model.rows().len(), 60);
    assert_eq!(model.matches(), 1000);
    assert!(!model.limited(), "display cap is distinct from a retained-index limit");
    let names: Vec<_> = model.rows().map(|row| row.name).collect();
    assert_eq!(names.first(), Some(&"Folder 0001"));
    assert_eq!(names.last(), Some(&"Folder 0060"));
    assert_eq!(inventory.queue, original_queue);
    assert_eq!(inventory.held, original_held);
    assert_eq!(inventory.generation, revision);
    for (id, state) in states {
        assert_eq!(inventory.folders[&id].state, state);
    }
    assert!(inventory.folders.values().any(|folder| folder.state == FetchState::Unknown));
}

#[test]
fn folder_jump_retained_subset_limit_finishes_matching_and_still_admits_a_fresh_exact_open() {
    let mut inventory = inventory();
    loaded(
        &mut inventory,
        vec![folder(2, 1, "Alpha", -1), folder(3, 1, "Beta", -1), folder(4, 1, "Gamma", -1)],
    );
    let current = source(&inventory);
    let mut model = Model {
        limits: Limits { nodes: 3, text: 1000 },
        ..Default::default()
    };
    model.sync(&inventory, Some(current), ALLOWED);
    ready(&mut model, &inventory);
    assert_eq!(model.phase(), Phase::Ready);
    assert!(model.limited());
    assert_eq!(model.matches(), 2, "count refers only to retained subset");
    assert_eq!(rows(&model), [id(2), id(3)]);
    let ticket = model.capture(id(3)).expect("fresh retained folder");
    assert!(model.validate(ticket, &inventory, Some(current), ALLOWED));
    assert_eq!(ticket.folder(), id(3));
    assert!(
        model.capture(id(4)).is_none(),
        "missing suffix cannot be inferred to rank after retained rows"
    );

    let mut text_limited = Model {
        limits: Limits { nodes: 20, text: 10 },
        ..Default::default()
    };
    text_limited.sync(&inventory, Some(current), ALLOWED);
    ready(&mut text_limited, &inventory);
    assert!(text_limited.limited());
    assert_eq!(rows(&text_limited), [id(2)]);
    assert!(text_limited.validate(
        text_limited.capture_selected().expect("retained Alpha"),
        &inventory,
        Some(current),
        ALLOWED
    ));
}

#[test]
fn folder_jump_parent_chain_rechecks_reject_trash_library_cycles_and_unversioned_metadata_replacement() {
    let mut inventory = inventory();
    loaded(
        &mut inventory,
        vec![
            folder(2, 1, "Clothes", -1),
            folder(3, 2, "Coat", -1),
            folder(4, 1, "Trash", 14),
            folder(5, 4, "Discarded coat", -1),
            folder(6, 1, "Library", -1),
            folder(7, 6, "Library coat", -1),
        ],
    );
    inventory.folders.get_mut(&id(6)).expect("library").library = true;
    // Corrupt loaded edge: the child's record disagrees with its listed parent.
    inventory.folders.get_mut(&id(1)).expect("root").children.push(id(3));
    let mut model = start(&inventory);
    model.set_query("coat");
    ready(&mut model, &inventory);
    assert_eq!(rows(&model), [id(3)]);
    let ticket = model.capture_selected().expect("Coat");
    assert!(model.validate(ticket, &inventory, Some(source(&inventory)), ALLOWED));
    let original = inventory.folders[&id(2)].clone();
    for parent in [id(4), id(6), id(3), id(999)] {
        inventory.folders.get_mut(&id(2)).expect("parent").info.parent = parent;
        assert!(
            !model.validate(ticket, &inventory, Some(source(&inventory)), ALLOWED),
            "final current chain rejects even a producer missing its generation bump"
        );
    }
    inventory.folders.insert(id(2), original);
    inventory.folders.get_mut(&id(3)).expect("folder").info.name = "Replacement sender label".into();
    assert!(!model.validate(ticket, &inventory, Some(source(&inventory)), ALLOWED));
    assert!(model.capture(inventory.root).is_none());
}

#[test]
fn folder_jump_rebuild_preserves_exact_selection_or_none_and_never_revives_held_targets() {
    let mut inventory = inventory();
    loaded(
        &mut inventory,
        vec![folder(2, 1, "Alpha", -1), folder(3, 1, "Beta", -1), folder(4, 1, "Gamma", -1)],
    );
    let current = source(&inventory);
    let mut model = start(&inventory);
    assert!(model.select(model.capture(id(3)).expect("Beta"), &inventory, Some(current), ALLOWED));
    let held = model.capture_selected().expect("selected Beta");
    let old_widget = model.rows().find(|row| row.folder == id(3)).expect("Beta row").widget;
    loaded(&mut inventory, vec![folder(2, 1, "Zulu", -1)]);
    model.sync(&inventory, Some(current), ALLOWED);
    assert!(!model.validate(held, &inventory, Some(current), ALLOWED));
    ready(&mut model, &inventory);
    assert_eq!(model.selected(), Some(id(3)));
    let new_widget = model.rows().find(|row| row.folder == id(3)).expect("Beta row").widget;
    assert_ne!(old_widget, new_widget);
    assert!(!model.validate(held, &inventory, Some(current), ALLOWED));
    let held_next = model.capture_navigation(false).expect("captured Gamma, not an index");
    assert_eq!(held_next.folder(), id(4));
    loaded(&mut inventory, vec![folder(4, 1, "Aardvark", -1)]);
    model.sync(&inventory, Some(current), ALLOWED);
    ready(&mut model, &inventory);
    assert!(!model.validate(held_next, &inventory, Some(current), ALLOWED));
    assert_eq!(model.selected(), Some(id(3)));
    inventory.folders.remove(&id(3));
    inventory.generation += 1;
    model.sync(&inventory, Some(current), ALLOWED);
    ready(&mut model, &inventory);
    assert_eq!(model.selected(), None, "disappeared selected folder does not choose another result");
    assert!(model.capture_selected().is_none());
    let explicit_down = model.capture_navigation(false).expect("fresh explicit navigation");
    assert_eq!(explicit_down.folder(), id(4));
}

#[test]
fn folder_jump_source_privacy_and_input_boundaries_purge_or_invalidate_without_aba_revival() {
    let mut inventory = inventory();
    loaded(&mut inventory, vec![folder(2, 1, "Private folder", -1)]);
    let current = source(&inventory);
    let mut model = start(&inventory);
    let held = model.capture_selected().expect("folder");
    model.cancel_input();
    assert!(!model.validate(held, &inventory, Some(current), ALLOWED));
    assert_eq!(model.rows().len(), 1);
    let held = model.capture_selected().expect("fresh folder");
    let denied = Policy {
        revision: 2,
        allowed: false,
    };
    model.sync(&inventory, Some(current), denied);
    assert_eq!(model.phase(), Phase::Unavailable);
    assert!(model.nodes.is_empty());
    assert!(model.seen.is_empty());
    assert!(model.rows().next().is_none());
    model.sync(&inventory, Some(current), ALLOWED);
    ready(&mut model, &inventory);
    assert!(!model.validate(held, &inventory, Some(current), ALLOWED));
    for replacement in [
        Source {
            session: id(902),
            ..current
        },
        Source { profile: 2, ..current },
        Source {
            window: id(903),
            ..current
        },
    ] {
        let held = model.capture_selected().expect("fresh folder");
        model.sync(&inventory, Some(replacement), ALLOWED);
        ready(&mut model, &inventory);
        model.sync(&inventory, Some(current), ALLOWED);
        ready(&mut model, &inventory);
        assert!(
            !model.validate(held, &inventory, Some(current), ALLOWED),
            "A→B→A source cannot revive an old press"
        );
    }
    model.sync(&inventory, Some(Source { owner: id(999), ..current }), ALLOWED);
    assert_eq!(model.phase(), Phase::Unavailable);
    model.sync(&inventory, Some(Source { profile: 0, ..current }), ALLOWED);
    assert_eq!(model.phase(), Phase::Unavailable);
}

#[test]
fn folder_jump_bounds_deep_and_oversize_loaded_metadata_without_recursive_or_unbounded_work() {
    let mut inventory = inventory();
    let mut folders = vec![
        folder(2, 1, &"x".repeat(MAX_NAME_BYTES + 1), -1),
        folder(3, 2, "Under oversize", -1),
    ];
    for depth in 1..=MAX_DEPTH + 2 {
        folders.push(folder(10 + depth as u128, if depth == 1 { 1 } else { 9 + depth as u128 }, "D", -1));
    }
    loaded(&mut inventory, folders);
    let mut model = start(&inventory);
    assert!(model.limited());
    assert!(!model.seen.contains_key(&id(2)));
    assert!(!model.seen.contains_key(&id(3)));
    assert!(
        model
            .nodes
            .iter()
            .all(|node| node.depth <= MAX_DEPTH && node.path.len() <= MAX_PATH_BYTES)
    );
    model.set_query("D");
    ready(&mut model, &inventory);
    assert_eq!(model.matches(), MAX_DEPTH);
    assert_eq!(model.rows().len(), MAX_RESULTS);
    model.set_query(&"q".repeat(MAX_QUERY_CHARS + 1));
    assert_eq!(model.phase(), Phase::InvalidQuery);
    assert_eq!(model.rows().len(), 0);
    assert!(model.capture_selected().is_none());
    model.set_query("D");
    ready(&mut model, &inventory);
    assert_eq!(model.phase(), Phase::Ready);
}

#[test]
fn folder_jump_exhaustion_and_exact_selection_changes_cannot_reuse_input_or_row_identity() {
    let mut inventory = inventory();
    loaded(&mut inventory, vec![folder(2, 1, "Alpha", -1), folder(3, 1, "Beta", -1)]);
    let current = source(&inventory);
    let mut model = start(&inventory);
    let held_open = model.capture_selected().expect("Alpha");
    assert!(model.select(model.capture(id(3)).expect("Beta"), &inventory, Some(current), ALLOWED));
    assert!(!model.validate(held_open, &inventory, Some(current), ALLOWED));
    model.input = u64::MAX;
    model.cancel_input();
    assert_eq!(model.phase(), Phase::Exhausted);
    assert!(model.nodes.is_empty());
    model.sync(&inventory, Some(current), ALLOWED);
    assert_eq!(model.phase(), Phase::Exhausted);

    let mut model = Model {
        serial: u64::MAX,
        ..Default::default()
    };
    model.sync(&inventory, Some(current), ALLOWED);
    model.advance(&inventory);
    assert_eq!(model.phase(), Phase::Exhausted);
    assert!(model.rows().next().is_none());
}
