use super::*;
use crate::session_host::chat_broker::OpenOperation;
use crate::session_host::chat_broker::OpenStatus;
use aurora_session::chat::{
    CompletedContacts, ContactChannel, ContactKey, ContactRow, ContactText, ContactsPolicy, OpenDirectOutcome, OpenDirectRequest,
};
fn contacts_snapshot() -> HostChatSnapshot {
    let mut snapshot = snapshot(source());
    let Conversation::Direct(resident) = direct() else { panic!() };
    snapshot.contacts.policy = Some(ContactsPolicy {
        source: source(),
        revision: 1,
        can_browse: true,
    });
    snapshot.contacts.friends = Some(CompletedContacts {
        key: ContactKey {
            source: source(),
            channel: ContactChannel::Friends,
            token: 1,
        },
        policy_revision: 1,
        limited: false,
        rows: vec![ContactRow {
            resident,
            display: ContactText::new("Synthetic offline friend").unwrap(),
            username: ContactText::new("synthetic.friend").unwrap(),
            online: false,
            can_open: true,
        }],
    });
    snapshot
}
fn draft(ui: &ChatUi, snapshot: &HostChatSnapshot) -> OpenDraft {
    let friends = snapshot.contacts.friends.as_ref().unwrap();
    OpenDraft {
        request: OpenDirectRequest {
            selection: friends.key,
            resident: friends.rows[0].resident,
            policy_revision: 1,
            request: 0,
        },
        instance: ui.instance,
        surface: ui.surface,
        generation: ui.view.contacts.generation,
    }
}
fn complete(snapshot: &mut HostChatSnapshot, mut draft: OpenDraft, ticket: u64) {
    draft.request.request = ticket;
    snapshot.contacts.opens.push(OpenOperation {
        request: draft.request,
        status: OpenStatus::Completed(OpenDirectOutcome::Opened {
            resident: draft.request.resident,
            key: ChatKey {
                source: source(),
                conversation: direct(),
            },
        }),
    });
}
#[test]
fn contacts_ui_opens_only_after_positive_ack_and_preserves_draft_and_read_state() {
    let mut snapshot = contacts_snapshot();
    let mut frames = Frames::new();
    frames.show(Some(&snapshot), Vec::new());
    frames.type_text(&snapshot, "Retained nearby draft");
    let draft = draft(&frames.ui, &snapshot);
    frames.ui.queued_open(draft, 50);
    frames.ui.sync(Some(&snapshot));
    assert_eq!(frames.ui.key().conversation, Conversation::Nearby);
    complete(&mut snapshot, draft, 50);
    frames.ui.sync(Some(&snapshot));
    assert_eq!(frames.ui.key().conversation, direct());
    frames.ui.select(Conversation::Nearby);
    frames.ui.sync(Some(&snapshot));
    assert_eq!(frames.ui.draft(), "Retained nearby draft");
    assert!(frames.ui.take_read_action().is_none() && frames.ui.take_close_action().is_none());
}
#[test]
fn contacts_ui_navigation_closed_surface_and_policy_aba_prevent_late_selection() {
    for change in 0..3 {
        let mut snapshot = contacts_snapshot();
        let mut ui = ChatUi::new(source());
        ui.sync(Some(&snapshot));
        let draft = draft(&ui, &snapshot);
        ui.queued_open(draft, 50);
        match change {
            0 => {
                ui.select(direct());
                ui.select(Conversation::Nearby);
            }
            1 => ui.close_surface(ChatSurface::Shared),
            _ => {
                snapshot.contacts.policy.as_mut().unwrap().revision = 2;
                ui.sync(Some(&snapshot));
                snapshot.contacts.policy.as_mut().unwrap().revision = 3;
            }
        }
        complete(&mut snapshot, draft, 50);
        ui.sync(Some(&snapshot));
        assert_eq!(ui.key().conversation, Conversation::Nearby);
        assert!(ui.view.contacts.pending.is_none());
        ui.queued_open(draft, 51);
        assert!(
            ui.view.contacts.pending.is_none(),
            "late queue callback cannot reclaim a retired generation"
        );
    }
}
#[test]
fn contacts_ui_actual_friend_row_selects_locally_then_explicit_open_captures_offline_target() {
    let snapshot = contacts_snapshot();
    let mut frames = Frames::new();
    assert!(frames.click(&snapshot, Id::unique("host_contacts_friends_rect")).is_none());
    let resident = snapshot.contacts.friends.as_ref().unwrap().rows[0].resident;
    assert!(frames.click(&snapshot, Id::unique(("host_contact_row_rect", resident))).is_none());
    assert_eq!(frames.ui.key().conversation, Conversation::Nearby);
    assert!(frames.ui.take_contact_action().is_none(), "local row selection never opens an IM");
    assert!(frames.click(&snapshot, Id::unique("host_contacts_open_rect")).is_none());
    let Some(ContactAction::Open(open)) = frames.ui.take_contact_action() else {
        panic!("explicit Open button must emit captured operation")
    };
    assert_eq!(open.request.selection, snapshot.contacts.friends.as_ref().unwrap().key);
    assert_eq!(open.request.resident, resident);
    assert_eq!(open.request.selection.source, source());
    assert_eq!(open.request.request, 0);
    assert!(frames.ui.take_read_action().is_none() && frames.ui.take_close_action().is_none());
}
