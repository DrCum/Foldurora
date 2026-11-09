//! Read-only prerequisite loading. Mutation approval is created only after
//! the complete bounded native scope is available and has been frozen.
use super::*;
type Key = (HostId, u64, String);
#[derive(Clone)]
pub(super) struct Job {
    capture: Capture,
    started: Instant,
    in_flight: Option<Uuid>,
    failed: bool,
}
#[derive(Clone)]
pub(super) struct Request {
    capture: Capture,
    read: Read,
}
#[derive(Clone)]
enum Read {
    Folder { parent: Uuid, owner: Uuid, library: bool },
    Items(Vec<Uuid>),
}
fn key(c: &Capture) -> Key {
    (c.endpoint, c.client, c.request_id.clone())
}
fn folder(inv: &Inventory, id: Uuid) -> Result<Option<Read>, &'static str> {
    let f = inv.folders.get(&id).ok_or("not_found")?;
    Ok((f.state != FetchState::Fetched).then_some(Read::Folder {
        parent: id,
        owner: if f.library { inv.lib_owner } else { inv.owner },
        library: f.library,
    }))
}
fn missing(inv: &Inventory, ids: impl IntoIterator<Item = Uuid>) -> Option<Read> {
    let ids = ids
        .into_iter()
        .filter(|id| !inv.items.contains_key(id) && !inv.folders.contains_key(id) && !inv.broken(id))
        .take(50)
        .collect::<Vec<_>>();
    (!ids.is_empty()).then_some(Read::Items(ids))
}
fn tree(inv: &Inventory, root: Uuid, maximum: usize, originals: bool) -> Result<Option<Read>, &'static str> {
    let mut stack = vec![root];
    let mut seen = HashSet::new();
    let mut linked = HashSet::new();
    let mut count = 0usize;
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            return Err("native_inventory_cycle");
        }
        count = count.saturating_add(1);
        if count > maximum {
            return Err("native_scope_limit");
        }
        if inv.items.contains_key(&id) {
            continue;
        }
        if let Some(read) = folder(inv, id)? {
            return Ok(Some(read));
        }
        let f = &inv.folders[&id];
        if maximum <= 512 {
            count = count.saturating_add(f.items.len());
            if count > maximum {
                return Err("native_scope_limit");
            }
        }
        if seen.len().saturating_add(stack.len()).saturating_add(f.children.len()) > maximum {
            return Err("native_scope_limit");
        }
        stack.extend(f.children.iter().copied());
        if originals {
            for id in &f.items {
                if let Some(i) = inv.items.get(id)
                    && matches!(i.asset_type, 24 | 25)
                    && !i.asset_id.is_nil()
                {
                    linked.insert(i.asset_id);
                }
            }
        }
    }
    Ok(missing(inv, linked))
}
fn need(app: &App, c: &Capture) -> Result<Option<Read>, &'static str> {
    let inv = &app.world.inventory;
    let a = &c.arguments;
    match c.tool {
        Tool::InventoryLandmark | Tool::InventoryGet | Tool::InventoryPermissionsGet => Ok(missing(inv, [uuid(a, "id")?])),
        Tool::InventoryGetMany => Ok(missing(inv, ids(a, "ids")?)),
        Tool::InventoryList => folder(inv, uuid(a, "folder_id")?),
        Tool::InventorySearch => {
            let root = optional_uuid(a, "folder_id")?.unwrap_or(inv.root);
            if inv.folders.values().all(|f| f.library || f.state == FetchState::Fetched) {
                Ok(None)
            } else {
                tree(inv, root, 16_384, false)
            }
        }
        Tool::InventoryResolvePath => {
            let mut found = vec![optional_uuid(a, "folder_id")?.unwrap_or(inv.root)];
            let parts = string(a, "path", 1024)?.split('/').filter(|p| !p.is_empty()).collect::<Vec<_>>();
            if parts.len() > 32 {
                return Err("invalid_args");
            }
            for part in parts {
                let mut next = Vec::new();
                for parent in found {
                    if let Some(read) = folder(inv, parent)? {
                        return Ok(Some(read));
                    }
                    let f = &inv.folders[&parent];
                    for id in f.children.iter().chain(&f.items) {
                        if rules::name(inv, *id) == part {
                            next.push(*id);
                            if next.len() > 50 {
                                return Err("native_scope_limit");
                            }
                        }
                    }
                }
                found = next;
            }
            Ok(None)
        }
        Tool::InventoryReplaceLinks => tree(inv, inv.root, 16_384, false),
        Tool::InventoryCopy | Tool::InventoryBatchCopy => {
            let roots = if c.tool == Tool::InventoryCopy {
                vec![uuid(a, "id")?]
            } else {
                ids(a, "ids")?
            };
            for id in roots {
                if let Some(read) = missing(inv, [id]) {
                    return Ok(Some(read));
                }
                if inv.folders.contains_key(&id) {
                    if let Some(read) = tree(inv, id, 512, true)? {
                        return Ok(Some(read));
                    }
                } else if let Some(i) = inv.items.get(&id)
                    && matches!(i.asset_type, 24 | 25)
                    && let Some(read) = missing(inv, [i.asset_id])
                {
                    return Ok(Some(read));
                }
            }
            Ok(None)
        }
        Tool::AppearanceOutfits => inv.system_folder(48).map(|id| tree(inv, id, 4096, false)).unwrap_or(Ok(None)),
        Tool::AppearanceOutfitItems => tree(inv, uuid(a, "outfit_id")?, 512, true),
        Tool::AppearanceWearOutfit => {
            let id = if let Some(id) = optional_uuid(a, "folder_id")? {
                id
            } else {
                let name = string(a, "folder_name", 128)?;
                let mut found = inv
                    .folders
                    .values()
                    .filter(|f| f.info.type_default == 47 && !f.library && f.info.name == name);
                let id = found.next().ok_or("not_found")?.info.id;
                if found.next().is_some() {
                    return Err("ambiguous_outfit");
                }
                id
            };
            if let Some(read) = tree(inv, id, 512, true)? {
                return Ok(Some(read));
            }
            if let Some(cof) = inv.system_folder(46) {
                tree(inv, cof, 512, true)
            } else {
                Ok(None)
            }
        }
        Tool::AppearanceWearItems | Tool::AppearanceDetach => {
            if let Some(read) = missing(inv, ids(a, "items_id")?) {
                return Ok(Some(read));
            }
            if let Some(cof) = inv.system_folder(46) {
                tree(inv, cof, 512, true)
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}
fn static_args(app: &mut App, state: &State, c: &Capture) -> Result<(), &'static str> {
    match c.tool {
        Tool::InventoryGet
        | Tool::InventoryGetMany
        | Tool::InventoryList
        | Tool::InventorySearch
        | Tool::InventoryResolvePath
        | Tool::AppearanceOutfits
        | Tool::AppearanceOutfitItems => {
            reads::prepare(app, state, c)?;
        }
        Tool::InventoryLandmark | Tool::InventoryPermissionsGet => {
            args(c, &["id"])?;
            uuid(&c.arguments, "id")?;
        }
        Tool::InventoryReplaceLinks => {
            args(c, &["source_id", "target_id", "skip_elicitation"])?;
            uuid(&c.arguments, "source_id")?;
            uuid(&c.arguments, "target_id")?;
            boolean(&c.arguments, "skip_elicitation", false)?;
        }
        Tool::InventoryCopy => {
            args(c, &["id", "parent_id", "policy", "skip_elicitation"])?;
            uuid(&c.arguments, "id")?;
            uuid(&c.arguments, "parent_id")?;
            boolean(&c.arguments, "skip_elicitation", false)?;
        }
        Tool::InventoryBatchCopy => {
            args(c, &["ids", "parent_id", "policy"])?;
            ids(&c.arguments, "ids")?;
            uuid(&c.arguments, "parent_id")?;
        }
        Tool::AppearanceWearOutfit => {
            args(c, &["folder_id", "folder_name", "append", "skip_elicitation"])?;
            optional_uuid(&c.arguments, "folder_id")?;
            boolean(&c.arguments, "append", false)?;
            boolean(&c.arguments, "skip_elicitation", false)?;
        }
        Tool::AppearanceWearItems | Tool::AppearanceDetach => {
            args(c, &["items_id", "replace", "skip_elicitation"])?;
            ids(&c.arguments, "items_id")?;
            boolean(&c.arguments, "replace", false)?;
            boolean(&c.arguments, "skip_elicitation", false)?;
        }
        _ => {}
    }
    if matches!(c.tool, Tool::InventoryCopy | Tool::InventoryBatchCopy)
        && c.arguments
            .get("policy")
            .is_some_and(|v| !matches!(v.as_str(), Some("default" | "strict" | "copyable_only")))
    {
        return Err("invalid_args");
    }
    Ok(())
}
pub(super) fn run(app: &mut App, assistant: &mut Assistant, c: &Capture) -> Result<bool, &'static str> {
    assistant.inventory.bind(c.binding, c.endpoint);
    static_args(app, &assistant.inventory, c)?;
    let key = key(c);
    if let Some(job) = assistant.inventory.loading.get(&key) {
        if job.capture != *c {
            return Err("stale_preparation");
        }
        if job.failed {
            assistant.inventory.loading.remove(&key);
            return Err("native_inventory_read_failed");
        }
        if job.started.elapsed() > Duration::from_secs(45) {
            assistant.inventory.loading.remove(&key);
            return Err("native_inventory_preparation_timeout");
        }
        if job.in_flight.is_some() {
            return Ok(false);
        }
    }
    let Some(read) = need(app, c)? else {
        assistant.inventory.loading.remove(&key);
        return Ok(true);
    };
    if assistant.inventory.loading.len() >= 32 && !assistant.inventory.loading.contains_key(&key)
        || assistant.inventory.load_requests.len() >= CAPACITY
    {
        return Err("preparation_capacity");
    }
    let request = Uuid::new_v4();
    let command = match &read {
        Read::Folder { parent, owner, library } => NetCommand::FetchAssistantInventoryFolder {
            request,
            parent: *parent,
            owner: *owner,
            library: *library,
        },
        Read::Items(items) => NetCommand::FetchAssistantInventoryItems {
            request,
            items: items.clone(),
            owner: app.world.agent_id,
        },
    };
    let runtime = assistant.runtime.clone();
    runtime.continue_preparation(c, Class::Read, || {
        let job = assistant.inventory.loading.entry(key).or_insert_with(|| Job {
            capture: c.clone(),
            started: Instant::now(),
            in_flight: None,
            failed: false,
        });
        job.in_flight = Some(request);
        assistant
            .inventory
            .load_requests
            .insert(request, Request { capture: c.clone(), read });
        app.send(command);
    })?;
    Ok(false)
}
pub(super) fn native_event(app: &mut App, event: &NetEvent) -> bool {
    let request = match event {
        NetEvent::AssistantInventoryFolder { request, .. } | NetEvent::AssistantInventoryItems { request, .. } => *request,
        _ => return false,
    };
    let Some(owned) = app.character_mcp.as_mut().and_then(|a| a.inventory.load_requests.remove(&request)) else {
        return false;
    };
    let source = super::super::source_current(app, &owned.capture);
    let mut okay = false;
    if source {
        match (event, &owned.read) {
            (
                NetEvent::AssistantInventoryFolder { parent, result, .. },
                Read::Folder {
                    parent: expected, owner, ..
                },
            ) if parent == expected => {
                if let Ok(contents) = result
                    && contents.iter().all(|c| c.folder_id == *parent && c.owner_id == *owner)
                {
                    app.world.inventory.apply(contents.clone());
                    okay = true;
                }
            }
            (NetEvent::AssistantInventoryItems { result: Ok(reads), .. }, Read::Items(asked))
                if reads.items.iter().all(|i| asked.contains(&i.id))
                    && reads.missing.iter().all(|id| asked.contains(id))
                    && reads.failed.is_empty() =>
            {
                app.world.inventory.add_items(reads.items.clone());
                app.world.inventory.items_missing(reads.missing.clone());
                okay = true;
            }
            _ => {}
        }
    }
    if let Some(a) = app.character_mcp.as_mut()
        && let Some(job) = a.inventory.loading.get_mut(&key(&owned.capture))
        && job.in_flight == Some(request)
    {
        job.in_flight = None;
        job.failed = !okay;
    }
    true
}

pub(super) fn prune(state: &mut State, runtime: &Runtime) {
    state
        .loading
        .retain(|_, job| runtime.continue_preparation(&job.capture, Class::Read, || ()).is_ok());
    state
        .load_requests
        .retain(|_, request| state.loading.contains_key(&key(&request.capture)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_mcp::assistant::{
        permissions::{Configuration, Level},
        runtime::{Runtime, ViewerRequest},
    };
    fn engine(read: Level) -> (Runtime, Capture) {
        let initial = synthetic_capture(Tool::InventoryCopy);
        let runtime = Runtime::default();
        runtime.observe(Some(initial.binding), true);
        let mut config = Configuration::default();
        config.permissions.set(Class::Read, read).unwrap();
        config.permissions.set(Class::Move, Level::Ask).unwrap();
        runtime.configure(config).unwrap();
        runtime.capabilities([Tool::InventoryCopy]);
        runtime.enable(initial.binding, initial.endpoint).unwrap();
        let reply = runtime.request(
            initial.client,
            ViewerRequest::Invoke {
                viewer_session: initial.endpoint,
                tool: Tool::InventoryCopy,
                arguments: serde_json::Map::new(),
                request_id: Some(Uuid::from_u128(9200).to_string()),
            },
        );
        assert_eq!(reply["status"], "preparing");
        let capture = runtime.preparations().pop().unwrap();
        (runtime, capture)
    }
    #[test]
    fn metadata_preparation_cannot_spend_or_self_grant_pending_mutation_authority() {
        let (runtime, c) = engine(Level::Allow);
        let mut reads = 0;
        runtime.continue_preparation(&c, Class::Read, || reads += 1).unwrap();
        assert_eq!(reads, 1);
        assert!(runtime.snapshot().approvals.is_empty());
        runtime.prepared(&c.request_id, &[], false).unwrap();
        assert_eq!(runtime.snapshot().approvals.len(), 1);
        assert!(runtime.ready().is_empty());
        assert!(runtime.continue_preparation(&c, Class::Read, || reads += 1).is_err());
        assert_eq!(reads, 1);
        let (denied, c) = engine(Level::Never);
        assert_eq!(
            denied.continue_preparation(&c, Class::Read, || reads += 1),
            Err("preparation_read_not_allowed")
        );
        assert_eq!(reads, 1);
    }
    #[test]
    fn cold_scope_is_loaded_before_freezing_and_later_descendants_require_fresh_approval() {
        use crate::world::inventory::Folder;
        use aurora_net::inventory::{FolderContents, InvFolder};
        let root = Uuid::from_u128(9100);
        let owner = Uuid::from_u128(9101);
        let mut inv = Inventory {
            root,
            owner,
            ..Default::default()
        };
        inv.folders.insert(
            root,
            Folder {
                info: InvFolder {
                    id: root,
                    name: "Synthetic cold folder".into(),
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Unknown,
                library: false,
            },
        );
        assert!(matches!(tree(&inv,root,512,true).unwrap(),Some(Read::Folder{parent,..})if parent==root));
        let state = State::default();
        assert!(state.evidence.borrow().is_empty());
        inv.apply(vec![FolderContents {
            folder_id: root,
            owner_id: owner,
            version: 1,
            folders: Vec::new(),
            items: Vec::new(),
        }]);
        assert!(tree(&inv, root, 512, true).unwrap().is_none());
        let c = synthetic_capture(Tool::InventoryCopy);
        freeze_evidence(&state, &c, inv.generation, [0; 32]).unwrap();
        inv.add_items(vec![InvItem {
            id: Uuid::from_u128(9102),
            parent: root,
            name: "Unapproved arrival".into(),
            ..Default::default()
        }]);
        assert_eq!(freeze_evidence(&state, &c, inv.generation, [0; 32]), Err("native_scope_changed"));
    }
    #[test]
    fn prerequisite_reads_reject_foreign_connection_or_retired_endpoint() {
        let (runtime, c) = engine(Level::Allow);
        let mut reads = 0;
        let mut other = c.clone();
        other.client += 1;
        assert!(runtime.continue_preparation(&other, Class::Read, || reads += 1).is_err());
        other = c.clone();
        other.endpoint = HostId::from_bytes([90; 16]).unwrap();
        assert!(runtime.continue_preparation(&other, Class::Read, || reads += 1).is_err());
        runtime.disable();
        assert!(runtime.continue_preparation(&c, Class::Read, || reads += 1).is_err());
        assert_eq!(reads, 0);
    }
}
