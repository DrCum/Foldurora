//! Host MCP authority, independent of character-assistant settings and IPC.
//!
//! Only the trusted host adapter may publish projections, grant access or
//! resolve approvals. Nothing in the MCP/private request enums grants access.
//! Publications must be rebuilt from a newly observed host snapshot: refreshing
//! an old `Arc<HostSnapshot>` would falsely extend its authority. A source gap,
//! replacement login or host shutdown retires grants and pending reads.

use crate::wire::{ClientId, HostHandler, HostRequest, error};
use aurora_session::{Binding, HostId, InstallationId};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub const MAX_PROFILES: usize = 5;
pub const MAX_CLIENTS: usize = 4;
pub const MAX_OPERATIONS: usize = 128;
pub const SNAPSHOT_FRESHNESS: Duration = Duration::from_secs(5);
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(70);
const RECEIPT_RETENTION: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AuthorityLevel {
    #[default]
    Never,
    Ask,
    Allow,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostPolicy {
    pub enabled: bool,
    pub metadata: AuthorityLevel,
    /// Reserved independently. No action dispatcher is implemented.
    pub actions: AuthorityLevel,
}

/// Human profile numbers are 1..=5. Convert the host's typed SlotId explicitly
/// with `SlotId::number()`; never use a row index or account name as identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProfileId(u8);

impl ProfileId {
    pub fn new(number: u8) -> Result<Self, AuthorityError> {
        if (1..=MAX_PROFILES as u8).contains(&number) {
            Ok(Self(number))
        } else {
            Err(AuthorityError::Invalid)
        }
    }

    pub fn number(self) -> u8 {
        self.0
    }
}

/// Bounded, non-control display text, supplied only by the trusted adapter.
/// This is not a generic container for URLs, filesystem paths or debug state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataLabel(String);

impl MetadataLabel {
    pub fn new(text: &str) -> Result<Self, AuthorityError> {
        if text.len() <= 128 && !text.chars().any(char::is_control) {
            Ok(Self(text.to_owned()))
        } else {
            Err(AuthorityError::Invalid)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Construct only after native privacy/RLVa/display rules permit BOTH labels.
/// Raw saved-account metadata, login URIs and names from a host status receipt
/// do not establish that permission. Omit this field when permission is unknown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativePermittedDisplay {
    pub account: MetadataLabel,
    pub grid: MetadataLabel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfilePhase {
    Stopped,
    Starting,
    Online,
    Unavailable,
}

/// The complete whitelist accepted from the host. Binding remains internal and
/// is replaced on every login generation; profile storage slots confer no grant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileProjection {
    pub id: ProfileId,
    pub label: MetadataLabel,
    pub phase: ProfilePhase,
    pub binding: Option<Binding>,
    pub native_display: Option<NativePermittedDisplay>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileTicket {
    pub profile: ProfileId,
    pub revision: u64,
    pub binding: Option<Binding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityError {
    Invalid,
    Unavailable,
    StaleSource,
    UnknownOperation,
    FinalOperation,
    Capacity,
}

impl fmt::Display for AuthorityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "host MCP authority: {self:?}")
    }
}
impl std::error::Error for AuthorityError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    AllowOnce,
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadKind {
    SessionsList,
    SessionStatus,
}

/// Trusted UI data, deliberately not serializable. No remote approval method
/// exists. Display the captured profiles and kind before allowing this read.
#[derive(Clone, Debug)]
pub struct PendingApproval {
    pub request_id: String,
    pub kind: ReadKind,
    pub profiles: Vec<ProfileTicket>,
    pub expires_in: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProfileGrant {
    ticket: ProfileTicket,
    metadata: AuthorityLevel,
    actions: AuthorityLevel,
}

struct ProfileState {
    projection: ProfileProjection,
    revision: u64,
    grant: Option<ProfileGrant>,
}

impl ProfileState {
    fn ticket(&self) -> ProfileTicket {
        ProfileTicket {
            profile: self.projection.id,
            revision: self.revision,
            binding: self.projection.binding,
        }
    }
}

#[derive(Default)]
struct ClientState {
    // At most one reference for each of the five current profile revisions.
    refs: BTreeMap<ProfileId, (ProfileTicket, String)>,
}

#[derive(Clone)]
struct ReadCapture {
    kind: ReadKind,
    profiles: Vec<ProfileTicket>,
    policy_revision: u64,
}

enum Outcome {
    Pending,
    Approved,
    Denied(&'static str),
    Consumed,
}

struct Operation {
    client: ClientId,
    capture: ReadCapture,
    created: Instant,
    finished: Option<Instant>,
    outcome: Outcome,
}

struct State {
    installation: InstallationId,
    host: HostId,
    policy: HostPolicy,
    policy_revision: u64,
    next_profile_revision: u64,
    updated: Option<Instant>,
    profiles: BTreeMap<ProfileId, ProfileState>,
    clients: BTreeMap<ClientId, ClientState>,
    operations: BTreeMap<String, Operation>,
}

/// Shared trusted-host authority. Serialize request evaluation, native approval
/// resolution and revocation under one small lock; no transport IO occurs here.
pub struct Authority(Mutex<State>);

impl Authority {
    pub fn new(installation: InstallationId, host: HostId) -> Self {
        Self(Mutex::new(State {
            installation,
            host,
            policy: HostPolicy::default(),
            policy_revision: 1,
            next_profile_revision: 1,
            updated: None,
            profiles: BTreeMap::new(),
            clients: BTreeMap::new(),
            operations: BTreeMap::new(),
        }))
    }

    fn lock(&self) -> Result<MutexGuard<'_, State>, AuthorityError> {
        self.0.lock().map_err(|_| AuthorityError::Unavailable)
    }

    /// Settings changes revoke pending decisions; enabling the host does not
    /// opt a profile in, change action authority, or enable any character client.
    pub fn configure(&self, policy: HostPolicy) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        if state.policy != policy {
            state.policy = policy;
            state.bump_policy()?;
            state.invalidate_operations("access_revoked", Instant::now());
            for client in state.clients.values_mut() {
                client.refs.clear();
            }
            if !policy.enabled {
                state.profiles.values_mut().for_each(|profile| profile.grant = None);
            }
        }
        Ok(())
    }

    pub fn policy(&self) -> Result<HostPolicy, AuthorityError> {
        Ok(self.lock()?.policy)
    }

    /// Publish a complete, freshly observed whitelist; never call to refresh
    /// old cached host state. Missing profiles and changed bindings revoke access.
    /// Source gaps also require explicit authorization again, even if a later
    /// provider accidentally republishes the same identity.
    pub fn publish(&self, profiles: Vec<ProfileProjection>) -> Result<(), AuthorityError> {
        self.publish_at(profiles, Instant::now())
    }

    fn publish_at(&self, profiles: Vec<ProfileProjection>, now: Instant) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        if profiles.len() > MAX_PROFILES {
            return Err(AuthorityError::Capacity);
        }
        let mut next = BTreeMap::new();
        for projection in profiles {
            let valid_binding = projection.binding.is_none_or(|binding| {
                binding.validate().is_ok() && binding.worker.installation == state.installation && binding.worker.host == state.host
            });
            if !valid_binding
                || (projection.phase == ProfilePhase::Stopped && projection.binding.is_some())
                || (projection.phase == ProfilePhase::Online && projection.binding.is_none_or(|binding| binding.session.is_none()))
                || (projection.native_display.is_some()
                    && (projection.phase != ProfilePhase::Online || projection.binding.is_none_or(|binding| binding.session.is_none())))
                || projection.binding.is_some_and(|binding| {
                    next.values()
                        .any(|other: &ProfileProjection| other.binding.is_some_and(|other| other.worker == binding.worker))
                })
                || next.insert(projection.id, projection).is_some()
            {
                return Err(AuthorityError::Invalid);
            }
        }
        if state.updated.is_some() && !state.fresh(now) {
            state.profiles.values_mut().for_each(|profile| profile.grant = None);
            state.invalidate_operations("source_stale", now);
            state.clients.values_mut().for_each(|client| client.refs.clear());
        }
        let removed: Vec<_> = state.profiles.keys().filter(|id| !next.contains_key(id)).copied().collect();
        for id in removed {
            state.revoke_profile(id, now);
            state.profiles.remove(&id);
        }
        for (id, projection) in next {
            if let Some(current) = state.profiles.get_mut(&id)
                && current.projection.binding == projection.binding
                && (projection.binding.is_some() || current.projection.phase == projection.phase)
                && current.projection.phase != ProfilePhase::Unavailable
                && projection.phase != ProfilePhase::Unavailable
            {
                current.projection = projection;
                continue;
            }
            state.revoke_profile(id, now);
            let revision = state.next_profile_revision;
            state.next_profile_revision = revision.checked_add(1).ok_or(AuthorityError::Unavailable)?;
            state.profiles.insert(
                id,
                ProfileState {
                    projection,
                    revision,
                    grant: None,
                },
            );
        }
        state.updated = Some(now);
        Ok(())
    }

    pub fn ticket(&self, profile: ProfileId) -> Result<ProfileTicket, AuthorityError> {
        let state = self.lock()?;
        if !state.fresh(Instant::now()) {
            return Err(AuthorityError::StaleSource);
        }
        state
            .profiles
            .get(&profile)
            .map(ProfileState::ticket)
            .ok_or(AuthorityError::StaleSource)
    }

    /// Explicit current-source opt-in by the trusted UI. A stopped profile's
    /// ticket does not match the worker/login that later occupies that slot.
    pub fn grant(&self, ticket: ProfileTicket, metadata: AuthorityLevel, actions: AuthorityLevel) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        let now = Instant::now();
        if !state.fresh(now) || !state.current(ticket) {
            return Err(AuthorityError::StaleSource);
        }
        let grant = ProfileGrant { ticket, metadata, actions };
        if state.profiles.get(&ticket.profile).and_then(|profile| profile.grant) != Some(grant) {
            state.revoke_profile(ticket.profile, now);
            let Some(profile) = state.profiles.get_mut(&ticket.profile) else {
                return Err(AuthorityError::StaleSource);
            };
            profile.grant = Some(grant);
        }
        Ok(())
    }

    pub fn revoke(&self, profile: ProfileId) -> Result<(), AuthorityError> {
        self.lock()?.revoke_profile(profile, Instant::now());
        Ok(())
    }

    /// Fail closed on host shutdown/provider loss. Reopening requires a new
    /// fresh publication, enablement and explicit grants.
    pub fn stop(&self) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        state.policy.enabled = false;
        state.updated = None;
        state.bump_policy()?;
        state.profiles.values_mut().for_each(|profile| profile.grant = None);
        state.invalidate_operations("host_disabled", Instant::now());
        state.clients.values_mut().for_each(|client| client.refs.clear());
        Ok(())
    }

    pub fn pending_approvals(&self) -> Result<Vec<PendingApproval>, AuthorityError> {
        let mut state = self.lock()?;
        let now = Instant::now();
        state.prune(now);
        Ok(state
            .operations
            .iter()
            .filter(|(_, operation)| matches!(operation.outcome, Outcome::Pending) && state.valid_capture(&operation.capture, now))
            .map(|(id, operation)| PendingApproval {
                request_id: id.clone(),
                kind: operation.capture.kind,
                profiles: operation.capture.profiles.clone(),
                expires_in: APPROVAL_TIMEOUT.saturating_sub(now.saturating_duration_since(operation.created)),
            })
            .collect())
    }

    /// Called only after a real local approval decision. This records the
    /// decision, without executing a read or changing future permission policy.
    pub fn resolve(&self, request_id: &str, decision: ApprovalDecision) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        let now = Instant::now();
        state.prune(now);
        let operation = state.operations.get(request_id).ok_or(AuthorityError::UnknownOperation)?;
        if !matches!(operation.outcome, Outcome::Pending) {
            return Err(AuthorityError::FinalOperation);
        }
        if !state.valid_capture(&operation.capture, now) {
            return Err(AuthorityError::StaleSource);
        }
        let operation = state.operations.get_mut(request_id).ok_or(AuthorityError::UnknownOperation)?;
        operation.outcome = match decision {
            ApprovalDecision::AllowOnce => Outcome::Approved,
            ApprovalDecision::Deny => Outcome::Denied("permission_denied"),
        };
        if decision == ApprovalDecision::Deny {
            operation.finished = Some(now);
        }
        Ok(())
    }
}

impl State {
    fn fresh(&self, now: Instant) -> bool {
        self.updated
            .is_some_and(|updated| now.saturating_duration_since(updated) <= SNAPSHOT_FRESHNESS)
    }

    fn current(&self, ticket: ProfileTicket) -> bool {
        self.profiles
            .get(&ticket.profile)
            .is_some_and(|profile| profile.ticket() == ticket && profile.projection.phase != ProfilePhase::Unavailable)
    }

    fn bump_policy(&mut self) -> Result<(), AuthorityError> {
        self.policy_revision = self.policy_revision.checked_add(1).ok_or(AuthorityError::Unavailable)?;
        Ok(())
    }

    fn invalidate_operations(&mut self, reason: &'static str, now: Instant) {
        for operation in self.operations.values_mut() {
            if matches!(operation.outcome, Outcome::Pending | Outcome::Approved) {
                operation.outcome = Outcome::Denied(reason);
                operation.finished = Some(now);
            }
        }
    }

    fn revoke_profile(&mut self, id: ProfileId, now: Instant) {
        if let Some(profile) = self.profiles.get_mut(&id) {
            profile.grant = None;
        }
        for client in self.clients.values_mut() {
            client.refs.remove(&id);
        }
        for operation in self.operations.values_mut() {
            if operation.capture.profiles.iter().any(|ticket| ticket.profile == id)
                && matches!(operation.outcome, Outcome::Pending | Outcome::Approved)
            {
                operation.outcome = Outcome::Denied("access_revoked");
                operation.finished = Some(now);
            }
        }
    }

    fn level(&self, profile: &ProfileState) -> AuthorityLevel {
        match profile
            .grant
            .filter(|grant| grant.ticket == profile.ticket())
            .map(|grant| grant.metadata)
        {
            None | Some(AuthorityLevel::Never) => AuthorityLevel::Never,
            Some(_) if self.policy.metadata == AuthorityLevel::Never => AuthorityLevel::Never,
            Some(AuthorityLevel::Ask) => AuthorityLevel::Ask,
            Some(_) if self.policy.metadata == AuthorityLevel::Ask => AuthorityLevel::Ask,
            Some(AuthorityLevel::Allow) => AuthorityLevel::Allow,
        }
    }

    fn valid_capture(&self, capture: &ReadCapture, now: Instant) -> bool {
        self.policy.enabled
            && self.policy.metadata != AuthorityLevel::Never
            && self.policy_revision == capture.policy_revision
            && self.fresh(now)
            && capture.profiles.iter().all(|ticket| {
                self.current(*ticket)
                    && self
                        .profiles
                        .get(&ticket.profile)
                        .is_some_and(|profile| self.level(profile) != AuthorityLevel::Never)
            })
    }

    fn prune(&mut self, now: Instant) {
        for operation in self.operations.values_mut() {
            if now.saturating_duration_since(operation.created) >= APPROVAL_TIMEOUT
                && matches!(operation.outcome, Outcome::Pending | Outcome::Approved)
            {
                operation.outcome = Outcome::Denied("approval_expired");
                operation.finished = Some(now);
            }
        }
        self.operations.retain(|_, operation| {
            operation
                .finished
                .is_none_or(|done| now.saturating_duration_since(done) < RECEIPT_RETENTION)
        });
    }

    fn register(&mut self, client: ClientId) -> bool {
        if client == 0 {
            return false;
        }
        if !self.clients.contains_key(&client) {
            if self.clients.len() >= MAX_CLIENTS {
                return false;
            }
            self.clients.insert(client, ClientState::default());
        }
        true
    }

    fn read(&mut self, client: ClientId, request: HostRequest, now: Instant) -> Value {
        self.prune(now);
        if !self.register(client) {
            return error("capacity");
        }
        if let HostRequest::OperationStatus { request_id } = request {
            return self.operation_status(client, &request_id, now);
        }
        if !self.policy.enabled {
            return error("host_disabled");
        }
        if self.policy.metadata == AuthorityLevel::Never {
            return error("permission_denied");
        }
        if !self.fresh(now) {
            return error("source_stale");
        }
        let (kind, profiles) = match request {
            HostRequest::SessionsList => (
                ReadKind::SessionsList,
                self.profiles
                    .values()
                    .filter(|profile| profile.projection.phase != ProfilePhase::Unavailable && self.level(profile) != AuthorityLevel::Never)
                    .map(ProfileState::ticket)
                    .collect::<Vec<_>>(),
            ),
            HostRequest::SessionStatus {
                profile_id,
                profile_revision,
                session_ref,
            } => {
                let Some(id) = profile_id.parse::<u8>().ok().and_then(|number| ProfileId::new(number).ok()) else {
                    return error("invalid_request");
                };
                let Some(profile) = self.profiles.get(&id) else {
                    return error("permission_denied");
                };
                if self.level(profile) == AuthorityLevel::Never {
                    return error("permission_denied");
                }
                let ticket = profile.ticket();
                let reference_matches = match (&profile.projection.binding, session_ref) {
                    (None, None) => true,
                    (Some(_), Some(reference)) => self
                        .clients
                        .get(&client)
                        .and_then(|state| state.refs.get(&id))
                        .is_some_and(|(current, token)| *current == ticket && *token == reference),
                    _ => false,
                };
                if profile_revision != ticket.revision || !reference_matches || !self.current(ticket) {
                    return error("stale_source");
                }
                (ReadKind::SessionStatus, vec![ticket])
            }
            HostRequest::OperationStatus { .. } => return error("invalid_request"),
        };
        let needs_approval = self.policy.metadata == AuthorityLevel::Ask
            || profiles.iter().any(|ticket| {
                self.profiles
                    .get(&ticket.profile)
                    .is_some_and(|profile| self.level(profile) == AuthorityLevel::Ask)
            });
        let capture = ReadCapture {
            kind,
            profiles,
            policy_revision: self.policy_revision,
        };
        if needs_approval {
            if self.operations.len() >= MAX_OPERATIONS {
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
            json!({ "status": "pending", "request_id": request_id })
        } else {
            self.deliver(client, &capture, now)
        }
    }

    fn operation_status(&mut self, client: ClientId, id: &str, now: Instant) -> Value {
        let Some(operation) = self.operations.get(id).filter(|operation| operation.client == client) else {
            return error("unknown_operation");
        };
        match operation.outcome {
            Outcome::Pending => {
                if !self.valid_capture(&operation.capture, now) {
                    let operation = self.operations.get_mut(id).expect("operation was found under authority lock");
                    operation.outcome = Outcome::Denied("access_revoked");
                    operation.finished = Some(now);
                    return error("access_revoked");
                }
                json!({ "status": "pending", "request_id": id })
            }
            Outcome::Approved => {
                let capture = operation.capture.clone();
                let operation = self.operations.get_mut(id).expect("operation was found under authority lock");
                operation.outcome = Outcome::Consumed;
                operation.finished = Some(now);
                self.deliver(client, &capture, now)
            }
            Outcome::Denied(reason) => error(reason),
            Outcome::Consumed => error("operation_consumed"),
        }
    }

    fn deliver(&mut self, client: ClientId, capture: &ReadCapture, now: Instant) -> Value {
        if !self.valid_capture(capture, now) {
            return error("access_revoked");
        }
        let mut rows = Vec::with_capacity(capture.profiles.len());
        for ticket in &capture.profiles {
            let Some(profile) = self.profiles.get(&ticket.profile) else {
                return error("access_revoked");
            };
            let Some(client) = self.clients.get_mut(&client) else {
                return error("unknown_client");
            };
            let session_ref = profile.projection.binding.map(|_| {
                let (current, token) = client
                    .refs
                    .entry(ticket.profile)
                    .or_insert_with(|| (*ticket, uuid::Uuid::new_v4().to_string()));
                if *current != *ticket {
                    *current = *ticket;
                    *token = uuid::Uuid::new_v4().to_string();
                }
                token.clone()
            });
            let mut row = json!({
                "profile_id": ticket.profile.number().to_string(),
                "profile_revision": ticket.revision,
                "label": profile.projection.label.as_str(),
                "phase": profile.projection.phase,
                "session_ref": session_ref,
            });
            if let Some(display) = &profile.projection.native_display {
                row["account"] = json!(display.account.as_str());
                row["grid"] = json!(display.grid.as_str());
            }
            rows.push(row);
        }
        match capture.kind {
            ReadKind::SessionsList => json!({ "status": "ok", "sessions": rows }),
            ReadKind::SessionStatus => match rows.into_iter().next() {
                Some(row) => json!({ "status": "ok", "session": row }),
                None => error("stale_source"),
            },
        }
    }
}

impl HostHandler for Authority {
    fn request(&self, client: ClientId, request: HostRequest) -> Value {
        match self.lock() {
            Ok(mut state) => state.read(client, request, Instant::now()),
            Err(_) => error("unavailable"),
        }
    }

    fn disconnected(&self, client: ClientId) {
        if let Ok(mut state) = self.lock() {
            state.clients.remove(&client);
            state.operations.retain(|_, operation| operation.client != client);
        }
    }
}

#[cfg(test)]
mod tests;
