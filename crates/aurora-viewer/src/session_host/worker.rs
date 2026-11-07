//! Viewer-side host bridge. Bootstrap is validated before profile writes;
//! window/input effects are applied only on this viewer's own event thread.

use super::transport::{self, WorkerConfig, WorkerIo};
use aurora_session::{AccountKey, Command, Event, GridKey, Message, RejectReason, WorkerEffects, WorkerGuard, WorkerMode, WorkerStatus};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

struct Startup {
    config: WorkerConfig,
    directory: PathBuf,
}
static STARTUP: OnceLock<Startup> = OnceLock::new();

pub(crate) fn initialize_startup(path: Option<&Path>) -> anyhow::Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    let cli = crate::cli::get();
    anyhow::ensure!(
        !cli.vr && cli.url.is_none() && cli.grid.is_none(),
        "host workers cannot override their bound login context"
    );
    let (config, directory) = transport::read_config(path)?;
    anyhow::ensure!(
        cli.saved_account.as_deref() == config.expected_account.as_ref().map(ToString::to_string).as_deref(),
        "worker account selection differs from its private configuration"
    );
    // A bootstrap belongs to one process launch only. Claim it before even
    // activating its profile, so a replay cannot write into a live profile.
    let claim = super::profile::create_private_file(&directory.join("consumed"))?;
    drop(claim);
    super::profile::activate(
        &config.identity.installation.to_string(),
        uuid::Uuid::from_bytes(*config.session.as_bytes()),
        config.generation,
        config.slot,
        config.identity.host,
    )?;
    STARTUP
        .set(Startup { config, directory })
        .map_err(|_| anyhow::anyhow!("worker bootstrap already initialized"))?;
    Ok(())
}

#[derive(Clone, Copy)]
enum DisplayGate {
    Login,
    Draining {
        ticket: u64,
        reply_to: u64,
        through: u64,
    },
    Standby,
    Fitting {
        reply_to: u64,
    },
    Settling {
        reply_to: u64,
        stable: u8,
    },
    Preparing {
        reply_to: u64,
        boundary: aurora_session::PrepareBoundary,
    },
    FrameFence {
        ticket: u64,
        reply_to: u64,
        boundary: aurora_session::PrepareBoundary,
    },
    Prepared,
    Active,
    GeometryPending,
    Detached,
}

impl DisplayGate {
    fn allows_input(self) -> bool {
        matches!(self, Self::Active)
    }
    fn allows_world(self) -> bool {
        matches!(self, Self::Preparing { .. } | Self::Active)
    }
    fn allows_submission(self) -> bool {
        matches!(self, Self::Login | Self::Preparing { .. } | Self::Active)
    }
}

pub(crate) struct WorkerBridge {
    config: WorkerConfig,
    io: WorkerIo,
    guard: WorkerGuard,
    since: Option<Instant>,
    detached: bool,
    generation: u64,
    previous_account: Option<(AccountKey, GridKey)>,
    presentation: NativePresentation,
    gate: DisplayGate,
    status: WorkerStatus,
    admitted: bool,
    supported: bool,
    next_ticket: u64,
    request_sequence: u64,
    device_epoch: u64,
    window: u64,
    last_frame: Option<aurora_session::FrameProof>,
    last_receipt: Option<Instant>,
    chat: Option<super::chat_io::WorkerChannel>,
}

impl WorkerBridge {
    pub fn from_startup() -> anyhow::Result<Option<Self>> {
        let Some(startup) = STARTUP.get() else {
            return Ok(None);
        };
        let chat = startup.config.chat.as_ref().and_then(|chat| {
            let permit = super::chat_transport::Permit {
                identity: startup.config.identity,
                lease_epoch: startup.config.lease_epoch,
                secret: chat.key.clone(),
            };
            match super::chat_io::WorkerChannel::start(chat.port, permit) {
                Ok(channel) => Some(channel),
                Err(_) => {
                    log::warn!("session chat IO could not start; native chat remains unavailable");
                    None
                }
            }
        });
        Ok(Some(Self {
            config: startup.config.clone(),
            io: WorkerIo::start(startup.config.clone(), startup.directory.clone())?,
            guard: WorkerGuard::new(startup.config.identity, startup.config.lease_epoch, 0)?,
            since: None,
            detached: false,
            generation: startup.config.generation - 1,
            previous_account: None,
            presentation: Default::default(),
            gate: DisplayGate::Login,
            status: WorkerStatus::default(),
            admitted: false,
            supported: false,
            next_ticket: 0,
            request_sequence: 0,
            device_epoch: 1,
            window: 0,
            last_frame: None,
            last_receipt: None,
            chat,
        }))
    }

    /// First decide ownership. The App applies returned input-release effects
    /// before pump_display may publish any completed handoff acknowledgment.
    pub fn tick(&mut self, status: WorkerStatus, account: Option<(AccountKey, GridKey)>) -> WorkerEffects {
        self.status = status;
        if self.detached {
            return WorkerEffects::default();
        }
        if account != self.previous_account {
            if account.is_some() {
                let Some(next) = self.generation.checked_add(1) else {
                    return self.fail_closed();
                };
                self.generation = next;
            }
            self.previous_account = account;
        }
        if account.is_some_and(|(account, grid)| {
            self.config.expected_account.is_some_and(|key| key != account) || self.config.expected_grid.is_some_and(|key| key != grid)
        }) || self.io.failed()
        {
            return self.fail_closed();
        }
        let session = account.map(|(account, grid)| {
            let mut session = self.config.session_binding(account, grid);
            session.generation = self.generation;
            session
        });
        let Ok(mut effects) = self.guard.update_session(session) else {
            return self.fail_closed();
        };
        if effects.quiesce {
            self.request_sequence = 0;
            self.admitted = false;
            self.last_frame = None;
        }
        let now = session_clock(&mut self.since, Instant::now());
        // Collect before mutating self; the transport queue has a strict cap.
        let commands: Vec<_> = self.io.commands().collect();
        for envelope in commands {
            let Message::Command(command) = &envelope.message else {
                continue;
            };
            if command.session_bound()
                && (!self.supported || !self.admitted || (self.status.blocked && !matches!(command, Command::Revoke { .. })))
            {
                if envelope.binding == self.guard.binding() && envelope.lease_epoch == self.config.lease_epoch {
                    self.io.reply(
                        self.guard.binding(),
                        envelope.sequence,
                        Event::Rejected {
                            activation: command.activation(),
                            reason: if !self.supported {
                                RejectReason::Unsupported
                            } else {
                                RejectReason::Blocked
                            },
                        },
                    );
                }
                continue;
            }
            if let Ok(transition) = self.guard.on_command(&envelope, now) {
                if self.guard.mode() != WorkerMode::Detached {
                    self.last_receipt = Some(Instant::now());
                }
                if transition.effects.quiesce || transition.effects.prepare {
                    self.request_sequence = envelope.sequence;
                }
                merge_effects(&mut effects, transition.effects);
                if let Some(reply) = transition.reply {
                    // Grant acknowledgment is deferred until the native window
                    // has been shown; detach until it has been restored.
                    if matches!(reply.event, Event::Granted { .. } | Event::Detached) {
                        self.request_sequence = reply.reply_to;
                    } else if !self.io.reply(self.guard.binding(), reply.reply_to, reply.event) {
                        merge_effects(&mut effects, self.fail_closed());
                    }
                } else if matches!(command, Command::Heartbeat { .. } | Command::SetViewport { .. }) {
                    let mut status = self.status.clone();
                    status.blocked |= !self.supported || (!self.admitted && self.status.phase == aurora_session::Phase::Online);
                    status.last_frame = self.last_frame;
                    self.io.reply(self.guard.binding(), envelope.sequence, Event::Status(status));
                }
            }
        }
        merge_effects(&mut effects, self.guard.tick(now));
        if self.guard.mode() == WorkerMode::Detached {
            merge_effects(&mut effects, self.fail_closed());
        }
        self.sync_chat_source();
        final_display_effects(effects, self.guard.mode())
    }

    pub fn input_allowed(&self) -> bool {
        self.detached
            || !self.supported
            || (self.status.keyboard_focused
                && (self.status.phase == aurora_session::Phase::Login
                    || (matches!(self.gate, DisplayGate::Login) && self.status.phase == aurora_session::Phase::Connecting)
                    || (self.gate.allows_input() && self.guard.input_owned() && self.bound_session().is_some())))
    }
    /// Fresh local channel authority only. Standby is eligible for native chat,
    /// but a source never grants world rendering, keyboard input or focus.
    pub(crate) fn bound_session(&self) -> Option<(aurora_session::Binding, u64)> {
        let binding = self.guard.binding();
        (!self.detached
            && self.status.phase == aurora_session::Phase::Online
            && binding.session.is_some()
            && receipt_is_current(self.last_receipt, Instant::now()))
        .then_some((binding, self.config.lease_epoch))
    }

    fn sync_chat_source(&self) {
        if let Some(chat) = &self.chat {
            let source = self
                .bound_session()
                .and_then(|binding| aurora_session::chat::ChatSource::try_from(binding).ok());
            let expires = self
                .last_receipt
                .and_then(|received| received.checked_add(std::time::Duration::from_millis(aurora_session::LEASE_MILLIS)));
            chat.set_source(source, expires);
        }
    }

    pub(crate) fn chat_ready(&self) -> bool {
        self.chat.as_ref().is_some_and(|chat| chat.ready())
    }
    pub(crate) fn chat_commands(&self) -> Vec<super::chat_io::ReceivedCommand> {
        self.chat.as_ref().map_or_else(Vec::new, |chat| chat.commands())
    }
    pub(crate) fn chat_send(&self, packet: &aurora_session::chat::ChatPacket) -> Result<(), super::chat_transport::LaneError> {
        self.chat.as_ref().ok_or(super::chat_transport::LaneError::NotReady)?.send(packet)
    }

    /// GUI review has no standalone/unsupported input fallbacks. Core Active
    /// alone precedes native grant completion and cannot authorize this effect.
    pub(crate) fn review_authorized(
        &self,
        source: aurora_session::chat::ChatSource,
        activation: u64,
        window: &winit::window::Window,
    ) -> Result<(), aurora_session::chat::ReviewOutcome> {
        use aurora_session::chat::ReviewOutcome;
        let current = self
            .bound_session()
            .and_then(|binding| aurora_session::chat::ChatSource::try_from(binding).ok());
        let facts = ReviewAuthority {
            current,
            activation: self.guard.activation(),
            managed: !self.detached && self.supported && self.admitted,
            active: self.guard.mode() == WorkerMode::Active,
            display_active: matches!(self.gate, DisplayGate::Active),
            render_owned: self.guard.render_owned(),
            input_owned: self.guard.input_owned(),
            blocked: self.status.blocked,
            native_matches: self.window != 0 && native_window(window) == self.window && self.status.native_window == self.window,
        };
        if !facts.accepts(source, activation) {
            return Err(ReviewOutcome::RejectedNative);
        }
        let viewport = self.guard.viewport().ok_or(ReviewOutcome::RejectedNative)?;
        if !matches!(
            self.presentation.state(window, viewport, self.config.host_pid),
            Ok(NativeState::Ready)
        ) {
            return Err(ReviewOutcome::RejectedNative);
        }
        #[cfg(windows)]
        if native::review_foreground(viewport.host_window, self.window, self.config.host_pid) {
            return Ok(());
        }
        Err(ReviewOutcome::NotForeground)
    }

    pub fn world_allowed(&self) -> bool {
        self.detached || !self.supported || self.gate.allows_world()
    }
    pub fn submission_allowed(&self) -> bool {
        self.detached || !self.supported || self.gate.allows_submission()
    }
    pub fn needs_prepared_frame(&self) -> bool {
        !self.detached && matches!(self.gate, DisplayGate::Preparing { .. })
    }
    pub fn hosted(&self) -> bool {
        !self.detached && self.guard.viewport().is_some()
    }
    pub fn render_priority(&self, keyboard_focused: bool) -> bool {
        if self.detached || !self.supported {
            keyboard_focused
        } else {
            #[cfg(windows)]
            if let Some(viewport) = self.guard.viewport() {
                // Scheduling only. Never assign host focus to App.focused or
                // use it as permission for movement, shortcuts or microphone.
                return self.gate.allows_world()
                    && matches!(self.guard.mode(), WorkerMode::Preparing | WorkerMode::Active)
                    && native::workspace_foreground(viewport.host_window, self.window, self.config.host_pid);
            }
            self.guard.render_priority(keyboard_focused)
        }
    }
    pub fn expected_resize(&self, width: u32, height: u32) -> bool {
        matches!(self.gate, DisplayGate::Settling { .. })
            && self
                .guard
                .viewport()
                .is_some_and(|view| view.rect.width == width && view.rect.height == height)
    }

    pub fn invalidate_display(&mut self) -> WorkerEffects {
        let mut effects = self.guard.invalidate_display();
        merge_effects(&mut effects, self.fail_closed());
        effects
    }

    fn fail_closed(&mut self) -> WorkerEffects {
        self.detached = true;
        self.admitted = false;
        if let Some(chat) = &self.chat {
            chat.set_source(None, None);
        }
        WorkerEffects {
            release_input: true,
            restore_window: true,
            ..Default::default()
        }
    }

    fn cancel_fence(&self, renderer: &mut aurora_render::Renderer) {
        if let DisplayGate::Draining { ticket, .. } | DisplayGate::FrameFence { ticket, .. } = self.gate {
            renderer.cancel_session_fence(ticket);
        }
    }

    fn start_fence(&mut self, renderer: &mut aurora_render::Renderer) -> Result<(u64, u64), aurora_render::SessionFenceError> {
        self.cancel_fence(renderer);
        let through = renderer.session_boundary()?.after_frame;
        self.next_ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or(aurora_render::SessionFenceError::StaleTicket)?;
        renderer.request_session_fence(self.next_ticket)?;
        Ok((self.next_ticket, through))
    }

    /// Run after held movement/PTT/cursor were released, on the native event
    /// thread. No device poll, file operation or GPU wait occurs on this path.
    pub fn pump_display(&mut self, effects: WorkerEffects, window: &winit::window::Window, renderer: &mut aurora_render::Renderer) -> bool {
        let previously_supported = self.supported;
        self.supported = cfg!(windows) && renderer.session_boundary().is_ok()
            || (cfg!(windows) && matches!(self.gate, DisplayGate::Draining { .. } | DisplayGate::FrameFence { .. }));
        let current = native_window(window);
        let changed = self.window != 0 && current != self.window;
        self.window = current;
        if changed {
            self.invalidate_display();
            return true;
        }
        if self.detached {
            if matches!(self.gate, DisplayGate::Detached) {
                return false;
            }
            self.cancel_fence(renderer);
            self.presentation.restore(window);
            self.gate = DisplayGate::Detached;
            self.io.reply(self.guard.binding(), self.request_sequence, Event::Detached);
            self.io.stop();
            return false;
        }
        if previously_supported && !self.supported && self.guard.binding().session.is_some() {
            // Losing the fence contract (for example entering XR) cannot turn
            // a managed world into an ordinary full-rate renderer while the
            // controller still believes it owns the prepared native surface.
            self.invalidate_display();
            return true;
        }
        if !self.supported {
            self.presentation.restore(window);
            self.gate = DisplayGate::Login;
            self.publish_status();
            return false;
        }
        let geometry_was_pending = matches!(self.gate, DisplayGate::GeometryPending);
        let result = self.apply_display(effects, window, renderer);
        if let Err(error) = result {
            log::warn!("session presentation detached: {error}");
            self.fail_closed();
            self.cancel_fence(renderer);
            self.presentation.restore(window);
            // The caller must release held native input before a second pump
            // publishes Detached. A native failure cannot acknowledge early.
            return true;
        }
        self.publish_status();
        // Geometry drift can precede the broker's Revoke. Release held native
        // input now, while retaining control IO for the fenced re-prepare.
        !geometry_was_pending && matches!(self.gate, DisplayGate::GeometryPending)
    }

    fn publish_status(&self) {
        let mut status = self.status.clone();
        status.blocked |= !self.supported || (!self.admitted && status.phase == aurora_session::Phase::Online);
        status.last_frame = self.last_frame;
        self.io.status(self.guard.binding(), status);
    }

    fn apply_display(
        &mut self,
        effects: WorkerEffects,
        window: &winit::window::Window,
        renderer: &mut aurora_render::Renderer,
    ) -> anyhow::Result<()> {
        if effects.restore_window {
            self.presentation.restore(window);
        }
        if self.guard.binding().session.is_none() {
            self.cancel_fence(renderer);
            self.gate = DisplayGate::Login;
            self.admitted = false;
            return Ok(());
        }
        if effects.quiesce && self.guard.binding().session.is_some() {
            let (ticket, through) = self.start_fence(renderer)?;
            self.gate = DisplayGate::Draining {
                ticket,
                reply_to: self.request_sequence,
                through,
            };
        }
        if effects.prepare {
            self.cancel_fence(renderer);
            self.presentation.hide(window);
            self.gate = DisplayGate::Fitting {
                reply_to: self.request_sequence,
            };
        }
        if matches!(
            self.gate,
            DisplayGate::Settling { .. }
                | DisplayGate::Preparing { .. }
                | DisplayGate::FrameFence { .. }
                | DisplayGate::Prepared
                | DisplayGate::Active
                | DisplayGate::GeometryPending
        ) {
            if let Some(viewport) = self.guard.viewport() {
                match self.presentation.state(window, viewport, self.config.host_pid)? {
                    NativeState::WorkerChanged => anyhow::bail!("native worker viewport changed after preparation"),
                    NativeState::HostChanged => {
                        self.cancel_fence(renderer);
                        self.gate = DisplayGate::GeometryPending;
                        self.presentation.hide(window);
                    }
                    NativeState::Ready => {}
                }
            }
            if matches!(self.gate, DisplayGate::GeometryPending) {
                // Even a drag back to the same rectangle needs a new broker
                // revoke/fence/prepare. A late old Grant cannot show this frame.
                return Ok(());
            }
        }
        if (matches!(self.gate, DisplayGate::Prepared | DisplayGate::Active) || effects.grant_input)
            && let Some(proof) = self.last_frame
        {
            anyhow::ensure!(
                renderer.session_boundary()?.surface_epoch == proof.surface_epoch,
                "render surface changed after preparation"
            );
        }
        if effects.grant_input {
            self.gate = DisplayGate::Active;
            window.set_visible(true);
            window.focus_window();
            if let Some(viewport) = self.guard.viewport() {
                native_order(window, viewport)?;
            }
            anyhow::ensure!(
                self.io.reply(
                    self.guard.binding(),
                    self.request_sequence,
                    Event::Granted {
                        activation: self.guard.activation()
                    }
                ),
                "host reply queue is full"
            );
        }
        if matches!(self.gate, DisplayGate::Active)
            && let Some(viewport) = self.guard.viewport()
        {
            native_order(window, viewport)?;
        }
        match self.gate {
            DisplayGate::Fitting { reply_to } => {
                if let Some(viewport) = self.guard.viewport()
                    && !self.presentation.fit(window, viewport, self.config.host_pid)?
                {
                    // The controller may already be moving again. Keep the
                    // old window hidden until a new exact viewport is usable.
                    return Ok(());
                }
                self.gate = DisplayGate::Settling { reply_to, stable: 0 };
            }
            DisplayGate::Draining { ticket, reply_to, through } => {
                if renderer.poll_session_fence(ticket)?.is_some() {
                    if reply_to != 0 {
                        let progress = aurora_session::GpuProgress {
                            device_epoch: self.device_epoch,
                            submitted: through,
                            completed: through,
                        };
                        let reply = self
                            .guard
                            .quiesced(reply_to, progress)?
                            .ok_or_else(|| anyhow::anyhow!("quiescence request changed before completion"))?;
                        anyhow::ensure!(
                            self.io.reply(self.guard.binding(), reply.reply_to, reply.event),
                            "host reply queue is full"
                        );
                    }
                    self.admitted = true;
                    self.gate = DisplayGate::Standby;
                    self.presentation.hide(window);
                }
            }
            DisplayGate::Settling { reply_to, stable } => {
                let size = window.inner_size();
                renderer.resize(size.width, size.height);
                if stable < 1 {
                    self.gate = DisplayGate::Settling {
                        reply_to,
                        stable: stable + 1,
                    };
                } else {
                    let boundary = renderer.session_boundary()?;
                    let boundary = aurora_session::PrepareBoundary {
                        device_epoch: self.device_epoch,
                        surface_epoch: boundary.surface_epoch,
                        after_frame: boundary.after_frame,
                    };
                    self.guard.prepare_boundary(reply_to, boundary)?;
                    self.gate = DisplayGate::Preparing { reply_to, boundary };
                }
            }
            DisplayGate::FrameFence {
                ticket,
                reply_to,
                boundary,
            } => {
                if let Some(completed) = renderer.poll_session_fence(ticket)? {
                    let proof = prepared_proof(boundary, completed)?;
                    let reply = self
                        .guard
                        .frame_ready(reply_to, proof)?
                        .ok_or_else(|| anyhow::anyhow!("prepared frame was not accepted"))?;
                    self.last_frame = Some(proof);
                    self.gate = DisplayGate::Prepared;
                    anyhow::ensure!(
                        self.io.reply(self.guard.binding(), reply.reply_to, reply.event),
                        "host reply queue is full"
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Called directly after the prepared packet handoff. Its CPU statistics
    /// are not evidence; fence only after a newer packet was actually accepted.
    pub fn after_submission(&mut self, renderer: &mut aurora_render::Renderer) -> bool {
        let DisplayGate::Preparing { reply_to, boundary } = self.gate else {
            return false;
        };
        let result = (|| -> Result<(), aurora_render::SessionFenceError> {
            if renderer.session_boundary()?.after_frame > boundary.after_frame {
                let (ticket, _) = self.start_fence(renderer)?;
                self.gate = DisplayGate::FrameFence {
                    ticket,
                    reply_to,
                    boundary,
                };
            }
            Ok(())
        })();
        if result.is_err() {
            self.fail_closed();
            return true;
        }
        false
    }
}

#[cfg(any(windows, test))]
fn contains_rect(x: i32, y: i32, width: i32, height: i32, rect: aurora_session::PhysicalRect) -> bool {
    width > 0
        && height > 0
        && i64::from(rect.x) >= i64::from(x)
        && i64::from(rect.y) >= i64::from(y)
        && i64::from(rect.x) + i64::from(rect.width) <= i64::from(x) + i64::from(width)
        && i64::from(rect.y) + i64::from(rect.height) <= i64::from(y) + i64::from(height)
}

fn prepared_proof(
    boundary: aurora_session::PrepareBoundary,
    completed: aurora_render::SessionCompletion,
) -> anyhow::Result<aurora_session::FrameProof> {
    let frame = completed
        .frame
        .and_then(|frame| frame.checked_add(1))
        .ok_or_else(|| anyhow::anyhow!("prepared frame was not presented"))?;
    let proof = aurora_session::FrameProof {
        device_epoch: boundary.device_epoch,
        surface_epoch: completed.surface_epoch,
        frame,
    };
    anyhow::ensure!(boundary.accepts(proof), "prepared frame belongs to an old surface or submission");
    Ok(proof)
}

fn session_clock(since: &mut Option<Instant>, now: Instant) -> u64 {
    now.saturating_duration_since(*since.get_or_insert(now))
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(any(windows, test))]
fn workspace_family_foreground(
    foreground: u64,
    host: u64,
    worker: u64,
    host_pid: u32,
    mut relation: impl FnMut(u64) -> Option<(u32, u64)>,
) -> bool {
    if foreground == 0 || host == 0 || host_pid == 0 {
        return false;
    }
    if foreground == worker && worker != 0 {
        return true;
    }
    let mut current = foreground;
    for _ in 0..8 {
        let Some((pid, owner)) = relation(current) else {
            return false;
        };
        if pid != host_pid {
            return false;
        }
        if current == host {
            return true;
        }
        if owner == 0 || owner == current {
            return false;
        }
        current = owner;
    }
    false
}

fn receipt_is_current(received: Option<Instant>, now: Instant) -> bool {
    received
        .and_then(|received| now.checked_duration_since(received))
        .is_some_and(|age| age < std::time::Duration::from_millis(aurora_session::LEASE_MILLIS))
}

struct ReviewAuthority {
    current: Option<aurora_session::chat::ChatSource>,
    activation: u64,
    managed: bool,
    active: bool,
    display_active: bool,
    render_owned: bool,
    input_owned: bool,
    blocked: bool,
    native_matches: bool,
}

impl ReviewAuthority {
    fn accepts(&self, source: aurora_session::chat::ChatSource, activation: u64) -> bool {
        source.validate().is_ok()
            && self.current == Some(source)
            && activation != 0
            && activation == self.activation
            && self.managed
            && self.active
            && self.display_active
            && self.render_owned
            && self.input_owned
            && !self.blocked
            && self.native_matches
    }
}

fn final_display_effects(mut effects: WorkerEffects, mode: WorkerMode) -> WorkerEffects {
    // A newer revoke can supersede a queued Prepare/Grant in the same UI tick.
    // Input release is cumulative; presentation authority is only final-state.
    effects.prepare &= mode == WorkerMode::Preparing;
    effects.grant_input &= mode == WorkerMode::Active;
    effects.quiesce &= matches!(mode, WorkerMode::Standby | WorkerMode::Revoking);
    effects
}

fn merge_effects(target: &mut WorkerEffects, next: WorkerEffects) {
    target.release_input |= next.release_input;
    target.quiesce |= next.quiesce;
    target.prepare |= next.prepare;
    target.grant_input |= next.grant_input;
    target.restore_window |= next.restore_window;
    target.apply_viewport |= next.apply_viewport;
}

pub(crate) fn native_window(window: &winit::window::Window) -> u64 {
    #[cfg(windows)]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            return handle.hwnd.get() as u64;
        }
    }
    let _ = window;
    0
}

/// Worker-owned top-level window styles, following `FSSessionWorker::embed`
/// (`indra/newview/fssessionworker.cpp`, pinned abbe3cb0, lines 588/640).
/// This deliberately never reparents a foreign window. The shell opens its
/// own viewport region only after this worker acknowledges the granted frame.
#[derive(Default)]
struct NativePresentation {
    hidden: bool,
    #[cfg(windows)]
    saved: Option<native::Saved>,
    #[cfg(windows)]
    host_rect: Option<aurora_session::PhysicalRect>,
}

impl NativePresentation {
    fn fit(&mut self, window: &winit::window::Window, viewport: aurora_session::HostedViewport, host_pid: u32) -> anyhow::Result<bool> {
        #[cfg(windows)]
        {
            if self.saved.is_none() {
                self.saved = Some(native::save(window)?);
            }
            let Some(host_rect) = native::fit(window, viewport, host_pid)? else {
                return Ok(false);
            };
            self.host_rect = Some(host_rect);
            self.hidden = true;
            window.set_visible(false);
            Ok(true)
        }
        #[cfg(not(windows))]
        {
            let _ = (window, viewport, host_pid);
            anyhow::bail!("native hosting is unsupported on this platform");
        }
    }

    fn state(
        &self,
        window: &winit::window::Window,
        viewport: aurora_session::HostedViewport,
        host_pid: u32,
    ) -> anyhow::Result<NativeState> {
        #[cfg(windows)]
        {
            native::state(window, viewport, host_pid, self.host_rect)
        }
        #[cfg(not(windows))]
        {
            let _ = (window, viewport, host_pid);
            anyhow::bail!("native hosting is unsupported on this platform");
        }
    }

    fn hide(&mut self, window: &winit::window::Window) {
        self.hidden = true;
        window.set_visible(false);
    }

    fn restore(&mut self, window: &winit::window::Window) {
        #[cfg(windows)]
        {
            self.host_rect = None;
            if let Some(saved) = self.saved.take() {
                native::restore(window, saved);
            }
        }
        if self.hidden {
            window.set_minimized(false);
            window.set_visible(true);
            self.hidden = false;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
enum NativeState {
    Ready,
    HostChanged,
    WorkerChanged,
}

#[cfg(any(windows, test))]
fn native_state(
    host_before: Option<aurora_session::PhysicalRect>,
    host_now: aurora_session::PhysicalRect,
    worker_now: aurora_session::PhysicalRect,
    viewport: aurora_session::HostedViewport,
    client_size: [u32; 2],
) -> NativeState {
    if worker_now != viewport.rect || client_size != [viewport.rect.width, viewport.rect.height] {
        NativeState::WorkerChanged
    } else if host_before != Some(host_now) {
        NativeState::HostChanged
    } else {
        NativeState::Ready
    }
}

fn native_order(window: &winit::window::Window, viewport: aurora_session::HostedViewport) -> anyhow::Result<()> {
    #[cfg(windows)]
    native::stay_below_host(window, viewport)?;
    #[cfg(not(windows))]
    let _ = (window, viewport);
    Ok(())
}

#[cfg(windows)]
mod native {
    use windows_sys::Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::Gdi::ClientToScreen,
        UI::WindowsAndMessaging::{
            GW_HWNDNEXT, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetClientRect, GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowRect,
            GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, IsZoomed, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
            SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
            WS_EX_APPWINDOW, WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_VISIBLE,
        },
    };
    use winit::{dpi::LogicalSize, platform::windows::WindowExtWindows, window::Window};

    pub(super) struct Saved {
        style: isize,
        extended: isize,
        rect: RECT,
        maximized: bool,
        fullscreen: Option<winit::window::Fullscreen>,
    }

    pub(super) fn save(window: &Window) -> std::io::Result<Saved> {
        let hwnd = super::native_window(window) as HWND;
        if hwnd.is_null() {
            return Err(std::io::Error::other("worker has no native window"));
        }
        // SAFETY: read only properties of this process's own winit window.
        unsafe {
            if !GetWindow(hwnd, GW_OWNER).is_null() {
                return Err(std::io::Error::other("worker world window has a native owner"));
            }
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut rect) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(Saved {
                style: GetWindowLongPtrW(hwnd, GWL_STYLE),
                extended: GetWindowLongPtrW(hwnd, GWL_EXSTYLE),
                rect,
                maximized: IsZoomed(hwnd) != 0,
                fullscreen: window.fullscreen(),
            })
        }
    }

    pub(super) fn fit(
        window: &Window,
        viewport: aurora_session::HostedViewport,
        host_pid: u32,
    ) -> std::io::Result<Option<aurora_session::PhysicalRect>> {
        viewport.validate().map_err(std::io::Error::other)?;
        let hwnd = super::native_window(window) as HWND;
        let host = viewport.host_window as HWND;
        let host_rect = host_rect(viewport, host_pid)?;
        // SAFETY: foreign host HWND is queried only. Styles and placement are
        // changed exclusively on this process's own world window.
        unsafe {
            if host == hwnd {
                return Err(std::io::Error::other("worker cannot host itself"));
            }
            if !GetWindow(hwnd, GW_OWNER).is_null() {
                return Err(std::io::Error::other("worker world window has a native owner"));
            }
            if !super::contains_rect(
                host_rect.x,
                host_rect.y,
                host_rect.width as i32,
                host_rect.height as i32,
                viewport.rect,
            ) {
                return Ok(None);
            }
            window.set_fullscreen(None);
            window.set_maximized(false);
            window.set_minimized(false);
            window.set_min_inner_size(None::<LogicalSize<u32>>);
            window.set_decorations(false);
            window.set_skip_taskbar(true);
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            let extended = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(
                hwnd,
                GWL_STYLE,
                (style & !((WS_OVERLAPPEDWINDOW | WS_CHILD) as isize)) | (WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS) as isize,
            );
            SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                (extended & !(WS_EX_APPWINDOW as isize)) | WS_EX_TOOLWINDOW as isize,
            );
            let rect = viewport.rect;
            // Immediately below the normal host, never globally topmost. The
            // host reveals only its acknowledged world region via its own HRGN.
            if SetWindowPos(
                hwnd,
                host,
                rect.x,
                rect.y,
                rect.width as i32,
                rect.height as i32,
                SWP_FRAMECHANGED | SWP_NOACTIVATE,
            ) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(Some(host_rect))
    }

    fn host_rect(viewport: aurora_session::HostedViewport, host_pid: u32) -> std::io::Result<aurora_session::PhysicalRect> {
        let host = viewport.host_window as HWND;
        let mut rect = RECT::default();
        let mut origin = POINT::default();
        let mut actual_pid = 0;
        // SAFETY: read only the host's current physical desktop client area.
        let valid = unsafe {
            host_pid != 0
                && IsWindow(host) != 0
                && GetWindowThreadProcessId(host, &mut actual_pid) != 0
                && actual_pid == host_pid
                && GetClientRect(host, &mut rect) != 0
                && ClientToScreen(host, &mut origin) != 0
        };
        if !valid {
            return Err(std::io::Error::other("host window does not belong to the configured controller"));
        }
        let rect = aurora_session::PhysicalRect {
            x: origin.x,
            y: origin.y,
            width: u32::try_from(rect.right - rect.left).map_err(std::io::Error::other)?,
            height: u32::try_from(rect.bottom - rect.top).map_err(std::io::Error::other)?,
        };
        // A minimized controller may temporarily have an empty client. It is
        // geometry drift, not disappearance or evidence of GPU completion.
        Ok(rect)
    }

    pub(super) fn workspace_foreground(host: u64, worker: u64, host_pid: u32) -> bool {
        // SAFETY: read actual foreground and native owners only. Every allowed
        // tool window must belong to this exact controller PID and owner root.
        let foreground = unsafe { GetForegroundWindow() } as u64;
        super::workspace_family_foreground(foreground, host, worker, host_pid, |window| unsafe {
            let mut pid = 0;
            if GetWindowThreadProcessId(window as HWND, &mut pid) == 0 {
                return None;
            }
            Some((pid, GetWindow(window as HWND, GW_OWNER) as u64))
        })
    }

    pub(super) fn review_foreground(host: u64, worker: u64, host_pid: u32) -> bool {
        let visible_owner = |window: u64, expected: u32| {
            let mut pid = 0;
            // SAFETY: query only live HWND properties; no foreign mutation.
            unsafe {
                window != 0
                    && expected != 0
                    && IsWindow(window as HWND) != 0
                    && IsWindowVisible(window as HWND) != 0
                    && IsIconic(window as HWND) == 0
                    && GetWindowThreadProcessId(window as HWND, &mut pid) != 0
                    && pid == expected
            }
        };
        if !visible_owner(host, host_pid) || !visible_owner(worker, std::process::id()) {
            return false;
        }
        // SAFETY: actual foreground, with visibility checked before trusting
        // the existing bounded exact-PID/owned-window family relationship.
        let foreground = unsafe { GetForegroundWindow() } as u64;
        if foreground == 0 || unsafe { IsWindowVisible(foreground as HWND) == 0 || IsIconic(foreground as HWND) != 0 } {
            return false;
        }
        super::workspace_family_foreground(foreground, host, worker, host_pid, |window| unsafe {
            let mut pid = 0;
            if GetWindowThreadProcessId(window as HWND, &mut pid) == 0 {
                return None;
            }
            Some((pid, GetWindow(window as HWND, GW_OWNER) as u64))
        })
    }

    pub(super) fn stay_below_host(window: &Window, viewport: aurora_session::HostedViewport) -> std::io::Result<()> {
        let hwnd = super::native_window(window) as HWND;
        let host = viewport.host_window as HWND;
        // SAFETY: query the host's Z-order, then change only this process's
        // own fitted window. Owned host tools stay above their owner/worker;
        // no foreign style, global topmost flag, focus or parenting changes.
        unsafe {
            if GetWindow(host, GW_HWNDNEXT) != hwnd && SetWindowPos(hwnd, host, 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE) == 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(())
    }

    pub(super) fn state(
        window: &Window,
        viewport: aurora_session::HostedViewport,
        host_pid: u32,
        host_before: Option<aurora_session::PhysicalRect>,
    ) -> anyhow::Result<super::NativeState> {
        let hwnd = super::native_window(window) as HWND;
        let host_now = host_rect(viewport, host_pid)?;
        let mut rect = RECT::default();
        // SAFETY: query the configured host and this viewer's native rectangle.
        anyhow::ensure!(
            unsafe { GetWindowRect(hwnd, &mut rect) != 0 },
            "could not read native worker rectangle"
        );
        let worker_now = aurora_session::PhysicalRect {
            x: rect.left,
            y: rect.top,
            width: u32::try_from(rect.right - rect.left)?,
            height: u32::try_from(rect.bottom - rect.top)?,
        };
        let size = window.inner_size();
        Ok(super::native_state(
            host_before,
            host_now,
            worker_now,
            viewport,
            [size.width, size.height],
        ))
    }

    pub(super) fn restore(window: &Window, saved: Saved) {
        let hwnd = super::native_window(window) as HWND;
        if hwnd.is_null() {
            return;
        }
        // save() runs after standby hides the window. Its WS_VISIBLE bit is
        // not the desired standalone visibility, and winit caches that state.
        // Force a real false -> true transition instead of a cached no-op.
        window.set_visible(false);
        window.set_decorations(saved.style & WS_CAPTION as isize != 0);
        window.set_min_inner_size(Some(LogicalSize::new(800, 560)));
        // SAFETY: restore only this process's own saved styles and geometry.
        unsafe {
            SetWindowLongPtrW(hwnd, GWL_STYLE, saved.style & !(WS_VISIBLE as isize));
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, saved.extended);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                saved.rect.left,
                saved.rect.top,
                saved.rect.right - saved.rect.left,
                saved.rect.bottom - saved.rect.top,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
        window.set_maximized(saved.maximized);
        window.set_fullscreen(saved.fullscreen);
        window.set_minimized(false);
        window.set_visible(true);
        window.set_skip_taskbar(false);
        window.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard_and_command() -> (WorkerGuard, aurora_session::Envelope) {
        let identity = aurora_session::WorkerIdentity {
            installation: aurora_session::InstallationId::from_bytes([1; 32]).unwrap(),
            host: aurora_session::HostId::from_bytes([2; 16]).unwrap(),
            worker: aurora_session::WorkerId::from_bytes([3; 16]).unwrap(),
        };
        let mut guard = WorkerGuard::new(identity, 1, 0).unwrap();
        guard
            .update_session(Some(aurora_session::SessionIdentity {
                session: aurora_session::SessionId::from_bytes([4; 16]).unwrap(),
                account: AccountKey::from_bytes([5; 32]).unwrap(),
                grid: GridKey::from_bytes([6; 32]).unwrap(),
                generation: 1,
            }))
            .unwrap();
        let envelope = aurora_session::Envelope {
            version: aurora_session::PROTOCOL_VERSION,
            binding: guard.binding(),
            lease_epoch: 1,
            sequence: 1,
            reply_to: 0,
            message: Message::Command(Command::Prepare {
                activation: 1,
                viewport: None,
            }),
        };
        (guard, envelope)
    }

    #[test]
    fn a_newer_revoke_supersedes_a_batched_prepare_or_grant_before_native_effects() {
        for grant_first in [false, true] {
            let (mut guard, mut command) = guard_and_command();
            let mut effects = guard.on_command(&command, 1).unwrap().effects;
            if grant_first {
                let boundary = aurora_session::PrepareBoundary {
                    device_epoch: 1,
                    surface_epoch: 1,
                    after_frame: 0,
                };
                guard.prepare_boundary(1, boundary).unwrap();
                guard
                    .frame_ready(
                        1,
                        aurora_session::FrameProof {
                            device_epoch: 1,
                            surface_epoch: 1,
                            frame: 1,
                        },
                    )
                    .unwrap();
                command.sequence += 1;
                command.message = Message::Command(Command::Grant { activation: 1 });
                effects = guard.on_command(&command, 2).unwrap().effects;
                assert!(effects.grant_input);
            } else {
                assert!(effects.prepare);
            }
            command.sequence += 1;
            command.message = Message::Command(Command::Revoke { activation: 2 });
            merge_effects(&mut effects, guard.on_command(&command, 3).unwrap().effects);
            let effects = final_display_effects(effects, guard.mode());
            assert!(effects.release_input && effects.quiesce);
            assert!(
                !effects.prepare && !effects.grant_input,
                "superseded commands cannot fit/show/focus or emit Granted"
            );
        }
    }

    #[test]
    fn standby_and_fence_waits_never_produce_scene_or_ui_frames() {
        let boundary = aurora_session::PrepareBoundary {
            device_epoch: 1,
            surface_epoch: 2,
            after_frame: 7,
        };
        for gate in [
            DisplayGate::Draining {
                ticket: 1,
                reply_to: 4,
                through: 7,
            },
            DisplayGate::Standby,
            DisplayGate::Fitting { reply_to: 5 },
            DisplayGate::Settling { reply_to: 5, stable: 0 },
            DisplayGate::FrameFence {
                ticket: 2,
                reply_to: 5,
                boundary,
            },
            DisplayGate::Prepared,
            DisplayGate::GeometryPending,
        ] {
            assert!(!gate.allows_input());
            assert!(!gate.allows_submission());
            assert!(!gate.allows_world());
        }
        assert!(DisplayGate::Login.allows_submission());
        assert!(!DisplayGate::Login.allows_world(), "manual login cannot warm up a full world");
        assert!(DisplayGate::Preparing { reply_to: 5, boundary }.allows_world());
        assert!(DisplayGate::Active.allows_world());
        assert!(DisplayGate::Active.allows_input());
    }

    #[test]
    fn host_drag_suspends_exact_worker_placement_without_accepting_native_worker_changes() {
        let host = aurora_session::PhysicalRect {
            x: -1900,
            y: 80,
            width: 1200,
            height: 900,
        };
        let viewport = aurora_session::HostedViewport {
            host_window: 17,
            rect: aurora_session::PhysicalRect {
                x: -1800,
                y: 180,
                width: 1000,
                height: 700,
            },
        };
        let size = [viewport.rect.width, viewport.rect.height];
        assert_eq!(native_state(Some(host), host, viewport.rect, viewport, size), NativeState::Ready);
        let moved = aurora_session::PhysicalRect { x: host.x + 10, ..host };
        assert!(contains_rect(
            moved.x,
            moved.y,
            moved.width as i32,
            moved.height as i32,
            viewport.rect
        ));
        assert_eq!(
            native_state(Some(host), moved, viewport.rect, viewport, size),
            NativeState::HostChanged,
            "a small move invalidates placement even while the old rectangle still fits"
        );
        let resized_worker = aurora_session::PhysicalRect {
            width: viewport.rect.width + 1,
            ..viewport.rect
        };
        assert_eq!(
            native_state(Some(host), moved, resized_worker, viewport, size),
            NativeState::WorkerChanged,
            "host movement cannot excuse an unexpected worker resize"
        );
        assert_eq!(
            native_state(Some(host), host, viewport.rect, viewport, [size[0] - 1, size[1]]),
            NativeState::WorkerChanged
        );
        // The guard may still retain the old Active mode until Revoke arrives.
        // Its native display gate nevertheless withholds all three authorities.
        let gate = DisplayGate::GeometryPending;
        assert!(!gate.allows_input() && !gate.allows_world() && !gate.allows_submission());
    }

    #[test]
    fn renderer_indices_prove_only_new_presented_frames_on_the_prepared_surface() {
        let boundary = aurora_session::PrepareBoundary {
            device_epoch: 3,
            surface_epoch: 4,
            after_frame: 7,
        };
        let completion = aurora_render::SessionCompletion {
            ticket: 5,
            surface_epoch: 4,
            frame: Some(7),
        };
        let proof = prepared_proof(boundary, completion).expect("first accepted new packet");
        assert_eq!((proof.device_epoch, proof.surface_epoch, proof.frame), (3, 4, 8));
        for invalid in [
            aurora_render::SessionCompletion {
                frame: Some(6),
                ..completion
            },
            aurora_render::SessionCompletion {
                surface_epoch: 3,
                ..completion
            },
            aurora_render::SessionCompletion { frame: None, ..completion },
        ] {
            assert!(prepared_proof(boundary, invalid).is_err());
        }
    }

    #[test]
    fn stale_viewports_cannot_escape_a_moved_or_resized_host_client() {
        let rect = aurora_session::PhysicalRect {
            x: -1800,
            y: -200,
            width: 640,
            height: 360,
        };
        assert!(contains_rect(-1920, -300, 1920, 1080, rect));
        assert!(!contains_rect(-1000, -300, 1000, 1080, rect));
        assert!(!contains_rect(-1920, -300, 400, 1080, rect));
        assert!(!contains_rect(-1920, -300, 0, 1080, rect));
    }

    #[test]
    fn native_source_authority_expires_on_local_receipt_age_even_without_a_frame_tick() {
        let received = Instant::now();
        assert!(!receipt_is_current(None, received));
        assert!(!receipt_is_current(Some(received + std::time::Duration::from_millis(1)), received));
        assert!(receipt_is_current(
            Some(received),
            received + std::time::Duration::from_millis(aurora_session::LEASE_MILLIS - 1)
        ));
        assert!(!receipt_is_current(
            Some(received),
            received + std::time::Duration::from_millis(aurora_session::LEASE_MILLIS)
        ));
    }

    #[test]
    fn render_priority_accepts_only_the_exact_host_owned_tool_family() {
        let relation = |window| match window {
            1 => Some((10, 0)),
            2 => Some((10, 1)),
            3 => Some((10, 2)),
            4 => Some((11, 1)),
            5 => Some((10, 0)),
            6 => Some((10, 6)),
            _ => None,
        };
        assert!(workspace_family_foreground(1, 1, 9, 10, relation));
        assert!(workspace_family_foreground(2, 1, 9, 10, relation));
        assert!(workspace_family_foreground(3, 1, 9, 10, relation));
        assert!(workspace_family_foreground(9, 1, 9, 10, relation));
        for window in [0, 4, 5, 6, 7] {
            assert!(!workspace_family_foreground(window, 1, 9, 10, relation));
        }
        assert!(
            !workspace_family_foreground(2, 1, 9, 12, relation),
            "the native host PID must match"
        );
        assert!(
            !workspace_family_foreground(100, 1, 9, 10, |window| Some((10, window + 1))),
            "owner walks are bounded"
        );
    }

    #[test]
    fn a_cold_renderer_does_not_consume_the_never_owned_startup_lease() {
        let process_start = Instant::now();
        let first_event = process_start + std::time::Duration::from_secs(60);
        let mut since = None;
        assert_eq!(session_clock(&mut since, first_event), 0);
        assert_eq!(
            session_clock(&mut since, first_event + std::time::Duration::from_millis(15_001)),
            15_001
        );
    }

    #[test]
    fn lease_effects_always_preserve_input_release_and_window_restoration() {
        let mut effects = WorkerEffects {
            quiesce: true,
            ..Default::default()
        };
        merge_effects(
            &mut effects,
            WorkerEffects {
                release_input: true,
                restore_window: true,
                ..Default::default()
            },
        );
        assert!(effects.quiesce && effects.release_input && effects.restore_window);
    }

    #[test]
    fn native_review_strict_authority_rejects_every_permissive_input_fallback_and_incomplete_grant() {
        let (guard, _) = guard_and_command();
        let source = aurora_session::chat::ChatSource::try_from((guard.binding(), 1)).unwrap();
        let valid = || ReviewAuthority {
            current: Some(source),
            activation: 7,
            managed: true,
            active: true,
            display_active: true,
            render_owned: true,
            input_owned: true,
            blocked: false,
            native_matches: true,
        };
        assert!(valid().accepts(source, 7));
        for case in 0..10 {
            let mut facts = valid();
            match case {
                0 => facts.current = None,         // stale lease/login source
                1 => facts.managed = false,        // detached or unsupported ordinary-input fallback
                2 => facts.active = false,         // Standby/Preparing
                3 => facts.display_active = false, // core Active before native grant/GeometryPending
                4 => facts.render_owned = false,
                5 => facts.input_owned = false,
                6 => facts.blocked = true,
                7 => facts.native_matches = false,
                8 => facts.activation = 8,
                9 => facts.activation = 0,
                _ => unreachable!(),
            }
            assert!(!facts.accepts(source, 7), "strict case {case}");
        }
        let mut changed = source;
        changed.session.generation += 1;
        assert!(!valid().accepts(changed, 7));
        changed = source;
        changed.lease_epoch += 1;
        assert!(!valid().accepts(changed, 7));
        assert!(!valid().accepts(source, 0));
    }
}
