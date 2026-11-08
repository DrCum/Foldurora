use crate::{
    Binding, Command, Envelope, Error, Event, FrameProof, HostId, HostedViewport, InstallationId, LEASE_MILLIS, MAX_WORKERS, Message,
    PROTOCOL_VERSION, Phase, WorkerId, WorkerIdentity, WorkerStatus,
};
use std::collections::BTreeMap;

mod control_v2;

const STEP_MILLIS: u64 = 12_000;
const HEARTBEAT_MILLIS: u64 = 1000;
/// Fixed allowance for native/GPU initialization before the first valid
/// status. Outbound heartbeats do not renew it or authorize activation.
pub const STARTUP_MILLIS: u64 = 300_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterEntry {
    pub binding: Binding,
    pub status: WorkerStatus,
    pub live: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostNotice {
    Switched(WorkerId),
    RolledBack(WorkerId),
    Parked(WorkerId),
    Blocked(WorkerId),
    Expired(WorkerId),
    Detached,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
// At most a handful of effects exist per tick; keep small control messages
// inline instead of adding a separate allocation to every heartbeat.
#[allow(clippy::large_enum_variant)]
pub enum Effect {
    Send(Envelope),
    Notice(HostNotice),
}

struct Entry {
    roster: RosterEntry,
    lease_epoch: u64,
    sent: u64,
    received: u64,
    generation: u64,
    last_seen: u64,
    last_heartbeat: Option<u64>,
    ordinary: Option<crate::show_viewer::ShowViewerTarget>,
    shows: control_v2::ShowOperations,
}
impl Entry {
    fn next_sequence(&self) -> Result<u64, Error> {
        self.sent.checked_add(1).ok_or(Error::Sequence)
    }
    fn receipt_timeout(&self) -> u64 {
        if self.received == 0 { STARTUP_MILLIS } else { LEASE_MILLIS }
    }
}

#[derive(Clone, Copy)]
struct Active {
    binding: Binding,
    viewport: Option<HostedViewport>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    ParkOld,
    RevokeOld,
    PrepareTarget,
    GrantTarget,
    RevokeTarget,
    PrepareOld,
    GrantOld,
}

#[derive(Clone, Copy)]
struct Pending {
    old: Option<Active>,
    target: Binding,
    viewport: Option<HostedViewport>,
    activation: u64,
    stage: Stage,
    sequence: u64,
    deadline: u64,
    baseline: Option<FrameProof>,
}
impl Pending {
    fn receiver(self) -> Binding {
        match self.stage {
            Stage::ParkOld | Stage::RevokeOld | Stage::PrepareOld | Stage::GrantOld => self.old.map_or(self.target, |old| old.binding),
            _ => self.target,
        }
    }
    fn involves(self, worker: WorkerId) -> bool {
        self.target.worker.worker == worker || self.old.is_some_and(|old| old.binding.worker.worker == worker)
    }
}

/// The caller supplies milliseconds from its own monotonic clock. Wire data
/// contains no trusted wall clock. At most five workers and one transaction
/// exist; effects are bounded messages, never background jobs or an IO queue.
pub struct Coordinator {
    installation: InstallationId,
    host: HostId,
    entries: BTreeMap<WorkerId, Entry>,
    active: Option<Active>,
    pending: Option<Pending>,
    activation: u64,
    now: u64,
    foreground: bool,
}

impl Coordinator {
    pub fn new(installation: InstallationId, host: HostId) -> Self {
        Self {
            installation,
            host,
            entries: BTreeMap::new(),
            active: None,
            pending: None,
            activation: 0,
            now: 0,
            foreground: false,
        }
    }
    pub fn workers(&self) -> impl Iterator<Item = &RosterEntry> {
        self.entries.values().map(|entry| &entry.roster)
    }
    pub fn active(&self) -> Option<WorkerId> {
        self.active.map(|active| active.binding.worker.worker)
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn set_workspace_foreground(&mut self, foreground: bool) {
        self.foreground = foreground;
    }

    pub fn register(&mut self, worker: WorkerId, lease_epoch: u64, now_ms: u64) -> Result<(), Error> {
        if lease_epoch == 0 {
            return Err(Error::Sequence);
        }
        if self.entries.contains_key(&worker) {
            return Err(Error::Identity);
        }
        if self.entries.len() >= MAX_WORKERS {
            return Err(Error::Capacity);
        }
        self.now = self.now.max(now_ms);
        let binding = Binding {
            worker: WorkerIdentity {
                installation: self.installation,
                host: self.host,
                worker,
            },
            session: None,
        };
        self.entries.insert(
            worker,
            Entry {
                roster: RosterEntry {
                    binding,
                    status: WorkerStatus::default(),
                    live: true,
                },
                lease_epoch,
                sent: 0,
                received: 0,
                generation: 0,
                last_seen: self.now,
                last_heartbeat: None,
                ordinary: None,
                shows: control_v2::ShowOperations::default(),
            },
        );
        Ok(())
    }

    /// Only remove a process after the transport has observed its exit. Other
    /// managed peers detach if ownership of this worker was still unresolved.
    pub fn remove(&mut self, worker: WorkerId) -> Vec<Effect> {
        let effects = if self.active() == Some(worker) || self.pending.is_some_and(|p| p.involves(worker)) {
            self.detach_all()
        } else {
            Vec::new()
        };
        self.entries.remove(&worker);
        effects
    }

    fn send(&mut self, worker: WorkerId, command: Command) -> Result<Envelope, Error> {
        let entry = self.entries.get_mut(&worker).ok_or(Error::Unavailable)?;
        let sequence = entry.next_sequence()?;
        let mut binding = entry.roster.binding;
        if !command.session_bound() {
            binding.session = None;
        }
        let envelope = Envelope {
            version: PROTOCOL_VERSION,
            binding,
            lease_epoch: entry.lease_epoch,
            sequence,
            reply_to: 0,
            message: Message::Command(command),
        };
        envelope.validate()?;
        if !matches!(envelope.message, Message::Command(Command::Heartbeat { .. })) {
            entry.ordinary = None;
        }
        entry.sent = sequence;
        Ok(envelope)
    }

    fn ready(&self, binding: Binding) -> bool {
        self.entries.get(&binding.worker.worker).is_some_and(|entry| {
            entry.roster.live
                && entry.roster.binding == binding
                && self.now.saturating_sub(entry.last_seen) < LEASE_MILLIS
                && binding.session.is_some()
                && entry.roster.status.phase == Phase::Online
                && !entry.roster.status.blocked
        })
    }

    fn next_activation(&mut self) -> Result<u64, Error> {
        self.activation = self.activation.checked_add(1).ok_or(Error::Sequence)?;
        Ok(self.activation)
    }

    pub fn activate(&mut self, worker: WorkerId, viewport: Option<HostedViewport>, now_ms: u64) -> Result<Vec<Effect>, Error> {
        if let Some(viewport) = viewport {
            viewport.validate()?;
        }
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        self.now = self.now.max(now_ms);
        let target = self.entries.get(&worker).ok_or(Error::Unavailable)?.roster.binding;
        if !self.ready(target) {
            return Err(Error::Unavailable);
        }
        if self
            .active
            .is_some_and(|active| active.binding == target && active.viewport == viewport)
        {
            return Ok(Vec::new());
        }
        let activation = self.next_activation()?;
        let stage = if self.active.is_some() {
            Stage::RevokeOld
        } else {
            Stage::PrepareTarget
        };
        let mut pending = Pending {
            old: self.active,
            target,
            viewport,
            activation,
            stage,
            sequence: 0,
            deadline: self.now.saturating_add(STEP_MILLIS),
            baseline: None,
        };
        let message = self.step(&mut pending)?;
        self.pending = Some(pending);
        Ok(vec![Effect::Send(message)])
    }

    /// Native host movement invalidates placement even if it ends at the
    /// original rectangle. Revoke and earn a new GPU proof instead of taking
    /// activate's ordinary same-viewport shortcut. Repeated geometry changes
    /// may supersede only a handoff involving this one binding, never a switch
    /// between different characters or an explicit park/rollback.
    pub fn refresh_viewport(&mut self, worker: WorkerId, viewport: HostedViewport, now_ms: u64) -> Result<Vec<Effect>, Error> {
        viewport.validate()?;
        self.now = self.now.max(now_ms);
        let target = self.entries.get(&worker).ok_or(Error::Unavailable)?.roster.binding;
        let old = if let Some(pending) = self.pending {
            if pending.target != target
                || pending.old.is_some_and(|old| old.binding != target)
                || !matches!(pending.stage, Stage::RevokeOld | Stage::PrepareTarget | Stage::GrantTarget)
            {
                return Err(Error::Busy);
            }
            Active {
                binding: target,
                viewport: pending.viewport,
            }
        } else {
            self.active.filter(|active| active.binding == target).ok_or(Error::Unavailable)?
        };
        if !self.ready(target) {
            // A native modal/permission gap cannot leave a hidden Active
            // worker renewed by heartbeats while a UI error stops refreshes.
            return Ok(self.detach_all());
        }
        let activation = self.next_activation()?;
        let mut pending = Pending {
            old: Some(old),
            target,
            viewport: Some(viewport),
            activation,
            stage: Stage::RevokeOld,
            sequence: 0,
            deadline: self.now.saturating_add(STEP_MILLIS),
            baseline: None,
        };
        let message = self.step(&mut pending)?;
        self.pending = Some(pending);
        Ok(vec![Effect::Send(message)])
    }

    /// Park before launching another login/renderer. Only a fresh GPU-idle
    /// acknowledgment emits Parked; no timeout or refusal authorizes a launch.
    pub fn park_active(&mut self, now_ms: u64) -> Result<Vec<Effect>, Error> {
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        self.now = self.now.max(now_ms);
        let Some(old) = self.active else { return Ok(Vec::new()) };
        let activation = self.next_activation()?;
        let mut pending = Pending {
            old: Some(old),
            target: old.binding,
            viewport: old.viewport,
            activation,
            stage: Stage::ParkOld,
            sequence: 0,
            deadline: self.now.saturating_add(STEP_MILLIS),
            baseline: None,
        };
        let message = self.step(&mut pending)?;
        self.pending = Some(pending);
        Ok(vec![Effect::Send(message)])
    }

    /// A standby process can detach independently. Detaching an active or
    /// transaction participant returns all peers to ordinary windows rather
    /// than guessing which outstanding grant may already have been applied.
    pub fn detach(&mut self, worker: WorkerId) -> Result<Vec<Effect>, Error> {
        let entry = self.entries.get(&worker).ok_or(Error::Unavailable)?;
        if !entry.roster.live {
            return Ok(Vec::new());
        }
        if self.active() == Some(worker) || self.pending.is_some_and(|p| p.involves(worker)) {
            return Ok(self.detach_all());
        }
        let message = self.send(worker, Command::Detach)?;
        if let Some(entry) = self.entries.get_mut(&worker) {
            entry.roster.live = false;
        }
        Ok(vec![Effect::Send(message)])
    }

    fn step(&mut self, pending: &mut Pending) -> Result<Envelope, Error> {
        let receiver = pending.receiver();
        if !self.ready(receiver) {
            return Err(Error::Unavailable);
        }
        let command = match pending.stage {
            Stage::ParkOld | Stage::RevokeOld | Stage::RevokeTarget => Command::Revoke {
                activation: pending.activation,
            },
            Stage::PrepareTarget | Stage::PrepareOld => {
                pending.baseline = self
                    .entries
                    .get(&receiver.worker.worker)
                    .and_then(|entry| entry.roster.status.last_frame);
                let viewport = if pending.stage == Stage::PrepareOld {
                    pending.old.and_then(|old| old.viewport)
                } else {
                    pending.viewport
                };
                Command::Prepare {
                    activation: pending.activation,
                    viewport,
                }
            }
            Stage::GrantTarget | Stage::GrantOld => Command::Grant {
                activation: pending.activation,
            },
        };
        let message = self.send(receiver.worker.worker, command)?;
        pending.sequence = message.sequence;
        pending.deadline = self.now.saturating_add(STEP_MILLIS);
        Ok(message)
    }

    pub fn observe(&mut self, envelope: &Envelope, now_ms: u64) -> Result<Vec<Effect>, Error> {
        envelope.validate()?;
        let Message::Event(event) = &envelope.message else {
            return Err(Error::Invalid);
        };
        self.now = self.now.max(now_ms);
        let worker = envelope.binding.worker.worker;
        let entry = self.entries.get(&worker).ok_or(Error::Unavailable)?;
        if envelope.binding.worker != entry.roster.binding.worker || envelope.lease_epoch != entry.lease_epoch {
            return Err(Error::Binding);
        }
        if !entry.roster.live || envelope.sequence <= entry.received {
            return Err(Error::Stale);
        }
        if self.now.saturating_sub(entry.last_seen) >= entry.receipt_timeout() {
            return Err(Error::Stale);
        }
        if envelope.reply_to > entry.sent {
            return Err(Error::Sequence);
        }
        if !matches!(event, Event::Status(_) | Event::Detached) && envelope.binding != entry.roster.binding {
            return Err(Error::Binding);
        }
        let session_changed = envelope.binding.session != entry.roster.binding.session;
        if session_changed
            && envelope
                .binding
                .session
                .is_some_and(|session| session.generation <= entry.generation)
        {
            return Err(Error::Stale);
        }
        if let Event::Status(status) = event
            && !session_changed
            && let (Some(old), Some(new)) = (entry.roster.status.last_frame, status.last_frame)
            && new != old
            && !new.newer_than(Some(old))
        {
            return Err(Error::Stale);
        }
        if !matches!(event, Event::Status(_) | Event::Detached) {
            let pending = self.pending.ok_or(Error::Stale)?;
            if pending.receiver() != envelope.binding || pending.sequence != envelope.reply_to || pending.activation != event.activation() {
                return Err(Error::Stale);
            }
            let valid = match (pending.stage, event) {
                (_, Event::Rejected { .. }) => true,
                (Stage::ParkOld | Stage::RevokeOld | Stage::RevokeTarget, Event::Quiesced { progress, .. }) => {
                    progress.quiescent()
                        && entry.roster.status.last_frame.is_none_or(|last| {
                            progress.device_epoch > last.device_epoch
                                || (progress.device_epoch == last.device_epoch && progress.completed >= last.frame)
                        })
                }
                (Stage::PrepareTarget | Stage::PrepareOld, Event::FrameReady { boundary, frame, .. }) => {
                    boundary.accepts(*frame) && frame.newer_than(pending.baseline)
                }
                (Stage::GrantTarget | Stage::GrantOld, Event::Granted { .. }) => true,
                _ => false,
            };
            if !valid {
                return Err(Error::Stale);
            }
            if self.now >= pending.deadline {
                return Ok(self.failed_step());
            }
        }
        let was_owned = self.active() == Some(worker) || self.pending.is_some_and(|p| p.involves(worker));
        let entry = self.entries.get_mut(&worker).ok_or(Error::Unavailable)?;
        entry.received = envelope.sequence;
        entry.last_seen = self.now;
        if let Event::Status(status) = event {
            // V1 has no ordinary observation. The V2 typed entry restores
            // only the exact observation in that same validated Status.
            entry.ordinary = None;
            let mut status = status.clone();
            if !session_changed && status.last_frame.is_none() {
                status.last_frame = entry.roster.status.last_frame;
            }
            entry.roster.binding = envelope.binding;
            if let Some(session) = envelope.binding.session {
                entry.generation = session.generation;
            }
            entry.roster.status = status.clone();
            if was_owned && (session_changed || status.phase != Phase::Online) {
                return Ok(self.fail_closed());
            }
            return Ok(Vec::new());
        }
        if matches!(event, Event::Detached) {
            entry.roster.live = false;
            return Ok(if was_owned { self.fail_closed() } else { Vec::new() });
        }
        if let Event::FrameReady { frame, .. } = event {
            entry.roster.status.last_frame = Some(*frame);
        }
        let Some(mut pending) = self.pending else {
            return Err(Error::Stale);
        };
        if pending.receiver() != envelope.binding || pending.sequence != envelope.reply_to || pending.activation != event.activation() {
            return Err(Error::Stale);
        }
        if self.now >= pending.deadline {
            return Ok(self.failed_step());
        }
        if matches!(event, Event::Rejected { .. }) {
            return Ok(self.failed_step());
        }
        pending.stage = match (pending.stage, event) {
            (Stage::ParkOld, Event::Quiesced { progress, .. }) if progress.quiescent() => {
                self.active = None;
                self.pending = None;
                return Ok(vec![Effect::Notice(HostNotice::Parked(worker))]);
            }
            (Stage::RevokeOld, Event::Quiesced { progress, .. }) if progress.quiescent() => {
                self.active = None;
                Stage::PrepareTarget
            }
            (Stage::RevokeTarget, Event::Quiesced { progress, .. }) if progress.quiescent() => Stage::PrepareOld,
            (Stage::PrepareTarget, Event::FrameReady { boundary, frame, .. })
                if boundary.accepts(*frame) && frame.newer_than(pending.baseline) =>
            {
                Stage::GrantTarget
            }
            (Stage::PrepareOld, Event::FrameReady { boundary, frame, .. })
                if boundary.accepts(*frame) && frame.newer_than(pending.baseline) =>
            {
                Stage::GrantOld
            }
            (stage @ (Stage::GrantTarget | Stage::GrantOld), Event::Granted { .. }) => {
                let rollback = stage == Stage::GrantOld;
                let binding = pending.receiver();
                let viewport = if rollback {
                    pending.old.and_then(|old| old.viewport)
                } else {
                    pending.viewport
                };
                self.active = Some(Active { binding, viewport });
                self.pending = None;
                return Ok(vec![Effect::Notice(if rollback {
                    HostNotice::RolledBack(worker)
                } else {
                    HostNotice::Switched(worker)
                })]);
            }
            _ => return Err(Error::Stale),
        };
        match self.step(&mut pending) {
            Ok(message) => {
                self.pending = Some(pending);
                Ok(vec![Effect::Send(message)])
            }
            Err(_) => Ok(self.fail_closed()),
        }
    }

    fn failed_step(&mut self) -> Vec<Effect> {
        let Some(mut pending) = self.pending else { return Vec::new() };
        if !matches!(pending.stage, Stage::PrepareTarget | Stage::GrantTarget) || !pending.old.is_some_and(|old| self.ready(old.binding)) {
            return self.fail_closed();
        }
        let Ok(activation) = self.next_activation() else {
            return self.fail_closed();
        };
        // A delayed Grant may already have reached the failed target. Revoke
        // and fence it before preparing the old viewer; never guess ownership.
        pending.activation = activation;
        pending.stage = Stage::RevokeTarget;
        match self.step(&mut pending) {
            Ok(message) => {
                self.pending = Some(pending);
                vec![Effect::Send(message)]
            }
            Err(_) => self.fail_closed(),
        }
    }

    fn fail_closed(&mut self) -> Vec<Effect> {
        let mut effects = self.detach_all();
        effects.push(Effect::Notice(HostNotice::Failed));
        effects
    }

    pub fn detach_all(&mut self) -> Vec<Effect> {
        self.active = None;
        self.pending = None;
        let workers: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.roster.live)
            .map(|(worker, _)| *worker)
            .collect();
        let mut effects = Vec::with_capacity(workers.len() + 1);
        for worker in workers {
            if let Ok(message) = self.send(worker, Command::Detach) {
                effects.push(Effect::Send(message));
            }
            if let Some(entry) = self.entries.get_mut(&worker) {
                entry.roster.live = false;
            }
        }
        effects.push(Effect::Notice(HostNotice::Detached));
        effects
    }

    pub fn tick(&mut self, now_ms: u64) -> Vec<Effect> {
        self.now = self.now.max(now_ms);
        let expired: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.roster.live && self.now.saturating_sub(entry.last_seen) >= entry.receipt_timeout())
            .map(|(worker, _)| *worker)
            .collect();
        let mut effects = Vec::new();
        for worker in expired {
            if let Some(entry) = self.entries.get_mut(&worker) {
                entry.roster.live = false;
            }
            if self.active() == Some(worker) || self.pending.is_some_and(|p| p.involves(worker)) {
                effects.extend(self.fail_closed());
            }
            effects.push(Effect::Notice(HostNotice::Expired(worker)));
        }
        if self.pending.is_some_and(|p| self.now >= p.deadline) {
            effects.extend(self.failed_step());
        }
        let workers: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                entry.roster.live
                    && entry
                        .last_heartbeat
                        .is_none_or(|last| self.now.saturating_sub(last) >= HEARTBEAT_MILLIS)
            })
            .map(|(worker, _)| *worker)
            .collect();
        for worker in workers {
            if let Ok(message) = self.send(
                worker,
                Command::Heartbeat {
                    workspace_foreground: self.foreground,
                },
            ) {
                effects.push(Effect::Send(message));
                if let Some(entry) = self.entries.get_mut(&worker) {
                    entry.last_heartbeat = Some(self.now);
                }
            }
        }
        effects
    }
}
