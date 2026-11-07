use super::*;

fn config() -> WorkerConfig {
    WorkerConfig {
        version: PROTOCOL_VERSION,
        identity: WorkerIdentity {
            installation: InstallationId::from_bytes([1; 32]).unwrap(),
            host: HostId::from_bytes([2; 16]).unwrap(),
            worker: WorkerId::from_bytes([3; 16]).unwrap(),
        },
        session: SessionId::from_bytes([4; 16]).unwrap(),
        generation: 1,
        slot: None,
        expected_account: Some(AccountKey::from_bytes([5; 32]).unwrap()),
        expected_grid: Some(GridKey::from_bytes([6; 32]).unwrap()),
        lease_epoch: 7,
        host_pid: 1,
        chat: None,
        preview: Some(crate::session_host::preview_bootstrap::PreviewBootstrap {
            port: 12345,
            key: crate::session_host::preview_bootstrap::PreviewSecret::synthetic([9; 32]).unwrap(),
        }),
    }
}

fn status(config: &WorkerConfig, sequence: u64) -> Envelope {
    Envelope {
        version: PROTOCOL_VERSION,
        binding: Binding {
            worker: config.identity,
            session: Some(config.session_binding(config.expected_account.unwrap(), config.expected_grid.unwrap())),
        },
        lease_epoch: config.lease_epoch,
        sequence,
        reply_to: 0,
        message: Message::Event(Event::Status(WorkerStatus {
            phase: Phase::Online,
            native_window: 12,
            ..Default::default()
        })),
    }
}

#[test]
fn preview_broker_source_proof_requires_real_accepted_full_binding_and_original_receipt() {
    let config = config();
    let mut coordinator = Coordinator::new(config.identity.installation, config.identity.host);
    coordinator.register(config.identity.worker, config.lease_epoch, 0).unwrap();
    let envelope = status(&config, 1);
    let mut child = ChildPreview::new();
    let now = Instant::now();
    assert!(
        child
            .observe(
                ChildReceipt {
                    config: &config,
                    binding: envelope.binding,
                    received: 0,
                    last_seen: None,
                    entry: coordinator.workers().next(),
                    owned: true
                },
                now
            )
            .is_none(),
        "startup allowance cannot authorize an image source"
    );
    coordinator.observe(&envelope, 1).unwrap();
    let proof = child
        .observe(
            ChildReceipt {
                config: &config,
                binding: envelope.binding,
                received: 1,
                last_seen: Some(now),
                entry: coordinator.workers().next(),
                owned: true,
            },
            now,
        )
        .unwrap();
    assert_eq!(proof.source().worker(), config.identity);
    assert_eq!(proof.source().session(), envelope.binding.session.unwrap());
    assert_eq!(proof.source().lease_epoch(), config.lease_epoch);
    assert_eq!(proof.observed_at(), now);
    assert_eq!(proof.expires_at(), now + REACHABLE);
    assert!(!proof.current(now + REACHABLE));
    // Actual Coordinator rejects duplicate input. No accepted receipt was
    // delivered, so rereading/publishing cannot change the original deadline.
    assert!(coordinator.observe(&envelope, 2).is_err());
    let copied = child
        .observe(
            ChildReceipt {
                config: &config,
                binding: envelope.binding,
                received: 1,
                last_seen: Some(now),
                entry: coordinator.workers().next(),
                owned: true,
            },
            now + HEARTBEAT,
        )
        .unwrap();
    assert_eq!(copied.expires_at(), proof.expires_at());
    assert!(proof.witness().same(&copied.witness()));
    assert!(
        child
            .observe(
                ChildReceipt {
                    config: &config,
                    binding: envelope.binding,
                    received: 1,
                    last_seen: Some(now),
                    entry: coordinator.workers().next(),
                    owned: true
                },
                now + REACHABLE
            )
            .is_none()
    );
    assert!(!proof.witness().current());
    let new_receipt = now + REACHABLE;
    coordinator.observe(&status(&config, 2), 3).unwrap();
    let renewed = child
        .observe(
            ChildReceipt {
                config: &config,
                binding: envelope.binding,
                received: 2,
                last_seen: Some(new_receipt),
                entry: coordinator.workers().next(),
                owned: true,
            },
            new_receipt,
        )
        .unwrap();
    assert!(!proof.witness().current(), "later Allowed never revives an old held witness");
    assert!(!proof.witness().same(&renewed.witness()));
}

#[test]
fn preview_broker_source_proof_rejects_config_session_grid_account_and_optional_absence() {
    for refusal in 0..9 {
        let mut config = config();
        let envelope = status(&config, 1);
        let mut coordinator = Coordinator::new(config.identity.installation, config.identity.host);
        coordinator.register(config.identity.worker, config.lease_epoch, 0).unwrap();
        coordinator.observe(&envelope, 1).unwrap();
        let mut child = ChildPreview::new();
        let now = Instant::now();
        let old = child
            .observe(
                ChildReceipt {
                    config: &config,
                    binding: envelope.binding,
                    received: 1,
                    last_seen: Some(now),
                    entry: coordinator.workers().next(),
                    owned: true,
                },
                now,
            )
            .unwrap();
        let mut binding = envelope.binding;
        let mut owned = true;
        let mut seen = Some(now);
        match refusal {
            0 => config.preview = None,
            1 => config.preview.as_mut().unwrap().port = 0,
            2 => config.session = SessionId::from_bytes([11; 16]).unwrap(),
            3 => config.expected_account = Some(AccountKey::from_bytes([11; 32]).unwrap()),
            4 => config.expected_grid = Some(GridKey::from_bytes([11; 32]).unwrap()),
            5 => binding.worker.host = HostId::from_bytes([11; 16]).unwrap(),
            6 => owned = false,
            7 => seen = None,
            8 => binding.session.as_mut().unwrap().generation += 1,
            _ => unreachable!(),
        }
        assert!(
            child
                .observe(
                    ChildReceipt {
                        config: &config,
                        binding,
                        received: 1,
                        last_seen: seen,
                        entry: coordinator.workers().next(),
                        owned
                    },
                    now
                )
                .is_none(),
            "refusal {refusal}"
        );
        assert!(!old.current(now));
    }
}

#[test]
fn preview_broker_none_bootstrap_keeps_exact_view_control_without_minting_image_authority() {
    let mut config = config();
    config.preview = None;
    let envelope = status(&config, 1);
    let mut coordinator = Coordinator::new(config.identity.installation, config.identity.host);
    coordinator.register(config.identity.worker, config.lease_epoch, 0).unwrap();
    coordinator.observe(&envelope, 1).unwrap();
    let now = Instant::now();
    let mut child = ChildPreview::new();
    let view = child
        .observe_view(
            ChildReceipt {
                config: &config,
                binding: envelope.binding,
                received: 1,
                last_seen: Some(now),
                entry: coordinator.workers().next(),
                owned: true,
            },
            now,
        )
        .unwrap();
    assert!(view.current(now));
    assert!(
        child
            .observe(
                ChildReceipt {
                    config: &config,
                    binding: envelope.binding,
                    received: 1,
                    last_seen: Some(now),
                    entry: coordinator.workers().next(),
                    owned: true
                },
                now
            )
            .is_none()
    );
    assert!(view.current(now), "missing optional images never removes canonical view identity");
    assert!(!view.current(now + REACHABLE));
}

#[test]
fn preview_frontdoor_failed_queue_and_terminal_overflow_revoke_before_any_broker_tick() {
    let frontdoor = Arc::new(Frontdoor::default());
    let generation = frontdoor.capture_generation();
    let capture = frontdoor.capture_fence(generation);
    let view = frontdoor.view_fence(frontdoor.view_generation());
    assert!(capture.current() && view.current());
    frontdoor.capture_intent().unwrap();
    assert!(!capture.current());
    assert!(view.current(), "ordinary handoff does not retire admitted history/view");
    // No queue/worker/Broker activity is needed to retire a refused view set.
    frontdoor.views_intent().unwrap();
    assert!(!view.current());
    let latest = frontdoor.capture_fence(frontdoor.capture_generation());
    latest.revoke();
    assert_ne!(frontdoor.capture_generation(), 0, "consumer cannot reset owner generation");
    frontdoor.capture.store(u64::MAX, Ordering::Release);
    let last = frontdoor.capture_fence(u64::MAX);
    assert!(last.current());
    assert!(frontdoor.capture_intent().is_err());
    assert!(!last.current());
    assert!(!frontdoor.current());
    assert!(frontdoor.views_intent().is_err());
    assert_eq!(frontdoor.capture_generation(), 0);
}

fn controller(capacity: usize) -> (HostController, Receiver<HostRequest>) {
    let (sender, receiver) = bounded(capacity);
    let preview_frontdoor = Arc::new(Frontdoor::default());
    let preview_reader = PreviewReader::new(preview_frontdoor.clone());
    (
        HostController {
            preview_frontdoor,
            preview_reader,
            requests: Mutex::new(RequestSender { sender, next: 1 }),
            snapshot: Arc::new(Mutex::new(Arc::new(HostSnapshot::default()))),
            stop: Arc::new(AtomicBool::new(false)),
            stopped: Arc::new(AtomicBool::new(false)),
            foreground: Arc::new(AtomicBool::new(false)),
        },
        receiver,
    )
}

#[test]
fn preview_controller_real_sender_contention_retires_capture_and_failed_view_before_return() {
    let (controller, _receiver) = controller(MAX_QUEUE);
    let capture = controller
        .preview_frontdoor
        .capture_fence(controller.preview_frontdoor.capture_generation());
    let view = controller
        .preview_frontdoor
        .view_fence(controller.preview_frontdoor.view_generation());
    let lock = controller.requests.lock().unwrap();
    assert_eq!(controller.open_manual_login(None, None), Err(HostError::Busy));
    assert!(!capture.current());
    assert!(view.current(), "ordinary failed handoff preserves admitted view history");
    assert_eq!(controller.set_preview_views(Vec::new()), Err(PreviewError::Busy));
    assert!(!view.current());
    assert_eq!(
        controller.preview_frontdoor.failed_views(),
        controller.preview_frontdoor.view_generation()
    );
    drop(lock);
}

#[test]
fn preview_controller_fifo_full_and_operation_credit_rejection_do_not_leak_or_reuse_admission() {
    let (controller, receiver) = controller(1);
    assert_eq!(controller.open_manual_login(None, None).unwrap(), 1);
    assert_eq!(controller.set_preview_views(Vec::new()), Err(PreviewError::Busy));
    let failed = controller.preview_frontdoor.failed_views();
    drop(receiver.try_recv().unwrap());
    let admitted = controller.set_preview_views(Vec::new()).unwrap();
    assert_eq!(admitted.request(), 2, "failed FIFO admission preserves the existing ticket policy");
    assert!(admitted.generation() > failed);
    let packet = receiver.try_recv().unwrap();
    assert_eq!(packet.ticket, admitted.request());
    assert!(
        matches!(&packet.intent, HostIntent::Preview(RequestKind::Views { generation, .. }, _) if *generation == admitted.generation())
    );
    drop(packet);
    let slots: Vec<_> = (0..16).map(|_| controller.preview_reader.reserve().unwrap()).collect();
    let before = controller.preview_frontdoor.view_generation();
    assert_eq!(controller.set_preview_views(Vec::new()), Err(PreviewError::Busy));
    assert!(controller.preview_frontdoor.view_generation() > before);
    assert_eq!(
        controller.preview_frontdoor.failed_views(),
        controller.preview_frontdoor.view_generation()
    );
    drop(slots);
    let slots: Vec<_> = (0..16).map(|_| controller.preview_reader.reserve().unwrap()).collect();
    assert!(controller.preview_reader.reserve().is_err());
    drop(slots);
    assert!(controller.set_preview_views(Vec::new()).is_ok());
}
