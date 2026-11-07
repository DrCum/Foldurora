use super::super::model::{Change, Reference, Tile};
use super::*;
use crate::gestures::Entry;
use crate::world::inventory::{
    FetchState, Folder, Inventory,
    actions::{PlaybackError, resolve_playback_item},
};
use aurora_assets::gesture::{Step, WAIT_TIME};
use aurora_net::inventory::{InvFolder, InvItem};
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn b(n: u128) -> BoardId {
    BoardId::try_from(id(n)).unwrap()
}
fn t(n: u128) -> TileId {
    TileId::try_from(id(n)).unwrap()
}
fn facts() -> Facts {
    Facts {
        agent: id(1),
        login: id(2),
        grid: GridKey::from_bytes([1; 32]).unwrap(),
        logical_generation: 1,
        profile_generation: 1,
        managed: None,
    }
}
struct Fixture {
    playback: Playback,
    data: Catalogue,
    inv: Inventory,
    manager: Manager,
    now: Instant,
}
impl Fixture {
    fn new(count: usize) -> Self {
        let f = facts();
        let mut playback = Playback::default();
        playback.bind(Some(f)).unwrap();
        playback.window(true).unwrap();
        let mut data = Catalogue::empty(f.agent, f.grid)
            .unwrap()
            .apply(Change::Create {
                id: b(1),
                name: "Board".into(),
            })
            .unwrap();
        let mut inv = Inventory {
            owner: f.agent,
            root: id(90),
            ..Default::default()
        };
        inv.folders.insert(
            inv.root,
            Folder {
                info: InvFolder {
                    id: inv.root,
                    type_default: 8,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        for n in 1..=count {
            let item = id(500 + n as u128);
            inv.items.insert(
                item,
                InvItem {
                    id: item,
                    parent: inv.root,
                    owner: f.agent,
                    asset_type: 21,
                    asset_id: id(1500 + n as u128),
                    ..Default::default()
                },
            );
            data = data
                .apply(Change::Add {
                    board: b(1),
                    tile: Tile {
                        id: t(n as u128),
                        item: Reference {
                            reference: item,
                            kind: Kind::Gesture,
                        },
                    },
                })
                .unwrap();
        }
        Self {
            playback,
            data,
            inv,
            manager: Manager::default(),
            now: Instant::now(),
        }
    }
    fn target(&self, n: u128) -> PlaybackItem {
        resolve_playback_item(&self.inv, id(500 + n), PlaybackKind::Gesture).unwrap()
    }
    fn ticket(&self, n: u128) -> Ticket {
        self.playback
            .capture(&self.data, b(1), t(n), self.target(n), &self.manager, self.now)
            .unwrap()
    }
    fn cache(&mut self, n: u128) {
        let target = self.target(n);
        self.manager.entries.insert(
            target.item(),
            Entry {
                asset: target.asset(),
                definition: Gesture {
                    steps: vec![
                        Step::Animation {
                            asset: id(2500 + n),
                            name: String::new(),
                            flags: 0,
                        },
                        Step::Wait {
                            seconds: 10.0,
                            flags: WAIT_TIME,
                        },
                    ],
                    ..Default::default()
                },
            },
        );
    }
    fn start(&mut self, n: u128) {
        self.cache(n);
        let target = self.target(n);
        let ticket = self.ticket(n);
        let Click::Ready(ready) = self
            .playback
            .click(ticket, &self.data, target, &mut self.manager, self.now)
            .unwrap()
        else {
            panic!("cached explicit click")
        };
        assert!(!ready.definition().steps.is_empty());
        self.playback.start(ready, &self.data, target, &mut self.manager, self.now).unwrap();
    }
}
#[test]
fn board_pending_add_decode_open_capture_and_cache_request_never_start_or_activate_a_native_run() {
    let mut f = Fixture::new(1);
    let target = f.target(1);
    let ticket = f.ticket(1);
    assert!(f.manager.active.is_empty());
    assert!(!f.manager.has_playing());
    assert_eq!(
        Catalogue::decode(&f.data.encode().unwrap(), f.data.account, f.data.grid).unwrap(),
        f.data
    );
    let Click::NeedsCache(captured) = f.playback.click(ticket, &f.data, target, &mut f.manager, f.now).unwrap() else {
        panic!("cache only")
    };
    assert!(captured == ticket);
    assert_eq!(captured.board(), b(1));
    assert_eq!(captured.item(), target.item());
    assert!(f.manager.active.is_empty());
    assert!(!f.manager.has_playing());
    let repeated = f
        .playback
        .capture(&f.data, b(1), t(1), target, &f.manager, f.now + Duration::from_secs(5))
        .unwrap();
    assert!(matches!(
        f.playback
            .click(repeated, &f.data, target, &mut f.manager, f.now + Duration::from_secs(5)),
        Ok(Click::AlreadyLoading)
    ));
    assert!(
        f.playback.pending(target.item()) == Some(ticket),
        "reclick cannot replace/renew the original pending intent"
    );
}
#[test]
fn board_pending_stop_before_load_keeps_late_cache_as_data_and_never_replays_the_cancelled_click() {
    let mut f = Fixture::new(1);
    let target = f.target(1);
    let ticket = f.ticket(1);
    f.playback.click(ticket, &f.data, target, &mut f.manager, f.now).unwrap();
    let stop = f.playback.capture_board(&f.data, b(1), f.now).unwrap();
    assert!(f.playback.stop_board(stop, &f.data, &mut f.manager, f.now).unwrap().is_empty());
    f.cache(1);
    assert!(f.playback.prepare_pending(ticket, &f.data, target, &f.manager, f.now).is_err());
    assert!(f.manager.entries.contains_key(&target.item()));
    assert!(!f.manager.has_playing());
    assert!(f.manager.active.is_empty());
    let fresh = f.ticket(1);
    let Click::Ready(ready) = f.playback.click(fresh, &f.data, target, &mut f.manager, f.now).unwrap() else {
        panic!("fresh explicit click")
    };
    f.playback.start(ready, &f.data, target, &mut f.manager, f.now).unwrap();
    assert!(f.manager.is_playing(target.item()));
}
#[test]
fn board_pending_original_deadline_bounded_queue_and_exact_cancel_do_not_refresh_or_retarget() {
    let mut f = Fixture::new(MAX_PENDING + 1);
    for n in 1..=MAX_PENDING as u128 {
        let target = f.target(n);
        let ticket = f.ticket(n);
        assert!(matches!(
            f.playback.click(ticket, &f.data, target, &mut f.manager, f.now),
            Ok(Click::NeedsCache(_))
        ));
    }
    let target = f.target(33);
    let ticket = f.ticket(33);
    assert!(matches!(
        f.playback.click(ticket, &f.data, target, &mut f.manager, f.now),
        Err(Error::Busy)
    ));
    let old = f.playback.pending(f.target(1).item()).unwrap();
    f.playback.expire(f.now + Duration::from_secs(29));
    assert!(f.playback.pending(old.item()).is_some());
    f.playback.expire(f.now + DEADLINE);
    assert!(f.playback.pending(old.item()).is_none());
    let fresh = f
        .playback
        .capture(&f.data, b(1), t(1), f.target(1), &f.manager, f.now + DEADLINE)
        .unwrap();
    f.playback
        .click(fresh, &f.data, f.target(1), &mut f.manager, f.now + DEADLINE)
        .unwrap();
    assert!(!f.playback.cancel_pending(old));
    assert!(f.playback.pending(fresh.item()) == Some(fresh));
    assert!(f.playback.cancel_pending(fresh));
    assert!(!f.manager.has_playing());
}
#[test]
fn board_pending_loaded_target_change_window_reopen_and_source_a_b_a_never_revive_old_intent() {
    for loss in 0..5 {
        let mut f = Fixture::new(1);
        let original = f.target(1);
        let ticket = f.ticket(1);
        f.playback.click(ticket, &f.data, original, &mut f.manager, f.now).unwrap();
        f.cache(1);
        match loss {
            0 => {
                f.inv.items.get_mut(&original.item()).unwrap().asset_id = id(9999);
            }
            1 => {
                f.playback.window(false).unwrap();
                f.playback.window(true).unwrap();
            }
            2 => {
                let mut other = facts();
                other.agent = id(99);
                f.playback.bind(Some(other)).unwrap();
                f.playback.bind(Some(facts())).unwrap();
            }
            3 => {
                f.playback.invalidate_actions().unwrap();
            }
            _ => {
                let mut next = facts();
                next.profile_generation += 1;
                f.playback.bind(Some(next)).unwrap();
            }
        }
        let current = f.target(1);
        assert!(f.playback.prepare_pending(ticket, &f.data, current, &f.manager, f.now).is_err());
        assert!(!f.manager.has_playing());
    }
}
#[test]
fn board_pending_external_native_start_is_never_claimed_and_old_stop_preserves_hotkey_replacement() {
    let mut f = Fixture::new(1);
    let target = f.target(1);
    let ticket = f.ticket(1);
    f.playback.click(ticket, &f.data, target, &mut f.manager, f.now).unwrap();
    f.cache(1);
    let definition = f.manager.entries[&target.item()].definition.clone();
    f.manager.play(target.item(), definition.clone(), Some(b'Q'));
    let native = f.manager.playback_id(target.item()).unwrap();
    assert!(f.playback.prepare_pending(ticket, &f.data, target, &f.manager, f.now).is_err());
    let stop = f.playback.capture_board(&f.data, b(1), f.now).unwrap();
    assert!(f.playback.stop_board(stop, &f.data, &mut f.manager, f.now).unwrap().is_empty());
    assert_eq!(f.manager.playback_id(target.item()), Some(native));
    f.manager.stop(target.item());
    f.start(1);
    let owned = f.manager.playback_id(target.item()).unwrap();
    let stop = f.playback.capture_board(&f.data, b(1), f.now).unwrap();
    f.manager.play(target.item(), definition, Some(b'Q'));
    let replacement = f.manager.playback_id(target.item()).unwrap();
    assert_ne!(owned, replacement);
    assert!(f.playback.stop_board(stop, &f.data, &mut f.manager, f.now).unwrap().is_empty());
    assert_eq!(f.manager.playback_id(target.item()), Some(replacement));
}
#[test]
fn board_stop_without_any_tile_still_stops_only_its_own_exact_runs_and_leaves_other_board() {
    let mut f = Fixture::new(2);
    f.start(1);
    let first = f.target(1);
    let second = f.target(2);
    f.data = f
        .data
        .apply(Change::Create {
            id: b(2),
            name: "Other".into(),
        })
        .unwrap()
        .apply(Change::Add {
            board: b(2),
            tile: Tile {
                id: t(9),
                item: Reference {
                    reference: second.reference(),
                    kind: Kind::Gesture,
                },
            },
        })
        .unwrap();
    f.cache(2);
    let ticket = f.playback.capture(&f.data, b(2), t(9), second, &f.manager, f.now).unwrap();
    let Click::Ready(ready) = f.playback.click(ticket, &f.data, second, &mut f.manager, f.now).unwrap() else {
        panic!("other board")
    };
    f.playback.start(ready, &f.data, second, &mut f.manager, f.now).unwrap();
    let other_id = f.manager.playback_id(second.item()).unwrap();
    f.data = f
        .data
        .apply(Change::Remove { board: b(1), tile: t(1) })
        .unwrap()
        .apply(Change::Remove { board: b(1), tile: t(2) })
        .unwrap();
    assert!(f.data.board(b(1)).unwrap().tiles.is_empty());
    let stop = f.playback.capture_board(&f.data, b(1), f.now).unwrap();
    f.playback.stop_board(stop, &f.data, &mut f.manager, f.now).unwrap();
    assert!(!f.manager.is_playing(first.item()));
    assert_eq!(f.manager.playback_id(second.item()), Some(other_id));
}
#[test]
fn board_held_start_never_becomes_stop_and_held_stop_never_stops_a_new_run() {
    let mut f = Fixture::new(1);
    f.cache(1);
    let target = f.target(1);
    let start = f.ticket(1);
    let definition = f.manager.entries[&target.item()].definition.clone();
    f.manager.play(target.item(), definition.clone(), None);
    let native = f.manager.playback_id(target.item()).unwrap();
    assert!(matches!(
        f.playback.click(start, &f.data, target, &mut f.manager, f.now),
        Err(Error::Stale)
    ));
    let stop = f.ticket(1);
    f.manager.play(target.item(), definition, None);
    let newer = f.manager.playback_id(target.item()).unwrap();
    assert_ne!(native, newer);
    assert!(matches!(
        f.playback.click(stop, &f.data, target, &mut f.manager, f.now),
        Err(Error::Stale)
    ));
    assert_eq!(f.manager.playback_id(target.item()), Some(newer));
}
#[test]
fn board_pending_disabled_and_failed_or_missing_inventory_targets_never_play_or_claim_ownership() {
    let mut f = Fixture::new(1);
    let target = f.target(1);
    let ticket = f.ticket(1);
    f.playback.click(ticket, &f.data, target, &mut f.manager, f.now).unwrap();
    f.cache(1);
    f.manager.enabled = false;
    assert!(matches!(
        f.playback.prepare_pending(ticket, &f.data, target, &f.manager, f.now),
        Err(Error::Blocked)
    ));
    assert!(f.playback.pending(target.item()).is_none());
    assert!(!f.manager.has_playing());
    f.inv.items.remove(&target.item());
    assert!(matches!(
        resolve_playback_item(&f.inv, target.reference(), PlaybackKind::Gesture),
        Err(PlaybackError::Missing)
    ));
}

#[test]
fn board_sound_validation_uses_current_typed_inventory_without_gesture_enable_or_native_run_changes() {
    let mut f = Fixture::new(1);
    let item = id(501);
    f.inv.items.get_mut(&item).unwrap().asset_type = 1;
    f.data = f
        .data
        .apply(Change::Remove { board: b(1), tile: t(1) })
        .unwrap()
        .apply(Change::Add {
            board: b(1),
            tile: Tile {
                id: t(1),
                item: Reference {
                    reference: item,
                    kind: Kind::Sound,
                },
            },
        })
        .unwrap();
    let target = resolve_playback_item(&f.inv, item, PlaybackKind::Sound).unwrap();
    f.manager.enabled = false;
    let ticket = f.playback.capture(&f.data, b(1), t(1), target, &f.manager, f.now).unwrap();
    assert!(f.playback.validate_sound(ticket, &f.data, target, f.now).is_ok());
    assert!(!f.manager.has_playing());
    assert!(f.playback.pending(item).is_none());
    f.playback.invalidate_actions().unwrap();
    assert!(f.playback.validate_sound(ticket, &f.data, target, f.now).is_err());
}
#[test]
fn board_action_counter_exhaustion_does_not_wrap_or_make_a_new_source_eligible() {
    let mut f = Fixture::new(1);
    let target = f.target(1);
    let ticket = f.ticket(1);
    f.playback.click(ticket, &f.data, target, &mut f.manager, f.now).unwrap();
    f.playback.action = u64::MAX;
    assert_eq!(f.playback.invalidate_actions(), Err(Error::Exhausted));
    assert!(f.playback.pending(target.item()).is_none());
    assert!(f.playback.capture(&f.data, b(1), t(1), target, &f.manager, f.now).is_err());
    f.playback.bind(Some(facts())).unwrap();
    assert!(f.playback.capture(&f.data, b(1), t(1), target, &f.manager, f.now).is_err());
}

#[test]
fn board_explicit_tile_stop_uses_its_captured_current_native_run_and_returns_existing_effects() {
    let mut f = Fixture::new(1);
    f.cache(1);
    let target = f.target(1);
    let definition = f.manager.entries[&target.item()].definition.clone();
    f.manager.play(target.item(), definition, None);
    assert_eq!(
        f.manager.tick(f.now, &std::collections::BTreeSet::new()),
        vec![Effect::Animation(id(2501), true)]
    );
    let ticket = f.ticket(1);
    let Click::Stopped(effects) = f.playback.click(ticket, &f.data, target, &mut f.manager, f.now).unwrap() else {
        panic!("explicit native tile stop")
    };
    assert_eq!(effects, vec![Effect::Animation(id(2501), false)]);
    assert!(!f.manager.is_playing(target.item()));
}
#[test]
fn board_normal_managed_activation_change_preserves_owned_run_but_retires_captured_actions() {
    use aurora_session::{AccountKey, Binding, HostId, InstallationId, SessionId, SessionIdentity, WorkerId, WorkerIdentity};
    let mut f = Fixture::new(1);
    let source = ChatSource::try_from((
        Binding {
            worker: WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).unwrap(),
                host: HostId::from_bytes([2; 16]).unwrap(),
                worker: WorkerId::from_bytes([3; 16]).unwrap(),
            },
            session: Some(SessionIdentity {
                session: SessionId::from_bytes([4; 16]).unwrap(),
                account: AccountKey::from_bytes([5; 32]).unwrap(),
                grid: f.data.grid,
                generation: 1,
            }),
        },
        1,
    ))
    .unwrap();
    let mut current = facts();
    current.managed = Some(Managed { source, activation: 1 });
    f.playback.bind(Some(current)).unwrap();
    f.start(1);
    let target = f.target(1);
    let native = f.manager.playback_id(target.item()).unwrap();
    let old = f.playback.capture_board(&f.data, b(1), f.now).unwrap();
    current.managed = Some(Managed { source, activation: 2 });
    f.playback.bind(Some(current)).unwrap();
    assert!(f.playback.stop_board(old, &f.data, &mut f.manager, f.now).is_err());
    assert_eq!(f.manager.playback_id(target.item()), Some(native));
    let fresh = f.playback.capture_board(&f.data, b(1), f.now).unwrap();
    f.playback.stop_board(fresh, &f.data, &mut f.manager, f.now).unwrap();
    assert!(!f.manager.is_playing(target.item()));
    current.logical_generation = 2;
    f.playback.bind(Some(current)).unwrap();
    assert!(
        f.playback.capture_board(&f.data, b(1), f.now).is_err(),
        "invalid managed facts cannot fall back to ordinary"
    );
}
#[test]
fn board_held_board_stop_expiry_keeps_current_owned_work_until_a_fresh_explicit_stop() {
    let mut f = Fixture::new(1);
    f.start(1);
    let target = f.target(1);
    let old = f.playback.capture_board(&f.data, b(1), f.now).unwrap();
    assert!(f.playback.stop_board(old, &f.data, &mut f.manager, f.now + DEADLINE).is_err());
    assert!(f.manager.is_playing(target.item()));
    let fresh = f.playback.capture_board(&f.data, b(1), f.now + DEADLINE).unwrap();
    f.playback.stop_board(fresh, &f.data, &mut f.manager, f.now + DEADLINE).unwrap();
    assert!(!f.manager.is_playing(target.item()));
}

#[test]
fn board_stop_retires_cached_and_pending_ready_before_native_start_without_replay() {
    for pending in [false, true] {
        let mut f = Fixture::new(1);
        let target = f.target(1);
        if !pending {
            f.cache(1);
        }
        let ticket = f.ticket(1);
        let ready = match f.playback.click(ticket, &f.data, target, &mut f.manager, f.now).unwrap() {
            Click::Ready(ready) => ready,
            Click::NeedsCache(queued) => {
                f.cache(1);
                f.playback.prepare_pending(queued, &f.data, target, &f.manager, f.now).unwrap()
            }
            _ => panic!("exact explicit start"),
        };
        let stop = f.playback.capture_board(&f.data, b(1), f.now).unwrap();
        f.playback.stop_board(stop, &f.data, &mut f.manager, f.now).unwrap();
        assert_eq!(f.playback.start(ready, &f.data, target, &mut f.manager, f.now), Err(Error::Stale));
        assert!(!f.manager.has_playing());
        assert!(f.playback.pending(target.item()).is_none());
        let fresh = f.ticket(1);
        let Click::Ready(ready) = f.playback.click(fresh, &f.data, target, &mut f.manager, f.now).unwrap() else {
            panic!("new explicit intent")
        };
        f.playback.start(ready, &f.data, target, &mut f.manager, f.now).unwrap();
        assert!(f.manager.is_playing(target.item()));
    }
}
#[test]
fn board_stop_cancels_only_its_ready_and_pending_domain_and_preserves_other_board_ready() {
    let mut f = Fixture::new(2);
    let second = f.target(2);
    f.data = f
        .data
        .apply(Change::Create {
            id: b(2),
            name: "Other".into(),
        })
        .unwrap()
        .apply(Change::Add {
            board: b(2),
            tile: Tile {
                id: t(9),
                item: Reference {
                    reference: second.reference(),
                    kind: Kind::Gesture,
                },
            },
        })
        .unwrap();
    f.cache(1);
    let first = f.target(1);
    let ticket = f.ticket(1);
    let Click::Ready(ready) = f.playback.click(ticket, &f.data, first, &mut f.manager, f.now).unwrap() else {
        panic!("first ready")
    };
    let second_ticket = f.playback.capture(&f.data, b(2), t(9), second, &f.manager, f.now).unwrap();
    assert!(matches!(
        f.playback.click(second_ticket, &f.data, second, &mut f.manager, f.now),
        Ok(Click::NeedsCache(_))
    ));
    let stop = f.playback.capture_board(&f.data, b(2), f.now).unwrap();
    f.playback.stop_board(stop, &f.data, &mut f.manager, f.now).unwrap();
    assert!(f.playback.pending(second.item()).is_none());
    f.playback.start(ready, &f.data, first, &mut f.manager, f.now).unwrap();
    assert!(f.manager.is_playing(first.item()));
    assert!(!f.manager.is_playing(second.item()));
}
#[test]
fn board_cancel_retires_held_play_and_stop_tickets_without_touching_a_later_explicit_run() {
    let mut f = Fixture::new(1);
    f.cache(1);
    let target = f.target(1);
    let held = f.ticket(1);
    let stop = f.playback.capture_board(&f.data, b(1), f.now).unwrap();
    f.playback.stop_board(stop, &f.data, &mut f.manager, f.now).unwrap();
    assert!(matches!(
        f.playback.click(held, &f.data, target, &mut f.manager, f.now),
        Err(Error::Stale)
    ));
    let fresh = f.ticket(1);
    let Click::Ready(ready) = f.playback.click(fresh, &f.data, target, &mut f.manager, f.now).unwrap() else {
        panic!("new click")
    };
    f.playback.start(ready, &f.data, target, &mut f.manager, f.now).unwrap();
    let run = f.manager.playback_id(target.item()).unwrap();
    assert!(f.playback.stop_board(stop, &f.data, &mut f.manager, f.now).is_err());
    assert_eq!(f.manager.playback_id(target.item()), Some(run));
    let from = f.playback.capture_board(&f.data, b(1), f.now).unwrap();
    f.playback.cancel_board(b(1), &f.data).unwrap();
    assert!(f.playback.stop_board(from, &f.data, &mut f.manager, f.now).is_err());
    assert_eq!(f.manager.playback_id(target.item()), Some(run));
}
