//! Memory-only loopback chat lane. Call connect and poll from the existing IO
//! workers, never from an event/frame callback. This module owns no threads,
//! transcript files, credentials, native input, or chat submission behavior.
//!
//! A dedicated random key stays in the private bootstrap, separate from public
//! worker identity. Mutual challenge-response verifies the host before a worker
//! sends ChatPackets. Every data packet retains its identity, lease, direction.
//! Callers additionally validate the exact current ChatSource and control lease
//! BEFORE consuming their receive sequence high-water mark. Reconnect never
//! replays a mutation: caller-owned receipts decide Pending/Uncertain behavior.

use super::bridge_auth::{AuthError, ClientAuth, ClientStep, HANDSHAKE_DEADLINE, Purpose, ServerAuth};
pub(crate) use super::bridge_auth::{BridgeSecret as ChatSecret, Permit};
use aurora_session::chat::{self, ChatMessage, ChatPacket};
use aurora_session::{HostId, InstallationId, MAX_WORKERS, WorkerIdentity};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::time::Instant;

const HEADER: usize = 4;
const MAX_PAYLOAD: usize = chat::MAX_PACKET_BYTES;
const MAX_FRAME: usize = HEADER + MAX_PAYLOAD;
const MAX_QUEUE: usize = 16;
const MAX_PACKETS: usize = 16;
const MAX_OPERATIONS: usize = 64;
impl Permit {
    fn matches(&self, packet: &ChatPacket) -> bool {
        packet.source.worker == self.identity && packet.source.lease_epoch == self.lease_epoch
    }
}

/// Errors carry categories only, never a packet, authentication value, JSON
/// parser excerpt, resident name, or chat text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaneError {
    Busy,
    NotReady,
    Closed,
    Authentication,
    Binding,
    Direction,
    Sequence,
    Malformed,
    Io(io::ErrorKind),
}

impl std::fmt::Display for LaneError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Busy => "session chat lane queue is full",
            Self::NotReady => "session chat lane is not authenticated",
            Self::Closed => "session chat lane is closed",
            Self::Authentication => "session chat lane authentication failed",
            Self::Binding => "session chat lane binding mismatch",
            Self::Direction => "session chat lane packet direction mismatch",
            Self::Sequence => "session chat lane outgoing sequence is stale",
            Self::Malformed => "session chat lane frame is invalid",
            Self::Io(_) => "session chat lane IO failed",
        })
    }
}

impl std::error::Error for LaneError {}

impl From<io::Error> for LaneError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

impl From<AuthError> for LaneError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::Malformed => Self::Malformed,
            AuthError::Rejected => Self::Authentication,
            AuthError::Randomness => Self::Io(io::ErrorKind::Other),
        }
    }
}

/// Includes both completed inbound and outbound frames, with a separate small
/// syscall bound for malicious one-byte progress or repeated Interrupted errors.
struct Budget {
    operations: usize,
    packets: usize,
}

impl Budget {
    fn new() -> Self {
        Self {
            operations: MAX_OPERATIONS,
            packets: MAX_PACKETS,
        }
    }

    fn operation(&mut self) -> bool {
        if self.operations == 0 || self.packets == 0 {
            return false;
        }
        self.operations -= 1;
        true
    }
}

enum ReadState {
    Pending,
    Frame,
    Closed,
}

struct FrameReader {
    bytes: [u8; MAX_FRAME],
    received: usize,
    expected: usize,
}

impl Default for FrameReader {
    fn default() -> Self {
        Self {
            bytes: [0; MAX_FRAME],
            received: 0,
            expected: HEADER,
        }
    }
}

impl FrameReader {
    fn read(&mut self, input: &mut impl Read, budget: &mut Budget) -> Result<ReadState, LaneError> {
        loop {
            if self.received == HEADER && self.expected == HEADER {
                let length = u32::from_be_bytes([self.bytes[0], self.bytes[1], self.bytes[2], self.bytes[3]]) as usize;
                if length == 0 || length > MAX_PAYLOAD {
                    return Err(LaneError::Malformed);
                }
                self.expected = HEADER + length;
            }
            if self.received == self.expected && self.expected > HEADER {
                if budget.packets == 0 {
                    return Ok(ReadState::Pending);
                }
                budget.packets -= 1;
                return Ok(ReadState::Frame);
            }
            if !budget.operation() {
                return Ok(ReadState::Pending);
            }
            match input.read(&mut self.bytes[self.received..self.expected]) {
                Ok(0) if self.received == 0 => return Ok(ReadState::Closed),
                Ok(0) => return Err(LaneError::Malformed),
                Ok(count) => self.received += count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(ReadState::Pending),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn payload(&self) -> Result<&[u8], LaneError> {
        if self.expected <= HEADER || self.received != self.expected {
            return Err(LaneError::Malformed);
        }
        Ok(&self.bytes[HEADER..self.expected])
    }

    fn consume(&mut self) {
        self.received = 0;
        self.expected = HEADER;
    }
}

struct Frame {
    bytes: Box<[u8; MAX_FRAME]>,
    length: usize,
    written: usize,
}

impl Frame {
    fn new(payload: &[u8]) -> Result<Self, LaneError> {
        if payload.is_empty() || payload.len() > MAX_PAYLOAD {
            return Err(LaneError::Malformed);
        }
        let mut bytes = Box::new([0; MAX_FRAME]);
        bytes[..HEADER].copy_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes[HEADER..HEADER + payload.len()].copy_from_slice(payload);
        Ok(Self {
            bytes,
            length: HEADER + payload.len(),
            written: 0,
        })
    }
}

struct Framing {
    reader: FrameReader,
    outgoing: VecDeque<Frame>,
}

impl Default for Framing {
    fn default() -> Self {
        Self {
            reader: FrameReader::default(),
            outgoing: VecDeque::with_capacity(MAX_QUEUE),
        }
    }
}

impl Framing {
    fn full(&self) -> bool {
        self.outgoing.len() == MAX_QUEUE
    }

    fn queue(&mut self, payload: &[u8]) -> Result<(), LaneError> {
        if self.full() {
            return Err(LaneError::Busy);
        }
        self.outgoing.push_back(Frame::new(payload)?);
        Ok(())
    }

    fn write(&mut self, output: &mut impl Write, budget: &mut Budget, maximum: usize) -> Result<(), LaneError> {
        let mut completed = 0;
        while completed < maximum {
            let Some(frame) = self.outgoing.front_mut() else { return Ok(()) };
            if !budget.operation() {
                return Ok(());
            }
            match output.write(&frame.bytes[frame.written..frame.length]) {
                Ok(0) => return Err(LaneError::Io(io::ErrorKind::WriteZero)),
                Ok(count) => {
                    frame.written += count;
                    if frame.written == frame.length {
                        self.outgoing.pop_front();
                        budget.packets -= 1;
                        completed += 1;
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

#[derive(Clone, Copy)]
enum Side {
    Host,
    Worker,
}

impl Side {
    fn outgoing(self, message: &ChatMessage) -> bool {
        matches!(
            (self, message),
            (Self::Host, ChatMessage::Command(_)) | (Self::Worker, ChatMessage::Event(_))
        )
    }

    fn incoming(self, message: &ChatMessage) -> bool {
        matches!(
            (self, message),
            (Self::Host, ChatMessage::Event(_)) | (Self::Worker, ChatMessage::Command(_))
        )
    }
}

struct Socket {
    stream: Option<TcpStream>,
    framing: Framing,
    permit: Permit,
    side: Side,
    sent: u64,
}

impl Socket {
    fn new(stream: TcpStream, framing: Framing, permit: Permit, side: Side) -> Self {
        Self {
            stream: Some(stream),
            framing,
            permit,
            side,
            sent: 0,
        }
    }

    fn queue(&mut self, packet: &ChatPacket) -> Result<(), LaneError> {
        if self.stream.is_none() {
            return Err(LaneError::Closed);
        }
        if self.framing.full() {
            return Err(LaneError::Busy);
        }
        if !self.permit.matches(packet) {
            return Err(LaneError::Binding);
        }
        if !self.side.outgoing(&packet.message) {
            return Err(LaneError::Direction);
        }
        if packet.sequence <= self.sent {
            return Err(LaneError::Sequence);
        }
        let bytes = chat::encode(packet).map_err(|_| LaneError::Malformed)?;
        self.framing.queue(&bytes)?;
        // Commit only after capacity, validation and framing all succeeded.
        self.sent = packet.sequence;
        Ok(())
    }

    fn write(&mut self, budget: &mut Budget, maximum: usize) -> Result<(), LaneError> {
        self.framing.write(self.stream.as_mut().ok_or(LaneError::Closed)?, budget, maximum)
    }

    fn read(&mut self, budget: &mut Budget) -> Result<ReadState, LaneError> {
        self.framing.reader.read(self.stream.as_mut().ok_or(LaneError::Closed)?, budget)
    }

    fn packet(&mut self) -> Result<ChatPacket, LaneError> {
        let packet = chat::decode(self.framing.reader.payload()?).map_err(|_| LaneError::Malformed)?;
        if !self.permit.matches(&packet) {
            return Err(LaneError::Binding);
        }
        if !self.side.incoming(&packet.message) {
            return Err(LaneError::Direction);
        }
        self.framing.reader.consume();
        Ok(packet)
    }

    fn packets(&mut self, budget: &mut Budget, maximum: usize) -> Result<Vec<ChatPacket>, LaneError> {
        let maximum = maximum.min(MAX_PACKETS);
        let mut packets = Vec::with_capacity(maximum);
        while budget.packets != 0 && packets.len() < maximum {
            match self.read(budget)? {
                ReadState::Frame => packets.push(self.packet()?),
                ReadState::Pending => break,
                ReadState::Closed => return Err(LaneError::Closed),
            }
        }
        Ok(packets)
    }

    fn close(&mut self) {
        self.stream = None;
        self.framing.outgoing.clear();
        self.framing.reader.consume();
    }
}

struct PendingClient {
    stream: TcpStream,
    framing: Framing,
    began: Instant,
    auth: ServerAuth,
}

/// One host listener and at most one unauthenticated socket. Authenticated
/// HostIo objects are owned by the broker's bounded per-worker roster.
pub(crate) struct Listener {
    listener: TcpListener,
    port: u16,
    scope: Option<(InstallationId, HostId)>,
    pending: Option<PendingClient>,
}

impl Listener {
    pub fn bind() -> io::Result<Self> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        if address.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) || address.port() == 0 {
            return Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "session chat listener is not loopback",
            ));
        }
        Ok(Self {
            listener,
            port: address.port(),
            scope: None,
            pending: None,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Supply only current, live child permits. Invalid clients are dropped
    /// locally and cannot turn malformed authentication into a broker failure.
    pub fn poll(&mut self, allowed: &[Permit]) -> io::Result<Option<HostIo>> {
        self.validate_scope(allowed)?;
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
                    let (auth, bytes) =
                        ServerAuth::begin(Purpose::Chat).map_err(|_| io::Error::other("session chat challenge could not be encoded"))?;
                    let mut framing = Framing::default();
                    framing
                        .queue(&bytes)
                        .map_err(|_| io::Error::other("session chat challenge queue is full"))?;
                    self.pending = Some(PendingClient {
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
        let mut pending = match self.pending.take() {
            Some(pending) => pending,
            None => return Ok(None),
        };
        let mut budget = Budget::new();
        if pending.framing.write(&mut pending.stream, &mut budget, 1).is_err() {
            return Ok(None);
        }
        match pending.framing.reader.read(&mut pending.stream, &mut budget) {
            Ok(ReadState::Pending) => {
                self.pending = Some(pending);
                return Ok(None);
            }
            Ok(ReadState::Closed) | Err(_) => return Ok(None),
            Ok(ReadState::Frame) => {}
        }
        let payload = match pending.framing.reader.payload() {
            Ok(payload) => payload,
            Err(_) => return Ok(None),
        };
        let (permit, acknowledgment) = match pending.auth.accept(payload, allowed) {
            Ok(accepted) => accepted,
            Err(_) => return Ok(None),
        };
        pending.framing.reader.consume();
        pending
            .framing
            .queue(&acknowledgment)
            .map_err(|_| io::Error::other("session chat authentication queue is full"))?;
        Ok(Some(HostIo {
            socket: Socket::new(pending.stream, pending.framing, permit, Side::Host),
        }))
    }

    fn validate_scope(&mut self, allowed: &[Permit]) -> io::Result<()> {
        if allowed.len() > MAX_WORKERS
            || allowed
                .iter()
                .enumerate()
                .any(|(index, permit)| !permit.valid() || allowed[..index].iter().any(|earlier| earlier.identity == permit.identity))
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid session chat admission roster"));
        }
        if let Some(first) = allowed.first() {
            let scope = (first.identity.installation, first.identity.host);
            if self.scope.is_some_and(|own| own != scope)
                || allowed
                    .iter()
                    .any(|permit| (permit.identity.installation, permit.identity.host) != scope)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "session chat admission belongs to another host",
                ));
            }
            self.scope = Some(scope);
        }
        Ok(())
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

    pub fn queue(&mut self, packet: &ChatPacket) -> Result<(), LaneError> {
        self.socket.queue(packet)
    }

    /// At most sixteen total completed frames are read/written per call.
    /// Reserve initial room for reads so a full outbound queue cannot starve
    /// inbound Send/Receipt traffic; idle read capacity is reused for writes.
    pub fn poll(&mut self) -> Result<Vec<ChatPacket>, LaneError> {
        let result = (|| {
            let mut budget = Budget::new();
            self.socket.write(&mut budget, MAX_PACKETS / 2)?;
            let packets = self.socket.packets(&mut budget, MAX_PACKETS)?;
            self.socket.write(&mut budget, MAX_PACKETS)?;
            Ok(packets)
        })();
        if result.is_err() {
            self.socket.close();
        }
        result
    }
}

pub(crate) struct WorkerIo {
    socket: Socket,
    began: Instant,
    ready: bool,
    auth: ClientAuth,
}

impl WorkerIo {
    /// May wait up to one second for local TCP connect. Call only on the IO
    /// worker; retries/cadence belong to that owner, never the viewer frame.
    pub fn connect(port: u16, permit: Permit) -> io::Result<Self> {
        if port == 0 || !permit.valid() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid session chat endpoint"));
        }
        let auth = ClientAuth::new(Purpose::Chat, permit.clone())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid session chat authentication"))?;
        let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
        let stream = TcpStream::connect_timeout(&address, HANDSHAKE_DEADLINE)?;
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        Ok(Self {
            socket: Socket::new(stream, Framing::default(), permit, Side::Worker),
            began: Instant::now(),
            ready: false,
            auth,
        })
    }

    pub fn ready(&self) -> bool {
        self.ready && self.socket.stream.is_some()
    }

    pub fn queue(&mut self, packet: &ChatPacket) -> Result<(), LaneError> {
        if !self.ready() {
            return Err(LaneError::NotReady);
        }
        self.socket.queue(packet)
    }

    pub fn poll(&mut self) -> Result<Vec<ChatPacket>, LaneError> {
        self.poll_bounded(MAX_PACKETS)
    }

    /// Socket writes/authentication continue even when the app's inbound queue
    /// is full. Limiting reads to available room avoids dropping mutation
    /// commands or deadlocking their outgoing receipts behind that full queue.
    pub fn poll_bounded(&mut self, maximum_incoming: usize) -> Result<Vec<ChatPacket>, LaneError> {
        let result = self.poll_inner(maximum_incoming.min(MAX_PACKETS));
        if result.is_err() {
            self.ready = false;
            self.socket.close();
        }
        result
    }

    fn poll_inner(&mut self, maximum_incoming: usize) -> Result<Vec<ChatPacket>, LaneError> {
        if !self.ready && self.began.elapsed() >= HANDSHAKE_DEADLINE {
            return Err(LaneError::Authentication);
        }
        let mut budget = Budget::new();
        self.socket.write(&mut budget, MAX_PACKETS / 2)?;
        if !self.ready {
            match self.socket.read(&mut budget)? {
                ReadState::Pending => return Ok(Vec::new()),
                ReadState::Closed => return Err(LaneError::Closed),
                ReadState::Frame => {}
            }
            let step = self.auth.receive(self.socket.framing.reader.payload()?).map_err(LaneError::from)?;
            self.socket.framing.reader.consume();
            match step {
                ClientStep::Ready => self.ready = true,
                ClientStep::Reply(bytes) => {
                    self.socket.framing.queue(&bytes)?;
                    self.socket.write(&mut budget, MAX_PACKETS)?;
                    return Ok(Vec::new());
                }
            }
        }
        let packets = self.socket.packets(&mut budget, maximum_incoming)?;
        self.socket.write(&mut budget, MAX_PACKETS)?;
        Ok(packets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::chat::{ChatCommand, ChatEvent, ChatSource};
    use aurora_session::{AccountKey, GridKey, SessionId, SessionIdentity, WorkerId};

    fn permit() -> Permit {
        Permit {
            identity: WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).expect("synthetic installation"),
                host: HostId::from_bytes([2; 16]).expect("synthetic host"),
                worker: WorkerId::from_bytes([3; 16]).expect("synthetic worker"),
            },
            lease_epoch: 7,
            secret: serde_json::from_str(&format!("\"{}\"", "07".repeat(32))).expect("synthetic private bootstrap key"),
        }
    }

    fn source(permit: &Permit) -> ChatSource {
        ChatSource {
            worker: permit.identity,
            lease_epoch: permit.lease_epoch,
            session: SessionIdentity {
                session: SessionId::from_bytes([4; 16]).expect("synthetic session"),
                account: AccountKey::from_bytes([5; 32]).expect("synthetic account"),
                grid: GridKey::from_bytes([6; 32]).expect("synthetic grid"),
                generation: 1,
            },
        }
    }

    fn packet(permit: &Permit, sequence: u64, side: Side) -> ChatPacket {
        ChatPacket {
            version: chat::VERSION,
            source: source(permit),
            sequence,
            reply_to: 0,
            message: match side {
                Side::Host => ChatMessage::Command(ChatCommand::Poll { after_event: 0 }),
                Side::Worker => ChatMessage::Event(ChatEvent::Gap { through_event: sequence }),
            },
        }
    }

    fn wire(payload: &[u8]) -> Vec<u8> {
        let frame = Frame::new(payload).expect("bounded test frame");
        frame.bytes[..frame.length].to_vec()
    }

    struct ShortReader {
        bytes: Vec<u8>,
        position: usize,
        chunk: usize,
        pause: bool,
    }

    impl Read for ShortReader {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.pause || self.position == self.bytes.len() {
                self.pause = false;
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let count = output.len().min(self.chunk).min(self.bytes.len() - self.position);
            output[..count].copy_from_slice(&self.bytes[self.position..self.position + count]);
            self.position += count;
            self.pause = true;
            Ok(count)
        }
    }

    struct ShortWriter {
        bytes: Vec<u8>,
        chunk: usize,
        pause: bool,
    }

    impl Write for ShortWriter {
        fn write(&mut self, input: &[u8]) -> io::Result<usize> {
            if self.pause {
                self.pause = false;
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let count = input.len().min(self.chunk);
            self.bytes.extend_from_slice(&input[..count]);
            self.pause = true;
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn chat_lane_fragmented_headers_payloads_and_writes_keep_frame_order() {
        let payloads: [&[u8]; 3] = [b"first", b"second", b"third"];
        let bytes: Vec<_> = payloads.iter().flat_map(|payload| wire(payload)).collect();
        let mut reader = ShortReader {
            bytes: bytes.clone(),
            position: 0,
            chunk: 1,
            pause: true,
        };
        let mut partial = FrameReader::default();
        let mut received = Vec::new();
        for _ in 0..128 {
            if matches!(
                partial.read(&mut reader, &mut Budget::new()).expect("partial read"),
                ReadState::Frame
            ) {
                received.push(partial.payload().expect("complete payload").to_vec());
                partial.consume();
            }
            if received.len() == payloads.len() {
                break;
            }
        }
        assert_eq!(received, payloads.map(<[u8]>::to_vec));
        assert_eq!(partial.bytes.len(), 4100);

        let mut outgoing = Framing::default();
        for payload in payloads {
            outgoing.queue(payload).expect("queue");
        }
        let mut writer = ShortWriter {
            bytes: Vec::new(),
            chunk: 2,
            pause: true,
        };
        for _ in 0..128 {
            outgoing.write(&mut writer, &mut Budget::new(), MAX_PACKETS).expect("partial write");
            if outgoing.outgoing.is_empty() {
                break;
            }
        }
        assert!(outgoing.outgoing.is_empty());
        assert_eq!(writer.bytes, bytes);
    }

    #[test]
    fn chat_lane_rejects_zero_oversized_and_truncated_frames_before_payload_allocation() {
        for length in [0, MAX_PAYLOAD as u32 + 1, u32::MAX] {
            let mut input = io::Cursor::new(length.to_be_bytes());
            let mut reader = FrameReader::default();
            assert!(matches!(reader.read(&mut input, &mut Budget::new()), Err(LaneError::Malformed)));
            assert_eq!(reader.bytes.len(), MAX_FRAME);
        }
        for bytes in [vec![0, 0], vec![0, 0, 0, 10, 1, 2]] {
            assert!(matches!(
                FrameReader::default().read(&mut io::Cursor::new(bytes), &mut Budget::new()),
                Err(LaneError::Malformed)
            ));
        }
        assert!(matches!(
            FrameReader::default().read(&mut io::Cursor::new([]), &mut Budget::new()),
            Ok(ReadState::Closed)
        ));
        let valid = wire(&[0; MAX_PAYLOAD]);
        let mut reader = FrameReader::default();
        assert!(matches!(
            reader.read(&mut io::Cursor::new(valid), &mut Budget::new()),
            Ok(ReadState::Frame)
        ));
        assert_eq!(reader.payload().expect("maximum payload").len(), MAX_PAYLOAD);
    }

    #[test]
    fn chat_lane_poll_bounds_include_interrupted_syscalls_and_completed_frames() {
        struct Interrupted(usize);
        impl Read for Interrupted {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                self.0 += 1;
                Err(io::ErrorKind::Interrupted.into())
            }
        }
        let mut interrupted = Interrupted(0);
        assert!(matches!(
            FrameReader::default().read(&mut interrupted, &mut Budget::new()),
            Ok(ReadState::Pending)
        ));
        assert_eq!(interrupted.0, MAX_OPERATIONS);
        let mut framing = Framing::default();
        for _ in 0..MAX_QUEUE {
            framing.queue(b"x").expect("bounded queue");
        }
        assert_eq!(framing.queue(b"overflow"), Err(LaneError::Busy));
        let mut budget = Budget::new();
        let mut output = Vec::new();
        framing.write(&mut output, &mut budget, MAX_PACKETS).expect("bounded write");
        assert_eq!(budget.packets, 0);
        assert_eq!(output.len(), MAX_PACKETS * (HEADER + 1));
        let mut input = io::Cursor::new(wire(b"cannot consume another frame"));
        assert!(matches!(
            FrameReader::default().read(&mut input, &mut budget),
            Ok(ReadState::Pending)
        ));
        assert_eq!(input.position(), 0);
    }

    fn connected() -> (Listener, HostIo, WorkerIo) {
        let mut listener = Listener::bind().expect("owned loopback listener");
        let permit = permit();
        let mut worker = WorkerIo::connect(listener.port(), permit.clone()).expect("owned loopback connect");
        let mut host = None;
        let deadline = Instant::now() + HANDSHAKE_DEADLINE;
        loop {
            worker.poll().expect("worker handshake poll");
            if host.is_none() {
                host = listener.poll(std::slice::from_ref(&permit)).expect("host handshake poll");
            }
            if let Some(host) = &mut host {
                host.poll().expect("host acknowledgment poll");
            }
            if worker.ready() {
                return (listener, host.expect("authenticated host"), worker);
            }
            assert!(Instant::now() < deadline, "owned loopback handshake did not complete");
            std::thread::yield_now();
        }
    }

    #[test]
    fn chat_lane_loopback_authenticates_and_exchanges_only_the_bound_directions() {
        let (_, mut host, mut worker) = connected();
        let permit = permit();
        assert!(host.identity() == permit.identity);
        assert_eq!(host.lease_epoch(), permit.lease_epoch);
        assert_eq!(host.queue(&packet(&permit, 1, Side::Worker)), Err(LaneError::Direction));
        assert_eq!(worker.queue(&packet(&permit, 1, Side::Host)), Err(LaneError::Direction));
        let mut foreign = packet(&permit, 1, Side::Host);
        foreign.source.worker.host = HostId::from_bytes([9; 16]).expect("other host");
        assert_eq!(host.queue(&foreign), Err(LaneError::Binding));
        foreign = packet(&permit, 1, Side::Worker);
        foreign.source.lease_epoch += 1;
        assert_eq!(worker.queue(&foreign), Err(LaneError::Binding));
        host.queue(&packet(&permit, 1, Side::Host)).expect("typed host command");
        worker.queue(&packet(&permit, 1, Side::Worker)).expect("typed native event");
        let mut host_received = Vec::new();
        let mut worker_received = Vec::new();
        let deadline = Instant::now() + HANDSHAKE_DEADLINE;
        while host_received.is_empty() || worker_received.is_empty() {
            host_received.extend(host.poll().expect("host poll"));
            worker_received.extend(worker.poll().expect("worker poll"));
            assert!(Instant::now() < deadline, "owned loopback packets did not arrive");
            std::thread::yield_now();
        }
        assert_eq!(host_received, [packet(&permit, 1, Side::Worker)]);
        assert_eq!(worker_received, [packet(&permit, 1, Side::Host)]);
    }

    #[test]
    fn chat_lane_loopback_busy_does_not_consume_an_outgoing_sequence_or_replay_a_send() {
        let (_, mut host, mut worker) = connected();
        let permit = permit();
        for sequence in 1..=MAX_QUEUE as u64 {
            worker.queue(&packet(&permit, sequence, Side::Worker)).expect("bounded event");
        }
        let next = packet(&permit, MAX_QUEUE as u64 + 1, Side::Worker);
        assert_eq!(worker.queue(&next), Err(LaneError::Busy));
        assert_eq!(worker.socket.sent, MAX_QUEUE as u64);
        let mut received = Vec::new();
        let deadline = Instant::now() + HANDSHAKE_DEADLINE;
        while received.len() < MAX_QUEUE {
            worker.poll().expect("worker flush");
            let batch = host.poll().expect("host receive");
            assert!(batch.len() <= MAX_PACKETS);
            received.extend(batch);
            assert!(Instant::now() < deadline, "owned bounded queue did not drain");
            std::thread::yield_now();
        }
        assert_eq!(
            received.iter().map(|packet| packet.sequence).collect::<Vec<_>>(),
            (1..=MAX_QUEUE as u64).collect::<Vec<_>>()
        );
        worker.queue(&next).expect("same caller sequence remains available after Busy");
        assert_eq!(worker.queue(&next), Err(LaneError::Sequence));
    }

    #[test]
    fn chat_lane_zero_incoming_room_still_flushes_responses_without_reading_commands() {
        let (_, mut host, mut worker) = connected();
        let permit = permit();
        for sequence in 1..=MAX_QUEUE as u64 {
            host.queue(&packet(&permit, sequence, Side::Host)).expect("queued commands");
            worker.queue(&packet(&permit, sequence, Side::Worker)).expect("queued responses");
        }
        // Model an App whose incoming queue is full and whose pending native
        // receipts must drain before it can process another command. A zero
        // read allowance must not prevent those outgoing receipts from moving.
        let mut responses = Vec::new();
        let deadline = Instant::now() + HANDSHAKE_DEADLINE;
        while responses.len() < MAX_QUEUE {
            responses.extend(host.poll().expect("host sends commands and receives responses"));
            assert!(
                worker
                    .poll_bounded(0)
                    .expect("write progress with full application input")
                    .is_empty()
            );
            assert_eq!(worker.socket.framing.reader.received, 0);
            assert!(Instant::now() < deadline, "full application input blocked outgoing responses");
            std::thread::yield_now();
        }
        assert!(worker.socket.framing.outgoing.is_empty());
        assert_eq!(
            responses.iter().map(|packet| packet.sequence).collect::<Vec<_>>(),
            (1..=MAX_QUEUE as u64).collect::<Vec<_>>()
        );

        let mut commands = Vec::new();
        while commands.len() < MAX_QUEUE {
            let batch = worker.poll_bounded(1).expect("one available input slot");
            assert!(batch.len() <= 1);
            commands.extend(batch);
            assert!(Instant::now() < deadline, "bounded input failed to resume");
            std::thread::yield_now();
        }
        assert_eq!(
            commands.iter().map(|packet| packet.sequence).collect::<Vec<_>>(),
            (1..=MAX_QUEUE as u64).collect::<Vec<_>>()
        );
    }

    #[test]
    fn chat_lane_unauthenticated_deadline_and_registry_scope_do_not_grow_state() {
        let mut listener = Listener::bind().expect("owned listener");
        let allowed = permit();
        listener.validate_scope(std::slice::from_ref(&allowed)).expect("host registry");
        let foreign = Permit {
            identity: WorkerIdentity {
                host: HostId::from_bytes([9; 16]).expect("foreign host"),
                ..allowed.identity
            },
            ..allowed.clone()
        };
        assert!(listener.validate_scope(&[foreign]).is_err());
        assert!(listener.validate_scope(&[allowed.clone(), allowed.clone()]).is_err());
        let mut invalid = WorkerIo::connect(
            listener.port(),
            Permit {
                lease_epoch: allowed.lease_epoch + 1,
                ..allowed.clone()
            },
        )
        .expect("invalid local peer");
        let deadline = Instant::now() + HANDSHAKE_DEADLINE;
        loop {
            if invalid.poll().is_err() {
                break;
            }
            assert!(listener.poll(std::slice::from_ref(&allowed)).expect("bounded admission").is_none());
            assert!(Instant::now() < deadline, "invalid peer was not closed");
            std::thread::yield_now();
        }
        assert!(!invalid.ready());
        let idle = TcpStream::connect((Ipv4Addr::LOCALHOST, listener.port())).expect("idle owned client");
        assert!(listener.poll(std::slice::from_ref(&allowed)).expect("accept idle").is_none());
        assert!(listener.pending.is_some());
        listener.pending.as_mut().expect("one pending client").began = Instant::now() - HANDSHAKE_DEADLINE;
        assert!(listener.poll(std::slice::from_ref(&allowed)).expect("expire idle").is_none());
        assert!(listener.pending.is_none());
        drop(idle);
    }

    #[test]
    fn chat_lane_a_fake_host_cannot_echo_a_worker_proof_into_host_authority() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("owned fake host");
        let permit = permit();
        let mut worker = WorkerIo::connect(listener.local_addr().expect("address").port(), permit.clone()).expect("owned client");
        let (mut fake, _) = listener.accept().expect("owned accepted socket");
        fake.set_nonblocking(true).expect("nonblocking fake host");
        let mut framing = Framing::default();
        let nonce = [8u8; 32];
        let challenge = serde_json::to_vec(&serde_json::json!({
            "type": "challenge", "data": {"version": 1, "nonce": nonce}
        }))
        .expect("public challenge");
        framing.queue(&challenge).expect("challenge frame");
        assert_eq!(worker.queue(&packet(&permit, 1, Side::Worker)), Err(LaneError::NotReady));
        let deadline = Instant::now() + HANDSHAKE_DEADLINE;
        let reflected = loop {
            framing.write(&mut fake, &mut Budget::new(), MAX_PACKETS).expect("send challenge");
            assert!(worker.poll().expect("client challenge processing").is_empty());
            if matches!(
                framing.reader.read(&mut fake, &mut Budget::new()).expect("read client proof"),
                ReadState::Frame
            ) {
                let proof: serde_json::Value = serde_json::from_slice(framing.reader.payload().expect("proof frame")).expect("typed proof");
                assert_eq!(proof["type"], "worker");
                break serde_json::from_value::<[u8; 32]>(proof["data"]["tag"].clone()).expect("public proof tag");
            }
            assert!(Instant::now() < deadline, "client proof did not arrive");
            std::thread::yield_now();
        };
        assert!(!worker.ready());
        framing.reader.consume();
        let echo = serde_json::to_vec(&serde_json::json!({
            "type": "accepted", "data": {"version": 1, "tag": reflected}
        }))
        .expect("reflected tag");
        framing.queue(&echo).expect("fake acknowledgment");
        let error = loop {
            framing
                .write(&mut fake, &mut Budget::new(), MAX_PACKETS)
                .expect("send fake acknowledgment");
            match worker.poll() {
                Err(error) => break error,
                Ok(packets) => assert!(packets.is_empty()),
            }
            assert!(Instant::now() < deadline, "fake host was not rejected");
            std::thread::yield_now();
        };
        assert_eq!(error, LaneError::Authentication);
        assert!(!worker.ready());
        assert!(worker.socket.stream.is_none());
    }
}
