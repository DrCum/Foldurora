//! Closed framing prerequisite for a dedicated last-active image lane. The
//! Legacy renderMonitor/sharingAllowed path (indra/newview/fssessionworker.cpp,
//! abbe3cb0ca66ee2484f277219f6a27706fab7fc6) motivates separate image/privacy
//! traffic; this wire grants no rendering, input, Read or Send authority.
//! Pixels are raw opaque top-down sRGB RGBA8, never serde/base64/image files.

use crate::preview_image::{CaptureClaim, MAX_IMAGE_BYTES, PreviewLayout, PreviewPolicy, PreviewSource, PreviewStamp};
use crate::{PreviewRate, PreviewSize};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};

pub const VERSION: u16 = 2;
pub const MAX_METADATA_BYTES: usize = 4096;
pub const HEADER_BYTES: usize = 48;
pub const TAG_BYTES: usize = 32;
pub const MAX_FRAME_BYTES: usize = HEADER_BYTES + MAX_METADATA_BYTES + MAX_IMAGE_BYTES + 2 * TAG_BYTES;

mod hello;
pub use hello::{HELLO_BYTES, Hello};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    Invalid,
    Version,
    Oversized,
    Direction,
    Binding,
    Sequence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Lane {
    Metadata = 1,
    Pixels = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Direction {
    HostToWorker = 1,
    WorkerToHost = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    lane: Lane,
    direction: Direction,
    connection: u64,
    role_epoch: u64,
    sequence: u64,
    metadata: u32,
    pixels: u32,
}

impl Header {
    pub fn new(
        lane: Lane,
        direction: Direction,
        connection: u64,
        role_epoch: u64,
        sequence: u64,
        metadata: usize,
        pixels: usize,
    ) -> Result<Self, WireError> {
        if connection == 0 || sequence == 0 || role_epoch == 0 {
            return Err(WireError::Sequence);
        }
        if metadata == 0 || metadata > MAX_METADATA_BYTES || pixels > MAX_IMAGE_BYTES {
            return Err(WireError::Oversized);
        }
        match lane {
            Lane::Metadata if pixels != 0 || role_epoch != 1 => return Err(WireError::Invalid),
            Lane::Pixels if direction != Direction::WorkerToHost => return Err(WireError::Direction),
            Lane::Pixels if pixels < 4 * 4 * 4 || !pixels.is_multiple_of(4) => return Err(WireError::Invalid),
            _ => {}
        }
        Ok(Self {
            lane,
            direction,
            connection,
            role_epoch,
            sequence,
            metadata: metadata as u32,
            pixels: pixels as u32,
        })
    }
    pub fn lane(self) -> Lane {
        self.lane
    }
    pub fn direction(self) -> Direction {
        self.direction
    }
    pub fn connection(self) -> u64 {
        self.connection
    }
    pub fn role_epoch(self) -> u64 {
        self.role_epoch
    }
    pub fn sequence(self) -> u64 {
        self.sequence
    }
    pub fn metadata_bytes(self) -> usize {
        self.metadata as usize
    }
    pub fn pixel_bytes(self) -> usize {
        self.pixels as usize
    }
    pub fn frame_bytes(self) -> usize {
        HEADER_BYTES + self.metadata_bytes() + self.pixel_bytes() + 2 * TAG_BYTES
    }

    /// All reserved bytes are canonical zero. Lengths are checked before a
    /// receiver even considers reserving image credit or allocating pixels.
    pub fn encode(self) -> [u8; HEADER_BYTES] {
        let mut bytes = [0; HEADER_BYTES];
        bytes[..4].copy_from_slice(b"APRV");
        bytes[4..6].copy_from_slice(&VERSION.to_be_bytes());
        bytes[6] = self.lane as u8;
        bytes[7] = self.direction as u8;
        bytes[8..16].copy_from_slice(&self.connection.to_be_bytes());
        bytes[16..24].copy_from_slice(&self.sequence.to_be_bytes());
        bytes[24..28].copy_from_slice(&self.metadata.to_be_bytes());
        bytes[28..32].copy_from_slice(&self.pixels.to_be_bytes());
        bytes[32..40].copy_from_slice(&self.role_epoch.to_be_bytes());
        bytes
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let bytes: &[u8; HEADER_BYTES] = bytes.try_into().map_err(|_| WireError::Invalid)?;
        if &bytes[..4] != b"APRV" || bytes[40..].iter().any(|byte| *byte != 0) {
            return Err(WireError::Invalid);
        }
        if u16::from_be_bytes([bytes[4], bytes[5]]) != VERSION {
            return Err(WireError::Version);
        }
        let lane = match bytes[6] {
            1 => Lane::Metadata,
            2 => Lane::Pixels,
            _ => return Err(WireError::Invalid),
        };
        let direction = match bytes[7] {
            1 => Direction::HostToWorker,
            2 => Direction::WorkerToHost,
            _ => return Err(WireError::Direction),
        };
        let u64_at = |offset| u64::from_be_bytes(std::array::from_fn(|index| bytes[offset + index]));
        let u32_at = |offset| u32::from_be_bytes(std::array::from_fn(|index| bytes[offset + index]));
        Self::new(
            lane,
            direction,
            u64_at(8),
            u64_at(32),
            u64_at(16),
            u32_at(24) as usize,
            u32_at(28) as usize,
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case", deny_unknown_fields)]
pub enum Metadata {
    Policy {
        connection: u64,
        policy: PreviewPolicy,
    },
    Invalidate {
        source: PreviewSource,
        connection: u64,
    },
    Demand {
        source: PreviewSource,
        connection: u64,
        demand: u64,
        size: PreviewSize,
        rate: PreviewRate,
    },
    Capture(CaptureClaim),
    Layout {
        connection: u64,
        layout: PreviewLayout,
    },
    CaptureStopped {
        source: PreviewSource,
        connection: u64,
        pixel_epoch: u64,
        stop: u64,
        policy: PreviewPolicy,
    },
    CaptureStopAck {
        source: PreviewSource,
        connection: u64,
        pixel_epoch: u64,
        stop: u64,
    },
    Stop {
        source: PreviewSource,
        connection: u64,
        demand: u64,
    },
    Stopped {
        source: PreviewSource,
        connection: u64,
        demand: u64,
    },
    Unavailable {
        source: PreviewSource,
        connection: u64,
        request: u64,
        reason: Unavailable,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unavailable {
    Denied,
    Unsupported,
    Busy,
    Expired,
}

impl std::fmt::Debug for Metadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreviewMetadata([redacted])")
    }
}

impl Metadata {
    pub fn source(self) -> PreviewSource {
        match self {
            Self::Policy { policy, .. } => policy.source(),
            Self::Capture(claim) => claim.source(),
            Self::Layout { layout, .. } => layout.source(),
            Self::Invalidate { source, .. }
            | Self::Demand { source, .. }
            | Self::Stop { source, .. }
            | Self::Stopped { source, .. }
            | Self::Unavailable { source, .. } => source,
            Self::CaptureStopped { source, .. } | Self::CaptureStopAck { source, .. } => source,
        }
    }
    pub fn connection(self) -> u64 {
        match self {
            Self::Policy { connection, .. }
            | Self::Invalidate { connection, .. }
            | Self::Demand { connection, .. }
            | Self::Stop { connection, .. }
            | Self::Stopped { connection, .. }
            | Self::Unavailable { connection, .. } => connection,
            Self::Layout { connection, .. } | Self::CaptureStopped { connection, .. } | Self::CaptureStopAck { connection, .. } => {
                connection
            }
            Self::Capture(claim) => claim.connection(),
        }
    }
    pub fn direction(self) -> Direction {
        match self {
            Self::Demand { .. } | Self::Capture(_) | Self::Stop { .. } | Self::CaptureStopAck { .. } => Direction::HostToWorker,
            Self::Layout { .. } | Self::CaptureStopped { .. } => Direction::WorkerToHost,
            Self::Policy { .. } | Self::Invalidate { .. } | Self::Stopped { .. } | Self::Unavailable { .. } => Direction::WorkerToHost,
        }
    }
    pub fn validate(self) -> Result<(), WireError> {
        if self.connection() == 0 {
            return Err(WireError::Sequence);
        }
        match self {
            Self::CaptureStopped {
                source,
                pixel_epoch,
                stop,
                policy,
                ..
            } => {
                if pixel_epoch == 0 || stop == 0 {
                    return Err(WireError::Sequence);
                }
                if policy.source() != source || policy.access().capture_active {
                    return Err(WireError::Binding);
                }
                Ok(())
            }
            Self::CaptureStopAck { pixel_epoch: 0, .. } | Self::CaptureStopAck { stop: 0, .. } => Err(WireError::Sequence),
            Self::Demand { demand: 0, .. }
            | Self::Stop { demand: 0, .. }
            | Self::Stopped { demand: 0, .. }
            | Self::Unavailable { request: 0, .. } => Err(WireError::Sequence),
            _ => Ok(()),
        }
    }
}

/// Fixed storage prevents serializers and queued metadata from growing a Vec.
/// Debug intentionally omits its encoded bytes and any source identifiers.
#[derive(Clone)]
pub struct MetadataBytes {
    bytes: [u8; MAX_METADATA_BYTES],
    len: usize,
}
impl MetadataBytes {
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}
impl std::fmt::Debug for MetadataBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreviewMetadataBytes([redacted])")
    }
}
impl Write for MetadataBytes {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let end = self
            .len
            .checked_add(input.len())
            .filter(|end| *end <= MAX_METADATA_BYTES)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "preview metadata exceeds limit"))?;
        self.bytes[self.len..end].copy_from_slice(input);
        self.len = end;
        Ok(input.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode_metadata_value(value: &impl Serialize) -> Result<MetadataBytes, WireError> {
    let mut bytes = MetadataBytes {
        bytes: [0; MAX_METADATA_BYTES],
        len: 0,
    };
    serde_json::to_writer(&mut bytes, value).map_err(|_| WireError::Oversized)?;
    Ok(bytes)
}
fn decode_metadata_value<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, WireError> {
    if bytes.is_empty() || bytes.len() > MAX_METADATA_BYTES {
        return Err(WireError::Oversized);
    }
    serde_json::from_slice(bytes).map_err(|_| WireError::Invalid)
}
pub fn encode_metadata(metadata: Metadata) -> Result<MetadataBytes, WireError> {
    metadata.validate()?;
    encode_metadata_value(&metadata)
}
pub fn decode_metadata(header: Header, bytes: &[u8]) -> Result<Metadata, WireError> {
    if header.lane != Lane::Metadata || header.metadata_bytes() != bytes.len() {
        return Err(WireError::Invalid);
    }
    let metadata: Metadata = decode_metadata_value(bytes)?;
    metadata.validate()?;
    if metadata.connection() != header.connection {
        return Err(WireError::Binding);
    }
    if metadata.direction() != header.direction {
        return Err(WireError::Direction);
    }
    Ok(metadata)
}
pub fn encode_stamp(stamp: PreviewStamp) -> Result<MetadataBytes, WireError> {
    encode_metadata_value(&stamp)
}
pub fn decode_stamp(header: Header, bytes: &[u8]) -> Result<PreviewStamp, WireError> {
    if header.lane != Lane::Pixels || header.metadata_bytes() != bytes.len() {
        return Err(WireError::Invalid);
    }
    let stamp: PreviewStamp = decode_metadata_value(bytes)?;
    if stamp.claim().connection() != header.connection {
        return Err(WireError::Binding);
    }
    if stamp.geometry().payload_bytes() != header.pixel_bytes() {
        return Err(WireError::Invalid);
    }
    Ok(stamp)
}

#[cfg(test)]
mod tests;
