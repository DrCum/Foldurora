use crate::control_v2::{
    ControlV2Command, ControlV2Envelope, ControlV2Message, ControlV2Reply, ControlV2Transition, ControlV2Work, SHOW_VIEWER_MILLIS,
    V2StartupClock,
};
use crate::show_viewer::ShowViewerOutcome;
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

struct Admission {
    binding: Binding,
    lease_epoch: u64,
    sequence: u64,
    session_bound: bool,
    exact_binding: bool,
    received_ms: u64,
    now_ms: u64,
}
#[derive(PartialEq, Eq)]
enum StartupClock {
    Pending,
    Initialized,
    Ineligible,
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
    startup_clock: StartupClock,
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
            startup_clock: StartupClock::Pending,
        })
    }

    pub fn binding(&self) -> Binding {
        Binding {
            worker: self.identity,
            session: self.session,
        }
    }
    /// One-shot initialization before this actual core has admitted any work
    /// or produced effects. Old ingress keeps its original stamp and therefore
    /// fails admission; this baseline is NEVER a native control receipt.
    pub fn initialize_startup_control_v2(&mut self, clock: V2StartupClock) -> Result<(), Error> {
        let now_ms = clock.millis();
        let eligible = self.startup_clock == StartupClock::Pending
            && self.sequence == 0
            && self.received <= now_ms
            && self.mode == WorkerMode::Standby
            && self.activation == 0
            && self.session.is_none()
            && self.generation == 0
            && self.viewport.is_none()
            && self.waiting.is_none()
            && !self.prepared
            && self.last_frame.is_none();
        self.startup_clock = StartupClock::Ineligible;
        if !eligible {
            return Err(Error::Stale);
        }
        self.received = now_ms;
        self.startup_clock = StartupClock::Initialized;
        Ok(())
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
    /// State information only, never physical ordinary-window permission.
    pub fn ordinary_standby(&self) -> bool {
        self.mode == WorkerMode::Standby && self.waiting.is_none() && self.viewport.is_none() && !self.prepared
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
        self.startup_clock = StartupClock::Ineligible;
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
        self.startup_clock = StartupClock::Ineligible;
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
        self.startup_clock = StartupClock::Ineligible;
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
        if now_ms < self.received {
            self.startup_clock = StartupClock::Ineligible;
        }
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
        if let Some(effects) = self.admit(Admission {
            binding: envelope.binding,
            lease_epoch: envelope.lease_epoch,
            sequence: envelope.sequence,
            session_bound: command.session_bound(),
            exact_binding: false,
            received_ms: now_ms,
            now_ms,
        })? {
            return Ok(WorkerTransition { effects, reply: None });
        }
        Ok(self.apply_command(command, envelope.sequence))
    }

    /// The caller supplies the ORIGINAL local IO stamp, not App dequeue time.
    /// Admission consumes this Guard's shared control sequence once. Returned
    /// Show work is bookkeeping only; actual reply/ledger reservation and the
    /// current ordinary native owner remain mandatory before any effect.
    pub fn on_control_v2(&mut self, envelope: &ControlV2Envelope, received_ms: u64, now_ms: u64) -> Result<ControlV2Transition, Error> {
        envelope.validate()?;
        let ControlV2Message::Command(command) = &envelope.message else {
            return Err(Error::Invalid);
        };
        let control = command.control();
        if let Some(effects) = self.admit(Admission {
            binding: envelope.binding,
            lease_epoch: envelope.lease_epoch,
            sequence: envelope.sequence,
            session_bound: control.as_ref().is_some_and(Command::session_bound),
            exact_binding: control.is_none(),
            received_ms,
            now_ms,
        })? {
            return Ok(ControlV2Transition {
                effects,
                ..Default::default()
            });
        }
        if let Some(command) = control {
            let result = self.apply_command(&command, envelope.sequence);
            return Ok(ControlV2Transition {
                effects: result.effects,
                reply: result.reply.map(ControlV2Reply::from_control),
                work: None,
            });
        }
        let work = match command {
            ControlV2Command::ShowViewer(request) => ControlV2Work::ShowViewer {
                request: *request,
                outcome: if now_ms - received_ms >= SHOW_VIEWER_MILLIS {
                    Some(ShowViewerOutcome::Expired)
                } else if self.mode != WorkerMode::Standby || self.waiting.is_some() || self.viewport.is_some() {
                    Some(ShowViewerOutcome::RejectedNative)
                } else {
                    None
                },
            },
            ControlV2Command::QueryShowViewer(request) => ControlV2Work::QueryShowViewer(*request),
            _ => return Err(Error::Invalid),
        };
        Ok(ControlV2Transition {
            work: Some(work),
            ..Default::default()
        })
    }

    fn admit(&mut self, admission: Admission) -> Result<Option<WorkerEffects>, Error> {
        let Admission {
            binding,
            lease_epoch,
            sequence,
            session_bound,
            exact_binding,
            received_ms,
            now_ms,
        } = admission;
        if binding.worker != self.identity || lease_epoch != self.lease_epoch {
            return Err(Error::Binding);
        }
        if received_ms > now_ms || received_ms < self.received {
            self.startup_clock = StartupClock::Ineligible;
            return Err(Error::Stale);
        }
        if self.mode == WorkerMode::Detached || sequence <= self.sequence {
            return Err(Error::Stale);
        }
        if binding.session != self.session && (exact_binding || session_bound || binding.session.is_some()) {
            return Err(Error::Binding);
        }
        if now_ms - self.received >= LEASE_MILLIS {
            return Ok(Some(self.detach()));
        }
        self.sequence = sequence;
        self.startup_clock = StartupClock::Ineligible;
        self.received = received_ms;
        Ok(None)
    }

    fn apply_command(&mut self, command: &Command, sequence: u64) -> WorkerTransition {
        let mut result = WorkerTransition::default();
        let mut rejected = false;
        match command {
            Command::Heartbeat { workspace_foreground } => self.foreground = *workspace_foreground,
            Command::Detach => {
                result.effects = self.detach();
                result.reply = Some(Reply {
                    reply_to: sequence,
                    event: Event::Detached,
                });
            }
            Command::Revoke { activation } if *activation >= self.activation => {
                self.activation = *activation;
                self.mode = WorkerMode::Revoking;
                self.prepared = false;
                self.waiting = Some(Waiting::Quiescence(sequence));
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
                self.waiting = Some(Waiting::Prepared(sequence, self.last_frame, None));
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
                    reply_to: sequence,
                    event: Event::Granted { activation: *activation },
                });
            }
            Command::SetViewport { activation, viewport }
                if self.mode == WorkerMode::Active && *activation == self.activation && *viewport == self.viewport =>
            {
                // An unchanged policy is idempotent. Any geometry/surface
                // change must revoke, fence and prepare a fresh frame first.
            }
            _ => rejected = true,
        }
        if rejected {
            result.reply = Some(Reply {
                reply_to: sequence,
                event: Event::Rejected {
                    activation: command.activation(),
                    reason: RejectReason::InvalidState,
                },
            });
        }
        result
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
        self.startup_clock = StartupClock::Ineligible;
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
