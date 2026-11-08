//! Native session roster state only; no network/UI/read/send effects.
//!
//! The owning World must supply its actual login and native incarnation, bind
//! asynchronous replies to the captured request, and retire on close/reset.
//! Folderstorm abbe3cb0 llimview.cpp:717–727,4900–4906 applies the accepted
//! initial speaker base followed by all retained pending membership updates.
//! Parsing or a participant observation alone never supplies that authority.

use super::{SessionAgent, SessionAgentBatch};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeSessionScope {
    owner: Uuid,
    login: u64,
}

impl NativeSessionScope {
    /// Caller supplies the actual native account and fresh login generation,
    /// never labels, a host selection, or fields received in a server payload.
    pub fn new(owner: Uuid, login: u64) -> Option<Self> {
        (!owner.is_nil() && login != 0).then_some(Self { owner, login })
    }
}

/// Local correlation only. These values must not enter an SL HTTP body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RosterInitialization {
    scope: NativeSessionScope,
    session: Uuid,
    incarnation: u64,
    request: Uuid,
    may_remap: bool,
}

impl RosterInitialization {
    pub fn scope(self) -> NativeSessionScope {
        self.scope
    }
    pub fn session(self) -> Uuid {
        self.session
    }
    pub fn incarnation(self) -> u64 {
        self.incarnation
    }
    pub fn request(self) -> Uuid {
        self.request
    }
    pub fn is_native_start(self) -> bool {
        self.may_remap
    }
}

/// Only a matched original callback returns a completion. The owning reducer
/// must check this before applying success OR failure to native session state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RosterCompletion {
    /// Apply the initial native base then this retained newest self-mute value.
    Initialized { self_text_mute: Option<bool> },
    /// An already initialized incarnation keeps its live state; a repeated
    /// acceptance response cannot overwrite it with an older bootstrap base.
    AcknowledgedExisting,
    /// Native presentation may remain available, but no complete roster exists.
    Unavailable { self_text_mute: Option<bool> },
}

pub struct RosterSnapshot<'a> {
    pub scope: NativeSessionScope,
    pub session: Uuid,
    pub incarnation: u64,
    pub revision: u64,
    pub participants: &'a BTreeSet<Uuid>,
    pub self_text_mute: Option<bool>,
}

#[derive(Clone)]
struct PendingAgent {
    order: u64,
    index: usize,
    agent: SessionAgent,
}

/// Single-native-session component. Its owner additionally bounds the number
/// of tracked/unknown sessions and retains overflow tombstones across eviction.
/// Participant/pending limits are local resource limits, not protocol limits.
pub struct SessionRosterAuthority {
    scope: NativeSessionScope,
    session: Uuid,
    incarnation: u64,
    request: Option<RosterInitialization>,
    existing_at_request: bool,
    pending_final: Option<Uuid>,
    participants: Option<BTreeSet<Uuid>>,
    self_text_mute: Option<bool>,
    pending: Vec<PendingAgent>,
    unconfirmed_senders: BTreeMap<Uuid, u64>,
    latest_pending_mute: Option<(u64, bool)>,
    last_order: u64,
    revision: u64,
    lost: bool,
    retired: bool,
    maximum_participants: usize,
    maximum_pending: usize,
}

impl SessionRosterAuthority {
    /// An unbound session can retain current-login deltas before its initial
    /// invitation/start callback. It has no completeness witness yet.
    pub fn new(scope: NativeSessionScope, session: Uuid, maximum_participants: usize, maximum_pending: usize) -> Option<Self> {
        (!session.is_nil()).then_some(Self {
            scope,
            session,
            incarnation: 0,
            request: None,
            existing_at_request: false,
            pending_final: None,
            participants: None,
            self_text_mute: None,
            pending: Vec::new(),
            unconfirmed_senders: BTreeMap::new(),
            latest_pending_mute: None,
            last_order: 0,
            revision: 0,
            lost: false,
            retired: false,
            maximum_participants,
            maximum_pending,
        })
    }

    /// Incarnation must come from native session creation, not an HTTP retry.
    /// Replacing a request in the same incarnation preserves pending updates.
    pub fn begin_acceptance(&mut self, scope: NativeSessionScope, incarnation: u64) -> Option<RosterInitialization> {
        self.begin(scope, incarnation, false)
    }

    pub fn begin_start(&mut self, scope: NativeSessionScope, incarnation: u64) -> Option<RosterInitialization> {
        self.begin(scope, incarnation, true)
    }

    fn begin(&mut self, scope: NativeSessionScope, incarnation: u64, may_remap: bool) -> Option<RosterInitialization> {
        if scope != self.scope || incarnation == 0 || incarnation < self.incarnation || (self.retired && incarnation == self.incarnation) {
            return None;
        }
        // A retry must not detach final-session deltas from the temporary
        // start that owns them, including by changing its initialization kind.
        if incarnation == self.incarnation && !may_remap && self.pending_final.is_some_and(|bound| bound != self.session) {
            return None;
        }
        if self.incarnation != 0 && self.incarnation != incarnation {
            self.pending_final = None;
            self.participants = None;
            self.self_text_mute = None;
            self.pending.clear();
            self.unconfirmed_senders.clear();
            self.latest_pending_mute = None;
            self.last_order = 0;
            self.revision = 0;
            self.lost = false;
        }
        self.retired = false;
        self.incarnation = incarnation;
        self.existing_at_request = self.participants.is_some() && !self.lost;
        if !may_remap {
            // Acceptance is already addressed to a final session. A later
            // same-incarnation start must not retarget its retained deltas.
            self.pending_final = Some(self.session);
        }
        let capture = RosterInitialization {
            scope,
            session: self.session,
            incarnation,
            request: Uuid::new_v4(),
            may_remap,
        };
        self.request = Some(capture);
        Some(capture)
    }

    /// `order` is the owning native reducer's checked, global event ordinal,
    /// not a server-provided number. It permits exact temp/final pending merge.
    pub fn observe(&mut self, scope: NativeSessionScope, batch: &SessionAgentBatch, order: u64) -> bool {
        if scope != self.scope || self.retired {
            return false;
        }
        if order == 0 || order <= self.last_order {
            self.lose();
            return false;
        }
        self.last_order = order;
        for agent in batch.agents() {
            if agent.agent == self.scope.owner
                && let Some(muted) = agent.text_muted
            {
                self.latest_pending_mute = Some((order, muted));
            }
        }
        // An unsolicited base in a membership-update event is not an original
        // initialization reply. Never silently replace/extend a proven roster.
        let Some(updates) = batch.updates().filter(|_| batch.baseline().is_none()) else {
            self.lose();
            return false;
        };
        if self.lost {
            return false;
        }
        let mut resolved_sender = false;
        for agent in updates {
            if agent.present.is_some() && self.unconfirmed_senders.get(&agent.agent).is_some_and(|observed| *observed < order) {
                self.unconfirmed_senders.remove(&agent.agent);
                resolved_sender = true;
            }
        }
        if let Some(participants) = &mut self.participants {
            match apply(
                participants,
                &mut self.self_text_mute,
                updates,
                self.scope.owner,
                self.maximum_participants,
            ) {
                Ok(true) => self.advance_revision(),
                Ok(false) if resolved_sender => self.advance_revision(),
                Ok(false) => true,
                Err(()) => {
                    self.lose();
                    false
                }
            }
        } else {
            if self
                .pending
                .len()
                .checked_add(updates.len())
                .is_none_or(|count| count > self.maximum_pending)
            {
                self.lose();
                return false;
            }
            self.pending.extend(updates.iter().enumerate().map(|(index, agent)| PendingAgent {
                order,
                index,
                agent: agent.clone(),
            }));
            true
        }
    }

    /// Actual live text can contradict completeness, but cannot add a proven
    /// member. Only a qualified base or later ENTER/LEAVE resolves uncertainty.
    pub fn observe_sender(&mut self, scope: NativeSessionScope, sender: Uuid, order: u64) -> bool {
        if scope != self.scope || self.retired || sender.is_nil() || sender == scope.owner {
            return false;
        }
        if order == 0 || order <= self.last_order {
            self.lose();
            return false;
        }
        self.last_order = order;
        if self.lost {
            return false;
        }
        if self
            .participants
            .as_ref()
            .is_some_and(|participants| participants.contains(&sender))
        {
            return true;
        }
        if !self.unconfirmed_senders.contains_key(&sender) && self.unconfirmed_senders.len() >= self.maximum_participants {
            self.lose();
            return false;
        }
        let added = self.unconfirmed_senders.insert(sender, order).is_none();
        if added && self.participants.is_some() {
            self.advance_revision()
        } else {
            true
        }
    }

    /// Merge only unbound current-login pending data when a native temporary
    /// ID becomes the final ID. The owner still validates the original start.
    pub fn merge_unbound(&mut self, scope: NativeSessionScope, capture: RosterInitialization, final_session: Uuid, other: &Self) -> bool {
        if scope != self.scope
            || self.scope != other.scope
            || self.participants.is_some()
            || self.request != Some(capture)
            || !capture.may_remap
            || final_session.is_nil()
            || other.session != final_session
            || other.incarnation != 0
            || other.request.is_some()
            || other.participants.is_some()
            || other.retired
        {
            return false;
        }
        if self.pending_final.is_some_and(|current| current != final_session) {
            return false;
        }
        self.pending_final = Some(final_session);
        if let Some(mute) = other.latest_pending_mute
            && self.latest_pending_mute.is_none_or(|current| mute.0 > current.0)
        {
            self.latest_pending_mute = Some(mute);
        }
        self.last_order = self.last_order.max(other.last_order);
        for (&sender, &order) in &other.unconfirmed_senders {
            if !self.unconfirmed_senders.contains_key(&sender) && self.unconfirmed_senders.len() >= self.maximum_participants {
                self.lose();
                return false;
            }
            let current = self.unconfirmed_senders.entry(sender).or_insert(order);
            *current = (*current).max(order);
        }
        if self.lost
            || other.lost
            || self
                .pending
                .len()
                .checked_add(other.pending.len())
                .is_none_or(|count| count > self.maximum_pending)
        {
            self.lose();
            return false;
        }
        self.pending.extend(other.pending.iter().cloned());
        self.pending.sort_by_key(|entry| (entry.order, entry.index));
        if self
            .pending
            .windows(2)
            .any(|pair| (pair[0].order, pair[0].index) == (pair[1].order, pair[1].index))
        {
            self.lose();
            return false;
        }
        true
    }

    /// Consume the original correlation before considering the payload. Missing
    /// or invalid data cannot be replayed later as a successful response.
    pub fn complete(
        &mut self,
        scope: NativeSessionScope,
        capture: RosterInitialization,
        final_session: Uuid,
        batch: &SessionAgentBatch,
    ) -> Option<RosterCompletion> {
        if scope != self.scope
            || self.request != Some(capture)
            || self.retired
            || capture.incarnation != self.incarnation
            || capture.session != self.session
            || final_session.is_nil()
            || (final_session != self.session && !capture.may_remap)
            || self.pending_final.is_some_and(|current| current != final_session)
            || (self.existing_at_request && final_session != self.session)
        {
            return None;
        }
        self.request = None;
        // Binding lasts for the native incarnation, even if this response has
        // no usable base and a subsequent HTTP attempt supplies it. Otherwise
        // retained A deltas could be replayed over a later B base.
        self.pending_final = Some(final_session);
        if self.existing_at_request {
            return Some(RosterCompletion::AcknowledgedExisting);
        }
        self.session = final_session;
        let latest = self.latest_pending_mute.map(|(_, muted)| muted);
        let Some(base) = batch.baseline().filter(|_| !self.lost) else {
            return Some(RosterCompletion::Unavailable { self_text_mute: latest });
        };
        let mut participants = BTreeSet::new();
        let mut own = None;
        if apply(&mut participants, &mut own, base, self.scope.owner, self.maximum_participants).is_err()
            || apply(
                &mut participants,
                &mut own,
                batch.updates().unwrap_or_default(),
                self.scope.owner,
                self.maximum_participants,
            )
            .is_err()
        {
            self.lose();
            return Some(RosterCompletion::Unavailable { self_text_mute: latest });
        }
        // Initial reply-local deltas precede the separately retained native
        // stream, following setSpeakers(base), updateSpeakers(pending).
        for entry in &self.pending {
            if apply(
                &mut participants,
                &mut own,
                std::slice::from_ref(&entry.agent),
                self.scope.owner,
                self.maximum_participants,
            )
            .is_err()
            {
                self.lose();
                return Some(RosterCompletion::Unavailable { self_text_mute: latest });
            }
            if entry.agent.present.is_some()
                && self
                    .unconfirmed_senders
                    .get(&entry.agent.agent)
                    .is_some_and(|observed| *observed < entry.order)
            {
                self.unconfirmed_senders.remove(&entry.agent.agent);
            }
        }
        self.unconfirmed_senders.retain(|sender, _| !participants.contains(sender));
        self.participants = Some(participants);
        self.self_text_mute = own;
        self.pending.clear();
        self.latest_pending_mute = None;
        if !self.advance_revision() {
            return Some(RosterCompletion::Unavailable { self_text_mute: own });
        }
        if !self.unconfirmed_senders.is_empty() {
            return Some(RosterCompletion::Unavailable { self_text_mute: own });
        }
        Some(RosterCompletion::Initialized { self_text_mute: own })
    }

    pub fn fail(&mut self, scope: NativeSessionScope, capture: RosterInitialization) -> bool {
        if scope != self.scope || self.request != Some(capture) || self.retired {
            return false;
        }
        self.retire();
        true
    }

    pub fn retire(&mut self) {
        self.request = None;
        self.participants = None;
        self.pending.clear();
        self.unconfirmed_senders.clear();
        self.latest_pending_mute = None;
        self.retired = true;
    }

    /// The owner could not retain required native history. This cannot grant
    /// or recover authority within the same native incarnation.
    pub fn invalidate(&mut self) {
        self.lose();
    }

    pub fn snapshot(&self, scope: NativeSessionScope, incarnation: u64) -> Option<RosterSnapshot<'_>> {
        if scope != self.scope
            || incarnation == 0
            || incarnation != self.incarnation
            || self.retired
            || self.lost
            || self.revision == 0
            || !self.unconfirmed_senders.is_empty()
        {
            return None;
        }
        Some(RosterSnapshot {
            scope,
            session: self.session,
            incarnation,
            revision: self.revision,
            participants: self.participants.as_ref()?,
            self_text_mute: self.self_text_mute,
        })
    }

    fn lose(&mut self) {
        self.lost = true;
        self.participants = None;
        self.pending.clear();
        self.unconfirmed_senders.clear();
    }

    fn advance_revision(&mut self) -> bool {
        let Some(revision) = self.revision.checked_add(1) else {
            self.lose();
            return false;
        };
        self.revision = revision;
        true
    }
}

fn apply(
    participants: &mut BTreeSet<Uuid>,
    own: &mut Option<bool>,
    agents: &[SessionAgent],
    owner: Uuid,
    maximum: usize,
) -> Result<bool, ()> {
    let mut changed = false;
    for agent in agents {
        match agent.present {
            Some(true) => {
                if !participants.contains(&agent.agent) && participants.len() >= maximum {
                    return Err(());
                }
                changed |= participants.insert(agent.agent);
            }
            Some(false) => {
                changed |= participants.remove(&agent.agent);
            }
            None => {}
        }
        if agent.agent == owner
            && let Some(muted) = agent.text_muted
        {
            changed |= *own != Some(muted);
            *own = Some(muted);
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::parse_session_agent_batch;
    use aurora_llsd::{Llsd, Map, llsd_map};

    fn scope() -> NativeSessionScope {
        NativeSessionScope::new(Uuid::from_u128(1), 7).unwrap()
    }
    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }
    fn base(ids: &[u128], own_mute: Option<bool>) -> SessionAgentBatch {
        let mut members = Map::new();
        for number in ids {
            let metadata = if *number == 1 {
                own_mute.map_or_else(|| llsd_map! {}, |muted| llsd_map! { "mutes" => llsd_map! { "text" => muted } })
            } else {
                llsd_map! {}
            };
            members.insert(id(*number).to_string(), metadata);
        }
        parse_session_agent_batch(&llsd_map! { "agent_info" => Llsd::Map(members) }, 64)
    }
    fn delta(number: u128, transition: Option<&str>, muted: Option<bool>) -> SessionAgentBatch {
        let mut update = Map::new();
        if let Some(transition) = transition {
            update.insert("transition".into(), transition.into());
        }
        if let Some(muted) = muted {
            update.insert("info".into(), llsd_map! { "mutes" => llsd_map! { "text" => muted } });
        }
        parse_session_agent_batch(
            &llsd_map! { "agent_updates" => llsd_map! { id(number).to_string() => Llsd::Map(update) } },
            64,
        )
    }
    fn roster() -> SessionRosterAuthority {
        SessionRosterAuthority::new(scope(), id(100), 4, 8).unwrap()
    }
    fn ready() -> (SessionRosterAuthority, RosterInitialization) {
        let mut roster = roster();
        let capture = roster.begin_acceptance(scope(), 1).unwrap();
        assert!(matches!(
            roster.complete(scope(), capture, id(100), &base(&[1, 2], None)),
            Some(RosterCompletion::Initialized { .. })
        ));
        (roster, capture)
    }

    #[test]
    fn conference_roster_state_replays_prebase_membership_and_self_mute_after_the_original_base() {
        let mut roster = roster();
        assert!(roster.observe(scope(), &delta(1, None, Some(true)), 1));
        assert!(roster.observe(scope(), &delta(2, Some("LEAVE"), None), 2));
        let capture = roster.begin_acceptance(scope(), 1).unwrap();
        assert!(roster.snapshot(scope(), 1).is_none());
        assert_eq!(
            roster.complete(scope(), capture, id(100), &base(&[1, 2], Some(false))),
            Some(RosterCompletion::Initialized {
                self_text_mute: Some(true)
            })
        );
        let snapshot = roster.snapshot(scope(), 1).unwrap();
        assert_eq!(snapshot.participants, &[id(1)].into_iter().collect::<BTreeSet<_>>());
        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.self_text_mute, Some(true));
        assert_eq!(capture.scope(), scope());
        assert_eq!(capture.session(), id(100));
        assert_eq!(capture.incarnation(), 1);
        assert!(!capture.request().is_nil() && !capture.is_native_start());
    }

    #[test]
    fn conference_roster_state_original_request_scope_and_incarnation_gate_success_and_failure() {
        let mut roster = roster();
        let first = roster.begin_acceptance(scope(), 1).unwrap();
        let replacement = roster.begin_acceptance(scope(), 1).unwrap();
        assert_ne!(first.request(), replacement.request());
        let body = base(&[1, 2], None);
        assert!(roster.complete(scope(), first, id(100), &body).is_none());
        assert!(!roster.fail(scope(), first));
        for wrong in [
            NativeSessionScope::new(id(9), 7).unwrap(),
            NativeSessionScope::new(id(1), 8).unwrap(),
        ] {
            assert!(roster.complete(wrong, replacement, id(100), &body).is_none());
            assert!(!roster.fail(wrong, replacement));
            assert!(!roster.observe(wrong, &delta(2, Some("LEAVE"), None), 1));
        }
        assert!(
            roster.complete(scope(), replacement, id(200), &body).is_none(),
            "acceptance cannot retarget the session"
        );
        assert!(roster.complete(scope(), replacement, id(100), &body).is_some());
        assert!(roster.complete(scope(), replacement, id(100), &body).is_none());
        assert!(!roster.fail(scope(), replacement));
        assert!(roster.snapshot(scope(), 2).is_none());
    }

    #[test]
    fn conference_roster_state_temp_final_merge_preserves_global_order_and_rejects_cross_target_data() {
        let mut temporary = roster();
        let capture = temporary.begin_start(scope(), 1).unwrap();
        assert!(capture.is_native_start());
        assert!(temporary.observe(scope(), &delta(2, Some("LEAVE"), None), 2));
        let mut final_pending = SessionRosterAuthority::new(scope(), id(200), 4, 8).unwrap();
        assert!(final_pending.observe(scope(), &delta(2, Some("ENTER"), None), 3));
        assert!(!final_pending.observe(scope(), &delta(1, None, Some(true)), 1));
        // Ordinals belong to the reducer; a backwards insertion is a lost
        // history, never data that may be silently sorted into authority.
        assert!(!temporary.merge_unbound(scope(), capture, id(200), &final_pending));
        assert!(matches!(
            temporary.complete(scope(), capture, id(200), &base(&[1, 2], Some(false))),
            Some(RosterCompletion::Unavailable { .. })
        ));

        let mut temporary = roster();
        let capture = temporary.begin_start(scope(), 1).unwrap();
        assert!(temporary.observe(scope(), &delta(2, Some("LEAVE"), None), 2));
        let mut final_pending = SessionRosterAuthority::new(scope(), id(200), 4, 8).unwrap();
        assert!(final_pending.observe(scope(), &delta(1, None, Some(true)), 1));
        assert!(final_pending.observe(scope(), &delta(2, Some("ENTER"), None), 3));
        assert!(!temporary.merge_unbound(scope(), capture, id(201), &final_pending));
        assert!(temporary.merge_unbound(scope(), capture, id(200), &final_pending));
        assert!(temporary.complete(scope(), capture, id(201), &base(&[1, 2], Some(false))).is_none());
        assert_eq!(
            temporary.complete(scope(), capture, id(200), &base(&[1, 2], Some(false))),
            Some(RosterCompletion::Initialized {
                self_text_mute: Some(true)
            })
        );
        let snapshot = temporary.snapshot(scope(), 1).unwrap();
        assert_eq!(snapshot.session, id(200));
        assert!(
            snapshot.participants.contains(&id(2)),
            "later ENTER follows earlier LEAVE across both pending owners"
        );
    }

    #[test]
    fn conference_roster_state_missing_invalid_and_unproven_bases_consume_but_never_complete() {
        for body in [
            parse_session_agent_batch(&llsd_map! {}, 64),
            SessionAgentBatch::unproven(base(&[1, 2], None).into_agents()),
            parse_session_agent_batch(&llsd_map! { "agent_info" => false }, 64),
        ] {
            let mut roster = roster();
            let capture = roster.begin_acceptance(scope(), 1).unwrap();
            assert!(matches!(
                roster.complete(scope(), capture, id(100), &body),
                Some(RosterCompletion::Unavailable { .. })
            ));
            assert!(roster.snapshot(scope(), 1).is_none());
            assert!(roster.complete(scope(), capture, id(100), &base(&[1, 2], None)).is_none());
        }
    }

    #[test]
    fn conference_roster_state_retry_preserves_the_merged_final_session_and_request_kind_binding() {
        let mut temporary = roster();
        let original = temporary.begin_start(scope(), 1).unwrap();
        let mut final_pending = SessionRosterAuthority::new(scope(), id(200), 4, 8).unwrap();
        assert!(final_pending.observe(scope(), &delta(2, Some("LEAVE"), None), 1));
        assert!(temporary.merge_unbound(scope(), original, id(200), &final_pending));

        let retry = temporary.begin_start(scope(), 1).unwrap();
        assert_ne!(original.request(), retry.request());
        assert!(temporary.begin_acceptance(scope(), 1).is_none());
        assert!(temporary.complete(scope(), retry, id(201), &base(&[1, 2], None)).is_none());
        assert!(temporary.snapshot(scope(), 1).is_none());
        assert!(matches!(
            temporary.complete(scope(), retry, id(200), &base(&[1, 2], None)),
            Some(RosterCompletion::Initialized { .. })
        ));
        let snapshot = temporary.snapshot(scope(), 1).unwrap();
        assert_eq!(snapshot.session, id(200));
        assert!(!snapshot.participants.contains(&id(2)));
        // Once initialization has reached that final session, an ordinary
        // acceptance acknowledgment may preserve the established live state.
        let acknowledgment = temporary.begin_acceptance(scope(), 1).unwrap();
        assert_eq!(
            temporary.complete(scope(), acknowledgment, id(200), &base(&[1, 2], None)),
            Some(RosterCompletion::AcknowledgedExisting)
        );
    }

    #[test]
    fn conference_roster_state_missing_completion_cannot_launder_retained_final_session_updates() {
        let mut temporary = roster();
        let original = temporary.begin_start(scope(), 1).unwrap();
        let mut final_pending = SessionRosterAuthority::new(scope(), id(200), 4, 8).unwrap();
        assert!(final_pending.observe(scope(), &delta(2, Some("LEAVE"), None), 1));
        assert!(temporary.merge_unbound(scope(), original, id(200), &final_pending));
        assert!(matches!(
            temporary.complete(scope(), original, id(200), &parse_session_agent_batch(&llsd_map! {}, 64)),
            Some(RosterCompletion::Unavailable { .. })
        ));

        let retry = temporary.begin_start(scope(), 1).unwrap();
        assert!(temporary.complete(scope(), retry, id(201), &base(&[1, 2], None)).is_none());
        assert!(temporary.snapshot(scope(), 1).is_none());
        assert!(matches!(
            temporary.complete(scope(), retry, id(200), &base(&[1, 2], None)),
            Some(RosterCompletion::Initialized { .. })
        ));
        assert!(!temporary.snapshot(scope(), 1).unwrap().participants.contains(&id(2)));
    }

    #[test]
    fn conference_roster_state_only_a_new_native_incarnation_clears_the_final_session_binding() {
        let mut temporary = roster();
        let original = temporary.begin_start(scope(), 1).unwrap();
        let mut final_pending = SessionRosterAuthority::new(scope(), id(200), 4, 8).unwrap();
        assert!(final_pending.observe(scope(), &delta(2, Some("LEAVE"), None), 1));
        assert!(temporary.merge_unbound(scope(), original, id(200), &final_pending));

        let replacement = temporary.begin_start(scope(), 2).unwrap();
        assert!(temporary.complete(scope(), original, id(200), &base(&[1, 2], None)).is_none());
        assert!(matches!(
            temporary.complete(scope(), replacement, id(201), &base(&[1, 2], None)),
            Some(RosterCompletion::Initialized { .. })
        ));
        let snapshot = temporary.snapshot(scope(), 2).unwrap();
        assert_eq!(snapshot.session, id(201));
        assert!(
            snapshot.participants.contains(&id(2)),
            "the old final session's LEAVE cannot affect this incarnation"
        );
    }

    #[test]
    fn conference_roster_state_fixed_acceptance_cannot_be_retargeted_by_a_start_retry() {
        let mut roster = roster();
        assert!(roster.observe(scope(), &delta(2, Some("LEAVE"), None), 1));
        let acceptance = roster.begin_acceptance(scope(), 1).unwrap();
        let switched = roster.begin_start(scope(), 1).unwrap();
        assert!(roster.complete(scope(), acceptance, id(100), &base(&[1, 2], None)).is_none());
        assert!(roster.complete(scope(), switched, id(200), &base(&[1, 2], None)).is_none());
        assert!(matches!(
            roster.complete(scope(), switched, id(100), &base(&[1, 2], None)),
            Some(RosterCompletion::Initialized { .. })
        ));
        assert!(!roster.snapshot(scope(), 1).unwrap().participants.contains(&id(2)));
    }

    #[test]
    fn conference_roster_state_pending_and_member_overflow_never_truncate_or_recover_from_a_new_cookie() {
        let mut roster = SessionRosterAuthority::new(scope(), id(100), 2, 1).unwrap();
        assert!(roster.observe(scope(), &delta(1, None, Some(true)), 1));
        assert!(!roster.observe(scope(), &delta(2, Some("LEAVE"), None), 2));
        for _ in 0..2 {
            let capture = roster.begin_acceptance(scope(), 1).unwrap();
            assert!(matches!(
                roster.complete(scope(), capture, id(100), &base(&[1, 2], None)),
                Some(RosterCompletion::Unavailable { .. })
            ));
            assert!(roster.snapshot(scope(), 1).is_none());
        }
        let mut roster = SessionRosterAuthority::new(scope(), id(100), 2, 4).unwrap();
        let capture = roster.begin_acceptance(scope(), 1).unwrap();
        assert!(matches!(
            roster.complete(scope(), capture, id(100), &base(&[1, 2, 3], None)),
            Some(RosterCompletion::Unavailable { .. })
        ));
        assert!(roster.snapshot(scope(), 1).is_none());
    }

    #[test]
    fn conference_roster_state_live_membership_revision_and_info_only_updates_preserve_departure() {
        let (mut roster, _) = ready();
        let before = roster.snapshot(scope(), 1).unwrap().revision;
        assert!(roster.observe(scope(), &delta(3, Some("ENTER"), None), 1));
        let added = roster.snapshot(scope(), 1).unwrap().revision;
        assert!(
            added > before,
            "an all-allowed roster change still invalidates a captured native send policy"
        );
        assert!(roster.observe(scope(), &delta(3, Some("LEAVE"), None), 2));
        let removed = roster.snapshot(scope(), 1).unwrap().revision;
        assert!(removed > added);
        assert!(roster.observe(scope(), &delta(3, None, Some(false)), 3));
        let snapshot = roster.snapshot(scope(), 1).unwrap();
        assert!(!snapshot.participants.contains(&id(3)));
        assert_eq!(
            snapshot.revision, removed,
            "other-member info is not an ENTER or our moderator permission"
        );
    }

    #[test]
    fn conference_roster_state_repeated_acceptance_cannot_overwrite_live_deltas_with_an_old_bootstrap() {
        let (mut roster, old) = ready();
        assert!(roster.observe(scope(), &delta(1, None, Some(true)), 1));
        assert!(roster.observe(scope(), &delta(2, Some("LEAVE"), None), 2));
        let revision = roster.snapshot(scope(), 1).unwrap().revision;
        let repeated = roster.begin_acceptance(scope(), 1).unwrap();
        assert_ne!(old.request(), repeated.request());
        assert_eq!(
            roster.complete(scope(), repeated, id(100), &base(&[1, 2], Some(false))),
            Some(RosterCompletion::AcknowledgedExisting)
        );
        let snapshot = roster.snapshot(scope(), 1).unwrap();
        assert_eq!(snapshot.self_text_mute, Some(true));
        assert!(!snapshot.participants.contains(&id(2)));
        assert_eq!(snapshot.revision, revision);
    }

    #[test]
    fn conference_roster_state_retirement_failure_and_older_incarnation_never_resurrect_authority() {
        let (mut roster, _) = ready();
        roster.retire();
        assert!(roster.snapshot(scope(), 1).is_none());
        assert!(roster.begin_acceptance(scope(), 1).is_none());
        let next = roster.begin_acceptance(scope(), 2).unwrap();
        assert!(roster.begin_acceptance(scope(), 1).is_none());
        assert!(roster.fail(scope(), next));
        assert!(!roster.fail(scope(), next));
        assert!(roster.complete(scope(), next, id(100), &base(&[1, 2], None)).is_none());
        assert!(roster.begin_acceptance(scope(), 2).is_none());
        assert!(roster.begin_acceptance(scope(), 0).is_none());
        assert!(NativeSessionScope::new(Uuid::nil(), 7).is_none());
        assert!(NativeSessionScope::new(id(1), 0).is_none());
    }

    #[test]
    fn conference_roster_state_unqualified_bases_and_out_of_order_deltas_cannot_replace_a_complete_roster() {
        let (mut roster, _) = ready();
        assert!(!roster.observe(scope(), &base(&[1, 3], None), 1));
        assert!(roster.snapshot(scope(), 1).is_none());
        let (mut roster, _) = ready();
        assert!(roster.observe(scope(), &delta(2, Some("LEAVE"), None), 2));
        assert!(!roster.observe(scope(), &delta(2, Some("ENTER"), None), 1));
        assert!(roster.snapshot(scope(), 1).is_none());
    }

    #[test]
    fn conference_roster_state_live_sender_only_suspends_and_an_absent_leave_resolves_with_a_new_revision() {
        let (mut roster, _) = ready();
        let before = roster.snapshot(scope(), 1).unwrap().revision;
        assert!(roster.observe_sender(scope(), id(3), 1));
        assert!(roster.snapshot(scope(), 1).is_none());
        assert!(!roster.participants.as_ref().unwrap().contains(&id(3)), "text cannot add a member");
        assert!(roster.observe(scope(), &delta(3, None, None), 2));
        assert!(
            roster.snapshot(scope(), 1).is_none(),
            "info-only data cannot resolve missing membership"
        );
        assert!(roster.observe(scope(), &delta(3, Some("LEAVE"), None), 3));
        let after = roster.snapshot(scope(), 1).unwrap();
        assert!(after.revision > before, "suspend→resolve cannot restore the old policy stamp");
        assert!(!after.participants.contains(&id(3)));
    }

    #[test]
    fn conference_roster_state_prebase_and_temp_final_sender_leave_order_never_invents_completeness() {
        for merge in [false, true] {
            for sender_after_leave in [false, true] {
                let mut temporary = roster();
                let capture = temporary.begin_start(scope(), 1).unwrap();
                let mut final_pending = SessionRosterAuthority::new(scope(), id(200), 4, 8).unwrap();
                let (sender_order, leave_order) = if sender_after_leave { (2, 1) } else { (1, 2) };
                if merge {
                    assert!(temporary.observe_sender(scope(), id(3), sender_order));
                    assert!(final_pending.observe(scope(), &delta(3, Some("LEAVE"), None), leave_order));
                    assert!(temporary.merge_unbound(scope(), capture, id(200), &final_pending));
                } else if sender_after_leave {
                    assert!(temporary.observe(scope(), &delta(3, Some("LEAVE"), None), 1));
                    assert!(temporary.observe_sender(scope(), id(3), 2));
                } else {
                    assert!(temporary.observe_sender(scope(), id(3), 1));
                    assert!(temporary.observe(scope(), &delta(3, Some("LEAVE"), None), 2));
                }
                let target = if merge { id(200) } else { id(100) };
                temporary.complete(scope(), capture, target, &base(&[1, 2, 3], None));
                assert_eq!(temporary.snapshot(scope(), 1).is_none(), sender_after_leave);
                if sender_after_leave {
                    assert!(temporary.observe(scope(), &delta(3, Some("ENTER"), None), 3));
                    assert!(temporary.snapshot(scope(), 1).unwrap().participants.contains(&id(3)));
                }
            }
        }
    }

    #[test]
    fn conference_roster_state_sender_observation_overflow_cannot_be_evicted_or_repaired_by_a_cookie() {
        let mut roster = roster();
        let capture = roster.begin_acceptance(scope(), 1).unwrap();
        for number in 3..7 {
            assert!(roster.observe_sender(scope(), id(number), number as u64));
        }
        assert!(!roster.observe_sender(scope(), id(7), 7));
        assert!(matches!(
            roster.complete(scope(), capture, id(100), &base(&[1, 2], None)),
            Some(RosterCompletion::Unavailable { .. })
        ));
        let retry = roster.begin_acceptance(scope(), 1).unwrap();
        roster.complete(scope(), retry, id(100), &base(&[1, 2], None));
        assert!(roster.snapshot(scope(), 1).is_none());
    }
}
