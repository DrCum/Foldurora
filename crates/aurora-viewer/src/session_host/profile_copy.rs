//! Stopped-slot local preference transaction, separate from worker control/chat.
//! Legacy behavior reference: fssessionprofileseed.h and
//! tools/session-host/main.cpp::selectProfileCopy (pinned abbe3, LGPL2.1).
//! Only four explicit Aurora families are supported; the first-Open offer and
//! broader Legacy groups remain separate gaps. No source raw data reaches DTOs.
use super::slots::SlotId;
use crate::settings::profile_seed::{Families, Summary};
use aurora_session::{HostId, InstallationId};
use crossbeam_channel::{Receiver, Sender, bounded};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

mod io;
#[cfg(test)]
mod tests;

const CONFIRMATION: Duration = Duration::from_secs(30);
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);
static PHYSICAL_WORKER: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Instance(u64);
impl Instance {
    pub(crate) fn fresh() -> Option<Self> {
        NEXT_INSTANCE
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .ok()
            .map(Self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    MainSaved,
    Slot(SlotId),
}
impl Source {
    pub(crate) fn slot(self) -> Option<SlotId> {
        match self {
            Self::MainSaved => None,
            Self::Slot(slot) => Some(slot),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub instance: Instance,
    pub incarnation: u64,
    pub target: SlotId,
    pub source: Source,
    pub families: Families,
}
impl Selection {
    fn valid(self) -> bool {
        self.incarnation != 0 && self.source.slot() != Some(self.target)
    }
    fn includes(self, slot: SlotId) -> bool {
        self.target == slot || self.source.slot() == Some(slot)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Capture {
    pub installation: InstallationId,
    pub controller: HostId,
    pub manager: Instance,
    pub operation: u64,
    pub selection: Selection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Busy,
    Stopped,
    Invalid,
    Spawn,
    Io,
    Projection(crate::settings::profile_seed::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Applied,
    CancelledBeforeCommit,
    Expired,
    Changed,
    Refused(Failure),
    /// A worker/channel failed without a terminal receipt. Inspect again;
    /// never replay an Apply or assert that the original target was unwritten.
    Uncertain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Inspecting,
    Prepared(Vec<Summary>),
    Applying,
    Terminal(Outcome),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct State {
    pub capture: Capture,
    pub phase: Phase,
}

struct PhysicalPermit;
impl PhysicalPermit {
    fn acquire() -> Option<Self> {
        PHYSICAL_WORKER
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self)
    }
}
impl Drop for PhysicalPermit {
    fn drop(&mut self) {
        PHYSICAL_WORKER.store(false, Ordering::Release);
    }
}

struct Inspect {
    capture: Capture,
    cancelled: Arc<AtomicBool>,
}
enum Command {
    Inspect(Inspect),
    Apply(Capture),
}
#[derive(Debug)]
enum Reply {
    Prepared(Capture, Vec<Summary>),
    Terminal(Capture, Outcome),
}
struct Worker {
    sender: Sender<Command>,
    replies: Receiver<Reply>,
    // Joining never occurs on the controller/GUI. The physical permit is owned
    // by the actual thread, including blocked IO, confirmation and retirement.
    _thread: std::thread::JoinHandle<()>,
}
struct Active {
    capture: Capture,
    cancelled: Arc<AtomicBool>,
}

pub(super) struct Lane {
    instance: Option<Instance>,
    worker: Option<Worker>,
    active: Option<Active>,
    state: Option<State>,
}
impl Default for Lane {
    fn default() -> Self {
        Self {
            instance: Instance::fresh(),
            worker: None,
            active: None,
            state: None,
        }
    }
}
impl Lane {
    pub(super) fn state(&self) -> Option<&State> {
        self.state.as_ref()
    }
    pub(super) fn reserved(&self, slot: SlotId) -> bool {
        self.active.as_ref().is_some_and(|active| active.capture.selection.includes(slot))
    }
    fn worker(&mut self) -> Result<&Worker, Failure> {
        self.worker_with(|job| std::thread::Builder::new().name("profile-copy-io".into()).spawn(job))
    }

    fn worker_with(
        &mut self,
        spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> std::io::Result<std::thread::JoinHandle<()>>,
    ) -> Result<&Worker, Failure> {
        if self.worker.is_none() {
            // Acquire before allocating channels or spawning. A retiring prior
            // controller's thread continues to charge this same process slot.
            let permit = PhysicalPermit::acquire().ok_or(Failure::Busy)?;
            let (sender, commands) = bounded(1);
            let (replies, receiver) = bounded(1);
            let thread = spawn(Box::new(move || {
                let _permit = permit;
                worker_loop(commands, replies);
            }))
            .map_err(|_| Failure::Spawn)?;
            self.worker = Some(Worker {
                sender,
                replies: receiver,
                _thread: thread,
            });
        }
        self.worker.as_ref().ok_or(Failure::Stopped)
    }

    pub(super) fn inspect(
        &mut self,
        installation: InstallationId,
        controller: HostId,
        operation: u64,
        selection: Selection,
    ) -> Result<(), Failure> {
        if self.active.is_some() {
            return Err(Failure::Busy);
        }
        if !selection.valid() || operation == 0 {
            return Err(Failure::Invalid);
        }
        let capture = Capture {
            installation,
            controller,
            manager: self.instance.ok_or(Failure::Stopped)?,
            operation,
            selection,
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let sent = self.worker().and_then(|worker| {
            worker
                .sender
                .try_send(Command::Inspect(Inspect {
                    capture,
                    cancelled: cancelled.clone(),
                }))
                .map_err(|_| Failure::Stopped)
        });
        if let Err(failure) = sent {
            self.state = Some(State {
                capture,
                phase: Phase::Terminal(Outcome::Refused(failure)),
            });
            return Err(failure);
        }
        self.active = Some(Active { capture, cancelled });
        self.state = Some(State {
            capture,
            phase: Phase::Inspecting,
        });
        Ok(())
    }

    pub(super) fn inspect_admitted(
        &mut self,
        installation: InstallationId,
        controller: HostId,
        operation: u64,
        selection: Selection,
        stopped: bool,
    ) {
        if self.active.is_some() {
            return;
        }
        if stopped {
            let _ = self.inspect(installation, controller, operation, selection);
        } else if let Some(manager) = self.instance {
            self.state = Some(State {
                capture: Capture {
                    installation,
                    controller,
                    manager,
                    operation,
                    selection,
                },
                phase: Phase::Terminal(Outcome::Refused(Failure::Busy)),
            });
        }
    }

    pub(super) fn apply(&mut self, capture: Capture) -> Result<(), Failure> {
        let active = self.active.as_ref().ok_or(Failure::Invalid)?;
        if active.capture != capture
            || active.cancelled.load(Ordering::Acquire)
            || !self
                .state
                .as_ref()
                .is_some_and(|state| state.capture == capture && matches!(state.phase, Phase::Prepared(_)))
        {
            return Err(Failure::Invalid);
        }
        self.worker
            .as_ref()
            .ok_or(Failure::Stopped)?
            .sender
            .try_send(Command::Apply(capture))
            .map_err(|_| Failure::Busy)?;
        self.state = Some(State {
            capture,
            phase: Phase::Applying,
        });
        Ok(())
    }

    pub(super) fn cancel(&self, capture: Capture) {
        if let Some(active) = &self.active
            && active.capture == capture
        {
            active.cancelled.store(true, Ordering::Release);
        }
    }
    pub(super) fn cancel_request(&self, selection: Selection, operation: u64) {
        if let Some(active) = &self.active
            && active.capture.selection == selection
            && active.capture.operation == operation
        {
            self.cancel(active.capture);
        }
    }
    pub(super) fn stop(&self) {
        if let Some(active) = &self.active {
            active.cancelled.store(true, Ordering::Release);
        }
    }
    pub(super) fn poll(&mut self) {
        let Some(worker) = &self.worker else { return };
        match worker.replies.try_recv() {
            Ok(Reply::Prepared(capture, summary)) if self.active.as_ref().is_some_and(|active| active.capture == capture) => {
                self.state = Some(State {
                    capture,
                    phase: Phase::Prepared(summary),
                });
            }
            Ok(Reply::Terminal(capture, outcome)) if self.active.as_ref().is_some_and(|active| active.capture == capture) => {
                self.active = None;
                self.state = Some(State {
                    capture,
                    phase: Phase::Terminal(outcome),
                });
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                if let Some(active) = self.active.take() {
                    self.state = Some(State {
                        capture: active.capture,
                        phase: Phase::Terminal(Outcome::Uncertain),
                    });
                }
                // No replacement thread for this lane: a missing receipt must
                // not become implicit replay or logical capacity refund.
            }
            _ => {}
        }
    }
}
impl Drop for Lane {
    fn drop(&mut self) {
        self.stop();
    }
}

fn worker_loop(commands: Receiver<Command>, replies: Sender<Reply>) {
    while let Ok(command) = commands.recv() {
        let Command::Inspect(inspect) = command else { continue };
        let capture = inspect.capture;
        let outcome = match io::Prepared::inspect(&inspect) {
            Err(outcome) => outcome,
            Ok(prepared) => {
                // This original deadline is never refreshed by Apply, reset or
                // a queued message. All locks stay in prepared while waiting.
                let deadline = Instant::now() + CONFIRMATION;
                if replies.send(Reply::Prepared(capture, prepared.summary.clone())).is_err() {
                    return;
                }
                loop {
                    if inspect.cancelled.load(Ordering::Acquire) {
                        break Outcome::CancelledBeforeCommit;
                    }
                    if Instant::now() >= deadline {
                        break Outcome::Expired;
                    }
                    match commands.recv_timeout(Duration::from_millis(25).min(deadline.saturating_duration_since(Instant::now()))) {
                        Ok(Command::Apply(exact)) if exact == capture => break prepared.apply(&inspect, deadline),
                        Ok(_) => {}
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
        };
        // io::Prepared and its lock handles have really dropped before this
        // terminal makes the Broker release reservations/admit another copy.
        if replies.send(Reply::Terminal(capture, outcome)).is_err() {
            return;
        }
    }
}
