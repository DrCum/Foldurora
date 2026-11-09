use super::*;
use aurora_session::{AccountKey, GridKey, InstallationId, SessionId, SessionIdentity, WorkerId, WorkerIdentity};
#[test]
fn typed_camera_image_requires_original_completed_capture_client_and_current_authority() {
    let e = engine(Level::Allow);
    let mut config = e.configuration().unwrap();
    config.permissions.set(Class::Camera, Level::Allow).unwrap();
    e.configure(config.clone()).unwrap();
    e.capabilities([Tool::CameraSnapshot]);
    let pending = invoke(&e, 1, Tool::CameraSnapshot);
    let id = pending["request_id"].as_str().unwrap();
    e.prepared(id, &[], false).unwrap();
    let capture = e.ready().pop().unwrap();
    e.execute(id, |_| json!({"status":"submitted"})).unwrap();
    let artifact = uuid::Uuid::new_v4();
    e.install_artifact(&capture, artifact, vec![1, 2, 3]).unwrap();
    e.install_camera_preview(&capture, artifact, 64, 48, vec![255, 216, 1, 2, 3, 255, 217])
        .unwrap();
    let read = ViewerRequest::Image {
        viewer_session: endpoint(),
        request_id: id.into(),
        artifact_id: artifact.to_string(),
    };
    assert_eq!(e.request(1, read.clone()), error("image_unavailable"));
    e.complete(&capture, json!({"artifact_id":artifact.to_string()})).unwrap();
    let response = e.request(1, read.clone());
    assert_eq!(response["data_base64"], "/9gBAgP/2Q==");
    assert!(serde_json::to_vec(&response).unwrap().len() <= MAX_RESULT_BYTES);
    assert_eq!(e.request(2, read.clone()), error("image_unavailable"));
    e.begin_native_update();
    assert_eq!(e.request(1, read.clone()), error("native_busy"));
    e.end_native_update();
    config.permissions.set(Class::Camera, Level::Never).unwrap();
    e.configure(config).unwrap();
    assert_eq!(e.request(1, read), error("image_unavailable"));
}
fn source(generation: u64) -> Binding {
    Binding {
        worker: WorkerIdentity {
            installation: InstallationId::from_bytes([1; 32]).unwrap(),
            host: HostId::from_bytes([2; 16]).unwrap(),
            worker: WorkerId::from_bytes([3; 16]).unwrap(),
        },
        session: Some(SessionIdentity {
            session: SessionId::from_bytes([4; 16]).unwrap(),
            account: AccountKey::from_bytes([5; 32]).unwrap(),
            grid: GridKey::from_bytes([6; 32]).unwrap(),
            generation,
        }),
    }
}
fn endpoint() -> HostId {
    HostId::from_bytes([7; 16]).unwrap()
}
fn engine(level: Level) -> Runtime {
    let e = Runtime::default();
    e.observe(Some(source(1)), true);
    let mut c = Configuration::default();
    c.permissions.set(Class::Edit, level).unwrap();
    e.configure(c).unwrap();
    e.capabilities([Tool::InventoryGet, Tool::InventoryRename, Tool::InventoryPurge]);
    e.enable(source(1), endpoint()).unwrap();
    e
}
fn invoke(e: &Runtime, client: u64, tool: Tool) -> Value {
    e.request(
        client,
        ViewerRequest::Invoke {
            viewer_session: endpoint(),
            tool,
            arguments: serde_json::Map::new(),
            request_id: None,
        },
    )
}
fn poll(e: &Runtime, client: u64, id: &str) -> Value {
    e.request(
        client,
        ViewerRequest::OperationStatus {
            viewer_session: endpoint(),
            request_id: id.into(),
        },
    )
}
fn prep(e: &Runtime, tool: Tool) -> String {
    let r = invoke(e, 1, tool);
    let id = r["request_id"].as_str().unwrap().to_owned();
    e.prepared(&id, &[], false).unwrap();
    id
}
#[test]
fn default_disabled_and_domain_separation() {
    let e = Runtime::default();
    e.observe(Some(source(1)), true);
    assert_eq!(invoke(&e, 1, Tool::InventoryGet), error("viewer_unavailable"));
    assert!(!e.snapshot().enabled);
    assert_eq!(
        crate::wire::HostHandler::request(&e, 1, crate::wire::HostRequest::SessionsList),
        error("wrong_authority_domain")
    );
}
#[test]
fn trusted_spending_review_remains_pending_even_when_classes_allow() {
    let e = engine(Level::Allow);
    let reply = invoke(&e, 1, Tool::InventoryRename);
    let id = reply["request_id"].as_str().unwrap();
    e.prepared_with_review(id, &[], false, true).unwrap();
    assert_eq!(poll(&e, 1, id)["status"], "pending_approval");
    assert!(e.ready().is_empty());
    assert!(e.execute(id, |_| panic!("spending review was self-approved")).is_err());
    e.resolve(id, true).unwrap();
    assert_eq!(e.ready().len(), 1);
}
#[test]
fn trusted_spending_review_cannot_override_never_or_disconnect() {
    let e = engine(Level::Allow);
    let reply = invoke(&e, 1, Tool::InventoryRename);
    let id = reply["request_id"].as_str().unwrap();
    e.prepared_with_review(id, &[Class::Upload], false, true).unwrap();
    assert_eq!(poll(&e, 1, id), error("permission_denied"));
    let e = engine(Level::Allow);
    let reply = invoke(&e, 1, Tool::InventoryRename);
    let id = reply["request_id"].as_str().unwrap();
    e.prepared_with_review(id, &[], false, true).unwrap();
    e.disconnected(1);
    assert!(e.resolve(id, true).is_err());
    assert!(e.ready().is_empty());
}
#[test]
fn factual_contents_reconciliation_never_revives_expired_action_authority() {
    let e = Runtime::default();
    e.observe(Some(source(1)), true);
    let mut config = Configuration::default();
    config.permissions.set(Class::TaskInventoryWrite, Level::Allow).unwrap();
    e.configure(config.clone()).unwrap();
    e.capabilities([Tool::ObjectContentsPut, Tool::ObjectContentsReconcile]);
    e.enable(source(1), endpoint()).unwrap();
    let put = prep(&e, Tool::ObjectContentsPut);
    let original = e.ready().pop().unwrap();
    e.execute(&put, |_| json!({"status":"submitted","copy_outcome":"unknown"})).unwrap();
    let reply = invoke(&e, 1, Tool::ObjectContentsReconcile);
    let id = reply["request_id"].as_str().unwrap();
    let preparing = e.preparations().pop().unwrap();
    let fact = json!({"status":"associated_unverified","copy_outcome":"unknown"});
    assert!(e.amend_reconciled(&original, &preparing, fact.clone()).is_err());
    e.prepared(id, &[], false).unwrap();
    let reconcile = e.ready().pop().unwrap();
    e.execute(id, |_| fact.clone()).unwrap();
    let mut other = reconcile.clone();
    other.client += 1;
    assert!(e.amend_reconciled(&original, &other, fact.clone()).is_err());
    e.amend_reconciled(&original, &reconcile, fact.clone()).unwrap();
    assert_eq!(poll(&e, 1, &put)["result"], fact);
    {
        let mut state = e.0.lock().unwrap();
        let old = state.operations.get_mut(&put).unwrap();
        old.created = Instant::now() - Duration::from_secs(80);
        old.stage = Stage::Denied("operation_expired");
    }
    e.amend_reconciled(&original, &reconcile, fact.clone()).unwrap();
    assert_eq!(poll(&e, 1, &put), error("operation_expired"));
    assert!(!e.capture_current(&original));
    assert!(e.continue_action(&original, || panic!("old effect authority restored")).is_err());
    // A new explicit native review may record facts after a policy revision;
    // it cannot make the previous actor readable or executable again.
    config.permissions.set(Class::Edit, Level::Allow).unwrap();
    e.configure(config).unwrap();
    let fresh = prep(&e, Tool::ObjectContentsReconcile);
    let capture = e.ready().pop().unwrap();
    e.execute(&fresh, |_| fact.clone()).unwrap();
    assert_ne!(original.policy_revision, capture.policy_revision);
    e.amend_reconciled(&original, &capture, fact).unwrap();
    assert_eq!(poll(&e, 1, &put), error("source_changed"));
    assert!(!e.capture_current(&original));
    assert!(
        e.continue_action(&original, || panic!("policy change reauthorized old copy"))
            .is_err()
    );
}
#[test]
fn invalid_deserialized_permissions_disable_old_allows_until_valid_configuration() {
    let e = engine(Level::Allow);
    let id = prep(&e, Tool::InventoryGet);
    let mut raw = serde_json::to_value(Configuration::default()).unwrap();
    // Deserialization intentionally preserves untrusted persisted values;
    // configure owns validation and must fail closed as a whole.
    raw["permissions"]["levels"]["read"] = json!("never");
    raw["permissions"]["levels"]["chat_read"] = json!("ask");
    let invalid: Configuration = serde_json::from_value(raw).unwrap();
    assert!(!invalid.permissions.validate());
    assert_eq!(e.configure(invalid), Err("invalid_permissions"));
    assert!(!e.snapshot().enabled);
    assert!(e.execute(&id, |_| panic!("old Allow survived invalid permissions")).is_err());
    assert_eq!(e.enable(source(1), endpoint()), Err("invalid_permissions"));
    assert_eq!(invoke(&e, 1, Tool::InventoryGet), error("viewer_unavailable"));
    let mut valid = Configuration::default();
    valid.permissions.set(Class::Read, Level::Never).unwrap();
    e.configure(valid).unwrap();
    e.enable(source(1), endpoint()).unwrap();
    assert_eq!(invoke(&e, 1, Tool::InventoryGet), error("permission_denied"));
}
#[test]
fn duplicate_invoke_cannot_deliver_finished_private_result_during_native_update() {
    let e = engine(Level::Allow);
    let id = uuid::Uuid::new_v4().to_string();
    let request = ViewerRequest::Invoke {
        viewer_session: endpoint(),
        tool: Tool::InventoryGet,
        arguments: serde_json::Map::new(),
        request_id: Some(id.clone()),
    };
    e.request(1, request.clone());
    e.prepared(&id, &[], false).unwrap();
    e.execute(&id, |_| json!({"status":"ok","private":"synthetic retained data"}))
        .unwrap();
    e.begin_native_update();
    assert_eq!(e.request(1, request.clone()), error("native_busy"));
    e.end_native_update();
    assert_eq!(e.request(1, request)["result"]["private"], "synthetic retained data");
}
#[test]
fn private_reads_stay_never_and_never_denies() {
    let e = engine(Level::Never);
    assert_eq!(invoke(&e, 1, Tool::InventoryRename), error("permission_denied"));
    for p in [
        crate::assistant::permissions::Preset::ReadOnly,
        crate::assistant::permissions::Preset::AskBeforeChanges,
    ] {
        let mut c = Configuration::default();
        c.permissions.preset(p);
        for d in crate::assistant::permissions::DEFINITIONS.iter().filter(|d| d.private_read) {
            assert_eq!(c.permissions.level(d.class), Level::Never);
        }
    }
}
#[test]
fn ask_requires_native_resolution_and_connection_owned_receipts() {
    let e = engine(Level::Ask);
    let id = prep(&e, Tool::InventoryRename);
    assert_eq!(poll(&e, 1, &id)["status"], "pending_approval");
    assert!(e.ready().is_empty());
    assert_eq!(poll(&e, 2, &id), error("unknown_operation"));
    assert!(e.execute(&id, |_| panic!("pending")).is_err());
    e.resolve(&id, true).unwrap();
    let c = e.ready().pop().unwrap();
    e.execute(&id, |_| json!({"status":"submitted"})).unwrap();
    assert!(e.capture_current(&c));
    e.complete(&c, json!({"status":"ok","name":"synthetic"})).unwrap();
    assert_eq!(poll(&e, 1, &id)["result"]["name"], "synthetic");
    assert!(e.execute(&id, |_| panic!("replay")).is_err());
}
#[test]
fn compound_classes_require_native_permissions() {
    let e = engine(Level::Allow);
    let r = invoke(&e, 1, Tool::InventoryRename);
    let id = r["request_id"].as_str().unwrap();
    e.prepared(id, &[Class::ChatRead], false).unwrap();
    assert_eq!(poll(&e, 1, id), error("permission_denied"));
}
#[test]
fn revocation_blocks_async_callbacks_and_finished_results() {
    let e = engine(Level::Allow);
    let id = prep(&e, Tool::InventoryRename);
    let c = e.ready().pop().unwrap();
    e.execute(&id, |_| json!({"status":"submitted"})).unwrap();
    let mut config = e.configuration().unwrap();
    config.permissions.set(Class::Edit, Level::Never).unwrap();
    e.configure(config).unwrap();
    assert!(!e.capture_current(&c));
    assert!(e.complete(&c, json!({"private":"sentinel"})).is_err());
    assert_eq!(poll(&e, 1, &id), error("source_changed"));
}
#[test]
fn replacement_and_stale_observation_do_not_reuse_enablement() {
    let e = engine(Level::Allow);
    let id = prep(&e, Tool::InventoryRename);
    e.observe(Some(source(2)), true);
    assert!(!e.snapshot().enabled);
    assert!(e.ready().is_empty());
    assert_eq!(poll(&e, 1, &id), error("viewer_unavailable"));
    assert!(e.enable(source(1), endpoint()).is_err());
    e.enable(source(2), endpoint()).unwrap();
    e.0.lock().unwrap().observed.as_mut().unwrap().1 = Instant::now() - FRESH - Duration::from_secs(1);
    assert_eq!(invoke(&e, 1, Tool::InventoryGet), error("viewer_unavailable"));
    e.observe(Some(source(2)), true);
    assert!(!e.snapshot().enabled);
}
#[test]
fn inactive_scope_and_pause_prevent_execution() {
    let e = engine(Level::Allow);
    let r = invoke(&e, 1, Tool::InventoryRename);
    let id = r["request_id"].as_str().unwrap().to_owned();
    e.prepared(&id, &[], true).unwrap();
    e.observe(Some(source(1)), false);
    assert!(e.ready().is_empty());
    assert!(e.execute(&id, |_| panic!("inactive")).is_err());
    e.pause(true);
    assert_eq!(
        e.request(
            1,
            ViewerRequest::Status {
                viewer_session: endpoint()
            }
        )["ready"],
        false
    );
    assert_eq!(invoke(&e, 1, Tool::InventoryGet), error("viewer_paused"));
}
#[test]
fn bounded_results_queues_dedup_and_disconnect() {
    let e = engine(Level::Allow);
    let id = uuid::Uuid::new_v4().to_string();
    let req = ViewerRequest::Invoke {
        viewer_session: endpoint(),
        tool: Tool::InventoryRename,
        arguments: serde_json::Map::new(),
        request_id: Some(id.clone()),
    };
    assert_eq!(e.request(1, req.clone()), e.request(1, req.clone()));
    assert_eq!(e.request(2, req), error("request_id_conflict"));
    e.prepared(&id, &[], false).unwrap();
    e.execute(&id, |_| json!({"status":"ok","data":"x".repeat(MAX_RESULT_BYTES)}))
        .unwrap();
    assert_eq!(poll(&e, 1, &id), error("response_limit"));
    for _ in 0..MAX_QUEUED {
        assert_eq!(invoke(&e, 1, Tool::InventoryGet)["status"], "preparing");
    }
    assert_eq!(invoke(&e, 1, Tool::InventoryGet), error("capacity"));
    e.disconnected(1);
    assert!(!e.has_operation(&id));
    assert_eq!(e.0.lock().unwrap().operations.len(), 0);
}
#[test]
fn expiry_and_permanent_delete_and_wire_roundtrip() {
    let e = engine(Level::Ask);
    let id = prep(&e, Tool::InventoryRename);
    e.0.lock().unwrap().operations.get_mut(&id).unwrap().created = Instant::now() - LIFETIME;
    assert!(e.resolve(&id, true).is_err());
    assert_eq!(poll(&e, 1, &id), error("operation_expired"));
    assert_eq!(invoke(&e, 1, Tool::InventoryPurge), error("permanent_delete_unavailable"));
    assert_eq!(invoke(&e, 1, Tool::ConfirmAction), error("native_approval_required"));
    let r = crate::wire::HostRequest::Assistant {
        request: ViewerRequest::Status {
            viewer_session: endpoint(),
        },
    };
    assert_eq!(
        serde_json::from_slice::<crate::wire::HostRequest>(&serde_json::to_vec(&r).unwrap()).unwrap(),
        r
    );
}

#[test]
fn authenticated_capture_client_endpoint_and_reenable_do_not_reuse_authority() {
    let e = engine(Level::Allow);
    let id = prep(&e, Tool::InventoryGet);
    let first = e.ready().into_iter().find(|c| c.request_id == id).unwrap();
    assert_eq!(first.client, 1);
    assert_eq!(first.endpoint, endpoint());
    e.execute(&id, |_| json!({"private":"first endpoint"})).unwrap();
    assert_eq!(poll(&e, 2, &id), error("unknown_operation"));
    let next = HostId::from_bytes([9; 16]).unwrap();
    e.disable();
    e.enable(source(1), next).unwrap();
    assert!(!e.capture_current(&first));
    assert_eq!(
        e.request(
            1,
            ViewerRequest::OperationStatus {
                viewer_session: next,
                request_id: id
            }
        ),
        error("unknown_operation")
    );
}
#[test]
fn native_busy_accepts_intents_but_never_delivers_private_receipts() {
    let e = engine(Level::Allow);
    e.begin_native_update();
    let value = invoke(&e, 1, Tool::InventoryGet);
    let id = value["request_id"].as_str().unwrap();
    e.prepared(id, &[], false).unwrap();
    e.execute(id, |_| json!({"private":"native result"})).unwrap();
    assert_eq!(poll(&e, 1, id), error("native_busy"));
    e.end_native_update();
    assert_eq!(poll(&e, 1, id)["result"]["private"], "native result");
    e.native_policy_changed();
    assert_eq!(poll(&e, 1, id), error("source_changed"));
}
#[test]
fn losing_active_authority_never_revives_after_focus_returns() {
    let e = engine(Level::Allow);
    let id = invoke(&e, 1, Tool::InventoryRename)["request_id"].as_str().unwrap().to_owned();
    e.prepared(&id, &[], true).unwrap();
    let capture = e.ready().pop().unwrap();
    e.observe(Some(source(1)), false);
    e.observe(Some(source(1)), true);
    assert!(e.ready().is_empty());
    assert!(!e.capture_current(&capture));
    assert!(e.execute(&id, |_| panic!("old action revived")).is_err());
    assert_eq!(poll(&e, 1, &id), error("native_action_revoked"));
}
#[test]
fn submitted_metadata_and_artifact_bytes_require_the_original_authenticated_owner() {
    let e = engine(Level::Allow);
    let id = prep(&e, Tool::InventoryGet);
    let capture = e.ready().pop().unwrap();
    e.execute(&id, |_| json!({"status":"submitted","native_operation_id":"opaque"}))
        .unwrap();
    assert_eq!(poll(&e, 1, &id)["result"]["native_operation_id"], "opaque");
    let token = uuid::Uuid::new_v4();
    e.install_artifact(&capture, token, vec![0, 1, 254, 255]).unwrap();
    e.complete(&capture, json!({"artifact_id":token.to_string()})).unwrap();
    let request = ViewerRequest::Artifact {
        viewer_session: endpoint(),
        request_id: id.clone(),
        artifact_id: token.to_string(),
        offset: 1,
        length: 2,
    };
    assert_eq!(e.request(2, request.clone()), error("artifact_unavailable"));
    assert_eq!(e.request(1, request.clone())["data"], "01fe");
    e.native_policy_changed();
    assert_eq!(e.request(1, request), error("artifact_unavailable"));
}
#[test]
fn asynchronous_native_commit_is_rejected_after_authenticated_disconnect() {
    let e = engine(Level::Allow);
    let id = prep(&e, Tool::InventoryGet);
    let capture = e.ready().pop().unwrap();
    e.execute(&id, |_| json!({"status":"submitted"})).unwrap();
    assert_eq!(e.continue_action(&capture, || 17), Ok(17));
    e.disconnected(1);
    assert!(e.continue_action(&capture, || panic!("disconnected client committed")).is_err());
}

#[test]
fn bounded_recipe_execution_budget_does_not_extend_approval_or_other_tools() {
    let e = engine(Level::Allow);
    let mut config = e.configuration().unwrap();
    config.permissions.set(Class::ScriptTest, Level::Ask).unwrap();
    e.configure(config).unwrap();
    e.capabilities([Tool::ScriptTestBegin]);
    let request = ViewerRequest::Invoke {
        viewer_session: endpoint(),
        tool: Tool::ScriptTestBegin,
        arguments: json!({"timeout_seconds":600}).as_object().unwrap().clone(),
        request_id: None,
    };
    let pending = e.request(1, request.clone());
    let expired = pending["request_id"].as_str().unwrap();
    e.prepared(expired, &[], true).unwrap();
    e.0.lock().unwrap().operations.get_mut(expired).unwrap().created = Instant::now() - LIFETIME;
    assert!(
        e.resolve(expired, true).is_err(),
        "large execution budgets never extend Ask approval"
    );
    let fresh = e.request(1, request);
    let id = fresh["request_id"].as_str().unwrap();
    e.prepared(id, &[], true).unwrap();
    e.resolve(id, true).unwrap();
    e.execute(id, |_| json!({"status":"submitted"})).unwrap();
    {
        let mut s = e.0.lock().unwrap();
        let o = s.operations.get_mut(id).unwrap();
        assert!(o.execution_deadline.unwrap().duration_since(Instant::now()) > Duration::from_secs(600));
        o.created = Instant::now() - Duration::from_secs(80);
    }
    assert_eq!(poll(&e, 1, id)["status"], "submitted");
    e.0.lock().unwrap().operations.get_mut(id).unwrap().execution_deadline = Some(Instant::now());
    assert_eq!(poll(&e, 1, id), error("operation_expired"));
}

#[test]
fn preference_preview_budget_starts_after_approval_and_is_bounded_to_native_ttl() {
    let e = engine(Level::Allow);
    let mut config = e.configuration().unwrap();
    config.permissions.set(Class::PreferencesRead, Level::Allow).unwrap();
    config.permissions.set(Class::PreferencesWrite, Level::Ask).unwrap();
    e.configure(config).unwrap();
    e.capabilities([Tool::PreferencesPreview]);
    let request = ViewerRequest::Invoke {
        viewer_session: endpoint(),
        tool: Tool::PreferencesPreview,
        arguments: json!({"ttl_seconds":300}).as_object().unwrap().clone(),
        request_id: None,
    };
    let pending = e.request(1, request.clone());
    let id = pending["request_id"].as_str().unwrap();
    e.prepared(id, &[Class::PreferencesRead], true).unwrap();
    e.0.lock().unwrap().operations.get_mut(id).unwrap().created = Instant::now() - LIFETIME;
    assert!(e.resolve(id, true).is_err());
    let pending = e.request(1, request);
    let id = pending["request_id"].as_str().unwrap();
    e.prepared(id, &[Class::PreferencesRead], true).unwrap();
    e.resolve(id, true).unwrap();
    e.execute(id, |_| json!({"status":"submitted"})).unwrap();
    let guard = e.0.lock().unwrap();
    let o = guard.operations.get(id).unwrap();
    let duration = o.execution_deadline.unwrap().saturating_duration_since(Instant::now());
    assert!(duration > Duration::from_secs(300) && duration <= Duration::from_secs(305));
}

#[test]
fn source_bound_test_stop_can_cancel_after_permissions_are_revoked_and_paused() {
    let e = engine(Level::Never);
    e.capabilities([Tool::ScriptTestStop]);
    e.pause(true);
    let reply = invoke(&e, 1, Tool::ScriptTestStop);
    let id = reply["request_id"].as_str().unwrap();
    e.prepared(id, &[], false).unwrap();
    assert_eq!(e.ready().len(), 1);
    e.execute(id, |_| json!({"state":"stopped","no_new_effects":true})).unwrap();
    assert_eq!(poll(&e, 1, id)["result"]["state"], "stopped");
    e.disable();
    assert_eq!(invoke(&e, 1, Tool::ScriptTestStop), error("viewer_unavailable"));
}

fn upload_report_pair() -> (Runtime, Capture, Capture) {
    let e = engine(Level::Allow);
    e.capabilities([Tool::InventoryUploadPreview, Tool::InventoryUploadStatus]);
    let id = prep(&e, Tool::InventoryUploadPreview);
    let original = e.ready().pop().unwrap();
    e.execute(&id, |_| json!({"files":[{"name":"Synthetic","encoded_sha256":"private-proof"}]}))
        .unwrap();
    let reply = invoke(&e, 1, Tool::InventoryUploadStatus);
    let status = e.preparations().pop().unwrap();
    e.bind_original_receipt(&status, &original).unwrap();
    e.prepared(reply["request_id"].as_str().unwrap(), &[], false).unwrap();
    (e, original, status)
}
#[test]
fn upload_status_dependency_is_atomic_before_execute_and_after_cached_delivery() {
    let (e, original, status) = upload_report_pair();
    e.request(
        1,
        ViewerRequest::Cancel {
            viewer_session: endpoint(),
            request_id: original.request_id.clone(),
        },
    );
    assert!(
        e.execute(&status.request_id, |_| panic!("retired report reached native execution"))
            .is_err()
    );
    for change in 0..3 {
        let (e, original, status) = upload_report_pair();
        e.execute(&status.request_id, |_| json!({"files":[{"encoded_sha256":"private-proof"}]}))
            .unwrap();
        assert_eq!(
            poll(&e, 1, &status.request_id)["result"]["files"][0]["encoded_sha256"],
            "private-proof"
        );
        match change {
            0 => {
                e.request(
                    1,
                    ViewerRequest::Cancel {
                        viewer_session: endpoint(),
                        request_id: original.request_id.clone(),
                    },
                );
            }
            1 => e.reject(&original.request_id, "native_result_access_revoked"),
            _ => e.0.lock().unwrap().operations.get_mut(&original.request_id).unwrap().created = Instant::now() - Duration::from_secs(71),
        }
        assert_eq!(poll(&e, 1, &status.request_id), error("access_revoked"));
    }
}
#[test]
fn upload_execution_budget_starts_after_approval_without_reviving_preview() {
    let e = engine(Level::Allow);
    let mut config = e.configuration().unwrap();
    config.permissions.set(Class::Upload, Level::Allow).unwrap();
    e.configure(config).unwrap();
    e.capabilities([Tool::InventoryUploadPreview, Tool::InventoryUploadExecute]);
    let parent = prep(&e, Tool::InventoryUploadPreview);
    e.execute(&parent, |_| json!({"plan_id":"immutable"})).unwrap();
    e.0.lock().unwrap().operations.get_mut(&parent).unwrap().created = Instant::now() - Duration::from_secs(71);
    let pending = invoke(&e, 1, Tool::InventoryUploadExecute);
    let id = pending["request_id"].as_str().unwrap();
    e.prepared_with_review(id, &[], false, true).unwrap();
    assert!(e.0.lock().unwrap().operations[id].execution_deadline.is_none());
    e.resolve(id, true).unwrap();
    e.execute(id, |_| json!({"status":"submitted"})).unwrap();
    let deadline = e.0.lock().unwrap().operations[id].execution_deadline.unwrap();
    assert!(deadline > Instant::now() + Duration::from_secs(590));
    assert!(deadline <= Instant::now() + Duration::from_secs(600));
    assert_eq!(poll(&e, 1, &parent), error("access_revoked"));
}

#[test]
fn protected_content_and_object_status_aliases_cannot_refresh_original_expiry_or_cross_clients() {
    for (tool, alias) in [
        (Tool::InventoryContentGet, Tool::InventoryContentOperationStatus),
        (Tool::ObjectContentsList, Tool::ObjectOperationStatus),
    ] {
        let e = engine(Level::Allow);
        let mut config = e.configuration().unwrap();
        config.permissions.set(Class::TaskInventoryRead, Level::Allow).unwrap();
        e.configure(config).unwrap();
        e.capabilities([tool, alias]);
        let original_id = prep(&e, tool);
        let original = e.ready().pop().unwrap();
        e.execute(&original_id, |_| json!({"text":"protected synthetic content"})).unwrap();
        let reply = invoke(&e, 2, alias);
        let foreign = e.preparations().into_iter().find(|c| c.client == 2).unwrap();
        assert!(e.bind_original_receipt(&foreign, &original).is_err());
        e.reject(reply["request_id"].as_str().unwrap(), "foreign_source");
        let reply = invoke(&e, 1, alias);
        let consumer = e.preparations().into_iter().find(|c| c.client == 1).unwrap();
        e.bind_original_receipt(&consumer, &original).unwrap();
        e.prepared(reply["request_id"].as_str().unwrap(), &[], false).unwrap();
        e.execute(&consumer.request_id, |_| json!({"text":"protected synthetic content"}))
            .unwrap();
        assert_eq!(poll(&e, 1, &consumer.request_id)["result"]["text"], "protected synthetic content");
        e.0.lock().unwrap().operations.get_mut(&original_id).unwrap().created = Instant::now() - Duration::from_secs(71);
        assert_eq!(poll(&e, 1, &consumer.request_id), error("access_revoked"));
        let reply = invoke(&e, 1, alias);
        let next = e
            .preparations()
            .into_iter()
            .find(|c| c.request_id == reply["request_id"].as_str().unwrap())
            .unwrap();
        assert!(e.bind_original_receipt(&next, &original).is_err());
    }
}
