use aurora_session::chat::{self, *};
use aurora_session::{
    AccountKey, Binding, ConversationId, GridKey, HostId, InstallationId, Label, SessionId, SessionIdentity, WorkerId, WorkerIdentity,
};

fn source(value: u8, generation: u64) -> ChatSource {
    ChatSource {
        worker: WorkerIdentity {
            installation: InstallationId::from_bytes([1; 32]).unwrap(),
            host: HostId::from_bytes([2; 16]).unwrap(),
            worker: WorkerId::from_bytes([value; 16]).unwrap(),
        },
        session: SessionIdentity {
            session: SessionId::from_bytes([value; 16]).unwrap(),
            account: AccountKey::from_bytes([value; 32]).unwrap(),
            grid: GridKey::from_bytes([3; 32]).unwrap(),
            generation,
        },
        lease_epoch: 1,
    }
}
fn key(source: ChatSource, conversation: Conversation) -> ChatKey {
    ChatKey { source, conversation }
}
fn policy(key: ChatKey, revision: u64) -> NativeChatPolicy {
    NativeChatPolicy {
        key,
        revision,
        lease_current: true,
        ready: true,
        can_send: true,
        can_read: true,
    }
}
fn request(key: ChatKey, number: u64, revision: u64) -> SendRequest {
    SendRequest {
        key,
        request: number,
        policy_revision: revision,
        text: ChatText::new("Synthetic private test words").unwrap(),
    }
}
fn line(key: ChatKey, event: u64) -> ChatLine {
    ChatLine {
        key,
        event,
        policy_revision: 1,
        sender: Label::new("Synthetic sender").unwrap(),
        text: ChatText::new("Synthetic private line").unwrap(),
        kind: LineKind::Incoming,
        truncated: false,
    }
}

#[test]
fn chat_utf8_byte_limits_controls_and_debug_redaction_are_enforced() {
    assert_eq!(ChatText::new(&"界".repeat(341)).unwrap().as_str().len(), 1023);
    assert_eq!(ChatText::new(&"界".repeat(342)), Err(ChatError::TooLong));
    assert_eq!(ChatText::new("nul\0truncation"), Err(ChatError::Invalid));
    assert_eq!(ChatText::new("hidden\u{001b}[31m"), Err(ChatError::Invalid));
    assert!(ChatText::new("literal\nline\twith\r\nUnicode 🌕").is_ok());
    let text = ChatText::new("Synthetic private test words").unwrap();
    assert!(!format!("{text:?}").contains("Synthetic private"));
    assert!(serde_json::from_str::<ChatText>("\"bad\\u0000text\"").is_err());
    let mut empty = request(key(source(4, 1), Conversation::Nearby), 1, 1);
    empty.text = ChatText::new(" \n\t").unwrap();
    assert_eq!(empty.validate(), Err(ChatError::Invalid));
}

#[test]
fn chat_wire_is_separate_bounded_and_rejects_offers_generic_input_and_cross_source_body() {
    let beta = source(4, 1);
    let key = key(beta, Conversation::Nearby);
    let mut send = request(key, u64::MAX, 1);
    send.text = ChatText::new(&"\"".repeat(MAX_TEXT_BYTES)).unwrap();
    let mut packet = ChatPacket {
        version: VERSION,
        source: beta,
        sequence: u64::MAX,
        reply_to: 0,
        message: ChatMessage::Command(ChatCommand::Send(send)),
    };
    let encoded = chat::encode(&packet).unwrap();
    assert!(encoded.len() <= MAX_PACKET_BYTES);
    assert_eq!(chat::decode(&encoded), Ok(packet.clone()));
    assert_eq!(chat::decode(&vec![b' '; MAX_PACKET_BYTES + 1]), Err(ChatError::Oversized));
    let mut unknown: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    unknown["message"]["body"]["type"] = "accept_offer".into();
    assert_eq!(chat::decode(&serde_json::to_vec(&unknown).unwrap()), Err(ChatError::Malformed));
    unknown["message"]["body"]["type"] = "send_keys".into();
    assert_eq!(chat::decode(&serde_json::to_vec(&unknown).unwrap()), Err(ChatError::Malformed));
    if let ChatMessage::Command(ChatCommand::Send(send)) = &mut packet.message {
        send.key.source = source(5, 1);
    }
    assert_eq!(chat::encode(&packet), Err(ChatError::Binding));
    packet.version += 1;
    assert_eq!(chat::encode(&packet), Err(ChatError::Version));

    let packet = ChatPacket {
        version: VERSION,
        source: beta,
        sequence: 1,
        reply_to: 0,
        message: ChatMessage::Event(ChatEvent::Line(ChatLine {
            sender: Label::new("Private sender label").unwrap(),
            ..line(key, 1)
        })),
    };
    let debug = format!("{packet:?}");
    assert!(!debug.contains("Private sender") && !debug.contains("Synthetic private"));
}

#[test]
fn send_admission_binds_every_source_domain_conversation_and_native_policy_revision() {
    let beta = source(4, 1);
    let key = key(beta, Conversation::Direct(ConversationId::from_bytes([9; 16]).unwrap()));
    let mut ledger = SendLedger::new(beta).unwrap();
    let original = request(key, 1, 1);
    for field in 0..8 {
        let mut changed = original.clone();
        match field {
            0 => changed.key.source.worker.installation = InstallationId::from_bytes([9; 32]).unwrap(),
            1 => changed.key.source.worker.host = HostId::from_bytes([9; 16]).unwrap(),
            2 => changed.key.source.worker.worker = WorkerId::from_bytes([9; 16]).unwrap(),
            3 => changed.key.source.session.session = SessionId::from_bytes([9; 16]).unwrap(),
            4 => changed.key.source.session.account = AccountKey::from_bytes([9; 32]).unwrap(),
            5 => changed.key.source.session.grid = GridKey::from_bytes([9; 32]).unwrap(),
            6 => changed.key.source.session.generation = 2,
            _ => changed.key.source.lease_epoch = 2,
        }
        assert_eq!(ledger.admit(&changed, policy(changed.key, 1)), Err(ChatError::Binding));
    }
    let wrong_conversation = ChatKey {
        conversation: Conversation::Group(ConversationId::from_bytes([9; 16]).unwrap()),
        ..key
    };
    assert_eq!(ledger.admit(&original, policy(wrong_conversation, 1)), Err(ChatError::Binding));
    assert_eq!(ledger.admit(&original, policy(key, 2)), Err(ChatError::StalePolicy));
    let current = request(key, 1, 2);
    assert_eq!(
        ledger.admit(&current, policy(key, 2)),
        Ok(SendAdmission::SubmitOnce),
        "no active-world or keyboard-focus grant is required for the exact inactive source"
    );
}

#[test]
fn duplicate_uncertain_and_evicted_requests_never_resubmit_native_mutation() {
    let beta = source(4, 1);
    let key = key(beta, Conversation::Nearby);
    let native = policy(key, 1);
    let mut ledger = SendLedger::new(beta).unwrap();
    let first = request(key, 1, 1);
    assert_eq!(ledger.admit(&first, native), Ok(SendAdmission::SubmitOnce));
    assert_eq!(
        ledger.admit(&first, native),
        Ok(SendAdmission::Duplicate(SendReceipt {
            key,
            request: 1,
            outcome: SendOutcome::Pending
        }))
    );
    assert_eq!(ledger.admit(&request(key, 2, 1), native), Err(ChatError::Busy));
    ledger.finish(key, 1, SendOutcome::Uncertain).unwrap();
    assert_eq!(
        ledger.admit(&first, native),
        Ok(SendAdmission::Duplicate(SendReceipt {
            key,
            request: 1,
            outcome: SendOutcome::Uncertain
        }))
    );
    let mut changed = first.clone();
    changed.text = ChatText::new("Changed target text").unwrap();
    assert_eq!(ledger.admit(&changed, native), Err(ChatError::RequestChanged));
    ledger.finish(key, 1, SendOutcome::AcceptedLocally).unwrap();
    assert_eq!(ledger.finish(key, 1, SendOutcome::RejectedNative), Err(ChatError::FinalReceipt));
    for number in 2..=(MAX_RECEIPTS as u64 + 3) {
        assert_eq!(ledger.admit(&request(key, number, 1), native), Ok(SendAdmission::SubmitOnce));
        ledger.finish(key, number, SendOutcome::AcceptedLocally).unwrap();
    }
    assert_eq!(ledger.retained_count(), MAX_RECEIPTS);
    assert_eq!(ledger.admit(&first, native), Err(ChatError::StaleRequest));
    assert_eq!(ledger.finish(key, 1, SendOutcome::AcceptedLocally), Err(ChatError::UnknownReceipt));
}

#[test]
fn permission_changes_cannot_be_rolled_back_by_a_stale_policy_snapshot() {
    let beta = source(4, 1);
    let key = key(beta, Conversation::Nearby);
    let mut ledger = SendLedger::new(beta).unwrap();
    let old = policy(key, 1);
    let mut lease_ledger = SendLedger::new(beta).unwrap();
    assert_eq!(
        lease_ledger.admit(
            &request(key, 1, 2),
            NativeChatPolicy {
                lease_current: false,
                ..policy(key, 2)
            }
        ),
        Err(ChatError::Unavailable)
    );
    assert_eq!(
        lease_ledger.admit(&request(key, 1, 1), old),
        Err(ChatError::StalePolicy),
        "revoked lease revision must remain newer than a cached valid snapshot"
    );
    assert_eq!(
        ledger.admit(
            &request(key, 1, 1),
            NativeChatPolicy {
                lease_current: false,
                ..old
            }
        ),
        Err(ChatError::Unavailable)
    );
    let denied = NativeChatPolicy {
        can_send: false,
        ..policy(key, 2)
    };
    assert_eq!(ledger.admit(&request(key, 1, 2), denied), Err(ChatError::Denied));
    assert_eq!(ledger.admit(&request(key, 1, 1), old), Err(ChatError::StalePolicy));
    assert_eq!(
        ledger.admit(&request(key, 1, 2), NativeChatPolicy { can_send: true, ..denied }),
        Err(ChatError::StalePolicy)
    );
    assert_eq!(
        ledger.admit(
            &request(key, 1, 3),
            NativeChatPolicy {
                ready: false,
                ..policy(key, 3)
            }
        ),
        Err(ChatError::Unavailable)
    );
    assert_eq!(ledger.admit(&request(key, 1, 4), policy(key, 4)), Ok(SendAdmission::SubmitOnce));
}

#[test]
fn source_reset_clears_private_history_drafts_and_receipts_without_reopening_same_session() {
    let beta = source(4, 1);
    let key = key(beta, Conversation::Nearby);
    let mut ledger = SendLedger::new(beta).unwrap();
    let mut drafts = ChatDrafts::new(beta).unwrap();
    let mut history = ChatHistory::new(beta).unwrap();
    let send = request(key, 1, 1);
    ledger.admit(&send, policy(key, 1)).unwrap();
    ledger.finish(key, 1, SendOutcome::Uncertain).unwrap();
    drafts.set(key, send.text.clone()).unwrap();
    history.push(line(key, 1), policy(key, 1)).unwrap();
    assert!(!ledger.reset(beta).unwrap() && !drafts.reset(beta).unwrap() && !history.reset(beta).unwrap());
    assert!(matches!(ledger.admit(&send, policy(key, 1)).unwrap(), SendAdmission::Duplicate(_)));
    assert_eq!(drafts.retained_count(), 1);
    assert_eq!(history.retained_count(), 1);
    let mut changed = source(4, 2);
    changed.session.session = SessionId::from_bytes([8; 16]).unwrap();
    assert!(ledger.reset(changed).unwrap() && drafts.reset(changed).unwrap() && history.reset(changed).unwrap());
    assert_eq!(ledger.retained_count(), 0);
    assert_eq!(drafts.retained_count(), 0);
    assert_eq!(history.retained_count(), 0);
    assert_eq!(ledger.admit(&send, policy(key, 1)), Err(ChatError::Binding));
    assert!(drafts.get(key).is_none());
    assert!(ledger.reset(beta).is_err());
    let renewed = ChatSource { lease_epoch: 2, ..changed };
    assert!(ledger.reset(renewed).unwrap());
    assert!(ledger.reset(changed).is_err());
    assert!(
        ChatSource::try_from((
            Binding {
                session: None,
                ..beta.binding()
            },
            1
        ))
        .is_err()
    );
}

#[test]
fn history_retention_is_bounded_gap_aware_and_current_receive_policy_gates_access() {
    let beta = source(4, 1);
    let key = key(beta, Conversation::Nearby);
    let allowed = policy(key, 1);
    let mut history = ChatHistory::new(beta).unwrap();
    history.push(line(key, 1), allowed).unwrap();
    history.push(line(key, 3), allowed).unwrap();
    assert!(history.has_gap());
    assert_eq!(history.push(line(key, 3), allowed), Err(ChatError::Sequence));
    assert_eq!(history.lines(allowed).count(), 2);
    let denied = NativeChatPolicy {
        can_read: false,
        ..policy(key, 2)
    };
    assert_eq!(
        history.push(line(key, 4), denied),
        Err(ChatError::StalePolicy),
        "a queued old line must still apply a freshly revoked native read policy"
    );
    assert_eq!(history.retained_count(), 0);
    assert_eq!(history.lines(allowed).count(), 0, "old read snapshots cannot reveal retained text");
    assert_eq!(history.apply_policy(allowed), Err(ChatError::StalePolicy));
    assert_eq!(
        history.push(
            ChatLine {
                policy_revision: 2,
                ..line(key, 4)
            },
            denied
        ),
        Err(ChatError::Denied)
    );
    let restored = policy(key, 3);
    assert_eq!(history.push(line(key, 4), restored), Err(ChatError::StalePolicy));
    for event in 4..=(MAX_RETAINED_LINES as u64 + 9) {
        history
            .push(
                ChatLine {
                    policy_revision: 3,
                    ..line(key, event)
                },
                restored,
            )
            .unwrap();
    }
    assert_eq!(history.retained_count(), MAX_RETAINED_LINES);
    assert_eq!(history.lines(restored).count(), MAX_RETAINED_LINES);
    let other = self::key(source(5, 1), Conversation::Nearby);
    assert_eq!(history.push(line(other, 1000), restored), Err(ChatError::Binding));
    history.note_gap(beta, 1000).unwrap();
    assert_eq!(history.push(line(key, 999), restored), Err(ChatError::Sequence));
}

#[test]
fn drafts_have_exact_conversation_identity_and_a_hard_catalogue_bound() {
    let beta = source(4, 1);
    let mut drafts = ChatDrafts::new(beta).unwrap();
    for value in 1..=MAX_CONVERSATIONS {
        let key = key(beta, Conversation::Direct(ConversationId::from_bytes([value as u8; 16]).unwrap()));
        drafts.set(key, ChatText::new("Synthetic draft").unwrap()).unwrap();
    }
    assert_eq!(drafts.retained_count(), MAX_CONVERSATIONS);
    let overflow = key(beta, Conversation::Nearby);
    assert_eq!(drafts.set(overflow, ChatText::new("Overflow").unwrap()), Err(ChatError::Capacity));
    let first = key(beta, Conversation::Direct(ConversationId::from_bytes([1; 16]).unwrap()));
    drafts.set(first, ChatText::new("Updated").unwrap()).unwrap();
    assert_eq!(drafts.get(first).unwrap().as_str(), "Updated");
    let alias = key(beta, Conversation::Group(ConversationId::from_bytes([1; 16]).unwrap()));
    assert!(drafts.get(alias).is_none());
    drafts.set(first, ChatText::default()).unwrap();
    drafts.set(overflow, ChatText::new("Now there is room").unwrap()).unwrap();
    assert_eq!(drafts.retained_count(), MAX_CONVERSATIONS);
}

fn read_request(key: ChatKey, request: u64) -> MarkReadRequest {
    MarkReadRequest {
        key,
        request,
        policy_revision: 1,
        observed: ObservedIncoming { epoch: 7, through: 9 },
        unread_revision: 11,
    }
}

#[test]
fn chat_v2_read_codec_maximal_rows_receipts_and_closed_schema_fit_packets() {
    let mut source = source(255, u64::MAX);
    source.lease_epoch = u64::MAX;
    for title in ["\"\\".repeat(64), "界".repeat(42)] {
        let rows: Vec<_> = (1..=MAX_READ_ROWS)
            .map(|index| ReadStateRow {
                conversation: Conversation::Direct(ConversationId::from_bytes([index as u8; 16]).unwrap()),
                title: Label::new(&title).unwrap(),
                policy_revision: u64::MAX,
                lease_current: true,
                ready: true,
                can_send: false,
                can_read: true,
                read: Some(ReadMetadata {
                    unread: u64::MAX,
                    unread_revision: u64::MAX,
                    observed: ObservedIncoming {
                        epoch: u64::MAX,
                        through: u64::MAX,
                    },
                    can_mark_read: false,
                }),
            })
            .collect();
        let mut packet = ChatPacket {
            version: VERSION,
            source,
            sequence: u64::MAX,
            reply_to: u64::MAX,
            message: ChatMessage::Event(ChatEvent::ReadBatch(rows.clone())),
        };
        let encoded = encode(&packet).unwrap();
        assert!(encoded.len() <= 3840, "compact source-relative maximum: {}", encoded.len());
        assert_eq!(decode(&encoded), Ok(packet.clone()));
        packet.version = 1;
        assert_eq!(decode(&serde_json::to_vec(&packet).unwrap()), Err(ChatError::Version));
        packet.version = VERSION;
        let mut unknown: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        unknown["message"]["body"]["data"][0]["read"]["export_event"] = 99.into();
        assert_eq!(decode(&serde_json::to_vec(&unknown).unwrap()), Err(ChatError::Malformed));
        packet.message = ChatMessage::Event(ChatEvent::ReadBatch(vec![rows[0].clone(); 2]));
        assert_eq!(packet.validate(), Err(ChatError::Invalid));
        packet.message = ChatMessage::Event(ChatEvent::ReadBatch(vec![rows[0].clone(); MAX_READ_ROWS + 1]));
        assert_eq!(packet.validate(), Err(ChatError::Capacity));
        let mut denied = rows[0].clone();
        denied.can_read = false;
        packet.message = ChatMessage::Event(ChatEvent::ReadBatch(vec![denied.clone()]));
        assert_eq!(packet.validate(), Err(ChatError::Denied));
        denied.read = None;
        packet.message = ChatMessage::Event(ChatEvent::ReadBatch(vec![denied]));
        assert!(encode(&packet).is_ok());
    }
    // Every Poll event remains a separate bounded packet: inline read policy
    // rows do not share a packet with a maximum-size escaped chat line.
    let largest_key = key(source, Conversation::Direct(ConversationId::from_bytes([255; 16]).unwrap()));
    let largest_policy = NativeChatPolicy {
        key: largest_key,
        revision: u64::MAX,
        lease_current: true,
        ready: true,
        can_send: false,
        can_read: true,
    };
    for event in [
        ChatEvent::Conversation(ConversationInfo {
            policy: largest_policy,
            title: Label::new(&"\"\\".repeat(64)).unwrap(),
        }),
        ChatEvent::Line(ChatLine {
            key: largest_key,
            event: u64::MAX,
            policy_revision: u64::MAX,
            sender: Label::new(&"\"\\".repeat(64)).unwrap(),
            text: ChatText::new(&"\\".repeat(MAX_TEXT_BYTES)).unwrap(),
            kind: LineKind::Incoming,
            truncated: false,
        }),
        ChatEvent::Receipt(SendReceipt {
            key: largest_key,
            request: u64::MAX,
            outcome: SendOutcome::AcceptedLocally,
        }),
        ChatEvent::Gap { through_event: u64::MAX },
    ] {
        let packet = ChatPacket {
            version: VERSION,
            source,
            sequence: u64::MAX,
            reply_to: u64::MAX,
            message: ChatMessage::Event(event),
        };
        let encoded = encode(&packet).unwrap();
        assert!(encoded.len() <= MAX_PACKET_BYTES);
        assert_eq!(decode(&encoded), Ok(packet));
    }
    let mut request = read_request(key(source, Conversation::Nearby), u64::MAX);
    request.observed = ObservedIncoming {
        epoch: u64::MAX,
        through: u64::MAX,
    };
    request.unread_revision = u64::MAX;
    request.policy_revision = u64::MAX;
    let mut ledger = ReadReceiptLedger::new(source);
    assert_eq!(ledger.reserve(request), Ok(None));
    let receipt = ledger.finish(request, ReadOutcome::Applied).unwrap();
    let query = ReadReceiptQuery {
        key: request.key,
        request: request.request,
    };
    let reply = ledger.query(query).unwrap();
    assert_eq!(reply.receipt, Some(receipt));
    for message in [
        ChatMessage::Command(ChatCommand::MarkRead(request)),
        ChatMessage::Command(ChatCommand::QueryReadReceipt(query)),
        ChatMessage::Event(ChatEvent::ReadReceipt(reply)),
    ] {
        let reply_to = if matches!(message, ChatMessage::Command(_)) { 0 } else { u64::MAX };
        let packet = ChatPacket {
            version: VERSION,
            source,
            sequence: u64::MAX,
            reply_to,
            message,
        };
        let encoded = encode(&packet).unwrap();
        assert_eq!(decode(&encoded), Ok(packet));
        assert!(!String::from_utf8(encoded).unwrap().contains("unread_count"));
    }
}

#[test]
fn chat_read_receipts_are_immutable_exact_bounded_and_evicted_requests_never_reapply() {
    let source = source(4, 1);
    let key = key(source, Conversation::Nearby);
    let mut ledger = ReadReceiptLedger::new(source);
    let first = read_request(key, 1);
    assert_eq!(ledger.reserve(first), Ok(None));
    assert_eq!(ledger.reserve(read_request(key, 2)), Err(ChatError::Busy));
    let receipt = ledger.finish(first, ReadOutcome::Applied).unwrap();
    assert_eq!(ledger.reserve(first), Ok(Some(receipt)));
    assert_eq!(ledger.finish(first, ReadOutcome::StaleObservation), Err(ChatError::FinalReceipt));
    let changed = MarkReadRequest {
        unread_revision: 12,
        ..first
    };
    assert_eq!(ledger.reserve(changed), Err(ChatError::RequestChanged));
    for number in 2..=MAX_READ_RECEIPTS as u64 + 1 {
        let request = read_request(key, number);
        assert_eq!(ledger.reserve(request), Ok(None));
        ledger.finish(request, ReadOutcome::AlreadyRead).unwrap();
    }
    assert_eq!(ledger.reserve(first), Err(ChatError::StaleRequest));
    assert_eq!(ledger.query(ReadReceiptQuery { key, request: 1 }).unwrap().receipt, None);
    let mut wrong = key;
    wrong.source.session.generation += 1;
    assert_eq!(ledger.query(ReadReceiptQuery { key: wrong, request: 1 }), Err(ChatError::Binding));
}
