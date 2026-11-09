use super::*;
pub(super) fn supports(t: Tool) -> bool {
    matches!(
        t,
        Tool::WorldLocation
            | Tool::WorldParcel
            | Tool::WorldNearbyAvatars
            | Tool::WorldNearbyObjects
            | Tool::WorldAudioSources
            | Tool::WorldMediaSources
    )
}
pub(super) fn prepare(app: &App, c: &Capture) -> Result<Vec<Class>, &'static str> {
    world_read(app)?;
    if matches!(c.tool, Tool::WorldLocation | Tool::WorldParcel) {
        args(c, &[])?;
    } else {
        args(c, &["radius", "limit"])?;
        finite(&c.arguments, "radius", 64., 1., 256.)?;
        limit(&c.arguments, 25, 100)?;
    }
    if app.world.main_region.is_none() {
        return Err("native_region_unavailable");
    }
    Ok(Vec::new())
}
pub(super) fn location(app: &App) -> Result<Value, &'static str> {
    world_read(app)?;
    let global = app.world.agent_global().ok_or("native_region_unavailable")?;
    Ok(
        json!({"region":clip(&app.world.region_name(),255),"region_handle":app.world.main_region.map(|h|h.to_string()),"position":xyz(app.world.agent.position),"global_position":super::global(global),"yaw":app.world.agent.yaw,"pitch":app.world.agent.pitch,"flying":app.world.agent.flying,"sitting":app.world.agent.is_sitting(),"observed":true}),
    )
}
fn loaded(app: &App, index: usize, o: &crate::world::objects::Object) -> bool {
    !o.full_id.is_nil()
        && !o.is_client_preview()
        && app.world.regions.contains_key(&o.key.region)
        && !app.world.derender.contains(&o.full_id)
        && app.world.parcel_object_visible(index)
}
fn root_position(app: &App, o: &crate::world::objects::Object) -> Option<Vec3> {
    let p = app.world.region_offset(o.key.region)? + o.position;
    p.is_finite().then_some(p)
}
fn bounded(mut rows: Vec<(f32, Value)>, limit: usize) -> (Vec<Value>, bool) {
    rows.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1["id"].as_str().cmp(&b.1["id"].as_str())));
    let limited = rows.len() > limit;
    rows.truncate(limit);
    let mut bytes = 0usize;
    let mut items = Vec::new();
    let mut limited = limited;
    for (_, row) in rows {
        let size = serde_json::to_vec(&row).map_or(24 * 1024, |b| b.len());
        if bytes + size > 18 * 1024 {
            limited = true;
            break;
        }
        bytes += size;
        items.push(row);
    }
    (items, limited)
}
fn keep_bounded(rows: &mut Vec<(f32, Value)>) {
    if rows.len() >= 1024 {
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        rows.truncate(512);
    }
}
pub(super) fn execute(app: &App, state: &mut State, c: &Capture) -> Result<Value, &'static str> {
    world_read(app)?;
    if c.tool == Tool::WorldLocation {
        return location(app);
    }
    if c.tool == Tool::WorldParcel {
        let Some(p) = app.world.parcel.as_ref() else {
            return Ok(json!({"loaded":false,"parcel":null}));
        };
        return Ok(
            json!({"loaded":true,"parcel":{"local_id":p.local_id,"name":clip(&p.name,255),"description":clip(&p.desc,2048),"owner_id":p.owner_id.to_string(),"group_id":p.group_id.to_string(),"group_owned":p.is_group_owned,"area":p.area,"allow_fly":p.flags&aurora_net::parcel_flags::ALLOW_FLY!=0,"allow_build":crate::world::land::agent_can_build(p,&crate::world::land::AgentRights{id:app.world.agent_id,groups:&app.world.groups.groups}),"see_avatars":p.see_avatars,"media_configured":!p.media_url.is_empty(),"music_configured":!p.music_url.is_empty()},"untrusted_data":true}),
        );
    }
    let radius = finite(&c.arguments, "radius", 64., 1., 256.)? as f32;
    let max = limit(&c.arguments, 25, 100)?;
    let me = app.world.agent.position;
    let mut rows = Vec::new();
    let mut examined = 0usize;
    match c.tool {
        Tool::WorldNearbyAvatars => {
            for (index, o) in app.world.objects.avatars().take(65_536) {
                examined += 1;
                keep_bounded(&mut rows);
                if o.full_id == app.world.agent_id || !loaded(app, index, o) {
                    continue;
                }
                let Some((position, _, _)) = crate::scene::Scene::object_transform_unshifted(&app.world, index, Instant::now()) else {
                    continue;
                };
                let distance = position.distance(me);
                if !distance.is_finite() || distance > radius {
                    continue;
                }
                let name = app
                    .world
                    .social
                    .avatar_names
                    .raw(&o.full_id)
                    .map(|n| clip(&n.display_name, 255))
                    .or_else(|| o.display_name().map(|s| clip(&s, 255)));
                rows.push((distance,json!({"id":o.full_id.to_string(),"position":xyz(position),"distance":distance,"name":name,"name_resolved":name.is_some(),"loaded":true})));
            }
        }
        Tool::WorldNearbyObjects => {
            for (index, slot) in app.world.objects.slots.iter().enumerate().take(65_536) {
                examined += 1;
                keep_bounded(&mut rows);
                let Some(o) = slot else { continue };
                if o.pcode != 9 || o.parent_id != 0 || o.is_attachment() || !loaded(app, index, o) {
                    continue;
                }
                let Some(position) = root_position(app, o) else { continue };
                let distance = position.distance(me);
                if !distance.is_finite() || distance > radius {
                    continue;
                }
                rows.push((distance,json!({"id":o.full_id.to_string(),"position":xyz(position),"distance":distance,"scale":xyz(o.scale),"rotation":[o.rotation.x,o.rotation.y,o.rotation.z,o.rotation.w],"properties_loaded":false})));
            }
        }
        Tool::WorldAudioSources => {
            let center = app.world.agent_global().ok_or("native_region_unavailable")?;
            for source in app.scene.sounds.history.entries().take(256) {
                examined += 1;
                keep_bounded(&mut rows);
                if source.started < state.audio_since || !source.position.is_finite() {
                    continue;
                }
                let distance = source.position.distance(center) as f32;
                if !distance.is_finite() || distance > radius {
                    continue;
                }
                if app.world.mutes.is_muted_id(&source.source)
                    || app.world.mutes.is_muted_id(&source.owner)
                    || app.world.derender.contains(&source.source)
                {
                    continue;
                }
                rows.push((distance,json!({"id":source.id.to_string(),"source_id":source.source.to_string(),"owner_id":source.owner.to_string(),"global_position":super::global(source.position),"distance":distance,"playing_observed":source.stopped.is_none(),"triggered":source.triggered,"looped":source.looped,"age_seconds":source.started.elapsed().as_secs_f64(),"audibility_confirmed":false})));
            }
        }
        Tool::WorldMediaSources => {
            for (index, slot) in app.world.objects.slots.iter().enumerate().take(65_536) {
                examined += 1;
                keep_bounded(&mut rows);
                let Some(o) = slot else { continue };
                if o.pcode != 9 || !loaded(app, index, o) {
                    continue;
                }
                let Some((position, _, hud)) = crate::scene::Scene::object_transform_unshifted(&app.world, index, Instant::now()) else {
                    continue;
                };
                if hud {
                    continue;
                }
                let distance = position.distance(me);
                if !distance.is_finite() || distance > radius {
                    continue;
                }
                if let Some(te) = &o.te {
                    let faces:Vec<_>=te.faces.iter().enumerate().take(64).filter(|(_,f)|f.media_flags&1!=0).map(|(face,_)|json!({"face":face,"association_loaded":app.media.object_media(&o.full_id).is_some(),"plugin_loaded":app.media.find(&crate::media::MediaKey::Prim{object:o.full_id,face:face as u8}).is_some()})).collect();
                    if !faces.is_empty() {
                        rows.push((
                            distance,
                            json!({"id":o.full_id.to_string(),"position":xyz(position),"distance":distance,"faces":faces}),
                        ));
                    }
                }
            }
        }
        _ => return Err("unsupported_capability"),
    }
    let (rows, limited) = bounded(rows, max);
    let mut out = json!({"items":rows,"radius":radius,"limited":limited||examined>=65_536,"coverage":"loaded_observations_only","absence_proven":false,"observed":true});
    if c.tool == Tool::WorldAudioSources {
        out["history_since_seconds"] = json!(state.audio_since.elapsed().as_secs_f64());
        out["asset_ids_included"] = json!(false);
    }
    if c.tool == Tool::WorldMediaSources {
        out["urls_included"] = json!(false);
        out["parcel"] = match &app.world.parcel {
            Some(p) => {
                json!({"loaded":true,"media_configured":!p.media_url.is_empty(),"music_configured":!p.music_url.is_empty(),"media_playing":app.media.parcel_status_playing(),"music_requested":app.music_playing,"media_enabled":app.settings.media.enabled})
            }
            None => json!({"loaded":false}),
        };
    }
    Ok(out)
}
