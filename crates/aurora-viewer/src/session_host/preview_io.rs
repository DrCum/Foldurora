//! Disabled bounded image IO core. Only dedicated threads touch sockets/HMAC;
//! callers exchange small typed snapshots or move already-credited captures.
//! This module is cfg(test) until actual native/Broker consumers are reviewed.
//! Synthetic fixture permits are not evidence of native/control authority.
pub(super) mod guards;
mod handshake;
mod host;
mod service;
mod socket;
mod state;
mod worker;

use super::preview_bootstrap::{PreviewBootstrap, PreviewSecret};
use super::preview_transport::{self as lane, Error, Pair, Received, Scope, TickBudget, UnadmittedPixels};
use aurora_session::preview_image::{CaptureClaim, ImageBudget, PreviewLayout, PreviewPolicy, PreviewSource, PreviewStamp};
use aurora_session::preview_wire::{Hello, Lane, Metadata, Unavailable};
use aurora_session::{HostId, InstallationId, MAX_WORKERS, PreviewRate, PreviewSize, WorkerIdentity};
use std::time::{Duration, Instant};

pub(crate) use guards::{CaptureControlGuard, CapturePermitGuard, ControlGuard, EpochFence, SourceRetentionGuard};
pub(crate) use service::{HostChannel, WorkerChannel};
pub(crate) use state::{HostPermit, HostUpdate, ReceivedCommand, SourceLease, WorkerState};

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
