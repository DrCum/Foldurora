//! Private rotation transactions, port of fsposelistener.cpp/fsposepolicy.h
//! (Folderstorm abbe3cb0, LGPL-2.1-or-later). Native manual history is separate.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub(super) use crate::ik_edit::poser::rotation_preset::supported;
use crate::ik_edit::poser::rotation_preset::{Rotations, encode, parse};
const FILE_LIMIT: usize = 65_536;
#[derive(Default)]
pub(super) struct State {
    sessions: BTreeMap<u64, Session>,
    presets: BTreeMap<u64, (Capture, Vec<Preset>)>,
    io: Option<(crossbeam_channel::Sender<Reply>, crossbeam_channel::Receiver<Reply>)>,
    pending: BTreeMap<String, Capture>,
    outcomes: BTreeMap<(u64, Uuid), (Capture, Value)>,
}
struct Session {
    epoch: Arc<AtomicU64>,
    id: Uuid,
    source: Capture,
    region: Option<u64>,
    active: bool,
    revision: u32,
    rotations: Rotations,
    history: Vec<Rotations>,
    stop_reason: &'static str,
}
#[derive(Clone)]
struct Preset {
    id: String,
    name: String,
    path: PathBuf,
    revision: String,
    rotations: Result<Rotations, &'static str>,
    bytes: Vec<u8>,
}
enum Work {
    Presets,
    Load { expected: String, path: PathBuf, bytes: Vec<u8> },
    Save { name: String, bytes: Vec<u8> },
}
enum Output {
    Presets(Vec<Preset>),
    Load(Rotations),
    Saved { id: String, revision: String },
    Failed(&'static str),
}
struct Reply {
    capture: Capture,
    output: Output,
}
pub(super) fn supports(t: Tool) -> bool {
    matches!(
        t,
        Tool::PoseStatus
            | Tool::PoseJoints
            | Tool::PosePresets
            | Tool::PoseStart
            | Tool::PoseUpdate
            | Tool::PoseUndo
            | Tool::PoseReset
            | Tool::PoseStop
            | Tool::PoseLoad
            | Tool::PoseSave
    )
}

pub(super) fn rotations(app: &App, value: &Value) -> Result<Rotations, &'static str> {
    let values = value
        .as_object()
        .filter(|v| !v.is_empty() && v.len() <= 64)
        .ok_or("rotation_joint_limit")?;
    let mut result = Rotations::new();
    for (name, q) in values {
        if !supported(name) || !app.world.avatar_lib.rig.names.contains_key(name) {
            return Err("unsupported_pose_joint");
        }
        let mut a = serde_json::Map::new();
        a.insert("q".into(), q.clone());
        result.insert(name.clone(), quaternion(&a, "q")?);
    }
    Ok(result)
}
fn value(rotations: &Rotations) -> Value {
    json!(rotations.iter().map(|(n, q)| (n, q.to_array())).collect::<BTreeMap<_, _>>())
}
fn native_conflict(app: &App) -> bool {
    app.ik_editor.posing() || app.scene.ik_edit.active || app.world.shared_pose.is_shared(app.world.agent_id)
}
fn live(app: &App, c: &Capture) -> Result<(), &'static str> {
    if uuid(&c.arguments, "target")? != app.world.agent_id || crate::ik_edit::Frame::new(&app.world).is_none() {
        return Err("explicit_live_self_required");
    }
    Ok(())
}
fn session<'a>(app: &App, s: &'a State, c: &Capture) -> Result<&'a Session, &'static str> {
    let session = s
        .sessions
        .get(&c.client)
        .filter(|s| same_source(&s.source, c) && s.region == app.world.main_region)
        .ok_or("pose_status_required")?;
    if uuid(&c.arguments, "pose_session")? != session.id
        || c.arguments.get("expected_revision").and_then(Value::as_u64) != Some(u64::from(session.revision))
    {
        return Err("pose_revision_conflict");
    }
    Ok(session)
}
fn next(session: &Session, changes: &Rotations) -> Result<Rotations, &'static str> {
    let mut current = session.rotations.clone();
    current.extend(changes.iter().map(|(n, q)| (n.clone(), *q)));
    if current.len() > 64 {
        return Err("rotation_joint_limit");
    }
    Ok(current)
}
fn preset<'a>(s: &'a State, c: &Capture) -> Result<&'a Preset, &'static str> {
    let id = string(&c.arguments, "preset_id", 71)?;
    let revision = string(&c.arguments, "preset_revision", 64)?;
    s.presets
        .get(&c.client)
        .filter(|(source, _)| same_source(source, c))
        .and_then(|(_, p)| p.iter().find(|p| p.id == id && p.revision == revision))
        .ok_or("preset_revision_conflict")
}
pub(super) fn prepare(app: &App, s: &State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    live(app, c)?;
    if matches!(c.tool, Tool::PosePresets | Tool::PoseLoad | Tool::PoseSave) && s.pending.len() >= 4 {
        return Err("pose_io_capacity");
    }
    if c.tool == Tool::PosePresets && !s.presets.contains_key(&c.client) && s.presets.len() >= 8 {
        return Err("pose_preset_cache_capacity");
    }
    if matches!(c.tool, Tool::PoseStatus | Tool::PoseJoints)
        && !s.sessions.contains_key(&c.client)
        && s.sessions.len() >= 64
        && s.sessions.values().all(|s| s.active)
    {
        return Err("pose_session_capacity");
    }
    let mut keys = vec!["target"];
    if matches!(c.tool, Tool::PoseStatus | Tool::PoseJoints | Tool::PosePresets) {
        if c.tool == Tool::PoseStatus {
            keys.push("operation_id");
            if c.arguments.contains_key("operation_id") {
                uuid(&c.arguments, "operation_id")?;
            }
        }
        args(c, &keys)?;
        return Ok(vec![Class::PoseRead]);
    }
    keys.extend(["pose_session", "expected_revision"]);
    match c.tool {
        Tool::PoseUpdate => keys.push("rotations"),
        Tool::PoseLoad => keys.extend(["preset_id", "preset_revision"]),
        Tool::PoseSave => keys.extend(["name", "expected_preset_revision"]),
        _ => {}
    }
    args(c, &keys)?;
    if native_conflict(app) {
        return Err("native_manual_or_shared_pose_owns_avatar");
    }
    let current = session(app, s, c)?;
    if c.tool == Tool::PoseStart {
        if current.active || s.sessions.values().any(|s| s.active) {
            return Err("private_pose_already_active");
        }
    } else {
        if !current.active {
            return Err("source_owned_pose_not_active");
        }
        match c.tool {
            Tool::PoseUpdate => {
                next(current, &rotations(app, &c.arguments["rotations"])?)?;
            }
            Tool::PoseLoad => {
                next(current, preset(s, c)?.rotations.as_ref().map_err(|e| *e)?)?;
            }
            Tool::PoseUndo if current.history.is_empty() => return Err("private_pose_history_empty"),
            Tool::PoseSave => {
                let name = string(&c.arguments, "name", 64)?;
                if name.is_empty()
                    || !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
                    || string(&c.arguments, "expected_preset_revision", 6)? != "absent"
                    || current.rotations.is_empty()
                {
                    return Err("new_confined_preset_required");
                }
            }
            _ => {}
        }
    }
    if s.pending.len() >= 4 {
        return Err("pose_io_capacity");
    }
    Ok(vec![Class::PoseRead])
}
fn status(app: &App, session: &Session) -> Value {
    json!({"target":app.world.agent_id,"pose_session":session.id,"revision":session.revision,"active":session.active,"native_poser_active":app.ik_editor.posing(),
        "self_shared":app.world.shared_pose.is_shared(app.world.agent_id),"rotation_only":true,"reference_frame":"local","visibility":"local_viewer","restoration_complete":true,
        "can_undo":!session.history.is_empty(),"owned_joints":value(&session.rotations),"stop_reason":session.stop_reason})
}
pub(super) fn evidence(app: &App, s: &State, c: &Capture) -> Result<Value, &'static str> {
    if matches!(c.tool, Tool::PoseStatus | Tool::PoseJoints | Tool::PosePresets) {
        return Ok(Value::Null);
    }
    let current = session(app, s, c)?;
    let effect = match c.tool {
        Tool::PoseUpdate => value(&next(current, &rotations(app, &c.arguments["rotations"])?)?),
        Tool::PoseLoad => value(&next(current, preset(s, c)?.rotations.as_ref().map_err(|e| *e)?)?),
        Tool::PoseUndo => current.history.last().map(value).unwrap_or(Value::Null),
        Tool::PoseReset => json!({}),
        _ => value(&current.rotations),
    };
    Ok(
        json!({"state":status(app,current),"effect_rotations":effect,"preset":if c.tool==Tool::PoseLoad{Some(preset(s,c)?.revision.clone())}else{None}}),
    )
}
fn checkpoint(session: &mut Session) {
    session.history.push(session.rotations.clone());
    if session.history.len() > 20 {
        session.history.remove(0);
    }
    session.revision = session.revision.saturating_add(1);
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn filename_id(name: &str) -> String {
    format!("preset-{}", hash(name.as_bytes()))
}
fn no_links(path: &Path) -> Result<(), &'static str> {
    let mut p = PathBuf::new();
    for part in path.components() {
        p.push(part.as_os_str());
        match fs::symlink_metadata(&p) {
            Ok(m) if m.file_type().is_symlink() => return Err("pose_store_symlink_refused"),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("pose_store_unavailable"),
        }
    }
    Ok(())
}
fn read(path: &Path) -> Result<Vec<u8>, &'static str> {
    no_links(path)?;
    let before = fs::symlink_metadata(path).map_err(|_| "pose_file_unavailable")?;
    if !before.is_file() || before.len() == 0 || before.len() > FILE_LIMIT as u64 {
        return Err("pose_file_limit");
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x20000);
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x100);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let mut file = options.open(path).map_err(|_| "pose_file_unavailable")?;
    let opened = file.metadata().map_err(|_| "pose_file_unavailable")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != opened.dev() || before.ino() != opened.ino() {
            return Err("pose_file_changed");
        }
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(FILE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "pose_file_unavailable")?;
    let after = file.metadata().map_err(|_| "pose_file_unavailable")?;
    no_links(path)?;
    if bytes.len() > FILE_LIMIT || before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err("pose_file_changed");
    }
    Ok(bytes)
}

fn work(root: PathBuf, work: Work, runtime: &Runtime, capture: &Capture, epoch: Option<(Arc<AtomicU64>, u64)>) -> Output {
    let result = (|| -> Result<Output, &'static str> {
        no_links(&root)?;
        match work {
            Work::Presets => {
                let mut entries = Vec::new();
                if !root.exists() {
                    return Ok(Output::Presets(entries));
                }
                for entry in fs::read_dir(&root).map_err(|_| "pose_store_unavailable")?.take(257) {
                    if entries.len() == 256 {
                        return Err("pose_preset_capacity");
                    }
                    let entry = entry.map_err(|_| "pose_store_unavailable")?;
                    let path = entry.path();
                    if path.extension().is_none_or(|s| s != "xml")
                        && !path.file_name().is_some_and(|s| s.to_string_lossy().ends_with(".aurora-pose.json"))
                    {
                        continue;
                    }
                    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    let bytes = read(&path)?;
                    entries.push(Preset {
                        id: filename_id(name),
                        name: name.trim_end_matches(".xml").to_string(),
                        revision: hash(&bytes),
                        rotations: parse(&bytes),
                        bytes,
                        path,
                    });
                }
                Ok(Output::Presets(entries))
            }
            Work::Load { expected, path, bytes } => {
                if path.parent() != Some(root.as_path()) || hash(&read(&path)?) != expected || hash(&bytes) != expected {
                    return Err("preset_revision_conflict");
                }
                Ok(Output::Load(parse(&bytes)?))
            }
            Work::Save { name, bytes } => {
                fs::create_dir_all(&root).map_err(|_| "pose_store_unavailable")?;
                no_links(&root)?;
                let file_name = format!("{name}.xml");
                let path = root.join(&file_name);
                if epoch
                    .as_ref()
                    .is_none_or(|(epoch, expected)| epoch.load(Ordering::Acquire) != *expected)
                {
                    return Err("pose_revision_conflict");
                }
                runtime.continue_action(capture, || ()).map_err(|_| "stale_source")?;
                let mut file = fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&path)
                    .map_err(|_| "preset_collision_or_store_unavailable")?;
                file.write_all(&bytes)
                    .and_then(|_| file.sync_all())
                    .map_err(|_| "preset_save_unconfirmed")?;
                Ok(Output::Saved {
                    id: filename_id(&file_name),
                    revision: hash(&bytes),
                })
            }
        }
    })();
    result.unwrap_or_else(Output::Failed)
}
fn queue(app: &mut App, s: &mut State, runtime: &Runtime, c: &Capture, job: Work) {
    let tx = s.io.get_or_insert_with(|| crossbeam_channel::bounded(8)).0.clone();
    let capture = c.clone();
    let runtime = runtime.clone();
    let epoch = if matches!(&job, Work::Save { .. }) {
        s.sessions
            .get(&c.client)
            .map(|s| (s.epoch.clone(), s.epoch.load(Ordering::Acquire)))
    } else {
        None
    };
    let root = crate::settings::account_dir(&app.world.agent_id).join("poses");
    s.pending.insert(c.request_id.clone(), c.clone());
    app.scene.jobs.execute(move || {
        let output = work(root, job, &runtime, &capture, epoch);
        let _ = tx.try_send(Reply { capture, output });
    });
}
pub(super) fn execute(app: &mut App, s: &mut State, runtime: &Runtime, c: &Capture) -> Result<Value, &'static str> {
    let result = execute_inner(app, s, runtime, c)?;
    if !matches!(c.tool, Tool::PoseStatus | Tool::PoseJoints | Tool::PosePresets) {
        if s.outcomes.len() >= 64
            && let Some(key) = s.outcomes.keys().next().copied()
        {
            s.outcomes.remove(&key);
        }
        if let Ok(id) = Uuid::parse_str(&c.request_id) {
            s.outcomes.insert((c.client, id), (c.clone(), result.clone()));
        }
    }
    Ok(result)
}
fn execute_inner(app: &mut App, s: &mut State, runtime: &Runtime, c: &Capture) -> Result<Value, &'static str> {
    if c.tool == Tool::PosePresets {
        queue(app, s, runtime, c, Work::Presets);
        return Ok(submitted(c));
    }
    if c.tool == Tool::PoseStatus || c.tool == Tool::PoseJoints {
        if c.arguments.contains_key("operation_id") {
            return s
                .outcomes
                .get(&(c.client, uuid(&c.arguments, "operation_id")?))
                .filter(|(source, _)| same_source(source, c))
                .map(|(_, v)| v.clone())
                .ok_or("pose_operation_not_owned");
        }
        if !s.sessions.contains_key(&c.client) && s.sessions.len() >= 64 {
            if let Some(key) = s.sessions.iter().find(|(_, s)| !s.active).map(|(id, _)| *id) {
                s.sessions.remove(&key);
            } else {
                return Err("pose_session_capacity");
            }
        }
        let current = s.sessions.entry(c.client).or_insert_with(|| Session {
            epoch: Arc::new(AtomicU64::new(1)),
            id: Uuid::new_v4(),
            source: c.clone(),
            region: app.world.main_region,
            active: false,
            revision: 0,
            rotations: Rotations::new(),
            history: Vec::new(),
            stop_reason: "",
        });
        if !same_source(&current.source, c) || current.region != app.world.main_region {
            current.epoch.fetch_add(1, Ordering::AcqRel);
            *current = Session {
                epoch: Arc::new(AtomicU64::new(1)),
                id: Uuid::new_v4(),
                source: c.clone(),
                region: app.world.main_region,
                active: false,
                revision: 0,
                rotations: Rotations::new(),
                history: Vec::new(),
                stop_reason: "source_changed",
            };
        }
        let mut result = status(app, current);
        if c.tool == Tool::PoseJoints {
            let values = crate::ik_edit::observed_rotations(&app.world, app.world.agent_id).ok_or("native_skeleton_unavailable")?;
            result["joints"] = json!(
                app.world
                    .avatar_lib
                    .rig
                    .canonical_names
                    .iter()
                    .zip(values)
                    .filter(|(n, _)| supported(n))
                    .map(|(n, q)| json!({"name":n,"rotation":q.to_array(),"frame":"local"}))
                    .collect::<Vec<_>>()
            );
        }
        return Ok(result);
    }
    if c.tool == Tool::PoseLoad {
        let p = preset(s, c)?.clone();
        queue(
            app,
            s,
            runtime,
            c,
            Work::Load {
                expected: p.revision,
                path: p.path,
                bytes: p.bytes,
            },
        );
        return Ok(submitted(c));
    }
    if c.tool == Tool::PoseSave {
        let current = session(app, s, c)?;
        let bytes = encode(&current.rotations);
        queue(
            app,
            s,
            runtime,
            c,
            Work::Save {
                name: string(&c.arguments, "name", 64)?.to_owned(),
                bytes,
            },
        );
        return Ok(submitted(c));
    }
    let changes = if c.tool == Tool::PoseUpdate {
        Some(rotations(app, &c.arguments["rotations"])?)
    } else {
        None
    };
    let current = s.sessions.get_mut(&c.client).ok_or("pose_status_required")?;
    current.epoch.fetch_add(1, Ordering::AcqRel);
    match c.tool {
        Tool::PoseStart => {
            current.id = Uuid::new_v4();
            current.revision = 0;
            current.active = true;
            current.rotations.clear();
            current.history.clear();
            current.stop_reason = "";
        }
        Tool::PoseStop => {
            current.active = false;
            current.stop_reason = "stopped_by_source";
        }
        Tool::PoseReset => {
            checkpoint(current);
            current.rotations.clear();
        }
        Tool::PoseUndo => {
            current.rotations = current.history.pop().ok_or("private_pose_history_empty")?;
            current.revision = current.revision.saturating_add(1);
        }
        Tool::PoseUpdate => {
            let next = next(current, &changes.ok_or("invalid_args")?)?;
            checkpoint(current);
            current.rotations = next;
        }
        _ => return Err("unsupported_capability"),
    }
    current.source = c.clone();
    let mut result = status(app, current);
    result["operation_id"] = json!(c.request_id);
    result["status"] = json!(if current.active { "submitted" } else { "stopped" });
    Ok(result)
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    let replies: Vec<_> = assistant
        .creative
        .pose
        .io
        .as_ref()
        .map(|(_, rx)| rx.try_iter().collect())
        .unwrap_or_default();
    for reply in replies {
        assistant.creative.pose.pending.remove(&reply.capture.request_id);
        if !binding_matches(app, assistant, &reply.capture) || !assistant.runtime.capture_current(&reply.capture) {
            continue;
        }
        let result = match reply.output {
            Output::Presets(presets) => {
                let rows:Vec<_>=presets.iter().map(|p|json!({"id":p.id,"name":p.name,"revision":p.revision,"supported":p.rotations.is_ok(),"reason":p.rotations.as_ref().err()})).collect();
                assistant
                    .creative
                    .pose
                    .presets
                    .insert(reply.capture.client, (reply.capture.clone(), presets));
                json!({"presets":rows})
            }
            Output::Load(changes) => {
                let current = assistant.creative.pose.sessions.get_mut(&reply.capture.client);
                if native_conflict(app) {
                    failure("native_manual_or_shared_pose_owns_avatar")
                } else if let Some(current) = current.filter(|s| {
                    same_source(&s.source, &reply.capture)
                        && s.active
                        && s.region == app.world.main_region
                        && uuid(&reply.capture.arguments, "pose_session").ok() == Some(s.id)
                }) {
                    let expected = reply.capture.arguments.get("expected_revision").and_then(Value::as_u64);
                    if expected != Some(current.revision as u64) {
                        failure("pose_revision_conflict")
                    } else if let Ok(next) = next(current, &changes) {
                        let runtime = assistant.runtime.clone();
                        if runtime
                            .continue_action(&reply.capture, || {
                                checkpoint(current);
                                current.epoch.fetch_add(1, Ordering::AcqRel);
                                current.rotations = next;
                                current.source = reply.capture.clone();
                            })
                            .is_ok()
                        {
                            let mut result = status(app, current);
                            result["status"] = json!("submitted");
                            result
                        } else {
                            failure("stale_source")
                        }
                    } else {
                        failure("rotation_joint_limit")
                    }
                } else {
                    failure("source_owned_pose_not_active")
                }
            }
            Output::Saved { id, revision } => json!({"saved":true,"preset_id":id,"preset_revision":revision}),
            Output::Failed(code) => failure(code),
        };
        // A loaded motion retains the approved continuation for every frame.
        if let Ok(id) = Uuid::parse_str(&reply.capture.request_id) {
            if assistant.creative.pose.outcomes.len() >= 64
                && !assistant.creative.pose.outcomes.contains_key(&(reply.capture.client, id))
                && let Some(key) = assistant.creative.pose.outcomes.keys().next().copied()
            {
                assistant.creative.pose.outcomes.remove(&key);
            }
            assistant
                .creative
                .pose
                .outcomes
                .insert((reply.capture.client, id), (reply.capture.clone(), result.clone()));
        }
        if reply.capture.tool != Tool::PoseLoad || result["status"] == "error" {
            let _ = assistant.runtime.complete(&reply.capture, result);
        }
    }
    sync(app, assistant);
}
pub(super) fn sync(app: &mut App, assistant: &mut Assistant) {
    app.scene.assistant_rotations.clear();
    let conflict = native_conflict(app);
    for session in assistant.creative.pose.sessions.values_mut() {
        if !session.active {
            continue;
        }
        if conflict
            || session.region != app.world.main_region
            || assistant.binding != Some(session.source.binding)
            || !assistant.runtime.capture_current(&session.source)
            || !active_allowed(app)
        {
            session.active = false;
            session.epoch.fetch_add(1, Ordering::AcqRel);
            session.stop_reason = "authority_or_native_ownership_changed";
            continue;
        }
        let channels: BTreeMap<_, _> = session
            .rotations
            .iter()
            .filter_map(|(n, q)| app.world.avatar_lib.rig.names.get(n).map(|b| (*b, *q)))
            .collect();
        let runtime = assistant.runtime.clone();
        let _ = runtime.continue_action(&session.source, || {
            app.scene.assistant_rotations.insert(app.world.agent_id, Arc::new(channels));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotation_presets_roundtrip_and_refuse_wider_state() {
        let pose = BTreeMap::from([("mWristLeft".into(), Quat::from_rotation_x(0.3))]);
        let encoded = encode(&pose);
        let decoded = parse(&encoded).unwrap();
        assert!(decoded["mWristLeft"].dot(pose["mWristLeft"]).abs() > 0.99999);
        let wider = String::from_utf8(encoded)
            .unwrap()
            .replace("<key>rotation</key>", "<key>position</key>");
        assert!(parse(wider.as_bytes()).is_err());
        assert!(!supported("mPelvis"));
        assert!(!supported("PELVIS"));
    }
    #[test]
    fn quaternion_joint_scope_is_closed() {
        for invalid in ["mRoot", "mAvatarRoot", "mPelvis", "mBad/path", "m", ""] {
            assert!(!supported(invalid));
        }
        assert!(supported("mHandIndex1Left"));
    }
}
