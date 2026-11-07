//! Bounded untrusted rendezvous; no session/account/grid or secret is disclosed.
//! Only a current owner permit can turn this identity hint into authentication.
use super::{Lane, WireError};
use crate::{HostId, InstallationId, WorkerId, WorkerIdentity};

pub const HELLO_BYTES: usize = 96;
const HELLO_VERSION: u16 = 1;
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Hello {
    identity: WorkerIdentity,
    lease_epoch: u64,
    lane: Lane,
    connection: u64,
}
impl std::fmt::Debug for Hello {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreviewHello([untrusted, redacted])")
    }
}
impl Hello {
    pub fn new(identity: WorkerIdentity, lease_epoch: u64, lane: Lane, connection: u64) -> Result<Self, WireError> {
        if lease_epoch == 0 || (lane == Lane::Metadata && connection != 0) || (lane == Lane::Pixels && connection == 0) {
            return Err(WireError::Binding);
        }
        Ok(Self {
            identity,
            lease_epoch,
            lane,
            connection,
        })
    }
    pub fn identity(self) -> WorkerIdentity {
        self.identity
    }
    pub fn lease_epoch(self) -> u64 {
        self.lease_epoch
    }
    pub fn lane(self) -> Lane {
        self.lane
    }
    pub fn connection(self) -> u64 {
        self.connection
    }
    pub fn encode(self) -> [u8; HELLO_BYTES] {
        let mut b = [0; HELLO_BYTES];
        b[..4].copy_from_slice(b"APHI");
        b[4..6].copy_from_slice(&HELLO_VERSION.to_be_bytes());
        b[6] = self.lane as u8;
        b[8..40].copy_from_slice(self.identity.installation.as_bytes());
        b[40..56].copy_from_slice(self.identity.host.as_bytes());
        b[56..72].copy_from_slice(self.identity.worker.as_bytes());
        b[72..80].copy_from_slice(&self.lease_epoch.to_be_bytes());
        b[80..88].copy_from_slice(&self.connection.to_be_bytes());
        b
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let b: &[u8; HELLO_BYTES] = bytes.try_into().map_err(|_| WireError::Invalid)?;
        if &b[..4] != b"APHI" || b[7] != 0 || b[88..].iter().any(|v| *v != 0) {
            return Err(WireError::Invalid);
        }
        if u16::from_be_bytes([b[4], b[5]]) != HELLO_VERSION {
            return Err(WireError::Version);
        }
        let lane = match b[6] {
            1 => Lane::Metadata,
            2 => Lane::Pixels,
            _ => return Err(WireError::Invalid),
        };
        let identity = WorkerIdentity {
            installation: InstallationId::from_bytes(std::array::from_fn(|i| b[8 + i])).map_err(|_| WireError::Binding)?,
            host: HostId::from_bytes(std::array::from_fn(|i| b[40 + i])).map_err(|_| WireError::Binding)?,
            worker: WorkerId::from_bytes(std::array::from_fn(|i| b[56 + i])).map_err(|_| WireError::Binding)?,
        };
        Self::new(
            identity,
            u64::from_be_bytes(std::array::from_fn(|i| b[72 + i])),
            lane,
            u64::from_be_bytes(std::array::from_fn(|i| b[80 + i])),
        )
    }
}
