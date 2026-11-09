use super::*;
use crate::{AccountKey, GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerId, WorkerIdentity};

fn source(generation: u64) -> ChatSource {
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
            generation,
        },
        lease_epoch: 1,
    }
}
fn peer(id: u8) -> ConversationId {
    ConversationId::from_bytes([id; 16]).unwrap()
}
fn row(id: u8, name: &str, online: bool, allowed: bool) -> ContactRow {
    ContactRow {
        resident: peer(id),
        display: ContactText::new(name).unwrap(),
        username: ContactText::new(name).unwrap(),
        online,
        can_open: allowed,
    }
}
fn directory() -> ContactDirectory {
    let mut directory = ContactDirectory::new(source(1)).unwrap();
    directory
        .set_policy(ContactsPolicy {
            source: source(1),
            revision: 1,
            can_browse: true,
        })
        .unwrap();
    directory
}
fn page(request: ContactPageRequest, total: u16, rows: Vec<ContactRow>) -> ContactPage {
    ContactPage {
        key: request.key,
        policy_revision: 1,
        offset: request.offset,
        total,
        limited: false,
        pending: false,
        rows,
    }
}
fn open(request: u64) -> OpenDirectRequest {
    OpenDirectRequest {
        selection: ContactKey {
            source: source(1),
            channel: ContactChannel::Friends,
            token: 1,
        },
        resident: peer(9),
        request,
        policy_revision: 1,
    }
}
fn opened(request: OpenDirectRequest) -> OpenDirectOutcome {
    OpenDirectOutcome::Opened {
        resident: request.resident,
        key: ChatKey {
            source: request.selection.source,
            conversation: Conversation::Direct(request.resident),
        },
    }
}

#[test]
fn contacts_complete_pages_are_atomic_sorted_and_offline_friends_remain_selectable() {
    let mut d = directory();
    let request = d.begin(ContactChannel::Friends, 1).unwrap();
    assert!(!d.accept(&page(request, 3, vec![row(9, "Zed", false, true)])).unwrap());
    assert!(d.completed(ContactChannel::Friends).is_none());
    assert!(!d.selected(request.key, peer(9)));
    let next = d.request(ContactChannel::Friends).unwrap();
    assert_eq!(next.offset, 1);
    assert!(
        d.accept(&page(next, 3, vec![row(8, "bob", true, false), row(7, "Alice", true, true)]))
            .unwrap()
    );
    let complete = d.completed(ContactChannel::Friends).unwrap();
    assert_eq!(
        complete.rows.iter().map(|row| row.resident).collect::<Vec<_>>(),
        vec![peer(7), peer(8), peer(9)]
    );
    assert!(d.selected(request.key, peer(9)));
    assert!(!d.selected(request.key, peer(8)));
    assert!(d.request(ContactChannel::Friends).is_none());
}

#[test]
fn contacts_new_search_retires_old_membership_while_friend_refresh_retains_its_last_complete_list() {
    let mut d = directory();
    let friends = d.begin(ContactChannel::Friends, 1).unwrap();
    d.accept(&page(friends, 1, vec![row(9, "Friend", false, true)])).unwrap();
    let search = d.begin(ContactChannel::Search, 1).unwrap();
    d.accept(&page(search, 1, vec![row(8, "Search", false, true)])).unwrap();
    let refreshing = d.begin(ContactChannel::Friends, 2).unwrap();
    let searching = d.begin(ContactChannel::Search, 2).unwrap();
    assert!(d.selected(friends.key, peer(9)));
    assert!(!d.selected(search.key, peer(8)));
    assert!(d.completed(ContactChannel::Search).is_none());
    assert_eq!(d.fail(search.key), Err(ChatError::StaleRequest));
    assert_eq!(d.request(ContactChannel::Search), Some(searching));
    d.fail(searching.key).unwrap();
    assert!(d.completed(ContactChannel::Search).is_none());
    assert_eq!(d.request(ContactChannel::Friends), Some(refreshing));
    assert!(d.selected(friends.key, peer(9)));
}

#[test]
fn contacts_bad_page_source_token_cursor_total_and_cross_page_duplicate_never_publish_partial_rows() {
    let mut d = directory();
    let first = d.begin(ContactChannel::Friends, 1).unwrap();
    d.accept(&page(first, 2, vec![row(9, "First", false, true)])).unwrap();
    let next = d.request(ContactChannel::Friends).unwrap();
    let correct = page(next, 2, vec![row(8, "Second", true, true)]);
    let mut foreign = correct.clone();
    foreign.key.source = source(2);
    assert_eq!(d.accept(&foreign), Err(ChatError::Binding));
    let mut wrong_token = correct.clone();
    wrong_token.key.token = 2;
    assert_eq!(d.accept(&wrong_token), Err(ChatError::StaleRequest));
    let mut wrong_cursor = correct.clone();
    wrong_cursor.offset = 0;
    assert_eq!(d.accept(&wrong_cursor), Err(ChatError::StaleRequest));
    let mut changed_total = correct.clone();
    changed_total.total = 3;
    assert_eq!(d.accept(&changed_total), Err(ChatError::Invalid));
    let mut duplicate = correct.clone();
    duplicate.rows[0].resident = peer(9);
    assert_eq!(d.accept(&duplicate), Err(ChatError::Invalid));
    assert_eq!(d.request(ContactChannel::Friends), Some(next));
    assert!(d.completed(ContactChannel::Friends).is_none());
    assert!(d.accept(&correct).unwrap());
}

#[test]
fn contacts_revoked_policy_clears_rows_and_staging_without_resetting_token_highwater() {
    let mut d = directory();
    let request = d.begin(ContactChannel::Friends, 1).unwrap();
    let ready = page(request, 1, vec![row(9, "Known", true, true)]);
    d.accept(&ready).unwrap();
    d.set_policy(ContactsPolicy {
        source: source(1),
        revision: 2,
        can_browse: false,
    })
    .unwrap();
    assert!(d.completed(ContactChannel::Friends).is_none());
    assert_eq!(d.accept(&ready), Err(ChatError::Denied));
    d.set_policy(ContactsPolicy {
        source: source(1),
        revision: 3,
        can_browse: true,
    })
    .unwrap();
    assert_eq!(d.begin(ContactChannel::Friends, 1), Err(ChatError::StaleRequest));
    assert!(d.begin(ContactChannel::Friends, 2).is_ok());
    assert_eq!(d.accept(&ready), Err(ChatError::StalePolicy));
    assert!(!d.selected(ready.key, peer(9)));
    assert_eq!(
        d.set_policy(ContactsPolicy {
            source: source(1),
            revision: 3,
            can_browse: false
        }),
        Err(ChatError::StalePolicy)
    );
}

#[test]
fn contacts_source_replacement_clears_selection_and_refuses_rollback() {
    let mut d = directory();
    let request = d.begin(ContactChannel::Friends, 1).unwrap();
    d.accept(&page(request, 1, vec![row(9, "Old account", true, true)])).unwrap();
    assert!(d.reset(source(2)).unwrap());
    assert!(!d.selected(request.key, peer(9)));
    assert!(d.completed(ContactChannel::Friends).is_none());
    assert_eq!(d.reset(source(1)), Err(ChatError::Binding));
    assert_eq!(
        d.accept(&page(request, 1, vec![row(9, "Old account", true, true)])),
        Err(ChatError::Binding)
    );
}

#[test]
fn contacts_pending_empty_and_channel_budgets_are_distinct() {
    let mut d = directory();
    let request = d.begin(ContactChannel::Search, 1).unwrap();
    let mut pending = page(request, 0, vec![]);
    pending.pending = true;
    assert!(!d.accept(&pending).unwrap());
    assert!(d.completed(ContactChannel::Search).is_none());
    pending.total = 1;
    assert_eq!(d.accept(&pending), Err(ChatError::Invalid));
    assert!(d.accept(&page(request, 0, vec![])).unwrap());
    assert!(d.completed(ContactChannel::Search).unwrap().rows.is_empty());
    let request = d.begin(ContactChannel::Search, 2).unwrap();
    assert_eq!(
        d.accept(&page(request, 101, vec![row(9, "Over limit", true, true)])),
        Err(ChatError::Capacity)
    );
    let request = d.begin(ContactChannel::Friends, 1).unwrap();
    let mut invalid = page(request, 33, (1..=33).map(|id| row(id, "Too many in page", false, true)).collect());
    assert_eq!(invalid.validate(), Err(ChatError::Capacity));
    invalid.rows.clear();
    invalid.total = 1;
    assert_eq!(invalid.validate(), Err(ChatError::Invalid));
}

#[test]
fn contacts_text_limits_are_utf8_bytes_and_controls_cannot_cross_the_protocol() {
    assert!(ContactText::<255>::new(&format!("{}a", "é".repeat(127))).is_ok());
    assert!(ContactText::<255>::new(&"é".repeat(128)).is_err());
    assert!(ContactText::<127>::new(&"u".repeat(127)).is_ok());
    assert!(ContactText::<127>::new(&"u".repeat(128)).is_err());
    for invalid in ["tab\tname", "line\nname", "nul\0name", "delete\u{7f}"] {
        assert!(ContactText::<255>::new(invalid).is_err());
        assert!(serde_json::from_value::<ContactText<255>>(serde_json::json!(invalid)).is_err());
    }
    assert!(row(9, "", false, true).validate().is_err());
}

#[test]
fn contacts_open_reserves_once_duplicate_and_query_never_authorize_another_native_open() {
    let request = open(1);
    let mut ledger = OpenDirectLedger::new(source(1)).unwrap();
    let mut opens = 0;
    if ledger.reserve(request).unwrap().is_none() {
        opens += 1;
    }
    assert_eq!(ledger.query(request).unwrap().state, OpenDirectState::Pending);
    assert!(ledger.reserve(request).unwrap().is_some());
    assert_eq!(ledger.reserve(open(2)), Err(ChatError::Busy));
    let receipt = ledger.finish(request, opened(request)).unwrap();
    assert_eq!(ledger.query(request).unwrap(), receipt);
    assert_eq!(ledger.reserve(request).unwrap(), Some(receipt.clone()));
    assert_eq!(opens, 1);
    let mut changed = request;
    changed.resident = peer(8);
    assert_eq!(ledger.query(changed), Err(ChatError::RequestChanged));
    assert_eq!(ledger.reserve(changed), Err(ChatError::RequestChanged));
    assert_eq!(
        ledger.finish(request, OpenDirectOutcome::RejectedNative),
        Err(ChatError::FinalReceipt)
    );
}

#[test]
fn contacts_open_eviction_keeps_highwater_and_foreign_queries_cannot_disclose_receipts() {
    let mut ledger = OpenDirectLedger::new(source(1)).unwrap();
    assert_eq!(ledger.query(open(400)).unwrap().state, OpenDirectState::Unknown);
    for id in 1..=MAX_RECEIPTS as u64 + 1 {
        let request = open(id);
        assert!(ledger.reserve(request).unwrap().is_none());
        ledger.finish(request, OpenDirectOutcome::RejectedNative).unwrap();
    }
    assert_eq!(ledger.query(open(1)).unwrap().state, OpenDirectState::Unknown);
    assert_eq!(ledger.reserve(open(1)), Err(ChatError::StaleRequest));
    let mut foreign = open(2);
    foreign.selection.source = source(2);
    assert_eq!(ledger.query(foreign), Err(ChatError::Binding));
    assert_eq!(ledger.reserve(foreign), Err(ChatError::Binding));
}

#[test]
fn contacts_open_positive_ack_cannot_replace_the_original_resident_or_conversation_kind() {
    let request = open(1);
    let mut ledger = OpenDirectLedger::new(source(1)).unwrap();
    ledger.reserve(request).unwrap();
    for conversation in [
        Conversation::Group(peer(9)),
        Conversation::Conference(peer(9)),
        Conversation::Direct(peer(8)),
    ] {
        let outcome = OpenDirectOutcome::Opened {
            resident: peer(9),
            key: ChatKey {
                source: source(1),
                conversation,
            },
        };
        assert_eq!(ledger.finish(request, outcome), Err(ChatError::Binding));
    }
    assert_eq!(ledger.query(request).unwrap().state, OpenDirectState::Pending);
    ledger.finish(request, opened(request)).unwrap();
}

#[test]
fn contacts_open_receipt_has_no_retained_private_title_and_fresh_view_requires_read_permission() {
    let request = open(1);
    let mut ledger = OpenDirectLedger::new(source(1)).unwrap();
    ledger.reserve(request).unwrap();
    let receipt = ledger.finish(request, opened(request)).unwrap();
    let mut view = OpenDirectView {
        policy: NativeChatPolicy {
            key: ChatKey {
                source: source(1),
                conversation: Conversation::Direct(peer(9)),
            },
            revision: 1,
            lease_current: true,
            ready: true,
            can_send: true,
            can_read: true,
        },
        title: Label::new("Private title").unwrap(),
        unread: 7,
    };
    assert!(view.validate(&receipt).is_ok());
    assert!(!serde_json::to_string(&receipt).unwrap().contains("Private title"));
    view.policy.can_read = false;
    assert_eq!(view.validate(&receipt), Err(ChatError::Denied));
    view.policy.can_read = true;
    view.policy.key.source = source(2);
    assert_eq!(view.validate(&receipt), Err(ChatError::Binding));
}

#[test]
fn contacts_wire_fits_actual_escaped_page_bytes_without_losing_cursor_or_membership() {
    use crate::chat::{ChatEvent, ChatMessage, ChatPacket, MAX_PACKET_BYTES, VERSION, decode, encode, fit_contact_page};
    let request = ContactPageRequest {
        key: open(1).selection,
        offset: 0,
    };
    let rows: Vec<_> = (1..=32)
        .map(|id| ContactRow {
            resident: peer(id),
            display: ContactText::new(&"\\".repeat(255)).unwrap(),
            username: ContactText::new(&"\"".repeat(127)).unwrap(),
            online: id % 2 == 0,
            can_open: true,
        })
        .collect();
    let mut directory = directory();
    assert_eq!(directory.begin(ContactChannel::Friends, 1).unwrap(), request);
    let mut offset = 0;
    let mut packets = 0;
    while offset < rows.len() {
        let packet = ChatPacket {
            version: VERSION,
            source: source(1),
            sequence: u64::MAX,
            reply_to: u64::MAX,
            message: ChatMessage::Event(ChatEvent::ContactPage(ContactPage {
                key: request.key,
                policy_revision: 1,
                offset: offset as u16,
                total: rows.len() as u16,
                limited: false,
                pending: false,
                rows: rows[offset..].to_vec(),
            })),
        };
        if offset == 0 {
            assert_eq!(encode(&packet), Err(ChatError::Oversized));
        }
        let fitted = fit_contact_page(packet).unwrap();
        let bytes = encode(&fitted).unwrap();
        assert!(bytes.len() <= MAX_PACKET_BYTES);
        let decoded = decode(&bytes).unwrap();
        let ChatMessage::Event(ChatEvent::ContactPage(page)) = decoded.message else {
            panic!()
        };
        assert_eq!(page.offset as usize, offset);
        assert_eq!(page.total as usize, rows.len());
        assert!(!page.rows.is_empty());
        offset += page.rows.len();
        assert_eq!(directory.accept(&page).unwrap(), offset == rows.len());
        packets += 1;
    }
    assert!(packets > 1);
    let complete = directory.completed(ContactChannel::Friends).unwrap();
    assert_eq!(complete.rows.len(), rows.len());
    for row in rows {
        assert!(complete.rows.contains(&row));
    }
}

#[test]
fn contacts_wire_rejects_cross_source_rows_and_unbound_or_forged_open_presentation() {
    use crate::chat::{ChatCommand, ChatEvent, ChatMessage, ChatPacket, VERSION, decode, encode};
    let request = open(1);
    let receipt = OpenDirectReceipt {
        request,
        state: OpenDirectState::Terminal(opened(request)),
    };
    for message in [
        ChatMessage::Command(ChatCommand::OpenDirect(request)),
        ChatMessage::Command(ChatCommand::QueryOpenDirect(request)),
        ChatMessage::Event(ChatEvent::OpenDirect(OpenDirectReply { receipt, view: None })),
    ] {
        let packet = ChatPacket {
            version: VERSION,
            source: source(1),
            sequence: 1,
            reply_to: if matches!(message, ChatMessage::Event(_)) { 1 } else { 0 },
            message,
        };
        assert_eq!(decode(&encode(&packet).unwrap()).unwrap(), packet);
        assert_eq!(
            encode(&ChatPacket {
                source: source(2),
                ..packet.clone()
            }),
            Err(ChatError::Binding)
        );
        assert_eq!(
            encode(&ChatPacket {
                version: VERSION - 1,
                ..packet.clone()
            }),
            Err(ChatError::Version)
        );
        if matches!(packet.message, ChatMessage::Event(_)) {
            assert_eq!(encode(&ChatPacket { reply_to: 0, ..packet }), Err(ChatError::Sequence));
        }
    }
}
