use super::tests::{connected, source};
use super::*;
use crate::session_host::chat_transport::WorkerIo;
use aurora_session::Label;
use aurora_session::chat::{ObservedIncoming, ReadStateRow, ReviewOutcome, ReviewReceipt, ReviewReceiptReply};

fn row() -> ReadStateRow {
    ReadStateRow {
        conversation: Conversation::Nearby,
        title: Label::new("Synthetic nearby").unwrap(),
        policy_revision: 1,
        lease_current: true,
        ready: true,
        can_send: false,
        can_read: true,
        incoming: None,
        incoming_stream: None,
        read: Some(ReadMetadata {
            unread: 3,
            unread_revision: 1,
            observed: ObservedIncoming { epoch: 1, through: 9 },
            can_mark_read: false,
        }),
    }
}

fn event(peer: &Peer, sequence: u64, event: ChatEvent) -> ChatPacket {
    ChatPacket {
        version: VERSION,
        source: source(),
        sequence,
        reply_to: peer.sent,
        message: ChatMessage::Event(event),
    }
}

fn setup() -> (Peer, WorkerIo, ReviewConversationRequest) {
    let (mut peer, worker) = connected();
    let row = row();
    peer.observe(&event(&peer, 1, ChatEvent::ReadBatch(vec![row.clone()]))).unwrap();
    let request = ReviewConversationRequest {
        target: ReviewCapture {
            key: ChatKey {
                source: source(),
                conversation: row.conversation,
            },
            policy_revision: row.policy_revision,
            observed: row.read.unwrap().observed,
        },
        request: 40,
        activation: 8,
    };
    (peer, worker, request)
}

#[test]
fn group_attention_review_broker_preserves_native_reads_and_latches_group_replacement_before_dispatch() {
    for change in 0..4 {
        let (mut peer, _worker) = connected();
        let mut row = row();
        row.conversation = Conversation::Group(aurora_session::ConversationId::from_bytes([8; 16]).unwrap());
        peer.observe(&event(&peer, 1, ChatEvent::ReadBatch(vec![row.clone()]))).unwrap();
        let request = ReviewConversationRequest {
            target: ReviewCapture {
                key: ChatKey {
                    source: source(),
                    conversation: row.conversation,
                },
                policy_revision: row.policy_revision,
                observed: row.read.unwrap().observed,
            },
            request: 40,
            activation: 8,
        };
        peer.watch_review(Some(request.target));
        assert!(peer.watched_review_current(request.target));
        match change {
            0 => {
                row.read.as_mut().unwrap().observed.through += 1;
                row.read.as_mut().unwrap().unread = 0;
                row.read.as_mut().unwrap().unread_revision += 1;
            }
            1 => row.read.as_mut().unwrap().observed.epoch += 1,
            2 => row.policy_revision += 1,
            3 => {}
            _ => unreachable!(),
        }
        // CloseBook owns hidden entries; use its real Close admission/receipt
        // tests for mutation. This case exercises the snapshot gate below.
        if change == 3 {
            let snapshot = HostChatSnapshot {
                contacts: Default::default(),
                source: source(),
                authenticated: true,
                live: true,
                conversations: peer.conversations.values().cloned().collect(),
                reads: peer.reads.clone(),
                closes: super::close::CloseSnapshot {
                    hidden: [row.conversation].into_iter().collect(),
                    ..Default::default()
                },
                lines: Vec::new(),
                receipts: Vec::new(),
                read_operations: Vec::new(),
                gap: false,
            };
            assert!(!snapshot.permits_review(request.target));
            continue;
        }
        peer.observe(&event(&peer, 2, ChatEvent::ReadBatch(vec![row]))).unwrap();
        if change == 0 {
            let reads = peer.reads.clone();
            peer.admit_review(request, Instant::now()).unwrap();
            peer.observe(&event(&peer, 3, reply(request, ReviewOutcome::OpenedLocally)))
                .unwrap();
            assert_eq!(peer.reads, reads);
            assert!(peer.pending.is_none() && peer.read_pending.is_none());
        } else {
            assert!(!peer.watched_review_current(request.target));
            assert_eq!(peer.admit_review(request, Instant::now()), Err(HostError::Unavailable));
            assert_eq!(peer.sent, 0);
        }
    }
}

fn reply(request: ReviewConversationRequest, outcome: ReviewOutcome) -> ChatEvent {
    ChatEvent::ReviewReceipt(ReviewReceiptReply {
        query: ReviewReceiptQuery {
            key: request.target.key,
            request: request.request,
        },
        receipt: Some(ReviewReceipt {
            target: request.target,
            request: request.request,
            activation: request.activation,
            outcome,
        }),
    })
}

fn commands(peer: &mut Peer, worker: &mut WorkerIo, minimum: usize) -> Vec<ChatPacket> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut commands = Vec::new();
    while commands.len() < minimum {
        peer.io.as_mut().unwrap().poll().unwrap();
        commands.extend(worker.poll().unwrap());
        assert!(Instant::now() < deadline, "owned synthetic loopback queue timed out");
        std::thread::yield_now();
    }
    commands
}

#[test]
fn chat_broker_review_allows_count_read_and_arrival_advances_without_native_mutation() {
    let (mut peer, _worker, request) = setup();
    let mut newer = row();
    newer.read.as_mut().unwrap().unread = 0;
    newer.read.as_mut().unwrap().unread_revision += 1;
    newer.read.as_mut().unwrap().observed.through += 2;
    peer.observe(&event(&peer, 2, ChatEvent::ReadBatch(vec![newer]))).unwrap();
    let reads = peer.reads.clone();
    peer.admit_review(request, Instant::now()).unwrap();
    peer.observe(&event(&peer, 3, reply(request, ReviewOutcome::OpenedLocally)))
        .unwrap();
    assert_eq!(
        peer.reads, reads,
        "Review receipt never clears or optimistically changes native read metadata"
    );
    assert!(peer.read_pending.is_none() && peer.pending.is_none());
    let delivery = peer.review_delivery(request).unwrap();
    assert_eq!(delivery.status, Status::Completed(ReviewOutcome::OpenedLocally));
    assert!(!delivery.recovering);
}

#[test]
fn chat_broker_review_rejects_changed_policy_epoch_and_backwards_cursor_before_enqueue() {
    for change in 0..3 {
        let (mut peer, _worker, request) = setup();
        let mut newer = row();
        match change {
            0 => newer.policy_revision += 1,
            1 => newer.read.as_mut().unwrap().observed.epoch += 1,
            2 => newer.read.as_mut().unwrap().observed.through -= 1,
            _ => unreachable!(),
        }
        peer.observe(&event(&peer, 2, ChatEvent::ReadBatch(vec![newer]))).unwrap();
        assert_eq!(peer.admit_review(request, Instant::now()), Err(HostError::Unavailable));
        assert_eq!(peer.sent, 0);
        assert!(peer.review_pending.is_none());
    }
}

#[test]
fn chat_broker_review_watch_latches_epoch_or_socket_loss_even_if_restored_before_host_poll() {
    for socket_loss in [false, true] {
        let (mut peer, _worker, request) = setup();
        peer.watch_review(Some(request.target));
        assert!(peer.watched_review_current(request.target));
        if socket_loss {
            peer.disconnect();
            let (mut fresh, _fresh_worker) = connected();
            peer.attach(fresh.io.take().unwrap()).unwrap();
        } else {
            let mut replaced = row();
            replaced.read.as_mut().unwrap().observed.epoch += 1;
            peer.observe(&event(&peer, 2, ChatEvent::ReadBatch(vec![replaced]))).unwrap();
        }
        peer.observe(&event(&peer, 3, ChatEvent::ReadBatch(vec![row()]))).unwrap();
        assert!(peer.review_allowed(request.target));
        assert!(
            !peer.watched_review_current(request.target),
            "a restored catalogue is not a fresh human gesture"
        );
    }
}

#[test]
fn chat_broker_review_duplicate_incarnations_in_one_batch_are_rejected_before_watch_or_high_water() {
    let (mut peer, _worker, request) = setup();
    peer.watch_review(Some(request.target));
    let mut replaced = row();
    replaced.read.as_mut().unwrap().observed.epoch += 1;
    let malformed = event(&peer, 1000, ChatEvent::ReadBatch(vec![replaced.clone(), row()]));
    assert_eq!(peer.observe(&malformed), Err(ChatError::Invalid));
    assert_eq!(peer.received, 1, "duplicate-key batch fails before receive high-water changes");
    assert!(peer.review_allowed(request.target));
    assert!(
        peer.watched_review_current(request.target),
        "rejected wire data cannot mutate the captured authority"
    );
    assert_eq!(peer.sent, 0);

    // A valid replacement in its own accepted packet does retire the capture;
    // restoring old metadata afterwards cannot revive that human gesture.
    peer.observe(&event(&peer, 2, ChatEvent::ReadBatch(vec![replaced]))).unwrap();
    peer.observe(&event(&peer, 3, ChatEvent::ReadBatch(vec![row()]))).unwrap();
    assert!(peer.review_allowed(request.target));
    assert!(!peer.watched_review_current(request.target));
}

#[test]
fn chat_broker_review_busy_queue_is_terminal_and_cannot_open_when_room_returns() {
    let (mut peer, mut worker, request) = setup();
    let mut count = 0;
    loop {
        let packet = ChatPacket {
            version: VERSION,
            source: source(),
            sequence: peer.sent + 1,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::Poll { after_event: 0 }),
        };
        match peer.io.as_mut().unwrap().queue(&packet) {
            Ok(()) => {
                // Match the ordinary producer: only admitted packets advance
                // both its high-water and the authenticated socket sequence.
                peer.sent = packet.sequence;
                count += 1;
                assert!(count <= 16);
            }
            Err(error) => {
                assert_eq!(error, LaneError::Busy, "fill capacity, not an invalid sequence");
                break;
            }
        }
    }
    assert!(count > 0);
    let admitted_before_review = peer.sent;
    assert_eq!(peer.admit_review(request, Instant::now()), Err(HostError::Busy));
    assert!(peer.review_pending.is_none());
    assert_eq!(peer.sent, admitted_before_review, "a refused Review emits no packet");
    let drained = commands(&mut peer, &mut worker, count);
    assert!(
        drained
            .iter()
            .all(|packet| !matches!(packet.message, ChatMessage::Command(ChatCommand::ReviewConversation(_))))
    );
    assert_eq!(peer.admit_review(request, Instant::now()), Err(HostError::Unavailable));
}

#[test]
fn chat_broker_review_receipts_match_full_capture_grant_and_request_before_high_water() {
    let (mut peer, _worker, request) = setup();
    peer.admit_review(request, Instant::now()).unwrap();
    for mismatch in 0..4 {
        let mut wrong = request;
        match mismatch {
            0 => wrong.target.policy_revision += 1,
            1 => wrong.target.observed.epoch += 1,
            2 => wrong.target.observed.through += 1,
            3 => wrong.activation += 1,
            _ => unreachable!(),
        }
        assert_eq!(
            peer.observe(&event(&peer, 1000, reply(wrong, ReviewOutcome::OpenedLocally))),
            Err(ChatError::RequestChanged)
        );
        assert_eq!(peer.received, 1);
    }
    peer.observe(&event(&peer, 2, reply(request, ReviewOutcome::NotForeground)))
        .unwrap();
    assert_eq!(
        peer.review_delivery(request).unwrap().status,
        Status::Completed(ReviewOutcome::NotForeground)
    );
    assert_eq!(
        peer.observe(&event(&peer, 1000, reply(request, ReviewOutcome::OpenedLocally))),
        Err(ChatError::FinalReceipt)
    );
    assert_eq!(peer.received, 2);
}

#[test]
fn chat_broker_review_old_terminal_duplicate_preserves_newer_exact_recovery_and_host_transaction() {
    let (mut peer, mut worker, first) = setup();
    let now = Instant::now();
    peer.admit_review(first, now).unwrap();
    peer.observe(&event(&peer, 2, reply(first, ReviewOutcome::OpenedLocally))).unwrap();
    let second = ReviewConversationRequest {
        request: first.request + 1,
        activation: first.activation + 1,
        ..first
    };
    peer.admit_review(second, now).unwrap();
    let viewport = aurora_session::HostedViewport {
        host_window: 1,
        rect: aurora_session::PhysicalRect {
            x: 0,
            y: 0,
            width: 640,
            height: 480,
        },
    };
    let grant = review::Grant {
        binding: second.target.key.source.binding(),
        activation: second.activation,
        viewport,
    };
    let mut transactions = review::Transactions::default();
    assert!(transactions.begin(second.target, second.request, viewport, 0));
    assert!(transactions.expect_activation(second.activation));
    transactions.granted(grant);
    assert_eq!(transactions.dispatch(grant), Some(second));
    transactions.queued(second);

    // A duplicate terminal A can be delivered after B's one-shot queue
    // admission. It must not consume B's receipt identity or recovery budget.
    peer.observe(&event(&peer, 3, reply(first, ReviewOutcome::OpenedLocally))).unwrap();
    assert_eq!(peer.review_pending.as_ref().unwrap().request, second);
    let delivery = peer.review_delivery(second).unwrap();
    assert!(delivery.recovering);
    assert_eq!(delivery.status, Status::Dispatched);
    transactions.delivery(Some(delivery));
    assert_eq!(transactions.full_request(), Some(second));
    assert_eq!(transactions.operations()[0].status, Status::Dispatched);

    let opens = commands(&mut peer, &mut worker, 2);
    assert_eq!(
        opens
            .iter()
            .filter(|packet| matches!(packet.message, ChatMessage::Command(ChatCommand::ReviewConversation(_))))
            .count(),
        2
    );
    peer.poll_review_receipt(now + POLL);
    let queries = commands(&mut peer, &mut worker, 1);
    assert!(
        queries.iter().all(|packet| matches!(packet.message,
        ChatMessage::Command(ChatCommand::QueryReviewReceipt(query)) if query.key == second.target.key && query.request == second.request))
    );
    assert_eq!(peer.review_pending.as_ref().unwrap().queries, 1);
    peer.observe(&event(&peer, 4, reply(second, ReviewOutcome::NotForeground))).unwrap();
    transactions.delivery(peer.review_delivery(second));
    assert_eq!(transactions.operations()[0].status, Status::Completed(ReviewOutcome::NotForeground));
    assert!(transactions.full_request().is_none());
    assert_eq!(
        peer.review_delivery(first).unwrap().status,
        Status::Completed(ReviewOutcome::OpenedLocally)
    );
}

#[test]
fn chat_broker_review_disconnect_reconnect_queries_receipt_without_replaying_open() {
    let (mut peer, mut worker, request) = setup();
    let now = Instant::now();
    peer.admit_review(request, now).unwrap();
    let first = commands(&mut peer, &mut worker, 1);
    assert_eq!(
        first
            .iter()
            .filter(|packet| matches!(packet.message, ChatMessage::Command(ChatCommand::ReviewConversation(_))))
            .count(),
        1
    );
    peer.disconnect();
    assert_eq!(peer.review_delivery(request).unwrap().status, Status::Uncertain);
    let (mut fresh, mut reconnected_worker) = connected();
    peer.attach(fresh.io.take().unwrap()).unwrap();
    peer.observe(&event(&peer, 2, ChatEvent::ReadBatch(vec![row()]))).unwrap();
    peer.poll_review_receipt(now + POLL);
    let next = commands(&mut peer, &mut reconnected_worker, 1);
    assert!(
        next.iter()
            .all(|packet| matches!(packet.message, ChatMessage::Command(ChatCommand::QueryReviewReceipt(_))))
    );
    peer.observe(&event(&peer, 3, reply(request, ReviewOutcome::OpenedLocally)))
        .unwrap();
    assert_eq!(
        peer.review_delivery(request).unwrap().status,
        Status::Completed(ReviewOutcome::OpenedLocally)
    );
}

#[test]
fn chat_broker_review_lookup_miss_or_recovery_deadline_remains_uncertain_and_never_replays() {
    for lookup_miss in [false, true] {
        let (mut peer, _worker, request) = setup();
        let now = Instant::now();
        peer.admit_review(request, now).unwrap();
        if lookup_miss {
            peer.observe(&event(
                &peer,
                2,
                ChatEvent::ReviewReceipt(ReviewReceiptReply {
                    query: ReviewReceiptQuery {
                        key: request.target.key,
                        request: request.request,
                    },
                    receipt: None,
                }),
            ))
            .unwrap();
        } else {
            for step in 1..=20 {
                peer.poll_review_receipt(now + POLL * step);
            }
        }
        let delivery = peer.review_delivery(request).unwrap();
        assert_eq!(delivery.status, Status::Uncertain);
        assert!(!delivery.recovering);
        assert!(peer.sent <= 1 + u64::from(REVIEW_QUERY_LIMIT));
        assert_eq!(peer.admit_review(request, now), Err(HostError::Unavailable));
        assert_eq!(
            peer.observe(&event(&peer, 1000, reply(request, ReviewOutcome::OpenedLocally))),
            Err(ChatError::UnknownReceipt)
        );
    }
}

#[test]
fn chat_broker_review_source_loss_purges_pending_receipts_and_late_source_cannot_revive() {
    let (mut peer, _worker, request) = setup();
    peer.admit_review(request, Instant::now()).unwrap();
    peer.set_control(None, false);
    assert!(peer.review_delivery(request).is_none());
    assert!(peer.review_pending.is_none());
    let mut replacement = source();
    replacement.session.generation += 1;
    peer.set_control(Some(replacement), true);
    assert_eq!(
        peer.observe(&event(&peer, 1000, reply(request, ReviewOutcome::OpenedLocally))),
        Err(ChatError::Binding)
    );
    assert!(peer.review_delivery(request).is_none());
}
