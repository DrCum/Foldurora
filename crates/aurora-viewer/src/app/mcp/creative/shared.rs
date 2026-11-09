// SPDX-License-Identifier: LGPL-2.1-or-later
//! Typed shared-pose operations and ordinary native owner consent.
//! Folderstorm abbe3cb0 fssharedposemanager.cpp / fssharedposepolicy.h.
use super::*;
use crate::world::shared_pose::{Joint, Pose};
use aurora_llsd::Llsd;
use aurora_net::bridge::{query::Query, shared_pose::decode_pose};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::Ordering},
};

#[derive(Default)]
pub(super) struct State {
    works: BTreeMap<Uuid, Work>,
    publication: Option<(Capture, Uuid, Uuid)>,
    stops: BTreeMap<Uuid, (Capture, Instant)>,
    io: Option<(crossbeam_channel::Sender<Reply>, crossbeam_channel::Receiver<Reply>)>,
    endpoint: Option<Uuid>,
    enable_wanted: bool,
    enable_ready: bool,
    inflight: usize,
}
struct Work {
    capture: Capture,
    target: Uuid,
    generation: Uuid,
    grant_generation: Uuid,
    session: Uuid,
    lease: Uuid,
    before: Pose,
    rotations: BTreeMap<String, [f32; 4]>,
    expected: BTreeMap<String, u32>,
    started: Instant,
    phase: &'static str,
    submitted: bool,
    finished: bool,
    reported: bool,
    result: Value,
}
enum Reply {
    Enable { token: Uuid, enabled: bool, accepted: bool },
    Packet { operation: Option<Uuid>, accepted: bool },
}
pub(super) fn supports(t: Tool) -> bool {
    matches!(
        t,
        Tool::PoseSharedSessions
            | Tool::PoseSharedGrants
            | Tool::PoseSharedJoints
            | Tool::PoseSharedOperationStatus
            | Tool::PoseSharedUpdate
            | Tool::PoseSharedCancel
            | Tool::PoseStartSharing
            | Tool::PoseStopSharing
    )
}
fn private(app: &App) -> Result<(), &'static str> {
    if app.world.rlva.has("showloc") || app.world.rlva.has("shownames") || app.world.rlva.has("shownearby") {
        Err("native_shared_pose_privacy_restriction")
    } else {
        Ok(())
    }
}
fn editable(app: &App, name: &str, joint: &Joint, pose: &Pose) -> bool {
    if !super::pose::supported(name) || !joint.enabled || joint.world_locked || joint.mirrored {
        return false;
    }
    let rig = &app.world.avatar_lib.rig;
    let Some(&bone) = rig.names.get(name) else {
        return false;
    };
    for locked in pose
        .iter()
        .filter(|(_, j)| j.world_locked)
        .filter_map(|(n, _)| rig.names.get(n).copied())
    {
        let mut parent = rig.parent[locked];
        while let Some(p) = parent {
            if p == bone {
                return false;
            }
            parent = rig.parent[p];
        }
    }
    true
}
fn snapshot(app: &App, target: Uuid) -> Value {
    let s = &app.world.shared_pose;
    let own = target == app.world.agent_id;
    let now = Instant::now();
    let pose = if own && s.owner.active() {
        Some((&s.owner.pose, &s.owner.revisions, s.owner.revision))
    } else {
        s.remotes.get(&target).map(|r| (&r.pose, &r.revisions, r.revision))
    };
    let generation = if own {
        s.public_generation
    } else {
        s.remotes.get(&target).map(|r| r.generation).unwrap_or_default()
    };
    let can_edit = if own {
        s.available && s.owner.active()
    } else {
        s.can_edit(target, now)
    };
    let joints:BTreeMap<_,_>=pose.map(|(pose,revisions,_)|pose.iter().filter(|(n,_)|super::pose::supported(n)).map(|(name,j)|{(name.clone(),json!({"rotation":&j.transform[..4],"revision":revisions.get(name).copied().unwrap_or(0),"editable":can_edit&&editable(app,name,j,pose),"reservation_known":own,"reserved":false}))}).collect()).unwrap_or_default();
    json!({"target":target,"available":s.available,"shared":s.is_shared(target),"can_edit":can_edit,"can_publish":own&&s.available&&!s.owner.active()&&app.ik_editor.native_shared_pose(&app.world).is_some(),"session_generation":generation,
        "grant_generation":if own{Some(s.public_generation)}else{s.grant_generation(target)},"native_pose_generation":if own{Some(s.native_generation)}else{None},"typed_rotation_protocol":true,"reservation_authority":if own{"owner_current"}else{"owner_reservation_required"},"pose_revision":pose.map(|(_,_,r)|r).unwrap_or(0),"joints":joints})
}
fn expected(c: &Capture, rotations: &BTreeMap<String, glam::Quat>) -> Result<BTreeMap<String, u32>, &'static str> {
    let map = c
        .arguments
        .get("expected_joint_revisions")
        .and_then(Value::as_object)
        .filter(|m| m.len() == rotations.len())
        .ok_or("invalid_joint_revisions")?;
    map.iter()
        .map(|(n, r)| {
            if !rotations.contains_key(n) {
                return Err("invalid_joint_revisions");
            }
            let r = r.as_u64().filter(|r| *r <= i32::MAX as u64).ok_or("invalid_joint_revisions")?;
            Ok((n.clone(), r as u32))
        })
        .collect()
}
pub(super) fn prepare(app: &App, state: &State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    private(app)?;
    let a = &c.arguments;
    let keys = match c.tool {
        Tool::PoseSharedSessions | Tool::PoseSharedGrants | Tool::PoseSharedJoints => vec!["target"],
        Tool::PoseSharedOperationStatus => vec!["target", "operation_id"],
        Tool::PoseSharedUpdate => vec!["target", "session_generation", "rotations", "expected_joint_revisions"],
        Tool::PoseSharedCancel => vec!["target", "session_generation", "operation_id"],
        _ => vec!["target", "session_generation"],
    };
    args(c, &keys)?;
    if matches!(c.tool, Tool::PoseSharedSessions | Tool::PoseSharedGrants) {
        if a.contains_key("target") {
            uuid(a, "target")?;
        }
        return Ok(vec![Class::PoseRead]);
    }
    if c.tool == Tool::PoseSharedOperationStatus {
        let id = uuid(a, "operation_id")?;
        if !state.works.get(&id).is_some_and(|w| same_source(&w.capture, c)) {
            return Err("shared_operation_not_owned");
        }
        return Ok(vec![Class::PoseRead]);
    }
    let target = uuid(a, "target")?;
    if !crate::world::shared_pose::present(&app.world, target) {
        return Err("nearby_ordinary_avatar_required");
    }
    if c.tool == Tool::PoseSharedJoints {
        return Ok(vec![Class::PoseRead]);
    }
    if !app.world.shared_pose.available || !active_allowed(app) {
        return Err("native_sharing_or_active_character_required");
    }
    let snap = snapshot(app, target);
    if uuid(a, "session_generation")?.to_string() != snap["session_generation"].as_str().unwrap_or_default() {
        return Err("native_shared_session_changed");
    }
    match c.tool {
        Tool::PoseStartSharing => {
            if target != app.world.agent_id
                || app.world.shared_pose.owner.active()
                || app.ik_editor.native_shared_pose(&app.world).is_none()
            {
                return Err("already_active_native_self_poser_required");
            }
        }
        Tool::PoseStopSharing => {
            if target != app.world.agent_id || !app.world.shared_pose.owner.active() {
                return Err("native_self_publication_required");
            }
        }
        Tool::PoseSharedCancel => {
            let id = uuid(a, "operation_id")?;
            if !state
                .works
                .get(&id)
                .is_some_and(|w| same_source(&w.capture, c) && w.target == target)
            {
                return Err("shared_operation_not_owned");
            }
        }
        Tool::PoseSharedUpdate => {
            if snap["can_edit"] != true {
                return Err("native_owner_grant_required");
            }
            let rotations = super::pose::rotations(app, &a["rotations"])?;
            let expected = expected(c, &rotations)?;
            for (name, revision) in expected {
                if snap["joints"][&name]["editable"] != true || snap["joints"][&name]["revision"].as_u64() != Some(revision as u64) {
                    return Err("shared_joint_revision_or_effect_conflict");
                }
            }
        }
        _ => return Err("unsupported_capability"),
    }
    Ok(vec![Class::PoseRead])
}
pub(super) fn evidence(app: &App, state: &State, c: &Capture) -> Result<Value, &'static str> {
    if matches!(
        c.tool,
        Tool::PoseSharedSessions | Tool::PoseSharedGrants | Tool::PoseSharedJoints | Tool::PoseSharedOperationStatus
    ) {
        return Ok(Value::Null);
    }
    let target = uuid(&c.arguments, "target")?;
    let mut value = json!({"source":snapshot(app,target),"operation":c.arguments});
    if c.tool == Tool::PoseStartSharing {
        let Some((generation, pose)) = app.ik_editor.native_shared_pose(&app.world) else {
            return Err("native_self_pose_required");
        };
        value["native_motion_generation"] = json!(generation);
        value["native_pose"]=json!(pose.iter().map(|(n,j)|json!({"name":n,"transform":j.transform,"priority":j.priority,"enabled":j.enabled,"world_locked":j.world_locked,"mirrored":j.mirrored})).collect::<Vec<_>>());
    }
    if c.tool == Tool::PoseSharedCancel {
        value["receipt"] = state
            .works
            .get(&uuid(&c.arguments, "operation_id")?)
            .map(|w| w.result.clone())
            .unwrap_or(Value::Null);
    }
    Ok(value)
}
fn finish(work: &mut Work, state: &'static str, reason: &'static str) {
    work.finished = true;
    work.phase = state;
    work.result["state"] = json!(state);
    work.result["status"] = json!("completed");
    work.result["reason"] = json!(reason);
    work.result["owner_accepted"] = json!(state == "owner_accepted");
    work.result["uncertain"] = json!(work.submitted && !matches!(state, "owner_accepted" | "owner_rejected"));
}
pub(super) fn execute(app: &mut App, state: &mut State, c: &Capture) -> Result<Value, &'static str> {
    let a = &c.arguments;
    if c.tool == Tool::PoseSharedOperationStatus {
        return state
            .works
            .get(&uuid(a, "operation_id")?)
            .filter(|w| same_source(&w.capture, c))
            .map(|w| w.result.clone())
            .ok_or("shared_operation_not_owned");
    }
    if matches!(c.tool, Tool::PoseSharedSessions | Tool::PoseSharedGrants | Tool::PoseSharedJoints) {
        if a.contains_key("target") {
            return Ok(json!({"available":app.world.shared_pose.available,"session":snapshot(app,uuid(a,"target")?)}));
        }
        let mut ids = vec![app.world.agent_id];
        ids.extend(
            app.world
                .shared_pose
                .remotes
                .keys()
                .copied()
                .filter(|id| c.tool != Tool::PoseSharedGrants || app.world.shared_pose.can_edit(*id, Instant::now()))
                .take(63),
        );
        return Ok(
            json!({"available":app.world.shared_pose.available,"sessions":ids.into_iter().map(|id|{let mut s=snapshot(app,id);s.as_object_mut().map(|s|s.remove("joints"));s}).collect::<Vec<_>>()}),
        );
    }
    let id = Uuid::parse_str(&c.request_id).map_err(|_| "invalid_request_id")?;
    let target = uuid(a, "target")?;
    let generation = uuid(a, "session_generation")?;
    if c.tool == Tool::PoseStartSharing {
        let (native, pose) = app.ik_editor.native_shared_pose(&app.world).ok_or("native_self_pose_required")?;
        if !app.world.shared_pose.publish(native, pose, Some(id)) {
            return Err("native_publication_changed");
        }
        let generation = app.world.shared_pose.public_generation;
        state.publication = Some((c.clone(), generation, native));
        let mut result = submitted(c);
        result["state"] = json!("published_local_session");
        result["session_generation"] = json!(generation);
        result["remote_synchronization"] = json!("unconfirmed");
        return Ok(result);
    }
    if c.tool == Tool::PoseStopSharing {
        if state.stops.len() >= 8 {
            return Err("shared_stop_capacity");
        }
        app.world.shared_pose.stop_publication(Some(id));
        state.publication = None;
        state.stops.insert(id, (c.clone(), Instant::now()));
        return Ok(
            json!({"status":"submitted","state":"publishing_stopped_local","native_pose_preserved":true,"remote_synchronization":"unconfirmed_until_native_peer_timeout","operation_id":c.request_id}),
        );
    }
    if c.tool == Tool::PoseSharedCancel {
        let original = uuid(a, "operation_id")?;
        let work = state
            .works
            .get_mut(&original)
            .filter(|w| same_source(&w.capture, c) && w.target == target)
            .ok_or("shared_operation_not_owned")?;
        app.world.shared_pose.cancel_packets(original);
        if !work.submitted {
            app.world.shared_pose.release(target, work.lease, None);
        }
        finish(
            work,
            if work.submitted { "submitted_unknown" } else { "cancelled" },
            "cancelled_by_source_no_replay",
        );
        return Ok(work.result.clone());
    }
    if state.works.len() >= 64 {
        let old = state.works.iter().find(|(_, w)| w.finished).map(|(id, _)| *id);
        if let Some(id) = old {
            state.works.remove(&id);
        } else {
            return Err("shared_operation_capacity");
        }
    }
    let qs = super::pose::rotations(app, &a["rotations"])?;
    let expected = expected(c, &qs)?;
    let lease = Uuid::new_v4();
    let now = Instant::now();
    let before: Pose = if target == app.world.agent_id {
        expected
            .keys()
            .filter_map(|n| app.world.shared_pose.owner.pose.get(n).map(|j| (n.clone(), j.clone())))
            .collect()
    } else {
        let remote = app.world.shared_pose.remotes.get(&target).ok_or("native_shared_session_changed")?;
        expected
            .keys()
            .filter_map(|n| remote.pose.get(n).map(|j| (n.clone(), j.clone())))
            .collect()
    };
    let rotations = qs.iter().map(|(n, q)| (n.clone(), q.to_array())).collect();
    let session = if target == app.world.agent_id {
        app.world.shared_pose.owner.session
    } else {
        app.world.shared_pose.token(target).ok_or("native_owner_grant_required")?.0
    };
    let grant_generation = if target == app.world.agent_id {
        generation
    } else {
        app.world
            .shared_pose
            .grant_generation(target)
            .ok_or("native_owner_grant_required")?
    };
    let mut work = Work {
        capture: c.clone(),
        target,
        generation,
        grant_generation,
        session,
        lease,
        before,
        rotations,
        expected,
        started: now,
        phase: "pending",
        submitted: false,
        finished: false,
        reported: false,
        result: json!({"status":"submitted","operation_id":c.request_id,"target":target,"session_generation":generation,"state":"pending","owner_accepted":false,"uncertain":false,"rotation_only":true,"retry_safe":false}),
    };
    if target == app.world.agent_id {
        if app
            .world
            .shared_pose
            .owner
            .acquire(target, Uuid::nil(), lease, &work.expected, now)
            .is_none()
        {
            return Err("shared_joint_reserved");
        }
        let changes: Pose = work
            .before
            .iter()
            .map(|(n, j)| (n.clone(), j.with_rotation(work.rotations[n])))
            .collect();
        let accepted = app
            .world
            .shared_pose
            .owner
            .can_apply(target, Uuid::nil(), lease, &work.expected, &changes, now)
            && app.ik_editor.apply_shared_rotations(&app.world, &work.before, &changes)
            && app
                .world
                .shared_pose
                .owner
                .apply(target, Uuid::nil(), lease, &work.expected, &changes, now);
        work.submitted = true;
        finish(&mut work, if accepted { "owner_accepted" } else { "owner_rejected" }, "");
    } else if !app.world.shared_pose.acquire(target, lease, &work.expected, id) {
        return Err("native_owner_grant_required");
    }
    let result = work.result.clone();
    work.reported = work.finished;
    state.works.insert(id, work);
    Ok(result)
}
fn current(app: &App, work: &Work) -> bool {
    let s = &app.world.shared_pose;
    if !s.available || !active_allowed(app) || private(app).is_err() {
        return false;
    }
    if work.target == app.world.agent_id {
        return s.public_generation == work.generation && s.owner.session == work.session;
    }
    s.remotes
        .get(&work.target)
        .is_some_and(|r| r.generation == work.generation && r.session == work.session)
        && s.grant_generation(work.target) == Some(work.grant_generation)
        && s.can_edit(work.target, Instant::now())
}
fn peer_reply(app: &mut App, state: &mut State, reply: crate::world::shared_pose::PeerReply) {
    let lease = reply.data["l"].as_uuid();
    let Some((id, work)) = state
        .works
        .iter_mut()
        .find(|(_, w)| w.target == reply.peer && w.lease == lease && w.session == reply.data["s"].as_uuid())
    else {
        return;
    };
    if work.finished && !(work.submitted && work.phase == "submitted_unknown" && reply.data["t"].as_str() == "ack") {
        return;
    }
    let kind = reply.data["t"].as_str();
    if kind == "ack" {
        let observed = decode_pose(reply.data["p"].as_binary());
        let accepted = reply.data["ok"].as_bool();
        let valid = work.submitted
            && reply.data["f"].as_bool()
            && matches!(reply.data["ok"], Llsd::Boolean(_))
            && (!accepted
                || observed.as_ref().is_some_and(|pose| {
                    work.before.iter().all(|(n, b)| {
                        pose.get(n).is_some_and(|j| {
                            j.transform[4..] == b.transform[4..]
                                && j.priority == b.priority
                                && j.enabled == b.enabled
                                && j.world_locked == b.world_locked
                                && j.mirrored == b.mirrored
                                && same_rotation(&j.transform[..4], &work.rotations[n])
                        })
                    })
                }));
        if valid {
            finish(work, if accepted { "owner_accepted" } else { "owner_rejected" }, "");
        } else {
            finish(work, "submitted_unknown", "malformed_or_premature_owner_acknowledgement");
        }
        return;
    }
    if !current(app, work) {
        finish(
            work,
            if work.submitted { "submitted_unknown" } else { "cancelled" },
            "native_grant_or_session_changed",
        );
        return;
    }
    if kind == "busy" {
        finish(work, "conflicted", "owner_joint_reservation_refused");
        return;
    }
    if kind != "locked" || work.phase != "pending" {
        return;
    }
    let observed = decode_pose(reply.data["p"].as_binary());
    if observed.as_ref() != Some(&work.before)
        || work
            .expected
            .iter()
            .any(|(n, r)| reply.data["jr"][n.as_str()].as_i32() != *r as i32)
    {
        finish(work, "conflicted", "owner_snapshot_or_joint_revision_changed");
        return;
    }
    work.phase = "reserved";
    work.result["state"] = json!("reserved");
    app.world
        .shared_pose
        .final_edit(work.target, work.lease, &work.expected, &work.rotations, *id);
}
fn same_rotation(a: &[f32], b: &[f32]) -> bool {
    a.len() == 4
        && b.len() == 4
        && [1., -1.]
            .iter()
            .any(|sign| a.iter().zip(b).all(|(a, b)| (*a - *sign * *b).abs() <= 1e-5))
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    if let Some((capture, generation, native)) = &assistant.creative.shared.publication {
        let taken_over = app.world.shared_pose.publication.is_none();
        let valid = assistant.runtime.capture_current(capture)
            && binding_matches(app, assistant, capture)
            && active_allowed(app)
            && app.world.shared_pose.public_generation == *generation
            && app.world.shared_pose.native_generation == *native;
        if taken_over {
            assistant.creative.shared.publication = None;
        } else if !valid {
            app.world.shared_pose.stop_publication(None);
            assistant.creative.shared.publication = None;
        }
    }
    for (id, w) in &mut assistant.creative.shared.works {
        if w.finished {
            continue;
        }
        if !assistant.runtime.capture_current(&w.capture) || !current(app, w) || w.started.elapsed() > Duration::from_secs(60) {
            app.world.shared_pose.cancel_packets(*id);
            finish(
                w,
                if w.submitted { "submitted_unknown" } else { "cancelled" },
                "authority_or_session_expired",
            );
        }
    }
    let replies: Vec<_> = app.world.shared_pose.replies.drain(..).collect();
    for reply in replies {
        peer_reply(app, &mut assistant.creative.shared, reply);
    }
    for work in assistant.creative.shared.works.values_mut().filter(|w| w.finished && !w.reported) {
        let _ = assistant.runtime.complete(&work.capture, work.result.clone());
        work.reported = true;
    }
    let expired: Vec<_> = assistant
        .creative
        .shared
        .stops
        .iter()
        .filter(|(_, (_, at))| at.elapsed() > Duration::from_secs(10))
        .map(|(id, _)| *id)
        .collect();
    for id in expired {
        if let Some((capture, _)) = assistant.creative.shared.stops.remove(&id) {
            let _=assistant.runtime.complete(&capture,json!({"status":"completed","state":"publishing_stopped_local","operation_id":capture.request_id,"native_pose_preserved":true,"remote_synchronization":"unconfirmed_until_native_peer_timeout"}));
        }
    }
}
/// Called independently of assistant enablement, before the scene poses. The
/// ordinary native owner grant UI and native sharing retain their own lifetime.
pub(super) fn native_tick(app: &mut App, assistant: &mut Assistant) {
    // Also runs when invalid configuration exits the assistant request tick.
    // Source-owned publication and pending edits must still lose authority.
    tick(app, assistant);
    let now = Instant::now();
    let endpoint = app.bridge_runtime.query_endpoint();
    let token = endpoint.as_ref().map(|e| e.token);
    let enabled = app.settings.pose_requests.sharing_enabled;
    let replies: Vec<_> = assistant
        .creative
        .shared
        .io
        .as_ref()
        .map(|(_, rx)| rx.try_iter().collect())
        .unwrap_or_default();
    for reply in replies {
        assistant.creative.shared.inflight = assistant.creative.shared.inflight.saturating_sub(1);
        match reply {
            Reply::Enable { token, enabled, accepted } => {
                if assistant.creative.shared.endpoint == Some(token) && assistant.creative.shared.enable_wanted == enabled {
                    assistant.creative.shared.enable_ready = enabled && accepted;
                }
            }
            Reply::Packet { operation, accepted } => {
                if let Some(id) = operation {
                    if let Some((capture, _)) = assistant.creative.shared.stops.remove(&id) {
                        let _=assistant.runtime.complete(&capture,json!({"status":"completed","state":"publishing_stopped_local","operation_id":capture.request_id,"native_pose_preserved":true,"transport_acknowledged":accepted,"remote_synchronization":"unconfirmed_until_native_peer_timeout"}));
                    }
                    if let Some(w) = assistant.creative.shared.works.get_mut(&id) {
                        w.result["transport_acknowledged"] = json!(accepted);
                        if !accepted && !w.finished {
                            finish(
                                w,
                                if w.submitted { "submitted_unknown" } else { "cancelled" },
                                "bridge_transport_failed_no_replay",
                            );
                        }
                    }
                }
            }
        }
    }
    if token != assistant.creative.shared.endpoint || enabled != assistant.creative.shared.enable_wanted {
        assistant.creative.shared.endpoint = token;
        assistant.creative.shared.enable_wanted = enabled;
        assistant.creative.shared.enable_ready = false;
        app.world.shared_pose.invalidate();
        if let Some(endpoint) = endpoint.clone() {
            let tx = assistant
                .creative
                .shared
                .io
                .get_or_insert_with(|| crossbeam_channel::bounded(8))
                .0
                .clone();
            assistant.creative.shared.inflight += 1;
            app.net.runtime().spawn(async move {
                let token = endpoint.token;
                let accepted = aurora_net::bridge::query::send(endpoint, Query::SharedPoseEnable(enabled))
                    .await
                    .is_ok();
                let _ = tx.try_send(Reply::Enable { token, enabled, accepted });
            });
        }
    }
    let native = app.ik_editor.native_shared_pose(&app.world);
    let mut service = std::mem::take(&mut app.world.shared_pose);
    service.observe(
        &app.world,
        token.unwrap_or_default(),
        assistant.creative.shared.enable_ready,
        enabled,
        native,
        now,
    );
    app.world.shared_pose = service;
    let edits: Vec<_> = app.world.shared_pose.edits.drain(..).collect();
    for edit in edits {
        let authorized = app.world.shared_pose.available
            && edit.native_generation == app.world.shared_pose.native_generation
            && edit.session == app.world.shared_pose.owner.session
            && app
                .world
                .shared_pose
                .owner
                .can_apply(edit.peer, edit.token, edit.lease, &edit.expected, &edit.changes, now);
        let accepted = authorized && app.ik_editor.apply_shared_rotations(&app.world, &edit.before, &edit.changes);
        app.world.shared_pose.confirm_edit(edit, accepted, now);
    }
    let remotes: Vec<_> = app.world.shared_pose.remotes.iter().map(|(id, r)| (*id, r.pose.clone())).collect();
    for (id, pose) in remotes {
        if app.scene.poser.contains_key(&id) {
            continue;
        }
        if let Some(layer) = remote_layer(app, &pose) {
            app.scene.poser.insert(id, Arc::new(layer));
        }
    }
    if assistant.creative.shared.inflight >= 2 {
        return;
    }
    let Some(packet) = app.world.shared_pose.take_packet(now) else {
        return;
    };
    let Some(endpoint) = endpoint else {
        return;
    };
    let capture = packet.operation.and_then(|id| {
        assistant
            .creative
            .shared
            .works
            .get(&id)
            .map(|w| w.capture.clone())
            .or_else(|| assistant.creative.shared.stops.get(&id).map(|(capture, _)| capture.clone()))
            .or_else(|| {
                assistant
                    .creative
                    .shared
                    .publication
                    .as_ref()
                    .filter(|(c, _, _)| Uuid::parse_str(&c.request_id).ok() == Some(id))
                    .map(|(c, _, _)| c.clone())
            })
    });
    if packet.operation.is_some() && capture.is_none() {
        return;
    }
    if let Some(id) = packet.operation
        && let Some(w) = assistant.creative.shared.works.get_mut(&id)
    {
        if w.finished {
            return;
        }
        if w.phase == "reserved" {
            w.submitted = true;
            w.result["state"] = json!("submitted");
            w.result["uncertain"] = json!(true);
        }
    }
    let runtime = assistant.runtime.clone();
    let epoch = app.world.shared_pose.epoch.clone();
    let expected = packet.epoch;
    let tx = assistant
        .creative
        .shared
        .io
        .get_or_insert_with(|| crossbeam_channel::bounded(8))
        .0
        .clone();
    assistant.creative.shared.inflight += 1;
    app.net.runtime().spawn(async move {
        let operation = packet.operation;
        let query = Query::SharedPosePacket {
            target: packet.target,
            packet: packet.packet,
        };
        let accepted = aurora_net::bridge::query::send_guarded(endpoint, query, move || {
            epoch.load(Ordering::Acquire) == expected && capture.as_ref().is_none_or(|c| runtime.continue_action(c, || ()).is_ok())
        })
        .await
        .is_ok();
        let _ = tx.try_send(Reply::Packet { operation, accepted });
    });
}
fn remote_layer(app: &App, pose: &Pose) -> Option<crate::ik_edit::poser::SubjectPose> {
    let rig = &app.world.avatar_lib.rig;
    let mut layer = crate::ik_edit::poser::SubjectPose {
        rotations: rig.local_rot.clone(),
        positions: rig.local_pos.clone(),
        scales: rig.scale.clone(),
        origin: rig.local_rot.clone(),
        locks: Default::default(),
    };
    for (n, j) in pose {
        let Some(&b) = rig.names.get(n) else {
            continue;
        };
        if !j.enabled {
            continue;
        }
        layer.rotations[b] = glam::Quat::from_array(j.transform[..4].try_into().ok()?).normalize();
        layer.positions[b] = Vec3::from_slice(&j.transform[4..7]);
        layer.scales[b] = Vec3::from_slice(&j.transform[7..10]);
    }
    layer.valid(rig).then_some(layer)
}
pub(super) fn ui(app: &mut App, _state: &mut State, ctx: &egui::Context, p: &crate::theme::Palette) {
    if let Some(invitation) = app.world.shared_pose.incoming.front().cloned() {
        let name = crate::world::poser_consent::name(&app.world, invitation.peer);
        crate::ui::widgets::Modal::new(egui::Id::unique(("native-shared-pose-consent", invitation.request))).show(ctx, |ui| {
            ui.heading(aurora_i18n::tr!("shared-pose-request", name = name));
            ui.label(aurora_i18n::tr!("shared-pose-consent-body"));
            ui.horizontal(|ui| {
                for (accept, key) in [(false, "poser-consent-refuse"), (true, "poser-consent-accept")] {
                    if ui.button(aurora_i18n::text(key)).clicked() {
                        if accept && app.ik_editor.native_shared_pose(&app.world).is_none() {
                            app.ik_editor.open_poser();
                            app.ik_editor.select_pose_subject(&app.world, app.world.agent_id);
                        }
                        let native = app.ik_editor.native_shared_pose(&app.world);
                        let mut service = std::mem::take(&mut app.world.shared_pose);
                        service.answer(invitation.clone(), accept, native, Instant::now());
                        app.world.shared_pose = service;
                    }
                }
            });
        });
    }
    if !app.world.shared_pose.ui_open {
        return;
    }
    let mut open = true;
    crate::ui::widgets::Floater::new(
        "native_shared_pose",
        aurora_i18n::tr!("shared-pose-title"),
        egui::pos2(250., 100.),
        egui::vec2(440., 420.),
    )
    .show(ctx, p, &mut open, |ui| {
        if crate::ui::widgets::checkbox(
            ui,
            p,
            &mut app.settings.pose_requests.sharing_enabled,
            aurora_i18n::tr!("shared-pose-enable"),
        )
        .changed()
        {
            app.settings.save();
        }
        ui.label(aurora_i18n::tr!("shared-pose-consent-body"));
        if app.world.shared_pose.available {
            ui.horizontal(|ui| {
                if ui.button(aurora_i18n::tr!("shared-pose-publish")).clicked() {
                    if app.world.shared_pose.owner.active() {
                        app.world.shared_pose.native_takeover();
                    } else {
                        if app.ik_editor.native_shared_pose(&app.world).is_none() {
                            app.ik_editor.open_poser();
                            app.ik_editor.select_pose_subject(&app.world, app.world.agent_id);
                        }
                        if let Some((native, pose)) = app.ik_editor.native_shared_pose(&app.world) {
                            app.world.shared_pose.publish(native, pose, None);
                        }
                    }
                }
                if ui.button(aurora_i18n::tr!("shared-pose-stop")).clicked() {
                    app.world.shared_pose.stop_publication(None);
                }
            });
            let peers: Vec<_> = app
                .world
                .objects
                .avatars()
                .map(|(_, o)| o.full_id)
                .filter(|id| *id != app.world.agent_id && crate::world::shared_pose::present(&app.world, *id))
                .take(64)
                .collect();
            for peer in peers {
                ui.horizontal(|ui| {
                    ui.label(crate::world::poser_consent::name(&app.world, peer));
                    if ui.button(aurora_i18n::tr!("shared-pose-request-edit")).clicked() {
                        app.world.shared_pose.request(peer, Instant::now());
                    }
                });
            }
            for peer in app.world.shared_pose.owner.editors(Instant::now()) {
                ui.horizontal(|ui| {
                    ui.label(crate::world::poser_consent::name(&app.world, peer));
                    if ui.button(aurora_i18n::tr!("poser-consent-withdraw")).clicked() {
                        app.world.shared_pose.revoke(peer);
                    }
                });
            }
        } else {
            ui.label(aurora_i18n::tr!("shared-pose-bridge-needed"));
        }
    });
    app.world.shared_pose.ui_open = open;
}
