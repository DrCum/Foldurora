//! Disabled bounded image IO core. Only dedicated threads touch sockets/HMAC;
//! callers exchange small typed snapshots or move already-credited captures.
//! This module is cfg(test) until actual native/Broker consumers are reviewed.
//! Synthetic fixture permits are not evidence of native/control authority.
mod demand;
pub(super) mod guards;
mod handshake;
mod host;
mod publication;
mod service;
mod socket;
mod state;
mod worker;

use super::preview_bootstrap::{PreviewBootstrap, PreviewSecret};
use super::preview_transport::{self as lane, Pair, Received, Scope, TickBudget, UnadmittedPixels};
use aurora_session::preview_image::{CaptureClaim, ImageBudget, PreviewLayout, PreviewPolicy, PreviewSource, PreviewStamp};
use aurora_session::preview_wire::{DemandConfiguration, DemandOutcome, DemandReceipt, Hello, Lane, Metadata, Unavailable};
use aurora_session::{HostId, InstallationId, MAX_WORKERS, PreviewRate, PreviewSize, WorkerIdentity};
use std::time::{Duration, Instant};

pub(crate) use super::preview_transport::Error;
pub(crate) use guards::{CaptureControlGuard, CapturePermitGuard, ControlGuard, EpochFence, SourceRetentionGuard};
pub(crate) use publication::{
    LayoutReceipt, ObservationDomain, PolicyReceipt, PublicationGuard, PublicationStamp, ReceiptOrigin, VersionedObservation,
    VersionedObservationSet,
};
pub(crate) use service::{HostChannel, WorkerChannel};
pub(crate) use state::{
    FrameHandoff, HostPermit, HostUpdate, Observation, ReceivedCommand, RetainedFrame, SourceLease, SourceUpdate, Status, WorkerState,
};

const PERIOD: Duration = Duration::from_millis(10);
const DEADLINE: Duration = Duration::from_secs(1);
const HEARTBEAT: Duration = Duration::from_millis(250);
const QUEUE: usize = 4;
const RAW_BYTES: usize = 32 * 1024 * 1024;
const UI_BYTES: usize = 24 * 1024 * 1024;
const SERVICE_BYTES: usize = 8 * 1024 * 1024;
const THREAD_STACK: usize = 256 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConnectionToken {
    source: PreviewSource,
    connection: u64,
    pixel_epoch: u64,
    incarnation: u64,
}
impl ConnectionToken {
    /// Transport correlation fixture only. This cannot mint source, control or
    /// native permission; real channels still require their registered token.
    #[cfg(test)]
    pub(crate) fn transport_fixture(source: PreviewSource, connection: u64, pixel_epoch: u64, incarnation: u64) -> Option<Self> {
        (connection != 0 && incarnation != 0).then_some(Self {
            source,
            connection,
            pixel_epoch,
            incarnation,
        })
    }

    pub fn same_metadata(self, other: Self) -> bool {
        self.source == other.source && self.connection == other.connection && self.incarnation == other.incarnation
    }
    pub fn source(self) -> PreviewSource {
        self.source
    }
    pub fn connection(self) -> u64 {
        self.connection
    }
    pub fn pixel_epoch(self) -> u64 {
        self.pixel_epoch
    }
}
impl std::fmt::Debug for ConnectionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreviewConnection([redacted])")
    }
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum PreviewCommand {
    RememberActive { demand: u64, size: PreviewSize, rate: PreviewRate },
    Capture(Box<CaptureClaim>),
    Stop { demand: u64 },
}
impl PreviewCommand {
    fn metadata(&self, token: ConnectionToken) -> Metadata {
        match self {
            Self::RememberActive { demand, size, rate } => Metadata::Demand {
                source: token.source,
                connection: token.connection,
                demand: *demand,
                size: *size,
                rate: *rate,
            },
            Self::Capture(claim) => Metadata::Capture(**claim),
            Self::Stop { demand } => Metadata::Stop {
                source: token.source,
                connection: token.connection,
                demand: *demand,
            },
        }
    }
}

#[cfg(test)]
mod tests;

/// Serialize owned test endpoints with the existing fixtures and global pools.
/// This guard is absent from production; no service operation waits on it.
#[cfg(test)]
pub(crate) fn owned_fixture_guard() -> std::sync::MutexGuard<'static, ()> {
    tests::OWNED.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
