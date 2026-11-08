//! Actual read-only Scope/Mask paths; no native/private IO fixture is run here.
use super::*;
use crate::world::inventory_archive::{
    controller::{Controller, testing as controller_testing},
    visibility::{CpuTurn, OwnerPolicy, VisibilityStatus, testing},
};

#[test]
fn inventory_archive_offline_fixture_retains_none_login_nil_agent_and_actual_owner_without_native_source() {
    let mut world = world();
    world.agent_id = Uuid::nil();
    world.login = None;
    let owner = world.inventory.owner;
    assert_eq!(
        Model::new(&world, grid("https://native.example.invalid")).err(),
        Some(Error::Unavailable)
    );
    world.bind_archive_offline_fixture();
    assert!(world.login.is_none());
    assert!(world.agent_id.is_nil());
    assert_eq!(world.inventory.owner, owner);
    let mut policy = OwnerPolicy::default();
    let read = world.inventory_archive.as_ref().unwrap().read(&world, &mut policy).unwrap();
    assert_eq!(read.status(), VisibilityStatus::OfflineReadOnly);
    assert!(read.passes(id(2)));
    assert!(read.passes(id(9)));
    assert!(!read.passes(id(999)));
    assert!(read.prepare_edit(id(2)).is_none());
}

#[test]
fn inventory_archive_offline_poll_pump_forged_native_edit_and_reply_never_admit_store_or_publish_success() {
    let world = world();
    let mut native = ready(&world);
    let ticket = native.prepare_edit(&world, id(2)).unwrap();
    let intent = native.begin_save(&world, ticket.clone(), true).unwrap();
    let mut memory_world = world;
    memory_world.bind_archive_offline_fixture();
    let mut memory = memory_world.inventory_archive.take().unwrap();
    for _ in 0..128 {
        memory.poll(&memory_world);
        let mut turn = CpuTurn::default();
        turn.advance();
        memory.pump(&memory_world, &mut turn);
        assert_eq!(testing::used(&turn).probes, 0);
    }
    assert!(memory.memory_only());
    assert_eq!(memory.status(), Some(Status::ReadOnly));
    assert!(memory.prepare_edit(&memory_world, id(2)).is_none());
    assert_eq!(memory.assign(&memory_world, ticket, true), Err(Error::Unavailable));
    controller_testing::reply(
        &mut memory,
        super::super::store::Reply::Saved {
            intent,
            result: Ok(Arc::from(b"not a receipt".as_slice())),
        },
    );
    assert!(memory.memory_only());
    assert_eq!(memory.status(), Some(Status::ReadOnly));
    assert_eq!(controller_testing::resources(&memory), (false, false, None));
    memory.retire();
    memory.retire();
    memory.observe(&memory_world);
    assert_eq!(controller_testing::resources(&memory), (false, false, None));
    assert_eq!(
        memory.read(&memory_world, &mut OwnerPolicy::default()).unwrap().status(),
        VisibilityStatus::Unavailable
    );
}

#[test]
fn inventory_archive_offline_actual_physical_replacement_and_reset_revoke_equal_facts_scope() {
    let mut world = world();
    world.bind_archive_offline_fixture();
    let instance = world.inventory_archive_instance;
    let mut policy = OwnerPolicy::default();
    let original = world
        .inventory_archive
        .as_ref()
        .unwrap()
        .read(&world, &mut policy)
        .unwrap()
        .binding();
    let mut fresh = super::world();
    let replacement = std::mem::take(&mut fresh.inventory);
    assert_eq!(replacement.generation, world.inventory.generation);
    world.replace_inventory(replacement);
    assert!(world.inventory_archive.is_none());
    assert_ne!(world.inventory_archive_instance, instance);
    world.bind_archive_offline_fixture();
    let next = world
        .inventory_archive
        .as_ref()
        .unwrap()
        .read(&world, &mut policy)
        .unwrap()
        .binding();
    assert!(original.is_some());
    assert!(original != next);
    let instance = world.inventory_archive_instance;
    world.reset();
    assert!(world.inventory_archive.is_none());
    assert_ne!(world.inventory_archive_instance, instance);
}

#[test]
fn inventory_archive_offline_scope_cannot_readopt_changed_owner_or_profile_or_become_persistent() {
    let mut world = world();
    world.bind_archive_offline_fixture();
    let original = world.inventory.owner;
    world.inventory.owner = id(999);
    world.observe_inventory_archive();
    world.inventory.owner = original;
    assert_eq!(
        world
            .inventory_archive
            .as_ref()
            .unwrap()
            .read(&world, &mut OwnerPolicy::default())
            .unwrap()
            .status(),
        VisibilityStatus::Unavailable
    );
    world.bind_archive_offline_fixture();
    controller_testing::previous_profile(world.inventory_archive.as_mut().unwrap());
    world.observe_inventory_archive();
    assert!(
        world.inventory_archive.is_none(),
        "retire offline; do not rebind a persistent mode from mutable facts"
    );
    world.bind_inventory_archive(None);
    world.poll_inventory_archive();
    assert!(!world.inventory_archive.as_ref().unwrap().memory_only());
    assert!(
        world
            .inventory_archive
            .as_ref()
            .unwrap()
            .read(&world, &mut OwnerPolicy::default())
            .is_none()
    );
}

#[test]
fn inventory_archive_offline_privacy_aba_and_native_binding_switch_advance_checked_owner_revision() {
    let mut world = world();
    world.rlva.configure(true);
    world.bind_archive_offline_fixture();
    let mut policy = OwnerPolicy::default();
    let first = world
        .inventory_archive
        .as_ref()
        .unwrap()
        .read(&world, &mut policy)
        .unwrap()
        .binding();
    let revision = policy.revision();
    restriction(&mut world, "@showinv=n");
    world.observe_inventory_archive();
    assert!(
        !world
            .inventory_archive
            .as_ref()
            .unwrap()
            .read(&world, &mut policy)
            .unwrap()
            .passes(id(2))
    );
    assert_ne!(policy.revision(), revision);
    restriction(&mut world, "@showinv=y");
    world.observe_inventory_archive();
    let restored = world
        .inventory_archive
        .as_ref()
        .unwrap()
        .read(&world, &mut policy)
        .unwrap()
        .binding();
    assert!(restored.is_some());
    assert!(first != restored);
    let native = ready(&world);
    let projection = super::super::visibility::Projection::default();
    let native_binding = projection.read(&world, &native, &mut policy).binding();
    assert!(native_binding.is_some());
    assert!(native_binding != restored);
    assert!(
        world
            .inventory_archive
            .as_ref()
            .unwrap()
            .read(&world, &mut policy)
            .unwrap()
            .binding()
            != native_binding
    );
}

#[test]
fn inventory_archive_native_missing_origin_and_corrupt_load_never_acquire_offline_status() {
    let world = world();
    let mut policy = OwnerPolicy::default();
    let missing = Controller::bind(&world, None);
    assert!(!missing.memory_only());
    assert!(missing.read(&world, &mut policy).is_none());
    let mut model = Model::new(&world, grid("https://native.example.invalid")).unwrap();
    model.complete_load(model.load_ticket().unwrap(), Err(Error::Invalid));
    let projection = super::super::visibility::Projection::default();
    let read = projection.read(&world, &model, &mut policy);
    assert_ne!(read.status(), VisibilityStatus::OfflineReadOnly);
    assert!(!read.passes(id(2)));
}

#[test]
fn inventory_archive_mask_lends_same_actual_inventory_to_rayon_without_requalification_or_clone() {
    use rayon::prelude::*;
    let mut world = world();
    world.bind_archive_offline_fixture();
    let mut policy = OwnerPolicy::default();
    testing::reset_qualifications();
    let read = world.inventory_archive.as_ref().unwrap().read(&world, &mut policy).unwrap();
    let mask = read.mask();
    assert!(mask.for_inventory(&world.inventory));
    let ids = [id(2), id(3), id(9), id(999)];
    for _ in 0..128 {
        assert_eq!(ids.par_iter().filter(|id| mask.passes(**id)).count(), 3);
    }
    assert_eq!(testing::qualifications(), 1);
    let unrelated = super::world();
    assert!(!mask.for_inventory(&unrelated.inventory));
}
