//! Capability-negotiated native host extension; repair Envelope V2 is unchanged.
//! A receipt never contains a simulator token, credential, path or chat body.
use crate::{Binding, Label};
use serde::{Deserialize, Serialize};

pub const VERSION: u16 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundMode {
    Warm,
    Economy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeAction {
    /// Preparation only; never opens native UI.
    RequestExit,
    RestoreExit,
    CommitExit,
    CancelExit,
    Background(BackgroundMode),
}

impl NativeAction {
    pub fn finalization(self) -> bool {
        matches!(self, Self::CommitExit | Self::CancelExit)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRequest {
    pub id: u64,
    pub action: NativeAction,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub exit: bool,
    pub background: bool,
    pub display: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermittedDisplay {
    pub account: Label,
    pub grid: Label,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeOutcome {
    Accepted,
    WaitingForRestore,
    ReadyForExit,
    Committed,
    PromptOpen,
    Cancelled,
    LogoutChosen,
    LoggedOut,
    QuitChosen,
    Applied,
    Rejected,
    Uncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeReceipt {
    pub request: NativeRequest,
    pub outcome: NativeOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub version: u16,
    pub binding: Binding,
    pub lease_epoch: u64,
    pub revision: u64,
    pub capabilities: Capabilities,
    pub display: Option<PermittedDisplay>,
    pub receipt: Option<NativeReceipt>,
}
impl Observation {
    pub fn validate(&self) -> bool {
        self.version == VERSION
            && self.revision != 0
            && self.lease_epoch != 0
            && self.binding.validate().is_ok()
            && self.binding.session.is_some()
            && self.receipt.is_none_or(|receipt| receipt.request.id != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_extension_rejects_unknown_action_and_extra_credential_fields() {
        assert!(serde_json::from_str::<NativeRequest>(r#"{"id":1,"action":"execute_command"}"#).is_err());
        assert!(serde_json::from_str::<NativeRequest>(r#"{"id":1,"action":"request_exit","password":"synthetic-forbidden"}"#).is_err());
        assert_eq!(
            serde_json::to_string(&NativeRequest {
                id: 1,
                action: NativeAction::RequestExit
            })
            .unwrap(),
            r#"{"id":1,"action":"request_exit"}"#
        );
    }
}
