//! Source-retaining exact-prim Contents insertion and explicit unresolved
//! association. A server observation is distinct from native optimistic state.
use super::*;
type Key = (HostId, u64, Uuid);
const INTENT_LIMIT: usize = 4096;
#[derive(Default)]
pub(in crate::app::mcp::inventory) struct State {
    operations: BTreeMap<Key, Put>,
    requests: BTreeMap<Key, Capture>,
    locks: BTreeMap<Uuid, Key>,
    observations: BTreeMap<Uuid, Observation>,
    amendments: Vec<(Capture, Capture, Value)>,
}
#[derive(Clone)]
struct Observation {
    binding: Binding,
    generation: u64,
    identity: Identity,
    serial: i16,
    at: Instant,
    items: Vec<TaskItem>,
    complete: bool,
}
#[derive(Clone)]
struct Put {
    capture: Capture,
    object: Uuid,
    source: InvItem,
    identity: Identity,
    previous: HashSet<Uuid>,
    generation: u64,
    submitted: Instant,
    next_read: Instant,
    reading: bool,
    attempts: u8,
    terminal: bool,
    unresolved: bool,
    receipt: Value,
    association: Option<(Uuid, Uuid, Uuid)>,
}
fn owner(c: &Capture, p: &Put) -> bool {
    p.capture.endpoint == c.endpoint && p.capture.client == c.client && p.capture.binding == c.binding
}
fn observed_candidate(source: &InvItem, previous: &HashSet<Uuid>, items: &[TaskItem], agent: Uuid) -> Option<Uuid> {
    let candidates = items
        .iter()
        .filter(|i| !i.is_folder && !previous.contains(&i.item_id) && i32::from(i.asset_type) == source.asset_type)
        .collect::<Vec<_>>();
    if source.asset_id.is_nil() || candidates.iter().any(|i| i.asset_id.is_nil()) {
        return None;
    }
    let matching = candidates
        .iter()
        .filter(|i| i.asset_id == source.asset_id && i.owner_id == agent && !i.group_owned)
        .collect::<Vec<_>>();
    (matching.len() == 1).then(|| matching[0].item_id)
}
fn operation<'a>(state: &'a super::super::State, c: &Capture) -> Result<&'a Put, &'static str> {
    let id = string(&c.arguments, "operation_id", 128)?;
    state
        .put
        .operations
        .values()
        .find(|p| owner(c, p) && p.capture.request_id == id)
        .ok_or("unknown_operation")
}
pub(super) fn generation(state: &super::super::State, id: Uuid) -> u64 {
    state.put.observations.get(&id).filter(|o| o.complete).map_or(0, |o| o.generation)
}
pub(super) fn locked(state: &super::super::State, object: Uuid) -> bool {
    state.put.locks.contains_key(&object)
}
fn source<'a>(app: &'a App, c: &Capture) -> Result<&'a InvItem, &'static str> {
    let id = uuid(&c.arguments, "source_item")?;
    let item = app.world.inventory.items.get(&id).ok_or("source_unavailable")?;
    if item.owner != app.world.agent_id
        || item.group_owned
        || item.owner_mask & rules::COPY == 0
        || !matches!(item.asset_type, 0 | 1 | 6 | 7 | 10)
        || rules::library(&app.world.inventory, id)
        || rules::in_type(&app.world.inventory, id, 14)
    {
        return Err("native_permission_denied");
    }
    ordinary(app, id)?;
    Ok(item)
}
pub(super) fn prepare(app: &App, state: &super::super::State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    let a = &c.arguments;
    let object = uuid(a, "object_id")?;
    let mut classes = vec![Class::Read, Class::WorldRead, Class::TaskInventoryRead, Class::TaskInventoryWrite];
    match c.tool {
        Tool::ObjectContentsPut => {
            args(c, &["object_id", "snapshot_id", "source_item", "client_request_id", "running"])?;
            identity(app, object)?;
            let request = uuid(a, "client_request_id")?;
            if let Some(old) = state.put.requests.get(&(c.endpoint, c.client, request))
                && (old.tool != c.tool
                    || old.arguments != c.arguments
                    || old.binding != c.binding
                    || old.policy_revision != c.policy_revision)
            {
                return Err("request_id_conflict");
            }
            if state.put.requests.contains_key(&(c.endpoint, c.client, request))
                && !state.put.operations.contains_key(&(c.endpoint, c.client, request))
            {
                return Err("client_request_already_used");
            }
            let snapshot = snapshot(app, state, c)?;
            if state.put.operations.contains_key(&(c.endpoint, c.client, request)) {
                let p = &state.put.operations[&(c.endpoint, c.client, request)];
                if p.capture.arguments != c.arguments {
                    return Err("request_id_conflict");
                }
                return Ok(classes);
            }
            if locked(state, object) {
                return Err("object_mutation_unresolved");
            }
            let item = source(app, c)?;
            if snapshot.items.iter().any(|i| i.item_id == item.id) {
                return Err("source_identity_already_present");
            }
            let running = boolean(a, "running", false)?;
            if running && item.asset_type != 10 {
                return Err("invalid_args");
            }
            if running {
                classes.push(Class::ScriptRun);
            }
            let idx = app.world.objects.index_of_uuid(&object).ok_or("object_unavailable")?;
            crate::build::contents_drop::accepts(&app.world, idx, item)?;
        }
        Tool::ObjectContentsReconcile => {
            args(c, &["object_id", "operation_id", "item_id", "snapshot_id", "client_request_id"])?;
            let reconcile = uuid(a, "client_request_id")?;
            let item = uuid(a, "item_id")?;
            let old = operation(state, c)?;
            if let Some(prior) = state.put.requests.get(&(c.endpoint, c.client, reconcile))
                && (prior.tool != c.tool
                    || prior.arguments != c.arguments
                    || prior.binding != c.binding
                    || prior.policy_revision != c.policy_revision)
            {
                return Err("request_id_conflict");
            }
            if old.object != object {
                return Err("wrong_operation_target");
            }
            if let Some((request, selected, snapshot)) = old.association {
                if request == reconcile && selected == item && snapshot == uuid(a, "snapshot_id")? {
                    return Ok(classes);
                }
                return Err("already_reconciled");
            }
            identity(app, object)?;
            let observed = snapshot(app, state, c)?;
            if !old.unresolved || !locked(state, object) {
                return Err("operation_already_completed");
            }
            if observed.identity != old.identity || observed.observed <= old.generation {
                return Err("fresh_contents_required");
            }
            let candidate = observed
                .items
                .iter()
                .find(|i| i.item_id == item && !i.is_folder)
                .ok_or("item_unavailable")?;
            if old.previous.contains(&item)
                || candidate.owner_id != app.world.agent_id
                || candidate.group_owned
                || i32::from(candidate.asset_type) != old.source.asset_type
                || (!old.source.asset_id.is_nil() && !candidate.asset_id.is_nil() && old.source.asset_id != candidate.asset_id)
            {
                return Err("not_a_reconciliation_candidate");
            }
            if reconcile == uuid(&old.capture.arguments, "client_request_id")?
                || state.put.requests.contains_key(&(c.endpoint, c.client, reconcile))
            {
                return Err("request_id_conflict");
            }
        }
        _ => return Err("unsupported_capability"),
    }
    Ok(classes)
}
pub(super) fn execute(app: &mut App, state: &mut super::super::State, c: &Capture) -> Result<Value, &'static str> {
    prepare(app, state, c)?;
    let id = uuid(&c.arguments, "object_id")?;
    if c.tool == Tool::ObjectContentsReconcile {
        let old = operation(state, c)?.clone();
        if old.association.is_some() {
            return Ok(old.receipt);
        }
        let selected = uuid(&c.arguments, "item_id")?;
        let reconcile = uuid(&c.arguments, "client_request_id")?;
        let snap = uuid(&c.arguments, "snapshot_id")?;
        let key = (
            old.capture.endpoint,
            old.capture.client,
            uuid(&old.capture.arguments, "client_request_id")?,
        );
        let mut value = old.receipt.clone();
        value["status"] = json!("associated_unverified");
        value["task_item_id"] = json!(selected.to_string());
        value["asset_identity_verified"] = json!(false);
        value["copy_outcome"] = json!("unknown");
        value["reconciliation_request_id"] = json!(reconcile.to_string());
        value["reconciliation_snapshot_id"] = json!(snap.to_string());
        if state.put.requests.len() >= INTENT_LIMIT {
            return Err("request_id_capacity");
        }
        state.put.requests.insert((c.endpoint, c.client, reconcile), c.clone());
        let p = state.put.operations.get_mut(&key).ok_or("unknown_operation")?;
        p.receipt = value.clone();
        p.association = Some((reconcile, selected, snap));
        p.terminal = true;
        p.unresolved = false;
        state.put.locks.remove(&id);
        state.put.amendments.push((old.capture.clone(), c.clone(), value.clone()));
        outcome(state, &old.capture, old.capture.request_id.clone(), value.clone());
        return Ok(value);
    }
    let request = uuid(&c.arguments, "client_request_id")?;
    let key = (c.endpoint, c.client, request);
    if let Some(old) = state.put.operations.get(&key) {
        return Ok(old.receipt.clone());
    }
    if state.put.operations.len() >= CAPACITY || state.put.requests.len() >= INTENT_LIMIT {
        return Err("operation_capacity");
    }
    let snapshot = snapshot(app, state, c)?.clone();
    let item = source(app, c)?.clone();
    let idx = object(app, id)?;
    let running = boolean(&c.arguments, "running", false)?;
    let drop = crate::build::contents_drop::item_drop(&app.world, idx, &item, running).ok_or("native_permission_denied")?;
    if drop.consume.is_some() {
        return Err("no_copy_transfer_forbidden");
    }
    let value = json!({"status":"submitted","operation_id":c.request_id,"client_request_id":request.to_string(),"source_retained":true,"confirmed":false,"copy_outcome":"unknown","retry_safe":false,"correlation":"awaiting_fresh_contents"});
    state.put.requests.insert(key, c.clone());
    state.put.operations.insert(
        key,
        Put {
            capture: c.clone(),
            object: id,
            source: item,
            identity: snapshot.identity,
            previous: snapshot.items.iter().map(|i| i.item_id).collect(),
            generation: snapshot.observed,
            submitted: Instant::now(),
            next_read: Instant::now() + Duration::from_millis(500),
            reading: false,
            attempts: 0,
            terminal: false,
            unresolved: true,
            receipt: value.clone(),
            association: None,
        },
    );
    state.put.locks.insert(id, key);
    if let BuildCmd::RezScript { item, .. } = &drop.command {
        execution_changed(state, c.binding, id, item.item_id);
    }
    app.send(NetCommand::Build(drop.command));
    outcome(state, c, c.request_id.clone(), value.clone());
    Ok(value)
}
pub(super) fn observe(state: &mut super::super::State, binding: Binding, id: Uuid, serial: i16, identity: Identity, items: &[TaskItem]) {
    if state.put.observations.len() >= CAPACITY && !state.put.observations.contains_key(&id) {
        return;
    }
    let generation = state
        .put
        .observations
        .get(&id)
        .filter(|o| o.binding == binding && o.identity == identity)
        .map_or(1, |o| o.generation.saturating_add(1));
    let mut ids = HashSet::new();
    let complete = items.len() <= 256
        && items.iter().all(|i| {
            i.name.len() <= 1024
                && i.description.len() <= 4096
                && (i.is_folder || (!i.item_id.is_nil() && i.parent_id == id && ids.insert(i.item_id)))
        });
    state.put.observations.insert(
        id,
        Observation {
            binding,
            generation,
            identity,
            serial,
            at: Instant::now(),
            items: if complete { items.to_vec() } else { Vec::new() },
            complete,
        },
    );
}
fn current(app: &App, assistant: &Assistant, c: &Capture) -> bool {
    super::super::super::binding_matches(app, assistant, c)
        && assistant.runtime.configuration().as_ref() == Some(&app.settings.local_assistant)
        && assistant.runtime.capture_current(c)
        && super::super::super::original_scope_current(app, assistant, c)
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    let runtime = assistant.runtime.clone();
    assistant
        .inventory
        .put
        .operations
        .retain(|_, p| p.unresolved || runtime.has_operation(&p.capture.request_id));
    // UUID intents have no embedded expiry: keep bounded tombstones for this
    // endpoint so pruning a receipt can never turn a replay into another copy.
    let updates = std::mem::take(&mut assistant.inventory.put.amendments);
    for (original, reconcile, value) in updates {
        if super::super::super::binding_matches(app, assistant, &reconcile) && prepare(app, &assistant.inventory, &reconcile).is_ok() {
            let _ = runtime.amend_reconciled(&original, &reconcile, value);
        }
    }
    let keys = assistant
        .inventory
        .put
        .operations
        .iter()
        .filter(|(_, p)| !p.terminal)
        .map(|(key, _)| *key)
        .take(4)
        .collect::<Vec<_>>();
    for key in keys {
        let Some(mut p) = assistant.inventory.put.operations.get(&key).cloned() else {
            continue;
        };
        let observed = assistant.inventory.put.observations.get(&p.object).filter(|o| {
            o.binding == p.capture.binding
                && o.identity == p.identity
                && o.complete
                && o.generation > p.generation
                && o.at > p.submitted
                && app
                    .build
                    .contents
                    .objects
                    .get(&p.object)
                    .is_some_and(|i| i.serial == o.serial && !i.loading)
        });
        if let Some(observed) = observed
            && let Some(verified) = observed_candidate(&p.source, &p.previous, &observed.items, app.world.agent_id)
        {
            p.receipt["status"] = json!("simulator_observed");
            p.receipt["task_item_id"] = json!(verified.to_string());
            p.receipt["asset_identity_verified"] = json!(true);
            p.receipt["copy_outcome"] = json!("observed");
            p.receipt["correlation"] = json!("fresh_contents_verified_asset");
            p.terminal = true;
            p.unresolved = false;
            assistant.inventory.put.locks.remove(&p.object);
        }
        if !p.terminal && p.submitted.elapsed() >= Duration::from_secs(25) {
            p.receipt["status"] = json!("uncertain");
            p.receipt["correlation"] = json!(if p.source.asset_id.is_nil() {
                "source_asset_undisclosed"
            } else {
                "fresh_contents_unresolved"
            });
            p.terminal = true;
        }
        if p.terminal {
            let _ = receipts::record(app, &mut assistant.inventory, &p.capture, &p.receipt);
            outcome(
                &mut assistant.inventory,
                &p.capture,
                p.capture.request_id.clone(),
                p.receipt.clone(),
            );
            let _ = runtime.complete(&p.capture, p.receipt.clone());
            assistant.inventory.put.operations.insert(key, p);
            continue;
        }
        if !current(app, assistant, &p.capture) {
            p.terminal = true;
            p.receipt["status"] = json!("uncertain");
            p.receipt["correlation"] = json!("authority_retired_after_submission");
            assistant.inventory.put.operations.insert(key, p);
            continue;
        }
        if !p.reading
            && Instant::now() >= p.next_read
            && p.attempts < 4
            && app.build.contents.objects.get(&p.object).is_none_or(|i| !i.loading)
            && let Ok(idx) = object(app, p.object)
        {
            let request = Uuid::new_v4();
            let capture = p.capture.clone();
            let object = p.object;
            if runtime
                .continue_action(&capture, || {
                    pending_for(&mut assistant.inventory, request, &capture, PendingKind::PutObserve { object });
                    app.build.request_contents(&app.world, idx, true);
                    app.flush_build();
                })
                .is_ok()
            {
                p.reading = true;
                p.attempts += 1;
                p.next_read = Instant::now() + Duration::from_secs(2);
            }
        }
        assistant.inventory.put.operations.insert(key, p);
    }
}

pub(super) fn native_event(app: &mut App, event: &NetEvent) -> bool {
    let NetEvent::TaskInventory { object, serial, result } = event else {
        return false;
    };
    let pending = app.character_mcp.as_ref().and_then(|a| {
        a.inventory
            .pending
            .iter()
            .find_map(|(id, p)| matches!(p.kind,PendingKind::PutObserve{object:id,..}if id==*object).then_some((*id, p.clone())))
    });
    let Some((request, pending)) = pending else {
        return false;
    };
    if let Some(a) = app.character_mcp.as_mut() {
        a.inventory.pending.remove(&request);
        for p in a.inventory.put.operations.values_mut().filter(|p| p.capture == pending.capture) {
            p.reading = false;
        }
    }
    if super::super::super::source_current(app, &pending.capture)
        && let Ok(items) = result
        && items.len() <= 256
    {
        let inv = app.build.contents.objects.entry(*object).or_default();
        inv.items = items.clone();
        if let Some(serial) = serial {
            inv.serial = *serial;
        }
        inv.loading = false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contents_copy_correlation_requires_fresh_unique_owned_asset_evidence() {
        let agent = Uuid::from_u128(7800);
        let source = InvItem {
            asset_id: Uuid::from_u128(7801),
            asset_type: 10,
            ..Default::default()
        };
        let item = TaskItem {
            item_id: Uuid::from_u128(7802),
            asset_id: source.asset_id,
            asset_type: 10,
            owner_id: agent,
            ..Default::default()
        };
        assert_eq!(
            observed_candidate(&source, &HashSet::new(), std::slice::from_ref(&item), agent),
            Some(item.item_id)
        );
        assert_eq!(
            observed_candidate(&source, &HashSet::from([item.item_id]), std::slice::from_ref(&item), agent),
            None
        );
        let mut other = item.clone();
        other.owner_id = Uuid::from_u128(7803);
        assert_eq!(observed_candidate(&source, &HashSet::new(), &[other], agent), None);
        other = item.clone();
        other.item_id = Uuid::from_u128(7804);
        assert_eq!(
            observed_candidate(&source, &HashSet::new(), &[item.clone(), other.clone()], agent),
            None
        );
        other.asset_id = Uuid::nil();
        assert_eq!(observed_candidate(&source, &HashSet::new(), &[item, other], agent), None);
    }
}
