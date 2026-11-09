//! Broker-owned MCP listener, native opt-ins, approvals and fresh projections.
//! Default disabled; worker-native redaction decisions are exact-binding and expire.
use super::{slots::SlotId, transport::HostSnapshot};
use aurora_mcp::authority::{Authority, AuthorityError, MetadataLabel, NativePermittedDisplay, ProfileId, ProfilePhase, ProfileProjection};
use aurora_session::{Binding, HostId, InstallationId, Phase};

/// Native preferences contain no current-login grants or client credentials.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Preferences {
    pub enabled: bool,
    pub permissions: aurora_mcp::authority::PermissionLevels,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum UiCommand {
    Configure(Preferences),
    Grant(aurora_mcp::authority::ProfileTicket, aurora_mcp::authority::PermissionLevels),
    Revoke(aurora_mcp::authority::ProfileId),
    Resolve(String, aurora_mcp::authority::ApprovalDecision),
}

#[derive(Clone, Default)]
pub(super) struct UiState {
    pub installation: Option<InstallationId>,
    pub authority: Option<std::sync::Arc<Authority>>,
    pub preferences: Preferences,
    pub profiles: Vec<aurora_mcp::authority::ProfileAccess>,
    pub pending: Vec<aurora_mcp::authority::PendingApproval>,
    pub available: bool,
    pub error: bool,
}
impl UiState {
    pub fn same(&self, other: &Self) -> bool {
        self.preferences == other.preferences
            && self.available == other.available
            && self.error == other.error
            && self
                .profiles
                .iter()
                .map(|p| (p.ticket, p.permissions))
                .eq(other.profiles.iter().map(|p| (p.ticket, p.permissions)))
            && self
                .pending
                .iter()
                .map(|p| &p.request_id)
                .eq(other.pending.iter().map(|p| &p.request_id))
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPreferences {
    version: u16,
    preferences: Preferences,
}

/// Listener, preference writes and authority belong to the actual host broker.
pub(super) struct Publication {
    authority: std::sync::Arc<Authority>,
    preferences: Preferences,
    path: Option<std::path::PathBuf>,
    listener: Option<aurora_mcp::ipc::Server>,
    installation: InstallationId,
    error: bool,
}
impl Publication {
    pub(super) fn new(installation: InstallationId, host: HostId) -> Self {
        let mut path = super::profile::host_root().ok().map(|root| {
            root.join("registries")
                .join(installation.to_string())
                .join("host-mcp-preferences.json")
        });
        let mut error = false;
        let preferences = match path
            .as_ref()
            .map(|path| super::transport::read_json::<StoredPreferences>(path, 2048))
        {
            Some(Ok(stored)) if stored.version == 1 => stored.preferences,
            Some(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => Preferences::default(),
            _ => {
                error = true;
                path = None;
                Preferences::default()
            }
        };
        let authority = std::sync::Arc::new(Authority::new(installation, host));
        let mut supported = vec![aurora_mcp::wire::HostAction::OpenLogin, aurora_mcp::wire::HostAction::Detach];
        if cfg!(windows) {
            use aurora_mcp::wire::HostAction;
            supported.extend([
                HostAction::Activate,
                HostAction::Logout,
                HostAction::Restart,
                HostAction::BackgroundMode {
                    mode: aurora_mcp::wire::BackgroundMode::Warm,
                },
                HostAction::MonitorOpen,
                HostAction::MonitorClose { monitor: 1 },
                HostAction::MonitorsOpenAll,
                HostAction::MonitorsArrange,
                HostAction::MonitorCycleConfigure {
                    interval_seconds: 2,
                    playing: false,
                },
                HostAction::MonitorCyclePause,
                HostAction::MonitorCycleNext,
            ]);
        }
        let _ = authority.support_actions(supported);
        let mut this = Self {
            authority,
            preferences,
            path,
            listener: None,
            installation,
            error,
        };
        this.configure_listener();
        this
    }

    fn configure_listener(&mut self) {
        // Revoke before dropping a listener; disconnects cannot renew authority.
        let _ = self
            .authority
            .configure_permissions(self.preferences.enabled, self.preferences.permissions);
        if !self.preferences.enabled {
            self.listener = None;
            return;
        }
        if self.listener.is_none() {
            self.listener = aurora_mcp::ipc::Discovery::production(self.installation)
                .and_then(|discovery| aurora_mcp::ipc::Server::start(discovery, true, self.authority.clone()))
                .ok();
            if self.listener.is_none() {
                self.error = true;
                let _ = self.authority.stop();
            }
        }
    }

    pub(super) fn command(&mut self, command: UiCommand) {
        let result = match command {
            UiCommand::Configure(preferences) => {
                let result = self
                    .path
                    .as_ref()
                    .ok_or_else(|| std::io::Error::other("host MCP preferences unavailable"))
                    .and_then(|path| super::slots::write_json(path, &StoredPreferences { version: 1, preferences }, 2048));
                if result.is_ok() {
                    self.preferences = preferences;
                    self.error = false;
                    self.configure_listener();
                }
                result.map_err(|_| AuthorityError::Unavailable)
            }
            UiCommand::Grant(ticket, levels) => self.authority.grant_permissions(ticket, levels),
            UiCommand::Revoke(profile) => self.authority.revoke(profile),
            UiCommand::Resolve(id, decision) => self.authority.resolve(&id, decision),
        };
        if result.is_err() {
            self.error = true;
        }
    }

    pub(super) fn observe(&mut self, snapshot: &HostSnapshot, closing: bool) {
        let owner = match snapshot.active {
            None => aurora_mcp::authority::ActiveOwner::None,
            Some(worker) => snapshot
                .workers
                .iter()
                .find(|source| source.binding.worker.worker == worker && source.reachable && source.binding.session.is_some())
                .map_or(aurora_mcp::authority::ActiveOwner::Unavailable, |source| {
                    aurora_mcp::authority::ActiveOwner::Bound(source.binding)
                }),
        };
        let ownership = self.authority.publish_ownership(aurora_mcp::authority::HostOwnership {
            active: owner,
            busy: snapshot.control_busy,
        });
        let permitted: Vec<_> = snapshot
            .native_observations
            .iter()
            .filter(|observation| observation.capabilities.display)
            .filter_map(|observation| {
                let display = observation.display.as_ref()?;
                let slot = snapshot
                    .workers
                    .iter()
                    .find(|worker| worker.binding == observation.binding && worker.reachable)?
                    .slot?;
                Some(PermittedDisplay {
                    slot,
                    binding: observation.binding,
                    display: NativePermittedDisplay {
                        account: MetadataLabel::new(display.account.as_str()).ok()?,
                        grid: MetadataLabel::new(display.grid.as_str()).ok()?,
                    },
                })
            })
            .collect();
        if closing || ownership.is_err() || publish_fresh(&self.authority, snapshot, &permitted).is_err() {
            let _ = self.authority.stop();
            self.listener = None;
            if !closing {
                self.error = true;
            }
        }
    }

    pub(super) fn authority(&self) -> &std::sync::Arc<Authority> {
        &self.authority
    }

    pub(super) fn ui(&self) -> UiState {
        UiState {
            installation: Some(self.installation),
            authority: Some(self.authority.clone()),
            preferences: self.preferences,
            profiles: self.authority.profile_access().unwrap_or_default(),
            pending: self.authority.pending_approvals().unwrap_or_default(),
            available: self.listener.is_some(),
            error: self.error,
        }
    }

    pub(super) fn dispatch(
        &self,
        request_id: &str,
        execute: impl FnOnce(&aurora_mcp::authority::ActionExecution) -> aurora_mcp::authority::ActionResult,
    ) {
        if matches!(
            self.authority.dispatch_submitted_action(request_id, execute),
            Err(AuthorityError::StaleSource)
        ) {
            let _ = self
                .authority
                .observe_action_result(request_id, aurora_mcp::authority::ActionResult::RejectedNative);
        }
    }

    pub(super) fn actions(&self, mut execute: impl FnMut(&aurora_mcp::authority::ActionExecution) -> aurora_mcp::authority::ActionResult) {
        for action in self
            .authority
            .ready_actions()
            .unwrap_or_default()
            .into_iter()
            .filter(|action| {
                action.action.class() != aurora_mcp::wire::PermissionClass::Monitors
                    && !matches!(
                        action.action,
                        aurora_mcp::wire::HostAction::Activate
                            | aurora_mcp::wire::HostAction::OpenLogin
                            | aurora_mcp::wire::HostAction::Detach
                            | aurora_mcp::wire::HostAction::Logout
                            | aurora_mcp::wire::HostAction::Restart
                    )
            })
            .take(16)
        {
            let _ = self.authority.execute_action(&action.request_id, |action| execute(action));
        }
    }
}
impl Drop for Publication {
    fn drop(&mut self) {
        let _ = self.authority.stop();
        self.listener = None;
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
        (
            Publication {
                authority: std::sync::Arc::new(Authority::new(installation, host)),
                preferences: Preferences::default(),
                path: None,
                listener: None,
                installation,
                error: false,
            },
            snapshot,
        )
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
        let (mut publication, snapshot) = fixture();
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
        let (mut publication, mut snapshot) = fixture();
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
        let (mut publication, mut snapshot) = fixture();
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
            let (mut publication, mut snapshot) = fixture();
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
        let (mut publication, mut snapshot) = fixture();
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
    #[test]
    fn presentation_actions_wait_for_native_gui_cover_instead_of_broker_poll_dispatch() {
        use aurora_mcp::{
            authority::PermissionLevels,
            wire::{HostAction, HostTarget, PermissionClass},
        };
        let (mut publication, mut snapshot) = fixture();
        snapshot.workers.clear();
        snapshot.slots[0].available = true;
        publication.observe(&snapshot, false);
        let mut levels = PermissionLevels::default();
        levels.set(PermissionClass::Metadata, AuthorityLevel::Allow);
        levels.set(PermissionClass::OpenLogin, AuthorityLevel::Allow);
        publication.authority.configure_permissions(true, levels).unwrap();
        let ticket = publication.authority.ticket(ProfileId::new(1).unwrap()).unwrap();
        publication.authority.grant_permissions(ticket, levels).unwrap();
        publication.authority.support_actions(vec![HostAction::OpenLogin]).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        assert_eq!(
            publication.authority.request(
                1,
                HostRequest::Action {
                    request_id: id,
                    target: Some(HostTarget {
                        profile_id: "1".into(),
                        profile_revision: ticket.revision,
                        session_ref: None
                    }),
                    monitor_revision: None,
                    action: HostAction::OpenLogin
                }
            )["status"],
            "queued"
        );
        publication.actions(|_| panic!("presentation must wait for Shell cover"));
        assert_eq!(publication.authority.ready_actions().unwrap().len(), 1);
    }
}
