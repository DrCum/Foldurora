use super::*;
use aurora_session::{AccountKey, GridKey, SessionId, SessionIdentity, WorkerId, WorkerIdentity};

fn identities() -> (InstallationId, HostId) {
    (InstallationId::from_bytes([1; 32]).unwrap(), HostId::from_bytes([2; 16]).unwrap())
}

fn profile(number: u8, generation: u64) -> ProfileProjection {
    let (installation, host) = identities();
    ProfileProjection {
        id: ProfileId::new(number).unwrap(),
        label: MetadataLabel::new(&format!("Synthetic character {number}")).unwrap(),
        phase: ProfilePhase::Online,
        binding: Some(Binding {
            worker: WorkerIdentity {
                installation,
                host,
                worker: WorkerId::from_bytes([number; 16]).unwrap(),
            },
            session: Some(SessionIdentity {
                session: SessionId::from_bytes([number + 10; 16]).unwrap(),
                account: AccountKey::from_bytes([number + 20; 32]).unwrap(),
                grid: GridKey::from_bytes([number + 30; 32]).unwrap(),
                generation,
            }),
        }),
        native_display: None,
    }
}

fn authority(metadata: AuthorityLevel) -> Authority {
    let (installation, host) = identities();
    let authority = Authority::new(installation, host);
    authority.publish(vec![profile(1, 1), profile(2, 1)]).unwrap();
    authority
        .configure(HostPolicy {
            enabled: true,
            metadata,
            actions: AuthorityLevel::Never,
        })
        .unwrap();
    authority
}

fn grant(authority: &Authority, number: u8, metadata: AuthorityLevel) {
    let ticket = authority.ticket(ProfileId::new(number).unwrap()).unwrap();
    authority.grant(ticket, metadata, AuthorityLevel::Never).unwrap();
}

fn list(authority: &Authority, client: u64) -> Value {
    authority.request(client, HostRequest::SessionsList)
}

fn poll(authority: &Authority, client: u64, pending: &Value) -> Value {
    authority.request(
        client,
        HostRequest::OperationStatus {
            request_id: pending["request_id"].as_str().unwrap().to_owned(),
        },
    )
}

fn status(row: &Value) -> HostRequest {
    HostRequest::SessionStatus {
        profile_id: row["profile_id"].as_str().unwrap().to_owned(),
        profile_revision: row["profile_revision"].as_u64().unwrap(),
        session_ref: row["session_ref"].as_str().map(str::to_owned),
    }
}

#[test]
fn disabled_by_default_and_independent_host_profile_metadata_action_authority() {
    let (installation, host) = identities();
    let authority = Authority::new(installation, host);
    authority.publish(vec![profile(1, 1)]).unwrap();
    grant(&authority, 1, AuthorityLevel::Allow);
    assert_eq!(list(&authority, 1), error("host_disabled"));
    authority
        .configure(HostPolicy {
            enabled: true,
            metadata: AuthorityLevel::Never,
            actions: AuthorityLevel::Allow,
        })
        .unwrap();
    assert_eq!(list(&authority, 1), error("permission_denied"));
    authority
        .configure(HostPolicy {
            enabled: true,
            metadata: AuthorityLevel::Allow,
            actions: AuthorityLevel::Never,
        })
        .unwrap();
    assert_eq!(list(&authority, 1)["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(authority.policy().unwrap().actions, AuthorityLevel::Never);
    authority.revoke(ProfileId::new(1).unwrap()).unwrap();
    assert_eq!(list(&authority, 1)["sessions"], json!([]));
}

#[test]
fn list_filters_unauthorized_profiles_and_projection_never_exports_bindings() {
    let authority = authority(AuthorityLevel::Allow);
    grant(&authority, 1, AuthorityLevel::Allow);
    let result = list(&authority, 1);
    let rows = result["sessions"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["profile_id"], "1");
    assert_eq!(rows[0].as_object().unwrap().len(), 5);
    let text = result.to_string();
    for secret_name in ["binding", "installation", "worker", "generation", "account", "grid", "chat", "path"] {
        assert!(!text.contains(&format!("\"{secret_name}\"")), "{text}");
    }
    assert!(!text.contains(&profile(1, 1).binding.unwrap().session.unwrap().session.to_string()));
    assert_eq!(
        authority.request(
            1,
            HostRequest::SessionStatus {
                profile_id: "2".into(),
                profile_revision: 1,
                session_ref: None
            }
        ),
        error("permission_denied")
    );
}

#[test]
fn ask_remains_pending_until_real_approval_then_delivers_exactly_once() {
    let authority = authority(AuthorityLevel::Ask);
    grant(&authority, 1, AuthorityLevel::Allow);
    let pending = list(&authority, 1);
    assert_eq!(pending["status"], "pending");
    assert_eq!(pending.as_object().unwrap().len(), 2);
    for _ in 0..3 {
        assert_eq!(poll(&authority, 1, &pending), pending);
    }
    let approvals = authority.pending_approvals().unwrap();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].profiles.len(), 1);
    assert_eq!(approvals[0].profiles[0].profile, ProfileId::new(1).unwrap());
    assert_eq!(approvals[0].kind, ReadKind::SessionsList);
    authority.resolve(&approvals[0].request_id, ApprovalDecision::AllowOnce).unwrap();
    let result = poll(&authority, 1, &pending);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(poll(&authority, 1, &pending), error("operation_consumed"));
    assert_eq!(list(&authority, 1)["status"], "pending", "AllowOnce must not change Ask policy");
}

#[test]
fn profile_ask_cannot_be_overridden_by_global_allow_or_action_allow() {
    let authority = authority(AuthorityLevel::Allow);
    let ticket = authority.ticket(ProfileId::new(1).unwrap()).unwrap();
    authority.grant(ticket, AuthorityLevel::Ask, AuthorityLevel::Allow).unwrap();
    let pending = list(&authority, 1);
    assert_eq!(pending["status"], "pending");
    authority
        .resolve(pending["request_id"].as_str().unwrap(), ApprovalDecision::Deny)
        .unwrap();
    assert_eq!(poll(&authority, 1, &pending), error("permission_denied"));
    assert_eq!(
        authority.resolve(pending["request_id"].as_str().unwrap(), ApprovalDecision::AllowOnce),
        Err(AuthorityError::FinalOperation)
    );
}

#[test]
fn client_scoped_refs_and_approval_results_cannot_be_reused_by_another_peer() {
    let authority = authority(AuthorityLevel::Allow);
    grant(&authority, 1, AuthorityLevel::Allow);
    let first = list(&authority, 1);
    let second = list(&authority, 2);
    let first_row = &first["sessions"][0];
    assert_ne!(first_row["session_ref"], second["sessions"][0]["session_ref"]);
    assert_eq!(authority.request(2, status(first_row)), error("stale_source"));
    assert_eq!(authority.request(1, status(first_row))["status"], "ok");
    authority
        .configure(HostPolicy {
            enabled: true,
            metadata: AuthorityLevel::Ask,
            actions: AuthorityLevel::Never,
        })
        .unwrap();
    let pending = list(&authority, 1);
    assert_eq!(poll(&authority, 2, &pending), error("unknown_operation"));
}

#[test]
fn repeated_lists_reuse_bounded_refs_and_disconnect_retires_every_operation() {
    let authority = authority(AuthorityLevel::Allow);
    grant(&authority, 1, AuthorityLevel::Allow);
    let first = list(&authority, 1);
    for _ in 0..20 {
        assert_eq!(list(&authority, 1), first);
    }
    assert_eq!(authority.lock().unwrap().clients[&1].refs.len(), 1);
    authority
        .configure(HostPolicy {
            enabled: true,
            metadata: AuthorityLevel::Ask,
            actions: AuthorityLevel::Never,
        })
        .unwrap();
    let pending = list(&authority, 1);
    authority.disconnected(1);
    assert_eq!(poll(&authority, 1, &pending), error("unknown_operation"));
    assert!(authority.pending_approvals().unwrap().is_empty());
    authority
        .configure(HostPolicy {
            enabled: true,
            metadata: AuthorityLevel::Allow,
            actions: AuthorityLevel::Never,
        })
        .unwrap();
    assert_ne!(
        list(&authority, 1)["sessions"][0]["session_ref"],
        first["sessions"][0]["session_ref"]
    );
}

#[test]
fn every_binding_component_and_stable_slot_replacement_require_new_authorization() {
    let authority = authority(AuthorityLevel::Allow);
    grant(&authority, 1, AuthorityLevel::Allow);
    let old = list(&authority, 1);
    let old_ticket = authority.ticket(ProfileId::new(1).unwrap()).unwrap();
    let mut replacement = profile(1, 2);
    replacement.label = MetadataLabel::new("Same persistent slot").unwrap();
    authority.publish(vec![replacement.clone(), profile(2, 1)]).unwrap();
    assert_eq!(list(&authority, 1)["sessions"], json!([]));
    assert_eq!(
        authority.grant(old_ticket, AuthorityLevel::Allow, AuthorityLevel::Allow),
        Err(AuthorityError::StaleSource)
    );
    grant(&authority, 1, AuthorityLevel::Allow);
    assert_eq!(authority.request(1, status(&old["sessions"][0])), error("stale_source"));
    let mut changes = Vec::new();
    let mut binding = replacement.binding.unwrap();
    binding.worker.worker = WorkerId::from_bytes([99; 16]).unwrap();
    changes.push(binding);
    binding = replacement.binding.unwrap();
    binding.session.as_mut().unwrap().session = SessionId::from_bytes([99; 16]).unwrap();
    changes.push(binding);
    binding = replacement.binding.unwrap();
    binding.session.as_mut().unwrap().account = AccountKey::from_bytes([99; 32]).unwrap();
    changes.push(binding);
    binding = replacement.binding.unwrap();
    binding.session.as_mut().unwrap().grid = GridKey::from_bytes([99; 32]).unwrap();
    changes.push(binding);
    for binding in changes {
        replacement.binding = Some(binding);
        authority.publish(vec![replacement.clone()]).unwrap();
        assert_eq!(list(&authority, 1)["sessions"], json!([]));
        grant(&authority, 1, AuthorityLevel::Allow);
    }
}

#[test]
fn stopped_profile_grant_does_not_authorize_next_launch_or_login() {
    let authority = authority(AuthorityLevel::Allow);
    let mut stopped = profile(1, 1);
    stopped.phase = ProfilePhase::Stopped;
    stopped.binding = None;
    authority.publish(vec![stopped.clone()]).unwrap();
    grant(&authority, 1, AuthorityLevel::Allow);
    assert!(list(&authority, 1)["sessions"][0]["session_ref"].is_null());
    stopped.phase = ProfilePhase::Starting;
    authority.publish(vec![stopped]).unwrap();
    assert_eq!(list(&authority, 1)["sessions"], json!([]));
    authority.publish(vec![profile(1, 1)]).unwrap();
    assert_eq!(list(&authority, 1)["sessions"], json!([]));
}

#[test]
fn revoked_pending_or_approved_read_never_retains_payload() {
    for approve in [false, true] {
        let authority = authority(AuthorityLevel::Ask);
        grant(&authority, 1, AuthorityLevel::Allow);
        let pending = list(&authority, 1);
        if approve {
            authority
                .resolve(pending["request_id"].as_str().unwrap(), ApprovalDecision::AllowOnce)
                .unwrap();
        }
        authority.revoke(ProfileId::new(1).unwrap()).unwrap();
        assert_eq!(poll(&authority, 1, &pending), error("access_revoked"));
        assert!(authority.pending_approvals().unwrap().is_empty());
    }
}

#[test]
fn final_delivery_checks_freshness_and_provider_gaps_require_new_opt_in() {
    let authority = authority(AuthorityLevel::Ask);
    grant(&authority, 1, AuthorityLevel::Allow);
    let pending = list(&authority, 1);
    authority
        .resolve(pending["request_id"].as_str().unwrap(), ApprovalDecision::AllowOnce)
        .unwrap();
    authority.lock().unwrap().updated = Some(Instant::now() - SNAPSHOT_FRESHNESS - Duration::from_millis(1));
    assert_eq!(poll(&authority, 1, &pending), error("access_revoked"));
    assert_eq!(list(&authority, 1), error("source_stale"));
    authority.publish(vec![profile(1, 1), profile(2, 1)]).unwrap();
    assert_eq!(authority.lock().unwrap().profiles[&ProfileId::new(1).unwrap()].grant, None);
}

#[test]
fn approval_expires_without_grant_and_capacity_is_bounded() {
    let authority = authority(AuthorityLevel::Ask);
    grant(&authority, 1, AuthorityLevel::Allow);
    let pending = list(&authority, 1);
    let id = pending["request_id"].as_str().unwrap().to_owned();
    authority.lock().unwrap().operations.get_mut(&id).unwrap().created = Instant::now() - APPROVAL_TIMEOUT;
    assert_eq!(poll(&authority, 1, &pending), error("approval_expired"));
    assert_eq!(
        authority.resolve(&id, ApprovalDecision::AllowOnce),
        Err(AuthorityError::FinalOperation)
    );
    for _ in 1..MAX_OPERATIONS {
        assert_eq!(list(&authority, 1)["status"], "pending");
    }
    assert_eq!(list(&authority, 1), error("capacity"));
    for client in 2..=MAX_CLIENTS as u64 {
        assert_eq!(list(&authority, client), error("capacity"));
    }
    assert_eq!(list(&authority, MAX_CLIENTS as u64 + 1), error("capacity"));
    assert_eq!(authority.lock().unwrap().clients.len(), MAX_CLIENTS);
    assert_eq!(authority.lock().unwrap().operations.len(), MAX_OPERATIONS);
}

#[test]
fn missing_native_permission_omits_names_and_later_privacy_revocation_omits_them() {
    let authority = authority(AuthorityLevel::Allow);
    grant(&authority, 1, AuthorityLevel::Allow);
    assert!(list(&authority, 1)["sessions"][0].get("account").is_none());
    let mut visible = profile(1, 1);
    visible.native_display = Some(NativePermittedDisplay {
        account: MetadataLabel::new("Synthetic resident").unwrap(),
        grid: MetadataLabel::new("Synthetic grid").unwrap(),
    });
    authority.publish(vec![visible]).unwrap();
    assert_eq!(list(&authority, 1)["sessions"][0]["account"], "Synthetic resident");
    authority.publish(vec![profile(1, 1)]).unwrap();
    assert!(list(&authority, 1)["sessions"][0].get("account").is_none());
}

#[test]
fn invalid_projections_do_not_refresh_freshness_or_expose_partial_state() {
    let authority = authority(AuthorityLevel::Allow);
    grant(&authority, 1, AuthorityLevel::Allow);
    let updated = authority.lock().unwrap().updated;
    let mut wrong_host = profile(1, 1);
    wrong_host.binding.as_mut().unwrap().worker.host = HostId::from_bytes([99; 16]).unwrap();
    assert_eq!(authority.publish(vec![wrong_host]), Err(AuthorityError::Invalid));
    let mut duplicate_source = profile(1, 1);
    duplicate_source.id = ProfileId::new(2).unwrap();
    assert_eq!(
        authority.publish(vec![profile(1, 1), duplicate_source]),
        Err(AuthorityError::Invalid)
    );
    assert_eq!(authority.publish(vec![profile(1, 1), profile(1, 1)]), Err(AuthorityError::Invalid));
    assert_eq!(authority.lock().unwrap().updated, updated);
    assert_eq!(list(&authority, 1)["sessions"].as_array().unwrap().len(), 1);
    assert!(MetadataLabel::new("line\nbreak").is_err());
    assert!(MetadataLabel::new(&"x".repeat(129)).is_err());
    assert!(ProfileId::new(0).is_err());
    assert!(ProfileId::new(6).is_err());
}

#[test]
fn shutdown_invalidates_approvals_and_old_refs_even_after_reenablement() {
    let authority = authority(AuthorityLevel::Ask);
    grant(&authority, 1, AuthorityLevel::Allow);
    let pending = list(&authority, 1);
    authority
        .resolve(pending["request_id"].as_str().unwrap(), ApprovalDecision::AllowOnce)
        .unwrap();
    authority.stop().unwrap();
    assert_eq!(poll(&authority, 1, &pending), error("host_disabled"));
    assert_eq!(list(&authority, 1), error("host_disabled"));
    authority.publish(vec![profile(1, 1)]).unwrap();
    authority
        .configure(HostPolicy {
            enabled: true,
            metadata: AuthorityLevel::Allow,
            actions: AuthorityLevel::Never,
        })
        .unwrap();
    assert_eq!(list(&authority, 1)["sessions"], json!([]));
}
