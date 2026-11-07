//! Codec/resource fixtures only: fake publication predicates do not prove
//! Renderer/native authorization. Real publication leases are tested by render.
use super::tests::{Harness, policy, scope, scoped_sessions, source};
use super::*;
use aurora_render::viewport_capture::{CaptureBoundary, CaptureBudget, CaptureCap, CaptureOutcome, CaptureQueue, CapturedViewport};
use aurora_session::preview_image::{PreviewAccess, PreviewGeometry, PreviewPolicy, PreviewStamp, SourceImages};
use aurora_session::preview_wire::Metadata;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn captured() -> (CapturedViewport, CaptureBudget) {
    let budget = CaptureBudget::new(7 * 1024 * 1024).unwrap();
    let boundary = CaptureBoundary::new(1, 1, (640, 360)).unwrap();
    let mut queue = CaptureQueue::new(boundary, budget.clone()).unwrap();
    let request = queue
        .request(
            aurora_render::WorldViewport {
                x: 0,
                y: 0,
                width: 640,
                height: 360,
            },
            CaptureCap::Small,
            0,
        )
        .unwrap();
    let callback = queue.begin_map(request.token(), boundary, 1, 1).unwrap();
    let mut pixels = budget.pixels(request.layout()).unwrap();
    pixels.bytes_mut().fill(255);
    callback.complete(true);
    queue.finish_map(request.token(), boundary, pixels.finish().unwrap(), 2).unwrap();
    let Some(CaptureOutcome::Ready(result)) = queue.take_result(3).unwrap() else {
        panic!("pure ready result");
    };
    (result, budget)
}
fn captured_harness() -> Harness {
    let mut h = Harness::new(4096, 4096, aurora_session::PreviewSize::Small, true);
    h.images = SourceImages::new(source(3), h.budget.clone()).unwrap();
    h.images.sync(policy(), 0).unwrap();
    let geometry = PreviewGeometry::new([640, 360], [0, 0, 640, 360], aurora_session::PreviewSize::Small).unwrap();
    let claim = h.images.request(8, 1, geometry, 1).unwrap();
    h.stamp = PreviewStamp::new(claim, 1, 1, 1, geometry).unwrap();
    h.worker
        .queue_metadata(Metadata::Policy {
            connection: 8,
            policy: policy(),
        })
        .unwrap();
    h
}

#[test]
fn preview_captured_sender_fixture_moves_renderer_allocation_and_never_creates_worker_pixel_copy() {
    // Pin the real production signature without fabricating a real lease.
    let _signature: fn(
        &mut Pair,
        PreviewStamp,
        CapturedViewport,
        aurora_render::viewport_capture::CapturePublicationGuard,
    ) -> Result<(), Error> = Pair::queue_captured;
    let mut h = captured_harness();
    let (frame, budget) = captured();
    let held = frame.pixels().clone();
    let pointer = held.bytes().as_ptr();
    let charged = budget.used();
    h.worker
        .queue_captured_fixture(h.stamp, frame, Arc::new(AtomicBool::new(true)))
        .unwrap();
    assert_eq!(budget.used(), charged);
    assert_eq!(h.budget.used(), 0);
    let received = h.receive_at(10);
    assert_eq!(held.bytes().as_ptr(), pointer);
    assert!(received.test_bytes().unwrap().iter().all(|byte| *byte == 255));
    assert_eq!(
        budget.used(),
        held.bytes().len(),
        "only the held original packed allocation remains after the moved sender drops"
    );
    drop(received);
    assert_eq!(h.budget.used(), 0);
    drop(held);
    assert_eq!(budget.used(), 0);
}

#[test]
fn preview_captured_sender_fixture_rechecks_revocation_between_partial_chunks_and_refunds_original_credit() {
    let mut h = captured_harness();
    let (frame, budget) = captured();
    let valid = Arc::new(AtomicBool::new(true));
    h.set_pixel_write_limit(17);
    h.worker.queue_captured_fixture(h.stamp, frame, valid.clone()).unwrap();
    h.worker_tick(10).unwrap();
    assert!(budget.used() > 0);
    valid.store(false, Ordering::Release);
    assert!(matches!(h.worker_tick(11), Err(Error::Expired)));
    assert_eq!(budget.used(), 0);
    assert_eq!(h.budget.used(), 0);
}

#[test]
fn preview_worker_capture_and_full_framing_queues_obey_seven_plus_one_mebibyte_partition() {
    let (frame, budget) = captured();
    let layout = aurora_render::viewport_capture::CaptureLayout::new(640, 360).unwrap();
    let mut held = Vec::new();
    while let Ok(mut reservation) = budget.pixels(layout) {
        reservation.bytes_mut().fill(255);
        held.push(reservation.finish().unwrap());
    }
    assert!(budget.used() <= 7 * 1024 * 1024);
    let framing = super::stream::worker_framing_bound();
    assert!(framing <= 512 * 1024);
    assert!(budget.used() + framing + 256 * 1024 + 256 * 1024 <= 8 * 1024 * 1024);
    let mut h = captured_harness();
    h.worker
        .queue_captured_fixture(h.stamp, frame, Arc::new(AtomicBool::new(true)))
        .unwrap();
    h.worker.close();
    drop(held);
    assert_eq!(budget.used(), 0);
}

#[test]
fn preview_fresh_pixel_epoch_attachment_keeps_old_unadmitted_arc_permanently_invalid() {
    let mut h = Harness::new(4096, 4096, aurora_session::PreviewSize::Small, false);
    h.queue();
    let old = h.receive();
    assert!(old.transport_current());
    let parked = PreviewPolicy::new(
        source(3),
        policy().epochs(),
        PreviewAccess {
            active: false,
            capture_active: false,
            activation: 2,
            ..policy().access()
        },
    )
    .unwrap();
    h.worker
        .queue_metadata(Metadata::Policy {
            connection: 8,
            policy: parked,
        })
        .unwrap();
    h.worker_tick(20).unwrap();
    h.host_tick(20).unwrap();
    assert!(!old.transport_current());
    let active = PreviewPolicy::new(
        source(3),
        policy().epochs(),
        PreviewAccess {
            activation: 3,
            ..policy().access()
        },
    )
    .unwrap();
    h.worker
        .queue_metadata(Metadata::Policy {
            connection: 8,
            policy: active,
        })
        .unwrap();
    h.worker_tick(21).unwrap();
    h.host_tick(21).unwrap();
    let (old_epoch, _) = scoped_sessions(scope(Lane::Pixels));
    assert!(matches!(h.host.attach_pixels(old_epoch), Err(Error::Sequence)));
    let fresh = Scope {
        role_epoch: 2,
        ..scope(Lane::Pixels)
    };
    let (wrong_source, _) = scoped_sessions(Scope {
        source: source(9),
        ..fresh
    });
    assert!(matches!(h.host.attach_pixels(wrong_source), Err(Error::Binding)));
    let (wrong_role, _) = scoped_sessions(scope(Lane::Metadata));
    assert!(matches!(h.host.attach_pixels(wrong_role), Err(Error::Binding)));
    let (host, worker) = scoped_sessions(fresh);
    h.host.attach_pixels(host).unwrap();
    h.worker.attach_pixels(worker).unwrap();
    assert!(
        !old.transport_current(),
        "a new validity Arc cannot re-enable a retired received frame"
    );
    let raw = vec![255; h.stamp.geometry().payload_bytes()];
    h.worker
        .queue_pixels(UnadmittedPixels::copy_from(h.stamp, &raw, &h.budget).unwrap())
        .unwrap();
    let received = h.receive_at(30);
    assert!(received.transport_current());
    assert!(!old.transport_current());
    h.worker
        .queue_metadata(Metadata::CaptureStopped {
            source: source(3),
            connection: 8,
            pixel_epoch: 1,
            stop: 1,
            policy: parked,
        })
        .unwrap();
    h.worker_tick(31).unwrap();
    h.host_tick(31).unwrap();
    assert!(received.transport_current(), "late old-role stop cannot close its replacement");
    drop(old);
    drop(received);
    assert_eq!(h.budget.used(), 0);
    let (duplicate, _) = scoped_sessions(fresh);
    assert!(matches!(h.host.attach_pixels(duplicate), Err(Error::Busy)));
}

#[test]
fn preview_pixel_epoch_is_mac_bound_even_when_untrusted_auth_prefix_is_relabelled() {
    let a = scope(Lane::Pixels);
    let b = Scope { role_epoch: 2, ..a };
    let mut host = HostHandshake::new(a, super::tests::secret(), 0).unwrap();
    let mut worker = WorkerHandshake::new(b, super::tests::secret(), 0).unwrap();
    let mut challenge = host.challenge().as_slice().to_vec();
    challenge[16..24].copy_from_slice(&2u64.to_be_bytes());
    let mut proof = worker.answer(&challenge, 1).unwrap().as_slice().to_vec();
    proof[16..24].copy_from_slice(&1u64.to_be_bytes());
    assert!(host.accept(&proof, 2).is_err());
}
