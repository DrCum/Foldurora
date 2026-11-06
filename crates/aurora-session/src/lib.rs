//! Pure session-host protocol and ownership rules. No files, clocks, windows,
//! networking, credentials or GPU work are accessed by this crate.
//!
//! Each worker remains a separate viewer process. Following the behavior of
//! `FSSessionWorker` / `Handoff` in `indra/newview/fssessionworker.cpp` and
//! `fssessionprotocol.h`, promotion requires acknowledged input revocation,
//! completed GPU work, a newly presented target frame, then an explicit grant.
//! Aurora has its own bounded V1 protocol, with no legacy wire compatibility.

pub mod chat;
mod coordinator;
mod identity;
mod preview;
mod protocol;
mod worker;

pub use coordinator::{Coordinator, Effect, HostNotice, RosterEntry};
pub use identity::{
    AccountKey, Binding, ConversationId, GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerId, WorkerIdentity,
};
pub use preview::{PreviewDemand, PreviewRate, PreviewSize, admit_previews};
pub use protocol::{
    Command, Envelope, Error, Event, FrameProof, GpuProgress, HostedViewport, Label, MAX_MESSAGE_BYTES, MAX_WORKERS, Message,
    PROTOCOL_VERSION, Phase, PhysicalRect, PrepareBoundary, RejectReason, WorkerMode, WorkerStatus, decode, encode,
};
pub use worker::{LEASE_MILLIS, Reply, WorkerEffects, WorkerGuard, WorkerTransition};
