use super::*;
use aurora_session::chat::{Conversation, ObservedIncoming};
use aurora_session::{FrameProof, PhysicalRect, PrepareBoundary, WorkerGuard};

fn target() -> ReviewCapture {
    ReviewCapture {
        key: ChatKey {
            source: ChatSource {
                worker: WorkerIdentity {
                    installation: InstallationId::from_bytes([1; 32]).unwrap(),
                    host: HostId::from_bytes([2; 16]).unwrap(),
                    worker: WorkerId::from_bytes([3; 16]).unwrap(),
                },
                session: SessionIdentity {
                    session: SessionId::from_bytes([4; 16]).unwrap(),
                    account: AccountKey::from_bytes([5; 32]).unwrap(),
                    grid: GridKey::from_bytes([6; 32]).unwrap(),
                    generation: 1,
                },
                lease_epoch: 7,
            },
            conversation: Conversation::Nearby,
        },
        policy_revision: 1,
        observed: ObservedIncoming { epoch: 1, through: 9 },
    }
}

fn viewport() -> HostedViewport {
    HostedViewport {
        host_window: 1,
        rect: PhysicalRect {
            x: 0,
            y: 0,
            width: 640,
            height: 480,
        },
    }
}

fn only_send(effects: &[Effect]) -> Envelope {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Send(envelope) => Some(envelope.clone()),
            _ => None,
        })
        .unwrap()
}

fn event(sequence: u64, reply_to: u64, event: Event) -> Envelope {
    Envelope {
        version: PROTOCOL_VERSION,
        binding: target().key.source.binding(),
        lease_epoch: target().key.source.lease_epoch,
        sequence,
        reply_to,
        message: Message::Event(event),
    }
}

#[test]
fn broker_review_existing_active_shortcut_requires_exact_retained_binding_viewport_and_child_grant() {
    let target = target();
    let active = ActiveViewport {
        worker: target.key.source.worker.worker,
        binding: target.key.source.binding(),
        viewport: viewport(),
        activation: 8,
    };
    let expected = ReviewGrant {
        binding: active.binding,
        activation: 8,
        viewport: viewport(),
    };
    assert_eq!(
        exact_review_grant(
            Some(active),
            Some(active.worker),
            Some(viewport()),
            false,
            Some(active.binding),
            Some((active.binding, 8))
        ),
        Some(expected)
    );
    for missing in 0..8 {
        let mut placement = Some(active);
        let mut worker = Some(active.worker);
        let mut viewport = Some(viewport());
        let mut binding = Some(active.binding);
        let mut granted = Some((active.binding, 8));
        let mut busy = false;
        match missing {
            0 => placement = None,
            1 => worker = None,
            2 => viewport.as_mut().unwrap().rect.x += 1,
            3 => binding.as_mut().unwrap().session.as_mut().unwrap().generation += 1,
            4 => granted = None,
            5 => granted.as_mut().unwrap().1 += 1,
            6 => granted.as_mut().unwrap().0.session.as_mut().unwrap().account = AccountKey::from_bytes([11; 32]).unwrap(),
            7 => busy = true,
            _ => unreachable!(),
        }
        assert!(
            exact_review_grant(placement, worker, viewport, busy, binding, granted).is_none(),
            "missing exact proof {missing}"
        );
    }
}

#[test]
fn broker_review_wrong_ack_sequence_binding_activation_lease_and_frame_cannot_dispatch() {
    let target = target();
    let source = target.key.source;
    let mut coordinator = Coordinator::new(source.worker.installation, source.worker.host);
    coordinator.register(source.worker.worker, source.lease_epoch, 0).unwrap();
    coordinator
        .observe(
            &event(
                1,
                0,
                Event::Status(WorkerStatus {
                    phase: Phase::Online,
                    native_window: 1,
                    ..Default::default()
                }),
            ),
            1,
        )
        .unwrap();
    let mut worker = WorkerGuard::new(source.worker, source.lease_epoch, 0).unwrap();
    worker.update_session(Some(source.session)).unwrap();
    let mut transactions = review::Transactions::default();
    assert!(transactions.begin(target, 40, viewport(), 2));
    let effects = coordinator.activate(source.worker.worker, Some(viewport()), 2).unwrap();
    let activation = review_activation(&effects).unwrap();
    assert!(transactions.expect_activation(activation));
    let prepare = only_send(&effects);
    worker.on_command(&prepare, 2).unwrap();
    worker
        .prepare_boundary(
            prepare.sequence,
            PrepareBoundary {
                device_epoch: 1,
                surface_epoch: 1,
                after_frame: 0,
            },
        )
        .unwrap();
    let ready = worker
        .frame_ready(
            prepare.sequence,
            FrameProof {
                device_epoch: 1,
                surface_epoch: 1,
                frame: 1,
            },
        )
        .unwrap()
        .unwrap();
    let mut invalid_frame = event(2, ready.reply_to, ready.event.clone());
    if let Message::Event(Event::FrameReady { boundary, .. }) = &mut invalid_frame.message {
        boundary.after_frame = 1;
    }
    assert!(coordinator.observe(&invalid_frame, 3).is_err());
    assert!(
        transactions
            .dispatch(ReviewGrant {
                binding: source.binding(),
                activation,
                viewport: viewport()
            })
            .is_none()
    );
    let grant = only_send(&coordinator.observe(&event(2, ready.reply_to, ready.event), 3).unwrap());
    let granted = worker.on_command(&grant, 4).unwrap().reply.unwrap();
    let exact = event(3, granted.reply_to, granted.event);
    for mismatch in 0..5 {
        let mut wrong = exact.clone();
        match mismatch {
            0 => wrong.sequence = 1,
            1 => wrong.reply_to += 1,
            2 => wrong.binding.session.as_mut().unwrap().generation += 1,
            3 => wrong.lease_epoch += 1,
            4 => {
                wrong.message = Message::Event(Event::Granted {
                    activation: activation + 1,
                })
            }
            _ => unreachable!(),
        }
        assert!(coordinator.observe(&wrong, 5).is_err(), "wrong ack {mismatch}");
        assert!(
            transactions
                .dispatch(ReviewGrant {
                    binding: source.binding(),
                    activation,
                    viewport: viewport()
                })
                .is_none()
        );
    }
    let effects = coordinator.observe(&exact, 5).unwrap();
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Notice(HostNotice::Switched(_))))
    );
    let proof = ReviewGrant {
        binding: source.binding(),
        activation,
        viewport: viewport(),
    };
    transactions.granted(proof);
    let dispatched = transactions.dispatch(proof).unwrap();
    assert_eq!(dispatched.target, target);
    transactions.queued(dispatched);
    assert!(transactions.dispatch(proof).is_none());
    assert!(
        coordinator.activate(source.worker.worker, Some(viewport()), 6).unwrap().is_empty(),
        "same exact Active viewport earns no new handoff"
    );
}

#[test]
fn broker_review_local_outcomes_do_not_cover_acknowledged_world_or_clear_control_errors() {
    let mut error = Some("Unrelated presentation failure".into());
    let intent = HostIntent::ReviewConversation(target(), viewport());
    assert!(!intent.controls_presentation());
    request_result(&mut error, intent.controls_presentation(), None, Err(HostError::Unavailable.into()));
    assert_eq!(error.as_deref(), Some("Unrelated presentation failure"));
    request_result(&mut error, false, None, Ok(()));
    assert_eq!(error.as_deref(), Some("Unrelated presentation failure"));
    assert!(!HostIntent::CancelReview(40).controls_presentation());
}
