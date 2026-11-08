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
fn key() -> ChatKey {
    ChatKey {
        source: source(),
        conversation: Conversation::Conference(ConversationId::from_bytes([7; 16]).unwrap()),
    }
}

#[test]
fn shared_conference_codec_roundtrips_read_only_native_rows_and_rejects_every_older_packet_version() {
    let row = ReadStateRow {
        conversation: key().conversation,
        title: Label::new("Known native Conference").unwrap(),
        policy_revision: 1,
        lease_current: true,
        ready: true,
        can_send: false,
        can_read: true,
        read: Some(ReadMetadata {
            unread: 2,
            unread_revision: 1,
            observed: ObservedIncoming { epoch: 1, through: 2 },
            can_mark_read: true,
        }),
        incoming: Some(NativeIncoming { stream: 5, serial: 2 }),
        incoming_stream: Some(5),
    };
    let mut packet = ChatPacket {
        version: VERSION,
        source: source(),
        sequence: 1,
        reply_to: 1,
        message: ChatMessage::Event(ChatEvent::ReadBatch(vec![row])),
    };
    assert_eq!(decode(&encode(&packet).unwrap()).unwrap(), packet);
    for version in 0..VERSION {
        packet.version = version;
        assert_eq!(packet.validate(), Err(ChatError::Version));
    }
}

#[test]
fn shared_conference_codec_keeps_read_close_review_and_foreground_authority_separate() {
    let observed = ObservedIncoming { epoch: 1, through: 2 };
    MarkReadRequest {
        key: key(),
        request: 1,
        policy_revision: 1,
        observed,
        unread_revision: 1,
    }
    .validate()
    .unwrap();
    let mut close = CloseRequest {
        key: key(),
        request: 2,
        policy_revision: 1,
        stream: 5,
    };
    close.validate().unwrap();
    close.stream = 0;
    assert_eq!(close.validate(), Err(ChatError::Sequence));
    let mut review = ReviewConversationRequest {
        target: ReviewCapture {
            key: key(),
            policy_revision: 1,
            observed,
        },
        request: 3,
        activation: 4,
    };
    review.validate().unwrap();
    review.activation = 0;
    assert_eq!(review.validate(), Err(ChatError::Sequence));
}
