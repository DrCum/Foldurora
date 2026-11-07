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
    let second_receipt = pending(&mut manager, second, 11);
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
}
