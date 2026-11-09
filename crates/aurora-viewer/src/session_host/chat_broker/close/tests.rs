use super::*;
use crate::session_host::chat_broker::tests::{connected, source};
use aurora_session::{ConversationId, Label};

fn conversation() -> Conversation {
    Conversation::Direct(ConversationId::from_bytes([77; 16]).unwrap())
}
fn row(stream: u64, serial: u64) -> ReadStateRow {
    ReadStateRow {
        conversation: conversation(),
        title: Label::new("Synthetic close target").unwrap(),
        policy_revision: 1,
        lease_current: true,
        ready: true,
        can_send: true,
        can_read: true,
        read: None,
        incoming: Some(NativeIncoming { stream, serial }),
        incoming_stream: Some(stream),
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
fn observe_row(peer: &mut Peer, sequence: u64, stream: u64, serial: u64) {
    peer.observe(&event(peer, sequence, ChatEvent::ReadBatch(vec![row(stream, serial)])))
        .unwrap();
}
fn request(ticket: u64) -> CloseRequest {
    CloseRequest {
        key: ChatKey {
            source: source(),
            conversation: conversation(),
        },
        request: ticket,
        policy_revision: 1,
        stream: 22,
    }
}
fn ack(peer: &mut Peer, sequence: u64, request: CloseRequest, result: CloseReceiptState) -> Result<(), ChatError> {
    peer.observe(&event(
        peer,
        sequence,
        ChatEvent::CloseReceipt(CloseReceiptReply { request, result }),
    ))
}
fn closed(serial: u64) -> CloseReceiptState {
    CloseReceiptState::Terminal(CloseOutcome::Closed(NativeIncoming { stream: 22, serial }))
}

#[test]
fn group_close_host_pending_and_other_conversations_cannot_replace_original_fence() {
    let (mut peer, _worker) = connected();
    let group = Conversation::Group(ConversationId::from_bytes([77; 16]).unwrap());
    let mut group_row = row(22, 1);
    group_row.conversation = group;
    peer.observe(&event(&peer, 1, ChatEvent::ReadBatch(vec![group_row.clone()])))
        .unwrap();
    let mut request = request(10);
    request.key.conversation = group;
    peer.admit_close(request, Instant::now()).unwrap();
    assert!(peer.closes.hidden(group));
    group_row.incoming = Some(NativeIncoming { stream: 22, serial: 2 });
    peer.observe(&event(&peer, 2, ChatEvent::ReadBatch(vec![group_row.clone()])))
        .unwrap();
    assert!(peer.closes.hidden(group), "no guessed fence while pending");
    ack(&mut peer, 3, request, closed(2)).unwrap();
    assert!(peer.closes.hidden(group), "the pre-capture arrival stays hidden");
    observe_row(&mut peer, 4, 22, 100); // Direct with the identical UUID bytes.
    assert!(peer.closes.hidden(group), "typed identities never alias");
    group_row.incoming = None;
    peer.observe(&event(&peer, 5, ChatEvent::ReadBatch(vec![group_row.clone()])))
        .unwrap();
    assert!(peer.closes.hidden(group), "recreation/unknown is not zero or newer");
    group_row.incoming = Some(NativeIncoming { stream: 22, serial: 3 });
    peer.observe(&event(&peer, 6, ChatEvent::ReadBatch(vec![group_row.clone()])))
        .unwrap();
    assert!(!peer.closes.hidden(group));
    let next = CloseRequest { request: 11, ..request };
    peer.admit_close(next, Instant::now()).unwrap();
    ack(&mut peer, 7, request, closed(2)).unwrap();
    assert_eq!(
        peer.closes.pending.as_ref().unwrap().request,
        next,
        "late receipt cannot retire a newer Group close"
    );
    ack(&mut peer, 8, next, closed(3)).unwrap();
    group_row.incoming = Some(NativeIncoming {
        stream: 23,
        serial: u64::MAX,
    });
    group_row.incoming_stream = Some(23);
    peer.observe(&event(&peer, 9, ChatEvent::ReadBatch(vec![group_row]))).unwrap();
    assert!(peer.closes.hidden(group), "a different incarnation cannot reopen");
    assert!(peer.read_pending.is_none() && peer.review_pending.is_none());
}

#[test]
fn host_close_fence_includes_arrivals_before_ack_and_only_newer_native_incoming_reopens() {
    let (mut peer, _worker) = connected();
    observe_row(&mut peer, 1, 22, 1);
    let reads = peer.reads.clone();
    peer.admit_close(request(10), Instant::now()).unwrap();
    assert!(peer.snapshot().unwrap().closes.hidden.contains(&conversation()));
    observe_row(&mut peer, 2, 22, 2);
    assert!(peer.closes.hidden(conversation()), "pending hides without guessing a fence");
    ack(&mut peer, 3, request(10), closed(2)).unwrap();
    assert!(peer.closes.hidden(conversation()), "a pre-ack arrival belongs to the native fence");
    observe_row(&mut peer, 4, 22, 2);
    assert!(
        peer.closes.hidden(conversation()),
        "catalogue/read/own-echo refresh is not newer activity"
    );
    observe_row(&mut peer, 5, 22, 3);
    assert!(!peer.closes.hidden(conversation()));
    assert_eq!(peer.reads, reads, "Close never acknowledges unread");
    assert!(peer.pending.is_none() && peer.read_pending.is_none() && peer.review_pending.is_none());
    peer.admit_close(request(11), Instant::now()).unwrap();
    ack(&mut peer, 6, request(10), closed(2)).unwrap();
    assert!(peer.closes.hidden(conversation()), "late duplicate cannot retire a newer close");
    assert_eq!(peer.closes.pending.as_ref().unwrap().request, request(11));
}

#[test]
fn host_close_ack_after_newer_catalogue_reopens_without_waiting_for_another_poll() {
    let (mut peer, _worker) = connected();
    observe_row(&mut peer, 1, 22, 1);
    peer.admit_close(request(10), Instant::now()).unwrap();
    observe_row(&mut peer, 2, 22, 4);
    ack(&mut peer, 3, request(10), closed(2)).unwrap();
    assert!(!peer.closes.hidden(conversation()));
}

#[test]
fn host_close_unknown_stream_change_and_missing_activity_do_not_guess_a_new_fence() {
    let (mut peer, _worker) = connected();
    observe_row(&mut peer, 1, 22, 1);
    peer.admit_close(request(10), Instant::now()).unwrap();
    ack(&mut peer, 2, request(10), CloseReceiptState::Unknown).unwrap();
    observe_row(&mut peer, 3, 22, u64::MAX);
    assert!(peer.closes.hidden(conversation()));
    ack(&mut peer, 4, request(10), closed(2)).unwrap();
    assert!(!peer.closes.hidden(conversation()));
    peer.admit_close(request(11), Instant::now()).unwrap();
    observe_row(&mut peer, 5, 23, u64::MAX);
    ack(&mut peer, 6, request(11), closed(3)).unwrap();
    assert!(
        peer.closes.hidden(conversation()),
        "different native incarnation is not newer activity"
    );
    let mut missing = row(23, 0);
    missing.incoming = None;
    peer.observe(&event(&peer, 7, ChatEvent::ReadBatch(vec![missing]))).unwrap();
    assert!(peer.closes.hidden(conversation()));
}

#[test]
fn host_close_receipts_are_exact_immutable_and_source_replacement_retires_everything() {
    let (mut peer, _worker) = connected();
    observe_row(&mut peer, 1, 22, 1);
    peer.admit_close(request(10), Instant::now()).unwrap();
    let changed = CloseRequest {
        policy_revision: 2,
        ..request(10)
    };
    assert_eq!(ack(&mut peer, 2, changed, closed(1)), Err(ChatError::UnknownReceipt));
    assert_eq!(peer.received, 1);
    ack(&mut peer, 2, request(10), closed(1)).unwrap();
    assert_eq!(ack(&mut peer, 3, request(10), closed(2)), Err(ChatError::FinalReceipt));
    assert!(peer.closes.hidden(conversation()));
    let mut next = source();
    next.session.generation += 1;
    peer.set_control(Some(next), true);
    assert!(peer.closes.hidden.is_empty() && peer.closes.operations.is_empty() && peer.closes.targets.is_empty());
    assert_eq!(ack(&mut peer, 3, request(10), closed(1)), Err(ChatError::Binding));
}

#[test]
fn host_close_rejection_restores_visibility_but_unknown_does_not_and_hidden_send_is_denied() {
    let (mut peer, _worker) = connected();
    observe_row(&mut peer, 1, 22, 1);
    peer.admit_close(request(10), Instant::now()).unwrap();
    assert_eq!(
        peer.send(request(10).key, 1, ChatText::new("must not send").unwrap(), 11, Instant::now()),
        Err(HostError::Unavailable)
    );
    ack(&mut peer, 2, request(10), CloseReceiptState::Terminal(CloseOutcome::Expired)).unwrap();
    assert!(!peer.closes.hidden(conversation()));
    peer.admit_close(request(12), Instant::now()).unwrap();
    ack(&mut peer, 3, request(12), CloseReceiptState::Unknown).unwrap();
    assert!(peer.closes.hidden(conversation()));
    assert_eq!(peer.closes.operations.back().unwrap().status, CloseStatus::Uncertain);
}

#[test]
fn host_close_queries_original_operation_once_and_never_resends_close() {
    let (mut peer, mut worker) = connected();
    observe_row(&mut peer, 1, 22, 1);
    let now = Instant::now();
    peer.admit_close(request(10), now).unwrap();
    let mut packets = Vec::new();
    for step in 0..25 {
        peer.poll(now + Duration::from_secs(step));
        peer.io.as_mut().unwrap().poll().unwrap();
        packets.extend(worker.poll().unwrap());
        std::thread::yield_now();
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while !packets
        .iter()
        .any(|packet| matches!(packet.message, ChatMessage::Command(ChatCommand::QueryCloseReceipt(_))))
    {
        peer.io.as_mut().unwrap().poll().unwrap();
        packets.extend(worker.poll().unwrap());
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(
        packets
            .iter()
            .filter(|packet| matches!(packet.message, ChatMessage::Command(ChatCommand::CloseConversation(_))))
            .count(),
        1
    );
    let queries: Vec<_> = packets
        .iter()
        .filter_map(|packet| match packet.message {
            ChatMessage::Command(ChatCommand::QueryCloseReceipt(request)) => Some(request),
            _ => None,
        })
        .collect();
    assert_eq!(queries, [request(10)]);
    assert!(peer.closes.pending.is_none());
    assert!(peer.closes.hidden(conversation()));
    assert_eq!(peer.closes.operations.back().unwrap().status, CloseStatus::Uncertain);
}

#[test]
fn host_close_operation_history_is_bounded_and_duplicate_admission_cannot_mutate_a_receipt() {
    let (mut peer, _worker) = connected();
    observe_row(&mut peer, 1, 22, 1);
    let now = Instant::now();
    peer.admit_close(request(10), now).unwrap();
    assert_eq!(peer.admit_close(request(10), now), Err(HostError::Busy));
    assert_eq!(peer.closes.operations[0].status, CloseStatus::Pending);
    for ticket in 11..150 {
        peer.closes.record(request(ticket), CloseStatus::Unavailable).unwrap();
    }
    assert_eq!(peer.closes.operations.len(), aurora_session::chat::MAX_CLOSE_RECEIPTS);
    assert_eq!(peer.closes.operations[0].request, request(10), "active record is never evicted");
    assert_eq!(peer.closes.highwater, 149);
}

#[test]
fn host_close_changed_policy_recovers_original_receipt_and_lost_authority_cannot_block_other_chats_forever() {
    for readable in [true, false] {
        let (mut peer, _worker) = connected();
        observe_row(&mut peer, 1, 22, 1);
        let now = Instant::now();
        peer.admit_close(request(10), now).unwrap();
        let mut changed = row(22, 1);
        changed.policy_revision = 2;
        changed.can_read = readable;
        changed.incoming = readable.then_some(NativeIncoming { stream: 22, serial: 1 });
        changed.incoming_stream = readable.then_some(22);
        peer.observe(&event(&peer, 2, ChatEvent::ReadBatch(vec![changed]))).unwrap();
        peer.poll_close_receipt(now + RECEIPT_WAIT);
        if readable {
            let pending = peer.closes.pending.as_ref().unwrap();
            assert_eq!(pending.request, request(10), "query keeps the original policy and stream");
            assert!(pending.queried_at.is_some());
        }
        peer.poll_close_receipt(now + RECEIPT_WAIT + RECEIPT_WAIT);
        assert!(peer.closes.pending.is_none());
        assert!(peer.closes.hidden(conversation()));
        assert_eq!(peer.closes.operations.back().unwrap().status, CloseStatus::Uncertain);
    }
}

#[test]
fn contacts_explicit_open_retires_original_close_but_never_a_later_close_or_incoming_fence() {
    let (mut peer, _worker) = connected();
    observe_row(&mut peer, 1, 22, 1);
    let first = request(10);
    peer.admit_close(first, Instant::now()).unwrap();
    let captured = peer.closes.highwater();
    peer.closes.explicit_open(conversation(), captured);
    assert!(!peer.closes.hidden(conversation()));
    assert!(peer.closes.pending.is_none());
    assert_eq!(peer.closes.snapshot().seen_through, 10);
    ack(&mut peer, 2, first, closed(1)).unwrap();
    assert!(!peer.closes.hidden(conversation()), "late old Close receipt cannot hide again");
    let later = request(11);
    peer.admit_close(later, Instant::now()).unwrap();
    peer.closes.explicit_open(conversation(), captured);
    assert!(peer.closes.hidden(conversation()), "duplicate Open receipt cannot undo later Close");
    assert_eq!(peer.closes.pending.as_ref().unwrap().request, later);
    ack(&mut peer, 3, later, closed(1)).unwrap();
    observe_row(&mut peer, 4, 22, 1);
    assert!(peer.closes.hidden(conversation()), "Open minted no native incoming credit");
    observe_row(&mut peer, 5, 22, 2);
    assert!(!peer.closes.hidden(conversation()));
}
