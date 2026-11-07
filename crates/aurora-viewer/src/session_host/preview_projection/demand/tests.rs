//! Pure reservation/correlation tests. Typed native receipts here are fixture
//! DATA, not evidence of actual native configuration or source permission.
use super::*;
use aurora_session::preview_wire::DemandRejection;
use aurora_session::{AccountKey, GridKey, HostId, InstallationId, PreviewRate, SessionId, SessionIdentity, WorkerId};

fn token(number: u8, connection: u64, pixel: u64) -> ConnectionToken {
    let source = PreviewSource::new(
        WorkerIdentity {
            installation: InstallationId::from_bytes([1; 32]).unwrap(),
            host: HostId::from_bytes([2; 16]).unwrap(),
            worker: WorkerId::from_bytes([number + 3; 16]).unwrap(),
        },
        SessionIdentity {
            session: SessionId::from_bytes([number + 4; 16]).unwrap(),
            account: AccountKey::from_bytes([5; 32]).unwrap(),
            grid: GridKey::from_bytes([6; 32]).unwrap(),
            generation: 1,
        },
        7,
    )
    .unwrap();
    ConnectionToken::transport_fixture(source, connection, pixel, 1).unwrap()
}
fn option(rate: PreviewRate) -> Options {
    Options {
        rate,
        ..Options::default()
    }
}
fn receipt(offer: Offer, revision: u64) -> DemandReceipt {
    let requested = configuration(offer.desired.unwrap());
    DemandReceipt {
        source: offer.token.source(),
        metadata_connection: offer.token.connection(),
        demand: offer.demand,
        requested,
        outcome: DemandOutcome::Accepted {
            effective: requested,
            native_revision: revision,
        },
    }
}
fn accept(ledger: &mut Ledger, token: ConnectionToken, desired: Options, now: Instant) -> Offer {
    let offer = ledger.offer(token, Some(desired), now).unwrap().unwrap();
    ledger.admission(offer, true);
    assert!(ledger.receipt(token, receipt(offer, offer.demand)));
    offer
}

#[test]
fn preview_host_empty_reservations_and_no_implicit_demand_or_capture() {
    let mut ledger = Ledger::new();
    let now = Instant::now();
    let token = token(1, 1, 1);
    assert_eq!(ledger.held(), 0);
    assert!(ledger.accepted(token).is_none());
    assert!(ledger.offer(token, None, now).unwrap().is_none());
    assert!(ledger.due(&[token], now).is_none());
    assert!(aggregate(&[], &[]).is_empty());
    assert_eq!(ledger.entries.len(), 0);
}

#[test]
fn preview_host_rate_reservation_includes_unsent_old_pending_and_uncertain_without_double_debit() {
    let mut ledger = Ledger::new();
    let now = Instant::now();
    let a = token(1, 1, 1);
    let b = token(2, 1, 1);
    let c = token(3, 1, 1);
    let offer = ledger.offer(a, Some(option(PreviewRate::Five)), now).unwrap().unwrap();
    assert_eq!(ledger.held(), 10, "reserve before IO queue admission");
    assert!(ledger.accepted(a).is_none());
    accept(&mut ledger, b, option(PreviewRate::Five), now);
    assert_eq!(ledger.held(), 20);
    assert!(ledger.offer(c, Some(option(PreviewRate::Half)), now).is_err());
    ledger.admission(offer, false);
    assert_eq!(ledger.held(), 10, "known non-admission releases only its proposal");
    let increase = accept(&mut ledger, a, option(PreviewRate::One), now);
    assert_eq!(ledger.held(), 12);
    let lower = ledger.offer(b, Some(option(PreviewRate::One)), now).unwrap().unwrap();
    ledger.admission(lower, true);
    assert_eq!(ledger.held(), 12, "old Five remains until exact native lowering ACK");
    assert!(ledger.receipt(b, receipt(lower, 99)));
    assert_eq!(ledger.held(), 4);
    assert_eq!(ledger.accepted(a).unwrap().demand, increase.demand);
}

#[test]
fn preview_host_receipts_require_original_source_connection_demand_config_and_native_outcome() {
    let mut ledger = Ledger::new();
    let now = Instant::now();
    let token = token(1, 8, 1);
    let offer = ledger.offer(token, Some(option(PreviewRate::Two)), now).unwrap().unwrap();
    ledger.admission(offer, true);
    let expected = receipt(offer, 12);
    for mismatch in 0..5 {
        let mut wrong = expected;
        match mismatch {
            0 => wrong.source = super::tests::token(2, 8, 1).source(),
            1 => wrong.metadata_connection += 1,
            2 => wrong.demand += 1,
            3 => wrong.requested.rate = PreviewRate::Half,
            4 => {
                wrong.outcome = DemandOutcome::Accepted {
                    effective: wrong.requested,
                    native_revision: 0,
                }
            }
            _ => unreachable!(),
        }
        assert!(!ledger.receipt(token, wrong));
        assert_eq!(ledger.held(), 4);
        assert!(ledger.accepted(token).is_none());
    }
    assert!(ledger.receipt(token, expected));
    assert!(!ledger.receipt(token, expected), "immutable duplicates cannot apply twice");
    let accepted = ledger.accepted(token).unwrap();
    assert_eq!(accepted.demand, offer.demand);
    assert_eq!(accepted.native_revision, 12);
}

#[test]
fn preview_host_native_rejection_preserves_old_config_and_stop_releases_only_exact_ack() {
    let mut ledger = Ledger::new();
    let now = Instant::now();
    let token = token(1, 1, 1);
    let old = accept(&mut ledger, token, option(PreviewRate::One), now);
    let proposed = ledger.offer(token, Some(option(PreviewRate::Five)), now).unwrap().unwrap();
    ledger.admission(proposed, true);
    let rejection = DemandReceipt {
        outcome: DemandOutcome::Rejected {
            reason: DemandRejection::Denied,
        },
        ..receipt(proposed, 2)
    };
    assert!(ledger.receipt(token, rejection));
    assert_eq!(ledger.held(), 2);
    assert_eq!(ledger.accepted(token).unwrap().demand, old.demand);
    let stop = ledger.offer(token, None, now).unwrap().unwrap();
    assert_eq!(stop.demand, old.demand);
    assert!(matches!(stop.command(), PreviewCommand::Stop { demand } if demand == old.demand));
    ledger.admission(stop, true);
    assert!(!ledger.stopped(token, old.demand + 1));
    assert_eq!(ledger.held(), 2);
    assert!(ledger.stopped(token, old.demand));
    assert!(!ledger.stopped(token, old.demand));
    assert_eq!(ledger.held(), 0);
    let new = accept(&mut ledger, token, option(PreviewRate::Half), now);
    assert!(new.demand > proposed.demand);
    assert!(
        !ledger.stopped(token, old.demand),
        "late Stop cannot retire a later accepted config"
    );
    assert_eq!(ledger.held(), 1);
}

#[test]
fn preview_host_timeout_stream_loss_and_relog_hold_child_reservation_until_actual_exit() {
    let mut ledger = Ledger::new();
    let now = Instant::now();
    let token = token(1, 1, 1);
    accept(&mut ledger, token, option(PreviewRate::One), now);
    let pending = ledger.offer(token, Some(option(PreviewRate::Five)), now).unwrap().unwrap();
    ledger.admission(pending, true);
    ledger.tick(now + RECEIPT_WAIT);
    assert!(ledger.uncertain(token.source()));
    assert_eq!(ledger.held(), 10);
    assert!(
        !ledger.receipt(token, receipt(pending, 9)),
        "no optimistic recovery from late outcome"
    );
    let new_source = PreviewSource::new(
        token.source().worker(),
        SessionIdentity {
            generation: 2,
            ..token.source().session()
        },
        7,
    )
    .unwrap();
    let relog = ConnectionToken::transport_fixture(new_source, 2, 0, 1).unwrap();
    assert!(ledger.offer(relog, Some(option(PreviewRate::Half)), now).is_err());
    ledger.lost(token.source().worker());
    assert_eq!(ledger.held(), 10, "reachability/detach/EOF never prove process exit");
    ledger.exited(super::tests::token(2, 1, 1).source().worker());
    assert_eq!(ledger.held(), 10);
    ledger.exited(token.source().worker());
    assert_eq!(ledger.held(), 0);
}

#[test]
fn preview_host_pixel_role_replacement_keeps_config_but_reopen_requires_fresh_demand() {
    let mut ledger = Ledger::new();
    let now = Instant::now();
    let first = token(1, 1, 1);
    let original = accept(&mut ledger, first, Options::default(), now);
    let replaced = token(1, 1, 5);
    assert_eq!(ledger.accepted(replaced).unwrap().token, replaced);
    assert_eq!(ledger.accepted(replaced).unwrap().demand, original.demand);
    ledger.visibility_lost(first.source().worker());
    assert_eq!(ledger.held(), 2, "logical view removal never releases native rate");
    assert!(ledger.accepted(replaced).is_none());
    let reopened = ledger.offer(replaced, Some(Options::default()), now).unwrap().unwrap();
    assert!(reopened.demand > original.demand);
    assert_eq!(ledger.held(), 2);
    ledger.admission(reopened, true);
    assert!(ledger.receipt(replaced, receipt(reopened, 10)));
    assert_eq!(ledger.accepted(replaced).unwrap().demand, reopened.demand);
}

#[test]
fn preview_host_capture_pacing_is_global_fair_and_never_accumulates_catchup_tokens() {
    let mut ledger = Ledger::new();
    let now = Instant::now();
    let a = token(1, 1, 1);
    let b = token(2, 1, 1);
    accept(&mut ledger, a, option(PreviewRate::Five), now);
    accept(&mut ledger, b, option(PreviewRate::Five), now);
    assert_eq!(ledger.due(&[a, b], now).unwrap().token, a);
    assert!(ledger.due(&[a, b], now + Duration::from_millis(99)).is_none());
    assert_eq!(ledger.due(&[a, b], now + Duration::from_millis(100)).unwrap().token, b);
    assert_eq!(ledger.due(&[a, b], now + Duration::from_millis(200)).unwrap().token, a);
    let later = now + Duration::from_secs(10);
    assert!(ledger.due(&[a, b], later).is_some());
    assert!(ledger.due(&[a, b], later).is_none());
    // Eligibility is separate from reservation: only actual Active is supplied.
    assert_eq!(ledger.due(&[a], later + Duration::from_millis(200)).unwrap().token, a);
    assert_eq!(ledger.held(), 20);
}
