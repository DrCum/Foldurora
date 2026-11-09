//! Closed host mutations. Admission, native approval and execution are distinct.
use super::*;
use crate::wire::{HostAction, HostTarget, PermissionClass};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionLevels {
    pub levels: [AuthorityLevel; 8],
}
impl Default for PermissionLevels {
    fn default() -> Self {
        Self {
            levels: [AuthorityLevel::Never; 8],
        }
    }
}
impl PermissionLevels {
    pub fn get(self, class: PermissionClass) -> AuthorityLevel {
        self.levels[class.index()]
    }
    pub fn set(&mut self, class: PermissionClass, level: AuthorityLevel) {
        self.levels[class.index()] = level;
    }
}

#[derive(Clone, Debug)]
pub struct ProfileAccess {
    pub ticket: ProfileTicket,
    pub label: MetadataLabel,
    pub permissions: Option<PermissionLevels>,
}

#[derive(Clone, Debug)]
pub struct ReadyAction {
    pub expected_owner: Option<Binding>,
    pub request_id: String,
    pub action: HostAction,
    pub profiles: Vec<ProfileTicket>,
}
pub type ActionExecution = ReadyAction;

/// Actual broker ownership, independent of GUI selection and profile labels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActiveOwner {
    None,
    Bound(Binding),
    #[default]
    Unavailable,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostOwnership {
    pub active: ActiveOwner,
    pub busy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorProjection {
    pub monitor: u8,
    pub source: ProfileTicket,
    pub incarnation: u64,
    pub visible: bool,
    pub enabled: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MonitorSnapshot {
    pub eligible: Vec<ProfileTicket>,
    pub monitors: Vec<MonitorProjection>,
    pub playing: bool,
    pub interval_seconds: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionResult {
    Submitted,
    Completed,
    RejectedNative,
    Unavailable,
    Uncertain,
}

pub(super) struct ActionOperation {
    pub(super) client: ClientId,
    request: (Option<HostTarget>, HostAction),
    profiles: Vec<ProfileTicket>,
    policy_revision: u64,
    monitor_revision: Option<u64>,
    owner: Option<HostOwnership>,
    created: Instant,
    finished: Option<Instant>,
    outcome: ActionOutcome,
    native_sent: bool,
    native_committed: bool,
    continuation_cancelled: bool,
}
#[derive(Clone, Copy)]
enum ActionOutcome {
    Pending,
    Ready,
    Submitted,
    NativePending,
    Completed,
    Failed(&'static str),
}

#[derive(Default)]
pub(super) struct ActionState {
    permissions: PermissionLevels,
    pub(super) grants: BTreeMap<ProfileId, (ProfileTicket, PermissionLevels)>,
    pub(super) operations: BTreeMap<String, ActionOperation>,
    supported: Vec<HostAction>,
    monitors: Option<(u64, Instant, MonitorSnapshot)>,
    next_monitor_revision: u64,
    ownership: Option<(HostOwnership, Instant)>,
}

impl Authority {
    pub fn publish_ownership(&self, ownership: HostOwnership) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        if let ActiveOwner::Bound(binding) = ownership.active
            && (binding.validate().is_err()
                || binding.session.is_none()
                || binding.worker.installation != state.installation
                || binding.worker.host != state.host)
        {
            return Err(AuthorityError::Invalid);
        }
        state.actions.ownership = Some((ownership, Instant::now()));
        Ok(())
    }

    pub fn publish_monitors(&self, snapshot: MonitorSnapshot) -> Result<u64, AuthorityError> {
        let mut state = self.lock()?;
        if snapshot.monitors.len() > 5 || snapshot.eligible.len() > 5 || !(2..=3600).contains(&snapshot.interval_seconds) {
            return Err(AuthorityError::Invalid);
        }
        let mut eligible = std::collections::BTreeSet::new();
        if snapshot.eligible.iter().any(|ticket| {
            !eligible.insert(ticket.profile) || !state.current(*ticket) || ticket.binding.is_none_or(|binding| binding.session.is_none())
        }) {
            return Err(AuthorityError::StaleSource);
        }
        let mut seen = std::collections::BTreeSet::new();
        if snapshot.monitors.iter().any(|monitor| {
            !(1..=5).contains(&monitor.monitor)
                || !seen.insert(monitor.monitor)
                || !state.current(monitor.source)
                || monitor.source.binding.is_none_or(|binding| binding.session.is_none())
        }) {
            return Err(AuthorityError::StaleSource);
        }
        let revision = match &state.actions.monitors {
            Some((revision, _, old)) if *old == snapshot => *revision,
            _ => {
                state.actions.next_monitor_revision = state
                    .actions
                    .next_monitor_revision
                    .checked_add(1)
                    .ok_or(AuthorityError::Unavailable)?;
                state.actions.next_monitor_revision
            }
        };
        state.actions.monitors = Some((revision, Instant::now(), snapshot));
        Ok(revision)
    }

    /// Trusted host preferences only. No request may change these settings.
    pub fn permission_levels(&self) -> Result<PermissionLevels, AuthorityError> {
        Ok(self.lock()?.actions.permissions)
    }

    pub fn configure_permissions(&self, enabled: bool, permissions: PermissionLevels) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        if state.policy.enabled != enabled || state.actions.permissions != permissions {
            state.policy = HostPolicy {
                enabled,
                metadata: permissions.get(PermissionClass::Metadata),
                actions: AuthorityLevel::Never,
            };
            state.actions.permissions = permissions;
            state.bump_policy()?;
            state.invalidate_operations("access_revoked", Instant::now());
            if !enabled {
                state.profiles.values_mut().for_each(|profile| profile.grant = None);
                state.actions.grants.clear();
                state.clients.values_mut().for_each(|client| client.refs.clear());
            }
        }
        Ok(())
    }

    pub fn grant_permissions(&self, ticket: ProfileTicket, permissions: PermissionLevels) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        let now = Instant::now();
        if !state.fresh(now) || !state.current(ticket) {
            return Err(AuthorityError::StaleSource);
        }
        if state.actions.grants.get(&ticket.profile) != Some(&(ticket, permissions)) {
            state.cancel_native(ticket.profile);
            state.revoke_profile(ticket.profile, now);
            state.profiles.get_mut(&ticket.profile).ok_or(AuthorityError::StaleSource)?.grant = Some(ProfileGrant {
                ticket,
                metadata: permissions.get(PermissionClass::Metadata),
                actions: AuthorityLevel::Never,
            });
            state.actions.grants.insert(ticket.profile, (ticket, permissions));
        }
        Ok(())
    }

    pub fn profile_access(&self) -> Result<Vec<ProfileAccess>, AuthorityError> {
        let state = self.lock()?;
        if !state.fresh(Instant::now()) {
            return Ok(Vec::new());
        }
        Ok(state
            .profiles
            .values()
            .filter(|profile| state.current(profile.ticket()))
            .map(|profile| ProfileAccess {
                ticket: profile.ticket(),
                label: profile.projection.label.clone(),
                permissions: state
                    .actions
                    .grants
                    .get(&profile.projection.id)
                    .filter(|(ticket, _)| *ticket == profile.ticket())
                    .map(|(_, levels)| *levels),
            })
            .collect())
    }

    /// Advertise only operations that have a genuine native adapter. Parameters
    /// are validated separately; support is compared by closed enum variant.
    pub fn support_actions(&self, actions: Vec<HostAction>) -> Result<(), AuthorityError> {
        if actions.len() > 16 {
            return Err(AuthorityError::Capacity);
        }
        let mut state = self.lock()?;
        state.actions.supported = actions;
        state.action_invalidate(None, "capability_changed", Instant::now());
        Ok(())
    }

    pub fn ready_actions(&self) -> Result<Vec<ReadyAction>, AuthorityError> {
        let mut state = self.lock()?;
        let now = Instant::now();
        state.prune(now);
        Ok(state
            .actions
            .operations
            .iter()
            .filter(|(_, op)| matches!(op.outcome, ActionOutcome::Ready) && state.action_valid(op, now))
            .map(|(id, op)| ReadyAction {
                expected_owner: match op.owner.map(|owner| owner.active) {
                    Some(ActiveOwner::Bound(binding)) => Some(binding),
                    _ => None,
                },
                request_id: id.clone(),
                action: op.request.1.clone(),
                profiles: op.profiles.clone(),
            })
            .collect())
    }

    /// Execute on the native owning thread, with final source/policy validation
    /// under the same short lock. The closure must only perform a bounded native
    /// admission, never wait for window/GPU/network completion or call Authority.
    pub fn execute_action(&self, request_id: &str, execute: impl FnOnce(&ActionExecution) -> ActionResult) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        let now = Instant::now();
        state.prune(now);
        let op = state.actions.operations.get(request_id).ok_or(AuthorityError::UnknownOperation)?;
        if !matches!(op.outcome, ActionOutcome::Ready) {
            return Err(AuthorityError::FinalOperation);
        }
        if !state.action_valid(op, now) {
            return Err(AuthorityError::StaleSource);
        }
        let execution = ActionExecution {
            expected_owner: match op.owner.map(|owner| owner.active) {
                Some(ActiveOwner::Bound(binding)) => Some(binding),
                _ => None,
            },
            request_id: request_id.into(),
            action: op.request.1.clone(),
            profiles: op.profiles.clone(),
        };
        // Mark before invoking native code so reentrant/replayed work cannot run.
        state
            .actions
            .operations
            .get_mut(request_id)
            .ok_or(AuthorityError::UnknownOperation)?
            .outcome = ActionOutcome::Submitted;
        let result = execute(&execution);
        let op = state
            .actions
            .operations
            .get_mut(request_id)
            .ok_or(AuthorityError::UnknownOperation)?;
        op.outcome = match result {
            ActionResult::Submitted => ActionOutcome::Submitted,
            ActionResult::Completed => ActionOutcome::Completed,
            ActionResult::RejectedNative => ActionOutcome::Failed("rejected_native"),
            ActionResult::Unavailable => ActionOutcome::Failed("native_unavailable"),
            ActionResult::Uncertain => ActionOutcome::Failed("uncertain"),
        };
        if result != ActionResult::Submitted {
            op.finished = Some(now);
        }
        Ok(())
    }

    /// GUI-covered presentation admission still waits in a bounded broker queue.
    /// Reconstruct the ORIGINAL capture and revalidate it at actual native
    /// dispatch. A duplicate queued intent cannot submit it twice.
    pub fn dispatch_submitted_action(
        &self,
        request_id: &str,
        execute: impl FnOnce(&ActionExecution) -> ActionResult,
    ) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        let now = Instant::now();
        let op = state.actions.operations.get(request_id).ok_or(AuthorityError::UnknownOperation)?;
        if !matches!(op.outcome, ActionOutcome::Submitted) {
            return Err(AuthorityError::FinalOperation);
        }
        if !state.action_valid(op, now) {
            return Err(AuthorityError::StaleSource);
        }
        let execution = ActionExecution {
            expected_owner: match op.owner.map(|owner| owner.active) {
                Some(ActiveOwner::Bound(binding)) => Some(binding),
                _ => None,
            },
            request_id: request_id.into(),
            action: op.request.1.clone(),
            profiles: op.profiles.clone(),
        };
        state
            .actions
            .operations
            .get_mut(request_id)
            .ok_or(AuthorityError::UnknownOperation)?
            .outcome = ActionOutcome::NativePending;
        if execution.action == HostAction::OpenLogin {
            state.actions.operations.get_mut(request_id).expect("locked operation").native_sent = true;
        }
        let result = execute(&execution);
        let op = state
            .actions
            .operations
            .get_mut(request_id)
            .ok_or(AuthorityError::UnknownOperation)?;
        op.outcome = match result {
            ActionResult::Submitted => ActionOutcome::NativePending,
            ActionResult::Completed => ActionOutcome::Completed,
            ActionResult::RejectedNative => ActionOutcome::Failed("rejected_native"),
            ActionResult::Unavailable => ActionOutcome::Failed("native_unavailable"),
            ActionResult::Uncertain => ActionOutcome::Failed("uncertain"),
        };
        if result != ActionResult::Submitted {
            op.finished = Some(now);
        }
        Ok(())
    }

    /// Final one-shot gate for a capability-negotiated worker command. Queueing
    /// a GUI intent is not submission to the native worker.
    pub fn dispatch_native_request(&self, id: &str, send: impl FnOnce() -> bool) -> bool {
        let Ok(mut state) = self.lock() else { return false };
        let Some(op) = state.actions.operations.get(id) else { return false };
        if op.native_sent
            || op.continuation_cancelled
            || !matches!(op.outcome, ActionOutcome::Submitted | ActionOutcome::NativePending)
            || !state.action_valid(op, Instant::now())
        {
            return false;
        }
        let op = state.actions.operations.get_mut(id).expect("locked operation");
        op.native_sent = true;
        op.outcome = ActionOutcome::NativePending;
        send()
    }

    /// An already accepted native exit may retire its original source. Only
    /// body-free continuation is allowed; explicit revoke, a new login in any
    /// captured profile, policy changes and disconnect still cancel it.
    pub fn native_continuation_allowed(&self, id: &str) -> bool {
        self.lock().is_ok_and(|state| {
            state.actions.operations.get(id).is_some_and(|op| {
                state.policy.enabled
                    && state.fresh(Instant::now())
                    && op.policy_revision == state.policy_revision
                    && state.clients.contains_key(&op.client)
                    && op.native_sent
                    && !op.continuation_cancelled
                    && matches!(op.outcome, ActionOutcome::NativePending)
            })
        })
    }

    /// One-shot cutoff for admitting the actual native user prompt. Preparation
    /// and window restoration are cancellable; a committed native prompt may
    /// proceed after later revocation, while native user/editor Cancel still wins.
    pub fn commit_native_exit(&self, id: &str, send: impl FnOnce() -> bool) -> bool {
        let Ok(mut state) = self.lock() else {
            return false;
        };
        let Some(op) = state.actions.operations.get(id) else {
            return false;
        };
        if !state.policy.enabled
            || !state.fresh(Instant::now())
            || op.policy_revision != state.policy_revision
            || !state.clients.contains_key(&op.client)
            || !op.native_sent
            || op.native_committed
            || op.continuation_cancelled
            || !matches!(op.request.1, HostAction::Logout | HostAction::Restart)
            || !matches!(op.outcome, ActionOutcome::NativePending)
        {
            return false;
        }
        state
            .actions
            .operations
            .get_mut(id)
            .expect("locked native operation")
            .native_committed = true;
        send()
    }

    /// Final continuation gate immediately around the native manual process
    /// admission. The closure must not reenter Authority or await user/network.
    pub fn continue_native_action<R>(&self, id: &str, execute: impl FnOnce() -> R) -> Option<R> {
        let state = self.lock().ok()?;
        let op = state.actions.operations.get(id)?;
        if !state.policy.enabled
            || !state.fresh(Instant::now())
            || op.policy_revision != state.policy_revision
            || !state.clients.contains_key(&op.client)
            || !op.native_sent
            || op.continuation_cancelled
            || !matches!(op.outcome, ActionOutcome::NativePending)
        {
            return None;
        }
        Some(execute())
    }

    /// Body-free native completion observation for an already submitted request.
    /// The owning adapter must correlate its actual native acknowledgment/exit;
    /// this is never callable from the MCP request enum or an approval button.
    pub fn observe_action_result(&self, request_id: &str, result: ActionResult) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        let op = state
            .actions
            .operations
            .get_mut(request_id)
            .ok_or(AuthorityError::UnknownOperation)?;
        if !matches!(op.outcome, ActionOutcome::Submitted | ActionOutcome::NativePending) {
            return Err(AuthorityError::FinalOperation);
        }
        op.outcome = match result {
            ActionResult::Submitted => return Ok(()),
            ActionResult::Completed => ActionOutcome::Completed,
            ActionResult::RejectedNative => ActionOutcome::Failed("rejected_native"),
            ActionResult::Unavailable => ActionOutcome::Failed("native_unavailable"),
            ActionResult::Uncertain => ActionOutcome::Failed("uncertain"),
        };
        op.finished = Some(Instant::now());
        Ok(())
    }

    /// Revalidate a native operation waiting in an owning adapter's queue.
    /// Dispatchers use this before the actual native submission; source/policy
    /// revocation is not excused by an earlier bounded-channel admission.
    pub fn action_still_authorized(&self, request_id: &str) -> bool {
        self.lock().is_ok_and(|state| {
            state.actions.operations.get(request_id).is_some_and(|op| {
                matches!(op.outcome, ActionOutcome::Submitted | ActionOutcome::NativePending) && state.action_valid(op, Instant::now())
            })
        })
    }
}

impl State {
    fn action_level(&self, ticket: ProfileTicket, class: PermissionClass) -> AuthorityLevel {
        let global = self.actions.permissions.get(class);
        let local = self
            .actions
            .grants
            .get(&ticket.profile)
            .filter(|(current, _)| *current == ticket)
            .map_or(AuthorityLevel::Never, |(_, levels)| levels.get(class));
        match (global, local) {
            (AuthorityLevel::Never, _) | (_, AuthorityLevel::Never) => AuthorityLevel::Never,
            (AuthorityLevel::Ask, _) | (_, AuthorityLevel::Ask) => AuthorityLevel::Ask,
            _ => AuthorityLevel::Allow,
        }
    }

    fn action_valid(&self, op: &ActionOperation, now: Instant) -> bool {
        self.policy.enabled
            && self.fresh(now)
            && self.policy_revision == op.policy_revision
            && (matches!(op.outcome, ActionOutcome::NativePending)
                || op.owner.is_none_or(|expected| {
                    self.actions.ownership.is_some_and(|(current, observed)| {
                        current == expected && !current.busy && now.saturating_duration_since(observed) <= SNAPSHOT_FRESHNESS
                    })
                }))
            && op.monitor_revision.is_none_or(|revision| {
                self.actions.monitors.as_ref().is_some_and(|(current, observed, _)| {
                    *current == revision && now.saturating_duration_since(*observed) <= SNAPSHOT_FRESHNESS
                })
            })
            && self
                .actions
                .supported
                .iter()
                .any(|action| std::mem::discriminant(action) == std::mem::discriminant(&op.request.1))
            && op
                .profiles
                .iter()
                .all(|ticket| self.current(*ticket) && self.action_level(*ticket, op.request.1.class()) != AuthorityLevel::Never)
    }

    pub(super) fn cancel_native(&mut self, profile: ProfileId) {
        for op in self.actions.operations.values_mut() {
            if op.profiles.iter().any(|ticket| ticket.profile == profile) {
                op.continuation_cancelled = true;
            }
        }
    }

    pub(super) fn replace_native_source(&mut self, profile: ProfileId, binding: Option<Binding>) {
        if let Some(binding) = binding {
            for op in self.actions.operations.values_mut() {
                if op
                    .profiles
                    .iter()
                    .any(|ticket| ticket.profile == profile && ticket.binding != Some(binding))
                {
                    op.continuation_cancelled = true;
                }
            }
        }
    }

    pub(super) fn action_invalidate(&mut self, profile: Option<ProfileId>, reason: &'static str, now: Instant) {
        for op in self.actions.operations.values_mut() {
            if profile.is_none_or(|profile| op.profiles.iter().any(|ticket| ticket.profile == profile))
                && matches!(op.outcome, ActionOutcome::Pending | ActionOutcome::Ready)
            {
                op.outcome = ActionOutcome::Failed(reason);
                op.finished = Some(now);
            }
        }
    }

    pub(super) fn action_prune(&mut self, now: Instant) {
        for op in self.actions.operations.values_mut() {
            if now.saturating_duration_since(op.created) >= APPROVAL_TIMEOUT
                && matches!(op.outcome, ActionOutcome::Pending | ActionOutcome::Ready)
            {
                op.outcome = ActionOutcome::Failed("approval_expired");
                op.finished = Some(now);
            }
            if now.saturating_duration_since(op.created) >= RECEIPT_RETENTION
                && matches!(op.outcome, ActionOutcome::Submitted | ActionOutcome::NativePending)
            {
                op.outcome = ActionOutcome::Failed("uncertain");
                op.finished = Some(now);
            }
        }
        self.actions.operations.retain(|_, op| {
            op.finished
                .is_none_or(|done| now.saturating_duration_since(done) < RECEIPT_RETENTION)
        });
    }

    pub(super) fn action_pending(&self, now: Instant) -> Vec<PendingApproval> {
        self.actions
            .operations
            .iter()
            .filter(|(_, op)| matches!(op.outcome, ActionOutcome::Pending) && self.action_valid(op, now))
            .map(|(id, op)| PendingApproval {
                action: Some(op.request.1.clone()),
                request_id: id.clone(),
                kind: ReadKind::Action,
                profiles: op.profiles.clone(),
                expires_in: APPROVAL_TIMEOUT.saturating_sub(now.saturating_duration_since(op.created)),
            })
            .collect()
    }

    pub(super) fn action_resolve(&mut self, id: &str, decision: ApprovalDecision, now: Instant) -> Result<(), AuthorityError> {
        let op = self.actions.operations.get(id).ok_or(AuthorityError::UnknownOperation)?;
        if !matches!(op.outcome, ActionOutcome::Pending) {
            return Err(AuthorityError::FinalOperation);
        }
        if !self.action_valid(op, now) {
            return Err(AuthorityError::StaleSource);
        }
        let op = self.actions.operations.get_mut(id).ok_or(AuthorityError::UnknownOperation)?;
        op.outcome = if decision == ApprovalDecision::AllowOnce {
            ActionOutcome::Ready
        } else {
            ActionOutcome::Failed("permission_denied")
        };
        if decision == ApprovalDecision::Deny {
            op.finished = Some(now);
        }
        Ok(())
    }

    pub(super) fn action_status(&mut self, client: ClientId, id: &str, now: Instant) -> Value {
        let Some(op) = self.actions.operations.get(id).filter(|op| op.client == client) else {
            return error("unknown_operation");
        };
        if matches!(op.outcome, ActionOutcome::Pending | ActionOutcome::Ready) && !self.action_valid(op, now) {
            let op = self.actions.operations.get_mut(id).expect("operation exists under lock");
            op.outcome = ActionOutcome::Failed("access_revoked");
            op.finished = Some(now);
        }
        let op = &self.actions.operations[id];
        let mut result = match op.outcome {
            ActionOutcome::Pending => json!({"status":"pending","request_id":id}),
            ActionOutcome::Ready => json!({"status":"queued","request_id":id}),
            ActionOutcome::Submitted | ActionOutcome::NativePending => json!({"status":"submitted","request_id":id}),
            ActionOutcome::Completed => json!({"status":"completed","request_id":id}),
            ActionOutcome::Failed("cancelled") => json!({"status":"cancelled","request_id":id}),
            ActionOutcome::Failed(reason) => error(reason),
        };
        if op.continuation_cancelled {
            result["continuation_cancelled"] = json!(true);
            result["native_request_sent"] = json!(op.native_sent);
            result["native_prompt_committed"] = json!(op.native_committed);
            result["native_effect_may_continue"] =
                json!(op.native_sent && !matches!(op.outcome, ActionOutcome::Completed | ActionOutcome::Failed(_)));
        }
        result
    }
    /// Authenticated source ownership survives expected old-source retirement,
    /// so the caller can disarm its own restart in that interval. No current
    /// profile grant is minted or consulted to release future authority.
    pub(super) fn cancel_owned_operation(&mut self, client: ClientId, id: &str, now: Instant) -> Value {
        if uuid::Uuid::parse_str(id).is_err() {
            return error("invalid_request");
        }
        if let Some(op) = self.actions.operations.get_mut(id).filter(|op| op.client == client) {
            op.continuation_cancelled = true;
            if matches!(op.outcome, ActionOutcome::Pending | ActionOutcome::Ready | ActionOutcome::Submitted) && !op.native_sent {
                op.outcome = ActionOutcome::Failed("cancelled");
                if op.finished.is_none() {
                    op.finished = Some(now);
                }
            }
            return json!({"status":"cancelled","request_id":id,"continuation_cancelled":true,"native_request_sent":op.native_sent,"native_prompt_committed":op.native_committed,"native_effect_may_continue":op.native_sent && !matches!(op.outcome,ActionOutcome::Completed|ActionOutcome::Failed(_)),"native_effect_rollback":false,"replacement_login_authorized":false});
        }
        if let Some(op) = self.operations.get_mut(id).filter(|op| op.client == client) {
            let future = matches!(op.outcome, Outcome::Pending | Outcome::Approved);
            if future {
                op.outcome = Outcome::Denied("cancelled");
                if op.finished.is_none() {
                    op.finished = Some(now);
                }
            }
            return json!({"status":"cancelled","request_id":id,"continuation_cancelled":future,"native_effect_may_continue":false,"native_effect_rollback":false,"replacement_login_authorized":false});
        }
        error("unknown_operation")
    }

    pub(super) fn action_request(
        &mut self,
        client: ClientId,
        id: String,
        target: Option<HostTarget>,
        monitor_revision: Option<u64>,
        action: HostAction,
        now: Instant,
    ) -> Value {
        if uuid::Uuid::parse_str(&id).is_err() {
            return error("invalid_request");
        }
        if let Some(previous) = self.actions.operations.get(&id) {
            if previous.client != client {
                return error("unknown_operation");
            }
            if previous.request != (target.clone(), action.clone()) || previous.monitor_revision != monitor_revision {
                return error("request_changed");
            }
            return self.action_status(client, &id, now);
        }
        if self.operations.contains_key(&id) {
            return error("request_changed");
        }
        if !self.policy.enabled {
            return error("host_disabled");
        }
        if !self.fresh(now) {
            return error("source_stale");
        }
        if !self
            .actions
            .supported
            .iter()
            .any(|supported| std::mem::discriminant(supported) == std::mem::discriminant(&action))
        {
            return error("unsupported_action");
        }
        if action.targeted() != target.is_some()
            || matches!(action, HostAction::MonitorClose { monitor } if !(1..=5).contains(&monitor))
            || matches!(action, HostAction::MonitorCycleConfigure { interval_seconds, .. } if !(2..=3600).contains(&interval_seconds))
        {
            return error("invalid_request");
        }
        if action.class() == PermissionClass::Monitors {
            if !self.actions.monitors.as_ref().is_some_and(|(revision, observed, _)| {
                Some(*revision) == monitor_revision && now.saturating_duration_since(*observed) <= SNAPSHOT_FRESHNESS
            }) {
                return error("stale_monitor");
            }
        } else if monitor_revision.is_some() {
            return error("invalid_request");
        }
        let mut profiles = if let Some(target) = &target {
            let Some(profile) = target
                .profile_id
                .parse::<u8>()
                .ok()
                .and_then(|id| ProfileId::new(id).ok())
                .and_then(|id| self.profiles.get(&id))
            else {
                return error("permission_denied");
            };
            let ticket = profile.ticket();
            let exact_ref = match (&ticket.binding, &target.session_ref) {
                (None, None) => true,
                (Some(_), Some(reference)) => self
                    .clients
                    .get(&client)
                    .and_then(|client| client.refs.get(&ticket.profile))
                    .is_some_and(|(current, value)| *current == ticket && value == reference),
                _ => false,
            };
            if ticket.revision != target.profile_revision || !exact_ref || !self.current(ticket) {
                return error("stale_source");
            }
            if matches!(action, HostAction::OpenLogin) != ticket.binding.is_none() {
                return error("native_unavailable");
            }
            vec![ticket]
        } else {
            let Some((_, _, monitors)) = &self.actions.monitors else {
                return error("stale_monitor");
            };
            let opted_in: Vec<_> = monitors
                .eligible
                .iter()
                .copied()
                .filter(|ticket| self.current(*ticket) && self.action_level(*ticket, PermissionClass::Monitors) != AuthorityLevel::Never)
                .collect();
            let mut affected = match &action {
                HostAction::MonitorClose { monitor } => monitors
                    .monitors
                    .iter()
                    .filter(|row| row.monitor == *monitor)
                    .map(|row| row.source)
                    .collect(),
                HostAction::MonitorsArrange => monitors.monitors.iter().filter(|row| row.visible).map(|row| row.source).collect(),
                HostAction::MonitorsOpenAll => opted_in,
                HostAction::MonitorCycleConfigure { .. } | HostAction::MonitorCycleNext => {
                    let mut sources = opted_in;
                    if let Some(current) = monitors.monitors.iter().find(|row| row.monitor == 5)
                        && !sources.contains(&current.source)
                    {
                        sources.push(current.source);
                    }
                    sources
                }
                HostAction::MonitorCyclePause => monitors
                    .monitors
                    .iter()
                    .filter(|row| row.monitor == 5)
                    .map(|row| row.source)
                    .collect(),
                _ => return error("invalid_request"),
            };
            affected.sort_by_key(|ticket| ticket.profile);
            affected.dedup();
            affected
        };
        let owner = if matches!(action, HostAction::Activate | HostAction::OpenLogin | HostAction::Restart) {
            let Some((ownership, observed)) = self.actions.ownership else {
                return error("current_owner_unavailable");
            };
            if ownership.busy {
                return error("busy");
            }
            if now.saturating_duration_since(observed) > SNAPSHOT_FRESHNESS {
                return error("source_stale");
            }
            match ownership.active {
                ActiveOwner::Unavailable => return error("current_owner_unavailable"),
                ActiveOwner::None => {}
                ActiveOwner::Bound(binding) => {
                    let Some(owner) = self
                        .profiles
                        .values()
                        .find(|profile| profile.projection.binding == Some(binding))
                        .map(ProfileState::ticket)
                    else {
                        return error("current_owner_not_opted_in");
                    };
                    if !self.current(owner) || self.action_level(owner, action.class()) == AuthorityLevel::Never {
                        return error("current_owner_not_opted_in");
                    }
                    if !profiles.contains(&owner) {
                        profiles.push(owner);
                    }
                }
            }
            Some(ownership)
        } else {
            None
        };
        let levels: Vec<_> = profiles.iter().map(|ticket| self.action_level(*ticket, action.class())).collect();
        if profiles.is_empty() || levels.contains(&AuthorityLevel::Never) {
            return error("permission_denied");
        }
        if self.actions.operations.len() + self.operations.len() >= MAX_OPERATIONS {
            return error("capacity");
        }
        let outcome = if levels.contains(&AuthorityLevel::Ask) {
            ActionOutcome::Pending
        } else {
            ActionOutcome::Ready
        };
        self.actions.operations.insert(
            id.clone(),
            ActionOperation {
                client,
                request: (target, action),
                profiles,
                policy_revision: self.policy_revision,
                monitor_revision,
                owner,
                created: now,
                finished: None,
                outcome,
                native_sent: false,
                native_committed: false,
                continuation_cancelled: false,
            },
        );
        self.action_status(client, &id, now)
    }

    pub(super) fn monitor_list(&mut self, client: ClientId, now: Instant) -> Value {
        if !self.policy.enabled {
            return error("host_disabled");
        }
        if self.policy.metadata == AuthorityLevel::Never {
            return error("permission_denied");
        }
        if !self.fresh(now) {
            return error("source_stale");
        }
        if !self
            .actions
            .monitors
            .as_ref()
            .is_some_and(|(_, observed, _)| now.saturating_duration_since(*observed) <= SNAPSHOT_FRESHNESS)
        {
            return error("native_monitor_projection_unavailable");
        }
        let profiles: Vec<_> = self
            .profiles
            .values()
            .filter(|profile| self.current(profile.ticket()) && self.level(profile) != AuthorityLevel::Never)
            .map(ProfileState::ticket)
            .collect();
        let ask = self.policy.metadata == AuthorityLevel::Ask
            || profiles.iter().any(|ticket| {
                self.profiles
                    .get(&ticket.profile)
                    .is_some_and(|profile| self.level(profile) == AuthorityLevel::Ask)
            });
        let capture = ReadCapture {
            kind: ReadKind::MonitorList,
            profiles,
            policy_revision: self.policy_revision,
        };
        if !ask {
            return self.deliver(client, &capture, now);
        }
        if self.operations.len() + self.actions.operations.len() >= MAX_OPERATIONS {
            return error("capacity");
        }
        let request_id = uuid::Uuid::new_v4().to_string();
        self.operations.insert(
            request_id.clone(),
            Operation {
                client,
                capture,
                created: now,
                finished: None,
                outcome: Outcome::Pending,
            },
        );
        json!({"status":"pending","request_id":request_id})
    }

    pub(super) fn deliver_monitors(&self, capture: &ReadCapture) -> Value {
        let Some((revision, observed, snapshot)) = &self.actions.monitors else {
            return error("native_monitor_projection_unavailable");
        };
        if observed.elapsed() > SNAPSHOT_FRESHNESS {
            return error("source_stale");
        }
        let monitors: Vec<_> = snapshot.monitors.iter().filter(|monitor| capture.profiles.contains(&monitor.source) && self.current(monitor.source)).map(|monitor| json!({"monitor":monitor.monitor,"profile_id":monitor.source.profile.number().to_string(),"profile_revision":monitor.source.revision,"visible":monitor.visible,"available":monitor.enabled,"cycling":monitor.monitor == 5})).collect();
        let cycle_visible = monitors.iter().any(|monitor| monitor["cycling"] == true);
        let mut result = json!({"status":"ok","monitor_revision":revision,"monitors":monitors});
        if cycle_visible {
            result["cycle_playing"] = json!(snapshot.playing);
            result["cycle_interval_seconds"] = json!(snapshot.interval_seconds);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::{WorkerId, WorkerIdentity};

    fn fixture() -> Authority {
        let installation = InstallationId::from_bytes([1; 32]).unwrap();
        let host = HostId::from_bytes([2; 16]).unwrap();
        let authority = Authority::new(installation, host);
        authority
            .publish(vec![ProfileProjection {
                id: ProfileId::new(1).unwrap(),
                label: MetadataLabel::new("Synthetic").unwrap(),
                phase: ProfilePhase::Starting,
                binding: Some(Binding {
                    worker: WorkerIdentity {
                        installation,
                        host,
                        worker: WorkerId::from_bytes([3; 16]).unwrap(),
                    },
                    session: None,
                }),
                native_display: None,
            }])
            .unwrap();
        let mut permissions = PermissionLevels::default();
        permissions.set(PermissionClass::Metadata, AuthorityLevel::Allow);
        permissions.set(PermissionClass::Detach, AuthorityLevel::Ask);
        authority.configure_permissions(true, permissions).unwrap();
        authority
            .grant_permissions(authority.ticket(ProfileId::new(1).unwrap()).unwrap(), permissions)
            .unwrap();
        authority.support_actions(vec![HostAction::Detach]).unwrap();
        authority
    }

    fn request(authority: &Authority, id: &str, action: HostAction) -> HostRequest {
        let row = authority.request(1, HostRequest::SessionsList)["sessions"][0].clone();
        HostRequest::Action {
            request_id: id.into(),
            action,
            monitor_revision: None,
            target: Some(HostTarget {
                profile_id: row["profile_id"].as_str().unwrap().into(),
                profile_revision: row["profile_revision"].as_u64().unwrap(),
                session_ref: row["session_ref"].as_str().map(str::to_owned),
            }),
        }
    }

    #[test]
    fn action_ask_requires_native_resolution_then_exactly_one_execution() {
        let authority = fixture();
        let id = uuid::Uuid::new_v4().to_string();
        let request = request(&authority, &id, HostAction::Detach);
        assert_eq!(authority.request(1, request.clone())["status"], "pending");
        assert!(authority.ready_actions().unwrap().is_empty());
        assert_eq!(
            authority.request(1, HostRequest::OperationStatus { request_id: id.clone() })["status"],
            "pending"
        );
        authority.resolve(&id, ApprovalDecision::AllowOnce).unwrap();
        assert_eq!(authority.ready_actions().unwrap().len(), 1);
        authority.execute_action(&id, |_| ActionResult::Submitted).unwrap();
        assert!(authority.ready_actions().unwrap().is_empty());
        assert_eq!(authority.request(1, request)["status"], "submitted");
        assert_eq!(
            authority.execute_action(&id, |_| panic!("replay")),
            Err(AuthorityError::FinalOperation)
        );
    }

    #[test]
    fn independent_action_policy_and_client_binding_cannot_be_bypassed() {
        let authority = fixture();
        let id = uuid::Uuid::new_v4().to_string();
        assert_eq!(
            authority.request(1, request(&authority, &id, HostAction::Logout))["error_code"],
            "unsupported_action"
        );
        let request = request(&authority, &id, HostAction::Detach);
        assert_eq!(authority.request(1, request.clone())["status"], "pending");
        assert_eq!(authority.request(2, request)["error_code"], "unknown_operation");
        authority.revoke(ProfileId::new(1).unwrap()).unwrap();
        assert_eq!(
            authority.resolve(&id, ApprovalDecision::AllowOnce),
            Err(AuthorityError::FinalOperation)
        );
        assert!(authority.ready_actions().unwrap().is_empty());
    }

    #[test]
    fn final_action_gate_rejects_replacement_and_disabled_host() {
        let authority = fixture();
        let id = uuid::Uuid::new_v4().to_string();
        authority.request(1, request(&authority, &id, HostAction::Detach));
        authority.resolve(&id, ApprovalDecision::AllowOnce).unwrap();
        authority.configure_permissions(false, PermissionLevels::default()).unwrap();
        assert_eq!(
            authority.execute_action(&id, |_| panic!("revoked execution")),
            Err(AuthorityError::FinalOperation)
        );
        assert!(
            authority
                .profile_access()
                .unwrap()
                .iter()
                .all(|profile| profile.permissions.is_none())
        );
    }
    #[test]
    fn monitor_actions_freeze_native_revision_and_reject_rebound_view() {
        use aurora_session::{AccountKey, GridKey, SessionId, SessionIdentity};
        let authority = fixture();
        let mut projection = authority.lock().unwrap().profiles[&ProfileId::new(1).unwrap()].projection.clone();
        projection.phase = ProfilePhase::Online;
        projection.binding.as_mut().unwrap().session = Some(SessionIdentity {
            session: SessionId::from_bytes([4; 16]).unwrap(),
            account: AccountKey::from_bytes([5; 32]).unwrap(),
            grid: GridKey::from_bytes([6; 32]).unwrap(),
            generation: 1,
        });
        authority.publish(vec![projection]).unwrap();
        let mut permissions = PermissionLevels::default();
        permissions.set(PermissionClass::Metadata, AuthorityLevel::Allow);
        permissions.set(PermissionClass::Monitors, AuthorityLevel::Ask);
        authority.configure_permissions(true, permissions).unwrap();
        let ticket = authority.ticket(ProfileId::new(1).unwrap()).unwrap();
        authority.grant_permissions(ticket, permissions).unwrap();
        authority.support_actions(vec![HostAction::MonitorClose { monitor: 1 }]).unwrap();
        let mut snapshot = MonitorSnapshot {
            eligible: vec![ticket],
            monitors: vec![MonitorProjection {
                monitor: 1,
                source: ticket,
                incarnation: 1,
                visible: true,
                enabled: true,
            }],
            playing: false,
            interval_seconds: 5,
        };
        let revision = authority.publish_monitors(snapshot.clone()).unwrap();
        let rows = authority.request(1, HostRequest::MonitorList);
        assert_eq!(rows["monitors"][0]["profile_id"], "1");
        assert!(rows.get("cycle_playing").is_none());
        let id = uuid::Uuid::new_v4().to_string();
        let request = HostRequest::Action {
            request_id: id.clone(),
            target: None,
            monitor_revision: Some(revision),
            action: HostAction::MonitorClose { monitor: 1 },
        };
        assert_eq!(authority.request(1, request.clone())["status"], "pending");
        authority.resolve(&id, ApprovalDecision::AllowOnce).unwrap();
        snapshot.monitors[0].incarnation += 1;
        assert_ne!(authority.publish_monitors(snapshot).unwrap(), revision);
        assert_eq!(
            authority.execute_action(&id, |_| panic!("rebound close")),
            Err(AuthorityError::StaleSource)
        );
        let mut new_request = request;
        if let HostRequest::Action { request_id, .. } = &mut new_request {
            *request_id = uuid::Uuid::new_v4().to_string();
        }
        assert_eq!(authority.request(1, new_request)["error_code"], "stale_monitor");
    }
    #[test]
    fn presentation_queue_dispatch_rechecks_revocation_and_cannot_replay() {
        let authority = fixture();
        let id = uuid::Uuid::new_v4().to_string();
        authority.request(1, request(&authority, &id, HostAction::Detach));
        authority.resolve(&id, ApprovalDecision::AllowOnce).unwrap();
        authority.execute_action(&id, |_| ActionResult::Submitted).unwrap();
        authority.revoke(ProfileId::new(1).unwrap()).unwrap();
        assert_eq!(
            authority.dispatch_submitted_action(&id, |_| panic!("revoked GUI queue")),
            Err(AuthorityError::StaleSource)
        );
        let authority = fixture();
        let id = uuid::Uuid::new_v4().to_string();
        authority.request(1, request(&authority, &id, HostAction::Detach));
        authority.resolve(&id, ApprovalDecision::AllowOnce).unwrap();
        authority.execute_action(&id, |_| ActionResult::Submitted).unwrap();
        authority
            .dispatch_submitted_action(&id, |execution| {
                assert_eq!(execution.profiles.len(), 1);
                ActionResult::Submitted
            })
            .unwrap();
        assert_eq!(
            authority.dispatch_submitted_action(&id, |_| panic!("native replay")),
            Err(AuthorityError::FinalOperation)
        );
    }

    #[test]
    fn monitor_capture_ignores_unused_stopped_profiles_and_freezes_only_opted_in_eligible_sources() {
        use aurora_session::{AccountKey, GridKey, SessionId, SessionIdentity};
        let authority = fixture();
        let mut online = authority.lock().unwrap().profiles[&ProfileId::new(1).unwrap()].projection.clone();
        online.phase = ProfilePhase::Online;
        online.binding.as_mut().unwrap().session = Some(SessionIdentity {
            session: SessionId::from_bytes([4; 16]).unwrap(),
            account: AccountKey::from_bytes([5; 32]).unwrap(),
            grid: GridKey::from_bytes([6; 32]).unwrap(),
            generation: 1,
        });
        let mut stopped = online.clone();
        stopped.id = ProfileId::new(2).unwrap();
        stopped.phase = ProfilePhase::Stopped;
        stopped.binding = None;
        let mut unapproved = online.clone();
        unapproved.id = ProfileId::new(3).unwrap();
        unapproved.binding.as_mut().unwrap().worker.worker = WorkerId::from_bytes([33; 16]).unwrap();
        authority.publish(vec![online, stopped, unapproved]).unwrap();
        let mut permissions = PermissionLevels::default();
        permissions.set(PermissionClass::Metadata, AuthorityLevel::Allow);
        permissions.set(PermissionClass::Monitors, AuthorityLevel::Ask);
        authority.configure_permissions(true, permissions).unwrap();
        let source = authority.ticket(ProfileId::new(1).unwrap()).unwrap();
        authority.grant_permissions(source, permissions).unwrap();
        let other = authority.ticket(ProfileId::new(3).unwrap()).unwrap();
        authority
            .support_actions(vec![
                HostAction::MonitorClose { monitor: 1 },
                HostAction::MonitorsOpenAll,
                HostAction::MonitorsArrange,
            ])
            .unwrap();
        let revision = authority
            .publish_monitors(MonitorSnapshot {
                eligible: vec![source, other],
                monitors: vec![MonitorProjection {
                    monitor: 1,
                    source,
                    incarnation: 1,
                    visible: true,
                    enabled: true,
                }],
                playing: false,
                interval_seconds: 5,
            })
            .unwrap();
        for action in [
            HostAction::MonitorClose { monitor: 1 },
            HostAction::MonitorsOpenAll,
            HostAction::MonitorsArrange,
        ] {
            let id = uuid::Uuid::new_v4().to_string();
            assert_eq!(
                authority.request(
                    1,
                    HostRequest::Action {
                        request_id: id.clone(),
                        target: None,
                        monitor_revision: Some(revision),
                        action
                    }
                )["status"],
                "pending"
            );
            assert_eq!(
                authority
                    .pending_approvals()
                    .unwrap()
                    .iter()
                    .find(|approval| approval.request_id == id)
                    .unwrap()
                    .profiles,
                vec![source]
            );
        }
    }
    fn owner_fixture(action: HostAction) -> Authority {
        use aurora_session::{AccountKey, GridKey, SessionId, SessionIdentity};
        let authority = fixture();
        let mut first = authority.lock().unwrap().profiles[&ProfileId::new(1).unwrap()].projection.clone();
        first.phase = ProfilePhase::Online;
        first.binding.as_mut().unwrap().session = Some(SessionIdentity {
            session: SessionId::from_bytes([4; 16]).unwrap(),
            account: AccountKey::from_bytes([5; 32]).unwrap(),
            grid: GridKey::from_bytes([6; 32]).unwrap(),
            generation: 1,
        });
        let mut owner = first.clone();
        owner.id = ProfileId::new(2).unwrap();
        owner.binding.as_mut().unwrap().worker.worker = WorkerId::from_bytes([7; 16]).unwrap();
        if action == HostAction::OpenLogin {
            first.phase = ProfilePhase::Stopped;
            first.binding = None;
        }
        authority.publish(vec![first, owner.clone()]).unwrap();
        let mut levels = PermissionLevels::default();
        levels.set(PermissionClass::Metadata, AuthorityLevel::Allow);
        levels.set(action.class(), AuthorityLevel::Ask);
        authority.configure_permissions(true, levels).unwrap();
        authority
            .grant_permissions(authority.ticket(ProfileId::new(1).unwrap()).unwrap(), levels)
            .unwrap();
        authority.support_actions(vec![action]).unwrap();
        authority
            .publish_ownership(HostOwnership {
                active: ActiveOwner::Bound(owner.binding.unwrap()),
                busy: false,
            })
            .unwrap();
        authority
    }

    #[test]
    fn presentation_captures_both_owners_and_revalidates_until_actual_handoff() {
        for action in [HostAction::Activate, HostAction::OpenLogin, HostAction::Restart] {
            let authority = owner_fixture(action.clone());
            let id = uuid::Uuid::new_v4().to_string();
            assert_eq!(
                authority.request(1, request(&authority, &id, action.clone()))["error_code"],
                "current_owner_not_opted_in"
            );
            let owner = authority.ticket(ProfileId::new(2).unwrap()).unwrap();
            let levels = authority.permission_levels().unwrap();
            authority.grant_permissions(owner, levels).unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            assert_eq!(authority.request(1, request(&authority, &id, action.clone()))["status"], "pending");
            let approval = authority.pending_approvals().unwrap().pop().unwrap();
            assert_eq!(approval.profiles.len(), 2);
            assert_eq!(approval.profiles[1], owner);
            authority.resolve(&id, ApprovalDecision::AllowOnce).unwrap();
            authority.execute_action(&id, |_| ActionResult::Submitted).unwrap();
            authority
                .publish_ownership(HostOwnership {
                    active: ActiveOwner::None,
                    busy: false,
                })
                .unwrap();
            assert_eq!(
                authority.dispatch_submitted_action(&id, |_| panic!("owner changed before handoff")),
                Err(AuthorityError::StaleSource)
            );
            authority
                .publish_ownership(HostOwnership {
                    active: ActiveOwner::Bound(owner.binding.unwrap()),
                    busy: false,
                })
                .unwrap();
            authority
                .dispatch_submitted_action(&id, |capture| {
                    assert_eq!(capture.expected_owner, owner.binding);
                    ActionResult::Submitted
                })
                .unwrap();
            // Its own successful handoff changes ownership. Only a minimal
            // submitted receipt remains; this is not an authorization renewal.
            authority
                .publish_ownership(HostOwnership {
                    active: ActiveOwner::None,
                    busy: false,
                })
                .unwrap();
            assert_eq!(
                authority.request(1, HostRequest::OperationStatus { request_id: id })["status"],
                "submitted"
            );
        }
    }

    #[test]
    fn native_continuation_requires_one_real_send_and_explicit_authority() {
        for cancellation in ["revoke", "replacement", "disconnect", "policy", "stale", "missing"] {
            let authority = owner_fixture(HostAction::Restart);
            let levels = authority.permission_levels().unwrap();
            authority
                .grant_permissions(authority.ticket(ProfileId::new(2).unwrap()).unwrap(), levels)
                .unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            authority.request(1, request(&authority, &id, HostAction::Restart));
            authority.resolve(&id, ApprovalDecision::AllowOnce).unwrap();
            authority.execute_action(&id, |_| ActionResult::Submitted).unwrap();
            assert!(!authority.native_continuation_allowed(&id), "GUI admission is not a worker send");
            assert!(authority.dispatch_native_request(&id, || true));
            assert!(!authority.dispatch_native_request(&id, || panic!("duplicate worker command")));
            let mut profiles: Vec<_> = authority.lock().unwrap().profiles.values().map(|p| p.projection.clone()).collect();
            profiles[0].binding = None;
            profiles[0].phase = ProfilePhase::Stopped;
            authority.publish(profiles.clone()).unwrap();
            assert!(
                authority.native_continuation_allowed(&id),
                "expected old-source retirement alone does not approve a new login"
            );
            match cancellation {
                "revoke" => authority.revoke(ProfileId::new(1).unwrap()).unwrap(),
                "replacement" => {
                    profiles[0].binding = profiles[1].binding;
                    profiles[0].binding.as_mut().unwrap().worker.worker = WorkerId::from_bytes([9; 16]).unwrap();
                    profiles[0].phase = ProfilePhase::Online;
                    authority.publish(profiles).unwrap();
                }
                "missing" => {
                    profiles.remove(0);
                    authority.publish(profiles).unwrap();
                }
                "disconnect" => authority.disconnected(1),
                "policy" => authority.configure_permissions(false, PermissionLevels::default()).unwrap(),
                "stale" => authority.lock().unwrap().updated = Some(Instant::now() - SNAPSHOT_FRESHNESS - Duration::from_secs(1)),
                _ => unreachable!(),
            }
            assert!(
                !authority.commit_native_exit(&id, || panic!("revoked prepared native prompt")),
                "{cancellation}"
            );
            assert!(!authority.native_continuation_allowed(&id), "{cancellation}");
            assert!(
                authority
                    .continue_native_action(&id, || panic!("cancelled restart spawn"))
                    .is_none()
            );
        }
    }
    #[test]
    fn final_native_prompt_commit_is_once_and_separate_from_restart_continuation() {
        let authority = owner_fixture(HostAction::Restart);
        let levels = authority.permission_levels().unwrap();
        authority
            .grant_permissions(authority.ticket(ProfileId::new(2).unwrap()).unwrap(), levels)
            .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        authority.request(1, request(&authority, &id, HostAction::Restart));
        authority.resolve(&id, ApprovalDecision::AllowOnce).unwrap();
        authority.execute_action(&id, |_| ActionResult::Submitted).unwrap();
        assert!(!authority.commit_native_exit(&id, || panic!("preparation not sent")));
        assert!(authority.dispatch_native_request(&id, || true));
        assert!(authority.commit_native_exit(&id, || true));
        assert!(!authority.commit_native_exit(&id, || panic!("duplicate committed native prompt")));
        // Committed user UI has its own native Cancel. Later revoke still
        // cancels any future manual process restart, with no inherited grant.
        authority.revoke(ProfileId::new(1).unwrap()).unwrap();
        assert!(!authority.native_continuation_allowed(&id));
        assert!(!authority.commit_native_exit(&id, || panic!("revoke cannot re-admit")));
    }
    #[test]
    fn owned_operation_cancel_disarms_restart_during_retired_source_interval() {
        let authority = owner_fixture(HostAction::Restart);
        let levels = authority.permission_levels().unwrap();
        authority
            .grant_permissions(authority.ticket(ProfileId::new(2).unwrap()).unwrap(), levels)
            .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let original = request(&authority, &id, HostAction::Restart);
        authority.request(1, original.clone());
        authority.resolve(&id, ApprovalDecision::AllowOnce).unwrap();
        authority.execute_action(&id, |_| ActionResult::Submitted).unwrap();
        assert!(authority.dispatch_native_request(&id, || true));
        let mut profiles: Vec<_> = authority.lock().unwrap().profiles.values().map(|p| p.projection.clone()).collect();
        profiles[0].binding = None;
        profiles[0].phase = ProfilePhase::Stopped;
        authority.publish(profiles).unwrap();
        assert!(authority.native_continuation_allowed(&id));
        let cancel = HostRequest::OperationCancel { request_id: id.clone() };
        assert_eq!(authority.request(2, cancel.clone())["error_code"], "unknown_operation");
        assert!(authority.native_continuation_allowed(&id));
        let created = authority.lock().unwrap().actions.operations[&id].created;
        assert_eq!(authority.request(1, cancel.clone())["status"], "cancelled");
        assert!(!authority.native_continuation_allowed(&id));
        assert!(!authority.commit_native_exit(&id, || panic!("cancelled native commit")));
        assert!(
            authority
                .continue_native_action(&id, || panic!("cancelled replacement launch"))
                .is_none()
        );
        assert_eq!(authority.request(1, cancel)["status"], "cancelled");
        assert_eq!(authority.lock().unwrap().actions.operations[&id].created, created);
        assert_eq!(authority.request(1, original)["continuation_cancelled"], true);
        assert!(authority.ready_actions().unwrap().is_empty());
    }
    #[test]
    fn cancel_retains_tombstone_and_committed_native_prompt_can_still_finish() {
        let authority = owner_fixture(HostAction::Restart);
        let levels = authority.permission_levels().unwrap();
        authority
            .grant_permissions(authority.ticket(ProfileId::new(2).unwrap()).unwrap(), levels)
            .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let original = request(&authority, &id, HostAction::Restart);
        authority.request(1, original.clone());
        authority.resolve(&id, ApprovalDecision::AllowOnce).unwrap();
        authority.execute_action(&id, |_| ActionResult::Submitted).unwrap();
        assert!(authority.dispatch_native_request(&id, || true));
        assert!(authority.commit_native_exit(&id, || true));
        let response = authority.request(1, HostRequest::OperationCancel { request_id: id.clone() });
        assert_eq!(response["native_prompt_committed"], true);
        assert_eq!(response["native_effect_may_continue"], true);
        assert!(!authority.native_continuation_allowed(&id));
        authority.observe_action_result(&id, ActionResult::Completed).unwrap();
        assert_eq!(authority.request(1, original)["status"], "completed");
        let id = uuid::Uuid::new_v4().to_string();
        let original = request(&authority, &id, HostAction::Restart);
        authority.request(1, original.clone());
        let response = authority.request(1, HostRequest::OperationCancel { request_id: id.clone() });
        assert_eq!(response["native_request_sent"], false);
        assert_eq!(authority.request(1, original)["status"], "cancelled");
        assert!(authority.resolve(&id, ApprovalDecision::AllowOnce).is_err());
    }
}
