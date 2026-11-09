//! One bounded ledger per actual worker. Native acceptance is distinct from
//! effect completion; process handles and private slot locks prove restart.
use super::*;
use aurora_mcp::{
    authority::{ActionExecution, ActionResult},
    wire::{BackgroundMode, HostAction},
};
use aurora_session::native::{NativeAction, NativeOutcome, NativeRequest};

const DEADLINE: Duration = Duration::from_secs(60);

pub(super) struct Operation {
    id: String,
    pub binding: Binding,
    slot: SlotId,
    action: HostAction,
    pub request: NativeRequest,
    expected_owner: Option<Binding>,
    began: Instant,
    sent: bool,
    committed: bool,
    rejected: bool,
    pub acknowledged: bool,
    restoring: bool,
    pub exited: bool,
    receipt: Option<NativeOutcome>,
    reopening: bool,
}

impl Broker {
    fn current_native_owner(&self) -> Result<Option<Binding>, ()> {
        match self.coordinator.active() {
            None => Ok(None),
            Some(worker) => self
                .children
                .iter()
                .find(|child| child.binding.worker.worker == worker && !child.detached && !child.exited)
                .map(|child| Some(child.binding))
                .ok_or(()),
        }
    }

    pub(super) fn begin_native_action(&mut self, action: &ActionExecution) -> ActionResult {
        let Some(source) = action.profiles.first() else {
            return ActionResult::RejectedNative;
        };
        let Some(binding) = source.binding else {
            return ActionResult::RejectedNative;
        };
        if self.native_actions.len() >= MAX_WORKERS || self.native_actions.iter().any(|op| op.binding.worker == binding.worker) {
            return ActionResult::RejectedNative;
        }
        let Some(child) = self
            .children
            .iter()
            .find(|child| child.binding == binding && !child.detached && !child.exited)
        else {
            return ActionResult::RejectedNative;
        };
        let Some(slot) = child.config.slot else {
            return ActionResult::Unavailable;
        };
        let Some(native) = child.fresh_native() else {
            return ActionResult::Unavailable;
        };
        let request = match action.action {
            HostAction::Logout | HostAction::Restart if native.capabilities.exit => NativeAction::RequestExit,
            HostAction::BackgroundMode { mode }
                if native.capabilities.background && self.coordinator.active() != Some(binding.worker.worker) =>
            {
                NativeAction::Background(match mode {
                    BackgroundMode::Warm => aurora_session::native::BackgroundMode::Warm,
                    BackgroundMode::Economy => aurora_session::native::BackgroundMode::Economy,
                })
            }
            _ => return ActionResult::Unavailable,
        };
        let Some(id) = self.next_native_request.checked_add(1) else {
            return ActionResult::Unavailable;
        };
        self.next_native_request = id;
        self.native_actions.push(Operation {
            id: action.request_id.clone(),
            binding,
            slot,
            action: action.action.clone(),
            request: NativeRequest { id, action: request },
            expected_owner: action.expected_owner,
            began: Instant::now(),
            sent: false,
            committed: false,
            rejected: false,
            acknowledged: false,
            restoring: false,
            exited: false,
            receipt: None,
            reopening: false,
        });
        ActionResult::Submitted
    }

    pub(super) fn manual_open_authorized(&self, capture: &PendingMcpOpen, gate: OpenGate) -> bool {
        let Some(authority) = &self.native_authority else {
            return false;
        };
        if !authority.native_continuation_allowed(&capture.id) || capture.began.elapsed() >= DEADLINE {
            return false;
        }
        let expected = if gate == OpenGate::Queued { capture.expected_owner } else { None };
        self.current_native_owner().is_ok_and(|owner| owner == expected)
    }

    fn cancel_native_preparation(&mut self, operation: &Operation) {
        if operation.restoring
            && self
                .children
                .iter()
                .any(|child| child.binding == operation.binding && child.restore_pending && !child.detached && !child.exited)
        {
            if let Ok(effect) = self.coordinator.finish_native_restore(operation.binding) {
                let _ = self.effects(vec![effect]);
            }
        } else if let Ok(effect) = self.coordinator.native_finalize(
            operation.binding,
            NativeRequest {
                id: operation.request.id,
                action: NativeAction::CancelExit,
            },
        ) {
            let _ = self.effects(vec![effect]);
        }
    }
    pub(super) fn cancel_native_preparations(&mut self) {
        let operations = std::mem::take(&mut self.native_actions);
        for operation in &operations {
            if operation.sent && !operation.committed && operation.request.action == NativeAction::RequestExit {
                self.cancel_native_preparation(operation);
            }
        }
        self.native_actions = operations;
    }

    pub(super) fn poll_native_actions(&mut self, now: u64) {
        let Some(authority) = self.native_authority.clone() else {
            return;
        };
        let operations = std::mem::take(&mut self.native_actions);
        let mut retained = Vec::with_capacity(MAX_WORKERS);
        for mut operation in operations {
            let mut terminal = None;
            if operation.rejected {
                terminal = Some(ActionResult::RejectedNative);
            } else if operation.began.elapsed() >= DEADLINE {
                terminal = Some(ActionResult::Uncertain);
            } else if operation.reopening {
                // No fresh account receives the old opt-in. Completion proves
                // only that the ordinary manual-login process actually spoke.
                if let Some(child) = self.children.iter().find(|child| {
                    child.config.slot == Some(operation.slot) && child.binding.worker != operation.binding.worker && !child.exited
                }) {
                    terminal = Some(
                        if child.config.expected_account.is_none()
                            && child.last_seen.is_some()
                            && child.status.as_ref().is_some_and(|status| status.phase == Phase::Login)
                        {
                            ActionResult::Completed
                        } else if child.binding.session.is_some() {
                            ActionResult::Uncertain
                        } else {
                            retained.push(operation);
                            continue;
                        },
                    );
                } else if !authority.native_continuation_allowed(&operation.id) {
                    self.opens
                        .retain(|open| open.mcp_open.as_ref().map(|capture| capture.id.as_str()) != Some(operation.id.as_str()));
                    terminal = Some(ActionResult::RejectedNative);
                }
            } else if !operation.sent {
                if !authority.action_still_authorized(&operation.id) {
                    terminal = Some(ActionResult::RejectedNative);
                } else if !self.coordinator.busy() {
                    let exact = self.children.iter().any(|child| {
                        child.binding == operation.binding
                            && !child.detached
                            && !child.exited
                            && child.last_seen.is_some_and(|seen| seen.elapsed() <= REACHABLE)
                            && child.fresh_native().is_some_and(|native| match operation.request.action {
                                NativeAction::RequestExit => native.capabilities.exit,
                                NativeAction::Background(_) => native.capabilities.background,
                                _ => false,
                            })
                    });
                    let owner_unchanged =
                        operation.action != HostAction::Restart || self.current_native_owner() == Ok(operation.expected_owner);
                    let sent = exact
                        && owner_unchanged
                        && authority.dispatch_native_request(&operation.id, || {
                            self.coordinator
                                .native_request(operation.binding.worker.worker, operation.request, now)
                                .ok()
                                .is_some_and(|effect| self.effects(vec![effect]).is_ok())
                        });
                    if sent {
                        operation.sent = true;
                    } else {
                        terminal = Some(ActionResult::RejectedNative);
                    }
                }
            } else if operation.request.action == NativeAction::RequestExit
                && !operation.committed
                && !authority.native_continuation_allowed(&operation.id)
            {
                terminal = Some(ActionResult::RejectedNative);
            } else {
                if let Some(child) = self.children.iter().find(|child| child.binding == operation.binding)
                    && let Some(receipt) = child
                        .fresh_native()
                        .and_then(|native| native.receipt)
                        .filter(|receipt| receipt.request == operation.request)
                {
                    operation.receipt = Some(receipt.outcome);
                }
                match operation.receipt {
                    Some(NativeOutcome::Rejected | NativeOutcome::Cancelled) => terminal = Some(ActionResult::RejectedNative),
                    Some(NativeOutcome::Uncertain) => terminal = Some(ActionResult::Uncertain),
                    Some(NativeOutcome::Applied)
                        if matches!(operation.request.action, NativeAction::Background(_)) && operation.acknowledged =>
                    {
                        terminal = Some(ActionResult::Completed)
                    }
                    Some(NativeOutcome::LogoutChosen | NativeOutcome::LoggedOut) if operation.action == HostAction::Restart => {
                        terminal = Some(ActionResult::RejectedNative)
                    }
                    Some(NativeOutcome::LoggedOut) if operation.action == HostAction::Logout => terminal = Some(ActionResult::Completed),
                    Some(NativeOutcome::ReadyForExit) if operation.restoring && !operation.committed => {
                        let ready = self
                            .children
                            .iter()
                            .find(|child| child.binding == operation.binding && child.detached && !child.exited)
                            .and_then(Child::fresh_native)
                            .is_some_and(|observation| {
                                observation.receipt.is_some_and(|receipt| {
                                    receipt.request == operation.request && receipt.outcome == NativeOutcome::ReadyForExit
                                })
                            });
                        if ready
                            && !authority.commit_native_exit(&operation.id, || {
                                self.coordinator
                                    .native_finalize(
                                        operation.binding,
                                        NativeRequest {
                                            id: operation.request.id,
                                            action: NativeAction::CommitExit,
                                        },
                                    )
                                    .ok()
                                    .is_some_and(|effect| self.effects(vec![effect]).is_ok())
                            })
                        {
                            terminal = Some(ActionResult::RejectedNative);
                        } else if ready {
                            operation.committed = true;
                        }
                    }
                    Some(NativeOutcome::Accepted)
                        if operation.acknowledged && !operation.restoring && operation.request.action == NativeAction::RequestExit =>
                    {
                        let fresh_capable = self
                            .children
                            .iter()
                            .find(|child| child.binding == operation.binding && !child.detached && !child.exited)
                            .and_then(Child::fresh_native)
                            .is_some_and(|observation| {
                                observation.capabilities.exit
                                    && observation.receipt.is_some_and(|receipt| {
                                        receipt.request == operation.request && receipt.outcome == NativeOutcome::Accepted
                                    })
                            });
                        if !fresh_capable || !authority.native_continuation_allowed(&operation.id) || self.coordinator.busy() {
                            terminal = Some(ActionResult::RejectedNative);
                        } else {
                            let effects = self.coordinator.restore_native_exit(
                                operation.binding,
                                NativeRequest {
                                    id: operation.request.id,
                                    action: NativeAction::RestoreExit,
                                },
                            );
                            if effects.is_ok_and(|effects| self.effects(effects).is_ok()) {
                                operation.restoring = true;
                            } else {
                                terminal = Some(ActionResult::Uncertain);
                            }
                        }
                    }
                    _ => {}
                }
                if operation.exited && terminal.is_none() {
                    if operation.receipt == Some(NativeOutcome::QuitChosen) {
                        if operation.action == HostAction::Logout {
                            terminal = Some(ActionResult::Completed);
                        } else if operation.action == HostAction::Restart {
                            if !authority.native_continuation_allowed(&operation.id)
                                || self.current_native_owner() != Ok(operation.owner_after_restore())
                                || self.coordinator.busy()
                                || self.choose_slot(Some(operation.slot)).is_err()
                            {
                                terminal = Some(ActionResult::RejectedNative);
                            } else {
                                self.opens.push_back(PendingOpen {
                                    slot: operation.slot,
                                    account: None,
                                    gate: OpenGate::Queued,
                                    reservation: None,
                                    mcp_open: Some(PendingMcpOpen {
                                        id: operation.id.clone(),
                                        expected_owner: operation.owner_after_restore(),
                                        began: operation.began,
                                    }),
                                });
                                operation.reopening = true;
                            }
                        }
                    } else {
                        terminal = Some(ActionResult::Uncertain);
                    }
                }
            }
            if let Some(result) = terminal {
                if operation.sent && !operation.committed && operation.request.action == NativeAction::RequestExit {
                    self.cancel_native_preparation(&operation);
                }
                self.opens
                    .retain(|open| open.mcp_open.as_ref().map(|capture| capture.id.as_str()) != Some(operation.id.as_str()));
                let _ = authority.observe_action_result(&operation.id, result);
            } else {
                retained.push(operation);
            }
        }
        self.native_actions.extend(retained);
    }
}
impl Operation {
    fn owner_after_restore(&self) -> Option<Binding> {
        self.expected_owner.filter(|owner| *owner != self.binding)
    }
    pub(super) fn rejected(&mut self, binding: Binding, request: NativeRequest) {
        if self.binding == binding && self.request.id == request.id {
            self.rejected = true;
        }
    }
    pub(super) fn observe(&mut self, child: &Child) {
        if child.binding == self.binding
            && let Some(receipt) = child
                .fresh_native()
                .and_then(|native| native.receipt)
                .filter(|receipt| receipt.request == self.request)
        {
            self.receipt = Some(receipt.outcome);
        }
    }
}
