use super::*;
use aurora_session::chat::{CloseOutcome, CloseReceiptState};
struct CloseNative {
    native: Native,
    captures: usize,
}
impl NativeChat for CloseNative {
    fn source(&self) -> Option<ChatSource> {
        self.native.source()
    }
    fn catalogue(&self) -> Vec<Conversation> {
        self.native.catalogue()
    }
    fn snapshot(&self, conversation: Conversation) -> Option<NativeSnapshot> {
        self.native.snapshot(conversation)
    }
    fn lines(&self, conversation: Conversation, cursor: NativeCursor, limit: usize) -> NativePage {
        self.native.lines(conversation, cursor, limit)
    }
    fn submit_literal(&mut self, key: ChatKey, text: &str) -> SendOutcome {
        self.native.submit_literal(key, text)
    }
    fn close_conversation(&mut self, _request: CloseRequest) -> CloseOutcome {
        self.captures += 1;
        CloseOutcome::Closed(NativeIncoming { stream: 1, serial: 0 })
    }
}
fn setup() -> (CloseNative, WorkerChat, CloseRequest) {
    let mut native = Native::new();
    native.direct = true;
    let native = CloseNative { native, captures: 0 };
    let mut worker = WorkerChat::default();
    worker.sync(&native).unwrap();
    let conversation = Conversation::Direct(ConversationId::from_bytes([7; 16]).unwrap());
    let request = CloseRequest {
        key: ChatKey {
            source: source(1),
            conversation,
        },
        request: 1,
        policy_revision: worker.states[&conversation].policy.revision,
        stream: 1,
    };
    (native, worker, request)
}
fn packet(request: CloseRequest, sequence: u64) -> ChatPacket {
    ChatPacket {
        version: VERSION,
        source: request.key.source,
        sequence,
        reply_to: 0,
        message: ChatMessage::Command(ChatCommand::CloseConversation(request)),
    }
}
#[test]
fn close_worker_reserves_reply_sequence_and_fails_lost_original_source_before_capture() {
    let (mut native, mut worker, request) = setup();
    worker.outgoing = u64::MAX - MAX_REPLY as u64 + 1;
    let now = Instant::now();
    assert_eq!(
        worker.handle_received(&mut native, &packet(request, 1), now, now),
        Err(ChatError::Sequence)
    );
    assert_eq!(native.captures, 0);
    assert_eq!(
        worker.closes.as_ref().unwrap().query(request).unwrap().result,
        CloseReceiptState::Unknown
    );
    let (mut native, mut worker, request) = setup();
    // context + require_read_authority calls happen before native capture.
    native.native.change = Some((native.native.calls.get() + 2, None));
    assert_eq!(
        worker.handle_received(&mut native, &packet(request, 1), now, now),
        Err(ChatError::Unavailable)
    );
    assert_eq!(native.captures, 0);
    assert_eq!(worker.current_source(), None);
    native.native.change = None;
    assert_eq!(worker.sync(&native), Err(ChatError::Binding));
    assert_eq!(native.captures, 0);
}
