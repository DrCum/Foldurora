//! Byte adapters for the state directly used by worker_loop. No thread, file,
//! native window or genuine managed permission is constructed by these cases.
use super::*;
use aurora_session::show_viewer::{ShowViewerLedger, ShowViewerRequest, WindowObservation};
use aurora_session::{HostId, InstallationId, WorkerEffects, WorkerGuard, WorkerId};
use std::collections::HashMap;

fn identity() -> WorkerIdentity {
    WorkerIdentity {
        installation: InstallationId::from_bytes([1; 32]).unwrap(),
        host: HostId::from_bytes([2; 16]).unwrap(),
        worker: WorkerId::from_bytes([3; 16]).unwrap(),
    }
}
fn command(sequence: u64) -> ControlV2Envelope {
    let binding = Binding {
        worker: identity(),
        session: None,
    };
    ControlV2Envelope {
        version: 2,
        binding,
        lease_epoch: 7,
        sequence,
        reply_to: 0,
        message: ControlV2Message::Command(ControlV2Command::ShowViewer(ShowViewerRequest {
            target: ShowViewerTarget {
                binding,
                lease_epoch: 7,
                window: WindowObservation {
                    app: 1,
                    window: 2,
                    ordinary: 3,
                },
            },
            request: sequence,
        })),
    }
}
fn fixture(origin: Instant) -> (WorkerEndpoint, WorkerState) {
    let (tx, commands) = bounded(DOWNSTREAM);
    let (reservations, rx) = bounded(REPLIES);
    let credits = Credits::new(REPLIES);
    let endpoint = WorkerEndpoint {
        commands,
        reservations,
        reply_credits: credits.clone(),
        status: Arc::new(Mutex::new(None)),
        stop: Arc::new(AtomicBool::new(false)),
        failed: Arc::new(AtomicBool::new(false)),
        exited: Arc::new(AtomicBool::new(false)),
        worker: identity(),
        lease_epoch: 7,
        origin,
    };
    let state = WorkerState::new(identity(), 7, tx, rx, credits, origin);
    (endpoint, state)
}
fn step(state: &mut WorkerState, ack: Option<&[u8]>, command: Option<Vec<u8>>, at: Instant, sink: &mut HashMap<String, Vec<u8>>) {
    state
        .step(ack, command, at, None, |name, bytes| {
            sink.insert(name.into(), bytes.into());
            Ok(())
        })
        .unwrap();
}

#[test]
fn v2_actual_worker_loop_full16_reply_and_15plus1_ingress_keep_time_and_drain_independent_ack() {
    let origin = Instant::now();
    let (endpoint, mut state) = fixture(origin);
    let mut sink = HashMap::new();
    for sequence in 1..=REPLIES as u64 {
        let original = command(sequence);
        let ControlV2Message::Command(ControlV2Command::ShowViewer(request)) = original.message else {
            panic!()
        };
        endpoint
            .reserve(&original)
            .unwrap()
            .complete(ControlV2Event::ShowViewerReceipt(ShowViewerReceipt {
                request,
                outcome: ShowViewerOutcome::Cancelled,
            }))
            .unwrap();
    }
    step(&mut state, None, None, origin, &mut sink);
    let first = wire::decode_control(&sink[REPLY]).unwrap();
    assert_eq!(endpoint.reply_credits.used.load(Ordering::Acquire), 16);
    let decoded_at = origin.checked_add(Duration::from_millis(1)).unwrap();
    for sequence in 17..=32 {
        step(
            &mut state,
            None,
            Some(wire::encode_control(&command(sequence)).unwrap()),
            decoded_at,
            &mut sink,
        );
    }
    assert_eq!(endpoint.commands.len(), 15);
    assert_eq!(state.downstream.used.load(Ordering::Acquire), 15);
    assert_eq!(state.stage.as_ref().unwrap().envelope.sequence, 32);
    assert_eq!(state.stage.as_ref().unwrap().received_at, decoded_at);
    assert_eq!(wire::decode_ingress_ack(&sink[INGRESS]).unwrap().command_sequence, 31);
    let original_bytes = state.stage.as_ref().unwrap().bytes.clone();
    let original_digest = state.stage.as_ref().unwrap().digest;
    assert!(matches!(endpoint.reserve(&command(17)), Err(Error::Capacity)));
    let exact = ControlV2Ack {
        version: 2,
        reply: ReplyAckV2 {
            binding: first.binding,
            lease_epoch: 7,
            event_sequence: first.sequence,
            reply_to: first.reply_to,
        },
    };
    for field in 0..4 {
        let mut wrong = exact;
        match field {
            0 => wrong.reply.lease_epoch += 1,
            1 => wrong.reply.event_sequence += 1,
            2 => wrong.reply.reply_to += 1,
            _ => wrong.reply.binding.worker.worker = WorkerId::from_bytes([8; 16]).unwrap(),
        }
        step(
            &mut state,
            Some(&wire::encode_reply_ack(&wrong).unwrap()),
            None,
            decoded_at,
            &mut sink,
        );
        assert_eq!(endpoint.reply_credits.used.load(Ordering::Acquire), 16);
        assert_eq!(state.stage.as_ref().unwrap().received_at, decoded_at);
    }
    let after_wait = origin.checked_add(Duration::from_secs(3)).unwrap();
    step(
        &mut state,
        Some(&wire::encode_reply_ack(&exact).unwrap()),
        None,
        after_wait,
        &mut sink,
    );
    assert_eq!(endpoint.reply_credits.used.load(Ordering::Acquire), 15);
    // Removing the head from channel does not free its physical downstream
    // ownership. It remains charged while the actual App holds that head.
    let head = endpoint.next_command().unwrap();
    assert_eq!(head.envelope.sequence, 17);
    step(&mut state, None, None, after_wait, &mut sink);
    assert!(state.stage.is_some());
    assert_eq!(state.downstream.used.load(Ordering::Acquire), 15);
    let reservation = endpoint.reserve(&head.envelope).unwrap();
    let ControlV2Message::Command(ControlV2Command::ShowViewer(request)) = head.envelope.message else {
        panic!()
    };
    let mut ledger = ShowViewerLedger::new(identity(), 7).unwrap();
    assert_eq!(ledger.reserve(request), Ok(None));
    let mut guard = WorkerGuard::new(identity(), 7, 0).unwrap();
    let transition = guard
        .on_control_v2(&head.envelope, head.received_ms(origin, after_wait).unwrap(), 3000)
        .unwrap();
    assert_eq!(transition.effects, WorkerEffects::default());
    assert_eq!(
        transition.work,
        Some(ControlV2Work::ShowViewer {
            request,
            outcome: Some(ShowViewerOutcome::Expired)
        })
    );
    assert!(!guard.render_owned() && !guard.input_owned());
    let receipt = ledger.finish(request, ShowViewerOutcome::Expired).unwrap();
    reservation.complete(ControlV2Event::ShowViewerReceipt(receipt)).unwrap();
    drop(head);
    step(&mut state, None, None, after_wait, &mut sink);
    assert!(state.stage.is_none());
    assert_eq!(endpoint.commands.len(), 15);
    let transferred = wire::decode_ingress_ack(&sink[INGRESS]).unwrap();
    assert_eq!(transferred.command_sequence, 32);
    assert_eq!(transferred.command_digest, original_digest);
    let tail = endpoint.commands.try_iter().last().unwrap();
    assert_eq!(tail.envelope.sequence, 32);
    assert_eq!(tail.received_at, decoded_at);
    assert_eq!(&*tail.bytes, &*original_bytes);
    assert_eq!(tail.digest, original_digest);
    assert!(!guard.input_owned() && !guard.render_owned());
}

#[test]
fn v2_actual_worker_state_never_remints_staging_or_requeues_transferred_file_and_retains_bytes_on_publish_failure() {
    let origin = Instant::now();
    let (endpoint, mut state) = fixture(origin);
    let mut sink = HashMap::new();
    let bytes = wire::encode_control(&command(1)).unwrap();
    step(&mut state, None, Some(bytes.clone()), origin, &mut sink);
    step(
        &mut state,
        None,
        Some(bytes),
        origin.checked_add(Duration::from_secs(1)).unwrap(),
        &mut sink,
    );
    assert_eq!(endpoint.commands.len(), 1);
    let original = endpoint.next_command().unwrap();
    assert_eq!(original.received_at, origin);
    assert_eq!(
        original.received_ms(origin, origin.checked_sub(Duration::from_millis(1)).unwrap()),
        Err(Error::Stale)
    );
    assert_eq!(
        original.received_ms(origin.checked_add(Duration::from_millis(1)).unwrap(), origin),
        Err(Error::Stale)
    );
    let ControlV2Message::Command(ControlV2Command::ShowViewer(request)) = original.envelope.message else {
        panic!()
    };
    endpoint
        .reserve(&original.envelope)
        .unwrap()
        .complete(ControlV2Event::ShowViewerReceipt(ShowViewerReceipt {
            request,
            outcome: ShowViewerOutcome::EffectUncertain,
        }))
        .unwrap();
    assert!(
        state
            .step(None, None, origin, None, |_, _| Err(io::Error::other(
                "synthetic publication refusal"
            )))
            .is_err()
    );
    let retained = state.replies.front().unwrap().bytes.clone().unwrap();
    let sequence = state.sequence;
    step(
        &mut state,
        None,
        None,
        origin.checked_add(Duration::from_secs(1)).unwrap(),
        &mut sink,
    );
    assert_eq!(sink[REPLY], retained);
    assert_eq!(state.sequence, sequence);
    assert!(state.replies.front().unwrap().published);
}

#[test]
fn v2_actual_host_writer_requires_ingress_not_native_reply_and_retains_original_bytes_age_and_bound() {
    let origin = Instant::now();
    let mut writer = HostControl::new(identity(), 7);
    let mut published = Vec::new();
    for sequence in 1..=16 {
        writer.enqueue(command(sequence), origin).unwrap();
    }
    assert!(writer.enqueue(command(17), origin).is_err());
    assert_eq!(writer.highest, 16);
    writer
        .flush_command(origin, |bytes| {
            published.push(bytes.to_vec());
            Ok(())
        })
        .unwrap();
    let first = published[0].clone();
    let ControlV2Message::Command(ControlV2Command::ShowViewer(request)) = command(1).message else {
        panic!()
    };
    let reply = ControlV2Envelope {
        version: 2,
        binding: request.target.binding,
        lease_epoch: 7,
        sequence: 1,
        reply_to: 1,
        message: ControlV2Message::Event(ControlV2Event::ShowViewerReceipt(ShowViewerReceipt {
            request,
            outcome: ShowViewerOutcome::Cancelled,
        })),
    };
    assert!(writer.reply(&reply).is_none());
    assert_eq!(writer.pending, Some(1));
    let receipt = ControlIngressAckV2 {
        version: 2,
        binding: request.target.binding,
        lease_epoch: 7,
        command_sequence: 1,
        command_digest: Sha256::digest(&first).into(),
    };
    let mut wrong = receipt;
    wrong.command_digest[0] ^= 1;
    assert!(!writer.ingress(&wrong));
    writer
        .flush_command(origin.checked_add(Duration::from_secs(3)).unwrap(), |bytes| {
            assert_eq!(bytes, first);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        writer.pending,
        Some(1),
        "an attempted publication cannot expire/reissue its native request"
    );
    assert!(writer.ingress(&receipt));
    assert!(writer.reply(&reply).is_some());
    assert_eq!(writer.receipts.len(), 1);
    assert_eq!(writer.operations.len(), 15);
    // Original capture expired while waiting behind the first transfer. It is
    // removed locally with its exact identity and is never transmitted.
    let expired = writer
        .flush_command(origin.checked_add(Duration::from_secs(3)).unwrap(), |_| {
            panic!("expired native Show was published")
        })
        .unwrap()
        .unwrap();
    assert_eq!(expired.request, 2);
    assert_eq!(writer.operations.len(), 14);
    assert!(!writer.ingress(&receipt));
}

#[test]
fn v2_reply_reservation_drop_and_effect_uncertainty_keep_original_scope_and_credit_until_ack() {
    let origin = Instant::now();
    let (endpoint, mut state) = fixture(origin);
    let mut sink = HashMap::new();
    let reservation = endpoint.reserve(&command(1)).unwrap();
    reservation.mark_effect_started();
    drop(reservation);
    assert_eq!(endpoint.reply_credits.used.load(Ordering::Acquire), 1);
    step(&mut state, None, None, origin, &mut sink);
    let envelope = wire::decode_control(&sink[REPLY]).unwrap();
    assert_eq!(envelope.binding, command(1).binding);
    assert_eq!(envelope.reply_to, 1);
    assert!(matches!(
        envelope.message,
        ControlV2Message::Event(ControlV2Event::ShowViewerReceipt(ShowViewerReceipt {
            outcome: ShowViewerOutcome::EffectUncertain,
            ..
        }))
    ));
    let ack = ControlV2Ack {
        version: 2,
        reply: ReplyAckV2 {
            binding: envelope.binding,
            lease_epoch: 7,
            event_sequence: envelope.sequence,
            reply_to: 1,
        },
    };
    step(&mut state, Some(&wire::encode_reply_ack(&ack).unwrap()), None, origin, &mut sink);
    assert_eq!(endpoint.reply_credits.used.load(Ordering::Acquire), 0);
    step(&mut state, Some(&wire::encode_reply_ack(&ack).unwrap()), None, origin, &mut sink);
    assert_eq!(endpoint.reply_credits.used.load(Ordering::Acquire), 0);
}

#[test]
fn v2_actual_worker_loop_slow_startup_preserves_old_stamp_and_fresh_post_ready_admission_without_rearm() {
    // Actual IO state/credits with simulated monotonic time. The pure startup
    // clock seam is exercised directly; no AppReady/window capability or
    // genuine native permission is fabricated by this CPU fixture.
    let origin = Instant::now();
    let (endpoint, mut state) = fixture(origin);
    let mut sink = HashMap::new();
    let old_bytes = wire::encode_control(&command(1)).unwrap();
    step(&mut state, None, Some(old_bytes.clone()), origin, &mut sink);
    let ready = origin.checked_add(Duration::from_secs(20)).unwrap();
    let old = endpoint.next_command().unwrap();
    assert_eq!(old.received_at, origin);
    assert_eq!(&*old.bytes, &*old_bytes);
    let mut guard = WorkerGuard::new(identity(), 7, 0).unwrap();
    guard.initialize_startup_control_v2(V2StartupClock::at_app_ready(20_000)).unwrap();
    assert_eq!(
        guard.on_control_v2(&old.envelope, old.received_ms(origin, ready).unwrap(), 20_000),
        Err(Error::Stale)
    );
    let reply = endpoint.reserve(&old.envelope).unwrap();
    let ControlV2Message::Command(captured) = &old.envelope.message else {
        panic!()
    };
    reply
        .complete(ControlV2Event::ControlUnavailable { command: captured.clone() })
        .unwrap();
    drop(old);
    step(&mut state, None, None, ready, &mut sink);
    let unavailable = wire::decode_control(&sink[REPLY]).unwrap();
    assert_eq!(unavailable.reply_to, 1);
    assert!(matches!(
        unavailable.message,
        ControlV2Message::Event(ControlV2Event::ControlUnavailable { .. })
    ));
    let after = ready.checked_add(Duration::from_millis(1)).unwrap();
    step(&mut state, None, Some(wire::encode_control(&command(2)).unwrap()), after, &mut sink);
    let fresh = endpoint.next_command().unwrap();
    assert_eq!(fresh.received_at, after);
    let transition = guard
        .on_control_v2(&fresh.envelope, fresh.received_ms(origin, after).unwrap(), 20_001)
        .unwrap();
    assert_eq!(transition.effects, WorkerEffects::default());
    assert!(matches!(transition.work, Some(ControlV2Work::ShowViewer { outcome: None, .. })));
    assert!(!guard.input_owned() && !guard.render_owned());
    assert_eq!(
        guard.initialize_startup_control_v2(V2StartupClock::at_app_ready(21_000)),
        Err(Error::Stale)
    );
    guard.invalidate_display();
    assert_eq!(
        guard.initialize_startup_control_v2(V2StartupClock::at_app_ready(22_000)),
        Err(Error::Stale)
    );
    assert!(guard.tick(35_001).restore_window);
    assert_eq!(
        guard.initialize_startup_control_v2(V2StartupClock::at_app_ready(40_000)),
        Err(Error::Stale)
    );
}

#[test]
fn v2_actual_staged_sixteenth_pre_ready_command_keeps_original_bytes_time_and_cannot_gain_authority() {
    let origin = Instant::now();
    let (endpoint, mut state) = fixture(origin);
    let mut sink = HashMap::new();
    for sequence in 1..=16 {
        step(
            &mut state,
            None,
            Some(wire::encode_control(&command(sequence)).unwrap()),
            origin,
            &mut sink,
        );
    }
    assert_eq!(endpoint.commands.len(), 15);
    let staged = state.stage.as_ref().unwrap();
    assert_eq!(staged.envelope.sequence, 16);
    let bytes = staged.bytes.clone();
    let digest = staged.digest;
    assert_eq!(wire::decode_ingress_ack(&sink[INGRESS]).unwrap().command_sequence, 15);
    // Removing from the channel alone retains all fifteen owned credits.
    let mut held = Vec::new();
    while let Some(command) = endpoint.next_command() {
        held.push(command);
    }
    let ready = origin.checked_add(Duration::from_secs(20)).unwrap();
    step(&mut state, None, None, ready, &mut sink);
    assert!(state.stage.is_some());
    let mut guard = WorkerGuard::new(identity(), 7, 0).unwrap();
    guard.initialize_startup_control_v2(V2StartupClock::at_app_ready(20_000)).unwrap();
    drop(held.pop());
    step(&mut state, None, None, ready, &mut sink);
    let old = endpoint.next_command().unwrap();
    assert_eq!(old.envelope.sequence, 16);
    assert_eq!(old.received_at, origin);
    assert!(old.predates_ready(ready));
    assert!(old.integrity());
    assert_eq!(&*old.bytes, &*bytes);
    assert_eq!(old.digest, digest);
    let ingress = wire::decode_ingress_ack(&sink[INGRESS]).unwrap();
    assert!(ingress.matches_ingress(&old.envelope, old.digest, identity(), 7));
    assert_eq!(
        guard.on_control_v2(&old.envelope, old.received_ms(origin, ready).unwrap(), 20_000),
        Err(Error::Stale)
    );
    let ControlV2Message::Command(captured) = &old.envelope.message else {
        panic!()
    };
    endpoint
        .reserve(&old.envelope)
        .unwrap()
        .complete(ControlV2Event::ControlUnavailable { command: captured.clone() })
        .unwrap();
    drop(old);
    drop(held);
    step(&mut state, None, None, ready, &mut sink);
    let terminal = wire::decode_control(&sink[REPLY]).unwrap();
    assert_eq!(terminal.reply_to, 16);
    assert!(wire::matches_reply(&command(16), &terminal));
    let ack = ControlV2Ack {
        version: 2,
        reply: ReplyAckV2 {
            binding: terminal.binding,
            lease_epoch: terminal.lease_epoch,
            event_sequence: terminal.sequence,
            reply_to: terminal.reply_to,
        },
    };
    step(&mut state, Some(&wire::encode_reply_ack(&ack).unwrap()), None, ready, &mut sink);
    assert_eq!(endpoint.reply_credits.used.load(Ordering::Acquire), 0);
    let after = ready.checked_add(Duration::from_millis(1)).unwrap();
    step(
        &mut state,
        None,
        Some(wire::encode_control(&command(17)).unwrap()),
        after,
        &mut sink,
    );
    let fresh = endpoint.next_command().unwrap();
    assert!(!fresh.predates_ready(ready));
    let transition = guard
        .on_control_v2(&fresh.envelope, fresh.received_ms(origin, after).unwrap(), 20_001)
        .unwrap();
    assert_eq!(transition.effects, WorkerEffects::default());
    assert!(matches!(transition.work, Some(ControlV2Work::ShowViewer { outcome: None, .. })));
    assert!(!guard.input_owned() && !guard.render_owned());
}
