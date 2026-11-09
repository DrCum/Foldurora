use super::*;
use crate::world::appearance::{self, Action};

pub(super) fn supports(t: Tool) -> bool {
    matches!(
        t,
        Tool::InventoryReplaceLinks
            | Tool::InventoryConfirmCopy
            | Tool::InventoryCreateFolder
            | Tool::InventoryCreateItem
            | Tool::InventoryMove
            | Tool::InventoryRename
            | Tool::InventoryCopy
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
            | Tool::InventoryPreviewBatchMove
            | Tool::InventoryPreviewBatchRename
            | Tool::InventoryExecutePlan
            | Tool::InventoryPreviewUndo
            | Tool::AppearanceWearOutfit
            | Tool::AppearanceWearItems
            | Tool::AppearanceDetach
    )
}
pub(super) fn prepare(app: &mut App, state: &State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    let a = &c.arguments;
    match c.tool {
        Tool::InventoryReplaceLinks => return links::prepare(app, c),
        Tool::InventoryCreateItem => {
            args(c, &["type", "parent_id", "name", "desc"])?;
            creation(app, c)?;
            return Ok(Vec::new());
        }
        Tool::InventoryCreateFolder => {
            args(c, &["parent_id", "name", "type"])?;
            uuid(a, "parent_id")?;
            string(a, "name", 63)?;
            if a.get("type").is_some_and(|v| v.as_str() != Some("") && v.as_str() != Some("none")) {
                return Err("unsupported_folder_type");
            }
        }
        Tool::InventoryMove | Tool::InventoryLink => {
            args(c, &["id", "parent_id"])?;
            uuid(a, "id")?;
            uuid(a, "parent_id")?;
        }
        Tool::InventoryCopy => {
            args(c, &["id", "parent_id", "policy", "skip_elicitation"])?;
            uuid(a, "id")?;
            uuid(a, "parent_id")?;
            copy_policy(a)?;
            boolean(a, "skip_elicitation", false)?;
        }
        Tool::InventoryBatchCopy => {
            args(c, &["ids", "parent_id", "policy"])?;
            ids(a, "ids")?;
            uuid(a, "parent_id")?;
            copy_policy(a)?;
        }
        Tool::InventoryBatchMove | Tool::InventoryPreviewBatchMove => {
            args(c, &["ids", "parent_id"])?;
            ids(a, "ids")?;
            uuid(a, "parent_id")?;
        }
        Tool::InventoryRename => {
            args(c, &["id", "name"])?;
            uuid(a, "id")?;
            string(a, "name", 63)?;
        }
        Tool::InventoryBatchRename | Tool::InventoryPreviewBatchRename => {
            args(c, &["items"])?;
            rename_rows(a)?;
        }
        Tool::InventoryTrash | Tool::InventoryRestore => {
            args(c, &["id"])?;
            uuid(a, "id")?;
        }
        Tool::InventorySetDescription => {
            args(c, &["id", "desc"])?;
            uuid(a, "id")?;
            string(a, "desc", 1023)?;
        }
        Tool::InventorySetFavorite => {
            args(c, &["id", "favorite"])?;
            uuid(a, "id")?;
            boolean(a, "favorite", false)?;
            if !a.contains_key("favorite") {
                return Err("invalid_args");
            }
        }
        Tool::InventorySetThumbnail => {
            args(c, &["id", "thumbnail_id"])?;
            uuid(a, "id")?;
            optional_uuid(a, "thumbnail_id")?;
        }
        Tool::InventoryNextOwnerSet => {
            args(c, &["id", "expected_revision", "copy", "modify", "transfer"])?;
            uuid(a, "id")?;
            string(a, "expected_revision", 128)?;
            for key in ["copy", "modify", "transfer"] {
                if !a.contains_key(key) {
                    return Err("invalid_args");
                }
                boolean(a, key, false)?;
            }
        }
        Tool::InventoryConfirmCopy => {
            args(c, &["plan_id", "confirm"])?;
            boolean(a, "confirm", true)?;
            let p = plan(state, c)?;
            if !p.confirm_only {
                return Err("wrong_plan_type");
            }
            if boolean(a, "confirm", true)?
                && (p.generation != app.world.inventory.generation || !p.before.iter().all(|f| f.current(&app.world.inventory)))
            {
                return Err("stale_plan");
            }
            return Ok(if boolean(a, "confirm", true)? {
                vec![Class::Move]
            } else {
                Vec::new()
            });
        }
        Tool::InventoryExecutePlan => {
            args(c, &["plan_id"])?;
            let p = plan(state, c)?;
            if p.confirm_only {
                return Err("wrong_plan_type");
            }
            if p.executed.is_none()
                && (p.generation != app.world.inventory.generation || !p.before.iter().all(|r| r.current(&app.world.inventory)))
            {
                return Err("stale_plan");
            }
            return Ok(vec![p.class]);
        }
        Tool::InventoryPreviewUndo => {
            args(c, &["operation_id", "ids"])?;
            string(a, "operation_id", 128)?;
            if a.contains_key("ids") {
                ids(a, "ids")?;
            }
            return Ok(Vec::new());
        }
        Tool::AppearanceWearOutfit => {
            args(c, &["folder_id", "folder_name", "append", "skip_elicitation"])?;
            appearance_action(app, c)?;
            return Ok(vec![Class::Read]);
        }
        Tool::AppearanceWearItems | Tool::AppearanceDetach => {
            args(c, &["items_id", "replace", "skip_elicitation"])?;
            ids(a, "items_id")?;
            boolean(a, "replace", false)?;
            boolean(a, "skip_elicitation", false)?;
            if !library::prepare(app, c)? {
                appearance_action(app, c)?;
            }
            return Ok(vec![Class::Read]);
        }
        _ => return Err("unsupported_capability"),
    }
    if matches!(c.tool, Tool::InventoryPreviewBatchMove | Tool::InventoryPreviewBatchRename) {
        build_change(app, state, c)?;
        return Ok(Vec::new());
    }
    let (_, _, _, mut classes) = build_change(app, state, c)?;
    if c.tool == Tool::InventoryCopy && boolean(a, "skip_elicitation", false)? {
        if classes.contains(&Class::NoCopy) {
            classes.push(Class::Read);
        }
        classes.retain(|class| *class != Class::NoCopy);
    }
    Ok(classes)
}
fn copy_policy(a: &serde_json::Map<String, Value>) -> Result<&str, &'static str> {
    let policy = a.get("policy").and_then(Value::as_str).unwrap_or("default");
    if matches!(policy, "default" | "strict" | "copyable_only") {
        Ok(policy)
    } else {
        Err("invalid_args")
    }
}
fn rename_rows(a: &serde_json::Map<String, Value>) -> Result<Vec<(Uuid, String)>, &'static str> {
    let rows = a
        .get("items")
        .and_then(Value::as_array)
        .filter(|rows| !rows.is_empty() && rows.len() <= 50)
        .ok_or("invalid_args")?;
    let mut seen = HashSet::new();
    rows.iter()
        .map(|row| {
            let row = row.as_object().ok_or("invalid_args")?;
            allow_keys(row, &["id", "name"])?;
            let id = uuid(row, "id")?;
            if !seen.insert(id) {
                return Err("invalid_args");
            }
            Ok((id, string(row, "name", 63)?.to_owned()))
        })
        .collect()
}
pub(super) fn plan<'a>(state: &'a State, c: &Capture) -> Result<&'a Plan, &'static str> {
    state
        .plans
        .get(&uuid(&c.arguments, "plan_id")?)
        .filter(|p| {
            p.endpoint == c.endpoint
                && p.policy_revision == c.policy_revision
                && p.client == c.client
                && p.binding == c.binding
                && p.expires > Instant::now()
        })
        .ok_or("stale_plan")
}
fn appearance_action(app: &App, c: &Capture) -> Result<Action, &'static str> {
    native_busy(app)?;
    let inv = &app.world.inventory;
    let a = &c.arguments;
    let action = if c.tool == Tool::AppearanceWearOutfit {
        let folder = match optional_uuid(a, "folder_id")? {
            Some(id) => id,
            None => {
                let name = string(a, "folder_name", 128)?;
                let found = inv
                    .folders
                    .values()
                    .filter(|f| f.info.name == name && !f.library && f.info.type_default == 47)
                    .map(|f| f.info.id)
                    .collect::<Vec<_>>();
                if found.len() != 1 {
                    return Err("ambiguous_outfit");
                }
                found[0]
            }
        };
        ordinary(app, folder)?;
        if inv
            .folders
            .get(&folder)
            .is_some_and(|f| f.items.len() > 128 || f.children.len() > 128)
        {
            return Err("native_appearance_limit");
        }
        tree_scope(inv, folder, 512)?;
        if !appearance::complete(inv, folder) {
            return Err("inventory_loading");
        }
        Action::WearOutfits {
            folders: vec![folder],
            append: boolean(a, "append", false)?,
        }
    } else {
        let targets = ids(a, "items_id")?;
        for id in &targets {
            ordinary(app, *id)?;
            let it = inv.items.get(id).ok_or("not_found")?;
            if rules::library(inv, *id) {
                return Err("copy_library_item_first");
            }
            if !matches!(it.asset_type, 5 | 6 | 13 | 21) {
                return Err("unsupported_item_type");
            }
        }
        if c.tool == Tool::AppearanceDetach {
            Action::RemoveItems(targets)
        } else {
            Action::WearItems {
                items: targets,
                replace: boolean(a, "replace", false)?,
                point: 0,
            }
        }
    };
    if inv
        .system_folder(46)
        .and_then(|id| inv.folders.get(&id))
        .is_some_and(|f| f.items.len() > 128)
    {
        return Err("native_appearance_limit");
    }
    appearance::plan(inv, action.clone(), &app.world.outfit_attachment_points()).map_err(|_| "native_appearance_refused")?;
    Ok(action)
}
pub(super) fn tracked_appearance(
    app: &mut App,
    state: &mut State,
    c: &Capture,
    action: Action,
    complete: bool,
) -> Result<Uuid, &'static str> {
    native_busy(app)?;
    if state.pending.len() >= CAPACITY {
        return Err("operation_capacity");
    }
    appearance::plan(&app.world.inventory, action.clone(), &app.world.outfit_attachment_points())
        .map_err(|_| "native_appearance_refused")?;
    app.apply_appearance_action(action);
    let request = app.appearance_ui.pending.map(|p| p.0).ok_or("native_appearance_refused")?;
    pending_for(state, request, c, PendingKind::Appearance { complete });
    Ok(request)
}
fn merge(change: &mut Mutation, more: Mutation) {
    change.operations.extend(more.operations);
    change.copies.extend(more.copies);
    change.refresh.extend(more.refresh);
    change.removed.extend(more.removed);
}
fn build_change(app: &App, state: &State, c: &Capture) -> Result<(Mutation, Vec<Frozen>, Vec<Expected>, Vec<Class>), &'static str> {
    let inv = &app.world.inventory;
    let a = &c.arguments;
    let facts = crate::ui::inventory::Facts::from_world(&app.world, app.appearance_ui.pending.is_some());
    let protected = &app.settings.inventory.protected;
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut extras = Vec::new();
    let mut change = Mutation::default();
    let mut freeze = |id| -> Result<(), &'static str> {
        ordinary(app, id)?;
        before.push(Frozen::capture(inv, id)?);
        Ok(())
    };
    match c.tool {
        Tool::InventoryCreateFolder => {
            let to = uuid(a, "parent_id")?;
            ordinary(app, to)?;
            if !rules::destination(inv, to) {
                return Err("native_permission_denied");
            }
            change = rules::create_folder(to, string(a, "name", 63)?).map_err(|_| "invalid_name")?;
        }
        Tool::InventoryMove
        | Tool::InventoryBatchMove
        | Tool::InventoryPreviewBatchMove
        | Tool::InventoryTrash
        | Tool::InventoryRestore => {
            let selected = if matches!(c.tool, Tool::InventoryBatchMove | Tool::InventoryPreviewBatchMove) {
                ids(a, "ids")?
            } else {
                vec![uuid(a, "id")?]
            };
            let to = match c.tool {
                Tool::InventoryTrash => inv.system_folder(14).ok_or("folder_unavailable")?,
                Tool::InventoryRestore => {
                    let id = selected[0];
                    if !rules::in_type(inv, id, 14) {
                        return Err("not_in_trash");
                    }
                    rules::default_folder(inv, id)
                        .filter(|id| !rules::in_type(inv, *id, 14))
                        .ok_or("folder_unavailable")?
                }
                _ => uuid(a, "parent_id")?,
            };
            ordinary(app, to)?;
            if c.tool != Tool::InventoryTrash && rules::in_type(inv, to, 14) {
                return Err("use_inventory_trash");
            }
            for id in &selected {
                freeze(*id)?;
                after.push(Expected {
                    id: *id,
                    parent: Some(to),
                    name: None,
                });
            }
            change = rules::move_selection(inv, &selected, to, protected, &facts.worn).map_err(|_| "native_permission_denied")?;
        }
        Tool::InventoryRename | Tool::InventoryBatchRename | Tool::InventoryPreviewBatchRename => {
            let rows = if c.tool == Tool::InventoryRename {
                vec![(uuid(a, "id")?, string(a, "name", 63)?.to_owned())]
            } else {
                rename_rows(a)?
            };
            for (id, name) in rows {
                freeze(id)?;
                if !rules::renameable(inv, id) {
                    return Err("native_permission_denied");
                }
                let name = rules::clean_name(&name).map_err(|_| "invalid_name")?;
                merge(
                    &mut change,
                    rules::patch(inv, id, llsd_map! {"name"=>name.clone()}).map_err(|_| "invalid_name")?,
                );
                after.push(Expected {
                    id,
                    parent: None,
                    name: Some(name),
                });
            }
        }
        Tool::InventoryCopy | Tool::InventoryBatchCopy => {
            let selected = if c.tool == Tool::InventoryCopy {
                vec![uuid(a, "id")?]
            } else {
                ids(a, "ids")?
            };
            let to = uuid(a, "parent_id")?;
            ordinary(app, to)?;
            if !rules::destination(inv, to) {
                return Err("native_permission_denied");
            }
            for id in selected {
                freeze(id)?;
                let scope = tree_scope(inv, id, 512)?;
                for child in scope.iter().filter(|child| **child != id) {
                    ordinary(app, *child)?;
                }
                let policy = copy_policy(a)?;
                let (more, moved) = copy_scope(inv, id, to, policy, protected, &facts.worn)?;
                if !moved.is_empty() {
                    extras.push(Class::NoCopy);
                }
                after.extend(moved.into_iter().map(|(id, parent)| Expected {
                    id,
                    parent: Some(parent),
                    name: None,
                }));
                merge(&mut change, more);
                if change.copies.len() + change.operations.len() > 512 {
                    return Err("request_limit");
                }
            }
            if change.copies.len() + change.operations.len() > 512 {
                return Err("request_limit");
            }
        }
        Tool::InventoryLink => {
            let id = uuid(a, "id")?;
            let to = uuid(a, "parent_id")?;
            freeze(id)?;
            ordinary(app, to)?;
            change = rules::paste(inv, &[id], to, false, true, protected, &facts.worn).map_err(|_| "native_permission_denied")?;
        }
        Tool::InventorySetFavorite => {
            let id = uuid(a, "id")?;
            freeze(id)?;
            change = rules::set_favorites(inv, &[id], boolean(a, "favorite", false)?).map_err(|_| "native_permission_denied")?;
        }
        Tool::InventorySetDescription => {
            let id = uuid(a, "id")?;
            freeze(id)?;
            let it = inv.items.get(&id).ok_or("unsupported_folder_description")?;
            if !rules::mutable(inv, id) || it.owner_mask & rules::MODIFY == 0 || matches!(it.asset_type, 2 | 24 | 25) {
                return Err("native_permission_denied");
            }
            change = rules::patch(inv, id, llsd_map! {"desc"=>string(a,"desc",1023)?}).map_err(|_| "invalid_args")?;
        }
        Tool::InventorySetThumbnail => {
            let id = uuid(a, "id")?;
            freeze(id)?;
            if !rules::mutable(inv, id) {
                return Err("native_permission_denied");
            }
            let image = optional_uuid(a, "thumbnail_id")?.unwrap_or_default();
            if !image.is_nil() && !crate::ui::inventory::thumbnail::image_source_allowed(inv, &image, false, false) {
                return Err("native_permission_denied");
            }
            change = rules::patch(inv, id, aurora_net::inventory::operations::metadata_thumbnail(image))
                .map_err(|_| "native_permission_denied")?;
        }
        Tool::InventoryNextOwnerSet => {
            let id = uuid(a, "id")?;
            freeze(id)?;
            let snapshot = state
                .permissions
                .get(string(a, "expected_revision", 128)?)
                .filter(|s| {
                    s.endpoint == c.endpoint
                        && s.policy_revision == c.policy_revision
                        && s.client == c.client
                        && s.binding == c.binding
                        && s.expires > Instant::now()
                })
                .ok_or("stale_revision")?;
            if inv.items.get(&id) != Some(&snapshot.item) {
                return Err("stale_revision");
            }
            change = rules::next_owner_change(
                inv,
                id,
                app.world.agent_id,
                boolean(a, "copy", false)?,
                boolean(a, "modify", false)?,
                boolean(a, "transfer", false)?,
            )
            .map_err(|_| "native_permission_denied")?;
        }
        _ => return Err("unsupported_capability"),
    }
    if !app.world.rlva_inventory_mutation_allowed(&change, Uuid::nil()) {
        return Err("native_restriction");
    }
    Ok((change, before, after, extras))
}

/// A mixed folder copy uses the shared native item guards. New folder IDs
/// are locally owned plan destinations, never caller-supplied patches.
fn copy_scope(
    inv: &Inventory,
    root: Uuid,
    destination: Uuid,
    policy: &str,
    protected: &HashSet<Uuid>,
    worn: &HashSet<Uuid>,
) -> Result<(Mutation, Vec<(Uuid, Uuid)>), &'static str> {
    use aurora_net::inventory::operations::Operation;
    if !rules::destination(inv, destination) || root == destination || rules::under(inv, destination, root) {
        return Err("native_permission_denied");
    }
    tree_scope(inv, root, 512)?;
    let mut change = Mutation {
        defer_folder_contents: true,
        ..Default::default()
    };
    let mut moved = Vec::new();
    let mut stack = vec![(root, destination)];
    while let Some((id, parent)) = stack.pop() {
        if let Some(folder) = inv.folders.get(&id) {
            let new = Uuid::new_v4();
            change.operations.push(Operation::Folder {
                id: new,
                parent,
                name: folder.info.name.clone(),
                type_default: if folder.info.type_default == 47 { 47 } else { -1 },
            });
            change.refresh.extend([parent, new]);
            stack.extend(folder.children.iter().chain(&folder.items).copied().map(|id| (id, new)));
        } else {
            let item = inv.items.get(&id).ok_or("not_found")?;
            let no_copy = !matches!(item.asset_type, 24 | 25) && item.owner_mask & rules::COPY == 0 && !rules::library(inv, id);
            if no_copy && policy == "strict" {
                return Err("no_copy_item");
            }
            if no_copy && policy == "copyable_only" {
                continue;
            }
            let mut more = if no_copy {
                rules::move_selection(inv, &[id], destination, protected, worn)
            } else {
                rules::paste(inv, &[id], destination, false, false, protected, worn)
            }
            .map_err(|_| "native_permission_denied")?;
            for op in &mut more.operations {
                match op {
                    Operation::Patch { body, .. } if no_copy => {
                        body.insert("parent_id", parent);
                    }
                    Operation::Links { parent: to, .. } => *to = parent,
                    _ => return Err("native_copy_plan_invalid"),
                }
            }
            for copy in &mut more.copies {
                copy.parent = parent;
            }
            more.refresh.push(parent);
            if no_copy {
                moved.push((id, parent));
            }
            merge(&mut change, more);
        }
    }
    Ok((change, moved))
}
pub(super) fn execute(app: &mut App, state: &mut State, c: &Capture) -> Result<Value, &'static str> {
    state.admit()?;
    if c.tool == Tool::InventoryCopy && boolean(&c.arguments, "skip_elicitation", false)? {
        let (change, before, after, classes) = build_change(app, state, c)?;
        if classes.contains(&Class::NoCopy) {
            if state.plans.len() >= PLAN_LIMIT {
                return Err("plan_capacity");
            }
            let proposed=after.iter().map(|a|json!({"item_id":a.id.to_string(),"old_parent_id":rules::parent(&app.world.inventory,a.id).map(|id|id.to_string()),"new_parent_id":a.parent.map(|id|id.to_string())})).collect::<Vec<_>>();
            let id = Uuid::new_v4();
            state.plans.insert(
                id,
                Plan {
                    endpoint: c.endpoint,
                    policy_revision: c.policy_revision,
                    client: c.client,
                    binding: c.binding,
                    expires: Instant::now() + Duration::from_secs(600),
                    class: Class::NoCopy,
                    generation: app.world.inventory.generation,
                    confirm_only: true,
                    before,
                    after,
                    change,
                    executed: None,
                },
            );
            return Ok(
                json!({"status":"confirmation_required","plan_id":id.to_string(),"proposed_moves":proposed,"copyable_items_submitted":false,"destination_created":false,"expires_in_seconds":600}),
            );
        }
    }
    match c.tool {
        Tool::InventoryReplaceLinks => return links::execute(app, state, c),
        Tool::InventoryConfirmCopy => {
            let p = plan(state, c)?.clone();
            if !p.confirm_only {
                return Err("wrong_plan_type");
            }
            if !boolean(&c.arguments, "confirm", true)? {
                state.plans.remove(&uuid(&c.arguments, "plan_id")?);
                return Ok(json!({"status":"declined","confirmed":false,"submitted":false}));
            }
            if let Some(id) = p.executed {
                return Ok(state.outcomes.get(&id).cloned().unwrap_or_else(|| failure("unknown_operation")));
            }
        }
        Tool::InventoryCreateItem => {
            native_busy(app)?;
            let (kind, parent, name) = creation(app, c)?;
            let request = Uuid::new_v4();
            let permissions = app.settings.inventory.creation_permissions.for_item(kind);
            let wearable_data = if let aurora_net::inventory::operations::NewItem::Wearable(subtype) = kind {
                Some(
                    crate::world::inventory::wearable::create_with_permissions(
                        subtype,
                        &name,
                        app.world.agent_id,
                        app.settings.inventory.creation_permissions.wearables.permissions(),
                    )
                    .map_err(|_| "native_wearable_definitions_unavailable")?,
                )
            } else {
                None
            };
            let description = c.arguments.get("desc").and_then(Value::as_str).unwrap_or("").to_owned();
            app.send(NetCommand::CreateAssistantInventoryItem {
                request,
                parent,
                kind,
                name,
                description,
                wearable_data,
                permissions,
            });
            pending_for(
                state,
                request,
                c,
                PendingKind::Created {
                    parent,
                    kind: i32::from(kind.types().0),
                },
            );
            outcome(state, c, c.request_id.clone(), submitted(c));
            return Ok(submitted(c));
        }
        Tool::AppearanceWearOutfit | Tool::AppearanceWearItems | Tool::AppearanceDetach => {
            if library::prepare(app, c)? {
                return library::execute(app, state, c);
            }
            let action = appearance_action(app, c)?;
            tracked_appearance(app, state, c, action, true)?;
            return Ok(submitted(c));
        }
        Tool::InventoryPreviewBatchMove | Tool::InventoryPreviewBatchRename => {
            if state.plans.len() >= PLAN_LIMIT {
                return Err("plan_capacity");
            }
            let (change, before, after, _) = build_change(app, state, c)?;
            let id = Uuid::new_v4();
            let class = if c.tool == Tool::InventoryPreviewBatchMove {
                Class::Move
            } else {
                Class::Edit
            };
            let rows=before.iter().zip(&after).map(|(b,a)|json!({"id":b.id.to_string(),"before":{"name":b.name(),"parent_id":b.parent().to_string()},"after":{"name":a.name,"parent_id":a.parent.map(|id|id.to_string())}})).collect::<Vec<_>>();
            state.plans.insert(
                id,
                Plan {
                    endpoint: c.endpoint,
                    policy_revision: c.policy_revision,
                    client: c.client,
                    binding: c.binding,
                    expires: Instant::now() + Duration::from_secs(600),
                    class,
                    generation: app.world.inventory.generation,
                    confirm_only: false,
                    before,
                    after,
                    change,
                    executed: None,
                },
            );
            return Ok(json!({"plan_id":id.to_string(),"rows":rows,"executed":false,"expires_in_seconds":600}));
        }
        Tool::InventoryPreviewUndo => {
            let original = state
                .history
                .iter()
                .find(|h| {
                    h.id == string(&c.arguments, "operation_id", 128).unwrap_or("")
                        && h.endpoint == c.endpoint
                        && h.policy_revision == c.policy_revision
                        && h.client == c.client
                        && h.binding == c.binding
                        && h.confirmed
                })
                .ok_or("undo_unavailable")?
                .clone();
            if state.plans.len() >= PLAN_LIMIT {
                return Err("plan_capacity");
            }
            let selected = if c.arguments.contains_key("ids") {
                Some(ids(&c.arguments, "ids")?)
            } else {
                None
            };
            let mut before = Vec::new();
            let mut after = Vec::new();
            let mut change = Mutation::default();
            let inv = &app.world.inventory;
            let worn = crate::ui::inventory::Facts::from_world(&app.world, false).worn;
            for (old, expected) in original.before.iter().zip(&original.after) {
                if selected.as_ref().is_some_and(|ids| !ids.contains(&old.id)) {
                    continue;
                }
                if !matches_expected(inv, expected) {
                    return Err("stale_plan");
                }
                before.push(Frozen::capture(inv, old.id)?);
                if original.class == Class::Edit {
                    if !rules::renameable(inv, old.id) {
                        return Err("native_permission_denied");
                    }
                    merge(
                        &mut change,
                        rules::patch(inv, old.id, llsd_map! {"name"=>old.name()}).map_err(|_| "native_permission_denied")?,
                    );
                    after.push(Expected {
                        id: old.id,
                        parent: None,
                        name: Some(old.name().to_owned()),
                    });
                } else {
                    if old.folder.is_some() {
                        return Err("undo_unavailable");
                    }
                    merge(
                        &mut change,
                        rules::move_selection(inv, &[old.id], old.parent(), &app.settings.inventory.protected, &worn)
                            .map_err(|_| "native_permission_denied")?,
                    );
                    after.push(Expected {
                        id: old.id,
                        parent: Some(old.parent()),
                        name: None,
                    });
                }
            }
            if before.is_empty() {
                return Err("undo_unavailable");
            }
            let id = Uuid::new_v4();
            state.plans.insert(
                id,
                Plan {
                    endpoint: c.endpoint,
                    policy_revision: c.policy_revision,
                    client: c.client,
                    binding: c.binding,
                    expires: Instant::now() + Duration::from_secs(600),
                    class: original.class,
                    generation: app.world.inventory.generation,
                    confirm_only: false,
                    before,
                    after,
                    change,
                    executed: None,
                },
            );
            return Ok(json!({"plan_id":id.to_string(),"executed":false,"expires_in_seconds":600}));
        }
        _ => {}
    }
    native_busy(app)?;
    let (change, before, after, history, history_class) = if matches!(c.tool, Tool::InventoryExecutePlan | Tool::InventoryConfirmCopy) {
        let p = plan(state, c)?.clone();
        if let Some(id) = p.executed {
            return Ok(state
                .outcomes
                .get(&id)
                .cloned()
                .unwrap_or_else(|| json!({"status":"submitted","operation_id":id,"retry_safe":false})));
        }
        if p.generation != app.world.inventory.generation || !p.before.iter().all(|b| b.current(&app.world.inventory)) {
            return Err("stale_plan");
        }
        (p.change, p.before, p.after, !p.confirm_only, p.class)
    } else {
        let (m, b, a, _) = build_change(app, state, c)?;
        (
            m,
            b,
            a,
            matches!(
                c.tool,
                Tool::InventoryMove | Tool::InventoryRename | Tool::InventoryBatchMove | Tool::InventoryBatchRename
            ),
            if matches!(c.tool, Tool::InventoryRename | Tool::InventoryBatchRename) {
                Class::Edit
            } else {
                Class::Move
            },
        )
    };
    if change.operations.is_empty() && change.copies.is_empty() {
        return Ok(json!({"status":"complete","changed":false}));
    }
    let request = Uuid::new_v4();
    app.mcp_submit_inventory_change(request, change)
        .map_err(|_| "native_busy_or_restricted")?;
    pending_for(
        state,
        request,
        c,
        PendingKind::Inventory {
            before,
            after,
            history,
            class: history_class,
        },
    );
    outcome(state, c, c.request_id.clone(), submitted(c));
    if matches!(c.tool, Tool::InventoryExecutePlan | Tool::InventoryConfirmCopy)
        && let Some(p) = state.plans.get_mut(&uuid(&c.arguments, "plan_id")?)
    {
        p.executed = Some(c.request_id.clone());
    }
    Ok(submitted(c))
}
fn matches_expected(inv: &Inventory, e: &Expected) -> bool {
    rules::parent(inv, e.id).is_some()
        && e.parent.is_none_or(|p| rules::parent(inv, e.id) == Some(p))
        && e.name.as_ref().is_none_or(|n| rules::name(inv, e.id) == *n)
}

pub(super) fn finish(app: &mut App, pending: Pending, result: Value) {
    let result = if super::super::source_current(app, &pending.capture) {
        result
    } else {
        failure("stale_source_or_authority")
    };
    let mut assistant = match app.character_mcp.take() {
        Some(assistant) => assistant,
        None => return,
    };
    let result = match receipts::record(app, &mut assistant.inventory, &pending.capture, &result) {
        Ok(()) => result,
        Err(code) => failure(code),
    };
    {
        if assistant.inventory.outcomes.len() < CAPACITY || assistant.inventory.outcomes.contains_key(&pending.capture.request_id) {
            outcome(
                &mut assistant.inventory,
                &pending.capture,
                pending.capture.request_id.clone(),
                result.clone(),
            );
        }
        let _ = assistant.runtime.complete(&pending.capture, result);
    }
    app.character_mcp = Some(assistant);
}
pub(super) fn native_event(app: &mut App, event: &NetEvent) -> bool {
    let request = match event {
        NetEvent::InventoryEdited { request, .. }
        | NetEvent::OutfitUpdated { request, .. }
        | NetEvent::AssistantInventoryCreated { request, .. }
        | NetEvent::AssistantInventoryAssetReady { request } => *request,
        _ => return false,
    };
    let Some(pending) = app.character_mcp.as_ref().and_then(|a| a.inventory.pending.get(&request)).cloned() else {
        return false;
    };
    if !matches!(
        pending.kind,
        PendingKind::Inventory { .. } | PendingKind::Appearance { .. } | PendingKind::Created { .. }
    ) {
        return false;
    }
    let source = super::super::source_current(app, &pending.capture);
    let allowed = super::super::continuation_current(app, &pending.capture);
    if !source {
        if matches!(event, NetEvent::AssistantInventoryAssetReady { .. }) {
            app.send(NetCommand::FinishAssistantInventoryItem { request, approved: false });
        }
        if let Some(a) = app.character_mcp.as_mut() {
            a.inventory.pending.remove(&request);
        }
        return true;
    }
    match (event, &pending.kind) {
        (NetEvent::AssistantInventoryAssetReady { .. }, PendingKind::Created { parent, .. }) => {
            let native_allowed = allowed && rules::destination(&app.world.inventory, *parent) && ordinary(app, *parent).is_ok();
            let committed = native_allowed
                && commit_continuation(app, &pending.capture, |app| {
                    app.send(NetCommand::FinishAssistantInventoryItem { request, approved: true });
                })
                .is_ok();
            if !committed {
                app.send(NetCommand::FinishAssistantInventoryItem { request, approved: false });
                if let Some(a) = app.character_mcp.as_mut() {
                    a.inventory.pending.remove(&request);
                }
                finish(app, pending, failure("native_creation_continuation_cancelled"));
            }
        }
        (NetEvent::AssistantInventoryCreated { result, .. }, PendingKind::Created { parent, kind }) => {
            let valid = result
                .as_ref()
                .is_ok_and(|it| it.parent == *parent && it.asset_type == *kind && it.owner == app.world.agent_id);
            if let Ok(item) = result
                && valid
            {
                app.world.inventory.add_items(vec![item.clone()]);
            }
            if let Some(a) = app.character_mcp.as_mut() {
                a.inventory.pending.remove(&request);
            }
            let value = match result {
                Ok(it) if valid => {
                    json!({"status":"confirmed","operation_id":pending.capture.request_id,"inventory_item_id":it.id.to_string(),"confirmed":true,"permissions_confirmed":false,"retry_safe":false})
                }
                _ => {
                    json!({"status":"uncertain","operation_id":pending.capture.request_id,"confirmed":false,"retry_safe":false,"error_code":"native_creation_unconfirmed"})
                }
            };
            finish(app, pending, value);
        }

        (
            NetEvent::InventoryEdited { result, .. },
            PendingKind::Inventory {
                before,
                after,
                history,
                class,
            },
        ) => {
            if let Ok(reply) = result {
                app.world.inventory.remove(&reply.removed);
                app.world.inventory.apply(reply.contents.clone());
                app.world.inventory.created_folders(reply.folders.clone());
                if !reply.copies.is_empty()
                    && allowed
                    && commit_continuation(app, &pending.capture, |app| {
                        app.send(NetCommand::FinishInventoryCopies {
                            request,
                            reply: reply.clone(),
                        })
                    })
                    .is_ok()
                {
                    return true;
                }
            }
            if app.inventory_ui.pending == Some(request) {
                app.inventory_ui.finish_mutation(result.is_ok());
                app.inventory_ui.pending_refresh.clear();
            }
            if let Some(a) = app.character_mcp.as_mut() {
                a.inventory.pending.remove(&request);
            }
            let confirmed = result.as_ref().is_ok_and(|r| !r.moves_queued && r.copies.is_empty())
                && after.iter().all(|e| matches_expected(&app.world.inventory, e));
            let value = json!({"status":if result.is_err(){"uncertain"}else if confirmed{"confirmed"}else{"unconfirmed"},"operation_id":pending.capture.request_id,"confirmed":confirmed,"retry_safe":false,"error_code":result.as_ref().err().map(|_|"native_operation_failed")});
            if *history && let Some(a) = app.character_mcp.as_mut() {
                while a.inventory.history.len() >= CAPACITY {
                    a.inventory.history.pop_front();
                }
                a.inventory.history.push_back(History {
                    endpoint: pending.capture.endpoint,
                    policy_revision: pending.capture.policy_revision,
                    client: pending.capture.client,
                    id: pending.capture.request_id.clone(),
                    binding: pending.capture.binding,
                    before: before.clone(),
                    after: after.clone(),
                    class: *class,
                    confirmed,
                    result: value.clone(),
                });
            }
            finish(app, pending, value);
        }
        (NetEvent::OutfitUpdated { result, .. }, PendingKind::Appearance { complete }) => {
            let owns_pending = app.appearance_ui.pending.is_some_and(|(id, _)| id == request);
            let sync = owns_pending && app.appearance_ui.pending.take().is_some_and(|(_, sync)| sync);
            if let Ok(contents) = result {
                app.world.inventory.apply(contents.clone());
                if sync && allowed {
                    let appearance::CofSync { commands, unknown } = appearance::sync_with_attachments(
                        &app.world.inventory,
                        app.world.agent_id,
                        &app.world.outfit_attachment_items(),
                        &app.appearance_ui.worn_before,
                        app.bridge_runtime.item(),
                        std::mem::take(&mut app.appearance_ui.attachments),
                    );
                    if commands.len() <= 256 {
                        let _ = commit_continuation(app, &pending.capture, |app| {
                            for command in commands {
                                app.send(command);
                            }
                        });
                    }
                    if !unknown.is_empty() {
                        app.appearance_ui.attachments.clear();
                    }
                }
            }
            if owns_pending {
                app.appearance_ui.attachments.clear();
                app.appearance_ui.pending_actor = Uuid::nil();
            }
            if let Some(a) = app.character_mcp.as_mut() {
                a.inventory.pending.remove(&request);
            }
            let value = json!({"status":if result.is_ok(){"submitted"}else{"uncertain"},"operation_id":pending.capture.request_id,"confirmed":false,"retry_safe":false});
            if *complete {
                finish(app, pending, value);
            }
        }
        _ => {}
    }
    true
}

fn creation(app: &App, c: &Capture) -> Result<(aurora_net::inventory::operations::NewItem, Uuid, String), &'static str> {
    use aurora_net::inventory::operations::NewItem;
    let kind = match string(&c.arguments, "type", 32)? {
        "notecard" => NewItem::Note,
        "lsl" | "script" => NewItem::Script,
        "gesture" => NewItem::Gesture,
        "material" => NewItem::Material,
        "landmark" => NewItem::Landmark,
        "sky" => NewItem::Settings(0),
        "water" => NewItem::Settings(1),
        "daycycle" => NewItem::Settings(2),
        name => {
            let names = [
                "shape",
                "skin",
                "hair",
                "eyes",
                "shirt",
                "pants",
                "shoes",
                "socks",
                "jacket",
                "gloves",
                "undershirt",
                "underpants",
                "skirt",
                "alpha",
                "tattoo",
                "physics",
                "universal",
            ];
            NewItem::Wearable(names.iter().position(|n| *n == name).ok_or("unsupported_item_type")? as u8)
        }
    };
    if c.arguments.contains_key("desc") {
        string(&c.arguments, "desc", 1024)?;
    }
    let parent = optional_uuid(&c.arguments, "parent_id")?
        .or_else(|| app.world.inventory.system_folder(i32::from(kind.types().0)))
        .unwrap_or(app.world.inventory.root);
    ordinary(app, parent)?;
    if !rules::destination(&app.world.inventory, parent) {
        return Err("native_permission_denied");
    }
    let name = c.arguments.get("name").and_then(Value::as_str).unwrap_or(match kind {
        NewItem::Note => "New notecard",
        NewItem::Script => "New script",
        NewItem::Gesture => "New gesture",
        NewItem::Material => "New material",
        NewItem::Landmark => "New landmark",
        NewItem::Wearable(_) => "New wearable",
        _ => "New settings",
    });
    let name = rules::clean_name(name).map_err(|_| "invalid_name")?;
    Ok((kind, parent, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_and_oversized_selections_are_rejected() {
        let id = Uuid::new_v4().to_string();
        assert!(ids(json!({"ids":[id.clone(),id]}).as_object().unwrap(), "ids").is_err());
        assert!(ids(json!({"ids":[]}).as_object().unwrap(), "ids").is_err());
    }
    #[test]
    fn plans_do_not_follow_a_reused_client_number_into_another_endpoint() {
        let mut state = State::default();
        let mut c = synthetic_capture(Tool::InventoryExecutePlan);
        let id = Uuid::new_v4();
        c.arguments = json!({"plan_id":id.to_string()}).as_object().unwrap().clone();
        state.plans.insert(
            id,
            Plan {
                endpoint: c.endpoint,
                client: c.client,
                policy_revision: c.policy_revision,
                binding: c.binding,
                expires: Instant::now() + Duration::from_secs(60),
                class: Class::Move,
                generation: 0,
                confirm_only: false,
                before: Vec::new(),
                after: Vec::new(),
                change: Mutation::default(),
                executed: None,
            },
        );
        assert!(plan(&state, &c).is_ok());
        let mut other = c.clone();
        other.client += 1;
        assert!(plan(&state, &other).is_err());
        other = c.clone();
        other.endpoint = HostId::from_bytes([9; 16]).unwrap();
        assert!(plan(&state, &other).is_err());
        other = c.clone();
        other.policy_revision += 1;
        assert!(plan(&state, &other).is_err());
    }
    #[test]
    fn mixed_folder_copy_obeys_no_copy_policy_and_preserves_native_item_rights() {
        use crate::world::inventory::Folder;
        use aurora_net::inventory::InvFolder;
        let agent = Uuid::from_u128(20);
        let root = Uuid::from_u128(21);
        let destination = Uuid::from_u128(22);
        let source = Uuid::from_u128(23);
        let copied = Uuid::from_u128(24);
        let moved = Uuid::from_u128(25);
        let mut inv = Inventory {
            owner: agent,
            root,
            ..Default::default()
        };
        for (id, parent) in [(root, Uuid::nil()), (destination, root), (source, root)] {
            inv.folders.insert(
                id,
                Folder {
                    info: InvFolder {
                        id,
                        parent,
                        type_default: -1,
                        name: id.to_string(),
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: if id == source { vec![copied, moved] } else { Vec::new() },
                    state: FetchState::Fetched,
                    library: false,
                },
            );
        }
        for (id, mask) in [
            (copied, rules::COPY | rules::MODIFY | rules::TRANSFER),
            (moved, rules::MODIFY | rules::TRANSFER),
        ] {
            inv.items.insert(
                id,
                InvItem {
                    id,
                    parent: source,
                    owner: agent,
                    asset_type: 7,
                    owner_mask: mask,
                    base_mask: mask,
                    ..Default::default()
                },
            );
        }
        let original = inv.items.get(&moved).unwrap().clone();
        assert_eq!(
            copy_scope(&inv, source, destination, "strict", &HashSet::new(), &HashSet::new()).err(),
            Some("no_copy_item")
        );
        let (copyable, unsafe_moves) = copy_scope(&inv, source, destination, "copyable_only", &HashSet::new(), &HashSet::new()).unwrap();
        assert_eq!(copyable.copies.len(), 1);
        assert!(unsafe_moves.is_empty());
        let (mixed, unsafe_moves) = copy_scope(&inv, source, destination, "default", &HashSet::new(), &HashSet::new()).unwrap();
        assert_eq!(mixed.copies.len(), 1);
        assert_eq!(unsafe_moves.len(), 1);
        assert_eq!(unsafe_moves[0].0, moved);
        assert_ne!(unsafe_moves[0].1, source);
        assert_eq!(
            inv.items.get(&moved),
            Some(&original),
            "planning never applies a no-copy move or changes creator rights"
        );
    }
    #[test]
    fn policy_and_rename_inputs_are_closed() {
        assert!(copy_policy(json!({"policy":"auto_approve"}).as_object().unwrap()).is_err());
        assert!(
            rename_rows(
                json!({"items":[{"id":Uuid::new_v4().to_string(),"name":"Name","permission":"allow"}]})
                    .as_object()
                    .unwrap()
            )
            .is_err()
        );
    }
}
