use super::*;
use aurora_net::scripts::ScriptTarget;
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub(super) struct Snapshot {
    endpoint: HostId,
    policy_revision: u64,
    client: u64,
    pub(super) expires: Instant,
    binding: Binding,
    item: Uuid,
    object: Uuid,
    asset: Uuid,
    revision: Uuid,
    text: String,
    hash: String,
    card: Option<aurora_net::inventory::notecard::Notecard>,
    experience: Option<Uuid>,
}
pub(super) fn supports(t: Tool) -> bool {
    matches!(
        t,
        Tool::InventoryReadNotecard
            | Tool::InventoryReadScript
            | Tool::InventoryContentGet
            | Tool::InventoryContentPreview
            | Tool::InventoryUpdateNotecard
            | Tool::InventorySaveCompileScript
    )
}
fn info(app: &App, id: Uuid, write: bool) -> Result<&InvItem, &'static str> {
    let inv = &app.world.inventory;
    let item = inv
        .items
        .get(&id)
        .filter(|it| matches!(it.asset_type, 7 | 10))
        .ok_or("unsupported_item_type")?;
    if !inv.can_preview(item.asset_type) {
        return Err("native_restriction");
    }
    ordinary(app, id)?;
    if item.asset_type == 10 {
        let gate = app
            .script_info(ScriptTarget {
                object: Uuid::nil(),
                item: id,
            })
            .ok_or("not_found")?;
        if !gate.can_read || write && !gate.can_edit {
            return Err("native_permission_denied");
        }
    } else if item.owner_mask & rules::COPY == 0 || write && !crate::world::inventory::preview::can_edit(inv, id) {
        return Err("native_permission_denied");
    }
    if write && (rules::in_type(inv, id, 14) || item.owner != app.world.agent_id) {
        return Err("native_permission_denied");
    }
    Ok(item)
}
fn editor_open(app: &App, id: Uuid) -> bool {
    app.inventory_ui.preview == Some(id)
        || app.inventory_ui.windows.iter().any(|w| w.state.preview == Some(id))
        || app
            .script_editors
            .documents
            .iter()
            .any(|d| d.target.item == id && d.target.object.is_nil())
}
fn snapshot<'a>(app: &App, state: &'a State, c: &Capture) -> Result<&'a Snapshot, &'static str> {
    let a = &c.arguments;
    let id = uuid(a, "id")?;
    let snapshot = state
        .content
        .get(&id)
        .filter(|s| {
            s.endpoint == c.endpoint
                && s.policy_revision == c.policy_revision
                && s.client == c.client
                && s.binding == c.binding
                && s.object.is_nil()
                && s.expires > Instant::now()
        })
        .ok_or("complete_source_required")?;
    let expected = uuid(a, "expected_asset_id")?;
    let item = info(app, id, false)?;
    if item.asset_id != snapshot.asset
        || (expected != snapshot.revision && expected != snapshot.asset)
        || string(a, "content_hash", 64)? != snapshot.hash
    {
        return Err("stale_source_snapshot");
    }
    Ok(snapshot)
}
pub(super) fn prepare(app: &mut App, state: &State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    let a = &c.arguments;
    let write = matches!(c.tool, Tool::InventoryUpdateNotecard | Tool::InventorySaveCompileScript);
    if matches!(
        c.tool,
        Tool::InventoryContentPreview | Tool::InventoryUpdateNotecard | Tool::InventorySaveCompileScript
    ) {
        args(c, &["id", "expected_asset_id", "content_hash", "text", "compiler"])?;
        let snapshot = snapshot(app, state, c)?;
        string(a, "text", SOURCE_LIMIT)?;
        if c.tool == Tool::InventorySaveCompileScript && a.get("compiler").and_then(Value::as_str) != Some("mono") {
            return Err("unsupported_compiler");
        }
        if snapshot.card.is_some() && a.get("compiler").is_some_and(|v| v.as_str() != Some("")) {
            return Err("invalid_args");
        }
        if let Some(card) = &snapshot.card {
            card.encode(string(a, "text", SOURCE_LIMIT)?)
                .map_err(|_| "invalid_notecard_markers")?;
        }
    } else {
        args(c, &["id"])?;
        uuid(a, "id")?;
    }
    let id = uuid(a, "id")?;
    let item = info(app, id, write)?;
    if matches!(c.tool, Tool::InventoryReadNotecard | Tool::InventoryUpdateNotecard) && item.asset_type != 7
        || matches!(c.tool, Tool::InventoryReadScript | Tool::InventorySaveCompileScript) && item.asset_type != 10
    {
        return Err("unsupported_item_type");
    }
    if editor_open(app, id) {
        return Err("native_editor_open");
    }
    if write {
        native_busy(app)?;
    }
    Ok(if write { vec![Class::Read] } else { Vec::new() })
}
pub(super) fn execute(app: &mut App, state: &mut State, c: &Capture) -> Result<Value, &'static str> {
    state.admit()?;
    let id = uuid(&c.arguments, "id")?;
    let item = info(
        app,
        id,
        matches!(c.tool, Tool::InventoryUpdateNotecard | Tool::InventorySaveCompileScript),
    )?
    .clone();
    match c.tool {
        Tool::InventoryContentPreview => {
            let snapshot = snapshot(app, state, c)?;
            let text = string(&c.arguments, "text", SOURCE_LIMIT)?;
            Ok(
                json!({"id":id.to_string(),"changed":text!=snapshot.text,"old_bytes":snapshot.text.len(),"new_bytes":text.len(),"old_lines":snapshot.text.lines().count(),"new_lines":text.lines().count(),"content_hash":snapshot.hash,"new_content_hash":hash(text),"source_saved":false}),
            )
        }
        Tool::InventoryUpdateNotecard | Tool::InventorySaveCompileScript => {
            let snapshot = snapshot(app, state, c)?.clone();
            let text = string(&c.arguments, "text", SOURCE_LIMIT)?.to_owned();
            let request = Uuid::new_v4();
            if c.tool == Tool::InventoryUpdateNotecard {
                let data = snapshot
                    .card
                    .as_ref()
                    .ok_or("unsupported_item_type")?
                    .encode(&text)
                    .map_err(|_| "invalid_notecard_markers")?;
                app.send(NetCommand::SaveInventoryContent {
                    request,
                    item: id,
                    asset_type: 7,
                    data,
                });
            } else {
                app.mcp_save_agent_script(request, id, snapshot.asset, text)
                    .map_err(|_| "native_permission_denied")?;
            }
            pending_for(
                state,
                request,
                c,
                PendingKind::ContentSave {
                    item: id,
                    asset: item.asset_id,
                    object: Uuid::nil(),
                    script: item.asset_type == 10,
                },
            );
            outcome(state, c, c.request_id.clone(), submitted(c));
            Ok(submitted(c))
        }
        _ => {
            if state
                .pending
                .values()
                .any(|p| matches!(p.kind,PendingKind::ContentRead{item,..}if item==id))
            {
                return Err("native_read_pending");
            }
            let request = Uuid::new_v4();
            pending_for(
                state,
                request,
                c,
                PendingKind::ContentRead {
                    item: id,
                    asset: item.asset_id,
                    object: Uuid::nil(),
                },
            );
            if item.asset_type == 10 {
                app.mcp_load_script(
                    request,
                    ScriptTarget {
                        object: Uuid::nil(),
                        item: id,
                    },
                )
                .map_err(|_| "native_permission_denied")?;
            } else {
                app.send(NetCommand::PreviewInventoryItem(item));
            }
            Ok(submitted(c))
        }
    }
}
fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}
fn read_result(
    app: &mut App,
    request: Uuid,
    pending: Pending,
    item: Uuid,
    asset: Uuid,
    object: Uuid,
    data: Result<&[u8], &String>,
    script: bool,
) -> Value {
    if !super::super::source_current(app, &pending.capture) || !super::super::continuation_current(app, &pending.capture) {
        return failure("stale_source");
    }
    if (object.is_nil() && (info(app, item, false).is_err() || app.world.inventory.items.get(&item).is_none_or(|it| it.asset_id != asset)))
        || (!object.is_nil()
            && app
                .script_info(ScriptTarget { object, item })
                .is_none_or(|info| !info.can_read || info.asset != asset))
    {
        return failure("stale_source_snapshot");
    }
    let Ok(data) = data else {
        return failure("native_source_read_failed");
    };
    if data.len() > 1024 * 1024 {
        return failure("source_limit");
    }
    let card = if script {
        None
    } else {
        match aurora_net::inventory::notecard::Notecard::decode(data) {
            Ok(card) => Some(card),
            Err(_) => return failure("invalid_notecard"),
        }
    };
    let text = if let Some(card) = &card {
        card.text.clone()
    } else {
        match std::str::from_utf8(data) {
            Ok(t) => t.to_owned(),
            Err(_) => return failure("invalid_utf8"),
        }
    };
    let complete = text.len() <= 16 * 1024;
    let result_text = if complete {
        text.clone()
    } else {
        let mut end = 16 * 1024;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text[..end].to_owned()
    };
    let revision = Uuid::new_v4();
    let digest = hash(&text);
    if complete
        && let Some(assistant) = app.character_mcp.as_mut()
        && (assistant.inventory.content.len() < 16 || assistant.inventory.content.contains_key(&item))
    {
        assistant.inventory.content.insert(
            item,
            Snapshot {
                endpoint: pending.capture.endpoint,
                policy_revision: pending.capture.policy_revision,
                client: pending.capture.client,
                expires: Instant::now() + Duration::from_secs(600),
                binding: pending.capture.binding,
                item,
                object,
                asset,
                revision,
                text: text.clone(),
                hash: digest.clone(),
                card,
                experience: None,
            },
        );
    }
    let mut out = json!({"id":item.to_string(),"object_id":(!object.is_nil()).then(||object.to_string()),"source_snapshot":revision.to_string(),"text":result_text,"complete":complete,"truncated":!complete,"bytes":text.len(),"stale":false,"expected_asset_id":revision.to_string(),"content_hash":digest,"best_effort_revision":true,"operation_id":pending.capture.request_id});
    if rules::asset_uuid_shown(&app.world.inventory, asset) {
        out["asset_id"] = json!(asset.to_string());
    } else {
        out["asset_id_protected"] = json!(true);
    }
    let _ = request;
    out
}
pub(super) fn native_event(app: &mut App, event: &NetEvent) -> bool {
    let request = match event {
        NetEvent::ScriptLoaded { request, .. }
        | NetEvent::ScriptSaved { request, .. }
        | NetEvent::ScriptExperiences { request, .. }
        | NetEvent::InventoryContentSaved { request, .. } => Some(*request),
        NetEvent::InventoryPreview { item, asset, .. } => app.character_mcp.as_ref().and_then(|a| {
            a.inventory.pending.iter().find_map(|(id, p)| {
                matches!(p.kind,PendingKind::ContentRead{item:i,asset:a,object}if i==*item&&a==*asset&&object.is_nil()).then_some(*id)
            })
        }),
        _ => None,
    };
    let Some(request) = request else { return false };
    let Some(pending) = app.character_mcp.as_ref().and_then(|a| a.inventory.pending.get(&request)).cloned() else {
        return false;
    };
    if !matches!(
        pending.kind,
        PendingKind::ContentRead { .. } | PendingKind::ContentSave { .. } | PendingKind::Metadata { .. }
    ) {
        return false;
    }
    let source = super::super::source_current(app, &pending.capture);
    let result = match (event, &pending.kind) {
        (NetEvent::ScriptLoaded { target, result, .. }, PendingKind::ContentRead { item, asset, object }) => {
            if target.item != *item || target.object != *object {
                return true;
            }
            read_result(
                app,
                request,
                pending.clone(),
                *item,
                *asset,
                *object,
                result.as_ref().map(|s| s.as_bytes()),
                true,
            )
        }
        (NetEvent::InventoryPreview { result, .. }, PendingKind::ContentRead { item, asset, .. }) => read_result(
            app,
            request,
            pending.clone(),
            *item,
            *asset,
            Uuid::nil(),
            result.as_ref().map(|v| v.as_slice()),
            false,
        ),
        (NetEvent::InventoryContentSaved { result, .. }, PendingKind::ContentSave { item, .. }) => {
            if source && let Ok(saved) = result {
                app.world.inventory.add_items(vec![saved.clone()]);
            }
            json!({"operation_id":pending.capture.request_id,"item_id":item.to_string(),"source_saved":source&&result.is_ok(),"confirmed":source&&result.is_ok(),"retry_safe":false,"status":if result.is_ok(){"confirmed"}else{"uncertain"}})
        }
        (NetEvent::ScriptSaved { result, .. }, PendingKind::ContentSave { item, object, .. }) => {
            if source && let Ok(saved) = result {
                if object.is_nil() {
                    if let Some(mut it) = app.world.inventory.items.get(item).cloned() {
                        it.asset_id = saved.asset;
                        app.world.inventory.add_items(vec![it]);
                    }
                } else if let Some(it) = app
                    .build
                    .contents
                    .objects
                    .get_mut(object)
                    .and_then(|inv| inv.items.iter_mut().find(|it| it.item_id == *item))
                {
                    it.asset_id = saved.asset;
                }
            }
            match result {
                Ok(saved) => {
                    json!({"operation_id":pending.capture.request_id,"source_saved":source,"compiled":if saved.checked{Some(saved.compiled)}else{None},"compiler_checked":saved.checked,"errors":saved.errors.iter().take(16).map(|s|clipped(s,256)).collect::<Vec<_>>(),"confirmed":source,"retry_safe":false,"status":"confirmed"})
                }
                Err(_) => {
                    json!({"operation_id":pending.capture.request_id,"source_saved":null,"compiled":null,"confirmed":false,"retry_safe":false,"status":"uncertain","error_code":"native_save_unconfirmed"})
                }
            }
        }
        (NetEvent::ScriptExperiences { target, result, .. }, PendingKind::Metadata { item, object, read }) => {
            if target.item != *item || target.object != *object {
                return true;
            }
            let native_current = super::super::continuation_current(app, &pending.capture)
                && objects::status_gate(app, &pending.capture).is_ok()
                && app
                    .script_info(ScriptTarget {
                        object: *object,
                        item: *item,
                    })
                    .is_some_and(|info| {
                        app.character_mcp
                            .as_ref()
                            .and_then(|a| a.inventory.content.get(item))
                            .is_some_and(|s| s.asset == info.asset)
                    });
            if !native_current {
                failure("stale_source_or_native_permission")
            } else if let Ok(experience) = result.associated.as_ref() {
                if let Some(snapshot) = app
                    .character_mcp
                    .as_mut()
                    .and_then(|a| a.inventory.content.get_mut(item))
                    .filter(|s| {
                        s.endpoint == pending.capture.endpoint
                            && s.policy_revision == pending.capture.policy_revision
                            && s.client == pending.capture.client
                            && s.binding == pending.capture.binding
                            && s.object == *object
                    })
                {
                    snapshot.experience = Some(*experience);
                    let mut value = read.clone();
                    value["experience_preserved"] = json!(true);
                    value
                } else {
                    failure("stale_source_snapshot")
                }
            } else {
                failure("native_experience_metadata_unavailable")
            }
        }
        _ => return false,
    };
    if let Some(a) = app.character_mcp.as_mut() {
        a.inventory.pending.remove(&request);
    }
    if let PendingKind::ContentRead { item, object, .. } = pending.kind.clone()
        && !object.is_nil()
        && result["complete"] == true
        && super::super::continuation_current(app, &pending.capture)
    {
        let metadata_request = Uuid::new_v4();
        if let Some(a) = app.character_mcp.as_mut() {
            pending_for(
                &mut a.inventory,
                metadata_request,
                &pending.capture,
                PendingKind::Metadata {
                    item,
                    object,
                    read: result,
                },
            );
        }
        if commit_continuation(app, &pending.capture, |app| {
            app.mcp_load_script_metadata(metadata_request, ScriptTarget { object, item })
        })
        .is_ok_and(|result| result.is_ok())
        {
            return true;
        }
        if let Some(a) = app.character_mcp.as_mut() {
            a.inventory.pending.remove(&metadata_request);
        }
        mutations::finish(app, pending, failure("native_experience_metadata_unavailable"));
        return true;
    }
    if source {
        mutations::finish(app, pending, result);
    }
    true
}

pub(super) fn task_read(
    app: &mut App,
    state: &mut State,
    c: &Capture,
    object: Uuid,
    item: Uuid,
    asset: Uuid,
) -> Result<Value, &'static str> {
    let info = app
        .script_info(ScriptTarget { object, item })
        .filter(|i| i.can_read && i.asset == asset)
        .ok_or("native_permission_denied")?;
    let _ = info;
    let request = Uuid::new_v4();
    pending_for(state, request, c, PendingKind::ContentRead { item, asset, object });
    app.mcp_load_script(request, ScriptTarget { object, item })
        .map_err(|_| "native_permission_denied")?;
    Ok(submitted(c))
}
fn task_snapshot<'a>(app: &App, state: &'a State, c: &Capture) -> Result<&'a Snapshot, &'static str> {
    let a = &c.arguments;
    let item = uuid(a, "item_id")?;
    let object = uuid(a, "object_id")?;
    let s = state
        .content
        .get(&item)
        .filter(|s| {
            s.endpoint == c.endpoint
                && s.policy_revision == c.policy_revision
                && s.client == c.client
                && s.binding == c.binding
                && s.object == object
                && s.expires > Instant::now()
                && s.revision == uuid(a, "source_snapshot").unwrap_or_default()
        })
        .ok_or("stale_source_snapshot")?;
    let info = app
        .script_info(ScriptTarget { object, item })
        .filter(|i| i.can_read && i.asset == s.asset)
        .ok_or("native_permission_denied")?;
    let _ = info;
    if uuid(a, "expected_asset_id")? != s.asset && uuid(a, "expected_asset_id")? != s.revision || string(a, "content_hash", 64)? != s.hash {
        return Err("stale_source_snapshot");
    }
    Ok(s)
}
pub(super) fn prepare_task(app: &App, state: &State, c: &Capture) -> Result<(), &'static str> {
    let s = task_snapshot(app, state, c)?;
    let a = &c.arguments;
    string(a, "text", SOURCE_LIMIT)?;
    if a.get("compiler").and_then(Value::as_str) != Some("mono") {
        return Err("unsupported_compiler");
    }
    if c.tool == Tool::ObjectScriptSaveCompile {
        uuid(a, "client_request_id")?;
        let target = ScriptTarget {
            object: s.object,
            item: s.item,
        };
        if app.script_info(target).is_none_or(|i| !i.can_edit) || app.script_editors.documents.iter().any(|d| d.target == target) {
            return Err("native_permission_denied_or_editor_open");
        }
        if s.experience.is_none() {
            return Err("native_experience_metadata_unavailable");
        }
        if !matches!(
            a.get("running_mode").and_then(Value::as_str).unwrap_or("stopped"),
            "stopped" | "preserve"
        ) {
            return Err("invalid_args");
        }
    }
    Ok(())
}
pub(super) fn task_execute(app: &mut App, state: &mut State, c: &Capture) -> Result<Value, &'static str> {
    let s = task_snapshot(app, state, c)?.clone();
    let text = string(&c.arguments, "text", SOURCE_LIMIT)?.to_owned();
    if c.tool == Tool::ObjectScriptPreview {
        return Ok(
            json!({"changed":text!=s.text,"old_bytes":s.text.len(),"new_bytes":text.len(),"source_saved":false,"new_content_hash":hash(&text)}),
        );
    }
    let running = if c.arguments.get("running_mode").and_then(Value::as_str) == Some("preserve") {
        objects::running_value(state, c, s.object, s.item)
            .filter(|(_, _, token, time)| {
                Some(*token) == optional_uuid(&c.arguments, "running_snapshot").ok().flatten() && time.elapsed() < Duration::from_secs(10)
            })
            .map(|r| r.0)
            .ok_or("stale_running_snapshot")?
    } else {
        false
    };
    let request = uuid(&c.arguments, "client_request_id")?;
    if let Some(old) = existing_outcome(state, c, &request.to_string())? {
        return Ok(old);
    }
    objects::execution_changed(state, c.binding, s.object, s.item);
    app.mcp_save_task_script(
        request,
        ScriptTarget {
            object: s.object,
            item: s.item,
        },
        s.asset,
        s.experience.ok_or("native_experience_metadata_unavailable")?,
        running,
        text,
    )
    .map_err(|_| "native_permission_denied")?;
    pending_for(
        state,
        request,
        c,
        PendingKind::ContentSave {
            item: s.item,
            asset: s.asset,
            object: s.object,
            script: true,
        },
    );
    outcome(state, c, request.to_string(), submitted(c));
    Ok(submitted(c))
}

pub(super) fn status_gate(app: &App, original: &Capture) -> Result<(), &'static str> {
    let id = uuid(&original.arguments, "id")?;
    info(app, id, false)?;
    Ok(())
}
