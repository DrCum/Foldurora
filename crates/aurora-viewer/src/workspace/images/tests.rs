//! Real native World/archive proof, no renderer, socket or private store IO.
use super::*;
use crate::world::{
    inventory::FetchState,
    inventory_archive::{controller::testing, visibility::CpuTurn},
};

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
    panic!("workspace image ordinary native projection did not settle");
}
#[test]
fn workspace_picture_policy_requalifies_exact_native_item_permissions_archive_source_and_privacy_without_fetches() {
    let mut world = crate::world::inventory_archive::tests::native_fixture_world();
    let id = |n| Uuid::from_u128(n);
    for (parent, children) in [(1, vec![2, 4, 5]), (2, vec![3]), (5, vec![6]), (7, vec![8])] {
        world.inventory.folders.get_mut(&id(parent)).unwrap().children = children.into_iter().map(id).collect();
    }
    for folder in world.inventory.folders.values_mut() {
        folder.state = FetchState::Fetched;
    }
    let item = world.inventory.items.get_mut(&id(9)).unwrap();
    item.asset_type = 0;
    item.inv_type = 0;
    item.asset_id = id(501);
    item.owner_mask = actions::FULL;
    world.inventory.folders.get_mut(&id(2)).unwrap().items = vec![id(9)];
    world.inventory_archive = Some(testing::native_loaded(&world, &[]));
    pump(&mut world);
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
        pump(&mut world);
        assert!(!policy.accepts(&world, &source, &reference));
        policy.refresh(&world, Some(&source), 1, std::slice::from_ref(&reference));
        assert!(!policy.allows(&reference));
    }
    world.inventory.items.get_mut(&id(9)).unwrap().owner_mask = actions::FULL;
    world.inventory.generation += 1;
    pump(&mut world);
    policy.refresh(&world, Some(&source), 1, std::slice::from_ref(&reference));
    assert!(policy.allows(&reference));
    world.inventory_archive = Some(testing::native_loaded(&world, &[id(2)]));
    assert!(!policy.current(&world, Some(&source), 1));
    pump(&mut world);
    assert!(!policy.accepts(&world, &source, &reference));
    policy.refresh(&world, Some(&source), 1, std::slice::from_ref(&reference));
    assert!(!policy.allows(&reference));
    world.inventory_archive = Some(testing::native_loaded(&world, &[]));
    pump(&mut world);
    assert!(policy.accepts(&world, &source, &reference));
    assert!(world.rlva.receive(&aurora_net::ChatMessage {
        from_name: "Synthetic image restriction".into(),
        source_id: id(601),
        owner_id: world.agent_id,
        source_type: aurora_net::ChatSourceType::Object,
        chat_type: aurora_net::ChatType::OwnerSay,
        position: glam::Vec3::ZERO,
        message: "@viewtexture=n".into(),
    }));
    assert!(!policy.accepts(&world, &source, &reference));
    policy.refresh(&world, None, 1, std::slice::from_ref(&reference));
    assert!(!policy.allows(&reference));
    assert!(
        world.inventory.queue.is_empty(),
        "metadata qualification never starts an inventory fetch"
    );
}
