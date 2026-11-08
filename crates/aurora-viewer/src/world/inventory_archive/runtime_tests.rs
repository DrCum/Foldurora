//! Authored, UNRUN native fixtures. Only explicit test polls start private IO;
//! synthetic identities and the existing cfg(test) selected profile are used.
use super::*;
use crate::session_host::profile::{NativeDirectory, NativeStoreCancellation, NativeStoreKind};
use crate::world::inventory_archive::{controller::Controller, visibility::OwnerPolicy};
use std::time::{Duration, Instant};

fn origin() -> GridDigest {
    grid(&format!("https://{}.example.invalid/archive-fixture", Uuid::new_v4()))
}
fn directory(world: &World, origin: GridDigest) -> NativeDirectory {
    NativeDirectory::select(NativeStoreKind::InventoryArchive, world.agent_id, &origin.namespace()).unwrap()
}
fn settled(controller: &mut Controller, world: &World) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while matches!(controller.status(), Some(Status::Loading | Status::Saving)) && Instant::now() < deadline {
        controller.poll(world);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!matches!(controller.status(), Some(Status::Loading | Status::Saving)));
}

#[test]
fn inventory_archive_controller_no_origin_bind_is_io_idle_and_cannot_include_or_edit() {
    let world = world();
    let mut controller = Controller::bind(&world, None);
    let mut policy = OwnerPolicy::default();
    assert_eq!(controller.status(), None);
    assert!(controller.read(&world, &mut policy).is_none());
    assert!(!controller.set_include(&world, &mut policy, true));
    assert!(controller.prepare_edit(&world, id(2)).is_none());
    controller.poll(&world);
    assert_eq!(controller.status(), None);
}

#[test]
fn inventory_archive_controller_actual_missing_load_never_eagerly_creates_store_or_empty_file() {
    let world = world();
    let origin = origin();
    let directory = directory(&world, origin);
    assert!(matches!(directory.read(), Err(error) if error.kind() == std::io::ErrorKind::NotFound));
    let mut controller = Controller::bind(&world, Some(origin));
    assert_eq!(controller.status(), Some(Status::Loading));
    settled(&mut controller, &world);
    assert_eq!(controller.status(), Some(Status::Ready));
    assert!(matches!(directory.read(), Err(error) if error.kind() == std::io::ErrorKind::NotFound));
    let mut policy = OwnerPolicy::default();
    assert!(controller.read(&world, &mut policy).unwrap().passes(id(2)));
    assert_eq!(world.inventory.queue.len(), 0);
}

#[test]
fn inventory_archive_controller_actual_corrupt_bytes_stay_unknown_and_explicit_include_needs_privacy() {
    let mut world = world();
    let origin = origin();
    let directory = directory(&world, origin);
    let bytes = b"{unknown archive version";
    {
        let _lock = directory.lock().unwrap();
        directory.publish_cancellable(bytes, None, &NativeStoreCancellation::new()).unwrap();
    }
    let mut controller = Controller::bind(&world, Some(origin));
    settled(&mut controller, &world);
    assert_eq!(controller.status(), Some(Status::LoadFailed));
    let mut policy = OwnerPolicy::default();
    assert!(!controller.read(&world, &mut policy).unwrap().passes(id(2)));
    assert!(controller.prepare_edit(&world, id(2)).is_none());
    assert!(controller.set_include(&world, &mut policy, true));
    assert!(controller.read(&world, &mut policy).unwrap().passes(id(2)));
    world.rlva.configure(true);
    restriction(&mut world, "@showinv=n");
    assert!(!controller.read(&world, &mut policy).unwrap().passes(id(2)));
    assert!(!controller.set_include(&world, &mut policy, true));
    assert_eq!(directory.read().unwrap().as_deref(), Some(bytes.as_slice()));
    assert_eq!(world.inventory.queue.len(), 0);
}

#[test]
fn inventory_archive_controller_actual_save_publishes_only_exact_original_and_failed_cas_keeps_flags() {
    let mut world = world();
    world.inventory.folders.get_mut(&id(8)).unwrap().library = true;
    for (parent, children) in [(1, vec![2, 4, 5]), (2, vec![3]), (5, vec![6]), (7, vec![8])] {
        world.inventory.folders.get_mut(&id(parent)).unwrap().children = children.into_iter().map(id).collect();
    }
    let origin = origin();
    let directory = directory(&world, origin);
    let mut controller = Controller::bind(&world, Some(origin));
    settled(&mut controller, &world);
    let ticket = controller.prepare_edit(&world, id(2)).unwrap();
    controller.assign(&world, ticket.clone(), true).unwrap();
    assert_eq!(controller.status(), Some(Status::Saving));
    settled(&mut controller, &world);
    assert_eq!(controller.status(), Some(Status::Ready));
    let bytes = directory.read().unwrap().unwrap();
    assert_eq!(
        Document::decode(&bytes, Source::observe(&world, origin).unwrap())
            .unwrap()
            .publication(),
        1
    );
    assert!(controller.prepare_edit(&world, id(2)).unwrap().previous());
    let mut turn = super::super::visibility::CpuTurn::default();
    let mut policy = OwnerPolicy::default();
    for _ in 0..100_000 {
        turn.advance();
        controller.pump(&world, &mut turn);
        if controller.read(&world, &mut policy).unwrap().status() == super::super::visibility::VisibilityStatus::Ready {
            break;
        }
    }
    let read = controller.read(&world, &mut policy).unwrap();
    assert_eq!(read.status(), super::super::visibility::VisibilityStatus::Ready);
    assert!(!read.passes(id(2)));
    assert!(!read.passes(id(3)));
    assert!(read.passes(id(4)));
    assert!(read.passes(id(6)));
    assert_eq!(controller.assign(&world, ticket, true), Err(Error::Stale));
    // A semantically identical outside replacement still fails original-byte
    // CAS. The old published mark remains; there is no optimistic unarchive.
    let mut changed = bytes.clone();
    changed.push(b' ');
    {
        let _lock = directory.lock().unwrap();
        directory
            .publish_cancellable(&changed, Some(&bytes), &NativeStoreCancellation::new())
            .unwrap();
    }
    let ticket = controller.prepare_edit(&world, id(2)).unwrap();
    controller.assign(&world, ticket, false).unwrap();
    settled(&mut controller, &world);
    assert_eq!(controller.status(), Some(Status::SaveFailed));
    assert!(controller.prepare_edit(&world, id(2)).unwrap().previous());
    assert_eq!(directory.read().unwrap().unwrap(), changed);
    assert_eq!(world.inventory.queue.len(), 0);
}

#[test]
fn inventory_archive_controller_source_loss_retires_before_poll_or_accepting_original_ticket() {
    let mut world = world();
    let origin = origin();
    let mut controller = Controller::bind(&world, Some(origin));
    settled(&mut controller, &world);
    let original = controller.prepare_edit(&world, id(2)).unwrap();
    world.inventory.owner = id(999);
    let mut policy = OwnerPolicy::default();
    assert!(!controller.read(&world, &mut policy).unwrap().passes(id(2)));
    assert_eq!(controller.assign(&world, original, true), Err(Error::Unavailable));
    assert_eq!(controller.status(), Some(Status::Retired));
    world.inventory.owner = world.agent_id;
    controller.poll(&world);
    assert_eq!(controller.status(), Some(Status::Retired));
    assert!(controller.prepare_edit(&world, id(2)).is_none());
}

#[test]
fn inventory_archive_controller_explicit_physical_rebind_rejects_same_id_generation_old_edit() {
    let world = world();
    let origin = origin();
    let mut old = Controller::bind(&world, Some(origin));
    settled(&mut old, &world);
    let original = old.prepare_edit(&world, id(2)).unwrap();
    let generation = world.inventory.generation;
    old.retire(); // The future actual World assignment helper MUST do this.
    let mut replacement = Controller::bind(&world, Some(origin));
    settled(&mut replacement, &world);
    assert_eq!(world.inventory.generation, generation);
    assert_eq!(replacement.assign(&world, original, true), Err(Error::Stale));
    assert!(!replacement.prepare_edit(&world, id(2)).unwrap().previous());
    assert_eq!(old.status(), Some(Status::Retired));
}
