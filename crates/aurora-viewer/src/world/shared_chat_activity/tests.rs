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

fn initialized_group(w: &mut World, group: Uuid) {
    w.groups.groups.push(aurora_net::GroupMembership {
        id: group,
        name: "Synthetic group".into(),
        insignia: Uuid::nil(),
        powers: 0,
        accept_notices: true,
        list_in_profile: false,
        contribution: 0,
    });
    let mut info = crate::world::groups::SessionInfo::default();
    info.group = true;
    info.active = true;
    w.groups.sessions.insert(group, info);
    w.session_lines(group);
}

fn group_im(w: &mut World, group: Uuid) {
    w.apply(aurora_net::NetEvent::InstantMessage(aurora_net::InstantMessage {
        from_agent_id: PEER,
        from_name: "Synthetic resident".into(),
        to_agent_id: OWNER,
        dialog: crate::world::groups::dialog::SESSION_SEND,
        session_id: group,
        message: "Synthetic group incoming".into(),
        offline: false,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp: 0,
    }));
}

#[test]
fn group_close_activity_actual_udp_is_per_conversation_not_a_shared_unread_counter() {
    let mut w = world();
    let group = Uuid::from_u128(0x996);
    let other = Uuid::from_u128(0x997);
    initialized_group(&mut w, group);
    initialized_group(&mut w, other);
    let zero = w.shared_chat_close_fence().unwrap();
    group_im(&mut w, group);
    let first = w.shared_chat_activity(group).unwrap();
    assert_eq!(first.newer_than(zero), Some(true));
    assert_eq!(w.shared_chat_activity(other), None);
    group_im(&mut w, other);
    im(&mut w, PEER, 0);
    assert_eq!(w.shared_chat_activity(group), Some(first));
    assert!(w.shared_chat_close_fence().unwrap().serial > first.serial);
    let latest = w.shared_chat_close_fence();
    w.own_shared_group_im(group, "Own group echo");
    for _ in 0..32 {
        assert_eq!(w.shared_chat_activity(group), Some(first));
    }
    assert_eq!(w.shared_chat_close_fence(), latest);
    group_im(&mut w, group);
    assert_eq!(w.shared_chat_activity(group).unwrap().newer_than(first), Some(true));
}

#[test]
fn conference_close_saved_direct_and_group_udp_keep_native_unread_without_live_activity_credit() {
    for group in [false, true] {
        let mut world = world();
        let session = if group { Uuid::from_u128(0x996) } else { PEER };
        if group {
            initialized_group(&mut world, session);
        }
        let mut message = aurora_net::InstantMessage {
            from_agent_id: PEER,
            from_name: "Synthetic resident".into(),
            to_agent_id: OWNER,
            dialog: if group { crate::world::groups::dialog::SESSION_SEND } else { 0 },
            session_id: session,
            message: "Live delivery".into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        };
        world.apply(aurora_net::NetEvent::InstantMessage(message.clone()));
        let fence = world.shared_chat_close_fence();
        let before = world.social.ims.iter().find(|entry| entry.other == session).unwrap().unread;
        message.offline = true;
        message.message = "Saved delivery".into();
        world.apply(aurora_net::NetEvent::InstantMessage(message.clone()));
        message.offline = false;
        world.apply(aurora_net::NetEvent::SavedInstantMessage {
            source: aurora_net::LoginSource::from_login(world.login.as_ref().unwrap()),
            message,
        });
        assert_eq!(world.shared_chat_close_fence(), fence);
        assert_eq!(
            world.social.ims.iter().find(|entry| entry.other == session).unwrap().unread,
            before + 2
        );
    }
}

#[test]
fn conference_close_saved_auto_response_preserves_unread_without_reopening_live_fence() {
    let mut world = world();
    let mut message = aurora_net::InstantMessage {
        from_agent_id: PEER,
        from_name: "Synthetic resident".into(),
        to_agent_id: OWNER,
        dialog: 0,
        session_id: PEER,
        message: "Live delivery".into(),
        offline: false,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp: 0,
    };
    world.apply(aurora_net::NetEvent::InstantMessage(message.clone()));
    let fence = world.shared_chat_close_fence();
    let before = world.social.ims.iter().find(|entry| entry.other == PEER).unwrap().unread;
    message.offline = true;
    message.dialog = crate::world::status::AUTO_RESPONSE;
    message.message = "Saved automatic response".into();
    world.apply(aurora_net::NetEvent::InstantMessage(message.clone()));
    assert_eq!(world.shared_chat_close_fence(), fence);
    let session = world.social.ims.iter().find(|entry| entry.other == PEER).unwrap();
    assert_eq!(session.unread, before + 1);
    assert!(session.lines.back().unwrap().text.contains("Saved automatic response"));
    message.offline = false;
    world.apply(aurora_net::NetEvent::SavedInstantMessage {
        source: aurora_net::LoginSource::from_login(world.login.as_ref().unwrap()),
        message: message.clone(),
    });
    assert_eq!(world.shared_chat_close_fence(), fence);
    assert_eq!(
        world.social.ims.iter().find(|entry| entry.other == PEER).unwrap().unread,
        before + 2
    );
    world.apply(aurora_net::NetEvent::InstantMessage(message));
    assert!(world.shared_chat_close_fence().unwrap().serial > fence.unwrap().serial);
}

#[test]
fn group_close_activity_uninitialized_restricted_and_nonincoming_lines_never_mint_or_backfill() {
    let group = Uuid::from_u128(0x996);
    for fault in 0..19 {
        let mut w = world();
        initialized_group(&mut w, group);
        let zero = w.shared_chat_close_fence();
        let mut entry = line();
        match fault {
            0 => w.groups.sessions.get_mut(&group).unwrap().group = false, // Conference.
            1 => w.groups.sessions.get_mut(&group).unwrap().active = false,
            2 => w.groups.sessions.get_mut(&group).unwrap().closed = true,
            3 => {
                w.groups.sessions.remove(&group);
            } // Membership alone is not Direct.
            4 => {
                w.groups.restoring.insert(group);
            }
            5 => {
                assert!(w.snooze_group_chat(group, 60));
            }
            6 => w.groups.mute_all_groups = true,
            7..=10 => {
                let rule = match fault {
                    7 => "@recvim=n",
                    8 => "@recvimfrom:00000000-0000-0000-0000-000000000996=n",
                    9 => "@recvimfrom:00000000-0000-0000-0000-000000000992=n",
                    _ => "@shownames=n",
                };
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
            }
            11 => entry.source = Uuid::nil(),
            12 => entry.source = OWNER,
            13 => entry.kind = ChatKind::ObjectIm,
            14 => entry.origin = Origin::Log,
            15 => entry.kind = ChatKind::System,
            16 => entry.kind = ChatKind::OWN,
            17 => {
                w.mutes
                    .add(
                        crate::world::mutes::Mute {
                            id: PEER,
                            name: entry.from.clone(),
                            kind: crate::world::mutes::MuteType::Agent,
                            flags: 0,
                        },
                        0,
                    )
                    .unwrap();
            }
            _ => w.mutes.state = crate::world::mutes::LoadState::Initial,
        }
        w.conversation_push(group, entry, true);
        assert_eq!(w.shared_chat_close_fence(), zero, "fault {fault}");
        assert_eq!(w.shared_chat_activity(group), None, "fault {fault}");
        if (7..=10).contains(&fault) {
            w.rlva.configure(false);
            assert_eq!(w.shared_chat_activity(group), None, "restoration cannot backfill");
            group_im(&mut w, group);
            assert_eq!(w.shared_chat_activity(group).unwrap().serial, 1);
        }
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
