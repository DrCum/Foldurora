//! Create, read back and verify each exact replacement before the original can
//! move to Trash. No permanent deletion operation exists in this adapter.
use super::*;
#[derive(Clone)]
pub(super) enum Stage {
    Created,
    Verify(Uuid),
    Trash(Uuid),
}
#[derive(Clone)]
struct Candidate {
    original: Frozen,
    parent: Uuid,
    parent_name: String,
}
#[derive(Clone)]
pub(super) struct Operation {
    capture: Capture,
    target: Frozen,
    target_id: Uuid,
    folder: bool,
    trash: Uuid,
    candidates: Vec<Candidate>,
    index: usize,
    results: Vec<Value>,
    started: Instant,
}
fn key(c: &Capture) -> (HostId, u64, String) {
    (c.endpoint, c.client, c.request_id.clone())
}
fn select(app: &App, c: &Capture) -> Result<Operation, &'static str> {
    args(c, &["source_id", "target_id", "skip_elicitation"])?;
    boolean(&c.arguments, "skip_elicitation", false)?;
    let inv = &app.world.inventory;
    let source = uuid(&c.arguments, "source_id")?;
    let target = rules::original(inv, uuid(&c.arguments, "target_id")?).ok_or("target_unavailable")?;
    if source == target {
        return Err("replacement_no_op");
    }
    ordinary(app, target)?;
    if rules::library(inv, target)
        || rules::in_type(inv, target, 14)
        || inv.items.get(&target).is_some_and(|i| matches!(i.asset_type, 2 | 24 | 25 | 49))
    {
        return Err("native_permission_denied");
    }
    let trash = inv.system_folder(14).ok_or("trash_unavailable")?;
    let mut candidates = Vec::new();
    for link in inv
        .items
        .values()
        .filter(|i| matches!(i.asset_type, 24 | 25) && i.asset_id == source && !rules::in_type(inv, i.id, 14))
    {
        if candidates.len() >= 50 {
            return Err("native_scope_limit");
        }
        candidates.push(Candidate {
            original: Frozen::capture(inv, link.id)?,
            parent: link.parent,
            parent_name: rules::name(inv, link.parent),
        });
    }
    candidates.sort_by_key(|row| row.original.id);
    Ok(Operation {
        capture: c.clone(),
        target: Frozen::capture(inv, target)?,
        target_id: target,
        folder: inv.folders.contains_key(&target),
        trash,
        candidates,
        index: 0,
        results: Vec::new(),
        started: Instant::now(),
    })
}
pub(super) fn prepare(app: &App, c: &Capture) -> Result<Vec<Class>, &'static str> {
    let _ = select(app, c)?;
    Ok(vec![Class::Read, Class::Trash])
}
pub(super) fn approval(app: &App, c: &Capture) -> Value {
    match select(app, c) {
        Ok(op) => {
            json!({"complete":true,"subject_count":op.candidates.len(),"target_id":op.target_id.to_string(),"target_name":op.target.name(),"subjects":op.candidates.iter().map(|r|json!({"id":r.original.id.to_string(),"name":r.original.name(),"parent_id":r.parent.to_string(),"parent_name":r.parent_name})).collect::<Vec<_>>(),"consequences":"Create and verify replacement links, then move eligible original links to Trash. Protected links are skipped."})
        }
        Err(code) => failure(code),
    }
}
fn eligible(app: &App, op: &Operation, row: &Candidate) -> bool {
    let inv = &app.world.inventory;
    row.original.current(inv)
        && rules::name(inv, row.parent) == row.parent_name
        && ordinary(app, row.original.id).is_ok()
        && rules::link_destination(inv, row.parent)
        && rules::removable(inv, row.original.id, &app.settings.inventory.protected)
        && op.target_id != row.parent
        && !rules::under(inv, row.parent, op.target_id)
}
fn link(op: &Operation, row: &Candidate) -> aurora_net::outfits::OutfitLink {
    let old = row.original.item.as_ref().expect("selected links are items");
    let replacement = op.target.item.as_ref();
    aurora_net::outfits::OutfitLink {
        target: op.target_id,
        name: op.target.name().to_owned(),
        desc: if replacement.is_some_and(|i| i.asset_type == 5) {
            old.desc.clone()
        } else {
            replacement.map(|i| i.desc.clone()).unwrap_or_default()
        },
        folder: op.folder,
        inv_type: replacement.map_or(8, |i| i.inv_type),
    }
}
fn next(app: &App, op: &mut Operation) -> Result<Option<(Candidate, aurora_net::outfits::OutfitLink)>, &'static str> {
    if op.started.elapsed() > Duration::from_secs(55) || !op.target.current(&app.world.inventory) {
        return Err("replacement_scope_changed_or_expired");
    }
    while let Some(row) = op.candidates.get(op.index).cloned() {
        if eligible(app, op, &row) {
            let link = link(op, &row);
            return Ok(Some((row, link)));
        }
        op.results.push(json!({"id":row.original.id.to_string(),"status":"skipped","original_preserved":true,"retry_safe":true,"trash_state":"not_submitted"}));
        op.index += 1;
    }
    Ok(None)
}
pub(super) fn execute(app: &mut App, state: &mut State, c: &Capture) -> Result<Value, &'static str> {
    if state.replacements.len() >= 32 {
        return Err("operation_capacity");
    }
    native_busy(app)?;
    let mut op = select(app, c)?;
    if let Some((row, link)) = next(app, &mut op)? {
        let request = Uuid::new_v4();
        pending_for(state, request, c, PendingKind::Links { stage: Stage::Created });
        state.replacements.insert(key(c), op);
        app.send(NetCommand::CreateAssistantInventoryLink {
            request,
            parent: row.parent,
            link,
        });
        Ok(submitted(c))
    } else {
        Ok(result(&op))
    }
}
fn result(op: &Operation) -> Value {
    let completed = op.results.iter().filter(|r| r["ok"] == true).count();
    json!({"operation_id":op.capture.request_id,"status":"complete","count":op.candidates.len(),"completed_count":completed,"partial":completed>0&&completed<op.candidates.len(),"results":op.results,"retry_safe":false})
}
fn complete(app: &mut App, mut op: Operation) {
    while let Some(row) = op.candidates.get(op.index) {
        op.results.push(json!({"id":row.original.id.to_string(),"status":"cancelled_before_submission","original_preserved":true,"retry_safe":true,"trash_state":"not_submitted"}));
        op.index += 1;
    }
    let value = result(&op);
    if let Some(a) = app.character_mcp.as_mut() {
        a.inventory.replacements.remove(&key(&op.capture));
    }
    mutations::finish(
        app,
        Pending {
            capture: op.capture,
            started: op.started,
            kind: PendingKind::Links { stage: Stage::Created },
        },
        value,
    );
}
fn advance(app: &mut App, mut op: Operation) {
    match next(app, &mut op) {
        Ok(Some((row, link))) => {
            let request = Uuid::new_v4();
            let capture = op.capture.clone();
            if commit_continuation(app, &capture, |app| {
                if let Some(a) = app.character_mcp.as_mut() {
                    a.inventory.replacements.insert(key(&capture), op.clone());
                    pending_for(&mut a.inventory, request, &capture, PendingKind::Links { stage: Stage::Created });
                }
                app.send(NetCommand::CreateAssistantInventoryLink {
                    request,
                    parent: row.parent,
                    link,
                });
            })
            .is_err()
            {
                complete(app, op);
            }
        }
        Ok(None) => complete(app, op),
        Err(code) => {
            op.results.push(failure(code));
            complete(app, op);
        }
    }
}
fn verified(op: &Operation, row: &Candidate, id: Uuid, item: &InvItem) -> bool {
    !id.is_nil()
        && id != row.original.id
        && item.id == id
        && item.parent == row.parent
        && item.asset_id == op.target_id
        && item.asset_type == if op.folder { 25 } else { 24 }
}
pub(super) fn native_event(app: &mut App, event: &NetEvent) -> bool {
    let request = match event {
        NetEvent::AssistantInventoryLinkCreated { request, .. }
        | NetEvent::AssistantInventoryItems { request, .. }
        | NetEvent::InventoryEdited { request, .. } => *request,
        _ => return false,
    };
    let found = app.character_mcp.as_ref().and_then(|a| {
        a.inventory
            .pending
            .get(&request)
            .filter(|p| matches!(p.kind, PendingKind::Links { .. }))
            .cloned()
    });
    let Some(pending) = found else {
        return false;
    };
    let op = app
        .character_mcp
        .as_ref()
        .and_then(|a| a.inventory.replacements.get(&key(&pending.capture)))
        .cloned();
    if let Some(a) = app.character_mcp.as_mut() {
        a.inventory.pending.remove(&request);
    }
    let Some(mut op) = op else {
        if super::super::source_current(app, &pending.capture)
            && let NetEvent::InventoryEdited { result: Ok(reply), .. } = event
        {
            app.world.inventory.remove(&reply.removed);
            app.world.inventory.apply(reply.contents.clone());
            app.world.inventory.created_folders(reply.folders.clone());
        }
        if app.inventory_ui.pending == Some(request) {
            app.inventory_ui.finish_mutation(false);
            app.inventory_ui.pending_refresh.clear();
        }
        return true;
    };
    if !super::super::source_current(app, &pending.capture) {
        return true;
    }
    let Some(row) = op.candidates.get(op.index).cloned() else {
        return true;
    };
    match (event, &pending.kind) {
        (NetEvent::AssistantInventoryLinkCreated { result: Ok(id), .. }, PendingKind::Links { stage: Stage::Created }) => {
            let read = Uuid::new_v4();
            let capture = op.capture.clone();
            if commit_continuation(app, &capture, |app| {
                if let Some(a) = app.character_mcp.as_mut() {
                    pending_for(&mut a.inventory, read, &capture, PendingKind::Links { stage: Stage::Verify(*id) });
                }
                app.send(NetCommand::FetchAssistantInventoryItems {
                    request: read,
                    items: vec![*id],
                    owner: app.world.agent_id,
                });
            })
            .is_err()
            {
                op.results.push(json!({"id":row.original.id.to_string(),"new_id":id.to_string(),"status":"cancelled_after_creation","original_preserved":true,"retry_safe":false,"trash_state":"not_submitted"}));
                op.index += 1;
                complete(app, op);
            }
        }
        (NetEvent::AssistantInventoryItems { result: Ok(reads), .. }, PendingKind::Links { stage: Stage::Verify(id) }) => {
            let matched = reads
                .items
                .iter()
                .find(|item| verified(&op, &row, *id, item) && item.owner == app.world.agent_id)
                .cloned();
            if let Some(item) = matched {
                app.world.inventory.add_items(vec![item]);
                let facts = crate::ui::inventory::Facts::from_world(&app.world, app.appearance_ui.pending.is_some());
                let change = if eligible(app, &op, &row) && op.target.current(&app.world.inventory) {
                    rules::move_selection(
                        &app.world.inventory,
                        &[row.original.id],
                        op.trash,
                        &app.settings.inventory.protected,
                        &facts.worn,
                    )
                    .ok()
                } else {
                    None
                };
                if let Some(change) = change {
                    let write = Uuid::new_v4();
                    let capture = op.capture.clone();
                    if commit_continuation(app, &capture, |app| {
                        if app.mcp_submit_inventory_change(write, change).is_ok() {
                            if let Some(a) = app.character_mcp.as_mut() {
                                pending_for(&mut a.inventory, write, &capture, PendingKind::Links { stage: Stage::Trash(*id) });
                            }
                            true
                        } else {
                            false
                        }
                    })
                    .is_ok_and(|sent| sent)
                    {
                        return true;
                    }
                }
            }
            op.results.push(json!({"id":row.original.id.to_string(),"new_id":id.to_string(),"status":"replacement_unverified_or_scope_changed","ok":false,"original_preserved":true,"retry_safe":false,"trash_state":"not_submitted"}));
            op.index += 1;
            advance(app, op);
        }
        (NetEvent::InventoryEdited { result, .. }, PendingKind::Links { stage: Stage::Trash(id) }) => {
            if let Ok(reply) = result {
                app.world.inventory.remove(&reply.removed);
                app.world.inventory.apply(reply.contents.clone());
                app.world.inventory.created_folders(reply.folders.clone());
            }
            if app.inventory_ui.pending == Some(request) {
                app.inventory_ui.finish_mutation(result.is_ok());
                app.inventory_ui.pending_refresh.clear();
            }
            op.results.push(json!({"id":row.original.id.to_string(),"new_id":id.to_string(),"ok":result.is_ok(),"status":if result.is_ok(){"replacement_verified_trash_submitted"}else{"trash_outcome_uncertain"},"original_preserved":if result.is_ok(){Some(false)}else{None},"retry_safe":false,"trash_state":"submitted","server_confirmed":false}));
            op.index += 1;
            advance(app, op);
        }
        _ => {
            op.results.push(json!({"id":row.original.id.to_string(),"status":"creation_or_read_outcome_uncertain","ok":false,"original_preserved":true,"retry_safe":false,"trash_state":"not_submitted"}));
            op.index += 1;
            advance(app, op);
        }
    }
    true
}

pub(super) fn prune(state: &mut State, runtime: &Runtime) {
    state
        .replacements
        .retain(|_, op| op.started.elapsed() < Duration::from_secs(70) && runtime.capture_current(&op.capture));
}
