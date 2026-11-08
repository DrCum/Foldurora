use super::*;
use crate::world::inventory::{FetchState, Folder};
use aurora_net::inventory::{InvFolder, InvItem};
use std::sync::OnceLock;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn grid(uri: &str) -> GridDigest {
    let mut pending = super::super::account_metadata_origin::Pending::default();
    pending.submitted(&super::super::account_metadata_origin::request(uri));
    pending.completed().unwrap()
}
fn folder(world: &mut World, n: u128, parent: u128, kind: i32, library: bool) {
    world.inventory.folders.insert(
        id(n),
        Folder {
            info: InvFolder {
                id: id(n),
                parent: id(parent),
                type_default: kind,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Unknown,
            library,
        },
    );
}
fn world() -> World {
    static LIBRARY: OnceLock<Arc<crate::scene::avatar::AvatarLibrary>> = OnceLock::new();
    let mut world = World::new(
        LIBRARY
            .get_or_init(|| Arc::new(crate::scene::avatar::AvatarLibrary::load()))
            .clone(),
    );
    world.agent_id = id(100);
    world.inventory.owner = world.agent_id;
    world.inventory.root = id(1);
    world.inventory.lib_root = id(7);
    world.login = Some(Arc::new(aurora_net::LoginResponse {
        agent_id: world.agent_id,
        session_id: id(101),
        secure_session_id: Uuid::nil(),
        first_name: String::new(),
        last_name: String::new(),
        sim_ip: std::net::Ipv4Addr::LOCALHOST,
        sim_port: 0,
        circuit_code: 0,
        region_x: 0,
        region_y: 0,
        region_size_x: 256,
        region_size_y: 256,
        seed_capability: String::new(),
        look_at: glam::Vec3::X,
        message: String::new(),
        agent_appearance_service: String::new(),
        inventory_root: id(1),
        mfa_hash: None,
        raw: aurora_llsd::Llsd::Map(Default::default()),
    }));
    for (n, parent, kind, library) in [
        (1, 0, 8, false),
        (2, 1, -1, false),
        (3, 2, -1, false),
        (4, 1, 47, false),
        (5, 1, 14, false),
        (6, 5, -1, false),
        (7, 0, 8, true),
        (8, 7, -1, false),
    ] {
        folder(&mut world, n, parent, kind, library);
    }
    world.inventory.items.insert(
        id(9),
        InvItem {
            id: id(9),
            parent: id(2),
            owner: world.agent_id,
            ..Default::default()
        },
    );
    world.inventory.generation = 1;
    world
}
fn ready(world: &World) -> Model {
    let mut model = Model::new(world, grid("https://archive.example.invalid/login")).unwrap();
    assert!(model.complete_load(model.load_ticket().unwrap(), Ok(None)));
    model
}
fn save(model: &mut Model, world: &World, item: u128, desired: bool) -> SaveIntent {
    let intent = model
        .begin_save(world, model.prepare_edit(world, id(item)).unwrap(), desired)
        .unwrap();
    assert!(model.complete_save(&intent, true));
    intent
}
fn restriction(world: &mut World, text: &str) {
    assert!(world.rlva.receive(&aurora_net::ChatMessage {
        from_name: "Synthetic archive restriction".into(),
        source_id: id(200),
        owner_id: world.agent_id,
        source_type: aurora_net::ChatSourceType::Object,
        chat_type: aurora_net::ChatType::OwnerSay,
        position: glam::Vec3::ZERO,
        message: text.into(),
    }));
}

#[path = "projection_tests.rs"]
mod projection_tests;

#[path = "runtime_tests.rs"]
mod runtime_tests;

#[path = "adapter_tests.rs"]
mod adapter_tests;

#[test]
fn inventory_archive_eligibility_uses_actual_ordinary_outfit_own_ancestry_without_fetches() {
    let mut world = world();
    for n in [2, 3, 4] {
        assert!(eligible(&world.inventory, id(n)));
    }
    for n in [0, 1, 5, 6, 7, 8, 9, 999] {
        assert!(!eligible(&world.inventory, id(n)));
    }
    // Actual Library identity is stronger than a missing/mislabelled flag.
    world.inventory.folders.get_mut(&id(7)).unwrap().library = false;
    world.inventory.folders.get_mut(&id(7)).unwrap().info.parent = id(1);
    assert!(!eligible(&world.inventory, id(8)));
    world.inventory.folders.get_mut(&id(7)).unwrap().info.type_default = -1;
    assert!(!eligible(&world.inventory, id(7)));
    for kind in 0..=100 {
        folder(&mut world, 1000 + kind as u128, 1, kind, false);
        assert_eq!(eligible(&world.inventory, id(1000 + kind as u128)), kind == 47);
    }
    world.inventory.folders.get_mut(&id(2)).unwrap().info.parent = id(3);
    assert!(!eligible(&world.inventory, id(3)), "bounded cycle fails closed");
    world.inventory.folders.get_mut(&id(2)).unwrap().info.parent = id(999);
    assert!(!eligible(&world.inventory, id(3)));
    world.inventory.folders.get_mut(&id(2)).unwrap().info.parent = id(1);
    world.inventory.folders.get_mut(&id(2)).unwrap().info.id = id(999);
    assert!(!eligible(&world.inventory, id(3)), "map-key aliases are not actual folders");
    world.inventory.lib_root = world.inventory.root;
    assert!(
        !eligible(&world.inventory, id(4)),
        "personal/Library root alias is not personal ancestry"
    );
    assert!(
        world.inventory.queue.is_empty(),
        "Unknown content is not fetched by local eligibility"
    );
}

#[test]
fn inventory_archive_target_depth_bound_and_missing_root_do_not_grant_eligibility() {
    let mut world = world();
    for n in 1000..1260 {
        folder(&mut world, n, if n == 1000 { 1 } else { n - 1 }, -1, false);
    }
    assert!(eligible(&world.inventory, id(1254)));
    assert!(!eligible(&world.inventory, id(1255)));
    world.inventory.folders.remove(&id(1));
    assert!(!eligible(&world.inventory, id(2)));
    assert!(world.inventory.queue.is_empty());
}

#[test]
fn inventory_archive_missing_load_is_explicit_known_empty_without_encodable_defaults() {
    let world = world();
    let mut model = Model::new(&world, grid("https://archive.example.invalid/login")).unwrap();
    assert_eq!(model.status(), Status::Loading);
    assert!(model.read(&world).is_none());
    assert!(model.include_allowed(&world));
    let load = model.load_ticket().unwrap();
    assert!(model.complete_load(load, Ok(None)));
    assert!(!model.complete_load(load, Err(Error::Invalid)));
    let read = model.read(&world).unwrap();
    assert_eq!(read.folders().len(), 0);
    assert!(!read.marked(id(2)));
    assert_eq!(read.source().account(), world.agent_id);
    assert_eq!(read.source().grid(), grid("https://archive.example.invalid/login"));
    assert_eq!(
        model.published.encode(),
        Err(Error::Invalid),
        "missing defaults never become an eager persisted success"
    );
    assert_eq!(model.published.publication(), 0);
}

#[test]
fn inventory_archive_codec_roundtrip_strict_size_version_scope_and_array_limits() {
    let world = world();
    let mut model = ready(&world);
    let intent = save(&mut model, &world, 2, true);
    let bytes = intent.next().encode().unwrap();
    assert!(bytes.len() <= MAX_BYTES);
    assert_eq!(Document::decode(&bytes, model.source()).unwrap(), *intent.next());
    let good: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for case in 0..11 {
        let mut value = good.clone();
        match case {
            0 => value["version"] = 99.into(),
            1 => value["publication"] = 0.into(),
            2 => value["unknown"] = true.into(),
            3 => value["account"] = id(999).to_string().into(),
            4 => value["grid"] = serde_json::to_value([0u8; 32]).unwrap(),
            5 => value["grid"] = serde_json::json!([1, 2]),
            6 => value["folders"] = serde_json::json!([id(2).to_string(), id(2).to_string()]),
            7 => value["folders"] = serde_json::json!([Uuid::nil().to_string()]),
            8 => value["folders"] = serde_json::json!({"folder":true}),
            9 => value["folders"] = serde_json::json!(["not-a-uuid"]),
            10 => value["folders"] = serde_json::to_value((1000..1000 + MAX_FLAGS + 1).map(|n| id(n as u128)).collect::<Vec<_>>()).unwrap(),
            _ => unreachable!(),
        }
        assert!(
            Document::decode(&serde_json::to_vec(&value).unwrap(), model.source()).is_err(),
            "case {case}"
        );
    }
    assert_eq!(Document::decode(&vec![b' '; MAX_BYTES + 1], model.source()), Err(Error::Capacity));
    assert!(Document::decode(b"malformed", model.source()).is_err());
    assert!(Document::decode(&[bytes.as_slice(), b" trailing"].concat(), model.source()).is_err());
    let duplicate = String::from_utf8(bytes)
        .unwrap()
        .replacen("\"version\":1", "\"version\":1,\"version\":1", 1);
    assert!(Document::decode(duplicate.as_bytes(), model.source()).is_err());
    assert_eq!(intent.expected().publication(), 0);
    assert_eq!(intent.next().publication(), 1);
}

#[test]
fn inventory_archive_load_reply_old_instance_cannot_replace_current_source_or_error() {
    let mut world = world();
    let old = Model::new(&world, grid("https://archive.example.invalid/login")).unwrap();
    let old_ticket = old.load_ticket().unwrap();
    Arc::make_mut(world.login.as_mut().unwrap()).session_id = id(999);
    let mut current = Model::new(&world, grid("https://archive.example.invalid/login")).unwrap();
    assert!(!current.complete_load(old_ticket, Ok(None)));
    assert_eq!(current.status(), Status::Loading);
    assert!(current.complete_load(current.load_ticket().unwrap(), Err(Error::Invalid)));
    assert_eq!(current.status(), Status::LoadFailed);
    assert!(current.read(&world).is_none());
    assert!(current.include_allowed(&world));
    assert!(!current.complete_load(old_ticket, Ok(None)));
    assert_eq!(current.status(), Status::LoadFailed);
    assert!(current.prepare_edit(&world, id(2)).is_none());
}

#[test]
fn inventory_archive_wrong_document_or_actual_login_root_never_becomes_ready_empty() {
    let mut world = world();
    let mut zero = Model::new(&world, grid("https://archive.example.invalid/login")).unwrap();
    assert!(zero.complete_load(zero.load_ticket().unwrap(), Ok(Some(Document::empty(zero.source())))));
    assert_eq!(zero.status(), Status::LoadFailed, "only Missing(None) may publish zero defaults");
    assert!(zero.read(&world).is_none());
    let mut current = Model::new(&world, grid("https://archive.example.invalid/login")).unwrap();
    let mut wrong = Document::empty(current.source());
    wrong.account = id(999);
    wrong.publication = 1;
    assert!(current.complete_load(current.load_ticket().unwrap(), Ok(Some(wrong))));
    assert_eq!(current.status(), Status::LoadFailed);
    assert!(current.read(&world).is_none());
    assert!(
        current.include_allowed(&world),
        "explicit Include is independent of failed mask readiness"
    );
    world.inventory.lib_root = world.inventory.root;
    assert!(!current.matches(&world) && !current.include_allowed(&world));
    assert!(Model::new(&world, grid("https://archive.example.invalid/login")).is_err());
    world.inventory.lib_root = id(7);
    world.inventory.folders.get_mut(&id(1)).unwrap().info.type_default = 14;
    assert!(!current.matches(&world) && !current.include_allowed(&world));
    assert!(Model::new(&world, grid("https://archive.example.invalid/login")).is_err());
    world.inventory.folders.get_mut(&id(1)).unwrap().info.type_default = 8;
    Arc::make_mut(world.login.as_mut().unwrap()).inventory_root = id(999);
    assert!(!current.matches(&world) && !current.include_allowed(&world));
    assert!(Model::new(&world, grid("https://archive.example.invalid/login")).is_err());
}

#[test]
fn inventory_archive_save_is_not_optimistic_failure_keeps_flags_and_fresh_retry_consumes_old_ticket() {
    let world = world();
    let mut model = ready(&world);
    save(&mut model, &world, 2, true);
    let ticket = model.prepare_edit(&world, id(3)).unwrap();
    let original = model.read(&world).unwrap().revision();
    let intent = model.begin_save(&world, ticket.clone(), true).unwrap();
    assert_eq!(model.status(), Status::Saving);
    assert!(!model.read(&world).unwrap().marked(id(3)));
    assert_eq!(model.begin_save(&world, ticket.clone(), true), Err(Error::Busy));
    assert!(model.complete_save(&intent, false));
    assert_eq!(model.status(), Status::SaveFailed);
    assert_eq!(model.read(&world).unwrap().revision(), original);
    assert!(model.read(&world).unwrap().marked(id(2)));
    assert_eq!(model.begin_save(&world, ticket, true), Err(Error::Stale));
    let fresh = save(&mut model, &world, 3, true);
    assert!(fresh.operation() > intent.operation());
    assert!(model.read(&world).unwrap().marked(id(2)) && model.read(&world).unwrap().marked(id(3)));
    assert!(!model.complete_save(&intent, false));
    assert_eq!(model.status(), Status::Ready);
    save(&mut model, &world, 2, false);
    assert!(!model.read(&world).unwrap().marked(id(2)));
    assert!(model.read(&world).unwrap().marked(id(3)));
}

#[test]
fn inventory_archive_edit_rechecks_every_original_field_and_actual_parent_type_owner() {
    let mut world = world();
    let mut model = ready(&world);
    let original = model.prepare_edit(&world, id(2)).unwrap();
    assert!(!original.previous());
    assert_eq!(original.namespace().2, id(2));
    for field in 0..13 {
        let mut ticket = original.clone();
        match field {
            0 => ticket.instance += 1,
            1 => ticket.source.account = id(999),
            2 => ticket.source.session = id(999),
            3 => ticket.source.root = id(999),
            4 => ticket.source.grid = grid("https://other.example.invalid/login"),
            5 => ticket.revision += 1,
            6 => ticket.item = id(4),
            7 => ticket.previous = true,
            8 => ticket.parent = id(999),
            9 => ticket.kind = 47,
            10 => ticket.inventory_generation += 1,
            11 => ticket.rlva_revision += 1,
            12 => ticket.rlva_enabled = !ticket.rlva_enabled,
            _ => unreachable!(),
        }
        assert_eq!(model.begin_save(&world, ticket, true), Err(Error::Stale), "field {field}");
    }
    world.inventory.folders.get_mut(&id(2)).unwrap().info.parent = id(5);
    assert_eq!(model.begin_save(&world, original.clone(), true), Err(Error::Stale));
    world.inventory.folders.get_mut(&id(2)).unwrap().info.parent = id(1);
    world.inventory.folders.get_mut(&id(2)).unwrap().info.type_default = 8;
    assert_eq!(model.begin_save(&world, original.clone(), true), Err(Error::Stale));
    world.inventory.folders.get_mut(&id(2)).unwrap().info.type_default = -1;
    world.inventory.owner = id(999);
    assert_eq!(model.begin_save(&world, original, true), Err(Error::Stale));
    assert!(model.read(&world).is_none() && !model.include_allowed(&world));
}

#[test]
fn inventory_archive_actual_rlva_source_and_generation_roundtrips_do_not_rebase_held_action() {
    let mut world = world();
    world.rlva.configure(true);
    let mut model = ready(&world);
    save(&mut model, &world, 2, true);
    let ticket = model.prepare_edit(&world, id(3)).unwrap();
    let original = model.read(&world).unwrap().revision();
    restriction(&mut world, "@showinv=n");
    assert!(world.rlva.has("showinv"));
    assert!(model.read(&world).is_none() && !model.include_allowed(&world));
    assert_eq!(model.begin_save(&world, ticket.clone(), true), Err(Error::Stale));
    restriction(&mut world, "@showinv=y");
    assert!(!world.rlva.has("showinv"));
    assert_eq!(model.read(&world).unwrap().revision(), original);
    assert_eq!(model.begin_save(&world, ticket, true), Err(Error::Stale));
    let held = model.prepare_edit(&world, id(3)).unwrap();
    world.inventory.generation += 2;
    assert_eq!(model.begin_save(&world, held, true), Err(Error::Stale));
    let held = model.prepare_edit(&world, id(3)).unwrap();
    Arc::make_mut(world.login.as_mut().unwrap()).session_id = id(999);
    assert!(model.read(&world).is_none());
    assert_eq!(model.begin_save(&world, held, true), Err(Error::Stale));
    assert!(!model.matches(&world));
}

#[test]
fn inventory_archive_same_number_old_save_cannot_clear_new_model_pending() {
    let world = world();
    let mut old = ready(&world);
    let old_intent = old.begin_save(&world, old.prepare_edit(&world, id(2)).unwrap(), true).unwrap();
    old.retire();
    let mut current = ready(&world);
    let fresh = current
        .begin_save(&world, current.prepare_edit(&world, id(3)).unwrap(), true)
        .unwrap();
    assert_eq!(old_intent.operation(), 1);
    assert_eq!(fresh.operation(), 1);
    assert_eq!(fresh.source(), current.source());
    assert!(!current.complete_save(&old_intent, false));
    assert_eq!(current.status(), Status::Saving);
    assert!(current.complete_save(&fresh, true));
    assert!(!current.read(&world).unwrap().marked(id(2)));
    assert!(current.read(&world).unwrap().marked(id(3)));
    assert!(!old.complete_save(&old_intent, true));
    assert!(old.read(&world).is_none() && !old.include_allowed(&world));
}

#[test]
fn inventory_archive_whole_intent_provenance_failure_does_not_overwrite_pending_or_published_flags() {
    let world = world();
    let mut model = ready(&world);
    let original = model.begin_save(&world, model.prepare_edit(&world, id(2)).unwrap(), true).unwrap();
    for field in 0..10 {
        let mut wrong = original.clone();
        match field {
            0 => wrong.instance += 1,
            1 => wrong.source.account = id(999),
            2 => wrong.source.session = id(999),
            3 => wrong.source.root = id(999),
            4 => wrong.source.grid = grid("https://other.example.invalid/login"),
            5 => wrong.operation += 1,
            6 => wrong.expected.publication += 1,
            7 => wrong.next.publication += 1,
            8 => wrong.expected.folders = Arc::new(BTreeSet::from([id(4)])),
            9 => wrong.next.folders = Arc::new(BTreeSet::from([id(4)])),
            _ => unreachable!(),
        }
        assert!(!model.complete_save(&wrong, false));
        assert_eq!(model.status(), Status::Saving);
        assert!(!model.read(&world).unwrap().marked(id(2)));
    }
    assert!(model.complete_save(&original, true));
    assert!(model.read(&world).unwrap().marked(id(2)));
}

#[test]
fn inventory_archive_capacity_and_counter_exhaustion_keep_published_mask_without_wrap() {
    let world = world();
    let mut model = ready(&world);
    model.published.folders = Arc::new((1000..1000 + MAX_FLAGS).map(|n| id(n as u128)).collect());
    let ticket = model.prepare_edit(&world, id(2)).unwrap();
    assert_eq!(model.begin_save(&world, ticket, true), Err(Error::Capacity));
    assert_eq!(model.read(&world).unwrap().folders().len(), MAX_FLAGS);
    for publication_exhaustion in [false, true] {
        let mut model = ready(&world);
        save(&mut model, &world, 2, true);
        if publication_exhaustion {
            model.published.publication = u64::MAX;
        } else {
            model.next_operation = u64::MAX;
        }
        let previous = model.read(&world).unwrap().revision();
        let ticket = model.prepare_edit(&world, id(3)).unwrap();
        assert_eq!(model.begin_save(&world, ticket, true), Err(Error::Exhausted));
        assert_eq!(model.status(), Status::ReadOnly);
        assert_eq!(model.read(&world).unwrap().revision(), previous);
        assert!(model.read(&world).unwrap().marked(id(2)));
        assert!(model.prepare_edit(&world, id(3)).is_none());
        assert!(model.pending.is_none());
    }
}

#[test]
fn inventory_archive_retire_never_revives_after_source_returns_and_region_change_is_not_new_source() {
    let mut world = world();
    let mut model = ready(&world);
    save(&mut model, &world, 2, true);
    let original = model.read(&world).unwrap().revision();
    world.main_region = Some(123);
    world.teleporting = true;
    assert_eq!(
        model.read(&world).unwrap().revision(),
        original,
        "local flags do not depend on camera/region/movement authority"
    );
    model.retire();
    assert_eq!(model.status(), Status::Retired);
    assert!(model.read(&world).is_none() && !model.include_allowed(&world));
    let fresh = ready(&world);
    assert_ne!(fresh.read(&world).unwrap().revision(), original);
    assert!(fresh.read(&world).unwrap().folders().next().is_none());
    assert!(model.load_ticket().is_none());
}
