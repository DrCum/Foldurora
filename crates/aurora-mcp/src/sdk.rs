//! Official MCP SDK adapter. Only typed read descriptors wait in queues;
//! metadata is fetched by the sole stdout writer immediately before delivery.
use crate::{
    ipc::{Client, Discovery},
    wire::{HostRequest, error},
};
use aurora_session::HostId;
use rmcp::{
    RoleServer, ServerHandler, ServiceExt,
    model::*,
    service::{RequestContext, RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{Transport, async_rw::AsyncRwTransport},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

pub const MESSAGE_LIMIT: usize = 65_536;
pub const REQUEST_CAPACITY: usize = 16;
const DEADLINE: Duration = Duration::from_secs(5);

#[derive(Clone)]
enum ReadCall {
    Controllers,
    Host(HostId, HostRequest),
}
#[derive(Default)]
struct Admission {
    requests: HashMap<RequestId, bool>,
    reads: HashMap<RequestId, ReadCall>,
    initialized_notification: bool,
    closed: bool,
}

struct Backend {
    discovery: Discovery,
    clients: BTreeMap<HostId, Client>,
}
impl Backend {
    fn read(&mut self, read: ReadCall) -> Value {
        match read {
            ReadCall::Controllers => match self.discovery.list() {
                Ok(rows) => {
                    json!({"status":"ok", "foundation_version":env!("CARGO_PKG_VERSION"), "controllers":rows.into_iter().map(|d| json!({"controller_instance":d.instance()})).collect::<Vec<_>>() })
                }
                Err(_) => error("controller_unavailable"),
            },
            ReadCall::Host(instance, request) => {
                if !self.clients.contains_key(&instance) {
                    if self.clients.len() >= 4 {
                        return error("capacity");
                    }
                    match self.discovery.connect(instance) {
                        Ok(client) => {
                            self.clients.insert(instance, client);
                        }
                        Err(_) => return error("controller_unavailable"),
                    }
                }
                match self.clients.get_mut(&instance).map(|c| c.call(request)) {
                    Some(Ok(reply)) => reply,
                    _ => {
                        self.clients.remove(&instance);
                        error("controller_unavailable")
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
struct Handler {
    admission: Arc<Mutex<Admission>>,
}
impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2025_11_25)
            .with_server_info(Implementation::new("foldurora-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions("Host reads require separate native host enablement and exact-profile authorization. Ask remains pending until an actual local approval. No character assistant or action tools are available.")
    }
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(ProtocolVersion::known_up_to(&ProtocolVersion::V_2025_11_25))
    }
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if request.is_some_and(|r| r.cursor.is_some()) {
            return Err(ErrorData::invalid_params("unsupported cursor", None));
        }
        Ok(ListToolsResult {
            tools: tools(),
            ..Default::default()
        })
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools().into_iter().find(|t| t.name == name)
    }
    async fn call_tool(&self, request: CallToolRequestParams, context: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        let read = parse_read(&request)?;
        let mut state = self
            .admission
            .lock()
            .map_err(|_| ErrorData::internal_error("transport unavailable", None))?;
        if state.closed || !state.requests.contains_key(&context.id) || state.reads.len() >= REQUEST_CAPACITY {
            return Err(ErrorData::internal_error("transport capacity", None));
        }
        state.reads.insert(context.id, read);
        // The writer substitutes a freshly authorized result; no protected body
        // is retained behind a slow client or an SDK response queue.
        Ok(CallToolResult::structured(json!({"status":"queued"})).into())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceArgs {
    controller_instance: HostId,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusArgs {
    controller_instance: HostId,
    profile_id: String,
    profile_revision: u64,
    session_ref: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationArgs {
    controller_instance: HostId,
    request_id: String,
}

fn parse_read(request: &CallToolRequestParams) -> Result<ReadCall, ErrorData> {
    let args = Value::Object(request.arguments.clone().unwrap_or_default());
    let invalid = || ErrorData::invalid_params("invalid host tool arguments", None);
    match request.name.as_ref() {
        "controller_list" if args.as_object().is_some_and(|a| a.is_empty()) => Ok(ReadCall::Controllers),
        "sessions_list" => {
            let a: InstanceArgs = serde_json::from_value(args).map_err(|_| invalid())?;
            Ok(ReadCall::Host(a.controller_instance, HostRequest::SessionsList))
        }
        "session_status" => {
            let a: StatusArgs = serde_json::from_value(args).map_err(|_| invalid())?;
            if a.profile_id.len() != 1
                || !matches!(a.profile_id.as_str(), "1" | "2" | "3" | "4" | "5")
                || a.session_ref.as_ref().is_some_and(|r| uuid::Uuid::parse_str(r).is_err())
            {
                return Err(invalid());
            }
            Ok(ReadCall::Host(
                a.controller_instance,
                HostRequest::SessionStatus {
                    profile_id: a.profile_id,
                    profile_revision: a.profile_revision,
                    session_ref: a.session_ref,
                },
            ))
        }
        "controller_operation_status" => {
            let a: OperationArgs = serde_json::from_value(args).map_err(|_| invalid())?;
            if uuid::Uuid::parse_str(&a.request_id).is_err() {
                return Err(invalid());
            }
            Ok(ReadCall::Host(
                a.controller_instance,
                HostRequest::OperationStatus { request_id: a.request_id },
            ))
        }
        "controller_list" => Err(invalid()),
        _ => Err(ErrorData::new(ErrorCode::METHOD_NOT_FOUND, "unsupported host tool/action", None)),
    }
}

fn tools() -> Vec<Tool> {
    [
        (
            "controller_list",
            "Discover explicitly enabled controllers for this installation; opaque references only.",
        ),
        (
            "sessions_list",
            "Read only explicitly permitted profiles; may return a pending native approval.",
        ),
        (
            "session_status",
            "Read one exact profile revision and login reference; no active-profile fallback.",
        ),
        (
            "controller_operation_status",
            "Poll this connection's original pending read; never approves it.",
        ),
    ]
    .into_iter()
    .map(|(name, description)| {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        if name != "controller_list" {
            properties.insert("controller_instance".into(), json!({"type":"string","pattern":"^[0-9a-f]{32}$"}));
            required.push("controller_instance");
        }
        if name == "session_status" {
            properties.insert("profile_id".into(), json!({"type":"string","enum":["1","2","3","4","5"]}));
            properties.insert("profile_revision".into(), json!({"type":"integer","minimum":1}));
            properties.insert("session_ref".into(), json!({"type":["string","null"],"maxLength":36}));
            required.extend(["profile_id", "profile_revision"]);
        }
        if name == "controller_operation_status" {
            properties.insert("request_id".into(), json!({"type":"string","maxLength":36}));
            required.push("request_id");
        }
        let schema = json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
        Tool::new(name, description, schema.as_object().expect("authored schema object").clone())
            .with_annotations(ToolAnnotations::new().read_only(true).open_world(false))
    })
    .collect()
}

struct BoundedTransport<T> {
    inner: T,
    state: Arc<Mutex<Admission>>,
}
impl<T: Transport<RoleServer>> Transport<RoleServer> for BoundedTransport<T> {
    type Error = io::Error;
    fn send(&mut self, item: TxJsonRpcMessage<RoleServer>) -> impl Future<Output = io::Result<()>> + Send + 'static {
        let bounded = encode(&item).is_ok();
        let send = self.inner.send(item);
        async move {
            if !bounded {
                return Err(io::Error::other("MCP output bound"));
            }
            tokio::time::timeout(DEADLINE, send)
                .await
                .map_err(|_| io::Error::other("MCP output deadline"))?
                .map_err(|_| io::Error::other("MCP transport closed"))
        }
    }
    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        loop {
            let item = self.inner.receive().await?;
            let Ok(mut state) = self.state.lock() else {
                return None;
            };
            if state.closed {
                return None;
            }
            match &item {
                JsonRpcMessage::Request(r) => {
                    if state.requests.len() >= REQUEST_CAPACITY || state.requests.contains_key(&r.id) {
                        state.closed = true;
                        return None;
                    }
                    state.requests.insert(r.id.clone(), false);
                }
                JsonRpcMessage::Notification(n) => {
                    // Unadvertised notifications never create SDK handler tasks.
                    let value = serde_json::to_value(n).ok()?;
                    match value["method"].as_str() {
                        Some("notifications/initialized") if !state.initialized_notification => state.initialized_notification = true,
                        Some("notifications/cancelled") => {
                            let id: RequestId = serde_json::from_value(value["params"]["requestId"].clone()).ok()?;
                            match state.requests.get_mut(&id) {
                                Some(_) => {
                                    // rmcp drops a cancelled request's response.
                                    // End this connection rather than retain an
                                    // admission slot or permit ID-reuse races.
                                    state.closed = true;
                                    return None;
                                }
                                _ => continue,
                            }
                        }
                        _ => continue,
                    }
                }
                _ => {
                    state.closed = true;
                    return None;
                }
            }
            return Some(item);
        }
    }
    async fn close(&mut self) -> io::Result<()> {
        self.inner.close().await.map_err(|_| io::Error::other("MCP transport closed"))
    }
}

struct Capped(Vec<u8>);
impl io::Write for Capped {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MESSAGE_LIMIT.saturating_sub(self.0.len()) {
            return Err(io::Error::other("MCP output bound"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn encode(value: &impl serde::Serialize) -> io::Result<Vec<u8>> {
    let mut output = Capped(Vec::new());
    serde_json::to_writer(&mut output, value).map_err(|_| io::Error::other("MCP output bound"))?;
    Ok(output.0)
}

async fn forward_output<W: AsyncWrite + Unpin>(
    input: tokio::io::DuplexStream,
    mut output: W,
    state: Arc<Mutex<Admission>>,
    backend: Arc<Mutex<Backend>>,
) -> io::Result<()> {
    let mut input = BufReader::new(input);
    loop {
        let mut line = Vec::new();
        let count = (&mut input).take((MESSAGE_LIMIT + 1) as u64).read_until(b'\n', &mut line).await?;
        if count == 0 {
            return Ok(());
        }
        if line.len() > MESSAGE_LIMIT || !line.ends_with(b"\n") {
            return Err(io::Error::other("MCP output bound"));
        }
        let mut message: Value = serde_json::from_slice(&line).map_err(|_| io::Error::other("MCP output encoding"))?;
        let id: Option<RequestId> = serde_json::from_value(message["id"].clone()).ok();
        let read = id.as_ref().and_then(|id| state.lock().ok()?.reads.remove(id));
        if state.lock().map_or(true, |s| s.closed) {
            return Err(io::Error::other("MCP transport closed"));
        }
        if message.get("error").is_none() && message["result"]["structuredContent"]["status"] == "queued" && read.is_none() {
            return Err(io::Error::other("MCP missing deferred read"));
        }
        if let Some(read) = read.filter(|_| message.get("error").is_none()) {
            let cancelled = id.as_ref().is_some_and(|id| {
                state
                    .lock()
                    .map_or(true, |s| s.closed || s.requests.get(id).copied().unwrap_or(true))
            });
            let value = if cancelled {
                error("cancelled")
            } else {
                let backend = backend.clone();
                tokio::task::spawn_blocking(move || backend.lock().map_or_else(|_| error("unavailable"), |mut b| b.read(read)))
                    .await
                    .map_err(|_| io::Error::other("MCP read failed"))?
            };
            let cancelled = id.as_ref().is_some_and(|id| {
                state
                    .lock()
                    .map_or(true, |s| s.closed || s.requests.get(id).copied().unwrap_or(true))
            });
            let value = if cancelled {
                if let Ok(mut backend) = backend.lock() {
                    backend.clients.clear();
                }
                error("cancelled")
            } else {
                value
            };
            let mut result = if value.get("error_code").is_some() {
                CallToolResult::structured_error(value)
            } else {
                CallToolResult::structured(value)
            };
            result.result_type = None; // Handshake protocol versions have no resultType.
            message["result"] = serde_json::to_value(result).map_err(|_| io::Error::other("MCP result encoding"))?;
        }
        if state.lock().map_or(true, |s| s.closed) {
            return Err(io::Error::other("MCP transport closed"));
        }
        let mut bytes = encode(&message)?;
        bytes.push(b'\n');
        tokio::time::timeout(DEADLINE, async {
            output.write_all(&bytes).await?;
            output.flush().await
        })
        .await
        .map_err(|_| io::Error::other("MCP stdout deadline"))??;
        if let Some(id) = id
            && let Ok(mut state) = state.lock()
        {
            state.requests.remove(&id);
            state.reads.remove(&id);
        }
    }
}

/// Runs the production SDK and transport; test harnesses inject only a trusted
/// discovery store, never an alternative permission or approval path.
pub async fn serve<R, W>(discovery: Discovery, input: R, output: W) -> io::Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let state = Arc::new(Mutex::new(Admission::default()));
    let backend = Arc::new(Mutex::new(Backend {
        discovery,
        clients: BTreeMap::new(),
    }));
    let (queue, queued) = tokio::io::duplex(MESSAGE_LIMIT);
    let mut writer = tokio::spawn(forward_output(queued, output, state.clone(), backend.clone()));
    let transport = BoundedTransport {
        inner: AsyncRwTransport::new_server(input, queue).with_max_line_length(MESSAGE_LIMIT),
        state: state.clone(),
    };
    let running = tokio::time::timeout(Duration::from_secs(30), Handler { admission: state.clone() }.serve(transport)).await;
    let result = match running {
        Ok(Ok(service)) => {
            tokio::select! {
                result = service.waiting() => result.map(|_| ()).map_err(|_| io::Error::other("MCP service ended")),
                result = &mut writer => match result {
                    Ok(Ok(())) if state.lock().is_ok_and(|state| !state.closed) => Ok(()),
                    _ => Err(io::Error::other("MCP output ended")),
                },
            }
        }
        _ => Err(io::Error::other("MCP initialization failed")),
    };
    if let Ok(mut state) = state.lock() {
        state.closed = true;
        state.reads.clear();
    }
    // Closing the stdio session closes all private connections and approvals.
    if let Ok(mut backend) = backend.lock() {
        backend.clients.clear();
    }
    writer.abort();
    result
}

#[cfg(test)]
mod tests;
