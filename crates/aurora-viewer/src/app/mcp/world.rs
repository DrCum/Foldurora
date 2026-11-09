//! Account-bound native observation and actions. The owning App thread alone
//! touches viewer state; protected asset IO and image encoding use its runtime.
//! Behavior reference: Folderstorm abbe3cb0 (fsassistant*), LGPL-2.1-or-later.
use super::*;
use aurora_net::{NetCommand, NetEvent};
use glam::{DVec3, Vec3};
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

mod camera;
mod gestures;
mod navigation;
mod observation;
mod social;
pub(super) use camera::EncoderLease as ImageEncoderLease;
pub(super) fn acquire_image_encoder() -> Option<std::sync::Arc<ImageEncoderLease>> {
    camera::acquire_encoder()
}
pub(super) fn image_encoder_available() -> bool {
    camera::encoder_available()
}
pub(super) fn capture_privacy(app: &App) -> Result<(), &'static str> {
    world_read(app)
}

const ROWS: usize = 100;
const SNAPSHOTS: usize = 16;
const RETENTION: usize = 512;

pub(super) struct State {
    binding: Option<Binding>,
    endpoint: Option<HostId>,
    runtime_revision: u64,
    policy: [u64; 5],
    authorization: Value,
    epoch: Uuid,
    social: social::State,
    navigation: navigation::State,
    camera: camera::State,
    gestures: gestures::State,
    snapshots: BTreeMap<Uuid, Snapshot>,
    events: VecDeque<NativeEvent>,
    private_events: BTreeMap<(HostId, u64), EventLane>,
    serial: u64,
    notifications: BTreeMap<u64, Uuid>,
    notification_fingerprint: BTreeMap<u64, String>,
    friends_fingerprint: String,
    audio_since: Instant,
    region: Option<u64>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            binding: None,
            endpoint: None,
            runtime_revision: 0,
            policy: [0; 5],
            authorization: Value::Null,
            epoch: Uuid::new_v4(),
            social: Default::default(),
            navigation: Default::default(),
            camera: Default::default(),
            gestures: Default::default(),
            snapshots: BTreeMap::new(),
            events: VecDeque::new(),
            private_events: BTreeMap::new(),
            serial: 0,
            notifications: BTreeMap::new(),
            notification_fingerprint: BTreeMap::new(),
            friends_fingerprint: String::new(),
            audio_since: Instant::now(),
            region: None,
        }
    }
}
#[derive(Clone)]
struct Snapshot {
    client: u64,
    endpoint: HostId,
    kind: &'static str,
    query: String,
    rows: Vec<Value>,
    limited: bool,
    expires: Instant,
}
#[derive(Clone)]
struct NativeEvent {
    serial: u64,
    topic: &'static str,
    value: Value,
}
#[derive(Default)]
struct EventLane {
    serial: u64,
    events: VecDeque<NativeEvent>,
}

pub(super) fn supports(tool: Tool) -> bool {
    social::supports(tool)
        || observation::supports(tool)
        || navigation::supports(tool)
        || camera::supports(tool)
        || gestures::supports(tool)
}
pub(super) fn validate_receipt(app: &App, state: &State, c: &Capture) -> bool {
    !gestures::supports(c.tool) || gestures::validate_receipt(app, state, c)
}
pub(super) fn policy_stamp(app: &App) -> [u64; 5] {
    [
        app.world.rlva.revision,
        app.world.mutes.version,
        app.world.contact_policy_revision(),
        app.world.derender.generation,
        app.world.parcel_visibility.version,
    ]
}
pub(super) fn observe_policy(runtime: &Runtime, previous: &mut Option<[u64; 5]>, stamp: [u64; 5]) {
    if previous.is_some_and(|old| old != stamp) {
        runtime.native_policy_changed();
    }
    *previous = Some(stamp);
}
pub(super) fn prepare(app: &mut App, state: &State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    native_ready(app)?;
    if social::supports(c.tool) {
        social::prepare(app, state, c)
    } else if observation::supports(c.tool) {
        observation::prepare(app, c)
    } else if navigation::supports(c.tool) {
        navigation::prepare(app, state, c)
    } else if camera::supports(c.tool) {
        camera::prepare(app, state, c)
    } else if gestures::supports(c.tool) {
        gestures::prepare(app, state, c)
    } else {
        Err("unsupported_capability")
    }
}
/// Evidence is captured before Ask and compared again immediately before native
/// submission. A UUID, asset or conversation incarnation cannot change under a
/// permission grant which described an earlier native target.
pub(super) fn evidence(app: &App, state: &State, c: &Capture) -> Result<Value, &'static str> {
    let detail = if social::supports(c.tool) {
        social::evidence(app, state, c)?
    } else if navigation::supports(c.tool) {
        navigation::evidence(app, state, c)?
    } else if camera::supports(c.tool) {
        camera::evidence(app, state, c)?
    } else if gestures::supports(c.tool) {
        gestures::evidence(app, state, c)?
    } else {
        Value::Null
    };
    Ok(json!({"policy":policy_stamp(app),"region":app.world.main_region,"detail":detail}))
}
pub(super) fn execute(app: &mut App, assistant: &mut Assistant, c: &Capture) -> Value {
    let result = if social::supports(c.tool) {
        social::execute(app, assistant, c)
    } else if observation::supports(c.tool) {
        observation::execute(app, &mut assistant.world, c)
    } else if navigation::supports(c.tool) {
        navigation::execute(app, assistant, c)
    } else if camera::supports(c.tool) {
        camera::execute(app, assistant, c)
    } else if gestures::supports(c.tool) {
        gestures::execute(app, assistant, c)
    } else {
        Err("unsupported_capability")
    };
    result.unwrap_or_else(failure)
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    let now = Instant::now();
    let authorization = serde_json::to_value(&app.settings.local_assistant).unwrap_or(Value::Null);
    let policy = policy_stamp(app);
    let endpoint = assistant.runtime.snapshot().viewer_session;
    let runtime_revision = assistant.runtime.policy_revision();
    if assistant.world.binding != assistant.binding
        || assistant.world.endpoint != endpoint
        || assistant.world.runtime_revision != runtime_revision
        || assistant.world.policy != policy
        || assistant.world.authorization != authorization
    {
        navigation::cancel(app, assistant, "source_or_policy_changed");
        gestures::cancel(app, assistant, "source_or_policy_changed");
        let old = std::mem::take(&mut assistant.world);
        // Pending IO keeps its captured ownership and is drained without
        // exporting it into a later login. No private result is carried over.
        assistant.world.camera = old.camera;
        assistant.world.gestures.io = old.gestures.io;
        assistant.world.binding = assistant.binding;
        assistant.world.endpoint = endpoint;
        assistant.world.runtime_revision = runtime_revision;
        assistant.world.policy = policy;
        assistant.world.authorization = authorization;
        social::baseline(app, &mut assistant.world);
    }
    if assistant.world.region != app.world.main_region {
        assistant.world.region = app.world.main_region;
        assistant.world.audio_since = now;
    }
    assistant.world.snapshots.retain(|_, s| s.expires > now);
    social::tick(app, assistant);
    navigation::tick(app, assistant);
    camera::tick(app, assistant);
    gestures::tick(app, assistant);
}
/// Retire only assistant-owned active work. This path performs cancellation
/// and cleanup without running observation or starting protected native IO.
pub(super) fn retire(app: &mut App, assistant: &mut Assistant) {
    navigation::cancel(app, assistant, "invalid_configuration");
    gestures::cancel(app, assistant, "invalid_configuration");
}
pub(super) fn native_event(app: &mut App, event: &NetEvent) -> bool {
    gestures::native_event(app, event);
    social::native_event(app, event)
}
pub(super) fn native_command(app: &mut App, command: &NetCommand) {
    gestures::native_command(app, command);
}
pub(super) fn snapshot_current(app: &App, id: Uuid) -> bool {
    camera::valid(app, id)
}
pub(super) fn snapshot_pixels(app: &mut App, id: Uuid, size: (u32, u32), rgba: Vec<u8>) {
    camera::pixels(app, id, size, rgba);
}

fn native_ready(app: &App) -> Result<(), &'static str> {
    if !matches!(app.screen, Screen::World)
        || !app.world.movement_complete
        || app.world.agent_id.is_nil()
        || app
            .world
            .login
            .as_ref()
            .is_none_or(|l| l.agent_id != app.world.agent_id || l.session_id.is_nil())
    {
        return Err("native_not_ready");
    }
    Ok(())
}
fn private_read(app: &App) -> Result<(), &'static str> {
    if !app.world.mutes.is_loaded() || !app.world.shared_chat_privacy_allowed() {
        Err("native_privacy_restriction")
    } else {
        Ok(())
    }
}
fn world_read(app: &App) -> Result<(), &'static str> {
    if app.world.rlva.has("showloc") || app.world.rlva.has("shownearby") || app.world.rlva.has("shownames") {
        Err("native_privacy_restriction")
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
fn boolean(a: &serde_json::Map<String, Value>, key: &str, default: bool) -> Result<bool, &'static str> {
    a.get(key).map_or(Ok(default), |v| v.as_bool().ok_or("invalid_args"))
}
fn count(a: &serde_json::Map<String, Value>, key: &str, default: usize, max: usize) -> Result<usize, &'static str> {
    a.get(key).map_or(Ok(default), |v| {
        v.as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n <= max)
            .ok_or("invalid_args")
    })
}
fn finite(a: &serde_json::Map<String, Value>, key: &str, default: f64, min: f64, max: f64) -> Result<f64, &'static str> {
    let n = a.get(key).map_or(Some(default), Value::as_f64).ok_or("invalid_args")?;
    if n.is_finite() && (min..=max).contains(&n) {
        Ok(n)
    } else {
        Err("invalid_args")
    }
}
fn vector(a: &serde_json::Map<String, Value>, key: &str) -> Result<DVec3, &'static str> {
    let v = a
        .get(key)
        .and_then(Value::as_array)
        .filter(|v| v.len() == 3)
        .ok_or("invalid_args")?;
    let value = DVec3::new(
        v[0].as_f64().ok_or("invalid_args")?,
        v[1].as_f64().ok_or("invalid_args")?,
        v[2].as_f64().ok_or("invalid_args")?,
    );
    if value.is_finite() { Ok(value) } else { Err("invalid_args") }
}
fn xyz(v: Vec3) -> Value {
    json!([v.x, v.y, v.z])
}
fn global(v: DVec3) -> Value {
    json!([v.x, v.y, v.z])
}
fn clip(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end]
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
        .collect()
}
fn current(app: &App, a: &Assistant, c: &Capture) -> bool {
    super::binding_matches(app, a, c)
        && a.runtime.configuration().as_ref() == Some(&app.settings.local_assistant)
        && a.runtime.capture_current(c)
        && super::original_scope_current(app, a, c)
}
fn hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
fn submitted(c: &Capture) -> Value {
    json!({"status":"submitted","operation_id":c.request_id,"confirmed":false,"retry_safe":false})
}
fn push_event(state: &mut State, topic: &'static str, value: Value) {
    debug_assert!(topic != "operations", "private operation receipts require an authenticated owner");
    if let Some(serial) = state.serial.checked_add(1) {
        state.serial = serial;
        state.events.push_back(NativeEvent { serial, topic, value });
        while state.events.len() > RETENTION {
            state.events.pop_front();
        }
    }
}
fn push_operation(state: &mut State, c: &Capture, value: Value) {
    let owner = (c.endpoint, c.client);
    if state.private_events.len() >= 16 && !state.private_events.contains_key(&owner) {
        return;
    }
    let lane = state.private_events.entry(owner).or_default();
    if let Some(serial) = lane.serial.checked_add(1) {
        lane.serial = serial;
        lane.events.push_back(NativeEvent {
            serial,
            topic: "operations",
            value,
        });
        while lane.events.len() > RETENTION {
            lane.events.pop_front();
        }
    }
}
fn snapshot_page(
    state: &mut State,
    c: &Capture,
    kind: &'static str,
    query: &str,
    rows: Vec<Value>,
    limited: bool,
) -> Result<Value, &'static str> {
    let offset = count(&c.arguments, "offset", 0, 2048)?;
    let limit = limit(&c.arguments, 50, ROWS)?;
    let id = match c.arguments.get("snapshot_id") {
        Some(v) => Uuid::parse_str(v.as_str().ok_or("invalid_args")?).map_err(|_| "invalid_args")?,
        None => {
            if offset != 0 {
                return Err("snapshot_required");
            }
            if state.snapshots.len() >= SNAPSHOTS {
                return Err("snapshot_capacity");
            }
            let id = Uuid::new_v4();
            state.snapshots.insert(
                id,
                Snapshot {
                    client: c.client,
                    endpoint: c.endpoint,
                    kind,
                    query: query.into(),
                    rows,
                    limited,
                    expires: Instant::now() + Duration::from_secs(60),
                },
            );
            id
        }
    };
    let s = state
        .snapshots
        .get(&id)
        .filter(|s| s.client == c.client && s.endpoint == c.endpoint && s.kind == kind && s.query == query && s.expires > Instant::now())
        .ok_or("stale_snapshot")?;
    let mut items = Vec::new();
    let mut bytes = 0;
    for row in s.rows.iter().skip(offset).take(limit) {
        let size = serde_json::to_vec(row).map_err(|_| "native_metadata_limit")?.len();
        if bytes + size > 18 * 1024 {
            break;
        }
        bytes += size;
        items.push(row.clone());
    }
    let next = offset + items.len();
    Ok(
        json!({"snapshot_id":id.to_string(),"items":items,"total":s.rows.len(),"offset":offset,"next_offset":if next<s.rows.len(){Some(next)}else{None},"limited":s.limited,"untrusted_data":true}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_mcp::assistant::runtime::ViewerRequest;
    pub(super) fn capture(tool: Tool) -> Capture {
        super::super::inventory::synthetic_capture(tool)
    }
    fn engine(tool: Tool) -> (Runtime, Capture) {
        let seed = capture(tool);
        let runtime = Runtime::default();
        runtime.observe(Some(seed.binding), true);
        let mut config = aurora_mcp::assistant::permissions::Configuration::default();
        for class in [Class::Read, Class::WorldRead, Class::Camera] {
            config.permissions.set(class, Level::Allow).unwrap();
        }
        runtime.configure(config).unwrap();
        runtime.capabilities([tool]);
        runtime.enable(seed.binding, seed.endpoint).unwrap();
        let v = runtime.request(
            seed.client,
            ViewerRequest::Invoke {
                viewer_session: seed.endpoint,
                tool,
                arguments: seed.arguments,
                request_id: Some(Uuid::new_v4().to_string()),
            },
        );
        let id = v["request_id"].as_str().unwrap();
        runtime.prepared(id, &[], tool.definition().active).unwrap();
        let c = runtime.ready().pop().unwrap();
        (runtime, c)
    }
    #[test]
    fn native_visibility_changes_retire_private_rows_camera_artifacts_and_inline_preview() {
        for index in [3, 4] {
            for tool in [Tool::WorldNearbyObjects, Tool::CameraSnapshot] {
                let (runtime, c) = engine(tool);
                let mut observed = None;
                let mut stamp = [0; 5];
                observe_policy(&runtime, &mut observed, stamp);
                runtime.execute(&c.request_id, |_| json!({"status":"submitted"})).unwrap();
                let image = Uuid::new_v4();
                if tool == Tool::CameraSnapshot {
                    runtime.install_artifact(&c, image, vec![255, 216, 1, 2, 255, 217]).unwrap();
                    runtime.install_camera_preview(&c, image, 1, 1, vec![255, 216, 255, 217]).unwrap();
                }
                runtime
                    .complete(&c, json!({"private_rows":["nearby object"],"artifact_id":image.to_string()}))
                    .unwrap();
                let poll = || {
                    runtime.request(
                        c.client,
                        ViewerRequest::OperationStatus {
                            viewer_session: c.endpoint,
                            request_id: c.request_id.clone(),
                        },
                    )
                };
                assert!(poll()["result"]["private_rows"].is_array());
                let image_read = || {
                    runtime.request(
                        c.client,
                        ViewerRequest::Image {
                            viewer_session: c.endpoint,
                            request_id: c.request_id.clone(),
                            artifact_id: image.to_string(),
                        },
                    )
                };
                if tool == Tool::CameraSnapshot {
                    assert_eq!(image_read()["mime_type"], "image/jpeg");
                }
                runtime.begin_native_update();
                stamp[index] += 1;
                observe_policy(&runtime, &mut observed, stamp);
                runtime.end_native_update();
                assert!(poll().get("result").is_none());
                assert!(image_read().get("data_base64").is_none());
                let artifact = runtime.request(
                    c.client,
                    ViewerRequest::Artifact {
                        viewer_session: c.endpoint,
                        request_id: c.request_id.clone(),
                        artifact_id: image.to_string(),
                        offset: 0,
                        length: 8,
                    },
                );
                assert!(artifact.get("data").is_none());
            }
        }
    }
}
