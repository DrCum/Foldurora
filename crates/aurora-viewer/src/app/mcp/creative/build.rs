//! FSBuildListener (Folderstorm abbe3cb0), LGPL-2.1-or-later.
//! Deliberately separate from the user's native Build selection.
use super::*;
use crate::world::inventory::actions as rules;
use aurora_net::{
    NetCommand,
    build::{BuildCmd, NewPrim, TransformUpdate},
};

const MAX_SNAPSHOTS: usize = 128;
#[derive(Default)]
pub(super) struct State {
    revisions: BTreeMap<Uuid, Revision>,
    rez: BTreeMap<(u64, Uuid), Rez>,
    retired_rez: BTreeSet<(u64, Uuid)>,
    io: Option<(crossbeam_channel::Sender<MaterialReply>, crossbeam_channel::Receiver<MaterialReply>)>,
    materials: BTreeMap<String, Instant>,
}
struct Revision {
    capture: Capture,
    region: u64,
    value: Value,
    expires: Instant,
    disclosed: Vec<Uuid>,
}
struct Rez {
    operation: Uuid,
    capture: Capture,
    region: u64,
    parent: Option<Uuid>,
    before: BTreeSet<Uuid>,
    position: Vec3,
    at: Instant,
    result: Value,
}
struct MaterialReply {
    capture: Capture,
    object: Uuid,
    face: u8,
    asset: Uuid,
    accepted: bool,
}
pub(super) fn supports(t: Tool) -> bool {
    matches!(
        t,
        Tool::BuildObjectGet
            | Tool::BuildObjectTransform
            | Tool::BuildObjectSurface
            | Tool::BuildObjectMaterial
            | Tool::BuildRezPrimitive
            | Tool::BuildRezInventory
            | Tool::ObjectRezIdentify
    )
}
pub(super) fn owns_rez(s: &State, c: &Capture) -> bool {
    uuid(&c.arguments, "operation_id")
        .ok()
        .is_some_and(|id| s.rez.values().any(|r| r.operation == id && same_source(&r.capture, c)))
}
pub(super) fn original_receipt(s: &State, c: &Capture) -> Result<Option<Capture>, &'static str> {
    if c.tool != Tool::ObjectOperationStatus {
        return Ok(None);
    }
    Ok(Some(tracked(s, c, uuid(&c.arguments, "operation_id")?)?.capture.clone()))
}
fn tracked<'a>(s: &'a State, c: &Capture, id: Uuid) -> Result<&'a Rez, &'static str> {
    s.rez
        .values()
        .find(|r| r.operation == id && same_source(&r.capture, c))
        .ok_or("unknown_operation")
}
fn rez_gate(app: &App, r: &Rez) -> Result<(), &'static str> {
    if Some(r.region) != app.world.main_region
        || app.world.rlva.has("showloc")
        || app.world.rlva.has("shownames")
        || app.world.rlva.has("interact")
    {
        return Err("native_rez_receipt_access_revoked");
    }
    if r.capture.tool == Tool::BuildRezInventory {
        item(app, uuid(&r.capture.arguments, "id")?, 6, false)?;
    }
    Ok(())
}
fn candidate(app: &App, r: &Rez, id: Uuid) -> bool {
    object(app, id, false)
        .ok()
        .and_then(|idx| app.world.objects.get(idx))
        .is_some_and(|o| {
            o.update_flags & crate::ui::context::flags::OBJECT_YOU_OWNER != 0
                && !r.before.contains(&id)
                && o.position.distance_squared(r.position) <= 64.
                && app
                    .world
                    .region_offset(o.key.region)
                    .is_some_and(|off| (o.position + off).distance(app.world.agent.position) <= 64.)
        })
}
pub(super) fn rez_receipt(app: &App, s: &State, c: &Capture, operation: Uuid) -> Result<Value, &'static str> {
    let r = tracked(s, c, operation)?;
    rez_gate(app, r)?;
    let original = if let Some(parent) = r.parent { tracked(s, c, parent)? } else { r };
    rez_gate(app, original)?;
    let mut result = r.result.clone();
    result["candidates"] = json!(
        app.world
            .objects
            .iter()
            .filter(|(_, o)| candidate(app, original, o.full_id))
            .take(64)
            .map(|(_, o)| { json!({"object_id":o.full_id,"position":o.position.to_array(),"association_confirmed":false}) })
            .collect::<Vec<_>>()
    );
    Ok(result)
}
fn identify_gate(app: &App, s: &State, c: &Capture) -> Result<(), &'static str> {
    args(c, &["object_id", "operation_id", "client_request_id"])?;
    let key = uuid(&c.arguments, "client_request_id")?;
    let id = uuid(&c.arguments, "object_id")?;
    let original = tracked(s, c, uuid(&c.arguments, "operation_id")?)?;
    rez_gate(app, original)?;
    if original.parent.is_some() {
        return Err("tracked_rez_operation_required");
    }
    if let Some(old) = s.rez.get(&(c.client, key)) {
        if same_source(&old.capture, c) && old.capture.arguments == c.arguments {
            return Ok(());
        }
        return Err("client_request_conflict");
    }
    if s.retired_rez.contains(&(c.client, key)) || s.retired_rez.len() >= 4096 || s.rez.len() >= 128 {
        return Err("rez_receipt_capacity_or_retired_request");
    }
    if original.result["correlation"] == "explicit_selection" {
        if original.result["object_id"].as_str() != Some(id.to_string().as_str()) {
            return Err("rez_association_immutable");
        }
    } else if !candidate(app, original, id) {
        return Err("not_a_current_rez_candidate");
    }
    Ok(())
}
pub(super) fn rez_identify(app: &App, s: &mut State, c: &Capture) -> Result<Value, &'static str> {
    identify_gate(app, s, c)?;
    let key = uuid(&c.arguments, "client_request_id")?;
    if let Some(old) = s.rez.get(&(c.client, key)) {
        return Ok(old.result.clone());
    }
    let original_id = uuid(&c.arguments, "operation_id")?;
    let object_id = uuid(&c.arguments, "object_id")?;
    let original = s
        .rez
        .values_mut()
        .find(|r| r.operation == original_id && same_source(&r.capture, c))
        .ok_or("unknown_operation")?;
    original.result["state"] = json!("selected_candidate");
    original.result["correlation"] = json!("explicit_selection");
    original.result["object_id"] = json!(object_id);
    original.result["object_id_confirmed"] = json!(false);
    let operation = Uuid::new_v4();
    let mut result = original.result.clone();
    result["status"] = json!("selected_candidate");
    result["operation_id"] = json!(operation);
    result["associated_rez_operation_id"] = json!(original_id);
    result["client_request_id"] = json!(key);
    let r = Rez {
        operation,
        capture: c.clone(),
        region: original.region,
        parent: Some(original_id),
        before: BTreeSet::new(),
        position: original.position,
        at: Instant::now(),
        result: result.clone(),
    };
    s.retired_rez.insert((c.client, key));
    s.rez.insert((c.client, key), r);
    Ok(result)
}
pub(super) fn validate_receipt(app: &App, s: &State, c: &Capture) -> bool {
    if matches!(c.tool, Tool::ObjectOperationStatus | Tool::ObjectRezIdentify) {
        return uuid(&c.arguments, "operation_id").is_ok_and(|id| rez_receipt(app, s, c, id).is_ok());
    }
    if app.world.rlva.has("showloc") || app.world.rlva.has("interact") {
        return false;
    }
    if c.tool != Tool::BuildObjectGet {
        return true;
    }
    let ids = crate::world::inventory::asset_ids::AssetIds::diagnostics(&app.world);
    s.revisions
        .values()
        .find(|r| r.capture.request_id == c.request_id && same_source(&r.capture, c) && Some(r.region) == app.world.main_region)
        .is_some_and(|r| {
            r.disclosed.iter().all(|id| ids.shown(id)) && uuid(&c.arguments, "id").ok().is_some_and(|id| object(app, id, false).is_ok())
        })
}
fn object(app: &App, id: Uuid, edit: bool) -> Result<usize, &'static str> {
    let idx = app.world.objects.index_of_uuid(&id).ok_or("object_unavailable")?;
    let o = app.world.objects.get(idx).ok_or("object_unavailable")?;
    if o.pcode != aurora_prim::params::LL_PCODE_VOLUME
        || o.parent_id != 0
        || Some(o.key.region) != app.world.main_region
        || o.is_client_preview()
        || app.build.num_faces(&app.world, idx) > 64
        || crate::build::family(&app.world, idx).len() > 256
    {
        return Err("unattached_current_region_root_required");
    }
    if app.world.rlva.has("showloc") || app.world.rlva.has("interact") {
        return Err("native_world_restriction");
    }
    use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_MOVE, OBJECT_YOU_OWNER};
    if edit
        && (o.update_flags & (OBJECT_YOU_OWNER | OBJECT_MODIFY | OBJECT_MOVE) != OBJECT_YOU_OWNER | OBJECT_MODIFY | OBJECT_MOVE
            || o.update_flags & crate::build::edits::FLAGS_USE_PHYSICS != 0
            || !app.world.rlva_can_edit(idx)
            || !app.world.rlva_can_touch(o.key.region, o.key.local_id))
    {
        return Err("native_build_permission_denied");
    }
    Ok(idx)
}
fn item(app: &App, id: Uuid, kind: i32, full: bool) -> Result<&aurora_net::inventory::InvItem, &'static str> {
    let inv = &app.world.inventory;
    let i = inv
        .items
        .get(&id)
        .filter(|i| {
            i.owner == app.world.agent_id
                && !i.group_owned
                && i.asset_type == kind
                && !i.asset_id.is_nil()
                && rules::under(inv, id, inv.root)
                && !rules::in_type(inv, id, 14)
                && !rules::library(inv, id)
        })
        .ok_or("owned_inventory_item_required")?;
    if (full && i.owner_mask & rules::FULL != rules::FULL) || (!full && i.owner_mask & rules::COPY == 0) {
        return Err("inventory_permissions_required");
    }
    Ok(i)
}
fn position(app: &App, a: &serde_json::Map<String, Value>) -> Result<Vec3, &'static str> {
    let p = Vec3::from_array(vecn(a, "position", -1e9, 1e9)?);
    let r = app.world.main().ok_or("native_not_ready")?;
    let off = app.world.region_offset(r.handle).ok_or("native_not_ready")?;
    let limit = app.world.rlva.distance_limit("fartouch", 1.5, Uuid::nil()).min(64.);
    if p.x < 0.
        || p.y < 0.
        || p.x >= r.heightmap.size_x as f32
        || p.y >= r.heightmap.size_y as f32
        || !(-256.0..=4096.0).contains(&p.z)
        || (p + off).distance(app.world.agent.position) > limit
    {
        return Err("build_destination_out_of_bounds");
    }
    Ok(p)
}
fn state(app: &App, idx: usize, protected: bool) -> Value {
    let Some(o) = app.world.objects.get(idx) else { return Value::Null };
    let ids = crate::world::inventory::asset_ids::AssetIds::diagnostics(&app.world);
    let asset = |id: Uuid| if protected { ids.id(id).to_string() } else { id.to_string() };
    let faces: Vec<_> = o.te.iter().flat_map(|te| te.faces.iter().take(app.build.num_faces(&app.world, idx))).enumerate().map(|(f,te)| {
        json!({"face":f,"texture_id":asset(te.texture),"material_id":asset(crate::build::BuildTool::gltf_asset(&app.world,idx,f as u8).unwrap_or_default()),
            "color":te.color,"glow":te.glow,"fullbright":te.fullbright(),"uv_scale":[te.scale_s,te.scale_t],"uv_offset":[te.offset_s,te.offset_t],"uv_rotation":te.rotation})
    }).collect();
    let children: Vec<_> = crate::build::family(&app.world, idx).into_iter().filter(|i| *i != idx).filter_map(|i|app.world.objects.get(i)).map(|o|
        json!({"id":o.full_id,"local_id":o.key.local_id,"position":o.position.to_array(),"rotation":o.rotation.to_array(),"scale":o.scale.to_array(),"flags":o.update_flags})).collect();
    json!({"id":o.full_id,"local_id":o.key.local_id,"region":o.key.region,"position":o.position.to_array(),"rotation":o.rotation.to_array(),"scale":o.scale.to_array(),
        "owner":o.owner_id,"flags":o.update_flags,"serial":o.full_update_serial,"linked_members":children,"faces":faces})
}
fn revision(app: &App, s: &State, c: &Capture) -> Result<(), &'static str> {
    let token = uuid(&c.arguments, "expected_revision")?;
    let old = s
        .revisions
        .get(&token)
        .filter(|r| same_source(&r.capture, c) && Some(r.region) == app.world.main_region && r.expires > Instant::now())
        .ok_or("stale_build_revision")?;
    let idx = object(app, uuid(&c.arguments, "id")?, true)?;
    if old.value != state(app, idx, false) {
        return Err("build_target_changed");
    }
    Ok(())
}
pub(super) fn prepare(app: &App, s: &State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    if c.tool == Tool::ObjectOperationStatus {
        args(c, &["operation_id"])?;
        let r = tracked(s, c, uuid(&c.arguments, "operation_id")?)?;
        rez_receipt(app, s, c, r.operation)?;
        let mut classes = r.capture.required_classes.clone();
        classes.extend([Class::TaskInventoryRead, Class::Read, Class::WorldRead]);
        return Ok(classes);
    }
    if c.tool == Tool::ObjectRezIdentify {
        identify_gate(app, s, c)?;
        return Ok(vec![Class::TaskInventoryRead, Class::Read, Class::WorldRead]);
    }
    if c.tool == Tool::BuildObjectMaterial && s.materials.len() >= 8 {
        return Err("material_operation_capacity");
    }
    let a = &c.arguments;
    let mut classes = vec![Class::WorldRead];
    if matches!(c.tool, Tool::BuildRezPrimitive | Tool::BuildRezInventory) {
        args(
            c,
            if c.tool == Tool::BuildRezPrimitive {
                &["shape", "position", "rotation", "scale", "client_request_id"]
            } else {
                &["id", "position", "client_request_id"]
            },
        )?;
        let p = position(app, a)?;
        if app.world.rlva.has("rez")
            || app.world.rlva.has("edit")
            || app.world.rlva.has("editworld")
            || app.world.rlva.has("showloc")
            || app.world.rlva.has("interact")
            || app.world.build_right_at(app.world.main_region.ok_or("native_not_ready")?, p, None)
                != crate::world::land::BuildRight::Allowed
            || app.world.parcel.as_ref().is_none_or(|current| {
                app.world
                    .known_parcel_at(app.world.main_region.unwrap_or_default(), p, None)
                    .is_none_or(|there| there.local_id != current.local_id)
            })
        {
            return Err("native_rez_permission_denied");
        }
        if a.contains_key("client_request_id") {
            uuid(a, "client_request_id")?;
        }
        if c.tool == Tool::BuildRezInventory {
            item(app, uuid(a, "id")?, 6, false)?;
            classes.push(Class::Read);
        } else {
            if !matches!(string(a, "shape", 10)?, "cube" | "sphere" | "cylinder" | "cone") {
                return Err("invalid_args");
            }
            quaternion(a, "rotation")?;
        }
    } else {
        let keys: &[&str] = match c.tool {
            Tool::BuildObjectGet => &["id"],
            Tool::BuildObjectTransform => &["id", "expected_revision", "position", "rotation", "scale"],
            Tool::BuildObjectSurface => &[
                "id",
                "expected_revision",
                "face",
                "color",
                "glow",
                "fullbright",
                "uv_scale",
                "uv_offset",
                "uv_rotation",
                "texture_item",
            ],
            _ => &["id", "expected_revision", "face", "material_item"],
        };
        args(c, keys)?;
        let idx = object(app, uuid(a, "id")?, c.tool != Tool::BuildObjectGet)?;
        if c.tool != Tool::BuildObjectGet {
            revision(app, s, c)?;
        }
        if c.tool == Tool::BuildObjectTransform {
            if !["position", "rotation", "scale"].iter().any(|k| a.contains_key(*k)) {
                return Err("invalid_args");
            }
            if a.contains_key("position") {
                position(app, a)?;
            }
            if a.contains_key("rotation") {
                quaternion(a, "rotation")?;
            }
            if a.contains_key("scale") && crate::build::family(&app.world, idx).len() > 1 {
                return Err("linked_scale_requires_native_build");
            }
        }
        if matches!(c.tool, Tool::BuildObjectSurface | Tool::BuildObjectMaterial) {
            let f = a
                .get("face")
                .and_then(Value::as_u64)
                .filter(|f| *f < 64 && (*f as usize) < app.build.num_faces(&app.world, idx))
                .ok_or("invalid_face")? as u8;
            if app
                .world
                .objects
                .get(idx)
                .and_then(|o| o.te.as_ref())
                .is_none_or(|te| f as usize >= te.faces.len())
            {
                return Err("texture_entry_unavailable");
            }
            if c.tool == Tool::BuildObjectSurface {
                if crate::build::BuildTool::gltf_asset(&app.world, idx, f).is_some() {
                    return Err("legacy_surface_required");
                }
                if ![
                    "color",
                    "glow",
                    "fullbright",
                    "uv_scale",
                    "uv_offset",
                    "uv_rotation",
                    "texture_item",
                ]
                .iter()
                .any(|k| a.contains_key(*k))
                {
                    return Err("invalid_args");
                }
                if a.contains_key("color") {
                    vecn::<4>(a, "color", 0., 1.)?;
                }
                if a.contains_key("glow") {
                    scalar(a, "glow", 0., 1.)?;
                }
                if a.contains_key("fullbright") && a["fullbright"].as_bool().is_none() {
                    return Err("invalid_args");
                }
                for k in ["uv_scale", "uv_offset"] {
                    if a.contains_key(k) {
                        vecn::<2>(a, k, -1e9, 1e9)?;
                    }
                }
                if a.contains_key("uv_rotation") {
                    scalar(a, "uv_rotation", -6.283186, 6.283186)?;
                }
                if a.contains_key("texture_item") {
                    item(app, uuid(a, "texture_item")?, 0, true)?;
                    classes.push(Class::Read);
                }
            } else {
                item(app, uuid(a, "material_item")?, 57, true)?;
                if !app.world.main_has_cap("ModifyMaterialParams") {
                    return Err("native_material_capability_unavailable");
                }
                classes.push(Class::Read);
            }
        }
    }
    if a.contains_key("scale") {
        let v = Vec3::from_array(vecn(a, "scale", 0.001, 256.)?);
        let l = app.world.prim_limits();
        if v.min_element() < l.min || v.max_element() > l.max.min(64.) {
            return Err("native_prim_scale_limit");
        }
    }
    Ok(classes)
}
pub(super) fn evidence(app: &App, s: &State, c: &Capture) -> Result<Value, &'static str> {
    let a = &c.arguments;
    if matches!(c.tool, Tool::ObjectRezIdentify | Tool::ObjectOperationStatus) {
        let r = tracked(s, c, uuid(a, "operation_id")?)?;
        rez_gate(app, r)?;
        return Ok(
            json!({"operation":r.operation,"region":r.region,"association":r.result.get("object_id"),"candidate":if c.tool==Tool::ObjectRezIdentify { Some(state(app,object(app,uuid(a,"object_id")?,false)?,false)) }else{None}}),
        );
    }
    let value = if matches!(c.tool, Tool::BuildRezPrimitive | Tool::BuildRezInventory) {
        json!({"position":position(app,a)?.to_array(),"agent":app.world.agent.position.to_array(),"parcel":app.world.parcel.as_ref().map(|p|p.local_id),"group":app.world.group_for_rez(app.settings.build.rez_under_land_group)})
    } else {
        state(app, object(app, uuid(a, "id")?, c.tool != Tool::BuildObjectGet)?, false)
    };
    let cargo = if a.contains_key("material_item") {
        Some(item(app, uuid(a, "material_item")?, 57, true)?)
    } else if a.contains_key("texture_item") {
        Some(item(app, uuid(a, "texture_item")?, 0, true)?)
    } else if c.tool == Tool::BuildRezInventory {
        Some(item(app, uuid(a, "id")?, 6, false)?)
    } else {
        None
    };
    Ok(
        json!({"target":value,"inventory":cargo.map(|i|json!({"id":i.id,"asset":i.asset_id,"owner":i.owner,"parent":i.parent,"base":i.base_mask,"permissions":i.owner_mask,"flags":i.flags}))}),
    )
}
pub(super) fn execute(app: &mut App, s: &mut State, runtime: &Runtime, c: &Capture) -> Result<Value, &'static str> {
    let a = &c.arguments;
    match c.tool {
        Tool::ObjectOperationStatus => rez_receipt(app, s, c, uuid(a, "operation_id")?),
        Tool::ObjectRezIdentify => rez_identify(app, s, c),
        Tool::BuildObjectGet => {
            if s.revisions.len() >= MAX_SNAPSHOTS {
                return Err("build_revision_capacity");
            }
            let idx = object(app, uuid(a, "id")?, false)?;
            let token = Uuid::new_v4();
            s.revisions.insert(
                token,
                Revision {
                    capture: c.clone(),
                    region: app.world.main_region.ok_or("native_not_ready")?,
                    value: state(app, idx, false),
                    expires: Instant::now() + Duration::from_secs(120),
                    disclosed: app
                        .world
                        .objects
                        .get(idx)
                        .and_then(|o| o.te.as_ref())
                        .map(|te| {
                            te.faces
                                .iter()
                                .take(app.build.num_faces(&app.world, idx))
                                .enumerate()
                                .flat_map(|(f, te)| {
                                    [
                                        te.texture,
                                        crate::build::BuildTool::gltf_asset(&app.world, idx, f as u8).unwrap_or_default(),
                                    ]
                                })
                                .filter(|id| crate::world::inventory::asset_ids::AssetIds::diagnostics(&app.world).shown(id))
                                .collect()
                        })
                        .unwrap_or_default(),
                },
            );
            Ok(
                json!({"building_tools":1,"status":"observed","target":state(app,idx,true),"expected_revision":token,"revision_seconds":120}),
            )
        }
        Tool::BuildObjectTransform => {
            let idx = object(app, uuid(a, "id")?, true)?;
            let p = if a.contains_key("position") {
                Some(position(app, a)?)
            } else {
                None
            };
            let q = if a.contains_key("rotation") {
                Some(quaternion(a, "rotation")?)
            } else {
                None
            };
            let scale = if a.contains_key("scale") {
                Some(Vec3::from_array(vecn(a, "scale", 0.001, 256.)?))
            } else {
                None
            };
            let o = app.world.objects.get_mut(idx).ok_or("object_unavailable")?;
            let key = o.key;
            if let Some(v) = p {
                o.position = v;
            }
            if let Some(v) = q {
                o.rotation = v;
            }
            if let Some(v) = scale {
                o.scale = v;
            }
            o.velocity = Vec3::ZERO;
            o.angular_velocity = Vec3::ZERO;
            o.render.needs_records = true;
            app.send(NetCommand::Build(BuildCmd::Transform {
                handle: key.region,
                updates: vec![TransformUpdate {
                    local_id: key.local_id,
                    position: p,
                    rotation: q,
                    scale,
                    linked: true,
                    uniform: false,
                }],
            }));
            Ok(submitted(c))
        }
        Tool::BuildObjectSurface => {
            let idx = object(app, uuid(a, "id")?, true)?;
            let face = a["face"].as_u64().ok_or("invalid_args")? as usize;
            let texture = if a.contains_key("texture_item") {
                Some(item(app, uuid(a, "texture_item")?, 0, true)?.asset_id)
            } else {
                None
            };
            let o = app.world.objects.get_mut(idx).ok_or("object_unavailable")?;
            let te = std::sync::Arc::make_mut(o.te.as_mut().ok_or("texture_entry_unavailable")?);
            let f = te.faces.get_mut(face).ok_or("invalid_face")?;
            if a.contains_key("color") {
                f.color = vecn(a, "color", 0., 1.)?;
            }
            if a.contains_key("glow") {
                f.glow = scalar(a, "glow", 0., 1.)?;
            }
            if let Some(v) = a.get("fullbright").and_then(Value::as_bool) {
                f.bump_shiny_fullbright = (f.bump_shiny_fullbright & !0x20) | if v { 0x20 } else { 0 };
            }
            if a.contains_key("uv_scale") {
                let v = vecn::<2>(a, "uv_scale", -1e9, 1e9)?;
                f.scale_s = v[0];
                f.scale_t = v[1];
            }
            if a.contains_key("uv_offset") {
                let v = vecn::<2>(a, "uv_offset", -1e9, 1e9)?;
                f.offset_s = v[0];
                f.offset_t = v[1];
            }
            if a.contains_key("uv_rotation") {
                f.rotation = scalar(a, "uv_rotation", -6.283186, 6.283186)?;
            }
            if let Some(v) = texture {
                f.texture = v;
            }
            o.material_dirty = true;
            o.render.needs_records = true;
            app.build.send_te(&app.world, idx);
            app.flush_build();
            Ok(submitted(c))
        }
        Tool::BuildObjectMaterial => {
            let id = uuid(a, "id")?;
            let asset = item(app, uuid(a, "material_item")?, 57, true)?.asset_id;
            let face = a["face"].as_u64().ok_or("invalid_args")? as u8;
            let cap = app
                .world
                .main()
                .and_then(|r| r.caps.get("ModifyMaterialParams"))
                .cloned()
                .ok_or("native_material_capability_unavailable")?;
            let body = aurora_llsd::Llsd::Array(vec![
                aurora_llsd::llsd_map! {"object_id"=>id,"side"=>face as i32,"asset_id"=>asset,"gltf_json"=>""},
            ]);
            let data = aurora_llsd::to_xml(&body);
            let http = app.net.caps_http();
            let runtime = runtime.clone();
            let capture = c.clone();
            let tx = s.io.get_or_insert_with(|| crossbeam_channel::bounded(8)).0.clone();
            s.materials.insert(c.request_id.clone(), Instant::now());
            app.net.runtime().spawn(async move {
                let accepted = if runtime.continue_action(&capture, || ()).is_ok() {
                    match http
                        .post(cap)
                        .header("Content-Type", "application/llsd+xml")
                        .body(data)
                        .timeout(Duration::from_secs(25))
                        .send()
                        .await
                    {
                        Ok(mut response) if response.status().is_success() => {
                            let mut bytes = Vec::new();
                            let mut complete = true;
                            loop {
                                match response.chunk().await {
                                    Ok(Some(chunk)) if chunk.len() <= 65_536usize.saturating_sub(bytes.len()) => {
                                        bytes.extend_from_slice(&chunk)
                                    }
                                    Ok(None) => break,
                                    _ => {
                                        complete = false;
                                        break;
                                    }
                                }
                            }
                            complete && aurora_llsd::from_xml(&bytes).is_ok_and(|reply| reply["success"].as_bool())
                        }
                        _ => false,
                    }
                } else {
                    false
                };
                let _ = tx.try_send(MaterialReply {
                    capture,
                    object: id,
                    face,
                    asset,
                    accepted,
                });
            });
            Ok(submitted(c))
        }
        Tool::BuildRezPrimitive | Tool::BuildRezInventory => {
            let client = if a.contains_key("client_request_id") {
                Some(uuid(a, "client_request_id")?)
            } else {
                None
            };
            if let Some(key) = client {
                if let Some(old) = s.rez.get(&(c.client, key)) {
                    if !same_source(&old.capture, c) || old.capture.arguments != c.arguments {
                        return Err("client_request_conflict");
                    }
                    return Ok(old.result.clone());
                }
                if s.rez.len() >= 128 {
                    return Err("rez_receipt_capacity");
                }
                if s.retired_rez.contains(&(c.client, key)) || s.retired_rez.len() >= 4096 {
                    return Err("retired_rez_request_id_or_capacity");
                }
            }
            let p = position(app, a)?;
            let handle = app.world.main_region.ok_or("native_not_ready")?;
            let group = app.world.group_for_rez(app.settings.build.rez_under_land_group);
            let command = if c.tool == Tool::BuildRezInventory {
                let i = item(app, uuid(a, "id")?, 6, false)?.clone();
                BuildCmd::RezInventory(Box::new(aurora_net::inventory::rez::RezInventory {
                    handle,
                    group,
                    source_folder: i.parent,
                    item: i,
                    ray_start: p,
                    ray_end: p,
                    target: Uuid::nil(),
                    selected: false,
                    remove: false,
                }))
            } else {
                let index = match string(a, "shape", 10)? {
                    "cube" => 0,
                    "cylinder" => 4,
                    "cone" => 6,
                    "sphere" => 8,
                    _ => return Err("invalid_args"),
                };
                BuildCmd::Add(Box::new(NewPrim {
                    handle,
                    group_id: group,
                    pcode: aurora_prim::params::LL_PCODE_VOLUME,
                    material: 3,
                    add_flags: 0,
                    shape: crate::build::shapes::shapes()[index].raw(),
                    ray_start: p,
                    ray_end: p,
                    ray_target: Uuid::nil(),
                    ray_end_is_intersection: false,
                    bypass_raycast: true,
                    scale: Vec3::from_array(vecn(a, "scale", 0.001, 256.)?),
                    rotation: quaternion(a, "rotation")?,
                    state: 0,
                }))
            };
            let mut result = submitted(c);
            result["state"] = json!("submitted_unknown");
            result["correlation"] = json!("unconfirmed");
            result["candidates"] = json!([]);
            result["object_id_confirmed"] = json!(false);
            result["permissions_source"] = json!(if c.tool == Tool::BuildRezInventory {
                "inventory_item"
            } else {
                "simulator_defaults"
            });
            if let Some(key) = client {
                let operation = Uuid::new_v4();
                result["operation_id"] = json!(operation);
                result["client_request_id"] = json!(key);
                let before: BTreeSet<_> = app
                    .world
                    .objects
                    .iter()
                    .filter(|(_, o)| {
                        Some(o.key.region) == app.world.main_region
                            && o.parent_id == 0
                            && o.pcode == aurora_prim::params::LL_PCODE_VOLUME
                            && o.update_flags & crate::ui::context::flags::OBJECT_YOU_OWNER != 0
                            && o.position.distance_squared(p) <= 64.
                    })
                    .map(|(_, o)| o.full_id)
                    .take(513)
                    .collect();
                if before.len() > 512 {
                    return Err("tracked_rez_nearby_root_capacity");
                }
                s.retired_rez.insert((c.client, key));
                s.rez.insert(
                    (c.client, key),
                    Rez {
                        operation,
                        capture: c.clone(),
                        region: handle,
                        parent: None,
                        before,
                        position: p,
                        at: Instant::now(),
                        result: result.clone(),
                    },
                );
            }
            app.send(NetCommand::Build(command));
            Ok(result)
        }
        _ => Err("unsupported_capability"),
    }
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    let now = Instant::now();
    assistant
        .creative
        .build
        .revisions
        .retain(|_, r| r.expires > now && assistant.binding == Some(r.capture.binding));
    assistant.creative.build.rez.retain(|_, r| {
        now.duration_since(r.at) < Duration::from_secs(600)
            && assistant.binding == Some(r.capture.binding)
            && Some(r.region) == app.world.main_region
    });
    assistant
        .creative
        .build
        .materials
        .retain(|_, at| now.duration_since(*at) < Duration::from_secs(30));
    let replies: Vec<_> = assistant
        .creative
        .build
        .io
        .as_ref()
        .map(|(_, rx)| rx.try_iter().collect())
        .unwrap_or_default();
    for reply in replies {
        assistant.creative.build.materials.remove(&reply.capture.request_id);
        if !binding_matches(app, assistant, &reply.capture) {
            continue;
        }
        let result = json!({"status":if reply.accepted{"material_http_accepted"}else{"material_transport_failed_or_unconfirmed"},"operation_id":reply.capture.request_id,"simulator_confirmed":false,"retry_safe":false});
        if reply.accepted
            && assistant.runtime.capture_current(&reply.capture)
            && revision(app, &assistant.creative.build, &reply.capture).is_ok()
            && let Some(idx) = app.world.objects.index_of_uuid(&reply.object)
        {
            if let Some(o) = app.world.objects.get_mut(idx) {
                o.extra.render_materials.retain(|(f, _)| *f != reply.face);
                o.extra.render_materials.push((reply.face, reply.asset));
                o.material_dirty = true;
                o.render.needs_records = true;
            }
            if let Some(o) = app.world.objects.get(idx)
                && let Some(overrides) = app.world.gltf_overrides.get_mut(&o.key)
            {
                let keep: Vec<_> = overrides.iter().filter(|(face, _)| *face != reply.face).cloned().collect();
                *overrides = std::sync::Arc::from(keep);
            }
        }
        let _ = assistant.runtime.complete(&reply.capture, result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tracked_rez_receipts_require_exact_client_endpoint_binding_and_policy() {
        let mut state = State::default();
        let original = synthetic_capture(Tool::BuildRezPrimitive);
        let operation = Uuid::new_v4();
        let request = Uuid::new_v4();
        state.rez.insert(
            (original.client, request),
            Rez {
                operation,
                capture: original.clone(),
                region: 1,
                parent: None,
                before: BTreeSet::new(),
                position: Vec3::ZERO,
                at: Instant::now(),
                result: json!({"operation_id":operation,"object_id_confirmed":false}),
            },
        );
        let mut status = original.clone();
        status.tool = Tool::ObjectOperationStatus;
        status.arguments = json!({"operation_id":operation}).as_object().unwrap().clone();
        assert!(owns_rez(&state, &status));
        for dimension in 0..4 {
            let mut foreign = status.clone();
            match dimension {
                0 => foreign.client += 1,
                1 => foreign.endpoint = HostId::from_bytes([8; 16]).unwrap(),
                2 => foreign.binding.session.as_mut().unwrap().generation += 1,
                _ => foreign.policy_revision += 1,
            }
            assert!(!owns_rez(&state, &foreign));
            assert_eq!(tracked(&state, &foreign, operation).err(), Some("unknown_operation"));
        }
        state.retired_rez.insert((original.client, request));
        state.rez.clear();
        assert!(!owns_rez(&state, &status));
        assert!(
            state.retired_rez.contains(&(original.client, request)),
            "receipt eviction must not permit a rez replay"
        );
    }
}
