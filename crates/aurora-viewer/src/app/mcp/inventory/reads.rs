use super::*;

pub(super) fn supports(t: Tool) -> bool {
    matches!(
        t,
        Tool::InventoryLandmark
            | Tool::InventoryGet
            | Tool::InventoryList
            | Tool::InventorySearch
            | Tool::InventorySystemFolder
            | Tool::InventoryTypes
            | Tool::InventoryGetMany
            | Tool::InventoryResolvePath
            | Tool::InventoryProtectedFolders
            | Tool::InventoryChanges
            | Tool::InventoryPermissionsGet
            | Tool::AppearanceOutfits
            | Tool::AppearanceOutfitItems
            | Tool::AppearanceWorn
            | Tool::InventoryHistory
            | Tool::InventoryContentOperationStatus
            | Tool::ObjectOperationStatus
    )
}
pub(super) fn prepare(app: &mut App, state: &State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    let a = &c.arguments;
    match c.tool {
        Tool::InventoryLandmark => {
            args(c, &["id"])?;
            landmarks::prepare(app, c)?;
            return Ok(vec![Class::WorldRead]);
        }
        Tool::InventoryGet | Tool::InventoryPermissionsGet => {
            args(c, &["id"])?;
            uuid(a, "id")?;
        }
        Tool::InventoryList => {
            args(c, &["folder_id", "limit", "offset"])?;
            uuid(a, "folder_id")?;
            limit(a, 20, 50)?;
            number(a, "offset", 0, 1_000_000)?;
        }
        Tool::InventorySearch => {
            args(
                c,
                &[
                    "query",
                    "name",
                    "desc",
                    "type",
                    "folder_id",
                    "filter_links",
                    "include_trash",
                    "creator_id",
                    "creator_name",
                    "inv_type",
                    "linked_id",
                    "worn",
                    "copyable",
                    "modifiable",
                    "favorite",
                    "created_after",
                    "created_before",
                    "limit",
                ],
            )?;
            for k in ["query", "name", "desc", "creator_name"] {
                if a.contains_key(k) {
                    string(a, k, 256)?;
                }
            }
            for k in ["creator_id", "folder_id", "linked_id"] {
                optional_uuid(a, k)?;
            }
            for k in ["include_trash", "worn", "copyable", "modifiable", "favorite"] {
                boolean(a, k, false)?;
            }
            for k in ["created_after", "created_before"] {
                if a.get(k).is_some_and(|v| v.as_i64().is_none()) {
                    return Err("invalid_args");
                }
            }
            if a.contains_key("type") && asset_type(string(a, "type", 32)?).is_none() {
                return Err("invalid_args");
            }
            if a.contains_key("inv_type") && aurora_net::inventory::inv_type_from_name(string(a, "inv_type", 32)?) < 0 {
                return Err("invalid_args");
            }
            if a.contains_key("filter_links") && !matches!(string(a, "filter_links", 32)?, "INCLUDE_LINKS" | "EXCLUDE_LINKS" | "ONLY_LINKS")
            {
                return Err("invalid_args");
            }
            limit(a, 20, 50)?;
        }
        Tool::InventoryGetMany => {
            args(c, &["ids"])?;
            ids(a, "ids")?;
        }
        Tool::InventoryResolvePath => {
            args(c, &["path", "folder_id"])?;
            let path = string(a, "path", 1024)?;
            if path.split('/').any(|p| p == ".." || p == ".") {
                return Err("invalid_args");
            }
            optional_uuid(a, "folder_id")?;
        }
        Tool::InventorySystemFolder => {
            args(c, &["type", "ft_name"])?;
            folder_type(a.get("type").or(a.get("ft_name")).and_then(Value::as_str).ok_or("invalid_args")?).ok_or("invalid_args")?;
        }
        Tool::InventoryTypes | Tool::InventoryProtectedFolders | Tool::AppearanceWorn | Tool::AppearanceOutfits => args(c, &[])?,
        Tool::AppearanceOutfitItems => {
            args(c, &["outfit_id"])?;
            uuid(a, "outfit_id")?;
        }
        Tool::InventoryChanges => {
            args(c, &["since"])?;
            number(a, "since", 0, usize::MAX)?;
        }
        Tool::InventoryHistory => {
            args(c, &["operation_id", "limit", "offset"])?;
            if a.contains_key("operation_id") {
                string(a, "operation_id", 128)?;
            }
            limit(a, 20, 50)?;
            number(a, "offset", 0, 128)?;
        }
        Tool::InventoryContentOperationStatus | Tool::ObjectOperationStatus => {
            args(c, &["operation_id"])?;
            return status_classes(state, c, |original| {
                if !receipts::validate(app, state, original) {
                    return Err("native_result_access_revoked");
                }
                if c.tool == Tool::ObjectOperationStatus && interactions::supports(original.tool) {
                    Ok(())
                } else if c.tool == Tool::ObjectOperationStatus {
                    objects::status_gate(app, original)
                } else {
                    content::status_gate(app, original)
                }
            });
        }
        _ => return Err("unsupported_capability"),
    }
    if c.tool == Tool::InventoryPermissionsGet {
        let id = uuid(a, "id")?;
        ordinary(app, id)?;
        let it = app.world.inventory.items.get(&id).ok_or("not_found")?;
        if it.owner != app.world.agent_id || !rules::mutable(&app.world.inventory, id) || rules::in_type(&app.world.inventory, id, 14) {
            return Err("native_permission_denied");
        }
    }
    Ok(Vec::new())
}

fn status_classes(
    state: &State,
    c: &Capture,
    native_gate: impl FnOnce(&Capture) -> Result<(), &'static str>,
) -> Result<Vec<Class>, &'static str> {
    let id = string(&c.arguments, "operation_id", 128)?;
    let original = state
        .outcome_captures
        .get(id)
        .filter(|o| o.endpoint == c.endpoint && o.policy_revision == c.policy_revision && o.client == c.client && o.binding == c.binding)
        .ok_or("unknown_operation")?;
    let object_domain = matches!(
        original.tool,
        Tool::ObjectScriptGet
            | Tool::ObjectScriptPreview
            | Tool::ObjectScriptSaveCompile
            | Tool::ObjectScriptStatus
            | Tool::ObjectScriptSetRunning
            | Tool::ObjectScriptReset
            | Tool::ObjectContentsList
            | Tool::ObjectContentsPut
            | Tool::ObjectContentsRename
            | Tool::ObjectContentsRemove
            | Tool::ObjectTake
            | Tool::ObjectTakeCopy
            | Tool::ObjectTouch
            | Tool::ObjectDialogReply
            | Tool::ObjectChannelSend
    );
    let agent_domain = matches!(
        original.tool,
        Tool::InventoryReadScript
            | Tool::InventoryReadNotecard
            | Tool::InventoryContentGet
            | Tool::InventoryContentPreview
            | Tool::InventoryUpdateNotecard
            | Tool::InventorySaveCompileScript
    );
    if object_domain != (c.tool == Tool::ObjectOperationStatus) || c.tool == Tool::InventoryContentOperationStatus && !agent_domain {
        return Err("wrong_operation_domain");
    }
    native_gate(original)?;
    let mut classes = original.required_classes.clone();
    classes.push(Class::Read);
    if object_domain {
        classes.extend([Class::TaskInventoryRead, Class::WorldRead]);
    }
    Ok(classes)
}
pub(super) fn execute(app: &mut App, state: &mut State, c: &Capture) -> Result<Value, &'static str> {
    let a = &c.arguments;
    let inv = &app.world.inventory;
    match c.tool {
        Tool::InventoryLandmark => landmarks::execute(app, state, c),
        Tool::InventoryTypes => Ok(
            json!({"asset_types":["texture","sound","callcard","landmark","clothing","object","notecard","lsltext","bodypart","snapshot","animation","gesture","link","link_f","mesh","settings","material"],"folder_types":["Textures","Sounds","Calling Cards","Landmarks","Clothing","Objects","Notecards","Scripts","Body Parts","Trash","Animations","Gestures","Current Outfit","Outfit","My Outfits","Received Items"]}),
        ),
        Tool::InventoryGet => record(app, uuid(a, "id")?),
        Tool::InventoryGetMany => Ok(
            json!({"items":ids(a,"ids")?.into_iter().map(|id|record(app,id).unwrap_or_else(|code|json!({"id":id.to_string(),"error_code":code}))).collect::<Vec<_>>(),"generation":inv.generation}),
        ),
        Tool::InventoryList => {
            let id = uuid(a, "folder_id")?;
            let f = inv.folders.get(&id).ok_or("not_found")?;
            if f.state != FetchState::Fetched {
                return Err("native_scope_changed");
            }
            let offset = number(a, "offset", 0, 1_000_000)?;
            let max = limit(a, 20, 50)?;
            let total = f.children.len() + f.items.len();
            let rows = f
                .children
                .iter()
                .chain(&f.items)
                .copied()
                .skip(offset)
                .take(max)
                .filter_map(|id| record(app, id).ok())
                .collect::<Vec<_>>();
            Ok(json!({"folder_id":id.to_string(),"items":rows,"total":total,"offset":offset,"complete":true,"generation":inv.generation}))
        }
        Tool::InventorySearch => search(app, a),
        Tool::InventorySystemFolder => {
            let kind =
                folder_type(a.get("type").or(a.get("ft_name")).and_then(Value::as_str).ok_or("invalid_args")?).ok_or("invalid_args")?;
            let id = inv.system_folder(kind).ok_or("folder_unavailable")?;
            Ok(json!({"folder_id":id.to_string(),"type":kind}))
        }
        Tool::InventoryResolvePath => {
            let root = optional_uuid(a, "folder_id")?.unwrap_or(inv.root);
            if !inv.folders.contains_key(&root) {
                return Err("not_found");
            }
            let mut found = vec![root];
            let components = string(a, "path", 1024)?.split('/').filter(|s| !s.is_empty()).collect::<Vec<_>>();
            if components.len() > 32 {
                return Err("invalid_args");
            }
            for part in components {
                let mut next = Vec::new();
                for parent in &found {
                    let f = inv.folders.get(parent).ok_or("inventory_loading")?;
                    if f.state != FetchState::Fetched {
                        return Err("inventory_loading");
                    }
                    for id in f.children.iter().chain(&f.items) {
                        if rules::name(inv, *id) == part {
                            next.push(*id);
                            if next.len() > ROW_LIMIT {
                                return Err("response_limit");
                            }
                        }
                    }
                }
                found = next;
            }
            Ok(
                json!({"matches":found.iter().filter_map(|id|record(app,*id).ok()).collect::<Vec<_>>(),"ambiguous":found.len()>1,"complete":true}),
            )
        }
        Tool::InventoryProtectedFolders => {
            let mut protected = app
                .settings
                .inventory
                .protected
                .iter()
                .filter(|id| inv.folders.contains_key(id))
                .copied()
                .collect::<Vec<_>>();
            protected.sort_unstable();
            let total = protected.len();
            Ok(
                json!({"folders":protected.into_iter().take(50).filter_map(|id|record(app,id).ok()).collect::<Vec<_>>(),"truncated":total>50}),
            )
        }
        Tool::InventoryChanges => {
            let since = number(a, "since", 0, usize::MAX)? as u64;
            let (changes, gap) = inv.changes_since(since);
            let rows = changes.into_iter().map(|(generation,id)| json!({"generation":generation,"id":id.to_string(),"removed":!inv.items.contains_key(&id)&&!inv.folders.contains_key(&id)})).collect::<Vec<_>>();
            Ok(json!({"generation":inv.generation,"changes":rows,"gap":gap,"resync_required":gap}))
        }
        Tool::InventoryPermissionsGet => {
            let id = uuid(a, "id")?;
            let item = inv.items.get(&id).ok_or("not_found")?.clone();
            let revision = Uuid::new_v4().to_string();
            if state.permissions.len() >= CAPACITY {
                return Err("snapshot_capacity");
            }
            state.permissions.insert(
                revision.clone(),
                PermissionSnapshot {
                    endpoint: c.endpoint,
                    policy_revision: c.policy_revision,
                    client: c.client,
                    binding: c.binding,
                    item: item.clone(),
                    expires: Instant::now() + Duration::from_secs(120),
                },
            );
            Ok(
                json!({"id":id.to_string(),"expected_revision":revision,"permissions":{"base":item.base_mask,"owner":item.owner_mask,"group":item.group_mask,"everyone":item.everyone_mask,"next_owner":item.next_owner_mask},"editable":rules::next_owner_editable(&item)}),
            )
        }
        Tool::AppearanceOutfits => {
            let mut outfits = inv
                .folders
                .values()
                .filter(|f| f.info.type_default == 47 && !f.library)
                .map(|f| f.info.id)
                .collect::<Vec<_>>();
            outfits.sort_unstable();
            let total = outfits.len();
            Ok(
                json!({"outfits":outfits.into_iter().take(50).filter_map(|id|record(app,id).ok()).collect::<Vec<_>>(),"truncated":total>50,"outfit_dirty":crate::world::appearance::dirty(inv)}),
            )
        }
        Tool::AppearanceOutfitItems => {
            let id = uuid(a, "outfit_id")?;
            if !crate::world::appearance::complete(inv, id) {
                return Err("native_scope_changed");
            }
            let links = crate::world::appearance::folder_links(inv, id);
            let total = links.len();
            Ok(
                json!({"items":links.iter().take(50).filter_map(|l|record(app,l.target).ok()).collect::<Vec<_>>(),"complete":true,"truncated":total>50}),
            )
        }
        Tool::AppearanceWorn => {
            let facts = crate::ui::inventory::Facts::from_world(&app.world, app.appearance_ui.pending.is_some());
            let mut worn = facts.worn.iter().copied().collect::<Vec<_>>();
            worn.sort_unstable();
            let total = worn.len();
            Ok(
                json!({"items":worn.into_iter().take(50).filter_map(|id|record(app,id).ok()).collect::<Vec<_>>(),"truncated":total>50,"outfit_dirty":crate::world::appearance::dirty(inv),"pending":app.appearance_ui.pending.is_some()}),
            )
        }
        Tool::InventoryHistory => {
            if let Some(id) = a.get("operation_id").and_then(Value::as_str) {
                return state
                    .history
                    .iter()
                    .find(|h| {
                        h.endpoint == c.endpoint
                            && h.policy_revision == c.policy_revision
                            && h.client == c.client
                            && h.binding == c.binding
                            && h.id == id
                    })
                    .map(|h| h.result.clone())
                    .ok_or("unknown_operation");
            }
            Ok(
                json!({"operations":state.history.iter().rev().filter(|h|h.endpoint==c.endpoint && h.policy_revision==c.policy_revision && h.client==c.client && h.binding==c.binding).skip(number(a,"offset",0,128)?).take(limit(a,20,50)?).map(|h|h.result.clone()).collect::<Vec<_>>()}),
            )
        }
        Tool::InventoryContentOperationStatus | Tool::ObjectOperationStatus => state
            .outcomes
            .get(string(a, "operation_id", 128)?)
            .cloned()
            .ok_or("unknown_operation"),
        _ => Err("unsupported_capability"),
    }
}

pub(super) fn record(app: &App, id: Uuid) -> Result<Value, &'static str> {
    let inv = &app.world.inventory;
    if let Some(f) = inv.folders.get(&id) {
        return Ok(
            json!({"id":id.to_string(),"name":clipped(&f.info.name,128),"parent_id":f.info.parent.to_string(),"is_folder":true,"preferred_type":f.info.type_default,"version":f.info.version,"complete":f.state==FetchState::Fetched,"descendant_count":f.children.len()+f.items.len(),"favorite":f.info.favorite,"library":f.library,"path":path(inv,id)}),
        );
    }
    let it = inv.items.get(&id).ok_or("not_found")?;
    let mut out = json!({"id":id.to_string(),"name":clipped(&it.name,128),"parent_id":it.parent.to_string(),"description":clipped(&it.desc,512),"asset_type":it.asset_type,"inventory_type":it.inv_type,"created_at":it.created_at,"creator_id":it.creator.to_string(),"owner_id":it.owner.to_string(),"group_id":it.group_id.to_string(),"group_owned":it.group_owned,"copyable":it.owner_mask&rules::COPY!=0,"modifiable":it.owner_mask&rules::MODIFY!=0,"transferable":it.owner_mask&rules::TRANSFER!=0,"favorite":it.favorite,"is_link":matches!(it.asset_type,24|25),"flags":it.flags,"permissions":{"base":it.base_mask,"owner":it.owner_mask,"group":it.group_mask,"everyone":it.everyone_mask,"next_owner":it.next_owner_mask},"sale_info":{"type":it.sale_type,"price":it.sale_price},"path":path(inv,id)});
    if matches!(it.asset_type, 24 | 25) {
        out["linked_id"] = json!(it.asset_id.to_string());
    } else if rules::asset_uuid_shown(inv, it.asset_id) && inv.can_preview(it.asset_type) {
        out["asset_id"] = json!(it.asset_id.to_string());
    }
    if !it.thumbnail.is_nil() && rules::asset_uuid_shown(inv, it.thumbnail) {
        out["thumbnail_id"] = json!(it.thumbnail.to_string());
    }
    Ok(out)
}
fn path(inv: &Inventory, id: Uuid) -> String {
    let mut names = rules::ancestors(inv, id)
        .take(33)
        .map(|id| clipped(&rules::name(inv, id), 128))
        .collect::<Vec<_>>();
    names.reverse();
    names.join("/")
}
fn asset_type(name: &str) -> Option<i32> {
    let n = match name.to_ascii_lowercase().as_str() {
        "lsl" | "script" => 10,
        "animation" => 20,
        "body_part" => 13,
        n => aurora_net::inventory::asset_type_from_name(n),
    };
    (n >= 0).then_some(n)
}
fn folder_type(name: &str) -> Option<i32> {
    Some(match name.to_ascii_lowercase().replace([' ', '_', '-'], "").as_str() {
        "textures" | "texture" => 0,
        "sounds" | "sound" => 1,
        "callingcards" => 2,
        "landmarks" | "landmark" => 3,
        "clothing" => 5,
        "objects" | "object" => 6,
        "notecards" | "notecard" => 7,
        "scripts" | "script" => 10,
        "bodyparts" => 13,
        "trash" => 14,
        "lostandfound" => 16,
        "animations" => 20,
        "gestures" => 21,
        "favorites" => 23,
        "currentoutfit" => 46,
        "outfit" => 47,
        "myoutfits" => 48,
        "receiveditems" => 50,
        "settings" => 56,
        "materials" => 57,
        _ => return None,
    })
}
fn search(app: &App, a: &serde_json::Map<String, Value>) -> Result<Value, &'static str> {
    let inv = &app.world.inventory;
    let query = a
        .get("query")
        .or(a.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    let desc = a.get("desc").and_then(Value::as_str).unwrap_or("").to_lowercase();
    let folder = optional_uuid(a, "folder_id")?;
    let creator = optional_uuid(a, "creator_id")?;
    let linked = optional_uuid(a, "linked_id")?;
    let kind = a.get("type").and_then(Value::as_str).and_then(asset_type);
    let inv_type = a
        .get("inv_type")
        .and_then(Value::as_str)
        .map(aurora_net::inventory::inv_type_from_name);
    let links = a.get("filter_links").and_then(Value::as_str).unwrap_or("INCLUDE_LINKS");
    let creator_name = a.get("creator_name").and_then(Value::as_str).unwrap_or("").to_lowercase();
    let worn = crate::ui::inventory::Facts::from_world(&app.world, app.appearance_ui.pending.is_some()).worn;
    let trash = inv.system_folder(14);
    let include_trash = boolean(a, "include_trash", false)?;
    let max = limit(a, 20, 50)?;
    let mut matched = std::collections::BTreeSet::new();
    let mut total = 0usize;
    for id in inv
        .items
        .values()
        .filter(|it| {
            it.name.to_lowercase().contains(&query)
                && it.desc.to_lowercase().contains(&desc)
                && folder.is_none_or(|f| rules::under(inv, it.id, f))
                && creator.is_none_or(|id| it.creator == id)
                && linked.is_none_or(|id| matches!(it.asset_type, 24 | 25) && it.asset_id == id)
                && kind.is_none_or(|k| it.asset_type == k)
                && inv_type.is_none_or(|k| it.inv_type == k)
                && match links {
                    "EXCLUDE_LINKS" => !matches!(it.asset_type, 24 | 25),
                    "ONLY_LINKS" => matches!(it.asset_type, 24 | 25),
                    _ => true,
                }
                && (include_trash || trash.is_none_or(|t| !rules::under(inv, it.id, t)))
                && (creator_name.is_empty()
                    || app
                        .world
                        .social
                        .names
                        .get(&it.creator)
                        .is_some_and(|n| n.to_lowercase().contains(&creator_name)))
                && [
                    ("worn", worn.contains(&it.id)),
                    ("copyable", it.owner_mask & rules::COPY != 0),
                    ("modifiable", it.owner_mask & rules::MODIFY != 0),
                    ("favorite", rules::is_favorite(inv, it.id)),
                ]
                .iter()
                .all(|(key, actual)| a.get(*key).and_then(Value::as_bool).is_none_or(|wanted| wanted == *actual))
                && a.get("created_after").and_then(Value::as_i64).is_none_or(|n| it.created_at >= n)
                && a.get("created_before").and_then(Value::as_i64).is_none_or(|n| it.created_at <= n)
        })
        .map(|it| it.id)
    {
        total = total.saturating_add(1);
        matched.insert(id);
        if matched.len() > max {
            matched.pop_last();
        }
    }
    Ok(
        json!({"items":matched.into_iter().take(max).filter_map(|id|record(app,id).ok()).collect::<Vec<_>>(),"total":total,"truncated":total>max,"complete":inv.folders.values().all(|f|f.library||f.state==FetchState::Fetched),"generation":inv.generation}),
    )
}

#[cfg(test)]
mod status_tests {
    use super::*;
    fn cached(original_tool: Tool, status_tool: Tool) -> (State, Capture) {
        let mut state = State::default();
        let mut original = synthetic_capture(original_tool);
        original.required_classes = vec![Class::TaskInventoryRead, Class::WorldRead];
        outcome(
            &mut state,
            &original,
            "private-result".into(),
            json!({"text":"synthetic task source"}),
        );
        let mut request = synthetic_capture(status_tool);
        request.arguments = json!({"operation_id":"private-result"}).as_object().unwrap().clone();
        (state, request)
    }
    #[test]
    fn typed_status_cannot_cross_content_authority_domains() {
        let (state, request) = cached(Tool::ObjectScriptGet, Tool::InventoryContentOperationStatus);
        assert_eq!(status_classes(&state, &request, |_| Ok(())).unwrap_err(), "wrong_operation_domain");
        let (state, request) = cached(Tool::InventoryReadScript, Tool::ObjectOperationStatus);
        assert_eq!(status_classes(&state, &request, |_| Ok(())).unwrap_err(), "wrong_operation_domain");
    }
    #[test]
    fn cached_task_source_requires_original_classes_and_current_native_permission() {
        let (state, request) = cached(Tool::ObjectScriptGet, Tool::ObjectOperationStatus);
        let classes = status_classes(&state, &request, |_| Ok(())).unwrap();
        assert!(classes.contains(&Class::TaskInventoryRead));
        assert!(classes.contains(&Class::WorldRead));
        assert!(classes.contains(&Class::Read));
        assert_eq!(
            status_classes(&state, &request, |_| Err("native_permission_denied")).unwrap_err(),
            "native_permission_denied"
        );
        assert_eq!(state.outcomes["private-result"]["text"], "synthetic task source");
        let mut other = request.clone();
        other.client += 1;
        assert_eq!(status_classes(&state, &other, |_| Ok(())).unwrap_err(), "unknown_operation");
        other = request.clone();
        other.endpoint = HostId::from_bytes([8; 16]).unwrap();
        assert_eq!(status_classes(&state, &other, |_| Ok(())).unwrap_err(), "unknown_operation");
        other = request.clone();
        other.policy_revision += 1;
        assert_eq!(status_classes(&state, &other, |_| Ok(())).unwrap_err(), "unknown_operation");
    }
}
