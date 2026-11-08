use super::*;
use aurora_session::chat::ReadOutcome;

const GROUP: Uuid = Uuid::from_u128(500);

fn group() -> Conversation {
    Conversation::Group(conversation_id(GROUP).unwrap())
}
fn group_world() -> World {
    let mut world = world();
    world.social.session_mut(GROUP);
    world.groups.groups.push(aurora_net::GroupMembership {
        id: GROUP,
        name: "Synthetic group".into(),
        insignia: Uuid::nil(),
        powers: 0,
        accept_notices: true,
        list_in_profile: false,
        contribution: 0,
    });
    let mut info = crate::world::groups::SessionInfo::default();
    info.name = "Synthetic group".into();
    info.group = true;
    info.active = true;
    world.groups.sessions.insert(GROUP, info);
    world
}
fn incoming(world: &mut World, text: &str) {
    world.apply(aurora_net::NetEvent::InstantMessage(aurora_net::InstantMessage {
        from_agent_id: PEER,
        from_name: "Synthetic Resident".into(),
        to_agent_id: OWNER,
        dialog: crate::world::groups::dialog::SESSION_SEND,
        session_id: GROUP,
        message: text.into(),
        offline: false,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp: 0,
    }));
}
fn page(world: &World) -> NativePage {
    let (_, clock) = native_lines(world, group()).unwrap();
    let cursor = NativeCursor {
        token: clock.token,
        next: clock.first,
    };
    lines(world, group(), cursor, SCAN_LINES)
}
fn native() -> Harness {
    Harness {
        world: group_world(),
        source: Some(source(1)),
        sent: Vec::new(),
    }
}

#[test]
fn shared_group_native_requires_existing_initialized_group_and_transcript() {
    for fault in 0..6 {
        let mut world = group_world();
        match fault {
            0 => {
                world.groups.sessions.remove(&GROUP);
            }
            1 => world.groups.sessions.get_mut(&GROUP).unwrap().active = false,
            2 => world.groups.sessions.get_mut(&GROUP).unwrap().closed = true,
            3 => world.groups.sessions.get_mut(&GROUP).unwrap().group = false,
            4 => world.social.ims.retain(|session| session.other != GROUP),
            _ => world.groups.groups.clear(),
        }
        assert!(!catalogue(&world).contains(&group()), "fault {fault}");
        assert!(snapshot(&world, group(), true).is_none());
        assert!(literal_command(&world, group(), "No join").is_none());
        assert!(world.groups.out.is_empty());
        assert_eq!(world.social.focus_im, None);
    }
}

#[test]
fn shared_group_native_exports_live_text_and_sends_only_literal_session_messages() {
    let mut world = group_world();
    incoming(&mut world, "Synthetic incoming");
    assert!(catalogue(&world).contains(&group()));
    assert!(!catalogue(&world).contains(&Conversation::Direct(conversation_id(GROUP).unwrap())));
    assert_eq!(page(&world).lines[0].text.as_str(), "Synthetic incoming");
    let policy = snapshot(&world, group(), true).unwrap();
    assert!(policy.ready && policy.can_read && policy.can_send);
    let command = literal_command(&world, group(), "/ao off :literal:").unwrap();
    assert!(
        matches!(command, NetCommand::SendImDialog { to: GROUP, dialog: 17, id: GROUP, ref message, ref bucket }
        if message == "/ao off :literal:" && bucket.is_empty())
    );
    world.own_shared_group_im(GROUP, "/ao off :literal:");
    let session = world.social.ims.iter().find(|session| session.other == GROUP).unwrap();
    assert_eq!(session.lines.back().unwrap().text, "/ao off :literal:");
    assert!(matches!(session.lines.back().unwrap().kind, ChatKind::Own(_)));
    assert_eq!(session.unread, 1, "own echo never marks read");
    assert!(world.groups.out.is_empty(), "no join/unblock command");
    assert_eq!(world.social.focus_im, None);
}

#[test]
fn shared_group_native_rechecks_group_and_individual_receive_restrictions() {
    for rule in [
        "@recvim=n",
        "@recvimfrom:00000000-0000-0000-0000-000000000037=n",
        "@recvimfrom:00000000-0000-0000-0000-0000000001f4=n",
    ] {
        let mut world = group_world();
        incoming(&mut world, "Private group body");
        restrict(&mut world, rule);
        let session = world.social.ims.iter().find(|session| session.other == GROUP).unwrap();
        assert_eq!(
            session.lines.back().unwrap().text,
            "Private group body",
            "native storage is untouched"
        );
        assert!(page(&world).lines.is_empty(), "export checks {rule}");
    }
    let mut world = group_world();
    incoming(&mut world, "No location disclosure");
    restrict(&mut world, "@showloc=n");
    let snapshot = snapshot(&world, group(), true).unwrap();
    assert!(!snapshot.can_read);
    assert_ne!(snapshot.title.as_str(), "Synthetic group");
    assert!(page(&world).lines.is_empty());
}

#[test]
fn shared_group_native_send_rechecks_moderator_mute_group_mute_and_rlva() {
    for fault in 0..4 {
        let mut world = group_world();
        match fault {
            0 => world.groups.sessions.get_mut(&GROUP).unwrap().text_muted = true,
            1 => restrict(&mut world, "@sendim=n"),
            2 => world.groups.mute_all_groups = true,
            _ => {
                world
                    .mutes
                    .add(
                        crate::world::mutes::Mute {
                            id: GROUP,
                            name: "Synthetic group".into(),
                            kind: crate::world::mutes::MuteType::Group,
                            flags: 0,
                        },
                        0,
                    )
                    .unwrap();
            }
        }
        assert!(literal_command(&world, group(), "Blocked literal").is_none(), "fault {fault}");
        world.own_shared_group_im(GROUP, "No raw echo");
        assert!(
            world
                .social
                .ims
                .iter()
                .find(|session| session.other == GROUP)
                .unwrap()
                .lines
                .is_empty()
        );
        assert!(world.groups.out.is_empty());
        assert_eq!(world.social.focus_im, None);
    }
}

#[test]
fn shared_group_unready_native_policy_does_not_export_raw_title_or_text() {
    let mut world = group_world();
    incoming(&mut world, "Retained body");
    for ui_ready in [false, true] {
        world.mutes.state = if ui_ready {
            crate::world::mutes::LoadState::Initial
        } else {
            crate::world::mutes::LoadState::Loaded
        };
        let snapshot = snapshot(&world, group(), ui_ready).unwrap();
        assert!(!snapshot.ready && !snapshot.can_read && !snapshot.can_send);
        assert_ne!(snapshot.title.as_str(), "Synthetic group");
        if ui_ready {
            assert!(page(&world).lines.is_empty());
        }
    }
}

#[test]
fn shared_group_worker_exports_codec_valid_rows_and_preserves_native_read_cas() {
    let mut native = native();
    incoming(&mut native.world, "A");
    let mut backend = WorkerChat::default();
    let a = capture_read(&mut native, &mut backend, group(), 1);
    incoming(&mut native.world, "B");
    assert_eq!(apply_read(&mut native, &mut backend, a, 11).outcome, ReadOutcome::StaleObservation);
    let ab = capture_read(&mut native, &mut backend, group(), 2);
    let receipt = apply_read(&mut native, &mut backend, ab, 21);
    assert_eq!(receipt.outcome, ReadOutcome::Applied);
    incoming(&mut native.world, "C");
    assert_eq!(apply_read(&mut native, &mut backend, ab, 22), receipt);
    assert_eq!(
        native
            .world
            .social
            .ims
            .iter()
            .find(|session| session.other == GROUP)
            .unwrap()
            .unread,
        1
    );
    for packet in backend.handle(&mut native, &poll(source(1), 30)).unwrap() {
        aurora_session::chat::encode(&packet).unwrap();
    }
    assert!(native.sent.is_empty(), "read never sends or joins");
}

#[test]
fn shared_group_worker_rejects_stale_permission_send_and_preserves_other_conversations() {
    let mut native = native();
    let mut backend = WorkerChat::default();
    backend.sync(&native).unwrap();
    let rows = backend.handle(&mut native, &poll(source(1), 1)).unwrap();
    let revision = rows
        .iter()
        .find_map(|packet| match &packet.message {
            ChatMessage::Event(ChatEvent::ReadBatch(rows)) => {
                rows.iter().find(|row| row.conversation == group()).map(|row| row.policy_revision)
            }
            _ => None,
        })
        .unwrap();
    native.world.groups.sessions.get_mut(&GROUP).unwrap().text_muted = true;
    let replies = backend
        .handle(&mut native, &send(source(1), 2, group(), revision, "Never send"))
        .unwrap();
    assert!(matches!(&replies[0].message, ChatMessage::Event(ChatEvent::Receipt(receipt))
        if receipt.outcome != SendOutcome::AcceptedLocally));
    assert!(native.sent.is_empty());
    assert!(catalogue(&native.world).contains(&Conversation::Direct(conversation_id(PEER).unwrap())));
    assert!(native.world.social.has_session(&GROUP));
    assert!(native.world.groups.out.is_empty());
}

#[test]
fn shared_group_worker_successful_literal_send_and_receipt_replay_submit_once() {
    let mut native = native();
    let mut backend = WorkerChat::default();
    backend.sync(&native).unwrap();
    let replies = backend.handle(&mut native, &poll(source(1), 1)).unwrap();
    let revision = replies
        .iter()
        .find_map(|packet| match &packet.message {
            ChatMessage::Event(ChatEvent::ReadBatch(rows)) => {
                rows.iter().find(|row| row.conversation == group()).map(|row| row.policy_revision)
            }
            _ => None,
        })
        .unwrap();
    let mut request = send(source(1), 2, group(), revision, "/ao off :literal:");
    let receipt = backend.handle(&mut native, &request).unwrap();
    assert!(matches!(&receipt[0].message, ChatMessage::Event(ChatEvent::Receipt(receipt))
        if receipt.outcome == SendOutcome::AcceptedLocally));
    request.sequence = 3;
    let replay = backend.handle(&mut native, &request).unwrap();
    assert_eq!(replay[0].message, receipt[0].message);
    assert_eq!(native.sent.len(), 1);
    assert!(
        matches!(&native.sent[0], NetCommand::SendImDialog { to: GROUP, dialog: 17, id: GROUP, message, bucket }
        if message == "/ao off :literal:" && bucket.is_empty())
    );
    let session = native.world.social.ims.iter().find(|session| session.other == GROUP).unwrap();
    assert_eq!(session.lines.len(), 1, "one native echo, no replay");
    assert_eq!(session.lines[0].text, "/ao off :literal:");
    assert_eq!(session.unread, 0);
    assert!(native.world.groups.out.is_empty());
    assert_eq!(native.world.social.focus_im, None);
}
