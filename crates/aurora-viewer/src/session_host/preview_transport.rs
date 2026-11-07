//! Unenabled image-lane prerequisite: authenticated fixed-role framing over
//! caller-owned nonblocking Read/Write. No endpoint, socket, thread, bootstrap,
//! image file or App/native/render bridge is started or modified here.
//! SourceImages remains the separate final authority; a valid MAC grants none.

mod admission;
mod auth;
mod captured;
mod stream;

use aurora_session::preview_image::PreviewSource;
use aurora_session::preview_wire::{Direction, Lane, WireError};
use std::io;

pub(super) use super::preview_bootstrap::PreviewSecret;
pub(super) use admission::{Admissions, RoundRobin};
pub(super) use auth::{Authenticated, HostHandshake, WorkerHandshake};
pub(super) use stream::{AdmittedPixels, Pair, Received, TickBudget, UnadmittedPixels, worker_storage_ceiling};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Invalid,
    Authentication,
    Binding,
    Direction,
    Sequence,
    Capacity,
    Busy,
    Closed,
    Expired,
    CaptureRevoked,
    StaleSnapshot,
    Io(io::ErrorKind),
}

impl From<WireError> for Error {
    fn from(error: WireError) -> Self {
        match error {
            WireError::Binding => Self::Binding,
            WireError::Direction => Self::Direction,
            WireError::Sequence => Self::Sequence,
            _ => Self::Invalid,
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("preview lane refused the operation")
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Host,
    Worker,
}
impl Side {
    fn outgoing(self) -> Direction {
        match self {
            Self::Host => Direction::HostToWorker,
            Self::Worker => Direction::WorkerToHost,
        }
    }
    fn incoming(self) -> Direction {
        match self {
            Self::Host => Direction::WorkerToHost,
            Self::Worker => Direction::HostToWorker,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Scope {
    pub source: PreviewSource,
    pub connection: u64,
    pub role_epoch: u64,
    pub lane: Lane,
}

impl Scope {
    fn valid(self) -> bool {
        self.connection != 0 && self.role_epoch != 0 && (self.lane != Lane::Metadata || self.role_epoch == 1)
    }
}

#[cfg(test)]
mod prerequisite_tests;
#[cfg(test)]
mod tests;
