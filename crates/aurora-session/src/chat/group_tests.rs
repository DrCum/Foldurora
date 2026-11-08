use super::*;
use crate::{AccountKey, GridKey, HostId, InstallationId, SessionId, WorkerId};

fn source() -> ChatSource {
    ChatSource {
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
    }
}
fn group() -> Conversation {
    Conversation::Group(ConversationId::from_bytes([7; 16]).unwrap())
}
fn row() -> ReadStateRow {
    ReadStateRow {
        conversation: group(),
        title: Label::new(&"g".repeat(128)).unwrap(),
        policy_revision: 1,
        lease_current: true,
        ready: true,
        can_send: true,
        can_read: true,
        read: Some(ReadMetadata {
            unread: 7,
            unread_revision: 2,
            observed: ObservedIncoming { epoch: 1, through: 7 },
            can_mark_read: true,
        }),
        incoming: None,
        incoming_stream: None,
    }
}

#[test]
fn shared_group_codec_bounds_real_read_rows_and_rejects_v4_consumers() {
    let mut packet = ChatPacket {
        version: VERSION,
        source: source(),
        sequence: 1,
        reply_to: 1,
        message: ChatMessage::Event(ChatEvent::ReadBatch(
            (0..MAX_READ_ROWS)
                .map(|index| {
                    let mut row = row();
                    row.conversation = Conversation::Group(ConversationId::from_bytes([index as u8 + 7; 16]).unwrap());
                    row
                })
                .collect(),
        )),
    };
    let bytes = encode(&packet).unwrap();
    assert!(bytes.len() <= MAX_PACKET_BYTES);
    assert_eq!(decode(&bytes).unwrap(), packet);
    for old in 0..VERSION {
        packet.version = old;
        assert_eq!(packet.validate(), Err(ChatError::Version));
    }
}

#[test]
fn shared_group_read_does_not_grant_close_review_or_conference_authority() {
    let key = ChatKey {
        source: source(),
        conversation: group(),
    };
    let mut request = MarkReadRequest {
        key,
        request: 1,
        policy_revision: 1,
        observed: ObservedIncoming { epoch: 1, through: 7 },
        unread_revision: 2,
    };
    request.validate().unwrap();
    assert!(
        ReviewConversationRequest {
            target: ReviewCapture {
                key,
                policy_revision: 1,
                observed: request.observed
            },
            request: 1,
            activation: 0 // Read authority alone supplies no foreground input grant.
        }
        .validate()
        .is_err()
    );
    assert!(
        CloseRequest {
            key,
            request: 1,
            policy_revision: 1,
            stream: 0 // A read observation supplies no native Close stream.
        }
        .validate()
        .is_err()
    );
    request.key.conversation = Conversation::Conference(ConversationId::from_bytes([7; 16]).unwrap());
    assert_eq!(request.validate(), Err(ChatError::Invalid));
    let mut invalid = row();
    invalid.conversation = request.key.conversation;
    assert_eq!(invalid.validate(source()), Err(ChatError::Invalid));
}

#[test]
fn shared_group_read_rows_refuse_close_fences_and_unread_without_native_permission() {
    let mut row = row();
    row.incoming_stream = Some(0);
    assert_eq!(row.validate(source()), Err(ChatError::Sequence));
    row.incoming_stream = None;
    row.can_read = false;
    assert_eq!(row.validate(source()), Err(ChatError::Denied));
}

#[test]
fn group_close_codec_bounds_maximum_provenance_and_requires_current_readable_policy() {
    let source = source();
    let rows: Vec<_> = (0..MAX_READ_ROWS)
        .map(|index| {
            let mut row = row();
            row.conversation = Conversation::Group(ConversationId::from_bytes([index as u8 + 7; 16]).unwrap());
            row.title = Label::new(&"\"\\".repeat(64)).unwrap();
            row.policy_revision = u64::MAX;
            row.incoming_stream = Some(u64::MAX);
            row.incoming = Some(NativeIncoming {
                stream: u64::MAX,
                serial: u64::MAX,
            });
            row
        })
        .collect();
    let packet = ChatPacket {
        version: VERSION,
        source,
        sequence: u64::MAX,
        reply_to: u64::MAX,
        message: ChatMessage::Event(ChatEvent::ReadBatch(rows.clone())),
    };
    assert_eq!(decode(&encode(&packet).unwrap()).unwrap(), packet);
    let mut changed = rows[0].clone();
    changed.incoming_stream = Some(7);
    assert_eq!(changed.validate(source), Err(ChatError::Binding));
    let mut changed = rows[0].clone();
    changed.can_read = false;
    assert_eq!(changed.validate(source), Err(ChatError::Denied));
    let mut request = CloseRequest {
        key: ChatKey {
            source,
            conversation: group(),
        },
        request: 1,
        policy_revision: 1,
        stream: u64::MAX,
    };
    request.validate().unwrap();
    for conversation in [
        Conversation::Nearby,
        Conversation::Conference(ConversationId::from_bytes([7; 16]).unwrap()),
    ] {
        request.key.conversation = conversation;
        assert_eq!(request.validate(), Err(ChatError::Invalid));
    }
}
