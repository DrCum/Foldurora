use super::*;
use aurora_net::build::{BuildCmd, TaskItem};
mod put;
pub(super) use put::State as PutState;

#[derive(Clone)]
pub(super) struct Snapshot {
    endpoint: HostId,
    policy_revision: u64,
    client: u64,
    pub(super) expires: Instant,
    binding: Binding,
    pub(super) object: Uuid,
    pub(super) serial: i16,
    observed: u64,
    pub(super) identity: Identity,
    pub(super) items: Vec<TaskItem>,
}
#[derive(Clone)]
pub(super) struct Running {
    pub(super) endpoint: Option<HostId>,
    pub(super) policy_revision: u64,
    pub(super) client: u64,
    in_flight: bool,
    execution: u64,
    binding: Binding,
    requested: Instant,
    pending: Option<Capture>,
    uncertain: bool,
    pub(super) value: Option<(bool, Option<bool>, Uuid, Instant)>,
}

pub(super) fn supports(t: Tool) -> bool {
    matches!(
        t,
        Tool::ObjectContentsPut
            | Tool::ObjectContentsReconcile
            | Tool::ObjectContentsList
            | Tool::ObjectScriptStatus
            | Tool::ObjectContentsRename
            | Tool::ObjectContentsRemove
            | Tool::ObjectScriptSetRunning
            | Tool::ObjectScriptReset
            | Tool::ObjectScriptGet
            | Tool::ObjectScriptPreview
            | Tool::ObjectScriptSaveCompile
    )
}
pub(super) fn object(app: &App, id: Uuid) -> Result<usize, &'static str> {
    let idx = app.world.objects.index_of_uuid(&id).ok_or("object_unavailable")?;
    let o = app.world.objects.get(idx).ok_or("object_unavailable")?;
    if (!o.owner_id.is_nil() && o.owner_id != app.world.agent_id)
        || app
            .build
            .props
            .get(&id)
            .is_some_and(|p| !p.owner_id.is_nil() && p.owner_id != app.world.agent_id)
        || (o.owner_id != app.world.agent_id && !app.build.props.get(&id).is_some_and(|p| p.owner_id == app.world.agent_id))
    {
        return Err("native_permission_denied");
    }
    if !app.build.contents_editable(&app.world, idx) || !app.world.rlva_can_edit(idx) {
        return Err("native_permission_denied");
    }
    Ok(idx)
}
pub(super) fn snapshot<'a>(app: &App, state: &'a State, c: &Capture) -> Result<&'a Snapshot, &'static str> {
    let id = uuid(&c.arguments, "object_id")?;
    let token = uuid(&c.arguments, "snapshot_id")?;
    let s = state
        .task
        .get(&token)
        .filter(|s| {
            s.endpoint == c.endpoint
                && s.policy_revision == c.policy_revision
                && s.client == c.client
                && s.binding == c.binding
                && s.object == id
                && s.expires > Instant::now()
        })
        .ok_or("stale_contents_snapshot")?;
    if identity(app, id).as_ref() != Ok(&s.identity) {
        return Err("stale_contents_snapshot");
    }
    if app
        .build
        .contents
        .objects
        .get(&id)
        .is_none_or(|i| i.loading || i.serial != s.serial || i.items != s.items)
    {
        return Err("stale_contents_snapshot");
    }
    Ok(s)
}
pub(super) fn item<'a>(app: &App, state: &'a State, c: &Capture) -> Result<&'a TaskItem, &'static str> {
    let s = snapshot(app, state, c)?;
    s.items
        .iter()
        .find(|i| i.item_id == uuid(&c.arguments, "item_id").unwrap_or_default() && !i.is_folder)
        .ok_or("item_unavailable")
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Identity {
    region: u64,
    local: u32,
    owner: Uuid,
    pub(super) root: Uuid,
}
fn identity(app: &App, id: Uuid) -> Result<Identity, &'static str> {
    let idx = object(app, id)?;
    let o = app.world.objects.get(idx).ok_or("object_unavailable")?;
    let (root, _) = crate::build::contents_drop::linkset(&app.world, idx).ok_or("object_unavailable")?;
    Ok(Identity {
        region: o.key.region,
        local: o.key.local_id,
        owner: o.owner_id,
        root: app.world.objects.get(root).ok_or("object_unavailable")?.full_id,
    })
}
fn observe(app: &mut App, event: &NetEvent) {
    let NetEvent::TaskInventory {
        object,
        serial: Some(serial),
        result: Ok(items),
    } = event
    else {
        return;
    };
    let Some(a) = app.character_mcp.as_ref() else {
        return;
    };
    let Some(binding) = a.binding else {
        return;
    };
    if app
        .world
        .login
        .as_ref()
        .is_none_or(|login| Some(login.session_id) != a.native_login || login.agent_id != app.world.agent_id)
    {
        return;
    }
    let Ok(identity) = identity(app, *object) else {
        return;
    };
    if let Some(a) = app.character_mcp.as_mut() {
        put::observe(&mut a.inventory, binding, *object, *serial, identity, items);
    }
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    put::tick(app, assistant);
}
pub(super) fn prepare(app: &mut App, state: &State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    let a = &c.arguments;
    let id = uuid(a, "object_id")?;
    if matches!(c.tool, Tool::ObjectContentsPut | Tool::ObjectContentsReconcile) {
        return put::prepare(app, state, c);
    }
    object(app, id)?;
    let mut classes = vec![Class::WorldRead, Class::TaskInventoryRead];
    if !c.tool.definition().read_only && put::locked(state, id) {
        return Err("object_mutation_unresolved");
    }
    match c.tool {
        Tool::ObjectContentsList => {
            args(c, &["object_id", "snapshot_id", "offset", "limit"])?;
            limit(a, 50, 50)?;
            number(a, "offset", 0, 256)?;
            if a.contains_key("snapshot_id") {
                snapshot(app, state, c)?;
            }
        }
        Tool::ObjectScriptStatus => {
            args(c, &["object_id", "item_id"])?;
            let item = uuid(a, "item_id")?;
            let info = app
                .script_info(aurora_net::scripts::ScriptTarget { object: id, item })
                .ok_or("item_unavailable")?;
            if !info.can_read {
                return Err("native_permission_denied");
            }
            if state.execution_uncertain {
                return Err("running_state_uncertain");
            }
            if state
                .running
                .get(&(id, item))
                .is_some_and(|r| r.binding == c.binding && (r.uncertain || r.in_flight))
            {
                return Err("running_state_pending_or_uncertain");
            }
        }
        Tool::ObjectScriptGet => {
            args(c, &["object_id", "item_id", "snapshot_id"])?;
            let it = item(app, state, c)?;
            if it.asset_type != 10 {
                return Err("unsupported_item_type");
            }
            classes.push(Class::Read);
        }
        Tool::ObjectScriptPreview | Tool::ObjectScriptSaveCompile => {
            args(
                c,
                &[
                    "object_id",
                    "item_id",
                    "source_snapshot",
                    "expected_asset_id",
                    "content_hash",
                    "text",
                    "compiler",
                    "client_request_id",
                    "running_mode",
                    "running_snapshot",
                ],
            )?;
            content::prepare_task(app, state, c)?;
            classes.push(Class::Read);
            if c.tool == Tool::ObjectScriptSaveCompile {
                classes.push(Class::TaskInventoryWrite);
                if a.get("running_mode").and_then(Value::as_str) == Some("preserve") {
                    let key = (id, uuid(a, "item_id")?);
                    let running = running_value(state, c, key.0, key.1)
                        .filter(|(_, _, token, time)| {
                            Some(*token) == optional_uuid(a, "running_snapshot").ok().flatten() && time.elapsed() < Duration::from_secs(10)
                        })
                        .ok_or("stale_running_snapshot")?;
                    if running.0 {
                        classes.push(Class::ScriptRun);
                    }
                }
            }
        }
        Tool::ObjectContentsRename | Tool::ObjectContentsRemove | Tool::ObjectScriptSetRunning | Tool::ObjectScriptReset => {
            args(c, &["object_id", "item_id", "snapshot_id", "client_request_id", "name", "running"])?;
            uuid(a, "client_request_id")?;
            let it = item(app, state, c)?;
            if it.owner_id != app.world.agent_id || it.owner_mask & (rules::COPY | rules::MODIFY) != (rules::COPY | rules::MODIFY) {
                return Err("native_permission_denied");
            }
            if matches!(c.tool, Tool::ObjectScriptSetRunning | Tool::ObjectScriptReset) && it.asset_type != 10 {
                return Err("unsupported_item_type");
            }
            if c.tool == Tool::ObjectContentsRename {
                rules::clean_name(string(a, "name", 63)?).map_err(|_| "invalid_name")?;
            }
            if c.tool == Tool::ObjectScriptSetRunning {
                if !a.contains_key("running") {
                    return Err("invalid_args");
                }
                boolean(a, "running", false)?;
            }
            classes.push(Class::Read);
        }
        _ => return Err("unsupported_capability"),
    }
    Ok(classes)
}
pub(super) fn execute(app: &mut App, state: &mut State, c: &Capture) -> Result<Value, &'static str> {
    state.admit()?;
    let id = uuid(&c.arguments, "object_id")?;
    if matches!(c.tool, Tool::ObjectContentsPut | Tool::ObjectContentsReconcile) {
        return put::execute(app, state, c);
    }
    let idx = object(app, id)?;
    match c.tool {
        Tool::ObjectContentsList => {
            if c.arguments.contains_key("snapshot_id") {
                let s = snapshot(app, state, c)?;
                return page(
                    s,
                    uuid(&c.arguments, "snapshot_id")?,
                    number(&c.arguments, "offset", 0, 256)?,
                    limit(&c.arguments, 50, 50)?,
                );
            }
            if state
                .pending
                .values()
                .any(|p| matches!(p.kind,PendingKind::Contents{object}if object==id))
                || app.build.contents.objects.get(&id).is_some_and(|i| i.loading)
            {
                return Err("native_read_pending");
            }
            let request = Uuid::new_v4();
            pending_for(state, request, c, PendingKind::Contents { object: id });
            app.build.request_contents(&app.world, idx, true);
            app.flush_build();
            Ok(submitted(c))
        }
        Tool::ObjectScriptStatus => {
            let target = uuid(&c.arguments, "item_id")?;
            if state.running.len() >= CAPACITY && !state.running.contains_key(&(id, target)) {
                return Err("query_capacity");
            }
            state.running.insert(
                (id, target),
                Running {
                    endpoint: Some(c.endpoint),
                    policy_revision: c.policy_revision,
                    client: c.client,
                    in_flight: true,
                    execution: state.execution.get(&(id, target)).copied().unwrap_or(0),
                    binding: c.binding,
                    requested: Instant::now(),
                    pending: Some(c.clone()),
                    uncertain: false,
                    value: None,
                },
            );
            let handle = app.world.objects.get(idx).ok_or("object_unavailable")?.key.region;
            app.send(NetCommand::Build(BuildCmd::GetScriptRunning {
                handle,
                object_id: id,
                item_id: target,
            }));
            Ok(submitted(c))
        }
        Tool::ObjectScriptGet => {
            let it = item(app, state, c)?.clone();
            content::task_read(app, state, c, id, it.item_id, it.asset_id)
        }
        Tool::ObjectScriptPreview | Tool::ObjectScriptSaveCompile => content::task_execute(app, state, c),
        _ => {
            let it = item(app, state, c)?.clone();
            let request = uuid(&c.arguments, "client_request_id")?;
            if let Some(existing) = existing_outcome(state, c, &request.to_string())? {
                return Ok(existing);
            }
            match c.tool {
                Tool::ObjectContentsRename => app
                    .build
                    .rename_item(&app.world, idx, it.item_id, string(&c.arguments, "name", 63)?),
                Tool::ObjectContentsRemove => app.build.remove_item(&app.world, idx, it.item_id),
                Tool::ObjectScriptSetRunning => {
                    execution_changed(state, c.binding, id, it.item_id);
                    app.build
                        .set_script_running(&app.world, idx, it.item_id, boolean(&c.arguments, "running", false)?)
                }
                Tool::ObjectScriptReset => {
                    execution_changed(state, c.binding, id, it.item_id);
                    let handle = app.world.objects.get(idx).ok_or("object_unavailable")?.key.region;
                    app.send(NetCommand::Build(BuildCmd::ScriptReset {
                        handle,
                        object_id: id,
                        item_id: it.item_id,
                    }));
                }
                _ => return Err("unsupported_capability"),
            }
            app.flush_build();
            let value = json!({"status":"submitted","operation_id":c.request_id,"client_request_id":request.to_string(),"confirmed":false,"retry_safe":false});
            outcome(state, c, request.to_string(), value.clone());
            outcome(state, c, c.request_id.clone(), value.clone());
            // No simulator transaction nonce exists for these writes. A later
            // explicit snapshot/status observation is required; never call the
            // optimistic native inventory cache a server acknowledgement.
            Ok(value)
        }
    }
}
fn page(s: &Snapshot, token: Uuid, offset: usize, max: usize) -> Result<Value, &'static str> {
    let mut rows = Vec::new();
    let mut bytes = 0usize;
    for i in s.items.iter().filter(|i| !i.is_folder).skip(offset).take(max) {
        let value = json!({"item_id":i.item_id.to_string(),"parent_id":i.parent_id.to_string(),"name":clipped(&i.name,128),"description":clipped(&i.description,256),"asset_type":i.asset_type,"inventory_type":i.inv_type,"creator_id":i.creator_id.to_string(),"copyable":i.owner_mask&rules::COPY!=0,"modifiable":i.owner_mask&rules::MODIFY!=0,"transferable":i.owner_mask&rules::TRANSFER!=0});
        let size = serde_json::to_vec(&value).map_err(|_| "native_metadata_limit")?.len();
        if bytes.saturating_add(size) > 18 * 1024 {
            break;
        }
        bytes += size;
        rows.push(value);
    }
    let total = s.items.iter().filter(|i| !i.is_folder).count();
    let next = offset.saturating_add(rows.len());
    Ok(
        json!({"object_id":s.object.to_string(),"snapshot_id":token.to_string(),"serial":s.serial,"items":rows,"total":total,"offset":offset,"next_offset":if next<total{Some(next)}else{None},"expires_in_seconds":s.expires.saturating_duration_since(Instant::now()).as_secs(),"server_observed":true}),
    )
}

/// All native execution changes invalidate issued running tokens, including
/// user actions which did not originate in the assistant.
pub(super) fn execution_changed(state: &mut State, binding: Binding, object: Uuid, item: Uuid) {
    if object.is_nil() {
        return;
    }
    execution_binding(state, binding);
    let key = (object, item);
    if state.execution.len() >= CAPACITY && !state.execution.contains_key(&key) {
        state.execution_uncertain = true;
        for r in state.running.values_mut() {
            r.value = None;
            r.uncertain = true;
        }
        return;
    }
    let revision = state.execution.entry(key).or_default();
    if let Some(next) = revision.checked_add(1) {
        *revision = next;
    } else {
        state.execution_uncertain = true;
    }
    if let Some(r) = state.running.get_mut(&key) {
        r.value = None;
        if r.in_flight {
            r.uncertain = true;
        }
    }
}
fn execution_binding(state: &mut State, binding: Binding) {
    if state.execution_binding != Some(binding) {
        state.execution_binding = Some(binding);
        state.execution.clear();
        state.execution_uncertain = false;
        state.running.clear();
    }
}
pub(super) fn running_value(state: &State, c: &Capture, object: Uuid, item: Uuid) -> Option<(bool, Option<bool>, Uuid, Instant)> {
    if state.execution_uncertain {
        return None;
    }
    state
        .running
        .get(&(object, item))
        .filter(|r| {
            r.binding == c.binding
                && r.client == c.client
                && r.endpoint == Some(c.endpoint)
                && r.policy_revision == c.policy_revision
                && !r.uncertain
                && !r.in_flight
                && r.execution == state.execution.get(&(object, item)).copied().unwrap_or(0)
        })
        .and_then(|r| r.value)
}
pub(super) fn observation(state: &State, object: Uuid) -> u64 {
    put::generation(state, object)
}
pub(super) fn recipe_query(app: &mut App, state: &mut State, c: &Capture, object_id: Uuid, item_id: Uuid) -> Result<(), &'static str> {
    let idx = object(app, object_id)?;
    let info = app
        .script_info(aurora_net::scripts::ScriptTarget {
            object: object_id,
            item: item_id,
        })
        .ok_or("item_unavailable")?;
    if !info.can_read {
        return Err("native_permission_denied");
    }
    if state.execution_uncertain || state.running.get(&(object_id, item_id)).is_some_and(|q| q.in_flight || q.uncertain) {
        return Err("native_query_busy");
    }
    if state.running.len() >= CAPACITY && !state.running.contains_key(&(object_id, item_id)) {
        return Err("query_capacity");
    }
    state.running.insert(
        (object_id, item_id),
        Running {
            endpoint: Some(c.endpoint),
            policy_revision: c.policy_revision,
            client: c.client,
            in_flight: true,
            execution: state.execution.get(&(object_id, item_id)).copied().unwrap_or(0),
            binding: c.binding,
            requested: Instant::now(),
            pending: None,
            uncertain: false,
            value: None,
        },
    );
    app.send(NetCommand::Build(BuildCmd::GetScriptRunning {
        handle: app.world.objects.get(idx).ok_or("object_unavailable")?.key.region,
        object_id,
        item_id,
    }));
    Ok(())
}
fn execution_target(command: &NetCommand) -> Option<(Uuid, Uuid)> {
    match command {
        NetCommand::SaveScript { target, .. } => Some((target.object, target.item)),
        NetCommand::Build(BuildCmd::SetScriptRunning { object_id, item_id, .. } | BuildCmd::ScriptReset { object_id, item_id, .. }) => {
            Some((*object_id, *item_id))
        }
        NetCommand::Build(BuildCmd::RezScript { item, .. }) => Some((item.parent_id, item.item_id)),
        _ => None,
    }
}
pub(super) fn native_command(app: &mut App, command: &NetCommand) {
    let Some(a) = app.character_mcp.as_mut() else {
        return;
    };
    let Some(binding) = a.binding else {
        return;
    };
    if let Some((object, item)) = execution_target(command) {
        execution_changed(&mut a.inventory, binding, object, item);
        return;
    }
    if let NetCommand::Build(BuildCmd::GetScriptRunning { object_id, item_id, .. }) = command {
        execution_binding(&mut a.inventory, binding);
        let key = (*object_id, *item_id);
        let revision = a.inventory.execution.get(&key).copied().unwrap_or(0);
        if let Some(r) = a.inventory.running.get_mut(&key) {
            if r.in_flight {
                r.uncertain = true;
            }
            r.in_flight = true;
            r.execution = revision;
            r.requested = Instant::now();
            r.value = None;
        } else if a.inventory.running.len() < CAPACITY {
            a.inventory.running.insert(
                key,
                Running {
                    endpoint: None,
                    policy_revision: 0,
                    client: 0,
                    in_flight: true,
                    execution: revision,
                    binding,
                    requested: Instant::now(),
                    pending: None,
                    uncertain: a.inventory.execution_uncertain,
                    value: None,
                },
            );
        }
    }
}
pub(super) fn native_event(app: &mut App, event: &NetEvent) -> bool {
    observe(app, event);
    if put::native_event(app, event) {
        return true;
    }
    match event {
        NetEvent::TaskInventory { object, serial, result } => {
            let found = app.character_mcp.as_ref().and_then(|a| {
                a.inventory
                    .pending
                    .iter()
                    .find_map(|(id, p)| matches!(p.kind,PendingKind::Contents{object:id}if id==*object).then_some((*id, p.clone())))
            });
            let Some((request, pending)) = found else { return false };
            let source = super::super::source_current(app, &pending.capture);
            let allowed = super::super::continuation_current(app, &pending.capture);
            if let Some(a) = app.character_mcp.as_mut() {
                a.inventory.pending.remove(&request);
            }
            if !source {
                return true;
            }
            if let Some(inv) = app.build.contents.objects.get_mut(object) {
                inv.loading = false;
            }
            let value = if let Ok(items) = result {
                if items.len() > 256 || items.iter().any(|i| i.name.len() > 1024 || i.description.len() > 4096) {
                    failure("contents_limit")
                } else if let Some(serial) = serial {
                    let snapshot = Snapshot {
                        endpoint: pending.capture.endpoint,
                        policy_revision: pending.capture.policy_revision,
                        client: pending.capture.client,
                        expires: Instant::now() + Duration::from_secs(120),
                        binding: pending.capture.binding,
                        object: *object,
                        serial: *serial,
                        observed: app.character_mcp.as_ref().map_or(0, |a| put::generation(&a.inventory, *object)),
                        identity: identity(app, *object).unwrap_or_default(),
                        items: items.clone(),
                    };
                    let token = Uuid::new_v4();
                    if let Some(inv) = app.build.contents.objects.get_mut(object) {
                        inv.items = items.clone();
                        inv.serial = *serial;
                    }
                    if snapshot.observed == 0 {
                        failure("native_observation_unavailable")
                    } else if allowed && object_check(app, *object) {
                        if app.character_mcp.as_ref().is_some_and(|a| a.inventory.task.len() >= 32) {
                            failure("snapshot_capacity")
                        } else {
                            if let Some(a) = app.character_mcp.as_mut() {
                                a.inventory.task.insert(token, snapshot.clone());
                            }
                            page(&snapshot, token, 0, 50).unwrap_or_else(failure)
                        }
                    } else {
                        failure("stale_source")
                    }
                } else {
                    failure("native_contents_revision_unavailable")
                }
            } else {
                failure("native_contents_read_failed")
            };
            mutations::finish(app, pending, value);
            true
        }
        NetEvent::ScriptRunning {
            object_id,
            item_id,
            running,
            mono,
        } => {
            let key = (*object_id, *item_id);
            let Some(mut query) = app.character_mcp.as_mut().and_then(|a| a.inventory.running.remove(&key)) else {
                return false;
            };
            let pending = query.pending.take();
            if pending.as_ref().is_some_and(|c| !super::super::source_current(app, c)) {
                return true;
            }
            query.in_flight = false;
            let token = Uuid::new_v4();
            let execution_current = app.character_mcp.as_ref().is_some_and(|a| {
                !a.inventory.execution_uncertain && a.inventory.execution.get(&key).copied().unwrap_or(0) == query.execution
            });
            let valid = !query.uncertain && execution_current && query.requested.elapsed() < Duration::from_secs(10);
            if valid {
                query.value = Some((*running, *mono, token, Instant::now()));
            } else {
                query.uncertain = true;
            }
            if let Some(a) = app.character_mcp.as_mut() {
                a.inventory.running.insert(key, query);
            }
            if let Some(capture) = pending {
                let p = Pending {
                    capture,
                    started: Instant::now(),
                    kind: PendingKind::Running {
                        object: *object_id,
                        item: *item_id,
                    },
                };
                let value = if valid && super::super::continuation_current(app, &p.capture) {
                    json!({"object_id":object_id.to_string(),"item_id":item_id.to_string(),"running":running,"mono":mono,"running_snapshot":token.to_string(),"expires_in_seconds":10,"server_observed":true})
                } else {
                    failure("running_state_uncertain")
                };
                mutations::finish(app, p, value);
            }
            false
        }
        _ => false,
    }
}
fn object_check(app: &App, id: Uuid) -> bool {
    object(app, id).is_ok()
}

pub(super) fn status_gate(app: &App, original: &Capture) -> Result<(), &'static str> {
    let object_id = uuid(&original.arguments, "object_id")?;
    object(app, object_id)?;
    if matches!(
        original.tool,
        Tool::ObjectScriptGet | Tool::ObjectScriptPreview | Tool::ObjectScriptSaveCompile | Tool::ObjectScriptStatus
    ) && (app.world.rlva.has("viewscript")
        || app
            .script_info(aurora_net::scripts::ScriptTarget {
                object: object_id,
                item: uuid(&original.arguments, "item_id")?,
            })
            .is_none_or(|i| !i.can_read))
    {
        return Err("native_permission_denied");
    }
    Ok(())
}

#[cfg(test)]
mod execution_tests {
    use super::*;
    fn ready(state: &mut State, c: &Capture, object: Uuid, item: Uuid) {
        state.execution_binding = Some(c.binding);
        state.running.insert(
            (object, item),
            Running {
                endpoint: Some(c.endpoint),
                policy_revision: c.policy_revision,
                client: c.client,
                in_flight: false,
                execution: state.execution.get(&(object, item)).copied().unwrap_or(0),
                binding: c.binding,
                requested: Instant::now(),
                pending: None,
                uncertain: false,
                value: Some((true, Some(true), Uuid::from_u128(8001), Instant::now())),
            },
        );
    }
    #[test]
    fn native_stop_reset_and_save_invalidate_preserve_snapshot() {
        let c = synthetic_capture(Tool::ObjectScriptSaveCompile);
        let object = Uuid::from_u128(8002);
        let item = Uuid::from_u128(8003);
        let commands = [
            NetCommand::Build(BuildCmd::SetScriptRunning {
                handle: 1,
                object_id: object,
                item_id: item,
                running: false,
            }),
            NetCommand::Build(BuildCmd::ScriptReset {
                handle: 1,
                object_id: object,
                item_id: item,
            }),
            NetCommand::SaveScript {
                request: Uuid::from_u128(8004),
                target: aurora_net::scripts::ScriptTarget { object, item },
                handle: 1,
                options: aurora_net::scripts::ScriptOptions {
                    mono: true,
                    running: false,
                    experience: Uuid::nil(),
                },
                source: "synthetic source".into(),
            },
        ];
        for command in commands {
            let mut state = State::default();
            ready(&mut state, &c, object, item);
            assert!(running_value(&state, &c, object, item).is_some());
            let target = execution_target(&command).expect("every native execution-changing route is observed");
            execution_changed(&mut state, c.binding, target.0, target.1);
            assert!(running_value(&state, &c, object, item).is_none());
            assert!(state.running[&(object, item)].value.is_none());
        }
    }
    #[test]
    fn native_mutation_during_probe_is_uncertain_and_revision_never_wraps() {
        let c = synthetic_capture(Tool::ObjectScriptStatus);
        let object = Uuid::from_u128(8010);
        let item = Uuid::from_u128(8011);
        let mut state = State::default();
        ready(&mut state, &c, object, item);
        state.running.get_mut(&(object, item)).unwrap().in_flight = true;
        execution_changed(&mut state, c.binding, object, item);
        assert!(state.running[&(object, item)].uncertain);
        state.execution.insert((object, item), u64::MAX);
        execution_changed(&mut state, c.binding, object, item);
        assert!(state.execution_uncertain);
        assert_eq!(state.execution[&(object, item)], u64::MAX);
        assert!(running_value(&state, &c, object, item).is_none());
    }
}
