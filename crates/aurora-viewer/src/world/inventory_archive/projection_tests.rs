//! Exercises the actual projection with only capacity/work/hash injection.
use super::*;
use crate::world::inventory_archive::visibility::{CpuTurn, OwnerPolicy, Projection, VisibilityStatus, testing};

fn loaded() -> World {
    let mut world = world();
    world.inventory.folders.get_mut(&id(8)).unwrap().library = true;
    for (parent, children) in [(1, vec![2, 4, 5]), (2, vec![3]), (5, vec![6]), (7, vec![8])] {
        world.inventory.folders.get_mut(&id(parent)).unwrap().children = children.into_iter().map(id).collect();
    }
    world
}
fn masked(world: &World) -> Model {
    let mut model = ready(world);
    save(&mut model, world, 2, true);
    model
}
fn finish(projection: &mut Projection, world: &World, model: &Model, turn: &mut CpuTurn) {
    for _ in 0..100_000 {
        assert!(turn.advance());
        projection.pump(world, model, turn);
        if matches!(
            projection.read(world, model, &mut OwnerPolicy::default()).status(),
            VisibilityStatus::Ready | VisibilityStatus::Limited
        ) {
            return;
        }
    }
    panic!("projection failed to reach a bounded stop");
}

#[test]
fn inventory_archive_projection_initialization_and_probe_cursors_resume_with_one_shared_turn() {
    let world = loaded();
    let model = masked(&world);
    let mut projection = testing::projection(32, 16, 16, MAX_DEPTH, true);
    let mut turn = CpuTurn::default();
    testing::limits(&mut turn, 2, 2, 1);
    for _ in 0..100 {
        assert!(turn.advance());
        projection.pump(&world, &model, &mut turn);
        if testing::initialized(&projection) > 0 {
            break;
        }
    }
    assert_eq!(testing::initialized(&projection), 1);
    // Another owner/framework cannot restart initialization or claim a budget.
    for _ in 0..20 {
        projection.pump(&world, &model, &mut turn);
    }
    assert_eq!(testing::initialized(&projection), 1);
    assert_eq!(testing::used(&turn).probes, 1);
    let mut total = (0, 0, 1);
    for _ in 0..200 {
        turn.advance();
        projection.pump(&world, &model, &mut turn);
        let used = testing::used(&turn);
        assert!(used.nodes <= 2 && used.edges <= 2 && used.probes <= 1);
        total.0 += used.nodes;
        total.1 += used.edges;
        total.2 += used.probes;
        if projection.read(&world, &model, &mut OwnerPolicy::default()).status() == VisibilityStatus::Ready {
            break;
        }
    }
    assert_eq!(
        total,
        (8, 6, 32 + 36),
        "one insert per node, one read per edge, resumed collision probes"
    );
    assert_eq!(
        projection.read(&world, &model, &mut OwnerPolicy::default()).status(),
        VisibilityStatus::Ready
    );
}

#[test]
fn inventory_archive_projection_actual_ancestry_library_trash_links_and_restore() {
    let mut world = loaded();
    let model = masked(&world);
    world.inventory.items.insert(
        id(10),
        InvItem {
            id: id(10),
            parent: id(4),
            asset_type: 24,
            asset_id: id(9),
            ..Default::default()
        },
    );
    let mut projection = testing::projection(32, 16, 16, MAX_DEPTH, false);
    let mut turn = CpuTurn::default();
    finish(&mut projection, &world, &model, &mut turn);
    let mut owner = OwnerPolicy::default();
    let read = projection.read(&world, &model, &mut owner);
    for n in [2, 3, 9, 999] {
        assert!(!read.passes(id(n)));
    }
    for n in [1, 4, 5, 6, 7, 8, 10] {
        assert!(read.passes(id(n)));
    }
    world.inventory.folders.get_mut(&id(1)).unwrap().children.retain(|v| *v != id(2));
    world.inventory.folders.get_mut(&id(5)).unwrap().children.push(id(2));
    world.inventory.folders.get_mut(&id(2)).unwrap().info.parent = id(5);
    world.inventory.generation += 1;
    assert!(
        !projection.read(&world, &model, &mut owner).passes(id(2)),
        "old allocation cannot answer changed ancestry"
    );
    finish(&mut projection, &world, &model, &mut turn);
    assert!(
        projection.read(&world, &model, &mut owner).passes(id(3)),
        "Trash recovery inherits through marked branch"
    );
    world.inventory.folders.get_mut(&id(5)).unwrap().children.retain(|v| *v != id(2));
    world.inventory.folders.get_mut(&id(1)).unwrap().children.push(id(2));
    world.inventory.folders.get_mut(&id(2)).unwrap().info.parent = id(1);
    world.inventory.generation += 1;
    finish(&mut projection, &world, &model, &mut turn);
    assert!(!projection.read(&world, &model, &mut owner).passes(id(3)));
    assert!(model.read(&world).unwrap().marked(id(2)));
    assert!(world.inventory.queue.is_empty());
}

#[test]
fn inventory_archive_projection_capacity_collision_depth_and_missing_stop_with_known_subset() {
    for mode in 0..4 {
        let mut world = loaded();
        if mode == 3 {
            world.inventory.folders.get_mut(&id(2)).unwrap().children.push(id(999));
        }
        let model = masked(&world);
        let mut projection = match mode {
            0 => testing::projection(32, 2, 16, MAX_DEPTH, true),
            1 => testing::projection(32, 16, 2, MAX_DEPTH, true),
            2 => testing::projection(32, 16, 16, 1, true),
            _ => testing::projection(32, 16, 16, MAX_DEPTH, true),
        };
        finish(&mut projection, &world, &model, &mut CpuTurn::default());
        let mut owner = OwnerPolicy::default();
        let read = projection.read(&world, &model, &mut owner);
        assert_eq!(read.status(), VisibilityStatus::Limited);
        assert!(read.passes(id(1)), "already proven personal root retained");
        for n in [2, 3, 4, 7, 8, 999] {
            assert!(!read.passes(id(n)), "hidden or unprocessed never admitted mode={mode} id={n}");
        }
        let mut turn = CpuTurn::default();
        for _ in 0..3 {
            turn.advance();
            projection.pump(&world, &model, &mut turn);
            let used = testing::used(&turn);
            assert_eq!(
                (used.nodes, used.edges, used.probes),
                (0, 0, 0),
                "Limited subset never rescans without a real key change"
            );
        }
    }
}

#[test]
fn inventory_archive_projection_duplicate_cycle_and_library_identity_corruption_fail_closed() {
    for mode in 0..5 {
        let mut world = loaded();
        match mode {
            0 => world.inventory.folders.get_mut(&id(2)).unwrap().children.push(id(2)),
            1 => world.inventory.folders.get_mut(&id(1)).unwrap().children = vec![4, 4].into_iter().map(id).collect(),
            2 => world.inventory.folders.get_mut(&id(3)).unwrap().info.parent = id(4),
            3 => {
                world.inventory.folders.get_mut(&id(7)).unwrap().library = false;
                world.inventory.folders.get_mut(&id(7)).unwrap().info.parent = id(2);
                world.inventory.folders.get_mut(&id(2)).unwrap().children.insert(0, id(7));
            }
            _ => world.inventory.folders.get_mut(&id(3)).unwrap().info.id = id(4),
        }
        let model = masked(&world);
        let mut projection = testing::projection(32, 16, 16, MAX_DEPTH, false);
        finish(&mut projection, &world, &model, &mut CpuTurn::default());
        let mut owner = OwnerPolicy::default();
        let read = projection.read(&world, &model, &mut owner);
        assert_eq!(read.status(), VisibilityStatus::Limited);
        assert!(!read.passes(id(7)));
        assert!(!read.passes(id(999)));
    }
}

#[test]
fn inventory_archive_projection_large_fanout_and_chain_visit_each_loaded_edge_once() {
    for chain in [false, true] {
        let mut world = loaded();
        world.inventory.folders.get_mut(&id(1)).unwrap().children = vec![id(2)];
        world.inventory.folders.get_mut(&id(2)).unwrap().children.clear();
        world.inventory.lib_root = Uuid::nil();
        for n in 20..220 {
            let parent = if chain && n > 20 { n - 1 } else { 2 };
            folder(&mut world, n, parent, -1, false);
            world.inventory.folders.get_mut(&id(parent)).unwrap().children.push(id(n));
        }
        let model = masked(&world);
        let mut projection = testing::projection(1024, 512, 64, MAX_DEPTH, false);
        let mut turn = CpuTurn::default();
        testing::limits(&mut turn, 7, 9, 11);
        let mut edges = 0;
        let mut nodes = 0;
        for _ in 0..10_000 {
            turn.advance();
            projection.pump(&world, &model, &mut turn);
            let used = testing::used(&turn);
            edges += used.edges;
            nodes += used.nodes;
            if projection.read(&world, &model, &mut OwnerPolicy::default()).status() == VisibilityStatus::Ready {
                break;
            }
        }
        assert_eq!((nodes, edges), (202, 201));
    }
}

#[test]
fn inventory_archive_projection_two_physical_snapshots_block_reset_allocation_until_actual_drop() {
    let mut world = loaded();
    let mut model = masked(&world);
    let mut projection = testing::projection(32, 16, 16, MAX_DEPTH, false);
    finish(&mut projection, &world, &model, &mut CpuTurn::default());
    let first = testing::retained(&projection).unwrap();
    let first_clone = first.clone();
    world.inventory.generation += 1;
    finish(&mut projection, &world, &model, &mut CpuTurn::default());
    let second = testing::retained(&projection).unwrap();
    assert_eq!(testing::bytes(&projection), 4 * 1024 * 1024);
    model.retire();
    let new_model = masked(&world);
    let mut replacement = testing::replacement(&projection);
    drop(projection);
    let mut turn = CpuTurn::default();
    turn.advance();
    replacement.pump(&world, &new_model, &mut turn);
    assert_eq!(testing::initialized(&replacement), 0);
    assert_eq!(
        replacement.read(&world, &new_model, &mut OwnerPolicy::default()).status(),
        VisibilityStatus::Loading
    );
    drop(first);
    turn.advance();
    replacement.pump(&world, &new_model, &mut turn);
    assert_eq!(
        testing::initialized(&replacement),
        0,
        "last Arc clone still owns the physical credit"
    );
    drop(first_clone);
    finish(&mut replacement, &world, &new_model, &mut turn);
    assert_eq!(testing::bytes(&replacement), 4 * 1024 * 1024);
    drop(second);
    assert_eq!(testing::bytes(&replacement), 2 * 1024 * 1024);
}

#[test]
fn inventory_archive_projection_mask_generation_privacy_and_model_changes_revoke_before_observe_or_paint() {
    let mut world = loaded();
    let mut model = masked(&world);
    let mut projection = testing::projection(32, 16, 16, MAX_DEPTH, false);
    let mut owner = OwnerPolicy::default();
    finish(&mut projection, &world, &model, &mut CpuTurn::default());
    let previous = projection.read(&world, &model, &mut owner).revision();
    save(&mut model, &world, 4, true);
    assert_eq!(projection.read(&world, &model, &mut owner).status(), VisibilityStatus::Loading);
    assert!(!projection.read(&world, &model, &mut owner).passes(id(4)));
    assert_ne!(owner.revision(), previous);
    finish(&mut projection, &world, &model, &mut CpuTurn::default());
    world.inventory.generation += 1;
    assert!(!projection.read(&world, &model, &mut owner).passes(id(1)));
    restriction(&mut world, "@showinv=n");
    assert_eq!(projection.read(&world, &model, &mut owner).status(), VisibilityStatus::Unavailable);
    restriction(&mut world, "@showinv=y");
    assert!(!projection.read(&world, &model, &mut owner).passes(id(1)));
    let replacement = masked(&world);
    assert!(!projection.read(&world, &replacement, &mut owner).passes(id(1)));
    world.inventory.owner = id(999);
    assert_eq!(
        projection.read(&world, &replacement, &mut owner).status(),
        VisibilityStatus::Unavailable
    );
}

#[test]
fn inventory_archive_projection_empty_known_and_explicit_include_bypass_without_privacy_or_source_bypass() {
    let mut world = loaded();
    let mut model = ready(&world);
    let projection = Projection::default();
    let mut owner = OwnerPolicy::default();
    assert!(
        projection.read(&world, &model, &mut owner).passes(id(3)),
        "known missing-file empty needs no allocation/observer"
    );
    save(&mut model, &world, 2, true);
    assert!(!projection.read(&world, &model, &mut owner).passes(id(3)));
    assert!(owner.set_include(&world, &model, true));
    assert!(projection.read(&world, &model, &mut owner).passes(id(3)));
    let mut other = OwnerPolicy::default();
    assert!(!projection.read(&world, &model, &mut other).passes(id(3)));
    restriction(&mut world, "@showinv=n");
    assert!(!projection.read(&world, &model, &mut owner).passes(id(3)));
    assert!(!owner.set_include(&world, &model, true));
    restriction(&mut world, "@showinv=y");
    assert!(
        projection.read(&world, &model, &mut owner).passes(id(3)),
        "owner choice survives privacy suspension under the same source"
    );
    let mut unknown = Model::new(&world, grid("https://archive.example.invalid/login")).unwrap();
    assert!(unknown.complete_load(unknown.load_ticket().unwrap(), Err(Error::Invalid)));
    assert!(
        !projection.read(&world, &unknown, &mut owner).passes(id(3)),
        "corrupt/unknown is never empty"
    );
    assert!(owner.set_include(&world, &unknown, true));
    assert!(
        projection.read(&world, &unknown, &mut owner).passes(id(3)),
        "explicit current-owner recovery needs no projection"
    );
    world.agent_id = id(999);
    assert!(!projection.read(&world, &unknown, &mut owner).passes(id(3)));
}

#[test]
fn inventory_archive_projection_owner_policy_checked_exhaustion_fails_closed() {
    let world = loaded();
    let model = ready(&world);
    let projection = Projection::default();
    let mut owner = OwnerPolicy::default();
    projection.read(&world, &model, &mut owner);
    testing::owner_exhaust(&mut owner);
    assert!(!owner.set_include(&world, &model, true));
    assert_eq!(owner.revision(), None);
    assert!(!projection.read(&world, &model, &mut owner).passes(id(3)));
}

#[test]
fn inventory_archive_projection_publication_advances_owner_revision_without_inventory_or_archive_change() {
    let world = loaded();
    let model = masked(&world);
    let mut projection = testing::projection(32, 16, 16, MAX_DEPTH, false);
    let mut owner = OwnerPolicy::default();
    let before = projection.read(&world, &model, &mut owner).revision();
    finish(&mut projection, &world, &model, &mut CpuTurn::default());
    let after = projection.read(&world, &model, &mut owner).revision();
    assert_ne!(before, after, "Loading caches cannot remain empty after publication");
    assert_eq!(
        projection.read(&world, &model, &mut owner).revision(),
        after,
        "read alone never churns revision"
    );
    let mut replacement = testing::replacement(&projection);
    finish(&mut replacement, &world, &model, &mut CpuTurn::default());
    assert_ne!(
        replacement.read(&world, &model, &mut owner).revision(),
        after,
        "new physical publication has a distinct stamp"
    );
}

#[test]
fn inventory_archive_projection_actual_depth_limit_has_bounded_stack_and_edge_work() {
    let mut world = loaded();
    world.inventory.lib_root = Uuid::nil();
    world.inventory.folders.get_mut(&id(1)).unwrap().children = vec![id(2)];
    world.inventory.folders.get_mut(&id(2)).unwrap().children.clear();
    for n in 20..280 {
        let parent = if n == 20 { 2 } else { n - 1 };
        folder(&mut world, n, parent, -1, false);
        world.inventory.folders.get_mut(&id(parent)).unwrap().children.push(id(n));
    }
    let model = masked(&world);
    let mut projection = testing::projection(2048, 1024, 64, MAX_DEPTH, false);
    let mut turn = CpuTurn::default();
    testing::limits(&mut turn, 3, 1, 1024);
    let mut total = (0, 0);
    for _ in 0..2000 {
        turn.advance();
        projection.pump(&world, &model, &mut turn);
        let used = testing::used(&turn);
        assert!(used.nodes <= 3 && used.edges <= 1 && used.probes <= 1024);
        total.0 += used.nodes;
        total.1 += used.edges;
        if projection.read(&world, &model, &mut OwnerPolicy::default()).status() == VisibilityStatus::Limited {
            break;
        }
    }
    assert_eq!(total, (MAX_DEPTH + 1, MAX_DEPTH + 1));
    assert!(!projection.read(&world, &model, &mut OwnerPolicy::default()).passes(id(279)));
}

#[test]
fn inventory_archive_projection_default_initialization_budget_and_canceled_builder_credit_are_physical() {
    let world = loaded();
    let mut model = masked(&world);
    let mut projection = Projection::default();
    let mut turn = CpuTurn::default();
    for _ in 0..100 {
        turn.advance();
        projection.pump(&world, &model, &mut turn);
        let used = testing::used(&turn);
        assert!(used.nodes <= 256 && used.edges <= 512 && used.probes <= 1024);
        if testing::initialized(&projection) > 0 {
            break;
        }
    }
    assert!(testing::initialized(&projection) > 0 && testing::initialized(&projection) <= 1024);
    assert_eq!(testing::bytes(&projection), 2 * 1024 * 1024);
    assert!(testing::payload(&projection).unwrap() <= 2 * 1024 * 1024);
    model.retire();
    projection.observe(&world, &model);
    assert_eq!(
        testing::bytes(&projection),
        0,
        "unpublished storage actually dropped before credit is available"
    );
    assert_eq!(
        projection.read(&world, &model, &mut OwnerPolicy::default()).status(),
        VisibilityStatus::Unavailable
    );
}

#[test]
fn inventory_archive_projection_qualified_read_reuses_one_borrowed_source_privacy_and_policy_proof() {
    let mut world = loaded();
    let mut model = masked(&world);
    let mut projection = testing::projection(32, 16, 16, MAX_DEPTH, false);
    finish(&mut projection, &world, &model, &mut CpuTurn::default());
    let mut owner = OwnerPolicy::default();
    {
        // World, Model and owner policy are all borrowed for the entire read:
        // none can be changed while consumers use this once-qualified proof.
        let read = projection.read(&world, &model, &mut owner);
        let revision = read.revision();
        for _ in 0..512 {
            assert!(read.passes(id(4)));
            assert!(!read.passes(id(3)));
            assert!(!read.passes(id(9)), "item uses its actual archived parent");
            assert!(!read.passes(id(999)));
            assert_eq!(read.revision(), revision);
        }
    }
    // End the immutable borrow, then change actual policy/source facts before
    // acquiring another read. No event observer is needed for stale denial.
    restriction(&mut world, "@showinv=n");
    assert!(!projection.read(&world, &model, &mut owner).passes(id(4)));
    restriction(&mut world, "@showinv=y");
    assert!(
        !projection.read(&world, &model, &mut owner).passes(id(4)),
        "privacy ABA invalidates the old table key"
    );
    finish(&mut projection, &world, &model, &mut CpuTurn::default());
    assert!(projection.read(&world, &model, &mut owner).passes(id(4)));
    assert!(owner.set_include(&world, &model, true));
    assert!(projection.read(&world, &model, &mut owner).passes(id(3)));
    model.retire();
    assert!(!projection.read(&world, &model, &mut owner).passes(id(3)));
    assert!(world.inventory.queue.is_empty());
}
