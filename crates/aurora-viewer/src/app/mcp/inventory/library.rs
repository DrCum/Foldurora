//! Correlated Library-to-owned inventory copies before native appearance wear.
//! The native copy callback never enters the ordinary unguarded wear-on-copy UI.
use super::*;
use crate::world::appearance::{self, Action};

#[derive(Clone)]
pub(super) struct CofProof {
    folder: Uuid,
    ids: Vec<Uuid>,
    frozen: Vec<Frozen>,
    worn: Vec<(Uuid, u8)>,
}
#[derive(Clone)]
pub(super) struct Group {
    capture: Capture,
    sources: Vec<InvItem>,
    items: Vec<Option<Uuid>>,
    verified: Vec<Option<InvItem>>,
    proof: CofProof,
    replace: bool,
    started: Instant,
    failed: bool,
}
pub(super) fn cof_proof(app: &App) -> Result<CofProof, &'static str> {
    let inv = &app.world.inventory;
    let folder = inv.system_folder(46).ok_or("current_outfit_unavailable")?;
    let cof = inv.folders.get(&folder).ok_or("current_outfit_unavailable")?;
    if cof.items.len() > 128 || !appearance::complete(inv, folder) {
        return Err("current_outfit_loading_or_limit");
    }
    let mut ids = cof.items.clone();
    ids.sort();
    let mut frozen = vec![Frozen::capture(inv, folder)?];
    for id in &ids {
        frozen.push(Frozen::capture(inv, *id)?);
        if let Some(original) = rules::original(inv, *id)
            && original != *id
            && inv.items.contains_key(&original)
        {
            frozen.push(Frozen::capture(inv, original)?);
        }
    }
    let mut worn = app.world.outfit_attachment_points().into_iter().collect::<Vec<_>>();
    worn.sort();
    if worn.len() > 128 {
        return Err("native_appearance_limit");
    }
    Ok(CofProof { folder, ids, frozen, worn })
}
impl CofProof {
    fn current(&self, app: &App) -> bool {
        let inv = &app.world.inventory;
        let mut ids = inv.folders.get(&self.folder).map(|f| f.items.clone()).unwrap_or_default();
        ids.sort();
        let mut worn = app.world.outfit_attachment_points().into_iter().collect::<Vec<_>>();
        worn.sort();
        ids == self.ids && worn == self.worn && appearance::complete(inv, self.folder) && self.frozen.iter().all(|f| f.current(inv))
    }
}
pub(super) fn prepare(app: &App, c: &Capture) -> Result<bool, &'static str> {
    if c.tool != Tool::AppearanceWearItems {
        return Ok(false);
    }
    let targets = ids(&c.arguments, "items_id")?;
    let inv = &app.world.inventory;
    if !targets.iter().any(|id| rules::library(inv, *id)) {
        return Ok(false);
    }
    native_busy(app)?;
    cof_proof(app)?;
    for id in targets {
        ordinary(app, id)?;
        let item = inv.items.get(&id).ok_or("item_unavailable")?;
        if !matches!(item.asset_type, 5 | 6 | 13 | 21) || rules::in_type(inv, id, 14) {
            return Err("unsupported_item_type");
        }
        if rules::library(inv, id) {
            if inv.lib_root.is_nil() || !rules::under(inv, id, inv.lib_root) || item.owner.is_nil() || item.asset_id.is_nil() {
                return Err("library_source_unavailable");
            }
        } else if item.owner != app.world.agent_id || !rules::under(inv, id, inv.root) {
            return Err("native_permission_denied");
        }
    }
    Ok(true)
}
pub(super) fn execute(app: &mut App, state: &mut State, c: &Capture) -> Result<Value, &'static str> {
    if !prepare(app, c)? {
        return Err("library_copy_not_required");
    }
    if state.library.len() >= 32 {
        return Err("library_copy_capacity");
    }
    let targets = ids(&c.arguments, "items_id")?;
    let sources = targets.iter().map(|id| app.world.inventory.items[id].clone()).collect::<Vec<_>>();
    let copies = sources.iter().filter(|i| rules::library(&app.world.inventory, i.id)).count();
    if state.pending.len().saturating_add(copies) > CAPACITY {
        return Err("operation_capacity");
    }
    let group = Uuid::new_v4();
    let proof = cof_proof(app)?;
    let replace = boolean(&c.arguments, "replace", false)?;
    let items = sources
        .iter()
        .map(|i| (!rules::library(&app.world.inventory, i.id)).then_some(i.id))
        .collect();
    state.library.insert(
        group,
        Group {
            capture: c.clone(),
            sources: sources.clone(),
            items,
            verified: vec![None; sources.len()],
            proof,
            replace,
            started: Instant::now(),
            failed: false,
        },
    );
    let copies_to_send = sources
        .iter()
        .enumerate()
        .filter(|(_, i)| rules::library(&app.world.inventory, i.id))
        .map(|(index, source)| (index, source.clone()))
        .collect::<Vec<_>>();
    for (index, source) in copies_to_send {
        let request = Uuid::new_v4();
        pending_for(state, request, c, PendingKind::LibraryCopy { group, index });
        app.send(NetCommand::CopyInventoryItem {
            request,
            copy: aurora_net::inventory::operations::CopyItem {
                item: source.id,
                owner: source.owner,
                parent: Uuid::nil(),
                name: String::new(),
            },
        });
    }
    Ok(
        json!({"status":"submitted","operation_id":c.request_id,"phase":"library_copy","wear_submitted":false,"copy_count":copies,"confirmed":false,"retry_safe":false}),
    )
}
fn valid_copy(source: &InvItem, copy: &InvItem, agent: Uuid) -> bool {
    !copy.id.is_nil()
        && copy.id != source.id
        && copy.owner == agent
        && !copy.group_owned
        && copy.asset_type == source.asset_type
        && copy.inv_type == source.inv_type
        && copy.asset_id == source.asset_id
        && copy.flags & 0xff == source.flags & 0xff
}
fn copy_current(source: &InvItem, verified: &InvItem, current: Option<&InvItem>, agent: Uuid) -> bool {
    current == Some(verified) && valid_copy(source, verified, agent)
}
fn stopped(group: &Group, code: &'static str) -> Value {
    json!({"status":"uncertain","operation_id":group.capture.request_id,"error_code":code,"copied_item_ids":group.items.iter().zip(&group.sources).filter_map(|(id,s)|id.filter(|id|*id!=s.id).map(|id|id.to_string())).collect::<Vec<_>>(),"wear_submitted":false,"confirmed":false,"retry_safe":false})
}
fn current_group(app: &App, group: &Group) -> bool {
    !group.failed
        && group.items.iter().all(Option::is_some)
        && group.proof.current(app)
        && group.sources.iter().all(|old| app.world.inventory.items.get(&old.id) == Some(old))
        && group
            .sources
            .iter()
            .zip(&group.items)
            .zip(&group.verified)
            .all(|((source, id), verified)| {
                if *id == Some(source.id) {
                    return true;
                }
                let Some(copy) = verified else {
                    return false;
                };
                *id == Some(copy.id)
                    && copy_current(source, copy, app.world.inventory.items.get(&copy.id), app.world.agent_id)
                    && rules::under(&app.world.inventory, copy.id, app.world.inventory.root)
                    && !rules::in_type(&app.world.inventory, copy.id, 14)
                    && ordinary(app, copy.id).is_ok()
            })
}
fn finish_group(app: &mut App, group: Group) {
    let valid = current_group(app, &group);
    if !valid {
        mutations::finish(
            app,
            Pending {
                capture: group.capture.clone(),
                started: group.started,
                kind: PendingKind::Appearance { complete: true },
            },
            stopped(&group, "library_wear_scope_changed_or_copy_failed"),
        );
        return;
    }
    let action = Action::WearItems {
        items: group.items.iter().flatten().copied().collect(),
        replace: group.replace,
        point: 0,
    };
    let committed = commit_continuation(app, &group.capture, |app| {
        if !current_group(app, &group) {
            return Err("library_wear_scope_changed");
        }
        let Some(mut assistant) = app.character_mcp.take() else {
            return Err("native_assistant_unavailable");
        };
        let result = mutations::tracked_appearance(app, &mut assistant.inventory, &group.capture, action, true);
        app.character_mcp = Some(assistant);
        result
    });
    if !matches!(committed, Ok(Ok(_))) {
        mutations::finish(
            app,
            Pending {
                capture: group.capture.clone(),
                started: group.started,
                kind: PendingKind::Appearance { complete: true },
            },
            stopped(&group, "library_wear_continuation_cancelled"),
        );
    }
}
pub(super) fn native_event(app: &mut App, event: &NetEvent) -> bool {
    let NetEvent::InventoryItemCopied { request, result } = event else {
        return false;
    };
    let Some(pending) = app.character_mcp.as_ref().and_then(|a| a.inventory.pending.get(request)).cloned() else {
        return false;
    };
    let PendingKind::LibraryCopy { group, index } = pending.kind else {
        return false;
    };
    let current = super::super::source_current(app, &pending.capture);
    let source = app
        .character_mcp
        .as_ref()
        .and_then(|a| a.inventory.library.get(&group))
        .and_then(|g| g.sources.get(index))
        .cloned();
    let valid = result.as_ref().is_ok_and(|copy| {
        current
            && source.as_ref().is_some_and(|s| valid_copy(s, copy, app.world.agent_id))
            && !copy.parent.is_nil()
            && rules::under(&app.world.inventory, copy.parent, app.world.inventory.root)
    });
    if valid && let Ok(copy) = result {
        app.world.inventory.add_items(vec![copy.clone()]);
    }
    let Some(assistant) = app.character_mcp.as_mut() else { return true };
    assistant.inventory.pending.remove(request);
    let Some(g) = assistant.inventory.library.get_mut(&group) else {
        return true;
    };
    if g.capture != pending.capture {
        return true;
    }
    if valid {
        g.items[index] = result.as_ref().ok().map(|i| i.id);
        g.verified[index] = result.as_ref().ok().cloned();
    } else {
        g.failed = true;
    }
    let still_pending = assistant
        .inventory
        .pending
        .values()
        .any(|p| matches!(p.kind,PendingKind::LibraryCopy {group:id,..}if id==group));
    if !still_pending && let Some(g) = assistant.inventory.library.remove(&group) {
        finish_group(app, g);
    }
    true
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    let retired = assistant
        .inventory
        .library
        .iter()
        .filter(|(_, g)| g.started.elapsed() > Duration::from_secs(45) || !assistant.runtime.capture_current(&g.capture))
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    for id in retired {
        if let Some(group) = assistant.inventory.library.remove(&id) {
            let value = stopped(&group, "library_copy_timeout_or_authority_retired");
            let _ = receipts::record(app, &mut assistant.inventory, &group.capture, &value);
            outcome(
                &mut assistant.inventory,
                &group.capture,
                group.capture.request_id.clone(),
                value.clone(),
            );
            let _ = assistant.runtime.complete(&group.capture, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn library_callback_cannot_substitute_another_asset_owner_or_wearable_type() {
        let source = InvItem {
            id: Uuid::from_u128(8100),
            asset_id: Uuid::from_u128(8101),
            asset_type: 5,
            inv_type: 18,
            flags: 4,
            ..Default::default()
        };
        let agent = Uuid::from_u128(8102);
        let copy = InvItem {
            id: Uuid::from_u128(8103),
            owner: agent,
            ..source.clone()
        };
        assert!(valid_copy(&source, &copy, agent));
        assert!(copy_current(&source, &copy, Some(&copy), agent));
        let mut edited_after_callback = copy.clone();
        edited_after_callback.name = "Changed while another copy was pending".into();
        assert!(valid_copy(&source, &edited_after_callback, agent));
        assert!(!copy_current(&source, &copy, Some(&edited_after_callback), agent));
        for alteration in 0..4 {
            let mut changed = copy.clone();
            match alteration {
                0 => changed.owner = Uuid::from_u128(8104),
                1 => changed.asset_id = Uuid::from_u128(8105),
                2 => changed.flags = 5,
                _ => changed.group_owned = true,
            }
            assert!(!valid_copy(&source, &changed, agent));
        }
    }
}
