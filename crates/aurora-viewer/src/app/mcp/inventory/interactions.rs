//! Exact owned-object interactions. No command strings, arbitrary Event API,
//! optimistic acquisition or notification-type substitution enters this path.
use super::*;
use glam::Vec3;
type Key = (HostId, u64, Uuid);
#[derive(Default)]
pub(super) struct State {
    intents: BTreeMap<Key, Capture>,
    takes: BTreeMap<Uuid, Take>,
    dialogs: BTreeMap<Uuid, Dialog>,
    replies: BTreeMap<Uuid, Reply>,
    sends: VecDeque<Instant>,
    evidence: FrozenEvidence,
}
#[derive(Clone)]
struct Take {
    capture: Capture,
    object: Uuid,
    destination: Uuid,
    copy: bool,
    submitted: Instant,
    ack: Option<bool>,
    items: BTreeMap<Uuid, InvItem>,
    removed: bool,
    rejected: bool,
}
#[derive(Clone)]
struct Dialog {
    capture: Capture,
    native: u64,
    object: Uuid,
    revision: String,
    expires: Instant,
}
#[derive(Clone)]
struct Reply {
    capture: Capture,
    native: u64,
    response: crate::world::notifications::Response,
    submitted: Instant,
}
pub(super) type FrozenEvidence = std::cell::RefCell<BTreeMap<(HostId, u64, String), (Capture, Value, Instant)>>;
pub(super) fn freeze_capture(evidence: &FrozenEvidence, c: &Capture, value: Value) -> Result<(), &'static str> {
    let mut evidence = evidence.borrow_mut();
    evidence.retain(|_, (_, _, at)| at.elapsed() < Duration::from_secs(605));
    let key = (c.endpoint, c.client, c.request_id.clone());
    if let Some((old, frozen, _)) = evidence.get(&key) {
        if old.binding != c.binding
            || old.policy_revision != c.policy_revision
            || old.tool != c.tool
            || old.arguments != c.arguments
            || *frozen != value
        {
            return Err("native_scope_changed");
        }
    } else {
        if evidence.len() >= CAPACITY {
            return Err("operation_capacity");
        }
        evidence.insert(key, (c.clone(), value, Instant::now()));
    }
    Ok(())
}
impl State {
    fn track_take(&mut self, c: &Capture, object: Uuid, destination: Uuid, copy: bool) -> Uuid {
        let request = Uuid::new_v4();
        self.takes.insert(
            request,
            Take {
                capture: c.clone(),
                object,
                destination,
                copy,
                submitted: Instant::now(),
                ack: None,
                items: BTreeMap::new(),
                removed: false,
                rejected: false,
            },
        );
        request
    }
    fn track_reply(&mut self, c: &Capture, native: u64, response: crate::world::notifications::Response) -> Uuid {
        let request = Uuid::new_v4();
        self.replies.insert(
            request,
            Reply {
                capture: c.clone(),
                native,
                response,
                submitted: Instant::now(),
            },
        );
        request
    }
}
impl Take {
    fn observe(&mut self, progress: &aurora_net::NativeTakeProgress) {
        match progress {
            aurora_net::NativeTakeProgress::Submitted { .. } => {}
            aurora_net::NativeTakeProgress::Ack(ok) => self.ack = Some(*ok),
            aurora_net::NativeTakeProgress::Items(items) => {
                for item in items {
                    self.items.insert(item.id, item.clone());
                }
            }
            aurora_net::NativeTakeProgress::Removed => self.removed = true,
            aurora_net::NativeTakeProgress::Rejected => self.rejected = true,
        }
    }
    fn confirmation(&self, agent: Uuid) -> Option<Uuid> {
        if self.rejected || self.ack != Some(true) || self.items.len() != 1 || !(self.copy || self.removed) {
            return None;
        }
        self.items
            .values()
            .next()
            .filter(|i| i.asset_type == 6 && i.owner == agent && i.parent == self.destination)
            .map(|i| i.id)
    }
}
pub(super) fn supports(t: Tool) -> bool {
    matches!(
        t,
        Tool::ObjectTakeCopy
            | Tool::ObjectTake
            | Tool::ObjectTouch
            | Tool::ObjectDialogList
            | Tool::ObjectDialogReply
            | Tool::ObjectChannelSend
    )
}
pub(super) fn object(app: &App, id: Uuid) -> Result<usize, &'static str> {
    diagnostics::privacy(app)?;
    let idx = objects::object(app, id)?;
    let o = app.world.objects.get(idx).ok_or("object_unavailable")?;
    if o.pcode != 9
        || o.is_client_preview()
        || app.world.main_region != Some(o.key.region)
        || o.owner_id != app.world.agent_id
        || app.world.derender.contains(&id)
        || !app.world.parcel_object_visible(idx)
        || app.world.mutes.is_muted_id(&id)
        || app.world.mutes.is_muted_id(&o.owner_id)
    {
        return Err("native_object_restriction");
    }
    let (root, attached) = crate::build::contents_drop::linkset(&app.world, idx).ok_or("object_unavailable")?;
    if attached {
        return Err("attachment_not_supported");
    }
    let root = app.world.objects.get(root).ok_or("object_unavailable")?;
    if root.owner_id != app.world.agent_id || root.update_flags & crate::ui::context::flags::AFFECTS_NAVMESH != 0 {
        return Err("native_object_restriction");
    }
    let (position, _, hud) =
        crate::scene::Scene::object_transform_unshifted(&app.world, idx, Instant::now()).ok_or("object_unavailable")?;
    if hud || !position.is_finite() || position.distance(app.world.agent.position) > 64. {
        return Err("object_out_of_range");
    }
    Ok(idx)
}
pub(super) fn freeze(app: &App, id: Uuid) -> Result<Value, &'static str> {
    let idx = object(app, id)?;
    let o = app.world.objects.get(idx).ok_or("object_unavailable")?;
    let (root, _) = crate::build::contents_drop::linkset(&app.world, idx).ok_or("object_unavailable")?;
    Ok(
        json!({"object":id.to_string(),"region":o.key.region.to_string(),"local":o.key.local_id,"owner":o.owner_id.to_string(),"root":app.world.objects.get(root).map(|o|o.full_id.to_string()),"flags":o.update_flags,"scale":[o.scale.x,o.scale.y,o.scale.z]}),
    )
}
pub(super) fn point(args: &serde_json::Map<String, Value>, key: &str, n: usize) -> Result<Vec<f32>, &'static str> {
    let values = args
        .get(key)
        .and_then(Value::as_array)
        .filter(|a| a.len() == n)
        .ok_or("invalid_args")?;
    values
        .iter()
        .map(|v| {
            v.as_f64()
                .filter(|n| n.is_finite() && n.abs() <= 1e9)
                .map(|v| v as f32)
                .ok_or("invalid_args")
        })
        .collect()
}
pub(super) fn surface(app: &App, idx: usize, a: &serde_json::Map<String, Value>) -> Result<aurora_net::TouchSurface, &'static str> {
    let supplied = ["face", "uv", "local_point"].iter().filter(|k| a.contains_key(**k)).count();
    if supplied == 0 {
        return Ok(Default::default());
    }
    if supplied != 3 {
        return Err("touch_surface_requires_all_fields");
    }
    let o = app.world.objects.get(idx).ok_or("object_unavailable")?;
    let face = a
        .get("face")
        .and_then(Value::as_i64)
        .filter(|f| *f >= 0 && (*f as usize) < app.build.num_faces(&app.world, idx))
        .ok_or("invalid_touch_face")? as i32;
    let uv = point(a, "uv", 2)?;
    if uv.iter().any(|n| !(0.0..=1.0).contains(n)) {
        return Err("invalid_touch_uv");
    }
    let p = point(a, "local_point", 3)?;
    let local = Vec3::new(p[0], p[1], p[2]);
    if !o.scale.is_finite() || local.abs().cmpgt(o.scale.abs() * 0.5 + Vec3::splat(0.001)).any() {
        return Err("touch_point_outside_prim");
    }
    let (position, rotation, hud) =
        crate::scene::Scene::object_transform_unshifted(&app.world, idx, Instant::now()).ok_or("object_unavailable")?;
    if hud {
        return Err("native_object_restriction");
    }
    let local_region = position - app.world.region_offset(o.key.region).ok_or("object_unavailable")? + rotation * local;
    Ok(aurora_net::TouchSurface {
        uv: Vec3::new(uv[0], uv[1], 0.),
        st: Vec3::new(uv[0], uv[1], 0.),
        face,
        position: local_region,
        normal: Vec3::ZERO,
        binormal: Vec3::ZERO,
    })
}
pub(super) fn touch(app: &mut App, id: Uuid, a: &serde_json::Map<String, Value>) -> Result<(), &'static str> {
    let idx = object(app, id)?;
    let surface = surface(app, idx, a)?;
    let o = app.world.objects.get(idx).ok_or("object_unavailable")?;
    let (key, offset) = (o.key, Vec3::ZERO);
    if !app
        .world
        .rlva_can_touch_at(key.region, key.local_id, (surface.face >= 0).then_some(surface.position))
    {
        return Err("native_touch_restriction");
    }
    app.send(NetCommand::ObjectGrab {
        handle: key.region,
        local_id: key.local_id,
        offset,
        surface,
    });
    app.send(NetCommand::ObjectRelease {
        handle: key.region,
        local_id: key.local_id,
        surface,
    });
    Ok(())
}
pub(super) fn channel(a: &serde_json::Map<String, Value>) -> Result<(i32, &str), &'static str> {
    let channel = a
        .get("channel")
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .filter(|n| *n != 0 && *n != i32::MAX)
        .ok_or("invalid_script_channel")?;
    let text = string(a, "text", 1023)?;
    if text.is_empty() || text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err("invalid_text");
    }
    Ok((channel, text))
}
pub(super) fn send_channel(app: &mut App, state: &mut State, a: &serde_json::Map<String, Value>) -> Result<(), &'static str> {
    let (channel, text) = channel(a)?;
    if app.world.rlva.outgoing_chat(text, channel, aurora_net::ChatType::Normal).is_empty() {
        return Err("native_channel_restriction");
    }
    let now = Instant::now();
    while state
        .sends
        .front()
        .is_some_and(|t| now.saturating_duration_since(*t) >= Duration::from_secs(60))
    {
        state.sends.pop_front();
    }
    if state.sends.len() >= 6
        || state
            .sends
            .back()
            .is_some_and(|t| now.saturating_duration_since(*t) < Duration::from_secs(1))
    {
        return Err("channel_rate_limit");
    }
    state.sends.push_back(now);
    app.send(NetCommand::Chat {
        message: text.into(),
        channel,
        chat_type: aurora_net::ChatType::Normal,
    });
    Ok(())
}
fn notification<'a>(
    app: &'a App,
    state: &State,
    c: &Capture,
) -> Result<
    (
        &'a crate::world::notifications::Notification,
        &'a crate::world::assistant_observations::DialogSource,
    ),
    &'static str,
> {
    let token = uuid(&c.arguments, "notification_id")?;
    let d = state
        .dialogs
        .get(&token)
        .filter(|d| {
            d.capture.endpoint == c.endpoint
                && d.capture.client == c.client
                && d.capture.policy_revision == c.policy_revision
                && d.capture.binding == c.binding
                && d.object == uuid(&c.arguments, "object_id").unwrap_or_default()
                && d.expires > Instant::now()
        })
        .ok_or("stale_dialog")?;
    let n = app
        .world
        .notifications
        .list
        .iter()
        .find(|n| n.id == d.native)
        .ok_or("stale_dialog")?;
    let source = app
        .world
        .assistant_observations
        .dialogs
        .get(&n.id)
        .ok_or("native_dialog_provenance_missing")?;
    if !app
        .world
        .login
        .as_ref()
        .is_some_and(|l| source.source.matches_login(l, app.world.agent_id))
        || Some(source.region) != app.world.main_region
        || source.circuit.is_nil()
        || dialog_revision(n, source) != d.revision
        || string(&c.arguments, "expected_dialog", 64)? != d.revision
    {
        return Err("stale_dialog");
    }
    Ok((n, source))
}
pub(super) fn receipt_dialogs(state: &super::State, c: &Capture, value: &Value) -> Result<Vec<Uuid>, &'static str> {
    value
        .get("dialogs")
        .and_then(Value::as_array)
        .ok_or("native_receipt_proof_unavailable")?
        .iter()
        .map(|row| {
            let token = row
                .get("notification_id")
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or("native_receipt_proof_unavailable")?;
            state
                .interactions
                .dialogs
                .get(&token)
                .filter(|d| d.capture == *c)
                .ok_or("native_receipt_proof_unavailable")?;
            Ok(token)
        })
        .collect()
}
pub(super) fn receipt_dialog_current(app: &App, state: &super::State, c: &Capture, token: Uuid) -> bool {
    let Some(d) = state
        .interactions
        .dialogs
        .get(&token)
        .filter(|d| d.capture == *c && d.expires > Instant::now())
    else {
        return false;
    };
    let mut request = c.clone();
    request.arguments.insert("notification_id".into(), json!(token.to_string()));
    request.arguments.insert("expected_dialog".into(), json!(d.revision));
    request.arguments.insert("object_id".into(), json!(d.object.to_string()));
    notification(app, &state.interactions, &request).is_ok()
}
fn dialog_revision(n: &crate::world::notifications::Notification, source: &crate::world::assistant_observations::DialogSource) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(format!("{}|{:?}|{}|{:?}", n.id, n.data, n.body, source.circuit).as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn reply(
    n: &crate::world::notifications::Notification,
    a: &serde_json::Map<String, Value>,
) -> Result<(i32, i32, String, crate::world::notifications::Response), &'static str> {
    use crate::world::notifications::{Data, Response};
    match &n.data {
        Data::Dialog { channel, buttons, .. } => {
            if a.contains_key("text") {
                return Err("button_dialog_requires_index");
            }
            let index = a
                .get("button_index")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n < buttons.len())
                .ok_or("invalid_button_index")?;
            let label = buttons[index].clone();
            if label.len() > 250 {
                return Err("native_button_label_limit");
            }
            Ok((*channel, index as i32, label, Response::Button(index)))
        }
        Data::TextBox { channel, .. } => {
            if a.contains_key("button_index") {
                return Err("textbox_requires_text");
            }
            let text = string(a, "text", 250)?;
            Ok((*channel, 0, text.into(), Response::SendText))
        }
        _ => Err("unsupported_notification_type"),
    }
}
pub(super) fn prepare(app: &App, state: &super::State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    let a = &c.arguments;
    let id = uuid(a, "object_id")?;
    let idx = object(app, id)?;
    let mut classes = vec![Class::WorldRead];
    match c.tool {
        Tool::ObjectTake | Tool::ObjectTakeCopy => {
            args(c, &["object_id", "snapshot_id", "destination_folder", "client_request_id"])?;
            native_busy(app)?;
            let s = objects::snapshot(app, state, c)?;
            let o = app.world.objects.get(idx).ok_or("object_unavailable")?;
            if crate::build::root_of(&app.world, idx) != idx
                || s.identity.root != id
                || o.update_flags & crate::ui::context::flags::OBJECT_MOVE == 0
            {
                return Err("root_linkset_required");
            }
            let family = crate::build::family(&app.world, idx);
            if family.len() > 1024 {
                return Err("linkset_limit");
            }
            for index in family {
                let p = app.world.objects.get(index).ok_or("object_unavailable")?;
                if p.is_avatar()
                    || p.owner_id != app.world.agent_id
                    || p.update_flags & crate::ui::context::flags::OBJECT_MOVE == 0
                    || c.tool == Tool::ObjectTakeCopy && p.update_flags & crate::ui::context::flags::OBJECT_COPY == 0
                {
                    return Err("native_take_restriction");
                }
            }
            if app.world.objects.avatars().any(|(_, av)| {
                av.parent_id != 0
                    && crate::build::root_of(&app.world, app.world.objects.index_of_uuid(&av.full_id).unwrap_or(usize::MAX)) == idx
            }) {
                return Err("occupied_linkset");
            }
            let dest = uuid(a, "destination_folder")?;
            ordinary(app, dest)?;
            if !rules::destination(&app.world.inventory, dest)
                || rules::library(&app.world.inventory, dest)
                || rules::in_type(&app.world.inventory, dest, 14)
            {
                return Err("native_destination_restriction");
            }
            classes.extend([Class::Read, Class::TaskInventoryRead]);
        }
        Tool::ObjectTouch => {
            args(c, &["object_id", "client_request_id", "face", "uv", "local_point"])?;
            surface(app, idx, a)?;
            let o = app.world.objects.get(idx).ok_or("object_unavailable")?;
            if !app.world.rlva_can_touch(o.key.region, o.key.local_id) {
                return Err("native_touch_restriction");
            }
            classes.push(Class::TaskInventoryRead);
        }
        Tool::ObjectChannelSend => {
            args(c, &["object_id", "client_request_id", "channel", "text"])?;
            let (channel, text) = channel(a)?;
            if app.world.rlva.outgoing_chat(text, channel, aurora_net::ChatType::Normal).is_empty() {
                return Err("native_channel_restriction");
            }
            classes.extend([Class::TaskInventoryRead, Class::ChatSendNearby]);
        }
        Tool::ObjectDialogList => {
            args(c, &["object_id"])?;
        }
        Tool::ObjectDialogReply => {
            args(
                c,
                &[
                    "object_id",
                    "client_request_id",
                    "notification_id",
                    "expected_dialog",
                    "button_index",
                    "text",
                ],
            )?;
            let (n, _) = notification(app, &state.interactions, c)?;
            let (channel, _, _, _) = reply(n, a)?;
            if (channel == 0 && app.world.rlva.has("sendchat")) || (channel != 0 && !app.world.rlva.allows_channel(channel)) {
                return Err("native_channel_restriction");
            }
            classes.push(Class::NotificationsRead);
            if channel == 0 {
                classes.push(Class::ChatSendNearby);
            }
        }
        _ => return Err("unsupported_capability"),
    }
    if c.tool != Tool::ObjectDialogList {
        uuid(a, "client_request_id")?;
        let mut scope = BTreeMap::new();
        scope.insert(id.to_string(), freeze(app, id)?);
        if matches!(c.tool, Tool::ObjectTake | Tool::ObjectTakeCopy) {
            for index in crate::build::family(&app.world, idx) {
                let o = app.world.objects.get(index).ok_or("object_unavailable")?;
                scope.insert(o.full_id.to_string(), freeze(app, o.full_id)?);
            }
        }
        freeze_capture(&state.interactions.evidence, c, json!(scope))?;
    }
    Ok(classes)
}
pub(super) fn execute(app: &mut App, state: &mut super::State, c: &Capture) -> Result<Value, &'static str> {
    let id = uuid(&c.arguments, "object_id")?;
    let idx = object(app, id)?;
    if c.tool == Tool::ObjectDialogList {
        let mut rows = Vec::new();
        for n in app.world.notifications.list.iter().filter(|n|matches!(&n.data,crate::world::notifications::Data::Dialog{object,..}|crate::world::notifications::Data::TextBox{object,..}if *object==id)).take(32){let Some(source)=app.world.assistant_observations.dialogs.get(&n.id).filter(|s|Some(s.region)==app.world.main_region&&app.world.login.as_ref().is_some_and(|l|s.source.matches_login(l,app.world.agent_id)))else{continue};let revision=dialog_revision(n,source);if state.interactions.dialogs.len()>=128{return Err("dialog_capacity");}let token=Uuid::new_v4();state.interactions.dialogs.insert(token,Dialog{capture:c.clone(),native:n.id,object:id,revision:revision.clone(),expires:Instant::now()+Duration::from_secs(120)});let buttons=match &n.data{crate::world::notifications::Data::Dialog{buttons,..}=>buttons.iter().enumerate().map(|(index,label)|json!({"index":index,"label":clipped(label,250)})).collect(),_=>Vec::new()};rows.push(json!({"notification_id":token.to_string(),"dialog_revision":revision,"object_id":id.to_string(),"message":clipped(&n.body,2048),"buttons":buttons,"text_box":matches!(n.data,crate::world::notifications::Data::TextBox{..}),"untrusted_data":true}));}
        return Ok(json!({"dialogs":rows,"untrusted_data":true}));
    }
    let request = uuid(&c.arguments, "client_request_id")?;
    let key = (c.endpoint, c.client, request);
    if let Some(old) = state.interactions.intents.get(&key) {
        return if old == c {
            existing_outcome(state, c, &c.request_id)?.ok_or("operation_pending")
        } else {
            Err("client_request_conflict")
        };
    }
    if state.interactions.intents.len() >= 4096 {
        return Err("interaction_intent_capacity");
    }
    state.interactions.intents.insert(key, c.clone());
    let mut value = submitted(c);
    value["client_request_id"] = json!(request.to_string());
    match c.tool {
        Tool::ObjectTouch => {
            touch(app, id, &c.arguments)?;
            value["script_handled_confirmed"] = json!(false);
            value["status"] = json!("outcome_unknown");
        }
        Tool::ObjectChannelSend => {
            send_channel(app, &mut state.interactions, &c.arguments)?;
            value["broadcast"] = json!(true);
            value["status"] = json!("outcome_unknown");
        }
        Tool::ObjectDialogReply => {
            let (n, source) = notification(app, &state.interactions, c)?;
            let (channel, index, label, response) = reply(n, &c.arguments)?;
            let (native, stamp) = (n.id, source.clone());
            let native_request = state.interactions.track_reply(c, native, response);
            app.send(NetCommand::AssistantDialogReply {
                source: stamp.source,
                request: native_request,
                circuit: stamp.circuit,
                region: stamp.region,
                sender: stamp.sender,
                object: id,
                channel,
                index,
                label,
            });
        }
        Tool::ObjectTake | Tool::ObjectTakeCopy => {
            let o = app.world.objects.get(idx).ok_or("object_unavailable")?;
            let destination = uuid(&c.arguments, "destination_folder")?;
            let copy = c.tool == Tool::ObjectTakeCopy;
            let (handle, local_id) = (o.key.region, o.key.local_id);
            if state.interactions.takes.len() >= 64 {
                return Err("take_capacity");
            }
            let native_request = state.interactions.track_take(c, id, destination, copy);
            app.send(NetCommand::AssistantTakeObject {
                source: aurora_net::LoginSource::from_login(app.world.login.as_ref().ok_or("native_not_ready")?),
                request: native_request,
                handle,
                object: id,
                local_id,
                destination,
                copy,
            });
        }
        _ => return Err("unsupported_capability"),
    }
    outcome(state, c, c.request_id.clone(), value.clone());
    outcome(state, c, request.to_string(), value.clone());
    Ok(value)
}
fn finished(app: &mut App, capture: Capture, value: Value) {
    if let Some(mut a) = app.character_mcp.take() {
        let proof = if let Some(id) = value
            .get("inventory_item_id")
            .and_then(Value::as_str)
            .and_then(|v| Uuid::parse_str(v).ok())
        {
            receipts::record_acquisition(app, &mut a.inventory, &capture, id)
        } else {
            receipts::record(app, &mut a.inventory, &capture, &value)
        };
        let value = if proof.is_ok() {
            value
        } else {
            json!({"status":"error","error":"native_result_access_revoked"})
        };
        outcome(&mut a.inventory, &capture, capture.request_id.clone(), value.clone());
        let _ = a.runtime.complete(&capture, value);
        app.character_mcp = Some(a);
    }
}
pub(super) fn native_event(app: &mut App, event: &NetEvent) -> bool {
    match event {
        NetEvent::AssistantTake { source, request, progress } => {
            let Some(mut take) = app
                .character_mcp
                .as_mut()
                .and_then(|a| a.inventory.interactions.takes.remove(request))
            else {
                return true;
            };
            if !super::super::source_current(app, &take.capture)
                || !app
                    .world
                    .login
                    .as_ref()
                    .is_some_and(|l| source.matches_login(l, app.world.agent_id))
            {
                return true;
            }
            take.observe(progress);
            if let Some(item) = take.confirmation(app.world.agent_id) {
                let value = json!({"status":"confirmed","operation_id":take.capture.request_id,"object_id":take.object.to_string(),"inventory_item_id":item.to_string(),"transaction_id":request.to_string(),"acknowledged":true,"source_removal_observed":take.removed,"confirmed":true,"retry_safe":false});
                finished(app, take.capture, value);
            } else if take.rejected || take.ack == Some(false) || take.items.len() > 1 {
                let value = json!({"status":"outcome_unknown","operation_id":take.capture.request_id,"confirmed":false,"retry_safe":false});
                finished(app, take.capture, value);
            } else if let Some(a) = app.character_mcp.as_mut() {
                a.inventory.interactions.takes.insert(*request, take);
            }
            true
        }
        NetEvent::AssistantDialogSubmitted { source, request, accepted } => {
            let Some(reply) = app
                .character_mcp
                .as_mut()
                .and_then(|a| a.inventory.interactions.replies.remove(request))
            else {
                return true;
            };
            if !super::super::source_current(app, &reply.capture)
                || !app
                    .world
                    .login
                    .as_ref()
                    .is_some_and(|l| source.matches_login(l, app.world.agent_id))
            {
                return true;
            }
            if *accepted {
                let _ = app
                    .world
                    .notifications
                    .respond(reply.native, reply.response, |_| Uuid::nil(), Uuid::nil());
            }
            finished(
                app,
                reply.capture.clone(),
                json!({"status":if *accepted{"submitted"}else{"rejected_native"},"operation_id":reply.capture.request_id,"native_submitted":accepted,"script_handled_confirmed":false,"retry_safe":false}),
            );
            true
        }
        _ => false,
    }
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    let now = Instant::now();
    assistant.inventory.interactions.dialogs.retain(|_, d| {
        d.expires > now
            && Some(d.capture.binding) == assistant.binding
            && d.capture.endpoint == assistant.inventory.endpoint.unwrap_or(d.capture.endpoint)
    });
    let expired: Vec<_> = assistant
        .inventory
        .interactions
        .takes
        .iter()
        .filter(|(_, t)| t.submitted.elapsed() > Duration::from_secs(60))
        .map(|(id, _)| *id)
        .collect();
    for id in expired {
        if let Some(t) = assistant.inventory.interactions.takes.remove(&id) {
            let value = json!({"status":"outcome_unknown","operation_id":t.capture.request_id,"confirmed":false,"retry_safe":false});
            outcome(&mut assistant.inventory, &t.capture, t.capture.request_id.clone(), value.clone());
            let _ = assistant.runtime.complete(&t.capture, value);
        }
    }
    let expired: Vec<_> = assistant
        .inventory
        .interactions
        .replies
        .iter()
        .filter(|(_, r)| r.submitted.elapsed() > Duration::from_secs(20))
        .map(|(id, _)| *id)
        .collect();
    for id in expired {
        if let Some(r) = assistant.inventory.interactions.replies.remove(&id) {
            let value = json!({"status":"outcome_unknown","operation_id":r.capture.request_id,"native_submitted":false,"script_handled_confirmed":false,"retry_safe":false});
            outcome(&mut assistant.inventory, &r.capture, r.capture.request_id.clone(), value.clone());
            let _ = assistant.runtime.complete(&r.capture, value);
        }
    }
    let _ = app;
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::NativeTakeProgress as P;
    #[test]
    fn same_caller_uuid_across_clients_never_collides_in_native_take_or_dialog_callbacks() {
        let mut state = State::default();
        let mut a = synthetic_capture(Tool::ObjectTake);
        let caller = Uuid::new_v4();
        a.arguments.insert("client_request_id".into(), json!(caller.to_string()));
        let mut b = a.clone();
        b.client += 1;
        b.request_id = "other-operation".into();
        let agent = Uuid::new_v4();
        let destination = Uuid::new_v4();
        let a_object = Uuid::new_v4();
        let b_object = Uuid::new_v4();
        let ar = state.track_take(&a, a_object, destination, false);
        let br = state.track_take(&b, b_object, destination, true);
        assert_ne!(ar, br);
        assert_eq!(state.takes.len(), 2);
        let ai = InvItem {
            id: Uuid::new_v4(),
            asset_type: 6,
            owner: agent,
            parent: destination,
            ..Default::default()
        };
        let bi = InvItem {
            id: Uuid::new_v4(),
            asset_type: 6,
            owner: agent,
            parent: destination,
            ..Default::default()
        };
        state.takes.get_mut(&ar).unwrap().observe(&P::Ack(true));
        assert_eq!(state.takes[&ar].confirmation(agent), None);
        state.takes.get_mut(&br).unwrap().observe(&P::Items(vec![bi.clone()]));
        state.takes.get_mut(&br).unwrap().observe(&P::Ack(true));
        assert_eq!(state.takes[&br].confirmation(agent), Some(bi.id));
        state.takes.get_mut(&ar).unwrap().observe(&P::Items(vec![ai.clone()]));
        assert_eq!(state.takes[&ar].confirmation(agent), None);
        state.takes.get_mut(&ar).unwrap().observe(&P::Removed);
        assert_eq!(state.takes[&ar].confirmation(agent), Some(ai.id));
        let completed_b = state.takes.remove(&br).unwrap();
        assert_eq!(completed_b.capture, b);
        assert_eq!(completed_b.object, b_object);
        assert_eq!(state.takes[&ar].capture, a);
        assert_eq!(state.takes[&ar].object, a_object);
        let ra = state.track_reply(&a, 11, crate::world::notifications::Response::Button(0));
        let rb = state.track_reply(&b, 22, crate::world::notifications::Response::Button(1));
        assert_ne!(ra, rb);
        assert_eq!(state.replies.remove(&rb).unwrap().native, 22);
        assert_eq!(state.replies[&ra].native, 11);
        assert_eq!(state.replies[&ra].capture, a);
    }
    #[test]
    fn derez_ack_never_confirms_a_wrong_destination_owner_link_or_ambiguous_item() {
        let c = synthetic_capture(Tool::ObjectTakeCopy);
        let agent = Uuid::new_v4();
        let destination = Uuid::new_v4();
        for fault in 0..4 {
            let mut state = State::default();
            let r = state.track_take(&c, Uuid::new_v4(), destination, true);
            let take = state.takes.get_mut(&r).unwrap();
            take.observe(&P::Ack(true));
            let mut item = InvItem {
                id: Uuid::new_v4(),
                asset_type: 6,
                owner: agent,
                parent: destination,
                ..Default::default()
            };
            match fault {
                0 => item.parent = Uuid::new_v4(),
                1 => item.owner = Uuid::new_v4(),
                2 => item.asset_type = 24,
                _ => {}
            }
            take.observe(&P::Items(vec![item]));
            if fault == 3 {
                take.observe(&P::Items(vec![InvItem {
                    id: Uuid::new_v4(),
                    asset_type: 6,
                    owner: agent,
                    parent: destination,
                    ..Default::default()
                }]));
            }
            assert_eq!(take.confirmation(agent), None);
        }
    }
    #[test]
    fn frozen_approval_scope_rejects_changed_linkset_recipe_or_authority() {
        let evidence = FrozenEvidence::default();
        let c = synthetic_capture(Tool::ObjectTake);
        let original = json!({"root":{"owner":"same"},"children":["child-a"]});
        freeze_capture(&evidence, &c, original.clone()).unwrap();
        let mut classes = c.clone();
        classes.required_classes.push(Class::WorldRead);
        freeze_capture(&evidence, &classes, original.clone()).unwrap();
        assert_eq!(
            freeze_capture(&evidence, &c, json!({"root":{"owner":"same"},"children":["child-b"]})),
            Err("native_scope_changed")
        );
        let mut changed = c.clone();
        changed.policy_revision += 1;
        assert_eq!(freeze_capture(&evidence, &changed, original), Err("native_scope_changed"));
    }
}
