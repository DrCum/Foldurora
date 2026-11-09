//! Broker-owned, default-disabled MCP metadata projection. No listener starts
//! until native enablement, current-profile grants and approval UI are connected.
//! Native account/grid display authority is absent: production publishes none.
use super::{slots::SlotId, transport::HostSnapshot};
use aurora_mcp::authority::{Authority, AuthorityError, MetadataLabel, NativePermittedDisplay, ProfileId, ProfilePhase, ProfileProjection};
use aurora_session::{Binding, HostId, InstallationId, Phase};

/// Lifetime belongs to the authoritative broker, never the GUI's cached Arc.
/// There is deliberately no public route to enable or grant this foundation.
pub(super) struct Publication {
    authority: Authority,
}

impl Publication {
    pub(super) fn new(installation: InstallationId, host: HostId) -> Self {
        Self {
            authority: Authority::new(installation, host),
        }
    }

    pub(super) fn observe(&self, snapshot: &HostSnapshot, closing: bool) {
        if closing || publish_fresh(&self.authority, snapshot, &[]).is_err() {
            // Reject the previous publication immediately if the new observation
            // cannot establish its identity. Never keep serving a stale grant.
            let _ = self.authority.stop();
        }
    }
}

impl Drop for Publication {
    fn drop(&mut self) {
        let _ = self.authority.stop();
    }
}

/// Supplied by the native permission owner for this exact fresh observation.
/// Host worker names alone do not establish RLVa/privacy permission.
pub(super) struct PermittedDisplay {
    pub slot: SlotId,
    pub binding: Binding,
    pub display: NativePermittedDisplay,
}

/// Invoke only from the live transport publication path. Polling an old Arc
/// would renew expired authority and is expressly not a valid caller.
pub(super) fn publish_fresh(authority: &Authority, snapshot: &HostSnapshot, permitted: &[PermittedDisplay]) -> Result<(), AuthorityError> {
    if snapshot.stopped || snapshot.registry_error.is_some() {
        return authority.stop();
    }
    let mut profiles = Vec::with_capacity(snapshot.slots.len());
    for slot in &snapshot.slots {
        let workers: Vec<_> = snapshot.workers.iter().filter(|worker| worker.slot == Some(slot.id)).collect();
        if workers.len() > 1 {
            return Err(AuthorityError::Invalid);
        }
        let worker = workers.first().copied();
        let phase = match worker {
            None if slot.available => ProfilePhase::Stopped,
            None => ProfilePhase::Unavailable,
            Some(worker) if !worker.reachable => ProfilePhase::Unavailable,
            Some(worker) => match worker.status.as_ref().map(|status| status.phase) {
                Some(Phase::Online) if worker.binding.session.is_some() => ProfilePhase::Online,
                Some(Phase::Login | Phase::Connecting) => ProfilePhase::Starting,
                _ => ProfilePhase::Unavailable,
            },
        };
        let native_display = worker.filter(|_| phase == ProfilePhase::Online).and_then(|worker| {
            permitted
                .iter()
                .find(|display| display.slot == slot.id && display.binding == worker.binding)
                .map(|d| d.display.clone())
        });
        profiles.push(ProfileProjection {
            id: ProfileId::new(slot.id.number())?,
            label: MetadataLabel::new(slot.label.as_str())?,
            phase,
            binding: worker.map(|worker| worker.binding),
            native_display,
        });
    }
    authority.publish(profiles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_host::{
        slots::{SlotLabel, SlotSummary},
        transport::WorkerSummary,
    };
    use aurora_mcp::{
        authority::{AuthorityLevel, HostPolicy},
        wire::{HostHandler, HostRequest},
    };
    use aurora_session::{AccountKey, GridKey, SessionId, SessionIdentity, WorkerId, WorkerIdentity, WorkerStatus};
    use serde_json::{Value, json};

    fn fixture() -> (Publication, HostSnapshot) {
        let installation = InstallationId::from_bytes([1; 32]).unwrap();
        let host = HostId::from_bytes([2; 16]).unwrap();
        let slot = SlotId::try_from(0).unwrap();
        let binding = Binding {
            worker: WorkerIdentity {
                installation,
                host,
                worker: WorkerId::from_bytes([3; 16]).unwrap(),
            },
            session: Some(SessionIdentity {
                session: SessionId::from_bytes([4; 16]).unwrap(),
                account: AccountKey::from_bytes([5; 32]).unwrap(),
                grid: GridKey::from_bytes([6; 32]).unwrap(),
                generation: 1,
            }),
        };
        let snapshot = HostSnapshot {
            slots: vec![SlotSummary {
                id: slot,
                label: SlotLabel::new("Synthetic profile").unwrap(),
                available: false,
            }],
            workers: vec![WorkerSummary {
                slot: Some(slot),
                binding,
                status: Some(WorkerStatus {
                    phase: Phase::Online,
                    ..Default::default()
                }),
                reachable: true,
                label: "Raw private account".into(),
                grid: "Raw private grid".into(),
            }],
            ..Default::default()
        };
        (Publication::new(installation, host), snapshot)
    }

    fn permit(publication: &Publication) {
        publication
            .authority
            .configure(HostPolicy {
                enabled: true,
                metadata: AuthorityLevel::Allow,
                actions: AuthorityLevel::Never,
            })
            .unwrap();
        let ticket = publication.authority.ticket(ProfileId::new(1).unwrap()).unwrap();
        publication
            .authority
            .grant(ticket, AuthorityLevel::Allow, AuthorityLevel::Never)
            .unwrap();
    }

    fn list(publication: &Publication) -> Value {
        publication.authority.request(1, HostRequest::SessionsList)
    }

    #[test]
    fn host_mcp_broker_projection_defaults_off_and_omits_raw_account_grid_names() {
        let (publication, snapshot) = fixture();
        publication.observe(&snapshot, false);
        assert_eq!(list(&publication)["error_code"], "host_disabled");
        permit(&publication);
        let response = list(&publication);
        let rows = response["sessions"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["label"], "Synthetic profile");
        assert_eq!(rows[0]["phase"], "online");
        assert!(rows[0].get("account").is_none());
        assert!(rows[0].get("grid").is_none());
        assert!(!response.to_string().contains("Raw private"));
    }

    #[test]
    fn host_mcp_invalid_broker_projection_revokes_prior_authority_immediately() {
        let (publication, mut snapshot) = fixture();
        publication.observe(&snapshot, false);
        permit(&publication);
        assert_eq!(list(&publication)["sessions"].as_array().unwrap().len(), 1);
        snapshot.workers.push(snapshot.workers[0].clone());
        publication.observe(&snapshot, false);
        assert_eq!(list(&publication)["error_code"], "host_disabled");
        assert_eq!(
            publication.authority.ticket(ProfileId::new(1).unwrap()),
            Err(AuthorityError::StaleSource)
        );
    }

    #[test]
    fn host_mcp_login_replacement_and_unreachable_source_require_new_grants() {
        let (publication, mut snapshot) = fixture();
        publication.observe(&snapshot, false);
        permit(&publication);
        let old = publication.authority.ticket(ProfileId::new(1).unwrap()).unwrap();
        snapshot.workers[0].binding.session.as_mut().unwrap().generation += 1;
        publication.observe(&snapshot, false);
        assert_eq!(list(&publication)["sessions"], json!([]));
        assert_eq!(
            publication.authority.grant(old, AuthorityLevel::Allow, AuthorityLevel::Never),
            Err(AuthorityError::StaleSource)
        );
        permit(&publication);
        snapshot.workers[0].reachable = false;
        publication.observe(&snapshot, false);
        assert_eq!(list(&publication)["sessions"], json!([]));
        snapshot.workers[0].reachable = true;
        publication.observe(&snapshot, false);
        assert_eq!(list(&publication)["sessions"], json!([]));
    }

    #[test]
    fn host_mcp_closing_stopped_and_registry_failure_retire_publication() {
        for (closing, stopped, registry_error) in [(true, false, None), (false, true, None), (false, false, Some("synthetic".into()))] {
            let (publication, mut snapshot) = fixture();
            publication.observe(&snapshot, false);
            permit(&publication);
            snapshot.stopped = stopped;
            snapshot.registry_error = registry_error;
            publication.observe(&snapshot, closing);
            assert_eq!(list(&publication)["error_code"], "host_disabled");
        }
    }

    #[test]
    fn host_mcp_native_display_requires_exact_current_binding_and_fresh_permission() {
        let (publication, mut snapshot) = fixture();
        let permitted = [PermittedDisplay {
            slot: snapshot.slots[0].id,
            binding: snapshot.workers[0].binding,
            display: NativePermittedDisplay {
                account: MetadataLabel::new("Permitted account").unwrap(),
                grid: MetadataLabel::new("Permitted grid").unwrap(),
            },
        }];
        publish_fresh(&publication.authority, &snapshot, &permitted).unwrap();
        permit(&publication);
        assert_eq!(list(&publication)["sessions"][0]["account"], "Permitted account");
        publication.observe(&snapshot, false);
        assert!(list(&publication)["sessions"][0].get("account").is_none());
        snapshot.workers[0].binding.session.as_mut().unwrap().generation += 1;
        publish_fresh(&publication.authority, &snapshot, &permitted).unwrap();
        permit(&publication);
        assert!(list(&publication)["sessions"][0].get("account").is_none());
    }
}
