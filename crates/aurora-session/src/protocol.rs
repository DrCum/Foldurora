use crate::Binding;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_MESSAGE_BYTES: usize = 4096;
pub const MAX_WORKERS: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Identity,
    Oversized,
    Malformed,
    Version,
    Sequence,
    Binding,
    Invalid,
    Busy,
    Unavailable,
    Capacity,
    Stale,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session protocol: {self:?}")
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Label(String);
impl Label {
    pub fn new(text: &str) -> Result<Self, Error> {
        if text.len() > 128 || text.chars().any(char::is_control) {
            return Err(Error::Invalid);
        }
        Ok(Self(text.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl<'de> Deserialize<'de> for Label {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::new(&text).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl PhysicalRect {
    pub fn validate(&self) -> Result<(), Error> {
        if self.width == 0
            || self.height == 0
            || self.width > 32768
            || self.height > 32768
            || self.x.checked_add(self.width as i32).is_none()
            || self.y.checked_add(self.height as i32).is_none()
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedViewport {
    pub host_window: u64,
    pub rect: PhysicalRect,
}
impl HostedViewport {
    pub fn validate(&self) -> Result<(), Error> {
        if self.host_window == 0 {
            return Err(Error::Invalid);
        }
        self.rect.validate()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuProgress {
    pub device_epoch: u64,
    pub submitted: u64,
    pub completed: u64,
}
impl GpuProgress {
    pub fn validate(&self) -> Result<(), Error> {
        if self.device_epoch == 0 || self.completed > self.submitted {
            Err(Error::Invalid)
        } else {
            Ok(())
        }
    }
    pub fn quiescent(&self) -> bool {
        self.device_epoch != 0 && self.submitted == self.completed
    }
}

/// This is a completed GPU frame whose present was issued on this surface,
/// not merely a queued render packet or the CPU's `stats.presented` flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameProof {
    pub device_epoch: u64,
    pub surface_epoch: u64,
    pub frame: u64,
}
impl FrameProof {
    pub fn validate(&self) -> Result<(), Error> {
        if self.device_epoch == 0 || self.surface_epoch == 0 || self.frame == 0 {
            Err(Error::Invalid)
        } else {
            Ok(())
        }
    }
    pub fn newer_than(&self, previous: Option<Self>) -> bool {
        self.validate().is_ok()
            && previous.is_none_or(|old| {
                self.device_epoch > old.device_epoch
                    || (self.device_epoch == old.device_epoch
                        && (self.surface_epoch > old.surface_epoch || (self.surface_epoch == old.surface_epoch && self.frame > old.frame)))
            })
    }
}

/// Captured after the worker has applied the requested window/viewport, before
/// submitting its prepared frame. `after_frame` includes every prior queued
/// submission, even one whose GPU completion has not been observed yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareBoundary {
    pub device_epoch: u64,
    pub surface_epoch: u64,
    pub after_frame: u64,
}
impl PrepareBoundary {
    pub fn validate(&self) -> Result<(), Error> {
        if self.device_epoch == 0 || self.surface_epoch == 0 {
            Err(Error::Invalid)
        } else {
            Ok(())
        }
    }
    pub fn accepts(&self, frame: FrameProof) -> bool {
        self.validate().is_ok()
            && frame.validate().is_ok()
            && frame.device_epoch == self.device_epoch
            && frame.surface_epoch == self.surface_epoch
            && frame.frame > self.after_frame
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Login,
    Connecting,
    Online,
    Disconnected,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerMode {
    #[default]
    Standby,
    Revoking,
    Preparing,
    Active,
    Detached,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerStatus {
    pub phase: Phase,
    pub name: Label,
    pub grid_label: Label,
    pub native_window: u64,
    pub keyboard_focused: bool,
    pub blocked: bool,
    pub last_frame: Option<FrameProof>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    Blocked,
    NotReady,
    StaleFrame,
    DeviceLost,
    Unsupported,
    InvalidState,
    NativeWindow,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Heartbeat { workspace_foreground: bool },
    Revoke { activation: u64 },
    Prepare { activation: u64, viewport: Option<HostedViewport> },
    Grant { activation: u64 },
    SetViewport { activation: u64, viewport: Option<HostedViewport> },
    Detach,
}
impl Command {
    pub fn activation(&self) -> u64 {
        match self {
            Self::Revoke { activation }
            | Self::Prepare { activation, .. }
            | Self::Grant { activation }
            | Self::SetViewport { activation, .. } => *activation,
            _ => 0,
        }
    }
    pub fn session_bound(&self) -> bool {
        !matches!(self, Self::Heartbeat { .. } | Self::Detach)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Status(WorkerStatus),
    Quiesced {
        activation: u64,
        progress: GpuProgress,
    },
    FrameReady {
        activation: u64,
        boundary: PrepareBoundary,
        frame: FrameProof,
    },
    Granted {
        activation: u64,
    },
    Detached,
    Rejected {
        activation: u64,
        reason: RejectReason,
    },
}
impl Event {
    pub fn activation(&self) -> u64 {
        match self {
            Self::Quiesced { activation, .. }
            | Self::FrameReady { activation, .. }
            | Self::Granted { activation }
            | Self::Rejected { activation, .. } => *activation,
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "direction", content = "body", rename_all = "snake_case", deny_unknown_fields)]
pub enum Message {
    Command(Command),
    Event(Event),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub version: u16,
    pub binding: Binding,
    pub lease_epoch: u64,
    pub sequence: u64,
    /// Zero for commands / unsolicited status; otherwise the exact command.
    pub reply_to: u64,
    pub message: Message,
}
impl Envelope {
    pub fn validate(&self) -> Result<(), Error> {
        if self.version != PROTOCOL_VERSION {
            return Err(Error::Version);
        }
        self.binding.validate()?;
        if self.lease_epoch == 0 || self.sequence == 0 {
            return Err(Error::Sequence);
        }
        match &self.message {
            Message::Command(command) => {
                if self.reply_to != 0 {
                    return Err(Error::Sequence);
                }
                if command.session_bound() && (command.activation() == 0 || self.binding.session.is_none()) {
                    return Err(Error::Binding);
                }
                if let Command::Prepare {
                    viewport: Some(viewport), ..
                }
                | Command::SetViewport {
                    viewport: Some(viewport), ..
                } = command
                {
                    viewport.validate()?;
                }
            }
            Message::Event(event) => {
                if !matches!(event, Event::Status(_) | Event::Detached)
                    && (self.reply_to == 0 || event.activation() == 0 || self.binding.session.is_none())
                {
                    return Err(Error::Binding);
                }
                match event {
                    Event::Status(status) => {
                        if status.phase == Phase::Online && self.binding.session.is_none() {
                            return Err(Error::Binding);
                        }
                        if let Some(frame) = status.last_frame {
                            frame.validate()?;
                        }
                    }
                    Event::Quiesced { progress, .. } => progress.validate()?,
                    Event::FrameReady { boundary, frame, .. } => {
                        boundary.validate()?;
                        frame.validate()?;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

pub fn encode(message: &Envelope) -> Result<Vec<u8>, Error> {
    message.validate()?;
    let bytes = serde_json::to_vec(message).map_err(|_| Error::Malformed)?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        Err(Error::Oversized)
    } else {
        Ok(bytes)
    }
}
pub fn decode(bytes: &[u8]) -> Result<Envelope, Error> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(Error::Oversized);
    }
    let message: Envelope = serde_json::from_slice(bytes).map_err(|_| Error::Malformed)?;
    message.validate()?;
    Ok(message)
}
