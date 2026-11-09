//! Closed private host operations; this wire is not the MCP protocol.
use serde::{Deserialize, Serialize};

pub type ClientId = u64;

/// Folderstorm controller_model.h's eight independent host authorities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionClass {
    Metadata,
    Activate,
    Background,
    Monitors,
    OpenLogin,
    Restart,
    Logout,
    Detach,
}
impl PermissionClass {
    pub const ALL: [Self; 8] = [
        Self::Metadata,
        Self::Activate,
        Self::Background,
        Self::Monitors,
        Self::OpenLogin,
        Self::Restart,
        Self::Logout,
        Self::Detach,
    ];
    pub const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostTarget {
    pub profile_id: String,
    pub profile_revision: u64,
    pub session_ref: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundMode {
    Warm,
    Economy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostAction {
    Activate,
    BackgroundMode { mode: BackgroundMode },
    OpenLogin,
    Restart,
    Logout,
    Detach,
    MonitorOpen,
    MonitorClose { monitor: u8 },
    MonitorsOpenAll,
    MonitorsArrange,
    MonitorCycleConfigure { interval_seconds: u32, playing: bool },
    MonitorCyclePause,
    MonitorCycleNext,
}
impl HostAction {
    pub fn class(&self) -> PermissionClass {
        match self {
            Self::Activate => PermissionClass::Activate,
            Self::BackgroundMode { .. } => PermissionClass::Background,
            Self::OpenLogin => PermissionClass::OpenLogin,
            Self::Restart => PermissionClass::Restart,
            Self::Logout => PermissionClass::Logout,
            Self::Detach => PermissionClass::Detach,
            _ => PermissionClass::Monitors,
        }
    }
    pub fn targeted(&self) -> bool {
        matches!(
            self,
            Self::Activate
                | Self::BackgroundMode { .. }
                | Self::OpenLogin
                | Self::Restart
                | Self::Logout
                | Self::Detach
                | Self::MonitorOpen
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostRequest {
    Assistant {
        request: crate::assistant::runtime::ViewerRequest,
    },
    MonitorList,
    Action {
        request_id: String,
        target: Option<HostTarget>,
        monitor_revision: Option<u64>,
        action: HostAction,
    },
    SessionsList,
    SessionStatus {
        profile_id: String,
        profile_revision: u64,
        session_ref: Option<String>,
    },
    OperationStatus {
        request_id: String,
    },
    /// Logical cancellation of only this authenticated connection's retained
    /// operation. It grants no profile, replacement login or native rollback.
    OperationCancel {
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
