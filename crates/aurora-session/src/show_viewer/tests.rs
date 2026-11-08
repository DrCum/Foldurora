use super::*;
use crate::{AccountKey, GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerId};

fn request(id: u64) -> ShowViewerRequest {
    ShowViewerRequest {
        target: ShowViewerTarget {
            binding: Binding {
                worker: WorkerIdentity {
                    installation: InstallationId::from_bytes([1; 32]).unwrap(),
                    host: HostId::from_bytes([2; 16]).unwrap(),
                    worker: WorkerId::from_bytes([3; 16]).unwrap(),
                },
                session: None,
            },
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
fn ledger() -> ShowViewerLedger {
    let target = request(1).target;
    ShowViewerLedger::new(target.binding.worker, target.lease_epoch).unwrap()
}

#[test]
fn show_viewer_data_scope_is_closed_nonzero_and_optional_session_is_exact() {
    let original = request(u64::MAX);
    original.validate().unwrap();
    let mut session = original;
    session.target.binding.session = Some(SessionIdentity {
        session: SessionId::from_bytes([4; 16]).unwrap(),
        account: AccountKey::from_bytes([5; 32]).unwrap(),
        grid: GridKey::from_bytes([6; 32]).unwrap(),
        generation: 1,
    });
    let receipt = ShowViewerReceipt {
        request: session,
        outcome: ShowViewerOutcome::ShownLocally,
    };
    assert!(receipt.matches(session));
    assert!(!receipt.matches(original));
    session.target.binding.session.as_mut().unwrap().generation = 0;
    assert_eq!(session.validate(), Err(Error::Identity));
    for field in 0..5 {
        let mut bad = original;
        match field {
            0 => bad.request = 0,
            1 => bad.target.lease_epoch = 0,
            2 => bad.target.window.app = 0,
            3 => bad.target.window.window = 0,
            _ => bad.target.window.ordinary = 0,
        }
        assert!(bad.validate().is_err());
        assert!(!receipt.matches(bad));
    }
    let bytes = serde_json::to_vec(&receipt).unwrap();
    assert!(bytes.len() < crate::MAX_MESSAGE_BYTES);
    assert_eq!(serde_json::from_slice::<ShowViewerReceipt>(&bytes).unwrap(), receipt);
    let mut value = serde_json::to_value(receipt).unwrap();
    value["request"]["target"]["window"]["hwnd"] = serde_json::json!(99);
    assert!(serde_json::from_value::<ShowViewerReceipt>(value).is_err());
    assert!(serde_json::from_str::<WindowObservation>(r#"{"app":1,"app":2,"window":3,"ordinary":4}"#).is_err());
}

#[test]
fn show_viewer_pending_duplicate_and_terminal_retry_never_reserve_another_effect() {
    let mut ledger = ledger();
    let original = request(1);
    let mut effects = 0;
    if ledger.reserve(original).unwrap().is_none() {
        effects += 1;
    }
    assert_eq!(ledger.reserve(original), Err(Error::Busy));
    assert_eq!(ledger.receipt(original), Ok(None));
    let receipt = ledger.finish(original, ShowViewerOutcome::ForegroundRefused).unwrap();
    for _ in 0..4 {
        assert_eq!(ledger.reserve(original), Ok(Some(receipt)));
        assert_eq!(ledger.receipt(original), Ok(Some(receipt)));
        assert_eq!(ledger.finish(original, ShowViewerOutcome::ShownLocally), Err(Error::Stale));
    }
    assert_eq!(effects, 1); // Pure bookkeeping, not an actual native fixture.
    assert_eq!(ledger.retained_count(), 1);
}

#[test]
fn show_viewer_changed_app_window_mode_session_and_source_cannot_borrow_a_receipt() {
    let original = request(1);
    let mut ledger = ledger();
    ledger.reserve(original).unwrap();
    let receipt = ledger.finish(original, ShowViewerOutcome::RejectedNative).unwrap();
    for field in 0..8 {
        let mut changed = original;
        match field {
            0 => changed.target.window.app += 1,
            1 => changed.target.window.window += 1,
            2 => changed.target.window.ordinary += 1,
            3 => {
                changed.target.binding.session = Some(SessionIdentity {
                    session: SessionId::from_bytes([4; 16]).unwrap(),
                    account: AccountKey::from_bytes([5; 32]).unwrap(),
                    grid: GridKey::from_bytes([6; 32]).unwrap(),
                    generation: 1,
                })
            }
            4 => changed.target.binding.worker.worker = WorkerId::from_bytes([8; 16]).unwrap(),
            5 => changed.target.binding.worker.host = HostId::from_bytes([9; 16]).unwrap(),
            6 => changed.target.binding.worker.installation = InstallationId::from_bytes([10; 32]).unwrap(),
            _ => changed.target.lease_epoch += 1,
        }
        assert!(!receipt.matches(changed));
        assert!(ledger.reserve(changed).is_err());
        assert!(ledger.receipt(changed).is_err());
        assert!(ledger.finish(changed, ShowViewerOutcome::ShownLocally).is_err());
        assert_eq!(ledger.receipt(original), Ok(Some(receipt)));
    }
}

#[test]
fn show_viewer_capacity_never_evicts_pending_or_consumes_a_rejected_request() {
    let mut ledger = ledger();
    for id in 1..=MAX_SHOW_RECEIPTS as u64 {
        assert_eq!(ledger.reserve(request(id)), Ok(None));
    }
    let next = request(MAX_SHOW_RECEIPTS as u64 + 1);
    assert_eq!(ledger.reserve(next), Err(Error::Capacity));
    assert_eq!(ledger.retained_count(), MAX_SHOW_RECEIPTS);
    for id in 1..=MAX_SHOW_RECEIPTS as u64 {
        assert_eq!(ledger.reserve(request(id)), Err(Error::Busy));
    }
    let done = request(7);
    ledger.finish(done, ShowViewerOutcome::Expired).unwrap();
    assert_eq!(ledger.reserve(next), Ok(None));
    assert_eq!(ledger.retained_count(), MAX_SHOW_RECEIPTS);
    assert_eq!(ledger.reserve(done), Err(Error::Stale));
    assert_eq!(ledger.receipt(done), Err(Error::Stale));
    assert_eq!(ledger.reserve(request(1)), Err(Error::Busy));
}

#[test]
fn show_viewer_evicted_or_skipped_old_id_and_maximum_highwater_never_reopen() {
    let mut ledger = ledger();
    for id in 1..=MAX_SHOW_RECEIPTS as u64 + 2 {
        let current = request(id * 2);
        ledger.reserve(current).unwrap();
        ledger.finish(current, ShowViewerOutcome::ShownLocally).unwrap();
    }
    assert_eq!(ledger.reserve(request(1)), Err(Error::Stale));
    assert_eq!(ledger.reserve(request(2)), Err(Error::Stale));
    assert_eq!(ledger.receipt(request(2)), Err(Error::Stale));
    assert_eq!(ledger.finish(request(2), ShowViewerOutcome::ShownLocally), Err(Error::Stale));
    let last = request(u64::MAX);
    assert_eq!(ledger.reserve(last), Ok(None));
    let final_receipt = ledger.finish(last, ShowViewerOutcome::Expired).unwrap();
    assert_eq!(ledger.reserve(last), Ok(Some(final_receipt)));
    assert_eq!(ledger.reserve(request(0)), Err(Error::Sequence));
    assert_eq!(ledger.reserve(request(1)), Err(Error::Stale));
    assert_eq!(ledger.retained_count(), MAX_SHOW_RECEIPTS);
}

#[test]
fn show_viewer_full_request_lookup_is_readonly_and_never_changes_pending_completion() {
    let mut ledger = ledger();
    let original = request(3);
    assert_eq!(ledger.receipt(original), Ok(None));
    assert_eq!(ledger.retained_count(), 0);
    ledger.reserve(original).unwrap();
    let mut changed = original;
    changed.target.window.ordinary += 1;
    assert_eq!(ledger.receipt(changed), Err(Error::Invalid));
    assert_eq!(ledger.finish(changed, ShowViewerOutcome::ShownLocally), Err(Error::Invalid));
    assert_eq!(ledger.receipt(original), Ok(None));
    let receipt = ledger.finish(original, ShowViewerOutcome::StaleTarget).unwrap();
    assert_eq!(ledger.receipt(original), Ok(Some(receipt)));
    assert_eq!(ledger.receipt(request(4)), Ok(None));
    assert_eq!(ledger.reserve(request(4)), Ok(None));
    assert_eq!(ledger.receipt(original), Ok(Some(receipt)));
}
