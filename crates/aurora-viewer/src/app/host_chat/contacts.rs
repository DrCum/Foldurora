//! Model-only host contact operations. Do not route through App's native
//! conversation-window opener: that path also changes focus/activation.
use super::*;
use crate::session_host::chat::{NativeContactPolicy, NativeContacts};
use aurora_session::chat::{ContactChannel, ContactRow, ContactText, MAX_FRIEND_CONTACTS, OpenDirectOutcome, OpenDirectRequest};
use std::collections::BTreeSet;

pub(super) fn policy(world: &World, ui_ready: bool) -> NativeContactPolicy {
    let can_browse = ui_ready
        && !world.agent_id.is_nil()
        && world.login.as_ref().is_some_and(|login| login.agent_id == world.agent_id)
        && world.mutes.is_loaded()
        && world.shared_chat_privacy_allowed();
    NativeContactPolicy {
        stamp: [
            world.rlva.revision,
            world.mutes.version,
            world.contact_policy_revision(),
            u64::from(can_browse),
        ],
        can_browse,
    }
}
fn eligible(world: &World, peer: Uuid) -> bool {
    !peer.is_nil()
        && peer != world.agent_id
        && !world.is_group_session(&peer)
        && !world.groups.is_member(&peer)
        && world.shared_chat_direct_readable(peer)
        && world.allows_direct_contact(peer)
        && world.rlva_can_start_im(peer)
        && world.rlva_allows_im("sendim", peer)
}
fn exportable(world: &World, peer: Uuid) -> bool {
    world.social.ims.iter().take(MAX_CONVERSATIONS - 1).any(|im| im.other == peer) || world.social.ims.len() < MAX_CONVERSATIONS - 1
}
fn text<const LIMIT: usize>(name: &str) -> ContactText<LIMIT> {
    let mut bounded = String::new();
    for ch in name.chars().filter(|ch| !ch.is_control()) {
        if bounded.len() + ch.len_utf8() > LIMIT {
            break;
        }
        bounded.push(ch);
    }
    ContactText::new(&bounded).expect("bounded non-control text")
}
pub(super) fn friends(world: &World, ui_ready: bool) -> Result<NativeContacts, ChatError> {
    if !policy(world, ui_ready).can_browse {
        return Err(ChatError::Denied);
    }
    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();
    // Bound inspected entries as well as exported rows. In the unusual case
    // of >2048 friendships, report a limited subset, never claim completeness.
    for friend in world.social.friends.iter().take(MAX_FRIEND_CONTACTS) {
        if friend.id == world.agent_id || !seen.insert(friend.id) {
            continue;
        }
        let Some(resident) = conversation_id(friend.id) else {
            continue;
        };
        let canonical = world.social.avatar_names.raw(&friend.id);
        let display = canonical.map(|name| name.display_name.as_str()).filter(|name| !name.is_empty());
        let display = text(display.unwrap_or(&friend.id.to_string()));
        rows.push(ContactRow {
            resident,
            display: if display.as_str().is_empty() {
                text(&friend.id.to_string())
            } else {
                display
            },
            username: text(canonical.map_or("", |name| name.username.as_str())),
            online: friend.online,
            can_open: eligible(world, friend.id) && exportable(world, friend.id),
        });
    }
    rows.sort_by(|a, b| {
        b.online
            .cmp(&a.online)
            .then_with(|| {
                a.display
                    .as_str()
                    .to_ascii_lowercase()
                    .cmp(&b.display.as_str().to_ascii_lowercase())
            })
            .then_with(|| a.username.as_str().cmp(b.username.as_str()))
            .then_with(|| a.resident.cmp(&b.resident))
    });
    Ok(NativeContacts {
        rows,
        limited: world.social.friends.len() > MAX_FRIEND_CONTACTS,
    })
}
pub(super) fn open(world: &mut World, request: OpenDirectRequest, ui_ready: bool) -> OpenDirectOutcome {
    let peer = uuid(request.resident);
    if request.validate().is_err()
        || request.selection.channel != ContactChannel::Friends
        || !policy(world, ui_ready).can_browse
        || !world
            .social
            .friends
            .iter()
            .take(MAX_FRIEND_CONTACTS)
            .any(|friend| friend.id == peer)
        || !eligible(world, peer)
    {
        return OpenDirectOutcome::RejectedNative;
    }
    if !exportable(world, peer) {
        return OpenDirectOutcome::Capacity;
    }
    // Crucially, startim was checked before session_mut: a new empty session
    // must not manufacture the existing-session exemption. No other effect.
    world.social.session_mut(peer);
    let conversation = Conversation::Direct(request.resident);
    if !catalogue(world).contains(&conversation) {
        return OpenDirectOutcome::Uncertain;
    }
    OpenDirectOutcome::Opened {
        resident: request.resident,
        key: ChatKey {
            source: request.selection.source,
            conversation,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::{AccountKey, GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerId, WorkerIdentity};
    const OWNER: Uuid = Uuid::from_u128(901);
    const PEER: Uuid = Uuid::from_u128(902);
    fn world() -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.bind_session_test_login(OWNER);
        world.mutes.state = crate::world::mutes::LoadState::Loaded;
        world.social.form_friendship(PEER);
        world
    }
    fn request() -> OpenDirectRequest {
        OpenDirectRequest {
            selection: aurora_session::chat::ContactKey {
                source: ChatSource {
                    worker: WorkerIdentity {
                        installation: InstallationId::from_bytes([1; 32]).unwrap(),
                        host: HostId::from_bytes([2; 16]).unwrap(),
                        worker: WorkerId::from_bytes([3; 16]).unwrap(),
                    },
                    session: SessionIdentity {
                        session: SessionId::from_bytes([4; 16]).unwrap(),
                        account: AccountKey::from_bytes([5; 32]).unwrap(),
                        grid: GridKey::from_bytes([6; 32]).unwrap(),
                        generation: 1,
                    },
                    lease_epoch: 1,
                },
                channel: ContactChannel::Friends,
                token: 1,
            },
            resident: conversation_id(PEER).unwrap(),
            request: 1,
            policy_revision: 1,
        }
    }
    fn restrict(world: &mut World, command: &str) {
        world.rlva.configure(true);
        world.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Synthetic controller".into(),
            source_id: Uuid::from_u128(903),
            owner_id: OWNER,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: ChatType::OwnerSay,
            message: command.into(),
            position: glam::Vec3::ZERO,
        }));
    }
    #[test]
    fn contacts_native_offline_friend_opens_only_model_without_focus_read_send_or_incoming_credit() {
        let mut world = world();
        assert!(friends(&world, true).unwrap().rows[0].can_open);
        let activity = world.shared_chat_close_fence();
        let focus = world.social.focus_im;
        assert!(matches!(open(&mut world, request(), true), OpenDirectOutcome::Opened { .. }));
        assert!(world.social.has_session(&PEER));
        assert_eq!(world.social.focus_im, focus);
        let session = &world.social.ims[0];
        assert_eq!(session.unread, 0);
        assert!(session.lines.is_empty() && session.input.is_empty());
        assert_eq!(world.shared_chat_close_fence(), activity);
        assert!(world.rlva.out.is_empty());
        world.social.ims[0].unread = 7;
        world.social.ims[0].input = "Retained native draft".into();
        assert!(matches!(open(&mut world, request(), true), OpenDirectOutcome::Opened { .. }));
        assert_eq!(world.social.ims.len(), 1);
        assert_eq!(world.social.ims[0].unread, 7);
        assert_eq!(world.social.ims[0].input, "Retained native draft");
    }
    #[test]
    fn contacts_native_checks_startim_before_creation_and_rechecks_friendship_privacy_and_send() {
        for restriction in ["@startim=n", "@sendim=n", "@recvim=n", "@shownames=n"] {
            let mut world = world();
            restrict(&mut world, restriction);
            assert_eq!(
                open(&mut world, request(), true),
                OpenDirectOutcome::RejectedNative,
                "{restriction}"
            );
            assert!(!world.social.has_session(&PEER));
        }
        let mut world = world();
        let offered = friends(&world, true).unwrap();
        assert!(offered.rows[0].can_open);
        world.social.end_friendship(&PEER);
        assert_eq!(open(&mut world, request(), true), OpenDirectOutcome::RejectedNative);
        assert!(world.social.ims.is_empty());
    }
    #[test]
    fn contacts_native_names_exclude_unqualified_social_headers_and_use_current_canonical_cache() {
        let mut world = world();
        world.social.names.insert(PEER, "Forged IM header".into());
        let rows = friends(&world, true).unwrap().rows;
        assert_eq!(rows[0].display.as_str(), PEER.to_string());
        assert!(rows[0].username.as_str().is_empty());
        world.social.avatar_names.on_legacy(PEER, "Canonical", "Resident");
        let rows = friends(&world, true).unwrap().rows;
        assert_eq!(rows[0].display.as_str(), "Canonical");
        assert_eq!(rows[0].username.as_str(), "Canonical");
    }
    #[test]
    fn contacts_native_refuses_unexportable_capacity_group_collision_and_fsdata_denial() {
        let mut world = world();
        for id in 1..MAX_CONVERSATIONS {
            world.social.session_mut(Uuid::from_u128(id as u128));
        }
        assert!(!friends(&world, true).unwrap().rows[0].can_open);
        assert_eq!(open(&mut world, request(), true), OpenDirectOutcome::Capacity);
        assert!(!world.social.has_session(&PEER));
        let mut world = self::world();
        use aurora_net::contact_policy::{Feed, NO_SUPPORT, Patch, SUPPORT};
        world.apply(aurora_net::NetEvent::ContactPolicy {
            source: aurora_net::LoginSource::from_login(world.login.as_ref().unwrap()),
            feed: Feed::Agents,
            result: Ok(Patch {
                agents: [(OWNER, NO_SUPPORT), (PEER, SUPPORT)].into(),
                legacy: false,
            }),
        });
        assert!(!friends(&world, true).unwrap().rows[0].can_open);
        assert_eq!(open(&mut world, request(), true), OpenDirectOutcome::RejectedNative);
        assert!(world.social.ims.is_empty());
    }
    #[test]
    fn contacts_native_friend_export_bounds_inspection_and_reports_limited_subset() {
        let mut world = world();
        world.social.friends.clear();
        world
            .social
            .friends
            .extend((1..=MAX_FRIEND_CONTACTS + 1).map(|id| crate::world::social::Friend {
                id: Uuid::from_u128(10_000 + id as u128),
                online: id % 2 == 0,
                rights_given: 0,
                rights_has: 0,
            }));
        let snapshot = friends(&world, true).unwrap();
        assert_eq!(snapshot.rows.len(), MAX_FRIEND_CONTACTS);
        assert!(snapshot.limited);
        assert!(snapshot.rows[..MAX_FRIEND_CONTACTS / 2].iter().all(|row| row.online));
    }
}
