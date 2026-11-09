use super::*;
use crate::world::{ChatKind, Origin};
use aurora_session::ConversationId;
use aurora_session::chat::Conversation;

#[derive(Default)]
pub(super) struct State {
    scans: BTreeMap<String, (u64, u64)>,
    lines: BTreeMap<String, VecDeque<Value>>,
    hidden: BTreeMap<(u64, String), u64>,
    drafts: BTreeMap<(u64, String), (u64, String)>,
    sends: VecDeque<Instant>,
    clients: BTreeSet<(u64, String)>,
    search: Option<Search>,
    cursors: BTreeMap<Uuid, Cursor>,
    profiles: BTreeMap<String, (Capture, Instant, Uuid)>,
    profile_io: Option<(crossbeam_channel::Sender<ProfileReply>, crossbeam_channel::Receiver<ProfileReply>)>,
}
struct Cursor {
    client: u64,
    endpoint: HostId,
    epoch: Uuid,
    topics: Vec<String>,
    public: u64,
    private: u64,
}
#[derive(Clone)]
struct Search {
    capture: Capture,
    request: Uuid,
    query: String,
    source: aurora_net::LoginSource,
    started: Instant,
}
struct ProfileReply {
    capture: Capture,
    resident: Uuid,
    result: Result<aurora_net::profile::AvatarProfile, &'static str>,
}

pub(super) fn supports(t: Tool) -> bool {
    matches!(
        t,
        Tool::ChatConversations
            | Tool::ChatHistory
            | Tool::ChatSearch
            | Tool::FriendsList
            | Tool::ResidentSearch
            | Tool::ResidentProfile
            | Tool::ChatOpenIm
            | Tool::ChatDraftGet
            | Tool::ChatDraftSet
            | Tool::ChatSend
            | Tool::ChatMarkRead
            | Tool::ChatClose
            | Tool::EventsPoll
            | Tool::NotificationsList
            | Tool::NotificationsGet
            | Tool::NotificationReview
    )
}
fn conversation(app: &App, id: &str) -> Result<Conversation, &'static str> {
    if id == "nearby" {
        return Ok(Conversation::Nearby);
    }
    let id = Uuid::parse_str(id).ok().filter(|id| !id.is_nil()).ok_or("invalid_args")?;
    if !app.world.social.ims.iter().take(2048).any(|s| s.other == id) {
        return Err("conversation_not_found");
    }
    let native = ConversationId::from_bytes(*id.as_bytes()).map_err(|_| "invalid_args")?;
    if app.world.shared_group_initialized(id) {
        Ok(Conversation::Group(native))
    } else if app.world.shared_conference_initialized(id) {
        Ok(Conversation::Conference(native))
    } else if !app.world.is_group_session(&id) && !app.world.groups.is_member(&id) {
        Ok(Conversation::Direct(native))
    } else {
        Err("conversation_unavailable")
    }
}
fn key(c: Conversation) -> String {
    match c {
        Conversation::Nearby => "nearby".into(),
        Conversation::Direct(id) | Conversation::Group(id) | Conversation::Conference(id) => Uuid::from_bytes(*id.as_bytes()).to_string(),
    }
}
fn topic(c: Conversation) -> &'static str {
    match c {
        Conversation::Nearby => "nearby",
        Conversation::Direct(_) => "im",
        Conversation::Group(_) => "group",
        Conversation::Conference(_) => "conference",
    }
}
fn native_id(c: Conversation) -> Option<Uuid> {
    match c {
        Conversation::Nearby => None,
        Conversation::Direct(id) | Conversation::Group(id) | Conversation::Conference(id) => Some(Uuid::from_bytes(*id.as_bytes())),
    }
}
fn policy(app: &App, c: Conversation, send: bool) -> Result<(), &'static str> {
    private_read(app)?;
    let p = super::super::super::host_chat::snapshot(&app.world, c, !app.exit_prompt.open).ok_or("conversation_not_found")?;
    if !p.ready || if send { !p.can_send } else { !p.can_read } {
        Err("native_chat_restriction")
    } else {
        Ok(())
    }
}
fn native_lines(
    app: &App,
    c: Conversation,
) -> Result<(&VecDeque<crate::world::ChatLine>, crate::world::transcripts::LineClock), &'static str> {
    match native_id(c) {
        None => Ok((&app.world.chat, app.world.chat_clock)),
        Some(id) => app
            .world
            .social
            .ims
            .iter()
            .take(2048)
            .find(|s| s.other == id)
            .map(|s| (&s.lines, s.clock))
            .ok_or("conversation_not_found"),
    }
}
fn all(app: &App) -> Vec<Conversation> {
    let mut out = vec![Conversation::Nearby];
    out.extend(
        app.world
            .social
            .ims
            .iter()
            .take(63)
            .filter_map(|s| conversation(app, &s.other.to_string()).ok()),
    );
    out
}
fn target(app: &App, c: &Capture) -> Result<Conversation, &'static str> {
    conversation(app, string(&c.arguments, "conversation_id", 36)?)
}
fn grid(app: &App) -> String {
    crate::session_host::transport::grid_key(&app.settings.login_uri())
        .map(|g| g.to_string())
        .unwrap_or_default()
}
fn resident(app: &App, c: &Capture) -> Result<Uuid, &'static str> {
    let id = uuid(&c.arguments, "resident_id")?;
    if string(&c.arguments, "grid", 128)? != grid(app) || !app.world.rlva_can_name(id) {
        return Err("resident_grid_mismatch");
    }
    Ok(id)
}
fn eligible(app: &App, id: Uuid) -> bool {
    id != app.world.agent_id
        && !app.world.is_group_session(&id)
        && !app.world.groups.is_member(&id)
        && app.world.shared_chat_direct_readable(id)
        && app.world.allows_direct_contact(id)
        && app.world.rlva_can_start_im(id)
        && app.world.rlva_allows_im("sendim", id)
}
fn text<'a>(c: &'a Capture, key: &str, empty: bool) -> Result<&'a str, &'static str> {
    let t = string(&c.arguments, key, 1023)?;
    if (!empty && t.trim().is_empty()) || t.chars().any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')) {
        Err("invalid_text")
    } else {
        Ok(t)
    }
}
fn pagination(c: &Capture) -> Result<(), &'static str> {
    limit(&c.arguments, 50, ROWS)?;
    count(&c.arguments, "offset", 0, 2048)?;
    if let Some(v) = c.arguments.get("snapshot_id") {
        Uuid::parse_str(v.as_str().ok_or("invalid_args")?).map_err(|_| "invalid_args")?;
    }
    Ok(())
}
fn topics(c: &Capture) -> Result<Vec<String>, &'static str> {
    let values = c
        .arguments
        .get("topics")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty() && v.len() <= 4)
        .ok_or("invalid_args")?;
    let mut out = BTreeSet::new();
    for v in values {
        let t = v
            .as_str()
            .filter(|s| matches!(*s, "chat" | "contacts" | "notifications" | "operations"))
            .ok_or("invalid_args")?;
        if !out.insert(t.to_owned()) {
            return Err("invalid_args");
        }
    }
    Ok(out.into_iter().collect())
}
fn notification<'a>(
    app: &'a App,
    state: &super::State,
    c: &Capture,
) -> Result<&'a crate::world::notifications::Notification, &'static str> {
    let id = uuid(&c.arguments, "notification_id")?;
    let native = state
        .notifications
        .iter()
        .find_map(|(n, i)| (*i == id).then_some(*n))
        .ok_or("notification_not_found")?;
    app.world
        .notifications
        .listed()
        .find(|n| n.id == native)
        .ok_or("notification_not_found")
}

pub(super) fn prepare(app: &mut App, state: &super::State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    private_read(app)?;
    let a = &c.arguments;
    let mut extra = Vec::new();
    match c.tool {
        Tool::ChatConversations | Tool::FriendsList | Tool::NotificationsList => {
            args(c, &["limit", "offset", "snapshot_id"])?;
            pagination(c)?;
        }
        Tool::ChatHistory | Tool::ChatDraftGet | Tool::ChatMarkRead | Tool::ChatClose => {
            args(c, &["conversation_id", "limit", "offset"])?;
            pagination(c)?;
            let target = target(app, c)?;
            policy(app, target, false)?;
            if c.tool == Tool::ChatClose && target == Conversation::Nearby {
                return Err("nearby_cannot_close");
            }
        }
        Tool::ChatSearch | Tool::ResidentSearch => {
            args(c, &["query", "conversation_id", "limit", "offset", "snapshot_id"])?;
            pagination(c)?;
            let query = string(a, "query", 127)?;
            if query.len() < if c.tool == Tool::ResidentSearch { 2 } else { 1 } || query.chars().any(char::is_control) {
                return Err("invalid_args");
            }
            if let Some(id) = a.get("conversation_id") {
                if c.tool == Tool::ResidentSearch {
                    return Err("invalid_args");
                }
                policy(app, conversation(app, id.as_str().ok_or("invalid_args")?)?, false)?;
            }
            if c.tool == Tool::ResidentSearch && a.get("snapshot_id").is_none() && !app.world.main_has_cap("AvatarPickerSearch") {
                return Err("native_capability_unavailable");
            }
        }
        Tool::ResidentProfile | Tool::ChatOpenIm => {
            args(c, &["resident_id", "grid"])?;
            let id = resident(app, c)?;
            if c.tool == Tool::ChatOpenIm
                && (!eligible(app, id) || app.world.social.ims.len() >= 2048 && !app.world.social.has_session(&id))
            {
                return Err("native_chat_restriction");
            }
            if c.tool == Tool::ResidentProfile && !app.world.main_has_cap("AgentProfile") && app.world.profiles.get(&id).is_none() {
                return Err("native_profile_unavailable");
            }
        }
        Tool::ChatDraftSet => {
            args(c, &["conversation_id", "expected_revision", "text"])?;
            let target = target(app, c)?;
            policy(app, target, false)?;
            text(c, "text", true)?;
            let revision = string(a, "expected_revision", 20)?.parse::<u64>().map_err(|_| "invalid_args")?;
            if state.social.drafts.get(&(c.client, key(target))).map_or(0, |d| d.0) != revision {
                return Err("draft_revision_conflict");
            }
        }
        Tool::ChatSend => {
            args(c, &["conversation_id", "topic", "text", "client_request_id"])?;
            let target = target(app, c)?;
            policy(app, target, true)?;
            if string(a, "topic", 16)? != topic(target) {
                return Err("native_topic_mismatch");
            }
            text(c, "text", false)?;
            let client = string(a, "client_request_id", 128)?;
            if client.is_empty() {
                return Err("invalid_args");
            }
            extra.push(match target {
                Conversation::Nearby => Class::ChatSendNearby,
                Conversation::Direct(_) => Class::ChatSendIM,
                Conversation::Group(_) => Class::ChatSendGroup,
                Conversation::Conference(_) => Class::ChatSendConference,
            });
        }
        Tool::EventsPoll => {
            args(c, &["topics", "start", "cursor", "limit", "wait_ms"])?;
            limit(a, 50, 100)?;
            if count(a, "wait_ms", 0, 0)? != 0 {
                return Err("invalid_args");
            }
            for t in topics(c)? {
                let class = match t.as_str() {
                    "contacts" => Class::FriendsRead,
                    "notifications" => Class::NotificationsRead,
                    _ => Class::ChatRead,
                };
                if !extra.contains(&class) {
                    extra.push(class);
                }
            }
            match (a.get("cursor"), a.get("start")) {
                (Some(v), None) => {
                    Uuid::parse_str(v.as_str().ok_or("invalid_args")?).map_err(|_| "invalid_args")?;
                }
                (None, Some(v)) if matches!(v.as_str(), Some("now" | "retained")) => {}
                _ => return Err("invalid_args"),
            }
        }
        Tool::NotificationsGet | Tool::NotificationReview => {
            args(c, &["notification_id"])?;
            notification(app, state, c)?;
            if c.tool == Tool::NotificationReview {
                extra.push(Class::NotificationsRead);
            }
        }
        _ => return Err("unsupported_capability"),
    }
    Ok(extra)
}
pub(super) fn evidence(app: &App, state: &super::State, c: &Capture) -> Result<Value, &'static str> {
    Ok(match c.tool {
        Tool::ChatSend | Tool::ChatMarkRead | Tool::ChatClose | Tool::ChatDraftSet => {
            let conv = target(app, c)?;
            let (_, clock) = native_lines(app, conv)?;
            let stamp = super::super::super::host_chat::snapshot(&app.world, conv, true)
                .ok_or("native_chat_restriction")?
                .stamp;
            json!({"conversation":key(conv),"topic":topic(conv),"token":clock.token,"read_revision":if c.tool==Tool::ChatMarkRead{Some(clock.revision)}else{None},"policy":stamp,"draft_revision":state.social.drafts.get(&(c.client,key(conv))).map_or(0,|v|v.0),"grid":grid(app)})
        }
        Tool::ChatOpenIm | Tool::ResidentProfile => {
            let id = resident(app, c)?;
            json!({"resident":id.to_string(),"grid":grid(app),"can_open":eligible(app,id),"existing":app.world.social.has_session(&id)})
        }
        Tool::NotificationReview => {
            let n = notification(app, state, c)?;
            json!({"id":n.id,"fingerprint":fingerprint(&format!("{:?}|{}|{}",n.data,n.title,n.body))})
        }
        _ => Value::Null,
    })
}
fn fingerprint(text: &str) -> String {
    hash(text.as_bytes())
}
fn line(app: &App, conv: Conversation, n: &crate::world::ChatLine) -> Option<Value> {
    if n.origin != Origin::Live || matches!(n.kind, ChatKind::Invite(_) | ChatKind::InventoryOffer(_) | ChatKind::Payment(_)) {
        return None;
    }
    let own = matches!(n.kind, ChatKind::Own(_));
    let system = matches!(n.kind, ChatKind::System);
    if !own
        && !system
        && (app.world.mutes.text_muted(&n.source, &n.from) || (!n.source.is_nil() && !app.world.rlva_allows_im("recvim", n.source)))
    {
        return None;
    }
    let clipped = clip(&n.text, 2048);
    Some(
        json!({"conversation_id":key(conv),"topic":topic(conv),"sender":clip(&n.from,255),"sender_id":if n.source.is_nil(){None}else{Some(n.source.to_string())},"text":clipped,"truncated":n.text.len()>2048,"own":own,"system":system,"time":crate::world::notification_history::secs(n.time),"untrusted_data":true}),
    )
}
pub(super) fn baseline(app: &App, state: &mut super::State) {
    for conv in all(app) {
        if let Ok((lines, clock)) = native_lines(app, conv) {
            state
                .social
                .scans
                .insert(key(conv), (clock.token, clock.first + lines.len() as u64));
        }
    }
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    if private_read(app).is_err() {
        return;
    }
    if app.settings.local_assistant.permissions.level(Class::ChatRead) != Level::Never {
        let mut budget = 256;
        for conv in all(app) {
            if budget == 0 {
                break;
            }
            if policy(app, conv, false).is_err() {
                continue;
            }
            let Ok((lines, clock)) = native_lines(app, conv) else { continue };
            let id = key(conv);
            let end = clock.first + lines.len() as u64;
            let old = assistant.world.social.scans.get(&id).copied().unwrap_or((clock.token, clock.first));
            let start = if old.0 == clock.token {
                old.1.max(clock.first).min(end)
            } else {
                end
            };
            let take = ((end - start) as usize).min(budget);
            let captured: Vec<_> = lines
                .iter()
                .skip((start - clock.first) as usize)
                .take(take)
                .filter_map(|n| line(app, conv, n))
                .collect();
            assistant.world.social.scans.insert(id.clone(), (clock.token, start + take as u64));
            budget -= take;
            for value in captured {
                let retained = assistant.world.social.lines.entry(id.clone()).or_default();
                retained.push_back(value.clone());
                while retained.len() > 128 {
                    retained.pop_front();
                }
                if value["own"] == false && value["system"] == false {
                    assistant.world.social.hidden.retain(|(_, conv), _| conv != &id);
                }
                push_event(&mut assistant.world, "chat", value);
            }
        }
    }
    if app.settings.local_assistant.permissions.level(Class::FriendsRead) != Level::Never {
        let rows = friends(app);
        let fp = fingerprint(&rows.to_string());
        if fp != assistant.world.friends_fingerprint {
            assistant.world.friends_fingerprint = fp;
            push_event(&mut assistant.world, "contacts", json!({"kind":"friends_changed","reconcile":true}));
        }
    }
    if app.settings.local_assistant.permissions.level(Class::NotificationsRead) != Level::Never {
        let native: Vec<_> = app
            .world
            .notifications
            .listed()
            .map(|n| (n.id, fingerprint(&format!("{:?}|{}|{}", n.data, n.title, n.body))))
            .collect();
        let removed: Vec<_> = assistant
            .world
            .notifications
            .keys()
            .filter(|id| !native.iter().any(|(n, _)| n == *id))
            .copied()
            .collect();
        for id in removed {
            if let Some(uuid) = assistant.world.notifications.remove(&id) {
                assistant.world.notification_fingerprint.remove(&id);
                push_event(
                    &mut assistant.world,
                    "notifications",
                    json!({"kind":"removed","notification_id":uuid.to_string()}),
                );
            }
        }
        for (id, fp) in native {
            let uuid = *assistant.world.notifications.entry(id).or_insert_with(Uuid::new_v4);
            if assistant.world.notification_fingerprint.get(&id) != Some(&fp) {
                assistant.world.notification_fingerprint.insert(id, fp);
                push_event(
                    &mut assistant.world,
                    "notifications",
                    json!({"kind":"changed","notification_id":uuid.to_string(),"reconcile":true}),
                );
            }
        }
    }
    if let Some(search) = assistant
        .world
        .social
        .search
        .as_ref()
        .filter(|s| s.started.elapsed() > Duration::from_secs(22))
        .cloned()
    {
        assistant.world.social.search = None;
        let _ = assistant.runtime.complete(&search.capture, failure("resident_search_timeout"));
    }
    let replies: Vec<_> = assistant
        .world
        .social
        .profile_io
        .as_ref()
        .map(|(_, rx)| rx.try_iter().take(8).collect())
        .unwrap_or_default();
    for reply in replies {
        assistant.world.social.profiles.remove(&reply.capture.request_id);
        if current(app, assistant, &reply.capture) && private_read(app).is_ok() && app.world.rlva_can_name(reply.resident) {
            let result = reply.result.map(|p| profile(app, &p)).unwrap_or_else(failure);
            let _ = assistant.runtime.complete(&reply.capture, result);
        }
    }
}
fn friends(app: &App) -> Value {
    let mut rows:Vec<_>=app.world.social.friends.iter().take(2048).filter(|f|f.id!=app.world.agent_id).map(|f|{let n=app.world.social.avatar_names.raw(&f.id);json!({"resident_id":f.id.to_string(),"display_name":clip(n.map_or("",|n|n.display_name.as_str()),255),"username":clip(n.map_or("",|n|n.username.as_str()),127),"online":f.online,"grid":grid(app),"can_open":eligible(app,f.id)})}).collect();
    rows.sort_by(|a, b| {
        b["online"]
            .as_bool()
            .cmp(&a["online"].as_bool())
            .then_with(|| a["display_name"].as_str().cmp(&b["display_name"].as_str()))
            .then_with(|| a["resident_id"].as_str().cmp(&b["resident_id"].as_str()))
    });
    json!(rows)
}
fn profile(app: &App, p: &aurora_net::profile::AvatarProfile) -> Value {
    json!({"resident_id":p.id.to_string(),"grid":grid(app),"about":clip(&p.sl_about,8192),"first_life_about":clip(&p.fl_about,4096),"image_id":p.sl_image.to_string(),"partner_id":if p.partner.is_nil(){None}else{Some(p.partner.to_string())},"online":p.online,"born":if p.hide_age==Some(true){None}else{p.born},"groups":p.groups.iter().take(32).map(|g|json!({"id":g.id.to_string(),"name":clip(&g.name,128)})).collect::<Vec<_>>(),"legacy":p.legacy,"untrusted_data":true,"private_notes_included":false})
}
fn notification_row(state: &super::State, n: &crate::world::notifications::Notification) -> Option<Value> {
    let id = state.notifications.get(&n.id)?;
    Some(
        json!({"notification_id":id.to_string(),"kind":format!("{:?}",n.kind).to_ascii_lowercase(),"title":clip(&n.title,255),"body":clip(&n.body,2048),"interactive":n.interactive(),"read":n.read,"time":crate::world::notification_history::secs(n.time),"untrusted_data":true}),
    )
}

pub(super) fn execute(app: &mut App, assistant: &mut Assistant, c: &Capture) -> Result<Value, &'static str> {
    let a = &c.arguments;
    match c.tool {
        Tool::ChatConversations => {
            let rows=all(app).into_iter().filter(|v|policy(app,*v,false).is_ok()&&!assistant.world.social.hidden.contains_key(&(c.client,key(*v)))).filter_map(|v|{let p=super::super::super::host_chat::snapshot(&app.world,v,true)?;let unread=app.world.read_counters(native_id(v)).map_or(0,|(_,n,_)|n);Some(json!({"conversation_id":key(v),"topic":topic(v),"title":p.title.as_str(),"can_send":p.can_send,"unread":unread,"retained_messages":assistant.world.social.lines.get(&key(v)).map_or(0,VecDeque::len)}))}).collect();
            snapshot_page(
                &mut assistant.world,
                c,
                "conversations",
                "",
                rows,
                app.world.social.ims.len() >= 2048,
            )
        }
        Tool::FriendsList => {
            let rows = friends(app).as_array().cloned().unwrap_or_default();
            snapshot_page(&mut assistant.world, c, "friends", "", rows, app.world.social.friends.len() > 2048)
        }
        Tool::ChatHistory | Tool::ChatSearch => {
            let filter = if c.tool == Tool::ChatHistory {
                Some(target(app, c)?)
            } else {
                a.get("conversation_id")
                    .map(|v| conversation(app, v.as_str().ok_or("invalid_args")?))
                    .transpose()?
            };
            let query = if c.tool == Tool::ChatSearch {
                string(a, "query", 127)?.to_lowercase()
            } else {
                String::new()
            };
            let mut rows = Vec::new();
            for (id, lines) in &assistant.world.social.lines {
                let Ok(conv) = conversation(app, id) else { continue };
                if filter.is_some_and(|f| f != conv) || policy(app, conv, false).is_err() {
                    continue;
                }
                for line in lines {
                    if query.is_empty() || line["text"].as_str().unwrap_or("").to_lowercase().contains(&query) {
                        rows.push(line.clone());
                        if rows.len() >= 2048 {
                            break;
                        }
                    }
                }
                if rows.len() >= 2048 {
                    break;
                }
            }
            let mut out = snapshot_page(
                &mut assistant.world,
                c,
                if c.tool == Tool::ChatHistory { "history" } else { "chat_search" },
                &format!("{}|{}", filter.map(key).unwrap_or_default(), query),
                rows,
                false,
            )?;
            out["retention"] =
                json!({"source":"observed_live_login","per_conversation":RETENTION,"disk_history":false,"may_have_gaps":true});
            Ok(out)
        }
        Tool::ChatDraftGet => {
            let conv = target(app, c)?;
            let (revision, text) = assistant
                .world
                .social
                .drafts
                .get(&(c.client, key(conv)))
                .cloned()
                .unwrap_or_default();
            Ok(json!({"conversation_id":key(conv),"revision":revision.to_string(),"text":text,"assistant_only":true}))
        }
        Tool::ChatDraftSet => {
            let conv = target(app, c)?;
            let id = key(conv);
            let expected = string(a, "expected_revision", 20)?.parse::<u64>().map_err(|_| "invalid_args")?;
            let text = text(c, "text", true)?.to_owned();
            let draft = assistant.world.social.drafts.entry((c.client, id.clone())).or_default();
            if draft.0 != expected {
                return Err("draft_revision_conflict");
            }
            draft.0 = draft.0.checked_add(1).ok_or("revision_exhausted")?;
            draft.1 = text;
            Ok(json!({"conversation_id":id,"revision":draft.0.to_string(),"assistant_only":true,"sent":false}))
        }
        Tool::ChatOpenIm => {
            let id = resident(app, c)?;
            if !eligible(app, id) {
                return Err("native_chat_restriction");
            }
            app.world.social.session_mut(id);
            assistant.world.social.hidden.remove(&(c.client, id.to_string()));
            Ok(json!({"conversation_id":id.to_string(),"topic":"im","grid":grid(app),"sent":false,"focused":false}))
        }
        Tool::ChatMarkRead => {
            let conv = target(app, c)?;
            let (unread, _) = app.world.mark_conversation_read(native_id(conv)).ok_or("conversation_not_found")?;
            app.world.acknowledge_read_messages();
            Ok(json!({"conversation_id":key(conv),"marked_read":true,"previous_unread":unread}))
        }
        Tool::ChatClose => {
            let conv = target(app, c)?;
            let (_, clock) = native_lines(app, conv)?;
            assistant.world.social.hidden.insert((c.client, key(conv)), clock.revision);
            Ok(json!({"conversation_id":key(conv),"hidden":true,"native_closed":false,"left_session":false}))
        }
        Tool::ChatSend => {
            let conv = target(app, c)?;
            let client = string(a, "client_request_id", 128)?.to_owned();
            if assistant.world.social.clients.contains(&(c.client, client.clone())) {
                return Err("client_request_already_used");
            }
            if assistant.world.social.clients.len() >= 4096 {
                return Err("send_intent_capacity");
            }
            let now = Instant::now();
            while assistant
                .world
                .social
                .sends
                .front()
                .is_some_and(|t| now.saturating_duration_since(*t) >= Duration::from_secs(60))
            {
                assistant.world.social.sends.pop_front();
            }
            if assistant.world.social.sends.len() >= 6
                || assistant
                    .world
                    .social
                    .sends
                    .back()
                    .is_some_and(|t| now.saturating_duration_since(*t) < Duration::from_secs(1))
            {
                return Err("chat_rate_limit");
            }
            let text = text(c, "text", false)?.to_owned();
            policy(app, conv, true)?;
            let command = match conv {
                Conversation::Nearby => NetCommand::Chat {
                    message: text.clone(),
                    channel: 0,
                    chat_type: aurora_net::ChatType::Normal,
                },
                Conversation::Direct(_) => NetCommand::SendIm {
                    to: native_id(conv).ok_or("invalid_args")?,
                    message: text.clone(),
                },
                _ => {
                    let id = native_id(conv).ok_or("invalid_args")?;
                    NetCommand::SendImDialog {
                        to: id,
                        dialog: crate::world::groups::dialog::SESSION_SEND,
                        id,
                        message: text.clone(),
                        bucket: Vec::new(),
                    }
                }
            };
            assistant.world.social.clients.insert((c.client, client.clone()));
            assistant.world.social.sends.push_back(now);
            app.send(command);
            match conv {
                Conversation::Direct(_) => app.world.own_im(native_id(conv).unwrap_or_default(), &text),
                Conversation::Group(_) => app.world.own_shared_group_im(native_id(conv).unwrap_or_default(), &text),
                Conversation::Conference(_) => app.world.own_shared_conference_im(native_id(conv).unwrap_or_default(), &text),
                Conversation::Nearby => {}
            }
            let value = json!({"status":"uncertain","operation_id":c.request_id,"client_request_id":client,"conversation_id":key(conv),"submitted":true,"delivery_confirmed":false,"retry_safe":false});
            push_operation(&mut assistant.world, c, value.clone());
            Ok(value)
        }
        Tool::ResidentSearch => {
            let query = string(a, "query", 127)?.to_owned();
            if a.get("snapshot_id").is_some() {
                return snapshot_page(&mut assistant.world, c, "resident_search", &query, vec![], false);
            }
            if assistant.world.social.search.is_some() {
                return Err("resident_search_busy");
            }
            let source = aurora_net::LoginSource::from_login(app.world.login.as_ref().ok_or("native_not_ready")?);
            let request = Uuid::new_v4();
            assistant.world.social.search = Some(Search {
                capture: c.clone(),
                request,
                query: query.clone(),
                source: source.clone(),
                started: Instant::now(),
            });
            app.send(NetCommand::ContactSearch { source, request, query });
            Ok(submitted(c))
        }
        Tool::ResidentProfile => {
            let id = resident(app, c)?;
            let Some(cap) = app.world.main().and_then(|r| r.caps.get("AgentProfile")).cloned() else {
                return app
                    .world
                    .profiles
                    .get(&id)
                    .map(|p| profile(app, p))
                    .ok_or("native_profile_unavailable");
            };
            if assistant.world.social.profiles.len() >= 8 {
                return Err("profile_capacity");
            }
            let (tx, _) = assistant
                .world
                .social
                .profile_io
                .get_or_insert_with(|| crossbeam_channel::bounded(8));
            let tx = tx.clone();
            let capture = c.clone();
            let http = app.net.caps_http();
            assistant
                .world
                .social
                .profiles
                .insert(c.request_id.clone(), (c.clone(), Instant::now(), id));
            app.net.runtime().spawn(async move {
                let result = async {
                    let mut response = http
                        .get(format!("{}/{id}", cap.trim_end_matches('/')))
                        .header("Accept", "application/llsd+xml")
                        .timeout(Duration::from_secs(20))
                        .send()
                        .await
                        .map_err(|_| "profile_transport")?;
                    if !response.status().is_success() {
                        return Err("profile_unavailable");
                    }
                    let mut bytes = Vec::new();
                    while let Some(chunk) = response.chunk().await.map_err(|_| "profile_transport")? {
                        if bytes.len() + chunk.len() > 256 * 1024 {
                            return Err("profile_limit");
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    let llsd = aurora_llsd::from_xml(&bytes).map_err(|_| "profile_invalid")?;
                    aurora_net::profile::parse_agent_profile(id, &llsd).ok_or("profile_invalid")
                }
                .await;
                let _ = tx.try_send(ProfileReply {
                    capture,
                    resident: id,
                    result,
                });
            });
            Ok(submitted(c))
        }
        Tool::NotificationsList => {
            let active: Vec<_> = app
                .world
                .notifications
                .listed()
                .filter_map(|n| notification_row(&assistant.world, n))
                .collect();
            let ids: Vec<_> = active
                .iter()
                .filter_map(|n| n["notification_id"].as_str().map(str::to_owned))
                .collect();
            let mut out = snapshot_page(&mut assistant.world, c, "notifications", "", active, false)?;
            out["active_ids"] = json!(ids);
            Ok(out)
        }
        Tool::NotificationsGet => {
            let n = notification(app, &assistant.world, c)?;
            notification_row(&assistant.world, n).ok_or("notification_not_found")
        }
        Tool::NotificationReview => {
            let id = notification(app, &assistant.world, c)?.id;
            app.review_input.begin();
            app.alt_codes.reset();
            egui::Popup::close_all(&app.egui_ctx);
            app.egui_ctx.stop_dragging();
            app.notif_ui.open = true;
            app.notif_ui.center.tab = crate::ui::notification_center::Tab::ToDo;
            app.notif_ui.center.query.clear();
            app.notif_ui.review = Some(id);
            if let Some(gfx) = &app.gfx {
                gfx.window.request_redraw();
            }
            Ok(json!({"notification_id":string(a,"notification_id",36)?,"opened_locally":true,"responded":false}))
        }
        Tool::EventsPoll => poll(&mut assistant.world, c),
        _ => Err("unsupported_capability"),
    }
}
fn poll(state: &mut super::State, c: &Capture) -> Result<Value, &'static str> {
    let topics = topics(c)?;
    let owner = (c.endpoint, c.client);
    let private_latest = state.private_events.get(&owner).map_or(0, |l| l.serial);
    let (private_first, public_first) = (
        state
            .private_events
            .get(&owner)
            .and_then(|l| l.events.front())
            .map_or(private_latest.saturating_add(1), |e| e.serial),
        state.events.front().map_or(state.serial.saturating_add(1), |e| e.serial),
    );
    let (public_after, private_after) = if let Some(id) = c.arguments.get("cursor") {
        let id = Uuid::parse_str(id.as_str().ok_or("invalid_args")?).map_err(|_| "invalid_args")?;
        let old = state.social.cursors.get(&id).ok_or("stale_cursor")?;
        if old.client != c.client || old.endpoint != c.endpoint || old.epoch != state.epoch || old.topics != topics {
            return Err("stale_cursor");
        }
        (old.public, old.private)
    } else if c.arguments.get("start").and_then(Value::as_str) == Some("now") {
        (state.serial, private_latest)
    } else {
        (public_first.saturating_sub(1), private_first.saturating_sub(1))
    };
    let wants_private = topics.iter().any(|t| t == "operations");
    let wants_public = topics.iter().any(|t| t != "operations");
    let gap =
        wants_public && public_after.saturating_add(1) < public_first || wants_private && private_after.saturating_add(1) < private_first;
    let maximum = limit(&c.arguments, 50, 100)?;
    let (mut items, mut bytes, mut public_next, mut private_next) = (Vec::new(), 0, public_after, private_after);
    let public = state.events.iter().filter(|e| e.serial > public_after).map(|e| (false, e));
    let private = state
        .private_events
        .get(&owner)
        .into_iter()
        .flat_map(|l| l.events.iter())
        .filter(|e| e.serial > private_after)
        .map(|e| (true, e));
    for (private, event) in public.chain(private) {
        if topics.iter().any(|t| t == event.topic) {
            let item = json!({"serial":format!("{}:{}",if private{"operations"}else{"native"},event.serial),"topic":event.topic,"data":event.value});
            let size = serde_json::to_vec(&item).map_err(|_| "response_limit")?.len();
            if items.len() >= maximum || bytes + size > 18 * 1024 {
                break;
            }
            bytes += size;
            items.push(item);
        }
        if private {
            private_next = event.serial;
        } else {
            public_next = event.serial;
        }
    }
    if state.social.cursors.len() >= 256 {
        state.social.cursors.clear();
    }
    let next = Uuid::new_v4();
    state.social.cursors.insert(
        next,
        Cursor {
            client: c.client,
            endpoint: c.endpoint,
            epoch: state.epoch,
            topics,
            public: public_next,
            private: private_next,
        },
    );
    Ok(
        json!({"events":items,"next_cursor":next.to_string(),"latest_cursor":{"native":if wants_public{Some(state.serial.to_string())}else{None},"operations":if wants_private{Some(private_latest.to_string())}else{None}},"gap":gap,"reconcile_required":gap,"untrusted_data":true}),
    )
}
pub(super) fn native_event(app: &mut App, event: &NetEvent) -> bool {
    let NetEvent::ContactSearch { source, request, result } = event else {
        return false;
    };
    let pending = app
        .character_mcp
        .as_ref()
        .and_then(|a| a.world.social.search.as_ref())
        .filter(|p| p.request == *request)
        .cloned();
    let Some(pending) = pending else { return false };
    let valid = super::super::continuation_current(app, &pending.capture)
        && app
            .world
            .login
            .as_ref()
            .is_some_and(|l| source.matches_login(l, app.world.agent_id) && pending.source.matches_login(l, app.world.agent_id))
        && private_read(app).is_ok();
    let value = if valid {
        match result {
            Ok(results) => {
                let rows=results.residents.iter().take(100).map(|r|json!({"resident_id":r.id.to_string(),"display_name":clip(&r.display,255),"username":clip(&r.username,127),"grid":grid(app),"can_open":eligible(app,r.id)})).collect();
                if let Some(mut assistant) = app.character_mcp.take() {
                    let value = snapshot_page(
                        &mut assistant.world,
                        &pending.capture,
                        "resident_search",
                        &pending.query,
                        rows,
                        results.limited,
                    )
                    .unwrap_or_else(failure);
                    assistant.world.social.search = None;
                    let _ = assistant.runtime.complete(&pending.capture, value);
                    app.character_mcp = Some(assistant);
                }
                return true;
            }
            Err(_) => failure("resident_search_failed"),
        }
    } else {
        failure("stale_source")
    };
    if let Some(assistant) = app.character_mcp.as_mut() {
        assistant.world.social.search = None;
        let _ = assistant.runtime.complete(&pending.capture, value);
    }
    true
}

#[cfg(test)]
mod event_privacy_tests {
    use super::*;
    #[test]
    fn operation_events_have_independent_authenticated_owner_lanes_and_no_foreign_gap() {
        let mut state = super::super::State::default();
        let mut a = super::super::tests::capture(Tool::EventsPoll);
        a.arguments = serde_json::from_value(json!({"topics":["operations"],"start":"retained"})).unwrap();
        let mut b = a.clone();
        b.client += 1;
        let empty = poll(&mut state, &b).unwrap();
        assert_eq!(empty["latest_cursor"]["operations"], "0");
        for i in 0..600 {
            push_operation(
                &mut state,
                &a,
                json!({"operation_id":format!("private-{i}"),"client_request_id":"secret"}),
            );
        }
        b.arguments.remove("start");
        b.arguments.insert("cursor".into(), empty["next_cursor"].clone());
        let quiet = poll(&mut state, &b).unwrap();
        assert_eq!(quiet["events"], json!([]));
        assert_eq!(quiet["gap"], false);
        assert_eq!(quiet["latest_cursor"]["operations"], "0");
        let own = poll(&mut state, &a).unwrap();
        assert!(!own["events"].as_array().unwrap().is_empty());
        let mut other_endpoint = a.clone();
        other_endpoint.endpoint = HostId::from_bytes([77; 16]).unwrap();
        assert_eq!(poll(&mut state, &other_endpoint).unwrap()["events"], json!([]));
        other_endpoint.arguments.remove("start");
        other_endpoint.arguments.insert("cursor".into(), own["next_cursor"].clone());
        assert_eq!(poll(&mut state, &other_endpoint), Err("stale_cursor"));
    }
}
