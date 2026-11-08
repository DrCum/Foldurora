//! Closed V2 control DATA and correlation, not native authority. Production IO
//! still selects V1 until the actual stamped FIFO, reply owners and native
//! consumer join is reviewed. There is no version-guessing/fallback decoder.
//!
//! Ingress acknowledgment releases only a command-file slot after real bounded
//! ownership transfer. Reply acknowledgment retires only an immutable published
//! reply. Neither is an event for WorkerGuard/Coordinator or renews their lease.
use crate::{
    Binding, Command, Envelope, Error, Event, FrameProof, GpuProgress, HostNotice, HostedViewport, Message, PROTOCOL_VERSION,
    PrepareBoundary, RejectReason, Reply, WorkerEffects, WorkerIdentity, WorkerStatus,
    show_viewer::{ShowViewerLookup, ShowViewerOutcome, ShowViewerReceipt, ShowViewerRequest, ShowViewerTarget},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const CONTROL_V2_VERSION: u16 = 2;
pub const MAX_CONTROL_V2_BYTES: usize = 4096;
pub const MAX_CONTROL_ACK_BYTES: usize = 1024;
pub const SHOW_VIEWER_MILLIS: u64 = 2000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlV2Command {
    Heartbeat { workspace_foreground: bool },
    Revoke { activation: u64 },
    Prepare { activation: u64, viewport: Option<HostedViewport> },
    Grant { activation: u64 },
    SetViewport { activation: u64, viewport: Option<HostedViewport> },
    Detach,
    ShowViewer(ShowViewerRequest),
    QueryShowViewer(ShowViewerRequest),
}
impl ControlV2Command {
    /// Typed adaptation to the existing control state machine, never wire
    /// fallback. Show/query deliberately cannot become an existing command.
    pub(crate) fn control(&self) -> Option<Command> {
        Some(match self {
            Self::Heartbeat { workspace_foreground } => Command::Heartbeat {
                workspace_foreground: *workspace_foreground,
            },
            Self::Revoke { activation } => Command::Revoke { activation: *activation },
            Self::Prepare { activation, viewport } => Command::Prepare {
                activation: *activation,
                viewport: *viewport,
            },
            Self::Grant { activation } => Command::Grant { activation: *activation },
            Self::SetViewport { activation, viewport } => Command::SetViewport {
                activation: *activation,
                viewport: *viewport,
            },
            Self::Detach => Command::Detach,
            Self::ShowViewer(_) | Self::QueryShowViewer(_) => return None,
        })
    }
    pub(crate) fn from_control(command: Command) -> Self {
        match command {
            Command::Heartbeat { workspace_foreground } => Self::Heartbeat { workspace_foreground },
            Command::Revoke { activation } => Self::Revoke { activation },
            Command::Prepare { activation, viewport } => Self::Prepare { activation, viewport },
            Command::Grant { activation } => Self::Grant { activation },
            Command::SetViewport { activation, viewport } => Self::SetViewport { activation, viewport },
            Command::Detach => Self::Detach,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "receipt", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)] // Closed immutable receipt, bounded16 at its actual owner.
pub enum ShowViewerQueryState {
    Unknown,
    Pending,
    Terminal(ShowViewerReceipt),
    Rejected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowViewerQueryResult {
    pub request: ShowViewerRequest,
    pub result: ShowViewerQueryState,
}
impl ShowViewerQueryResult {
    pub fn validate(&self) -> Result<(), Error> {
        self.request.validate()?;
        if let ShowViewerQueryState::Terminal(receipt) = self.result
            && !receipt.matches(self.request)
        {
            return Err(Error::Binding);
        }
        Ok(())
    }
    pub fn from_lookup(request: ShowViewerRequest, lookup: ShowViewerLookup) -> Result<Self, Error> {
        let result = Self {
            request,
            result: match lookup {
                ShowViewerLookup::Unknown => ShowViewerQueryState::Unknown,
                ShowViewerLookup::Pending => ShowViewerQueryState::Pending,
                ShowViewerLookup::Terminal(receipt) => ShowViewerQueryState::Terminal(receipt),
            },
        };
        result.validate()?;
        Ok(result)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)] // Bounded body-free DTOs stay inline, like existing control events.
pub enum ControlV2Event {
    Status {
        status: WorkerStatus,
        ordinary: Option<ShowViewerTarget>,
    },
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
    ShowViewerReceipt(ShowViewerReceipt),
    ShowViewerQuery(ShowViewerQueryResult),
}
impl ControlV2Event {
    pub(crate) fn control(&self) -> Option<Event> {
        Some(match self {
            Self::Status { status, .. } => Event::Status(status.clone()),
            Self::Quiesced { activation, progress } => Event::Quiesced {
                activation: *activation,
                progress: *progress,
            },
            Self::FrameReady {
                activation,
                boundary,
                frame,
            } => Event::FrameReady {
                activation: *activation,
                boundary: *boundary,
                frame: *frame,
            },
            Self::Granted { activation } => Event::Granted { activation: *activation },
            Self::Detached => Event::Detached,
            Self::Rejected { activation, reason } => Event::Rejected {
                activation: *activation,
                reason: *reason,
            },
            Self::ShowViewerReceipt(_) | Self::ShowViewerQuery(_) => return None,
        })
    }
    pub(crate) fn from_control(event: Event) -> Self {
        match event {
            Event::Status(status) => Self::Status { status, ordinary: None },
            Event::Quiesced { activation, progress } => Self::Quiesced { activation, progress },
            Event::FrameReady {
                activation,
                boundary,
                frame,
            } => Self::FrameReady {
                activation,
                boundary,
                frame,
            },
            Event::Granted { activation } => Self::Granted { activation },
            Event::Detached => Self::Detached,
            Event::Rejected { activation, reason } => Self::Rejected { activation, reason },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "direction", content = "body", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)]
pub enum ControlV2Message {
    Command(ControlV2Command),
    Event(ControlV2Event),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlV2Envelope {
    pub version: u16,
    pub binding: Binding,
    pub lease_epoch: u64,
    pub sequence: u64,
    pub reply_to: u64,
    pub message: ControlV2Message,
}
impl ControlV2Envelope {
    pub fn validate(&self) -> Result<(), Error> {
        if self.version != CONTROL_V2_VERSION {
            return Err(Error::Version);
        }
        self.binding.validate()?;
        if self.lease_epoch == 0 || self.sequence == 0 {
            return Err(Error::Sequence);
        }
        let target = match &self.message {
            ControlV2Message::Command(command) => {
                if self.reply_to != 0 {
                    return Err(Error::Sequence);
                }
                if let Some(command) = command.control() {
                    return self.control_envelope(Message::Command(command)).validate();
                }
                match command {
                    ControlV2Command::ShowViewer(request) | ControlV2Command::QueryShowViewer(request) => {
                        request.validate()?;
                        Some(request.target)
                    }
                    _ => return Err(Error::Invalid),
                }
            }
            ControlV2Message::Event(event) => {
                if let Some(control) = event.control() {
                    self.control_envelope(Message::Event(control)).validate()?;
                }
                match event {
                    ControlV2Event::Status { ordinary, .. } => *ordinary,
                    ControlV2Event::ShowViewerReceipt(receipt) => {
                        if self.reply_to == 0 {
                            return Err(Error::Sequence);
                        }
                        receipt.validate()?;
                        Some(receipt.request.target)
                    }
                    ControlV2Event::ShowViewerQuery(result) => {
                        if self.reply_to == 0 {
                            return Err(Error::Sequence);
                        }
                        result.validate()?;
                        Some(result.request.target)
                    }
                    _ => None,
                }
            }
        };
        if let Some(target) = target {
            target.validate()?;
            if target.binding != self.binding || target.lease_epoch != self.lease_epoch {
                return Err(Error::Binding);
            }
        }
        Ok(())
    }
    /// Reuses existing typed control validation/state only. No V1 bytes are
    /// decoded here, and Show/receipt/ACK cannot use this internal adapter.
    pub(crate) fn control_envelope(&self, message: Message) -> Envelope {
        Envelope {
            version: PROTOCOL_VERSION,
            binding: self.binding,
            lease_epoch: self.lease_epoch,
            sequence: self.sequence,
            reply_to: self.reply_to,
            message,
        }
    }
    pub(crate) fn from_control(envelope: Envelope) -> Self {
        Self {
            version: CONTROL_V2_VERSION,
            binding: envelope.binding,
            lease_epoch: envelope.lease_epoch,
            sequence: envelope.sequence,
            reply_to: envelope.reply_to,
            message: match envelope.message {
                Message::Command(command) => ControlV2Message::Command(ControlV2Command::from_control(command)),
                Message::Event(event) => ControlV2Message::Event(ControlV2Event::from_control(event)),
            },
        }
    }
}

fn encode_bounded<T: Serialize>(message: &T, limit: usize) -> Result<Vec<u8>, Error> {
    let bytes = serde_json::to_vec(message).map_err(|_| Error::Malformed)?;
    if bytes.len() > limit { Err(Error::Oversized) } else { Ok(bytes) }
}
fn decode_bounded<T: DeserializeOwned>(bytes: &[u8], limit: usize) -> Result<T, Error> {
    if bytes.len() > limit {
        return Err(Error::Oversized);
    }
    serde_json::from_slice(bytes).map_err(|_| Error::Malformed)
}
pub fn encode_control(message: &ControlV2Envelope) -> Result<Vec<u8>, Error> {
    message.validate()?;
    encode_bounded(message, MAX_CONTROL_V2_BYTES)
}
pub fn decode_control(bytes: &[u8]) -> Result<ControlV2Envelope, Error> {
    let message: ControlV2Envelope = decode_bounded(bytes, MAX_CONTROL_V2_BYTES)?;
    message.validate()?;
    Ok(message)
}

/// Exact terminal/query correlation, never ingress acknowledgment or a native
/// success decision. In particular a Show receipt cannot acknowledge Grant.
pub fn matches_reply(command: &ControlV2Envelope, reply: &ControlV2Envelope) -> bool {
    if command.validate().is_err()
        || reply.validate().is_err()
        || command.binding.worker != reply.binding.worker
        || command.lease_epoch != reply.lease_epoch
        || command.sequence != reply.reply_to
    {
        return false;
    }
    let (ControlV2Message::Command(command_body), ControlV2Message::Event(event)) = (&command.message, &reply.message) else {
        return false;
    };
    match (command_body, event) {
        (ControlV2Command::ShowViewer(request), ControlV2Event::ShowViewerReceipt(receipt)) => {
            command.binding == reply.binding && receipt.matches(*request)
        }
        (ControlV2Command::QueryShowViewer(request), ControlV2Event::ShowViewerQuery(result)) => {
            command.binding == reply.binding && result.request == *request
        }
        _ => {
            let (Some(control), Some(event)) = (command_body.control(), event.control()) else {
                return false;
            };
            if control.session_bound() && reply.binding != command.binding {
                return false;
            }
            matches_normal_reply(&control, &event)
        }
    }
}

fn matches_normal_reply(command: &Command, event: &Event) -> bool {
    if matches!(event, Event::Rejected { .. }) {
        return command.activation() == event.activation();
    }
    match (command, event) {
        (Command::Heartbeat { .. } | Command::SetViewport { .. }, Event::Status(_)) | (Command::Detach, Event::Detached) => true,
        (Command::Revoke { activation }, Event::Quiesced { activation: observed, .. })
        | (Command::Prepare { activation, .. }, Event::FrameReady { activation: observed, .. })
        | (Command::Grant { activation }, Event::Granted { activation: observed }) => activation == observed,
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyAckV2 {
    pub binding: Binding,
    pub lease_epoch: u64,
    pub event_sequence: u64,
    pub reply_to: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlV2Ack {
    pub version: u16,
    pub reply: ReplyAckV2,
}
impl ControlV2Ack {
    pub fn validate(&self) -> Result<(), Error> {
        if self.version != CONTROL_V2_VERSION {
            return Err(Error::Version);
        }
        self.reply.binding.validate()?;
        if self.reply.lease_epoch == 0 || self.reply.event_sequence == 0 || self.reply.reply_to == 0 {
            return Err(Error::Sequence);
        }
        Ok(())
    }
    /// Caller must supply ONLY its oldest successfully published immutable
    /// control reply, never an unpublished reservation or the latest status.
    pub fn matches_published(&self, published: &ControlV2Envelope, worker: WorkerIdentity, lease_epoch: u64) -> bool {
        self.validate().is_ok()
            && published.validate().is_ok()
            && matches!(published.message, ControlV2Message::Event(_))
            && published.reply_to != 0
            && self.reply.binding.worker == worker
            && self.reply.lease_epoch == lease_epoch
            && self.reply.binding == published.binding
            && self.reply.lease_epoch == published.lease_epoch
            && self.reply.event_sequence == published.sequence
            && self.reply.reply_to == published.reply_to
    }
}
pub fn encode_reply_ack(message: &ControlV2Ack) -> Result<Vec<u8>, Error> {
    message.validate()?;
    encode_bounded(message, MAX_CONTROL_ACK_BYTES)
}
pub fn decode_reply_ack(bytes: &[u8]) -> Result<ControlV2Ack, Error> {
    let message: ControlV2Ack = decode_bounded(bytes, MAX_CONTROL_ACK_BYTES)?;
    message.validate()?;
    Ok(message)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlIngressAckV2 {
    pub version: u16,
    pub binding: Binding,
    pub lease_epoch: u64,
    pub command_sequence: u64,
    /// SHA256 of the original immutable published bytes, computed by actual IO.
    pub command_digest: [u8; 32],
}
impl ControlIngressAckV2 {
    pub fn validate(&self) -> Result<(), Error> {
        if self.version != CONTROL_V2_VERSION {
            return Err(Error::Version);
        }
        self.binding.validate()?;
        if self.lease_epoch == 0 || self.command_sequence == 0 {
            return Err(Error::Sequence);
        }
        Ok(())
    }
    /// Transport metadata only. Caller retains the original byte digest; this
    /// module cannot prove first ingress/transfer and cannot grant an effect.
    pub fn matches_ingress(&self, pending: &ControlV2Envelope, digest: [u8; 32], worker: WorkerIdentity, lease_epoch: u64) -> bool {
        self.validate().is_ok()
            && pending.validate().is_ok()
            && matches!(pending.message, ControlV2Message::Command(_))
            && self.binding.worker == worker
            && self.lease_epoch == lease_epoch
            && self.binding == pending.binding
            && self.lease_epoch == pending.lease_epoch
            && self.command_sequence == pending.sequence
            && self.command_digest == digest
    }
}
pub fn encode_ingress_ack(message: &ControlIngressAckV2) -> Result<Vec<u8>, Error> {
    message.validate()?;
    encode_bounded(message, MAX_CONTROL_ACK_BYTES)
}
pub fn decode_ingress_ack(bytes: &[u8]) -> Result<ControlIngressAckV2, Error> {
    let message: ControlIngressAckV2 = decode_bounded(bytes, MAX_CONTROL_ACK_BYTES)?;
    message.validate()?;
    Ok(message)
}

/// Body-free work for the actual App consumer, NEVER a native permit. An
/// outcome already determined by expiry/state must be finished without effect;
/// None still requires every actual native/source/window/input check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlV2Work {
    ShowViewer {
        request: ShowViewerRequest,
        outcome: Option<ShowViewerOutcome>,
    },
    QueryShowViewer(ShowViewerRequest),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlV2Reply {
    pub reply_to: u64,
    pub event: ControlV2Event,
}
impl ControlV2Reply {
    /// Also adapts the real Guard's delayed quiesced/frame_ready completion.
    pub fn from_control(reply: Reply) -> Self {
        Self {
            reply_to: reply.reply_to,
            event: ControlV2Event::from_control(reply.event),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ControlV2Transition {
    pub effects: WorkerEffects,
    pub reply: Option<ControlV2Reply>,
    pub work: Option<ControlV2Work>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum ControlV2Effect {
    Send(ControlV2Envelope),
    Notice(HostNotice),
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ControlV2Observation {
    pub effects: Vec<ControlV2Effect>,
    pub show_receipt: Option<ShowViewerReceipt>,
    pub show_query: Option<ShowViewerQueryResult>,
}

#[cfg(test)]
pub(crate) mod tests;
