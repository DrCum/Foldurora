use super::*;
use crate::show_viewer::{ShowViewerLedger, WindowObservation};
use crate::{AccountKey, GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerGuard, WorkerId, WorkerMode};

pub(crate) fn binding() -> Binding {
    Binding {
        worker: WorkerIdentity {
            installation: InstallationId::from_bytes([1; 32]).unwrap(),
            host: HostId::from_bytes([2; 16]).unwrap(),
            worker: WorkerId::from_bytes([3; 16]).unwrap(),
        },
        session: None,
    }
}
pub(crate) fn session() -> SessionIdentity {
    SessionIdentity {
        session: SessionId::from_bytes([4; 16]).unwrap(),
        account: AccountKey::from_bytes([5; 32]).unwrap(),
        grid: GridKey::from_bytes([6; 32]).unwrap(),
        generation: 1,
    }
}
pub(crate) fn request(id: u64) -> ShowViewerRequest {
    ShowViewerRequest {
        target: ShowViewerTarget {
            binding: binding(),
            lease_epoch: 7,
            window: WindowObservation {
                app: 1,
                window: 2,
                ordinary: 3,
            },
        },
        request: id,
    }
}
pub(crate) fn command(sequence: u64, body: ControlV2Command) -> ControlV2Envelope {
    ControlV2Envelope {
        version: CONTROL_V2_VERSION,
        binding: binding(),
        lease_epoch: 7,
        sequence,
        reply_to: 0,
        message: ControlV2Message::Command(body),
    }
}
pub(crate) fn event(sequence: u64, reply_to: u64, body: ControlV2Event) -> ControlV2Envelope {
    ControlV2Envelope {
        version: CONTROL_V2_VERSION,
        binding: binding(),
        lease_epoch: 7,
        sequence,
        reply_to,
        message: ControlV2Message::Event(body),
    }
}

#[test]
fn control_v2_closed_codecs_keep_v1_separate_and_validate_every_nested_show_scope() {
    let original = command(1, ControlV2Command::ShowViewer(request(1)));
    let bytes = encode_control(&original).unwrap();
    assert_eq!(decode_control(&bytes), Ok(original.clone()));
    assert!(crate::decode(&bytes).is_err());
    let v1 = Envelope {
        version: PROTOCOL_VERSION,
        binding: binding(),
        lease_epoch: 7,
        sequence: 1,
        reply_to: 0,
        message: Message::Command(Command::Heartbeat {
            workspace_foreground: false,
        }),
    };
    assert_eq!(decode_control(&crate::encode(&v1).unwrap()), Err(Error::Version));
    let mut wrong = original.clone();
    wrong.version = PROTOCOL_VERSION;
    assert_eq!(encode_control(&wrong), Err(Error::Version));
    for path in [
        vec![],
        vec!["message"],
        vec!["message", "body"],
        vec!["message", "body", "data"],
        vec!["message", "body", "data", "target"],
        vec!["message", "body", "data", "target", "window"],
    ] {
        let mut value = serde_json::to_value(&original).unwrap();
        let mut at = &mut value;
        for key in path {
            at = &mut at[key];
        }
        at["arbitrary_handle"] = 99.into();
        assert_eq!(decode_control(&serde_json::to_vec(&value).unwrap()), Err(Error::Malformed));
    }
    for field in 0..7 {
        let mut wrong = original.clone();
        let ControlV2Message::Command(ControlV2Command::ShowViewer(request)) = &mut wrong.message else {
            panic!()
        };
        match field {
            0 => request.target.binding.session = Some(session()),
            1 => request.target.binding.worker.worker = WorkerId::from_bytes([8; 16]).unwrap(),
            2 => request.target.lease_epoch += 1,
            3 => request.target.window.app = 0,
            4 => request.target.window.window = 0,
            5 => request.target.window.ordinary = 0,
            _ => request.request = 0,
        }
        assert!(encode_control(&wrong).is_err());
    }
    let mut with_session = request(2);
    with_session.target.binding.session = Some(session());
    let mut exact = command(2, ControlV2Command::QueryShowViewer(with_session));
    exact.binding = with_session.target.binding;
    assert_eq!(decode_control(&encode_control(&exact).unwrap()), Ok(exact));
    assert_eq!(decode_control(&vec![b' '; MAX_CONTROL_V2_BYTES + 1]), Err(Error::Oversized));
    let unknown = String::from_utf8(bytes).unwrap().replace("show_viewer", "send_arbitrary_input");
    assert_eq!(decode_control(unknown.as_bytes()), Err(Error::Malformed));
}

#[test]
fn control_v2_all_six_normal_commands_share_existing_validation_and_reply_kinds() {
    let source = Binding {
        session: Some(session()),
        ..binding()
    };
    for body in [
        ControlV2Command::Heartbeat {
            workspace_foreground: true,
        },
        ControlV2Command::Revoke { activation: 1 },
        ControlV2Command::Prepare {
            activation: 1,
            viewport: None,
        },
        ControlV2Command::Grant { activation: 1 },
        ControlV2Command::SetViewport {
            activation: 1,
            viewport: None,
        },
        ControlV2Command::Detach,
    ] {
        let mut envelope = command(1, body);
        envelope.binding = source;
        assert_eq!(decode_control(&encode_control(&envelope).unwrap()), Ok(envelope.clone()));
        assert_eq!(
            ControlV2Envelope::from_control(envelope.control_envelope(Message::Command(match &envelope.message {
                ControlV2Message::Command(command) => command.control().unwrap(),
                _ => panic!(),
            }))),
            envelope
        );
    }
    let mut prepare = command(
        1,
        ControlV2Command::Prepare {
            activation: 1,
            viewport: None,
        },
    );
    assert_eq!(encode_control(&prepare), Err(Error::Binding));
    prepare.binding = source;
    let mut wrong = prepare.clone();
    wrong.message = ControlV2Message::Command(ControlV2Command::Prepare {
        activation: 0,
        viewport: None,
    });
    assert_eq!(encode_control(&wrong), Err(Error::Binding));
    let mut ready = event(
        2,
        1,
        ControlV2Event::FrameReady {
            activation: 1,
            boundary: PrepareBoundary {
                device_epoch: 1,
                surface_epoch: 1,
                after_frame: 10,
            },
            frame: FrameProof {
                device_epoch: 1,
                surface_epoch: 1,
                frame: 11,
            },
        },
    );
    ready.binding = source;
    assert!(matches_reply(&prepare, &ready));
    ready.binding.session = Some(SessionIdentity {
        generation: 2,
        ..session()
    });
    assert!(!matches_reply(&prepare, &ready));
}

#[test]
fn control_v2_reply_and_ingress_ack_lanes_are_closed_bounded_and_cannot_cross_decode() {
    let receipt = ShowViewerReceipt {
        request: request(1),
        outcome: ShowViewerOutcome::EffectUncertain,
    };
    let reply = event(11, 1, ControlV2Event::ShowViewerReceipt(receipt));
    let ack = ControlV2Ack {
        version: CONTROL_V2_VERSION,
        reply: ReplyAckV2 {
            binding: binding(),
            lease_epoch: 7,
            event_sequence: 11,
            reply_to: 1,
        },
    };
    let ingress = ControlIngressAckV2 {
        version: CONTROL_V2_VERSION,
        binding: binding(),
        lease_epoch: 7,
        command_sequence: 1,
        command_digest: [9; 32],
    };
    let reply_bytes = encode_reply_ack(&ack).unwrap();
    let ingress_bytes = encode_ingress_ack(&ingress).unwrap();
    assert_eq!(decode_reply_ack(&reply_bytes), Ok(ack));
    assert_eq!(decode_ingress_ack(&ingress_bytes), Ok(ingress));
    assert!(reply_bytes.len() <= MAX_CONTROL_ACK_BYTES && ingress_bytes.len() <= MAX_CONTROL_ACK_BYTES);
    assert!(decode_reply_ack(&ingress_bytes).is_err());
    assert!(decode_ingress_ack(&reply_bytes).is_err());
    assert!(decode_control(&reply_bytes).is_err());
    assert!(decode_control(&ingress_bytes).is_err());
    assert!(decode_ingress_ack(&encode_control(&reply).unwrap()).is_err());
    assert_eq!(decode_ingress_ack(&vec![b' '; MAX_CONTROL_ACK_BYTES + 1]), Err(Error::Oversized));
    assert_eq!(decode_reply_ack(&vec![b' '; MAX_CONTROL_ACK_BYTES + 1]), Err(Error::Oversized));
    let mut unknown = serde_json::to_value(ingress).unwrap();
    unknown["received_at"] = 123.into();
    assert_eq!(decode_ingress_ack(&serde_json::to_vec(&unknown).unwrap()), Err(Error::Malformed));
    let mut bad_digest = serde_json::to_value(ingress).unwrap();
    bad_digest["command_digest"] = serde_json::json!([9]);
    assert_eq!(decode_ingress_ack(&serde_json::to_vec(&bad_digest).unwrap()), Err(Error::Malformed));
    assert_eq!(encode_reply_ack(&ControlV2Ack { version: 1, ..ack }), Err(Error::Version));
    assert_eq!(
        encode_ingress_ack(&ControlIngressAckV2 {
            command_sequence: 0,
            ..ingress
        }),
        Err(Error::Sequence)
    );
}

#[test]
fn control_v2_ack_matchers_require_original_full_scope_digest_and_current_publication_head() {
    let command = command(1, ControlV2Command::ShowViewer(request(1)));
    let reply = event(
        11,
        1,
        ControlV2Event::ShowViewerReceipt(ShowViewerReceipt {
            request: request(1),
            outcome: ShowViewerOutcome::Cancelled,
        }),
    );
    let ack = ControlV2Ack {
        version: 2,
        reply: ReplyAckV2 {
            binding: binding(),
            lease_epoch: 7,
            event_sequence: 11,
            reply_to: 1,
        },
    };
    let ingress = ControlIngressAckV2 {
        version: 2,
        binding: binding(),
        lease_epoch: 7,
        command_sequence: 1,
        command_digest: [9; 32],
    };
    assert!(ack.matches_published(&reply, binding().worker, 7));
    assert!(ingress.matches_ingress(&command, [9; 32], binding().worker, 7));
    for field in 0..8 {
        let mut bad_ack = ack;
        let mut bad_ingress = ingress;
        match field {
            0 => {
                bad_ack.reply.binding.session = Some(session());
                bad_ingress.binding.session = Some(session());
            }
            1 => {
                bad_ack.reply.binding.worker.worker = WorkerId::from_bytes([8; 16]).unwrap();
                bad_ingress.binding.worker = bad_ack.reply.binding.worker;
            }
            2 => {
                bad_ack.reply.binding.worker.host = HostId::from_bytes([8; 16]).unwrap();
                bad_ingress.binding.worker = bad_ack.reply.binding.worker;
            }
            3 => {
                bad_ack.reply.binding.worker.installation = InstallationId::from_bytes([8; 32]).unwrap();
                bad_ingress.binding.worker = bad_ack.reply.binding.worker;
            }
            4 => {
                bad_ack.reply.lease_epoch += 1;
                bad_ingress.lease_epoch += 1;
            }
            5 => {
                bad_ack.reply.event_sequence += 1;
                bad_ingress.command_sequence += 1;
            }
            6 => {
                bad_ack.reply.reply_to += 1;
                bad_ingress.command_digest[0] ^= 1;
            }
            _ => {
                bad_ack.version = 1;
                bad_ingress.version = 1;
            }
        }
        assert!(!bad_ack.matches_published(&reply, binding().worker, 7));
        assert!(!bad_ingress.matches_ingress(&command, [9; 32], binding().worker, 7));
    }
    let mut next_command = command.clone();
    next_command.sequence += 1;
    let mut next_reply = reply.clone();
    next_reply.sequence += 1;
    next_reply.reply_to += 1;
    assert!(!ingress.matches_ingress(&next_command, [9; 32], binding().worker, 7));
    assert!(!ack.matches_published(&next_reply, binding().worker, 7));
    assert!(!ingress.matches_ingress(&command, [8; 32], binding().worker, 7));
    assert!(!ack.matches_published(&reply, binding().worker, 8));
    assert!(!ingress.matches_ingress(&command, [9; 32], session_worker(), 7));
    let status = event(
        50,
        0,
        ControlV2Event::Status {
            status: WorkerStatus::default(),
            ordinary: None,
        },
    );
    assert!(!ack.matches_published(&status, binding().worker, 7));
    assert!(!ingress.matches_ingress(&status, [9; 32], binding().worker, 7));
    // A solicited heartbeat Status IS a retained control reply; only the
    // separate unsolicited latest status (reply_to0) is never retireable.
    let solicited = event(
        11,
        1,
        ControlV2Event::Status {
            status: WorkerStatus::default(),
            ordinary: None,
        },
    );
    assert!(ack.matches_published(&solicited, binding().worker, 7));
}
fn session_worker() -> WorkerIdentity {
    WorkerIdentity {
        worker: WorkerId::from_bytes([8; 16]).unwrap(),
        ..binding().worker
    }
}

#[test]
fn control_v2_show_query_results_are_exact_and_never_acknowledge_native_activation() {
    let request = request(1);
    let show = command(1, ControlV2Command::ShowViewer(request));
    let query = command(2, ControlV2Command::QueryShowViewer(request));
    for result in [
        ShowViewerQueryState::Unknown,
        ShowViewerQueryState::Pending,
        ShowViewerQueryState::Rejected,
        ShowViewerQueryState::Terminal(ShowViewerReceipt {
            request,
            outcome: ShowViewerOutcome::EffectUncertain,
        }),
    ] {
        let reply = event(3, 2, ControlV2Event::ShowViewerQuery(ShowViewerQueryResult { request, result }));
        assert_eq!(decode_control(&encode_control(&reply).unwrap()), Ok(reply.clone()));
        assert!(matches_reply(&query, &reply));
        assert!(!matches_reply(&show, &reply));
    }
    let receipt = event(
        4,
        1,
        ControlV2Event::ShowViewerReceipt(ShowViewerReceipt {
            request,
            outcome: ShowViewerOutcome::ShownLocally,
        }),
    );
    assert!(matches_reply(&show, &receipt));
    let mut grant = command(1, ControlV2Command::Grant { activation: 1 });
    grant.binding.session = Some(session());
    assert!(!matches_reply(&grant, &receipt));
    let mut forged = request;
    forged.target.window.ordinary += 1;
    let bad = event(
        4,
        2,
        ControlV2Event::ShowViewerQuery(ShowViewerQueryResult {
            request,
            result: ShowViewerQueryState::Terminal(ShowViewerReceipt {
                request: forged,
                outcome: ShowViewerOutcome::ShownLocally,
            }),
        }),
    );
    assert_eq!(encode_control(&bad), Err(Error::Binding));
}

#[test]
fn control_v2_guard_original_stamp_rejects_future_regression_and_does_not_renew_at_dequeue() {
    let mut guard = WorkerGuard::new(binding().worker, 7, 0).unwrap();
    let show = command(1, ControlV2Command::ShowViewer(request(1)));
    assert_eq!(guard.on_control_v2(&show, 101, 100), Err(Error::Stale));
    let accepted = guard.on_control_v2(&show, 50, 100).unwrap();
    assert_eq!(accepted.effects, WorkerEffects::default());
    assert!(accepted.reply.is_none());
    assert_eq!(
        accepted.work,
        Some(ControlV2Work::ShowViewer {
            request: request(1),
            outcome: None
        })
    );
    let query = command(2, ControlV2Command::QueryShowViewer(request(1)));
    assert_eq!(guard.on_control_v2(&query, 49, 101), Err(Error::Stale));
    assert_eq!(
        guard.on_control_v2(&query, 51, 101).unwrap().work,
        Some(ControlV2Work::QueryShowViewer(request(1)))
    );
    assert_eq!(guard.mode(), WorkerMode::Standby);
    assert_eq!(guard.activation(), 0);
    assert_eq!(guard.viewport(), None);
    assert!(!guard.input_owned() && !guard.render_owned());
    assert_eq!(guard.tick(15_050), WorkerEffects::default());
    assert!(guard.tick(15_051).restore_window, "lease uses original51, not dequeue101");
    assert_eq!(guard.mode(), WorkerMode::Detached);
}

#[test]
fn control_v2_guard_exact_none_scope_and_expired_show_consume_shared_sequence_without_effect() {
    let mut guard = WorkerGuard::new(binding().worker, 7, 0).unwrap();
    let expired = command(1, ControlV2Command::ShowViewer(request(1)));
    let accepted = guard.on_control_v2(&expired, 10, 2010).unwrap();
    assert_eq!(
        accepted.work,
        Some(ControlV2Work::ShowViewer {
            request: request(1),
            outcome: Some(ShowViewerOutcome::Expired)
        })
    );
    assert_eq!(accepted.effects, WorkerEffects::default());
    assert_eq!(guard.on_control_v2(&expired, 11, 2011), Err(Error::Stale));
    guard.update_session(Some(session())).unwrap();
    let wrong_none = command(2, ControlV2Command::ShowViewer(request(2)));
    assert_eq!(guard.on_control_v2(&wrong_none, 2012, 2012), Err(Error::Binding));
    let heartbeat = Envelope {
        version: 1,
        binding: binding(),
        lease_epoch: 7,
        sequence: 2,
        reply_to: 0,
        message: Message::Command(Command::Heartbeat {
            workspace_foreground: true,
        }),
    };
    guard.on_command(&heartbeat, 2012).unwrap(); // Existing V1 None permissiveness is unchanged.
    let mut exact = request(3);
    exact.target.binding = guard.binding();
    let mut show = command(3, ControlV2Command::ShowViewer(exact));
    show.binding = exact.target.binding;
    assert_eq!(guard.on_control_v2(&show, 2013, 2013).unwrap().effects, WorkerEffects::default());
    let mut replay = heartbeat;
    replay.sequence = 3;
    assert_eq!(guard.on_command(&replay, 2014), Err(Error::Stale));
}

#[test]
fn control_v2_guard_lease_expiry_and_maximum_sequence_never_reopen_or_grant() {
    let mut guard = WorkerGuard::new(binding().worker, 7, 0).unwrap();
    let last = command(u64::MAX, ControlV2Command::ShowViewer(request(u64::MAX)));
    assert!(guard.on_control_v2(&last, 1, 1).unwrap().work.is_some());
    assert_eq!(guard.on_control_v2(&last, 2, 2), Err(Error::Stale));
    let wrapped = command(0, ControlV2Command::ShowViewer(request(1)));
    assert_eq!(guard.on_control_v2(&wrapped, 2, 2), Err(Error::Sequence));
    assert!(!guard.input_owned() && !guard.render_owned());
    let mut late = WorkerGuard::new(binding().worker, 7, 0).unwrap();
    let detached = late
        .on_control_v2(&command(1, ControlV2Command::ShowViewer(request(1))), 1, crate::LEASE_MILLIS)
        .unwrap();
    assert!(detached.effects.restore_window && detached.effects.release_input);
    assert!(detached.work.is_none() && detached.reply.is_none());
    assert_eq!(late.mode(), WorkerMode::Detached);
}

#[test]
fn control_v2_show_in_preparing_active_and_revoking_never_borrows_normal_control_authority() {
    let mut guard = WorkerGuard::new(binding().worker, 7, 0).unwrap();
    guard.update_session(Some(session())).unwrap();
    let mut prepare = command(
        1,
        ControlV2Command::Prepare {
            activation: 1,
            viewport: None,
        },
    );
    prepare.binding = guard.binding();
    assert!(guard.on_control_v2(&prepare, 1, 1).unwrap().effects.prepare);
    let mut original = request(1);
    original.target.binding = guard.binding();
    let mut show = command(2, ControlV2Command::ShowViewer(original));
    show.binding = guard.binding();
    for (sequence, mode) in [(2, WorkerMode::Preparing), (4, WorkerMode::Active), (6, WorkerMode::Revoking)] {
        show.sequence = sequence;
        let result = guard.on_control_v2(&show, sequence, sequence).unwrap();
        assert_eq!(
            result.work,
            Some(ControlV2Work::ShowViewer {
                request: original,
                outcome: Some(ShowViewerOutcome::RejectedNative)
            })
        );
        assert_eq!(result.effects, WorkerEffects::default());
        assert!(result.reply.is_none());
        assert_eq!(guard.mode(), mode);
        assert_eq!(guard.activation(), 1);
        if sequence == 2 {
            guard
                .prepare_boundary(
                    1,
                    PrepareBoundary {
                        device_epoch: 1,
                        surface_epoch: 1,
                        after_frame: 10,
                    },
                )
                .unwrap();
            guard
                .frame_ready(
                    1,
                    FrameProof {
                        device_epoch: 1,
                        surface_epoch: 1,
                        frame: 11,
                    },
                )
                .unwrap()
                .unwrap();
            let mut grant = command(3, ControlV2Command::Grant { activation: 1 });
            grant.binding = guard.binding();
            assert!(guard.on_control_v2(&grant, 3, 3).unwrap().effects.grant_input);
        } else if sequence == 4 {
            assert!(guard.input_owned() && guard.render_owned());
            let mut revoke = command(5, ControlV2Command::Revoke { activation: 1 });
            revoke.binding = guard.binding();
            assert!(guard.on_control_v2(&revoke, 5, 5).unwrap().effects.release_input);
        }
    }
    assert!(!guard.input_owned() && !guard.render_owned());
}

#[test]
fn control_v2_ledger_query_distinguishes_unknown_pending_and_immutable_cancelled_or_uncertain() {
    let mut ledger = ShowViewerLedger::new(binding().worker, 7).unwrap();
    for (id, outcome) in [(1, ShowViewerOutcome::Cancelled), (2, ShowViewerOutcome::EffectUncertain)] {
        let original = request(id);
        assert_eq!(ledger.query(original), Ok(ShowViewerLookup::Unknown));
        assert_eq!(ledger.retained_count(), (id - 1) as usize);
        assert_eq!(ledger.reserve(original), Ok(None));
        assert_eq!(ledger.query(original), Ok(ShowViewerLookup::Pending));
        let receipt = ledger.finish(original, outcome).unwrap();
        for _ in 0..3 {
            assert_eq!(ledger.query(original), Ok(ShowViewerLookup::Terminal(receipt)));
            assert_eq!(ledger.reserve(original), Ok(Some(receipt)));
            assert_eq!(ledger.finish(original, ShowViewerOutcome::ShownLocally), Err(Error::Stale));
        }
        let mut changed = original;
        changed.target.window.ordinary += 1;
        assert_eq!(ledger.query(changed), Err(Error::Invalid));
    }
    assert_eq!(ledger.retained_count(), 2);
    assert_eq!(ledger.query(request(3)), Ok(ShowViewerLookup::Unknown));
}

#[test]
fn control_v2_startup_clock_is_one_shot_without_retiming_old_ingress_or_renewing_native_receipt() {
    let mut guard = WorkerGuard::new(binding().worker, 7, 0).unwrap();
    guard.initialize_startup_control_v2(V2StartupClock::at_app_ready(20_000)).unwrap();
    let old = command(1, ControlV2Command::ShowViewer(request(1)));
    assert_eq!(guard.on_control_v2(&old, 1, 20_001), Err(Error::Stale));
    assert_eq!(guard.mode(), WorkerMode::Standby);
    assert!(!guard.input_owned() && !guard.render_owned());
    let fresh = command(2, ControlV2Command::ShowViewer(request(2)));
    assert_eq!(
        guard.on_control_v2(&fresh, 20_002, 20_003).unwrap().work,
        Some(ControlV2Work::ShowViewer {
            request: request(2),
            outcome: None
        })
    );
    assert_eq!(
        guard.initialize_startup_control_v2(V2StartupClock::at_app_ready(30_000)),
        Err(Error::Stale)
    );
    assert!(guard.tick(35_002).restore_window);
    assert_eq!(
        guard.initialize_startup_control_v2(V2StartupClock::at_app_ready(40_000)),
        Err(Error::Stale)
    );
    assert_eq!(guard.mode(), WorkerMode::Detached);
    let value = serde_json::json!({"version":2,"binding":binding(),"lease_epoch":7,"sequence":1,"reply_to":0,
        "message":{"direction":"command","body":{"type":"initialize_startup_clock","data":20_000}}});
    assert_eq!(decode_control(&serde_json::to_vec(&value).unwrap()), Err(Error::Malformed));
}
#[test]
fn control_v2_startup_clock_refuses_every_prior_admission_effect_regression_or_consumed_attempt() {
    for scenario in 0..8 {
        let mut guard = WorkerGuard::new(binding().worker, 7, 0).unwrap();
        match scenario {
            0 => {
                let normal = Envelope {
                    version: 1,
                    binding: binding(),
                    lease_epoch: 7,
                    sequence: 1,
                    reply_to: 0,
                    message: Message::Command(Command::Heartbeat {
                        workspace_foreground: false,
                    }),
                };
                guard.on_command(&normal, 1).unwrap();
            }
            1 => {
                guard
                    .on_control_v2(
                        &command(
                            1,
                            ControlV2Command::Heartbeat {
                                workspace_foreground: false,
                            },
                        ),
                        1,
                        1,
                    )
                    .unwrap();
            }
            2 => {
                guard.invalidate_display();
            }
            3 => {
                guard.update_session(Some(session())).unwrap();
                guard.update_session(None).unwrap();
            }
            4 => {
                guard.tick(crate::LEASE_MILLIS);
            }
            5 => {
                guard.initialize_startup_control_v2(V2StartupClock::at_app_ready(10)).unwrap();
                guard.tick(9);
            }
            6 => {
                assert_eq!(
                    guard.on_control_v2(&command(1, ControlV2Command::ShowViewer(request(1))), 10, 9),
                    Err(Error::Stale)
                );
            }
            _ => {
                guard
                    .frame_ready(
                        0,
                        FrameProof {
                            device_epoch: 1,
                            surface_epoch: 1,
                            frame: 1,
                        },
                    )
                    .unwrap();
            }
        }
        assert_eq!(
            guard.initialize_startup_control_v2(V2StartupClock::at_app_ready(30_000)),
            Err(Error::Stale)
        );
        assert_eq!(
            guard.initialize_startup_control_v2(V2StartupClock::at_app_ready(40_000)),
            Err(Error::Stale)
        );
    }
    let mut guard = WorkerGuard::new(binding().worker, 7, 10).unwrap();
    assert_eq!(
        guard.initialize_startup_control_v2(V2StartupClock::at_app_ready(9)),
        Err(Error::Stale)
    );
    assert_eq!(
        guard.initialize_startup_control_v2(V2StartupClock::at_app_ready(11)),
        Err(Error::Stale)
    );
}
#[test]
fn control_v2_unavailable_is_exact_transport_retirement_without_existing_control_or_ledger_success() {
    for original in [
        command(
            1,
            ControlV2Command::Heartbeat {
                workspace_foreground: false,
            },
        ),
        command(2, ControlV2Command::Detach),
        command(3, ControlV2Command::ShowViewer(request(1))),
        command(4, ControlV2Command::QueryShowViewer(request(1))),
    ] {
        let ControlV2Message::Command(captured) = &original.message else {
            panic!()
        };
        let unavailable = event(
            10,
            original.sequence,
            ControlV2Event::ControlUnavailable { command: captured.clone() },
        );
        assert_eq!(decode_control(&encode_control(&unavailable).unwrap()), Ok(unavailable.clone()));
        assert!(matches_reply(&original, &unavailable));
        assert_eq!(unavailable.existing_control(), Ok(None));
        let mut wrong = unavailable.clone();
        wrong.reply_to += 1;
        assert!(!matches_reply(&original, &wrong));
        wrong = unavailable;
        wrong.binding.session = Some(session());
        assert!(!matches_reply(&original, &wrong));
    }
    let mut ledger = ShowViewerLedger::new(binding().worker, 7).unwrap();
    ledger.reserve(request(1)).unwrap();
    ledger.reserve(request(2)).unwrap();
    assert_eq!(ledger.pending_requests().collect::<Vec<_>>(), vec![request(1), request(2)]);
    ledger.finish(request(1), ShowViewerOutcome::Cancelled).unwrap();
    assert_eq!(ledger.pending_requests().collect::<Vec<_>>(), vec![request(2)]);
}
