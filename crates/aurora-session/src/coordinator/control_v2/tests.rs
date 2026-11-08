use super::*;
use crate::control_v2::{
    ControlV2Reply, ControlV2Work, ShowViewerQueryResult,
    tests::{binding, command, event, request, session},
};
use crate::show_viewer::ShowViewerOutcome;
use crate::{GpuProgress, PrepareBoundary, WorkerEffects, WorkerGuard, WorkerMode};

fn ordinary_host() -> Coordinator {
    let source = binding();
    let mut host = Coordinator::new(source.worker.installation, source.worker.host);
    host.register(source.worker.worker, 7, 0).unwrap();
    host.observe_control_v2(
        &event(
            1,
            0,
            ControlV2Event::Status {
                // Current ordinary Login has status.blocked=true in the actual App;
                // the advertised observation is separate metadata, never permission.
                status: WorkerStatus {
                    blocked: true,
                    ..Default::default()
                },
                ordinary: Some(request(1).target),
            },
        ),
        0,
    )
    .unwrap();
    host
}
fn sent(effects: Vec<ControlV2Effect>) -> ControlV2Envelope {
    effects
        .into_iter()
        .find_map(|effect| match effect {
            ControlV2Effect::Send(envelope) => Some(envelope),
            _ => None,
        })
        .unwrap()
}
fn reply(binding: Binding, sequence: u64, reply: ControlV2Reply) -> ControlV2Envelope {
    ControlV2Envelope {
        version: CONTROL_V2_VERSION,
        binding,
        lease_epoch: 7,
        sequence,
        reply_to: reply.reply_to,
        message: ControlV2Message::Event(reply.event),
    }
}
fn online_status(sequence: u64) -> ControlV2Envelope {
    let mut status = event(
        sequence,
        0,
        ControlV2Event::Status {
            status: WorkerStatus {
                phase: Phase::Online,
                ..Default::default()
            },
            ordinary: None,
        },
    );
    status.binding.session = Some(session());
    status
}
fn frame(number: u64) -> FrameProof {
    FrameProof {
        device_epoch: 1,
        surface_epoch: 1,
        frame: number,
    }
}

#[test]
fn control_v2_real_coordinator_and_guard_keep_normal_prepare_grant_revoke_and_shared_v1_counters() {
    let mut host = ordinary_host();
    host.observe_control_v2(&online_status(2), 1).unwrap();
    let mut guard = WorkerGuard::new(binding().worker, 7, 0).unwrap();
    guard.update_session(Some(session())).unwrap();
    guard.frame_ready(0, frame(10)).unwrap();
    let prepare = sent(host.activate_control_v2(binding().worker.worker, None, 1).unwrap());
    assert_eq!(prepare.sequence, 1);
    assert_eq!(host.ordinary_target_control_v2(binding().worker.worker), None);
    let transition = guard.on_control_v2(&prepare, 1, 1).unwrap();
    assert!(transition.effects.prepare && transition.work.is_none());
    assert!(!guard.input_owned() && !guard.render_owned());
    guard
        .prepare_boundary(
            prepare.sequence,
            PrepareBoundary {
                device_epoch: 1,
                surface_epoch: 1,
                after_frame: 10,
            },
        )
        .unwrap();
    assert!(guard.frame_ready(prepare.sequence, frame(10)).unwrap().is_none());
    let ready = guard.frame_ready(prepare.sequence, frame(11)).unwrap().unwrap();
    let grant = sent(
        host.observe_control_v2(&reply(guard.binding(), 3, ControlV2Reply::from_control(ready)), 2)
            .unwrap()
            .effects,
    );
    assert_eq!(grant.sequence, 2);
    assert_eq!(host.active(), None);
    let accepted = guard.on_control_v2(&grant, 2, 2).unwrap();
    assert!(accepted.effects.grant_input);
    let observed = host
        .observe_control_v2(&reply(guard.binding(), 4, accepted.reply.unwrap()), 3)
        .unwrap();
    assert_eq!(
        observed.effects,
        vec![ControlV2Effect::Notice(HostNotice::Switched(binding().worker.worker))]
    );
    assert_eq!(host.active(), Some(binding().worker.worker));
    assert!(guard.input_owned());
    let heartbeat = sent(host.tick_control_v2(1000));
    assert_eq!(heartbeat.sequence, 3);
    assert_eq!(
        guard.on_control_v2(&heartbeat, 1000, 1000).unwrap().effects,
        WorkerEffects::default()
    );
    let v1 = host
        .tick(2000)
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Send(envelope) => Some(envelope),
            _ => None,
        })
        .unwrap();
    assert_eq!(v1.sequence, 4);
    guard.on_command(&v1, 2000).unwrap();
    assert_eq!(guard.on_control_v2(&heartbeat, 2001, 2001), Err(Error::Stale));
    let park = sent(host.park_active_control_v2(2001).unwrap());
    assert_eq!(park.sequence, 5);
    assert!(guard.on_control_v2(&park, 2001, 2001).unwrap().effects.quiesce);
    assert_eq!(guard.mode(), WorkerMode::Revoking);
    assert!(!guard.input_owned());
    assert!(
        guard
            .quiesced(
                park.sequence,
                GpuProgress {
                    device_epoch: 1,
                    submitted: 12,
                    completed: 11
                }
            )
            .unwrap()
            .is_none()
    );
    let idle = guard
        .quiesced(
            park.sequence,
            GpuProgress {
                device_epoch: 1,
                submitted: 12,
                completed: 12,
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        host.observe_control_v2(&reply(guard.binding(), 5, ControlV2Reply::from_control(idle)), 2002)
            .unwrap()
            .effects,
        vec![ControlV2Effect::Notice(HostNotice::Parked(binding().worker.worker))]
    );
    assert_eq!(host.active(), None);
    assert!(!host.busy());
}

#[test]
fn control_v2_old_show_completion_cannot_ack_current_prepare_or_change_current_source() {
    let mut host = ordinary_host();
    let show = host.show_viewer_control_v2(request(1), 1).unwrap();
    host.observe_control_v2(&online_status(2), 2).unwrap();
    let prepare = sent(host.activate_control_v2(binding().worker.worker, None, 2).unwrap());
    let receipt = ShowViewerReceipt {
        request: request(1),
        outcome: ShowViewerOutcome::ShownLocally,
    };
    let mut completion = event(3, prepare.sequence, ControlV2Event::ShowViewerReceipt(receipt));
    assert_eq!(host.observe_control_v2(&completion, 3), Err(Error::Stale));
    let worker = binding().worker.worker;
    assert!(host.busy());
    assert_eq!(host.active(), None);
    assert_eq!(host.entries[&worker].received, 2);
    completion.reply_to = show.sequence;
    let observed = host.observe_control_v2(&completion, 3).unwrap();
    assert_eq!(observed.show_receipt, Some(receipt));
    assert!(observed.effects.is_empty());
    assert!(host.busy());
    assert_eq!(host.pending.unwrap().sequence, prepare.sequence);
    assert_eq!(host.entries[&worker].roster.binding.session, Some(session()));
    assert_eq!(host.entries[&worker].ordinary, None);
    assert_eq!(host.query_show_viewer_control_v2(request(1), 3), Err(Error::Binding));
    let changed = event(
        4,
        show.sequence,
        ControlV2Event::ShowViewerReceipt(ShowViewerReceipt {
            outcome: ShowViewerOutcome::EffectUncertain,
            ..receipt
        }),
    );
    assert_eq!(host.observe_control_v2(&changed, 4), Err(Error::Stale));
    assert_eq!(host.entries[&worker].received, 3);
    let ready = ControlV2Reply {
        reply_to: prepare.sequence,
        event: ControlV2Event::FrameReady {
            activation: host.pending.unwrap().activation,
            boundary: PrepareBoundary {
                device_epoch: 1,
                surface_epoch: 1,
                after_frame: 10,
            },
            frame: frame(11),
        },
    };
    let grant = sent(
        host.observe_control_v2(&reply(host.entries[&worker].roster.binding, 4, ready), 4)
            .unwrap()
            .effects,
    );
    assert_eq!(grant.sequence, 3, "Show and activation use the same sent counter");
}

#[test]
fn control_v2_queries_recover_exact_terminal_without_resending_show_or_retargeting() {
    let mut host = ordinary_host();
    host.show_viewer_control_v2(request(1), 1).unwrap();
    assert_eq!(host.show_viewer_control_v2(request(1), 1), Err(Error::Stale));
    let query = host.query_show_viewer_control_v2(request(1), 2).unwrap();
    assert_eq!(host.query_show_viewer_control_v2(request(1), 2), Err(Error::Busy));
    let pending = event(
        2,
        query.sequence,
        ControlV2Event::ShowViewerQuery(ShowViewerQueryResult {
            request: request(1),
            result: ShowViewerQueryState::Pending,
        }),
    );
    assert!(host.observe_control_v2(&pending, 2).unwrap().effects.is_empty());
    let query = host.query_show_viewer_control_v2(request(1), 3).unwrap();
    let receipt = ShowViewerReceipt {
        request: request(1),
        outcome: ShowViewerOutcome::Cancelled,
    };
    let terminal = event(
        3,
        query.sequence,
        ControlV2Event::ShowViewerQuery(ShowViewerQueryResult {
            request: request(1),
            result: ShowViewerQueryState::Terminal(receipt),
        }),
    );
    let observed = host.observe_control_v2(&terminal, 3).unwrap();
    assert_eq!(observed.show_query.unwrap().request, request(1));
    assert!(observed.effects.is_empty());
    assert_eq!(host.show_viewer_control_v2(request(1), 4), Err(Error::Stale));
    let mut changed = request(1);
    changed.target.window.window += 1;
    assert_eq!(host.query_show_viewer_control_v2(changed, 4), Err(Error::Binding));
    assert_eq!(host.active(), None);
    assert!(!host.busy());
    let entry = &host.entries[&binding().worker.worker];
    assert_eq!(entry.shows.entries.len(), 1);
    assert_eq!(entry.shows.entries[0].receipt, Some(receipt));
    // A native-mode/source ABA cannot reset operation highwater/history.
    host.observe_control_v2(&online_status(4), 4).unwrap();
    let mut target = request(2).target;
    target.window.ordinary += 1;
    host.observe_control_v2(
        &event(
            5,
            0,
            ControlV2Event::Status {
                status: WorkerStatus::default(),
                ordinary: Some(target),
            },
        ),
        5,
    )
    .unwrap();
    assert_eq!(host.show_viewer_control_v2(request(1), 5), Err(Error::Unavailable));
    let fresh = ShowViewerRequest { request: 2, target };
    host.show_viewer_control_v2(fresh, 5).unwrap();
    assert_eq!(host.entries[&binding().worker.worker].shows.entries.len(), 2);
}

#[test]
fn control_v2_actual_entry_capacity_never_evicts_pending_or_spends_counter_on_failed_admission() {
    let mut host = ordinary_host();
    let worker = binding().worker.worker;
    for id in 1..=MAX_SHOW_RECEIPTS as u64 {
        host.show_viewer_control_v2(request(id), 1).unwrap();
    }
    assert_eq!(host.show_viewer_control_v2(request(17), 1), Err(Error::Capacity));
    assert_eq!(host.entries[&worker].sent, 16);
    assert_eq!(host.entries[&worker].shows.highwater, 16);
    let query = host.query_show_viewer_control_v2(request(1), 2).unwrap();
    assert_eq!(query.sequence, 17);
    let terminal = ShowViewerReceipt {
        request: request(1),
        outcome: ShowViewerOutcome::EffectUncertain,
    };
    host.observe_control_v2(
        &event(
            2,
            query.sequence,
            ControlV2Event::ShowViewerQuery(ShowViewerQueryResult {
                request: request(1),
                result: ShowViewerQueryState::Terminal(terminal),
            }),
        ),
        2,
    )
    .unwrap();
    assert_eq!(host.show_viewer_control_v2(request(17), 3).unwrap().sequence, 18);
    assert_eq!(host.entries[&worker].shows.entries.len(), MAX_SHOW_RECEIPTS);
    assert_eq!(host.query_show_viewer_control_v2(request(1), 3), Err(Error::Stale));
    host.observe_control_v2(
        &event(
            3,
            2,
            ControlV2Event::ShowViewerReceipt(ShowViewerReceipt {
                request: request(2),
                outcome: ShowViewerOutcome::Cancelled,
            }),
        ),
        3,
    )
    .unwrap();
    host.show_viewer_control_v2(request(u64::MAX), 3).unwrap();
    assert_eq!(host.entries[&worker].shows.highwater, u64::MAX);
    assert_eq!(host.show_viewer_control_v2(request(1), 4), Err(Error::Stale));
    assert_eq!(host.entries[&worker].shows.entries.len(), MAX_SHOW_RECEIPTS);
}

#[test]
fn control_v2_actual_shared_counter_exhaustion_is_terminal_without_new_operation_or_activation() {
    let mut host = ordinary_host();
    let worker = binding().worker.worker;
    host.entries.get_mut(&worker).unwrap().sent = u64::MAX - 1;
    assert_eq!(host.show_viewer_control_v2(request(1), 1).unwrap().sequence, u64::MAX);
    assert_eq!(host.show_viewer_control_v2(request(2), 1), Err(Error::Sequence));
    assert_eq!(host.query_show_viewer_control_v2(request(1), 1), Err(Error::Sequence));
    assert_eq!(host.entries[&worker].shows.highwater, 1);
    assert_eq!(host.entries[&worker].shows.entries.len(), 1);
    assert_eq!(host.entries[&worker].shows.entries[0].query, None);
    assert!(host.tick_control_v2(1000).is_empty());
    assert_eq!(host.entries[&worker].sent, u64::MAX);
    let mut host = ordinary_host();
    host.observe_control_v2(&online_status(2), 1).unwrap();
    host.activation = u64::MAX;
    assert_eq!(host.activate_control_v2(worker, None, 1), Err(Error::Sequence));
    assert_eq!(host.entries[&worker].sent, 0);
    assert!(!host.busy());
    assert_eq!(host.active(), None);
}

#[test]
fn control_v2_metadata_requires_exact_none_scope_and_control_send_invalidates_old_observation() {
    let mut host = ordinary_host();
    let worker = binding().worker.worker;
    let old = host.entries[&worker].received;
    let mut wrong = request(1).target;
    wrong.binding.session = Some(session());
    let status = event(
        2,
        0,
        ControlV2Event::Status {
            status: WorkerStatus::default(),
            ordinary: Some(wrong),
        },
    );
    assert_eq!(host.observe_control_v2(&status, 1), Err(Error::Binding));
    assert_eq!(host.entries[&worker].received, old);
    assert_eq!(host.ordinary_target_control_v2(worker), Some(request(1).target));
    // Direct V1 calls share the same Entry and cannot leave a V2 observation
    // looking ordinary after real presentation ownership changes.
    host.detach(worker).unwrap();
    assert_eq!(host.ordinary_target_control_v2(worker), None);
    assert_eq!(host.show_viewer_control_v2(request(1), 1), Err(Error::Unavailable));
    let mut guard = WorkerGuard::new(binding().worker, 7, 0).unwrap();
    let invalid = command(1, ControlV2Command::ShowViewer(request(1)));
    assert_eq!(guard.on_control_v2(&invalid, 5, 4), Err(Error::Stale));
    assert_eq!(
        guard.on_control_v2(&invalid, 4, 4).unwrap().work,
        Some(ControlV2Work::ShowViewer {
            request: request(1),
            outcome: None
        })
    );
}
