//! V2 admission/completion on the actual WorkerBridge. This owns the boot's
//! ledger and original async reply reservations, never a second control guard.
use super::super::transport::control_v2::{ReceivedControl, ReplyReservation};
use super::*;
use aurora_session::control_v2::{
    ControlV2Command, ControlV2Event, ControlV2Message, ControlV2Reply, ControlV2Work, ShowViewerQueryResult, ShowViewerQueryState,
};
use aurora_session::show_viewer::{ShowViewerLedger, ShowViewerOutcome, ShowViewerReceipt, ShowViewerRequest};
use aurora_session::{Error, Reply};
use std::collections::VecDeque;

struct Head {
    command: ReceivedControl,
    reply: Option<ReplyReservation>,
}
pub(crate) struct ShowWork {
    request: ShowViewerRequest,
    received_at: Instant,
    reply: ReplyReservation,
    new: bool,
    stored: Option<ShowViewerReceipt>,
    query: bool,
}
impl ShowWork {
    pub fn request(&self) -> ShowViewerRequest {
        self.request
    }
    pub fn received_at(&self) -> Instant {
        self.received_at
    }
    pub fn lookup_only(&self) -> bool {
        self.query || self.stored.is_some()
    }
    pub fn mark_effect_started(&self) {
        self.reply.mark_effect_started();
    }
}
pub(super) struct Runtime {
    ledger: ShowViewerLedger,
    head: Option<Head>,
    replies: VecDeque<ReplyReservation>,
    show: Option<ShowWork>,
    strict_receipt: Option<Instant>,
    retired: bool,
    startup_attempted: bool,
    ready_at: Option<Instant>,
}
impl Runtime {
    pub(super) fn new(config: &WorkerConfig) -> Result<Self, Error> {
        Ok(Self {
            ledger: ShowViewerLedger::new(config.identity, config.lease_epoch)?,
            head: None,
            replies: VecDeque::with_capacity(16),
            show: None,
            strict_receipt: None,
            retired: false,
            startup_attempted: false,
            ready_at: None,
        })
    }
    fn cancel_waiting(&mut self) {
        for reply in self.replies.drain(..) {
            let ControlV2Message::Command(command) = &reply.original().message else {
                continue;
            };
            let event = ControlV2Event::ControlUnavailable { command: command.clone() };
            let _ = reply.complete(event);
        }
    }
    pub(super) fn cancel_source(&mut self) {
        self.strict_receipt = None;
        if let Some(work) = self.show.take() {
            self.cancel_show(work);
        }
        self.cancel_waiting();
    }
    pub(super) fn retire(&mut self, keep_detach: bool) {
        self.retired = true;
        self.strict_receipt = None;
        if let Some(work) = self.show.take() {
            self.cancel_show(work);
        }
        let abandoned: Vec<_> = self.ledger.pending_requests().collect();
        for request in abandoned {
            let _ = self.ledger.finish(request, ShowViewerOutcome::EffectUncertain);
        }
        self.head = None;
        let mut retained = VecDeque::with_capacity(1);
        for reply in self.replies.drain(..) {
            if keep_detach && matches!(reply.original().message, ControlV2Message::Command(ControlV2Command::Detach)) {
                retained.push_back(reply);
            }
            // All others drop into an immutable original ControlUnavailable;
            // that is never a fake Status/Detached/native success.
        }
        self.replies = retained;
    }
    fn cancel_show(&mut self, work: ShowWork) {
        if work.new {
            let outcome = if work.reply.effect_started() {
                ShowViewerOutcome::EffectUncertain
            } else {
                ShowViewerOutcome::Cancelled
            };
            if let Ok(receipt) = self.ledger.finish(work.request, outcome) {
                let _ = work.reply.complete(ControlV2Event::ShowViewerReceipt(receipt));
            }
        }
        // A denied duplicate/query does not overwrite/disclose prior outcome.
    }
    pub(super) fn unavailable(&mut self, reply_to: u64) {
        if let Some(index) = self.replies.iter().position(|reply| reply.original().sequence == reply_to) {
            // Drop publishes only the captured command's ControlUnavailable.
            self.replies.remove(index);
        }
    }
    pub(super) fn reply(&mut self, reply_to: u64, event: Event) -> bool {
        let Some(index) = self.replies.iter().position(|reply| reply.original().sequence == reply_to) else {
            return false;
        };
        let Some(reply) = self.replies.remove(index) else { return false };
        reply
            .complete(ControlV2Reply::from_control(Reply { reply_to, event }).event)
            .is_ok()
    }
    fn poll(&mut self, bridge: &mut WorkerBridge) -> Result<WorkerEffects, Error> {
        if self.retired || self.show.is_some() || self.ready_at.is_none() {
            return Ok(WorkerEffects::default());
        }
        if self.head.is_none() {
            self.head = bridge
                .io
                .v2()
                .and_then(|io| io.next_command())
                .map(|command| Head { command, reply: None });
        }
        let Some(head) = &mut self.head else {
            return Ok(WorkerEffects::default());
        };
        if head.reply.is_none() {
            match bridge.io.v2().ok_or(Error::Unavailable)?.reserve(&head.command.envelope) {
                Ok(reply) => head.reply = Some(reply),
                Err(Error::Capacity) => return Ok(WorkerEffects::default()),
                Err(error) => return Err(error),
            }
        }
        if !head.command.integrity() {
            return Err(Error::Invalid);
        }
        if self.ready_at.is_some_and(|ready| head.command.predates_ready(ready)) {
            // Transport accepted old bytes before genuine App readiness, not
            // native work. Preserve identity/highwater without granting them a
            // fresh timestamp or any Guard lease/sequence authority.
            if let ControlV2Message::Command(ControlV2Command::ShowViewer(request)) = &head.command.envelope.message {
                if let Ok(None) = self.ledger.reserve(*request) {
                    let _ = self.ledger.finish(*request, ShowViewerOutcome::StaleTarget);
                }
            }
            self.head = None;
            return Ok(WorkerEffects::default());
        }
        let ControlV2Message::Command(command) = &head.command.envelope.message else {
            return Err(Error::Invalid);
        };
        let now = Instant::now();
        let origin = bridge.io.v2().ok_or(Error::Unavailable)?.origin;
        let received_ms = head.command.received_ms(origin, now)?;
        let now_ms: u64 = now
            .checked_duration_since(origin)
            .ok_or(Error::Stale)?
            .as_millis()
            .try_into()
            .map_err(|_| Error::Sequence)?;
        // Ledger reservation precedes any effect. If all retained entries are
        // pending, keep this same head/reply/stamp rather than reminting work.
        let stored = if let ControlV2Command::ShowViewer(request) = command {
            match self.ledger.reserve(*request) {
                Ok(stored) => stored,
                Err(Error::Capacity) => return Ok(WorkerEffects::default()),
                Err(_) => {
                    self.head = None;
                    return Ok(WorkerEffects::default());
                }
            }
        } else {
            None
        };
        let Some(mut head) = self.head.take() else {
            return Err(Error::Invalid);
        };
        let Some(reply) = head.reply.take() else {
            return Err(Error::Invalid);
        };
        let envelope = &head.command.envelope;
        let normal = envelope.existing_control()?;
        if let Some(normal) = &normal {
            let Message::Command(command) = &normal.message else {
                return Err(Error::Invalid);
            };
            if command.session_bound()
                && (!bridge.supported || !bridge.admitted || (bridge.status.blocked && !matches!(command, Command::Revoke { .. })))
            {
                if envelope.binding == bridge.guard.binding() {
                    let event = ControlV2Reply::from_control(Reply {
                        reply_to: envelope.sequence,
                        event: Event::Rejected {
                            activation: command.activation(),
                            reason: if !bridge.supported {
                                RejectReason::Unsupported
                            } else {
                                RejectReason::Blocked
                            },
                        },
                    })
                    .event;
                    reply.complete(event)?;
                }
                return Ok(WorkerEffects::default());
            }
        }
        let transition = match bridge.guard.on_control_v2(envelope, received_ms, now_ms) {
            Ok(transition) => transition,
            Err(_) => {
                if let ControlV2Message::Command(ControlV2Command::ShowViewer(request)) = &envelope.message
                    && stored.is_none()
                {
                    let receipt = self.ledger.finish(*request, ShowViewerOutcome::StaleTarget)?;
                    reply.complete(ControlV2Event::ShowViewerReceipt(receipt))?;
                }
                return Ok(WorkerEffects::default());
            }
        };
        bridge.last_receipt = Some(head.command.received_at);
        if let Some(normal) = normal {
            let Message::Command(command) = normal.message else {
                return Err(Error::Invalid);
            };
            if !matches!(command, Command::Heartbeat { .. }) {
                ordinary::invalidate();
            }
            bridge.preview_accepted_command(&command);
            if transition.effects.quiesce || transition.effects.prepare {
                self.cancel_waiting();
                bridge.request_sequence = envelope.sequence;
            }
            if let Some(completion) = transition.reply {
                if matches!(completion.event, ControlV2Event::Granted { .. } | ControlV2Event::Detached) {
                    bridge.request_sequence = envelope.sequence;
                    self.replies.push_back(reply);
                } else {
                    reply.complete(completion.event)?;
                }
            } else if matches!(command, Command::Heartbeat { .. } | Command::SetViewport { .. }) {
                if envelope.binding == bridge.guard.binding() {
                    let mut status = bridge.status.clone();
                    status.last_frame = bridge.last_frame;
                    status.blocked |= !bridge.supported || (!bridge.admitted && status.phase == aurora_session::Phase::Online);
                    reply.complete(ControlV2Event::Status { status, ordinary: None })?;
                }
                // An unbound heartbeat after login cannot claim Online Status
                // with its original None header. It retires as unavailable;
                // separate latest Status reports the actual current source.
            } else {
                self.replies.push_back(reply);
            }
            return Ok(transition.effects);
        }
        self.strict_receipt = Some(head.command.received_at);
        match transition.work {
            Some(ControlV2Work::ShowViewer { request, outcome }) => {
                if stored.is_none() && outcome.is_some() {
                    let receipt = self.ledger.finish(request, outcome.ok_or(Error::Invalid)?)?;
                    reply.complete(ControlV2Event::ShowViewerReceipt(receipt))?;
                } else {
                    self.show = Some(ShowWork {
                        request,
                        received_at: head.command.received_at,
                        reply,
                        new: stored.is_none(),
                        stored,
                        query: false,
                    });
                }
            }
            Some(ControlV2Work::QueryShowViewer(request)) => {
                self.show = Some(ShowWork {
                    request,
                    received_at: head.command.received_at,
                    reply,
                    new: false,
                    stored: None,
                    query: true,
                })
            }
            None => {
                if let ControlV2Message::Command(ControlV2Command::ShowViewer(request)) = &envelope.message
                    && stored.is_none()
                {
                    let receipt = self.ledger.finish(*request, ShowViewerOutcome::Expired)?;
                    reply.complete(ControlV2Event::ShowViewerReceipt(receipt))?;
                }
            }
        }
        Ok(transition.effects)
    }
    fn finish(&mut self, work: ShowWork, result: crate::app::host_show_viewer::ShowResult) {
        if result.request() != work.request {
            self.cancel_show(work);
            return;
        }
        let event = if work.query {
            let result = if result.lookup_allowed() {
                self.ledger
                    .query(work.request)
                    .ok()
                    .and_then(|lookup| ShowViewerQueryResult::from_lookup(work.request, lookup).ok())
            } else {
                None
            }
            .unwrap_or(ShowViewerQueryResult {
                request: work.request,
                result: ShowViewerQueryState::Rejected,
            });
            ControlV2Event::ShowViewerQuery(result)
        } else if let Some(receipt) = work.stored {
            if !result.lookup_allowed() {
                return;
            }
            ControlV2Event::ShowViewerReceipt(receipt)
        } else if work.new {
            let Ok(receipt) = self.ledger.finish(work.request, result.outcome()) else {
                return;
            };
            ControlV2Event::ShowViewerReceipt(receipt)
        } else {
            return;
        };
        let _ = work.reply.complete(event);
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.retire(false);
    }
}

impl WorkerBridge {
    pub(crate) fn app_ready_control_v2(&mut self, ready: crate::app::host_show_viewer::AppReady) -> WorkerEffects {
        let Some(runtime) = &mut self.control_v2 else {
            return WorkerEffects::default();
        };
        if runtime.startup_attempted || runtime.retired {
            return WorkerEffects::default();
        }
        runtime.startup_attempted = true;
        let now = Instant::now();
        let clock = self
            .io
            .v2()
            .and_then(|io| now.checked_duration_since(io.origin))
            .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok());
        if !ready.current()
            || runtime.ledger.retained_count() != 0
            || runtime.head.is_some()
            || runtime.show.is_some()
            || !runtime.replies.is_empty()
        {
            return self.fail_closed();
        }
        let Some(clock) = clock else { return self.fail_closed() };
        if self
            .guard
            .initialize_startup_control_v2(aurora_session::control_v2::V2StartupClock::at_app_ready(clock))
            .is_err()
        {
            return self.fail_closed();
        }
        runtime.ready_at = Some(now);
        WorkerEffects::default()
    }
    pub(super) fn control_clock_started(&self) -> bool {
        self.control_v2.as_ref().is_none_or(|runtime| runtime.ready_at.is_some())
    }
    pub(crate) fn uses_control_v2(&self) -> bool {
        self.control_v2.is_some()
    }
    pub(crate) fn observe_ordinary(&mut self, observation: Option<crate::app::host_show_viewer::OrdinaryObservation>) {
        self.ordinary = observation;
    }
    fn ordinary_presentation(&self) -> bool {
        let ordinary = !self.presentation.hidden;
        #[cfg(windows)]
        let ordinary = ordinary && self.presentation.saved.is_none() && self.presentation.host_rect.is_none();
        ordinary
    }
    pub(super) fn ordinary_target(&self) -> Option<aurora_session::show_viewer::ShowViewerTarget> {
        if self.control_v2.as_ref()?.retired
            || self.detached
            || self.io.failed()
            || !self.guard.ordinary_standby()
            || !matches!(self.gate, DisplayGate::Login)
            || !self.ordinary_presentation()
        {
            return None;
        }
        Some(aurora_session::show_viewer::ShowViewerTarget {
            binding: self.guard.binding(),
            lease_epoch: self.config.lease_epoch,
            window: self.ordinary.as_ref()?.numbers()?,
        })
    }
    pub(crate) fn ordinary_authorized(
        &self,
        observation: &crate::app::host_show_viewer::OrdinaryObservation,
        window: &std::sync::Arc<winit::window::Window>,
        request: ShowViewerRequest,
        received_at: Instant,
        lookup: bool,
    ) -> bool {
        let now = Instant::now();
        let Some(runtime) = &self.control_v2 else { return false };
        let Some(age) = now.checked_duration_since(received_at) else {
            return false;
        };
        !runtime.retired
            && !self.detached
            && !self.io.failed()
            && self.config.identity == request.target.binding.worker
            && self.guard.binding() == request.target.binding
            && self.config.lease_epoch == request.target.lease_epoch
            && self.guard.ordinary_standby()
            && matches!(self.gate, DisplayGate::Login)
            && self.ordinary_presentation()
            && receipt_is_current(runtime.strict_receipt, now)
            && (lookup || age < std::time::Duration::from_millis(aurora_session::control_v2::SHOW_VIEWER_MILLIS))
            && self
                .ordinary
                .as_ref()
                .is_some_and(|owned| owned.same_owner(observation) && owned.current_window(window))
            && observation.current_window(window)
            && observation.numbers() == Some(request.target.window)
    }
    pub(crate) fn publish_ordinary(&self) {
        self.publish_status();
    }
    pub(super) fn cancel_control_source(&mut self) {
        if let Some(runtime) = &mut self.control_v2 {
            runtime.cancel_source();
        }
    }
    pub(super) fn poll_control_v2(&mut self) -> WorkerEffects {
        let Some(mut runtime) = self.control_v2.take() else {
            return WorkerEffects::default();
        };
        let result = runtime.poll(self);
        self.control_v2 = Some(runtime);
        match result {
            Ok(effects) => effects,
            Err(_) => self.fail_closed(),
        }
    }
    pub(crate) fn take_show_work(&mut self) -> Option<ShowWork> {
        self.control_v2.as_mut()?.show.take()
    }
    pub(crate) fn finish_show_work(&mut self, work: ShowWork, result: crate::app::host_show_viewer::ShowResult) {
        if let Some(runtime) = &mut self.control_v2 {
            runtime.finish(work, result);
        }
    }
    pub(super) fn retire_control_v2(&mut self, keep_detach: bool) {
        if let Some(runtime) = &mut self.control_v2 {
            runtime.retire(keep_detach);
        }
    }
    pub(super) fn reply_original(&mut self, binding: aurora_session::Binding, reply_to: u64, event: Event) -> bool {
        if let Some(runtime) = &mut self.control_v2 {
            runtime.reply(reply_to, event)
        } else {
            self.io.reply(binding, reply_to, event)
        }
    }
}
