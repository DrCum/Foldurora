use crate::{
    Binding, Command, Envelope, Error, Event, FrameProof, GpuProgress, HostedViewport, Message, PrepareBoundary, RejectReason,
    SessionIdentity, WorkerIdentity, WorkerMode,
};

pub const LEASE_MILLIS: u64 = 15_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkerEffects {
    pub release_input: bool,
    pub quiesce: bool,
    pub prepare: bool,
    pub grant_input: bool,
    pub restore_window: bool,
    pub apply_viewport: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub reply_to: u64,
    pub event: Event,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkerTransition {
    pub effects: WorkerEffects,
    pub reply: Option<Reply>,
}

#[derive(Clone, Copy)]
enum Waiting {
    Quiescence(u64),
    Prepared(u64, Option<FrameProof>, Option<PrepareBoundary>),
}

/// Only validated, fresh local receipt renews this lease. The transport must
/// call tick even while no window frame is drawn. Returned effects are applied
/// by the worker's window thread, never by an IO thread.
pub struct WorkerGuard {
    identity: WorkerIdentity,
    session: Option<SessionIdentity>,
    generation: u64,
    lease_epoch: u64,
    sequence: u64,
    received: u64,
    mode: WorkerMode,
    activation: u64,
    foreground: bool,
    viewport: Option<HostedViewport>,
    waiting: Option<Waiting>,
    prepared: bool,
    last_frame: Option<FrameProof>,
}

impl WorkerGuard {
    pub fn new(identity: WorkerIdentity, lease_epoch: u64, now_ms: u64) -> Result<Self, Error> {
        if lease_epoch == 0 {
            return Err(Error::Sequence);
        }
        Ok(Self {
            identity,
            session: None,
            generation: 0,
            lease_epoch,
            sequence: 0,
            received: now_ms,
            mode: WorkerMode::Standby,
            activation: 0,
            foreground: false,
            viewport: None,
            waiting: None,
            prepared: false,
            last_frame: None,
        })
    }

    pub fn binding(&self) -> Binding {
        Binding {
            worker: self.identity,
            session: self.session,
        }
    }
    pub fn mode(&self) -> WorkerMode {
        self.mode
    }
    pub fn activation(&self) -> u64 {
        self.activation
    }
    pub fn render_owned(&self) -> bool {
        self.mode == WorkerMode::Active
    }
    pub fn input_owned(&self) -> bool {
        self.mode == WorkerMode::Active
    }
    pub fn viewport(&self) -> Option<HostedViewport> {
        self.viewport
    }
    /// Scheduling permission only: it never fabricates native keyboard focus.
    pub fn render_priority(&self, keyboard_focused: bool) -> bool {
        matches!(self.mode, WorkerMode::Active | WorkerMode::Preparing) && (self.foreground || keyboard_focused)
    }

    pub fn update_session(&mut self, session: Option<SessionIdentity>) -> Result<WorkerEffects, Error> {
        Binding {
            worker: self.identity,
            session,
        }
        .validate()?;
        if session == self.session {
            return Ok(WorkerEffects::default());
        }
        if session.is_some_and(|next| next.generation <= self.generation) {
            return Err(Error::Stale);
        }
        if let Some(next) = session {
            self.generation = next.generation;
        }
        self.session = session;
        self.last_frame = None;
        self.waiting = None;
        self.prepared = false;
        self.viewport = None;
        if self.mode != WorkerMode::Detached {
            self.mode = WorkerMode::Standby;
        }
        Ok(WorkerEffects {
            release_input: true,
            quiesce: true,
            restore_window: true,
            ..Default::default()
        })
    }

    /// Before device/surface recreation: a prepared frame for the retired
    /// display can never authorize the replacement. A new Prepare is needed.
    pub fn invalidate_display(&mut self) -> WorkerEffects {
        self.last_frame = None;
        self.waiting = None;
        self.prepared = false;
        self.viewport = None;
        if self.mode != WorkerMode::Detached {
            self.mode = WorkerMode::Standby;
        }
        WorkerEffects {
            release_input: true,
            quiesce: true,
            restore_window: true,
            ..Default::default()
        }
    }

    fn detach(&mut self) -> WorkerEffects {
        self.mode = WorkerMode::Detached;
        self.waiting = None;
        self.prepared = false;
        self.viewport = None;
        self.foreground = false;
        WorkerEffects {
            release_input: true,
            restore_window: true,
            ..Default::default()
        }
    }

    pub fn tick(&mut self, now_ms: u64) -> WorkerEffects {
        if self.mode != WorkerMode::Detached && now_ms.saturating_sub(self.received) >= LEASE_MILLIS {
            self.detach()
        } else {
            WorkerEffects::default()
        }
    }

    pub fn on_command(&mut self, envelope: &Envelope, now_ms: u64) -> Result<WorkerTransition, Error> {
        envelope.validate()?;
        let Message::Command(command) = &envelope.message else {
            return Err(Error::Invalid);
        };
        if envelope.binding.worker != self.identity || envelope.lease_epoch != self.lease_epoch {
            return Err(Error::Binding);
        }
        if self.mode == WorkerMode::Detached || envelope.sequence <= self.sequence || now_ms < self.received {
            return Err(Error::Stale);
        }
        if command.session_bound() && envelope.binding.session != self.session
            || !command.session_bound() && envelope.binding.session.is_some() && envelope.binding.session != self.session
        {
            return Err(Error::Binding);
        }
        if now_ms - self.received >= LEASE_MILLIS {
            return Ok(WorkerTransition {
                effects: self.detach(),
                reply: None,
            });
        }
        self.sequence = envelope.sequence;
        self.received = now_ms;
        let mut result = WorkerTransition::default();
        let mut rejected = false;
        match command {
            Command::Heartbeat { workspace_foreground } => self.foreground = *workspace_foreground,
            Command::Detach => {
                result.effects = self.detach();
                result.reply = Some(Reply {
                    reply_to: envelope.sequence,
                    event: Event::Detached,
                });
            }
            Command::Revoke { activation } if *activation >= self.activation => {
                self.activation = *activation;
                self.mode = WorkerMode::Revoking;
                self.prepared = false;
                self.waiting = Some(Waiting::Quiescence(envelope.sequence));
                result.effects = WorkerEffects {
                    release_input: true,
                    quiesce: true,
                    ..Default::default()
                };
            }
            Command::Prepare { activation, viewport } if self.mode == WorkerMode::Standby && *activation >= self.activation => {
                self.activation = *activation;
                self.mode = WorkerMode::Preparing;
                self.viewport = *viewport;
                self.prepared = false;
                self.waiting = Some(Waiting::Prepared(envelope.sequence, self.last_frame, None));
                result.effects = WorkerEffects {
                    release_input: true,
                    prepare: true,
                    apply_viewport: viewport.is_some(),
                    restore_window: viewport.is_none(),
                    ..Default::default()
                };
            }
            Command::Grant { activation } if self.mode == WorkerMode::Preparing && self.prepared && *activation == self.activation => {
                self.mode = WorkerMode::Active;
                self.prepared = false;
                result.effects.grant_input = true;
                result.reply = Some(Reply {
                    reply_to: envelope.sequence,
                    event: Event::Granted { activation: *activation },
                });
            }
            Command::SetViewport { activation, viewport } if self.mode == WorkerMode::Active && *activation == self.activation => {
                self.viewport = *viewport;
                result.effects.apply_viewport = viewport.is_some();
                result.effects.restore_window = viewport.is_none();
            }
            _ => rejected = true,
        }
        if rejected {
            result.reply = Some(Reply {
                reply_to: envelope.sequence,
                event: Event::Rejected {
                    activation: command.activation(),
                    reason: RejectReason::InvalidState,
                },
            });
        }
        Ok(result)
    }

    /// Called after drawing has stopped and all prior submissions completed.
    /// CPU packet acceptance or presentation alone cannot satisfy this fence.
    pub fn quiesced(&mut self, reply_to: u64, progress: GpuProgress) -> Result<Option<Reply>, Error> {
        progress.validate()?;
        let Some(Waiting::Quiescence(sequence)) = self.waiting else {
            return Ok(None);
        };
        if sequence != reply_to {
            return Err(Error::Stale);
        }
        if self.last_frame.is_some_and(|last| {
            progress.device_epoch < last.device_epoch || (progress.device_epoch == last.device_epoch && progress.submitted < last.frame)
        }) {
            return Err(Error::Stale);
        }
        if !progress.quiescent() {
            return Ok(None);
        }
        self.waiting = None;
        self.mode = WorkerMode::Standby;
        Ok(Some(Reply {
            reply_to: sequence,
            event: Event::Quiesced {
                activation: self.activation,
                progress,
            },
        }))
    }

    pub fn prepare_boundary(&mut self, reply_to: u64, boundary: PrepareBoundary) -> Result<(), Error> {
        boundary.validate()?;
        let Some(Waiting::Prepared(sequence, baseline, current)) = self.waiting else {
            return Err(Error::Invalid);
        };
        if sequence != reply_to
            || current.is_some_and(|old| old != boundary)
            || baseline.is_some_and(|old| {
                boundary.device_epoch < old.device_epoch
                    || (boundary.device_epoch == old.device_epoch
                        && (boundary.surface_epoch < old.surface_epoch || boundary.after_frame < old.frame))
            })
        {
            return Err(Error::Stale);
        }
        self.waiting = Some(Waiting::Prepared(sequence, baseline, Some(boundary)));
        Ok(())
    }

    /// Also records ordinary completed frames, so a later Prepare cannot
    /// reuse one from before its request or from a retired device/surface.
    pub fn frame_ready(&mut self, reply_to: u64, frame: FrameProof) -> Result<Option<Reply>, Error> {
        frame.validate()?;
        if let Some(Waiting::Prepared(sequence, _, _)) = self.waiting
            && sequence != reply_to
            && reply_to != 0
        {
            return Err(Error::Stale);
        }
        if reply_to != 0 && !matches!(self.waiting, Some(Waiting::Prepared(_, _, _))) {
            return Err(Error::Stale);
        }
        if let Some(Waiting::Prepared(_, baseline, boundary)) = self.waiting
            && reply_to != 0
            && (boundary.is_none_or(|boundary| !boundary.accepts(frame)) || !frame.newer_than(baseline))
        {
            return Ok(None);
        }
        if self.last_frame.is_some_and(|old| !frame.newer_than(Some(old)) && frame != old) {
            return Err(Error::Stale);
        }
        self.last_frame = Some(frame);
        let Some(Waiting::Prepared(sequence, baseline, boundary)) = self.waiting else {
            return Ok(None);
        };
        if reply_to == 0 {
            self.waiting = Some(Waiting::Prepared(sequence, Some(frame), boundary));
            return Ok(None);
        }
        let Some(boundary) = boundary else { return Ok(None) };
        if !frame.newer_than(baseline) || !boundary.accepts(frame) {
            return Ok(None);
        }
        self.waiting = None;
        self.prepared = true;
        Ok(Some(Reply {
            reply_to: sequence,
            event: Event::FrameReady {
                activation: self.activation,
                boundary,
                frame,
            },
        }))
    }
}
