//! Production metadata entrypoints composed with actual native World facts and
//! SourceImages. These fixtures mint no real renderer/native publication guard.
use super::super::HostPreview;
use super::super::tests::{facts, world};
use super::*;
use aurora_session::preview_image::{HOST_IMAGE_BYTES, ImageBudget, SourceImages};
use aurora_session::{PreviewRate, PreviewSize};

fn config(rate: PreviewRate) -> DemandConfiguration {
    DemandConfiguration {
        size: PreviewSize::Small,
        rate,
    }
}

#[test]
fn native_producer_layout_precedes_first_claim_and_exact_configuration_matches_actual_host_model() {
    let world = world();
    let now = Instant::now();
    let mut native = HostPreview::default();
    native.sync(facts(&world, now), now);
    let policy = native.policy.unwrap();
    let observed = native.observed.unwrap();
    let mut ledger = Ledger::default();
    assert!(native_layout(policy, ledger.current(), observed.active, observed.viewport).is_none());
    ledger.apply(policy.source(), 1, 7, config(PreviewRate::One), None).unwrap();
    // No pending claim exists: the CPU publishes this layout independently.
    let layout = native_layout(policy, ledger.current(), observed.active, observed.viewport).unwrap();
    assert!(layout.matches_policy(policy));
    let budget = ImageBudget::new(HOST_IMAGE_BYTES).unwrap();
    let mut host = SourceImages::new(policy.source(), budget).unwrap();
    host.sync(policy, 0).unwrap();
    let claim = host.request(1, 7, layout.geometry(PreviewSize::Small).unwrap(), 1).unwrap();
    assert!(claim_matches(policy, Some(layout), ledger.current(), claim));
    let wrong_demand = Applied {
        demand: 8,
        ..ledger.current().unwrap()
    };
    assert!(!claim_matches(policy, Some(layout), Some(wrong_demand), claim));
    let wrong_size = Applied {
        config: DemandConfiguration {
            size: PreviewSize::Large,
            ..config(PreviewRate::One)
        },
        ..ledger.current().unwrap()
    };
    assert!(!claim_matches(policy, Some(layout), Some(wrong_size), claim));
    let mut wrong_active = observed.active.unwrap();
    wrong_active.activation += 1;
    assert!(native_layout(policy, ledger.current(), Some(wrong_active), observed.viewport).is_none());
    wrong_active = observed.active.unwrap();
    wrong_active.boundary = aurora_render::viewport_capture::CaptureBoundary::new(41, 10, (1280, 720)).unwrap();
    assert!(native_layout(policy, ledger.current(), Some(wrong_active), observed.viewport).is_none());
    assert!(
        native_layout(
            policy,
            ledger.current(),
            observed.active,
            Some(WorldViewport {
                x: u32::MAX,
                y: 0,
                width: 8,
                height: 8
            })
        )
        .is_none()
    );
}

#[test]
fn native_producer_layout_and_claim_stop_on_park_while_actual_admitted_history_survives() {
    let world = world();
    let now = Instant::now();
    let mut native = HostPreview::default();
    native.sync(facts(&world, now), now);
    let active = native.policy.unwrap();
    let observed = native.observed.unwrap();
    let mut ledger = Ledger::default();
    ledger.apply(active.source(), 1, 7, config(PreviewRate::One), None).unwrap();
    let layout = native_layout(active, ledger.current(), observed.active, observed.viewport).unwrap();
    let geometry = layout.geometry(PreviewSize::Small).unwrap();
    let budget = ImageBudget::new(HOST_IMAGE_BYTES).unwrap();
    let mut host = SourceImages::new(active.source(), budget.clone()).unwrap();
    host.sync(active, 0).unwrap();
    let claim = host.request(1, 7, geometry, 1).unwrap();
    let history = host
        .admit(
            PreviewStamp::new(claim, 1, 9, 1, geometry).unwrap(),
            budget.reserve(geometry).unwrap(),
            2,
        )
        .unwrap();
    let late = host.request(1, 7, geometry, 3).unwrap();
    let mut parked = facts(&world, now);
    parked.authority = None;
    parked.capture_ready = false;
    parked.capture_generation += 1;
    native.sync(parked, now);
    let policy = native.policy.unwrap();
    assert_eq!(policy.epochs(), active.epochs());
    assert!(native_layout(policy, ledger.current(), None, observed.viewport).is_none());
    assert!(!claim_matches(policy, Some(layout), ledger.current(), late));
    host.sync(policy, 4).unwrap();
    assert!(history.current(4));
    assert!(host.retained(4).unwrap().is_some());
    assert!(
        host.admit(
            PreviewStamp::new(late, 2, 9, 2, geometry).unwrap(),
            budget.reserve(geometry).unwrap(),
            5
        )
        .is_err()
    );
    assert!(host.request(1, 7, geometry, 5).is_err());
    // Reactivation gets a new exact active claim; it cannot revive the old one.
    let mut restored = facts(&world, now);
    restored.authority.as_mut().unwrap().activation += 1;
    restored.capture_generation += 2;
    native.sync(restored, now);
    let policy = native.policy.unwrap();
    let observed = native.observed.unwrap();
    let fresh_layout = native_layout(policy, ledger.current(), observed.active, observed.viewport).unwrap();
    assert!(!claim_matches(policy, Some(fresh_layout), ledger.current(), late));
    host.sync(policy, 6).unwrap();
    assert!(history.current(6));
    let fresh = host.request(1, 7, fresh_layout.geometry(PreviewSize::Small).unwrap(), 7).unwrap();
    assert!(claim_matches(policy, Some(fresh_layout), ledger.current(), fresh));
}

#[test]
fn native_producer_rate_gate_uses_original_request_time_without_catchup_or_future_clock() {
    let world = world();
    let now = Instant::now();
    let source = facts(&world, now).scope.unwrap().source;
    for rate in [PreviewRate::Half, PreviewRate::One, PreviewRate::Two, PreviewRate::Five] {
        let mut producer = Producer::default();
        assert!(!producer.rate_ready(now), "no implicit demand");
        producer.ledger.apply(source, 1, 1, config(rate), None).unwrap();
        assert!(producer.rate_ready(now));
        producer.last_capture = Some(now);
        let interval = Duration::from_millis(rate.interval_millis());
        assert!(!producer.rate_ready(now + interval - Duration::from_nanos(1)));
        assert!(producer.rate_ready(now + interval));
        assert!(!producer.rate_ready(now - Duration::from_millis(1)));
        // Only a real successful request updates last_capture; no timer catchup.
        producer.last_capture = Some(now + interval * 20);
        assert!(!producer.rate_ready(now + interval * 20));
        producer.exhausted = true;
        assert!(!producer.rate_ready(now + interval * 40));
    }
}

#[test]
fn native_producer_command_age_has_absolute_deadline_and_metadata_stays_inside_fixed_reserve() {
    let now = Instant::now();
    assert!(Producer::fresh(now, now));
    assert!(Producer::fresh(now, now + TTL - Duration::from_millis(1)));
    assert!(!Producer::fresh(now, now + TTL));
    // Windows Instant arithmetic can round a 1 ns offset back to the same
    // counter tick. Keep the future-time check distinct from the exact expiry.
    let future = now + Duration::from_millis(1);
    assert!(future > now);
    assert!(!Producer::fresh(future, now));
    let producer = Producer::default();
    // Real static layouts + actual receipt allocation, conservatively reserving
    // two contexts/pending commands/accepted proofs and 32 small Arc allocations.
    let bytes = std::mem::size_of::<Producer>()
        + producer.ledger.allocated_bytes()
        + 2 * (std::mem::size_of::<Context>()
            + std::mem::size_of::<Pending>()
            + std::mem::size_of::<ReceivedCommand>()
            + std::mem::size_of::<crate::session_host::worker::AcceptedNativePreviewSource>())
        + 32 * 256;
    assert!(bytes <= crate::session_host::worker::NATIVE_PREVIEW_METADATA_BYTES);
    assert!(producer.pending.is_none() && !producer.ever_requested && !producer.retirement_needed);
}

#[test]
fn native_producer_capture_barrier_and_full_retirement_have_distinct_permanent_witnesses() {
    let mut producer = Producer::default();
    let retention = EpochFence::new();
    let capture = EpochFence::new();
    producer.retain = Some(retention.clone());
    producer.capture = Some(capture.clone());
    producer.capture_barrier();
    assert!(!capture.current());
    assert!(
        retention.current(),
        "ordinary pre-intent capture stop preserves historical authority"
    );
    let next_capture = EpochFence::new();
    producer.capture = Some(next_capture.clone());
    assert!(!capture.current(), "old capture can never revive after a new grant");
    producer.retire();
    assert!(!next_capture.current());
    assert!(!retention.current());
    assert!(producer.retain.is_none() && producer.control.is_none());
    producer.retain = Some(EpochFence::new());
    assert!(!retention.current(), "same-looking restored source cannot revive the old witness");
}

#[test]
fn native_producer_busy_connection_read_preserves_original_config_but_cannot_authorize_capture() {
    let world = world();
    let now = Instant::now();
    let mut native = HostPreview::default();
    native.sync(facts(&world, now), now);
    let active = native.policy.unwrap();
    let token = ConnectionToken::transport_fixture(active.source(), 1, 1, 1).unwrap();
    let mut producer = Producer::default();
    assert!(producer.new_metadata_domain(Some(token)));
    let receipt = producer
        .ledger
        .apply(active.source(), 1, 7, config(PreviewRate::One), None)
        .unwrap();
    let original = producer.ledger.current();
    producer.last_capture = Some(now);
    assert!(!producer.new_metadata_domain(None), "unavailable read cannot prove replacement");
    assert_eq!(producer.connection, Some(token));
    assert_eq!(producer.ledger.current(), original);
    assert_eq!(producer.last_capture, Some(now), "original rate age cannot renew");
    let mut unavailable = facts(&world, now);
    unavailable.remembered = false; // Actual current read None, never cached token.
    native.sync(unavailable, now);
    let policy = native.policy.unwrap();
    assert!(!policy.access().capture_active);
    assert_eq!(
        policy.epochs(),
        active.epochs(),
        "unavailable read does not revoke admitted history"
    );
    assert!(
        native_layout(
            policy,
            producer.ledger.current(),
            native.observed.unwrap().active,
            native.observed.unwrap().viewport
        )
        .is_none()
    );
    assert!(!producer.new_metadata_domain(Some(token)));
    assert_eq!(
        producer.ledger.apply(active.source(), 1, 7, config(PreviewRate::One), None),
        Some(receipt)
    );
    let next_pixel = ConnectionToken::transport_fixture(active.source(), 1, 2, 1).unwrap();
    assert!(
        !producer.new_metadata_domain(Some(next_pixel)),
        "fresh pixel role is the same metadata domain"
    );
    let reconnected = ConnectionToken::transport_fixture(active.source(), 2, 1, 1).unwrap();
    assert!(
        producer.new_metadata_domain(Some(reconnected)),
        "genuine reconnect must retire old demand"
    );
    producer.ledger.reset_domain(); // Same branch as observe, before layout/admission.
    assert!(producer.ledger.current().is_none());
    assert!(
        !producer.rate_ready(now + Duration::from_secs(10)),
        "old accepted config is never replayed"
    );
}

#[test]
fn native_producer_post_poll_native_loss_or_retarget_rejects_original_ready_claim() {
    let world = world();
    let now = Instant::now();
    let mut native = HostPreview::default();
    native.sync(facts(&world, now), now);
    let policy = native.policy.unwrap();
    let observed = native.observed.unwrap();
    let mut ledger = Ledger::default();
    ledger.apply(policy.source(), 1, 7, config(PreviewRate::One), None).unwrap();
    let layout = native_layout(policy, ledger.current(), observed.active, observed.viewport).unwrap();
    let mut host = SourceImages::new(policy.source(), ImageBudget::new(HOST_IMAGE_BYTES).unwrap()).unwrap();
    host.sync(policy, 0).unwrap();
    let claim = host.request(1, 7, layout.geometry(PreviewSize::Small).unwrap(), 1).unwrap();
    let pending = Pending {
        token: ConnectionToken::transport_fixture(policy.source(), 1, 1, 1).unwrap(),
        claim,
        deadline: now + TTL,
        gpu: None, // No fake real renderer token/result is minted.
    };
    let actual = facts(&world, now).authority.unwrap();
    assert!(pending_authority_matches(
        policy,
        Some(layout),
        ledger.current(),
        &pending,
        Some(actual),
        now
    ));
    // Production re-queries preview_authority after poll: a newly hidden,
    // minimized, unfitted or lost native window returns None even if the old
    // copied Context and atomic capture generation have not changed.
    assert!(!pending_authority_matches(
        policy,
        Some(layout),
        ledger.current(),
        &pending,
        None,
        now
    ));
    let mut changed = actual;
    changed.activation += 1;
    assert!(!pending_authority_matches(
        policy,
        Some(layout),
        ledger.current(),
        &pending,
        Some(changed),
        now
    ));
    changed = actual;
    changed.boundary = aurora_render::viewport_capture::CaptureBoundary::new(42, 9, (1280, 720)).unwrap();
    assert!(!pending_authority_matches(
        policy,
        Some(layout),
        ledger.current(),
        &pending,
        Some(changed),
        now
    ));
    changed = actual;
    changed.scope.expires_at = now;
    assert!(!pending_authority_matches(
        policy,
        Some(layout),
        ledger.current(),
        &pending,
        Some(changed),
        now
    ));
    assert!(!pending_authority_matches(
        policy,
        Some(layout),
        ledger.current(),
        &pending,
        Some(actual),
        now + TTL
    ));
    // Original ready claim/deadline remain immutable; no fresh proof substitutes.
    assert_eq!(pending.claim, claim);
    assert_eq!(pending.deadline, now + TTL);
}

#[test]
fn live_native_pending_rechecks_exact_source_boundary_lease_and_deadline_after_sampling() {
    let world = world();
    let now = Instant::now();
    let mut f = facts(&world, now);
    let active = f.authority.take().unwrap();
    let authority = crate::session_host::worker::InactivePreviewAuthority {
        scope: active.scope,
        boundary: active.boundary,
    };
    f.inactive = Some(authority);
    let mut native = HostPreview::default();
    native.sync(f, now);
    let policy = native.policy.unwrap();
    let observed = native.observed.unwrap();
    let mut ledger = Ledger::default();
    ledger.apply(policy.source(), 1, 7, config(PreviewRate::One), None).unwrap();
    let layout = inactive_layout(policy, ledger.current(), observed.inactive, observed.viewport).unwrap();
    let mut model = SourceImages::new(policy.source(), ImageBudget::new(HOST_IMAGE_BYTES).unwrap()).unwrap();
    model.sync(policy, 0).unwrap();
    let claim = model
        .request_inactive(1, 7, layout.geometry(PreviewSize::Small).unwrap(), 1)
        .unwrap();
    let mut pending = Pending {
        token: ConnectionToken::transport_fixture(policy.source(), 1, 1, 1).unwrap(),
        claim,
        deadline: now + TTL,
        gpu: None,
    };
    assert!(pending_inactive_matches(
        policy,
        Some(layout),
        ledger.current(),
        &pending,
        Some(authority),
        now
    ));
    assert!(!pending_inactive_matches(
        policy,
        Some(layout),
        ledger.current(),
        &pending,
        None,
        now
    ));
    for invalid in 0..5 {
        let mut changed = authority;
        match invalid {
            0 => changed.scope.expires_at = now,
            1 => {
                changed.scope.source = PreviewSource::new(
                    policy.source().worker(),
                    policy.source().session(),
                    policy.source().lease_epoch() + 1,
                )
                .unwrap()
            }
            2 => changed.boundary = aurora_render::viewport_capture::CaptureBoundary::new(42, 9, (1280, 720)).unwrap(),
            3 => changed.boundary = aurora_render::viewport_capture::CaptureBoundary::new(41, 10, (1280, 720)).unwrap(),
            4 => changed.boundary = aurora_render::viewport_capture::CaptureBoundary::new(41, 9, (640, 360)).unwrap(),
            _ => unreachable!(),
        }
        assert!(
            !pending_inactive_matches(policy, Some(layout), ledger.current(), &pending, Some(changed), now),
            "changed boundary case {invalid}"
        );
    }
    pending.deadline = now;
    assert!(!pending_inactive_matches(
        policy,
        Some(layout),
        ledger.current(),
        &pending,
        Some(authority),
        now
    ));
}
