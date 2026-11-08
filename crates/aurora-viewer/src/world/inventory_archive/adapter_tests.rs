//! Real World/controller hooks; native fixtures are authored UNRUN for Root.
//! No App constructor/socket/native window is needed for these source cases.
use super::*;
use crate::world::inventory_archive::{
    controller::{Controller, testing as controller_testing},
    visibility::{CpuTurn, OwnerPolicy, VisibilityStatus, testing},
};
use std::time::{Duration, Instant};

fn origin() -> GridDigest {
    grid(&format!("https://{}.example.invalid/archive-adapter", Uuid::new_v4()))
}
fn loaded() -> World {
    let mut world = world();
    world.inventory.folders.get_mut(&id(8)).unwrap().library = true;
    for (parent, children) in [(1, vec![2, 4, 5]), (2, vec![3]), (5, vec![6]), (7, vec![8])] {
        world.inventory.folders.get_mut(&id(parent)).unwrap().children = children.into_iter().map(id).collect();
    }
    world
}
fn settled(world: &mut World) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while matches!(
        world.inventory_archive.as_ref().and_then(Controller::status),
        Some(Status::Loading | Status::Saving)
    ) && Instant::now() < deadline
    {
        world.poll_inventory_archive();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(world.inventory_archive.as_ref().and_then(Controller::status), Some(Status::Ready));
}
fn assign(world: &mut World, ticket: EditTicket, desired: bool) -> Result<(), Error> {
    // Test calls the actual data controller; no UI/native gesture permission
    // is inferred. There is no event/UI callback in this concrete scope.
    let mut archive = world.inventory_archive.take().unwrap();
    let result = archive.assign(world, ticket, desired);
    world.inventory_archive = Some(archive);
    result
}
fn prepare(world: &World, target: u128) -> EditTicket {
    world.inventory_archive.as_ref().unwrap().prepare_edit(world, id(target)).unwrap()
}
fn marked(world: &mut World, origin: GridDigest) {
    world.bind_inventory_archive(Some(origin));
    settled(world);
    let ticket = prepare(world, 2);
    assign(world, ticket, true).unwrap();
    settled(world);
}
fn small(world: &mut World) {
    controller_testing::projection(
        world.inventory_archive.as_mut().unwrap(),
        testing::projection(32, 16, 16, MAX_DEPTH, false),
    );
}
fn initialized(world: &World) -> usize {
    controller_testing::initialized(world.inventory_archive.as_ref().unwrap())
}

#[test]
fn inventory_archive_world_actual_replacement_helper_retires_equal_ids_and_generation_before_rebind() {
    let mut world = loaded();
    let origin = origin();
    world.bind_inventory_archive(Some(origin));
    settled(&mut world);
    let original = prepare(&world, 2);
    let mut fresh = loaded();
    let replacement = std::mem::take(&mut fresh.inventory);
    assert_eq!(replacement.root, world.inventory.root);
    assert_eq!(replacement.owner, world.inventory.owner);
    assert_eq!(replacement.generation, world.inventory.generation);
    world.replace_inventory(replacement);
    assert!(
        world.inventory_archive.is_none(),
        "retired before any future paint or original edit"
    );
    world.bind_inventory_archive(Some(origin));
    settled(&mut world);
    assert_ne!(prepare(&world, 2).namespace().0, original.namespace().0);
    assert_eq!(assign(&mut world, original, true), Err(Error::Stale));
    assert!(!prepare(&world, 2).previous());
}

#[test]
fn inventory_archive_world_reset_keeps_external_app_turn_budget_across_new_model_and_layout_repeats() {
    let mut world = loaded();
    let origin = origin();
    marked(&mut world, origin);
    small(&mut world);
    let mut app_turn = CpuTurn::default();
    testing::limits(&mut app_turn, 2, 2, 1);
    app_turn.advance();
    world.pump_inventory_archive(&mut app_turn);
    assert_eq!(initialized(&world), 1);
    assert_eq!(testing::used(&app_turn).probes, 1);
    world
        .metadata_login
        .submitted(&super::super::super::account_metadata_origin::request(
            "https://pending.example.invalid",
        ));
    world.reset();
    assert!(world.inventory_archive.is_none());
    assert!(world.metadata_login.completed().is_none());
    assert_eq!(testing::used(&app_turn).probes, 1);
    let mut replacement = loaded();
    replacement.bind_inventory_archive(Some(origin));
    settled(&mut replacement);
    small(&mut replacement);
    for _ in 0..64 {
        replacement.pump_inventory_archive(&mut app_turn);
        replacement.observe_inventory_archive();
    }
    assert_eq!(initialized(&replacement), 0);
    assert_eq!(testing::used(&app_turn).probes, 1);
    app_turn.advance();
    replacement.pump_inventory_archive(&mut app_turn);
    assert_eq!(initialized(&replacement), 1);
}

#[test]
fn inventory_archive_world_event_observers_only_revoke_without_full_qualification_or_work() {
    let mut world = loaded();
    marked(&mut world, origin());
    small(&mut world);
    let mut turn = CpuTurn::default();
    testing::limits(&mut turn, 2, 2, 1);
    turn.advance();
    world.pump_inventory_archive(&mut turn);
    assert_eq!(initialized(&world), 1);
    testing::reset_qualifications();
    for _ in 0..512 {
        world.observe_inventory_archive();
    }
    assert_eq!(initialized(&world), 1);
    assert_eq!(testing::qualifications(), 0);
    assert_eq!(testing::used(&turn).probes, 1);
    world.inventory.generation += 1;
    world.observe_inventory_archive();
    assert_eq!(initialized(&world), 0);
    assert_eq!(testing::qualifications(), 0);
    turn.advance();
    world.pump_inventory_archive(&mut turn);
    assert_eq!(initialized(&world), 1);
    testing::reset_qualifications();
    world.rlva.configure(true);
    restriction(&mut world, "@showinv=n");
    world.observe_inventory_archive();
    assert_eq!(initialized(&world), 0);
    assert_eq!(testing::qualifications(), 0);
    let mut policy = OwnerPolicy::default();
    assert!(
        !world
            .inventory_archive
            .as_ref()
            .unwrap()
            .read(&world, &mut policy)
            .unwrap()
            .passes(id(4))
    );
    assert_eq!(testing::qualifications(), 1);
    assert_eq!(testing::used(&turn).probes, 1);
}

#[test]
fn inventory_archive_world_profile_boundary_fresh_read_denies_then_rebinds_only_original_origin() {
    let mut world = loaded();
    let original_origin = origin();
    world.bind_inventory_archive(Some(original_origin));
    settled(&mut world);
    let original = prepare(&world, 2);
    let mut policy = OwnerPolicy::default();
    assert!(world.inventory_archive.as_ref().unwrap().set_include(&world, &mut policy, true));
    controller_testing::previous_profile(world.inventory_archive.as_mut().unwrap());
    let archive = world.inventory_archive.as_ref().unwrap();
    assert!(archive.read(&world, &mut policy).is_none());
    assert!(archive.prepare_edit(&world, id(2)).is_none());
    assert!(!archive.set_include(&world, &mut policy, true));
    // The actual World boundary handles the profile edge BEFORE take/restore;
    // the fresh model is still Loading and has not started private IO.
    world.observe_inventory_archive();
    assert_eq!(world.inventory_archive.as_ref().and_then(Controller::status), Some(Status::Loading));
    assert!(
        !world
            .inventory_archive
            .as_ref()
            .unwrap()
            .read(&world, &mut policy)
            .unwrap()
            .passes(id(4))
    );
    settled(&mut world);
    assert_eq!(assign(&mut world, original, true), Err(Error::Stale));
    let ticket = prepare(&world, 2);
    assign(&mut world, ticket, true).unwrap();
    settled(&mut world);
    let directory = crate::session_host::profile::NativeDirectory::select(
        crate::session_host::profile::NativeStoreKind::InventoryArchive,
        world.agent_id,
        &original_origin.namespace(),
    )
    .unwrap();
    let document = Document::decode(
        &directory.read().unwrap().unwrap(),
        Source::observe(&world, original_origin).unwrap(),
    )
    .unwrap();
    assert!(document.folders.contains(&id(2)));
}

#[test]
fn inventory_archive_world_unbound_origin_or_invalid_actual_source_never_rebinds_from_profile_or_form() {
    let mut world = loaded();
    world
        .metadata_login
        .submitted(&super::super::super::account_metadata_origin::request(
            "https://original.example.invalid",
        ));
    let original = world.metadata_login.completed();
    assert!(original.is_some());
    assert!(world.metadata_login.completed().is_none());
    // Same None path used by actual App's offline/demo binding policy.
    world.bind_inventory_archive(None);
    controller_testing::previous_profile(world.inventory_archive.as_mut().unwrap());
    world.observe_inventory_archive();
    world.poll_inventory_archive();
    assert_eq!(world.inventory_archive.as_ref().and_then(Controller::status), None);
    assert!(
        world
            .inventory_archive
            .as_ref()
            .unwrap()
            .read(&world, &mut OwnerPolicy::default())
            .is_none()
    );
    world.inventory.owner = id(999);
    world.bind_inventory_archive(original);
    assert_eq!(world.inventory_archive.as_ref().and_then(Controller::status), None);
    world.inventory.owner = world.agent_id;
    world.observe_inventory_archive();
    assert_eq!(world.inventory_archive.as_ref().and_then(Controller::status), None);
    assert_eq!(world.inventory.queue.len(), 0);
}

#[test]
fn inventory_archive_world_publication_without_inventory_change_revokes_mask_before_next_paint() {
    let mut world = loaded();
    marked(&mut world, origin());
    small(&mut world);
    let mut turn = CpuTurn::default();
    let mut policy = OwnerPolicy::default();
    for _ in 0..512 {
        turn.advance();
        world.pump_inventory_archive(&mut turn);
        if world
            .inventory_archive
            .as_ref()
            .unwrap()
            .read(&world, &mut policy)
            .unwrap()
            .status()
            == VisibilityStatus::Ready
        {
            break;
        }
    }
    let archive = world.inventory_archive.as_ref().unwrap();
    assert!(!archive.read(&world, &mut policy).unwrap().passes(id(2)));
    let revision = policy.revision();
    let generation = world.inventory.generation;
    let ticket = prepare(&world, 2);
    assign(&mut world, ticket, false).unwrap();
    settled(&mut world);
    assert_eq!(world.inventory.generation, generation);
    // Empty current metadata is a legitimate Ready shortcut, not an old
    // retained masked snapshot or an unknown/load-failure empty fallback.
    let read = world.inventory_archive.as_ref().unwrap().read(&world, &mut policy).unwrap();
    assert_eq!(read.status(), VisibilityStatus::Ready);
    assert!(read.passes(id(2)));
    assert_ne!(read.revision(), revision);
    assert_eq!(world.inventory.queue.len(), 0);
}
