use super::*;
use std::sync::atomic::AtomicUsize;

struct Fixture {
    path: PathBuf,
    discovery: Discovery,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("aurora-mcp-{}", Uuid::new_v4().simple()));
        let discovery = Discovery::fixture(&path, InstallationId::from_bytes([1; 32]).unwrap()).unwrap();
        Self { path, discovery }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
struct Handler {
    calls: AtomicUsize,
    closed: AtomicUsize,
}
impl Handler {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            closed: AtomicUsize::new(0),
        })
    }
}
impl HostHandler for Handler {
    fn request(&self, client: u64, request: HostRequest) -> serde_json::Value {
        self.calls.fetch_add(1, Ordering::Relaxed);
        assert_ne!(client, 0);
        match request {
            HostRequest::SessionsList => serde_json::json!({"profiles":[{"display_name":"Synthetic"}]}),
            _ => crate::wire::error("unsupported_action"),
        }
    }
    fn disconnected(&self, _: u64) {
        self.closed.fetch_add(1, Ordering::Relaxed);
    }
}
fn eventually(predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !predicate() {
        assert!(Instant::now() < deadline, "IPC event did not settle");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn actual_transport_discovery_persistent_calls_and_owned_cleanup() {
    let fixture = Fixture::new();
    let handler = Handler::new();
    assert!(Server::start(fixture.discovery.clone(), false, handler.clone()).is_err());
    assert!(fixture.discovery.list().unwrap().is_empty());
    let server = Server::start(fixture.discovery.clone(), true, handler.clone()).unwrap();
    let descriptors = fixture.discovery.list().unwrap();
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].instance(), server.instance());
    let mut client = fixture.discovery.connect(server.instance()).unwrap();
    assert_eq!(
        client.call(HostRequest::SessionsList).unwrap()["profiles"][0]["display_name"],
        "Synthetic"
    );
    assert_eq!(
        client.call(HostRequest::SessionsList).unwrap()["profiles"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(handler.calls.load(Ordering::Relaxed), 2);
    drop(client);
    eventually(|| handler.closed.load(Ordering::Relaxed) == 1);
    drop(server);
    assert!(fixture.discovery.list().unwrap().is_empty());
}

#[test]
fn malformed_and_oversized_frames_disconnect_without_dispatch() {
    let fixture = Fixture::new();
    let handler = Handler::new();
    let server = Server::start(fixture.discovery.clone(), true, handler.clone()).unwrap();
    for payload in [b"{\"bad\":true}".as_slice(), b"{".as_slice()] {
        let mut stream = platform::Stream::connect(&fixture.discovery.root, server.instance()).unwrap();
        stream.timeout(Duration::from_secs(1)).unwrap();
        stream.write_all(&(payload.len() as u32).to_le_bytes()).unwrap();
        stream.write_all(payload).unwrap();
        let mut out = [0; 1];
        assert_eq!(stream.read(&mut out).unwrap_or(0), 0);
    }
    let mut stream = platform::Stream::connect(&fixture.discovery.root, server.instance()).unwrap();
    stream.timeout(Duration::from_secs(1)).unwrap();
    stream.write_all(&((REQUEST_LIMIT + 1) as u32).to_le_bytes()).unwrap();
    let mut out = [0; 1];
    assert_eq!(stream.read(&mut out).unwrap_or(0), 0);
    assert_eq!(handler.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn wrong_secret_and_stale_incarnation_are_refused() {
    let fixture = Fixture::new();
    let handler = Handler::new();
    let server = Server::start(fixture.discovery.clone(), true, handler.clone()).unwrap();
    let wire = fixture.discovery.read(server.instance()).unwrap();
    let mut stream = platform::Stream::connect(&fixture.discovery.root, server.instance()).unwrap();
    let request = RequestEnvelope {
        version: 1,
        installation: wire.installation,
        instance: wire.instance,
        secret: "0".repeat(64),
        request: HostRequest::SessionsList,
    };
    write_frame(&mut stream, &request, REQUEST_LIMIT).unwrap();
    assert!(read_frame(&mut stream, RESPONSE_LIMIT).is_err());
    let mut client = fixture.discovery.connect(server.instance()).unwrap();
    let mut replacement = wire.clone();
    replacement.secret = "a".repeat(64);
    replace_fixture_descriptor(&fixture.discovery, &replacement);
    assert!(fixture.discovery.read(wire.instance).unwrap() == replacement);
    assert!(client.call(HostRequest::SessionsList).is_err());
    drop(server);
    assert!(
        fixture.discovery.path(wire.instance).exists(),
        "old service must preserve a replacement descriptor"
    );
    assert_eq!(handler.calls.load(Ordering::Relaxed), 0);
}

/// A fully published synthetic replacement, not an in-place truncation raced
/// against the live listener's strict read/delete/no-write Windows handles.
/// The replacement keeps the production private creator and validation path.
fn replace_fixture_descriptor(discovery: &Discovery, wire: &DescriptorWire) {
    discovery.checked().unwrap();
    let staged = discovery.root.join(format!("fixture-replacement-{}", Uuid::new_v4().simple()));
    let mut file = platform::create_private_file(&staged).unwrap();
    file.write_all(&serde_json::to_vec(wire).unwrap()).unwrap();
    file.sync_all().unwrap();
    drop(file);
    std::fs::rename(staged, discovery.path(wire.instance)).unwrap();
}
fn synthetic_wire(discovery: &Discovery) -> DescriptorWire {
    DescriptorWire {
        version: 1,
        installation: discovery.installation,
        instance: HostId::from_bytes(*Uuid::new_v4().as_bytes()).unwrap(),
        secret: "1".repeat(64),
        domain: discovery.domain,
    }
}
fn publish_fixture_file(discovery: &Discovery, wire: &DescriptorWire) -> platform::FileIdentity {
    let mut file = platform::create_private_file(&discovery.path(wire.instance)).unwrap();
    file.write_all(&serde_json::to_vec(wire).unwrap()).unwrap();
    file.sync_all().unwrap();
    platform::file_identity(&file).unwrap()
}
#[test]
fn descriptor_atomic_replacement_preserves_open_reader_and_validates_private_successor() {
    let fixture = Fixture::new();
    let wire = synthetic_wire(&fixture.discovery);
    let original = publish_fixture_file(&fixture.discovery, &wire);
    let mut reader = platform::open_read(&fixture.discovery.path(wire.instance)).unwrap();
    #[cfg(windows)]
    {
        // This deliberately held actual reader identifies OUR conflict. It is
        // not evidence identifying the handle from the old Windows failure.
        let error = std::fs::write(fixture.discovery.path(wire.instance), b"must not replace").unwrap_err();
        assert_eq!(error.raw_os_error(), Some(32));
        assert!(fixture.discovery.read(wire.instance).unwrap() == wire);
    }
    let mut replacement = wire.clone();
    replacement.secret = "a".repeat(64);
    replace_fixture_descriptor(&fixture.discovery, &replacement);
    assert!(fixture.discovery.read(wire.instance).unwrap() == replacement);
    assert!(platform::path_identity(&fixture.discovery.path(wire.instance)).unwrap() != original);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert!(serde_json::from_slice::<DescriptorWire>(&bytes).unwrap() == wire);
}
#[test]
fn original_publication_cleanup_preserves_changed_descriptor_with_the_same_file_identity() {
    let fixture = Fixture::new();
    let wire = synthetic_wire(&fixture.discovery);
    let identity = publish_fixture_file(&fixture.discovery, &wire);
    let publication = Publication {
        discovery: fixture.discovery.clone(),
        instance: wire.instance,
        identity,
        wire: Some(wire.clone()),
    };
    let mut replacement = wire.clone();
    replacement.secret = "a".repeat(64);
    // No listener/reader thread in this case. Exercise the distinct cleanup
    // wire-content guard after the file-identity guard succeeds, without races.
    std::fs::write(fixture.discovery.path(wire.instance), serde_json::to_vec(&replacement).unwrap()).unwrap();
    assert!(platform::path_identity(&fixture.discovery.path(wire.instance)).unwrap() == identity);
    assert!(fixture.discovery.read(wire.instance).unwrap() == replacement);
    drop(publication);
    assert!(fixture.discovery.read(wire.instance).unwrap() == replacement);
}

#[test]
fn service_shutdown_cancels_connected_clients() {
    let fixture = Fixture::new();
    let handler = Handler::new();
    let server = Server::start(fixture.discovery.clone(), true, handler.clone()).unwrap();
    let mut client = fixture.discovery.connect(server.instance()).unwrap();
    client.call(HostRequest::SessionsList).unwrap();
    let at = Instant::now();
    drop(server);
    assert!(at.elapsed() < Duration::from_secs(2));
    assert_eq!(handler.closed.load(Ordering::Relaxed), 1);
    assert!(client.call(HostRequest::SessionsList).is_err());
}

#[test]
fn idle_connection_survives_frame_deadline_for_human_approval_polling() {
    let fixture = Fixture::new();
    let handler = Handler::new();
    let server = Server::start(fixture.discovery.clone(), true, handler.clone()).unwrap();
    let mut client = fixture.discovery.connect(server.instance()).unwrap();
    client.call(HostRequest::SessionsList).unwrap();
    thread::sleep(DEADLINE + Duration::from_millis(150));
    assert_eq!(handler.closed.load(Ordering::Relaxed), 0);
    client.call(HostRequest::SessionsList).unwrap();
    assert_eq!(handler.calls.load(Ordering::Relaxed), 2);
}

#[test]
fn truncated_disconnect_does_not_dispatch_and_listener_survives() {
    let fixture = Fixture::new();
    let handler = Handler::new();
    let server = Server::start(fixture.discovery.clone(), true, handler.clone()).unwrap();
    let mut stream = platform::Stream::connect(&fixture.discovery.root, server.instance()).unwrap();
    stream.timeout(Duration::from_secs(1)).unwrap();
    stream.write_all(&128u32.to_le_bytes()).unwrap();
    stream.write_all(b"{").unwrap();
    drop(stream);
    eventually(|| handler.closed.load(Ordering::Relaxed) == 1);
    assert_eq!(handler.calls.load(Ordering::Relaxed), 0);
    let mut client = fixture.discovery.connect(server.instance()).unwrap();
    client.call(HostRequest::SessionsList).unwrap();
    assert_eq!(handler.calls.load(Ordering::Relaxed), 1);
}

#[cfg(unix)]
#[test]
fn only_four_connections_are_admitted_until_a_slot_closes() {
    let fixture = Fixture::new();
    let handler = Handler::new();
    let server = Server::start(fixture.discovery.clone(), true, handler.clone()).unwrap();
    let mut clients = Vec::new();
    for _ in 0..CLIENT_LIMIT {
        let mut client = fixture.discovery.connect(server.instance()).unwrap();
        client.call(HostRequest::SessionsList).unwrap();
        clients.push(client);
    }
    let wire = fixture.discovery.read(server.instance()).unwrap();
    let mut fifth = platform::Stream::connect(&fixture.discovery.root, wire.instance).unwrap();
    let request = RequestEnvelope {
        version: 1,
        installation: wire.installation,
        instance: wire.instance,
        secret: wire.secret,
        request: HostRequest::SessionsList,
    };
    write_frame(&mut fifth, &request, REQUEST_LIMIT).unwrap();
    fifth.timeout(Duration::from_millis(100)).unwrap();
    let mut byte = [0];
    let error = fifth.read(&mut byte).expect_err("fifth client must wait outside dispatch");
    assert!(matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut));
    assert_eq!(handler.calls.load(Ordering::Relaxed), CLIENT_LIMIT);
    clients.pop();
    let response = read_frame(&mut fifth, RESPONSE_LIMIT).unwrap();
    assert!(
        serde_json::from_slice::<ResponseEnvelope>(&response)
            .unwrap()
            .result
            .get("profiles")
            .is_some()
    );
    assert_eq!(handler.calls.load(Ordering::Relaxed), CLIENT_LIMIT + 1);
}

#[cfg(unix)]
#[test]
fn links_foreign_modes_and_descriptor_overflow_fail_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = Fixture::new();
    let handler = Handler::new();
    let server = Server::start(fixture.discovery.clone(), true, handler).unwrap();
    let path = fixture.discovery.path(server.instance());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(fixture.discovery.connect(server.instance()).is_err());
    // Test explicitly repairs its own synthetic file; production never does.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let alias = fixture.discovery.root.join(format!("controller-{}.json", Uuid::new_v4().simple()));
    symlink(&path, &alias).unwrap();
    assert_eq!(fixture.discovery.list().unwrap().len(), 1);
    for _ in 0..DESCRIPTOR_COUNT {
        let path = fixture.discovery.root.join(format!("controller-{}.json", Uuid::new_v4().simple()));
        platform::create_private_file(&path).unwrap();
    }
    assert!(fixture.discovery.list().is_err());
}

#[test]
fn framing_serialization_stops_at_bound() {
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        limit: 8,
    };
    assert!(serde_json::to_writer(&mut writer, &"x".repeat(128)).is_err());
    assert!(writer.bytes.len() <= 8);
}
