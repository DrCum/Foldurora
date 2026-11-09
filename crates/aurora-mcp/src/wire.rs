//! Closed private host operations; this wire is not the MCP protocol.
use serde::{Deserialize, Serialize};

pub type ClientId = u64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostRequest {
    SessionsList,
    SessionStatus {
        profile_id: String,
        profile_revision: u64,
        session_ref: Option<String>,
    },
    OperationStatus {
        request_id: String,
    },
}

/// Implementations construct a whitelist response; no raw host state is serialized.
pub trait HostHandler: Send + Sync + 'static {
    fn request(&self, client: ClientId, request: HostRequest) -> serde_json::Value;
    fn disconnected(&self, client: ClientId);
}

pub fn error(code: &str) -> serde_json::Value {
    serde_json::json!({"status":"error", "error_code":code})
}
