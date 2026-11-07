//! Pure local request cancellation: synthetic identities, no endpoint/native
//! command, renderer grant or demand-reservation owner is constructed here.
use super::*;
use crate::{AccountKey, GridKey, HostId, InstallationId, SessionId, WorkerId};

fn source() -> PreviewSource {
    PreviewSource::new(
        WorkerIdentity {
            installation: InstallationId::from_bytes([1; 32]).unwrap(),
            host: HostId::from_bytes([2; 16]).unwrap(),
            worker: WorkerId::from_bytes([3; 16]).unwrap(),
        },
        SessionIdentity {
            session: SessionId::from_bytes([4; 16]).unwrap(),
            account: AccountKey::from_bytes([5; 32]).unwrap(),
            grid: GridKey::from_bytes([6; 32]).unwrap(),
            generation: 1,
        },
        7,
    )
    .unwrap()
}
fn policy() -> PreviewPolicy {
    PreviewPolicy::new(
        source(),
        PreviewEpochs::new(1, 1, 1, 1, 1).unwrap(),
        PreviewAccess {
            ready: true,
            active: true,
            activation: 2,
            capture_active: true,
            retain_last_active: true,
        },
    )
    .unwrap()
}
fn geometry() -> PreviewGeometry {
    PreviewGeometry::new([640, 360], [0, 0, 640, 360], PreviewSize::Small).unwrap()
}
fn model(budget: &ImageBudget) -> SourceImages {
    let mut model = SourceImages::new(source(), budget.clone()).unwrap();
    model.sync_observed(policy(), 0, 0).unwrap();
    model
}
fn stamp(claim: CaptureClaim, sequence: u64) -> PreviewStamp {
    PreviewStamp::new(claim, sequence, claim.epochs().surface(), sequence, claim.geometry()).unwrap()
}
fn history(budget: &ImageBudget) -> (SourceImages, AdmittedImage) {
    let mut model = model(budget);
    let claim = model.request(8, 7, geometry(), 1).unwrap();
    let image = model.admit(stamp(claim, 1), budget.reserve(geometry()).unwrap(), 2).unwrap();
    (model, image)
}

#[test]
fn preview_cancel_old_owner_request_one_cannot_cancel_recreated_owner_request_one() {
    let budget = ImageBudget::new(4 * MAX_IMAGE_BYTES).unwrap();
    let mut old = model(&budget);
    let old_claim = old.request(8, 7, geometry(), 1).unwrap();
    drop(old);
    let mut fresh = model(&budget);
    let fresh_claim = fresh.request(8, 7, geometry(), 1).unwrap();
    assert_eq!(old_claim.request(), 1);
    assert_eq!(fresh_claim.request(), 1);
    assert_ne!(old_claim, fresh_claim);
    assert!(!fresh.cancel_capture(old_claim));
    assert_eq!(fresh.request(8, 7, geometry(), 2), Err(ImageError::Busy));
    assert_eq!(budget.used(), 0);
    let image = fresh.admit(stamp(fresh_claim, 1), budget.reserve(geometry()).unwrap(), 3).unwrap();
    assert!(image.current(3));
    assert_eq!(image.stamp().claim(), fresh_claim);
    assert_eq!(budget.used(), geometry().payload_bytes());
    drop(image);
    drop(fresh);
    assert_eq!(budget.used(), 0);
}

#[test]
fn preview_cancel_each_mismatched_claim_scope_preserves_real_pending_and_retained_history() {
    // Isolate every captured identity field while keeping the actual model's
    // pending request fixed. These are caller-supplied metadata lookalikes,
    // never proof of a native capture or an invented parent authority.
    for wrong in 0..8 {
        let budget = ImageBudget::new(4 * MAX_IMAGE_BYTES).unwrap();
        let (mut model, held) = history(&budget);
        let expected = model.request(8, 9, geometry(), 3).unwrap();
        let wrong = match wrong {
            0 => CaptureClaim {
                source: PreviewSource::new(
                    source().worker(),
                    SessionIdentity {
                        generation: 2,
                        ..source().session()
                    },
                    source().lease_epoch(),
                )
                .unwrap(),
                ..expected
            },
            1 => CaptureClaim {
                epochs: PreviewEpochs::new(2, 1, 1, 1, 1).unwrap(),
                ..expected
            },
            2 => CaptureClaim {
                activation: expected.activation + 1,
                ..expected
            },
            3 => CaptureClaim {
                instance: expected.instance + 1,
                ..expected
            },
            4 => CaptureClaim {
                connection: expected.connection + 1,
                ..expected
            },
            5 => CaptureClaim {
                demand: expected.demand + 1,
                ..expected
            },
            6 => CaptureClaim {
                request: expected.request + 1,
                ..expected
            },
            _ => CaptureClaim {
                geometry: PreviewGeometry::new([640, 360], [0, 0, 640, 360], PreviewSize::Medium).unwrap(),
                ..expected
            },
        };
        let charged = budget.used();
        assert!(!model.cancel_capture(wrong));
        assert_eq!(budget.used(), charged);
        assert!(held.current(4));
        assert_eq!(model.retained(4).unwrap().unwrap().stamp(), held.stamp());
        assert_eq!(model.request(8, 9, geometry(), 5), Err(ImageError::Busy));
        let fresh = model.admit(stamp(expected, 2), budget.reserve(geometry()).unwrap(), 6).unwrap();
        assert_eq!(fresh.stamp().claim(), expected);
        assert!(fresh.current(6));
        assert_eq!(budget.used(), 2 * geometry().payload_bytes());
        drop(fresh);
        drop(held);
        drop(model);
        assert_eq!(budget.used(), 0);
    }
}

#[test]
fn preview_cancel_exact_pending_keeps_history_credit_and_all_request_high_water_marks() {
    let budget = ImageBudget::new(4 * MAX_IMAGE_BYTES).unwrap();
    let (mut model, held) = history(&budget);
    let pending = model.request(8, 9, geometry(), 3).unwrap();
    let arriving = budget.reserve(geometry()).unwrap();
    let charged = budget.used();
    assert!(model.cancel_capture(pending));
    assert!(!model.cancel_capture(pending));
    assert_eq!(
        budget.used(),
        charged,
        "cancelling metadata cannot refund still-owned incoming credit"
    );
    assert!(held.current(4));
    assert_eq!(model.retained(4).unwrap().unwrap().stamp(), held.stamp());
    assert!(matches!(model.admit(stamp(pending, 2), arriving, 5), Err(ImageError::Stale)));
    assert_eq!(
        budget.used(),
        geometry().payload_bytes(),
        "only the rejected incoming credit was dropped"
    );
    assert_eq!(model.request(7, 100, geometry(), 6), Err(ImageError::Stale));
    assert_eq!(model.request(8, 8, geometry(), 7), Err(ImageError::Stale));
    let next = model.request(8, 9, geometry(), 8).unwrap();
    assert_eq!(next.request(), pending.request() + 1);
    assert!(!model.cancel_capture(pending));
    assert!(matches!(
        model.admit(stamp(pending, 3), budget.reserve(geometry()).unwrap(), 9),
        Err(ImageError::Stale)
    ));
    assert_eq!(model.request(8, 9, geometry(), 10), Err(ImageError::Busy));
    // Cancellation must preserve native sequence high-water too: sequence1 was
    // already admitted, so it cannot become valid again for the next request.
    assert!(matches!(
        model.admit(stamp(next, 1), budget.reserve(geometry()).unwrap(), 11),
        Err(ImageError::Stale)
    ));
    let image = model.admit(stamp(next, 2), budget.reserve(geometry()).unwrap(), 12).unwrap();
    assert_eq!(image.stamp().claim(), next);
    assert!(held.current(999));
    assert!(!held.current(1000));
    assert!(!image.current(1000), "cancellation did not refresh original policy expiry");
    drop(image);
    drop(held);
    drop(model);
    assert_eq!(budget.used(), 0);
}

#[test]
fn preview_cancel_already_admitted_claim_or_no_pending_request_never_changes_history() {
    let budget = ImageBudget::new(4 * MAX_IMAGE_BYTES).unwrap();
    let (mut model, held) = history(&budget);
    let admitted = held.stamp().claim();
    let charged = budget.used();
    assert!(!model.cancel_capture(admitted));
    assert_eq!(budget.used(), charged);
    assert!(held.current(3));
    assert_eq!(model.retained(3).unwrap().unwrap().stamp(), held.stamp());
    let pending = model.request(8, 8, geometry(), 4).unwrap();
    assert!(!model.cancel_capture(admitted));
    assert!(model.cancel_capture(pending));
    assert_eq!(model.retained(5).unwrap().unwrap().stamp(), held.stamp());
    model.retire();
    assert!(!model.cancel_capture(pending));
    assert!(!held.current(5));
    assert_eq!(budget.used(), charged, "held history keeps credit after legitimate retirement");
    drop(held);
    drop(model);
    assert_eq!(budget.used(), 0);
}
