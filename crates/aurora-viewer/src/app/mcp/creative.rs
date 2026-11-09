//! Typed creative adapters. Behavior reference: Folderstorm abbe3cb0,
//! fsbuildlistener.cpp / fsposelistener.cpp / fssharedposemanager.cpp
//! (LGPL-2.1-or-later). No arbitrary native command or file path is accepted.
use super::*;
use glam::{Quat, Vec3};
use std::time::{Duration, Instant};

mod build;
mod pose;
mod shared;

#[derive(Default)]
pub(super) struct State {
    build: build::State,
    pose: pose::State,
    shared: shared::State,
}
pub(super) fn supports(t: Tool) -> bool {
    build::supports(t) || pose::supports(t) || shared::supports(t)
}
pub(super) fn handles(state: &State, c: &Capture) -> bool {
    supports(c.tool) || c.tool == Tool::ObjectOperationStatus && build::owns_rez(&state.build, c)
}
pub(super) fn original_receipt(state: &State, c: &Capture) -> Result<Option<Capture>, &'static str> {
    if c.tool == Tool::ObjectOperationStatus && build::owns_rez(&state.build, c) {
        build::original_receipt(&state.build, c)
    } else {
        Ok(None)
    }
}
pub(super) fn rez_receipt(app: &App, state: &State, c: &Capture, operation: Uuid) -> Result<Value, &'static str> {
    ready(app)?;
    build::rez_receipt(app, &state.build, c, operation)
}
pub(super) fn validate_receipt(app: &App, state: &State, c: &Capture) -> bool {
    if c.tool == Tool::ObjectOperationStatus {
        return uuid(&c.arguments, "operation_id").is_ok_and(|id| rez_receipt(app, state, c, id).is_ok());
    }
    if ready(app).is_err() {
        return false;
    }
    if build::supports(c.tool) || c.tool == Tool::ObjectOperationStatus {
        build::validate_receipt(app, &state.build, c)
    } else if pose::supports(c.tool) {
        pose::prepare(app, &state.pose, c).is_ok() || !c.tool.definition().read_only
    } else {
        shared::prepare(app, &state.shared, c).is_ok() || !c.tool.definition().read_only
    }
}
pub(super) fn prepare(app: &mut App, state: &State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    ready(app)?;
    if build::supports(c.tool) || c.tool == Tool::ObjectOperationStatus {
        build::prepare(app, &state.build, c)
    } else if pose::supports(c.tool) {
        pose::prepare(app, &state.pose, c)
    } else {
        shared::prepare(app, &state.shared, c)
    }
}
pub(super) fn evidence(app: &App, state: &State, c: &Capture) -> Result<Value, &'static str> {
    let detail = if build::supports(c.tool) || c.tool == Tool::ObjectOperationStatus {
        build::evidence(app, &state.build, c)?
    } else if pose::supports(c.tool) {
        pose::evidence(app, &state.pose, c)?
    } else {
        shared::evidence(app, &state.shared, c)?
    };
    Ok(json!({"policy":world::policy_stamp(app),"region":app.world.main_region,"detail":detail}))
}
pub(super) fn execute(app: &mut App, assistant: &mut Assistant, c: &Capture) -> Value {
    let result = if build::supports(c.tool) || c.tool == Tool::ObjectOperationStatus {
        build::execute(app, &mut assistant.creative.build, &assistant.runtime, c)
    } else if pose::supports(c.tool) {
        pose::execute(app, &mut assistant.creative.pose, &assistant.runtime, c)
    } else {
        shared::execute(app, &mut assistant.creative.shared, c)
    };
    result.unwrap_or_else(failure)
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    build::tick(app, assistant);
    pose::tick(app, assistant);
    shared::tick(app, assistant);
}
pub(super) fn scene(app: &mut App, assistant: &mut Assistant) {
    pose::sync(app, assistant);
    shared::native_tick(app, assistant);
}
pub(super) fn ui(app: &mut App, state: &mut State, ctx: &egui::Context, p: &crate::theme::Palette) {
    shared::ui(app, &mut state.shared, ctx, p);
}
fn ready(app: &App) -> Result<(), &'static str> {
    if !matches!(app.screen, Screen::World)
        || app.demo
        || app.world.teleporting
        || !app.world.movement_complete
        || app.world.main_region.is_none()
        || app.world.agent_id.is_nil()
        || app
            .world
            .login
            .as_ref()
            .is_none_or(|l| l.agent_id != app.world.agent_id || l.session_id.is_nil())
    {
        Err("native_not_ready")
    } else {
        Ok(())
    }
}
fn args(c: &Capture, keys: &[&str]) -> Result<(), &'static str> {
    if c.arguments
        .keys()
        .any(|k| !keys.contains(&k.as_str()) && !matches!(k.as_str(), "viewer_session" | "viewer_pid" | "request_id"))
    {
        Err("invalid_args")
    } else {
        Ok(())
    }
}
fn same_source(a: &Capture, b: &Capture) -> bool {
    a.client == b.client && a.endpoint == b.endpoint && a.binding == b.binding && a.policy_revision == b.policy_revision
}
fn vecn<const N: usize>(a: &serde_json::Map<String, Value>, key: &str, min: f64, max: f64) -> Result<[f32; N], &'static str> {
    let values = a
        .get(key)
        .and_then(Value::as_array)
        .filter(|v| v.len() == N)
        .ok_or("invalid_args")?;
    let mut out = [0.; N];
    for (out, value) in out.iter_mut().zip(values) {
        let n = value
            .as_f64()
            .filter(|n| n.is_finite() && (min..=max).contains(n))
            .ok_or("invalid_args")?;
        *out = n as f32;
    }
    Ok(out)
}
fn quaternion(a: &serde_json::Map<String, Value>, key: &str) -> Result<Quat, &'static str> {
    let q = Quat::from_array(vecn(a, key, -1.001, 1.001)?);
    if (q.length_squared() - 1.).abs() > 0.002 {
        return Err("invalid_quaternion");
    }
    let q = q.normalize();
    Ok(if q.w < 0. { -q } else { q })
}
fn scalar(a: &serde_json::Map<String, Value>, key: &str, min: f64, max: f64) -> Result<f32, &'static str> {
    a.get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && (min..=max).contains(v))
        .map(|v| v as f32)
        .ok_or("invalid_args")
}
fn submitted(c: &Capture) -> Value {
    json!({"status":"submitted","operation_id":c.request_id,"simulator_confirmed":false,"retry_safe":false})
}

#[cfg(test)]
pub(super) fn synthetic_capture(tool: Tool) -> Capture {
    use aurora_session::{GridKey, InstallationId};
    Capture {
        client: 1,
        endpoint: HostId::from_bytes([7; 16]).unwrap(),
        request_id: Uuid::new_v4().to_string(),
        binding: Binding {
            worker: WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).unwrap(),
                host: HostId::from_bytes([2; 16]).unwrap(),
                worker: WorkerId::from_bytes([3; 16]).unwrap(),
            },
            session: Some(SessionIdentity {
                session: SessionId::from_bytes([4; 16]).unwrap(),
                account: AccountKey::from_bytes([5; 32]).unwrap(),
                grid: GridKey::from_bytes([6; 32]).unwrap(),
                generation: 1,
            }),
        },
        tool,
        arguments: serde_json::Map::new(),
        policy_revision: 10,
        required_classes: vec![Class::WorldRead],
    }
}
