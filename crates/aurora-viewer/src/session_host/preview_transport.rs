//! Preview-only nonblocking sockets. Authentication uses fixed endpoint
//! purposes; the small control stream cannot wait behind a partial RGBA image.
//! All methods belong on IO workers, never the App or software-window thread.

use super::bridge_auth::{ClientAuth, ClientStep, HANDSHAKE_DEADLINE, MAX_AUTH_BYTES, Permit, Purpose, ServerAuth};
use super::preview_protocol::{self as wire, Header};
use aurora_session::{HostId, InstallationId, MAX_WORKERS, WorkerIdentity};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::time::Instant;

const PREFIX: usize = 4;
const MAX_OPERATIONS: usize = 64;
const MAX_MESSAGES: usize = 16;
const MAX_CONTROL_QUEUE: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Control,
    Image,
}

impl Kind {
    fn purpose(self) -> Purpose {
        match self {
            Self::Control => Purpose::PreviewControl,
            Self::Image => Purpose::PreviewImage,
        }
    }

    fn maximum(self) -> usize {
        match self {
            Self::Control => wire::MAX_HEADER + 14,
            Self::Image => wire::MAX_PACKET,
        }
    }

    fn outgoing(self, worker: bool, header: &Header) -> bool {
        matches!(
            (self, worker, header),
            (Self::Control, false, Header::Subscribe(_) | Header::Probe(_) | Header::Suspend(_))
                | (
                    Self::Control,
                    true,
                    Header::State(_) | Header::Probe(_) | Header::Sample(_) | Header::Peer(_) | Header::Suspended(_)
                )
                | (Self::Image, true, Header::Image(_))
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Busy,
    NotReady,
    Closed,
    Authentication,
    Binding,
    Direction,
    Malformed,
    Io(io::ErrorKind),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Busy => "preview lane is busy",
            Self::NotReady => "preview lane is not authenticated",
            Self::Closed => "preview lane is closed",
            Self::Authentication => "preview lane authentication failed",
            Self::Binding => "preview lane binding mismatch",
            Self::Direction => "preview lane direction mismatch",
            Self::Malformed => "preview lane frame is invalid",
            Self::Io(_) => "preview lane IO failed",
        })
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

pub(crate) struct Packet {
    pub header: Header,
    pub rgba: Vec<u8>,
}

struct Budget {
    operations: usize,
    messages: usize,
    bytes: usize,
}
impl Budget {
    fn new(kind: Kind) -> Self {
        Self {
            operations: MAX_OPERATIONS,
            messages: MAX_MESSAGES,
            // One maximum image per poll. Even malicious one-byte progress
            // remains bounded by the independent syscall count.
            bytes: kind.maximum() + MAX_AUTH_BYTES + PREFIX * MAX_MESSAGES,
        }
    }
    fn operation(&mut self, remaining: usize) -> usize {
        if self.operations == 0 || self.messages == 0 || self.bytes == 0 {
            return 0;
        }
        self.operations -= 1;
        remaining.min(self.bytes)
    }
}

#[derive(Default)]
struct Reader {
    prefix: [u8; PREFIX],
    prefix_received: usize,
    payload: Vec<u8>,
    received: usize,
}
impl Reader {
    fn read(&mut self, stream: &mut impl Read, budget: &mut Budget, maximum: usize) -> Result<bool, Error> {
        loop {
            let prefix = self.prefix_received < PREFIX;
            if !prefix && self.payload.is_empty() {
                let length = u32::from_be_bytes(self.prefix) as usize;
                if length == 0 || length > maximum {
                    return Err(Error::Malformed);
                }
                self.payload.resize(length, 0);
            }
            if !prefix && self.received == self.payload.len() {
                if budget.messages == 0 {
                    return Ok(false);
                }
                budget.messages -= 1;
                return Ok(true);
            }
            let bytes = if prefix {
                &mut self.prefix[self.prefix_received..]
            } else {
                &mut self.payload[self.received..]
            };
            let allowed = budget.operation(bytes.len());
            if allowed == 0 {
                return Ok(false);
            }
            match stream.read(&mut bytes[..allowed]) {
                Ok(0) => return Err(Error::Closed),
                Ok(count) => {
                    budget.bytes -= count;
                    if prefix {
                        self.prefix_received += count;
                    } else {
                        self.received += count;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    fn take(&mut self) -> Vec<u8> {
        self.prefix_received = 0;
        self.received = 0;
        std::mem::take(&mut self.payload)
    }
}

struct Frame {
    prefix: [u8; PREFIX],
    payload: Vec<u8>,
    written: usize,
    authentication: bool,
    authority: bool,
}
impl Frame {
    fn new(payload: Vec<u8>, authentication: bool, maximum: usize) -> Result<Self, Error> {
        if payload.is_empty() || payload.len() > maximum {
            return Err(Error::Malformed);
        }
        Ok(Self {
            prefix: (payload.len() as u32).to_be_bytes(),
            payload,
            written: 0,
            authentication,
            authority: false,
        })
    }
}

#[derive(Default)]
struct Framing {
    reader: Reader,
    outgoing: VecDeque<Frame>,
}
impl Framing {
    fn authentication(&mut self, bytes: Vec<u8>) -> Result<(), Error> {
        self.outgoing.push_back(Frame::new(bytes, true, MAX_AUTH_BYTES)?);
        Ok(())
    }

    fn queue(&mut self, bytes: Vec<u8>, kind: Kind, priority: bool) -> Result<(), Error> {
        let frame = Frame::new(bytes, false, kind.maximum())?;
        if priority || kind == Kind::Image {
            // A partial TCP frame must finish. Keep authentication proofs even
            // if the host queues its first subscription before they flush.
            self.outgoing.retain(|old| old.authentication || old.authority || old.written != 0);
        }
        let maximum = if kind == Kind::Image { 2 } else { MAX_CONTROL_QUEUE };
        if self.outgoing.len() >= maximum {
            return Err(Error::Busy);
        }
        self.outgoing.push_back(frame);
        Ok(())
    }

    fn queue_authority(&mut self, bytes: Vec<u8>, kind: Kind) -> Result<(), Error> {
        let mut frame = Frame::new(bytes, false, kind.maximum())?;
        frame.authority = true;
        // A newer suspension retires an unsent older one, while a partial TCP
        // frame must finish. Ordinary status replacement cannot erase this
        // request/proof before it actually flushes.
        self.outgoing.retain(|old| old.authentication || old.written != 0);
        if self.outgoing.len() >= MAX_CONTROL_QUEUE {
            return Err(Error::Busy);
        }
        self.outgoing.push_back(frame);
        Ok(())
    }

    fn write(&mut self, stream: &mut impl Write, budget: &mut Budget) -> Result<(), Error> {
        self.write_current(stream, budget, &mut || true)
    }

    fn write_current(&mut self, stream: &mut impl Write, budget: &mut Budget, current: &mut impl FnMut() -> bool) -> Result<(), Error> {
        while let Some(frame) = self.outgoing.front_mut() {
            if !current() {
                return Err(Error::Closed);
            }
            let bytes = if frame.written < PREFIX {
                &frame.prefix[frame.written..]
            } else {
                &frame.payload[frame.written - PREFIX..]
            };
            let allowed = budget.operation(bytes.len());
            if allowed == 0 {
                return Ok(());
            }
            match stream.write(&bytes[..allowed]) {
                Ok(0) => return Err(Error::Io(io::ErrorKind::WriteZero)),
                Ok(count) => {
                    budget.bytes -= count;
                    frame.written += count;
                    if frame.written == PREFIX + frame.payload.len() {
                        self.outgoing.pop_front();
                        budget.messages -= 1;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}

struct Socket {
    stream: TcpStream,
    framing: Framing,
    permit: Permit,
    kind: Kind,
    worker: bool,
}
impl Socket {
    fn queue(&mut self, header: &Header, rgba: &[u8], priority: bool) -> Result<(), Error> {
        if !self.matches(header) {
            return Err(Error::Binding);
        }
        if !self.kind.outgoing(self.worker, header) {
            return Err(Error::Direction);
        }
        let bytes = wire::encode(header, rgba).map_err(|_| Error::Malformed)?;
        if matches!(header, Header::Suspend(_) | Header::Suspended(_)) {
            self.framing.queue_authority(bytes, self.kind)
        } else {
            self.framing.queue(bytes, self.kind, priority)
        }
    }

    fn matches(&self, header: &Header) -> bool {
        let source = header.source();
        source.worker == self.permit.identity && source.lease_epoch == self.permit.lease_epoch
    }

    fn packets(&mut self, budget: &mut Budget) -> Result<Vec<Packet>, Error> {
        let mut packets = Vec::new();
        while packets.len() < MAX_MESSAGES && self.framing.reader.read(&mut self.stream, budget, self.kind.maximum())? {
            let bytes = self.framing.reader.take();
            let (header, rgba) = wire::decode(&bytes).map_err(|_| Error::Malformed)?;
            if !self.matches(&header) {
                return Err(Error::Binding);
            }
            if !self.kind.outgoing(!self.worker, &header) {
                return Err(Error::Direction);
            }
            packets.push(Packet {
                header,
                rgba: rgba.to_vec(),
            });
        }
        Ok(packets)
    }
}

struct Pending {
    stream: TcpStream,
    framing: Framing,
    began: Instant,
    auth: ServerAuth,
}

pub(crate) struct Listener {
    listener: TcpListener,
    kind: Kind,
    scope: Option<(InstallationId, HostId)>,
    pending: Option<Pending>,
}
impl Listener {
    pub fn bind(kind: Kind) -> io::Result<Self> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            kind,
            scope: None,
            pending: None,
        })
    }
    pub fn port(&self) -> io::Result<u16> {
        let address = self.listener.local_addr()?;
        if address.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) || address.port() == 0 {
            return Err(io::Error::other("preview listener is not loopback"));
        }
        Ok(address.port())
    }

    pub fn poll(&mut self, allowed: &[Permit]) -> io::Result<Option<HostIo>> {
        if allowed.len() > MAX_WORKERS
            || allowed
                .iter()
                .enumerate()
                .any(|(index, p)| p.lease_epoch == 0 || allowed[..index].iter().any(|old| old.identity == p.identity))
        {
            return Err(io::Error::other("invalid preview admission roster"));
        }
        if let Some(first) = allowed.first() {
            let scope = (first.identity.installation, first.identity.host);
            if self.scope.is_some_and(|old| old != scope) || allowed.iter().any(|p| (p.identity.installation, p.identity.host) != scope) {
                return Err(io::Error::other("preview admission belongs to another host"));
            }
            self.scope = Some(scope);
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.began.elapsed() >= HANDSHAKE_DEADLINE)
        {
            self.pending = None;
        }
        if self.pending.is_none() {
            match self.listener.accept() {
                Ok((stream, address)) => {
                    if address.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) {
                        return Ok(None);
                    }
                    stream.set_nonblocking(true)?;
                    stream.set_nodelay(true)?;
                    let (auth, challenge) =
                        ServerAuth::begin(self.kind.purpose()).map_err(|_| io::Error::other("preview authentication unavailable"))?;
                    let mut framing = Framing::default();
                    framing
                        .authentication(challenge)
                        .map_err(|_| io::Error::other("preview authentication unavailable"))?;
                    self.pending = Some(Pending {
                        stream,
                        framing,
                        began: Instant::now(),
                        auth,
                    });
                }
                Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => return Ok(None),
                Err(error) => return Err(error),
            }
        }
        let Some(mut pending) = self.pending.take() else {
            return Ok(None);
        };
        let mut budget = Budget::new(Kind::Control);
        if pending.framing.write(&mut pending.stream, &mut budget).is_err() {
            return Ok(None);
        }
        match pending.framing.reader.read(&mut pending.stream, &mut budget, MAX_AUTH_BYTES) {
            Ok(false) => {
                self.pending = Some(pending);
                return Ok(None);
            }
            Err(_) => return Ok(None),
            Ok(true) => {}
        }
        let bytes = pending.framing.reader.take();
        let Ok((permit, proof)) = pending.auth.accept(&bytes, allowed) else {
            return Ok(None);
        };
        pending
            .framing
            .authentication(proof)
            .map_err(|_| io::Error::other("preview authentication unavailable"))?;
        Ok(Some(HostIo {
            socket: Socket {
                stream: pending.stream,
                framing: pending.framing,
                permit,
                kind: self.kind,
                worker: false,
            },
        }))
    }
}

pub(crate) struct HostIo {
    socket: Socket,
}
impl HostIo {
    pub fn identity(&self) -> WorkerIdentity {
        self.socket.permit.identity
    }
    pub fn lease_epoch(&self) -> u64 {
        self.socket.permit.lease_epoch
    }
    pub fn queue(&mut self, header: &Header) -> Result<(), Error> {
        self.socket.queue(header, &[], matches!(header, Header::Subscribe(_)))
    }
    pub fn poll(&mut self) -> Result<Vec<Packet>, Error> {
        self.poll_current(|| true)
    }

    /// An image producer supplies its local privacy/source generation check.
    /// Recheck before EVERY bounded write, then let the owner drop the whole
    /// socket on change so a partial old frame can never be completed later.
    pub fn poll_current(&mut self, mut current: impl FnMut() -> bool) -> Result<Vec<Packet>, Error> {
        if !current() {
            return Err(Error::Closed);
        }
        let mut budget = Budget::new(self.socket.kind);
        self.socket
            .framing
            .write_current(&mut self.socket.stream, &mut budget, &mut current)?;
        let packets = self.socket.packets(&mut budget)?;
        self.socket
            .framing
            .write_current(&mut self.socket.stream, &mut budget, &mut current)?;
        Ok(packets)
    }
}

pub(crate) struct WorkerIo {
    socket: Socket,
    auth: Option<ClientAuth>,
    began: Instant,
}
impl WorkerIo {
    pub fn connect(port: u16, permit: Permit, kind: Kind) -> io::Result<Self> {
        if port == 0 || permit.lease_epoch == 0 {
            return Err(io::Error::other("invalid preview endpoint"));
        }
        let stream = TcpStream::connect_timeout(&SocketAddrV4::new(Ipv4Addr::LOCALHOST, port).into(), HANDSHAKE_DEADLINE)?;
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        let auth = ClientAuth::new(kind.purpose(), permit.clone()).map_err(|_| io::Error::other("preview authentication unavailable"))?;
        Ok(Self {
            socket: Socket {
                stream,
                framing: Framing::default(),
                permit,
                kind,
                worker: true,
            },
            auth: Some(auth),
            began: Instant::now(),
        })
    }
    pub fn ready(&self) -> bool {
        self.auth.is_none()
    }
    pub fn has_pending_writes(&self) -> bool {
        !self.socket.framing.outgoing.is_empty()
    }
    pub fn queue(&mut self, header: &Header, rgba: &[u8]) -> Result<(), Error> {
        if !self.ready() {
            return Err(Error::NotReady);
        }
        self.socket.queue(header, rgba, matches!(header, Header::State(_)))
    }
    pub fn poll(&mut self) -> Result<Vec<Packet>, Error> {
        self.poll_current(|| true)
    }
    pub fn poll_current(&mut self, mut current: impl FnMut() -> bool) -> Result<Vec<Packet>, Error> {
        if !current() {
            return Err(Error::Closed);
        }
        let mut budget = Budget::new(self.socket.kind);
        self.socket
            .framing
            .write_current(&mut self.socket.stream, &mut budget, &mut current)?;
        if let Some(auth) = &mut self.auth {
            if self.began.elapsed() >= HANDSHAKE_DEADLINE {
                return Err(Error::Authentication);
            }
            if !self
                .socket
                .framing
                .reader
                .read(&mut self.socket.stream, &mut budget, MAX_AUTH_BYTES)?
            {
                return Ok(Vec::new());
            }
            match auth
                .receive(&self.socket.framing.reader.take())
                .map_err(|_| Error::Authentication)?
            {
                ClientStep::Reply(bytes) => {
                    self.socket.framing.authentication(bytes)?;
                    self.socket
                        .framing
                        .write_current(&mut self.socket.stream, &mut budget, &mut current)?;
                    return Ok(Vec::new());
                }
                ClientStep::Ready => self.auth = None,
            }
        }
        let packets = self.socket.packets(&mut budget)?;
        self.socket
            .framing
            .write_current(&mut self.socket.stream, &mut budget, &mut current)?;
        Ok(packets)
    }
}

#[cfg(test)]
mod tests {
    use super::super::bridge_auth::BridgeSecret;
    use super::super::preview_protocol::{Availability, ImageHeader, State};
    use super::*;
    use aurora_session::PreviewSize;
    use std::time::Duration;

    fn source() -> wire::PreviewSource {
        wire::fixture_source(3)
    }
    fn permit() -> Permit {
        Permit {
            identity: source().worker,
            lease_epoch: source().lease_epoch,
            secret: serde_json::from_str::<BridgeSecret>(&format!("\"{}\"", "07".repeat(32))).unwrap(),
        }
    }
    fn state(epoch: u64) -> Header {
        Header::State(State {
            source: source(),
            subscription_revision: 1,
            epoch,
            availability: Availability::Restricted,
            challenge: 1,
        })
    }
    fn image(sequence: u64) -> Header {
        Header::Image(ImageHeader {
            source: source(),
            subscription_revision: 1,
            epoch: 1,
            sequence,
            size: PreviewSize::Small,
            challenge: 1,
        })
    }

    #[test]
    fn preview_outer_lengths_are_refused_before_allocating_auth_or_image_payloads() {
        for maximum in [MAX_AUTH_BYTES, Kind::Control.maximum(), Kind::Image.maximum()] {
            for length in [0, maximum + 1, u32::MAX as usize] {
                let mut input = io::Cursor::new((length as u32).to_be_bytes());
                let mut reader = Reader::default();
                assert_eq!(
                    reader.read(&mut input, &mut Budget::new(Kind::Image), maximum),
                    Err(Error::Malformed)
                );
                assert!(reader.payload.is_empty());
            }
        }
    }

    #[test]
    fn preview_partial_progress_has_independent_byte_and_syscall_bounds() {
        struct Interrupted(usize);
        impl Read for Interrupted {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                self.0 += 1;
                Err(io::ErrorKind::Interrupted.into())
            }
        }
        let mut input = Interrupted(0);
        assert!(
            !Reader::default()
                .read(&mut input, &mut Budget::new(Kind::Image), Kind::Image.maximum())
                .unwrap()
        );
        assert_eq!(input.0, MAX_OPERATIONS);

        let payload = vec![9; Kind::Image.maximum()];
        let mut framing = Framing::default();
        framing.queue(payload, Kind::Image, false).unwrap();
        let mut budget = Budget::new(Kind::Image);
        budget.bytes = 11;
        let mut output = Vec::new();
        framing.write(&mut output, &mut budget).unwrap();
        assert_eq!(output.len(), 11);
        assert_eq!(framing.outgoing.front().unwrap().written, 11);
    }

    #[test]
    fn latest_image_replaces_only_unsent_images_and_state_priority_preserves_authentication() {
        let mut framing = Framing::default();
        framing.authentication(vec![1]).unwrap();
        framing.queue(vec![2], Kind::Control, false).unwrap();
        framing.queue(vec![3], Kind::Control, true).unwrap();
        assert_eq!(framing.outgoing.len(), 2);
        assert!(framing.outgoing[0].authentication);
        assert_eq!(framing.outgoing[1].payload, vec![3]);

        framing = Framing::default();
        framing.queue(vec![1], Kind::Image, false).unwrap();
        framing.outgoing.front_mut().unwrap().written = 2;
        for value in [2, 3, 4] {
            framing.queue(vec![value], Kind::Image, false).unwrap();
        }
        assert_eq!(framing.outgoing.len(), 2);
        assert_eq!(framing.outgoing[0].payload, vec![1]);
        assert_eq!(framing.outgoing[1].payload, vec![4]);
    }

    #[test]
    fn a_latest_status_never_erases_an_unsent_gpu_suspension_request_or_proof() {
        let mut framing = Framing::default();
        framing.queue_authority(vec![1], Kind::Control).unwrap();
        for value in [2, 3, 4] {
            framing.queue(vec![value], Kind::Control, true).unwrap();
        }
        assert_eq!(framing.outgoing.len(), 2);
        assert!(framing.outgoing[0].authority);
        assert_eq!(framing.outgoing[0].payload, vec![1]);
        assert_eq!(framing.outgoing[1].payload, vec![4]);
        framing.queue_authority(vec![5], Kind::Control).unwrap();
        assert_eq!(framing.outgoing.len(), 1, "newer authority retires an entirely unsent request");
        assert_eq!(framing.outgoing[0].payload, vec![5]);
    }

    fn connected(kind: Kind) -> (WorkerIo, HostIo) {
        let mut listener = Listener::bind(kind).unwrap();
        let permit = permit();
        let mut worker = WorkerIo::connect(listener.port().unwrap(), permit.clone(), kind).unwrap();
        assert_eq!(worker.queue(&state(1), &[]), Err(Error::NotReady));
        let mut host = None;
        let end = Instant::now() + HANDSHAKE_DEADLINE;
        while Instant::now() < end {
            worker.poll().unwrap();
            if host.is_none() {
                host = listener.poll(std::slice::from_ref(&permit)).unwrap();
            }
            if let Some(host) = &mut host {
                host.poll().unwrap();
            }
            if worker.ready() {
                return (worker, host.unwrap());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("local purpose-bound preview authentication did not complete");
    }

    #[test]
    fn authenticated_preview_control_and_binary_image_lanes_keep_distinct_directions() {
        let (mut worker, mut host) = connected(Kind::Control);
        assert_eq!(
            worker.queue(&image(1), &vec![0; PreviewSize::Small.bytes() as usize]),
            Err(Error::Direction)
        );
        assert_eq!(host.queue(&state(1)), Err(Error::Direction));
        worker.queue(&state(2), &[]).unwrap();
        let end = Instant::now() + HANDSHAKE_DEADLINE;
        let packet = loop {
            worker.poll().unwrap();
            if let Some(packet) = host.poll().unwrap().pop() {
                break packet;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(packet.header, state(2));
        assert!(packet.rgba.is_empty());

        let (mut worker, mut host) = connected(Kind::Image);
        assert_eq!(worker.queue(&state(1), &[]), Err(Error::Direction));
        let pixels = vec![19; PreviewSize::Small.bytes() as usize];
        worker.queue(&image(1), &pixels).unwrap();
        let end = Instant::now() + HANDSHAKE_DEADLINE;
        let packet = loop {
            worker.poll().unwrap();
            if let Some(packet) = host.poll().unwrap().pop() {
                break packet;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(packet.header, image(1));
        assert_eq!(packet.rgba, pixels);
    }
}
