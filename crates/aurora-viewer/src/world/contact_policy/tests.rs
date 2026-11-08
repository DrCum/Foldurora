use super::*;
use crate::world::mutes::{LoadState, MuteType};
use aurora_net::NetEvent;

const OWNER: Uuid = Uuid::from_u128(0xc011_1001);
const SUPPORTER: Uuid = Uuid::from_u128(0xc011_1002);
const OTHER: Uuid = Uuid::from_u128(0xc011_1003);

fn world() -> World {
    let mut w = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    w.bind_session_test_login(OWNER);
    w.contact_policy = State::for_login(w.login.as_ref().unwrap());
    w.mutes.state = LoadState::Loaded;
    w
}
fn patch(flags: &[(Uuid, i32)]) -> Patch {
    Patch {
        agents: flags.iter().copied().collect(),
        legacy: false,
    }
}
fn apply(w: &mut World, feed: Feed, patch: Patch) {
    let reply = w.apply(NetEvent::ContactPolicy {
        source: LoginSource::from_login(w.login.as_ref().unwrap()),
        feed,
        result: Ok(patch),
    });
    assert!(matches!(reply, Some(NetEvent::ContactPolicy { .. })));
}

#[test]
fn contact_policy_hard_refusal_is_shared_before_unblock_echo_single_open_and_typing() {
    let mut w = world();
    w.rlva_configure(false);
    w.block(SUPPORTER, "Support Resident", MuteType::Agent).unwrap();
    w.take_social_commands();
    let s = w.social.session_mut(SUPPORTER);
    s.unread = 7;
    s.input = "Keep this draft".into();
    apply(&mut w, Feed::Agents, patch(&[(OWNER, NO_SUPPORT), (SUPPORTER, SUPPORT)]));
    assert!(w.prepare_native_direct_im(SUPPORTER, "Never echo or transmit").is_none());
    w.own_im(SUPPORTER, "Alternate echo path");
    w.start_text_conference(vec![SUPPORTER]);
    assert!(w.mutes.is_muted_id(&SUPPORTER));
    assert!(w.social.ims[0].lines.is_empty());
    assert_eq!((w.social.ims[0].unread, w.social.ims[0].input.as_str()), (7, "Keep this draft"));
    assert_eq!(w.social.focus_im, None);
    assert!(w.take_social_commands().is_empty());
    for dialog in [0, 41, 42] {
        assert!(!w.contact_command_allowed(&NetCommand::SendImDialog {
            to: SUPPORTER,
            dialog,
            id: Uuid::nil(),
            message: "No placeholder".into(),
            bucket: vec![]
        }));
    }
    for dialog in [17, 20] {
        assert!(w.contact_command_allowed(&NetCommand::SendImDialog {
            to: SUPPORTER,
            dialog,
            id: Uuid::nil(),
            message: "Other protocol".into(),
            bucket: vec![]
        }));
    }
    assert!(!w.contact_command_allowed(&NetCommand::SendIm {
        to: SUPPORTER,
        message: "No text".into()
    }));
    assert!(w.allows_direct_contact(OTHER));
    assert!(!w.contact_command_allowed(&NetCommand::ImReply {
        to: SUPPORTER,
        busy: false,
        offline: true,
        message: "Auto direct".into()
    }));
    assert!(w.contact_command_allowed(&NetCommand::ImReply {
        to: SUPPORTER,
        busy: true,
        offline: false,
        message: "Busy response".into()
    }));
}

#[test]
fn contact_policy_missing_flags_allow_and_modern_explicit_zero_restores_contact() {
    let mut w = world();
    assert!(w.allows_direct_contact(SUPPORTER));
    apply(&mut w, Feed::Data, patch(&[(OWNER, NO_SUPPORT)]));
    assert!(w.allows_direct_contact(SUPPORTER));
    apply(&mut w, Feed::Data, patch(&[(SUPPORTER, SUPPORT)]));
    assert!(!w.allows_direct_contact(SUPPORTER));
    let before = w.contact_policy_revision();
    apply(&mut w, Feed::Data, patch(&[(OWNER, 0)]));
    assert!(w.contact_policy_revision() > before);
    assert!(w.prepare_native_direct_im(SUPPORTER, "Allowed").is_some());
    assert_eq!(w.social.ims[0].lines.back().unwrap().text, "Allowed");
}

#[test]
fn contact_policy_feed_order_failures_missing_rows_and_legacy_do_not_downgrade_known_flags() {
    for order in [[Feed::Data, Feed::Agents], [Feed::Agents, Feed::Data]] {
        let mut w = world();
        for feed in order {
            apply(
                &mut w,
                feed,
                if feed == Feed::Data {
                    patch(&[(OWNER, 0), (SUPPORTER, SUPPORT)])
                } else {
                    patch(&[(OWNER, NO_SUPPORT)])
                },
            );
        }
        assert!(!w.allows_direct_contact(SUPPORTER));
        apply(&mut w, Feed::Agents, patch(&[]));
        apply(
            &mut w,
            Feed::Agents,
            Patch {
                agents: [(OWNER, 0)].into(),
                legacy: true,
            },
        );
        let revision = w.contact_policy_revision();
        assert!(
            w.apply(NetEvent::ContactPolicy {
                source: LoginSource::from_login(w.login.as_ref().unwrap()),
                feed: Feed::Agents,
                result: Err(Error::Transport),
            })
            .is_none()
        );
        assert_eq!(w.contact_policy_revision(), revision);
        assert!(!w.allows_direct_contact(SUPPORTER));
    }
}

#[test]
fn contact_policy_old_login_reply_cannot_change_current_flags_even_when_wire_ids_repeat() {
    let mut w = world();
    let old = w.login.as_ref().unwrap().clone();
    let next = Arc::new((*old).clone());
    w.login = Some(next.clone());
    w.contact_policy = State::for_login(&next);
    let result = w.apply(NetEvent::ContactPolicy {
        source: LoginSource::from_login(&old),
        feed: Feed::Agents,
        result: Ok(patch(&[(OWNER, NO_SUPPORT), (SUPPORTER, SUPPORT)])),
    });
    assert!(result.is_none());
    assert_eq!(w.contact_policy_revision(), 0);
    assert!(w.allows_direct_contact(SUPPORTER));
}

#[test]
fn contact_policy_revision_exhaustion_is_terminal_and_cannot_restore_send_by_wrapping() {
    let mut w = world();
    w.contact_policy.revision = u64::MAX;
    assert!(
        w.contact_policy
            .apply(w.login.as_ref().unwrap(), Feed::Agents, &patch(&[(OWNER, NO_SUPPORT)]))
            .is_err()
    );
    assert!(!w.allows_direct_contact(OTHER));
    assert_eq!(w.contact_policy_revision(), u64::MAX);
    assert!(
        w.contact_policy
            .apply(w.login.as_ref().unwrap(), Feed::Agents, &patch(&[(OWNER, 0)]))
            .is_err()
    );
    assert!(!w.allows_direct_contact(OTHER));
}

#[test]
fn contact_policy_cached_flags_validate_schema_feed_and_captured_grid_without_clearing_on_error() {
    let root = std::env::temp_dir().join(format!("aurora-contact-policy-{}", Uuid::new_v4()));
    let grid = GridDigest::from_uri(aurora_net::login::SL_MAIN_GRID).unwrap();
    let wrong = GridDigest::from_uri(aurora_net::login::SL_BETA_GRID).unwrap();
    let mut w = world();
    apply(&mut w, Feed::Agents, patch(&[(OWNER, NO_SUPPORT), (SUPPORTER, SUPPORT)]));
    w.save_contact_policy_cache(&root, grid, Feed::Agents);
    let file = root.join(cache_file(grid, Feed::Agents));
    assert!(read_cache(&file, grid, Feed::Agents).is_ok());
    assert!(read_cache(&file, wrong, Feed::Agents).is_err());
    assert!(read_cache(&file, grid, Feed::Data).is_err());
    let mut fresh = world();
    fresh.load_contact_policy_cache(&root, grid);
    assert!(!fresh.allows_direct_contact(SUPPORTER));
    std::fs::write(&file, b"corrupt").unwrap();
    fresh.load_contact_policy_cache(&root, grid);
    assert!(!fresh.allows_direct_contact(SUPPORTER));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn contact_policy_accumulation_cannot_truncate_new_refusals_into_permitted_contact() {
    let mut w = world();
    let full = Patch {
        agents: (1..=MAX_AGENTS).map(|id| (Uuid::from_u128(id as u128), 0)).collect(),
        legacy: false,
    };
    apply(&mut w, Feed::Agents, full);
    let result = w
        .contact_policy
        .apply(w.login.as_ref().unwrap(), Feed::Agents, &patch(&[(SUPPORTER, SUPPORT)]));
    assert_eq!(result, Err(Error::TooLarge));
    assert!(!w.allows_direct_contact(SUPPORTER));
    assert!(!w.allows_direct_contact(OTHER));
    assert_eq!(w.contact_policy.feeds[1].len(), MAX_AGENTS);
}

#[test]
fn contact_policy_oversized_network_reply_latches_loss_in_both_feed_orders_and_ignores_old_source() {
    for fail_first in [true, false] {
        let mut w = world();
        let source = LoginSource::from_login(w.login.as_ref().unwrap());
        let oversized = || NetEvent::ContactPolicy {
            source: source.clone(),
            feed: Feed::Data,
            result: Err(Error::TooLarge),
        };
        if fail_first {
            w.apply(oversized());
        }
        apply(&mut w, Feed::Agents, patch(&[(OWNER, 0), (SUPPORTER, SUPPORT)]));
        if !fail_first {
            w.apply(oversized());
        }
        assert!(!w.allows_direct_contact(SUPPORTER));
        assert!(!w.allows_direct_contact(OTHER));
        w.bind_session_test_login(OWNER);
        w.apply(oversized());
        assert!(w.allows_direct_contact(OTHER));
        assert_eq!(w.contact_policy_revision(), 0);
    }
}

#[test]
fn contact_policy_primary_legacy_tracks_lower_modern_zero_through_both_orders_and_cache_reload() {
    let grid = GridDigest::from_uri(aurora_net::login::SL_MAIN_GRID).unwrap();
    for legacy_first in [true, false] {
        let root = std::env::temp_dir().join(format!("aurora-contact-legacy-{}", Uuid::new_v4()));
        let mut w = world();
        let legacy = || Patch {
            agents: [(OWNER, 0), (SUPPORTER, SUPPORT)].into(),
            legacy: true,
        };
        if legacy_first {
            apply(&mut w, Feed::Agents, legacy());
        }
        apply(&mut w, Feed::Data, patch(&[(OWNER, NO_SUPPORT)]));
        if !legacy_first {
            apply(&mut w, Feed::Agents, legacy());
        }
        assert!(!w.allows_direct_contact(SUPPORTER));
        for feed in Feed::ALL {
            w.save_contact_policy_cache(&root, grid, feed);
        }
        let mut reloaded = world();
        reloaded.load_contact_policy_cache(&root, grid);
        assert!(!reloaded.allows_direct_contact(SUPPORTER));
        for model in [&mut w, &mut reloaded] {
            apply(model, Feed::Data, patch(&[(OWNER, 0)]));
            assert!(
                model.allows_direct_contact(SUPPORTER),
                "legacy must not freeze an inherited refusal"
            );
            apply(model, Feed::Agents, patch(&[(OWNER, NO_SUPPORT)]));
            apply(model, Feed::Agents, legacy());
            assert!(
                !model.allows_direct_contact(SUPPORTER),
                "legacy cannot clear actual modern primary flags"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn contact_policy_primary_provenance_change_advances_revision_even_when_bits_are_equal() {
    let mut w = world();
    apply(&mut w, Feed::Data, patch(&[(SUPPORTER, SUPPORT)]));
    apply(
        &mut w,
        Feed::Agents,
        Patch {
            agents: [(OWNER, 0)].into(),
            legacy: true,
        },
    );
    let before = w.contact_policy_revision();
    apply(&mut w, Feed::Agents, patch(&[(OWNER, 0)]));
    assert!(w.contact_policy_revision() > before);
    apply(&mut w, Feed::Data, patch(&[(OWNER, NO_SUPPORT)]));
    assert!(
        w.allows_direct_contact(SUPPORTER),
        "explicit modern primary zero stays authoritative"
    );
}

#[test]
fn contact_policy_mixed_cache_validates_both_maps_before_install_and_rejects_flattened_schema() {
    let root = std::env::temp_dir().join(format!("aurora-contact-cache-{}", Uuid::new_v4()));
    let grid = GridDigest::from_uri(aurora_net::login::SL_MAIN_GRID).unwrap();
    let mut w = world();
    apply(&mut w, Feed::Agents, patch(&[(OWNER, NO_SUPPORT), (SUPPORTER, SUPPORT)]));
    let file = cache_file(grid, Feed::Agents);
    for legacy in [
        aurora_llsd::llsd_map! { OTHER.to_string() => 16 },
        aurora_llsd::llsd_map! { OWNER.to_string().to_uppercase() => aurora_llsd::llsd_map! { "support" => true } },
    ] {
        let bytes = aurora_llsd::to_xml(&aurora_llsd::llsd_map! {
            "schema" => CACHE_SCHEMA, "grid" => grid.namespace(), "feed" => "agents",
            "Agents" => aurora_llsd::llsd_map! { OWNER.to_string() => 0 }, "SupportAgents" => legacy
        });
        write_cache(&root, &file, &bytes).unwrap();
        assert!(read_cache(&root.join(&file), grid, Feed::Agents).is_err());
        w.load_contact_policy_cache(&root, grid);
        assert!(
            !w.allows_direct_contact(SUPPORTER),
            "no partially installed zero from an invalid cache"
        );
    }
    let flattened = aurora_llsd::to_xml(&aurora_llsd::llsd_map! {
        "schema" => 1, "grid" => grid.namespace(), "feed" => "agents",
        "Agents" => aurora_llsd::llsd_map! { OWNER.to_string() => NO_SUPPORT }
    });
    let old = format!("contact-policy-v1-{}-agents.xml", grid.namespace());
    write_cache(&root, &old, &flattened).unwrap();
    write_cache(&root, &file, &flattened).unwrap();
    assert!(read_cache(&root.join(&file), grid, Feed::Agents).is_err());
    w.load_contact_policy_cache(&root, grid);
    assert!(!w.allows_direct_contact(SUPPORTER));
    assert_eq!(
        std::fs::read(root.join(old)).unwrap(),
        flattened,
        "old evidence is not repaired by guessing"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn contact_policy_mixed_cache_applies_one_combined_budget_to_both_maps() {
    let root = std::env::temp_dir().join(format!("aurora-contact-budget-{}", Uuid::new_v4()));
    let grid = GridDigest::from_uri(aurora_net::login::SL_MAIN_GRID).unwrap();
    let mut modern = Llsd::new_map();
    for id in 1..=MAX_AGENTS {
        modern.insert(Uuid::from_u128(id as u128).to_string(), 0);
    }
    let bytes = aurora_llsd::to_xml(&aurora_llsd::llsd_map! {
        "schema" => CACHE_SCHEMA, "grid" => grid.namespace(), "feed" => "agents", "Agents" => modern,
        "SupportAgents" => aurora_llsd::llsd_map! { SUPPORTER.to_string() => aurora_llsd::llsd_map! { "support" => true } }
    });
    let file = cache_file(grid, Feed::Agents);
    write_cache(&root, &file, &bytes).unwrap();
    assert_eq!(read_cache(&root.join(file), grid, Feed::Agents), Err(Error::TooLarge));
    let mut w = world();
    w.load_contact_policy_cache(&root, grid);
    assert!(!w.allows_direct_contact(OTHER));
    std::fs::remove_dir_all(root).unwrap();
}
