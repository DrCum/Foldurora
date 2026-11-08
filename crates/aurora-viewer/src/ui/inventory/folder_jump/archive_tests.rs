//! Same actual native metadata read for index, final parent path and policy.
//! Pure receipt fixture has no Worker/native action capability or file IO.
use super::*;
use crate::world::inventory_archive::{
    controller::testing,
    visibility::{CpuTurn, VisibilityStatus},
};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn world(flags: &[Uuid]) -> World {
    let mut world = crate::world::inventory_archive::tests::native_fixture_world();
    world.inventory.folders.get_mut(&id(8)).unwrap().library = true;
    for (parent, children) in [(1, vec![2, 4, 5]), (2, vec![3]), (5, vec![6]), (7, vec![8])] {
        world.inventory.folders.get_mut(&id(parent)).unwrap().children = children.into_iter().map(id).collect();
    }
    for n in [2, 3, 4, 6] {
        world.inventory.folders.get_mut(&id(n)).unwrap().info.name = format!("Folder {n}");
    }
    world.inventory_archive = Some(testing::native_loaded(&world, flags));
    finish(&mut world);
    world
}
fn finish(world: &mut World) {
    let mut state = archive::State::default();
    let mut turn = CpuTurn::default();
    for _ in 0..256 {
        turn.advance();
        world.pump_inventory_archive(&mut turn);
        if state.read(world).unwrap().status() == VisibilityStatus::Ready {
            return;
        }
    }
    panic!("actual fixture projection did not publish");
}
fn source(world: &World) -> Source {
    Source {
        agent: world.agent_id,
        session: world.login.as_ref().unwrap().session_id,
        owner: world.inventory.owner,
        root: world.inventory.root,
        profile: 1,
        window: Uuid::nil(),
    }
}
fn ready(
    model: &mut model::Model,
    inventory: &crate::world::inventory::Inventory,
    mask: Option<crate::world::inventory_archive::visibility::MaskRead<'_>>,
) {
    for _ in 0..256 {
        model.advance_visible(inventory, mask);
        if model.phase() == model::Phase::Ready {
            return;
        }
    }
    panic!("actual gated chooser did not finish");
}

#[test]
fn folder_jump_archive_actual_mask_filters_index_and_final_ancestry_and_always_rejects_trash_recovery() {
    let world = world(&[id(2)]);
    let mut state = archive::State::default();
    let instance = state.instance();
    let read = state.read(&world).unwrap();
    let gate = archive::State::ordinary(instance, Some(&read), world.inventory_archive_instance);
    let policy = Policy {
        revision: read.revision().unwrap(),
        allowed: true,
    };
    let mut model = model::Model::default();
    model.sync_visible(&world.inventory, Some(source(&world)), policy, gate.mask());
    ready(&mut model, &world.inventory, gate.mask());
    let rows: Vec<_> = model.rows().map(|row| row.folder).collect();
    assert_eq!(rows, [id(4)]);
    assert!(gate.passes(id(6)), "Archive retains Trash recovery visibility");
    assert!(
        !current_folder(&world.inventory, id(6), gate.mask()),
        "chooser never accepts Trash ancestry"
    );
    assert!(!current_folder(&world.inventory, id(3), gate.mask()));
    let ticket = model.capture(id(4)).unwrap();
    assert!(model.validate_visible(ticket, &world.inventory, Some(source(&world)), policy, gate.mask()));
}

#[test]
fn folder_jump_archive_exact_publication_without_inventory_change_retires_old_index_and_final_ticket() {
    let mut world = world(&[]);
    let mut state = archive::State::default();
    let instance = state.instance();
    let mut model = model::Model::default();
    let original;
    {
        let read = state.read(&world).unwrap();
        let gate = archive::State::ordinary(instance, Some(&read), world.inventory_archive_instance);
        let policy = Policy {
            revision: read.revision().unwrap(),
            allowed: true,
        };
        model.sync_visible(&world.inventory, Some(source(&world)), policy, gate.mask());
        ready(&mut model, &world.inventory, gate.mask());
        model.set_query("Folder 3");
        ready(&mut model, &world.inventory, gate.mask());
        original = model.capture(id(3)).unwrap();
    }
    let generation = world.inventory.generation;
    let mut archive = world.inventory_archive.take().unwrap();
    testing::publish_fixture(&mut archive, &world, id(2), true);
    world.inventory_archive = Some(archive);
    finish(&mut world);
    assert_eq!(world.inventory.generation, generation);
    let read = state.read(&world).unwrap();
    let gate = archive::State::ordinary(instance, Some(&read), world.inventory_archive_instance);
    let policy = Policy {
        revision: read.revision().unwrap(),
        allowed: true,
    };
    assert!(!model.validate_visible(original, &world.inventory, Some(source(&world)), policy, gate.mask()));
    model.sync_visible(&world.inventory, Some(source(&world)), policy, gate.mask());
    ready(&mut model, &world.inventory, gate.mask());
    assert!(!model.rows().any(|row| row.folder == id(2) || row.folder == id(3)));
    assert!(!current_folder(&world.inventory, id(3), gate.mask()));
}

#[test]
fn folder_jump_archive_other_inventory_or_unknown_read_cannot_use_a_qualified_mask() {
    let world = world(&[]);
    let mut state = archive::State::default();
    let instance = state.instance();
    let read = state.read(&world).unwrap();
    let gate = archive::State::ordinary(instance, Some(&read), world.inventory_archive_instance);
    let other = crate::world::inventory_archive::tests::native_fixture_world();
    assert!(
        !current_folder(&other.inventory, id(4), gate.mask()),
        "same IDs cannot substitute another Inventory"
    );
    let mut model = model::Model::default();
    let policy = Policy {
        revision: 1,
        allowed: true,
    };
    model.sync_visible(&other.inventory, Some(source(&other)), policy, gate.mask());
    assert_eq!(model.phase(), model::Phase::Unavailable);
    assert!(!current_folder(&world.inventory, id(4), None));
    assert!(!current_folder(
        &world.inventory,
        id(4),
        archive::Gate::Exempt(archive::Exemption::Picker).mask()
    ));
}

#[test]
fn folder_jump_archive_checked_owner_incarnation_and_mask_policy_revisions_never_alias() {
    let world = world(&[id(2)]);
    let mut left = archive::State::default();
    let mut right = archive::State::default();
    let mut chooser = Chooser::default();
    chooser.owner = Some(Uuid::nil());
    let first;
    {
        let instance = left.instance();
        let read = left.read(&world).unwrap();
        let gate = archive::State::ordinary(instance, Some(&read), world.inventory_archive_instance);
        first = chooser.context(&world, true, true, gate).1.revision;
    }
    let instance = right.instance();
    let read = right.read(&world).unwrap();
    let gate = archive::State::ordinary(instance, Some(&read), world.inventory_archive_instance);
    let next = chooser.context(&world, true, true, gate).1.revision;
    assert!(next > first);
    chooser.archive_revision = u64::MAX;
    chooser.archive_key = None;
    assert!(!chooser.context(&world, true, true, gate).1.allowed);
    assert!(chooser.archive_exhausted);
}
