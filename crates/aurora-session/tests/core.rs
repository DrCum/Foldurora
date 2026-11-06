use aurora_session::*;

fn worker_id(value: u8) -> WorkerId {
    WorkerId::from_bytes([value; 16]).unwrap()
}
fn identity(value: u8) -> WorkerIdentity {
    WorkerIdentity {
        installation: InstallationId::from_bytes([1; 32]).unwrap(),
        host: HostId::from_bytes([2; 16]).unwrap(),
        worker: worker_id(value),
    }
}
fn session(value: u8, generation: u64) -> SessionIdentity {
    SessionIdentity {
        session: SessionId::from_bytes([value; 16]).unwrap(),
        account: AccountKey::from_bytes([value; 32]).unwrap(),
        grid: GridKey::from_bytes([5; 32]).unwrap(),
        generation,
    }
}
fn frame(value: u64) -> FrameProof {
    FrameProof {
        device_epoch: 1,
        surface_epoch: 1,
        frame: value,
    }
}
fn progress(submitted: u64, completed: u64) -> GpuProgress {
    GpuProgress {
        device_epoch: 1,
        submitted,
        completed,
    }
}
fn command(binding: Binding, sequence: u64, command: Command) -> Envelope {
    Envelope {
        version: PROTOCOL_VERSION,
        binding,
        lease_epoch: 1,
        sequence,
        reply_to: 0,
        message: Message::Command(command),
    }
}
fn event(binding: Binding, sequence: u64, reply_to: u64, event: Event) -> Envelope {
    Envelope {
        version: PROTOCOL_VERSION,
        binding,
        lease_epoch: 1,
        sequence,
        reply_to,
        message: Message::Event(event),
    }
}
fn message(effects: Vec<Effect>) -> Envelope {
    effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Send(message) => Some(message),
            _ => None,
        })
        .expect("a protocol step")
}

struct Peer {
    guard: WorkerGuard,
    sequence: u64,
    status: WorkerStatus,
}
impl Peer {
    fn new(value: u8) -> Self {
        let mut guard = WorkerGuard::new(identity(value), 1, 0).unwrap();
        guard.update_session(Some(session(value, 1))).unwrap();
        guard.frame_ready(0, frame(10)).unwrap();
        Self {
            guard,
            sequence: 0,
            status: WorkerStatus {
                phase: Phase::Online,
                name: Label::new("Synthetic character").unwrap(),
                grid_label: Label::new("Synthetic grid").unwrap(),
                native_window: value as u64,
                last_frame: Some(frame(10)),
                ..Default::default()
            },
        }
    }
    fn envelope(&mut self, reply: Reply) -> Envelope {
        self.sequence += 1;
        event(self.guard.binding(), self.sequence, reply.reply_to, reply.event)
    }
    fn status(&mut self) -> Envelope {
        self.envelope(Reply {
            reply_to: 0,
            event: Event::Status(self.status.clone()),
        })
    }
    fn accept(&mut self, command: &Envelope, now: u64) -> WorkerTransition {
        self.guard.on_command(command, now).unwrap()
    }
    fn ready(&mut self, command: &Envelope, value: u64) -> Envelope {
        self.guard
            .prepare_boundary(
                command.sequence,
                PrepareBoundary {
                    device_epoch: 1,
                    surface_epoch: 1,
                    after_frame: value - 1,
                },
            )
            .unwrap();
        let reply = self
            .guard
            .frame_ready(command.sequence, frame(value))
            .unwrap()
            .expect("fresh prepared frame");
        self.status.last_frame = Some(frame(value));
        self.envelope(reply)
    }
    fn idle(&mut self, command: &Envelope, value: u64) -> Envelope {
        let reply = self
            .guard
            .quiesced(command.sequence, progress(value, value))
            .unwrap()
            .expect("GPU idle");
        self.envelope(reply)
    }
}
fn coordinator(peers: &mut [&mut Peer]) -> Coordinator {
    let identity = identity(1);
    let mut host = Coordinator::new(identity.installation, identity.host);
    for peer in peers {
        host.register(peer.guard.binding().worker.worker, 1, 0).unwrap();
        assert!(host.observe(&peer.status(), 0).unwrap().is_empty());
    }
    host
}
fn initial_activate(host: &mut Coordinator, peer: &mut Peer) {
    let prepare = message(host.activate(peer.guard.binding().worker.worker, None, 0).unwrap());
    assert!(peer.accept(&prepare, 0).effects.prepare);
    let ready = peer.ready(&prepare, 11);
    let grant = message(host.observe(&ready, 0).unwrap());
    assert_eq!(host.active(), None);
    let accepted = peer.accept(&grant, 0).reply.unwrap();
    let ack = peer.envelope(accepted);
    assert_eq!(
        host.observe(&ack, 0).unwrap(),
        vec![Effect::Notice(HostNotice::Switched(peer.guard.binding().worker.worker))]
    );
}

#[test]
fn wire_roundtrip_is_bounded_versioned_and_rejects_unknown_or_malformed_fields() {
    let binding = Binding {
        worker: identity(3),
        session: Some(session(3, 1)),
    };
    let original = command(
        binding,
        1,
        Command::Prepare {
            activation: 1,
            viewport: Some(HostedViewport {
                host_window: 17,
                rect: PhysicalRect {
                    x: -1920,
                    y: -80,
                    width: 1920,
                    height: 1080,
                },
            }),
        },
    );
    let bytes = encode(&original).unwrap();
    assert!(bytes.len() < MAX_MESSAGE_BYTES);
    assert_eq!(decode(&bytes), Ok(original.clone()));
    let mut changed = original.clone();
    changed.version += 1;
    assert_eq!(encode(&changed), Err(Error::Version));
    changed = original.clone();
    changed.binding.session.as_mut().unwrap().generation = 0;
    assert_eq!(encode(&changed), Err(Error::Identity));
    changed = original;
    changed.reply_to = 5;
    assert_eq!(encode(&changed), Err(Error::Sequence));
    assert_eq!(decode(&vec![b' '; MAX_MESSAGE_BYTES + 1]), Err(Error::Oversized));
    let mut unknown: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    unknown["generic_command"] = "send arbitrary OS input".into();
    assert_eq!(decode(&serde_json::to_vec(&unknown).unwrap()), Err(Error::Malformed));
    unknown.as_object_mut().unwrap().remove("generic_command");
    unknown["message"]["body"]["data"]["viewport"]["rect"]["width"] = 0.into();
    assert_eq!(decode(&serde_json::to_vec(&unknown).unwrap()), Err(Error::Invalid));
}

#[test]
fn opaque_identity_domains_canonical_hex_and_labels_cannot_bypass_deserialization() {
    assert!(WorkerId::from_bytes([0; 16]).is_err());
    assert!(WorkerId::parse_hex(&"A".repeat(32)).is_err());
    assert!(WorkerId::parse_hex("../profile").is_err());
    let id = worker_id(7);
    assert_eq!(WorkerId::parse_hex(&id.to_string()), Ok(id));
    assert_eq!(id.as_bytes(), &[7; 16]);
    assert!(Label::new("text\nother").is_err());
    assert!(Label::new(&"é".repeat(65)).is_err());
    assert!(serde_json::from_str::<Label>("\"invalid\\nlabel\"").is_err());
    assert!(serde_json::from_str::<WorkerId>("\"00000000000000000000000000000000\"").is_err());
    assert!(
        PhysicalRect {
            x: i32::MAX,
            y: 0,
            width: 1,
            height: 1
        }
        .validate()
        .is_err()
    );
}

#[test]
fn worker_requires_captured_request_new_frame_and_gpu_quiescence_before_grant() {
    let mut peer = Peer::new(3);
    let binding = peer.guard.binding();
    let premature = peer.accept(&command(binding, 1, Command::Grant { activation: 1 }), 0);
    assert!(!premature.effects.grant_input);
    assert!(matches!(premature.reply.unwrap().event, Event::Rejected { .. }));
    let prepare = command(
        binding,
        2,
        Command::Prepare {
            activation: 1,
            viewport: None,
        },
    );
    peer.accept(&prepare, 0);
    peer.guard
        .prepare_boundary(
            2,
            PrepareBoundary {
                device_epoch: 1,
                surface_epoch: 1,
                after_frame: 10,
            },
        )
        .unwrap();
    assert_eq!(peer.guard.frame_ready(999, frame(11)), Err(Error::Stale));
    assert!(peer.guard.frame_ready(2, frame(10)).unwrap().is_none());
    assert!(peer.guard.frame_ready(0, frame(11)).unwrap().is_none());
    assert!(
        peer.guard.frame_ready(2, frame(11)).unwrap().is_none(),
        "ordinary frame cannot be retagged as prepared"
    );
    assert!(peer.guard.frame_ready(2, frame(12)).unwrap().is_some());
    assert!(
        peer.accept(&command(binding, 3, Command::Grant { activation: 1 }), 0)
            .effects
            .grant_input
    );
    let revoke = command(binding, 4, Command::Revoke { activation: 2 });
    assert!(peer.accept(&revoke, 0).effects.release_input);
    assert!(!peer.guard.input_owned());
    assert_eq!(peer.guard.quiesced(999, progress(12, 12)), Err(Error::Stale));
    assert!(peer.guard.quiesced(4, progress(13, 12)).unwrap().is_none());
    assert!(
        peer.guard.quiesced(4, progress(11, 11)).is_err(),
        "fence predates known completed work"
    );
    assert!(peer.guard.quiesced(4, progress(13, 13)).unwrap().is_some());
    assert_eq!(peer.guard.mode(), WorkerMode::Standby);
}

#[test]
fn rejected_or_replayed_messages_do_not_renew_expired_lease_or_restore_authority() {
    let mut peer = Peer::new(3);
    let mut heartbeat = command(
        Binding {
            session: None,
            ..peer.guard.binding()
        },
        1,
        Command::Heartbeat {
            workspace_foreground: true,
        },
    );
    peer.accept(&heartbeat, 1);
    assert_eq!(peer.guard.on_command(&heartbeat, 10_000), Err(Error::Stale));
    heartbeat.sequence = 2;
    heartbeat.binding.worker.worker = worker_id(9);
    assert_eq!(peer.guard.on_command(&heartbeat, 14_000), Err(Error::Binding));
    let expiry = peer.guard.tick(15_001);
    assert!(expiry.restore_window && expiry.release_input);
    assert_eq!(peer.guard.mode(), WorkerMode::Detached);
    assert_eq!(peer.guard.tick(15_002), WorkerEffects::default());
    heartbeat.binding.worker.worker = worker_id(3);
    assert_eq!(peer.guard.on_command(&heartbeat, 15_003), Err(Error::Stale));
}

#[test]
fn late_command_detaches_and_session_changes_never_reuse_old_generation() {
    let mut peer = Peer::new(3);
    let old = peer.guard.binding();
    peer.guard.update_session(None).unwrap();
    assert_eq!(peer.guard.update_session(Some(session(4, 1))), Err(Error::Stale));
    assert!(peer.guard.update_session(Some(session(4, 2))).unwrap().release_input);
    assert_eq!(
        peer.guard.on_command(
            &command(
                old,
                1,
                Command::Prepare {
                    activation: 1,
                    viewport: None
                }
            ),
            0
        ),
        Err(Error::Binding)
    );
    let heartbeat = command(
        Binding {
            session: None,
            ..peer.guard.binding()
        },
        2,
        Command::Heartbeat {
            workspace_foreground: true,
        },
    );
    assert!(peer.guard.on_command(&heartbeat, LEASE_MILLIS).unwrap().effects.restore_window);
    assert_eq!(peer.guard.mode(), WorkerMode::Detached);
}

#[test]
fn handoff_revokes_and_fences_old_worker_then_requires_target_ready_and_explicit_grant() {
    let (mut alpha, mut beta) = (Peer::new(3), Peer::new(4));
    let mut host = coordinator(&mut [&mut alpha, &mut beta]);
    initial_activate(&mut host, &mut alpha);
    let revoke = message(host.activate(worker_id(4), None, 0).unwrap());
    assert!(matches!(revoke.message, Message::Command(Command::Revoke { .. })));
    alpha.accept(&revoke, 0);
    assert!(!alpha.guard.input_owned() && !beta.guard.input_owned());
    let idle = alpha.idle(&revoke, 11);
    let prepare = message(host.observe(&idle, 0).unwrap());
    assert_eq!(host.active(), None);
    beta.accept(&prepare, 0);
    let ready = beta.ready(&prepare, 11);
    let mut wrong = ready.clone();
    wrong.sequence = 1000;
    if let Message::Event(Event::FrameReady { activation, .. }) = &mut wrong.message {
        *activation += 1;
    }
    assert_eq!(host.observe(&wrong, 0), Err(Error::Stale));
    let grant = message(host.observe(&ready, 0).unwrap());
    assert!(!beta.guard.input_owned());
    let reply = beta.accept(&grant, 0).reply.unwrap();
    let ack = beta.envelope(reply);
    host.observe(&ack, 0).unwrap();
    assert_eq!(host.active(), Some(worker_id(4)));
    assert!(!alpha.guard.input_owned() && beta.guard.input_owned());
}

#[test]
fn uncertain_grant_rollback_first_revokes_target_and_cannot_reactivate_earlier_reply() {
    let (mut alpha, mut beta) = (Peer::new(3), Peer::new(4));
    let mut host = coordinator(&mut [&mut alpha, &mut beta]);
    initial_activate(&mut host, &mut alpha);
    let revoke = message(host.activate(worker_id(4), None, 0).unwrap());
    alpha.accept(&revoke, 0);
    let prepare = message(host.observe(&alpha.idle(&revoke, 11), 0).unwrap());
    beta.accept(&prepare, 0);
    let grant = message(host.observe(&beta.ready(&prepare, 11), 0).unwrap());
    let delayed_granted = beta.accept(&grant, 0).reply.unwrap();
    assert!(beta.guard.input_owned());
    let rollback_revoke = message(host.tick(12_000));
    assert!(matches!(rollback_revoke.message, Message::Command(Command::Revoke { .. })));
    assert!(beta.accept(&rollback_revoke, 12_000).effects.release_input);
    assert!(!alpha.guard.input_owned() && !beta.guard.input_owned());
    let late = beta.envelope(delayed_granted);
    assert_eq!(host.observe(&late, 12_000), Err(Error::Stale));
    let old_prepare = message(host.observe(&beta.idle(&rollback_revoke, 11), 12_000).unwrap());
    assert_eq!(old_prepare.binding.worker.worker, worker_id(3));
    alpha.accept(&old_prepare, 12_000);
    let old_grant = message(host.observe(&alpha.ready(&old_prepare, 12), 12_000).unwrap());
    let reply = alpha.accept(&old_grant, 12_000).reply.unwrap();
    assert_eq!(
        host.observe(&alpha.envelope(reply), 12_000).unwrap(),
        vec![Effect::Notice(HostNotice::RolledBack(worker_id(3)))]
    );
    assert_eq!(host.active(), Some(worker_id(3)));
    assert!(!beta.guard.input_owned());
}

#[test]
fn host_lease_expiry_late_ack_and_identity_change_fail_closed_without_new_grants() {
    let mut alpha = Peer::new(3);
    let mut host = coordinator(&mut [&mut alpha]);
    let prepare = message(host.activate(worker_id(3), None, 0).unwrap());
    alpha.accept(&prepare, 0);
    let ready = alpha.ready(&prepare, 11);
    let effects = host.observe(&ready, 12_000).unwrap();
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Send(Envelope {
            message: Message::Command(Command::Detach),
            ..
        })
    )));
    assert_eq!(host.active(), None);
    assert!(host.activate(worker_id(3), None, 12_000).is_err());

    let mut alpha = Peer::new(3);
    let mut host = coordinator(&mut [&mut alpha]);
    initial_activate(&mut host, &mut alpha);
    let mut changed = alpha.status();
    changed.binding.session = Some(session(4, 1));
    assert_eq!(host.observe(&changed, 1), Err(Error::Stale));
    changed.sequence += 1;
    changed.binding.session = Some(session(4, 2));
    assert!(host.observe(&changed, 2).unwrap().contains(&Effect::Notice(HostNotice::Failed)));
    assert_eq!(host.active(), None);

    let mut alpha = Peer::new(3);
    let mut host = coordinator(&mut [&mut alpha]);
    let expired = host.tick(LEASE_MILLIS);
    assert!(expired.contains(&Effect::Notice(HostNotice::Expired(worker_id(3)))));
    assert!(host.observe(&alpha.status(), LEASE_MILLIS).is_err());
}

#[test]
fn host_chrome_render_priority_never_fabricates_keyboard_focus_or_input_grants() {
    let mut alpha = Peer::new(3);
    let mut host = coordinator(&mut [&mut alpha]);
    initial_activate(&mut host, &mut alpha);
    host.set_workspace_foreground(true);
    let heartbeat = message(host.tick(1));
    alpha.accept(&heartbeat, 1);
    assert!(
        alpha.guard.render_priority(false),
        "host chat focus keeps active world's rendering priority"
    );
    host.set_workspace_foreground(false);
    let heartbeat = message(host.tick(1001));
    alpha.accept(&heartbeat, 1001);
    assert!(!alpha.guard.render_priority(false));
    assert!(alpha.guard.render_priority(true));
    assert!(alpha.guard.input_owned(), "ownership and native keyboard focus are separate");
}

#[test]
fn old_surface_or_pre_prepare_submission_cannot_satisfy_hosted_readiness() {
    let mut peer = Peer::new(3);
    let prepare = command(
        peer.guard.binding(),
        1,
        Command::Prepare {
            activation: 1,
            viewport: Some(HostedViewport {
                host_window: 42,
                rect: PhysicalRect {
                    x: -1000,
                    y: 0,
                    width: 1000,
                    height: 760,
                },
            }),
        },
    );
    peer.accept(&prepare, 0);
    assert!(
        peer.guard.frame_ready(1, frame(11)).unwrap().is_none(),
        "a boundary must be captured first"
    );
    let boundary = PrepareBoundary {
        device_epoch: 1,
        surface_epoch: 2,
        after_frame: 12,
    };
    peer.guard.prepare_boundary(1, boundary).unwrap();
    assert!(
        peer.guard.frame_ready(1, frame(13)).unwrap().is_none(),
        "old standalone surface cannot authorize hosted input"
    );
    assert!(
        peer.guard
            .frame_ready(
                1,
                FrameProof {
                    surface_epoch: 2,
                    ..frame(12)
                }
            )
            .unwrap()
            .is_none(),
        "prior queued work is below the submission boundary"
    );
    assert!(
        peer.guard
            .prepare_boundary(
                1,
                PrepareBoundary {
                    after_frame: 11,
                    ..boundary
                }
            )
            .is_err(),
        "a pending Prepare cannot be retargeted"
    );
    let ready = peer
        .guard
        .frame_ready(
            1,
            FrameProof {
                surface_epoch: 2,
                ..frame(13)
            },
        )
        .unwrap()
        .unwrap();
    assert!(matches!(ready.event, Event::FrameReady { boundary: observed, .. } if observed == boundary));
    assert!(!peer.guard.input_owned());
}

#[test]
fn uncorrelated_reply_neither_poisoned_sequence_nor_postponed_peer_expiry() {
    let mut peer = Peer::new(3);
    let mut host = coordinator(&mut [&mut peer]);
    let prepare = message(host.activate(worker_id(3), None, 0).unwrap());
    peer.accept(&prepare, 0);
    let mut wrong = peer.ready(&prepare, 11);
    wrong.sequence = 9000;
    if let Message::Event(Event::FrameReady { activation, .. }) = &mut wrong.message {
        *activation += 1;
    }
    assert_eq!(host.observe(&wrong, 11_000), Err(Error::Stale));
    assert!(
        host.tick(15_000).contains(&Effect::Notice(HostNotice::Expired(worker_id(3)))),
        "invalid reply cannot renew local receipt lease"
    );
}

#[test]
fn every_identity_domain_and_lease_epoch_are_checked_before_applying_a_command() {
    let mut peer = Peer::new(3);
    let original = command(
        peer.guard.binding(),
        1,
        Command::Prepare {
            activation: 1,
            viewport: None,
        },
    );
    for field in 0..8 {
        let mut changed = original.clone();
        match field {
            0 => changed.binding.worker.installation = InstallationId::from_bytes([9; 32]).unwrap(),
            1 => changed.binding.worker.host = HostId::from_bytes([9; 16]).unwrap(),
            2 => changed.binding.worker.worker = worker_id(9),
            3 => changed.binding.session.as_mut().unwrap().session = SessionId::from_bytes([9; 16]).unwrap(),
            4 => changed.binding.session.as_mut().unwrap().account = AccountKey::from_bytes([9; 32]).unwrap(),
            5 => changed.binding.session.as_mut().unwrap().grid = GridKey::from_bytes([9; 32]).unwrap(),
            6 => changed.binding.session.as_mut().unwrap().generation = 9,
            _ => changed.lease_epoch = 9,
        }
        assert_eq!(peer.guard.on_command(&changed, 1000), Err(Error::Binding));
        assert_eq!(peer.guard.mode(), WorkerMode::Standby);
    }
    assert!(
        peer.accept(&original, 1000).effects.prepare,
        "rejected bindings cannot consume the legitimate sequence"
    );
}

#[test]
fn recreation_invalidates_prepared_grant_and_viewport_cannot_change_during_promotion() {
    let mut peer = Peer::new(3);
    let prepare = command(
        peer.guard.binding(),
        1,
        Command::Prepare {
            activation: 1,
            viewport: None,
        },
    );
    peer.accept(&prepare, 0);
    let _ready = peer.ready(&prepare, 11);
    let change = command(
        peer.guard.binding(),
        2,
        Command::SetViewport {
            activation: 1,
            viewport: None,
        },
    );
    assert!(matches!(peer.accept(&change, 0).reply.unwrap().event, Event::Rejected { .. }));
    assert!(peer.guard.invalidate_display().release_input);
    assert_eq!(peer.guard.mode(), WorkerMode::Standby);
    let grant = command(peer.guard.binding(), 3, Command::Grant { activation: 1 });
    assert!(matches!(peer.accept(&grant, 0).reply.unwrap().event, Event::Rejected { .. }));
    assert!(!peer.guard.input_owned());
}

#[test]
fn parking_requires_exact_quiescence_before_authorizing_a_new_login() {
    let mut alpha = Peer::new(3);
    let mut host = coordinator(&mut [&mut alpha]);
    assert!(host.park_active(0).unwrap().is_empty());
    initial_activate(&mut host, &mut alpha);
    let revoke = message(host.park_active(0).unwrap());
    assert_eq!(host.active(), Some(worker_id(3)));
    assert!(host.busy());
    assert_eq!(host.park_active(0), Err(Error::Busy));
    assert!(alpha.accept(&revoke, 0).effects.release_input);
    assert!(alpha.guard.quiesced(revoke.sequence, progress(12, 11)).unwrap().is_none());
    let mut premature = event(
        alpha.guard.binding(),
        9000,
        revoke.sequence,
        Event::Quiesced {
            activation: alpha.guard.activation(),
            progress: progress(12, 11),
        },
    );
    assert_eq!(host.observe(&premature, 0), Err(Error::Stale));
    premature.message = Message::Event(Event::Quiesced {
        activation: alpha.guard.activation() + 1,
        progress: progress(12, 12),
    });
    assert_eq!(host.observe(&premature, 0), Err(Error::Stale));
    let idle = alpha.idle(&revoke, 12);
    assert_eq!(
        host.observe(&idle, 0).unwrap(),
        vec![Effect::Notice(HostNotice::Parked(worker_id(3)))]
    );
    assert_eq!(host.active(), None);
    assert!(!host.busy());
    assert!(!alpha.guard.input_owned());
    assert!(host.park_active(0).unwrap().is_empty());
}

#[test]
fn parking_failure_never_emits_parked_and_standby_detachment_does_not_promote_anyone() {
    let (mut alpha, mut beta) = (Peer::new(3), Peer::new(4));
    let mut host = coordinator(&mut [&mut alpha, &mut beta]);
    initial_activate(&mut host, &mut alpha);
    let detach = message(host.detach(worker_id(4)).unwrap());
    assert_eq!(detach.binding.worker.worker, worker_id(4));
    assert!(matches!(detach.message, Message::Command(Command::Detach)));
    assert_eq!(host.active(), Some(worker_id(3)));
    assert!(host.detach(worker_id(4)).unwrap().is_empty());
    let _revoke = host.park_active(0).unwrap();
    let timeout = host.tick(12_000);
    assert!(timeout.contains(&Effect::Notice(HostNotice::Failed)));
    assert!(!timeout.iter().any(|effect| matches!(effect, Effect::Notice(HostNotice::Parked(_)))));
    assert_eq!(host.active(), None);
    assert!(!host.busy());
    assert!(host.workers().all(|entry| !entry.live));

    let mut alpha = Peer::new(3);
    let mut host = coordinator(&mut [&mut alpha]);
    initial_activate(&mut host, &mut alpha);
    let detached = host.detach(worker_id(3)).unwrap();
    assert!(detached.contains(&Effect::Notice(HostNotice::Detached)));
    assert_eq!(host.active(), None);
}

#[test]
fn active_viewport_change_repeats_fence_prepare_and_grant_on_the_same_worker() {
    let mut alpha = Peer::new(3);
    let mut host = coordinator(&mut [&mut alpha]);
    initial_activate(&mut host, &mut alpha);
    assert!(
        host.activate(worker_id(3), None, 0).unwrap().is_empty(),
        "unchanged geometry is idempotent"
    );
    let viewport = Some(HostedViewport {
        host_window: 42,
        rect: PhysicalRect {
            x: -1200,
            y: 50,
            width: 1200,
            height: 800,
        },
    });
    let heartbeat = message(host.tick(1));
    let bypass = command(
        alpha.guard.binding(),
        heartbeat.sequence,
        Command::SetViewport {
            activation: alpha.guard.activation(),
            viewport,
        },
    );
    assert!(matches!(alpha.accept(&bypass, 1).reply.unwrap().event, Event::Rejected { .. }));
    assert_eq!(alpha.guard.viewport(), None);
    assert!(alpha.guard.input_owned());
    let revoke = message(host.activate(worker_id(3), viewport, 1).unwrap());
    assert!(matches!(revoke.message, Message::Command(Command::Revoke { .. })));
    alpha.accept(&revoke, 1);
    assert!(!alpha.guard.input_owned());
    let prepare = message(host.observe(&alpha.idle(&revoke, 11), 1).unwrap());
    assert!(matches!(prepare.message, Message::Command(Command::Prepare { viewport: observed, .. }) if observed == viewport));
    assert_eq!(host.active(), None);
    alpha.accept(&prepare, 1);
    let grant = message(host.observe(&alpha.ready(&prepare, 12), 1).unwrap());
    assert!(!alpha.guard.input_owned());
    let reply = alpha.accept(&grant, 1).reply.unwrap();
    host.observe(&alpha.envelope(reply), 1).unwrap();
    assert_eq!(host.active(), Some(worker_id(3)));
    assert_eq!(alpha.guard.viewport(), viewport);
    assert!(alpha.guard.input_owned());
    assert!(!host.busy());
}

#[test]
fn preview_admission_reserves_pending_rates_and_never_renders_a_promoted_or_stale_source() {
    let demand = PreviewDemand {
        requested: Some(PreviewRate::Five),
        visible: true,
        inactive: true,
        session_current: true,
        ..Default::default()
    };
    assert_eq!(admit_previews(&[demand; 4], 20).unwrap(), vec![Some(PreviewRate::Two); 4]);
    assert_eq!(PreviewRate::Half.interval_millis(), 2000);
    assert_eq!(PreviewRate::Five.interval_millis(), 200);
    assert_eq!(PreviewSize::Large.bytes(), 640 * 360 * 4);
    let current = PreviewDemand {
        acknowledged_half_fps: 10,
        ..demand
    };
    let opening = PreviewDemand {
        pending_half_fps: 10,
        ..demand
    };
    let rates = admit_previews(&[current, opening, demand], 20).unwrap();
    assert_eq!(
        rates,
        vec![Some(PreviewRate::Two), Some(PreviewRate::Two), None],
        "lowering is not freed until acknowledged"
    );
    assert_eq!(
        admit_previews(
            &[
                PreviewDemand { inactive: false, ..demand },
                PreviewDemand {
                    session_current: false,
                    ..demand
                }
            ],
            20
        )
        .unwrap(),
        vec![None, None]
    );
    assert!(admit_previews(&[demand; 5], 20).is_err());
    assert_eq!(admit_previews(&[demand], 0).unwrap(), vec![None]);
}
