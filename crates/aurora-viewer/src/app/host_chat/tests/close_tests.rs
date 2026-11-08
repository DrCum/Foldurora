use super::*;
use aurora_session::chat::{
    ChatCommand, ChatEvent, ChatMessage, CloseOutcome, CloseReceiptReply, CloseReceiptState, CloseRequest, NativeIncoming, VERSION,
};
use std::sync::Arc;
use std::time::Duration;
fn native() -> CloseNative {
    let mut world = world();
    let session = Uuid::from_u128(0x811);
    // Exact actual LoggedIn hook, isolated from its unrelated profile loads.
    world.bind_shared_chat_activity(OWNER, session);
    world.login = Some(Arc::new(aurora_net::LoginResponse {
        agent_id: OWNER,
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
    }));
    CloseNative {
        inner: Harness {
            world,
            source: Some(source(1)),
            sent: Vec::new(),
        },
        captures: 0,
    }
}
struct CloseNative {
    inner: Harness,
    captures: usize,
}
impl NativeChat for CloseNative {
    fn source(&self) -> Option<ChatSource> {
        self.inner.source()
    }
    fn catalogue(&self) -> Vec<Conversation> {
        self.inner.catalogue()
    }
    fn snapshot(&self, conversation: Conversation) -> Option<NativeSnapshot> {
        self.inner.snapshot(conversation)
    }
    fn lines(&self, conversation: Conversation, cursor: NativeCursor, limit: usize) -> NativePage {
        self.inner.lines(conversation, cursor, limit)
    }
    fn submit_literal(&mut self, key: ChatKey, text: &str) -> SendOutcome {
        self.inner.submit_literal(key, text)
    }
    fn incoming(&self, conversation: Conversation) -> Option<NativeIncoming> {
        incoming(&self.inner.world, conversation, true)
    }
    fn incoming_stream(&self) -> Option<u64> {
        self.inner.world.shared_chat_close_fence().map(|incoming| incoming.stream)
    }
    fn close_conversation(&mut self, request: CloseRequest) -> CloseOutcome {
        self.captures += 1;
        close_native_conversation(&self.inner.world, request, true, &|| self.inner.source == Some(request.key.source))
    }
}
fn direct(peer: Uuid) -> Conversation {
    Conversation::Direct(conversation_id(peer).unwrap())
}
fn im(native: &mut CloseNative, peer: Uuid) {
    native
        .inner
        .world
        .apply(aurora_net::NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: peer,
            from_name: "Synthetic resident".into(),
            to_agent_id: OWNER,
            dialog: 0,
            session_id: peer,
            message: "Synthetic native incoming".into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        }));
}
fn capture(native: &mut CloseNative, worker: &mut WorkerChat, sequence: u64) -> CloseRequest {
    capture_for(native, worker, direct(PEER), sequence)
}
fn capture_for(native: &mut CloseNative, worker: &mut WorkerChat, conversation: Conversation, sequence: u64) -> CloseRequest {
    let packets = worker.handle(native, &poll(source(1), sequence)).unwrap();
    let row = packets
        .iter()
        .find_map(|packet| match &packet.message {
            ChatMessage::Event(ChatEvent::ReadBatch(rows)) => rows.iter().find(|row| row.conversation == conversation),
            _ => None,
        })
        .unwrap();
    CloseRequest {
        key: ChatKey {
            source: source(1),
            conversation,
        },
        request: sequence,
        policy_revision: row.policy_revision,
        stream: row.incoming_stream.unwrap(),
    }
}

fn group_native() -> (CloseNative, Conversation) {
    let mut native = native();
    let id = Uuid::from_u128(0x812);
    let w = &mut native.inner.world;
    w.groups.groups.push(aurora_net::GroupMembership {
        id,
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
    w.groups.sessions.insert(id, info);
    w.social.session_mut(id);
    (native, Conversation::Group(conversation_id(id).unwrap()))
}
fn group_im(native: &mut CloseNative, conversation: Conversation) {
    let Conversation::Group(id) = conversation else {
        panic!("Group fixture")
    };
    native
        .inner
        .world
        .apply(aurora_net::NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: PEER,
            from_name: "Synthetic resident".into(),
            to_agent_id: OWNER,
            dialog: crate::world::groups::dialog::SESSION_SEND,
            session_id: uuid(id),
            message: "Synthetic group incoming".into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        }));
}

#[test]
fn group_close_actual_worker_fence_is_immutable_and_never_leaves_reads_or_sends() {
    let (mut native, group) = group_native();
    let Conversation::Group(id) = group else { unreachable!() };
    let id = uuid(id);
    let mut worker = WorkerChat::default();
    group_im(&mut native, group);
    let request = capture_for(&mut native, &mut worker, group, 1);
    group_im(&mut native, group); // Before the original native capture.
    let now = Instant::now();
    let original = receipt(worker.handle_received(&mut native, &command(request, 2, false), now, now).unwrap());
    let CloseReceiptState::Terminal(CloseOutcome::Closed(fence)) = original.result else {
        panic!("Group fence")
    };
    assert_eq!(Some(fence), incoming(&native.inner.world, group, true));
    let current = native.inner.world.social.ims.iter().find(|session| session.other == id).unwrap();
    assert_eq!(current.unread, 2, "Close is not Read");
    assert!(native.inner.world.shared_group_initialized(id));
    assert!(native.inner.world.groups.out.is_empty() && native.inner.sent.is_empty());
    assert_eq!(native.inner.world.social.focus_im, None);
    im(&mut native, PEER); // Another conversation must not update this Group.
    assert_eq!(incoming(&native.inner.world, group, true), Some(fence));
    group_im(&mut native, group);
    assert_eq!(incoming(&native.inner.world, group, true).unwrap().newer_than(fence), Some(true));
    let later = now + Duration::from_secs(9);
    assert_eq!(
        receipt(
            worker
                .handle_received(&mut native, &command(request, 3, true), later, later)
                .unwrap()
        ),
        original
    );
    assert_eq!(
        receipt(
            worker
                .handle_received(&mut native, &command(request, 4, false), now, later)
                .unwrap()
        ),
        original
    );
    assert_eq!(native.captures, 1);
    assert!(native.inner.world.groups.out.is_empty() && native.inner.sent.is_empty());
}

#[test]
fn group_close_empty_native_barrier_and_missing_authority_do_not_guess_activity() {
    let (mut native, group) = group_native();
    let mut worker = WorkerChat::default();
    let request = capture_for(&mut native, &mut worker, group, 1);
    assert_eq!(incoming(&native.inner.world, group, true), None);
    assert_eq!(
        close_native_conversation(&native.inner.world, request, true, &|| true),
        CloseOutcome::Closed(NativeIncoming {
            stream: request.stream,
            serial: 0
        })
    );
    assert_eq!(
        close_native_conversation(&native.inner.world, request, true, &|| false),
        CloseOutcome::RejectedNative
    );
    assert_eq!(
        close_native_conversation(&native.inner.world, request, false, &|| true),
        CloseOutcome::RejectedNative
    );
    restrict(&mut native.inner.world, "@recvim=n");
    assert_eq!(
        close_native_conversation(&native.inner.world, request, true, &|| true),
        CloseOutcome::RejectedNative
    );
    assert!(native.inner.world.groups.out.is_empty());
    assert_eq!(native.captures, 0);
}
fn command(request: CloseRequest, sequence: u64, query: bool) -> ChatPacket {
    ChatPacket {
        version: VERSION,
        source: request.key.source,
        sequence,
        reply_to: 0,
        message: ChatMessage::Command(if query {
            ChatCommand::QueryCloseReceipt(request)
        } else {
            ChatCommand::CloseConversation(request)
        }),
    }
}
fn receipt(packets: Vec<ChatPacket>) -> CloseReceiptReply {
    assert_eq!(packets.len(), 1);
    let ChatMessage::Event(ChatEvent::CloseReceipt(receipt)) = packets[0].message else {
        panic!("expected exact close receipt")
    };
    receipt
}
#[test]
fn close_actual_worker_duplicate_and_later_query_keep_original_fence_without_native_read_or_leave() {
    let mut native = native();
    let mut worker = WorkerChat::default();
    im(&mut native, PEER);
    let request = capture(&mut native, &mut worker, 1);
    let count = native.inner.world.social.ims.len();
    let unread = native.inner.world.social.ims[0].unread;
    let now = Instant::now();
    let original = receipt(worker.handle_received(&mut native, &command(request, 2, false), now, now).unwrap());
    let CloseReceiptState::Terminal(CloseOutcome::Closed(through)) = original.result else {
        panic!("native fence unavailable")
    };
    assert_eq!(through, native.inner.world.shared_chat_activity(PEER).unwrap());
    im(&mut native, PEER);
    assert_eq!(
        native.inner.world.shared_chat_activity(PEER).unwrap().newer_than(through),
        Some(true)
    );
    // Recovery has its own fresh ingress after the original deadline. It does
    // not refresh or invoke the original capture, and duplicate Close is fixed.
    let later = now + Duration::from_secs(9);
    assert_eq!(
        receipt(
            worker
                .handle_received(&mut native, &command(request, 3, true), later, later)
                .unwrap()
        ),
        original
    );
    assert_eq!(
        receipt(
            worker
                .handle_received(&mut native, &command(request, 4, false), now, later)
                .unwrap()
        ),
        original
    );
    assert_eq!(native.captures, 1);
    assert_eq!(native.inner.world.social.ims.len(), count);
    assert_eq!(native.inner.world.social.ims[0].unread, unread + 1);
    assert!(native.inner.sent.is_empty());
    let unknown = CloseRequest { request: 2, ..request };
    assert_eq!(
        receipt(
            worker
                .handle_received(&mut native, &command(unknown, 5, true), later, later)
                .unwrap()
        )
        .result,
        CloseReceiptState::Unknown
    );
    assert_eq!(native.captures, 1);
    let session = native.inner.world.login.as_ref().unwrap().session_id;
    native.inner.world.bind_shared_chat_activity(OWNER, session);
    let replacement = native.inner.world.shared_chat_close_fence().unwrap();
    assert_ne!(replacement.stream, through.stream);
    assert_eq!(native.inner.world.shared_chat_activity(PEER), None);
    let lookup = receipt(
        worker
            .handle_received(&mut native, &command(request, 6, true), later, later)
            .unwrap(),
    );
    assert_eq!(lookup, original);
    assert!(!lookup.matches(CloseRequest {
        stream: replacement.stream,
        ..request
    }));
    assert_eq!(native.captures, 1);
}
#[test]
fn close_actual_worker_expired_future_missing_and_stale_policy_do_not_capture() {
    for age in [None, Some(Duration::from_secs(3)), Some(Duration::ZERO)] {
        let mut native = native();
        let mut worker = WorkerChat::default();
        let request = capture(&mut native, &mut worker, 1);
        let now = Instant::now();
        let packets = match age {
            None => worker.handle(&mut native, &command(request, 2, false)),
            Some(age) if age.is_zero() => {
                worker.handle_received(&mut native, &command(request, 2, false), now + Duration::from_secs(1), now)
            }
            Some(age) => worker.handle_received(&mut native, &command(request, 2, false), now, now + age),
        }
        .unwrap();
        assert_eq!(receipt(packets).result, CloseReceiptState::Terminal(CloseOutcome::Expired));
        assert_eq!(native.captures, 0);
        assert_eq!(
            receipt(worker.handle_received(&mut native, &command(request, 3, false), now, now).unwrap()).result,
            CloseReceiptState::Terminal(CloseOutcome::Expired)
        );
        assert_eq!(native.captures, 0);
    }
    let mut native = native();
    let mut worker = WorkerChat::default();
    let request = capture(&mut native, &mut worker, 1);
    restrict(&mut native.inner.world, "@sendim=n");
    let now = Instant::now();
    assert_eq!(
        receipt(worker.handle_received(&mut native, &command(request, 2, false), now, now).unwrap()).result,
        CloseReceiptState::Terminal(CloseOutcome::StaleTarget)
    );
    assert_eq!(native.captures, 0);
}
#[test]
fn close_actual_native_zero_barrier_original_stream_and_final_source_privacy_guards() {
    let mut native = native();
    let mut worker = WorkerChat::default();
    let request = capture(&mut native, &mut worker, 1);
    assert_eq!(incoming(&native.inner.world, direct(PEER), true), None);
    assert_eq!(
        close_native_conversation(&native.inner.world, request, true, &|| true),
        CloseOutcome::Closed(NativeIncoming {
            stream: request.stream,
            serial: 0
        })
    );
    assert_eq!(
        close_native_conversation(&native.inner.world, request, false, &|| true),
        CloseOutcome::RejectedNative
    );
    assert_eq!(
        close_native_conversation(&native.inner.world, request, true, &|| false),
        CloseOutcome::RejectedNative
    );
    let calls = std::cell::Cell::new(0);
    assert_eq!(
        close_native_conversation(&native.inner.world, request, true, &|| {
            let n = calls.get();
            calls.set(n + 1);
            n == 0
        }),
        CloseOutcome::StaleTarget
    );
    let session = native.inner.world.login.as_ref().unwrap().session_id;
    native.inner.world.bind_shared_chat_activity(OWNER, session);
    let current = native.inner.world.shared_chat_close_fence().unwrap();
    assert_ne!(current.stream, request.stream);
    let now = Instant::now();
    let stale = receipt(worker.handle_received(&mut native, &command(request, 2, false), now, now).unwrap());
    assert_eq!(stale.result, CloseReceiptState::Terminal(CloseOutcome::StaleTarget));
    assert_eq!(
        receipt(worker.handle_received(&mut native, &command(request, 3, true), now, now).unwrap()),
        stale
    );
    assert_eq!(native.captures, 1);
    assert_eq!(
        current.newer_than(NativeIncoming {
            stream: request.stream,
            serial: 0
        }),
        None
    );
    let current_request = CloseRequest {
        stream: current.stream,
        ..request
    };
    assert!(!stale.matches(current_request));
    restrict(&mut native.inner.world, "@showloc=n");
    assert_eq!(incoming(&native.inner.world, direct(PEER), true), None);
    assert_eq!(
        close_native_conversation(&native.inner.world, current_request, true, &|| true),
        CloseOutcome::RejectedNative
    );
    assert_eq!(
        worker.handle_received(&mut native, &command(request, 4, true), now, now),
        Err(ChatError::Denied)
    );
    assert!(native.inner.sent.is_empty());
}
#[test]
fn close_activity_existing_fair_rows_export_all_native_catalogue_entries_without_host_view() {
    let mut native = native();
    for value in 1..=62 {
        im(&mut native, Uuid::from_u128(0xa000 + value));
    }
    im(&mut native, PEER);
    let mut worker = WorkerChat::default();
    let mut seen = std::collections::BTreeMap::new();
    for sequence in 1..=64 {
        for packet in worker.handle(&mut native, &poll(source(1), sequence)).unwrap() {
            if let ChatMessage::Event(ChatEvent::ReadBatch(rows)) = packet.message {
                assert!(rows.len() <= aurora_session::chat::MAX_READ_ROWS);
                for row in rows {
                    if let Conversation::Direct(peer) = row.conversation {
                        let expected = native.inner.world.shared_chat_activity(uuid(peer));
                        assert!(expected.is_some());
                        assert_eq!(row.incoming, expected);
                        assert_eq!(row.incoming_stream, expected.map(|stamp| stamp.stream));
                        seen.insert(peer, row.incoming);
                    } else {
                        assert_eq!(row.incoming, None);
                        assert_eq!(row.incoming_stream, None);
                    }
                }
            }
        }
    }
    assert_eq!(seen.len(), 63);
    assert_eq!(native.captures, 0);
    assert!(native.inner.sent.is_empty());
}
