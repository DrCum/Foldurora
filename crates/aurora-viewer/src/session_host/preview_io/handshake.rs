//! Fixed role handshake IO with one absolute admission deadline. All buffers
//! are fixed; no source/private packet body is ever included in an error.
use super::*;
use lane::{Authenticated, HostHandshake, WorkerHandshake};
use std::io::{Read, Write};
use std::net::TcpStream;

const HELLO: usize = aurora_session::preview_wire::HELLO_BYTES;
const CHALLENGE: usize = 56;
const PROOF: usize = 88;
const ACCEPT: usize = 56;

pub(super) struct Buffer {
    bytes: [u8; 96],
    len: usize,
    position: usize,
}
impl Buffer {
    fn read(len: usize) -> Self {
        Self {
            bytes: [0; 96],
            len,
            position: 0,
        }
    }
    fn write(bytes: &[u8]) -> Self {
        let mut value = Self::read(bytes.len());
        value.bytes[..bytes.len()].copy_from_slice(bytes);
        value
    }
    fn input(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
    fn poll(&mut self, stream: &mut (impl Read + Write), write: bool, operations: &mut usize) -> Result<bool, Error> {
        while self.position < self.len && *operations > 0 {
            *operations -= 1;
            let result = if write {
                stream.write(&self.bytes[self.position..self.len])
            } else {
                stream.read(&mut self.bytes[self.position..self.len])
            };
            match result {
                Ok(0) => return Err(Error::Closed),
                Ok(n) => self.position += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(Error::Io(e.kind())),
            }
        }
        Ok(self.position == self.len)
    }
}
enum HostStage {
    Hello(Buffer),
    Await(Hello),
    Challenge { buffer: Buffer, auth: Box<HostHandshake> },
    Proof { buffer: Buffer, auth: Box<HostHandshake> },
    Accept { buffer: Buffer, auth: Box<Authenticated> },
    Done,
}
pub(super) struct HostPending {
    pub stream: TcpStream,
    pub began: Instant,
    stage: HostStage,
    pub permit_revision: u64,
}
impl HostPending {
    pub fn new(stream: TcpStream, now: Instant) -> Self {
        Self {
            stream,
            began: now,
            stage: HostStage::Hello(Buffer::read(HELLO)),
            permit_revision: 0,
        }
    }
    pub fn hello(&self) -> Option<Hello> {
        if let HostStage::Await(hello) = self.stage {
            Some(hello)
        } else {
            None
        }
    }
    pub fn challenge(&mut self, scope: Scope, secret: PreviewSecret, revision: u64, clock: u64) -> Result<(), Error> {
        if !matches!(self.stage, HostStage::Await(_)) {
            return Err(Error::Sequence);
        }
        let auth = HostHandshake::new(scope, secret, clock)?;
        self.stage = HostStage::Challenge {
            buffer: Buffer::write(auth.challenge().as_slice()),
            auth: Box::new(auth),
        };
        self.permit_revision = revision;
        Ok(())
    }
    pub fn poll(&mut self, now: Instant, clock: u64) -> Result<Option<Authenticated>, Error> {
        if now.checked_duration_since(self.began).is_none_or(|age| age >= DEADLINE) {
            return Err(Error::Expired);
        }
        let mut operations = 4;
        loop {
            let stage = std::mem::replace(&mut self.stage, HostStage::Done);
            self.stage = match stage {
                HostStage::Hello(mut buffer) => {
                    if !buffer.poll(&mut self.stream, false, &mut operations)? {
                        HostStage::Hello(buffer)
                    } else {
                        HostStage::Await(Hello::decode(buffer.input())?)
                    }
                }
                HostStage::Await(hello) => HostStage::Await(hello),
                HostStage::Challenge { mut buffer, auth } => {
                    if buffer.poll(&mut self.stream, true, &mut operations)? {
                        HostStage::Proof {
                            buffer: Buffer::read(PROOF),
                            auth,
                        }
                    } else {
                        HostStage::Challenge { buffer, auth }
                    }
                }
                HostStage::Proof { mut buffer, mut auth } => {
                    if buffer.poll(&mut self.stream, false, &mut operations)? {
                        let (reply, auth) = auth.accept(buffer.input(), clock)?;
                        HostStage::Accept {
                            buffer: Buffer::write(reply.as_slice()),
                            auth: Box::new(auth),
                        }
                    } else {
                        HostStage::Proof { buffer, auth }
                    }
                }
                HostStage::Accept { mut buffer, auth } => {
                    if buffer.poll(&mut self.stream, true, &mut operations)? {
                        return Ok(Some(*auth));
                    }
                    HostStage::Accept { buffer, auth }
                }
                HostStage::Done => return Err(Error::Sequence),
            };
            if operations == 0 || matches!(self.stage, HostStage::Await(_)) {
                return Ok(None);
            }
            // A non-progressing WouldBlock must not spin in the same poll.
            if matches!(&self.stage,HostStage::Hello(b)|HostStage::Challenge{buffer:b,..}|HostStage::Proof{buffer:b,..}|HostStage::Accept{buffer:b,..} if b.position<b.len)
            {
                return Ok(None);
            }
        }
    }
}
enum WorkerStage {
    Connect,
    Hello(Buffer),
    Challenge(Buffer),
    Proof { buffer: Buffer, auth: Box<WorkerHandshake> },
    Accept { buffer: Buffer, auth: Box<WorkerHandshake> },
    Done,
}
pub(super) struct WorkerPending {
    pub stream: TcpStream,
    began: Instant,
    source: PreviewSource,
    secret: PreviewSecret,
    lane: Lane,
    connection: u64,
    minimum_epoch: u64,
    stage: WorkerStage,
}
impl WorkerPending {
    pub fn new(
        port: u16,
        source: PreviewSource,
        secret: PreviewSecret,
        lane: Lane,
        connection: u64,
        minimum_epoch: u64,
        now: Instant,
    ) -> Result<Self, Error> {
        let stream = socket::connect(port).map_err(|e| Error::Io(e.kind()))?;
        Ok(Self {
            stream,
            began: now,
            source,
            secret,
            lane,
            connection,
            minimum_epoch,
            stage: WorkerStage::Connect,
        })
    }
    pub fn poll(&mut self, now: Instant, clock: u64) -> Result<Option<Authenticated>, Error> {
        if now.checked_duration_since(self.began).is_none_or(|age| age >= DEADLINE) {
            return Err(Error::Expired);
        }
        let mut operations = 4;
        loop {
            let stage = std::mem::replace(&mut self.stage, WorkerStage::Done);
            self.stage = match stage {
                WorkerStage::Connect => {
                    if !socket::connected(&self.stream).map_err(|e| Error::Io(e.kind()))? {
                        WorkerStage::Connect
                    } else {
                        WorkerStage::Hello(Buffer::write(
                            &Hello::new(self.source.worker(), self.source.lease_epoch(), self.lane, self.connection)?.encode(),
                        ))
                    }
                }
                WorkerStage::Hello(mut buffer) => {
                    if buffer.poll(&mut self.stream, true, &mut operations)? {
                        WorkerStage::Challenge(Buffer::read(CHALLENGE))
                    } else {
                        WorkerStage::Hello(buffer)
                    }
                }
                WorkerStage::Challenge(mut buffer) => {
                    if buffer.poll(&mut self.stream, false, &mut operations)? {
                        let bytes = buffer.input();
                        let connection = u64::from_be_bytes(bytes[8..16].try_into().map_err(|_| Error::Invalid)?);
                        let role_epoch = u64::from_be_bytes(bytes[16..24].try_into().map_err(|_| Error::Invalid)?);
                        if connection == 0
                            || (self.lane == Lane::Pixels && connection != self.connection)
                            || role_epoch <= self.minimum_epoch
                        {
                            return Err(Error::Binding);
                        }
                        let scope = Scope {
                            source: self.source,
                            connection,
                            role_epoch,
                            lane: self.lane,
                        };
                        let mut auth = WorkerHandshake::new(scope, self.secret.clone(), clock)?;
                        let proof = auth.answer(bytes, clock)?;
                        WorkerStage::Proof {
                            buffer: Buffer::write(proof.as_slice()),
                            auth: Box::new(auth),
                        }
                    } else {
                        WorkerStage::Challenge(buffer)
                    }
                }
                WorkerStage::Proof { mut buffer, auth } => {
                    if buffer.poll(&mut self.stream, true, &mut operations)? {
                        WorkerStage::Accept {
                            buffer: Buffer::read(ACCEPT),
                            auth,
                        }
                    } else {
                        WorkerStage::Proof { buffer, auth }
                    }
                }
                WorkerStage::Accept { mut buffer, mut auth } => {
                    if buffer.poll(&mut self.stream, false, &mut operations)? {
                        return auth.finish(buffer.input(), clock).map(Some);
                    }
                    WorkerStage::Accept { buffer, auth }
                }
                WorkerStage::Done => return Err(Error::Sequence),
            };
            if operations == 0 || matches!(self.stage, WorkerStage::Connect) {
                return Ok(None);
            }
            if matches!(&self.stage,WorkerStage::Hello(b)|WorkerStage::Challenge(b)|WorkerStage::Proof{buffer:b,..}|WorkerStage::Accept{buffer:b,..} if b.position<b.len)
            {
                return Ok(None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fragment {
        input: std::collections::VecDeque<u8>,
        output: Vec<u8>,
        maximum: usize,
        interrupt: bool,
        eof: bool,
        calls: usize,
    }
    impl Read for Fragment {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            self.calls += 1;
            if self.interrupt {
                self.interrupt = false;
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            if self.input.is_empty() {
                return if self.eof {
                    Ok(0)
                } else {
                    Err(std::io::ErrorKind::WouldBlock.into())
                };
            }
            let n = self.maximum.min(out.len()).min(self.input.len());
            for byte in &mut out[..n] {
                *byte = self.input.pop_front().unwrap();
            }
            Ok(n)
        }
    }
    impl Write for Fragment {
        fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
            self.calls += 1;
            if self.interrupt {
                self.interrupt = false;
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            if self.maximum == 0 {
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            let n = self.maximum.min(input.len());
            self.output.extend_from_slice(&input[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn preview_io_auth_fixed_buffers_preserve_every_fragment_shape_and_per_poll_operation_limit() {
        for length in [HELLO, CHALLENGE, PROOF, ACCEPT] {
            for maximum in 1..=length {
                let expected: Vec<_> = (0..length).map(|n| n as u8).collect();
                let mut io = Fragment {
                    input: expected.iter().copied().collect(),
                    output: Vec::new(),
                    maximum,
                    interrupt: true,
                    eof: false,
                    calls: 0,
                };
                let mut reader = Buffer::read(length);
                for _ in 0..length + 2 {
                    let mut operations = 4;
                    let before = io.calls;
                    let done = reader.poll(&mut io, false, &mut operations).unwrap();
                    assert!(io.calls - before <= 4);
                    if done {
                        break;
                    }
                }
                assert_eq!(reader.input(), expected);
                let mut writer = Buffer::write(&expected);
                io.interrupt = true;
                for _ in 0..length + 2 {
                    let mut operations = 4;
                    if writer.poll(&mut io, true, &mut operations).unwrap() {
                        break;
                    }
                }
                assert_eq!(io.output, expected);
            }
        }
    }
    #[test]
    fn preview_io_auth_eof_and_wouldblock_never_complete_partial_or_reuse_old_bytes() {
        for cut in 0..HELLO {
            let mut io = Fragment {
                input: vec![7; cut].into(),
                output: Vec::new(),
                maximum: 3,
                interrupt: false,
                eof: true,
                calls: 0,
            };
            let mut reader = Buffer::read(HELLO);
            let mut failed = false;
            for _ in 0..HELLO {
                let mut operations = 4;
                if matches!(reader.poll(&mut io, false, &mut operations), Err(Error::Closed)) {
                    failed = true;
                    break;
                }
            }
            assert!(failed);
        }
        let mut io = Fragment {
            input: Default::default(),
            output: Vec::new(),
            maximum: 0,
            interrupt: false,
            eof: false,
            calls: 0,
        };
        let mut reader = Buffer::read(HELLO);
        for _ in 0..100 {
            let mut operations = 4;
            assert!(!reader.poll(&mut io, false, &mut operations).unwrap());
            assert_eq!(reader.position, 0);
            assert_eq!(operations, 3);
        }
    }
}
