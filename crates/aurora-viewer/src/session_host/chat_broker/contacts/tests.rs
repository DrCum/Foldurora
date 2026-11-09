use super::*;
use crate::session_host::chat_broker::tests::{connected, source};
use aurora_session::chat::{ContactRow, ContactText, NativeChatPolicy, OpenDirectReceipt, OpenDirectView};
use aurora_session::{ConversationId, Label};
fn resident() -> ConversationId {
    ConversationId::from_bytes([77; 16]).unwrap()
}
fn key(token: u64) -> ContactKey {
    ContactKey {
        source: source(),
        channel: ContactChannel::Friends,
        token,
    }
}
fn policy(revision: u64, can_browse: bool) -> ContactsPolicy {
    ContactsPolicy {
        source: source(),
        revision,
        can_browse,
    }
}
fn page(token: u64) -> ContactPage {
    ContactPage {
        key: key(token),
        policy_revision: 1,
        offset: 0,
        total: 1,
        limited: false,
        pending: false,
        rows: vec![ContactRow {
            resident: resident(),
            display: ContactText::new("Synthetic Resident").unwrap(),
            username: ContactText::new("synthetic").unwrap(),
            online: false,
            can_open: true,
        }],
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
fn setup() -> (Peer, crate::session_host::chat_transport::WorkerIo) {
    let (mut peer, worker) = connected();
    peer.observe(&event(&peer, 1, ChatEvent::ContactsPolicy(policy(1, true)))).unwrap();
    peer.refresh_contacts(source(), Instant::now()).unwrap();
    let reply = ChatPacket {
        reply_to: peer.contacts.flight.as_ref().unwrap().sequence,
        ..event(&peer, 2, ChatEvent::ContactPage(page(1)))
    };
    peer.observe(&reply).unwrap();
    (peer, worker)
}
fn request(ticket: u64) -> OpenDirectRequest {
    OpenDirectRequest {
        selection: key(1),
        resident: resident(),
        policy_revision: 1,
        request: ticket,
    }
}
fn opened(request: OpenDirectRequest, view: bool) -> OpenDirectReply {
    let key = ChatKey {
        source: source(),
        conversation: Conversation::Direct(resident()),
    };
    OpenDirectReply {
        receipt: OpenDirectReceipt {
            request,
            state: OpenDirectState::Terminal(OpenDirectOutcome::Opened { resident: resident(), key }),
        },
        view: view.then(|| OpenDirectView {
            policy: NativeChatPolicy {
                key,
                revision: 1,
                lease_current: true,
                ready: true,
                can_read: true,
                can_send: true,
            },
            title: Label::new("Fresh native title").unwrap(),
            unread: 8,
        }),
    }
}
#[test]
fn contacts_host_stages_exact_reply_and_revokes_selection_across_policy_aba() {
    let (mut peer, _worker) = setup();
    assert!(peer.contacts.snapshot(true).selected(key(1), resident(), 1));
    peer.refresh_contacts(source(), Instant::now()).unwrap();
    assert!(
        peer.contacts.snapshot(true).selected(key(1), resident(), 1),
        "completed list remains during refresh"
    );
    let wrong = event(&peer, 3, ChatEvent::ContactPage(page(1)));
    assert_eq!(peer.observe(&wrong), Err(ChatError::StaleRequest));
    peer.observe(&event(&peer, 4, ChatEvent::ContactsPolicy(policy(2, false)))).unwrap();
    assert!(peer.contacts.snapshot(true).friends.is_none());
    peer.observe(&event(&peer, 5, ChatEvent::ContactsPolicy(policy(3, true)))).unwrap();
    assert!(!peer.contacts.snapshot(true).selected(key(1), resident(), 1));
    assert!(peer.contacts.directory.as_ref().unwrap().request(ContactChannel::Friends).is_none());
}
#[test]
fn contacts_host_receipt_alone_cannot_publish_title_and_recovery_never_resends_open() {
    let (mut peer, _worker) = setup();
    let now = Instant::now();
    let request = request(40);
    peer.admit_open_direct(request, now).unwrap();
    let sequence = peer.sent;
    peer.poll_open_direct(now + Duration::from_secs(2));
    assert_eq!(peer.sent, sequence + 1);
    peer.observe(&event(&peer, 3, ChatEvent::OpenDirect(opened(request, false))))
        .unwrap();
    assert!(!peer.conversations.contains_key(&Conversation::Direct(resident())));
    assert!(peer.contacts.pending.is_none());
    assert!(matches!(
        peer.contacts.snapshot(true).opens[0].status,
        OpenStatus::Completed(OpenDirectOutcome::Opened { .. })
    ));
    let before = peer.sent;
    peer.poll_open_direct(now + Duration::from_secs(9));
    assert_eq!(peer.sent, before);
    peer.observe(&event(&peer, 4, ChatEvent::OpenDirect(opened(request, true))))
        .unwrap();
    assert_eq!(
        peer.conversations[&Conversation::Direct(resident())].title.as_str(),
        "Fresh native title"
    );
    assert!(peer.reads.is_empty(), "Open never fabricates read metadata or clears unread");
}
