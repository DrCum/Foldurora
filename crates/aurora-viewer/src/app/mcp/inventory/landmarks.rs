use super::*;
use crate::world::landmarks::LandmarkAsset;
pub(super) fn prepare(app: &App, c: &Capture) -> Result<(), &'static str> {
    let item = app.world.inventory.items.get(&uuid(&c.arguments, "id")?).ok_or("not_found")?;
    if item.asset_type != 3 || item.asset_id.is_nil() {
        return Err("unsupported_item_type");
    }
    if app.world.rlva.has("showloc") || app.world.rlva.has("showworldmap") {
        return Err("native_location_restriction");
    }
    Ok(())
}
pub(super) fn execute(app: &mut App, state: &mut State, c: &Capture) -> Result<Value, &'static str> {
    state.admit()?;
    prepare(app, c)?;
    let id = uuid(&c.arguments, "id")?;
    let item = app.world.inventory.items[&id].clone();
    let request = Uuid::new_v4();
    pending_for(
        state,
        request,
        c,
        PendingKind::Landmark {
            item: Box::new(Frozen::capture(&app.world.inventory, id)?),
            asset: item.asset_id,
            region: None,
        },
    );
    app.send(NetCommand::ReadAssistantLandmark {
        request,
        item: Box::new(item),
    });
    Ok(submitted(c))
}
fn reply(app: &App, p: &Pending, global: glam::DVec3, region: Uuid) -> Value {
    let PendingKind::Landmark { item, asset, .. } = &p.kind else {
        return failure("invalid_landmark_result");
    };
    if !global.is_finite() {
        return failure("native_landmark_response_invalid");
    }
    let mut value = json!({"id":item.id.to_string(),"region_id":(!region.is_nil()).then(||region.to_string()),"global_pos":[global.x,global.y,global.z],"stale":false,"complete":true});
    if rules::asset_uuid_shown(&app.world.inventory, *asset) {
        value["asset_id"] = json!(asset.to_string());
    } else {
        value["asset_id_protected"] = json!(true);
    }
    value
}
pub(super) fn native_event(app: &mut App, event: &NetEvent) -> bool {
    match event {
        NetEvent::AssistantLandmark {
            request,
            item,
            asset,
            result,
        } => {
            let Some(mut pending) = app.character_mcp.as_mut().and_then(|a| a.inventory.pending.remove(request)) else {
                return true;
            };
            let PendingKind::Landmark {
                item: frozen,
                asset: expected,
                ..
            } = &pending.kind
            else {
                return true;
            };
            if *item != frozen.id || asset != expected || !super::super::source_current(app, &pending.capture) {
                return true;
            }
            if !frozen.current(&app.world.inventory) || prepare(app, &pending.capture).is_err() {
                mutations::finish(app, pending, failure("stale_landmark_source"));
                return true;
            }
            let parsed = result.as_ref().ok().and_then(|bytes| crate::world::landmarks::parse(bytes));
            match parsed {
                Some(LandmarkAsset::Global(global)) => {
                    let value = reply(app, &pending, global, Uuid::nil());
                    mutations::finish(app, pending, value);
                }
                Some(LandmarkAsset::Region { region_id, pos }) if pos.is_finite() => {
                    let known = app
                        .world
                        .regions
                        .iter()
                        .find_map(|(handle, r)| r.info.as_ref().filter(|i| i.region_id == region_id).map(|_| *handle));
                    if let Some(handle) = known {
                        let (x, y) = aurora_net::handle_to_origin(handle);
                        let value = reply(
                            app,
                            &pending,
                            glam::DVec3::new(f64::from(x) + f64::from(pos.x), f64::from(y) + f64::from(pos.y), f64::from(pos.z)),
                            region_id,
                        );
                        mutations::finish(app, pending, value);
                    } else if commit_continuation(app, &pending.capture, |app| app.send(NetCommand::RegionHandleRequest(region_id))).is_ok()
                    {
                        if let PendingKind::Landmark { region, .. } = &mut pending.kind {
                            *region = Some((region_id, pos));
                        }
                        if let Some(a) = app.character_mcp.as_mut() {
                            a.inventory.pending.insert(*request, pending);
                        }
                    } else {
                        mutations::finish(app, pending, failure("stale_source_or_authority"));
                    }
                }
                _ => mutations::finish(app, pending, failure("native_landmark_read_failed")),
            }
            true
        }
        NetEvent::RegionIdHandle { region_id, handle } => {
            let found = app
                .character_mcp
                .as_ref()
                .map(|a| {
                    a.inventory
                        .pending
                        .iter()
                        .filter_map(|(id, p)| {
                            matches!(p.kind,PendingKind::Landmark{region:Some((region,_)),..}if region==*region_id)
                                .then_some((*id, p.clone()))
                        })
                        .take(CAPACITY)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for (id, pending) in found {
                if let Some(a) = app.character_mcp.as_mut() {
                    a.inventory.pending.remove(&id);
                }
                if !super::super::source_current(app, &pending.capture) {
                    continue;
                }
                let PendingKind::Landmark {
                    ref item,
                    region: Some((region, pos)),
                    ..
                } = pending.kind
                else {
                    continue;
                };
                let value = if item.current(&app.world.inventory) && prepare(app, &pending.capture).is_ok() {
                    let (x, y) = aurora_net::handle_to_origin(*handle);
                    reply(
                        app,
                        &pending,
                        glam::DVec3::new(f64::from(x) + f64::from(pos.x), f64::from(y) + f64::from(pos.y), f64::from(pos.z)),
                        region,
                    )
                } else {
                    failure("stale_landmark_source")
                };
                mutations::finish(app, pending, value);
            }
            false
        }
        _ => false,
    }
}
