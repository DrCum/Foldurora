//! Literal, account-bound chat through this worker's native world. No keyboard
//! events, chatbar/gesture commands, offer replies, group joins or unblocking.

use super::{App, Screen};
use crate::session_host::chat::{NativeChat, NativeCursor, NativeLine, NativePage, NativeSnapshot, SCAN_LINES, WorkerChat};
use crate::world::{ChatKind, Origin, World};
use aurora_net::{ChatType, NetCommand};
use aurora_session::chat::{
    ChatError, ChatKey, ChatPacket, ChatSource, ChatText, Conversation, LineKind, MAX_CONVERSATIONS, MAX_TEXT_BYTES, SendOutcome,
};
use aurora_session::{AccountKey, ConversationId, Label};
use uuid::Uuid;

impl App {
    pub(super) fn host_chat_sync(&mut self, backend: &mut WorkerChat, authority: &dyn Fn() -> Option<ChatSource>) -> Result<(), ChatError> {
        self.configure_host_chat_rules();
        backend.sync(&AppChat { app: self, authority })
    }

    pub(super) fn host_chat_packet(
        &mut self,
        backend: &mut WorkerChat,
        authority: &dyn Fn() -> Option<ChatSource>,
        packet: &ChatPacket,
    ) -> Result<Vec<ChatPacket>, ChatError> {
        self.configure_host_chat_rules();
        backend.handle(&mut AppChat { app: self, authority }, packet)
    }

    fn configure_host_chat_rules(&mut self) {
        // The same runtime layer as send_from; never save Preferences here.
        self.world.rlva_configure(self.settings.rlva_enabled);
        self.world.rlva.preferences.clone_from(&self.settings.rlva);
    }
}

struct AppChat<'a> {
    app: &'a mut App,
    authority: &'a dyn Fn() -> Option<ChatSource>,
}

impl NativeChat for AppChat<'_> {
    fn source(&self) -> Option<ChatSource> {
        let source = (self.authority)()?;
        if !matches!(self.app.screen, Screen::World) || self.app.world.agent_id.is_nil() {
            return None;
        }
        let uri = self.app.settings.login_uri();
        let account = AccountKey::parse_hex(&crate::characters::identity(&uri, &self.app.settings.username)).ok()?;
        let grid = crate::session_host::transport::grid_key(&uri).ok()?;
        (source.session.account == account && source.session.grid == grid).then_some(source)
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
        if let Conversation::Direct(peer) = key.conversation {
            self.app.world.own_im(uuid(peer), text);
        }
        // App::send / NetClient::send have no enqueue result. The call is made,
        // but we cannot promise acceptance or remote delivery. Never retry it.
        SendOutcome::Uncertain
    }
}

fn uuid(id: ConversationId) -> Uuid {
    Uuid::from_bytes(*id.as_bytes())
}
fn conversation_id(id: Uuid) -> Option<ConversationId> {
    ConversationId::from_bytes(*id.as_bytes()).ok()
}

fn catalogue(world: &World) -> Vec<Conversation> {
    let mut result = vec![Conversation::Nearby];
    // Bound the inspected native entries too: thousands of group tabs must
    // not turn a small IPC request into an unbounded main-thread search.
    result.extend(
        world
            .social
            .ims
            .iter()
            .take(MAX_CONVERSATIONS - 1)
            .filter(|session| !world.is_group_session(&session.other))
            .filter_map(|session| conversation_id(session.other).map(Conversation::Direct)),
    );
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
        Conversation::Direct(peer) if !world.is_group_session(&uuid(peer)) => world
            .social
            .ims
            .iter()
            .take(MAX_CONVERSATIONS - 1)
            .find(|session| session.other == uuid(peer))
            .map(|session| (&session.lines, session.clock)),
        _ => None,
    }
}

fn snapshot(world: &World, conversation: Conversation, ui_ready: bool) -> Option<NativeSnapshot> {
    let (entries, clock) = native_lines(world, conversation)?;
    let r = &world.rlva;
    // Native UI can mask these at draw time. Fail closed here rather than
    // exporting raw historical names/locations from an earlier privacy state.
    let privacy = r.enabled() && (r.has("shownames") || r.has("showloc") || r.has("shownearby"));
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
            let can_read = !muted && !privacy && world.rlva_allows_im("recvim", peer);
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
        _ => return None,
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
    Some(NativeSnapshot {
        title: label(&title),
        stamp: [r.revision, world.mutes.version, flags, world.main_region.unwrap_or(0)],
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
        _ => None,
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
        fn submit_literal(&mut self, key: ChatKey, text: &str) -> SendOutcome {
            if self.source() != Some(key.source) {
                return SendOutcome::RejectedNative;
            }
            let Some(command) = literal_command(&self.world, key.conversation, text) else {
                return SendOutcome::RejectedNative;
            };
            self.sent.push(command); // An offline native sink: no network/runtime/login.
            if let Conversation::Direct(peer) = key.conversation {
                self.world.own_im(uuid(peer), text);
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
}
