use super::*;
use crate::session_host::chat_broker::HostChatSnapshot;
use crate::session_host::slots::{SlotLabel, SlotSummary};
use crate::session_host::transport::WorkerSummary;
use aurora_session::chat::{ChatSource, ConversationInfo, ObservedIncoming};
use aurora_session::{
    AccountKey, ConversationId, GridKey, HostId, InstallationId, Phase, SessionId, SessionIdentity, WorkerId, WorkerIdentity, WorkerStatus,
};
use std::sync::Arc;

fn source(number: u8) -> ChatSource {
    ChatSource {
        worker: WorkerIdentity {
            installation: InstallationId::from_bytes([1; 32]).unwrap(),
            host: HostId::from_bytes([2; 16]).unwrap(),
            worker: WorkerId::from_bytes([number; 16]).unwrap(),
        },
        session: SessionIdentity {
            session: SessionId::from_bytes([number; 16]).unwrap(),
            account: AccountKey::from_bytes([number; 32]).unwrap(),
            grid: GridKey::from_bytes([6; 32]).unwrap(),
            generation: 1,
        },
        lease_epoch: 7,
    }
}

fn direct(number: u8) -> Conversation {
    Conversation::Direct(ConversationId::from_bytes([number; 16]).unwrap())
}

fn unread() -> ReadMetadata {
    ReadMetadata {
        unread: 2,
        unread_revision: 1,
        observed: ObservedIncoming { epoch: 1, through: 2 },
        can_mark_read: true,
    }
}

fn fixture(workers: usize, conversations: usize) -> HostSnapshot {
    let mut host = HostSnapshot::default();
    for (index, slot) in SlotId::all().take(workers).enumerate() {
        let source = source(index as u8 + 1);
        host.slots.push(SlotSummary {
            id: slot,
            label: SlotLabel::new("Synthetic profile").unwrap(),
            available: false,
        });
        host.workers.push(WorkerSummary {
            slot: Some(slot),
            binding: source.binding(),
            status: Some(WorkerStatus {
                phase: Phase::Online,
                name: Label::new("Synthetic account").unwrap(),
                grid_label: Label::new("Synthetic grid").unwrap(),
                ..Default::default()
            }),
            reachable: true,
            label: "Fallback profile is not the account".into(),
            grid: "Fallback profile is not the grid".into(),
        });
        let conversations: Vec<_> = (1..=conversations)
            .map(|number| ConversationInfo {
                title: Label::new("Synthetic conversation").unwrap(),
                policy: NativeChatPolicy {
                    key: ChatKey {
                        source,
                        conversation: direct(number as u8),
                    },
                    revision: number as u64,
                    lease_current: true,
                    ready: true,
                    can_read: true,
                    can_send: true,
                },
            })
            .collect();
        let reads = conversations.iter().map(|info| (info.policy.key.conversation, unread())).collect();
        host.chat.push(Arc::new(HostChatSnapshot {
            source,
            authenticated: true,
            live: true,
            conversations,
            lines: Vec::new(),
            receipts: Vec::new(),
            reads,
            read_operations: Vec::new(),
            gap: false,
        }));
    }
    host.active = host.workers.first().map(|worker| worker.binding.worker.worker);
    host
}

fn key(worker: u8, conversation: u8) -> ChatKey {
    ChatKey {
        source: source(worker),
        conversation: direct(conversation),
    }
}

fn chat(host: &mut HostSnapshot, index: usize) -> &mut HostChatSnapshot {
    Arc::make_mut(&mut host.chat[index])
}

fn read(host: &mut HostSnapshot, index: usize, conversation: u8) -> &mut ReadMetadata {
    chat(host, index).reads.get_mut(&direct(conversation)).unwrap()
}

fn dismiss(model: &mut Attention, key: ChatKey) {
    let ticket = model.capture(key).expect("visible exact row");
    assert!(model.dismiss(ticket));
}

fn navigate(model: &mut Attention, direction: Direction) -> Option<ChatKey> {
    let ticket = model.capture_navigation(direction)?;
    model.open_chat(ticket)
}

#[test]
fn attention_active_and_standby_readable_rows_ignore_send_and_mark_capacity() {
    let mut host = fixture(5, 1);
    let mut model = Attention::default();
    chat(&mut host, 1).conversations[0].policy.can_send = false;
    read(&mut host, 1, 1).can_mark_read = false;
    // Zero is a valid native incoming cursor even with positive unread.
    read(&mut host, 1, 1).observed.through = 0;
    model.sync(&host, &Preferences::default());
    assert_eq!(model.rows().len(), 5);
    assert!(
        model
            .rows()
            .iter()
            .any(|row| row.key == key(2, 1) && !row.read.can_mark_read && !row.policy.can_send)
    );
    assert!(model.selected().is_none());
    host.active = Some(source(4).worker.worker);
    model.sync(&host, &Preferences::default());
    assert_eq!(model.rows().len(), 5);
    assert!(model.selected().is_none());
}

#[test]
fn attention_nearby_off_direct_on_and_unsupported_classes_never_adapt() {
    let mut host = fixture(1, 1);
    let peer = chat(&mut host, 0);
    for conversation in [
        Conversation::Nearby,
        Conversation::Group(ConversationId::from_bytes([9; 16]).unwrap()),
    ] {
        let mut info = peer.conversations[0].clone();
        info.policy.key.conversation = conversation;
        peer.conversations.push(info);
        peer.reads.insert(conversation, unread());
    }
    let mut prefs = Preferences::default();
    let mut model = Attention::default();
    model.sync(&host, &prefs);
    assert_eq!(model.rows().len(), 1);
    assert_eq!(model.rows()[0].key, key(1, 1));
    prefs.nearby = true;
    model.sync(&host, &prefs);
    assert_eq!(model.rows().len(), 2);
    prefs.direct = false;
    model.sync(&host, &prefs);
    assert_eq!(model.rows().len(), 1);
    assert_eq!(model.rows()[0].key.conversation, Conversation::Nearby);
}

#[test]
fn attention_stable_slot_filters_survive_rename_roster_reorder_and_relog() {
    let mut host = fixture(3, 1);
    let mut prefs = Preferences::default();
    prefs.include_slots[1] = false;
    let mut model = Attention::default();
    model.sync(&host, &prefs);
    assert_eq!(model.rows().iter().map(|row| row.key).collect::<Vec<_>>(), [key(1, 1), key(3, 1)]);
    host.slots[1].label = SlotLabel::new("Renamed synthetic profile").unwrap();
    host.workers.reverse();
    host.slots.reverse();
    host.chat.reverse();
    // Relog the filtered stable slot with a fresh exact session binding.
    let peer = chat(&mut host, 1);
    peer.source.session.generation += 1;
    peer.source.session.session = SessionId::from_bytes([12; 16]).unwrap();
    peer.conversations[0].policy.key.source = peer.source;
    let relogged = peer.source;
    host.workers[1].binding = relogged.binding();
    model.sync(&host, &prefs);
    assert_eq!(model.rows().iter().map(|row| row.key).collect::<Vec<_>>(), [key(1, 1), key(3, 1)]);
    prefs.include_slots[1] = true;
    model.sync(&host, &prefs);
    assert_eq!(model.rows().len(), 3);
    assert!(
        model
            .rows()
            .iter()
            .any(|row| row.key.source == relogged && row.profile == "Renamed synthetic profile")
    );
}

#[test]
fn attention_unslotted_workers_cannot_borrow_saved_slot_preferences() {
    let mut host = fixture(2, 1);
    host.workers[0].slot = None;
    let mut model = Attention::default();
    model.sync(&host, &Preferences::default());
    assert_eq!(model.rows().len(), 1);
    assert_eq!(model.rows()[0].key, key(2, 1));
}

#[test]
fn attention_labels_never_authorize_or_redirect_exact_chat_actions() {
    let mut host = fixture(2, 1);
    let mut model = Attention::default();
    model.sync(&host, &Preferences::default());
    let ticket = model.capture(key(1, 1)).unwrap();
    host.workers[0].status.as_mut().unwrap().name = Label::new("Renamed account label").unwrap();
    host.workers[0].label = "Different fallback".into();
    host.slots[0].label = SlotLabel::new("Same synthetic profile").unwrap();
    chat(&mut host, 0).conversations[0].title = Label::new("Renamed conversation").unwrap();
    model.sync(&host, &Preferences::default());
    assert_eq!(model.open_chat(ticket), Some(key(1, 1)));
    assert_eq!(model.rows()[0].account.as_str(), "Renamed account label");
    assert_eq!(model.rows()[0].grid.as_str(), "Synthetic grid");
    // Same labels do not substitute a different bound worker/account/grid.
    host.workers.remove(0);
    model.sync(&host, &Preferences::default());
    assert_eq!(model.selected(), Some(key(1, 1)));
    assert!(model.capture(key(1, 1)).is_none());
    assert_eq!(model.open_chat(ticket), None);
}

#[test]
fn attention_closed_model_purges_rows_books_and_tickets_on_authority_loss() {
    for loss in 0..12 {
        let mut host = fixture(2, 2);
        let mut model = Attention::default();
        let prefs = Preferences::default();
        model.sync(&host, &prefs);
        let ticket = model.capture(key(1, 2)).unwrap();
        dismiss(&mut model, key(1, 1));
        let original = host.clone();
        match loss {
            0 => chat(&mut host, 0).authenticated = false,
            1 => chat(&mut host, 0).live = false,
            2 => host.workers[0].reachable = false,
            3 => chat(&mut host, 0).source.lease_epoch += 1,
            4 => host.workers[0].binding.session.as_mut().unwrap().account = AccountKey::from_bytes([13; 32]).unwrap(),
            5 => host.workers[0].binding.session.as_mut().unwrap().grid = GridKey::from_bytes([14; 32]).unwrap(),
            6 => host.workers[0].binding.session.as_mut().unwrap().generation += 1,
            7 => chat(&mut host, 0)
                .conversations
                .iter_mut()
                .for_each(|info| info.policy.can_read = false),
            8 => chat(&mut host, 0)
                .conversations
                .iter_mut()
                .for_each(|info| info.policy.ready = false),
            9 => chat(&mut host, 0)
                .conversations
                .iter_mut()
                .for_each(|info| info.policy.lease_current = false),
            10 => chat(&mut host, 0).conversations.clear(),
            11 => chat(&mut host, 0).reads.clear(),
            _ => unreachable!(),
        }
        // No show/window call is involved in any revocation.
        model.sync(&host, &prefs);
        assert!(model.rows().iter().all(|row| row.key.source != source(1)), "loss {loss}");
        assert!(model.dismissed.is_empty(), "loss {loss}");
        assert!(!model.dismiss(ticket), "loss {loss}");
        model.sync(&original, &prefs);
        assert!(model.capture(key(1, 1)).is_some(), "dismissal must not revive after loss {loss}");
        assert!(
            model.open_chat(ticket).is_none(),
            "old incarnation must not revive after loss {loss}"
        );
    }
}

#[test]
fn attention_invalid_policy_and_read_observations_fail_closed() {
    for invalid in 0..5 {
        let mut host = fixture(1, 1);
        match invalid {
            0 => chat(&mut host, 0).conversations[0].policy.revision = 0,
            1 => read(&mut host, 0, 1).unread_revision = 0,
            2 => read(&mut host, 0, 1).observed.epoch = 0,
            3 => chat(&mut host, 0).source.lease_epoch = 0,
            4 => read(&mut host, 0, 1).unread = 0,
            _ => unreachable!(),
        }
        let mut model = Attention::default();
        model.sync(&host, &Preferences::default());
        assert!(model.rows().is_empty(), "invalid {invalid}");
    }
}

#[test]
fn attention_dismissal_is_local_and_same_count_new_arrival_returns() {
    let mut host = fixture(1, 1);
    let original_read = host.chat[0].reads.clone();
    let mut model = Attention::default();
    model.sync(&host, &Preferences::default());
    dismiss(&mut model, key(1, 1));
    assert_eq!(host.chat[0].reads, original_read);
    assert!(host.chat[0].read_operations.is_empty());
    model.sync(&host, &Preferences::default());
    assert!(model.rows().is_empty());
    // A read/arrival replacement can keep the native badge unchanged.
    read(&mut host, 0, 1).observed.through += 1;
    read(&mut host, 0, 1).unread_revision += 2;
    model.sync(&host, &Preferences::default());
    assert_eq!(model.rows().len(), 1);
    assert_eq!(model.dismissed.len(), 1);
}

#[test]
fn attention_dismissal_matches_epoch_cursor_and_count_not_unread_revision() {
    for change in 0..6 {
        let mut host = fixture(1, 1);
        let mut model = Attention::default();
        model.sync(&host, &Preferences::default());
        dismiss(&mut model, key(1, 1));
        let read = read(&mut host, 0, 1);
        match change {
            0 => read.observed.epoch += 1,
            1 => read.observed.through += 1,
            2 => read.unread += 1,
            3 => read.unread -= 1,
            4 => read.observed.through -= 1,
            5 => read.unread_revision += 1,
            _ => unreachable!(),
        }
        model.sync(&host, &Preferences::default());
        assert_eq!(model.rows().is_empty(), change >= 4, "change {change}");
    }
}

#[test]
fn attention_dismissal_count_roundtrip_keeps_original_cursor_comparator() {
    let mut host = fixture(1, 1);
    read(&mut host, 0, 1).unread = 5;
    let mut model = Attention::default();
    let prefs = Preferences::default();
    model.sync(&host, &prefs);
    dismiss(&mut model, key(1, 1));
    read(&mut host, 0, 1).unread = 4;
    model.sync(&host, &prefs);
    assert_eq!(model.rows().len(), 1);
    assert_eq!(model.dismissed.len(), 1);
    read(&mut host, 0, 1).unread = 5;
    model.sync(&host, &prefs);
    assert!(model.rows().is_empty());
    read(&mut host, 0, 1).observed.through += 1;
    model.sync(&host, &prefs);
    assert_eq!(model.rows().len(), 1);
}

#[test]
fn attention_policy_replacement_invalidates_dismissal_and_gesture() {
    let mut host = fixture(1, 2);
    let mut model = Attention::default();
    model.sync(&host, &Preferences::default());
    dismiss(&mut model, key(1, 1));
    let ticket = model.capture(key(1, 2)).unwrap();
    for info in &mut chat(&mut host, 0).conversations {
        info.policy.revision += 10;
    }
    model.sync(&host, &Preferences::default());
    assert_eq!(model.rows().len(), 2);
    assert!(model.dismissed.is_empty());
    assert!(!model.dismiss(ticket));
    assert!(model.open_chat(ticket).is_none());
}

#[test]
fn attention_filters_preserve_dismissal_but_reset_visible_row_incarnation() {
    let host = fixture(2, 2);
    let mut prefs = Preferences::default();
    let mut model = Attention::default();
    model.sync(&host, &prefs);
    dismiss(&mut model, key(1, 1));
    let before = model.rows().iter().find(|row| row.key == key(1, 2)).unwrap().first_observed;
    let ticket = model.capture(key(1, 2)).unwrap();
    prefs.include_slots[0] = false;
    model.sync(&host, &prefs);
    assert_eq!(model.dismissed.len(), 1);
    prefs.include_slots[0] = true;
    model.sync(&host, &prefs);
    assert!(model.capture(key(1, 1)).is_none());
    let restored = model.rows().iter().find(|row| row.key == key(1, 2)).unwrap();
    assert!(restored.first_observed > before);
    assert!(!model.dismiss(ticket));
}

#[test]
fn attention_local_observation_order_and_ids_survive_unchanged_metadata_and_arrival() {
    let mut host = fixture(2, 1);
    let mut prefs = Preferences::default();
    let mut model = Attention::default();
    // B is observed before A even though its stable slot sorts after A.
    chat(&mut host, 0).reads.clear();
    model.sync(&host, &prefs);
    chat(&mut host, 0).reads.insert(direct(1), unread());
    model.sync(&host, &prefs);
    assert_eq!(model.rows().iter().map(|row| row.key).collect::<Vec<_>>(), [key(2, 1), key(1, 1)]);
    let ids: Vec<_> = model.rows().iter().map(Row::id).collect();
    let earlier = model.capture(key(2, 1)).unwrap();
    let later = model.capture(key(1, 1)).unwrap();
    model.sync(&host, &prefs);
    assert_eq!(model.rows().iter().map(Row::id).collect::<Vec<_>>(), ids);
    assert!(model.validate(earlier));
    assert!(model.validate(later));

    // Another incoming arrival updates the earlier row's gesture observation,
    // while preserving its first-unread position and widget identity.
    read(&mut host, 1, 1).unread += 1;
    read(&mut host, 1, 1).observed.through += 1;
    model.sync(&host, &prefs);
    assert_eq!(model.rows().iter().map(Row::id).collect::<Vec<_>>(), ids);
    assert!(!model.validate(earlier));
    assert!(model.validate(later));
    prefs.oldest_first = true;
    host.chat.reverse();
    model.sync(&host, &prefs);
    // No offers exist: oldest_first must never mean descending/newest-first.
    assert_eq!(model.rows().iter().map(|row| row.key).collect::<Vec<_>>(), [key(2, 1), key(1, 1)]);
    assert_eq!(model.rows().iter().map(Row::id).collect::<Vec<_>>(), ids);
}

#[test]
fn attention_selection_survives_reorder_without_unavailable_fallback() {
    let mut host = fixture(2, 2);
    let prefs = Preferences::default();
    let mut model = Attention::default();
    model.sync(&host, &prefs);
    assert!(model.select(key(2, 2)));
    host.chat.reverse();
    host.workers.reverse();
    for index in 0..2 {
        chat(&mut host, index).conversations.reverse();
    }
    model.sync(&host, &prefs);
    assert_eq!(model.selected(), Some(key(2, 2)));
    host.chat.retain(|peer| peer.source != source(2));
    model.sync(&host, &prefs);
    assert_eq!(model.selected(), Some(key(2, 2)));
    assert!(model.capture(key(2, 2)).is_none());
    assert_eq!(navigate(&mut model, Direction::Next), Some(key(1, 1)));
    assert_eq!(navigate(&mut model, Direction::Previous), Some(key(1, 2)));
    assert_eq!(navigate(&mut model, Direction::Next), Some(key(1, 1)));
}

#[test]
fn attention_explicit_navigation_wraps_and_never_mutates_native_state() {
    let host = fixture(2, 1);
    let before = host.chat.clone();
    let mut model = Attention::default();
    model.sync(&host, &Preferences::default());
    assert_eq!(navigate(&mut model, Direction::Previous), Some(key(2, 1)));
    assert_eq!(navigate(&mut model, Direction::Next), Some(key(1, 1)));
    let ticket = model.capture(key(1, 1)).unwrap();
    assert_eq!(model.open_chat(ticket), Some(key(1, 1)));
    assert!(host.chat == before);
    assert_eq!(host.active, Some(source(1).worker.worker));
}

#[test]
fn attention_stale_gestures_cannot_dismiss_newer_or_retarget_selection() {
    let mut host = fixture(2, 1);
    let prefs = Preferences::default();
    let mut model = Attention::default();
    model.sync(&host, &prefs);
    let stale = model.capture(key(1, 1)).unwrap();
    read(&mut host, 0, 1).observed.through += 1;
    model.sync(&host, &prefs);
    assert!(!model.dismiss(stale));
    assert!(model.open_chat(stale).is_none());
    assert_eq!(model.rows().len(), 2);
    let prior_selection = model.capture(key(1, 1)).unwrap();
    model.select(key(2, 1));
    assert!(!model.dismiss(prior_selection));
    assert!(model.open_chat(prior_selection).is_none());
    let current = model.capture(key(1, 1)).unwrap();
    assert_eq!(model.open_chat(current), Some(key(1, 1)));
    assert_eq!(model.selected(), Some(key(1, 1)));
}

#[test]
fn attention_navigation_press_ticket_never_retargets_after_row_replacement() {
    let mut host = fixture(2, 2);
    let prefs = Preferences::default();
    let mut model = Attention::default();
    model.sync(&host, &prefs);
    model.select(key(1, 1));
    let pressed = model.capture_navigation(Direction::Next).unwrap();
    assert_eq!(pressed.key(), key(1, 2));
    assert_eq!(model.selected(), Some(key(1, 1)));
    chat(&mut host, 0).reads.remove(&direct(2));
    model.sync(&host, &prefs);
    // Next now points at a different sender; releasing the old gesture may
    // neither choose it nor use its valid but unrelated current policy.
    assert_eq!(model.capture_navigation(Direction::Next).unwrap().key(), key(2, 1));
    assert!(model.open_chat(pressed).is_none());
    assert_eq!(model.selected(), Some(key(1, 1)));
}

#[test]
fn attention_row_widget_ids_survive_metadata_updates_but_never_reuse() {
    let mut host = fixture(1, 1);
    let mut prefs = Preferences::default();
    let mut model = Attention::default();
    model.sync(&host, &prefs);
    let id = model.rows()[0].id();
    read(&mut host, 0, 1).unread += 1;
    model.sync(&host, &prefs);
    assert_eq!(model.rows()[0].id(), id);
    prefs.direct = false;
    model.sync(&host, &prefs);
    prefs.direct = true;
    model.sync(&host, &prefs);
    let restored_id = model.rows()[0].id();
    assert!(restored_id > id);
    model.clear();
    model.sync(&host, &prefs);
    assert!(model.rows()[0].id() > restored_id);
}

#[test]
fn attention_reminder_book_is_fifo_bounded_to_128_and_contains_no_bodies() {
    let host = fixture(3, 64);
    let mut model = Attention::default();
    let prefs = Preferences::default();
    model.sync(&host, &prefs);
    let keys: Vec<_> = model.rows().iter().take(MAX_DISMISSALS + 1).map(|row| row.key).collect();
    for key in &keys {
        dismiss(&mut model, *key);
    }
    assert_eq!(model.dismissed.len(), MAX_DISMISSALS);
    assert_eq!(model.dismissed.front().unwrap().key, keys[1]);
    model.sync(&host, &prefs);
    assert!(model.capture(keys[0]).is_some());
    assert!(model.capture(keys[1]).is_none());
    assert_eq!(model.rows().len(), 64);
}

#[test]
fn attention_redismiss_updates_cursor_without_renewing_fifo_age() {
    let mut host = fixture(3, 64);
    let mut model = Attention::default();
    let prefs = Preferences::default();
    model.sync(&host, &prefs);
    let keys: Vec<_> = model.rows().iter().take(MAX_DISMISSALS + 1).map(|row| row.key).collect();
    for key in keys.iter().take(MAX_DISMISSALS) {
        dismiss(&mut model, *key);
    }
    read(&mut host, 0, 1).observed.through += 1;
    model.sync(&host, &prefs);
    dismiss(&mut model, keys[0]);
    assert_eq!(model.dismissed.front().unwrap().key, keys[0]);
    assert_eq!(model.dismissed.front().unwrap().read.observed.through, 3);
    dismiss(&mut model, keys[MAX_DISMISSALS]);
    assert_eq!(model.dismissed.front().unwrap().key, keys[1]);
    model.sync(&host, &prefs);
    assert!(model.capture(keys[0]).is_some());
}

#[test]
fn attention_rows_are_capped_per_source_and_full_host() {
    let mut host = fixture(5, 65);
    // Duplicate publications must not amplify capacity or alias identities.
    host.chat.push(host.chat[0].clone());
    let mut model = Attention::default();
    model.sync(&host, &Preferences::default());
    assert_eq!(model.rows().len(), MAX_ROWS);
    for number in 1..=5 {
        assert_eq!(
            model.rows().iter().filter(|row| row.key.source == source(number)).count(),
            MAX_CONVERSATIONS
        );
        assert!(model.capture(key(number, 65)).is_none());
    }
}

#[test]
fn attention_stop_and_explicit_clear_purge_without_reusing_old_tickets() {
    let mut host = fixture(1, 2);
    let mut model = Attention::default();
    model.sync(&host, &Preferences::default());
    let ticket = model.capture(key(1, 2)).unwrap();
    dismiss(&mut model, key(1, 1));
    host.stopped = true;
    model.sync(&host, &Preferences::default());
    assert!(model.rows().is_empty());
    assert!(model.dismissed.is_empty());
    host.stopped = false;
    model.sync(&host, &Preferences::default());
    assert!(model.open_chat(ticket).is_none());
    let ticket = model.capture(key(1, 2)).unwrap();
    model.clear();
    model.sync(&host, &Preferences::default());
    assert!(model.open_chat(ticket).is_none());
    assert!(model.selected().is_none());
}
