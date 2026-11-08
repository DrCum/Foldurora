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
    let snapshot = group_snapshot();
    // Read metadata alone supplies no native incoming stream for Close.
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
    assert!(
        frames
            .context
            .data(|data| data.get_temp::<Rect>(Id::unique("host_chat_close_rect")))
            .is_none()
    );
    assert!(frames.ui.take_close_action().is_none());
    assert_eq!(frames.ui.view.conversation, group());
}

#[test]
fn group_close_real_ui_preserves_unicode_undo_and_hides_all_selected_surfaces() {
    use crate::session_host::chat_broker::{CloseOperation, CloseStatus};
    let mut snapshot = group_snapshot();
    snapshot.closes.targets.insert(group(), 22);
    let mut frames = Frames::new();
    frames.navigate(&snapshot, group());
    frames.type_text(&snapshot, "Group draft · こんにちは");
    for _ in 0..65 {
        frames.show(Some(&snapshot), Vec::new());
    }
    let first = frames.ui.draft().to_owned();
    frames.type_text(&snapshot, " second edit");
    for _ in 0..65 {
        frames.show(Some(&snapshot), Vec::new());
    }
    let before = frames.ui.draft().to_owned();
    frames.click(&snapshot, Id::unique("host_chat_close_rect"));
    let close = frames.ui.take_close_action().unwrap();
    assert_eq!(close.capture.key.conversation, group());
    for surface in ChatSurface::ALL {
        frames.ui.with_surface(surface, |ui| {
            ui.select(group());
            ui.sync(Some(&snapshot));
        });
    }
    frames.ui.queued_close(close, 40);
    for surface in ChatSurface::ALL {
        frames.ui.with_surface(surface, |ui| {
            assert_eq!(ui.view.conversation, Conversation::Nearby);
            assert!(ui.take_close_action().is_none() && ui.take_read_action().is_none());
        });
    }
    assert_eq!(frames.ui.drafts[&group()].text, before);
    frames.show(Some(&snapshot), Vec::new());
    assert!(frames.ui.hidden.contains(&group()), "old snapshot cannot undo local admission");
    snapshot.closes.hidden.insert(group());
    snapshot.closes.operations.push(CloseOperation {
        request: close.request(40),
        status: CloseStatus::Completed(aurora_session::chat::CloseOutcome::Closed(aurora_session::chat::NativeIncoming {
            stream: 22,
            serial: 2,
        })),
    });
    frames.ui.sync(Some(&snapshot));
    assert_eq!(frames.ui.drafts[&group()].text, before);
    snapshot.closes.hidden.clear(); // Broker proved newer native Group incoming.
    frames.navigate(&snapshot, group());
    assert_eq!(frames.ui.draft(), before);
    for _ in 0..3 {
        frames.undo(&snapshot);
        if frames.ui.draft() != before {
            break;
        }
    }
    assert_eq!(frames.ui.draft(), first);
    assert!(frames.ui.receipt.is_none() && frames.ui.take_read_action().is_none());
}
