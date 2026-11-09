//! Frozen local bytes, native fee ceilings and one-shot upload continuations.
use super::*;
use aurora_net::inventory::{
    InvFolder,
    upload::{Kind, Permissions, Prices, Request as UploadRequest},
};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
static READER_BUSY: AtomicBool = AtomicBool::new(false);
struct ReaderLease;
impl ReaderLease {
    fn acquire() -> Option<Self> {
        READER_BUSY
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self)
    }
}
impl Drop for ReaderLease {
    fn drop(&mut self) {
        READER_BUSY.store(false, Ordering::Release);
    }
}
struct ReaderResult<T> {
    value: T,
    _lease: ReaderLease,
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
mod files;
mod mesh;
mod snapshot;
pub(super) fn supports(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::InventoryUploadPreview
            | Tool::InventoryUploadExecute
            | Tool::InventoryUploadStatus
            | Tool::InventoryUploadCancel
            | Tool::InventoryUploadMeshReview
            | Tool::InventorySnapshotUpload
    )
}
type Key = (HostId, u64, String);
#[derive(Default)]
pub(super) struct State {
    mesh: mesh::State,
    snapshot: snapshot::State,
    plans: BTreeMap<Uuid, Plan>,
    ready: BTreeMap<Key, Result<Uuid, &'static str>>,
    preparing: Option<Capture>,
    io: Option<(crossbeam_channel::Sender<Prepared>, crossbeam_channel::Receiver<Prepared>)>,
    pending: BTreeMap<Uuid, PendingUpload>,
    reports: BTreeMap<Key, ReportProof>,
}
impl State {
    pub(super) fn reset(&mut self) {
        let mesh = std::mem::take(&mut self.mesh);
        let snapshot = std::mem::take(&mut self.snapshot);
        *self = Self::default();
        self.mesh = mesh;
        self.snapshot = snapshot;
    }
}
struct ReportProof {
    capture: Capture,
    login: aurora_net::LoginSource,
}
struct Prepared {
    capture: Capture,
    started: Instant,
    login: aurora_net::LoginSource,
    files: ReaderResult<Result<Vec<File>, &'static str>>,
}
struct File {
    name: String,
    folders: Vec<String>,
    kind: Kind,
    dimensions: (u32, u32),
    permissions: Permissions,
    data: Arc<[u8]>,
    input_hash: String,
    encoded_hash: String,
    encoded_bytes: usize,
    cost: i32,
    state: &'static str,
    item: Option<Uuid>,
    charged: Option<i32>,
    cost_known: bool,
    error: Option<&'static str>,
}
struct Plan {
    source: Capture,
    login: aurora_net::LoginSource,
    target: Frozen,
    files: Vec<File>,
    folders: BTreeMap<Vec<String>, Uuid>,
    created: BTreeMap<Uuid, Frozen>,
    expires: Instant,
    execution: Option<Capture>,
    cancelled: bool,
    done: bool,
    in_flight: bool,
    index: usize,
    report_deadline: Option<Instant>,
}
#[derive(Clone)]
enum PendingKind {
    Folder(Vec<String>, Uuid, String),
    File(usize, u8),
}
#[derive(Clone)]
struct PendingUpload {
    plan: Uuid,
    capture: Capture,
    kind: PendingKind,
    started: Instant,
}
fn key(c: &Capture) -> Key {
    (c.endpoint, c.client, c.request_id.clone())
}
fn owner(a: &Capture, b: &Capture) -> bool {
    a.endpoint == b.endpoint && a.client == b.client && a.binding == b.binding
}
fn prices(app: &App) -> Prices {
    app.world
        .login
        .as_ref()
        .map(|l| Prices::from_login(&l.raw))
        .unwrap_or_default()
        .with_fallback(app.world.upload_cost)
}
fn destination(app: &App, id: Uuid) -> Result<(), &'static str> {
    if app.world.inventory.owner != app.world.agent_id || !rules::destination(&app.world.inventory, id) {
        return Err("native_upload_destination_unavailable");
    }
    ordinary(app, id)
}
fn inputs(c: &Capture) -> Result<(Vec<String>, Option<String>, u32), &'static str> {
    args(c, &["files", "directory", "folder_id", "max_texture_dimension"])?;
    let files = match c.arguments.get("files") {
        None => Vec::new(),
        Some(Value::Array(rows)) if !rows.is_empty() && rows.len() <= 32 => rows
            .iter()
            .map(|v| {
                v.as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 4096)
                    .map(str::to_owned)
                    .ok_or("invalid_args")
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err("invalid_args"),
    };
    let directory = c
        .arguments
        .get("directory")
        .map(|_| string(&c.arguments, "directory", 4096).map(str::to_owned))
        .transpose()?;
    if files.is_empty() == directory.is_none() {
        return Err("invalid_args");
    }
    let dimension = number(&c.arguments, "max_texture_dimension", 1024, 2048)?;
    if !matches!(dimension, 1024 | 2048) {
        return Err("invalid_args");
    }
    Ok((files, directory, dimension as u32))
}
fn plan<'a>(state: &'a State, c: &Capture, spending: bool) -> Result<(Uuid, &'a Plan), &'static str> {
    let id = uuid(&c.arguments, "plan_id")?;
    let p = state.plans.get(&id).filter(|p| owner(&p.source, c)).ok_or("unknown_upload_plan")?;
    if spending && (p.source.policy_revision != c.policy_revision || p.expires <= Instant::now()) {
        return Err("stale_upload_plan");
    }
    Ok((id, p))
}
fn total(p: &Plan) -> Result<i32, &'static str> {
    p.files
        .iter()
        .try_fold(0i32, |sum, f| sum.checked_add(f.cost).ok_or("upload_price_limit"))
}
pub(super) fn preparation(app: &mut App, assistant: &mut Assistant, c: &Capture) -> Result<bool, &'static str> {
    assistant.inventory.bind(c.binding, c.endpoint);
    if c.tool == Tool::InventoryUploadStatus {
        let (_, p) = plan(&assistant.inventory.uploads, c, false)?;
        assistant
            .runtime
            .bind_original_receipt(c, p.execution.as_ref().unwrap_or(&p.source))?;
        return Ok(true);
    }
    if c.tool != Tool::InventoryUploadPreview {
        return Ok(true);
    }
    let (paths, directory, dimension) = inputs(c)?;
    let folder = uuid(&c.arguments, "folder_id")?;
    destination(app, folder)?;
    if let Some(result) = assistant.inventory.uploads.ready.get(&key(c)) {
        return result.map(|_| true);
    }
    if let Some(preparing) = &assistant.inventory.uploads.preparing {
        return if preparing == c {
            Ok(false)
        } else {
            Err("native_upload_preparation_busy")
        };
    }
    if assistant.inventory.uploads.plans.len() >= 4 {
        let retired = assistant
            .inventory
            .uploads
            .plans
            .iter()
            .filter(|(_, p)| p.done)
            .min_by_key(|(_, p)| p.expires)
            .map(|(id, _)| *id);
        if let Some(id) = retired {
            assistant.inventory.uploads.plans.remove(&id);
        } else {
            return Err("upload_plan_capacity");
        }
    }
    let mut options = app.settings.imports.texture();
    options.max_width = dimension;
    options.max_height = dimension;
    let prefs = app.settings.imports.clone();
    let sound_limit = if app.settings.grid == crate::settings::GridChoice::Custom {
        60
    } else {
        30
    };
    let (tx, _) = assistant.inventory.uploads.io.get_or_insert_with(|| crossbeam_channel::bounded(1));
    let tx = tx.clone();
    let capture = c.clone();
    let started = Instant::now();
    let login = aurora_net::LoginSource::from_login(app.world.login.as_ref().ok_or("native_login_unavailable")?);
    let wake = app.egui_ctx.clone();
    let runtime = assistant.runtime.clone();
    let lease = ReaderLease::acquire().ok_or("native_upload_preparation_busy")?;
    runtime.continue_preparation(c, Class::Read, || {
        assistant.inventory.uploads.preparing = Some(c.clone());
        app.scene.jobs.execute(move || {
            let result = (|| {
                let source = files::collect(&paths, directory.as_deref()).map_err(|_| "native_upload_inputs_rejected")?;
                let mut encoded = Vec::new();
                let mut bytes = 0usize;
                for input in source {
                    let input_hash = hash(&input.bytes);
                    let output = crate::imports::assistant::encode(&input.bytes, &input.extension, options, sound_limit)?;
                    bytes = bytes.checked_add(output.bytes.len()).ok_or("upload_encoded_batch_limit")?;
                    if bytes > files::BATCH_LIMIT {
                        return Err("upload_encoded_batch_limit");
                    }
                    encoded.push(File {
                        name: input.name,
                        folders: input.folders,
                        kind: output.kind,
                        dimensions: output.dimensions,
                        permissions: prefs.permissions_for(output.kind),
                        encoded_hash: hash(&output.bytes),
                        input_hash,
                        encoded_bytes: output.bytes.len(),
                        data: output.bytes,
                        cost: 0,
                        state: "unsent",
                        item: None,
                        charged: None,
                        cost_known: false,
                        error: None,
                    });
                }
                Ok(encoded)
            })();
            let _ = tx.try_send(Prepared {
                capture,
                started,
                login,
                files: ReaderResult {
                    value: result,
                    _lease: lease,
                },
            });
            wake.request_repaint();
        });
    })?;
    Ok(false)
}
pub(super) fn prepare(app: &App, state: &super::State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    if c.tool == Tool::InventorySnapshotUpload {
        return snapshot::prepare(app, state, c);
    }
    if c.tool == Tool::InventoryUploadMeshReview {
        return mesh::prepare(app, state, c);
    }
    match c.tool {
        Tool::InventoryUploadPreview => {
            inputs(c)?;
            destination(app, uuid(&c.arguments, "folder_id")?)?;
            state
                .uploads
                .ready
                .get(&key(c))
                .ok_or("native_upload_preparation_wait")?
                .as_ref()
                .map_err(|e| *e)?;
            Ok(Vec::new())
        }
        Tool::InventoryUploadExecute => {
            args(c, &["plan_id"])?;
            let (_, p) = plan(&state.uploads, c, true)?;
            if p.execution.is_some() {
                return Err("upload_plan_already_executed");
            }
            destination(app, p.target.id)?;
            if !target_current(app, &p.target) {
                return Err("native_upload_destination_changed");
            }
            if !app.world.main_has_cap("NewFileAgentInventory") || !app.world.main_has_cap("FetchInventory2") {
                return Err("native_upload_capability_unavailable");
            }
            let current = prices(app);
            for f in &p.files {
                if current.cost(f.kind, f.dimensions).is_none_or(|v| v > f.cost) {
                    return Err("native_upload_price_increased");
                }
            }
            if total(p)? > 0 && app.world.balance.is_none_or(|balance| balance < total(p).unwrap_or(i32::MAX)) {
                return Err("native_insufficient_balance");
            }
            Ok(vec![Class::Read, Class::Create])
        }
        Tool::InventoryUploadStatus | Tool::InventoryUploadCancel => {
            args(c, &["plan_id"])?;
            let (_, p) = plan(&state.uploads, c, false)?;
            if c.tool == Tool::InventoryUploadStatus && !report_current(app, p, c) {
                return Err("native_result_access_revoked");
            }
            Ok(Vec::new())
        }
        _ => Err("unsupported_capability"),
    }
}
fn report_current(app: &App, p: &Plan, c: &Capture) -> bool {
    let original = p.execution.as_ref().unwrap_or(&p.source);
    original.policy_revision == c.policy_revision
        && p.report_deadline.is_some_and(|deadline| Instant::now() < deadline)
        && original
            .required_classes
            .iter()
            .all(|class| app.settings.local_assistant.permissions.level(*class) != aurora_mcp::assistant::permissions::Level::Never)
}
fn target_current(app: &App, old: &Frozen) -> bool {
    let Some((parent, name, kind, _)) = &old.folder else {
        return false;
    };
    app.world
        .inventory
        .folders
        .get(&old.id)
        .is_some_and(|f| f.info.parent == *parent && f.info.name == *name && f.info.type_default == *kind)
}
pub(super) fn force_review(state: &super::State, c: &Capture) -> bool {
    if c.tool == Tool::InventorySnapshotUpload {
        return snapshot::force_review(state, c);
    }
    c.tool == Tool::InventoryUploadExecute && plan(&state.uploads, c, true).is_ok_and(|(_, p)| total(p).is_ok_and(|n| n > 0))
}
fn report(id: Uuid, p: &Plan) -> Value {
    json!({"plan_id":id.to_string(),"status":if p.done {"complete"}else if p.cancelled {"cancelled"}else if p.execution.is_some(){"submitted"}else{"preview"},"operation_id":p.execution.as_ref().map(|c|&c.request_id),"approved_ceiling":total(p).ok(),"cancelled":p.cancelled,"retry_safe":false,"files":p.files.iter().map(|f|json!({"name":f.name,"inventory_folders":f.folders,"kind":f.kind.names().0,"encoded_bytes":f.encoded_bytes,"dimensions":f.dimensions,"source_sha256":f.input_hash,"encoded_sha256":f.encoded_hash,"quoted_ceiling":f.cost,"state":f.state,"inventory_item_id":f.item.map(|id|id.to_string()),"charged_cost":f.charged.filter(|_|f.cost_known),"charge_known":f.cost_known,"error_code":f.error})).collect::<Vec<_>>(),"folder_policy":"create_fresh_relative_hierarchy"})
}
pub(super) fn approval(state: &super::State, c: &Capture) -> Value {
    if c.tool == Tool::InventorySnapshotUpload {
        return snapshot::approval(state, c);
    }
    if c.tool == Tool::InventoryUploadMeshReview {
        return json!({"native_mesh_review":true,"upload_submitted":false,"manual_upload_required":true,"destination_folder":c.arguments.get("folder_id")});
    }
    match plan(&state.uploads, c, true) {
        Ok((id, p)) => report(id, p),
        Err(code) => failure(code),
    }
}
pub(super) fn execute(app: &mut App, state: &mut super::State, c: &Capture) -> Result<Value, &'static str> {
    if c.tool == Tool::InventorySnapshotUpload {
        return snapshot::execute(app, state, c);
    }
    if c.tool == Tool::InventoryUploadMeshReview {
        return mesh::execute(app, state, c);
    }
    prepare(app, state, c)?;
    if c.tool == Tool::InventoryUploadPreview {
        let id = *state
            .uploads
            .ready
            .get(&key(c))
            .ok_or("native_upload_preparation_wait")?
            .as_ref()
            .map_err(|e| *e)?;
        let p = state.uploads.plans.get(&id).ok_or("unknown_upload_plan")?;
        let value = report(id, p);
        let login = p.login.clone();
        record_report(&mut state.uploads, c, login)?;
        return Ok(value);
    }
    let (id, _) = plan(&state.uploads, c, c.tool == Tool::InventoryUploadExecute)?;
    let p = state.uploads.plans.get_mut(&id).ok_or("unknown_upload_plan")?;
    let result = match c.tool {
        Tool::InventoryUploadExecute => {
            p.execution = Some(c.clone());
            Ok(json!({"status":"submitted","plan_id":id.to_string(),"operation_id":c.request_id,"retry_safe":false}))
        }
        Tool::InventoryUploadCancel => {
            p.cancelled = true;
            let unsent = p.files.iter().filter(|f| f.state == "unsent").count();
            for f in p.files.iter_mut().filter(|f| f.state == "unsent") {
                f.state = "cancelled_unsent";
            }
            Ok(
                json!({"status":"cancelled_unsent","plan_id":id.to_string(),"unsent_cancelled":unsent,"in_flight_outcome_unknown":p.in_flight,"retry_safe":false}),
            )
        }
        Tool::InventoryUploadStatus => Ok(report(id, p)),
        _ => Err("unsupported_capability"),
    };
    if c.tool == Tool::InventoryUploadStatus {
        let login = p.login.clone();
        record_report(&mut state.uploads, c, login)?;
    }
    result
}
fn record_report(state: &mut State, c: &Capture, login: aurora_net::LoginSource) -> Result<(), &'static str> {
    if state.reports.len() >= CAPACITY && !state.reports.contains_key(&key(c)) {
        return Err("native_receipt_proof_capacity");
    }
    state.reports.insert(key(c), ReportProof { capture: c.clone(), login });
    Ok(())
}
pub(super) fn validate_receipt(app: &App, state: &super::State, c: &Capture) -> bool {
    state.uploads.reports.get(&key(c)).is_some_and(|proof| {
        proof.capture == *c
            && app
                .world
                .login
                .as_ref()
                .is_some_and(|login| proof.login.matches_login(login, app.world.agent_id))
            && c.required_classes
                .iter()
                .all(|class| app.settings.local_assistant.permissions.level(*class) != aurora_mcp::assistant::permissions::Level::Never)
    })
}
fn acknowledgment(id: Uuid, p: &Plan) -> Value {
    json!({"plan_id":id.to_string(),"status":if p.cancelled{"stopped"}else{"complete"},"file_count":p.files.len(),"folder_count_created":p.folders.len().saturating_sub(1),"retry_safe":false,"files":p.files.iter().enumerate().map(|(index,f)|json!({"index":index,"state":f.state,"inventory_item_id":f.item.map(|id|id.to_string()),"charged_cost":f.charged.filter(|_|f.cost_known),"charge_known":f.cost_known,"approved_ceiling":f.cost,"error_code":f.error})).collect::<Vec<_>>()})
}

fn source_current(app: &App, assistant: &Assistant, p: &Plan, c: &Capture) -> bool {
    super::super::binding_matches(app, assistant, c)
        && app
            .world
            .login
            .as_ref()
            .is_some_and(|login| p.login.matches_login(login, app.world.agent_id))
        && assistant.runtime.configuration().as_ref() == Some(&app.settings.local_assistant)
}
fn native_source_current(app: &App, p: &Plan) -> bool {
    app.world
        .login
        .as_ref()
        .is_some_and(|l| p.login.matches_login(l, app.world.agent_id))
}
fn stop_unsent(p: &mut Plan, code: &'static str) {
    p.cancelled = true;
    for f in p.files.iter_mut().filter(|f| f.state == "unsent") {
        f.state = "cancelled_unsent";
        f.error = Some(code);
    }
}
fn complete(app: &App, assistant: &mut Assistant, id: Uuid, p: &mut Plan) {
    if p.done || p.in_flight {
        return;
    }
    if !p.cancelled && p.index < p.files.len() {
        return;
    }
    p.done = true;
    for f in &mut p.files {
        f.data = Arc::from([]);
    }
    if let Some(c) = &p.execution {
        let value = acknowledgment(id, p);
        let _ = receipts::record(app, &mut assistant.inventory, c, &value);
        outcome(&mut assistant.inventory, c, c.request_id.clone(), value.clone());
        let _ = assistant.runtime.complete(c, value);
    }
}
fn next_folder(p: &Plan) -> Option<Vec<String>> {
    for f in &p.files {
        for depth in 1..=f.folders.len() {
            let path = f.folders[..depth].to_vec();
            if !p.folders.contains_key(&path) {
                return Some(path);
            }
        }
    }
    None
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    mesh::tick(app, assistant);
    snapshot::tick(app, assistant);
    let runtime = assistant.runtime.clone();
    let expired = assistant
        .inventory
        .uploads
        .pending
        .iter()
        .filter(|(_, p)| p.started.elapsed() > Duration::from_secs(95))
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    for request in expired {
        if let Some(pending) = assistant.inventory.uploads.pending.remove(&request)
            && let Some(p) = assistant.inventory.uploads.plans.get_mut(&pending.plan)
        {
            p.in_flight = false;
            if let PendingKind::File(index, phase) = pending.kind {
                p.files[index].state = if phase == 0 { "offer_unconfirmed" } else { "uncertain" };
                p.files[index].error = Some("native_upload_acknowledgment_timeout");
                if phase == 0 {
                    app.send(NetCommand::FinishAssistantUpload {
                        source: p.login.clone(),
                        request,
                        approved: false,
                    });
                }
            }
            stop_unsent(p, "upload_stopped_after_acknowledgment_timeout");
        }
    }
    let replies = assistant
        .inventory
        .uploads
        .io
        .as_ref()
        .map(|(_, rx)| rx.try_iter().take(1).collect::<Vec<_>>())
        .unwrap_or_default();
    for reply in replies {
        if assistant.inventory.uploads.preparing.as_ref() == Some(&reply.capture) {
            assistant.inventory.uploads.preparing = None;
        }
        if !super::super::binding_matches(app, assistant, &reply.capture)
            || app
                .world
                .login
                .as_ref()
                .is_none_or(|l| !reply.login.matches_login(l, app.world.agent_id))
        {
            continue;
        }
        let result = (|| {
            if reply.started.elapsed() > Duration::from_secs(45) {
                return Err("native_upload_preparation_timeout");
            }
            let mut encoded = reply.files.value?;
            let price = prices(app);
            for f in &mut encoded {
                f.cost = price.cost(f.kind, f.dimensions).ok_or("native_upload_price_unavailable")?;
            }
            let retained = assistant
                .inventory
                .uploads
                .plans
                .values()
                .flat_map(|p| &p.files)
                .map(|f| f.data.len())
                .sum::<usize>();
            let incoming = encoded.iter().map(|f| f.data.len()).sum::<usize>();
            if assistant.inventory.uploads.plans.len() >= 4 || retained.checked_add(incoming).is_none_or(|bytes| bytes > 256 * 1024 * 1024)
            {
                return Err("upload_plan_capacity");
            }
            let folder = uuid(&reply.capture.arguments, "folder_id")?;
            destination(app, folder)?;
            let target = Frozen::capture(&app.world.inventory, folder)?;
            let id = Uuid::new_v4();
            let p = Plan {
                source: reply.capture.clone(),
                login: reply.login,
                target,
                files: encoded,
                folders: BTreeMap::from([(Vec::new(), folder)]),
                created: BTreeMap::new(),
                expires: Instant::now() + Duration::from_secs(300),
                execution: None,
                cancelled: false,
                done: false,
                in_flight: false,
                index: 0,
                report_deadline: None,
            };
            total(&p)?;
            if serde_json::to_vec(&report(id, &p)).map_or(true, |bytes| bytes.len() > 16 * 1024) {
                return Err("upload_metadata_limit");
            }
            runtime.continue_preparation(&reply.capture, Class::Read, || {
                assistant.inventory.uploads.plans.insert(id, p);
            })?;
            Ok(id)
        })();
        assistant.inventory.uploads.ready.insert(key(&reply.capture), result);
    }
    assistant.inventory.uploads.ready.retain(|key, _| runtime.has_operation(&key.2));
    assistant
        .inventory
        .uploads
        .reports
        .retain(|_, proof| runtime.has_operation(&proof.capture.request_id));
    let ids = assistant.inventory.uploads.plans.keys().copied().collect::<Vec<_>>();
    for id in ids {
        let Some(mut p) = assistant.inventory.uploads.plans.remove(&id) else {
            continue;
        };
        let original = p.execution.as_ref().unwrap_or(&p.source).clone();
        p.report_deadline = runtime.retained_receipt_authority(&original);
        if p.execution.is_none() {
            if p.expires > Instant::now() {
                assistant.inventory.uploads.plans.insert(id, p);
            }
            continue;
        }
        let c = original;
        if p.done && !runtime.has_operation(&c.request_id) {
            continue;
        }
        if !source_current(app, assistant, &p, &c) || !runtime.capture_current(&c) {
            stop_unsent(&mut p, "upload_authority_retired");
        }
        if !p.done && !p.in_flight && !p.cancelled {
            let result = (|| {
                destination(app, p.target.id)?;
                if !target_current(app, &p.target) || !p.created.values().all(|f| target_current(app, f)) {
                    return Err("native_upload_destination_changed");
                }
                if assistant.inventory.uploads.pending.len() >= CAPACITY {
                    return Err("upload_pending_capacity");
                }
                if let Some(path) = next_folder(&p) {
                    let parent_path = path[..path.len() - 1].to_vec();
                    let parent = *p.folders.get(&parent_path).ok_or("native_upload_folder_unavailable")?;
                    destination(app, parent)?;
                    let name = path.last().ok_or("native_upload_folder_unavailable")?.clone();
                    let request = Uuid::new_v4();
                    let folder = InvFolder {
                        id: Uuid::new_v4(),
                        parent,
                        name: name.clone(),
                        type_default: -1,
                        version: 1,
                        ..Default::default()
                    };
                    runtime.continue_action(&c, || {
                        app.send(NetCommand::CreateAssistantUploadFolder {
                            source: p.login.clone(),
                            request,
                            folder,
                        });
                    })?;
                    assistant.inventory.uploads.pending.insert(
                        request,
                        PendingUpload {
                            plan: id,
                            capture: c.clone(),
                            kind: PendingKind::Folder(path, parent, name),
                            started: Instant::now(),
                        },
                    );
                    p.in_flight = true;
                } else if p.index < p.files.len() {
                    let f = &mut p.files[p.index];
                    let folder = *p.folders.get(&f.folders).ok_or("native_upload_folder_unavailable")?;
                    destination(app, folder)?;
                    if prices(app).cost(f.kind, f.dimensions).is_none_or(|cost| cost > f.cost) {
                        return Err("native_upload_price_increased");
                    }
                    let request = Uuid::new_v4();
                    let upload = UploadRequest {
                        kind: f.kind,
                        folder,
                        name: f.name.clone(),
                        description: String::new(),
                        permissions: f.permissions,
                        expected_cost: f.cost,
                        data: f.data.to_vec(),
                    };
                    runtime.continue_action(&c, || {
                        app.send(NetCommand::PrepareAssistantUpload {
                            source: p.login.clone(),
                            request,
                            upload,
                        });
                    })?;
                    f.state = "offer_pending";
                    assistant.inventory.uploads.pending.insert(
                        request,
                        PendingUpload {
                            plan: id,
                            capture: c.clone(),
                            kind: PendingKind::File(p.index, 0),
                            started: Instant::now(),
                        },
                    );
                    p.in_flight = true;
                }
                Ok(())
            })();
            if let Err(code) = result {
                stop_unsent(&mut p, code);
            }
        }
        complete(app, assistant, id, &mut p);
        assistant.inventory.uploads.plans.insert(id, p);
    }
}
pub(super) fn retire(app: &mut App, assistant: &mut Assistant) {
    mesh::retire(app, &mut assistant.inventory.uploads.mesh);
    snapshot::retire(app, &mut assistant.inventory.uploads.snapshot);
}

pub(super) fn original(state: &super::State, c: &Capture) -> Result<Option<Capture>, &'static str> {
    if c.tool != Tool::InventoryUploadStatus {
        return Ok(None);
    }
    let (_, p) = plan(&state.uploads, c, false)?;
    Ok(Some(p.execution.as_ref().unwrap_or(&p.source).clone()))
}
pub(super) fn native_event(app: &mut App, event: &NetEvent) -> bool {
    let request = match event {
        NetEvent::AssistantUploadReady { request, .. }
        | NetEvent::AssistantUploadCommitted { request, .. }
        | NetEvent::AssistantUploadVerified { request, .. }
        | NetEvent::AssistantUploadFolderCreated { request, .. }
        | NetEvent::AssistantThumbnailPatched { request, .. }
        | NetEvent::AssistantThumbnailVerified { request, .. } => *request,
        _ => return false,
    };
    if let NetEvent::AssistantUploadVerified {
        source,
        result: Ok(receipt),
        ..
    } = event
        && app
            .world
            .login
            .as_ref()
            .is_some_and(|l| source.matches_login(l, app.world.agent_id))
        && receipt.item.owner == app.world.agent_id
    {
        app.world.inventory.add_items(vec![receipt.item.clone()]);
    }
    if snapshot::native_event(app, event) {
        return true;
    }
    let Some(mut assistant) = app.character_mcp.take() else {
        return true;
    };
    let Some(mut pending) = assistant.inventory.uploads.pending.remove(&request) else {
        app.character_mcp = Some(assistant);
        return true;
    };
    let Some(mut p) = assistant.inventory.uploads.plans.remove(&pending.plan) else {
        app.character_mcp = Some(assistant);
        return true;
    };
    let runtime = assistant.runtime.clone();
    let source = source_current(app, &assistant, &p, &pending.capture);
    let native_source = native_source_current(app, &p);
    let current = source && runtime.capture_current(&pending.capture);
    let mut keep = false;
    match (event, &pending.kind) {
        (NetEvent::AssistantUploadFolderCreated { result, .. }, PendingKind::Folder(path, parent, name)) => {
            p.in_flight = false;
            match result {
                Ok(folder)
                    if native_source
                        && !folder.id.is_nil()
                        && folder.id != *parent
                        && folder.parent == *parent
                        && folder.name == *name
                        && folder.type_default == -1 =>
                {
                    app.world.inventory.created_folders(vec![folder.clone()]);
                    p.folders.insert(path.clone(), folder.id);
                    if let Ok(frozen) = Frozen::capture(&app.world.inventory, folder.id) {
                        p.created.insert(folder.id, frozen);
                    } else {
                        stop_unsent(&mut p, "native_upload_folder_unavailable");
                    }
                }
                _ => stop_unsent(&mut p, "native_upload_folder_creation_unconfirmed"),
            }
        }
        (NetEvent::AssistantUploadReady { result, .. }, PendingKind::File(index, 0)) => {
            let index = *index;
            let destination = p.folders.get(&p.files[index].folders).copied();
            let valid = current
                && !p.cancelled
                && result.as_ref().is_ok_and(|price| *price >= 0 && *price <= p.files[index].cost)
                && destination.is_some_and(|id| destination_allowed(app, id))
                && prices(app)
                    .cost(p.files[index].kind, p.files[index].dimensions)
                    .is_some_and(|cost| cost <= p.files[index].cost)
                && target_current(app, &p.target)
                && p.created.values().all(|f| target_current(app, f));
            if valid
                && runtime
                    .continue_action(&pending.capture, || {
                        app.send(NetCommand::FinishAssistantUpload {
                            source: p.login.clone(),
                            request,
                            approved: true,
                        })
                    })
                    .is_ok()
            {
                p.files[index].state = "payment_submitted";
                pending.kind = PendingKind::File(index, 1);
                pending.started = Instant::now();
                keep = true;
            } else {
                app.send(NetCommand::FinishAssistantUpload {
                    source: p.login.clone(),
                    request,
                    approved: false,
                });
                p.files[index].state = "cancelled_before_payment";
                p.files[index].error = Some(if result.is_err() {
                    "native_upload_offer_rejected"
                } else {
                    "native_upload_payment_authority_retired"
                });
                p.in_flight = false;
                stop_unsent(&mut p, "upload_stopped_before_payment");
            }
        }
        (NetEvent::AssistantUploadCommitted { result, .. }, PendingKind::File(index, 1)) => {
            let index = *index;
            match result {
                Ok(done) if p.folders.get(&p.files[index].folders) == Some(&done.folder) && done.kind == p.files[index].kind => {
                    let f = &mut p.files[index];
                    f.item = Some(done.item);
                    f.charged = Some(done.cost);
                    f.cost_known = done.cost_known;
                    f.state = "uploaded_unverified";
                    if done.cost_known && done.cost > f.cost {
                        f.error = Some("native_upload_charge_exceeded_ceiling");
                        stop_unsent(&mut p, "upload_stopped_after_unexpected_charge");
                    }
                    if current
                        && runtime
                            .continue_action(&pending.capture, || {
                                app.send(NetCommand::VerifyAssistantUpload {
                                    source: p.login.clone(),
                                    request,
                                    approved: true,
                                })
                            })
                            .is_ok()
                    {
                        pending.kind = PendingKind::File(index, 2);
                        pending.started = Instant::now();
                        keep = true;
                    } else {
                        p.in_flight = false;
                        stop_unsent(&mut p, "upload_verification_authority_retired");
                    }
                }
                _ => {
                    p.files[index].state = "uncertain";
                    p.files[index].error = Some("native_upload_outcome_unknown");
                    p.in_flight = false;
                    stop_unsent(&mut p, "upload_stopped_after_uncertain_submission");
                }
            }
        }
        (NetEvent::AssistantUploadVerified { result, .. }, PendingKind::File(index, 2)) => {
            let index = *index;
            p.in_flight = false;
            match result {
                Ok(receipt)
                    if native_source
                        && p.files[index].item == Some(receipt.item.id)
                        && receipt.item.owner == app.world.agent_id
                        && p.folders.get(&p.files[index].folders) == Some(&receipt.item.parent) =>
                {
                    p.files[index].state = "confirmed";
                    p.index = index + 1;
                }
                _ => {
                    p.files[index].state = "uploaded_unverified";
                    p.files[index].error = Some("native_upload_inventory_verification_failed");
                    stop_unsent(&mut p, "upload_stopped_after_unverified_submission");
                }
            }
        }
        _ => {
            keep = true;
        }
    }
    if !source {
        stop_unsent(&mut p, "upload_source_retired");
    }
    if keep {
        assistant.inventory.uploads.pending.insert(request, pending.clone());
    }
    complete(app, &mut assistant, pending.plan, &mut p);
    assistant.inventory.uploads.plans.insert(pending.plan, p);
    app.character_mcp = Some(assistant);
    true
}
fn destination_allowed(app: &App, id: Uuid) -> bool {
    destination(app, id).is_ok()
}

pub(super) fn snapshot_current(app: &App, id: Uuid) -> bool {
    snapshot::valid(app, id)
}
pub(super) fn snapshot_pixels(app: &mut App, id: Uuid, size: (u32, u32), rgba: Vec<u8>) {
    snapshot::pixels(app, id, size, rgba);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reader_and_completed_output_keep_admission_across_endpoint_reset() {
        let lease = ReaderLease::acquire().expect("first reader");
        let output = ReaderResult {
            value: vec![1u8; 128],
            _lease: lease,
        };
        let c = synthetic_capture(Tool::InventoryUploadPreview);
        let mut state = super::super::State::default();
        state.uploads.preparing = Some(c.clone());
        state.bind(c.binding, c.endpoint);
        assert!(state.uploads.preparing.is_none());
        assert!(
            ReaderLease::acquire().is_none(),
            "reset admitted another reader while output retained resources"
        );
        assert_eq!(output.value.len(), 128);
        drop(output);
        assert!(ReaderLease::acquire().is_some());
    }
    #[test]
    fn encoded_proof_hash_is_exact_lowercase_sha256() {
        assert_eq!(hash(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
