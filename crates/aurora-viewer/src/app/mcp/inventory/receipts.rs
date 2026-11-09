//! Private evidence retained at the exact point a result body is produced.
//! Actor grants alone never keep a cached source readable after native rights,
//! ownership, inventory revision, or protected-asset visibility change.
use super::*;
use aurora_net::build::TaskItem;

type Key = (HostId, u64, String);
#[derive(Clone)]
pub(super) struct Proof {
    capture: Capture,
    source: Capture,
    recorded: Instant,
    evidence: Evidence,
}
#[derive(Clone)]
enum Evidence {
    BodyFree,
    Upload,
    Native {
        objects: Vec<(Uuid, Value)>,
        dialogs: Vec<Uuid>,
    },
    Protected {
        inventory: Vec<Frozen>,
        task: Vec<TaskItem>,
        object: Option<Object>,
        assets: Vec<Uuid>,
        content: bool,
    },
}
#[derive(Clone, PartialEq)]
struct Object {
    id: Uuid,
    region: u64,
    local: u32,
    owner: Uuid,
    flags: u32,
    props_owner: Option<Uuid>,
    props_serial: Option<i16>,
    serial: i16,
}
fn key(c: &Capture) -> Key {
    (c.endpoint, c.client, c.request_id.clone())
}
impl Object {
    fn capture(app: &App, id: Uuid) -> Result<Self, &'static str> {
        let idx = objects::object(app, id)?;
        let o = app.world.objects.get(idx).ok_or("object_unavailable")?;
        let props = app.build.props.get(&id);
        // Never let an old properties record override a contradictory known
        // current owner. Unknown owner data still follows the native gate.
        if !o.owner_id.is_nil() && o.owner_id != app.world.agent_id {
            return Err("native_permission_denied");
        }
        Ok(Self {
            id,
            region: o.key.region,
            local: o.key.local_id,
            owner: o.owner_id,
            flags: o.update_flags,
            props_owner: props.map(|p| p.owner_id),
            props_serial: props.map(|p| p.inventory_serial),
            serial: app.build.contents.objects.get(&id).ok_or("contents_unavailable")?.serial,
        })
    }
}
fn content_tool(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::InventoryReadNotecard
            | Tool::InventoryReadScript
            | Tool::InventoryContentGet
            | Tool::InventoryContentPreview
            | Tool::ObjectScriptGet
            | Tool::ObjectScriptPreview
    )
}
fn metadata_tool(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::InventoryLandmark
            | Tool::InventoryPreviewBatchMove
            | Tool::InventoryPreviewBatchRename
            | Tool::InventoryCopy
            | Tool::InventoryGet
            | Tool::InventoryGetMany
            | Tool::InventoryList
            | Tool::InventorySearch
            | Tool::InventoryResolvePath
            | Tool::InventoryProtectedFolders
            | Tool::InventoryPermissionsGet
            | Tool::AppearanceOutfits
            | Tool::AppearanceOutfitItems
            | Tool::AppearanceWorn
            | Tool::ObjectContentsList
            | Tool::ObjectScriptStatus
            | Tool::InventorySystemFolder
            | Tool::InventoryHistory
            | Tool::InventoryChanges
            | Tool::InventoryPreviewUndo
    )
}
fn private_diagnostics(tool: Tool, value: &Value) -> bool {
    matches!(tool, Tool::InventorySaveCompileScript | Tool::ObjectScriptSaveCompile)
        && value.get("errors").and_then(Value::as_array).is_some_and(|e| !e.is_empty())
}
fn body_free_status(value: &Value) -> bool {
    value.as_object().is_some_and(|map| {
        map.keys().all(|key| {
            matches!(
                key.as_str(),
                "status"
                    | "error"
                    | "reason"
                    | "operation_id"
                    | "client_request_id"
                    | "confirmed"
                    | "retry_safe"
                    | "uncertain"
                    | "native_submitted"
            )
        }) && matches!(value["status"].as_str(), Some("error" | "submitted" | "loading"))
    })
}
fn factual_tool(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::InventoryTypes
            | Tool::InventoryReplaceLinks
            | Tool::InventoryConfirmCopy
            | Tool::InventoryCreateFolder
            | Tool::InventoryCreateItem
            | Tool::InventoryMove
            | Tool::InventoryRename
            | Tool::InventoryBatchMove
            | Tool::InventoryBatchRename
            | Tool::InventoryBatchCopy
            | Tool::InventoryTrash
            | Tool::InventoryRestore
            | Tool::InventoryLink
            | Tool::InventorySetDescription
            | Tool::InventorySetFavorite
            | Tool::InventorySetThumbnail
            | Tool::InventoryNextOwnerSet
            | Tool::InventoryExecutePlan
            | Tool::AppearanceWearOutfit
            | Tool::AppearanceWearItems
            | Tool::AppearanceDetach
            | Tool::InventoryUpdateNotecard
            | Tool::InventorySaveCompileScript
            | Tool::ObjectContentsPut
            | Tool::ObjectContentsReconcile
            | Tool::ObjectContentsRename
            | Tool::ObjectContentsRemove
            | Tool::ObjectScriptSetRunning
            | Tool::ObjectScriptReset
            | Tool::ObjectScriptSaveCompile
            | Tool::ObjectTake
            | Tool::ObjectTakeCopy
            | Tool::ObjectTouch
            | Tool::ObjectDialogReply
            | Tool::ObjectChannelSend
            | Tool::ScriptTestBegin
            | Tool::ScriptTestStatus
            | Tool::ScriptTestStop
            | Tool::InventoryUploadExecute
            | Tool::InventorySnapshotUpload
            | Tool::InventoryUploadCancel
            | Tool::InventoryUploadMeshReview
            | Tool::ScriptDebugPoll
            | Tool::ObjectDialogList
    )
}
fn unprotected_allowed(tool: Tool, value: &Value) -> bool {
    factual_tool(tool)
        || body_free_status(value)
            && (content_tool(tool)
                || metadata_tool(tool)
                || matches!(tool, Tool::InventoryContentOperationStatus | Tool::ObjectOperationStatus))
}
pub(super) fn record_native_objects(
    app: &App,
    state: &mut State,
    c: &Capture,
    ids: &[Uuid],
    dialogs: Vec<Uuid>,
) -> Result<(), &'static str> {
    if ids.len() > 128 || dialogs.len() > 32 || state.receipts.len() >= CAPACITY && !state.receipts.contains_key(&key(c)) {
        return Err("receipt_evidence_limit");
    }
    let objects = ids
        .iter()
        .copied()
        .map(|id| Ok((id, interactions::freeze(app, id)?)))
        .collect::<Result<Vec<_>, &'static str>>()?;
    state.receipts.insert(
        key(c),
        Proof {
            capture: c.clone(),
            source: c.clone(),
            recorded: Instant::now(),
            evidence: Evidence::Native { objects, dialogs },
        },
    );
    if !validate(app, state, c) {
        state.receipts.remove(&key(c));
        return Err("native_result_access_revoked");
    }
    Ok(())
}
pub(super) fn record_acquisition(app: &App, state: &mut State, c: &Capture, id: Uuid) -> Result<(), &'static str> {
    let mut inventory = Vec::new();
    ordinary(app, id)?;
    for ancestor in rules::ancestors(&app.world.inventory, id).take(33) {
        inventory.push(Frozen::capture(&app.world.inventory, ancestor)?);
    }
    inventory.push(Frozen::capture(&app.world.inventory, id)?);
    state.receipts.insert(
        key(c),
        Proof {
            capture: c.clone(),
            source: c.clone(),
            recorded: Instant::now(),
            evidence: Evidence::Protected {
                inventory,
                task: Vec::new(),
                object: None,
                assets: Vec::new(),
                content: false,
            },
        },
    );
    Ok(())
}
fn scan(value: &Value, ids: &mut HashSet<Uuid>, assets: &mut HashSet<Uuid>, depth: usize) -> Result<(), &'static str> {
    if depth > 12 {
        return Err("receipt_evidence_limit");
    }
    match value {
        Value::Object(map) => {
            for (name, value) in map {
                if matches!(name.as_str(), "id" | "item_id" | "folder_id" | "parent_id")
                    && let Some(id) = value.as_str().and_then(|v| Uuid::parse_str(v).ok())
                {
                    ids.insert(id);
                }
                if matches!(name.as_str(), "asset_id" | "thumbnail_id")
                    && let Some(id) = value.as_str().and_then(|v| Uuid::parse_str(v).ok())
                {
                    assets.insert(id);
                }
                if ids.len() > 128 || assets.len() > 128 {
                    return Err("receipt_evidence_limit");
                }
                scan(value, ids, assets, depth + 1)?;
            }
        }
        Value::Array(rows) => {
            for row in rows {
                scan(row, ids, assets, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
/// This function is called without Runtime calls inside the initial execute
/// mutex. On capacity failure the caller returns a body-free error instead.
pub(super) fn record(app: &App, state: &mut State, c: &Capture, value: &Value) -> Result<(), &'static str> {
    state.receipts.retain(|_, p| p.recorded.elapsed() < Duration::from_secs(600));
    if state.receipts.len() >= CAPACITY && !state.receipts.contains_key(&key(c)) {
        return Err("receipt_evidence_limit");
    }
    if matches!(c.tool, Tool::InventoryUploadPreview | Tool::InventoryUploadStatus) && value["status"] != "error" {
        if !uploads::validate_receipt(app, state, c) {
            return Err("native_receipt_proof_unavailable");
        }
        state.receipts.insert(
            key(c),
            Proof {
                capture: c.clone(),
                source: c.clone(),
                recorded: Instant::now(),
                evidence: Evidence::Upload,
            },
        );
        return Ok(());
    }
    if matches!(c.tool, Tool::InventoryContentOperationStatus | Tool::ObjectOperationStatus) && value["status"] != "error" {
        let operation = string(&c.arguments, "operation_id", 128)?;
        let original = state.outcome_captures.get(operation).ok_or("unknown_operation")?;
        let old = state
            .receipts
            .get(&key(original))
            .filter(|p| p.capture == *original)
            .ok_or("native_receipt_proof_unavailable")?;
        if !validate(app, state, original) {
            return Err("native_result_access_revoked");
        }
        let mut proof = old.clone();
        proof.capture = c.clone();
        proof.recorded = Instant::now();
        state.receipts.insert(key(c), proof);
        return Ok(());
    }
    if !body_free_status(value) {
        if c.tool == Tool::ScriptDebugPoll {
            let ids = value
                .get("entries")
                .and_then(Value::as_array)
                .ok_or("native_receipt_proof_unavailable")?
                .iter()
                .map(|r| {
                    r.get("object_id")
                        .and_then(Value::as_str)
                        .and_then(|v| Uuid::parse_str(v).ok())
                        .ok_or("native_receipt_proof_unavailable")
                })
                .collect::<Result<Vec<_>, _>>()?;
            return record_native_objects(app, state, c, &ids, Vec::new());
        }
        if c.tool == Tool::ObjectDialogList {
            let dialogs = interactions::receipt_dialogs(state, c, value)?;
            return record_native_objects(app, state, c, &[uuid(&c.arguments, "object_id")?], dialogs);
        }
    }
    let protected = (metadata_tool(c.tool) || content_tool(c.tool) || private_diagnostics(c.tool, value))
        && value["status"] != "error"
        && value["status"] != "submitted"
        && value["status"] != "loading";
    let evidence = if !protected {
        if !unprotected_allowed(c.tool, value) {
            return Err("native_receipt_proof_unavailable");
        }
        Evidence::BodyFree
    } else {
        let mut ids = HashSet::new();
        let mut assets = HashSet::new();
        scan(value, &mut ids, &mut assets, 0)?;
        let mut inventory = Vec::new();
        let mut task = Vec::new();
        let content = content_tool(c.tool) || private_diagnostics(c.tool, value);
        let object_id = optional_uuid(&c.arguments, "object_id")?;
        let object = if let Some(id) = object_id {
            let object = Object::capture(app, id)?;
            let current = &app.build.contents.objects.get(&id).ok_or("contents_unavailable")?.items;
            if let Ok(Some(item)) = optional_uuid(&c.arguments, "item_id") {
                ids.insert(item);
            }
            for id in ids {
                if let Some(item) = current.iter().find(|i| i.item_id == id && !i.is_folder) {
                    task.push(item.clone());
                }
            }
            if content && task.is_empty() {
                return Err("native_receipt_proof_unavailable");
            }
            Some(object)
        } else {
            if content && let Ok(Some(id)) = optional_uuid(&c.arguments, "id") {
                ids.insert(id);
            }
            let mut frozen = HashSet::new();
            for id in ids {
                if !app.world.inventory.items.contains_key(&id) && !app.world.inventory.folders.contains_key(&id) {
                    continue;
                }
                for ancestor in rules::ancestors(&app.world.inventory, id).take(33) {
                    if frozen.insert(ancestor) {
                        inventory.push(Frozen::capture(&app.world.inventory, ancestor)?);
                    }
                    if inventory.len() > 512 {
                        return Err("receipt_evidence_limit");
                    }
                }
                if frozen.insert(id) {
                    inventory.push(Frozen::capture(&app.world.inventory, id)?);
                }
            }
            if content && inventory.iter().all(|f| f.item.is_none()) {
                return Err("native_receipt_proof_unavailable");
            }
            None
        };
        Evidence::Protected {
            inventory,
            task,
            object,
            assets: assets.into_iter().collect(),
            content,
        }
    };
    state.receipts.insert(
        key(c),
        Proof {
            capture: c.clone(),
            source: c.clone(),
            recorded: Instant::now(),
            evidence,
        },
    );
    if !validate(app, state, c) {
        state.receipts.remove(&key(c));
        return Err("native_result_access_revoked");
    }
    Ok(())
}
impl Proof {
    fn native_current(&self, object: impl Fn(Uuid) -> Option<Value>, dialog: impl Fn(Uuid) -> bool) -> bool {
        match &self.evidence {
            Evidence::Native { objects, dialogs } => {
                self.recorded.elapsed() < Duration::from_secs(600)
                    && objects.iter().all(|(id, old)| object(*id).as_ref() == Some(old))
                    && dialogs.iter().all(|id| dialog(*id))
            }
            _ => false,
        }
    }
    fn inventory_current(&self, inventory: &Inventory) -> bool {
        match &self.evidence {
            Evidence::BodyFree | Evidence::Native { .. } | Evidence::Upload => true,
            Evidence::Protected {
                inventory: items, assets, ..
            } => {
                self.recorded.elapsed() < Duration::from_secs(600)
                    && items.iter().all(|f| f.current(inventory))
                    && assets.iter().all(|id| rules::asset_uuid_shown(inventory, *id))
            }
        }
    }
}
pub(super) fn validate(app: &App, state: &State, c: &Capture) -> bool {
    let Some(proof) = state.receipts.get(&key(c)).filter(|p| p.capture == *c) else {
        return false;
    };
    match &proof.evidence {
        Evidence::Upload => uploads::validate_receipt(app, state, c),
        Evidence::BodyFree => true,
        Evidence::Native { .. } => {
            diagnostics::privacy(app).is_ok()
                && proof.native_current(
                    |id| interactions::freeze(app, id).ok(),
                    |id| interactions::receipt_dialog_current(app, state, &proof.source, id),
                )
        }
        Evidence::Protected {
            inventory,
            task,
            object,
            assets: _,
            content,
        } => {
            if !proof.inventory_current(&app.world.inventory) {
                return false;
            }
            if proof.source.tool == Tool::InventoryLandmark && landmarks::prepare(app, &proof.source).is_err() {
                return false;
            }
            if let Some(object) = object {
                if Object::capture(app, object.id).as_ref() != Ok(object) {
                    return false;
                }
                if !task.iter().all(|old| {
                    app.build
                        .contents
                        .objects
                        .get(&object.id)
                        .is_some_and(|i| i.items.iter().any(|now| now == old))
                }) {
                    return false;
                }
                if (*content || proof.source.tool == Tool::ObjectScriptStatus) && objects::status_gate(app, &proof.source).is_err() {
                    return false;
                }
            } else {
                if inventory
                    .iter()
                    .filter_map(|f| f.item.as_ref())
                    .any(|i| !app.world.inventory.can_preview(i.asset_type))
                {
                    return false;
                }
                if *content && content::status_gate(app, &proof.source).is_err() {
                    return false;
                }
            }
            true
        }
    }
}

pub(super) fn prune(state: &mut State, runtime: &Runtime) {
    state.receipts.retain(|_, proof| runtime.has_operation(&proof.capture.request_id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_mcp::assistant::permissions::{Configuration, Level};
    use aurora_mcp::assistant::runtime::{Runtime, ViewerRequest};
    fn engine(tool: Tool) -> (Runtime, Capture) {
        let c = synthetic_capture(tool);
        let runtime = Runtime::default();
        runtime.observe(Some(c.binding), true);
        let mut configuration = Configuration::default();
        configuration.permissions.set(Class::Edit, Level::Allow).unwrap();
        runtime.configure(configuration).unwrap();
        runtime.capabilities([tool]);
        runtime.enable(c.binding, c.endpoint).unwrap();
        let reply = runtime.request(
            c.client,
            ViewerRequest::Invoke {
                viewer_session: c.endpoint,
                tool,
                arguments: c.arguments.clone(),
                request_id: Some(Uuid::from_u128(4200).to_string()),
            },
        );
        assert_eq!(reply["status"], "preparing", "{reply}");
        let id = reply["request_id"].as_str().unwrap();
        runtime.prepared(id, &[], false).unwrap();
        let capture = runtime.ready().pop().unwrap();
        (runtime, capture)
    }
    fn poll(runtime: &Runtime, c: &Capture) -> Value {
        runtime.request(
            c.client,
            ViewerRequest::OperationStatus {
                viewer_session: c.endpoint,
                request_id: c.request_id.clone(),
            },
        )
    }
    #[test]
    fn body_markers_do_not_classify_unknown_or_bodyful_successes_as_factual() {
        for status in ["submitted", "loading", "error"] {
            assert!(!unprotected_allowed(Tool::CameraSnapshot, &json!({"status":status})));
            assert!(!unprotected_allowed(
                Tool::InventoryContentGet,
                &json!({"status":status,"text":"private source"})
            ));
        }
        assert!(unprotected_allowed(
            Tool::InventoryContentGet,
            &json!({"status":"submitted","operation_id":"opaque","confirmed":false,"retry_safe":false})
        ));
        assert!(unprotected_allowed(
            Tool::ObjectTouch,
            &json!({"status":"outcome_unknown","confirmed":false})
        ));
    }
    #[test]
    fn generic_private_diagnostic_and_dialog_receipts_are_withheld_after_native_owner_or_circuit_drift() {
        for dialog_changed in [false, true] {
            let (runtime, c) = engine(Tool::InventoryContentGet);
            let object = Uuid::new_v4();
            let token = Uuid::new_v4();
            let native = json!({"owner":"original","region":10,"local":7});
            let proof = Proof {
                capture: c.clone(),
                source: c.clone(),
                recorded: Instant::now(),
                evidence: Evidence::Native {
                    objects: vec![(object, native.clone())],
                    dialogs: if dialog_changed { vec![token] } else { Vec::new() },
                },
            };
            assert!(proof.native_current(|_| Some(native.clone()), |_| true));
            runtime
                .execute(&c.request_id, |_| json!({"text":"protected diagnostic or dialog body"}))
                .unwrap();
            assert_eq!(poll(&runtime, &c)["result"]["text"], "protected diagnostic or dialog body");
            runtime.begin_native_update();
            let changed = if dialog_changed {
                native.clone()
            } else {
                json!({"owner":"different","region":10,"local":7})
            };
            assert!(!proof.native_current(|_| Some(changed.clone()), |_| !dialog_changed));
            runtime.reject_withheld(&c).unwrap();
            runtime.end_native_update();
            assert!(poll(&runtime, &c).get("result").is_none());
        }
    }
    fn item() -> InvItem {
        InvItem {
            id: Uuid::from_u128(4001),
            asset_id: Uuid::from_u128(4002),
            owner: Uuid::from_u128(4003),
            asset_type: 10,
            owner_mask: rules::COPY | rules::MODIFY | rules::TRANSFER,
            base_mask: rules::COPY | rules::MODIFY | rules::TRANSFER,
            ..Default::default()
        }
    }
    #[test]
    fn generic_operation_status_withholds_source_after_native_rights_or_asset_changes() {
        for change in 0..5 {
            let mut inv = Inventory::default();
            let original = item();
            inv.add_items(vec![original.clone()]);
            let (runtime, c) = engine(Tool::InventoryContentGet);
            let proof = Proof {
                capture: c.clone(),
                source: c.clone(),
                recorded: Instant::now(),
                evidence: Evidence::Protected {
                    inventory: vec![Frozen::capture(&inv, original.id).unwrap()],
                    task: Vec::new(),
                    object: None,
                    assets: vec![original.asset_id],
                    content: true,
                },
            };
            assert!(proof.inventory_current(&inv));
            runtime
                .execute(
                    &c.request_id,
                    |_| json!({"text":"synthetic protected source","asset_id":original.asset_id.to_string()}),
                )
                .unwrap();
            assert_eq!(poll(&runtime, &c)["result"]["text"], "synthetic protected source");
            runtime.begin_native_update();
            let mut changed = original.clone();
            match change {
                0 => changed.owner_mask &= !rules::COPY,
                1 => changed.owner_mask &= !rules::TRANSFER,
                2 => changed.owner_mask &= !rules::MODIFY,
                3 => changed.owner = Uuid::from_u128(4005),
                _ => changed.asset_id = Uuid::from_u128(4004),
            }
            inv.add_items(vec![changed]);
            assert!(!proof.inventory_current(&inv));
            runtime.reject_withheld(&c).unwrap();
            runtime.end_native_update();
            assert_eq!(poll(&runtime, &c), aurora_mcp::wire::error("native_result_access_revoked"));
        }
    }
    #[test]
    fn withholding_is_capture_owned_and_body_free_ack_survives_creator_rights_loss() {
        let (runtime, c) = engine(Tool::InventoryRename);
        runtime
            .execute(&c.request_id, |_| json!({"status":"confirmed","confirmed":true}))
            .unwrap();
        let mut forged = c.clone();
        forged.client += 1;
        assert_eq!(runtime.reject_withheld(&forged), Err("stale_source"));
        forged = c.clone();
        forged.endpoint = HostId::from_bytes([88; 16]).unwrap();
        assert_eq!(runtime.reject_withheld(&forged), Err("stale_source"));
        let proof = Proof {
            capture: c.clone(),
            source: c.clone(),
            recorded: Instant::now(),
            evidence: Evidence::BodyFree,
        };
        let mut inv = Inventory::default();
        let mut changed = item();
        changed.owner_mask = 0;
        inv.add_items(vec![changed]);
        assert!(proof.inventory_current(&inv));
        assert_eq!(poll(&runtime, &c)["result"]["confirmed"], true);
    }
}
