use super::*;
use aurora_session::{
    AccountKey, GridKey, HostId, InstallationId, PreviewRate, PreviewSize, SessionId, SessionIdentity, WorkerId, WorkerIdentity,
};

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
fn fixture() -> MonitorManager {
    let mut manager = MonitorManager::new().unwrap();
    manager.sources = (1..=5).map(source).collect();
    manager.active = Some(source(5).preview());
    manager.model.sync(&manager.sources, manager.order, manager.active).unwrap();
    manager
}
fn open(manager: &mut MonitorManager, number: u8) -> View {
    manager.open(source(number).preview(), &mut |_, _, _| {}).unwrap()
}
fn desired() -> Options {
    Options {
        size: PreviewSize::Medium,
        rate: PreviewRate::Two,
    }
}
fn pending(manager: &mut MonitorManager, view: View, request: u64) -> SettingsReceipt {
    let base = manager.state(view);
    let ticket = manager.model.capture(view).unwrap();
    let receipt = SettingsReceipt {
        request,
        view,
        namespace: manager.namespace(view).unwrap(),
        ticket,
        base,
        desired: desired(),
        outcome: SettingsOutcome::Accepted {
            options: desired(),
            effective: desired(),
            demand: request,
            native_revision: 1,
        },
    };
    manager.entries[index(view)].settings = Some(SettingsPending {
        request,
        namespace: receipt.namespace,
        ticket,
        binding: manager.model.binding(view).unwrap(),
        base,
        desired: desired(),
    });
    manager.entries[index(view)].state.request = Some((request, desired()));
    manager.bump(view).unwrap();
    receipt
}

#[test]
fn monitor_manager_starts_without_subscriptions_and_reuses_exact_active_binding() {
    let mut manager = fixture();
    assert!(manager.desired().is_empty());
    let view = open(&mut manager, 5);
    let namespace = manager.namespace(view).unwrap();
    assert_eq!(open(&mut manager, 5), view);
    assert_eq!(manager.namespace(view), Some(namespace));
    assert_eq!(manager.desired().len(), 1);
    assert_eq!(manager.state(view).accepted, Options::default());
    assert!(!manager.state(view).chat);
    manager
        .model
        .sync(&manager.sources, manager.order, Some(source(1).preview()))
        .unwrap();
    manager.reconcile().unwrap();
    assert_eq!(
        manager.namespace(view),
        Some(namespace),
        "ordinary activation is not a new subscription"
    );
    assert_eq!(manager.model.binding(view).unwrap().source, source(5));
}

#[test]
fn monitor_manager_minimize_reopen_retires_namespace_and_old_settings_without_losing_binding() {
    let mut manager = fixture();
    let view = open(&mut manager, 1);
    let old = manager.namespace(view).unwrap();
    let receipt = pending(&mut manager, view, 10);
    let binding = manager.model.binding(view).unwrap();
    let mut flushed = Vec::new();
    manager
        .hide(view, &mut |view, binding, state| flushed.push((view, binding, state)))
        .unwrap();
    assert_eq!(flushed[0].1, binding);
    assert!(manager.namespace(view).is_none());
    assert!(manager.desired().is_empty());
    assert_eq!(open(&mut manager, 1), view);
    assert_ne!(manager.namespace(view), Some(old));
    manager
        .settings_receipt(receipt, &mut |_, _, _| panic!("retired receipt cannot flush a new owner"))
        .unwrap();
    assert_eq!(manager.state(view).accepted, Options::default());
    assert!(manager.state(view).request.is_none());
}

#[test]
fn monitor_manager_exact_options_ack_preserves_later_chat_pin_and_consumes_once() {
    let mut manager = fixture();
    let view = open(&mut manager, 1);
    let receipt = pending(&mut manager, view, 10);
    manager.entries[index(view)].state.chat = true;
    manager.entries[index(view)].state.pinned = true;
    manager.bump(view).unwrap();
    manager.cancel_input().unwrap();
    let mut flushed = Vec::new();
    manager
        .settings_receipt(receipt, &mut |view, binding, state| flushed.push((view, binding, state)))
        .unwrap();
    let state = manager.state(view);
    assert!(state.chat && state.pinned);
    assert_eq!(state.accepted, desired());
    assert_eq!(manager.model.binding(view).unwrap().options, desired());
    assert!(state.request.is_none() && !state.failed);
    assert_eq!(flushed.len(), 1);
    assert!(flushed[0].2.chat && flushed[0].2.pinned);
    manager
        .settings_receipt(receipt, &mut |_, _, _| panic!("duplicate ACK cannot replay"))
        .unwrap();
    assert_eq!(manager.state(view), state);
}

#[test]
fn monitor_manager_out_of_order_views_and_uncertain_settings_remain_independent() {
    let mut manager = fixture();
    let first = open(&mut manager, 1);
    let second = open(&mut manager, 2);
    let mut first_receipt = pending(&mut manager, first, 10);
    let mut second_receipt = pending(&mut manager, second, 11);
    // A duplicate view may already keep a higher actual native aggregate. The
    // typed receipt acknowledges this view's exact desired preference, while
    // its effective field truthfully describes the shared source configuration.
    second_receipt.outcome = SettingsOutcome::Accepted {
        options: desired(),
        effective: Options {
            size: PreviewSize::Large,
            rate: PreviewRate::Five,
        },
        demand: 11,
        native_revision: 2,
    };
    manager.settings_receipt(second_receipt, &mut |_, _, _| {}).unwrap();
    first_receipt.outcome = SettingsOutcome::Uncertain;
    manager.settings_receipt(first_receipt, &mut |_, _, _| {}).unwrap();
    assert_eq!(manager.state(second).accepted, desired());
    assert_eq!(manager.state(first).accepted, Options::default());
    assert!(manager.state(first).failed && manager.state(first).request.is_none());
    let retry = pending(&mut manager, first, 12);
    manager
        .settings_receipt(first_receipt, &mut |_, _, _| panic!("older result cannot consume retry"))
        .unwrap();
    assert_eq!(manager.state(first).request, Some((12, desired())));
    manager.settings_receipt(retry, &mut |_, _, _| {}).unwrap();
    assert_eq!(manager.state(first).accepted, desired());
}

#[test]
fn monitor_manager_same_slot_replacement_cannot_ack_or_restore_the_old_source() {
    let mut manager = fixture();
    let view = open(&mut manager, 1);
    let receipt = pending(&mut manager, view, 10);
    let mut chat = source(1).chat();
    chat.session.generation += 1;
    let replacement = Source::new(source(1).slot(), chat, true).unwrap();
    manager.sources[0] = replacement;
    manager.model.sync(&manager.sources, manager.order, manager.active).unwrap();
    manager.reconcile().unwrap();
    manager
        .settings_receipt(receipt, &mut |_, _, _| panic!("replacement cannot acknowledge old source"))
        .unwrap();
    assert!(!manager.model.binding(view).unwrap().enabled);
    assert_eq!(manager.model.binding(view).unwrap().source, source(1));
    assert!(manager.namespace(view).is_none());
    assert!(manager.open(source(1).preview(), &mut |_, _, _| {}).is_err());
    assert_eq!(manager.open(replacement.preview(), &mut |_, _, _| {}).unwrap(), view);
    assert_eq!(manager.model.binding(view).unwrap().source, replacement);
}

#[test]
fn monitor_manager_exchange_typed_outcome_changes_only_origin_once_after_actual_active_sync() {
    let mut manager = fixture();
    let view = open(&mut manager, 1);
    let other = open(&mut manager, 2);
    let exchange = manager.model.begin_exchange(manager.model.capture(view).unwrap()).unwrap().unwrap();
    let receipt = ExchangeReceipt {
        request: 19,
        namespace: manager.namespace(view).unwrap(),
        exchange,
        outcome: ExchangeOutcome::Succeeded {
            activation: 22,
            viewport: HostedViewport {
                host_window: 1,
                rect: aurora_session::PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 640,
                    height: 480,
                },
            },
        },
    };
    manager.exchange = Some(ExchangePending {
        request: receipt.request,
        namespace: receipt.namespace,
        exchange,
        viewport: match receipt.outcome {
            ExchangeOutcome::Succeeded { viewport, .. } => viewport,
            _ => unreachable!(),
        },
    });
    manager
        .model
        .sync(&manager.sources, manager.order, Some(source(1).preview()))
        .unwrap();
    let old_other = manager.model.binding(other);
    let mut flushed = Vec::new();
    manager
        .exchange_receipt(receipt, &mut |view, binding, _| flushed.push((view, binding)))
        .unwrap();
    assert_eq!(flushed[0].1.source, source(1));
    assert_eq!(manager.model.binding(view).unwrap().source, source(5));
    assert_eq!(manager.model.binding(other), old_other);
    let final_binding = manager.model.binding(view);
    manager
        .exchange_receipt(receipt, &mut |_, _, _| panic!("receipt replay cannot exchange twice"))
        .unwrap();
    assert_eq!(manager.model.binding(view), final_binding);
}

#[test]
fn monitor_manager_popup_and_modal_timer_resume_without_catchup_or_stale_local_ticket() {
    let mut manager = fixture();
    manager.open_cycling(0, &mut |_, _, _| {}).unwrap();
    let original = manager.model.binding(View::Cycling).unwrap();
    let ticket = manager.model.capture_view(View::Cycling).unwrap();
    manager
        .tick(100, false, false, &mut |_, _, _| {
            panic!("an ordinary pre-deadline poll must preserve queued native Pause input")
        })
        .unwrap();
    manager
        .tick(9_999, false, false, &mut |_, _, _| {
            panic!("no delivered edit flush without a transition")
        })
        .unwrap();
    manager
        .tick(20_000, false, true, &mut |_, _, _| panic!("open Options suspends cycling"))
        .unwrap();
    manager.tick(30_000, false, false, &mut |_, _, _| {}).unwrap();
    assert_eq!(manager.model.binding(View::Cycling), Some(original));
    manager.tick(39_999, false, false, &mut |_, _, _| {}).unwrap();
    assert_eq!(manager.model.binding(View::Cycling), Some(original));
    manager.tick(40_000, false, false, &mut |_, _, _| {}).unwrap();
    assert_ne!(manager.model.binding(View::Cycling).unwrap().source, original.source);
    manager.tick(50_001, true, false, &mut |_, _, _| {}).unwrap();
    manager.tick(60_001, false, false, &mut |_, _, _| {}).unwrap();
    assert!(
        !manager.model.validate_view(ticket),
        "block/unblock cannot revive an old local gesture"
    );
    let mut one = fixture();
    one.sources.retain(|source| [self::source(1), self::source(5)].contains(source));
    one.model.sync(&one.sources, one.order, one.active).unwrap();
    one.open_cycling(0, &mut |_, _, _| {}).unwrap();
    let same = one.model.binding(View::Cycling);
    one.tick(10_000, false, false, &mut |_, _, _| {
        panic!("one inactive source does not change and must retain queued input")
    })
    .unwrap();
    assert_eq!(one.model.binding(View::Cycling), same);
}

#[test]
fn monitor_manager_modal_and_exchange_input_blockers_do_not_silently_stop_explicit_views() {
    let mut manager = fixture();
    let first = open(&mut manager, 1);
    let second = open(&mut manager, 2);
    let mut submitted = Vec::new();
    manager
        .publish_with(|set| {
            submitted = set;
            Ok(ViewSetRequest::admitted(7, 10))
        })
        .unwrap();
    let published = submitted.clone();
    manager.tick(0, true, false, &mut |_, _, _| {}).unwrap();
    assert!(manager.model.capture_view(first).is_none());
    let remaining = published.iter().find(|descriptor| descriptor.view == second).copied().unwrap();
    let mut flushed = Vec::new();
    manager.close(first, &mut |view, binding, _| flushed.push((view, binding))).unwrap();
    assert_eq!(flushed[0].0, first);
    assert!(manager.namespace(first).is_none());
    manager
        .publish_with(|set| {
            submitted = set;
            Ok(ViewSetRequest::admitted(8, 11))
        })
        .unwrap();
    assert_eq!(submitted, [remaining], "closing one modal-blocked view cannot remove the other");
    manager.tick(1, false, false, &mut |_, _, _| {}).unwrap();
    manager
        .model
        .begin_exchange(manager.model.capture(second).unwrap())
        .unwrap()
        .unwrap();
    assert!(manager.model.capture_view(second).is_none());
    // Actual full-set submit while Exchange suppresses new gestures. A retained
    // typed correlation carries source identity; it is never image permission.
    manager.dirty = true;
    manager
        .publish_with(|set| {
            submitted = set;
            Ok(ViewSetRequest::admitted(9, 12))
        })
        .unwrap();
    assert_eq!(submitted, [remaining], "Exchange cannot silently stop an explicit view");
}

#[test]
fn monitor_ticket_same_binding_ignores_local_input_and_options_but_never_model_or_source_replacement() {
    let mut manager = fixture();
    let view = open(&mut manager, 1);
    let original = manager.model.capture(view).unwrap();
    manager.cancel_input().unwrap();
    let focused = manager.model.capture(view).unwrap();
    assert_ne!(original, focused);
    assert!(original.same_binding(focused));
    let binding = manager.model.binding(view).unwrap();
    manager.model.ack_options(view, binding, desired()).unwrap();
    assert!(original.same_binding(manager.model.capture(view).unwrap()));
    let mut other = fixture();
    let other_view = open(&mut other, 1);
    assert!(
        !original.same_binding(other.model.capture(other_view).unwrap()),
        "a new manager cannot inherit an old namespace identity"
    );
    manager.close(view, &mut |_, _, _| {}).unwrap();
    open(&mut manager, 1);
    assert!(
        !original.same_binding(manager.model.capture(view).unwrap()),
        "reopened incarnation is distinct even for the same full source"
    );
}

#[test]
fn monitor_manager_published_revision_exhaustion_stops_views_without_reusing_a_state_or_namespace() {
    let mut manager = fixture();
    let view = open(&mut manager, 1);
    let old = manager.namespace(view).unwrap();
    manager.publish_with(|_| Ok(ViewSetRequest::admitted(1, 1))).unwrap();
    manager.entries[index(view)].state.revision = u64::MAX;
    assert_eq!(manager.observe_pin(view, true), Err(PreviewError::Exhausted));
    assert!(manager.namespace(view).is_none());
    assert_eq!(manager.state(view).revision, 0);
    let mut stopped = false;
    manager
        .publish_with(|set| {
            assert!(set.is_empty());
            stopped = true;
            Ok(ViewSetRequest::admitted(2, 2))
        })
        .unwrap();
    assert!(stopped, "fail-closed state must reach the actual full-set publication path");
    assert_eq!(manager.open(source(1).preview(), &mut |_, _, _| {}), Err(PreviewError::Stopped));
    assert_ne!(manager.namespace(view), Some(old));
}

#[test]
fn monitor_host_recovery_rejects_stale_press_and_only_explicit_fresh_play_rebinds_unavailable_cycler() {
    let mut manager = fixture();
    manager.open_cycling(0, &mut |_, _, _| {}).unwrap();
    manager.model.pause().unwrap();
    let old_namespace = manager.namespace(View::Cycling).unwrap();
    manager.publish_with(|_| Ok(ViewSetRequest::admitted(1, 1))).unwrap();
    manager.sources.retain(|source| *source != self::source(1));
    manager.model.sync(&manager.sources, manager.order, manager.active).unwrap();
    manager.reconcile().unwrap();
    manager
        .publish_with(|set| {
            assert!(set.is_empty());
            Ok(ViewSetRequest::admitted(2, 2))
        })
        .unwrap();
    assert_eq!(manager.model.binding(View::Cycling).unwrap().source, source(1));
    assert!(
        manager.namespace(View::Cycling).is_none(),
        "no invented view proof for the tombstone"
    );
    let old = manager.recovery().unwrap().unwrap();
    let context = egui::Context::default();
    crate::theme::Theme::default().apply(&context, 1.0);
    let mut frame = 0;
    let mut at = egui::Pos2::ZERO;
    let mut show = |recovery: Recovery, pressed: Option<bool>| {
        frame += 1;
        let events = pressed
            .map(|pressed| {
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            })
            .unwrap_or_default();
        let mut captured = None;
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(380.0, 420.0))),
                time: Some(frame as f64 / 60.0),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                let response = crate::session_host::ui::monitor_recovery_button(ui, recovery, true);
                at = response.rect.center();
                if response.clicked() {
                    captured = Some(recovery);
                }
            },
        );
        output.textures_delta.clear();
        captured
    };
    for _ in 0..4 {
        assert!(show(old, None).is_none());
    }
    assert!(show(old, Some(true)).is_none());
    manager.cancel_input().unwrap();
    let current = manager.recovery().unwrap().unwrap();
    assert_ne!(old.id, current.id);
    assert!(
        show(current, Some(false)).is_none(),
        "release cannot use a replacement local-view ticket"
    );
    assert!(
        !manager
            .resume(old, 10, &mut |_, _, _| panic!("stale recovery cannot flush/rebind"))
            .unwrap()
    );
    assert_eq!(manager.model.binding(View::Cycling).unwrap().source, source(1));
    for _ in 0..4 {
        assert!(show(current, None).is_none());
    }
    assert!(show(current, Some(true)).is_none());
    let captured = show(current, Some(false)).expect("real fresh recovery release");
    let mut flushed = Vec::new();
    assert!(
        manager
            .resume(captured, 20, &mut |view, binding, _| flushed.push((view, binding)))
            .unwrap()
    );
    assert_eq!(flushed[0].1.source, source(1));
    assert_eq!(manager.model.binding(View::Cycling).unwrap().source, source(2));
    assert_ne!(manager.namespace(View::Cycling), Some(old_namespace));
    assert!(manager.namespace(View::Cycling).is_some());
    assert!(manager.recovery().unwrap().is_none());
    assert!(manager.model.playing());
    assert_eq!(manager.desired()[0].source, source(2).preview());
}
