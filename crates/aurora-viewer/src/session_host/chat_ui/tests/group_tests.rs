use super::*;

fn group_snapshot() -> HostChatSnapshot {
    let mut snapshot = snapshot(source());
    snapshot.conversations[1].policy.key.conversation = group();
    snapshot.reads.insert(
        group(),
        ReadMetadata {
            unread: 2,
            unread_revision: 3,
            observed: aurora_session::chat::ObservedIncoming { epoch: 2, through: 2 },
            can_mark_read: true,
        },
    );
    snapshot
}
fn group() -> Conversation {
    Conversation::Group(ConversationId::from_bytes([11; 16]).unwrap())
}

#[test]
fn shared_group_real_composer_keeps_one_unicode_draft_across_all_six_surfaces() {
    let snapshot = group_snapshot();
    let mut frames = Frames::new();
    frames.navigate(&snapshot, group());
    frames.type_text(&snapshot, "Shared 🌕 group draft");
    for surface in ChatSurface::ALL {
        frames.ui.with_surface(surface, |ui| {
            ui.select(group());
        });
        let (key, text, _, _) = frames.ui.retained_for(surface);
        assert_eq!(
            key,
            ChatKey {
                source: source(),
                conversation: group()
            }
        );
        assert_eq!(text, "Shared 🌕 group draft", "{surface:?}");
    }
    let send = frames.click(&snapshot, Id::unique("host_chat_send_rect")).unwrap();
    assert_eq!(send.key.conversation, group());
    assert_eq!(send.text.as_str(), "Shared 🌕 group draft");
}

#[test]
fn shared_group_real_mark_read_captures_exact_source_without_close_authority() {
    let mut snapshot = group_snapshot();
    // Even a supplied group fence must not enable the Direct-only Close path.
    snapshot.closes.targets.insert(group(), 9);
    let mut frames = Frames::new();
    frames.navigate(&snapshot, group());
    frames.type_text(&snapshot, "Retain while reading");
    assert!(frames.click(&snapshot, Id::unique("host_chat_read_rect")).is_none());
    let read = frames.ui.take_read_action().unwrap();
    assert_eq!(
        read.key,
        ChatKey {
            source: source(),
            conversation: group()
        }
    );
    assert_eq!(read.metadata, snapshot.reads[&group()]);
    assert_eq!(frames.ui.draft(), "Retain while reading");
    assert!(frames.click(&snapshot, Id::unique("host_chat_close_rect")).is_none());
    assert!(frames.ui.take_close_action().is_none());
    assert_eq!(frames.ui.view.conversation, group());
}
