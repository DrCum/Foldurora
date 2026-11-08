use super::*;
use crate::{AccountKey, GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerId, WorkerIdentity};
fn request() -> CloseRequest {
    CloseRequest {
        key: ChatKey {
            source: ChatSource {
                worker: WorkerIdentity {
                    installation: InstallationId::from_bytes([1; 32]).unwrap(),
                    host: HostId::from_bytes([2; 16]).unwrap(),
                    worker: WorkerId::from_bytes([3; 16]).unwrap(),
                },
                session: SessionIdentity {
                    session: SessionId::from_bytes([4; 16]).unwrap(),
                    account: AccountKey::from_bytes([5; 32]).unwrap(),
                    grid: GridKey::from_bytes([6; 32]).unwrap(),
                    generation: 1,
                },
                lease_epoch: 1,
            },
            conversation: Conversation::Direct(ConversationId::from_bytes([7; 16]).unwrap()),
        },
        request: 1,
        policy_revision: 1,
        stream: 12,
    }
}
#[test]
fn close_native_stream_zero_overflow_and_replacement_are_never_new_activity() {
    let zero = NativeIncoming { stream: 1, serial: 0 };
    assert_eq!(zero.validate(), Ok(()));
    assert_eq!(zero.checked_next().unwrap().newer_than(zero), Some(true));
    assert_eq!(zero.newer_than(zero), Some(false));
    assert_eq!(
        NativeIncoming {
            stream: 2,
            serial: u64::MAX
        }
        .newer_than(zero),
        None
    );
    assert_eq!(NativeIncoming::default().validate(), Err(ChatError::Sequence));
    assert_eq!(NativeIncoming { serial: u64::MAX, ..zero }.checked_next(), None);
    assert_eq!(NativeIncoming::default().checked_next(), None);
}
#[test]
fn close_ledger_query_is_readonly_and_duplicates_keep_original_terminal_fence() {
    let request = request();
    let mut ledger = CloseReceiptLedger::new(request.key.source);
    assert_eq!(ledger.query(request).unwrap().result, CloseReceiptState::Unknown);
    assert_eq!(ledger.reserve(request), Ok(None));
    assert_eq!(ledger.query(request).unwrap().result, CloseReceiptState::Pending);
    assert_eq!(ledger.reserve(request), Err(ChatError::Busy));
    let outcome = CloseOutcome::Closed(NativeIncoming { stream: 12, serial: 3 });
    let original = ledger.finish(request, outcome).unwrap();
    for _ in 0..8 {
        assert_eq!(ledger.query(request).unwrap(), original);
        assert_eq!(ledger.reserve(request), Ok(Some(original)));
    }
    assert_eq!(ledger.finish(request, CloseOutcome::RejectedNative), Err(ChatError::FinalReceipt));
    let changed = CloseRequest {
        policy_revision: 2,
        ..request
    };
    assert_eq!(ledger.query(changed), Err(ChatError::RequestChanged));
    assert_eq!(ledger.reserve(changed), Err(ChatError::RequestChanged));
    let unknown = CloseRequest { request: 2, ..request };
    assert_eq!(ledger.query(unknown).unwrap().result, CloseReceiptState::Unknown);
    assert_eq!(ledger.reserve(unknown), Ok(None));
    let rejected = ledger.finish(unknown, CloseOutcome::RejectedNative).unwrap();
    assert_ne!(original.result, rejected.result);
}
#[test]
fn close_ledger_bounded_eviction_cannot_replay_and_pending_is_never_evicted() {
    let first = request();
    let mut ledger = CloseReceiptLedger::new(first.key.source);
    ledger.reserve(first).unwrap();
    assert_eq!(ledger.reserve(CloseRequest { request: 2, ..first }), Err(ChatError::Busy));
    ledger.finish(first, CloseOutcome::Expired).unwrap();
    for number in 2..=MAX_CLOSE_RECEIPTS as u64 + 1 {
        let request = CloseRequest { request: number, ..first };
        ledger.reserve(request).unwrap();
        ledger.finish(request, CloseOutcome::StaleTarget).unwrap();
        assert!(ledger.records.len() <= MAX_CLOSE_RECEIPTS);
    }
    assert_eq!(ledger.query(first).unwrap().result, CloseReceiptState::Unknown);
    assert_eq!(ledger.reserve(first), Err(ChatError::StaleRequest));
    let last = CloseRequest {
        request: u64::MAX,
        ..first
    };
    ledger.reserve(last).unwrap();
    ledger.finish(last, CloseOutcome::Expired).unwrap();
    assert_eq!(
        ledger.query(last).unwrap().result,
        CloseReceiptState::Terminal(CloseOutcome::Expired)
    );
    let mut foreign = first;
    foreign.key.source.lease_epoch += 1;
    assert_eq!(ledger.query(foreign), Err(ChatError::Binding));
}
#[test]
fn close_wire_is_direct_only_exact_scoped_and_unknown_is_not_a_zero_fence() {
    let request = request();
    let mut packet = ChatPacket {
        version: VERSION,
        source: request.key.source,
        sequence: 1,
        reply_to: 0,
        message: ChatMessage::Command(ChatCommand::CloseConversation(request)),
    };
    let bytes = encode(&packet).unwrap();
    assert!(bytes.len() < MAX_PACKET_BYTES);
    assert_eq!(decode(&bytes).unwrap(), packet);
    packet.version = VERSION - 1;
    assert_eq!(encode(&packet), Err(ChatError::Version));
    packet.version = VERSION;
    packet.message = ChatMessage::Command(ChatCommand::QueryCloseReceipt(request));
    assert_eq!(decode(&encode(&packet).unwrap()).unwrap(), packet);
    let nearby = CloseRequest {
        key: ChatKey {
            conversation: Conversation::Nearby,
            ..request.key
        },
        ..request
    };
    assert_eq!(nearby.validate(), Err(ChatError::Invalid));
    let mut changed = request;
    changed.key.source.lease_epoch += 1;
    packet.message = ChatMessage::Command(ChatCommand::CloseConversation(changed));
    assert_eq!(encode(&packet), Err(ChatError::Binding));
    packet.message = ChatMessage::Event(ChatEvent::CloseReceipt(CloseReceiptReply {
        request,
        result: CloseReceiptState::Unknown,
    }));
    assert!(encode(&packet).is_err());
    packet.reply_to = 1;
    assert_eq!(decode(&encode(&packet).unwrap()).unwrap(), packet);
    let malformed = br#"{"stream":1,"serial":0,"extra":1}"#;
    assert!(serde_json::from_slice::<NativeIncoming>(malformed).is_err());
    assert!(
        !CloseReceiptReply {
            request,
            result: CloseReceiptState::Terminal(CloseOutcome::Closed(NativeIncoming::default()))
        }
        .matches(request)
    );
}
#[test]
fn close_activity_full_fair_batch_fits_existing_packet_budget() {
    let mut request = request();
    request.key.source.session.generation = u64::MAX;
    request.key.source.lease_epoch = u64::MAX;
    for title in ["\"\\".repeat(64), "界".repeat(42)] {
        let rows = (0..MAX_READ_ROWS)
            .map(|n| ReadStateRow {
                conversation: Conversation::Direct(ConversationId::from_bytes([n as u8 + 1; 16]).unwrap()),
                title: Label::new(&title).unwrap(),
                policy_revision: u64::MAX,
                lease_current: true,
                ready: true,
                can_send: true,
                can_read: true,
                read: Some(ReadMetadata {
                    unread: u64::MAX,
                    unread_revision: u64::MAX,
                    observed: ObservedIncoming {
                        epoch: u64::MAX,
                        through: u64::MAX,
                    },
                    can_mark_read: true,
                }),
                incoming: Some(NativeIncoming {
                    stream: u64::MAX,
                    serial: u64::MAX,
                }),
                incoming_stream: Some(u64::MAX),
            })
            .collect();
        let packet = ChatPacket {
            version: VERSION,
            source: request.key.source,
            sequence: u64::MAX,
            reply_to: u64::MAX,
            message: ChatMessage::Event(ChatEvent::ReadBatch(rows)),
        };
        let bytes = encode(&packet).unwrap();
        assert!(bytes.len() <= MAX_PACKET_BYTES);
        assert_eq!(decode(&bytes).unwrap(), packet);
    }
}

#[test]
fn close_wire_rejects_cross_stream_terminal_and_activity_without_current_incarnation() {
    let request = request();
    let invalid = CloseReceiptReply {
        request,
        result: CloseReceiptState::Terminal(CloseOutcome::Closed(NativeIncoming {
            stream: request.stream + 1,
            serial: 0,
        })),
    };
    assert_eq!(invalid.validate(), Err(ChatError::Binding));
    let mut row = ReadStateRow {
        conversation: request.key.conversation,
        title: Label::new("Synthetic resident").unwrap(),
        policy_revision: 1,
        lease_current: true,
        ready: true,
        can_send: false,
        can_read: true,
        read: None,
        incoming: None,
        incoming_stream: Some(request.stream),
    };
    assert_eq!(row.validate(request.key.source), Ok(()));
    // A current stream can be known while session activity remains unknown.
    row.incoming = Some(NativeIncoming {
        stream: request.stream,
        serial: 1,
    });
    row.incoming_stream = None;
    assert_eq!(row.validate(request.key.source), Err(ChatError::Binding));
    row.incoming_stream = Some(request.stream + 1);
    assert_eq!(row.validate(request.key.source), Err(ChatError::Binding));
    row.incoming = None;
    row.incoming_stream = Some(0);
    assert_eq!(row.validate(request.key.source), Err(ChatError::Sequence));
    row.incoming_stream = Some(request.stream);
    row.can_read = false;
    assert_eq!(row.validate(request.key.source), Err(ChatError::Denied));
    row.can_read = true;
    row.conversation = Conversation::Nearby;
    assert_eq!(row.validate(request.key.source), Err(ChatError::Denied));
}
