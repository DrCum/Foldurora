//! Native configuration acknowledgement only. Policy, layout and pixels never
//! stand in for actual native application; exact source/connection joins remain
//! mandatory at both owners. No clock, render grant or arbitrary text is carried.
use super::{PreviewSource, WireError};
use crate::{PreviewRate, PreviewSize};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DemandConfiguration {
    pub size: PreviewSize,
    pub rate: PreviewRate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DemandRejection {
    Denied,
    Unsupported,
    Busy,
    Stale,
    Expired,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DemandOutcome {
    Accepted {
        effective: DemandConfiguration,
        native_revision: u64,
    },
    Rejected {
        reason: DemandRejection,
    },
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DemandReceipt {
    pub source: PreviewSource,
    pub metadata_connection: u64,
    pub demand: u64,
    pub requested: DemandConfiguration,
    pub outcome: DemandOutcome,
}
impl DemandReceipt {
    pub fn validate(self) -> Result<(), WireError> {
        if self.metadata_connection == 0 || self.demand == 0 {
            return Err(WireError::Sequence);
        }
        match self.outcome {
            DemandOutcome::Accepted { native_revision: 0, .. } => Err(WireError::Sequence),
            DemandOutcome::Accepted { effective, .. } if effective != self.requested => Err(WireError::Binding),
            _ => Ok(()),
        }
    }
}
impl std::fmt::Debug for DemandReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeDemandReceipt([redacted])")
    }
}
