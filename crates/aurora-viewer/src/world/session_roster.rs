//! Native ownership for Conference roster proof. These are local resource
//! budgets, not Second Life limits. Presentation keeps its existing packet
//! data even when proof is unavailable. Never evict lost history into authority.

use super::World;
use aurora_net::social::{
    NativeSessionScope, RosterCompletion, RosterInitialization, RosterSnapshot, SessionAgentBatch, SessionRosterAuthority,
};
use std::collections::HashMap;
use uuid::Uuid;

const MAX_TRACKED: usize = 128;
const MAX_PARTICIPANTS: usize = 1024;
const MAX_PENDING: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
enum InitializationPath {
    Start,
    Acceptance,
}

/// Native callback ownership lives with the existing native session, separately
/// from bounded completeness evidence. Losing roster proof cannot drop a valid
/// native initialization or voice callback.
#[derive(Debug, Clone, Copy)]
pub(super) struct NativeInitialization {
    capture: RosterInitialization,
    start: bool,
    preserve_live: bool,
    observed_through: u64,
}

struct Entry {
    incarnation: u64,
    retired: bool,
    authority: SessionRosterAuthority,
    request: Option<(InitializationPath, RosterInitialization)>,
}

#[derive(Default)]
pub(super) struct NativeSessionRosters {
    scope: Option<NativeSessionScope>,
    entries: HashMap<Uuid, Entry>,
    closed: HashMap<Uuid, u64>,
    closed_history_lost_through: Option<u64>,
    pending_mutes: HashMap<Uuid, (u64, bool)>,
    mute_history_lost_through: Option<u64>,
    incarnation: u64,
    order: u64,
    lost: bool,
}

impl NativeSessionRosters {
    pub(super) fn bind(&mut self, scope: Option<NativeSessionScope>) {
        if self.scope != scope {
            *self = Self { scope, ..Self::default() };
        }
    }

    fn entry(&mut self, session: Uuid) -> Option<&mut Entry> {
        let scope = self.scope?;
        if self.lost || session.is_nil() {
            return None;
        }
        if !self.entries.contains_key(&session) {
            if self.entries.len() == MAX_TRACKED {
                // Retain a login-wide loss latch. Dropping an unknown update
                // then accepting a later base would invent complete history.
                self.lost = true;
                return None;
            }
            self.entries.insert(
                session,
                Entry {
                    incarnation: 0,
                    retired: false,
                    authority: SessionRosterAuthority::new(scope, session, MAX_PARTICIPANTS, MAX_PENDING)?,
                    request: None,
                },
            );
        }
        self.entries.get_mut(&session)
    }

    fn allocate(&mut self, session: Uuid) -> u64 {
        let Some(next) = self.incarnation.checked_add(1) else {
            self.lost = true;
            return 0;
        };
        self.incarnation = next;
        self.closed.remove(&session);
        if let Some(entry) = self.entry(session) {
            entry.incarnation = next;
            entry.retired = false;
            entry.request = None;
        }
        next
    }

    fn begin(&mut self, session: Uuid, incarnation: u64, path: InitializationPath, fixed: bool) -> Option<RosterInitialization> {
        let scope = self.scope?;
        if incarnation == 0 {
            return None;
        }
        let entry = self.entry(session)?;
        if entry.incarnation != incarnation {
            return None;
        }
        let capture = if fixed {
            entry.authority.begin_acceptance(scope, incarnation)
        } else {
            entry.authority.begin_start(scope, incarnation)
        }?;
        entry.request = Some((path, capture));
        Some(capture)
    }

    fn next_order(&mut self) -> Option<u64> {
        self.scope?;
        let Some(order) = self.order.checked_add(1) else {
            self.lost = true;
            self.mute_history_lost_through = Some(u64::MAX);
            return None;
        };
        self.order = order;
        Some(order)
    }

    fn observe(&mut self, session: Uuid, batch: &SessionAgentBatch) -> Option<u64> {
        let scope = self.scope?;
        let order = self.next_order()?;
        if let Some(entry) = self.entry(session) {
            entry.authority.observe(scope, batch, order);
        }
        Some(order)
    }

    fn observe_sender(&mut self, session: Uuid, sender: Uuid) {
        let Some(scope) = self.scope else { return };
        let Some(order) = self.next_order() else { return };
        if let Some(entry) = self.entry(session) {
            entry.authority.observe_sender(scope, sender, order);
        }
    }

    fn accepts(&self, session: Uuid, capture: RosterInitialization, path: InitializationPath) -> bool {
        !self.lost
            && self.scope == Some(capture.scope())
            && self
                .entries
                .get(&session)
                .is_some_and(|entry| entry.incarnation == capture.incarnation() && entry.request == Some((path, capture)))
    }

    fn complete(
        &mut self,
        session: Uuid,
        final_session: Uuid,
        capture: RosterInitialization,
        batch: &SessionAgentBatch,
    ) -> Option<RosterCompletion> {
        let scope = self.scope?;
        let path = self.entries.get(&session)?.request?.0;
        if !self.accepts(session, capture, path) {
            return None;
        }
        if final_session != session {
            if path != InitializationPath::Start || !capture.is_native_start() || final_session.is_nil() {
                return None;
            }
            // Only final-ID updates that preceded native initialization may
            // merge. Never overwrite another initialized native owner.
            if self
                .entries
                .get(&final_session)
                .is_some_and(|entry| entry.incarnation != 0 || entry.retired)
            {
                return None;
            }
        }
        let mut entry = self.entries.remove(&session)?;
        let pending = if final_session != session {
            self.entries.remove(&final_session)
        } else {
            None
        };
        if let Some(pending) = &pending
            && !entry.authority.merge_unbound(scope, capture, final_session, &pending.authority)
        {
            entry.authority.invalidate();
        }
        let result = entry.authority.complete(scope, capture, final_session, batch);
        if result.is_some() {
            entry.request = None;
            self.entries.insert(final_session, entry);
        } else {
            self.entries.insert(session, entry);
            if let Some(pending) = pending {
                self.entries.insert(final_session, pending);
            }
        }
        result
    }

    fn fail(&mut self, session: Uuid, capture: RosterInitialization, retire: bool) -> bool {
        let Some(scope) = self.scope else { return false };
        let Some(path) = self.entries.get(&session).and_then(|entry| entry.request.map(|request| request.0)) else {
            return false;
        };
        if !self.accepts(session, capture, path) {
            return false;
        }
        let entry = self.entries.get_mut(&session).expect("validated native owner");
        if retire {
            entry.authority.fail(scope, capture);
            entry.retired = true;
        } else {
            // Native non-404 acceptance failures keep the visible session.
            // Consume the old callback without inventing a complete base.
            entry.authority.complete(scope, capture, session, &SessionAgentBatch::default());
        }
        entry.request = None;
        true
    }

    pub(super) fn retire(&mut self, session: Uuid) {
        // Close history is independent of roster-proof saturation. It gates
        // only ambiguous remapping replies, never unrelated fixed-target chat.
        if !session.is_nil() && self.scope.is_some() {
            if self.closed.contains_key(&session) || self.closed.len() < MAX_TRACKED {
                self.closed.insert(session, self.incarnation);
            } else {
                // Loss constrains starts already pending at the close, never
                // a genuine new native incarnation created afterward.
                self.closed_history_lost_through = Some(self.incarnation);
            }
        }
        self.pending_mutes.remove(&session);
        if let Some(entry) = self.entry(session) {
            entry.authority.retire();
            entry.request = None;
            entry.retired = true;
        }
    }

    fn retire_proof(&mut self, session: Uuid) {
        // Proof refusal is not a native leave or ForceClose.
        if let Some(entry) = self.entries.get_mut(&session) {
            entry.authority.retire();
            entry.request = None;
            entry.retired = true;
        }
    }
}

impl World {
    pub(super) fn roster_scope(&self) -> Option<NativeSessionScope> {
        let login = self.login.as_ref()?;
        (self.agent_id == login.agent_id && self.shared_chat_incoming_owner == Some((login.agent_id, login.session_id)))
            .then_some(self.groups.rosters.scope)
            .flatten()
    }

    fn bind_roster_scope(&mut self) -> Option<NativeSessionScope> {
        let scope = self.roster_scope();
        self.groups.rosters.bind(scope);
        scope
    }

    pub(super) fn allocate_session_incarnation(&mut self, session: Uuid) -> u64 {
        if self.bind_roster_scope().is_none() {
            return 0;
        }
        self.groups.rosters.allocate(session)
    }

    pub(crate) fn begin_session_initialization(&mut self, session: Uuid, start: bool) -> Option<RosterInitialization> {
        let scope = self.bind_roster_scope()?;
        let info = self.groups.sessions.get(&session)?;
        let mut incarnation = info.native_incarnation;
        let new_incarnation = info.closed || info.native_scope != Some(scope) || incarnation == 0;
        let fixed = !start || info.group || self.groups.is_member(&session);
        let preserve_live = info.active && !info.closed && !new_incarnation;
        if new_incarnation {
            incarnation = self.allocate_session_incarnation(session);
            let pending_mute = self.groups.rosters.pending_mutes.remove(&session);
            let info = self.groups.sessions.get_mut(&session)?;
            info.native_incarnation = incarnation;
            info.native_scope = Some(scope);
            info.native_pending_text_mute = pending_mute;
            info.native_initializations = [None; 2];
            info.native_mute_floor = 0;
            info.conference_known = false;
            info.active = false;
            info.closed = false;
        }
        if incarnation == 0 {
            return None;
        }
        let capture = self
            .groups
            .rosters
            .begin(
                session,
                incarnation,
                if start {
                    InitializationPath::Start
                } else {
                    InitializationPath::Acceptance
                },
                fixed,
            )
            .or_else(|| {
                self.groups.rosters.retire_proof(session);
                let mut correlation = SessionRosterAuthority::new(scope, session, 0, 0)?;
                if fixed {
                    correlation.begin_acceptance(scope, incarnation)
                } else {
                    correlation.begin_start(scope, incarnation)
                }
            })?;
        self.groups.sessions.get_mut(&session)?.native_initializations[usize::from(start)] = Some(NativeInitialization {
            capture,
            start,
            preserve_live,
            observed_through: self.groups.rosters.order,
        });
        Some(capture)
    }

    pub(super) fn observe_session_roster(&mut self, session: Uuid, batch: &SessionAgentBatch) {
        if self.bind_roster_scope().is_some() {
            let Some(order) = self.groups.rosters.observe(session, batch) else {
                return;
            };
            let mute = batch
                .agents()
                .iter()
                .rev()
                .find_map(|agent| (agent.agent == self.agent_id).then_some(agent.text_muted).flatten());
            if let Some(muted) = mute {
                if let Some(info) = self.groups.sessions.get_mut(&session) {
                    info.native_pending_text_mute = Some((order, muted));
                } else if self.groups.rosters.pending_mutes.contains_key(&session) || self.groups.rosters.pending_mutes.len() < MAX_TRACKED
                {
                    self.groups.rosters.pending_mutes.insert(session, (order, muted));
                } else {
                    self.groups.rosters.mute_history_lost_through = Some(order);
                }
            }
        }
    }

    pub(super) fn take_pending_session_mute(&mut self, session: Uuid) -> Option<(u64, bool)> {
        self.groups.rosters.pending_mutes.remove(&session)
    }

    pub(super) fn observe_session_sender(&mut self, session: Uuid, sender: Uuid) {
        if !session.is_nil() && !sender.is_nil() && sender != self.agent_id && self.bind_roster_scope().is_some() {
            self.groups.rosters.observe_sender(session, sender);
        }
    }

    pub(super) fn session_start_capture(&mut self, session: Uuid) -> Option<RosterInitialization> {
        self.bind_roster_scope()?;
        let original = self.groups.sessions.get(&session)?.native_initializations[1]?;
        if !original.start {
            return None;
        }
        let capture = original.capture;
        self.session_capture_current(session, capture, true).then_some(capture)
    }

    pub(super) fn session_capture_current(&self, session: Uuid, capture: RosterInitialization, start: bool) -> bool {
        self.roster_scope() == Some(capture.scope())
            && self.groups.sessions.get(&session).is_some_and(|info| {
                info.native_incarnation == capture.incarnation()
                    && info.native_scope == Some(capture.scope())
                    && info.native_initializations[usize::from(start)]
                        .is_some_and(|original| original.capture == capture && original.start == start)
            })
    }

    pub(super) fn complete_session_roster(
        &mut self,
        session: Uuid,
        final_session: Uuid,
        capture: RosterInitialization,
        batch: &SessionAgentBatch,
    ) -> Option<RosterCompletion> {
        self.bind_roster_scope()?;
        let original = self
            .groups
            .sessions
            .get(&session)?
            .native_initializations
            .iter()
            .flatten()
            .find(|original| original.capture == capture)
            .copied()?;
        if !self.session_capture_current(session, capture, original.start)
            || final_session.is_nil()
            || (session != final_session && (!original.start || !capture.is_native_start()))
        {
            return None;
        }
        if final_session != session
            && (self
                .groups
                .rosters
                .closed_history_lost_through
                .is_some_and(|through| capture.incarnation() <= through)
                || self
                    .groups
                    .rosters
                    .closed
                    .get(&final_session)
                    .is_some_and(|through| capture.incarnation() <= *through))
        {
            // Even an unknown final UUID can have a real ForceClose tombstone.
            // A delayed temporary-ID reply cannot erase that native close.
            self.cancel_session_initialization(session);
            return None;
        }
        let latest_mute = [
            self.groups.sessions.get(&session)?.native_pending_text_mute,
            self.groups.rosters.pending_mutes.remove(&final_session),
        ]
        .into_iter()
        .flatten()
        .max_by_key(|entry| entry.0);
        let native_was_ready = self.groups.sessions.get(&session)?.active && !self.groups.sessions.get(&session)?.closed;
        let mute_floor = original.observed_through.max(self.groups.sessions.get(&session)?.native_mute_floor);
        let later_unmute = latest_mute.is_some_and(|(order, muted)| !muted && order > mute_floor);
        let initial_mute = batch
            .agents()
            .iter()
            .rev()
            .find_map(|agent| (agent.agent == self.agent_id).then_some(agent.text_muted).flatten());
        let bootstrap_mutes = initial_mute == Some(true) && !later_unmute;
        if bootstrap_mutes {
            self.groups.sessions.get_mut(&session)?.native_mute_floor = mute_floor;
        }
        self.groups.sessions.get_mut(&session)?.native_pending_text_mute = latest_mute;
        self.groups.sessions.get_mut(&session)?.native_initializations[usize::from(original.start)] = None;
        let mut completion = self
            .groups
            .rosters
            .complete(session, final_session, capture, batch)
            .unwrap_or(if original.preserve_live {
                RosterCompletion::AcknowledgedExisting
            } else {
                RosterCompletion::Unavailable { self_text_mute: None }
            });
        let mute = if bootstrap_mutes
            || self
                .groups
                .rosters
                .mute_history_lost_through
                .is_some_and(|through| latest_mute.is_none_or(|latest| latest.0 <= through))
        {
            // Lost native permission history can never infer unmute from an
            // older bootstrap. A later explicit current self delta resolves it.
            Some(true)
        } else {
            latest_mute.map(|(_, muted)| muted)
        };
        match &mut completion {
            RosterCompletion::Initialized { self_text_mute } | RosterCompletion::Unavailable { self_text_mute } => {
                if mute.is_some() {
                    *self_text_mute = mute;
                }
            }
            RosterCompletion::AcknowledgedExisting => {}
        }
        if native_was_ready || original.preserve_live {
            // Group voice acceptance also starts the Group chat. Both native
            // callbacks are current, but their unordered bootstrap lists must
            // not replace an already live permission state. Stream deltas win;
            // without one, a bootstrap may tighten but cannot infer unmute.
            let info = self.groups.sessions.get_mut(&session)?;
            if latest_mute.is_some_and(|(_, muted)| muted) || bootstrap_mutes || (final_session != session && mute == Some(true)) {
                info.text_muted = true;
            } else if later_unmute {
                info.text_muted = false;
            }
            completion = RosterCompletion::AcknowledgedExisting;
        }
        Some(completion)
    }

    pub(super) fn fail_session_roster(&mut self, session: Uuid, capture: RosterInitialization, retire: bool) -> bool {
        if self.bind_roster_scope().is_none() {
            return false;
        }
        let Some(original) = self.groups.sessions.get(&session).and_then(|info| {
            info.native_initializations
                .iter()
                .flatten()
                .find(|original| original.capture == capture)
                .copied()
        }) else {
            return false;
        };
        if !self.session_capture_current(session, capture, original.start) {
            return false;
        }
        self.groups
            .sessions
            .get_mut(&session)
            .expect("validated native owner")
            .native_initializations[usize::from(original.start)] = None;
        self.groups.rosters.fail(session, capture, retire);
        true
    }

    pub(super) fn retire_session_roster(&mut self, session: Uuid) {
        if let Some(info) = self.groups.sessions.get_mut(&session) {
            info.native_initializations = [None; 2];
        }
        self.groups.rosters.retire(session);
    }

    pub(super) fn cancel_session_initialization(&mut self, session: Uuid) {
        if let Some(info) = self.groups.sessions.get_mut(&session) {
            info.native_initializations = [None; 2];
        }
        self.groups.rosters.retire_proof(session);
    }

    pub(crate) fn session_roster(&self, session: Uuid) -> Option<RosterSnapshot<'_>> {
        let scope = self.roster_scope()?;
        let info = self.groups.sessions.get(&session)?;
        if !info.active || info.closed || self.groups.rosters.lost || self.groups.rosters.scope != Some(scope) {
            return None;
        }
        let entry = self.groups.rosters.entries.get(&session)?;
        entry.authority.snapshot(scope, info.native_incarnation)
    }

    pub(super) fn apply_completed_roster(&mut self, session: Uuid, completion: RosterCompletion) {
        let exact = self.session_roster(session).map(|snapshot| snapshot.participants.clone());
        let mute = match completion {
            RosterCompletion::Initialized { self_text_mute } | RosterCompletion::Unavailable { self_text_mute } => self_text_mute,
            RosterCompletion::AcknowledgedExisting => None,
        };
        if let Some(info) = self.groups.sessions.get_mut(&session) {
            if let Some(participants) = exact {
                info.left.extend(info.participants.difference(&participants).copied());
                info.left.retain(|id| !participants.contains(id));
                info.participants = participants;
            }
            if let Some(muted) = mute {
                info.text_muted = muted;
            }
        }
    }
}

#[cfg(test)]
impl World {
    /// Bind the actual native login hook without reading any account files.
    pub(crate) fn bind_session_test_login(&mut self, owner: Uuid) {
        let session = Uuid::new_v4();
        self.agent_id = owner;
        self.login = Some(std::sync::Arc::new(aurora_net::LoginResponse {
            agent_id: owner,
            session_id: session,
            secure_session_id: Uuid::nil(),
            first_name: String::new(),
            last_name: String::new(),
            sim_ip: std::net::Ipv4Addr::LOCALHOST,
            sim_port: 0,
            circuit_code: 0,
            region_x: 0,
            region_y: 0,
            region_size_x: 256,
            region_size_y: 256,
            seed_capability: String::new(),
            look_at: glam::Vec3::X,
            message: String::new(),
            agent_appearance_service: String::new(),
            inventory_root: Uuid::nil(),
            mfa_hash: None,
            raw: aurora_llsd::llsd_map! {},
        }));
        self.bind_shared_chat_activity(owner, session);
    }
}

#[cfg(test)]
mod tests;
