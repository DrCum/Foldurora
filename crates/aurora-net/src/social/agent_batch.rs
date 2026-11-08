//! Provenance for native session membership, separate from presentation.
//!
//! Folderstorm abbe3cb0 LLIMSpeakerMgr::setSpeakers/updateSpeakers and
//! llimview.cpp initialization apply a full base before retained deltas. The
//! legacy flattened vector cannot establish that base: missing and explicitly
//! empty lists differ, and silently skipped malformed IDs lose authority.
//! This parser changes no native packets or legacy presentation behavior.

use super::{SessionAgent, parse_session_agents};
use aurora_llsd::{Llsd, Map};
use std::collections::HashSet;
use uuid::Uuid;

/// Immutable native presentation entries with sealed roster provenance.
/// No caller can replace the entries while retaining a completeness witness.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionAgentBatch {
    agents: Vec<SessionAgent>,
    baseline_count: Option<usize>,
    valid: bool,
}

impl SessionAgentBatch {
    /// A synthetic/legacy vector retains native presentation, never authority.
    pub fn unproven(agents: Vec<SessionAgent>) -> Self {
        Self {
            agents,
            baseline_count: None,
            valid: false,
        }
    }

    pub fn agents(&self) -> &[SessionAgent] {
        &self.agents
    }

    pub fn into_agents(self) -> Vec<SessionAgent> {
        self.agents
    }

    /// Valid ordered data may contain deltas without a full baseline. This is
    /// not proof of an initialized session or a complete current recipient set.
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// Some(empty) is an explicit, validated empty base. None grants nothing.
    pub fn baseline(&self) -> Option<&[SessionAgent]> {
        if !self.valid {
            return None;
        }
        self.agents.get(..self.baseline_count?)
    }

    pub fn updates(&self) -> Option<&[SessionAgent]> {
        if !self.valid {
            return None;
        }
        self.agents.get(self.baseline_count.unwrap_or(0)..)
    }
}

/// `maximum_entries` is the caller's local authority-resource limit, not an SL
/// or Folderstorm participant limit. Overflow invalidates the whole witness;
/// it never turns a truncated roster into a complete one. Legacy presentation
/// entries remain identical to parse_session_agents, including on refusal.
pub fn parse_session_agent_batch(body: &Llsd, maximum_entries: usize) -> SessionAgentBatch {
    let agents = parse_session_agents(body);
    let mut batch = SessionAgentBatch::unproven(agents);
    let Some(map) = body.as_map() else {
        return batch;
    };
    let Ok(base) = baseline(map, maximum_entries) else {
        return batch;
    };
    let Ok(updates) = updates(map, maximum_entries) else {
        return batch;
    };
    let Some(total) = base.unwrap_or(0).checked_add(updates) else {
        return batch;
    };
    if total <= maximum_entries && total == batch.agents.len() {
        batch.baseline_count = base;
        batch.valid = true;
    }
    batch
}

fn baseline(body: &Map, maximum: usize) -> Result<Option<usize>, ()> {
    // Preserve native precedence, including a valid empty agent_info map.
    if let Some(Llsd::Map(info)) = body.get("agent_info") {
        if info.len() > maximum {
            return Err(());
        }
        let mut seen = HashSet::new();
        for (id, metadata) in info {
            if !seen.insert(agent_id(id)?) {
                return Err(());
            }
            validate_info(metadata)?;
        }
        return Ok(Some(info.len()));
    }
    if let Some(Llsd::Array(agents)) = body.get("agents") {
        if agents.len() > maximum {
            return Err(());
        }
        let mut seen = HashSet::new();
        for agent in agents {
            let id = agent.as_uuid();
            if id.is_nil() || !seen.insert(id) {
                return Err(());
            }
        }
        return Ok(Some(agents.len()));
    }
    if body.contains_key("agent_info") || body.contains_key("agents") {
        return Err(());
    }
    Ok(None)
}

fn updates(body: &Map, maximum: usize) -> Result<usize, ()> {
    if let Some(Llsd::Map(updates)) = body.get("agent_updates") {
        if updates.len() > maximum {
            return Err(());
        }
        let mut seen = HashSet::new();
        for (id, update) in updates {
            if !seen.insert(agent_id(id)?) {
                return Err(());
            }
            let map = update.as_map().ok_or(())?;
            if let Some(transition) = map.get("transition")
                && !matches!(transition, Llsd::String(value) if value == "ENTER" || value == "LEAVE")
            {
                return Err(());
            }
            if let Some(info) = map.get("info") {
                validate_info(info)?;
            }
        }
        return Ok(updates.len());
    }
    if let Some(Llsd::Map(updates)) = body.get("updates") {
        if updates.len() > maximum {
            return Err(());
        }
        let mut seen = HashSet::new();
        for (id, transition) in updates {
            if !seen.insert(agent_id(id)?) || !matches!(transition, Llsd::String(value) if value == "ENTER" || value == "LEAVE") {
                return Err(());
            }
        }
        return Ok(updates.len());
    }
    if body.contains_key("agent_updates") || body.contains_key("updates") {
        return Err(());
    }
    Ok(0)
}

fn agent_id(value: &str) -> Result<Uuid, ()> {
    let id = Uuid::parse_str(value).map_err(|_| ())?;
    if id.is_nil() { Err(()) } else { Ok(id) }
}

fn validate_info(value: &Llsd) -> Result<(), ()> {
    let info = value.as_map().ok_or(())?;
    if info.get("is_moderator").is_some_and(|value| !matches!(value, Llsd::Boolean(_))) {
        return Err(());
    }
    if let Some(mutes) = info.get("mutes") {
        let mutes = mutes.as_map().ok_or(())?;
        if mutes.get("text").is_some_and(|value| !matches!(value, Llsd::Boolean(_))) {
            return Err(());
        }
    }
    // Native future fields remain forward-compatible; they grant no authority.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    #[test]
    fn conference_roster_batch_distinguishes_empty_base_missing_and_unproven_vectors() {
        for body in [
            llsd_map! { "agent_info" => llsd_map! {} },
            llsd_map! { "agents" => Llsd::Array(Vec::new()) },
        ] {
            let batch = parse_session_agent_batch(&body, 0);
            assert!(batch.is_valid());
            assert_eq!(batch.baseline(), Some([].as_slice()));
            assert_eq!(batch.updates(), Some([].as_slice()));
        }
        let missing = parse_session_agent_batch(&llsd_map! {}, 0);
        assert!(missing.is_valid());
        assert!(missing.baseline().is_none());
        let vector = vec![SessionAgent {
            agent: Uuid::from_u128(10),
            present: Some(true),
            ..Default::default()
        }];
        let unproven = SessionAgentBatch::unproven(vector.clone());
        assert!(!unproven.is_valid());
        assert!(unproven.baseline().is_none() && unproven.updates().is_none());
        assert_eq!(unproven.into_agents(), vector);
        assert!(!SessionAgentBatch::default().is_valid());
    }

    #[test]
    fn conference_roster_batch_retains_full_base_then_ordered_membership_and_info_deltas() {
        let a = Uuid::from_u128(10);
        let b = Uuid::from_u128(20);
        let c = Uuid::from_u128(30);
        let body = llsd_map! {
            "agent_info" => llsd_map! {
                a.to_string() => llsd_map! { "mutes" => llsd_map! { "text" => false } },
                b.to_string() => llsd_map! {},
            },
            "agent_updates" => llsd_map! {
                a.to_string() => llsd_map! { "transition" => "LEAVE", "info" => llsd_map! { "mutes" => llsd_map! { "text" => true } } },
                c.to_string() => llsd_map! { "transition" => "ENTER" },
            },
        };
        let batch = parse_session_agent_batch(&body, 4);
        assert!(batch.is_valid());
        assert_eq!(batch.agents(), parse_session_agents(&body));
        let base = batch.baseline().unwrap();
        assert_eq!(base.len(), 2);
        assert_eq!(base[0].text_muted, Some(false));
        let updates = batch.updates().unwrap();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].agent, a);
        assert_eq!(updates[0].present, Some(false));
        assert_eq!(updates[0].text_muted, Some(true));
        assert!(!updates[0].entered && updates[1].entered);
        assert_eq!(updates[1].agent, c);
        let info_only = parse_session_agent_batch(
            &llsd_map! {
                "agent_updates" => llsd_map! { a.to_string() => llsd_map! { "info" => llsd_map! { "is_moderator" => true } } }
            },
            1,
        );
        assert!(info_only.is_valid() && info_only.baseline().is_none());
        assert_eq!(info_only.updates().unwrap()[0].present, None, "an info update is not an ENTER");
    }

    #[test]
    fn conference_roster_batch_invalid_shapes_ids_transitions_and_permissions_never_grant_a_base() {
        let a = Uuid::from_u128(10);
        for body in [
            Llsd::Undef,
            llsd_map! { "agent_info" => false },
            llsd_map! { "agents" => "not an array" },
            llsd_map! { "agent_info" => llsd_map! { "not-a-uuid" => llsd_map! {} } },
            llsd_map! { "agents" => Llsd::Array(vec![Uuid::nil().into()]) },
            llsd_map! { "agents" => Llsd::Array(vec![a.into(), a.into()]) },
            llsd_map! { "agent_info" => llsd_map! { a.to_string() => false } },
            llsd_map! { "agent_info" => llsd_map! { a.to_string() => llsd_map! { "is_moderator" => "false" } } },
            llsd_map! { "agent_info" => llsd_map! { a.to_string() => llsd_map! { "mutes" => llsd_map! { "text" => 0 } } } },
            llsd_map! { "agent_updates" => false },
            llsd_map! { "agent_updates" => llsd_map! { a.to_string() => llsd_map! { "transition" => "unknown" } } },
            llsd_map! { "agent_updates" => llsd_map! { a.to_string() => llsd_map! { "info" => false } } },
            llsd_map! { "updates" => llsd_map! { a.to_string() => false } },
        ] {
            let batch = parse_session_agent_batch(&body, 4);
            assert!(!batch.is_valid(), "{body:?}");
            assert!(batch.baseline().is_none() && batch.updates().is_none());
            assert_eq!(batch.agents(), parse_session_agents(&body), "native presentation remains unchanged");
        }
    }

    #[test]
    fn conference_roster_batch_local_bound_is_exact_and_overflow_never_truncates_into_completeness() {
        let a = Uuid::from_u128(10);
        let b = Uuid::from_u128(20);
        let body = llsd_map! {
            "agents" => Llsd::Array(vec![a.into(), b.into()]),
            "updates" => llsd_map! { b.to_string() => "LEAVE" },
        };
        let exact = parse_session_agent_batch(&body, 3);
        assert!(exact.is_valid());
        assert_eq!(exact.baseline().unwrap().len(), 2);
        assert_eq!(exact.updates().unwrap().len(), 1);
        for limit in [0, 1, 2] {
            let refused = parse_session_agent_batch(&body, limit);
            assert!(!refused.is_valid());
            assert!(refused.baseline().is_none() && refused.updates().is_none());
            assert_eq!(refused.agents(), exact.agents());
        }
    }

    #[test]
    fn conference_roster_batch_native_shape_precedence_and_future_info_do_not_invent_deltas() {
        let a = Uuid::from_u128(10);
        let body = llsd_map! {
            "agent_info" => llsd_map! {},
            "agents" => Llsd::Array(vec![a.into()]),
            "agent_updates" => llsd_map! {},
            "updates" => llsd_map! { a.to_string() => "ENTER" },
        };
        let batch = parse_session_agent_batch(&body, 0);
        assert!(batch.is_valid());
        assert_eq!(batch.baseline(), Some([].as_slice()));
        assert!(batch.agents().is_empty());
        let future = parse_session_agent_batch(
            &llsd_map! {
                "agent_info" => llsd_map! { a.to_string() => llsd_map! { "future-field" => true } }
            },
            1,
        );
        assert!(future.is_valid());
        assert_eq!(future.baseline().unwrap()[0].moderator, None);
        assert_eq!(future.baseline().unwrap()[0].text_muted, None);
    }

    #[test]
    fn conference_roster_batch_voice_only_metadata_preserves_unknown_text_mute() {
        let a = Uuid::from_u128(10);
        let batch = parse_session_agent_batch(
            &llsd_map! {
                "agent_info" => llsd_map! { a.to_string() => llsd_map! { "mutes" => llsd_map! { "voice" => true } } }
            },
            1,
        );
        assert!(batch.is_valid());
        assert_eq!(batch.baseline().unwrap()[0].text_muted, None);
    }

    #[test]
    fn conference_roster_batch_uuid_alias_duplicates_cannot_supply_ambiguous_permission_order() {
        let a = Uuid::from_u128(10);
        for body in [
            llsd_map! { "agent_info" => llsd_map! {
                a.to_string() => llsd_map! { "mutes" => llsd_map! { "text" => false } },
                a.to_string().to_uppercase() => llsd_map! { "mutes" => llsd_map! { "text" => true } },
            } },
            llsd_map! { "agent_updates" => llsd_map! {
                a.to_string() => llsd_map! { "transition" => "ENTER" },
                a.to_string().to_uppercase() => llsd_map! { "transition" => "LEAVE" },
            } },
        ] {
            let batch = parse_session_agent_batch(&body, 2);
            assert!(!batch.is_valid());
            assert_eq!(batch.agents().len(), 2);
            assert_eq!(batch.agents(), parse_session_agents(&body));
        }
    }
}
