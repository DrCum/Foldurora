use super::*;

fn conference() -> Conversation {
    Conversation::Conference(ConversationId::from_bytes([11; 16]).unwrap())
}
fn conference_snapshot() -> HostChatSnapshot {
    let mut snapshot = snapshot(source());
    snapshot.conversations[1].policy.key.conversation = conference();
    snapshot.reads.insert(
        conference(),
        ReadMetadata {
            unread: 2,
            unread_revision: 3,
            observed: aurora_session::chat::ObservedIncoming { epoch: 2, through: 2 },
            can_mark_read: true,
        },
    );
    snapshot.closes.targets.insert(conference(), 22);
    snapshot
}

#[test]
fn shared_conference_real_composer_and_explicit_read_keep_one_unicode_draft_across_all_six_surfaces() {
    let snapshot = conference_snapshot();
    let mut frames = Frames::new();
    frames.navigate(&snapshot, conference());
    frames.type_text(&snapshot, "Shared conference · こんにちは 🌕");
    for surface in ChatSurface::ALL {
        frames.ui.with_surface(surface, |ui| {
            ui.select(conference());
        });
        let (key, text, _, _) = frames.ui.retained_for(surface);
        assert_eq!(
            key,
            ChatKey {
                source: source(),
                conversation: conference()
            }
        );
        assert_eq!(text, "Shared conference · こんにちは 🌕");
    }
    assert!(frames.ui.take_read_action().is_none(), "navigation never marks read");
    frames.click(&snapshot, Id::unique("host_chat_read_rect"));
    let read = frames.ui.take_read_action().unwrap();
    assert_eq!(read.key.conversation, conference());
    assert_eq!(read.metadata.observed.epoch, 2);
    assert_eq!(frames.ui.draft(), "Shared conference · こんにちは 🌕");
    let mut sender = Frames::new();
    sender.navigate(&snapshot, conference());
    sender.type_text(&snapshot, "Shared conference · こんにちは 🌕");
    let send = sender.click(&snapshot, Id::unique("host_chat_send_rect")).unwrap();
    assert_eq!(send.key.conversation, conference());
    assert_eq!(send.text.as_str(), "Shared conference · こんにちは 🌕");
}

#[test]
fn shared_conference_real_close_captures_native_stream_without_read_or_send_and_retains_draft() {
    let snapshot = conference_snapshot();
    let mut frames = Frames::new();
    frames.navigate(&snapshot, conference());
    frames.type_text(&snapshot, "Retained conference draft 🌕");
    assert!(frames.click(&snapshot, Id::unique("host_chat_close_rect")).is_none());
    let close = frames.ui.take_close_action().unwrap();
    assert_eq!(close.capture.key.conversation, conference());
    assert_eq!(close.capture.stream, 22);
    assert!(frames.ui.take_read_action().is_none());
    assert_eq!(frames.ui.draft(), "Retained conference draft 🌕");
    assert!(frames.ui.take_close_action().is_none());
}
