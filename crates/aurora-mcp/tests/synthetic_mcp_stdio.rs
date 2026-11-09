//! Actual sidecar stdio -> current-user IPC -> typed synthetic host authority.
//! XDG_DATA_HOME isolates only this child; no grid, user profile or credentials.
#![cfg(target_os = "linux")]

use aurora_mcp::authority::{
    ApprovalDecision, Authority, AuthorityLevel, HostPolicy, MetadataLabel, NativePermittedDisplay, ProfileId, ProfilePhase,
    ProfileProjection,
};
use aurora_mcp::ipc::{Discovery, Server};
use aurora_mcp::sdk::MESSAGE_LIMIT;
use aurora_session::{AccountKey, Binding, GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerId, WorkerIdentity};
use serde_json::{Value, json};
use std::fs::{self, DirBuilder};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(8);

struct Fixture {
    base: PathBuf,
    data: PathBuf,
    installation: InstallationId,
    host: HostId,
    discovery: Discovery,
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!("foldurora-mcp-stdio-{}", uuid::Uuid::new_v4().simple()));
        DirBuilder::new().mode(0o700).create(&base).unwrap();
        let data = base.join("data");
        // Match Linux ProjectDirs::from("org", "Aurora", "AuroraViewer").
        let app = data.join("auroraviewer");
        fs::create_dir_all(&app).unwrap();
        let host_root = app.join("session-host");
        DirBuilder::new().mode(0o700).create(&host_root).unwrap();
        let installation = InstallationId::from_bytes([0x51; 32]).unwrap();
        let host = HostId::from_bytes([0x52; 16]).unwrap();
        let discovery = Discovery::fixture(&host_root.join("mcp"), installation).unwrap();
        Self {
            base,
            data,
            installation,
            host,
            discovery,
        }
    }

    fn projection(&self, number: u8, generation: u64) -> ProfileProjection {
        ProfileProjection {
            id: ProfileId::new(number).unwrap(),
            label: MetadataLabel::new(&format!("Synthetic character {number}")).unwrap(),
            phase: ProfilePhase::Online,
            binding: Some(Binding {
                worker: WorkerIdentity {
                    installation: self.installation,
                    host: self.host,
                    worker: WorkerId::from_bytes([number; 16]).unwrap(),
                },
                session: Some(SessionIdentity {
                    session: SessionId::from_bytes([number + 10; 16]).unwrap(),
                    account: AccountKey::from_bytes([number + 20; 32]).unwrap(),
                    grid: GridKey::from_bytes([number + 30; 32]).unwrap(),
                    generation,
                }),
            }),
            native_display: None,
        }
    }

    fn publish(&self, authority: &Authority, generation: u64) {
        authority
            .publish(vec![self.projection(1, generation), self.projection(2, 1)])
            .unwrap();
    }

    fn grant(&self, authority: &Authority, metadata: AuthorityLevel) {
        let ticket = authority.ticket(ProfileId::new(1).unwrap()).unwrap();
        authority.grant(ticket, metadata, AuthorityLevel::Never).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

struct Sidecar {
    child: Child,
    input: Option<ChildStdin>,
    lines: mpsc::Receiver<Result<Vec<u8>, String>>,
    reader: Option<JoinHandle<()>>,
    transcript: Vec<u8>,
    next_id: u64,
}

impl Sidecar {
    fn start(fixture: &Fixture) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_foldurora-mcp"))
            .args(["--installation", &fixture.installation.to_string()])
            .env("XDG_DATA_HOME", &fixture.data)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::sync_channel(64);
        let reader = thread::spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                let mut bytes = Vec::new();
                match Read::by_ref(&mut output)
                    .take((MESSAGE_LIMIT + 1) as u64)
                    .read_until(b'\n', &mut bytes)
                {
                    Ok(0) => break,
                    Ok(_) if bytes.len() <= MESSAGE_LIMIT && bytes.ends_with(b"\n") => {
                        if sender.send(Ok(bytes)).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {
                        let _ = sender.send(Err("sidecar emitted oversized or partial JSON line".into()));
                        break;
                    }
                    Err(error) => {
                        let _ = sender.send(Err(format!("sidecar stdout read failed: {:?}", error.kind())));
                        break;
                    }
                }
            }
        });
        Self {
            child,
            input: Some(input),
            lines,
            reader: Some(reader),
            transcript: Vec::new(),
            next_id: 1,
        }
    }

    fn send_raw(&mut self, bytes: &[u8]) {
        let input = self.input.as_mut().expect("sidecar stdin is open");
        input.write_all(bytes).unwrap();
        input.flush().unwrap();
    }

    fn send(&mut self, value: Value) {
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        self.send_raw(&bytes);
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}));
        let deadline = Instant::now() + RESPONSE_TIMEOUT;
        loop {
            let remaining = deadline.checked_duration_since(Instant::now()).expect("MCP response deadline");
            let bytes = self
                .lines
                .recv_timeout(remaining)
                .expect("sidecar did not return a bounded response")
                .expect("valid stdout framing");
            assert!(self.transcript.len() + bytes.len() <= 1024 * 1024, "bounded smoke transcript");
            self.transcript.extend_from_slice(&bytes);
            let response: Value = serde_json::from_slice(&bytes).expect("sidecar stdout contains only JSON-RPC");
            assert_eq!(response["jsonrpc"], "2.0");
            assert!(
                !(response.get("result").is_some() && response.get("error").is_some()),
                "JSON-RPC result and error are exclusive: {response}"
            );
            if response["id"] == id {
                return response;
            }
            // Invalid JSON can produce a null-id protocol error. Other unsolicited
            // responses are never allowed to masquerade as the requested result.
            assert!(
                response["id"].is_null() && response.get("error").is_some(),
                "unexpected MCP message: {response}"
            );
        }
    }

    fn initialize(&mut self, requested: &str, negotiated: &str) {
        let response = self.rpc(
            "initialize",
            json!({
                "protocolVersion": requested,
                "capabilities": {},
                "clientInfo": {"name":"foldurora-synthetic-smoke", "version":"1"},
            }),
        );
        assert!(response.get("error").is_none(), "initialization failed: {response}");
        assert_eq!(response["result"]["protocolVersion"], negotiated);
        assert!(response["result"]["capabilities"]["tools"].is_object());
        assert!(response["result"]["capabilities"].get("resources").is_none());
        self.send(json!({"jsonrpc":"2.0", "method":"notifications/initialized"}));
    }

    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        let response = self.rpc("tools/call", json!({"name":name, "arguments":arguments}));
        assert!(response.get("error").is_none(), "tool protocol error: {response}");
        let result = response["result"]["structuredContent"].clone();
        assert!(result.is_object(), "expected structured MCP tool result: {response}");
        assert_ne!(result["status"], "queued", "private placeholder escaped onto actual stdout");
        result
    }

    fn close(&mut self) -> ExitStatus {
        self.input.take();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                if let Some(reader) = self.reader.take() {
                    reader.join().unwrap();
                }
                return status;
            }
            assert!(Instant::now() < deadline, "sidecar did not exit after stdin EOF");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn assert_redacted(&self, fixture: &Fixture) {
        let output = String::from_utf8_lossy(&self.transcript);
        assert!(!output.contains(&fixture.base.to_string_lossy().to_string()));
        for name in [
            "secret",
            "binding",
            "installation",
            "worker",
            "session_id",
            "secure_session_id",
            "password",
            "chat_contents",
        ] {
            assert!(!output.contains(&format!("\"{name}\"")), "private field escaped: {name}");
        }
        for number in [1, 2] {
            let binding = fixture.projection(number, 1).binding.unwrap();
            let session = binding.session.unwrap();
            for id in [session.session.to_string(), session.account.to_string(), session.grid.to_string()] {
                assert!(!output.contains(&id), "internal source identity escaped");
            }
        }
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Drop the receiver before joining, so a blocked test-output send ends.
        let (_, empty) = mpsc::channel();
        self.lines = empty;
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn configure(authority: &Authority, metadata: AuthorityLevel) {
    authority
        .configure(HostPolicy {
            enabled: true,
            metadata,
            actions: AuthorityLevel::Never,
        })
        .unwrap();
}

fn status_args(instance: HostId, row: &Value) -> Value {
    json!({
        "controller_instance":instance,
        "profile_id":row["profile_id"],
        "profile_revision":row["profile_revision"],
        "session_ref":row["session_ref"],
    })
}

fn poll(sidecar: &mut Sidecar, instance: HostId, pending: &Value) -> Value {
    sidecar.tool(
        "controller_operation_status",
        json!({"controller_instance":instance,"request_id":pending["request_id"]}),
    )
}

#[test]
fn synthetic_mcp_stdio_host_authority_end_to_end() {
    let fixture = Fixture::new();
    let authority = Arc::new(Authority::new(fixture.installation, fixture.host));
    fixture.publish(&authority, 1);
    assert!(Server::start(fixture.discovery.clone(), false, authority.clone()).is_err());
    assert!(fixture.discovery.list().unwrap().is_empty());
    configure(&authority, AuthorityLevel::Never);
    let server = Server::start(fixture.discovery.clone(), true, authority.clone()).unwrap();
    let instance = server.instance();
    let host_args = json!({"controller_instance":instance});
    let mut sidecar = Sidecar::start(&fixture);
    sidecar.initialize("2025-11-25", "2025-11-25");

    let tools = sidecar.rpc("tools/list", json!({}));
    let tools = tools["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 4);
    for name in ["controller_list", "sessions_list", "session_status", "controller_operation_status"] {
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == name && tool["annotations"]["readOnlyHint"] == true)
        );
    }
    let controllers = sidecar.tool("controller_list", json!({}));
    assert_eq!(controllers["controllers"], json!([{"controller_instance":instance}]));
    assert_eq!(sidecar.tool("sessions_list", host_args.clone())["error_code"], "permission_denied");

    configure(&authority, AuthorityLevel::Allow);
    assert_eq!(sidecar.tool("sessions_list", host_args.clone())["sessions"], json!([]));
    fixture.grant(&authority, AuthorityLevel::Allow);
    let listed = sidecar.tool("sessions_list", host_args.clone());
    assert_eq!(listed["sessions"].as_array().unwrap().len(), 1);
    let old_row = listed["sessions"][0].clone();
    assert_eq!(old_row["profile_id"], "1");
    assert!(old_row.get("account").is_none());
    assert!(old_row.get("grid").is_none());
    assert_eq!(sidecar.tool("session_status", status_args(instance, &old_row))["session"], old_row);
    let mut other_profile = status_args(instance, &old_row);
    other_profile["profile_id"] = json!("2");
    assert_eq!(sidecar.tool("session_status", other_profile)["error_code"], "permission_denied");

    fixture.publish(&authority, 2);
    assert_eq!(
        sidecar.tool("sessions_list", host_args.clone())["sessions"],
        json!([]),
        "a stable slot cannot authorize replacement login"
    );
    fixture.grant(&authority, AuthorityLevel::Allow);
    assert_eq!(
        sidecar.tool("session_status", status_args(instance, &old_row))["error_code"],
        "stale_source"
    );
    let current = sidecar.tool("sessions_list", host_args.clone());
    assert_ne!(current["sessions"][0]["session_ref"], old_row["session_ref"]);

    // Only an explicit native-permitted display projection introduces names.
    let mut permitted = fixture.projection(1, 2);
    permitted.native_display = Some(NativePermittedDisplay {
        account: MetadataLabel::new("SYNTHETIC PERMITTED RESIDENT").unwrap(),
        grid: MetadataLabel::new("SYNTHETIC PERMITTED GRID").unwrap(),
    });
    authority.publish(vec![permitted, fixture.projection(2, 1)]).unwrap();
    let named = sidecar.tool("sessions_list", host_args.clone());
    assert_eq!(named["sessions"][0]["account"], "SYNTHETIC PERMITTED RESIDENT");
    fixture.publish(&authority, 2);
    assert!(
        sidecar.tool("sessions_list", host_args.clone())["sessions"][0]
            .get("account")
            .is_none()
    );

    configure(&authority, AuthorityLevel::Ask);
    let pending = sidecar.tool("sessions_list", host_args.clone());
    assert_eq!(pending["status"], "pending");
    assert_eq!(pending.as_object().unwrap().len(), 2, "Ask returns no metadata payload");
    assert_eq!(poll(&mut sidecar, instance, &pending), pending);
    assert_eq!(authority.pending_approvals().unwrap().len(), 1);
    // A real approval can take longer than the partial-frame timeout. Fresh
    // source observations keep the host current without resolving the approval.
    for _ in 0..2 {
        thread::sleep(Duration::from_secs(3));
        fixture.publish(&authority, 2);
    }
    assert_eq!(poll(&mut sidecar, instance, &pending), pending);
    authority
        .resolve(pending["request_id"].as_str().unwrap(), ApprovalDecision::AllowOnce)
        .unwrap();
    assert_eq!(poll(&mut sidecar, instance, &pending)["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(poll(&mut sidecar, instance, &pending)["error_code"], "operation_consumed");

    let denied = sidecar.tool("sessions_list", host_args.clone());
    authority
        .resolve(denied["request_id"].as_str().unwrap(), ApprovalDecision::Deny)
        .unwrap();
    assert_eq!(poll(&mut sidecar, instance, &denied)["error_code"], "permission_denied");
    let revoked = sidecar.tool("sessions_list", host_args.clone());
    authority
        .resolve(revoked["request_id"].as_str().unwrap(), ApprovalDecision::AllowOnce)
        .unwrap();
    authority.revoke(ProfileId::new(1).unwrap()).unwrap();
    assert_eq!(poll(&mut sidecar, instance, &revoked)["error_code"], "access_revoked");

    let unknown = sidecar.rpc("tools/call", json!({"name":"send_message","arguments":{}}));
    assert!(unknown.get("error").is_some(), "unsupported action must fail explicitly");
    sidecar.send_raw(b"{not valid JSON}\n");
    assert!(sidecar.rpc("ping", json!({})).get("result").is_some());
    sidecar.send_raw(b"{\"jsonrpc\":\"2.0\",\"id\":null,\"method\":42}\n");
    assert!(sidecar.rpc("ping", json!({})).get("result").is_some());
    let mut oversized = vec![b'x'; MESSAGE_LIMIT + 1];
    oversized.push(b'\n');
    sidecar.send_raw(&oversized);
    assert!(sidecar.rpc("ping", json!({})).get("result").is_some());

    fixture.publish(&authority, 2);
    fixture.grant(&authority, AuthorityLevel::Allow);
    let pending_at_eof = sidecar.tool("sessions_list", host_args);
    assert_eq!(pending_at_eof["status"], "pending");
    assert_eq!(authority.pending_approvals().unwrap().len(), 1);
    let status = sidecar.close();
    assert!(status.success(), "ordinary MCP stdin EOF should exit cleanly: {status}");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !authority.pending_approvals().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "stdio EOF did not revoke private pending reads");
        thread::sleep(Duration::from_millis(10));
    }
    sidecar.assert_redacted(&fixture);
    drop(server);
    assert!(fixture.discovery.list().unwrap().is_empty(), "owned discovery removed at shutdown");
}

#[test]
fn synthetic_stdio_negotiates_older_protocol_and_unknown_version_fallback() {
    let fixture = Fixture::new();
    for (requested, expected) in [("2025-06-18", "2025-06-18"), ("2099-01-01", "2025-11-25")] {
        let mut sidecar = Sidecar::start(&fixture);
        sidecar.initialize(requested, expected);
        assert!(sidecar.rpc("ping", json!({})).get("result").is_some());
        assert_eq!(sidecar.rpc("tools/list", json!({}))["result"]["tools"].as_array().unwrap().len(), 4);
        assert!(sidecar.close().success(), "negotiated session should close cleanly");
        sidecar.assert_redacted(&fixture);
    }
}
