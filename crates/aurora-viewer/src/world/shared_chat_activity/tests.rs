use super::*;
use std::sync::Arc;
use std::time::SystemTime;
const OWNER: Uuid = Uuid::from_u128(0x991);
const PEER: Uuid = Uuid::from_u128(0x992);
fn login(agent: Uuid, session: Uuid) -> Arc<aurora_net::LoginResponse> {
    Arc::new(aurora_net::LoginResponse {
        agent_id: agent,
        session_id: session,
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
        inventory_root: Uuid::nil(),
        mfa_hash: None,
        raw: aurora_llsd::llsd_map! {},
    })
}
fn world() -> World {
    let mut w = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    // Bind the exact actual LoggedIn producer hook without invoking the old
    // account-filesystem loads of the full LoggedIn event in this unit fixture.
    w.login = Some(login(OWNER, Uuid::from_u128(0x993)));
    w.agent_id = OWNER;
    w.bind_shared_chat_activity(OWNER, w.login.as_ref().unwrap().session_id);
    w.mutes.state = crate::world::mutes::LoadState::Loaded;
    w
}
fn line() -> ChatLine {
    ChatLine {
        time: SystemTime::now(),
        from: "Synthetic resident".into(),
        text: "Synthetic incoming".into(),
        kind: ChatKind::Im,
        source: PEER,
        origin: Origin::Live,
    }
}
fn im(w: &mut World, from: Uuid, dialog: u8) {
    w.apply(aurora_net::NetEvent::InstantMessage(aurora_net::InstantMessage {
        from_agent_id: from,
        from_name: "Synthetic resident".into(),
        to_agent_id: OWNER,
        dialog,
        session_id: from,
        message: "Synthetic incoming".into(),
        offline: false,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp: 0,
    }));
}
#[test]
fn close_activity_actual_im_first_message_recreation_and_export_reads_do_not_remint() {
    let mut w = world();
    let zero = w.shared_chat_close_fence().unwrap();
    assert_eq!(zero.serial, 0);
    assert_eq!(w.shared_chat_activity(PEER), None);
    im(&mut w, PEER, 0);
    let first = w.shared_chat_activity(PEER).unwrap();
    assert_eq!(first.serial, 1);
    assert_eq!(first.newer_than(zero), Some(true));
    let unread = w.social.ims.iter().find(|s| s.other == PEER).unwrap().unread;
    for _ in 0..128 {
        assert_eq!(w.shared_chat_activity(PEER), Some(first));
        assert_eq!(w.shared_chat_close_fence(), Some(first));
    }
    assert_eq!(w.social.ims.iter().find(|s| s.other == PEER).unwrap().unread, unread);
    w.social.ims.retain(|s| s.other != PEER);
    assert_eq!(w.shared_chat_activity(PEER), None);
    im(&mut w, PEER, 0);
    assert_eq!(w.shared_chat_activity(PEER).unwrap().serial, 2);
    let second = w.shared_chat_activity(PEER).unwrap();
    im(&mut w, Uuid::from_u128(0x994), 0);
    assert_eq!(w.shared_chat_activity(PEER), Some(second));
    assert!(w.shared_chat_close_fence().unwrap().serial > second.serial);
}
#[test]
fn close_activity_history_own_system_cards_groups_metadata_and_nearby_do_not_mint() {
    let mut w = world();
    let zero = w.shared_chat_close_fence();
    for origin in [Origin::Log, Origin::ServerLog] {
        let mut old = line();
        old.origin = origin;
        w.conversation_push(PEER, old, false);
    }
    for (kind, from) in [(ChatKind::System, PEER), (ChatKind::Im, OWNER), (ChatKind::Im, Uuid::nil())] {
        let mut excluded = line();
        excluded.kind = kind;
        excluded.source = from;
        w.conversation_push(PEER, excluded, true);
    }
    w.conversation_line(PEER, line());
    w.conversation_card(PEER, line());
    w.own_im(PEER, "Synthetic own line");
    w.system_message("Synthetic system message");
    w.social.names.insert(PEER, "Renamed resident".into());
    w.groups.sessions.insert(PEER, crate::world::groups::SessionInfo::default());
    w.conversation_push(PEER, line(), true);
    assert_eq!(w.shared_chat_close_fence(), zero);
    assert_eq!(w.shared_chat_activity(PEER), None);
}
#[test]
fn close_activity_current_privacy_and_mutes_exclude_input_without_later_backfill() {
    for rule in ["@shownames=n", "@showloc=n", "@shownearby=n", "@recvim=n"] {
        let mut w = world();
        let zero = w.shared_chat_close_fence();
        w.rlva.configure(true);
        w.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Synthetic controller".into(),
            source_id: Uuid::from_u128(0x995),
            owner_id: OWNER,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: rule.into(),
        }));
        im(&mut w, PEER, 0);
        assert_eq!(w.shared_chat_close_fence(), zero, "{rule}");
        assert_eq!(w.shared_chat_activity(PEER), None);
        w.rlva.configure(false);
        assert_eq!(w.shared_chat_activity(PEER), None);
        im(&mut w, PEER, 0);
        assert_eq!(w.shared_chat_activity(PEER).unwrap().serial, 1);
    }
    let mut w = world();
    let zero = w.shared_chat_close_fence();
    w.mutes
        .add(
            crate::world::mutes::Mute {
                id: PEER,
                name: "Synthetic resident".into(),
                kind: crate::world::mutes::MuteType::Agent,
                flags: 0,
            },
            0,
        )
        .unwrap();
    im(&mut w, PEER, 0);
    assert_eq!(w.shared_chat_close_fence(), zero);
    assert_eq!(w.shared_chat_activity(PEER), None);
}
#[test]
fn close_activity_native_source_aba_and_checked_exhaustion_fail_closed() {
    let mut w = world();
    im(&mut w, PEER, 0);
    let original = w.shared_chat_activity(PEER).unwrap();
    let session = w.login.as_ref().unwrap().session_id;
    // The actual LoggedIn hook runs even on identical native IDs. Old native
    // session metadata may never be relabelled into the new stream.
    w.bind_shared_chat_activity(OWNER, session);
    let replacement = w.shared_chat_close_fence().unwrap();
    assert_ne!(replacement.stream, original.stream);
    assert_eq!(replacement.serial, 0);
    assert_eq!(w.shared_chat_activity(PEER), None);
    assert_eq!(replacement.newer_than(original), None);
    w.login = Some(login(OWNER, Uuid::from_u128(0x996)));
    assert_eq!(w.shared_chat_close_fence(), None);
    w.login = Some(login(OWNER, session));
    w.shared_chat_incoming = Some(NativeIncoming {
        serial: u64::MAX,
        ..replacement
    });
    w.conversation_push(PEER, line(), true);
    assert_eq!(w.shared_chat_close_fence(), None);
    w.conversation_push(PEER, line(), true);
    assert_eq!(w.shared_chat_close_fence(), None);
    assert_eq!(w.shared_chat_activity(PEER), None);
    w.bind_shared_chat_activity(OWNER, session);
    assert_eq!(w.shared_chat_close_fence().unwrap().serial, 0);
    w.agent_id = Uuid::from_u128(0x997);
    assert_eq!(w.shared_chat_close_fence(), None);
    w.reset();
    assert_eq!(w.shared_chat_close_fence(), None);
    assert_eq!(w.shared_chat_activity(PEER), None);
}
