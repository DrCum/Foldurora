//! Real native World/archive proof, no renderer, socket or private store IO.
use super::*;
use crate::world::{
    inventory::FetchState,
    inventory_archive::{controller::testing, visibility::CpuTurn},
};

fn pump_to(world: &mut World, transition: &str, expected: VisibilityStatus) {
    let mut turn = CpuTurn::default();
    let mut state = State::default();
    let mut last = None;
    for index in 0..256 {
        assert!(turn.advance(), "CPU turn exhausted at {transition}");
        world.pump_inventory_archive(&mut turn);
        last = state.read(world).map(|r| r.status());
        if last == Some(expected) {
            eprintln!(
                "workspace image transition={transition} status={expected:?} turns={} generation={}",
                index + 1,
                world.inventory.generation
            );
            return;
        }
        assert!(
            matches!(last, Some(VisibilityStatus::Loading)),
            "workspace image transition={transition} stopped at {last:?}, expected {expected:?}; generation={} controller={:?}",
            world.inventory.generation,
            world.inventory_archive.as_ref().and_then(|a| a.status())
        );
    }
    panic!(
        "workspace image transition={transition} did not settle in256 turns: {last:?}, expected {expected:?}; generation={} controller={:?}",
        world.inventory.generation,
        world.inventory_archive.as_ref().and_then(|a| a.status())
    );
}
fn pump(world: &mut World, transition: &str) {
    pump_to(world, transition, VisibilityStatus::Ready);
}
fn picture_world() -> World {
    let mut world = crate::world::inventory_archive::tests::native_fixture_world();
    world.rlva.configure(true);
    let id = |n| Uuid::from_u128(n);
    for (parent, children) in [(1, vec![2, 4, 5]), (2, vec![3]), (5, vec![6]), (7, vec![8])] {
        world.inventory.folders.get_mut(&id(parent)).unwrap().children = children.into_iter().map(id).collect();
    }
    // Native Library descendants carry Library provenance just like their root.
    world.inventory.folders.get_mut(&id(8)).unwrap().library = true;
    for folder in world.inventory.folders.values_mut() {
        folder.state = FetchState::Fetched;
    }
    let item = world.inventory.items.get_mut(&id(9)).unwrap();
    item.asset_type = 0;
    item.inv_type = 0;
    item.asset_id = id(501);
    item.owner_mask = actions::FULL;
    world.inventory.folders.get_mut(&id(2)).unwrap().items = vec![id(9)];
    world
}
#[test]
fn workspace_picture_policy_requalifies_exact_native_item_permissions_archive_source_and_privacy_without_fetches() {
    let mut world = picture_world();
    let id = |n| Uuid::from_u128(n);
    world.inventory_archive = Some(testing::native_loaded(&world, &[]));
    pump(&mut world, "initial ordinary metadata");
    let source = Source {
        agent: world.agent_id,
        login: world.login.as_ref().unwrap().session_id,
        grid: world.inventory_archive.as_ref().unwrap().native_grid(&world).unwrap().namespace(),
        hosted: None,
    };
    let mut policy = Policy::default();
    let reference = policy.choose(&world, &source, id(501)).unwrap();
    assert_eq!(reference.item, id(9));
    policy.refresh(&world, Some(&source), 1, std::slice::from_ref(&reference));
    assert!(policy.current(&world, Some(&source), 1));
    assert!(policy.allows(&reference));
    assert!(!policy.current(&world, Some(&source), 2));
    let mut wrong = source.clone();
    wrong.login = Uuid::new_v4();
    assert!(!policy.current(&world, Some(&wrong), 1));
    assert!(policy.choose(&world, &wrong, id(501)).is_none());
    wrong = source.clone();
    wrong.grid = "b".repeat(64);
    assert!(policy.choose(&world, &wrong, id(501)).is_none());
    for bit in [actions::COPY, actions::MODIFY, actions::TRANSFER] {
        world.inventory.items.get_mut(&id(9)).unwrap().owner_mask = actions::FULL & !bit;
        world.inventory.generation += 1;
        assert!(!policy.current(&world, Some(&source), 1));
        pump(
            &mut world,
            match bit {
                actions::COPY => "copy revoked",
                actions::MODIFY => "modify revoked",
                actions::TRANSFER => "transfer revoked",
                _ => unreachable!(),
            },
        );
        assert!(!policy.accepts(&world, &source, &reference));
        policy.refresh(&world, Some(&source), 1, std::slice::from_ref(&reference));
        assert!(!policy.allows(&reference));
    }
    world.inventory.items.get_mut(&id(9)).unwrap().owner_mask = actions::FULL;
    world.inventory.generation += 1;
    pump(&mut world, "permissions restored");
    policy.refresh(&world, Some(&source), 1, std::slice::from_ref(&reference));
    assert!(policy.allows(&reference));
    world.inventory_archive = Some(testing::native_loaded(&world, &[id(2)]));
    assert!(!policy.current(&world, Some(&source), 1));
    pump(&mut world, "archive folder2 published");
    assert!(!policy.accepts(&world, &source, &reference));
    policy.refresh(&world, Some(&source), 1, std::slice::from_ref(&reference));
    assert!(!policy.allows(&reference));
    world.inventory_archive = Some(testing::native_loaded(&world, &[]));
    pump(&mut world, "archive cleared");
    assert!(policy.accepts(&world, &source, &reference));
    assert!(world.rlva.enabled(), "restriction acceptance needs the real native opt-in");
    assert!(world.rlva.receive(&aurora_net::ChatMessage {
        from_name: "Synthetic image restriction".into(),
        source_id: id(601),
        owner_id: world.agent_id,
        source_type: aurora_net::ChatSourceType::Object,
        chat_type: aurora_net::ChatType::OwnerSay,
        position: glam::Vec3::ZERO,
        message: "@viewtexture=n".into(),
    }));
    assert!(world.rlva.has("viewtexture"), "the native restriction is actually installed");
    assert!(!policy.accepts(&world, &source, &reference));
    policy.refresh(&world, None, 1, std::slice::from_ref(&reference));
    assert!(!policy.allows(&reference));
    assert!(
        world.inventory.queue.is_empty(),
        "metadata qualification never starts an inventory fetch"
    );
}

/// Reproduce the original malformed fixture through the genuine native owner.
/// Empty archive metadata bypasses traversal; marked metadata must fail closed
/// on inconsistent Library provenance, then recover only after real data repair.
#[test]
fn workspace_picture_inconsistent_library_metadata_is_limited_until_repaired_without_archive_or_permission_bypass() {
    let mut world = picture_world();
    let id = |n| Uuid::from_u128(n);
    world.inventory.folders.get_mut(&id(8)).unwrap().library = false;
    let mut ordinary = world.inventory.items[&id(9)].clone();
    ordinary.id = id(10);
    ordinary.parent = id(4);
    ordinary.asset_id = id(502);
    world.inventory.items.insert(ordinary.id, ordinary);
    world.inventory.folders.get_mut(&id(4)).unwrap().items = vec![id(10)];
    world.inventory_archive = Some(testing::native_loaded(&world, &[]));
    pump(&mut world, "malformed Library with empty archive bypass");
    let source = Source {
        agent: world.agent_id,
        login: world.login.as_ref().unwrap().session_id,
        grid: world.inventory_archive.as_ref().unwrap().native_grid(&world).unwrap().namespace(),
        hosted: None,
    };
    let mut policy = Policy::default();
    assert!(policy.choose(&world, &source, id(501)).is_some());
    assert!(policy.choose(&world, &source, id(502)).is_some());
    world.inventory_archive = Some(testing::native_loaded(&world, &[id(2)]));
    pump_to(
        &mut world,
        "malformed Library after archive folder2 published",
        VisibilityStatus::Limited,
    );
    assert!(policy.choose(&world, &source, id(501)).is_none());
    assert!(
        policy.choose(&world, &source, id(502)).is_none(),
        "Limited is never ordinary permission, even outside marked branch"
    );
    world.inventory.folders.get_mut(&id(8)).unwrap().library = true;
    world.inventory.generation += 1;
    pump(&mut world, "Library provenance repaired, archive retained");
    assert!(policy.choose(&world, &source, id(501)).is_none(), "archived item remains hidden");
    assert!(
        policy.choose(&world, &source, id(502)).is_some(),
        "ordinary item recovers only with coherent native projection"
    );
    assert!(world.inventory.queue.is_empty(), "repair never fetches inventory");
}
