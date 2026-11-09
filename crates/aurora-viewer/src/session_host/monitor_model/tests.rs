use super::*;
use aurora_session::{AccountKey, GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerId, WorkerIdentity};

fn source(number: u8) -> Source {
    Source::new(
        SlotId::try_from(number - 1).unwrap(),
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
        },
        true,
    )
    .unwrap()
}

fn order() -> [SlotId; MAX_WORKERS] {
    std::array::from_fn(|index| SlotId::try_from(index as u8).unwrap())
}

fn all() -> [Source; MAX_WORKERS] {
    std::array::from_fn(|index| source(index as u8 + 1))
}

fn fixture(active: Option<u8>) -> MonitorSet {
    let mut model = MonitorSet::new().unwrap();
    model.sync(&all(), order(), active.map(|number| source(number).preview())).unwrap();
    model
}

#[test]
fn monitor_source_mapping_rejects_invalid_session_or_lease_without_aliases() {
    let source = source(1);
    assert_eq!(source.preview().worker(), source.chat().worker);
    assert_eq!(source.preview().session(), source.chat().session);
    assert_eq!(source.preview().lease_epoch(), source.chat().lease_epoch);
    let mut invalid = source.chat();
    invalid.lease_epoch = 0;
    assert_eq!(Source::new(source.slot(), invalid, true), Err(Error::Invalid));
    invalid = source.chat();
    invalid.session.generation = 0;
    assert_eq!(Source::new(source.slot(), invalid, true), Err(Error::Invalid));
}

#[test]
fn monitor_bindings_map_exactly_four_fixed_and_cycling_to_separate_canonical_surfaces() {
    let mut model = fixture(None);
    let expected = [
        ChatSurface::Monitor1,
        ChatSurface::Monitor2,
        ChatSurface::Monitor3,
        ChatSurface::Monitor4,
    ];
    for (index, surface) in expected.into_iter().enumerate() {
        let view = model.open_fixed(source(index as u8 + 1)).unwrap();
        assert_eq!(view.surface(), surface);
        let binding = model.binding(view).unwrap();
        assert_eq!(binding.source.chat(), source(index as u8 + 1).chat());
        assert_eq!(binding.source.slot(), SlotId::try_from(index as u8).unwrap());
        assert_eq!(binding.options.size, PreviewSize::Small);
        assert_eq!(binding.options.rate, PreviewRate::One);
        assert_eq!(
            binding.options,
            Options {
                size: PreviewSize::Small,
                rate: PreviewRate::One
            }
        );
        assert_eq!(
            model.open_fixed(binding.source),
            Ok(view),
            "duplicate fixed Open reuses its binding"
        );
    }
    assert_eq!(model.open_fixed(source(5)), Err(Error::Capacity));
    let mut reverse = order();
    reverse.reverse();
    model.sync(&all(), reverse, None).unwrap();
    model.open_cycler(0).unwrap();
    assert_eq!(View::Cycling.surface(), ChatSurface::Cycling);
    assert_eq!(model.binding(View::Cycling).unwrap().source, source(5));
    assert_eq!(model.visible_sources().len(), 5, "no Active can yield all five unique sources");
    assert!(!View::ALL.iter().any(|view| view.surface() == ChatSurface::Shared));
}

#[test]
fn monitor_explicit_active_open_survives_ordinary_park_and_activation_without_fallback() {
    let mut model = fixture(Some(1));
    assert!(model.visible_sources().is_empty(), "source discovery creates no demand");
    let view = model.open_fixed(source(1)).unwrap();
    let original = model.binding(view).unwrap();
    assert_eq!(model.visible_sources(), vec![source(1)]);
    model.sync(&all(), order(), Some(source(2).preview())).unwrap();
    assert_eq!(
        model.binding(view),
        Some(original),
        "ordinary activation does not exchange monitors"
    );
    model.sync(&all(), order(), None).unwrap();
    assert_eq!(model.binding(view), Some(original));
    assert_eq!(model.visible_sources(), vec![source(1)], "no automatic all-account subscriptions");
}

#[test]
fn monitor_fixed_and_cycler_deduplicate_visible_sources_without_freeing_hidden_peer() {
    let mut model = fixture(Some(5));
    let view = model.open_fixed(source(1)).unwrap();
    model.open_cycler(0).unwrap();
    assert_eq!(model.visible_sources(), vec![source(1)]);
    model.set_visible(view, false).unwrap();
    assert_eq!(model.visible_sources(), vec![source(1)], "cycler still owns explicit visibility");
    model.set_visible(View::Cycling, false).unwrap();
    assert!(
        model.visible_sources().is_empty(),
        "last minimized view lets owner purge source pixels"
    );
    model.set_visible(view, true).unwrap();
    assert_eq!(model.visible_sources(), vec![source(1)]);
    model.close(view).unwrap();
    assert!(
        model.visible_sources().is_empty(),
        "a hidden peer cannot retain visible pixel demand"
    );
}

#[test]
fn monitor_source_replacement_never_rebinds_an_existing_saved_slot_or_reuses_held_tickets() {
    for field in 0..8 {
        let mut model = fixture(None);
        let view = model.open_fixed(source(1)).unwrap();
        let held = model.capture(view).unwrap();
        let mut chat = source(1).chat();
        match field {
            0 => chat.worker.installation = InstallationId::from_bytes([9; 32]).unwrap(),
            1 => chat.worker.host = HostId::from_bytes([9; 16]).unwrap(),
            2 => chat.worker.worker = WorkerId::from_bytes([9; 16]).unwrap(),
            3 => chat.session.account = AccountKey::from_bytes([9; 32]).unwrap(),
            4 => chat.session.grid = GridKey::from_bytes([9; 32]).unwrap(),
            5 => chat.session.session = SessionId::from_bytes([9; 16]).unwrap(),
            6 => chat.session.generation += 1,
            _ => chat.lease_epoch += 1,
        }
        let replacement = Source::new(source(1).slot(), chat, true).unwrap();
        let mut current = all();
        current[0] = replacement;
        model.sync(&current, order(), None).unwrap();
        assert!(!model.binding(view).unwrap().enabled);
        assert_eq!(
            model.binding(view).unwrap().source,
            source(1),
            "no new sender chosen from slot alone"
        );
        assert!(!model.validate(held));
        assert!(model.visible_sources().is_empty());
        assert_eq!(model.open_fixed(replacement), Ok(view));
        assert_ne!(model.binding(view).unwrap().incarnation, held.binding.incarnation);
        assert!(!model.validate(held));
    }
}

#[test]
fn monitor_source_unavailability_restoration_never_resurrects_a_paused_binding_or_ticket() {
    let mut model = fixture(None);
    let view = model.open_fixed(source(1)).unwrap();
    let held = model.capture(view).unwrap();
    let mut unavailable = all();
    unavailable[0] = Source::new(source(1).slot(), source(1).chat(), false).unwrap();
    model.sync(&unavailable, order(), None).unwrap();
    assert!(!model.binding(view).unwrap().enabled && !model.validate(held));
    let local = model.capture_view(view).unwrap();
    assert!(
        model.validate_view(local) && !model.validate(local),
        "local scope grants no source authority"
    );
    model.set_blocked(true).unwrap();
    assert!(!model.validate_view(local));
    model.set_blocked(false).unwrap();
    assert!(!model.validate_view(local), "unblocking cannot revive an old local action");
    model.sync(&all(), order(), None).unwrap();
    assert!(
        !model.binding(view).unwrap().enabled,
        "fresh availability is not explicit rebind consent"
    );
    assert!(model.visible_sources().is_empty());
    assert_eq!(model.open_fixed(source(1)), Ok(view));
    assert!(model.binding(view).unwrap().enabled && !model.validate(held));
}

#[test]
fn monitor_held_tickets_cancel_on_focus_modal_visibility_close_and_model_recreation() {
    let mut model = fixture(None);
    let view = model.open_fixed(source(1)).unwrap();
    let focus = model.capture(view).unwrap();
    model.cancel_input().unwrap();
    assert!(!model.validate(focus));
    let modal = model.capture(view).unwrap();
    model.set_blocked(true).unwrap();
    assert!(model.capture(view).is_none());
    model.set_blocked(false).unwrap();
    assert!(!model.validate(modal));
    let hidden = model.capture(view).unwrap();
    model.set_visible(view, false).unwrap();
    model.set_visible(view, true).unwrap();
    assert!(!model.validate(hidden));
    let closed = model.capture(view).unwrap();
    model.close(view).unwrap();
    model.open_fixed(source(1)).unwrap();
    assert!(!model.validate(closed));
    let mut other = super::MonitorSet::new().unwrap();
    other.sync(&all(), order(), None).unwrap();
    other.open_fixed(source(1)).unwrap();
    assert!(!other.validate(model.capture(view).unwrap()), "no cross-instance ticket reuse");
}

#[test]
fn monitor_held_navigation_keeps_exact_source_after_reorder_or_earlier_source_disappearance() {
    let mut model = fixture(Some(5));
    model.open_cycler(0).unwrap();
    for _ in 0..2 {
        let next = model.capture_navigation(Direction::Next).unwrap();
        model.navigate(next).unwrap();
    }
    assert_eq!(model.binding(View::Cycling).unwrap().source, source(3));
    let navigation = model.capture_navigation(Direction::Next).unwrap();
    assert_eq!(navigation.target(), source(4));
    let mut reversed = order();
    reversed.reverse();
    let current = [source(1), source(3), source(4), source(5)];
    model.sync(&current, reversed, Some(source(5).preview())).unwrap();
    let changed = model.navigate(navigation).unwrap().unwrap();
    assert_eq!(changed.view, View::Cycling);
    assert_eq!(changed.previous, source(3));
    assert_eq!(changed.current, source(4));
    assert!(!model.playing(), "explicit Next pauses before any composer can acquire input");
    assert!(model.navigate(navigation).unwrap().is_none(), "one press cannot apply twice");
    assert_eq!(model.binding(View::Cycling).unwrap().source, source(4));
}

#[test]
fn monitor_held_next_target_replacement_or_promotion_cannot_choose_a_new_target() {
    for promotion in [false, true] {
        let mut model = fixture(Some(5));
        model.open_cycler(0).unwrap();
        let navigation = model.capture_navigation(Direction::Next).unwrap();
        let mut sources = all();
        let active = if promotion {
            Some(source(2).preview())
        } else {
            let mut chat = source(2).chat();
            chat.lease_epoch += 1;
            sources[1] = Source::new(source(2).slot(), chat, true).unwrap();
            Some(source(5).preview())
        };
        model.sync(&sources, order(), active).unwrap();
        assert!(model.navigate(navigation).unwrap().is_none());
        assert_eq!(model.binding(View::Cycling).unwrap().source, source(1));
    }
}

#[test]
fn monitor_paused_source_loss_stays_unavailable_until_explicit_next_or_play() {
    let mut model = fixture(Some(5));
    model.open_cycler(0).unwrap();
    model.pause().unwrap();
    model.sync(&all()[1..], order(), Some(source(5).preview())).unwrap();
    assert!(!model.binding(View::Cycling).unwrap().enabled);
    assert!(model.tick(60_000).unwrap().is_none());
    assert_eq!(model.binding(View::Cycling).unwrap().source, source(1));
    assert!(model.visible_sources().is_empty());
    let next = model.capture_navigation(Direction::Next).unwrap();
    assert_eq!(model.navigate(next).unwrap().unwrap().current, source(2));
    model.sync(&all()[2..], order(), Some(source(5).preview())).unwrap();
    assert!(model.tick(120_000).unwrap().is_none());
    assert_eq!(model.play(120_000).unwrap().unwrap().current, source(3));
}

#[test]
fn monitor_timer_has_one_local_transition_and_rebases_on_modal_and_minimize() {
    let mut model = fixture(Some(5));
    model.open_cycler(100).unwrap();
    assert!(model.playing());
    assert_eq!(model.interval(), 10);
    assert!(model.tick(10_099).unwrap().is_none());
    assert_eq!(model.tick(10_100).unwrap().unwrap().current, source(2));
    assert_eq!(
        model.tick(90_000).unwrap().unwrap().current,
        source(3),
        "missed intervals do not catch up"
    );
    model.set_blocked(true).unwrap();
    assert!(model.tick(150_000).unwrap().is_none());
    model.set_blocked(false).unwrap();
    assert!(model.tick(150_000).unwrap().is_none());
    assert!(model.tick(159_999).unwrap().is_none());
    model.set_visible(View::Cycling, false).unwrap();
    assert!(model.tick(200_000).unwrap().is_none());
    model.set_visible(View::Cycling, true).unwrap();
    assert!(model.tick(200_000).unwrap().is_none());
    assert_eq!(model.tick(210_000).unwrap().unwrap().current, source(4));
    // There is no native command variant or chat state in timer output. The
    // Active source and all fixed assignments remain outside that transition.
    assert_eq!(model.active, Some(source(5).preview()));
    assert!(View::ALL[..FIXED].iter().all(|view| model.binding(*view).is_none()));
}

#[test]
fn monitor_cycle_previous_pauses_wraps_stable_slots_and_skips_active() {
    let mut model = fixture(Some(5));
    model.open_cycler(0).unwrap();
    let previous = model.capture_navigation(Direction::Previous).unwrap();
    assert_eq!(previous.target(), source(4));
    assert_eq!(model.navigate(previous).unwrap().unwrap().current, source(4));
    assert!(!model.playing());
    assert!(model.tick(60_000).unwrap().is_none());
    model.set_interval(2, 60_000).unwrap();
    assert_eq!(model.interval(), 2);
    assert_eq!(model.set_interval(1, 60_000), Err(Error::Invalid));
    assert_eq!(model.set_interval(3601, 60_000), Err(Error::Invalid));
    model.play(60_000).unwrap();
    assert_eq!(model.tick(62_000).unwrap().unwrap().current, source(1));
}

#[test]
fn monitor_exchange_waits_for_exact_outcome_and_rebinds_only_origin_once() {
    let mut model = fixture(Some(1));
    let view = model.open_fixed(source(2)).unwrap();
    let other = model.open_fixed(source(3)).unwrap();
    let other_before = model.binding(other);
    let capture = model.capture(view).unwrap();
    assert_eq!(capture.source(), source(2));
    assert_eq!(capture.view(), view);
    let exchange = model.begin_exchange(capture).unwrap().unwrap();
    assert_eq!(exchange.target(), source(2));
    assert_eq!(exchange.previous(), Some(source(1)));
    assert_eq!(
        model.binding(view).unwrap().source,
        source(2),
        "queue admission is not exchange completion"
    );
    assert!(model.begin_exchange(capture).unwrap().is_none());
    assert!(model.capture(other).is_none(), "pending handoff blocks further gestures");
    model.sync(&all(), order(), Some(source(2).preview())).unwrap();
    assert_eq!(model.finish_exchange(exchange, true), Ok(ExchangeResult::Exchanged));
    assert_eq!(model.binding(view).unwrap().source, source(1));
    assert_eq!(model.binding(other), other_before);
    assert_eq!(model.finish_exchange(exchange, true), Ok(ExchangeResult::Ignored));
    assert!(!model.validate(capture));
}

#[test]
fn monitor_exchange_rollback_and_wrong_or_late_outcome_cannot_consume_a_later_request() {
    let mut model = fixture(Some(1));
    let view = model.open_fixed(source(2)).unwrap();
    let original = model.binding(view);
    let held = model.capture(view).unwrap();
    let failed = model.begin_exchange(held).unwrap().unwrap();
    assert_eq!(model.finish_exchange(failed, false), Ok(ExchangeResult::Kept));
    assert_eq!(model.binding(view), original);
    assert!(!model.validate(held), "rollback cannot replay the already-consumed press");
    let fresh = model.capture(view).unwrap();
    let next = model.begin_exchange(fresh).unwrap().unwrap();
    assert_ne!(failed.operation, next.operation);
    assert_eq!(model.finish_exchange(failed, true), Ok(ExchangeResult::Ignored));
    assert_eq!(model.pending, Some(next));
    model.sync(&all(), order(), Some(source(2).preview())).unwrap();
    assert_eq!(model.finish_exchange(next, true), Ok(ExchangeResult::Exchanged));
}

#[test]
fn monitor_exchange_missing_previous_duplicate_fixed_or_wrong_active_stops_without_fallback() {
    for reason in 0..4 {
        let mut model = fixture((reason != 0).then_some(1));
        let view = model.open_fixed(source(2)).unwrap();
        if reason == 1 {
            model.open_fixed(source(1)).unwrap();
        }
        let exchange = model.begin_exchange(model.capture(view).unwrap()).unwrap().unwrap();
        let current = if reason == 2 { all()[1..].to_vec() } else { all().to_vec() };
        model
            .sync(&current, order(), Some(source(if reason == 3 { 3 } else { 2 }).preview()))
            .unwrap();
        assert_eq!(model.finish_exchange(exchange, true), Ok(ExchangeResult::Stopped));
        assert!(!model.binding(view).unwrap().enabled);
        assert_eq!(model.binding(view).unwrap().source, source(2));
    }
}

#[test]
fn monitor_exchange_source_replacement_close_and_cycler_duplicate_rules_remain_exact() {
    let mut model = fixture(Some(1));
    model.open_fixed(source(1)).unwrap();
    model.open_cycler(0).unwrap();
    let exchange = model.begin_exchange(model.capture(View::Cycling).unwrap()).unwrap().unwrap();
    assert!(!model.playing(), "Exchange pauses the cycling sender");
    model.sync(&all(), order(), Some(source(2).preview())).unwrap();
    assert_eq!(model.finish_exchange(exchange, true), Ok(ExchangeResult::Exchanged));
    assert_eq!(
        model.binding(View::Cycling).unwrap().source,
        source(1),
        "cycler may duplicate a fixed previous Active"
    );

    let mut model = fixture(Some(1));
    let view = model.open_fixed(source(2)).unwrap();
    let exchange = model.begin_exchange(model.capture(view).unwrap()).unwrap().unwrap();
    let mut replaced = all();
    let mut chat = source(2).chat();
    chat.session.generation += 1;
    replaced[1] = Source::new(source(2).slot(), chat, true).unwrap();
    model.sync(&replaced, order(), Some(replaced[1].preview())).unwrap();
    assert_eq!(model.finish_exchange(exchange, true), Ok(ExchangeResult::Stopped));
    assert!(!model.binding(view).unwrap().enabled);

    let mut model = fixture(Some(1));
    let view = model.open_fixed(source(2)).unwrap();
    let exchange = model.begin_exchange(model.capture(view).unwrap()).unwrap().unwrap();
    model.close(view).unwrap();
    model.sync(&all(), order(), Some(source(2).preview())).unwrap();
    assert_eq!(model.finish_exchange(exchange, true), Ok(ExchangeResult::Ignored));
    assert!(model.binding(view).is_none());
}

#[test]
fn monitor_invalid_catalogue_clock_and_identifier_exhaustion_fail_closed() {
    let mut model = fixture(None);
    let view = model.open_fixed(source(1)).unwrap();
    let held = model.capture(view).unwrap();
    let mut invalid_order = order();
    invalid_order[1] = invalid_order[0];
    assert_eq!(model.sync(&all(), invalid_order, None), Err(Error::Invalid));
    assert!(model.binding(view).is_none() && model.visible_sources().is_empty() && !model.validate(held));
    assert_eq!(model.open_fixed(source(1)), Err(Error::Busy));

    let mut model = fixture(None);
    let mut duplicate = all();
    duplicate[1] = duplicate[0];
    assert_eq!(model.sync(&duplicate, order(), None), Err(Error::Invalid));
    let mut model = fixture(None);
    model.open_cycler(100).unwrap();
    let held = model.capture(View::Cycling).unwrap();
    assert_eq!(model.tick(99), Err(Error::Clock));
    assert!(!model.playing() && !model.validate(held));
    let mut model = fixture(None);
    model.open_cycler(0).unwrap();
    let held = model.capture(View::Cycling).unwrap();
    assert_eq!(model.play(u64::MAX), Err(Error::Clock));
    assert!(!model.playing() && !model.validate(held));
    let mut model = fixture(None);
    let view = model.open_fixed(source(1)).unwrap();
    let held = model.capture(view).unwrap();
    model.serial = u64::MAX;
    assert_eq!(model.cancel_input(), Err(Error::Exhausted));
    assert!(model.binding(view).is_none() && !model.validate(held));
    assert_eq!(model.sync(&all(), order(), None), Err(Error::Exhausted));
}

#[test]
fn mcp_cycling_scope_excludes_unapproved_sources_and_replacement_logins() {
    let mut model = fixture(Some(5));
    model.set_cycle_scope(Some(vec![source(2).preview(), source(4).preview()])).unwrap();
    model.open_cycler(0).unwrap();
    assert_eq!(model.binding(View::Cycling).unwrap().source, source(2));
    let next = model.capture_navigation(Direction::Next).unwrap();
    assert_eq!(next.target(), source(4));
    model.navigate(next).unwrap();
    model.play(1).unwrap();
    model.tick(20_000).unwrap();
    assert_eq!(model.binding(View::Cycling).unwrap().source, source(2));
    let mut changed = all();
    changed[1].chat.session.generation += 1;
    changed[1].preview = PreviewSource::new(changed[1].chat.worker, changed[1].chat.session, changed[1].chat.lease_epoch).unwrap();
    model.sync(&changed, order(), Some(source(5).preview())).unwrap();
    model.play(20_001).unwrap();
    assert_eq!(model.binding(View::Cycling).unwrap().source, source(4));
    model.set_cycle_scope(None).unwrap();
    assert!(model.capture_navigation(Direction::Next).is_some());
}
