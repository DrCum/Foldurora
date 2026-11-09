//! Private current-user host IPC. MCP itself remains on the sidecar's stdio.
//!
//! Folderstorm reference: controller_pipe.h/controller_host.inc at
//! abbe3cb0ca66ee2484f277219f6a27706fab7fc6. This transport deliberately uses a
//! separate, bounded JSON envelope; it is not compatible with FSC9 or worker IPC.
use crate::wire::{HostHandler, HostRequest};
use aurora_session::{HostId, InstallationId};
use serde::{Deserialize, Serialize};
use std::fs::{File, Metadata};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[cfg(unix)]
#[path = "ipc/unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "ipc/windows.rs"]
mod platform;

/// Entire private frame limits, including the four-byte length prefix.
pub const REQUEST_LIMIT: usize = 4096;
pub const RESPONSE_LIMIT: usize = 65_536;
const DESCRIPTOR_LIMIT: u64 = 2048;
const DESCRIPTOR_COUNT: usize = 64;
const CLIENT_LIMIT: usize = 4;
const DEADLINE: Duration = Duration::from_secs(5);
// A human can leave an Ask receipt pending while the stdio client is idle.
// This is distinct from the five-second deadline after a frame starts arriving.
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn denied() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "private host access refused")
}

/// Deliberately cannot serialize or debug private discovery credentials/paths.
pub struct Descriptor {
    wire: DescriptorWire,
}
impl Descriptor {
    pub fn instance(&self) -> HostId {
        self.wire.instance
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptorWire {
    version: u32,
    installation: InstallationId,
    instance: HostId,
    secret: String,
}

#[derive(Clone)]
pub struct Discovery {
    root: PathBuf,
    installation: InstallationId,
}
impl Discovery {
    /// Derives the viewer installation's namespace; the sidecar binary is never hashed.
    pub fn production(installation: InstallationId) -> io::Result<Self> {
        let dirs = directories::ProjectDirs::from("org", "Aurora", "AuroraViewer")
            .ok_or_else(|| invalid("current-user host directory unavailable"))?;
        let data = dirs.data_local_dir();
        ordinary_directories(data)?;
        let host = data.join("session-host");
        platform::private_directory(&host)?;
        let mcp = host.join("mcp");
        platform::private_directory(&mcp)?;
        let root = mcp.join(installation.to_string());
        platform::private_directory(&root)?;
        Ok(Self { root, installation })
    }

    /// Trusted synthetic harness only. No production CLI accepts this root.
    pub fn fixture(root: &Path, installation: InstallationId) -> io::Result<Self> {
        if !root.is_absolute() {
            return Err(invalid("synthetic namespace must be absolute"));
        }
        let parent = root.parent().ok_or_else(|| invalid("synthetic namespace has no parent"))?;
        ordinary_directories(parent)?;
        platform::private_directory(root)?;
        let root = root.join(installation.to_string());
        platform::private_directory(&root)?;
        Ok(Self { root, installation })
    }

    fn checked(&self) -> io::Result<()> {
        ordinary_existing(&self.root)?;
        platform::validate_directory(&self.root)?;
        // Production's private ancestors are checked too. Synthetic fixture's
        // parent is private; the ordinary root above it may be the system TEMP.
        let parent = self.root.parent().ok_or_else(|| invalid("missing namespace parent"))?;
        platform::validate_directory(parent)?;
        if parent.file_name().is_some_and(|n| n == "mcp") {
            platform::validate_directory(parent.parent().ok_or_else(|| invalid("missing host root"))?)?;
        }
        Ok(())
    }

    fn path(&self, instance: HostId) -> PathBuf {
        self.root.join(format!("controller-{instance}.json"))
    }

    fn read(&self, instance: HostId) -> io::Result<DescriptorWire> {
        self.checked()?;
        let path = self.path(instance);
        let before = std::fs::symlink_metadata(&path)?;
        platform::validate_file(&path, &before)?;
        if !before.is_file() || before.len() > DESCRIPTOR_LIMIT {
            return Err(invalid("invalid host descriptor"));
        }
        let mut file = platform::open_read(&path)?;
        let opened = file.metadata()?;
        let opened_identity = platform::file_identity(&file)?;
        platform::validate_file(&path, &opened)?;
        if !platform::same_object(&before, &opened) {
            return Err(denied());
        }
        let mut data = Vec::with_capacity(DESCRIPTOR_LIMIT as usize);
        Read::by_ref(&mut file).take(DESCRIPTOR_LIMIT + 1).read_to_end(&mut data)?;
        if data.len() as u64 > DESCRIPTOR_LIMIT || platform::path_identity(&path)? != opened_identity {
            return Err(invalid("host descriptor changed"));
        }
        let wire: DescriptorWire = serde_json::from_slice(&data).map_err(|_| invalid("invalid host descriptor"))?;
        if wire.version != 1 || wire.installation != self.installation || wire.instance != instance || !valid_secret(&wire.secret) {
            return Err(invalid("invalid host descriptor identity"));
        }
        Ok(wire)
    }

    pub fn list(&self) -> io::Result<Vec<Descriptor>> {
        self.checked()?;
        let mut out = Vec::new();
        let mut count = 0;
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(id) = name.strip_prefix("controller-").and_then(|n| n.strip_suffix(".json")) else {
                continue;
            };
            count += 1;
            if count > DESCRIPTOR_COUNT {
                return Err(invalid("host descriptor capacity exceeded"));
            }
            let Ok(instance) = HostId::parse_hex(id) else {
                continue;
            };
            if let Ok(wire) = self.read(instance) {
                out.push(Descriptor { wire });
            }
        }
        out.sort_by_key(Descriptor::instance);
        Ok(out)
    }

    pub fn connect(&self, instance: HostId) -> io::Result<Client> {
        let wire = self.read(instance)?;
        let stream = platform::Stream::connect(&self.root, instance)?;
        if self.read(instance)? != wire {
            return Err(denied());
        }
        Ok(Client {
            discovery: self.clone(),
            wire,
            stream,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestEnvelope {
    version: u32,
    installation: InstallationId,
    instance: HostId,
    secret: String,
    request: HostRequest,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseEnvelope {
    version: u32,
    instance: HostId,
    result: serde_json::Value,
}

pub struct Client {
    discovery: Discovery,
    wire: DescriptorWire,
    stream: platform::Stream,
}
impl Client {
    pub fn call(&mut self, request: HostRequest) -> io::Result<serde_json::Value> {
        let deadline = Instant::now() + DEADLINE;
        if self.discovery.read(self.wire.instance)? != self.wire {
            return Err(denied());
        }
        let envelope = RequestEnvelope {
            version: 1,
            installation: self.wire.installation,
            instance: self.wire.instance,
            secret: self.wire.secret.clone(),
            request,
        };
        write_frame_until(&mut self.stream, &envelope, REQUEST_LIMIT, deadline)?;
        let bytes = read_frame_until(&mut self.stream, RESPONSE_LIMIT, deadline)?;
        let response: ResponseEnvelope = serde_json::from_slice(&bytes).map_err(|_| invalid("invalid private host response"))?;
        if response.version != 1 || response.instance != self.wire.instance || self.discovery.read(self.wire.instance)? != self.wire {
            return Err(denied());
        }
        Ok(response.result)
    }
}

pub struct Server {
    instance: HostId,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
struct Publication {
    discovery: Discovery,
    instance: HostId,
    identity: platform::FileIdentity,
    wire: Option<DescriptorWire>,
}
impl Drop for Publication {
    fn drop(&mut self) {
        let path = self.discovery.path(self.instance);
        if self.discovery.checked().is_ok()
            && platform::path_identity(&path).is_ok_and(|identity| identity == self.identity)
            && self
                .wire
                .as_ref()
                .is_none_or(|wire| self.discovery.read(self.instance).is_ok_and(|current| current == *wire))
        {
            let _ = std::fs::remove_file(path);
        }
    }
}
impl Server {
    pub fn start(discovery: Discovery, enabled: bool, handler: Arc<dyn HostHandler>) -> io::Result<Self> {
        if !enabled {
            return Err(denied());
        }
        discovery.checked()?;
        if discovery.list()?.len() >= DESCRIPTOR_COUNT {
            return Err(invalid("host descriptor capacity exceeded"));
        }
        let instance = HostId::from_bytes(*Uuid::new_v4().as_bytes()).map_err(|_| invalid("host identity unavailable"))?;
        let secret = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let wire = DescriptorWire {
            version: 1,
            installation: discovery.installation,
            instance,
            secret,
        };
        let listener = platform::Listener::bind(&discovery.root, instance)?;
        let path = discovery.path(instance);
        let mut file = platform::create_private_file(&path)?;
        let mut publication = Publication {
            discovery: discovery.clone(),
            instance,
            identity: platform::file_identity(&file)?,
            wire: None,
        };
        let bytes = serde_json::to_vec(&wire).map_err(|_| invalid("host descriptor encoding failed"))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        if discovery.read(instance)? != wire {
            return Err(denied());
        }
        publication.wire = Some(wire.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = thread::Builder::new().name("host-mcp-listener".into()).spawn(move || {
            serve(listener, discovery, wire, stopping, handler, publication);
        })?;
        Ok(Self {
            instance,
            stop,
            thread: Some(thread),
        })
    }
    pub fn instance(&self) -> HostId {
        self.instance
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(
    mut listener: platform::Listener,
    discovery: Discovery,
    wire: DescriptorWire,
    stop: Arc<AtomicBool>,
    handler: Arc<dyn HostHandler>,
    _publication: Publication,
) {
    let mut workers: Vec<(JoinHandle<()>, platform::Cancel)> = Vec::with_capacity(CLIENT_LIMIT);
    while !stop.load(Ordering::Acquire) {
        let mut at = 0;
        while at < workers.len() {
            if workers[at].0.is_finished() {
                let (thread, _) = workers.swap_remove(at);
                let _ = thread.join();
            } else {
                at += 1;
            }
        }
        if !discovery.read(wire.instance).is_ok_and(|current| current == wire) {
            break;
        }
        if workers.len() < CLIENT_LIMIT {
            match listener.accept() {
                Ok((stream, cancel)) => {
                    let discovery = discovery.clone();
                    let wire = wire.clone();
                    let handler = handler.clone();
                    let id_bytes = *Uuid::new_v4().as_bytes();
                    let client = u64::from_le_bytes(id_bytes[..8].try_into().unwrap_or([1; 8])).max(1);
                    let worker = thread::Builder::new().name("host-mcp-client".into()).spawn(move || {
                        struct Disconnected(Arc<dyn HostHandler>, u64);
                        impl Drop for Disconnected {
                            fn drop(&mut self) {
                                self.0.disconnected(self.1);
                            }
                        }
                        let _guard = Disconnected(handler.clone(), client);
                        let _ = connection(stream, &discovery, &wire, client, &*handler);
                    });
                    if let Ok(worker) = worker {
                        workers.push((worker, cancel));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(_) => break,
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    for (_, cancel) in &workers {
        cancel.cancel();
    }
    for (worker, _) in workers {
        let _ = worker.join();
    }
}

fn connection(
    mut stream: platform::Stream,
    discovery: &Discovery,
    wire: &DescriptorWire,
    client: u64,
    handler: &dyn HostHandler,
) -> io::Result<()> {
    loop {
        let raw = read_request(&mut stream)?;
        let request: RequestEnvelope = serde_json::from_slice(&raw).map_err(|_| invalid("invalid private host request"))?;
        if request.version != 1
            || request.installation != wire.installation
            || request.instance != wire.instance
            || !constant_equal(request.secret.as_bytes(), wire.secret.as_bytes())
            || discovery.read(wire.instance)? != *wire
        {
            return Err(denied());
        }
        let result = handler.request(client, request.request);
        // Recheck before returning a read, including after synchronous native approval.
        if discovery.read(wire.instance)? != *wire {
            return Err(denied());
        }
        let response = ResponseEnvelope {
            version: 1,
            instance: wire.instance,
            result,
        };
        write_frame(&mut stream, &response, RESPONSE_LIMIT)?;
    }
}

fn valid_secret(secret: &str) -> bool {
    secret.len() == 64 && secret.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn constant_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |out, (a, b)| out | (a ^ b)) == 0
}

trait TimedStream: Read + Write {
    fn timeout(&mut self, timeout: Duration) -> io::Result<()>;
}
fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|n| !n.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "private host deadline"))
}
fn read_exact(stream: &mut impl TimedStream, mut bytes: &mut [u8], deadline: Instant) -> io::Result<()> {
    while !bytes.is_empty() {
        stream.timeout(remaining(deadline)?)?;
        match stream.read(bytes) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "private host disconnected")),
            Ok(n) => bytes = &mut bytes[n..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
#[cfg(test)]
fn read_frame(stream: &mut impl TimedStream, limit: usize) -> io::Result<Vec<u8>> {
    read_frame_until(stream, limit, Instant::now() + DEADLINE)
}
fn read_frame_until(stream: &mut impl TimedStream, limit: usize, deadline: Instant) -> io::Result<Vec<u8>> {
    let mut length = [0; 4];
    read_exact(stream, &mut length, deadline)?;
    read_body(stream, length, limit, deadline)
}
fn read_request(stream: &mut impl TimedStream) -> io::Result<Vec<u8>> {
    let mut length = [0; 4];
    read_exact(stream, &mut length[..1], Instant::now() + IDLE_TIMEOUT)?;
    let deadline = Instant::now() + DEADLINE;
    read_exact(stream, &mut length[1..], deadline)?;
    read_body(stream, length, REQUEST_LIMIT, deadline)
}
fn read_body(stream: &mut impl TimedStream, length: [u8; 4], limit: usize, deadline: Instant) -> io::Result<Vec<u8>> {
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > limit.saturating_sub(4) {
        return Err(invalid("private host frame exceeds limit"));
    }
    let mut bytes = vec![0; length];
    read_exact(stream, &mut bytes, deadline)?;
    Ok(bytes)
}
struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(invalid("private host response exceeds limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn write_frame(stream: &mut impl TimedStream, value: &impl Serialize, limit: usize) -> io::Result<()> {
    write_frame_until(stream, value, limit, Instant::now() + DEADLINE)
}
fn write_frame_until(stream: &mut impl TimedStream, value: &impl Serialize, limit: usize, deadline: Instant) -> io::Result<()> {
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        limit: limit.saturating_sub(4),
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| invalid("private host response exceeds limit"))?;
    let mut frame = Vec::with_capacity(writer.bytes.len() + 4);
    frame.extend_from_slice(&(writer.bytes.len() as u32).to_le_bytes());
    frame.extend_from_slice(&writer.bytes);
    let mut bytes = frame.as_slice();
    while !bytes.is_empty() {
        stream.timeout(remaining(deadline)?)?;
        match stream.write(bytes) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "private host disconnected")),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    stream.flush()
}

fn ordinary_directories(path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(invalid("host namespace must be absolute"));
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(invalid("host namespace traversal"));
        }
        current.push(component);
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        match std::fs::symlink_metadata(&current) {
            Ok(_) => platform::plain_directory(&current)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                std::fs::create_dir(&current)?;
                platform::plain_directory(&current)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
fn ordinary_existing(path: &Path) -> io::Result<()> {
    for parent in path.ancestors() {
        platform::plain_directory(parent)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "ipc/tests.rs"]
mod tests;
