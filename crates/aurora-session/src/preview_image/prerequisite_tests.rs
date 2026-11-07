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
fn admitted() -> (SourceImages, AdmittedImage) {
    let budget = ImageBudget::new(MAX_IMAGE_BYTES).unwrap();
    let mut model = SourceImages::new(source(), budget.clone()).unwrap();
    model.sync_observed(policy(), 0, 0).unwrap();
    let claim = model.request(1, 1, geometry(), 1).unwrap();
    let stamp = PreviewStamp::new(claim, 1, 1, 1, geometry()).unwrap();
    let image = model.admit(stamp, budget.reserve(geometry()).unwrap(), 5).unwrap();
    (model, image)
}
#[test]
fn preview_observed_cached_policy_cannot_rejuvenate_original_authenticated_expiry() {
    let (mut model, image) = admitted();
    model.sync_observed(policy(), 0, 999).unwrap();
    assert!(image.current(999));
    assert!(model.retained(1000).unwrap().is_none());
    assert!(!image.current(1000));
    assert_eq!(model.sync_observed(policy(), 0, 1000), Err(ImageError::Stale));
    model.sync_observed(policy(), 1000, 1000).unwrap();
    assert!(model.retained(1000).unwrap().is_none());
    assert!(!image.current(1000));
}
#[test]
fn preview_observed_future_and_overflowing_receipts_retire_current_history() {
    for future in [true, false] {
        let (mut model, image) = admitted();
        let error = if future {
            model.sync_observed(policy(), 21, 20)
        } else {
            model.sync_observed(policy(), u64::MAX, u64::MAX)
        };
        assert_eq!(error, Err(ImageError::Clock));
        assert!(!image.current(20));
        assert!(model.request(1, 1, geometry(), u64::MAX).is_err());
    }
}
#[test]
fn preview_claim_accessors_and_layout_check_exact_policy_without_creating_authority() {
    let (mut model, _) = admitted();
    let claim = model.request(1, 9, geometry(), 10).unwrap();
    assert_eq!(claim.epochs(), policy().epochs());
    assert_eq!(claim.activation(), 2);
    assert_eq!(claim.demand(), 9);
    assert!(claim.matches_policy(policy()));
    let layout = PreviewLayout::new(source(), policy().epochs(), 2, 9, 11, [640, 360], [0, 0, 640, 360]).unwrap();
    assert!(layout.matches_policy(policy()));
    assert_eq!(layout.geometry(PreviewSize::Small).unwrap(), geometry());
    assert_eq!(layout.device_epoch(), 11);
    for wrong in 0..3 {
        let p = match wrong {
            0 => PreviewPolicy::new(source(), PreviewEpochs::new(2, 1, 1, 1, 1).unwrap(), policy().access()).unwrap(),
            1 => PreviewPolicy::new(
                source(),
                policy().epochs(),
                PreviewAccess {
                    activation: 3,
                    ..policy().access()
                },
            )
            .unwrap(),
            _ => PreviewPolicy::new(
                source(),
                policy().epochs(),
                PreviewAccess {
                    capture_active: false,
                    ..policy().access()
                },
            )
            .unwrap(),
        };
        assert!(!claim.matches_policy(p));
        assert!(!layout.matches_policy(p));
    }
}
#[test]
fn preview_layout_rejects_unknown_fields_zero_epochs_and_overflowing_crop() {
    let layout = PreviewLayout::new(source(), policy().epochs(), 2, 1, 1, [640, 360], [0, 0, 640, 360]).unwrap();
    let mut value = serde_json::to_value(layout).unwrap();
    value["path"] = "ignored".into();
    assert!(serde_json::from_value::<PreviewLayout>(value).is_err());
    assert!(PreviewLayout::new(source(), policy().epochs(), 2, 1, 1, [640, 360], [u32::MAX, 0, 4, 4]).is_err());
    for values in [[0, 1, 1], [2, 0, 1], [2, 1, 0]] {
        assert!(
            PreviewLayout::new(
                source(),
                policy().epochs(),
                values[0],
                values[1],
                values[2],
                [640, 360],
                [0, 0, 640, 360]
            )
            .is_err()
        );
    }
}

#[test]
fn preview_actual_surface_epoch_resets_only_under_advanced_render_and_never_revives_old_claims_or_history() {
    let budget = ImageBudget::new(2 * MAX_IMAGE_BYTES).unwrap();
    let mut model = SourceImages::new(source(), budget.clone()).unwrap();
    let actual = PreviewPolicy::new(source(), PreviewEpochs::new(1, 1, 1, 1, 9).unwrap(), policy().access()).unwrap();
    model.sync_observed(actual, 0, 0).unwrap();
    let claim9 = model.request(1, 1, geometry(), 1).unwrap();
    let stamp9 = PreviewStamp::new(claim9, 1, 9, 1, geometry()).unwrap();
    let held9 = model.admit(stamp9, budget.reserve(geometry()).unwrap(), 2).unwrap();
    let parked = PreviewPolicy::new(
        source(),
        actual.epochs(),
        PreviewAccess {
            active: false,
            capture_active: false,
            ..actual.access()
        },
    )
    .unwrap();
    model.sync_observed(parked, 3, 3).unwrap();
    assert!(held9.current(3), "normal park preserves actual-surface history");
    model.sync_observed(actual, 4, 4).unwrap();
    let pending9 = model.request(1, 2, geometry(), 5).unwrap();
    let late9 = PreviewStamp::new(pending9, 2, 9, 2, geometry()).unwrap();
    let unadvanced = PreviewPolicy::new(source(), PreviewEpochs::new(2, 1, 1, 1, 1).unwrap(), actual.access()).unwrap();
    assert_eq!(model.sync_observed(unadvanced, 6, 6), Err(ImageError::Stale));
    assert!(!held9.current(6));
    assert!(model.admit(late9, budget.reserve(geometry()).unwrap(), 7).is_err());
    let replacement = PreviewPolicy::new(source(), PreviewEpochs::new(2, 1, 2, 1, 1).unwrap(), actual.access()).unwrap();
    model.sync_observed(replacement, 8, 8).unwrap();
    assert!(model.retained(8).unwrap().is_none());
    assert!(!held9.current(8));
    let new_claim = model.request(1, 3, geometry(), 9).unwrap();
    assert_eq!(new_claim.epochs().surface(), 1);
    assert_eq!(new_claim.epochs().render(), 2);
    assert!(model.admit(late9, budget.reserve(geometry()).unwrap(), 10).is_err());
    let new_stamp = PreviewStamp::new(new_claim, 3, 1, 3, geometry()).unwrap();
    let fresh = model.admit(new_stamp, budget.reserve(geometry()).unwrap(), 11).unwrap();
    assert!(fresh.current(11));
    assert!(!held9.current(11));
    assert_eq!(model.sync_observed(actual, 12, 12), Err(ImageError::Stale));
    assert!(!fresh.current(12));
    model.sync_observed(replacement, 13, 13).unwrap();
    assert!(!held9.current(13));
    assert!(!fresh.current(13));
    assert!(model.retained(13).unwrap().is_none());
}
