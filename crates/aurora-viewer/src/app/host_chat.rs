//! Literal, account-bound chat through this worker's native world. No keyboard
//! events, chatbar/gesture commands, offer replies, group joins or unblocking.

use super::{App, Screen};
use crate::session_host::chat::{NativeChat, NativeCursor, NativeLine, NativePage, NativeSnapshot, SCAN_LINES, WorkerChat};
use crate::session_host::worker::WorkerBridge;
use crate::world::{ChatKind, Origin, World};
use aurora_net::{ChatType, NetCommand};
use aurora_session::chat::{
    ChatError, ChatKey, ChatPacket, ChatSource, ChatText, Conversation, LineKind, MAX_CONVERSATIONS, MAX_TEXT_BYTES, SendOutcome,
};
use aurora_session::{AccountKey, ConversationId, Label};
use std::time::Instant;
use uuid::Uuid;

impl App {
    pub(super) fn host_chat_sync(&mut self, backend: &mut WorkerChat, bridge: &WorkerBridge) -> Result<(), ChatError> {
        self.configure_host_chat_rules();
        self.reconcile_host_read_interest();
        backend.sync(&AppChat { app: self, bridge })
    }

    pub(super) fn host_chat_packet(
        &mut self,
        backend: &mut WorkerChat,
        bridge: &WorkerBridge,
        packet: &ChatPacket,
        received_at: Instant,
    ) -> Result<Vec<ChatPacket>, ChatError> {
        self.configure_host_chat_rules();
        self.reconcile_host_read_interest();
        backend.handle_received(&mut AppChat { app: self, bridge }, packet, received_at, Instant::now())
    }

    fn reconcile_host_read_interest(&mut self) {
        reconcile_read_interest(&mut self.world);
    }

    pub(super) fn configure_host_chat_rules(&mut self) {
        // The same runtime layer as send_from; never save Preferences here.
        self.world.rlva_configure(self.settings.rlva_enabled);
        self.world.rlva.preferences.clone_from(&self.settings.rlva);
    }
}

pub(super) fn source(app: &App, bridge: &WorkerBridge) -> Option<ChatSource> {
    let source = bridge.bound_session().and_then(|binding| ChatSource::try_from(binding).ok())?;
    if !matches!(app.screen, Screen::World) || app.world.agent_id.is_nil() {
        return None;
    }
    let uri = app.settings.login_uri();
    let account = AccountKey::parse_hex(&crate::characters::identity(&uri, &app.settings.username)).ok()?;
    let grid = crate::session_host::transport::grid_key(&uri).ok()?;
    (source.session.account == account && source.session.grid == grid).then_some(source)
}

struct AppChat<'a> {
    app: &'a mut App,
    bridge: &'a WorkerBridge,
}

impl NativeChat for AppChat<'_> {
    fn source(&self) -> Option<ChatSource> {
        source(self.app, self.bridge)
    }
    fn catalogue(&self) -> Vec<Conversation> {
        catalogue(&self.app.world)
    }
    fn snapshot(&self, conversation: Conversation) -> Option<NativeSnapshot> {
        self.source()?;
        snapshot(&self.app.world, conversation, !self.app.exit_prompt.open)
    }
    fn lines(&self, conversation: Conversation, cursor: NativeCursor, limit: usize) -> NativePage {
        if self.source().is_none() {
            return NativePage {
                cursor,
                gap: true,
                lines: Vec::new(),
            };
        }
        lines(&self.app.world, conversation, cursor, limit)
    }
    fn read_metadata(&self, conversation: Conversation) -> Option<aurora_session::chat::ReadMetadata> {
        self.source()?;
        read_metadata(&self.app.world, conversation, !self.app.exit_prompt.open)
    }
    fn incoming(&self, conversation: Conversation) -> Option<aurora_session::chat::NativeIncoming> {
        self.source()?;
        incoming(&self.app.world, conversation, !self.app.exit_prompt.open)
    }
    fn incoming_stream(&self) -> Option<u64> {
        self.source()?;
        self.app.world.shared_chat_close_fence().map(|stamp| stamp.stream)
    }
    fn close_conversation(&mut self, request: aurora_session::chat::CloseRequest) -> aurora_session::chat::CloseOutcome {
        let bridge = self.bridge;
        close_native_conversation(&self.app.world, request, !self.app.exit_prompt.open, &|| {
            bridge.bound_session().and_then(|binding| ChatSource::try_from(binding).ok()) == Some(request.key.source)
        })
    }
    fn mark_read(&mut self, request: aurora_session::chat::MarkReadRequest) -> aurora_session::chat::ReadOutcome {
        if self.source() != Some(request.key.source) {
            return aurora_session::chat::ReadOutcome::RejectedNative;
        }
        let bridge = self.bridge;
        mark_native_read(&mut self.app.world, request, !self.app.exit_prompt.open, &|| {
            bridge.bound_session().and_then(|binding| ChatSource::try_from(binding).ok()) == Some(request.key.source)
        })
    }
    fn review_conversation(&mut self, request: aurora_session::chat::ReviewConversationRequest) -> aurora_session::chat::ReviewOutcome {
        self.app.review_existing_conversation(self.bridge, request)
    }
    fn submit_literal(&mut self, key: ChatKey, text: &str) -> SendOutcome {
        if self.source() != Some(key.source) {
            return SendOutcome::RejectedNative;
        }
        let Some(state) = snapshot(&self.app.world, key.conversation, !self.app.exit_prompt.open) else {
            return SendOutcome::RejectedNative;
        };
        if !state.ready || !state.can_send {
            return SendOutcome::RejectedNative;
        }
        let Some(command) = literal_command(&self.app.world, key.conversation, text) else {
            return SendOutcome::RejectedNative;
        };
        // Check the lease again immediately before the owning App's common
        // restriction boundary. No await or cross-thread world access occurs.
        if self.source() != Some(key.source) {
            return SendOutcome::RejectedNative;
        }
        self.app.send(command);
        match key.conversation {
            Conversation::Direct(peer) => self.app.world.own_im(uuid(peer), text),
            Conversation::Group(session) => self.app.world.own_shared_group_im(uuid(session), text),
            Conversation::Conference(session) => self.app.world.own_shared_conference_im(uuid(session), text),
            _ => {}
        }
        // App::send / NetClient::send have no enqueue result. The call is made,
        // but we cannot promise acceptance or remote delivery. Never retry it.
        SendOutcome::Uncertain
    }
}

fn reconcile_read_interest(world: &mut World) {
    let current: Vec<_> = catalogue(world)
        .into_iter()
        .filter_map(|conversation| {
            let key = native_read_key(conversation)?;
            let (token, raw, badge) = world.read_counters(key)?;
            Some((key, token, raw, badge))
        })
        .collect();
    world.read_ledger.reconcile(&current);
}

fn mark_native_read(
    world: &mut World,
    request: aurora_session::chat::MarkReadRequest,
    ui_ready: bool,
    source_current: &dyn Fn() -> bool,
) -> aurora_session::chat::ReadOutcome {
    use aurora_session::chat::ReadOutcome;
    reconcile_read_interest(world);
    let Some(current) = read_metadata(world, request.key.conversation, ui_ready) else {
        return ReadOutcome::RejectedNative;
    };
    if !current.can_mark_read {
        return ReadOutcome::RejectedNative;
    }
    if current.observed != request.observed || current.unread_revision != request.unread_revision {
        return ReadOutcome::StaleObservation;
    }
    let Some(key) = native_read_key(request.key.conversation) else {
        return ReadOutcome::RejectedNative;
    };
    // The native owner does not yield between this exact lease check and reset.
    if !source_current() {
        return ReadOutcome::RejectedNative;
    }
    let Some((count, _)) = world.mark_conversation_read(key) else {
        return ReadOutcome::RejectedNative;
    };
    world.acknowledge_read_messages();
    if count == 0 {
        ReadOutcome::AlreadyRead
    } else {
        ReadOutcome::Applied
    }
}

pub(super) fn native_read_key(conversation: Conversation) -> Option<Option<Uuid>> {
    match conversation {
        Conversation::Nearby => Some(None),
        Conversation::Direct(peer) | Conversation::Group(peer) | Conversation::Conference(peer) => Some(Some(uuid(peer))),
    }
}

pub(super) fn read_metadata(world: &World, conversation: Conversation, ui_ready: bool) -> Option<aurora_session::chat::ReadMetadata> {
    let policy = snapshot(world, conversation, ui_ready)?;
    if !policy.ready || !policy.can_read {
        return None;
    }
    let state = world.read_ledger.state(native_read_key(conversation)?)?;
    Some(aurora_session::chat::ReadMetadata {
        unread: state.unread,
        unread_revision: state.revision,
        observed: aurora_session::chat::ObservedIncoming {
            epoch: state.epoch,
            through: state.incoming,
        },
        can_mark_read: state.revision < u64::MAX,
    })
}

fn uuid(id: ConversationId) -> Uuid {
    Uuid::from_bytes(*id.as_bytes())
}

fn incoming(world: &World, conversation: Conversation, ui_ready: bool) -> Option<aurora_session::chat::NativeIncoming> {
    let policy = snapshot(world, conversation, ui_ready)?;
    if !policy.ready || !policy.can_read {
        return None;
    }
    let (Conversation::Direct(peer) | Conversation::Group(peer) | Conversation::Conference(peer)) = conversation else {
        return None;
    };
    world.shared_chat_activity(uuid(peer))
}

/// Native CloseChat is only an ordered fence (Folderstorm abbe3cb0,
/// FSSessionWorker::chatCommand). Never close/leave a native session or Read.
fn close_native_conversation(
    world: &World,
    request: aurora_session::chat::CloseRequest,
    ui_ready: bool,
    source_current: &dyn Fn() -> bool,
) -> aurora_session::chat::CloseOutcome {
    use aurora_session::chat::CloseOutcome;
    let readable = snapshot(world, request.key.conversation, ui_ready).is_some_and(|policy| policy.ready && policy.can_read);
    if request.validate().is_err() || !source_current() || !readable {
        return CloseOutcome::RejectedNative;
    }
    let Some(through) = world.shared_chat_close_fence() else {
        return CloseOutcome::RejectedNative;
    };
    if through.stream != request.stream || !source_current() {
        return CloseOutcome::StaleTarget;
    }
    CloseOutcome::Closed(through)
}
fn conversation_id(id: Uuid) -> Option<ConversationId> {
    ConversationId::from_bytes(*id.as_bytes()).ok()
}

fn catalogue(world: &World) -> Vec<Conversation> {
    let mut result = vec![Conversation::Nearby];
    // Bound the inspected native entries too: thousands of group tabs must
    // not turn a small IPC request into an unbounded main-thread search.
    result.extend(world.social.ims.iter().take(MAX_CONVERSATIONS - 1).filter_map(|session| {
        let id = conversation_id(session.other)?;
        if world.shared_group_initialized(session.other) {
            Some(Conversation::Group(id))
        } else if world.shared_conference_initialized(session.other) {
            Some(Conversation::Conference(id))
        } else if !world.is_group_session(&session.other) && !world.groups.is_member(&session.other) {
            Some(Conversation::Direct(id))
        } else {
            None
        }
    }));
    result
}

fn native_lines(
    world: &World,
    conversation: Conversation,
) -> Option<(
    &std::collections::VecDeque<crate::world::ChatLine>,
    crate::world::transcripts::LineClock,
)> {
    match conversation {
        Conversation::Nearby => Some((&world.chat, world.chat_clock)),
        Conversation::Direct(peer) if !world.is_group_session(&uuid(peer)) && !world.groups.is_member(&uuid(peer)) => world
            .social
            .ims
            .iter()
            .take(MAX_CONVERSATIONS - 1)
            .find(|session| session.other == uuid(peer))
            .map(|session| (&session.lines, session.clock)),
        Conversation::Group(id) if world.shared_group_initialized(uuid(id)) => world
            .social
            .ims
            .iter()
            .take(MAX_CONVERSATIONS - 1)
            .find(|session| session.other == uuid(id))
            .map(|session| (&session.lines, session.clock)),
        Conversation::Conference(id) if world.shared_conference_initialized(uuid(id)) => world
            .social
            .ims
            .iter()
            .take(MAX_CONVERSATIONS - 1)
            .find(|session| session.other == uuid(id))
            .map(|session| (&session.lines, session.clock)),
        _ => None,
    }
}

pub(super) fn snapshot(world: &World, conversation: Conversation, ui_ready: bool) -> Option<NativeSnapshot> {
    let (entries, clock) = native_lines(world, conversation)?;
    let r = &world.rlva;
    // Native UI can mask these at draw time. Fail closed here rather than
    // exporting raw historical names/locations from an earlier privacy state.
    let privacy = !world.shared_chat_privacy_allowed();
    let ready = ui_ready && !world.agent_id.is_nil() && world.mutes.is_loaded();
    let (can_send, can_read, title) = match conversation {
        Conversation::Nearby => (
            world.main_region.is_some() && world.movement_complete && !r.has("sendchat"),
            !privacy,
            aurora_i18n::tr!("common-nearby-chat").to_owned(),
        ),
        Conversation::Direct(peer) => {
            let peer = uuid(peer);
            let muted = world.mutes.is_muted_id(&peer);
            let can_read = world.shared_chat_direct_readable(peer);
            let title = if can_read {
                world.social.name_of(&peer)
            } else {
                aurora_i18n::tr!("rlva-blocked").to_owned()
            };
            (
                !muted && world.rlva_can_start_im(peer) && world.rlva_allows_im("sendim", peer),
                can_read,
                title,
            )
        }
        Conversation::Group(session) => {
            let session = uuid(session);
            let can_read = ready && world.shared_group_readable(session);
            let title = if can_read {
                world.session_title(&session)
            } else {
                aurora_i18n::tr!("rlva-blocked").to_owned()
            };
            (world.shared_group_sendable(session), can_read, title)
        }
        Conversation::Conference(session) => {
            let session = uuid(session);
            let can_read = ready && world.shared_conference_readable(session);
            let title = if can_read {
                world.session_title(&session)
            } else {
                aurora_i18n::tr!("rlva-blocked").to_owned()
            };
            (world.shared_conference_sendable(session), can_read, title)
        }
    };
    let flags = u64::from(r.enabled())
        | (u64::from(r.preferences.allow_ooc) << 1)
        | (u64::from(r.preferences.show_ellipsis) << 2)
        | (u64::from(r.preferences.split_redirect_chat) << 3)
        | (u64::from(r.preferences.show_redirect_typing) << 4);
    let end = clock.first.saturating_add(entries.len() as u64);
    // Initial unrestricted snapshots contain at most the last eight live
    // entries. Once restrictions changed, never re-export earlier raw text.
    let next = if r.enabled() && r.revision > 0 {
        end
    } else {
        end.saturating_sub(SCAN_LINES as u64).max(clock.first)
    };
    let (incarnation, roster_revision) = if let Conversation::Conference(id) = conversation {
        world.shared_conference_stamp(uuid(id))?
    } else {
        (0, 0)
    };
    Some(NativeSnapshot {
        title: label(&title),
        stamp: [
            r.revision,
            world.mutes.version,
            flags,
            world.main_region.unwrap_or(0),
            incarnation,
            roster_revision,
        ],
        ready,
        can_send: ready && can_send,
        can_read: ready && can_read,
        initial: NativeCursor { token: clock.token, next },
    })
}

fn label(text: &str) -> Label {
    let mut bounded = String::new();
    for ch in text.chars().filter(|ch| !ch.is_control()) {
        if bounded.len() + ch.len_utf8() > 128 {
            break;
        }
        bounded.push(ch);
    }
    Label::new(&bounded).unwrap_or_default()
}

fn line_text(text: &str) -> (ChatText, bool) {
    let mut value = String::new();
    let mut truncated = false;
    for ch in text.chars() {
        let ch = if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') {
            '\u{fffd}'
        } else {
            ch
        };
        if value.len() + ch.len_utf8() > MAX_TEXT_BYTES {
            truncated = true;
            break;
        }
        value.push(ch);
    }
    (ChatText::new(&value).unwrap_or_default(), truncated)
}

fn lines(world: &World, conversation: Conversation, cursor: NativeCursor, limit: usize) -> NativePage {
    let Some((entries, clock)) = native_lines(world, conversation) else {
        return NativePage {
            cursor,
            gap: true,
            lines: Vec::new(),
        };
    };
    let end = clock.first.saturating_add(entries.len() as u64);
    let group = matches!(conversation, Conversation::Group(_) | Conversation::Conference(_));
    if group && !snapshot(world, conversation, true).is_some_and(|state| state.ready && state.can_read) {
        return NativePage {
            cursor: NativeCursor {
                token: clock.token,
                next: end,
            },
            gap: true,
            lines: Vec::new(),
        };
    }
    if cursor.token != clock.token {
        return NativePage {
            cursor: NativeCursor {
                token: clock.token,
                next: end,
            },
            gap: true,
            lines: Vec::new(),
        };
    }
    let start = cursor.next.max(clock.first).min(end);
    let count = (end - start).min(limit.min(SCAN_LINES) as u64);
    let mut result = Vec::new();
    for native in entries.iter().skip((start - clock.first) as usize).take(count as usize) {
        if native.origin != Origin::Live {
            continue;
        }
        let kind = match native.kind {
            ChatKind::Local(_) | ChatKind::Object(_) | ChatKind::Im | ChatKind::ObjectIm => LineKind::Incoming,
            ChatKind::Own(_) => LineKind::Own,
            ChatKind::System => LineKind::System,
            ChatKind::Invite(_) | ChatKind::InventoryOffer(_) | ChatKind::Payment(_) => continue,
        };
        if !matches!(kind, LineKind::Own | LineKind::System) && world.mutes.text_muted(&native.source, &native.from) {
            continue;
        }
        // Native IM_SESSION_SEND leaves existing group text unfiltered. Apply
        // both group and individual receive restrictions at the export edge,
        // as Folderstorm filterChatEvent does (abbe3cb0, LGPL-2.1).
        if group && kind == LineKind::Incoming && (native.source.is_nil() || !world.rlva_allows_im("recvim", native.source)) {
            continue;
        }
        let (text, truncated) = line_text(&native.text);
        result.push(NativeLine {
            sender: label(&native.from),
            text,
            kind,
            truncated,
        });
    }
    NativePage {
        cursor: NativeCursor {
            token: clock.token,
            next: start + count,
        },
        gap: cursor.next < clock.first,
        lines: result,
    }
}

fn literal_command(world: &World, conversation: Conversation, text: &str) -> Option<NetCommand> {
    if ChatText::new(text).ok()?.as_str().trim().is_empty() {
        return None;
    }
    let state = snapshot(world, conversation, true)?;
    if !state.ready || !state.can_send {
        return None;
    }
    match conversation {
        Conversation::Nearby => {
            if world.rlva.outgoing_chat(text, 0, ChatType::Normal).is_empty() {
                return None;
            }
            Some(NetCommand::Chat {
                message: text.into(),
                channel: 0,
                chat_type: ChatType::Normal,
            })
        }
        Conversation::Direct(peer) => Some(NetCommand::SendIm {
            to: uuid(peer),
            message: text.into(),
        }),
        Conversation::Group(session) | Conversation::Conference(session) => Some(NetCommand::SendImDialog {
            to: uuid(session),
            dialog: crate::world::groups::dialog::SESSION_SEND,
            id: uuid(session),
            message: text.into(),
            bucket: Vec::new(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::chat::{ChatCommand, ChatEvent, ChatMessage, SendReceipt, SendRequest, VERSION};
    use aurora_session::{GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerId, WorkerIdentity};
    use std::sync::Arc;

    const PEER: Uuid = Uuid::from_u128(55);
    const OWNER: Uuid = Uuid::from_u128(99);
    const CONTROLLER: Uuid = Uuid::from_u128(77);

    fn source(generation: u64) -> ChatSource {
        ChatSource {
            worker: WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).unwrap(),
                host: HostId::from_bytes([2; 16]).unwrap(),
                worker: WorkerId::from_bytes([3; 16]).unwrap(),
            },
            session: SessionIdentity {
                session: SessionId::from_bytes([4; 16]).unwrap(),
                account: AccountKey::from_bytes([5; 32]).unwrap(),
                grid: GridKey::from_bytes([6; 32]).unwrap(),
                generation,
            },
            lease_epoch: 1,
        }
    }
    fn world() -> World {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = OWNER;
        world.main_region = Some(1);
        world.movement_complete = true;
        world.mutes.state = crate::world::mutes::LoadState::Loaded;
        world.social.names.insert(PEER, "Synthetic Resident".into());
        world.social.session_mut(PEER);
        world
    }
    fn restrict(world: &mut World, text: &str) {
        world.rlva.configure(true);
        world.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Synthetic controller".into(),
            source_id: CONTROLLER,
            owner_id: OWNER,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: text.into(),
        }));
    }
    fn say(world: &mut World, text: &str) {
        world.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Synthetic Resident".into(),
            source_id: PEER,
            owner_id: PEER,
            source_type: aurora_net::ChatSourceType::Agent,
            chat_type: ChatType::Normal,
            position: glam::Vec3::ZERO,
            message: text.into(),
        }));
    }

    struct Harness {
        world: World,
        source: Option<ChatSource>,
        sent: Vec<NetCommand>,
    }
    impl NativeChat for Harness {
        fn source(&self) -> Option<ChatSource> {
            self.source
        }
        fn catalogue(&self) -> Vec<Conversation> {
            catalogue(&self.world)
        }
        fn snapshot(&self, conversation: Conversation) -> Option<NativeSnapshot> {
            snapshot(&self.world, conversation, true)
        }
        fn lines(&self, conversation: Conversation, cursor: NativeCursor, limit: usize) -> NativePage {
            lines(&self.world, conversation, cursor, limit)
        }
        fn read_metadata(&self, conversation: Conversation) -> Option<aurora_session::chat::ReadMetadata> {
            self.source()?;
            read_metadata(&self.world, conversation, true)
        }
        fn mark_read(&mut self, request: aurora_session::chat::MarkReadRequest) -> aurora_session::chat::ReadOutcome {
            let source = self.source;
            mark_native_read(&mut self.world, request, true, &|| source == Some(request.key.source))
        }
        fn submit_literal(&mut self, key: ChatKey, text: &str) -> SendOutcome {
            if self.source() != Some(key.source) {
                return SendOutcome::RejectedNative;
            }
            let Some(command) = literal_command(&self.world, key.conversation, text) else {
                return SendOutcome::RejectedNative;
            };
            self.sent.push(command); // An offline native sink: no network/runtime/login.
            match key.conversation {
                Conversation::Direct(peer) => self.world.own_im(uuid(peer), text),
                Conversation::Group(session) => self.world.own_shared_group_im(uuid(session), text),
                Conversation::Conference(session) => self.world.own_shared_conference_im(uuid(session), text),
                _ => {}
            }
            SendOutcome::AcceptedLocally
        }
    }
    fn poll(source: ChatSource, sequence: u64) -> ChatPacket {
        ChatPacket {
            version: VERSION,
            source,
            sequence,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::Poll { after_event: 0 }),
        }
    }
    fn send(source: ChatSource, sequence: u64, conversation: Conversation, revision: u64, text: &str) -> ChatPacket {
        ChatPacket {
            version: VERSION,
            source,
            sequence,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::Send(SendRequest {
                key: ChatKey { source, conversation },
                request: sequence,
                policy_revision: revision,
                text: ChatText::new(text).unwrap(),
            })),
        }
    }

    #[test]
    fn host_chat_native_nearby_is_literal_and_resident_im_uses_existing_session_without_unblocking() {
        let mut world = world();
        let command = literal_command(&world, Conversation::Nearby, "/ao off").unwrap();
        assert!(matches!(command, NetCommand::Chat { ref message, channel: 0, chat_type: ChatType::Normal } if message == "/ao off"));
        let direct = Conversation::Direct(conversation_id(PEER).unwrap());
        assert!(matches!(
            literal_command(&world, direct, "Synthetic IM"),
            Some(NetCommand::SendIm { to: PEER, .. })
        ));
        let _ = world
            .mutes
            .add(
                crate::world::mutes::Mute {
                    id: PEER,
                    name: "Synthetic Resident".into(),
                    kind: crate::world::mutes::MuteType::Agent,
                    flags: 0,
                },
                0,
            )
            .unwrap();
        let version = world.mutes.version;
        assert!(literal_command(&world, direct, "Still blocked").is_none());
        assert!(world.mutes.is_muted_id(&PEER));
        assert_eq!(world.mutes.version, version);
        let unknown = Conversation::Direct(conversation_id(Uuid::from_u128(555)).unwrap());
        let count = world.social.ims.len();
        assert!(literal_command(&world, unknown, "No new conversation").is_none());
        assert_eq!(world.social.ims.len(), count);
        assert!(literal_command(&world, Conversation::Group(conversation_id(PEER).unwrap()), "No group join").is_none());
    }

    #[test]
    fn host_chat_native_fresh_rlva_and_mute_readiness_reject_sends_without_raw_echo() {
        let mut world = world();
        world.mutes.state = crate::world::mutes::LoadState::Initial;
        assert!(literal_command(&world, Conversation::Nearby, "Wait for block list").is_none());
        world.mutes.state = crate::world::mutes::LoadState::Loaded;
        restrict(&mut world, "@sendim=n,sendchat=n");
        let count = world.social.ims[0].lines.len();
        assert!(literal_command(&world, Conversation::Nearby, "Restricted text").is_none());
        assert!(literal_command(&world, Conversation::Direct(conversation_id(PEER).unwrap()), "Restricted IM").is_none());
        assert_eq!(world.social.ims[0].lines.len(), count);
    }

    #[test]
    fn host_chat_native_snapshot_excludes_interactive_cards_history_and_oversized_payloads() {
        let mut world = world();
        say(&mut world, "Synthetic live line");
        let clock = world.chat_clock;
        let mut offer = world.chat.back().unwrap().clone();
        offer.kind = ChatKind::InventoryOffer(1);
        world.chat.push_back(offer);
        world.chat_clock.pushed_back();
        let mut history = world.chat.front().unwrap().clone();
        history.origin = Origin::Log;
        world.chat.push_back(history);
        world.chat_clock.pushed_back();
        let page = lines(
            &world,
            Conversation::Nearby,
            NativeCursor {
                token: clock.token,
                next: clock.first,
            },
            1000,
        );
        assert_eq!(page.lines.len(), 1);
        assert_eq!(page.lines[0].text.as_str(), "Synthetic live line");
        let (bounded, truncated) = line_text(&"🌕".repeat(400));
        assert!(truncated && bounded.as_str().len() <= MAX_TEXT_BYTES);
        let changed = lines(
            &world,
            Conversation::Nearby,
            NativeCursor {
                token: clock.token + 1,
                next: clock.first,
            },
            1,
        );
        assert!(changed.gap && changed.lines.is_empty());
    }

    #[test]
    fn host_chat_native_backend_sends_while_inactive_and_rechecks_privacy_before_export() {
        let context = source(1);
        let mut native = Harness {
            world: world(),
            source: Some(context),
            sent: Vec::new(),
        };
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        say(&mut native.world, "Synthetic private live text");
        let first = backend.handle(&mut native, &poll(context, 1)).unwrap();
        assert!(first.iter().any(|packet| matches!(&packet.message, ChatMessage::Event(ChatEvent::Line(line)) if line.text.as_str() == "Synthetic private live text")));
        let revision = first
            .iter()
            .find_map(|packet| match &packet.message {
                ChatMessage::Event(ChatEvent::Conversation(info)) if info.policy.key.conversation == Conversation::Nearby => {
                    Some(info.policy.revision)
                }
                _ => None,
            })
            .unwrap();
        let reply = backend
            .handle(&mut native, &send(context, 2, Conversation::Nearby, revision, "/ao off"))
            .unwrap();
        assert_eq!(native.sent.len(), 1);
        assert!(matches!(&native.sent[0], NetCommand::Chat { message, channel: 0, .. } if message == "/ao off"));
        assert!(matches!(
            reply[0].message,
            ChatMessage::Event(ChatEvent::Receipt(SendReceipt {
                outcome: SendOutcome::AcceptedLocally,
                ..
            }))
        ));
        restrict(&mut native.world, "@shownames=n");
        let hidden = backend.handle(&mut native, &poll(context, 3)).unwrap();
        assert!(
            !hidden
                .iter()
                .any(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Line(_))))
        );
        let outgoing = backend.outgoing_sequence();
        native.source = None;
        assert_eq!(backend.sync(&native), Err(ChatError::Unavailable));
        assert_eq!(backend.current_source(), None);
        assert_eq!(backend.outgoing_sequence(), outgoing);
        native.source = Some(source(2));
        backend.sync(&native).unwrap();
        let restored = backend.handle(&mut native, &poll(source(2), 4)).unwrap();
        assert!(
            !restored
                .iter()
                .any(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Line(_))))
        );
        assert!(backend.outgoing_sequence() > outgoing);
    }
    fn capture_read(
        native: &mut Harness,
        backend: &mut WorkerChat,
        conversation: Conversation,
        request: u64,
    ) -> aurora_session::chat::MarkReadRequest {
        reconcile_read_interest(&mut native.world);
        backend.sync(native).unwrap();
        let metadata = native.read_metadata(conversation).unwrap();
        let replies = backend.handle(native, &poll(source(1), request * 10)).unwrap();
        let revision = replies
            .iter()
            .find_map(|packet| match &packet.message {
                ChatMessage::Event(ChatEvent::ReadBatch(rows)) => rows
                    .iter()
                    .find(|row| row.conversation == conversation)
                    .map(|row| row.policy_revision),
                _ => None,
            })
            .unwrap();
        aurora_session::chat::MarkReadRequest {
            key: ChatKey {
                source: source(1),
                conversation,
            },
            request,
            policy_revision: revision,
            observed: metadata.observed,
            unread_revision: metadata.unread_revision,
        }
    }

    fn apply_read(
        native: &mut Harness,
        backend: &mut WorkerChat,
        request: aurora_session::chat::MarkReadRequest,
        sequence: u64,
    ) -> aurora_session::chat::ReadReceipt {
        let packet = ChatPacket {
            version: VERSION,
            source: source(1),
            sequence,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::MarkRead(request)),
        };
        let replies = backend.handle(native, &packet).unwrap();
        match replies[0].message {
            ChatMessage::Event(ChatEvent::ReadReceipt(reply)) => reply.receipt.unwrap(),
            _ => panic!("typed read receipt"),
        }
    }

    #[test]
    fn host_chat_native_read_rejects_new_arrivals_equal_count_replacement_and_native_reads() {
        use aurora_session::chat::ReadOutcome;
        let mut native = Harness {
            world: world(),
            source: Some(source(1)),
            sent: Vec::new(),
        };
        let mut backend = WorkerChat::default();
        say(&mut native.world, "A");
        let a = capture_read(&mut native, &mut backend, Conversation::Nearby, 1);
        say(&mut native.world, "B");
        assert_eq!(apply_read(&mut native, &mut backend, a, 11).outcome, ReadOutcome::StaleObservation);
        assert_eq!(native.world.chat_badge, 2);
        let ab = capture_read(&mut native, &mut backend, Conversation::Nearby, 2);
        native.world.nearby_read();
        say(&mut native.world, "C");
        say(&mut native.world, "D");
        assert_eq!(native.world.chat_badge, 2, "counter returned to captured count");
        assert_eq!(apply_read(&mut native, &mut backend, ab, 21).outcome, ReadOutcome::StaleObservation);
        let cd = capture_read(&mut native, &mut backend, Conversation::Nearby, 3);
        let receipt = apply_read(&mut native, &mut backend, cd, 31);
        assert_eq!(receipt.outcome, ReadOutcome::Applied);
        say(&mut native.world, "E");
        assert_eq!(apply_read(&mut native, &mut backend, cd, 32), receipt);
        assert_eq!(native.world.chat_badge, 1, "duplicate cannot clear a later arrival");
        assert!(native.sent.is_empty());
        let e = capture_read(&mut native, &mut backend, Conversation::Nearby, 4);
        native.world.nearby_read();
        assert_eq!(apply_read(&mut native, &mut backend, e, 41).outcome, ReadOutcome::StaleObservation);
    }

    #[test]
    fn host_chat_native_read_renumber_history_replacement_and_privacy_are_authoritative() {
        use aurora_session::chat::{ReadOutcome, ReadReceiptQuery};
        let mut native = Harness {
            world: world(),
            source: Some(source(1)),
            sent: Vec::new(),
        };
        let mut backend = WorkerChat::default();
        say(&mut native.world, "A");
        let a = capture_read(&mut native, &mut backend, Conversation::Nearby, 1);
        native.world.clear_nearby_chat();
        assert_eq!(apply_read(&mut native, &mut backend, a, 11).outcome, ReadOutcome::StaleObservation);
        assert_eq!(native.world.chat_badge, 1, "clear history preserves native unread semantics");
        let direct = Conversation::Direct(conversation_id(PEER).unwrap());
        let old = capture_read(&mut native, &mut backend, direct, 2);
        native.world.leave_session(PEER);
        native.world.social.session_mut(PEER);
        assert_eq!(
            apply_read(&mut native, &mut backend, old, 21).outcome,
            ReadOutcome::StaleObservation
        );
        let next = capture_read(&mut native, &mut backend, direct, 3);
        let before = native.world.read_ledger.state(Some(PEER)).unwrap();
        native.world.own_im(PEER, "own");
        assert_eq!(native.world.read_ledger.state(Some(PEER)), Some(before));
        let mut history = native.world.social.ims[0].lines[0].clone();
        history.origin = Origin::Log;
        native.world.social.ims[0].lines.push_front(history);
        native.world.social.ims[0].clock.pushed_front(1);
        reconcile_read_interest(&mut native.world);
        assert_eq!(
            native.world.read_ledger.state(Some(PEER)),
            Some(before),
            "history creates no incoming mark"
        );
        assert_eq!(apply_read(&mut native, &mut backend, next, 31).outcome, ReadOutcome::AlreadyRead);
        restrict(&mut native.world, "@recvim=n");
        let query = ChatPacket {
            version: VERSION,
            source: source(1),
            sequence: 32,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::QueryReadReceipt(ReadReceiptQuery {
                key: next.key,
                request: next.request,
            })),
        };
        assert_eq!(backend.handle(&mut native, &query), Err(ChatError::Denied));
        assert!(read_metadata(&native.world, direct, true).is_none());
        assert!(native.sent.is_empty());
    }

    #[test]
    fn host_chat_native_read_quiet_badges_own_system_and_unexported_cards_keep_native_semantics() {
        let mut native = Harness {
            world: world(),
            source: Some(source(1)),
            sent: Vec::new(),
        };
        let mut backend = WorkerChat::default();
        reconcile_read_interest(&mut native.world);
        let original = native.world.read_ledger.state(None).unwrap();
        native.world.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Synthetic Owner".into(),
            source_id: OWNER,
            owner_id: OWNER,
            source_type: aurora_net::ChatSourceType::Agent,
            chat_type: ChatType::Normal,
            position: glam::Vec3::ZERO,
            message: "own nearby echo".into(),
        }));
        assert_eq!(native.world.read_ledger.state(None).unwrap().incoming, original.incoming);
        assert!(native.world.read_ledger.state(None).unwrap().revision > original.revision);
        native.world.system_message("local notice");
        assert_eq!(native.world.read_ledger.state(None).unwrap().incoming, original.incoming);
        native.world.quiet_automated_chat = true;
        let badge = native.world.chat_badge;
        native.world.system_message("quiet local notice");
        assert_eq!(native.world.chat_badge, badge);
        assert_eq!(native.world.chat_unread, 3);
        let observed = capture_read(&mut native, &mut backend, Conversation::Nearby, 1);
        assert_eq!(native.read_metadata(Conversation::Nearby).unwrap().unread, badge as u64);
        apply_read(&mut native, &mut backend, observed, 11);
        assert_eq!((native.world.chat_unread, native.world.chat_badge), (0, 0));
        let direct = Conversation::Direct(conversation_id(PEER).unwrap());
        let observed = capture_read(&mut native, &mut backend, direct, 2);
        // The counted card helper is exercised by actual producer tests; a
        // typing announcement is a System card and still native incoming.
        native.world.announce_incoming_im = true;
        native.world.leave_session(PEER);
        native.world.apply(aurora_net::NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: PEER,
            from_name: "Synthetic Resident".into(),
            to_agent_id: OWNER,
            dialog: 41,
            session_id: PEER,
            message: String::new(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        }));
        reconcile_read_interest(&mut native.world);
        assert_eq!(native.world.social.ims[0].unread, 1);
        assert_ne!(native.read_metadata(direct).unwrap().observed, observed.observed);
        assert!(native.sent.is_empty());
    }

    #[test]
    fn host_chat_native_read_fresh_source_mute_and_privacy_reject_before_reset() {
        use aurora_session::chat::ReadOutcome;
        let mut native = Harness {
            world: world(),
            source: Some(source(1)),
            sent: Vec::new(),
        };
        let mut backend = WorkerChat::default();
        say(&mut native.world, "private unread");
        let observed = capture_read(&mut native, &mut backend, Conversation::Nearby, 1);
        assert_eq!(
            mark_native_read(&mut native.world, observed, true, &|| false),
            ReadOutcome::RejectedNative
        );
        assert_eq!(native.world.chat_badge, 1);
        for change in 0..4 {
            let mut wrong = observed.key.source;
            match change {
                0 => wrong.session.generation += 1,
                1 => wrong.lease_epoch += 1,
                2 => wrong.session.account = AccountKey::from_bytes([8; 32]).unwrap(),
                _ => wrong.session.grid = GridKey::from_bytes([8; 32]).unwrap(),
            }
            let request = aurora_session::chat::MarkReadRequest {
                key: ChatKey {
                    source: wrong,
                    ..observed.key
                },
                ..observed
            };
            let packet = ChatPacket {
                version: VERSION,
                source: wrong,
                sequence: 999,
                reply_to: 0,
                message: ChatMessage::Command(ChatCommand::MarkRead(request)),
            };
            assert_eq!(backend.handle(&mut native, &packet), Err(ChatError::Binding));
            assert_eq!(native.world.chat_badge, 1);
        }
        restrict(&mut native.world, "@showloc=n");
        assert_eq!(
            mark_native_read(&mut native.world, observed, true, &|| true),
            ReadOutcome::RejectedNative
        );
        assert_eq!(native.world.chat_badge, 1);
        let direct = Conversation::Direct(conversation_id(PEER).unwrap());
        let mut clean = world();
        reconcile_read_interest(&mut clean);
        let metadata = read_metadata(&clean, direct, true).unwrap();
        let request = aurora_session::chat::MarkReadRequest {
            key: ChatKey {
                source: source(1),
                conversation: direct,
            },
            request: 1,
            policy_revision: 1,
            observed: metadata.observed,
            unread_revision: metadata.unread_revision,
        };
        clean
            .mutes
            .add(
                crate::world::mutes::Mute {
                    id: PEER,
                    name: "Synthetic Resident".into(),
                    kind: crate::world::mutes::MuteType::Agent,
                    flags: 0,
                },
                0,
            )
            .unwrap();
        assert_eq!(mark_native_read(&mut clean, request, true, &|| true), ReadOutcome::RejectedNative);
        assert!(native.sent.is_empty());
    }

    mod close_tests;
    mod conference_tests;
    mod group_tests;
}
