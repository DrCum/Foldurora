//! Actual World policy and sealed queue metadata. These cases do not assert
//! native HWND eligibility or stand in for the Windows hosted acceptance run.
use super::tests::{facts, world};
use super::*;
use aurora_session::preview_image::{HOST_IMAGE_BYTES, ImageBudget, PreviewKind, SourceImages};
use aurora_session::{PreviewRate, PreviewSize};

#[test]
fn live_native_policy_never_activated_source_requires_exact_independent_inactive_authority() {
    let world = world();
    let now = Instant::now();
    let mut facts = facts(&world, now);
    let active = facts.authority.take().unwrap();
    facts.inactive = Some(crate::session_host::worker::InactivePreviewAuthority {
        scope: active.scope,
        boundary: active.boundary,
    });
    let mut model = HostPreview::default();
    model.sync(facts, now);
    let policy = model.policy.unwrap();
    assert!(policy.access().capture_inactive);
    assert!(!policy.access().active);
    assert!(!policy.access().capture_active);
    assert_eq!(policy.access().activation, 0, "no active grant or fabricated activation");
    let mut host = SourceImages::new(policy.source(), ImageBudget::new(HOST_IMAGE_BYTES).unwrap()).unwrap();
    host.sync(policy, 0).unwrap();
    let mut ledger = super::demand::Ledger::default();
    ledger
        .apply(
            policy.source(),
            1,
            1,
            aurora_session::preview_wire::DemandConfiguration {
                size: PreviewSize::Small,
                rate: PreviewRate::One,
            },
            None,
        )
        .unwrap();
    let observed = model.observed.unwrap();
    let layout = super::producer::inactive_layout(policy, ledger.current(), observed.inactive, observed.viewport).unwrap();
    assert_eq!(layout.kind(), PreviewKind::LiveInactive);
    let geometry = layout.geometry(PreviewSize::Small).unwrap();
    assert!(host.request(1, 1, geometry, 1).is_err());
    let claim = host.request_inactive(1, 1, geometry, 1).unwrap();
    assert_eq!(claim.activation(), 0);
    assert_eq!(claim.kind(), PreviewKind::LiveInactive);
}

#[test]
fn live_native_policy_rejects_missing_expired_foreign_or_physical_mismatch_and_renews_without_capture_churn() {
    let world = world();
    let now = Instant::now();
    for invalid in 0..8 {
        let mut facts = facts(&world, now);
        let active = facts.authority.take().unwrap();
        facts.inactive = Some(crate::session_host::worker::InactivePreviewAuthority {
            scope: active.scope,
            boundary: active.boundary,
        });
        match invalid {
            0 => facts.inactive = None,
            1 => facts.inactive.as_mut().unwrap().scope.expires_at = now,
            2 => {
                facts.inactive.as_mut().unwrap().boundary =
                    aurora_render::viewport_capture::CaptureBoundary::new(42, 9, (1280, 720)).unwrap()
            }
            3 => {
                facts.inactive.as_mut().unwrap().boundary =
                    aurora_render::viewport_capture::CaptureBoundary::new(41, 10, (1280, 720)).unwrap()
            }
            4 => facts.world.privacy = true,
            5 => facts.photo.panel = true,
            6 => facts.remembered = false,
            7 => {
                let source = active.scope.source;
                facts.inactive.as_mut().unwrap().scope.source =
                    aurora_session::preview_image::PreviewSource::new(source.worker(), source.session(), source.lease_epoch() + 1).unwrap();
            }
            _ => unreachable!(),
        }
        let mut model = HostPreview::default();
        model.sync(facts, now);
        assert!(!model.policy.unwrap().access().capture_inactive, "invalid case {invalid}");
    }
    let mut model = HostPreview::default();
    for tick in 0..3 {
        let mut facts = facts(&world, now + std::time::Duration::from_millis(tick));
        let active = facts.authority.take().unwrap();
        facts.inactive = Some(crate::session_host::worker::InactivePreviewAuthority {
            scope: active.scope,
            boundary: active.boundary,
        });
        assert!(
            !model.sync(facts, now + std::time::Duration::from_millis(tick)),
            "ordinary same-source lease renewal must not churn capture"
        );
        assert!(model.policy.unwrap().access().capture_inactive);
    }
}
