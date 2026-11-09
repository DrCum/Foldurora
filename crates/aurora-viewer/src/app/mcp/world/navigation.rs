use super::*;
#[derive(Default)]
pub(super) struct State {
    owned: Option<Owned>,
    last: BTreeMap<u64, Value>,
    completed: Vec<(Capture, Value)>,
}
#[derive(Clone)]
struct Owned {
    capture: Capture,
    id: Uuid,
    region: u64,
    target: DVec3,
    deadline: Instant,
    kind: Kind,
    submitted: bool,
}
#[derive(Clone)]
enum Kind {
    Move { started: Instant, local: Vec3 },
    Teleport { handle: u64, local: Vec3 },
}
pub(super) fn supports(t: Tool) -> bool {
    matches!(
        t,
        Tool::NavigationStatus | Tool::NavigationMoveTo | Tool::NavigationFlyingSet | Tool::NavigationTeleport | Tool::NavigationStop
    )
}
fn idle(app: &App) -> Result<(), &'static str> {
    if app.world.agent.is_sitting()
        || app.native_anchor.blocks_walk(app.world.agent.seated, app.world.agent.ground_sit)
        || app.world.teleporting
        || app.autopilot.is_some()
        || app.world.following_avatar.is_some()
        || app.world.maturity.holds_teleport()
    {
        Err("native_navigation_busy")
    } else {
        Ok(())
    }
}
fn local_target(app: &App, c: &Capture) -> Result<(DVec3, Vec3), &'static str> {
    let target = vector(&c.arguments, "position")?;
    let origin = app.world.main_origin().ok_or("native_region_unavailable")?;
    let local = target - DVec3::new(origin.0 as f64, origin.1 as f64, 0.);
    let region = app.world.main().ok_or("native_region_unavailable")?;
    if local.x < 0.
        || local.y < 0.
        || local.x >= region.heightmap.size_x as f64
        || local.y >= region.heightmap.size_y as f64
        || !(-4096.0..=4096.0).contains(&local.z)
        || local.distance(app.world.agent.position.as_dvec3()) > 64.
    {
        return Err("navigation_target_out_of_range");
    }
    if !app.world.rlva_can_teleport_global(target) {
        return Err("native_navigation_restriction");
    }
    Ok((target, local.as_vec3()))
}
fn teleport_target(c: &Capture) -> Result<(DVec3, u64, Vec3), &'static str> {
    let target = vector(&c.arguments, "position")?;
    if !(0.0..=u32::MAX as f64).contains(&target.x)
        || !(0.0..=u32::MAX as f64).contains(&target.y)
        || !(-4096.0..=4096.0).contains(&target.z)
    {
        return Err("invalid_args");
    }
    let x = (target.x / 256.).floor() as u32 * 256;
    let y = (target.y / 256.).floor() as u32 * 256;
    Ok((
        target,
        aurora_net::origin_to_handle(x, y),
        Vec3::new((target.x - x as f64) as f32, (target.y - y as f64) as f32, target.z as f32),
    ))
}
pub(super) fn prepare(app: &App, state: &super::State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    world_read(app)?;
    match c.tool {
        Tool::NavigationStatus => args(c, &[])?,
        Tool::NavigationStop => {
            args(c, &["operation_id"])?;
            let id = uuid(&c.arguments, "operation_id")?;
            if !state
                .navigation
                .owned
                .as_ref()
                .is_some_and(|n| n.id == id && same_source(&n.capture, c))
            {
                return Err("navigation_not_owned");
            }
            if state
                .navigation
                .owned
                .as_ref()
                .is_some_and(|n| matches!(n.kind, Kind::Teleport { .. }) && n.submitted && !app.named_teleport.owns(n.id))
            {
                return Err("teleport_cancel_not_current");
            }
        }
        Tool::NavigationFlyingSet => {
            args(c, &["flying"])?;
            idle(app)?;
            let desired = c.arguments.get("flying").and_then(Value::as_bool).ok_or("invalid_args")?;
            if desired && !app.world.agent.flying && !crate::autopilot::can_takeoff(&app.world) {
                return Err("native_flight_restriction");
            }
        }
        Tool::NavigationMoveTo => {
            args(c, &["position", "mode", "stop_distance", "timeout_seconds"])?;
            idle(app)?;
            if state.navigation.owned.is_some() {
                return Err("assistant_navigation_busy");
            }
            local_target(app, c)?;
            let mode = string(&c.arguments, "mode", 8)?;
            if !matches!(mode, "walk" | "fly") {
                return Err("invalid_args");
            }
            if mode == "walk" && app.world.agent.flying {
                return Err("flight_mode_mismatch");
            }
            if mode == "fly" && !app.world.agent.flying && !crate::autopilot::can_takeoff(&app.world) {
                return Err("native_flight_restriction");
            }
            finite(&c.arguments, "stop_distance", 1., 0.25, 4.)?;
            let timeout = count(&c.arguments, "timeout_seconds", 0, 30)?;
            if timeout == 0 {
                return Err("invalid_args");
            }
        }
        Tool::NavigationTeleport => {
            args(c, &["position"])?;
            idle(app)?;
            if state.navigation.owned.is_some() {
                return Err("assistant_navigation_busy");
            }
            let (target, handle, position) = teleport_target(c)?;
            if !app.world.rlva_can_teleport_global(target)
                || !app.world.rlva_allows_command(
                    &NetCommand::TeleportTo {
                        handle,
                        position,
                        look_at: app.camera.forward(),
                    },
                    Uuid::nil(),
                )
            {
                return Err("native_teleport_restriction");
            }
        }
        _ => return Err("unsupported_capability"),
    };
    Ok(if c.tool == Tool::NavigationStatus {
        vec![]
    } else {
        vec![Class::WorldRead]
    })
}
pub(super) fn evidence(app: &App, state: &super::State, c: &Capture) -> Result<Value, &'static str> {
    Ok(match c.tool {
        Tool::NavigationMoveTo | Tool::NavigationFlyingSet | Tool::NavigationTeleport => {
            json!({"agent":app.world.agent_id.to_string(),"position":xyz(app.world.agent.position),"flying":app.world.agent.flying,"seated":app.world.agent.is_sitting(),"pilot":app.autopilot.as_ref().map(|p|format!("{:?}",p.started)),"teleporting":app.world.teleporting,"maturity_synced":app.world.maturity.synced()})
        }
        Tool::NavigationStop => json!({"owned":state.navigation.owned.as_ref().map(|p|p.id.to_string())}),
        _ => Value::Null,
    })
}
fn owns(app: &App, n: &Owned) -> bool {
    match n.kind {
        Kind::Move { started, local } => app.autopilot.as_ref().is_some_and(|p| p.started == started && p.target == local),
        Kind::Teleport { .. } => n.submitted && app.world.teleporting && app.named_teleport.owns(n.id),
    }
}
fn same_source(a: &Capture, b: &Capture) -> bool {
    a.client == b.client && a.endpoint == b.endpoint && a.binding == b.binding && a.policy_revision == b.policy_revision
}
fn submit_teleport(app: &mut App, request: Uuid, handle: u64, local: Vec3, dest: String) -> bool {
    let command = NetCommand::TeleportTo {
        handle,
        position: local,
        look_at: app.camera.forward(),
    };
    app.named_teleport.arm_owned(request, &command);
    let accepted = app.request_user_teleport(command, dest, Some(handle) != app.world.main_region);
    app.named_teleport.finish_owned(request) && accepted
}
pub(super) fn execute(app: &mut App, assistant: &mut Assistant, c: &Capture) -> Result<Value, &'static str> {
    match c.tool {
        Tool::NavigationStatus => {
            let mut value = observation::location(app)?;
            value["pilot_active"] = json!(app.autopilot.is_some());
            value["teleporting"] = json!(app.world.teleporting);
            value["teleport_failed"] = json!(app.world.tp_failed);
            value["last_operation"] = assistant.world.navigation.last.get(&c.client).cloned().unwrap_or(Value::Null);
            value["owned_operation_id"] = assistant
                .world
                .navigation
                .owned
                .as_ref()
                .filter(|n| n.capture.client == c.client)
                .map(|n| json!(n.id.to_string()))
                .unwrap_or(Value::Null);
            Ok(value)
        }
        Tool::NavigationFlyingSet => {
            let desired = c.arguments.get("flying").and_then(Value::as_bool).ok_or("invalid_args")?;
            let changed = desired != app.world.agent.flying;
            if changed {
                app.toggle_fly();
            }
            Ok(json!({"flying":app.world.agent.flying,"changed":changed,"requested":desired,"server_confirmed":false}))
        }
        Tool::NavigationMoveTo => {
            let (target, local) = local_target(app, c)?;
            let flying = string(&c.arguments, "mode", 8)? == "fly";
            if flying && !app.world.agent.flying {
                app.toggle_fly();
            }
            if app.world.agent.flying != flying {
                return Err("native_flight_restriction");
            }
            let started = Instant::now();
            let stop = finite(&c.arguments, "stop_distance", 1., 0.25, 4.)? as f32;
            let pilot = crate::autopilot::AutoPilot::start_bounded(app.world.agent.position, local, stop, flying, started);
            app.world.following_avatar = None;
            app.follow_recovery = None;
            app.follow_ui.open = false;
            app.autopilot = Some(pilot);
            app.autopilot_stop_pending = false;
            app.autopilot_flags = 0;
            app.release_native_movelock();
            app.movement_runtime.cancel_water_exit();
            let id = Uuid::new_v4();
            let n = Owned {
                capture: c.clone(),
                id,
                region: app.world.main_region.ok_or("native_region_unavailable")?,
                target,
                deadline: started + Duration::from_secs(count(&c.arguments, "timeout_seconds", 30, 30)? as u64),
                kind: Kind::Move { started, local },
                submitted: true,
            };
            assistant.world.navigation.owned = Some(n);
            let result = json!({"status":"submitted","operation_id":id.to_string(),"request_id":c.request_id,"target":global(target),"mode":if flying{"fly"}else{"walk"},"arrival_confirmed":false,"retry_safe":false});
            assistant.world.navigation.last.insert(c.client, result.clone());
            Ok(result)
        }
        Tool::NavigationTeleport => {
            let (target, handle, local) = teleport_target(c)?;
            let id = Uuid::new_v4();
            let mut n = Owned {
                capture: c.clone(),
                id,
                region: app.world.main_region.ok_or("native_region_unavailable")?,
                target,
                deadline: Instant::now() + Duration::from_secs(60),
                kind: Kind::Teleport { handle, local },
                submitted: false,
            };
            if app.world.maturity.synced() {
                if !submit_teleport(app, id, handle, local, clip(&app.world.region_name(), 255)) {
                    return Err("native_teleport_restriction");
                }
                n.submitted = true;
            }
            let value = json!({"status":"submitted","operation_id":id.to_string(),"request_id":c.request_id,"waiting_maturity":!n.submitted,"native_submitted":n.submitted,"arrival_confirmed":false,"retry_safe":false});
            assistant.world.navigation.last.insert(c.client, value.clone());
            assistant.world.navigation.owned = Some(n);
            Ok(value)
        }
        Tool::NavigationStop => {
            let id = uuid(&c.arguments, "operation_id")?;
            let n = assistant
                .world
                .navigation
                .owned
                .as_ref()
                .filter(|n| n.id == id && same_source(&n.capture, c))
                .cloned()
                .ok_or("navigation_not_owned")?;
            let owned = owns(app, &n);
            let teleport_cancel_requested = owned && matches!(n.kind, Kind::Teleport { .. });
            let cancelled = if owned {
                match n.kind {
                    Kind::Move { .. } => {
                        app.stop_autopilot(true);
                        true
                    }
                    Kind::Teleport { .. } => {
                        app.send(NetCommand::TeleportCancel { request: n.id });
                        app.named_teleport.retire_owned(n.id);
                        false
                    }
                }
            } else {
                !n.submitted
            };
            finish(
                assistant,
                &n,
                "cancelled",
                json!({"cancelled":cancelled,"teleport_cancel_requested":teleport_cancel_requested,"simulator_confirmed":false,"arrival_undoable":false}),
            );
            assistant.world.navigation.owned = None;
            Ok(
                json!({"operation_id":id.to_string(),"cancelled":cancelled,"teleport_cancel_requested":teleport_cancel_requested,"simulator_confirmed":false,"owned_work_retired":true,"newer_native_navigation_untouched":true}),
            )
        }
        _ => Err("unsupported_capability"),
    }
}
fn finish(assistant: &mut Assistant, n: &Owned, status: &str, details: Value) {
    let mut value = json!({"status":status,"operation_id":n.id.to_string(),"request_id":n.capture.request_id,"retry_safe":false});
    value["details"] = details;
    assistant.world.navigation.last.insert(n.capture.client, value.clone());
    assistant.world.navigation.completed.push((n.capture.clone(), value));
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    for (capture, result) in std::mem::take(&mut assistant.world.navigation.completed) {
        let _ = assistant.runtime.complete(&capture, result);
    }
    let Some(mut n) = assistant.world.navigation.owned.clone() else {
        return;
    };
    let allowed = current(app, assistant, &n.capture)
        && (matches!(n.kind, Kind::Teleport { .. }) || super::super::active_allowed(app))
        && world_read(app).is_ok();
    let region = matches!(n.kind, Kind::Teleport { .. }) || app.world.main_region == Some(n.region);
    if !allowed || !region || Instant::now() >= n.deadline {
        if owns(app, &n) && matches!(n.kind, Kind::Move { .. }) {
            app.stop_autopilot(true);
        }
        finish(
            assistant,
            &n,
            if !allowed {
                "revoked"
            } else if !region {
                "region_changed"
            } else {
                "timeout"
            },
            Value::Null,
        );
        assistant.world.navigation.owned = None;
        return;
    }
    if let Kind::Teleport { handle, local } = n.kind {
        if n.submitted && !app.named_teleport.related(n.id) {
            finish(
                assistant,
                &n,
                "cancelled_or_replaced",
                json!({"newer_native_navigation_untouched":true}),
            );
            assistant.world.navigation.owned = None;
            return;
        }
        if !n.submitted && app.world.maturity.synced() {
            let engine = assistant.runtime.clone();
            if !app.world.teleporting
                && app.autopilot.is_none()
                && engine
                    .continue_action(&n.capture, || submit_teleport(app, n.id, handle, local, String::new()))
                    .unwrap_or(false)
            {
                n.submitted = true;
                assistant.world.navigation.owned = Some(n.clone());
            } else {
                finish(assistant, &n, "rejected_native", Value::Null);
                assistant.world.navigation.owned = None;
                return;
            }
        }
        if n.submitted && !app.world.teleporting {
            let arrived = app.world.agent_global().is_some_and(|p| p.distance(n.target) < 4.);
            finish(
                assistant,
                &n,
                if arrived { "arrival_observed" } else { "outcome_unknown" },
                json!({"arrival_observed":arrived}),
            );
            assistant.world.navigation.owned = None;
        }
    } else if !owns(app, &n) {
        let arrived = app.world.agent_global().is_some_and(|p| p.distance(n.target) <= 4.);
        finish(
            assistant,
            &n,
            if arrived { "arrival_observed" } else { "cancelled_or_replaced" },
            json!({"arrival_observed":arrived}),
        );
        assistant.world.navigation.owned = None;
    }
}
pub(super) fn cancel(app: &mut App, assistant: &mut Assistant, reason: &'static str) {
    if let Some(n) = assistant.world.navigation.owned.take() {
        if owns(app, &n) && matches!(n.kind, Kind::Move { .. }) {
            app.stop_autopilot(true);
        }
        finish(assistant, &n, reason, Value::Null);
    }
}
