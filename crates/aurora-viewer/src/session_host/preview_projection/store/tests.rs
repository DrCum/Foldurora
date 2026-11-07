//! Real owned authenticated metadata IO -> production Store -> SourceImages.
//! Synthetic policies/control envelopes exercise protocol/model scope only;
//! they establish no actual native window, renderer or capture authorization.
use super::*;
use crate::session_host::monitor_model::{MonitorSet, Source};
use crate::session_host::preview_io::owned_fixture_guard;
use crate::session_host::preview_transport::{Pair, Received, Scope, TickBudget, WorkerHandshake};
use crate::session_host::transport::preview_control::fixtures::AcceptedControl;
use aurora_session::chat::ChatSource;
use aurora_session::preview_image::{PreviewAccess, PreviewEpochs, PreviewGeometry, PreviewStamp};
use aurora_session::preview_wire::{Hello, Lane, Metadata};
use aurora_session::{HostId, InstallationId, PreviewSize};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpStream};

fn until<T>(mut poll: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(value) = poll() {
            return value;
        }
        assert!(Instant::now() < deadline, "bounded owned metadata fixture timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
struct Peer {
    pair: Pair,
    stream: TcpStream,
    pixels: Option<TcpStream>,
    source: PreviewSource,
    connection: u64,
    key: crate::session_host::preview_bootstrap::PreviewSecret,
    port: u16,
    clock: Instant,
    inbox: VecDeque<Metadata>,
    heartbeat: bool,
    current_policy: Option<PreviewPolicy>,
    last_policy: Instant,
}
impl Peer {
    fn auth(
        port: u16,
        source: PreviewSource,
        key: crate::session_host::preview_bootstrap::PreviewSecret,
        lane: Lane,
        metadata_connection: u64,
    ) -> (TcpStream, crate::session_host::preview_transport::Authenticated) {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(1))).unwrap();
        stream
            .write_all(
                &Hello::new(source.worker(), source.lease_epoch(), lane, metadata_connection)
                    .unwrap()
                    .encode(),
            )
            .unwrap();
        let mut challenge = [0; 56];
        stream.read_exact(&mut challenge).unwrap();
        // Same bounded advertised-context decode as WorkerPending, followed by
        // full mutual HMAC validation. No guessed connection/role identity.
        let connection = u64::from_be_bytes(challenge[8..16].try_into().unwrap());
        let role_epoch = u64::from_be_bytes(challenge[16..24].try_into().unwrap());
        assert_ne!(connection, 0);
        if lane == Lane::Pixels {
            assert_eq!(connection, metadata_connection);
        }
        let scope = Scope {
            source,
            connection,
            role_epoch,
            lane,
        };
        let mut auth = WorkerHandshake::new(scope, key, 0).unwrap();
        stream.write_all(auth.answer(&challenge, 1).unwrap().as_slice()).unwrap();
        let mut accepted = [0; 56];
        stream.read_exact(&mut accepted).unwrap();
        let auth = auth.finish(&accepted, 2).unwrap();
        stream.set_nonblocking(true).unwrap();
        (stream, auth)
    }
    fn new(config: &crate::session_host::transport::WorkerConfig, source: PreviewSource) -> Self {
        let bootstrap = config.preview.as_ref().unwrap();
        let (stream, auth) = Self::auth(bootstrap.port, source, bootstrap.key.clone(), Lane::Metadata, 0);
        let connection = auth.scope().connection;
        Self {
            pair: Pair::metadata(auth, None).unwrap(),
            stream,
            pixels: None,
            source,
            connection,
            key: bootstrap.key.clone(),
            port: bootstrap.port,
            clock: Instant::now(),
            inbox: VecDeque::new(),
            heartbeat: false,
            current_policy: None,
            last_policy: Instant::now(),
        }
    }
    fn pump(&mut self) -> Vec<Received> {
        if self.heartbeat
            && self.last_policy.elapsed() >= Duration::from_millis(250)
            && let Some(policy) = self.current_policy
        {
            self.pair
                .queue_metadata(Metadata::Policy {
                    connection: self.connection,
                    policy,
                })
                .unwrap();
            self.last_policy = Instant::now();
        }
        let packets = self
            .pair
            .poll_roles(
                &mut self.stream,
                self.pixels.as_mut(),
                self.clock.elapsed().as_millis() as u64,
                &mut TickBudget::new(),
            )
            .unwrap();
        for packet in &packets {
            if let Received::Metadata { metadata, .. } = packet {
                assert!(self.inbox.len() < 32, "bounded fake-peer command inbox");
                self.inbox.push_back(*metadata);
            }
        }
        packets
    }
    fn policy(&mut self, policy: PreviewPolicy) {
        self.current_policy = Some(policy);
        self.last_policy = Instant::now();
        self.pair
            .queue_metadata(Metadata::Policy {
                connection: self.connection,
                policy,
            })
            .unwrap();
        let _ = self.pump();
    }
    fn pixels(&mut self) {
        let (stream, auth) = Self::auth(self.port, self.source, self.key.clone(), Lane::Pixels, self.connection);
        self.pair.attach_pixels(auth).unwrap();
        self.pixels = Some(stream);
    }
    fn demand(&mut self) -> aurora_session::preview_wire::DemandReceipt {
        let receipt = until(|| {
            let _ = self.pump();
            let at = self.inbox.iter().position(|metadata| matches!(metadata, Metadata::Demand { .. }))?;
            if let Metadata::Demand {
                source,
                connection,
                demand,
                size,
                rate,
            } = self.inbox.remove(at)?
            {
                let requested = aurora_session::preview_wire::DemandConfiguration { size, rate };
                Some(aurora_session::preview_wire::DemandReceipt {
                    source,
                    metadata_connection: connection,
                    demand,
                    requested,
                    outcome: aurora_session::preview_wire::DemandOutcome::Accepted {
                        effective: requested,
                        native_revision: demand,
                    },
                })
            } else {
                None
            }
        });
        self.pair.queue_metadata(Metadata::DemandReceipt(receipt)).unwrap();
        let _ = self.pump();
        receipt
    }
}
struct Owned {
    _serial: std::sync::MutexGuard<'static, ()>,
    runtime: BrokerProjection,
    reader: PreviewReader,
    channel: Arc<HostChannel>,
    control: AcceptedControl,
    model: MonitorSet,
    descriptor: ViewDescriptor,
    peer: Peer,
}
impl Owned {
    fn new() -> Self {
        let serial = owned_fixture_guard();
        let frontdoor = Arc::new(Frontdoor::default());
        let reader = PreviewReader::new(frontdoor.clone());
        let installation = InstallationId::from_bytes([1; 32]).unwrap();
        let host = HostId::from_bytes([2; 16]).unwrap();
        let mut runtime = BrokerProjection::new(reader.clone(), installation, host, true);
        let bootstrap = until(|| match runtime.bootstrap() {
            BootstrapAdmission::Ready(bootstrap) => Some(bootstrap),
            BootstrapAdmission::Pending => None,
            BootstrapAdmission::Absent => panic!("owned fixture service refused startup"),
        });
        let mut control = AcceptedControl::new(Some(bootstrap));
        assert!(control.accept_status(1, Instant::now()));
        let source = control.source().unwrap();
        let view = control.view().unwrap();
        let presentation = ViewSource {
            slot: SlotId::try_from(0).unwrap(),
            source: source.source(),
            available: true,
        };
        runtime
            .publish_control(vec![(presentation, view)], vec![source.clone()], None)
            .unwrap();
        let permit = HostPermit::from_host(&source, control.config().preview.as_ref().unwrap().key.clone()).unwrap();
        runtime.publish_permits(&[permit]);
        let channel = reader.0.io.lock().unwrap().as_ref().unwrap().1.clone();
        let peer = Peer::new(control.config(), source.source());
        let model_source = Source::new(
            presentation.slot,
            ChatSource::try_from((
                aurora_session::Binding {
                    worker: source.source().worker(),
                    session: Some(source.source().session()),
                },
                source.source().lease_epoch(),
            ))
            .unwrap(),
            true,
        )
        .unwrap();
        let mut model = MonitorSet::new().unwrap();
        model
            .sync(&[model_source], std::array::from_fn(|at| SlotId::try_from(at as u8).unwrap()), None)
            .unwrap();
        let view = model.open_fixed(model_source).unwrap();
        let descriptor = ViewDescriptor {
            view,
            namespace: ViewNamespace::fresh().unwrap(),
            ticket: model.capture_view(view).unwrap(),
            source: source.source(),
            visible: true,
            options: Options::default(),
        };
        let generation = frontdoor.views_intent().unwrap();
        runtime.queue(
            1,
            RequestKind::Views {
                generation,
                views: vec![descriptor],
            },
            reader.reserve().unwrap(),
        );
        runtime.poll(&[]);
        Self {
            _serial: serial,
            runtime,
            reader,
            channel,
            control,
            model,
            descriptor,
            peer,
        }
    }
    fn policy(&self, activation: u64, capture: bool) -> PreviewPolicy {
        PreviewPolicy::new(
            self.descriptor.source,
            PreviewEpochs::new(1, 1, 1, 1, 1).unwrap(),
            PreviewAccess {
                ready: true,
                active: capture,
                activation,
                capture_active: capture,
                retain_last_active: true,
            },
        )
        .unwrap()
    }
    fn publish(&mut self, policy: PreviewPolicy) -> VersionedObservationSet {
        let old = self.channel.try_observations().ok().and_then(|set| {
            set.sources()
                .first()
                .and_then(|source| source.policy().map(|receipt| receipt.origin()))
        });
        self.peer.policy(policy);
        until(|| {
            let _ = self.peer.pump();
            let set = self.channel.try_observations().ok()?;
            let origin = set.sources().first()?.policy()?.origin();
            (Some(origin) != old).then_some(set)
        })
    }
    fn apply(&self, set: &VersionedObservationSet) -> Result<(), PreviewError> {
        let control = self.reader.0.control.try_lock().unwrap().clone();
        let mut locked = self.reader.0.store.try_lock().unwrap();
        let now = self.reader.0.now();
        locked.apply_control(&control)?;
        locked.apply_metadata(
            &self.reader.0,
            &control,
            set,
            self.reader.0.io_generation.load(Ordering::Acquire),
            &self.channel.image_budget(),
            now,
        )
    }
    fn admitted(&self) -> AdmittedImage {
        let mut locked = self.reader.0.store.lock().unwrap();
        let now = self.reader.0.now();
        let entry = locked
            .entries
            .iter_mut()
            .find(|entry| entry.source == self.descriptor.source)
            .unwrap();
        let model = entry.model.as_mut().unwrap();
        let geometry = PreviewGeometry::new([640, 360], [0, 0, 640, 360], PreviewSize::Small).unwrap();
        // Metadata-only admitted fixture, no private pixel/OS permission claim.
        let claim = model.request(self.peer.connection, 1, geometry, now).unwrap();
        model
            .admit(
                PreviewStamp::new(claim, 1, 1, 1, geometry).unwrap(),
                self.channel.image_budget().reserve(geometry).unwrap(),
                now,
            )
            .unwrap()
    }
    fn capture_ready(&mut self) {
        let viewport = HostedViewport {
            host_window: 1,
            rect: aurora_session::PhysicalRect {
                x: 0,
                y: 0,
                width: 640,
                height: 360,
            },
        };
        let activation = self.control.activate(viewport);
        assert_eq!(activation, 1);
        let source = self.control.source().unwrap();
        let view = self.control.view().unwrap();
        self.runtime
            .publish_control(
                vec![(
                    ViewSource {
                        slot: SlotId::try_from(0).unwrap(),
                        source: source.source(),
                        available: true,
                    },
                    view,
                )],
                vec![source.clone()],
                Some(source.source()),
            )
            .unwrap();
        self.runtime
            .publish_permits(&[HostPermit::from_host(&source, self.control.config().preview.as_ref().unwrap().key.clone()).unwrap()]);
        let policy = self.publish(self.policy(activation, true));
        self.apply(&policy).unwrap();
        self.runtime.poll(&[]);
        let receipt = self.peer.demand();
        until(|| {
            let _ = self.peer.pump();
            self.channel
                .try_observations()
                .ok()?
                .sources()
                .iter()
                .any(|source| source.accepted_demand() == Some(&receipt))
                .then_some(())
        });
        self.runtime.poll(&[]);
        let layout = aurora_session::preview_image::PreviewLayout::new(
            self.descriptor.source,
            self.policy(activation, true).epochs(),
            activation,
            receipt.demand,
            1,
            [640, 360],
            [0, 0, 640, 360],
        )
        .unwrap();
        self.peer
            .pair
            .queue_metadata(Metadata::Layout {
                connection: self.peer.connection,
                layout,
            })
            .unwrap();
        let _ = self.peer.pump();
        until(|| {
            let _ = self.peer.pump();
            self.channel
                .try_observations()
                .ok()?
                .sources()
                .iter()
                .any(|source| source.layout().is_some_and(|receipt| receipt.layout() == layout))
                .then_some(())
        });
        self.peer.pixels();
        let set = until(|| {
            let _ = self.peer.pump();
            let set = self.channel.try_observations().ok()?;
            (set.sources().first()?.observation().token.pixel_epoch() > 0).then_some(set)
        });
        self.apply(&set).unwrap();
        self.peer.heartbeat = true;
    }
    fn prepared_frame(&mut self, sequence: u64) -> Arc<PreparedImage> {
        let proof = self.control.capture(&self.runtime, self.reader.0.frontdoor.clone()).unwrap();
        let set = self.channel.try_observations().unwrap();
        self.apply(&set).unwrap();
        let token = set.sources()[0].observation().token;
        let claim = {
            let mut store = self.reader.0.store.lock().unwrap();
            let now = self.reader.0.now();
            let accepted = store.ledger.accepted(token).unwrap();
            let entry = store.entries.iter_mut().find(|entry| entry.source == token.source()).unwrap();
            let geometry = entry.layout.unwrap().layout().geometry(accepted.options.size).unwrap();
            let claim = entry
                .model
                .as_mut()
                .unwrap()
                .request(token.connection(), accepted.demand, geometry, now)
                .unwrap();
            entry.pending = Some(claim);
            entry.pending_at = now;
            entry.pending_layout = entry.layout.map(|receipt| receipt.layout());
            claim
        };
        self.channel
            .try_command(
                token,
                crate::session_host::preview_io::PreviewCommand::Capture(Box::new(claim)),
                Some(CaptureControlGuard::from_host(&proof).unwrap()),
            )
            .unwrap();
        let received = until(|| {
            self.peer.pump().into_iter().find_map(|packet| {
                if let Received::Metadata {
                    metadata: Metadata::Capture(claim),
                    ..
                } = packet
                {
                    Some(claim)
                } else {
                    None
                }
            })
        });
        assert_eq!(received, claim);
        let budget = aurora_render::viewport_capture::CaptureBudget::new(7 * 1024 * 1024).unwrap();
        let boundary = aurora_render::viewport_capture::CaptureBoundary::new(1, 1, (640, 360)).unwrap();
        let mut queue = aurora_render::viewport_capture::CaptureQueue::new(boundary, budget.clone()).unwrap();
        let request = queue
            .request(
                aurora_render::WorldViewport {
                    x: 0,
                    y: 0,
                    width: 640,
                    height: 360,
                },
                aurora_render::viewport_capture::CaptureCap::Small,
                0,
            )
            .unwrap();
        let callback = queue.begin_map(request.token(), boundary, sequence, 1).unwrap();
        let mut pixels = budget.pixels(request.layout()).unwrap();
        pixels.bytes_mut().fill(255);
        callback.complete(true);
        queue.finish_map(request.token(), boundary, pixels.finish().unwrap(), 2).unwrap();
        let Some(aurora_render::viewport_capture::CaptureOutcome::Ready(frame)) = queue.take_result(3).unwrap() else {
            panic!("pure captured fixture")
        };
        let stamp = PreviewStamp::new(claim, sequence, 1, sequence, claim.geometry()).unwrap();
        self.peer
            .pair
            .queue_captured_fixture(stamp, frame, Arc::new(std::sync::atomic::AtomicBool::new(true)))
            .unwrap();
        let prepared = until(|| {
            let _ = self.peer.pump();
            self.runtime.poll(&[]);
            let read = self.reader.try_read(self.descriptor.view, self.descriptor.namespace).ok()?;
            read.prepared().filter(|image| image.admitted().stamp() == stamp).cloned()
        });
        assert_eq!(budget.used(), 0, "moved sender capture credit retired after actual IO completion");
        prepared
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        self.runtime.stop();
        until(|| self.channel.ended().then_some(()));
    }
}

#[test]
fn preview_host_older_authenticated_gui_policy_cannot_purge_newer_activation_history_or_pending_claim() {
    let mut owned = Owned::new();
    let old = owned.publish(owned.policy(3, true));
    owned.apply(&old).unwrap();
    let admitted = owned.admitted();
    std::thread::sleep(Duration::from_millis(2));
    let newer = owned.publish(owned.policy(4, true));
    owned.apply(&newer).unwrap();
    let policy_time = owned.reader.0.observed(newer.sources()[0].policy().unwrap().received_at()).unwrap();
    let claim = {
        let mut store = owned.reader.0.store.lock().unwrap();
        let now = owned.reader.0.now();
        let entry = store
            .entries
            .iter_mut()
            .find(|entry| entry.source == owned.descriptor.source)
            .unwrap();
        let claim = entry
            .model
            .as_mut()
            .unwrap()
            .request(owned.peer.connection, 1, admitted.stamp().geometry(), now)
            .unwrap();
        entry.pending = Some(claim);
        entry.pending_at = now;
        claim
    };
    assert_eq!(owned.apply(&old), Err(PreviewError::Stale));
    assert!(
        admitted.current(policy_time + 999),
        "stale receipt did not shorten newer policy expiry"
    );
    let mut store = owned.reader.0.store.lock().unwrap();
    let now = owned.reader.0.now();
    let entry = store
        .entries
        .iter_mut()
        .find(|entry| entry.source == owned.descriptor.source)
        .unwrap();
    assert_eq!(entry.pending, Some(claim));
    let model = entry.model.as_mut().unwrap();
    assert_eq!(model.retained(now).unwrap().unwrap().stamp(), admitted.stamp());
    assert!(
        model
            .admit(
                PreviewStamp::new(claim, 2, 1, 2, claim.geometry()).unwrap(),
                owned.channel.image_budget().reserve(claim.geometry()).unwrap(),
                now
            )
            .is_ok()
    );
}

#[test]
fn preview_host_outer_pixel_status_and_equal_receipt_do_not_renew_model_policy_deadline() {
    let mut owned = Owned::new();
    let first = owned.publish(owned.policy(1, true));
    owned.apply(&first).unwrap();
    let admitted = owned.admitted();
    let original = *first.sources()[0].policy().unwrap();
    owned.peer.pixels();
    let status = until(|| {
        let _ = owned.peer.pump();
        let set = owned.channel.try_observations().ok()?;
        (set.stamp().serial() > first.stamp().serial() && set.sources().first()?.observation().token.pixel_epoch() > 0).then_some(set)
    });
    assert!(status.sources()[0].policy().copied() == Some(original));
    owned.apply(&status).unwrap();
    owned.apply(&status).unwrap();
    let at = owned.reader.0.observed(original.received_at()).unwrap();
    assert!(
        !admitted.current(at + 1000),
        "status/equal reread must not extend native observation expiry"
    );
    let heartbeat = owned.publish(owned.policy(1, true));
    owned.apply(&heartbeat).unwrap();
    let renewed = owned
        .reader
        .0
        .observed(heartbeat.sources()[0].policy().unwrap().received_at())
        .unwrap();
    assert!(admitted.current(renewed + 999));
    assert!(!admitted.current(renewed + 1000));
}

#[test]
fn preview_host_normal_park_keeps_model_history_and_failed_view_reopen_gets_new_owner_and_namespace() {
    let mut owned = Owned::new();
    let active = owned.publish(owned.policy(1, true));
    owned.apply(&active).unwrap();
    let admitted = owned.admitted();
    let parked = owned.publish(owned.policy(1, false));
    owned.apply(&parked).unwrap();
    assert!(admitted.current(owned.reader.0.now()));
    assert!(
        owned
            .reader
            .try_read(owned.descriptor.view, owned.descriptor.namespace)
            .unwrap()
            .view_current()
    );
    let old_namespace = owned.descriptor.namespace;
    let generation = owned.reader.0.frontdoor.views_intent().unwrap();
    owned.reader.0.frontdoor.view_failed(generation);
    owned.runtime.poll(&[]);
    assert!(!admitted.current(owned.reader.0.now()));
    assert!(owned.reader.try_read(owned.descriptor.view, old_namespace).is_err());
    owned.model.close(owned.descriptor.view).unwrap();
    let source = owned.descriptor.ticket.source();
    let view = owned.model.open_fixed(source).unwrap();
    owned.descriptor.namespace = ViewNamespace::fresh().unwrap();
    owned.descriptor.ticket = owned.model.capture_view(view).unwrap();
    let generation = owned.reader.0.frontdoor.views_intent().unwrap();
    owned.runtime.queue(
        2,
        RequestKind::Views {
            generation,
            views: vec![owned.descriptor],
        },
        owned.reader.reserve().unwrap(),
    );
    owned.runtime.poll(&[]);
    let restored = owned.publish(owned.policy(2, true));
    owned.apply(&restored).unwrap();
    assert_ne!(old_namespace, owned.descriptor.namespace);
    assert!(
        !admitted.current(owned.reader.0.now()),
        "fresh policy cannot revive a retired model's held history"
    );
    assert!(owned.reader.try_read(view, old_namespace).is_err());
    assert!(owned.reader.try_read(view, owned.descriptor.namespace).unwrap().view_current());
    // Accepted control has not been renewed by IO peeks or view recreation.
    assert!(!owned.control.accept_status(1, Instant::now()));
}

#[test]
fn preview_host_new_frame_refreshes_only_the_old_exact_continuously_retained_credited_pair() {
    let mut owned = Owned::new();
    owned.capture_ready();
    let old = owned.prepared_frame(1);
    let old_stamp = old.admitted().stamp();
    let old_age = old.admitted().age();
    let heartbeat = owned.publish(owned.policy(1, true));
    owned.apply(&heartbeat).unwrap();
    assert!(!old.current(), "new authenticated publication retires the old read wrapper");
    let newer = owned.prepared_frame(2);
    assert_ne!(newer.revision(), old.revision());
    let read = owned.reader.try_read(owned.descriptor.view, owned.descriptor.namespace).unwrap();
    let refreshed = read.refresh_prepared(&old).unwrap();
    assert!(refreshed.current());
    assert!(old.same_payload(&refreshed), "refresh retains the exact credited allocation");
    assert_eq!(refreshed.revision(), old.revision());
    assert_eq!(refreshed.admitted().stamp(), old_stamp);
    assert_eq!(refreshed.admitted().age(), old_age);
    assert_ne!(refreshed.admitted().stamp(), newer.admitted().stamp());
}

#[test]
fn preview_host_old_pair_refresh_refuses_a_rebound_view_even_when_source_history_is_still_valid() {
    let mut owned = Owned::new();
    owned.capture_ready();
    let old = owned.prepared_frame(1);
    let old_namespace = owned.descriptor.namespace;
    owned.model.close(owned.descriptor.view).unwrap();
    let view = owned.model.open_fixed(owned.descriptor.ticket.source()).unwrap();
    owned.descriptor.ticket = owned.model.capture_view(view).unwrap();
    owned.descriptor.namespace = ViewNamespace::fresh().unwrap();
    let generation = owned.reader.0.frontdoor.views_intent().unwrap();
    owned.runtime.queue(
        2,
        RequestKind::Views {
            generation,
            views: vec![owned.descriptor],
        },
        owned.reader.reserve().unwrap(),
    );
    owned.runtime.poll(&[]);
    assert!(
        old.admitted().current(owned.reader.0.now()),
        "continuous source history remains admitted"
    );
    let read = owned.reader.try_read(view, owned.descriptor.namespace).unwrap();
    assert!(read.refresh_prepared(&old).is_none());
    assert!(owned.reader.try_read(view, old_namespace).is_err());
}

#[test]
fn preview_host_old_pair_refresh_refuses_authenticated_privacy_denial_and_releases_unpublished_output() {
    let mut owned = Owned::new();
    owned.capture_ready();
    let old = owned.prepared_frame(1);
    let denied = PreviewPolicy::new(
        owned.descriptor.source,
        PreviewEpochs::new(2, 2, 1, 1, 1).unwrap(),
        PreviewAccess {
            activation: 1,
            ..Default::default()
        },
    )
    .unwrap();
    owned.peer.policy(denied);
    until(|| (!old.current()).then_some(()));
    owned.runtime.poll(&[]);
    let read = owned.reader.try_read(owned.descriptor.view, owned.descriptor.namespace).unwrap();
    assert!(read.view_current(), "image privacy does not remove canonical source control");
    assert!(read.refresh_prepared(&old).is_none());
    assert!(!read.image_current());
}

#[test]
fn preview_host_old_pair_refresh_refuses_original_native_policy_expiry_without_any_repaint_renewal() {
    let mut owned = Owned::new();
    owned.capture_ready();
    let old = owned.prepared_frame(1);
    owned.peer.heartbeat = false;
    // No further native policy heartbeat is emitted by this synthetic peer.
    // Repeated real reads cannot extend the one original authenticated receipt.
    until(|| {
        owned.runtime.poll(&[]);
        (!old.current()).then_some(())
    });
    let read = owned.reader.try_read(owned.descriptor.view, owned.descriptor.namespace).unwrap();
    assert!(read.view_current());
    assert!(read.refresh_prepared(&old).is_none());
    assert!(!old.current());
}
