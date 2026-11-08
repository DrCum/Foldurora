use super::*;
use crate::session_host::chat_broker::{CloseOperation, CloseStatus};
use aurora_session::chat::{CloseOutcome, NativeIncoming};

fn close_snapshot() -> HostChatSnapshot {
    let mut snapshot = snapshot(source());
    snapshot.closes.targets.insert(direct(), 22);
    snapshot
}

#[test]
fn host_chat_real_close_retains_unicode_draft_and_undo_through_hide_and_reopen() {
    let mut frames = Frames::new();
    let mut snapshot = close_snapshot();
    frames.navigate(&snapshot, direct());
    frames.type_text(&snapshot, "Retained draft · こんにちは");
    let first = frames.ui.draft().to_owned();
    // egui commits distinct Undo checkpoints after a stable one-second pause.
    for _ in 0..65 {
        frames.show(Some(&snapshot), Vec::new());
    }
    frames.type_text(&snapshot, " second edit");
    for _ in 0..65 {
        frames.show(Some(&snapshot), Vec::new());
    }
    let before = frames.ui.draft().to_owned();
    assert_ne!(before, first);
    assert!(frames.click(&snapshot, Id::unique("host_chat_close_rect")).is_none());
    let close = frames.ui.take_close_action().expect("explicit Close button");
    assert_eq!(close.capture.key.conversation, direct());
    assert_eq!(close.capture.policy_revision, 2);
    assert_eq!(close.capture.stream, 22);
    assert!(frames.ui.take_read_action().is_none());
    frames.ui.queued_close(close, 40);
    assert_eq!(frames.ui.view.conversation, Conversation::Nearby);
    assert_eq!(frames.ui.drafts[&direct()].text, before);
    frames.show(Some(&snapshot), Vec::new());
    assert!(
        frames.ui.hidden.contains(&direct()),
        "old snapshot cannot undo local FIFO admission"
    );
    assert!(!frames.ui.select_existing(&snapshot, direct()));
    snapshot.closes.hidden.insert(direct());
    snapshot.closes.operations.push(CloseOperation {
        request: close.request(40),
        status: CloseStatus::Pending,
    });
    frames.ui.sync(Some(&snapshot));
    assert!(frames.ui.pending_close.is_none());
    assert_eq!(frames.ui.drafts[&direct()].text, before);
    snapshot.closes.operations[0].status = CloseStatus::Completed(CloseOutcome::Closed(NativeIncoming { stream: 22, serial: 2 }));
    snapshot.closes.hidden.clear(); // broker proved strictly newer native incoming
    frames.navigate(&snapshot, direct());
    assert_eq!(frames.ui.draft(), before);
    for _ in 0..3 {
        frames.undo(&snapshot);
        if frames.ui.draft() != before {
            break;
        }
    }
    assert_eq!(
        frames.ui.draft(),
        first,
        "the pre-close Undo checkpoint survived in the source-owned editor"
    );
    assert!(frames.ui.receipt.is_none(), "Close never sends a message");
}

#[test]
fn host_chat_close_retires_all_selected_surface_gestures_without_clearing_other_drafts() {
    let mut frames = Frames::new();
    let snapshot = close_snapshot();
    frames.type_text(&snapshot, "Nearby remains intact");
    frames.navigate(&snapshot, direct());
    frames.type_text(&snapshot, "Shared monitor draft");
    let close = {
        frames.click(&snapshot, Id::unique("host_chat_close_rect"));
        frames.ui.take_close_action().unwrap()
    };
    for surface in ChatSurface::ALL {
        frames.ui.with_surface(surface, |model| {
            model.select(direct());
            model.sync(Some(&snapshot));
        });
    }
    frames.ui.queued_close(close, 41);
    for surface in ChatSurface::ALL {
        frames.ui.with_surface(surface, |model| {
            assert_eq!(model.view.conversation, Conversation::Nearby);
            assert!(model.take_close_action().is_none() && model.take_read_action().is_none());
            assert_eq!(model.draft(), "Nearby remains intact");
        });
    }
    assert_eq!(frames.ui.drafts[&direct()].text, "Shared monitor draft");
    // Security retirement still purges every parked draft, including hidden.
    let mut restricted = snapshot.clone();
    restricted.conversations[1].policy.can_read = false;
    restricted.conversations[1].policy.can_send = false;
    restricted.conversations[1].policy.revision += 1;
    frames.ui.sync(Some(&restricted));
    assert!(!frames.ui.drafts.contains_key(&direct()));
    assert_eq!(frames.ui.drafts[&Conversation::Nearby].text, "Nearby remains intact");
}

#[test]
fn host_chat_close_held_click_cannot_target_replaced_stream_policy_or_nearby() {
    for change in 0..3 {
        let mut frames = Frames::new();
        let mut snapshot = close_snapshot();
        frames.navigate(&snapshot, direct());
        let at = frames.rect(&snapshot, Id::unique("host_chat_close_rect")).center();
        frames.show(
            Some(&snapshot),
            vec![
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        match change {
            0 => {
                snapshot.closes.targets.insert(direct(), 23);
            }
            1 => {
                snapshot.conversations[1].policy.revision += 1;
            }
            _ => {
                frames.ui.select(Conversation::Nearby);
            }
        }
        frames.show(
            Some(&snapshot),
            vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(frames.ui.take_close_action().is_none());
        assert!(frames.ui.receipt.is_none());
    }
}

#[test]
fn host_chat_close_queue_failure_keeps_selection_text_and_newer_source_ignores_old_intent() {
    let mut frames = Frames::new();
    let snapshot = close_snapshot();
    frames.navigate(&snapshot, direct());
    frames.type_text(&snapshot, "Keep on queue failure");
    frames.click(&snapshot, Id::unique("host_chat_close_rect"));
    let close = frames.ui.take_close_action().unwrap();
    frames.ui.close_queue_failed(close, HostError::Busy);
    assert_eq!(frames.ui.view.conversation, direct());
    assert_eq!(frames.ui.draft(), "Keep on queue failure");
    assert!(frames.ui.hidden.is_empty());
    let mut newer = ChatUi::for_instance(source(), frames.ui.instance + 1);
    newer.sync(Some(&snapshot));
    newer.queued_close(close, 42);
    assert!(newer.hidden.is_empty() && newer.pending_close.is_none());
}
