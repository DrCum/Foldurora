use super::*;
use aurora_session::chat::{ChatKey, Conversation, ObservedIncoming};
use aurora_session::{AccountKey, GridKey, HostId, InstallationId, PhysicalRect, SessionId, SessionIdentity, WorkerId, WorkerIdentity};

pub(super) fn target() -> ReviewCapture {
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

pub(super) fn viewport() -> HostedViewport {
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

fn grant(activation: u64) -> Grant {
    Grant {
        binding: target().key.source.binding(),
        activation,
        viewport: viewport(),
    }
}

fn waiting() -> Transactions {
    let mut transactions = Transactions::default();
    assert!(transactions.begin(target(), 40, viewport(), 10));
    assert!(transactions.expect_activation(8));
    transactions
}

fn dispatched() -> (Transactions, ReviewConversationRequest) {
    let mut transactions = waiting();
    transactions.granted(grant(8));
    let request = transactions.dispatch(grant(8)).unwrap();
    transactions.queued(request);
    (transactions, request)
}

#[test]
fn review_target_allows_native_read_and_incoming_advance_without_send_or_mark_capacity() {
    let target = target();
    let policy = NativeChatPolicy {
        key: target.key,
        revision: target.policy_revision,
        lease_current: true,
        ready: true,
        can_read: true,
        can_send: false,
    };
    let read = ReadMetadata {
        unread: 0,
        unread_revision: 99,
        observed: ObservedIncoming {
            through: 12,
            ..target.observed
        },
        can_mark_read: false,
    };
    assert!(current_target(
        target,
        Some(target.key.source),
        true,
        true,
        Some(policy),
        Some(read)
    ));
    for denial in 0..9 {
        let mut source = target.key.source;
        let mut policy = policy;
        let mut read = read;
        match denial {
            0 => source.session.generation += 1,
            1 => source.lease_epoch += 1,
            2 => source.session.account = AccountKey::from_bytes([11; 32]).unwrap(),
            3 => source.session.grid = GridKey::from_bytes([12; 32]).unwrap(),
            4 => policy.revision += 1,
            5 => policy.can_read = false,
            6 => policy.lease_current = false,
            7 => read.observed.epoch += 1,
            8 => read.observed.through = target.observed.through - 1,
            _ => unreachable!(),
        }
        assert!(
            !current_target(target, Some(source), true, true, Some(policy), Some(read)),
            "denial {denial}"
        );
    }
    assert!(!current_target(
        target,
        Some(target.key.source),
        false,
        true,
        Some(policy),
        Some(read)
    ));
    assert!(!current_target(
        target,
        Some(target.key.source),
        true,
        false,
        Some(policy),
        Some(read)
    ));
}

#[test]
fn review_requires_accepted_exact_grant_not_target_or_consumed_host_ticket_alone() {
    let mut transactions = waiting();
    assert!(
        transactions.dispatch(grant(8)).is_none(),
        "having the right active identity is insufficient before accepted Switched"
    );
    transactions.granted(grant(8));
    let request = transactions.dispatch(grant(8)).unwrap();
    assert_eq!(request.target, target());
    assert_eq!(request.request, 40);
    assert_eq!(request.activation, 8);
    transactions.queued(request);
    assert!(
        transactions.dispatch(grant(8)).is_none(),
        "the captured operation dispatches at most once"
    );
}

#[test]
fn review_wrong_binding_activation_or_viewport_grants_cannot_release_capture() {
    for wrong in 0..4 {
        let mut transactions = waiting();
        let mut candidate = grant(8);
        match wrong {
            0 => candidate.binding.session.as_mut().unwrap().generation += 1,
            1 => candidate.activation += 1,
            2 => candidate.viewport.rect.x += 1,
            3 => candidate.binding.worker.worker = WorkerId::from_bytes([13; 16]).unwrap(),
            _ => unreachable!(),
        }
        transactions.granted(candidate);
        assert!(transactions.dispatch(grant(8)).is_none());
        assert_eq!(transactions.operations()[0].status, Status::Cancelled);
    }
}

#[test]
fn review_a_to_b_to_a_and_same_worker_new_activation_never_revive_old_capture() {
    let mut transactions = waiting();
    transactions.control_activation(9);
    transactions.granted(grant(8));
    transactions.granted(grant(10));
    assert!(transactions.dispatch(grant(8)).is_none());
    assert_eq!(transactions.operations()[0].status, Status::Cancelled);
    let mut rebound = waiting();
    assert!(
        !rebound.expect_activation(10),
        "a captured grant is never rebound to a later transaction"
    );
    assert!(rebound.dispatch(grant(10)).is_none());
}

#[test]
fn review_cancel_is_exact_and_postdispatch_result_stays_uncertain_until_receipt() {
    let mut transactions = waiting();
    transactions.cancel(39);
    assert!(transactions.waiting());
    transactions.granted(grant(8));
    assert!(transactions.dispatch(grant(8)).is_some());
    transactions.cancel(40);
    assert!(!transactions.waiting());
    assert!(
        transactions.dispatch(grant(8)).is_none(),
        "cancel before native admission wins even after grant"
    );
    assert!(transactions.begin(target(), 41, viewport(), 12));
    transactions.expect_activation(9);
    transactions.cancel(40);
    assert!(transactions.waiting(), "an old Cancel cannot cancel the next Review");

    let (mut sent, request) = dispatched();
    sent.cancel(40);
    assert_eq!(sent.operations()[0].status, Status::Uncertain);
    sent.delivery(Some(Delivery {
        request,
        status: Status::Dispatched,
        recovering: true,
    }));
    assert_eq!(sent.operations()[0].status, Status::Uncertain);
    sent.delivery(Some(Delivery {
        request,
        status: Status::Completed(ReviewOutcome::OpenedLocally),
        recovering: false,
    }));
    assert_eq!(sent.operations()[0].status, Status::Completed(ReviewOutcome::OpenedLocally));
    assert!(sent.target().is_none());
}

#[test]
fn review_deadline_distinguishes_unsent_expiry_from_dispatched_uncertainty() {
    let mut unsent = waiting();
    unsent.tick(15_010, true);
    assert_eq!(unsent.operations()[0].status, Status::Expired);
    unsent.granted(grant(8));
    assert!(unsent.dispatch(grant(8)).is_none());
    let (mut sent, request) = dispatched();
    sent.tick(15_010, true);
    assert_eq!(sent.operations()[0].status, Status::Uncertain);
    sent.delivery(Some(Delivery {
        request,
        status: Status::Completed(ReviewOutcome::OpenedLocally),
        recovering: false,
    }));
    assert_eq!(
        sent.operations()[0].status,
        Status::Uncertain,
        "an expired recovery cannot revive a later operation"
    );
}

#[test]
fn review_loss_or_rollback_before_dispatch_guarantees_no_later_open() {
    for status in [Status::Cancelled, Status::Unavailable, Status::Busy] {
        let mut transactions = waiting();
        transactions.interrupt(status);
        transactions.granted(grant(8));
        assert!(transactions.dispatch(grant(8)).is_none());
        assert_eq!(transactions.operations()[0].status, status);
    }
    let mut transactions = waiting();
    transactions.tick(11, false);
    transactions.granted(grant(8));
    assert!(transactions.dispatch(grant(8)).is_none());
    assert_eq!(transactions.operations()[0].status, Status::Unavailable);
}

#[test]
fn review_status_book_is_bounded_and_busy_rejections_never_evict_unfinished_action() {
    let mut transactions = waiting();
    for request in 41..100 {
        assert!(!transactions.begin(target(), request, viewport(), 11));
    }
    let operations = transactions.operations();
    assert_eq!(operations.len(), MAX_OPERATIONS);
    assert_eq!(operations[0].request, 40);
    assert_eq!(operations[0].status, Status::WaitingForGrant);
    assert!(operations[1..].iter().all(|operation| operation.status == Status::Busy));
    transactions.cancel(40);
    assert!(transactions.begin(target(), 100, viewport(), 12));
    assert_eq!(transactions.operations().len(), MAX_OPERATIONS);
    transactions.cancel(100);
    for request in 101..140 {
        assert!(transactions.begin(target(), request, viewport(), 13));
        transactions.cancel(request);
    }
    assert!(
        !transactions.begin(target(), 40, viewport(), 14),
        "evicting a status never makes its old ticket executable again"
    );
}

#[test]
fn review_missing_delivery_after_dispatch_is_uncertain_and_never_requeues() {
    let (mut transactions, request) = dispatched();
    transactions.delivery(None);
    assert_eq!(transactions.operations()[0].status, Status::Uncertain);
    assert!(transactions.dispatch(grant(8)).is_none());
    transactions.delivery(Some(Delivery {
        request,
        status: Status::Completed(ReviewOutcome::OpenedLocally),
        recovering: false,
    }));
    assert_eq!(transactions.operations()[0].status, Status::Uncertain);
}
