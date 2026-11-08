use super::*;
use aurora_llsd::llsd_map;
use aurora_session::chat::{CloseOutcome, CloseRequest, MarkReadRequest, ReadOutcome};

const CONFERENCE: Uuid = Uuid::from_u128(600);
const SECOND: Uuid = Uuid::from_u128(56);

fn conference() -> Conversation {
    Conversation::Conference(conversation_id(CONFERENCE).unwrap())
}
fn roster(muted: bool) -> aurora_llsd::Llsd {
    llsd_map! { "agent_info" => llsd_map! {
        OWNER.to_string() => llsd_map! { "mutes" => llsd_map! { "text" => muted } },
        PEER.to_string() => llsd_map! {}, SECOND.to_string() => llsd_map! {},
    } }
}
fn conference_world() -> World {
    let mut world = world();
    world.bind_session_test_login(OWNER);
    world.start_text_conference(vec![PEER, SECOND]);
    let temporary = *world.groups.sessions.keys().next().unwrap();
    world.apply(aurora_net::NetEvent::SessionStarted {
        temp_session: temporary,
        session: CONFERENCE,
        success: true,
        error: String::new(),
        agents: aurora_net::social::parse_session_agent_batch(&roster(false), aurora_net::social::MAX_SESSION_ROSTER_ENTRIES),
    });
    world.groups.take_commands();
    world.social.focus_im = None;
    world
}
fn incoming(world: &mut World, from: Uuid, text: &str) {
    world.apply(aurora_net::NetEvent::InstantMessage(aurora_net::InstantMessage {
        from_agent_id: from,
        from_name: "Synthetic resident".into(),
        to_agent_id: OWNER,
        dialog: crate::world::groups::dialog::SESSION_SEND,
        session_id: CONFERENCE,
        message: text.into(),
        offline: false,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp: 0,
    }));
}
fn update(world: &mut World, peer: Uuid, transition: &str) {
    world.apply(aurora_net::NetEvent::SessionAgents {
        session: CONFERENCE,
        agents: aurora_net::social::parse_session_agent_batch(
            &llsd_map! {
                "agent_updates" => llsd_map! { peer.to_string() => llsd_map! { "transition" => transition } }
            },
            aurora_net::social::MAX_SESSION_ROSTER_ENTRIES,
        ),
    });
}
fn page(world: &World) -> NativePage {
    let (_, clock) = native_lines(world, conference()).unwrap();
    lines(
        world,
        conference(),
        NativeCursor {
            token: clock.token,
            next: clock.first,
        },
        SCAN_LINES,
    )
}

#[test]
fn shared_conference_native_positive_kind_roster_and_literal_no_create_echo_are_required() {
    let mut world = conference_world();
    assert!(catalogue(&world).contains(&conference()));
    assert!(snapshot(&world, conference(), true).is_some_and(|policy| policy.ready && policy.can_send && policy.can_read));
    let command = literal_command(&world, conference(), "/ao off · 🌕").unwrap();
    assert!(
        matches!(command, NetCommand::SendImDialog { to: CONFERENCE, dialog: 17, id: CONFERENCE, ref message, ref bucket }
        if message == "/ao off · 🌕" && bucket.is_empty())
    );
    let count = world.social.ims.len();
    world.own_shared_conference_im(CONFERENCE, "Literal own echo");
    assert_eq!(world.social.ims.len(), count);
    assert_eq!(
        world.social.ims.iter().find(|session| session.other == CONFERENCE).unwrap().unread,
        0
    );
    assert_eq!(page(&world).lines[0].kind, LineKind::Own);
    assert!(world.groups.out.is_empty());
    assert!(world.social.focus_im.is_none());
    assert!(snapshot(&world, Conversation::Direct(conversation_id(CONFERENCE).unwrap()), true).is_none());
    assert!(snapshot(&world, Conversation::Group(conversation_id(CONFERENCE).unwrap()), true).is_none());
}

#[test]
fn shared_conference_native_all_current_recipients_are_checked_independently_of_partial_presentation() {
    let mut world = conference_world();
    world.groups.sessions.get_mut(&CONFERENCE).unwrap().participants.remove(&SECOND);
    restrict(&mut world, &format!("@sendimto:{SECOND}=n"));
    assert!(!world.shared_conference_sendable(CONFERENCE));
    assert!(literal_command(&world, conference(), "No restricted recipient").is_none());
    world.own_shared_conference_im(CONFERENCE, "No echo");
    assert!(page(&world).lines.is_empty());
    assert!(world.groups.out.is_empty());
}

#[test]
fn shared_conference_native_allowed_membership_changes_invalidate_captured_send_policy_and_permission_changes_reject_it() {
    let context = source(1);
    let mut native = Harness {
        world: conference_world(),
        source: Some(context),
        sent: Vec::new(),
    };
    let mut backend = WorkerChat::default();
    let first = backend.handle(&mut native, &poll(context, 1)).unwrap();
    let revision = first
        .iter()
        .find_map(|packet| match &packet.message {
            ChatMessage::Event(ChatEvent::Conversation(info)) if info.policy.key.conversation == conference() => Some(info.policy.revision),
            _ => None,
        })
        .unwrap();
    let before = snapshot(&native.world, conference(), true).unwrap();
    update(&mut native.world, Uuid::from_u128(57), "ENTER");
    let after = snapshot(&native.world, conference(), true).unwrap();
    assert!(before.can_send && after.can_send);
    assert_ne!(before.stamp, after.stamp);
    let reply = backend
        .handle(&mut native, &send(context, 2, conference(), revision, "Stale member set"))
        .unwrap();
    assert!(reply.iter().any(|packet| matches!(&packet.message, ChatMessage::Event(ChatEvent::Receipt(receipt)) if receipt.outcome != SendOutcome::AcceptedLocally)));
    assert!(native.sent.is_empty());
    restrict(&mut native.world, &format!("@sendimto:{}=n", Uuid::from_u128(57)));
    assert!(literal_command(&native.world, conference(), "No restricted member").is_none());
}

#[test]
fn shared_conference_native_receive_filters_real_senders_and_never_reuses_raw_history_after_privacy_changes() {
    let mut world = conference_world();
    incoming(&mut world, PEER, "Allowed body");
    incoming(&mut world, SECOND, "Restricted body");
    restrict(&mut world, &format!("@recvimfrom:{SECOND}=n"));
    let exported = page(&world);
    assert!(exported.lines.iter().any(|line| line.text.as_str() == "Allowed body"));
    assert!(exported.lines.iter().all(|line| line.text.as_str() != "Restricted body"));
    restrict(&mut world, "@showloc=n");
    assert!(!snapshot(&world, conference(), true).unwrap().can_read);
    assert!(page(&world).lines.is_empty());
}

#[test]
fn shared_conference_native_read_cas_and_close_fence_preserve_native_session_unread_and_outbox() {
    let mut world = conference_world();
    incoming(&mut world, PEER, "First incoming");
    reconcile_read_interest(&mut world);
    let observed = read_metadata(&world, conference(), true).unwrap();
    let request = MarkReadRequest {
        key: ChatKey {
            source: source(1),
            conversation: conference(),
        },
        request: 1,
        policy_revision: 1,
        observed: observed.observed,
        unread_revision: observed.unread_revision,
    };
    incoming(&mut world, SECOND, "Newer incoming");
    assert_eq!(mark_native_read(&mut world, request, true, &|| true), ReadOutcome::StaleObservation);
    let unread = world.social.ims.iter().find(|session| session.other == CONFERENCE).unwrap().unread;
    let stream = world.shared_chat_close_fence().unwrap().stream;
    let close = CloseRequest {
        key: request.key,
        request: 2,
        policy_revision: 1,
        stream,
    };
    assert!(matches!(
        close_native_conversation(&world, close, true, &|| true),
        CloseOutcome::Closed(_)
    ));
    assert!(world.shared_conference_initialized(CONFERENCE));
    assert_eq!(
        world.social.ims.iter().find(|session| session.other == CONFERENCE).unwrap().unread,
        unread
    );
    assert!(world.groups.out.is_empty());
    assert!(world.social.focus_im.is_none());
    let fresh = read_metadata(&world, conference(), true).unwrap();
    assert_eq!(
        mark_native_read(
            &mut world,
            MarkReadRequest {
                request: 3,
                observed: fresh.observed,
                unread_revision: fresh.unread_revision,
                ..request
            },
            true,
            &|| true
        ),
        ReadOutcome::Applied
    );
    assert!(world.shared_conference_initialized(CONFERENCE));
    assert!(world.groups.out.is_empty());
}

#[test]
fn shared_conference_native_incoming_kind_is_not_inferred_from_empty_or_partial_group_data() {
    let mut world = world();
    world.bind_session_test_login(OWNER);
    world.apply(aurora_net::NetEvent::SessionInvite(aurora_net::SessionInvite {
        session: CONFERENCE,
        from: PEER,
        from_name: "Synthetic resident".into(),
        message: "Native invitation".into(),
        session_name: "Unclassified native chat".into(),
        offline: false,
        timestamp: 0,
    }));
    let capture = world
        .groups
        .take_commands()
        .iter()
        .find_map(|command| match command {
            NetCommand::ChatSession {
                method: aurora_net::SessionMethod::AcceptInvitation,
                initialization,
                ..
            } => *initialization,
            _ => None,
        })
        .unwrap();
    world.apply(aurora_net::NetEvent::ChatSessionReply {
        method: aurora_net::SessionMethod::AcceptInvitation,
        session: CONFERENCE,
        initialization: Some(capture),
        result: Some(roster(false)),
        not_found: false,
    });
    world.apply(aurora_net::NetEvent::Groups(Vec::new()));
    assert!(world.session_roster(CONFERENCE).is_some());
    assert!(!catalogue(&world).contains(&conference()));
    assert!(snapshot(&world, conference(), true).is_none());
    world.calls.voice_enabled = true;
    world.apply(aurora_net::NetEvent::VoiceInvite(aurora_net::VoiceInvite {
        session: CONFERENCE,
        session_name: "Known conference".into(),
        from: PEER,
        from_name: "Synthetic resident".into(),
        p2p: false,
        voice: llsd_map! { "invitation_type" => 1, "channel_uri" => "synthetic", "channel_credentials" => "synthetic" },
    }));
    world.answer_call_invite(CONFERENCE, crate::world::calls::InviteAnswer::Accept);
    let capture = world
        .groups
        .take_commands()
        .iter()
        .find_map(|command| match command {
            NetCommand::ChatSession {
                method: aurora_net::SessionMethod::AcceptInvitation,
                initialization,
                ..
            } => *initialization,
            _ => None,
        })
        .unwrap();
    world.apply(aurora_net::NetEvent::ChatSessionReply {
        method: aurora_net::SessionMethod::AcceptInvitation,
        session: CONFERENCE,
        initialization: Some(capture),
        result: Some(roster(false)),
        not_found: false,
    });
    assert!(catalogue(&world).contains(&conference()));
}

#[test]
fn shared_conference_native_unknown_and_muted_live_senders_suspend_send_without_hiding_read_or_close() {
    for muted in [false, true] {
        let mut world = conference_world();
        let peer = Uuid::from_u128(57);
        if muted {
            world
                .block(peer, "Synthetic resident", crate::world::mutes::MuteType::Agent)
                .unwrap();
            world.groups.take_commands();
        }
        incoming(&mut world, peer, "Unconfirmed native member");
        let policy = snapshot(&world, conference(), true).unwrap();
        assert!(policy.ready && policy.can_read && !policy.can_send);
        assert!(literal_command(&world, conference(), "No guessed recipient set").is_none());
        assert!(world.session_roster(CONFERENCE).is_none());
        let close = CloseRequest {
            key: ChatKey {
                source: source(1),
                conversation: conference(),
            },
            request: 1,
            policy_revision: 1,
            stream: world.shared_chat_close_fence().unwrap().stream,
        };
        assert!(matches!(
            close_native_conversation(&world, close, true, &|| true),
            CloseOutcome::Closed(_)
        ));
        update(&mut world, peer, "ENTER");
        assert!(world.session_roster(CONFERENCE).unwrap().participants.contains(&peer));
        assert!(snapshot(&world, conference(), true).unwrap().can_send);
    }
}

#[test]
fn shared_conference_native_sender_suspend_and_absent_leave_cannot_reuse_cached_send_policy_without_an_intermediate_poll() {
    let context = source(1);
    let mut native = Harness {
        world: conference_world(),
        source: Some(context),
        sent: Vec::new(),
    };
    let mut backend = WorkerChat::default();
    let first = backend.handle(&mut native, &poll(context, 1)).unwrap();
    let revision = first
        .iter()
        .find_map(|packet| match &packet.message {
            ChatMessage::Event(ChatEvent::Conversation(info)) if info.policy.key.conversation == conference() => Some(info.policy.revision),
            _ => None,
        })
        .unwrap();
    let before = snapshot(&native.world, conference(), true).unwrap().stamp;
    let peer = Uuid::from_u128(57);
    incoming(&mut native.world, peer, "Unconfirmed sender");
    assert!(native.world.session_roster(CONFERENCE).is_none());
    update(&mut native.world, peer, "LEAVE");
    assert!(native.world.session_roster(CONFERENCE).is_some());
    assert_ne!(snapshot(&native.world, conference(), true).unwrap().stamp, before);
    backend
        .handle(&mut native, &send(context, 2, conference(), revision, "No old policy reuse"))
        .unwrap();
    assert!(native.sent.is_empty());
}

#[test]
fn shared_conference_native_muted_eq_invitation_revokes_recipient_proof_before_body_policy_and_enter_restores_it() {
    let mut world = conference_world();
    let peer = Uuid::from_u128(57);
    world
        .block(peer, "Synthetic resident", crate::world::mutes::MuteType::Agent)
        .unwrap();
    world.groups.take_commands();
    world.apply(aurora_net::NetEvent::SessionInvite(aurora_net::SessionInvite {
        session: CONFERENCE,
        from: peer,
        from_name: "Synthetic resident".into(),
        message: "Dropped body".into(),
        session_name: "Native conference".into(),
        offline: false,
        timestamp: 0,
    }));
    assert!(world.session_roster(CONFERENCE).is_none());
    assert!(snapshot(&world, conference(), true).is_some_and(|policy| policy.can_read && !policy.can_send));
    assert!(page(&world).lines.iter().all(|line| line.text.as_str() != "Dropped body"));
    assert!(literal_command(&world, conference(), "No missing-recipient broadcast").is_none());
    update(&mut world, peer, "ENTER");
    assert!(world.session_roster(CONFERENCE).unwrap().participants.contains(&peer));
}

#[test]
fn shared_conference_native_saved_udp_and_eq_delivery_keep_unread_but_cannot_mint_live_close_activity_or_membership() {
    let mut world = conference_world();
    incoming(&mut world, PEER, "Live incoming");
    let before = world.shared_chat_close_fence().unwrap();
    let unread = world.social.ims.iter().find(|session| session.other == CONFERENCE).unwrap().unread;
    world.apply(aurora_net::NetEvent::InstantMessage(aurora_net::InstantMessage {
        from_agent_id: PEER,
        from_name: "Synthetic resident".into(),
        to_agent_id: OWNER,
        dialog: 17,
        session_id: CONFERENCE,
        message: "Saved message".into(),
        offline: true,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp: 1,
    }));
    world.apply(aurora_net::NetEvent::SessionInvite(aurora_net::SessionInvite {
        session: CONFERENCE,
        from: PEER,
        from_name: "Synthetic resident".into(),
        message: "Saved invitation".into(),
        session_name: "Native conference".into(),
        offline: true,
        timestamp: 1,
    }));
    world.apply(aurora_net::NetEvent::SavedInstantMessage(aurora_net::InstantMessage {
        from_agent_id: Uuid::from_u128(57),
        from_name: "Previously present resident".into(),
        to_agent_id: OWNER,
        dialog: 17,
        session_id: CONFERENCE,
        message: "Saved capability delivery with legacy online flag".into(),
        offline: false,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp: 1,
    }));
    assert_eq!(world.shared_chat_close_fence(), Some(before));
    assert_eq!(world.shared_chat_activity(CONFERENCE), Some(before));
    assert_eq!(
        world.social.ims.iter().find(|session| session.other == CONFERENCE).unwrap().unread,
        unread + 3
    );
    assert!(world.session_roster(CONFERENCE).is_some());
}
