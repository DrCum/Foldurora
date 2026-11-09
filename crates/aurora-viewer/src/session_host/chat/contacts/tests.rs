use super::*;
use aurora_session::chat::{ContactText, OpenDirectReceipt};
use aurora_session::*;
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
fn peer() -> ConversationId {
    ConversationId::from_bytes([9; 16]).unwrap()
}
fn selection() -> ContactKey {
    ContactKey {
        source: source(1),
        channel: ContactChannel::Friends,
        token: 1,
    }
}
fn operation() -> OpenDirectRequest {
    OpenDirectRequest {
        selection: selection(),
        resident: peer(),
        request: 1,
        policy_revision: 1,
    }
}
fn packet(sequence: u64, command: ChatCommand) -> ChatPacket {
    ChatPacket {
        version: VERSION,
        source: source(1),
        sequence,
        reply_to: 0,
        message: ChatMessage::Command(command),
    }
}
struct Native {
    generation: u64,
    stamp: u64,
    readable: bool,
    opened: bool,
    opens: usize,
    allow: bool,
    display: String,
    searches: usize,
    search_key: Option<ContactKey>,
    search_ready: bool,
}
impl Default for Native {
    fn default() -> Self {
        Self {
            generation: 1,
            stamp: 1,
            readable: true,
            opened: false,
            opens: 0,
            allow: true,
            display: "Synthetic friend".into(),
            searches: 0,
            search_key: None,
            search_ready: false,
        }
    }
}
impl NativeChat for Native {
    fn source(&self) -> Option<ChatSource> {
        Some(source(self.generation))
    }
    fn catalogue(&self) -> Vec<Conversation> {
        if self.opened { vec![Conversation::Direct(peer())] } else { vec![] }
    }
    fn snapshot(&self, _: Conversation) -> Option<NativeSnapshot> {
        self.opened.then(|| NativeSnapshot {
            title: Label::new(&self.display).unwrap(),
            stamp: [self.stamp; 6],
            ready: true,
            can_send: true,
            can_read: self.readable,
            initial: NativeCursor::default(),
        })
    }
    fn lines(&self, _: Conversation, cursor: NativeCursor, _: usize) -> NativePage {
        NativePage {
            cursor,
            gap: false,
            lines: vec![],
        }
    }
    fn submit_literal(&mut self, _: ChatKey, _: &str) -> SendOutcome {
        panic!("Open must never send")
    }
    fn contacts_policy(&self) -> Option<NativeContactPolicy> {
        Some(NativeContactPolicy {
            stamp: [self.stamp; 4],
            can_browse: self.readable,
        })
    }
    fn friends(&self) -> Result<NativeContacts, ChatError> {
        Ok(NativeContacts {
            limited: false,
            rows: vec![ContactRow {
                resident: peer(),
                display: ContactText::new(&self.display).unwrap(),
                username: ContactText::new("synthetic.friend").unwrap(),
                online: false,
                can_open: true,
            }],
        })
    }
    fn start_search(&mut self, key: ContactKey, _query: &str) -> Result<(), ChatError> {
        self.searches += 1;
        self.search_key = Some(key);
        Ok(())
    }
    fn search(&self, key: ContactKey) -> Result<Option<NativeContacts>, ChatError> {
        if self.search_key != Some(key) {
            return Err(ChatError::Binding);
        }
        if !self.search_ready {
            return Ok(None);
        }
        self.friends().map(Some)
    }
    fn open_direct(&mut self, request: OpenDirectRequest) -> OpenDirectOutcome {
        self.opens += 1;
        if !self.allow {
            return OpenDirectOutcome::RejectedNative;
        }
        self.opened = true;
        OpenDirectOutcome::Opened {
            resident: request.resident,
            key: ChatKey {
                source: request.selection.source,
                conversation: Conversation::Direct(request.resident),
            },
        }
    }
}
fn run(worker: &mut WorkerChat, native: &mut Native, sequence: u64, command: ChatCommand) -> Vec<ChatPacket> {
    let now = Instant::now();
    worker.handle_received(native, &packet(sequence, command), now, now).unwrap()
}
fn prepare(worker: &mut WorkerChat, native: &mut Native) {
    let result = run(
        worker,
        native,
        1,
        ChatCommand::ContactPage(ContactPageRequest {
            key: selection(),
            offset: 0,
        }),
    );
    assert!(result.iter().any(|packet| matches!(&packet.message, ChatMessage::Event(ChatEvent::ContactPage(page)) if !page.rows[0].online && page.rows[0].can_open)));
}
fn receipt(packets: &[ChatPacket]) -> &OpenDirectReply {
    packets
        .iter()
        .find_map(|packet| {
            if let ChatMessage::Event(ChatEvent::OpenDirect(reply)) = &packet.message {
                Some(reply)
            } else {
                None
            }
        })
        .unwrap()
}
#[test]
fn contacts_worker_open_is_once_only_query_is_body_free_and_privacy_revocation_removes_view() {
    let (mut worker, mut native) = (WorkerChat::default(), Native::default());
    prepare(&mut worker, &mut native);
    let opened = run(&mut worker, &mut native, 2, ChatCommand::OpenDirect(operation()));
    assert!(receipt(&opened).view.is_some());
    assert_eq!(native.opens, 1);
    native.display = "Fresh title".into();
    let duplicate = run(&mut worker, &mut native, 3, ChatCommand::OpenDirect(operation()));
    assert_eq!(receipt(&opened).receipt, receipt(&duplicate).receipt);
    assert_eq!(receipt(&duplicate).view.as_ref().unwrap().title.as_str(), "Fresh title");
    native.readable = false;
    native.stamp += 1;
    let recovered = run(&mut worker, &mut native, 4, ChatCommand::QueryOpenDirect(operation()));
    assert_eq!(receipt(&opened).receipt, receipt(&recovered).receipt);
    assert!(receipt(&recovered).view.is_none());
    assert_eq!(native.opens, 1);
    assert!(
        !aurora_session::chat::encode(recovered.last().unwrap())
            .unwrap()
            .windows(5)
            .any(|bytes| bytes == b"Fresh")
    );
}
#[test]
fn contacts_worker_rejects_changed_policy_missing_selection_expired_age_and_native_refusal() {
    for case in 0..4 {
        let (mut worker, mut native) = (WorkerChat::default(), Native::default());
        prepare(&mut worker, &mut native);
        let mut request = operation();
        match case {
            0 => native.stamp += 1,
            1 => request.selection.token += 1,
            2 => {}
            _ => native.allow = false,
        }
        let packets = if case == 2 {
            worker.handle(&mut native, &packet(2, ChatCommand::OpenDirect(request))).unwrap()
        } else {
            run(&mut worker, &mut native, 2, ChatCommand::OpenDirect(request))
        };
        assert!(matches!(
            receipt(&packets).receipt.state,
            OpenDirectState::Terminal(OpenDirectOutcome::StaleTarget | OpenDirectOutcome::Expired | OpenDirectOutcome::RejectedNative)
        ));
        assert!(!native.opened);
        assert_eq!(native.opens, usize::from(case == 3));
    }
}
#[test]
fn contacts_worker_pages_keep_snapshot_but_policy_and_source_changes_revoke_old_selection() {
    let (mut worker, mut native) = (WorkerChat::default(), Native::default());
    prepare(&mut worker, &mut native);
    native.display = "Replacement title".into();
    let same = run(
        &mut worker,
        &mut native,
        2,
        ChatCommand::ContactPage(ContactPageRequest {
            key: selection(),
            offset: 0,
        }),
    );
    assert!(same.iter().any(|packet| matches!(&packet.message, ChatMessage::Event(ChatEvent::ContactPage(page)) if page.rows[0].display.as_str() == "Synthetic friend")));
    native.generation = 2;
    assert_eq!(
        worker.handle(&mut native, &packet(3, ChatCommand::OpenDirect(operation()))),
        Err(ChatError::Binding)
    );
    assert_eq!(native.opens, 0);
    let unknown = OpenDirectReceipt {
        request: OpenDirectRequest {
            selection: ContactKey {
                source: source(2),
                ..selection()
            },
            ..operation()
        },
        state: OpenDirectState::Unknown,
    };
    // Rejected old-source packets cannot initialize the new contact backend.
    assert!(worker.contacts.opens.is_none());
    let now = Instant::now();
    let query = ChatPacket {
        source: source(2),
        ..packet(4, ChatCommand::QueryOpenDirect(unknown.request))
    };
    let recovered = worker.handle_received(&mut native, &query, now, now).unwrap();
    assert_eq!(receipt(&recovered).receipt, unknown);
    assert!(receipt(&recovered).view.is_none());
    assert_eq!(native.opens, 0);
}

#[test]
fn contact_search_worker_pending_poll_and_duplicate_begin_never_repeat_http_and_old_policy_cannot_open() {
    use aurora_session::chat::ContactSearchRequest;
    let (mut worker, mut native) = (WorkerChat::default(), Native::default());
    prepare(&mut worker, &mut native);
    let key = ContactKey {
        channel: ContactChannel::Search,
        token: 2,
        ..selection()
    };
    let request = ContactSearchRequest {
        key,
        policy_revision: 1,
        query: ContactText::new("Synthetic Name").unwrap(),
    };
    for sequence in 2..=3 {
        let reply = run(&mut worker, &mut native, sequence, ChatCommand::SearchContacts(request.clone()));
        assert!(
            reply
                .iter()
                .any(|packet| matches!(&packet.message,ChatMessage::Event(ChatEvent::ContactPage(page)) if page.pending && page.key==key))
        );
    }
    assert_eq!(native.searches, 1);
    let changed = ContactSearchRequest {
        query: ContactText::new("Different query").unwrap(),
        ..request
    };
    assert_eq!(
        worker.handle(&mut native, &packet(4, ChatCommand::SearchContacts(changed))),
        Err(ChatError::RequestChanged)
    );
    native.search_ready = true;
    let rows = run(
        &mut worker,
        &mut native,
        5,
        ChatCommand::ContactPage(ContactPageRequest { key, offset: 0 }),
    );
    assert!(
        rows.iter().any(
            |packet| matches!(&packet.message,ChatMessage::Event(ChatEvent::ContactPage(page)) if !page.pending && page.rows.len()==1)
        )
    );
    let open = OpenDirectRequest {
        selection: key,
        ..operation()
    };
    native.stamp += 1;
    let result = run(&mut worker, &mut native, 6, ChatCommand::OpenDirect(open));
    assert_eq!(
        receipt(&result).receipt.state,
        OpenDirectState::Terminal(OpenDirectOutcome::StaleTarget)
    );
    assert_eq!(native.opens, 0);
}
