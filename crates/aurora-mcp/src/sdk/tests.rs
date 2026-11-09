use super::*;

#[test]
fn output_bound_is_enforced_during_serialization() {
    assert!(encode(&json!({"text":"x".repeat(MESSAGE_LIMIT)})).is_err());
    assert!(encode(&json!({"status":"ok"})).is_ok());
}

#[test]
fn only_closed_read_tool_arguments_are_accepted() {
    assert!(parse_read(&CallToolRequestParams::new("controller_list")).is_ok());
    for name in ["session_logout", "send_message", "AssistantConfigure", "run_command"] {
        assert!(parse_read(&CallToolRequestParams::new(name)).is_err());
    }
    assert!(
        parse_read(&CallToolRequestParams::new("controller_list").with_arguments(json!({"approve":true}).as_object().unwrap().clone()))
            .is_err()
    );
    assert_eq!(tools().len(), 4);
    assert!(tools().iter().all(|t| t.annotations.as_ref().unwrap().read_only_hint == Some(true)));
}

#[test]
fn handshake_versions_exclude_initialization_free_protocol() {
    let handler = Handler {
        admission: Arc::new(Mutex::new(Admission::default())),
    };
    assert_eq!(handler.get_info().protocol_version, ProtocolVersion::V_2025_11_25);
    assert!(handler.supported_protocol_versions().iter().all(ProtocolVersion::has_initialize));
    assert!(handler.get_info().capabilities.tools.is_some());
    assert!(handler.get_info().capabilities.resources.is_none());
}

struct Incoming(std::collections::VecDeque<RxJsonRpcMessage<RoleServer>>);
impl Transport<RoleServer> for Incoming {
    type Error = io::Error;
    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        self.0.pop_front()
    }
    fn send(&mut self, _: TxJsonRpcMessage<RoleServer>) -> impl Future<Output = io::Result<()>> + Send + 'static {
        std::future::ready(Ok(()))
    }
    async fn close(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn rpc(value: Value) -> RxJsonRpcMessage<RoleServer> {
    serde_json::from_value(value).unwrap()
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

#[test]
fn admission_precedes_sdk_task_creation_and_duplicate_ids_close() {
    runtime().block_on(async {
        for duplicate in [false, true] {
            let requests = (0..=REQUEST_CAPACITY)
                .map(|i| rpc(json!({"jsonrpc":"2.0","id":if duplicate {0}else{i},"method":"ping"})))
                .collect();
            let state = Arc::new(Mutex::new(Admission::default()));
            let mut transport = BoundedTransport {
                inner: Incoming(requests),
                state: state.clone(),
            };
            let accepted = if duplicate { 1 } else { REQUEST_CAPACITY };
            for _ in 0..accepted {
                assert!(transport.receive().await.is_some());
            }
            assert!(transport.receive().await.is_none());
            assert!(state.lock().unwrap().closed);
            assert_eq!(state.lock().unwrap().requests.len(), accepted);
        }
    });
}

#[test]
fn cancellation_retires_connection_instead_of_leaking_sdk_dropped_response() {
    runtime().block_on(async {
        let incoming = vec![
            rpc(json!({"jsonrpc":"2.0","id":1,"method":"ping"})),
            rpc(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}})),
        ];
        let state = Arc::new(Mutex::new(Admission::default()));
        let mut transport = BoundedTransport {
            inner: Incoming(incoming.into()),
            state: state.clone(),
        };
        assert!(transport.receive().await.is_some());
        assert!(transport.receive().await.is_none());
        assert!(state.lock().unwrap().closed);
    });
}

#[test]
fn unadvertised_notification_flood_never_reaches_sdk_tasks() {
    runtime().block_on(async {
        let mut incoming: std::collections::VecDeque<_> = (0..100)
            .map(|_| rpc(json!({"jsonrpc":"2.0","method":"notifications/arbitrary"})))
            .collect();
        incoming.push_back(rpc(json!({"jsonrpc":"2.0","id":1,"method":"ping"})));
        let state = Arc::new(Mutex::new(Admission::default()));
        let mut transport = BoundedTransport {
            inner: Incoming(incoming),
            state: state.clone(),
        };
        assert!(matches!(transport.receive().await, Some(JsonRpcMessage::Request(_))));
        assert_eq!(state.lock().unwrap().requests.len(), 1);
    });
}

#[test]
fn missing_descriptor_or_closed_transport_never_delivers_queued_placeholder() {
    runtime().block_on(async {
        for closed in [false, true] {
            let root = std::env::temp_dir().join(format!("mcp-writer-test-{}", uuid::Uuid::new_v4()));
            let discovery = Discovery::fixture(&root, aurora_session::InstallationId::from_bytes([1; 32]).unwrap()).unwrap();
            let backend = Arc::new(Mutex::new(Backend {
                discovery,
                clients: BTreeMap::new(),
            }));
            let state = Arc::new(Mutex::new(Admission {
                closed,
                ..Default::default()
            }));
            let (mut tx, rx) = tokio::io::duplex(MESSAGE_LIMIT);
            tx.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"structuredContent\":{\"status\":\"queued\"}}}\n")
                .await
                .unwrap();
            drop(tx);
            assert!(forward_output(rx, tokio::io::sink(), state, backend).await.is_err());
            std::fs::remove_dir_all(root).unwrap();
        }
    });
}
