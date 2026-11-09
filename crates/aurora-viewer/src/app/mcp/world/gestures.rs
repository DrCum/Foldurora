use super::*;
use crate::gestures::{Effect, Manager, PlaybackId};
use crate::world::inventory::actions as rules;
use aurora_assets::gesture::{ANIM_STOP, Gesture, Step, WAIT_ANIMATIONS, WAIT_RELEASE, WAIT_TIME};

#[derive(Default)]
pub(super) struct State {
    inspected: BTreeMap<(u64, Uuid), Inspected>,
    pending: BTreeMap<String, (Capture, Uuid, Uuid)>,
    pub(super) io: Option<(crossbeam_channel::Sender<Reply>, crossbeam_channel::Receiver<Reply>)>,
    manager: Manager,
    playback: Option<Playback>,
    clients: BTreeSet<(u64, String)>,
    revisions: BTreeMap<Uuid, u64>,
    serial: u64,
    last: BTreeMap<(u64, Uuid), Value>,
    completed: Vec<(Capture, Value)>,
    receipts: BTreeMap<(HostId, u64, String), Receipt>,
}
struct Receipt {
    capture: Capture,
    items: Vec<aurora_net::inventory::InvItem>,
}
impl Receipt {
    fn current(&self, inv: &crate::world::inventory::Inventory, agent: Uuid) -> bool {
        self.items
            .iter()
            .all(|old| current_item(inv, agent, old.id).is_ok_and(|now| now == old))
    }
}
fn receipt_key(c: &Capture) -> (HostId, u64, String) {
    (c.endpoint, c.client, c.request_id.clone())
}
fn record(state: &mut State, c: &Capture, items: Vec<aurora_net::inventory::InvItem>) -> Result<(), &'static str> {
    let key = receipt_key(c);
    if state.receipts.len() >= 128 && !state.receipts.contains_key(&key) {
        return Err("gesture_receipt_capacity");
    }
    state.receipts.insert(key, Receipt { capture: c.clone(), items });
    Ok(())
}
pub(super) fn validate_receipt(app: &App, state: &super::State, c: &Capture) -> bool {
    state
        .gestures
        .receipts
        .get(&receipt_key(c))
        .is_some_and(|proof| proof.capture == *c && proof.current(&app.world.inventory, app.world.agent_id))
}
#[derive(Clone)]
struct Inspected {
    item: aurora_net::inventory::InvItem,
    hash: String,
    definition: Gesture,
    animations: Vec<(Uuid, String)>,
    expires: Instant,
}
pub(super) struct Reply {
    capture: Capture,
    item: aurora_net::inventory::InvItem,
    result: Result<Inspected, &'static str>,
}
struct Playback {
    capture: Capture,
    id: Uuid,
    item: Uuid,
    native: PlaybackId,
    deadline: Instant,
    owned: BTreeMap<Uuid, u64>,
    cleanup_complete: bool,
}
pub(super) fn supports(t: Tool) -> bool {
    matches!(t, Tool::GestureList | Tool::GestureStatus | Tool::GesturePlay | Tool::GestureStop)
}
fn item(app: &App, id: Uuid) -> Result<&aurora_net::inventory::InvItem, &'static str> {
    current_item(&app.world.inventory, app.world.agent_id, id)
}
fn current_item(inv: &crate::world::inventory::Inventory, agent: Uuid, id: Uuid) -> Result<&aurora_net::inventory::InvItem, &'static str> {
    if inv.owner != agent || !inv.can_preview(21) {
        return Err("native_gesture_restriction");
    }
    let item = inv
        .items
        .get(&id)
        .filter(|it| it.asset_type == 21 && !it.asset_id.is_nil())
        .ok_or("gesture_item_not_found")?;
    if item.owner_mask & rules::COPY == 0
        || rules::in_type(inv, id, 14)
        || (!rules::under(inv, id, inv.root) && !(inv.lib_root != Uuid::nil() && rules::under(inv, id, inv.lib_root)))
    {
        return Err("gesture_copy_permission_required");
    }
    Ok(item)
}
fn inspected<'a>(app: &App, state: &'a super::State, c: &Capture) -> Result<&'a Inspected, &'static str> {
    let id = uuid(&c.arguments, "item_id")?;
    let entry = state
        .gestures
        .inspected
        .get(&(c.client, id))
        .filter(|s| s.expires > Instant::now())
        .ok_or("gesture_inspection_required")?;
    if item(app, id)? != &entry.item
        || uuid(&c.arguments, "asset_id")? != entry.item.asset_id
        || string(&c.arguments, "asset_hash", 64)? != entry.hash
    {
        return Err("gesture_inspection_changed");
    }
    Ok(entry)
}
fn allowed(app: &App, g: &Gesture) -> Result<(), &'static str> {
    if app.world.rlva.has("sendgesture") {
        return Err("native_gesture_restriction");
    }
    for step in &g.steps {
        match step {
            Step::Animation { asset, flags, .. } => {
                if app.world.derender.animation_blocked(asset)
                    || !app.world.rlva_allows_command(
                        &NetCommand::AgentAnimation {
                            anim: *asset,
                            start: flags & ANIM_STOP == 0,
                        },
                        Uuid::nil(),
                    )
                {
                    return Err("native_animation_restriction");
                }
            }
            Step::Sound { asset, .. } => {
                if app.world.sound_blocked(&app.world.agent_id, &app.world.agent_id)
                    || app.world.sound_blacklisted(*asset, app.world.agent_id, app.world.agent_id)
                {
                    return Err("native_sound_restriction");
                }
            }
            Step::Chat { text, .. } => {
                private_read(app)?;
                if app.world.rlva.outgoing_chat(text, 0, aurora_net::ChatType::Normal).is_empty() {
                    return Err("native_chat_restriction");
                }
            }
            Step::Wait { .. } => {}
        }
    }
    Ok(())
}
pub(super) fn prepare(app: &App, state: &super::State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    let mut classes = Vec::new();
    match c.tool {
        Tool::GestureList => {
            args(c, &["limit"])?;
            limit(&c.arguments, 50, 100)?;
        }
        Tool::GestureStatus => {
            args(c, &["item_id"])?;
            item(app, uuid(&c.arguments, "item_id")?)?;
            if !app.world.main_has_cap("ViewerAsset") {
                return Err("native_capability_unavailable");
            }
        }
        Tool::GesturePlay => {
            args(c, &["item_id", "asset_id", "asset_hash", "client_request_id"])?;
            let inspected = inspected(app, state, c)?;
            let client = string(&c.arguments, "client_request_id", 128)?;
            if client.is_empty() {
                return Err("invalid_args");
            }
            if state.gestures.playback.is_some() || app.gesture_state.manager.is_playing(inspected.item.id) {
                return Err("native_gesture_busy");
            }
            allowed(app, &inspected.definition)?;
            classes.push(Class::GestureRead);
            if inspected.definition.steps.iter().any(|s| matches!(s, Step::Chat { .. })) {
                classes.push(Class::ChatSendNearby);
            }
        }
        Tool::GestureStop => {
            args(c, &["item_id", "playback_id"])?;
            let id = uuid(&c.arguments, "item_id")?;
            let playback = uuid(&c.arguments, "playback_id")?;
            if !state
                .gestures
                .playback
                .as_ref()
                .is_some_and(|p| p.item == id && p.id == playback && p.capture.client == c.client && p.capture.binding == c.binding)
            {
                return Err("gesture_playback_not_owned");
            }
            classes.push(Class::GestureRead);
        }
        _ => return Err("unsupported_capability"),
    };
    Ok(classes)
}
pub(super) fn evidence(app: &App, state: &super::State, c: &Capture) -> Result<Value, &'static str> {
    Ok(match c.tool {
        Tool::GestureStatus => {
            let i = item(app, uuid(&c.arguments, "item_id")?)?;
            json!({"item_id":i.id.to_string(),"asset_id":i.asset_id.to_string(),"permissions":i.owner_mask,"parent":i.parent.to_string()})
        }
        Tool::GesturePlay => {
            let i = inspected(app, state, c)?;
            json!({"item_id":i.item.id.to_string(),"asset_id":i.item.asset_id.to_string(),"hash":i.hash,"effects":effects(&i.definition),"animations":i.animations.iter().map(|(id,hash)|json!({"id":id.to_string(),"hash":hash})).collect::<Vec<_>>()})
        }
        Tool::GestureStop => json!({"playback":state.gestures.playback.as_ref().map(|p|p.id.to_string())}),
        _ => Value::Null,
    })
}
fn effects(g: &Gesture) -> Vec<Value> {
    g.steps
        .iter()
        .map(|s| match s {
            Step::Animation { asset, flags, .. } => json!({"kind":"animation","asset_id":asset.to_string(),"start":flags&ANIM_STOP==0}),
            Step::Sound { asset, .. } => json!({"kind":"sound","asset_id":asset.to_string()}),
            Step::Chat { text, .. } => json!({"kind":"chat","text":text,"channel":0,"literal":true}),
            Step::Wait { seconds, flags } => json!({"kind":"wait","seconds":seconds,"flags":flags}),
        })
        .collect()
}
fn result(state: &State, client: u64, entry: &Inspected) -> Value {
    json!({"item_id":entry.item.id.to_string(),"asset_id":entry.item.asset_id.to_string(),"asset_hash":entry.hash,"name":clip(&entry.item.name,255),"effects":effects(&entry.definition),"animation_hashes":entry.animations.iter().map(|(id,hash)|json!({"asset_id":id.to_string(),"sha256":hash})).collect::<Vec<_>>(),"playback":state.last.get(&(client,entry.item.id)).cloned().unwrap_or(Value::Null),"activated":false,"untrusted_data":true})
}
async fn asset(http: &reqwest::Client, cap: &str, kind: &str, id: Uuid, max: usize) -> Result<Vec<u8>, &'static str> {
    let url = format!("{cap}{}{kind}={id}", if cap.contains('?') { "&" } else { "?" });
    let mut response = http
        .get(url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| "gesture_asset_transport")?;
    if !response.status().is_success() {
        return Err("gesture_asset_unavailable");
    }
    if response.content_length().is_some_and(|n| n > max as u64) {
        return Err("gesture_asset_limit");
    }
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "gesture_asset_transport")? {
        if chunk.len() > max - data.len() {
            return Err("gesture_asset_limit");
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}
async fn inspect(http: reqwest::Client, cap: String, item: aurora_net::inventory::InvItem) -> Result<Inspected, &'static str> {
    let raw = asset(&http, &cap, "gesture_id", item.asset_id, 16 * 1024).await?;
    let definition = Gesture::decode(&raw).map_err(|_| "gesture_invalid")?;
    if serde_json::to_vec(&effects(&definition)).map_or(true, |b| b.len() > 16 * 1024) {
        return Err("gesture_effect_summary_limit");
    }
    if definition.steps.len() > 64 {
        return Err("gesture_step_limit");
    }
    let mut animations = BTreeSet::new();
    let mut wait = 0.;
    for step in &definition.steps {
        match step {
            Step::Animation { asset, flags, .. } => {
                if asset.is_nil() || flags & !ANIM_STOP != 0 || crate::gestures::builtins::ANIMATIONS.iter().any(|(_, id)| id == asset) {
                    return Err("gesture_builtin_or_unsupported_animation");
                }
                animations.insert(*asset);
            }
            Step::Sound { asset, flags, .. } => {
                if asset.is_nil() || *flags != 0 {
                    return Err("gesture_unsupported_sound");
                }
            }
            Step::Chat { text, flags } => {
                if *flags != 0 || text.is_empty() || text.len() > 1023 || text.chars().any(|c| c.is_control()) {
                    return Err("gesture_unsupported_chat");
                }
            }
            Step::Wait { seconds, flags } => {
                if flags & !(WAIT_TIME | WAIT_ANIMATIONS | WAIT_RELEASE) != 0
                    || flags & WAIT_RELEASE != 0
                    || !seconds.is_finite()
                    || *seconds > 120.
                {
                    return Err("gesture_unsupported_wait");
                }
                if flags & WAIT_TIME != 0 {
                    wait += seconds;
                }
                if wait > 120. {
                    return Err("gesture_duration_limit");
                }
            }
        }
    }
    if animations.len() > 16 {
        return Err("gesture_animation_limit");
    }
    let mut hashes = Vec::new();
    let mut total = 0usize;
    for id in animations {
        let raw = asset(&http, &cap, "animatn_id", id, 1024 * 1024).await?;
        total += raw.len();
        if total > 4 * 1024 * 1024 {
            return Err("gesture_animation_limit");
        }
        if raw.get(..4) != Some(&[1, 0, 0, 0]) {
            return Err("gesture_animation_version");
        }
        let decoded = aurora_assets::anim::parse_animation(&raw).map_err(|_| "gesture_animation_invalid")?;
        if !decoded.emote_name.is_empty() {
            return Err("gesture_animation_embedded_emote");
        }
        hashes.push((id, hash(&raw)));
    }
    Ok(Inspected {
        item,
        hash: hash(&raw),
        definition,
        animations: hashes,
        expires: Instant::now() + Duration::from_secs(300),
    })
}
pub(super) fn execute(app: &mut App, assistant: &mut Assistant, c: &Capture) -> Result<Value, &'static str> {
    match c.tool {
        Tool::GestureList => {
            let limit = limit(&c.arguments, 50, 100)?;
            let mut ids = app.gesture_state.manager.active.clone();
            ids.extend(
                assistant
                    .world
                    .gestures
                    .inspected
                    .keys()
                    .filter(|(client, _)| *client == c.client)
                    .map(|(_, id)| *id),
            );
            let disclosed: Vec<_> = ids.iter().take(limit).filter_map(|id| item(app, *id).ok().cloned()).collect();
            let items:Vec<_>=disclosed.iter().map(|i|json!({"item_id":i.id.to_string(),"name":clip(&i.name,255),"active":app.gesture_state.manager.active.contains(&i.id),"assistant_inspected":assistant.world.gestures.inspected.contains_key(&(c.client,i.id)),"native_playing":app.gesture_state.manager.is_playing(i.id)})).collect();
            record(&mut assistant.world.gestures, c, disclosed)?;
            Ok(json!({"items":items,"limited":ids.len()>limit,"coverage":"active_and_this_client_inspected","untrusted_data":true}))
        }
        Tool::GestureStatus => {
            let id = uuid(&c.arguments, "item_id")?;
            let item = item(app, id)?.clone();
            record(&mut assistant.world.gestures, c, vec![item.clone()])?;
            if let Some(entry) = assistant
                .world
                .gestures
                .inspected
                .get(&(c.client, id))
                .filter(|e| e.item == item && e.expires > Instant::now())
            {
                return Ok(result(&assistant.world.gestures, c.client, entry));
            }
            if assistant.world.gestures.pending.len() >= 4 || assistant.world.gestures.inspected.len() >= 32 {
                return Err("gesture_inspection_capacity");
            }
            let cap = app
                .world
                .main()
                .and_then(|r| r.caps.get("ViewerAsset"))
                .cloned()
                .ok_or("native_capability_unavailable")?;
            let (tx, _) = assistant.world.gestures.io.get_or_insert_with(|| crossbeam_channel::bounded(4));
            let tx = tx.clone();
            let capture = c.clone();
            let http = app.net.caps_http();
            assistant
                .world
                .gestures
                .pending
                .insert(c.request_id.clone(), (c.clone(), id, item.asset_id));
            app.net.runtime().spawn(async move {
                let result = inspect(http, cap, item.clone()).await;
                let _ = tx.try_send(Reply { capture, item, result });
            });
            Ok(submitted(c))
        }
        Tool::GesturePlay => {
            let inspected = inspected(app, &assistant.world, c)?.clone();
            record(&mut assistant.world.gestures, c, vec![inspected.item.clone()])?;
            let client = string(&c.arguments, "client_request_id", 128)?.to_owned();
            if assistant.world.gestures.clients.contains(&(c.client, client.clone())) {
                return Err("client_request_already_used");
            }
            if assistant.world.gestures.clients.len() >= 4096 {
                return Err("gesture_intent_capacity");
            }
            allowed(app, &inspected.definition)?;
            assistant
                .world
                .gestures
                .manager
                .play(inspected.item.id, inspected.definition.clone(), None);
            let native = assistant
                .world
                .gestures
                .manager
                .playback_id(inspected.item.id)
                .ok_or("native_gesture_unavailable")?;
            let id = Uuid::new_v4();
            assistant.world.gestures.clients.insert((c.client, client));
            assistant.world.gestures.playback = Some(Playback {
                capture: c.clone(),
                id,
                item: inspected.item.id,
                native,
                deadline: Instant::now() + Duration::from_secs(120),
                owned: BTreeMap::new(),
                cleanup_complete: true,
            });
            app.gesture_prefetch(&inspected.definition);
            let value = json!({"status":"submitted","playback_id":id.to_string(),"item_id":inspected.item.id.to_string(),"operation_id":c.request_id,"delivery_confirmed":false,"activated":false,"retry_safe":false});
            assistant.world.gestures.last.insert((c.client, inspected.item.id), value.clone());
            Ok(value)
        }
        Tool::GestureStop => {
            record(&mut assistant.world.gestures, c, Vec::new())?;
            let id = uuid(&c.arguments, "playback_id")?;
            let playback = assistant
                .world
                .gestures
                .playback
                .as_ref()
                .filter(|p| p.id == id && p.capture.client == c.client)
                .ok_or("gesture_playback_not_owned")?;
            let item = playback.item;
            cancel(app, assistant, "stopped");
            Ok(assistant.world.gestures.last.get(&(c.client, item)).cloned().unwrap_or(Value::Null))
        }
        _ => Err("unsupported_capability"),
    }
}
fn revision(state: &mut State, anim: Uuid) -> u64 {
    if state.revisions.len() >= 1024 && !state.revisions.contains_key(&anim) {
        state.revisions.clear();
        if let Some(p) = state.playback.as_mut() {
            p.cleanup_complete = false;
        }
    }
    state.serial = state.serial.saturating_add(1);
    state.revisions.insert(anim, state.serial);
    state.serial
}
fn dispatch(app: &mut App, assistant: &mut Assistant, effects: Vec<Effect>, cleanup: bool) {
    for effect in effects {
        match effect {
            Effect::Animation(anim, start) => {
                let can_stop = assistant
                    .world
                    .gestures
                    .playback
                    .as_ref()
                    .and_then(|p| p.owned.get(&anim))
                    .is_some_and(|r| assistant.world.gestures.revisions.get(&anim) == Some(r));
                if !start && !can_stop {
                    if let Some(p) = assistant.world.gestures.playback.as_mut() {
                        p.cleanup_complete = false;
                    }
                    continue;
                }
                if start && app.world.animations_of(&app.world.agent_id).iter().any(|a| a.id == anim) {
                    if let Some(p) = assistant.world.gestures.playback.as_mut() {
                        p.cleanup_complete = false;
                    }
                    continue;
                }
                let seq = revision(&mut assistant.world.gestures, anim);
                if let Some(p) = assistant.world.gestures.playback.as_mut() {
                    if start {
                        p.owned.insert(anim, seq);
                    } else {
                        p.owned.remove(&anim);
                    }
                }
                app.gesture_effects(vec![Effect::Animation(anim, start)]);
            }
            Effect::Sound(asset) => {
                if !cleanup {
                    app.gesture_effects(vec![Effect::Sound(asset)]);
                }
            }
            Effect::Chat(text) => {
                if !cleanup {
                    app.send(NetCommand::Chat {
                        message: text,
                        channel: 0,
                        chat_type: aurora_net::ChatType::Normal,
                    });
                }
            }
        }
    }
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    assistant
        .world
        .gestures
        .receipts
        .retain(|_, proof| assistant.runtime.has_operation(&proof.capture.request_id));
    for (capture, result) in std::mem::take(&mut assistant.world.gestures.completed) {
        let _ = assistant.runtime.complete(&capture, result);
    }
    let replies: Vec<_> = assistant
        .world
        .gestures
        .io
        .as_ref()
        .map(|(_, rx)| rx.try_iter().take(4).collect())
        .unwrap_or_default();
    for reply in replies {
        assistant.world.gestures.pending.remove(&reply.capture.request_id);
        if !current(app, assistant, &reply.capture)
            || item(app, reply.item.id).is_err()
            || item(app, reply.item.id).is_ok_and(|i| i != &reply.item)
        {
            continue;
        }
        let value = match reply.result {
            Ok(entry) => {
                let value = result(&assistant.world.gestures, reply.capture.client, &entry);
                assistant
                    .world
                    .gestures
                    .inspected
                    .insert((reply.capture.client, entry.item.id), entry);
                value
            }
            Err(code) => failure(code),
        };
        let _ = assistant.runtime.complete(&reply.capture, value);
    }
    assistant.world.gestures.inspected.retain(|_, s| s.expires > Instant::now());
    let Some(p) = assistant.world.gestures.playback.as_ref() else {
        return;
    };
    let valid = current(app, assistant, &p.capture)
        && super::super::active_allowed(app)
        && Instant::now() < p.deadline
        && assistant
            .world
            .gestures
            .inspected
            .get(&(p.capture.client, p.item))
            .is_some_and(|i| item(app, p.item).is_ok_and(|native| native == &i.item) && allowed(app, &i.definition).is_ok());
    if !valid {
        cancel(app, assistant, "revoked_or_expired");
        return;
    }
    let signaled = app.world.animations_of(&app.world.agent_id).iter().map(|a| a.id).collect();
    let effects = assistant
        .world
        .gestures
        .manager
        .tick_ready(Instant::now(), &signaled, |step| match step {
            Step::Animation { asset, flags, .. } => flags & ANIM_STOP != 0 || app.scene.anims.asset_settled(asset),
            Step::Sound { asset, .. } => app.scene.sounds.asset_settled(asset),
            _ => true,
        });
    let capture = assistant.world.gestures.playback.as_ref().map(|p| p.capture.clone());
    if let Some(capture) = capture {
        let engine = assistant.runtime.clone();
        if engine
            .continue_action(&capture, || dispatch(app, assistant, effects, false))
            .is_err()
        {
            cancel(app, assistant, "revoked");
            return;
        }
    }
    if !assistant.world.gestures.manager.has_playing() {
        cancel(app, assistant, "completed");
    }
}
pub(super) fn cancel(app: &mut App, assistant: &mut Assistant, reason: &'static str) {
    let Some(p) = assistant.world.gestures.playback.as_ref() else {
        return;
    };
    let (item, native) = (p.item, p.native);
    let effects = assistant.world.gestures.manager.stop_exact(item, native);
    if assistant
        .world
        .gestures
        .playback
        .as_ref()
        .is_some_and(|p| assistant.binding == Some(p.capture.binding))
    {
        dispatch(app, assistant, effects, true);
    }
    if let Some(p) = assistant.world.gestures.playback.take() {
        let value = json!({"status":reason,"playback_id":p.id.to_string(),"item_id":p.item.to_string(),"cleanup_complete":p.cleanup_complete,"chat_and_sound_recalled":false,"retry_safe":false});
        assistant.world.gestures.last.insert((p.capture.client, p.item), value.clone());
        assistant.world.gestures.completed.push((p.capture, value));
    }
}
pub(super) fn native_command(app: &mut App, command: &NetCommand) {
    if let NetCommand::AgentAnimation { anim, .. } = command
        && let Some(a) = app.character_mcp.as_mut()
    {
        revision(&mut a.world.gestures, *anim);
    }
}
pub(super) fn native_event(app: &mut App, event: &NetEvent) {
    if let NetEvent::AvatarAnimations { avatar, anims, sources } = event
        && *avatar == app.world.agent_id
        && let Some(a) = app.character_mcp.as_mut()
    {
        for ((id, _), source) in anims.iter().zip(sources) {
            if *source != app.world.agent_id {
                revision(&mut a.world.gestures, *id);
            }
        }
    }
}

#[cfg(test)]
mod receipt_tests {
    use super::*;
    #[test]
    fn inspected_gesture_receipt_rejects_creator_rights_asset_or_source_replacement() {
        use crate::world::inventory::{Folder, Inventory};
        use aurora_net::inventory::{InvFolder, InvItem};
        let owner = Uuid::from_u128(7101);
        let root = Uuid::from_u128(7102);
        let original = InvItem {
            id: Uuid::from_u128(7103),
            parent: root,
            owner,
            asset_id: Uuid::from_u128(7104),
            asset_type: 21,
            owner_mask: rules::COPY | rules::TRANSFER,
            ..Default::default()
        };
        let mut inv = Inventory {
            owner,
            root,
            ..Default::default()
        };
        inv.folders.insert(
            root,
            Folder {
                info: InvFolder {
                    id: root,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: crate::world::inventory::FetchState::Fetched,
                library: false,
            },
        );
        inv.add_items(vec![original.clone()]);
        // A receipt freezes the original item, independently of the mutable
        // inspection cache. A later cache refresh cannot bless this old body.
        let current = |inv: &Inventory| current_item(inv, owner, original.id).is_ok_and(|i| i == &original);
        assert!(current(&inv));
        for change in 0..3 {
            let mut changed = original.clone();
            match change {
                0 => changed.owner_mask &= !rules::COPY,
                1 => changed.asset_id = Uuid::from_u128(7105),
                _ => changed.owner = Uuid::from_u128(7106),
            }
            inv.add_items(vec![changed]);
            assert!(!current(&inv));
        }
        inv.items.clear();
        assert!(!current(&inv));
    }
}
