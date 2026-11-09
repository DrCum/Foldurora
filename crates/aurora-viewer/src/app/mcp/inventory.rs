//! Closed native inventory adapters. No caller-provided LLSD or network command
//! crosses this boundary. Folderstorm source: abbe3cb0ca66ee2484f277219f6a27706fab7fc6.
use super::*;
use crate::world::inventory::{FetchState, Inventory, actions as rules};
use aurora_llsd::llsd_map;
use aurora_net::{
    NetCommand, NetEvent,
    inventory::{InvItem, operations::Mutation},
};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

mod content;
mod diagnostics;
mod interactions;
mod landmarks;
mod library;
mod links;
mod mutations;
mod objects;
mod preparation;
mod reads;
mod receipts;
mod testing;
mod uploads;

const CAPACITY: usize = 128;
const ROW_LIMIT: usize = 50;
const PLAN_LIMIT: usize = 32;
const SOURCE_LIMIT: usize = 65_536;

#[derive(Default)]
pub(super) struct State {
    uploads: uploads::State,
    interactions: interactions::State,
    diagnostics: diagnostics::State,
    testing: testing::State,
    binding: Option<Binding>,
    endpoint: Option<HostId>,
    receipts: BTreeMap<(HostId, u64, String), receipts::Proof>,
    loading: BTreeMap<(HostId, u64, String), preparation::Job>,
    load_requests: BTreeMap<Uuid, preparation::Request>,
    replacements: BTreeMap<(HostId, u64, String), links::Operation>,
    library: BTreeMap<Uuid, library::Group>,
    pending: BTreeMap<Uuid, Pending>,
    plans: BTreeMap<Uuid, Plan>,
    history: VecDeque<History>,
    permissions: BTreeMap<String, PermissionSnapshot>,
    content: BTreeMap<Uuid, content::Snapshot>,
    task: BTreeMap<Uuid, objects::Snapshot>,
    running: BTreeMap<(Uuid, Uuid), objects::Running>,
    put: objects::PutState,
    execution_binding: Option<Binding>,
    execution: BTreeMap<(Uuid, Uuid), u64>,
    execution_uncertain: bool,
    outcomes: BTreeMap<String, Value>,
    outcome_captures: BTreeMap<String, Capture>,
    evidence: std::cell::RefCell<BTreeMap<(HostId, u64, String), (Binding, u64, [u8; 32], Instant)>>,
}
#[derive(Clone)]
struct Pending {
    capture: Capture,
    started: Instant,
    kind: PendingKind,
}
#[derive(Clone)]
enum PendingKind {
    Inventory {
        before: Vec<Frozen>,
        after: Vec<Expected>,
        history: bool,
        class: Class,
    },
    ContentRead {
        item: Uuid,
        asset: Uuid,
        object: Uuid,
    },
    ContentSave {
        item: Uuid,
        object: Uuid,
        script: bool,
    },
    Contents {
        object: Uuid,
    },
    Running,
    Metadata {
        item: Uuid,
        object: Uuid,
        read: Value,
    },
    Created {
        parent: Uuid,
        kind: i32,
    },
    PutObserve {
        object: Uuid,
    },
    Links {
        stage: links::Stage,
    },
    Landmark {
        item: Box<Frozen>,
        asset: Uuid,
        region: Option<(Uuid, glam::Vec3)>,
    },
    Appearance {
        complete: bool,
    },
    LibraryCopy {
        group: Uuid,
        index: usize,
    },
}
#[derive(Clone)]
struct PermissionSnapshot {
    endpoint: HostId,
    policy_revision: u64,
    client: u64,
    binding: Binding,
    item: InvItem,
    expires: Instant,
}
#[derive(Clone)]
struct Frozen {
    id: Uuid,
    item: Option<InvItem>,
    folder: Option<(Uuid, String, i32, i32)>,
}
impl Frozen {
    fn capture(inv: &Inventory, id: Uuid) -> Result<Self, &'static str> {
        if let Some(item) = inv.items.get(&id) {
            if item.name.len() > 1024 || item.desc.len() > 4096 {
                return Err("native_metadata_limit");
            }
            return Ok(Self {
                id,
                item: Some(item.clone()),
                folder: None,
            });
        }
        let f = inv.folders.get(&id).ok_or("not_found")?;
        if f.info.name.len() > 1024 {
            return Err("native_metadata_limit");
        }
        Ok(Self {
            id,
            item: None,
            folder: Some((f.info.parent, f.info.name.clone(), f.info.type_default, f.info.version)),
        })
    }
    fn current(&self, inv: &Inventory) -> bool {
        self.item.as_ref().map_or_else(
            || {
                self.folder.as_ref().is_some_and(|old| {
                    inv.folders
                        .get(&self.id)
                        .is_some_and(|f| old == &(f.info.parent, f.info.name.clone(), f.info.type_default, f.info.version))
                })
            },
            |old| inv.items.get(&self.id) == Some(old),
        )
    }
    fn parent(&self) -> Uuid {
        self.item
            .as_ref()
            .map(|i| i.parent)
            .or_else(|| self.folder.as_ref().map(|f| f.0))
            .unwrap_or_default()
    }
    fn name(&self) -> &str {
        self.item
            .as_ref()
            .map(|i| i.name.as_str())
            .or_else(|| self.folder.as_ref().map(|f| f.1.as_str()))
            .unwrap_or("")
    }
}
#[derive(Clone)]
struct Expected {
    id: Uuid,
    parent: Option<Uuid>,
    name: Option<String>,
}
#[derive(Clone)]
struct Plan {
    endpoint: HostId,
    policy_revision: u64,
    client: u64,
    binding: Binding,
    expires: Instant,
    class: Class,
    generation: u64,
    confirm_only: bool,
    before: Vec<Frozen>,
    after: Vec<Expected>,
    change: Mutation,
    executed: Option<String>,
}
#[derive(Clone)]
struct History {
    endpoint: HostId,
    policy_revision: u64,
    client: u64,
    id: String,
    binding: Binding,
    before: Vec<Frozen>,
    after: Vec<Expected>,
    class: Class,
    confirmed: bool,
    result: Value,
}

impl State {
    fn bind(&mut self, binding: Binding, endpoint: HostId) {
        if self.binding != Some(binding) || self.endpoint != Some(endpoint) {
            self.binding = Some(binding);
            self.endpoint = Some(endpoint);
            self.receipts.clear();
            self.loading.clear();
            self.replacements.clear();
            self.library.clear();
            self.uploads.reset();
            self.put = Default::default();
            self.plans.clear();
            self.history.clear();
            self.permissions.clear();
            self.content.clear();
            self.task.clear();
            if self.execution_binding != Some(binding) {
                self.execution_binding = Some(binding);
                self.running.clear();
                self.execution.clear();
                self.execution_uncertain = false;
            } else {
                for query in self.running.values_mut() {
                    query.value = None;
                }
            }
            self.outcomes.clear();
            self.outcome_captures.clear();
            self.evidence.borrow_mut().retain(|key, e| e.0 == binding && key.0 == endpoint);
            // Keep old pending identities until their callbacks arrive, so an
            // old-login reply is consumed rather than reaching native new state.
        }
        let now = Instant::now();
        self.plans.retain(|_, p| p.expires > now);
        self.permissions.retain(|_, p| p.expires > now);
        self.content.retain(|_, s| s.expires > now);
        self.task.retain(|_, s| s.expires > now);
        self.pending
            .retain(|_, p| now.saturating_duration_since(p.started) < Duration::from_secs(600));
    }
    fn admit(&self) -> Result<(), &'static str> {
        if self.pending.len() >= CAPACITY || self.outcomes.len() >= CAPACITY {
            Err("operation_capacity")
        } else {
            Ok(())
        }
    }
}

pub(super) fn supports(tool: Tool) -> bool {
    reads::supports(tool)
        || mutations::supports(tool)
        || content::supports(tool)
        || objects::supports(tool)
        || interactions::supports(tool)
        || diagnostics::supports(tool)
        || testing::supports(tool)
        || uploads::supports(tool)
}
pub(super) fn original_receipt(state: &State, c: &Capture) -> Result<Option<Capture>, &'static str> {
    if uploads::supports(c.tool) {
        return uploads::original(state, c);
    }
    if matches!(c.tool, Tool::InventoryContentOperationStatus | Tool::ObjectOperationStatus) {
        let id = string(&c.arguments, "operation_id", 128)?;
        if let Some(original) = state.outcome_captures.get(id) {
            if original.client != c.client
                || original.endpoint != c.endpoint
                || original.binding != c.binding
                || original.policy_revision != c.policy_revision
            {
                return Err("stale_original_receipt");
            }
            return Ok(Some(original.clone()));
        }
    }
    testing::original(state, c)
}
pub(super) fn preparation(app: &mut App, assistant: &mut Assistant, capture: &Capture) -> Result<bool, &'static str> {
    if uploads::supports(capture.tool) {
        return uploads::preparation(app, assistant, capture);
    }
    preparation::run(app, assistant, capture)
}
pub(super) fn prepare(app: &mut App, state: &State, capture: &Capture) -> Result<Vec<Class>, &'static str> {
    if !capture.tool.definition().read_only && !matches!(capture.tool, Tool::ScriptTestStop | Tool::InventoryUploadCancel) {
        use sha2::{Digest, Sha256};
        let object = optional_uuid(&capture.arguments, "object_id")?;
        let facts = object
            .and_then(|id| {
                app.world
                    .objects
                    .index_of_uuid(&id)
                    .and_then(|i| app.world.objects.get(i))
                    .map(|o| {
                        format!(
                            "{}:{}:{}:{}:{:?}",
                            o.key.region,
                            o.key.local_id,
                            o.owner_id,
                            o.update_flags,
                            app.build.contents.objects.get(&id).map(|i| (&i.items, i.serial))
                        )
                    })
            })
            .unwrap_or_default();
        let digest: [u8; 32] = Sha256::digest(facts.as_bytes()).into();
        freeze_evidence(state, capture, app.world.inventory.generation, digest)?;
    }
    if uploads::supports(capture.tool) {
        uploads::prepare(app, state, capture)
    } else if interactions::supports(capture.tool) {
        interactions::prepare(app, state, capture)
    } else if diagnostics::supports(capture.tool) {
        diagnostics::prepare(app, &state.diagnostics, capture)
    } else if testing::supports(capture.tool) {
        testing::prepare(app, state, capture)
    } else if reads::supports(capture.tool) {
        reads::prepare(app, state, capture)
    } else if mutations::supports(capture.tool) {
        mutations::prepare(app, state, capture)
    } else if content::supports(capture.tool) {
        content::prepare(app, state, capture)
    } else if objects::supports(capture.tool) {
        objects::prepare(app, state, capture)
    } else {
        Err("unsupported_capability")
    }
}
fn freeze_evidence(state: &State, capture: &Capture, generation: u64, digest: [u8; 32]) -> Result<(), &'static str> {
    let mut evidence = state.evidence.borrow_mut();
    evidence.retain(|_, e| e.3.elapsed() < Duration::from_secs(600));
    let key = (capture.endpoint, capture.client, capture.request_id.clone());
    if let Some((binding, original_generation, old, _)) = evidence.get(&key) {
        if *binding != capture.binding || *original_generation != generation || *old != digest {
            return Err("native_scope_changed");
        }
    } else {
        if evidence.len() >= CAPACITY {
            return Err("operation_capacity");
        }
        evidence.insert(key, (capture.binding, generation, digest, Instant::now()));
    }
    Ok(())
}
/// Human-facing native scope for an approval. This stays in the viewer UI;
/// it contains inventory labels, never asset UUIDs or machine paths.
pub(super) fn approval_evidence(app: &App, state: &State, c: &Capture) -> Value {
    if uploads::supports(c.tool) {
        return uploads::approval(state, c);
    }
    if c.tool == Tool::InventoryReplaceLinks {
        return links::approval(app, c);
    }
    let mut targets = std::collections::BTreeSet::new();
    let mut complete = true;
    for key in ["id", "folder_id", "parent_id", "outfit_id", "item_id"] {
        if let Ok(Some(id)) = optional_uuid(&c.arguments, key) {
            targets.insert(id);
        }
    }
    for key in ["ids", "items_id"] {
        if let Some(values) = c.arguments.get(key).and_then(Value::as_array) {
            for value in values.iter().take(51) {
                if let Some(id) = value.as_str().and_then(|s| Uuid::parse_str(s).ok()) {
                    targets.insert(id);
                }
            }
        }
    }
    if let Some(values) = c.arguments.get("items").and_then(Value::as_array) {
        for value in values.iter().take(51) {
            if let Some(id) = value.get("id").and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok()) {
                targets.insert(id);
            }
        }
    }
    if matches!(c.tool, Tool::InventoryExecutePlan | Tool::InventoryConfirmCopy)
        && let Ok(plan) = mutations::plan(state, c)
    {
        targets.extend(plan.before.iter().map(|f| f.id));
        targets.extend(plan.after.iter().filter_map(|f| f.parent));
    }
    if c.tool == Tool::AppearanceWearOutfit
        && !c.arguments.contains_key("folder_id")
        && let Some(name) = c.arguments.get("folder_name").and_then(Value::as_str)
    {
        targets.extend(
            app.world
                .inventory
                .folders
                .values()
                .filter(|f| !f.library && f.info.type_default == 47 && f.info.name == name)
                .take(2)
                .map(|f| f.info.id),
        );
    }
    if matches!(c.tool, Tool::InventoryCopy | Tool::InventoryBatchCopy | Tool::AppearanceWearOutfit) {
        let roots = targets
            .iter()
            .copied()
            .filter(|id| Some(*id) != optional_uuid(&c.arguments, "parent_id").ok().flatten())
            .collect::<Vec<_>>();
        for root in roots {
            match tree_scope(&app.world.inventory, root, 512) {
                Ok(ids) => targets.extend(ids),
                Err(_) => complete = false,
            }
        }
    }
    if targets.len() > 1024 {
        complete = false;
    }
    let rows=targets.iter().take(1024).map(|id| {
        let name=rules::name(&app.world.inventory,*id);
        json!({"id":id.to_string(),"name":clipped(&name,256),"name_truncated":name.chars().count()>256,"kind":if app.world.inventory.folders.contains_key(id){"folder"}else{"item"}})
    }).collect::<Vec<_>>();
    let object = optional_uuid(&c.arguments, "object_id")
        .ok()
        .flatten()
        .map(|id| json!({"id":id.to_string(),"name":app.build.props.get(&id).map(|p|clipped(&p.name,256))}));
    json!({"targets":rows,"target_count":targets.len(),"complete":complete,"object":object,"destination_id":optional_uuid(&c.arguments,"parent_id").ok().flatten().map(|id|id.to_string()),"requested_name":c.arguments.get("name"),"copy_policy":c.arguments.get("policy"),"native_generation":app.world.inventory.generation})
}
pub(super) fn force_review(state: &State, c: &Capture) -> bool {
    uploads::force_review(state, c)
}
pub(super) fn retire(app: &mut App, assistant: &mut Assistant) {
    uploads::retire(app, assistant);
}
fn tree_scope(inv: &Inventory, root: Uuid, max: usize) -> Result<Vec<Uuid>, &'static str> {
    let mut stack = vec![(root, 0usize)];
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    while let Some((id, depth)) = stack.pop() {
        if depth > 32 || !seen.insert(id) || rows.len() >= max {
            return Err("native_scope_limit");
        }
        rows.push(id);
        if let Some(folder) = inv.folders.get(&id) {
            if folder.state != FetchState::Fetched {
                return Err("inventory_loading");
            }
            if folder
                .children
                .len()
                .saturating_add(folder.items.len())
                .saturating_add(stack.len())
                .saturating_add(rows.len())
                > max
            {
                return Err("native_scope_limit");
            }
            stack.extend(folder.children.iter().chain(&folder.items).copied().map(|id| (id, depth + 1)));
        } else if !inv.items.contains_key(&id) {
            return Err("not_found");
        }
    }
    Ok(rows)
}
pub(super) fn execute(app: &mut App, assistant: &mut Assistant, capture: &Capture) -> Value {
    assistant.inventory.bind(capture.binding, capture.endpoint);
    let result = if uploads::supports(capture.tool) {
        uploads::execute(app, &mut assistant.inventory, capture)
    } else if interactions::supports(capture.tool) {
        interactions::execute(app, &mut assistant.inventory, capture)
    } else if diagnostics::supports(capture.tool) {
        diagnostics::execute(app, &mut assistant.inventory.diagnostics, capture)
    } else if testing::supports(capture.tool) {
        testing::execute(app, assistant, capture)
    } else if reads::supports(capture.tool) {
        reads::execute(app, &mut assistant.inventory, capture)
    } else if mutations::supports(capture.tool) {
        mutations::execute(app, &mut assistant.inventory, capture)
    } else if content::supports(capture.tool) {
        content::execute(app, &mut assistant.inventory, capture)
    } else if objects::supports(capture.tool) {
        objects::execute(app, &mut assistant.inventory, capture)
    } else {
        Err("unsupported_capability")
    };
    let value = result.unwrap_or_else(failure);
    if let Err(code) = receipts::record(app, &mut assistant.inventory, capture, &value) {
        return failure(code);
    }
    value
}
pub(super) fn validate_receipt(app: &App, state: &State, capture: &Capture) -> bool {
    receipts::validate(app, state, capture)
}
fn commit_continuation<R>(app: &mut App, capture: &Capture, commit: impl FnOnce(&mut App) -> R) -> Result<R, &'static str> {
    if !super::continuation_current(app, capture) {
        return Err("stale_source_or_authority");
    }
    let runtime = app.character_mcp.as_ref().ok_or("stale_source")?.runtime.clone();
    runtime.continue_action(capture, || commit(app))
}
fn args(c: &Capture, keys: &[&str]) -> Result<(), &'static str> {
    if c.arguments
        .keys()
        .any(|k| !keys.contains(&k.as_str()) && !matches!(k.as_str(), "viewer_session" | "viewer_pid"))
    {
        return Err("invalid_args");
    }
    Ok(())
}
fn optional_uuid(a: &serde_json::Map<String, Value>, key: &str) -> Result<Option<Uuid>, &'static str> {
    match a.get(key) {
        None => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(_) => uuid(a, key).map(Some),
    }
}
fn boolean(a: &serde_json::Map<String, Value>, key: &str, default: bool) -> Result<bool, &'static str> {
    a.get(key).map_or(Ok(default), |v| v.as_bool().ok_or("invalid_args"))
}
fn number(a: &serde_json::Map<String, Value>, key: &str, default: usize, max: usize) -> Result<usize, &'static str> {
    a.get(key).map_or(Ok(default), |v| {
        v.as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n <= max)
            .ok_or("invalid_args")
    })
}
fn ids(a: &serde_json::Map<String, Value>, key: &str) -> Result<Vec<Uuid>, &'static str> {
    let values = a.get(key).ok_or("invalid_args")?;
    let values: Vec<&Value> = match values {
        Value::Array(v) => v.iter().collect(),
        Value::String(_) => vec![values],
        _ => return Err("invalid_args"),
    };
    if values.is_empty() || values.len() > ROW_LIMIT {
        return Err("invalid_args");
    }
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|v| {
            v.as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .filter(|id| !id.is_nil() && seen.insert(*id))
                .ok_or("invalid_args")
        })
        .collect()
}
fn ordinary(app: &App, id: Uuid) -> Result<(), &'static str> {
    if crate::world::marketplace::root(&app.world.inventory).is_some_and(|root| rules::under(&app.world.inventory, id, root)) {
        Err("marketplace_unsupported")
    } else {
        Ok(())
    }
}
fn native_busy(app: &App) -> Result<(), &'static str> {
    if app.inventory_ui.pending.is_some() || app.appearance_ui.pending.is_some() {
        Err("native_busy")
    } else {
        Ok(())
    }
}
fn submitted(c: &Capture) -> Value {
    json!({"status":"submitted","operation_id":c.request_id,"confirmed":false,"retry_safe":false})
}
fn clipped(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}
fn pending_for(state: &mut State, request: Uuid, c: &Capture, kind: PendingKind) {
    state.pending.insert(
        request,
        Pending {
            capture: c.clone(),
            started: Instant::now(),
            kind,
        },
    );
}

/// Called before ordinary App event routing. Only owned native request IDs are
/// consumed; all existing user-driven paths keep their normal handlers.
pub(super) fn native_event(app: &mut App, event: &NetEvent) -> bool {
    uploads::native_event(app, event)
        || interactions::native_event(app, event)
        || preparation::native_event(app, event)
        || library::native_event(app, event)
        || links::native_event(app, event)
        || landmarks::native_event(app, event)
        || content::native_event(app, event)
        || objects::native_event(app, event)
        || mutations::native_event(app, event)
        || matches!(
            event,
            NetEvent::AssistantInventoryCreated { .. }
                | NetEvent::AssistantInventoryAssetReady { .. }
                | NetEvent::AssistantInventoryFolder { .. }
                | NetEvent::AssistantInventoryItems { .. }
                | NetEvent::AssistantInventoryLinkCreated { .. }
                | NetEvent::AssistantLandmark { .. }
        )
}

pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    uploads::tick(app, assistant);
    diagnostics::tick(app, assistant);
    interactions::tick(app, assistant);
    testing::tick(app, assistant);
    objects::tick(app, assistant);
    library::tick(app, assistant);
    let runtime = assistant.runtime.clone();
    preparation::prune(&mut assistant.inventory, &runtime);
    links::prune(&mut assistant.inventory, &runtime);
    receipts::prune(&mut assistant.inventory, &runtime);
    let live = assistant
        .inventory
        .outcome_captures
        .iter()
        .filter(|(_, c)| runtime.has_operation(&c.request_id))
        .map(|(key, _)| key.clone())
        .collect::<HashSet<_>>();
    assistant.inventory.outcomes.retain(|key, _| live.contains(key));
    assistant.inventory.outcome_captures.retain(|key, _| live.contains(key));
    if let Some(request) = app.inventory_ui.pending
        && assistant
            .inventory
            .pending
            .get(&request)
            .is_some_and(|p| !runtime.has_operation(&p.capture.request_id) || p.started.elapsed() >= Duration::from_secs(600))
    {
        app.inventory_ui.finish_mutation(false);
        app.inventory_ui.pending_refresh.clear();
    }
}
pub(super) fn native_command(app: &mut App, command: &NetCommand) {
    objects::native_command(app, command);
}

fn outcome(state: &mut State, c: &Capture, key: String, value: Value) {
    if state.outcomes.len() < CAPACITY || state.outcomes.contains_key(&key) {
        state.outcomes.insert(key.clone(), value);
        state.outcome_captures.insert(key, c.clone());
    }
}

fn existing_outcome(state: &State, c: &Capture, key: &str) -> Result<Option<Value>, &'static str> {
    let Some(value) = state.outcomes.get(key) else { return Ok(None) };
    let old = state.outcome_captures.get(key).ok_or("unknown_operation")?;
    if old.endpoint != c.endpoint
        || old.client != c.client
        || old.binding != c.binding
        || old.tool != c.tool
        || old.arguments != c.arguments
    {
        return Err("request_id_conflict");
    }
    if old.policy_revision != c.policy_revision {
        return Err("stale_operation");
    }
    Ok(Some(value.clone()))
}

pub(super) fn snapshot_current(app: &App, id: Uuid) -> bool {
    uploads::snapshot_current(app, id)
}
pub(super) fn snapshot_pixels(app: &mut App, id: Uuid, size: (u32, u32), rgba: Vec<u8>) {
    uploads::snapshot_pixels(app, id, size, rgba);
}

#[cfg(test)]
pub(super) fn synthetic_capture(tool: Tool) -> Capture {
    use aurora_session::{AccountKey, GridKey, InstallationId};
    Capture {
        client: 1,
        endpoint: HostId::from_bytes([7; 16]).unwrap(),
        request_id: "synthetic-operation".into(),
        binding: Binding {
            worker: WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).unwrap(),
                host: HostId::from_bytes([2; 16]).unwrap(),
                worker: WorkerId::from_bytes([3; 16]).unwrap(),
            },
            session: Some(SessionIdentity {
                session: SessionId::from_bytes([4; 16]).unwrap(),
                account: AccountKey::from_bytes([5; 32]).unwrap(),
                grid: GridKey::from_bytes([6; 32]).unwrap(),
                generation: 1,
            }),
        },
        tool,
        arguments: serde_json::Map::new(),
        policy_revision: 10,
        required_classes: vec![Class::Read],
    }
}
#[cfg(test)]
mod boundary_tests {
    use super::*;
    #[test]
    fn pending_folder_scope_rejects_a_new_descendant_even_if_folder_record_matches() {
        let state = State::default();
        let c = synthetic_capture(Tool::InventoryCopy);
        let root = Uuid::from_u128(101);
        let mut inv = Inventory::default();
        inv.folders.insert(
            root,
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: root,
                    name: "Approved folder".into(),
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        let frozen = Frozen::capture(&inv, root).unwrap();
        freeze_evidence(&state, &c, inv.generation, [0; 32]).unwrap();
        inv.add_items(vec![InvItem {
            id: Uuid::from_u128(102),
            parent: root,
            name: "Arrived while Ask pending".into(),
            ..Default::default()
        }]);
        assert!(frozen.current(&inv), "folder identity alone does not freeze descendants");
        assert_eq!(freeze_evidence(&state, &c, inv.generation, [0; 32]), Err("native_scope_changed"));
    }
    #[test]
    fn operation_dedup_is_owned_by_authenticated_connection_and_incarnation() {
        let mut state = State::default();
        let c = synthetic_capture(Tool::InventoryReadScript);
        outcome(&mut state, &c, "known-id".into(), json!({"text":"synthetic protected source"}));
        assert!(existing_outcome(&state, &c, "known-id").unwrap().is_some());
        let mut other = c.clone();
        other.client += 1;
        assert_eq!(existing_outcome(&state, &other, "known-id").unwrap_err(), "request_id_conflict");
        other = c.clone();
        other.endpoint = HostId::from_bytes([8; 16]).unwrap();
        assert_eq!(existing_outcome(&state, &other, "known-id").unwrap_err(), "request_id_conflict");
        other = c.clone();
        other.policy_revision += 1;
        assert_eq!(existing_outcome(&state, &other, "known-id").unwrap_err(), "stale_operation");
    }
}
