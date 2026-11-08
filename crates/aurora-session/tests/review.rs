use aurora_session::chat::*;
use aurora_session::{
    AccountKey, ConversationId, GridKey, HostId, InstallationId, PROTOCOL_VERSION, SessionId, SessionIdentity, WorkerId, WorkerIdentity,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::fmt::Debug;

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

fn request(number: u64) -> ReviewConversationRequest {
    ReviewConversationRequest {
        target: ReviewCapture {
            key: ChatKey {
                source: source(),
                conversation: Conversation::Direct(ConversationId::from_bytes([7; 16]).unwrap()),
            },
            policy_revision: 11,
            observed: ObservedIncoming { epoch: 13, through: 17 },
        },
        request: number,
        activation: 19,
    }
}

fn query(request: ReviewConversationRequest) -> ReviewReceiptQuery {
    ReviewReceiptQuery {
        key: request.target.key,
        request: request.request,
    }
}

fn receipt(request: ReviewConversationRequest, outcome: ReviewOutcome) -> ReviewReceipt {
    ReviewReceipt {
        target: request.target,
        request: request.request,
        activation: request.activation,
        outcome,
    }
}

fn round_trip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: &T) -> Vec<u8> {
    let bytes = serde_json::to_vec(value).unwrap();
    assert!(bytes.len() <= MAX_PACKET_BYTES, "bounded DTO serialized to {} bytes", bytes.len());
    assert_eq!(&serde_json::from_slice::<T>(&bytes).unwrap(), value);
    bytes
}

fn escaped_strings(value: &Value) -> String {
    match value {
        Value::String(value) => {
            let escaped: String = value.chars().map(|ch| format!("\\u{:04x}", ch as u32)).collect();
            format!("\"{escaped}\"")
        }
        Value::Object(values) => {
            let fields: Vec<_> = values
                .iter()
                .map(|(key, value)| format!("{}:{}", serde_json::to_string(key).unwrap(), escaped_strings(value)))
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        _ => serde_json::to_string(value).unwrap(),
    }
}

#[test]
fn review_dtos_validate_full_source_positive_sequences_and_nearby_or_direct_only() {
    let valid = request(1);
    assert!(valid.validate().is_ok());
    let mut nearby = valid;
    nearby.target.key.conversation = Conversation::Nearby;
    nearby.target.observed.through = 0;
    assert!(
        nearby.validate().is_ok(),
        "an initial native observation may have no counted arrivals"
    );

    for changed in [
        ReviewConversationRequest { request: 0, ..valid },
        ReviewConversationRequest { activation: 0, ..valid },
        ReviewConversationRequest {
            target: ReviewCapture {
                policy_revision: 0,
                ..valid.target
            },
            ..valid
        },
        ReviewConversationRequest {
            target: ReviewCapture {
                observed: ObservedIncoming { epoch: 0, through: 17 },
                ..valid.target
            },
            ..valid
        },
    ] {
        assert_eq!(changed.validate(), Err(ChatError::Sequence));
        assert_eq!(receipt(changed, ReviewOutcome::RejectedNative).validate(), Err(ChatError::Sequence));
    }

    for conversation in [
        Conversation::Group(ConversationId::from_bytes([7; 16]).unwrap()),
        Conversation::Conference(ConversationId::from_bytes([7; 16]).unwrap()),
    ] {
        let mut invalid = valid;
        invalid.target.key.conversation = conversation;
        assert_eq!(invalid.target.validate(), Err(ChatError::Invalid));
        assert_eq!(invalid.validate(), Err(ChatError::Invalid));
        assert_eq!(query(invalid).validate(), Err(ChatError::Invalid));
        assert_eq!(receipt(invalid, ReviewOutcome::OpenedLocally).validate(), Err(ChatError::Invalid));
    }

    let mut invalid = valid;
    invalid.target.key.source.lease_epoch = 0;
    assert_eq!(invalid.validate(), Err(ChatError::Binding));
    assert!(matches!(
        ReviewReceiptLedger::new(invalid.target.key.source),
        Err(ChatError::Binding)
    ));
    invalid = valid;
    invalid.target.key.source.session.generation = 0;
    assert_eq!(invalid.validate(), Err(ChatError::Binding));
    assert_eq!(
        ReviewReceiptQuery {
            request: 0,
            ..query(valid)
        }
        .validate(),
        Err(ChatError::Sequence)
    );
}

#[test]
fn review_dtos_are_closed_body_free_and_maximum_full_id_serialization_is_bounded() {
    let mut maximum = request(u64::MAX);
    maximum.target.key.source = ChatSource {
        worker: WorkerIdentity {
            installation: InstallationId::from_bytes([255; 32]).unwrap(),
            host: HostId::from_bytes([255; 16]).unwrap(),
            worker: WorkerId::from_bytes([255; 16]).unwrap(),
        },
        session: SessionIdentity {
            session: SessionId::from_bytes([255; 16]).unwrap(),
            account: AccountKey::from_bytes([255; 32]).unwrap(),
            grid: GridKey::from_bytes([255; 32]).unwrap(),
            generation: u64::MAX,
        },
        lease_epoch: u64::MAX,
    };
    maximum.target.key.conversation = Conversation::Direct(ConversationId::from_bytes([255; 16]).unwrap());
    maximum.target.policy_revision = u64::MAX;
    maximum.target.observed = ObservedIncoming {
        epoch: u64::MAX,
        through: u64::MAX,
    };
    maximum.activation = u64::MAX;
    assert!(maximum.validate().is_ok());
    round_trip(&maximum.target);
    let encoded = round_trip(&maximum);
    round_trip(&query(maximum));
    for outcome in [
        ReviewOutcome::OpenedLocally,
        ReviewOutcome::RejectedNative,
        ReviewOutcome::StaleTarget,
        ReviewOutcome::NotForeground,
        ReviewOutcome::Expired,
    ] {
        let receipt = receipt(maximum, outcome);
        assert!(receipt.validate().is_ok() && receipt.matches(maximum));
        round_trip(&receipt);
        let reply = ReviewReceiptReply {
            query: query(maximum),
            receipt: Some(receipt),
        };
        assert_eq!(reply.validate_for(maximum), Ok(()));
        round_trip(&reply);
    }
    round_trip(&ReviewReceiptReply {
        query: query(maximum),
        receipt: None,
    });

    // Identifiers serialize as fixed lowercase hex: there is no free text that
    // could increase output through quoting/Unicode escapes. Escaped input IDs
    // still round-trip to the same closed typed capture and canonical output.
    assert!(!encoded.contains(&b'\\'));
    let escaped = escaped_strings(&serde_json::to_value(maximum).unwrap());
    assert!(escaped.len() <= MAX_PACKET_BYTES);
    let escaped_request = serde_json::from_str::<ReviewConversationRequest>(&escaped).unwrap();
    assert_eq!(escaped_request, maximum);
    assert_eq!(serde_json::to_vec(&escaped_request).unwrap(), encoded);

    for payload in ["\"\\".repeat(128), "界".repeat(128)] {
        for field in ["text", "path", "asset_id", "notification_id", "button", "channel", "version"] {
            let mut unknown = serde_json::to_value(maximum).unwrap();
            unknown[field] = payload.clone().into();
            assert!(serde_json::from_value::<ReviewConversationRequest>(unknown).is_err());
        }
    }
    let mut unknown = serde_json::to_value(maximum.target).unwrap();
    unknown["read"] = true.into();
    assert!(serde_json::from_value::<ReviewCapture>(unknown).is_err());
    let mut unknown = serde_json::to_value(receipt(maximum, ReviewOutcome::OpenedLocally)).unwrap();
    unknown["text"] = "\"\\Synthetic text".into();
    assert!(serde_json::from_value::<ReviewReceipt>(unknown).is_err());
    let mut unknown = serde_json::to_value(query(maximum)).unwrap();
    unknown["activation"] = 1.into();
    assert!(serde_json::from_value::<ReviewReceiptQuery>(unknown).is_err());
    let mut unknown = serde_json::to_value(ReviewReceiptReply {
        query: query(maximum),
        receipt: None,
    })
    .unwrap();
    unknown["retry"] = true.into();
    assert!(serde_json::from_value::<ReviewReceiptReply>(unknown).is_err());
    for outcome in ["pending", "uncertain", "send", "accept_offer", "export_asset"] {
        assert!(serde_json::from_value::<ReviewOutcome>(outcome.into()).is_err());
    }
    let mut bad_id: Value = serde_json::from_slice(&encoded).unwrap();
    bad_id["target"]["key"]["conversation"]["id"] = "\"\\".repeat(16).into();
    assert!(serde_json::from_value::<ReviewConversationRequest>(bad_id).is_err());
}

#[test]
fn review_v3_codec_roundtrips_closed_variants_and_rejects_old_versions() {
    // Review retains its V3 payload inside the Group-capable V5 envelope.
    assert_eq!(VERSION, 5);
    assert_eq!(PROTOCOL_VERSION, 1);
    let operation = request(1);
    for message in [
        ChatMessage::Command(ChatCommand::ReviewConversation(operation)),
        ChatMessage::Command(ChatCommand::QueryReviewReceipt(query(operation))),
        ChatMessage::Event(ChatEvent::ReviewReceipt(ReviewReceiptReply {
            query: query(operation),
            receipt: Some(receipt(operation, ReviewOutcome::OpenedLocally)),
        })),
        ChatMessage::Event(ChatEvent::ReviewReceipt(ReviewReceiptReply {
            query: query(operation),
            receipt: None,
        })),
    ] {
        let packet = ChatPacket {
            version: VERSION,
            source: source(),
            sequence: 1,
            reply_to: if matches!(message, ChatMessage::Command(_)) { 0 } else { 1 },
            message,
        };
        let encoded = encode(&packet).unwrap();
        assert_eq!(decode(&encoded), Ok(packet.clone()));
        for version in [1, 2, 3, 5] {
            let unsupported = ChatPacket { version, ..packet.clone() };
            assert_eq!(encode(&unsupported), Err(ChatError::Version));
            assert_eq!(decode(&serde_json::to_vec(&unsupported).unwrap()), Err(ChatError::Version));
        }
        let mut unknown: Value = serde_json::from_slice(&encoded).unwrap();
        unknown["message"]["body"]["data"]["text"] = "\"\\Synthetic text".into();
        assert_eq!(decode(&serde_json::to_vec(&unknown).unwrap()), Err(ChatError::Malformed));
        for unsupported in ["accept_offer", "send_keys", "review_attention", "export_asset"] {
            let mut unknown: Value = serde_json::from_slice(&encoded).unwrap();
            unknown["message"]["body"]["type"] = unsupported.into();
            assert_eq!(decode(&serde_json::to_vec(&unknown).unwrap()), Err(ChatError::Malformed));
        }
        let debug = format!("{packet:?}");
        assert!(debug.contains("review"));
        for private_field in ["policy_revision", "activation", "observed", "installation", "account", "grid"] {
            assert!(
                !debug.contains(private_field),
                "packet Debug must not include captures or grant/source identity"
            );
        }
    }
}

#[test]
fn review_v3_maximum_packets_and_escaped_input_obey_the_existing_wire_bound() {
    let mut maximum = request(u64::MAX);
    // Every opaque ID already serializes at its full fixed width, regardless
    // of its byte values. Maximize all variable-width sequence fields too.
    maximum.target.key.source.session.generation = u64::MAX;
    maximum.target.key.source.lease_epoch = u64::MAX;
    maximum.target.policy_revision = u64::MAX;
    maximum.target.observed = ObservedIncoming {
        epoch: u64::MAX,
        through: u64::MAX,
    };
    maximum.activation = u64::MAX;
    for message in [
        ChatMessage::Command(ChatCommand::ReviewConversation(maximum)),
        ChatMessage::Command(ChatCommand::QueryReviewReceipt(query(maximum))),
        ChatMessage::Event(ChatEvent::ReviewReceipt(ReviewReceiptReply {
            query: query(maximum),
            receipt: Some(receipt(maximum, ReviewOutcome::RejectedNative)),
        })),
    ] {
        let packet = ChatPacket {
            version: VERSION,
            source: maximum.target.key.source,
            sequence: u64::MAX,
            reply_to: if matches!(message, ChatMessage::Command(_)) { 0 } else { u64::MAX },
            message,
        };
        let encoded = encode(&packet).unwrap();
        assert!(encoded.len() <= MAX_PACKET_BYTES);
        assert_eq!(decode(&encoded), Ok(packet.clone()));
        assert!(
            !encoded.contains(&b'\\'),
            "no accepted field contains free text requiring output escaping"
        );
        let installation = Value::String(maximum.target.key.source.worker.installation.to_string());
        let escaped_one = String::from_utf8(encoded.clone()).unwrap().replacen(
            &serde_json::to_string(&installation).unwrap(),
            &escaped_strings(&installation),
            1,
        );
        assert!(escaped_one.len() <= MAX_PACKET_BYTES);
        let decoded = decode(escaped_one.as_bytes()).unwrap();
        assert_eq!(decoded, packet);
        assert_eq!(encode(&decoded).unwrap(), encoded);
        let escaped = escaped_strings(&serde_json::to_value(&packet).unwrap());
        assert_eq!(serde_json::from_str::<ChatPacket>(&escaped).unwrap(), packet);
        // Arbitrary escaped input must pass the same byte cap before parsing;
        // canonical output is bounded even when accepted input used escapes.
        assert!(escaped.len() > MAX_PACKET_BYTES);
        assert_eq!(decode(escaped.as_bytes()), Err(ChatError::Oversized));
    }
}

#[test]
fn review_v3_packet_validation_binds_source_query_receipt_and_command_direction() {
    let original = request(1);
    let mut other = original;
    other.target.key.source.session.generation += 1;
    for message in [
        ChatMessage::Command(ChatCommand::ReviewConversation(other)),
        ChatMessage::Command(ChatCommand::QueryReviewReceipt(query(other))),
        ChatMessage::Event(ChatEvent::ReviewReceipt(ReviewReceiptReply {
            query: query(other),
            receipt: Some(receipt(other, ReviewOutcome::OpenedLocally)),
        })),
    ] {
        let packet = ChatPacket {
            version: VERSION,
            source: source(),
            sequence: 1,
            reply_to: if matches!(message, ChatMessage::Command(_)) { 0 } else { 1 },
            message,
        };
        assert_eq!(encode(&packet), Err(ChatError::Binding));
        assert_eq!(decode(&serde_json::to_vec(&packet).unwrap()), Err(ChatError::Binding));
    }
    let mut packet = ChatPacket {
        version: VERSION,
        source: source(),
        sequence: 1,
        reply_to: 1,
        message: ChatMessage::Command(ChatCommand::ReviewConversation(original)),
    };
    assert_eq!(packet.validate(), Err(ChatError::Sequence));
    packet.reply_to = 0;
    packet.message = ChatMessage::Command(ChatCommand::QueryReviewReceipt(ReviewReceiptQuery {
        request: 0,
        ..query(original)
    }));
    assert_eq!(packet.validate(), Err(ChatError::Sequence));
    packet.message = ChatMessage::Event(ChatEvent::ReviewReceipt(ReviewReceiptReply {
        query: query(original),
        receipt: Some(receipt(original, ReviewOutcome::OpenedLocally)),
    }));
    assert_eq!(packet.validate(), Err(ChatError::Sequence), "receipt events must answer a command");
    packet.reply_to = 1;
    packet.message = ChatMessage::Event(ChatEvent::ReviewReceipt(ReviewReceiptReply {
        query: query(original),
        receipt: Some(receipt(other, ReviewOutcome::OpenedLocally)),
    }));
    assert_eq!(packet.validate(), Err(ChatError::Binding));
    let mut invalid = original;
    invalid.target.policy_revision = 0;
    packet.message = ChatMessage::Event(ChatEvent::ReviewReceipt(ReviewReceiptReply {
        query: query(original),
        receipt: Some(receipt(invalid, ReviewOutcome::OpenedLocally)),
    }));
    assert_eq!(packet.validate(), Err(ChatError::Sequence));
    invalid = original;
    invalid.target.key.conversation = Conversation::Group(ConversationId::from_bytes([7; 16]).unwrap());
    packet.reply_to = 0;
    packet.message = ChatMessage::Command(ChatCommand::ReviewConversation(invalid));
    assert_eq!(packet.validate(), Err(ChatError::Invalid));
}

#[test]
fn review_lost_reply_and_same_source_reconnect_recover_receipt_without_reopening() {
    let operation = request(1);
    let mut ledger = ReviewReceiptLedger::new(source()).unwrap();
    let mut native_opens = 0;
    if ledger.reserve(operation).unwrap().is_none() {
        native_opens += 1;
    }
    let original = ledger.finish(operation, ReviewOutcome::OpenedLocally).unwrap();
    // The native action finished but its response was lost. Reauthentication of
    // the same control source cannot clear the ledger or resubmit the action.
    assert_eq!(ledger.reset(source()), Ok(false));
    if ledger.reserve(operation).unwrap().is_none() {
        native_opens += 1;
    }
    let recovered = ledger.query(query(operation)).unwrap();
    assert_eq!(recovered.validate_for(operation), Ok(()));
    assert_eq!(recovered.receipt, Some(original));
    assert_eq!(native_opens, 1);
    assert_eq!(ledger.retained_count(), 1);
}

#[test]
fn review_unfinished_reservations_block_replay_new_operations_and_eviction() {
    let mut ledger = ReviewReceiptLedger::new(source()).unwrap();
    for number in 1..MAX_REVIEW_RECEIPTS as u64 {
        let operation = request(number);
        assert_eq!(ledger.reserve(operation), Ok(None));
        ledger.finish(operation, ReviewOutcome::RejectedNative).unwrap();
    }
    let pending = request(MAX_REVIEW_RECEIPTS as u64);
    assert_eq!(ledger.reserve(pending), Ok(None));
    assert_eq!(ledger.retained_count(), MAX_REVIEW_RECEIPTS);
    assert_eq!(ledger.reserve(pending), Err(ChatError::Busy));
    assert_eq!(ledger.reset(source()), Ok(false));
    assert_eq!(ledger.reserve(pending), Err(ChatError::Busy));
    assert_eq!(ledger.reserve(request(pending.request + 1)), Err(ChatError::Busy));
    assert_eq!(ledger.query(query(pending)).unwrap().receipt, None);
    assert_eq!(ledger.retained_count(), MAX_REVIEW_RECEIPTS);
    assert!(
        ledger.query(query(request(1))).unwrap().receipt.is_some(),
        "a blocked admission cannot evict older receipts"
    );
    let final_receipt = ledger.finish(pending, ReviewOutcome::Expired).unwrap();
    assert_eq!(ledger.reserve(pending), Ok(Some(final_receipt)));
}

#[test]
fn review_terminal_receipts_are_strictly_immutable_for_every_outcome() {
    for outcome in [
        ReviewOutcome::OpenedLocally,
        ReviewOutcome::RejectedNative,
        ReviewOutcome::StaleTarget,
        ReviewOutcome::NotForeground,
        ReviewOutcome::Expired,
    ] {
        let operation = request(1);
        let mut ledger = ReviewReceiptLedger::new(source()).unwrap();
        assert_eq!(ledger.reserve(operation), Ok(None));
        let original = ledger.finish(operation, outcome).unwrap();
        assert_eq!(ledger.finish(operation, outcome), Err(ChatError::FinalReceipt));
        assert_eq!(ledger.finish(operation, ReviewOutcome::OpenedLocally), Err(ChatError::FinalReceipt));
        assert_eq!(ledger.reserve(operation), Ok(Some(original)));
        assert_eq!(ledger.query(query(operation)).unwrap().receipt, Some(original));
    }
}

#[test]
fn review_request_changes_cannot_finish_query_or_attribute_another_capture() {
    let original = request(1);
    let mut ledger = ReviewReceiptLedger::new(source()).unwrap();
    ledger.reserve(original).unwrap();
    let mut changed_policy = original;
    changed_policy.target.policy_revision += 1;
    let mut changed_epoch = original;
    changed_epoch.target.observed.epoch += 1;
    let mut changed_through = original;
    changed_through.target.observed.through += 1;
    let mut changed_grant = original;
    changed_grant.activation += 1;
    let mut changed_conversation = original;
    changed_conversation.target.key.conversation = Conversation::Nearby;
    for changed in [changed_policy, changed_epoch, changed_through, changed_grant, changed_conversation] {
        assert_eq!(ledger.reserve(changed), Err(ChatError::RequestChanged));
        assert_eq!(ledger.finish(changed, ReviewOutcome::OpenedLocally), Err(ChatError::RequestChanged));
    }
    assert_eq!(ledger.query(query(changed_conversation)), Err(ChatError::RequestChanged));
    let immutable = ledger.finish(original, ReviewOutcome::OpenedLocally).unwrap();
    assert_eq!(ledger.query(query(changed_conversation)), Err(ChatError::RequestChanged));
    for changed in [changed_policy, changed_epoch, changed_through, changed_grant] {
        assert!(!immutable.matches(changed));
        let reply = ReviewReceiptReply {
            query: query(original),
            receipt: Some(receipt(changed, ReviewOutcome::OpenedLocally)),
        };
        assert_eq!(reply.validate(), Ok(()));
        assert_eq!(reply.validate_for(original), Err(ChatError::RequestChanged));
    }
    for changed in [changed_conversation, ReviewConversationRequest { request: 2, ..original }] {
        let reply = ReviewReceiptReply {
            query: query(original),
            receipt: Some(receipt(changed, ReviewOutcome::OpenedLocally)),
        };
        assert_eq!(reply.validate(), Err(ChatError::Binding));
    }
    let absent = ReviewReceiptReply {
        query: query(original),
        receipt: None,
    };
    assert_eq!(absent.validate_for(original), Ok(()));
    assert_eq!(absent.validate_for(changed_conversation), Err(ChatError::Binding));
    assert_eq!(
        ledger.finish(request(2), ReviewOutcome::OpenedLocally),
        Err(ChatError::UnknownReceipt)
    );
}

#[test]
fn review_bounded_history_fences_evicted_and_skipped_ids_and_cannot_overflow() {
    let mut ledger = ReviewReceiptLedger::new(source()).unwrap();
    for number in 1..=MAX_REVIEW_RECEIPTS as u64 + 1 {
        let operation = request(number * 2);
        ledger.reserve(operation).unwrap();
        ledger.finish(operation, ReviewOutcome::OpenedLocally).unwrap();
        assert!(ledger.retained_count() <= MAX_REVIEW_RECEIPTS);
    }
    assert_eq!(ledger.retained_count(), MAX_REVIEW_RECEIPTS);
    assert_eq!(ledger.reserve(request(2)), Err(ChatError::StaleRequest));
    assert_eq!(ledger.reserve(request(3)), Err(ChatError::StaleRequest));
    assert_eq!(ledger.query(query(request(2))).unwrap().receipt, None);
    assert_eq!(
        ledger.finish(request(2), ReviewOutcome::OpenedLocally),
        Err(ChatError::UnknownReceipt)
    );
    let last = request(u64::MAX);
    ledger.reserve(last).unwrap();
    let final_receipt = ledger.finish(last, ReviewOutcome::Expired).unwrap();
    assert_eq!(ledger.reserve(last), Ok(Some(final_receipt)));
    assert_eq!(ledger.reserve(request(u64::MAX - 1)), Err(ChatError::StaleRequest));
    assert_eq!(ledger.reserve(request(0)), Err(ChatError::Sequence));
    assert_eq!(ledger.query(query(request(0))), Err(ChatError::Sequence));
}

#[test]
fn review_source_reset_is_monotonic_exact_and_retires_old_reservations() {
    let original = request(1);
    let mut ledger = ReviewReceiptLedger::new(source()).unwrap();
    ledger.reserve(original).unwrap();
    let mut next = source();
    next.session.generation += 1;
    next.session.account = AccountKey::from_bytes([8; 32]).unwrap();
    assert_eq!(ledger.reset(next), Ok(true));
    assert_eq!(ledger.retained_count(), 0);
    assert_eq!(ledger.reserve(original), Err(ChatError::Binding));
    assert_eq!(ledger.finish(original, ReviewOutcome::OpenedLocally), Err(ChatError::Binding));
    assert_eq!(ledger.query(query(original)), Err(ChatError::Binding));
    assert_eq!(ledger.reset(source()), Err(ChatError::Binding));
    let mut current = original;
    current.target.key.source = next;
    ledger.reserve(current).unwrap();
    let mut invalid = next;
    invalid.session.grid = GridKey::from_bytes([9; 32]).unwrap();
    assert_eq!(ledger.reset(invalid), Err(ChatError::Binding));
    invalid = next;
    invalid.lease_epoch = 0;
    assert_eq!(ledger.reset(invalid), Err(ChatError::Binding));
    assert_eq!(
        ledger.reserve(current),
        Err(ChatError::Busy),
        "rejected resets preserve the unfinished reservation"
    );

    let mut newer_lease = next;
    newer_lease.lease_epoch += 1;
    assert_eq!(ledger.reset(newer_lease), Ok(true));
    assert_eq!(ledger.reset(next), Err(ChatError::Binding));
    let mut wrong = current;
    wrong.target.key.source = next;
    assert_eq!(ledger.reserve(wrong), Err(ChatError::Binding));
    current.target.key.source = newer_lease;
    assert_eq!(
        ledger.reserve(current),
        Ok(None),
        "a fresh exact source owns its own request namespace"
    );
    let mut changed_login = newer_lease;
    changed_login.session.session = SessionId::from_bytes([10; 16]).unwrap();
    changed_login.lease_epoch += 1;
    assert_eq!(
        ledger.reset(changed_login),
        Err(ChatError::Binding),
        "a new lease cannot excuse a stale login generation"
    );
    changed_login.session.generation += 1;
    assert_eq!(ledger.reset(changed_login), Ok(true));
    let mut other_worker = changed_login;
    other_worker.worker.worker = WorkerId::from_bytes([11; 16]).unwrap();
    assert_eq!(
        ledger.reset(other_worker),
        Ok(true),
        "a fresh worker identity follows the common source rule"
    );
}
