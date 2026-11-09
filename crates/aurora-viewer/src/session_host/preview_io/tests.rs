//! Owned loopback fixtures only, authored but not executed by the source author.
//! Synthetic control/native predicates prove codec/resource/lifecycle behavior;
//! none of them establishes actual native or Renderer capture authorization.
use super::state::{FrameHandoff, Observation, Status};
use super::*;
use aurora_render::viewport_capture::{CaptureBoundary, CaptureBudget, CaptureCap, CaptureOutcome, CaptureQueue, CapturedViewport};
use aurora_session::preview_image::{PreviewAccess, PreviewEpochs, SourceImages};
use aurora_session::{AccountKey, GridKey, HostedViewport, PhysicalRect, SessionId, SessionIdentity, WorkerId};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};

pub(super) static OWNED: Mutex<()> = Mutex::new(());
pub(super) fn source(generation: u64) -> PreviewSource {
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
            generation,
        },
        7,
    )
    .unwrap()
}
pub(super) fn control(source: PreviewSource) -> ControlGuard {
    ControlGuard {
        source,
        expires_at: Instant::now() + Duration::from_secs(5),
        witness: EpochFence::new(),
    }
}
pub(super) fn state(control: ControlGuard, capture: Option<EpochFence>) -> WorkerState {
    let policy = PreviewPolicy::new(
        control.source,
        PreviewEpochs::new(1, 1, 1, 1, 1).unwrap(),
        PreviewAccess {
            capture_inactive: false,
            ready: true,
            active: capture.is_some(),
            activation: 1,
            capture_active: capture.is_some(),
            retain_last_active: true,
        },
    )
    .unwrap();
    let observed = Instant::now();
    WorkerState {
        policy,
        layout: capture
            .as_ref()
            .map(|_| PreviewLayout::new(control.source, policy.epochs(), 1, 1, 1, [640, 360], [0, 0, 640, 360]).unwrap()),
        retention: SourceRetentionGuard::fixture(control.clone(), policy, (observed + DEADLINE).min(control.expires_at)),
        capture,
        native_observed_at: observed,
    }
}
fn until<T>(mut poll: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(value) = poll() {
            return value;
        }
        assert!(Instant::now() < deadline, "bounded owned fixture timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
struct Owned {
    _serial: MutexGuard<'static, ()>,
    host: HostChannel,
    worker: WorkerChannel,
    control: ControlGuard,
    state: WorkerState,
}
impl Owned {
    fn new(capture: bool) -> Self {
        let serial = owned_fixture_guard();
        let source = source(1);
        let control = control(source);
        let host = HostChannel::start(source.worker().installation, source.worker().host).unwrap();
        let port = until(|| host.port());
        let key = PreviewSecret::synthetic([7; 32]).unwrap();
        host.publish_permits(&[HostPermit {
            source,
            secret: key.clone(),
            control: control.clone(),
            registration: 1,
        }])
        .unwrap();
        let worker = WorkerChannel::start(PreviewBootstrap { port, key }, source.worker(), source.lease_epoch()).unwrap();
        worker.set_source(Some(SourceLease { control: control.clone() })).unwrap();
        let state = state(control.clone(), capture.then(EpochFence::new));
        worker.publish_state(state.clone()).unwrap();
        Self {
            _serial: serial,
            host,
            worker,
            control,
            state,
        }
    }
    fn accept_demand(&self, id: u64, config: DemandConfiguration) -> DemandReceipt {
        let token = self.ready().token;
        self.host
            .try_command(
                token,
                PreviewCommand::RememberActive {
                    demand: id,
                    size: config.size,
                    rate: config.rate,
                },
                None,
            )
            .unwrap();
        let command = until(|| self.worker.take_command());
        assert!(
            matches!(command.command, PreviewCommand::RememberActive { demand, size, rate } if demand==id && size==config.size && rate==config.rate)
        );
        let receipt = DemandReceipt {
            source: token.source,
            metadata_connection: token.connection,
            demand: id,
            requested: config,
            outcome: DemandOutcome::Accepted {
                effective: config,
                native_revision: id,
            },
        };
        self.worker.try_demand_receipt(command.token, receipt).unwrap();
        until(|| {
            self.host
                .try_observations()
                .ok()?
                .sources()
                .iter()
                .any(|o| o.accepted_demand() == Some(&receipt))
                .then_some(())
        });
        receipt
    }
    fn observation(&self, status: Status) -> Option<Observation> {
        self.host
            .take_update()?
            .sources
            .into_iter()
            .map(|u| u.observation)
            .find(|o| o.current() && o.status == status)
    }
    fn ready(&self) -> Observation {
        until(|| {
            self.observation(Status::Pixels).filter(|o| {
                self.worker
                    .connection()
                    .is_some_and(|t| t.pixel_epoch() == o.token.pixel_epoch() && t.pixel_epoch() > 0)
            })
        })
    }
    fn park(&mut self) {
        self.state = state(self.control.clone(), None);
        self.worker.publish_state(self.state.clone()).unwrap();
    }
    fn restore(&mut self) {
        self.state = state(self.control.clone(), Some(EpochFence::new()));
        self.worker.publish_state(self.state.clone()).unwrap();
    }
    fn capture(&self, size: PreviewSize) -> (SourceImages, FrameHandoff, CaptureBudget) {
        let observed = self.ready();
        let geometry = observed.layout.unwrap().geometry(size).unwrap();
        let mut model = SourceImages::new(self.control.source, self.host.image_budget()).unwrap();
        let now = self.host.shared.clock(Instant::now());
        model
            .sync_observed(
                observed.policy.unwrap(),
                self.host.shared.clock(observed.policy_received_at.unwrap()),
                now,
            )
            .unwrap();
        let claim = model.request(observed.token.connection(), 1, geometry, now).unwrap();
        self.host
            .try_command(
                observed.token,
                PreviewCommand::Capture(Box::new(claim)),
                Some(CaptureControlGuard {
                    control: self.control.clone(),
                    mode: crate::session_host::transport::preview_control::CaptureMode::Active {
                        activation: 1,
                        viewport: HostedViewport {
                            host_window: 1,
                            rect: PhysicalRect {
                                x: 0,
                                y: 0,
                                width: 640,
                                height: 360,
                            },
                        },
                    },
                    witness: EpochFence::new(),
                }),
            )
            .unwrap();
        let received = until(|| self.worker.take_command());
        assert!(received.received_at <= Instant::now());
        assert_eq!(received.token.source(), self.control.source);
        assert!(matches!(&received.command,PreviewCommand::Capture(c) if **c==claim));
        let (frame, budget) = frame(size);
        let stamp = PreviewStamp::new(claim, 1, 1, 1, geometry).unwrap();
        self.worker
            .fixture_frame(received.token, stamp, frame, Arc::new(AtomicBool::new(true)))
            .unwrap();
        let handoff = until(|| self.host.take_update()?.sources.into_iter().find_map(|u| u.frame));
        assert!(handoff.received_at <= Instant::now());
        assert_eq!(handoff.claim(), claim);
        (model, handoff, budget)
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        self.worker.stop();
        self.host.stop();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !(self.worker.ended() && self.host.ended()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(self.worker.ended() && self.host.ended(), "owned IO threads must terminate");
    }
}
pub(super) fn frame(size: PreviewSize) -> (CapturedViewport, CaptureBudget) {
    let budget = CaptureBudget::new(7 * 1024 * 1024).unwrap();
    let boundary = CaptureBoundary::new(1, 1, (640, 360)).unwrap();
    let mut queue = CaptureQueue::new(boundary, budget.clone()).unwrap();
    let cap = match size {
        PreviewSize::Small => CaptureCap::Small,
        PreviewSize::Medium => CaptureCap::Medium,
        PreviewSize::Large => CaptureCap::Large,
    };
    let request = queue
        .request(
            aurora_render::WorldViewport {
                x: 0,
                y: 0,
                width: 640,
                height: 360,
            },
            cap,
            0,
        )
        .unwrap();
    let callback = queue.begin_map(request.token(), boundary, 1, 1).unwrap();
    let mut pixels = budget.pixels(request.layout()).unwrap();
    pixels.bytes_mut().fill(255);
    callback.complete(true);
    queue.finish_map(request.token(), boundary, pixels.finish().unwrap(), 2).unwrap();
    let Some(CaptureOutcome::Ready(frame)) = queue.take_result(3).unwrap() else {
        panic!("pure fixture result")
    };
    (frame, budget)
}
#[test]
fn preview_io_owned_loopback_roundtrip_normal_stop_ack_retains_only_admitted_history_and_fresh_role() {
    let mut io = Owned::new(true);
    let old = io.ready();
    let (mut model, handoff, budget) = io.capture(PreviewSize::Small);
    let image = handoff
        .admit(&mut model, &io.control, io.host.shared.clock(Instant::now()))
        .unwrap();
    assert!(
        image
            .with_pixels(&io.control, io.host.shared.clock(Instant::now()), |pixels| pixels
                .iter()
                .all(|b| *b == 255))
            .unwrap()
    );
    assert_eq!(budget.used(), 0);
    io.park();
    let parked = until(|| io.observation(Status::CaptureStopped));
    assert_eq!(parked.token.connection(), old.token.connection());
    assert!(
        image
            .with_pixels(&io.control, io.host.shared.clock(Instant::now()), |_| ())
            .is_some(),
        "capture-only stop must preserve admitted history"
    );
    io.restore();
    let fresh = io.ready();
    assert_eq!(fresh.token.connection(), old.token.connection());
    assert!(fresh.token.pixel_epoch() > old.token.pixel_epoch());
    assert!(io.host.try_command(old.token, PreviewCommand::Stop { demand: 1 }, None).is_err());
    assert!(
        image
            .with_pixels(&io.control, io.host.shared.clock(Instant::now()), |_| ())
            .is_some()
    );
    io.host.publish_permits(&[]).unwrap();
    assert!(
        image
            .with_pixels(&io.control, io.host.shared.clock(Instant::now()), |_| ())
            .is_none(),
        "source removal revokes held frame synchronously"
    );
}
#[test]
fn preview_io_owned_loopback_metadata_progresses_while_pixel_writer_is_explicitly_stalled() {
    let io = Owned::new(true);
    let observed = io.ready();
    io.worker.shared.stall_pixels.store(true, Ordering::Release);
    let geometry = observed.layout.unwrap().geometry(PreviewSize::Large).unwrap();
    let mut model = SourceImages::new(source(1), io.host.image_budget()).unwrap();
    let now = io.host.shared.clock(Instant::now());
    model.sync(observed.policy.unwrap(), now).unwrap();
    let claim = model.request(observed.token.connection(), 1, geometry, now).unwrap();
    io.host
        .try_command(
            observed.token,
            PreviewCommand::Capture(Box::new(claim)),
            Some(CaptureControlGuard {
                control: io.control.clone(),
                mode: crate::session_host::transport::preview_control::CaptureMode::Active {
                    activation: 1,
                    viewport: HostedViewport {
                        host_window: 1,
                        rect: PhysicalRect {
                            x: 0,
                            y: 0,
                            width: 640,
                            height: 360,
                        },
                    },
                },
                witness: EpochFence::new(),
            }),
        )
        .unwrap();
    let command = until(|| io.worker.take_command());
    assert!(matches!(command.command,PreviewCommand::Capture(c) if *c==claim));
    let (frame, budget) = frame(PreviewSize::Large);
    io.worker
        .fixture_frame(
            io.worker.connection().unwrap(),
            PreviewStamp::new(claim, 1, 1, 1, geometry).unwrap(),
            frame,
            Arc::new(AtomicBool::new(true)),
        )
        .unwrap();
    let mut previous = SourceImages::new(source(1), io.host.image_budget()).unwrap();
    previous.sync(observed.policy.unwrap(), now).unwrap();
    let old_claim = previous.request(observed.token.connection(), 1, geometry, now).unwrap();
    assert_eq!(old_claim.request(), claim.request());
    assert_ne!(old_claim, claim);
    io.worker
        .try_unavailable(io.worker.connection().unwrap(), old_claim, Unavailable::Busy)
        .unwrap();
    until(|| {
        io.host
            .take_update()?
            .sources
            .into_iter()
            .find(|u| u.observation.unavailable == Some((old_claim, Unavailable::Busy)))
    });
    assert!(budget.used() > 0);
    assert!(!io.worker.frame_available());
    assert_eq!(io.host.image_budget().used(), 0);
    io.worker.shared.stall_pixels.store(false, Ordering::Release);
    io.worker.shared.wake.notify_one();
    let handoff = until(|| io.host.take_update()?.sources.into_iter().find_map(|u| u.frame));
    let admitted = handoff
        .admit(&mut model, &io.control, io.host.shared.clock(Instant::now()))
        .unwrap();
    assert_eq!(
        admitted
            .with_pixels(&io.control, io.host.shared.clock(Instant::now()), |pixels| pixels.len())
            .unwrap(),
        640 * 360 * 4
    );
    assert_eq!(budget.used(), 0);
}
#[test]
fn preview_io_owned_loopback_reverse_pixel_eof_before_exact_stop_ack_purges_history() {
    let mut io = Owned::new(true);
    let (mut model, handoff, _) = io.capture(PreviewSize::Small);
    let image = handoff
        .admit(&mut model, &io.control, io.host.shared.clock(Instant::now()))
        .unwrap();
    io.worker.shared.break_before_ack.store(true, Ordering::Release);
    io.park();
    until(|| {
        image
            .with_pixels(&io.control, io.host.shared.clock(Instant::now()), |_| ())
            .is_none()
            .then_some(())
    });
    assert!(
        !io.host
            .take_update()
            .unwrap()
            .sources
            .into_iter()
            .any(|u| u.observation.current() && u.observation.status == Status::CaptureStopped)
    );
}
#[test]
fn preview_io_owned_loopback_original_native_expiry_and_service_stop_invalidate_held_frames() {
    let io = Owned::new(true);
    let (mut model, handoff, _) = io.capture(PreviewSize::Small);
    let image = handoff
        .admit(&mut model, &io.control, io.host.shared.clock(Instant::now()))
        .unwrap();
    let bytes = io.host.image_budget().used();
    assert!(bytes > 0);
    // No App/native observation is published again. IO heartbeats cannot mint
    // a new native policy receipt or keep this source alive indefinitely.
    until(|| {
        if let Some(update) = io.host.take_update() {
            for entry in update.sources {
                if entry.observation.current()
                    && let (Some(policy), Some(received)) = (entry.observation.policy, entry.observation.policy_received_at)
                {
                    let _ = model.sync_observed(policy, io.host.shared.clock(received), io.host.shared.clock(Instant::now()));
                }
            }
        }
        image
            .with_pixels(&io.control, io.host.shared.clock(Instant::now()), |_| ())
            .is_none()
            .then_some(())
    });
    assert_eq!(
        io.host.image_budget().used(),
        bytes,
        "revocation does not refund physically held bytes"
    );
    drop(image);
    drop(model);
    until(|| (io.host.image_budget().used() == 0).then_some(()));
    io.worker.stop();
    io.host.stop();
    until(|| (io.worker.ended() && io.host.ended()).then_some(()));
    assert!(io.host.port().is_none());
}
#[test]
fn preview_io_owned_loopback_pending_handshake_cap_and_unknown_hello_never_replace_incumbent() {
    let io = Owned::new(false);
    let incumbent = until(|| io.observation(Status::Metadata));
    let port = io.host.port().unwrap();
    let mut idle = Vec::new();
    for _ in 0..11 {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        stream.write_all(b"A").unwrap();
        idle.push(stream);
    }
    let stats = until(|| {
        let u = io.host.take_update()?;
        (u.pending_roles == 10).then_some(u)
    });
    assert_eq!(stats.connected_roles, 1);
    assert_eq!(stats.raw_bytes, 0);
    drop(idle);
    until(|| (io.host.take_update()?.pending_roles == 0).then_some(()));
    let hello = Hello::new(source(2).worker(), source(2).lease_epoch() + 1, Lane::Metadata, 0).unwrap();
    let mut foreign = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    foreign.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    foreign.write_all(&hello.encode()).unwrap();
    let mut byte = [0];
    assert!(matches!(foreign.read(&mut byte), Ok(0)));

    assert_eq!(until(|| io.observation(Status::Metadata)).token, incumbent.token);
}
#[test]
fn preview_io_owned_loopback_no_online_source_never_connects_and_full_source_mac_rejects_relog_mismatch() {
    let _serial = owned_fixture_guard();
    let s = source(1);
    let host = HostChannel::start(s.worker().installation, s.worker().host).unwrap();
    let port = until(|| host.port());
    let key = PreviewSecret::synthetic([8; 32]).unwrap();
    let permit = control(s);
    host.publish_permits(&[HostPermit {
        source: s,
        secret: key.clone(),
        control: permit,
        registration: 1,
    }])
    .unwrap();
    let worker = WorkerChannel::start(PreviewBootstrap { port, key }, s.worker(), s.lease_epoch()).unwrap();
    std::thread::sleep(Duration::from_millis(40));
    assert!(worker.connection().is_none());
    assert!(host.take_update().unwrap().sources.is_empty());
    let wrong = control(source(2));
    worker.set_source(Some(SourceLease { control: wrong.clone() })).unwrap();
    worker.publish_state(state(wrong, Some(EpochFence::new()))).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(worker.connection().is_none());
    assert!(host.take_update().unwrap().sources.is_empty());
    worker.stop();
    host.stop();
    until(|| (worker.ended() && host.ended()).then_some(()));
}

#[test]
fn preview_io_owned_loopback_full_reconnect_never_replays_old_commands_or_revives_prior_history() {
    let io = Owned::new(true);
    let old = io.ready();
    let (mut model, handoff, _) = io.capture(PreviewSize::Small);
    let image = handoff
        .admit(&mut model, &io.control, io.host.shared.clock(Instant::now()))
        .unwrap();
    io.worker.set_source(None).unwrap();
    assert!(io.worker.connection().is_none());
    io.worker
        .set_source(Some(SourceLease {
            control: io.control.clone(),
        }))
        .unwrap();
    // An actual CPU owner keeps observing native state while IO waits for its
    // normal reconnect retry. A clone of the startup receipt expires after 1s.
    assert!(!io.state.current(io.state.native_observed_at + DEADLINE));
    let mut next_observation = Instant::now();
    let mut observations = 0;
    let fresh = until(|| {
        let now = Instant::now();
        if now >= next_observation {
            let observed = state(io.control.clone(), io.state.capture.clone());
            assert_eq!(observed.policy, io.state.policy);
            assert!(matches!((&observed.capture, &io.state.capture), (Some(observed), Some(original)) if observed.same(original)));
            match io.worker.publish_state(observed) {
                Ok(()) => {
                    observations += 1;
                    next_observation = now + HEARTBEAT;
                }
                // The real API fails closed on contention. A later synthetic
                // CPU observation may recover; the existing until bound stays fixed.
                Err(Error::Busy) => return None,
                Err(error) => panic!("fresh synthetic native observation rejected: {error:?}"),
            }
        }
        io.observation(Status::Pixels)
            .filter(|o| o.token.connection() > old.token.connection())
    });
    assert!(observations > 0, "reconnect requires newly observed native state");
    assert!(
        image
            .with_pixels(&io.control, io.host.shared.clock(Instant::now()), |_| ())
            .is_none()
    );
    assert!(io.worker.take_command().is_none());
    assert!(io.host.try_command(old.token, PreviewCommand::Stop { demand: 1 }, None).is_err());
    assert!(io.host.take_update().unwrap().sources.into_iter().all(|u| u.frame.is_none()));
    assert_ne!(fresh.token, old.token);
}

#[test]
fn preview_io_owned_loopback_partial_handshake_progress_does_not_reset_absolute_deadline() {
    let _serial = owned_fixture_guard();
    let s = source(1);
    let host = HostChannel::start(s.worker().installation, s.worker().host).unwrap();
    let port = until(|| host.port());
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    stream.write_all(b"A").unwrap();
    until(|| (host.take_update()?.pending_roles == 1).then_some(()));
    std::thread::sleep(Duration::from_millis(850));
    stream.write_all(b"P").unwrap();
    std::thread::sleep(Duration::from_millis(250));
    until(|| (host.take_update()?.pending_roles == 0).then_some(()));
    let mut byte = [0];
    assert!(matches!(stream.read(&mut byte), Ok(0)));
    assert_eq!(host.image_budget().used(), 0);
    host.stop();
    until(|| host.ended().then_some(()));
}

#[test]
fn preview_io_owned_loopback_held_frame_keeps_original_capture_fence_and_cannot_admit_after_local_intent() {
    let io = Owned::new(true);
    let (mut model, handoff, _) = io.capture(PreviewSize::Small);
    handoff.original_capture.witness.revoke();
    assert!(
        handoff
            .admit(&mut model, &io.control, io.host.shared.clock(Instant::now()))
            .is_err()
    );
    assert!(model.retained(io.host.shared.clock(Instant::now())).unwrap().is_none());
}
#[test]
fn preview_io_owned_loopback_explicit_stop_is_capture_only_until_exact_native_demand_ack() {
    let io = Owned::new(true);
    let (mut model, handoff, _) = io.capture(PreviewSize::Small);
    let image = handoff
        .admit(&mut model, &io.control, io.host.shared.clock(Instant::now()))
        .unwrap();
    let token = io.ready().token;
    io.accept_demand(
        1,
        DemandConfiguration {
            size: PreviewSize::Small,
            rate: PreviewRate::One,
        },
    );
    io.host.try_command(token, PreviewCommand::Stop { demand: 1 }, None).unwrap();
    let stop = until(|| io.worker.take_command());
    assert!(matches!(stop.command, PreviewCommand::Stop { demand: 1 }));
    io.worker.acknowledge_stop(stop.token, 1).unwrap();
    until(|| {
        io.host
            .take_update()?
            .sources
            .into_iter()
            .find(|s| s.observation.stopped_demand == Some(1))
    });
    assert!(
        image
            .with_pixels(&io.control, io.host.shared.clock(Instant::now()), |_| ())
            .is_some()
    );
    assert!(io.host.take_update().unwrap().sources.into_iter().all(|s| s.frame.is_none()));
}

#[test]
fn preview_io_owned_loopback_valid_mac_wrong_original_claim_keeps_cache_and_pending_correct_frame() {
    let io = Owned::new(true);
    let (mut model, handoff, _) = io.capture(PreviewSize::Small);
    let old = handoff
        .admit(&mut model, &io.control, io.host.shared.clock(Instant::now()))
        .unwrap();
    let baseline = io.host.image_budget().used();
    let observed = io.ready();
    let geometry = observed.layout.unwrap().geometry(PreviewSize::Small).unwrap();
    let now = io.host.shared.clock(Instant::now());
    let claim = model.request(observed.token.connection(), 1, geometry, now).unwrap();
    io.host
        .try_command(
            observed.token,
            PreviewCommand::Capture(Box::new(claim)),
            Some(CaptureControlGuard {
                control: io.control.clone(),
                mode: crate::session_host::transport::preview_control::CaptureMode::Active {
                    activation: 1,
                    viewport: HostedViewport {
                        host_window: 1,
                        rect: PhysicalRect {
                            x: 0,
                            y: 0,
                            width: 640,
                            height: 360,
                        },
                    },
                },
                witness: EpochFence::new(),
            }),
        )
        .unwrap();
    let command = until(|| io.worker.take_command());
    let mut other = SourceImages::new(source(1), io.host.image_budget()).unwrap();
    other.sync(observed.policy.unwrap(), now).unwrap();
    let wrong = other.request(observed.token.connection(), 1, geometry, now).unwrap();
    assert_ne!(wrong, claim);
    until(|| io.worker.frame_available().then_some(()));
    let (wrong_frame, budget) = frame(PreviewSize::Small);
    io.worker
        .fixture_frame(
            command.token,
            PreviewStamp::new(wrong, 2, 1, 1, geometry).unwrap(),
            wrong_frame,
            Arc::new(AtomicBool::new(true)),
        )
        .unwrap();
    until(|| (budget.used() == 0 && io.host.image_budget().used() == baseline).then_some(()));
    assert!(until(|| io.host.take_update()).sources.into_iter().all(|s| s.frame.is_none()));
    assert_eq!(
        model
            .retained(io.host.shared.clock(Instant::now()))
            .unwrap()
            .unwrap()
            .stamp()
            .sequence(),
        1
    );
    assert!(old.with_pixels(&io.control, io.host.shared.clock(Instant::now()), |_| ()).is_some());
    until(|| io.worker.frame_available().then_some(()));
    let (correct, budget) = frame(PreviewSize::Small);
    io.worker
        .fixture_frame(
            command.token,
            PreviewStamp::new(claim, 3, 1, 1, geometry).unwrap(),
            correct,
            Arc::new(AtomicBool::new(true)),
        )
        .unwrap();
    let correct = until(|| io.host.take_update()?.sources.into_iter().find_map(|s| s.frame));
    let image = correct
        .admit(&mut model, &io.control, io.host.shared.clock(Instant::now()))
        .unwrap();
    assert!(
        image
            .with_pixels(&io.control, io.host.shared.clock(Instant::now()), |_| ())
            .is_some()
    );
    assert_eq!(budget.used(), 0);
}

#[test]
fn preview_io_owned_loopback_pixel_callback_discards_result_when_source_revokes_during_prepare() {
    let io = Owned::new(true);
    let (mut model, handoff, _) = io.capture(PreviewSize::Small);
    let image = handoff
        .admit(&mut model, &io.control, io.host.shared.clock(Instant::now()))
        .unwrap();
    let result = image.with_pixels(&io.control, io.host.shared.clock(Instant::now()), |pixels| {
        assert!(!pixels.is_empty());
        io.control.witness.revoke();
        pixels.len()
    });
    assert!(result.is_none());
}

#[test]
fn preview_io_owned_loopback_old_handoff_and_history_reject_same_source_replacement_control_witness() {
    let io = Owned::new(true);
    let (mut model, handoff, _) = io.capture(PreviewSize::Small);
    let replacement = control(io.control.source);
    assert!(
        handoff
            .admit(&mut model, &replacement, io.host.shared.clock(Instant::now()))
            .is_err()
    );
    let (mut model, handoff, _) = io.capture(PreviewSize::Small);
    let image = handoff
        .admit(&mut model, &io.control, io.host.shared.clock(Instant::now()))
        .unwrap();
    assert!(
        image
            .with_pixels(&replacement, io.host.shared.clock(Instant::now()), |_| ())
            .is_none()
    );
    assert!(
        image
            .with_pixels(&io.control, io.host.shared.clock(Instant::now()), |_| ())
            .is_some()
    );
}
#[test]
fn preview_io_owned_loopback_park_after_host_pixel_accept_before_worker_finish_preserves_history() {
    let mut io = Owned::new(true);
    let old = io.ready();
    let (mut model, handoff, _) = io.capture(PreviewSize::Small);
    let image = handoff
        .admit(&mut model, &io.control, io.host.shared.clock(Instant::now()))
        .unwrap();
    io.park();
    until(|| io.observation(Status::CaptureStopped));
    io.worker.shared.hold_pixel_accept.store(true, Ordering::Release);
    io.restore();
    let registered = until(|| {
        io.observation(Status::Pixels)
            .filter(|o| o.token.pixel_epoch() > old.token.pixel_epoch())
    });
    assert!(io.worker.connection().unwrap().pixel_epoch() < registered.token.pixel_epoch());
    io.park();
    io.worker.shared.hold_pixel_accept.store(false, Ordering::Release);
    io.worker.shared.wake.notify_one();
    until(|| {
        io.observation(Status::CaptureStopped)
            .filter(|o| o.token.pixel_epoch() == registered.token.pixel_epoch())
    });
    assert!(!io.worker.frame_available());
    assert!(
        image
            .with_pixels(&io.control, io.host.shared.clock(Instant::now()), |_| ())
            .is_some()
    );
}

#[test]
fn preview_io_owned_loopback_park_before_pixel_challenge_retires_role_without_losing_history() {
    let mut io = Owned::new(true);
    let old = io.ready();
    let (mut model, handoff, _) = io.capture(PreviewSize::Small);
    let image = handoff
        .admit(&mut model, &io.control, io.host.shared.clock(Instant::now()))
        .unwrap();
    io.park();
    until(|| io.observation(Status::CaptureStopped));
    io.host.shared.hold_pixel_challenge.store(true, Ordering::Release);
    io.restore();
    until(|| io.host.shared.pixel_hello_waiting.load(Ordering::Acquire).then_some(()));
    until(|| {
        io.host
            .take_update()?
            .sources
            .into_iter()
            .find(|u| u.observation.policy.is_some_and(|p| p.access().capture_active))
    });
    io.park();
    until(|| {
        io.host
            .take_update()?
            .sources
            .into_iter()
            .find(|u| u.observation.policy.is_some_and(|p| !p.access().capture_active))
    });
    io.host.shared.hold_pixel_challenge.store(false, Ordering::Release);
    io.host.shared.wake.notify_one();
    until(|| {
        io.observation(Status::CaptureStopped)
            .filter(|o| o.token.pixel_epoch() > old.token.pixel_epoch())
    });
    assert!(!io.worker.frame_available());
    assert!(
        image
            .with_pixels(&io.control, io.host.shared.clock(Instant::now()), |_| ())
            .is_some()
    );
}

#[test]
fn preview_io_v4_exact_native_demand_receipts_and_metadata_peek_leave_owned_frame_untouched() {
    let io = Owned::new(true);
    let config = DemandConfiguration {
        size: PreviewSize::Small,
        rate: PreviewRate::One,
    };
    let receipt = io.accept_demand(1, config);
    let (mut model, frame, _) = io.capture(PreviewSize::Small);
    let budget = io.host.image_budget().used();
    {
        let mut m = io.host.shared.mailbox.lock().unwrap();
        let source = m.updates.iter_mut().find(|u| u.observation.token.source == receipt.source).unwrap();
        source.frame = Some(frame);
        io.host.shared.publication.publish(&mut m).unwrap();
    }
    let first = io.host.try_observations().unwrap();
    let second = io.host.try_observations().unwrap();
    assert_eq!(first.stamp().publisher(), second.stamp().publisher());
    assert!(first.stamp() == second.stamp() || !first.current());
    assert_eq!(io.host.image_budget().used(), budget);
    assert!(
        first
            .sources()
            .iter()
            .any(|o| o.demand_receipt() == Some(&receipt) && o.accepted_demand() == Some(&receipt))
    );
    let frame = io.host.take_update().unwrap().sources.into_iter().find_map(|u| u.frame).unwrap();
    assert!(io.host.take_update().unwrap().sources.into_iter().all(|u| u.frame.is_none()));
    assert!(io.host.try_observations().unwrap().current());
    let image = frame.admit(&mut model, &io.control, io.host.shared.clock(Instant::now())).unwrap();
    assert!(
        image
            .with_pixels(&io.control, io.host.shared.clock(Instant::now()), |bytes| bytes.len())
            .is_some()
    );
    let held = first.head_guard();
    io.host.stop();
    assert!(!held.current());
}
#[test]
fn preview_io_v4_demand_receipt_rejects_wrong_source_connection_config_and_old_terminal_without_retarget() {
    let io = Owned::new(true);
    let config = DemandConfiguration {
        size: PreviewSize::Small,
        rate: PreviewRate::One,
    };
    let first = io.accept_demand(1, config);
    let original_token = io.worker.connection().unwrap();
    let next = DemandConfiguration {
        size: PreviewSize::Large,
        rate: PreviewRate::Five,
    };
    let token = io.ready().token;
    io.host
        .try_command(
            token,
            PreviewCommand::RememberActive {
                demand: 2,
                size: next.size,
                rate: next.rate,
            },
            None,
        )
        .unwrap();
    let command = until(|| io.worker.take_command());
    let good = DemandReceipt {
        demand: 2,
        requested: next,
        outcome: DemandOutcome::Accepted {
            effective: next,
            native_revision: 2,
        },
        ..first
    };
    assert!(
        io.worker
            .try_demand_receipt(command.token, DemandReceipt { source: source(2), ..good })
            .is_err()
    );
    assert!(
        io.worker
            .try_demand_receipt(
                command.token,
                DemandReceipt {
                    metadata_connection: good.metadata_connection + 1,
                    ..good
                }
            )
            .is_err()
    );
    let wrong_config = DemandReceipt {
        requested: config,
        outcome: DemandOutcome::Accepted {
            effective: config,
            native_revision: 2,
        },
        ..good
    };
    assert!(wrong_config.validate().is_ok());
    assert!(io.worker.try_demand_receipt(command.token, wrong_config).is_err());
    io.worker.try_demand_receipt(command.token, good).unwrap();
    until(|| {
        io.host
            .try_observations()
            .ok()?
            .sources()
            .iter()
            .any(|o| o.accepted_demand() == Some(&good))
            .then_some(())
    });
    let before = io.host.try_observations().unwrap().sources()[0].policy().unwrap().origin().serial();
    io.worker.try_demand_receipt(original_token, first).unwrap();
    // The later authenticated heartbeat is queued after the duplicate outcome.
    // Waiting for it proves the host consumed that duplicate, not just its queue.
    until(|| (io.host.try_observations().ok()?.sources()[0].policy()?.origin().serial() > before).then_some(()));
    let latest = io.host.try_observations().unwrap();
    assert!(latest.sources().iter().any(|o| o.accepted_demand() == Some(&good)));
    assert!(io.host.try_command(token, PreviewCommand::Stop { demand: 1 }, None).is_err());
    assert!(io.host.try_command(token, PreviewCommand::Stop { demand: 2 }, None).is_ok());
}

#[test]
fn preview_io_handoff_claim_cancels_only_exact_pending_not_recreated_owner_request_one() {
    let io = Owned::new(true);
    let (_original_model, handoff, _capture_budget) = io.capture(PreviewSize::Small);
    let original = handoff.claim();
    let original_received_at = handoff.received_at;
    let charged = io.host.image_budget().used();
    assert_eq!(charged, original.geometry().payload_bytes());
    assert_eq!(handoff.claim(), original);
    assert_eq!(handoff.received_at, original_received_at);
    assert_eq!(
        io.host.image_budget().used(),
        charged,
        "metadata access cannot copy/refund the held allocation"
    );
    let observed = io.ready();
    let mut replacement = SourceImages::new(original.source(), io.host.image_budget()).unwrap();
    let now = io.host.shared.clock(Instant::now());
    replacement
        .sync_observed(
            observed.policy.unwrap(),
            io.host.shared.clock(observed.policy_received_at.unwrap()),
            now,
        )
        .unwrap();
    let fresh = replacement
        .request(original.connection(), original.demand(), original.geometry(), now)
        .unwrap();
    assert_eq!(fresh.request(), original.request());
    assert_ne!(fresh, original, "model ownership is part of the immutable full claim");
    assert!(!replacement.cancel_capture(handoff.claim()));
    assert!(matches!(
        replacement.request(fresh.connection(), fresh.demand(), fresh.geometry(), now),
        Err(aurora_session::preview_image::ImageError::Busy)
    ));
    assert_eq!(io.host.image_budget().used(), charged);
    drop(handoff);
    assert_eq!(
        io.host.image_budget().used(),
        0,
        "discard releases only its own unadmitted allocation"
    );
    assert!(replacement.cancel_capture(fresh));
    let next = replacement
        .request(fresh.connection(), fresh.demand(), fresh.geometry(), now)
        .unwrap();
    assert_eq!(next.request(), fresh.request() + 1);
}
#[test]
fn preview_io_exported_owned_fixture_guard_excludes_the_existing_mutex_on_another_thread() {
    let held = owned_fixture_guard();
    let blocked = std::thread::scope(|scope| {
        scope
            .spawn(|| matches!(OWNED.try_lock(), Err(std::sync::TryLockError::WouldBlock)))
            .join()
            .unwrap()
    });
    assert!(blocked, "projection and existing IO fixtures must share one exclusion domain");
    drop(held);
    // Reacquire through the same exported entry after releasing it. Other normal
    // fixtures may run between these calls; no second lock/order is introduced.
    drop(owned_fixture_guard());
}
