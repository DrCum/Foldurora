use super::auth::Authenticated;
use super::*;
use aurora_session::preview_image::{
    ImageBudget, MAX_IMAGE_BYTES, PreviewAccess, PreviewEpochs, PreviewGeometry, PreviewPolicy, PreviewSource, PreviewStamp, SourceImages,
};
use aurora_session::preview_wire::{self as wire, HEADER_BYTES, Header, Metadata, TAG_BYTES};
use aurora_session::{AccountKey, GridKey, HostId, InstallationId, PreviewSize, SessionId, SessionIdentity, WorkerId, WorkerIdentity};
use std::{
    cell::RefCell,
    collections::VecDeque,
    io::{Read, Write},
    rc::Rc,
};

fn source(id: u8) -> PreviewSource {
    PreviewSource::new(
        WorkerIdentity {
            installation: InstallationId::from_bytes([1; 32]).unwrap(),
            host: HostId::from_bytes([2; 16]).unwrap(),
            worker: WorkerId::from_bytes([id; 16]).unwrap(),
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
fn scope(lane: Lane) -> Scope {
    Scope {
        source: source(3),
        connection: 8,
        lane,
    }
}
fn secret() -> PreviewSecret {
    PreviewSecret::synthetic([7; 32]).unwrap()
}
fn sessions(lane: Lane) -> (Authenticated, Authenticated) {
    let mut host = HostHandshake::new(scope(lane), secret(), 0).unwrap();
    let mut worker = WorkerHandshake::new(scope(lane), secret(), 0).unwrap();
    let answer = worker.answer(host.challenge().as_slice(), 1).unwrap();
    let (accepted, host) = host.accept(answer.as_slice(), 2).unwrap();
    (host, worker.finish(accepted.as_slice(), 3).unwrap())
}
fn policy() -> PreviewPolicy {
    PreviewPolicy::new(
        source(3),
        PreviewEpochs::new(1, 1, 1, 1, 1).unwrap(),
        PreviewAccess {
            ready: true,
            active: true,
            activation: 1,
            capture_active: true,
            retain_last_active: true,
        },
    )
    .unwrap()
}

fn color(index: usize) -> [u8; 4] {
    [(index % 251) as u8, ((index / 7) % 253) as u8, ((index / 13) % 255) as u8, 255]
}

#[derive(Default)]
struct Channel {
    bytes: VecDeque<u8>,
    closed: bool,
}
struct Pipe {
    input: Rc<RefCell<Channel>>,
    output: Rc<RefCell<Channel>>,
    read: usize,
    write: usize,
    stalled: bool,
    interrupted: bool,
    calls: usize,
}
impl Pipe {
    fn pair(read: usize, write: usize) -> (Self, Self) {
        let a = Rc::new(RefCell::new(Channel::default()));
        let b = Rc::new(RefCell::new(Channel::default()));
        (
            Self {
                input: a.clone(),
                output: b.clone(),
                read,
                write,
                stalled: false,
                interrupted: false,
                calls: 0,
            },
            Self {
                input: b,
                output: a,
                read,
                write,
                stalled: false,
                interrupted: false,
                calls: 0,
            },
        )
    }
}
impl Read for Pipe {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.interrupted {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let mut channel = self.input.borrow_mut();
        if channel.bytes.is_empty() {
            return if channel.closed {
                Ok(0)
            } else {
                Err(io::ErrorKind::WouldBlock.into())
            };
        }
        let count = output.len().min(self.read).min(channel.bytes.len());
        for byte in &mut output[..count] {
            *byte = channel.bytes.pop_front().unwrap();
        }
        Ok(count)
    }
}
impl Write for Pipe {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.interrupted {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.stalled {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let mut channel = self.output.borrow_mut();
        if channel.closed {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let count = input.len().min(self.write);
        channel.bytes.extend(&input[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Harness {
    host: Pair,
    worker: Pair,
    host_meta: Pipe,
    worker_meta: Pipe,
    host_pixels: Pipe,
    worker_pixels: Pipe,
    budget: ImageBudget,
    stamp: PreviewStamp,
    images: SourceImages,
}
impl Harness {
    fn new(read: usize, write: usize, cap: PreviewSize, large: bool) -> Self {
        Self::with_budget(read, write, cap, large, ImageBudget::new(4 * MAX_IMAGE_BYTES).unwrap())
    }
    fn with_budget(read: usize, write: usize, cap: PreviewSize, large: bool, budget: ImageBudget) -> Self {
        let mut images = SourceImages::new(source(3), budget.clone()).unwrap();
        images.sync(policy(), 0).unwrap();
        let geometry = if large {
            PreviewGeometry::new([16, 9], [0, 0, 16, 9], cap).unwrap()
        } else {
            PreviewGeometry::new([1, 45], [0, 0, 1, 45], cap).unwrap()
        };
        let claim = images.request(8, 1, geometry, 1).unwrap();
        let stamp = PreviewStamp::new(claim, 1, 1, 1, geometry).unwrap();
        let (hm, wm) = sessions(Lane::Metadata);
        let (hp, wp) = sessions(Lane::Pixels);
        let (host_meta, worker_meta) = Pipe::pair(read, write);
        let (host_pixels, worker_pixels) = Pipe::pair(read, write);
        Self {
            host: Pair::new(hm, hp, budget.clone()).unwrap(),
            worker: Pair::new(wm, wp, budget.clone()).unwrap(),
            host_meta,
            worker_meta,
            host_pixels,
            worker_pixels,
            budget,
            stamp,
            images,
        }
    }
    fn queue(&mut self) {
        self.worker
            .queue_metadata(Metadata::Policy {
                connection: 8,
                policy: policy(),
            })
            .unwrap();
        let mut raw = vec![0; self.stamp.geometry().payload_bytes()];
        for (index, pixel) in raw.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            pixel.copy_from_slice(&color(index));
        }
        self.worker
            .queue_pixels(UnadmittedPixels::copy_from(self.stamp, &raw, &self.budget).unwrap())
            .unwrap();
    }
    fn worker_tick(&mut self, now: u64) -> Result<Vec<Received>, Error> {
        self.worker
            .poll(&mut self.worker_meta, &mut self.worker_pixels, now, &mut TickBudget::new())
    }
    fn host_tick(&mut self, now: u64) -> Result<Vec<Received>, Error> {
        self.host
            .poll(&mut self.host_meta, &mut self.host_pixels, now, &mut TickBudget::new())
    }
    fn receive(&mut self) -> UnadmittedPixels {
        self.receive_at(10)
    }
    fn receive_at(&mut self, now: u64) -> UnadmittedPixels {
        for _ in 0..20_000 {
            self.worker_tick(now).unwrap();
            for packet in self.host_tick(now).unwrap() {
                if let Received::UnadmittedPixels(pixels) = packet {
                    return pixels;
                }
            }
        }
        panic!("bounded synthetic image did not finish");
    }
    fn fill_wire(&mut self) {
        for _ in 0..100 {
            self.worker_tick(10).unwrap();
            if self.budget.used() == 0 {
                return;
            }
        }
        panic!("synthetic writer did not finish");
    }
}

#[test]
fn preview_auth_mutual_roles_domains_source_lease_and_nonce_replay_fail_closed() {
    assert!(PreviewSecret::random().is_ok());
    assert_eq!(format!("{:?}", secret()), "PreviewSecret([redacted])");
    for wrong in 0..12 {
        let mut host = HostHandshake::new(scope(Lane::Metadata), secret(), 0).unwrap();
        let challenge = host.challenge();
        let mut altered = scope(Lane::Metadata);
        let mut key = secret();
        match wrong {
            0 => altered.lane = Lane::Pixels,
            1 => altered.connection += 1,
            2 => altered.source = source(9),
            3 => {
                let old = altered.source;
                altered.source = PreviewSource::new(old.worker(), old.session(), old.lease_epoch() + 1).unwrap();
            }
            4 => key = PreviewSecret::synthetic([9; 32]).unwrap(),
            5 => {}
            6..=11 => {
                let old = altered.source;
                let mut worker = old.worker();
                let mut session = old.session();
                match wrong {
                    6 => session.account = AccountKey::from_bytes([9; 32]).unwrap(),
                    7 => session.grid = GridKey::from_bytes([9; 32]).unwrap(),
                    8 => session.generation += 1,
                    9 => session.session = SessionId::from_bytes([9; 16]).unwrap(),
                    10 => worker.host = HostId::from_bytes([9; 16]).unwrap(),
                    11 => worker.installation = InstallationId::from_bytes([9; 32]).unwrap(),
                    _ => unreachable!(),
                }
                altered.source = PreviewSource::new(worker, session, old.lease_epoch()).unwrap();
            }
            _ => unreachable!(),
        }
        let mut worker = WorkerHandshake::new(altered, key, 0).unwrap();
        if let Ok(answer) = worker.answer(challenge.as_slice(), 1) {
            if wrong == 5 {
                let mut replacement = HostHandshake::new(scope(Lane::Metadata), secret(), 0).unwrap();
                assert!(replacement.accept(answer.as_slice(), 2).is_err());
            } else {
                assert!(host.accept(answer.as_slice(), 2).is_err());
            }
        } else {
            assert!(matches!(wrong, 0 | 1));
        }
    }
    let mut host = HostHandshake::new(scope(Lane::Pixels), secret(), 0).unwrap();
    let mut worker = WorkerHandshake::new(scope(Lane::Pixels), secret(), 0).unwrap();
    let answer = worker.answer(host.challenge().as_slice(), 1).unwrap();
    let mut reflected = answer.as_slice()[..16].to_vec();
    reflected[7] = 3;
    reflected.extend_from_slice(&answer.as_slice()[48..80]);
    assert!(
        worker.finish(&reflected, 2).is_err(),
        "a correctly shaped reflected worker MAC cannot authenticate the host role"
    );
    let (_, session) = host.accept(answer.as_slice(), 2).unwrap();
    assert!(host.accept(answer.as_slice(), 3).is_err());
    drop(session);
}

#[test]
fn preview_auth_every_partial_proof_and_absolute_deadline_reject_before_session() {
    let host = HostHandshake::new(scope(Lane::Metadata), secret(), 0).unwrap();
    let challenge = host.challenge();
    for cut in 0..challenge.as_slice().len() {
        let mut worker = WorkerHandshake::new(scope(Lane::Metadata), secret(), 0).unwrap();
        assert!(worker.answer(&challenge.as_slice()[..cut], 1).is_err());
    }
    let mut worker = WorkerHandshake::new(scope(Lane::Metadata), secret(), 0).unwrap();
    let answer = worker.answer(challenge.as_slice(), 1).unwrap();
    for cut in 0..answer.as_slice().len() {
        let mut host = HostHandshake::new(scope(Lane::Metadata), secret(), 0).unwrap();
        assert!(host.accept(&answer.as_slice()[..cut], 2).is_err());
    }
    let mut host = HostHandshake::new(scope(Lane::Metadata), secret(), 0).unwrap();
    let mut worker = WorkerHandshake::new(scope(Lane::Metadata), secret(), 0).unwrap();
    let answer = worker.answer(host.challenge().as_slice(), 999).unwrap();
    assert!(host.accept(answer.as_slice(), 1000).is_err());
    assert!(
        host.accept(answer.as_slice(), 2).is_err(),
        "an observed expiry is terminal even if a later caller supplies an older clock"
    );
    assert!(
        worker.answer(challenge.as_slice(), 999).is_err(),
        "progress cannot reset the handshake deadline/state"
    );
}

fn signed_frame(auth: &Authenticated, header: Header, metadata: &[u8], pixels: &[u8]) -> Vec<u8> {
    let tag = auth.send.header(header, metadata);
    let mut body = auth.send.body(header, metadata, &tag);
    body.update(pixels);
    let finish = auth.send.finish(body);
    let mut bytes = header.encode().to_vec();
    bytes.extend_from_slice(metadata);
    bytes.extend_from_slice(&tag);
    bytes.extend_from_slice(pixels);
    bytes.extend_from_slice(&finish);
    bytes
}

#[test]
fn preview_stream_authenticated_wrong_source_or_invalid_geometry_is_rejected_before_allocation() {
    for wrong_source in [true, false] {
        let budget = ImageBudget::new(MAX_IMAGE_BYTES).unwrap();
        let (hm, wm) = sessions(Lane::Metadata);
        let (hp, wp) = sessions(Lane::Pixels);
        let mut host = Pair::new(hm, hp, budget.clone()).unwrap();
        let (mut meta, _) = Pipe::pair(4096, 4096);
        let (mut pixels, _) = Pipe::pair(4096, 4096);
        let message = Metadata::Policy {
            connection: 8,
            policy: policy(),
        };
        let metadata = wire::encode_metadata(message).unwrap();
        let header = Header::new(Lane::Metadata, Direction::WorkerToHost, 8, 1, metadata.as_slice().len(), 0).unwrap();
        meta.input
            .borrow_mut()
            .bytes
            .extend(signed_frame(&wm, header, metadata.as_slice(), &[]));
        let mut images = SourceImages::new(if wrong_source { source(9) } else { source(3) }, budget.clone()).unwrap();
        let p = PreviewPolicy::new(
            if wrong_source { source(9) } else { source(3) },
            policy().epochs(),
            policy().access(),
        )
        .unwrap();
        images.sync(p, 0).unwrap();
        let geometry = PreviewGeometry::new([1, 45], [0, 0, 1, 45], PreviewSize::Small).unwrap();
        let claim = images.request(8, 1, geometry, 1).unwrap();
        let stamp = PreviewStamp::new(claim, 1, 1, 1, geometry).unwrap();
        let encoded = wire::encode_stamp(stamp).unwrap();
        let mut bytes = encoded.as_slice().to_vec();
        if !wrong_source {
            let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            value["geometry"]["size"] = serde_json::json!([u32::MAX, 180]);
            bytes = serde_json::to_vec(&value).unwrap();
        }
        let raw = vec![255; geometry.payload_bytes()];
        let header = Header::new(Lane::Pixels, Direction::WorkerToHost, 8, 1, bytes.len(), raw.len()).unwrap();
        pixels.input.borrow_mut().bytes.extend(signed_frame(&wp, header, &bytes, &raw));
        assert!(host.poll(&mut meta, &mut pixels, 10, &mut TickBudget::new()).is_err());
        assert_eq!(budget.used(), 0);
    }
}

#[test]
fn preview_stream_reordered_or_replayed_frame_retires_held_pixels_without_reopening() {
    let mut h = Harness::new(4096, 4096, PreviewSize::Small, false);
    h.queue();
    h.fill_wire();
    let replay = h.host_pixels.input.borrow().bytes.clone();
    let mut held = None;
    for packet in h.host_tick(10).unwrap() {
        if let Received::UnadmittedPixels(pixels) = packet {
            held = Some(pixels);
        }
    }
    let held = held.unwrap();
    assert!(held.transport_current());
    h.host_pixels.input.borrow_mut().bytes.extend(replay);
    assert!(matches!(h.host_tick(20), Err(Error::Sequence)));
    assert!(held.test_bytes().is_none());
    drop(held);
    assert_eq!(h.budget.used(), 0);
}

#[test]
fn preview_stream_zero_byte_write_and_source_reconnect_drop_payload_credit() {
    let mut h = Harness::new(4096, 0, PreviewSize::Small, false);
    h.queue();
    assert!(matches!(h.worker_tick(10), Err(Error::Closed)));
    assert_eq!(h.budget.used(), 0);
    let (mut host, worker) = sessions(Lane::Metadata);
    let (hp, _) = sessions(Lane::Pixels);
    host.scope.connection += 1;
    assert!(matches!(Pair::new(host, hp, h.budget.clone()), Err(Error::Binding)));
    drop(worker);
    let mut fresh = Harness::new(4096, 4096, PreviewSize::Small, false);
    assert!(
        fresh.host_tick(10).unwrap().is_empty(),
        "reauthentication supplies no old frame or SourceImages admission"
    );
    let mut old = Harness::new(4096, 4096, PreviewSize::Small, false);
    old.queue();
    old.fill_wire();
    fresh
        .host_meta
        .input
        .borrow_mut()
        .bytes
        .extend(old.host_meta.input.borrow().bytes.iter().copied());
    fresh
        .host_pixels
        .input
        .borrow_mut()
        .bytes
        .extend(old.host_pixels.input.borrow().bytes.iter().copied());
    assert!(
        matches!(fresh.host_tick(20), Err(Error::Authentication)),
        "old connection bytes fail the new nonce-derived MAC before allocation"
    );
    assert_eq!(fresh.budget.used(), 0);
}

#[test]
fn preview_stream_fragmented_read_write_matrix_roundtrips_exact_opaque_pixels_and_shared_credit() {
    for read in [1, 2, 3, 7, 64, 4096] {
        for write in [1, 2, 5, 31, 4096] {
            let mut harness = Harness::new(read, write, PreviewSize::Small, false);
            harness.queue();
            let pixels = harness.receive();
            assert_eq!(pixels.test_bytes().unwrap().len(), harness.stamp.geometry().payload_bytes());
            assert!(
                pixels
                    .test_bytes()
                    .unwrap()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .enumerate()
                    .all(|(index, pixel)| *pixel == color(index))
            );
            assert_eq!(harness.budget.used(), harness.stamp.geometry().payload_bytes());
            drop(pixels);
            assert_eq!(harness.budget.used(), 0);
        }
    }
}

#[test]
fn preview_stream_header_mac_precedes_shared_credit_reservation_and_full_mac_failure_purges() {
    for header_tag in [true, false] {
        let mut h = Harness::new(4096, 4096, PreviewSize::Small, false);
        h.queue();
        h.fill_wire();
        assert_eq!(h.budget.used(), 0);
        {
            let mut queue = h.host_pixels.input.borrow_mut();
            let bytes = queue.bytes.make_contiguous();
            let header = Header::decode(&bytes[..HEADER_BYTES]).unwrap();
            let index = if header_tag {
                HEADER_BYTES + header.metadata_bytes() + TAG_BYTES - 1
            } else {
                bytes.len() - 1
            };
            bytes[index] ^= 1;
        }
        assert!(matches!(h.host_tick(10), Err(Error::Authentication)));
        assert_eq!(h.budget.used(), 0);
    }
}

#[test]
fn preview_stream_partial_payload_eof_absolute_timeout_and_alpha_failure_drop_all_credit() {
    for failure in 0..3 {
        let mut h = Harness::new(4096, 4096, PreviewSize::Small, false);
        h.queue();
        h.fill_wire();
        {
            let mut channel = h.host_pixels.input.borrow_mut();
            let header = Header::decode(channel.bytes.make_contiguous().get(..HEADER_BYTES).unwrap()).unwrap();
            let pixel_start = HEADER_BYTES + header.metadata_bytes() + TAG_BYTES;
            if failure == 2 {
                channel.bytes.make_contiguous()[pixel_start + 3] = 0;
            } else {
                channel.bytes.truncate(pixel_start + 16);
                channel.closed = failure == 0;
            }
        }
        let result = h.host_tick(10);
        if failure == 1 {
            assert!(result.is_ok());
            assert!(h.budget.used() > 0);
            h.worker
                .queue_metadata(Metadata::Policy {
                    connection: 8,
                    policy: policy(),
                })
                .unwrap();
            h.worker_tick(1009).unwrap();
            h.host_tick(1009).unwrap();
            assert!(matches!(h.host_tick(1010), Err(Error::Expired)));
        } else {
            assert!(result.is_err());
        }
        assert_eq!(h.budget.used(), 0);
    }
}

#[test]
fn preview_stream_metadata_revoke_overtakes_stalled_image_and_allowed_cannot_resurrect_it() {
    let mut h = Harness::new(4096, 4096, PreviewSize::Small, false);
    h.queue();
    h.worker_pixels.stalled = true;
    h.worker_tick(10).unwrap();
    h.host_tick(10).unwrap();
    assert!(h.budget.used() > 0);
    let denied = PreviewPolicy::new(
        source(3),
        PreviewEpochs::new(2, 2, 1, 1, 1).unwrap(),
        PreviewAccess {
            activation: 1,
            ..Default::default()
        },
    )
    .unwrap();
    h.worker
        .queue_metadata(Metadata::Policy {
            connection: 8,
            policy: denied,
        })
        .unwrap();
    assert_eq!(h.budget.used(), 0);
    let restored = PreviewPolicy::new(source(3), PreviewEpochs::new(3, 2, 1, 1, 1).unwrap(), policy().access()).unwrap();
    h.worker
        .queue_metadata(Metadata::Policy {
            connection: 8,
            policy: restored,
        })
        .unwrap();
    h.worker_tick(20).unwrap();
    let metadata = h.host_tick(20).unwrap();
    assert!(
        metadata
            .iter()
            .any(|packet| matches!(packet,Received::Metadata(Metadata::Policy {policy,..})if !policy.access().retain_last_active))
    );
    let raw = vec![255; h.stamp.geometry().payload_bytes()];
    assert!(matches!(
        h.worker
            .queue_pixels(UnadmittedPixels::copy_from(h.stamp, &raw, &h.budget).unwrap()),
        Err(Error::Closed)
    ));
    assert_eq!(h.budget.used(), 0);
}

#[test]
fn preview_stream_metadata_eof_silence_and_pair_drop_invalidate_unhanded_images() {
    for failure in 0..3 {
        let mut h = Harness::new(4096, 4096, PreviewSize::Small, false);
        h.queue();
        let pixels = h.receive();
        assert!(pixels.transport_current());
        match failure {
            0 => {
                h.host_meta.input.borrow_mut().closed = true;
                assert!(h.host_tick(20).is_err());
            }
            1 => {
                assert!(matches!(h.host_tick(1010), Err(Error::Expired)));
            }
            2 => {
                drop(h.host);
            }
            _ => unreachable!(),
        }
        assert!(pixels.test_bytes().is_none());
        assert!(h.budget.used() > 0, "held allocations stay charged until their owner drops them");
        drop(pixels);
        assert_eq!(h.budget.used(), 0);
    }
}

#[test]
fn preview_stream_normal_park_retains_only_already_source_admitted_history() {
    let mut h = Harness::new(4096, 4096, PreviewSize::Small, false);
    h.queue();
    let received = h.receive();
    let admitted: AdmittedPixels = received.admit(&mut h.images, 10).unwrap();
    assert!(admitted.image().current(10));
    assert!(!admitted.bytes(10).unwrap().is_empty());
    let parked = PreviewPolicy::new(
        source(3),
        policy().epochs(),
        PreviewAccess {
            ready: true,
            active: false,
            activation: 2,
            capture_active: false,
            retain_last_active: true,
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
    for packet in h.host_tick(20).unwrap() {
        if let Received::Metadata(Metadata::Policy { policy, .. }) = packet {
            h.images.sync(policy, 20).unwrap();
        }
    }
    assert!(h.images.retained(20).unwrap().is_some());
    assert!(admitted.image().current(20));
    assert!(admitted.bytes(20).is_some());
    h.host_meta.input.borrow_mut().closed = true;
    assert!(h.host_tick(30).is_err());
    // Future adapter's mandatory error handling: loss retires SourceImages;
    // a capture-grant change above deliberately did not do that.
    h.images.retire();
    assert!(!admitted.image().current(30));
    assert!(admitted.bytes(30).is_none());
    drop(admitted);
    assert_eq!(h.budget.used(), 0);
}

enum CorruptCapture {
    Policy,
    Activation,
    Claim,
    NativeSequence,
}

fn reject_authenticated_capture_without_cache_replacement(corruption: CorruptCapture) {
    let mut h = Harness::new(4096, 4096, PreviewSize::Small, false);
    h.queue();
    let cached = h.receive().admit(&mut h.images, 10).unwrap();
    let original = cached.image().stamp();
    let charged = h.budget.used();
    assert_eq!(charged, original.geometry().payload_bytes());
    let pending = h.images.request(8, 1, original.geometry(), 20).unwrap();
    let mut fields = serde_json::to_value(pending).unwrap();
    match corruption {
        CorruptCapture::Policy => fields["epochs"]["revision"] = 2.into(),
        CorruptCapture::Activation => fields["activation"] = 2.into(),
        CorruptCapture::Claim => fields["request"] = (pending.request() + 1).into(),
        CorruptCapture::NativeSequence => {}
    }
    // Existing checked serde constructors validate the forged descriptor's
    // shape. It is self-consistent and genuinely MACed by the authenticated
    // worker, but it is not the captured authority/request issued by this host.
    let forged = serde_json::from_value(fields).unwrap();
    let sequence = if matches!(corruption, CorruptCapture::NativeSequence) {
        1
    } else {
        2
    };
    let descriptor = PreviewStamp::new(forged, sequence, 1, 2, original.geometry()).unwrap();
    let raw = vec![255; original.geometry().payload_bytes()];
    h.worker
        .queue_pixels(UnadmittedPixels::copy_from(descriptor, &raw, &h.budget).unwrap())
        .unwrap();
    let unadmitted = h.receive_at(30);
    assert!(
        unadmitted.transport_current(),
        "MAC/scope freshness is intentionally separate from capture authority"
    );
    assert_eq!(h.budget.used(), 2 * charged);
    assert!(matches!(
        unadmitted.admit(&mut h.images, 30),
        Err(aurora_session::preview_image::ImageError::Stale)
    ));
    assert_eq!(
        h.budget.used(),
        charged,
        "owned rejection drops bytes and refunds only the rejected frame"
    );
    assert_eq!(h.images.retained(30).unwrap().unwrap().stamp(), original);
    assert_eq!(cached.image().stamp(), original);
    assert!(
        cached
            .bytes(30)
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .all(|(index, pixel)| *pixel == color(index))
    );

    // A rejected native sequence/claim cannot advance the model's accepted
    // sequence or consume its original pending request. The next fresh wire
    // packet can deliver that exact still-current request successfully.
    let correct = PreviewStamp::new(pending, 2, 1, 3, original.geometry()).unwrap();
    h.worker
        .queue_pixels(UnadmittedPixels::copy_from(correct, &raw, &h.budget).unwrap())
        .unwrap();
    let replacement = h.receive_at(40).admit(&mut h.images, 40).unwrap();
    assert_eq!(h.images.retained(40).unwrap().unwrap().stamp(), correct);
    assert_eq!(replacement.image().stamp(), correct);
    assert!(replacement.bytes(40).unwrap().iter().all(|byte| *byte == 255));
    h.images.retire();
    drop(cached);
    drop(replacement);
    assert_eq!(h.budget.used(), 0);
}

#[test]
fn preview_admission_rejects_valid_mac_wrong_capture_policy_without_replacing_cache() {
    reject_authenticated_capture_without_cache_replacement(CorruptCapture::Policy);
}
#[test]
fn preview_admission_rejects_valid_mac_wrong_activation_without_replacing_cache() {
    reject_authenticated_capture_without_cache_replacement(CorruptCapture::Activation);
}
#[test]
fn preview_admission_rejects_valid_mac_wrong_claim_without_replacing_cache() {
    reject_authenticated_capture_without_cache_replacement(CorruptCapture::Claim);
}
#[test]
fn preview_admission_rejects_valid_mac_stale_native_sequence_without_replacing_cache() {
    reject_authenticated_capture_without_cache_replacement(CorruptCapture::NativeSequence);
}

#[test]
fn preview_stream_shared_credit_exhaustion_rejects_before_pixel_allocation_and_refunds_on_drop() {
    let budget = ImageBudget::new(2880).unwrap();
    let mut h = Harness::with_budget(4096, 4096, PreviewSize::Small, false, budget.clone());
    h.queue();
    h.fill_wire();
    let hold = budget.reserve(h.stamp.geometry()).unwrap();
    assert!(matches!(h.host_tick(10), Err(Error::Capacity)));
    assert_eq!(budget.used(), 2880);
    drop(hold);
    assert_eq!(budget.used(), 0);
}

#[test]
fn preview_admission_caps_pairs_and_pending_roles_and_rejects_incumbent_replacement() {
    let base = source(3);
    let mut admissions = Admissions::new(base.worker().installation, base.worker().host);
    let mut slots = Vec::new();
    for worker in 1..=5 {
        for lane in [Lane::Metadata, Lane::Pixels] {
            let scope = Scope {
                source: source(worker),
                connection: 8,
                lane,
            };
            slots.push((admissions.begin(scope).unwrap(), scope));
        }
    }
    assert_eq!(slots.len(), 10);
    assert!(admissions.begin(scope(Lane::Metadata)).is_err());
    let (pixel_index, pixel_scope) = slots[1];
    assert!(admissions.finish(pixel_index, pixel_scope).is_err());
    let (meta_index, meta_scope) = slots[0];
    admissions.finish(meta_index, meta_scope).unwrap();
    admissions.finish(pixel_index, pixel_scope).unwrap();
    admissions.release(meta_index, meta_scope);
    let fresh = Scope {
        connection: 9,
        ..meta_scope
    };
    let fresh_index = admissions.begin(fresh).unwrap();
    admissions.release(meta_index, meta_scope);
    admissions.finish(fresh_index, fresh).unwrap();
    assert!(admissions.begin(fresh).is_err(), "late old release cannot remove a new incumbent");
}

#[test]
fn preview_poll_byte_budget_is_shared_and_rotation_covers_all_five_slots() {
    let mut order = RoundRobin::default();
    let mut seen = [false; 5];
    for _ in 0..5 {
        let mut budget = TickBudget::new();
        let indices = order.order(5).unwrap();
        for index in indices {
            let before = budget.remaining_bytes();
            let mut h = Harness::new(usize::MAX, usize::MAX, PreviewSize::Large, true);
            h.queue();
            h.worker.poll(&mut h.worker_meta, &mut h.worker_pixels, 10, &mut budget).unwrap();
            let spent = before - budget.remaining_bytes();
            assert!(spent <= 64 * 1024);
            if spent > 0 {
                seen[index] = true;
            }
        }
        assert!(budget.remaining_bytes() <= 256 * 1024);
    }
    assert!(seen.into_iter().all(|seen| seen));
    assert!(order.order(6).is_err());
}

#[test]
fn preview_stream_interrupted_io_and_repeated_empty_polls_are_bounded() {
    let mut h = Harness::new(1, 1, PreviewSize::Small, false);
    h.host_meta.interrupted = true;
    h.worker_meta.interrupted = true;
    for now in [0, 1, 2] {
        let before = h.host_meta.calls;
        let mut budget = TickBudget::new();
        assert!(
            h.host
                .poll(&mut h.host_meta, &mut h.host_pixels, now, &mut budget)
                .unwrap()
                .is_empty()
        );
        assert_eq!(budget.remaining_bytes(), 256 * 1024);
        assert!(h.host_meta.calls - before <= 16);
    }
    h.host_meta.interrupted = false;
    h.host_pixels.input.borrow_mut().closed = true;
    assert!(
        h.host_tick(1000).is_err(),
        "no authenticated Policy heartbeat expires even without a first pixel byte"
    );
    assert_eq!(h.budget.used(), 0);
}
