//! Real World/native archive projection; no store worker, socket or native UI.
use super::*;
use crate::world::inventory_archive::{controller::testing, visibility::CpuTurn};
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn fixture(flags: &[Uuid]) -> (World, Source) {
    let mut world = crate::world::inventory_archive::tests::native_fixture_world();
    world.inventory.folders.get_mut(&id(8)).unwrap().library = true;
    for (parent, children) in [(1, vec![2, 4, 5]), (2, vec![3]), (5, vec![6]), (7, vec![8])] {
        world.inventory.folders.get_mut(&id(parent)).unwrap().children = children.into_iter().map(id).collect();
    }
    let mut avatar = crate::demo::beacons::objects().remove(0);
    avatar.local_id = 100;
    avatar.full_id = world.agent_id;
    avatar.parent_id = 0;
    avatar.pcode = 47;
    world.objects.upsert(1, avatar);
    let mut item = world.inventory.items[&id(9)].clone();
    item.id = id(10);
    item.asset_type = 6;
    item.parent = id(4);
    item.name = "Ordinary HUD".into();
    world.inventory.items.insert(item.id, item);
    world.inventory_archive = Some(testing::native_loaded(&world, flags));
    pump(&mut world);
    let source = Source {
        agent: world.agent_id,
        login: world.login.as_ref().unwrap().session_id,
        grid: "a".repeat(64),
        hosted: None,
    };
    (world, source)
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
    panic!("native ordinary HUD projection did not settle");
}
fn wanted() -> HudSet {
    HudSet::try_from(vec![HudRef { item: id(10), point: 31 }]).unwrap()
}
fn wear(world: &mut World, point: u8) {
    let mut object = crate::demo::beacons::objects().remove(0);
    object.local_id = 101;
    object.full_id = id(110);
    object.parent_id = 100;
    object.pcode = 9;
    object.state = point.rotate_left(4);
    object.name_values = format!("AttachItemID STRING RW SV {}", id(10));
    world.objects.upsert(1, object);
}
#[test]
fn workspace_hud_projection_uses_real_worn_item_and_native_point_without_asset_access() {
    let (mut world, source) = fixture(&[]);
    let mut policy = State::default();
    assert!(worn(&world, &source, &mut policy).unwrap().is_empty());
    assert!(!resolve(&world, &source, &mut policy, &wanted()).unwrap()[0].worn);
    // A missing/protected asset UUID is not a reason to reject a valid wear.
    assert!(world.inventory.items[&id(10)].asset_id.is_nil());
    wear(&mut world, 31);
    let rows = worn(&world, &source, &mut policy).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].reference, wanted().refs()[0]);
    assert_eq!(rows[0].name, "Ordinary HUD");
    wear(&mut world, 32);
    assert!(
        resolve(&world, &source, &mut policy, &wanted()).unwrap().is_empty(),
        "never move a worn item to another point"
    );
}
#[test]
fn workspace_hud_projection_refuses_foreign_link_trash_library_archived_and_unknown_items() {
    for bad in 0..5 {
        let marked = [id(4)];
        let (mut world, source) = fixture(if bad == 4 { &marked } else { &[] });
        let item = world.inventory.items.get_mut(&id(10)).unwrap();
        match bad {
            0 => item.owner = id(999),
            1 => {
                item.asset_type = 24;
                item.asset_id = id(9);
            }
            2 => item.parent = id(6),
            3 => item.parent = id(8),
            _ => {}
        }
        world.inventory.generation += 1;
        pump(&mut world);
        let mut policy = State::default();
        assert!(resolve(&world, &source, &mut policy, &wanted()).unwrap().is_empty(), "case {bad}");
    }
    let (mut world, mut source) = fixture(&[]);
    let mut policy = State::default();
    source.login = id(777);
    assert!(worn(&world, &source, &mut policy).is_none());
    source.login = world.login.as_ref().unwrap().session_id;
    world.inventory_archive = None;
    assert!(resolve(&world, &source, &mut policy, &wanted()).is_none());
}
#[test]
fn workspace_hud_runtime_retires_consent_on_focus_binding_rules_and_archive_publication() {
    let (mut world, source) = fixture(&[]);
    let mut runtime = Runtime::default();
    runtime.sync(&world, Some(&source), 1, true);
    runtime.offer(&world, &source, &wanted());
    let nonce = runtime.lane.review.as_ref().unwrap().nonce;
    runtime.sync(&world, Some(&source), 1, false);
    runtime.decide(&world, &source, nonce, true);
    assert!(!runtime.busy());
    assert!(runtime.view.review.is_none());
    for cause in 0..3 {
        runtime.sync(&world, Some(&source), 1, true);
        runtime.offer(&world, &source, &wanted());
        let nonce = runtime.lane.review.as_ref().unwrap().nonce;
        match cause {
            0 => runtime.sync(&world, Some(&source), 2, true),
            1 => {
                world.rlva.configure(true);
                runtime.sync(&world, Some(&source), 1, true);
            }
            _ => {
                let mut archive = world.inventory_archive.take().unwrap();
                testing::publish_fixture(&mut archive, &world, id(4), true);
                world.inventory_archive = Some(archive);
                runtime.sync(&world, Some(&source), 1, true);
            }
        }
        runtime.decide(&world, &source, nonce, true);
        assert!(!runtime.busy());
    }
}
#[test]
fn workspace_hud_runtime_inventory_outfit_generation_does_not_replay_or_drop_native_flight() {
    let (mut world, source) = fixture(&[]);
    let mut runtime = Runtime::default();
    runtime.sync(&world, Some(&source), 1, true);
    runtime.offer(&world, &source, &wanted());
    let nonce = runtime.lane.review.as_ref().unwrap().nonce;
    runtime.decide(&world, &source, nonce, true);
    assert_eq!(runtime.next(&world, false).unwrap(), wanted().refs());
    world.inventory.generation += 1;
    pump(&mut world);
    runtime.sync(&world, Some(&source), 1, true);
    assert!(runtime.busy());
    assert!(runtime.next(&world, false).is_none());
    wear(&mut world, 31);
    assert!(runtime.next(&world, false).is_none());
    assert!(!runtime.busy());
    assert_eq!(runtime.lane.progress, super::super::hud_lane::Progress::Complete);
}

#[test]
fn workspace_hud_projection_obeys_add_locks_and_hides_locked_worn_names_and_showinv() {
    let (mut world, source) = fixture(&[]);
    world.rlva.configure(true);
    let command = |world: &mut World, text: &str| {
        assert!(world.rlva.receive(&aurora_net::ChatMessage {
            from_name: "Synthetic HUD restriction".into(),
            source_id: id(999),
            owner_id: world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: text.into(),
        }));
    };
    command(&mut world, "@addattach=n,remattach=n");
    pump(&mut world);
    let mut policy = State::default();
    assert!(resolve(&world, &source, &mut policy, &wanted()).unwrap().is_empty());
    wear(&mut world, 31);
    world.rlva.preferences.hide_locked_attachments = false;
    assert_eq!(worn(&world, &source, &mut policy).unwrap().len(), 1);
    world.rlva.preferences.hide_locked_attachments = true;
    assert!(worn(&world, &source, &mut policy).unwrap().is_empty());
    assert!(resolve(&world, &source, &mut policy, &wanted()).unwrap().is_empty());
    command(&mut world, "@showinv=n");
    assert!(worn(&world, &source, &mut policy).is_none());
    assert!(resolve(&world, &source, &mut policy, &wanted()).is_none());
}
