use super::*;
use aurora_llsd::{Llsd, Map, llsd_map};
use aurora_net::{NetEvent, SessionMethod};

const OWNER: Uuid = Uuid::from_u128(1);
const PEER: Uuid = Uuid::from_u128(2);
const SESSION: Uuid = Uuid::from_u128(100);

fn world() -> World {
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.bind_session_test_login(OWNER);
    world.mutes.state = crate::world::mutes::LoadState::Loaded;
    world
}

fn base(participants: &[Uuid], muted: bool) -> Llsd {
    let mut members = Map::new();
    for participant in participants {
        let metadata = if *participant == OWNER {
            llsd_map! { "mutes" => llsd_map! { "text" => muted } }
        } else {
            llsd_map! {}
        };
        members.insert(participant.to_string(), metadata);
    }
    llsd_map! { "agent_info" => Llsd::Map(members) }
}

fn batch(body: &Llsd) -> SessionAgentBatch {
    aurora_net::social::parse_session_agent_batch(body, aurora_net::social::MAX_SESSION_ROSTER_ENTRIES)
}

fn delta(session: Uuid, participant: Uuid, transition: Option<&str>, muted: Option<bool>) -> NetEvent {
    let mut update = Map::new();
    if let Some(transition) = transition {
        update.insert("transition".into(), transition.into());
    }
    if let Some(muted) = muted {
        update.insert("info".into(), llsd_map! { "mutes" => llsd_map! { "text" => muted } });
    }
    NetEvent::SessionAgents {
        session,
        agents: batch(&llsd_map! { "agent_updates" => llsd_map! { participant.to_string() => Llsd::Map(update) } }),
    }
}

fn acceptance(world: &mut World) -> RosterInitialization {
    world.info_mut(SESSION, "Synthetic conference");
    world.session_lines(SESSION);
    world.begin_session_initialization(SESSION, false).unwrap()
}

fn accepted(capture: RosterInitialization, result: Option<Llsd>, not_found: bool) -> NetEvent {
    NetEvent::ChatSessionReply {
        method: SessionMethod::AcceptInvitation,
        session: SESSION,
        initialization: Some(capture),
        result,
        not_found,
    }
}

fn group_member(world: &mut World) {
    world.groups.groups.push(aurora_net::GroupMembership {
        id: SESSION,
        name: "Synthetic group".into(),
        insignia: Uuid::nil(),
        powers: 0,
        accept_notices: true,
        list_in_profile: true,
        contribution: 0,
    });
}

fn voice_acceptance(world: &mut World) -> (RosterInitialization, Vec<aurora_net::NetCommand>) {
    world.accept_voice_invite(aurora_net::VoiceInvite {
        session: SESSION,
        session_name: "Synthetic group".into(),
        from: PEER,
        from_name: "Synthetic resident".into(),
        p2p: false,
        voice: llsd_map! { "channel_uri" => "synthetic-channel", "channel_credentials" => "synthetic-credential" },
    });
    let commands = world.groups.take_commands();
    let capture = commands
        .iter()
        .find_map(|command| match command {
            aurora_net::NetCommand::ChatSession {
                method: SessionMethod::AcceptInvitation,
                initialization,
                ..
            } => *initialization,
            _ => None,
        })
        .unwrap();
    (capture, commands)
}

fn group_started(success: bool, muted: bool) -> NetEvent {
    NetEvent::SessionStarted {
        temp_session: SESSION,
        session: SESSION,
        success,
        error: if success { String::new() } else { "synthetic-start-failure".into() },
        agents: batch(&base(&[OWNER, PEER], muted)),
    }
}

fn history_count(world: &mut World) -> usize {
    world
        .groups
        .take_commands()
        .iter()
        .filter(|command| {
            matches!(
                command,
                aurora_net::NetCommand::ChatSession {
                    method: SessionMethod::FetchHistory,
                    session: SESSION,
                    ..
                }
            )
        })
        .count()
}

#[test]
fn conference_roster_native_group_voice_start_success_survives_http_failure_in_either_order_with_history_once() {
    for start_first in [false, true] {
        let mut world = world();
        group_member(&mut world);
        let (capture, commands) = voice_acceptance(&mut world);
        let accept = commands
            .iter()
            .position(|command| {
                matches!(
                    command,
                    aurora_net::NetCommand::ChatSession {
                        method: SessionMethod::AcceptInvitation,
                        ..
                    }
                )
            })
            .unwrap();
        let start = commands
            .iter()
            .position(|command| {
                matches!(
                    command,
                    aurora_net::NetCommand::SendImDialog {
                        dialog: super::super::groups::dialog::SESSION_GROUP_START,
                        ..
                    }
                )
            })
            .unwrap();
        assert!(accept < start, "native wire order is preserved");
        world.apply(delta(SESSION, OWNER, None, Some(true)));
        let events = if start_first {
            [group_started(true, false), accepted(capture, None, false)]
        } else {
            [accepted(capture, None, false), group_started(true, false)]
        };
        for event in events {
            world.apply(event);
        }
        assert!(world.groups.sessions[&SESSION].active && !world.groups.sessions[&SESSION].closed);
        assert!(world.groups.sessions[&SESSION].text_muted);
        assert!(world.session_roster(SESSION).is_some());
        assert_eq!(history_count(&mut world), 1);
    }
}

#[test]
fn conference_roster_native_group_voice_successful_acceptance_survives_redundant_start_failure_in_either_order() {
    for start_first in [false, true] {
        let mut world = world();
        group_member(&mut world);
        let (capture, _) = voice_acceptance(&mut world);
        let success = accepted(capture, Some(base(&[OWNER, PEER], true)), false);
        let events = if start_first {
            [group_started(false, false), success]
        } else {
            [success, group_started(false, false)]
        };
        for event in events {
            world.apply(event);
        }
        assert!(world.groups.sessions[&SESSION].active && !world.groups.sessions[&SESSION].closed);
        assert!(world.groups.sessions[&SESSION].text_muted);
        assert_eq!(history_count(&mut world), 1);
    }
}

#[test]
fn conference_roster_native_group_voice_404_retires_both_original_paths_before_any_late_success() {
    for start_first in [false, true] {
        let mut world = world();
        group_member(&mut world);
        let (capture, _) = voice_acceptance(&mut world);
        let events = if start_first {
            [group_started(true, false), accepted(capture, None, true)]
        } else {
            [accepted(capture, None, true), group_started(true, false)]
        };
        for event in events {
            world.apply(event);
        }
        assert!(!world.groups.sessions[&SESSION].active && world.groups.sessions[&SESSION].closed);
        assert!(world.session_roster(SESSION).is_none());
        assert_eq!(history_count(&mut world), usize::from(start_first));
        world.apply(accepted(capture, Some(base(&[OWNER, PEER], false)), false));
        world.apply(group_started(true, false));
        assert!(!world.groups.sessions[&SESSION].active && world.groups.sessions[&SESSION].closed);
        assert_eq!(history_count(&mut world), 0);
    }
}

#[test]
fn conference_roster_native_group_voice_retries_replace_only_the_same_path_and_preserve_newer_stream_mute() {
    let mut world = world();
    group_member(&mut world);
    let (old, _) = voice_acceptance(&mut world);
    let retry = world.begin_session_initialization(SESSION, false).unwrap();
    world.apply(accepted(old, None, true));
    assert!(!world.groups.sessions[&SESSION].closed);
    world.apply(delta(SESSION, OWNER, None, Some(true)));
    world.apply(group_started(true, false));
    assert!(world.groups.sessions[&SESSION].active && world.groups.sessions[&SESSION].text_muted);
    world.apply(accepted(retry, Some(base(&[OWNER, PEER], false)), false));
    assert!(world.groups.sessions[&SESSION].active && world.groups.sessions[&SESSION].text_muted);
    assert!(world.session_roster(SESSION).is_some());
    assert_eq!(history_count(&mut world), 1);
}

#[test]
fn conference_roster_native_group_voice_existing_live_group_uses_acceptance_without_restarting_or_unmuting() {
    let mut world = world();
    group_member(&mut world);
    let initial = acceptance(&mut world);
    world.apply(accepted(initial, Some(base(&[OWNER, PEER], true)), false));
    assert_eq!(history_count(&mut world), 1);
    let (capture, commands) = voice_acceptance(&mut world);
    assert!(!commands.iter().any(|command| matches!(
        command,
        aurora_net::NetCommand::SendImDialog {
            dialog: super::super::groups::dialog::SESSION_GROUP_START,
            ..
        }
    )));
    world.apply(accepted(capture, Some(base(&[OWNER, PEER], false)), false));
    assert!(world.groups.sessions[&SESSION].active && world.groups.sessions[&SESSION].text_muted);
    assert_eq!(history_count(&mut world), 0);
}

#[test]
fn conference_roster_native_group_voice_unordered_bootstraps_cannot_infer_unmute_but_a_live_delta_can() {
    for start_first in [false, true] {
        let mut world = world();
        group_member(&mut world);
        let (capture, _) = voice_acceptance(&mut world);
        let success = accepted(capture, Some(base(&[OWNER, PEER], true)), false);
        let events = if start_first {
            [group_started(true, false), success]
        } else {
            [success, group_started(true, false)]
        };
        for event in events {
            world.apply(event);
        }
        assert!(world.groups.sessions[&SESSION].text_muted);
        assert!(!world.shared_group_sendable(SESSION));
        assert_eq!(history_count(&mut world), 1);
        world.apply(delta(SESSION, OWNER, None, Some(false)));
        assert!(!world.groups.sessions[&SESSION].text_muted && world.shared_group_sendable(SESSION));
    }
}

#[test]
fn conference_roster_native_group_voice_new_intent_mute_is_not_overridden_by_an_older_lifetime_unmute_or_counterpart() {
    for already_active in [false, true] {
        let mut world = world();
        group_member(&mut world);
        if already_active {
            let initial = acceptance(&mut world);
            world.apply(accepted(initial, Some(base(&[OWNER, PEER], false)), false));
            history_count(&mut world);
            world.apply(delta(SESSION, OWNER, None, Some(false)));
        }
        let (original, _) = voice_acceptance(&mut world);
        let current = if already_active {
            original
        } else {
            world.apply(delta(SESSION, OWNER, None, Some(false)));
            world.begin_session_initialization(SESSION, false).unwrap()
        };
        world.apply(accepted(current, Some(base(&[OWNER, PEER], true)), false));
        assert!(world.groups.sessions[&SESSION].text_muted);
        assert!(!world.shared_group_sendable(SESSION));
        if !already_active {
            // The still-current counterpart predates the new acceptance intent.
            // Its older false cannot clear the newer bootstrap's mute either.
            world.apply(group_started(true, false));
            assert!(world.groups.sessions[&SESSION].text_muted);
            assert!(!world.shared_group_sendable(SESSION));
        }
        world.apply(delta(SESSION, OWNER, None, Some(false)));
        assert!(!world.groups.sessions[&SESSION].text_muted && world.shared_group_sendable(SESSION));
    }
}

#[test]
fn conference_roster_native_acceptance_replays_prebase_updates_and_repeated_bootstrap_cannot_unmute_live_state() {
    let mut world = world();
    let capture = acceptance(&mut world);
    world.apply(delta(SESSION, OWNER, None, Some(true)));
    world.apply(delta(SESSION, PEER, Some("LEAVE"), None));
    world.apply(accepted(capture, Some(base(&[OWNER, PEER], false)), false));
    let snapshot = world.session_roster(SESSION).unwrap();
    assert_eq!(snapshot.self_text_mute, Some(true));
    assert!(!snapshot.participants.contains(&PEER));
    assert!(world.groups.sessions[&SESSION].text_muted);
    let revision = snapshot.revision;
    let repeated = world.begin_session_initialization(SESSION, false).unwrap();
    world.apply(accepted(repeated, Some(base(&[OWNER, PEER], false)), false));
    assert_eq!(world.session_roster(SESSION).unwrap().revision, revision);
    assert!(world.groups.sessions[&SESSION].text_muted);
    assert!(!world.groups.sessions[&SESSION].participants.contains(&PEER));
    world.apply(delta(SESSION, PEER, Some("ENTER"), None));
    assert!(world.session_roster(SESSION).unwrap().revision > revision);
}

#[test]
fn conference_roster_native_replaced_callbacks_cannot_apply_success_failure_or_duplicate_completion() {
    let mut world = world();
    let old = acceptance(&mut world);
    let current = world.begin_session_initialization(SESSION, false).unwrap();
    for stale in [accepted(old, None, true), accepted(old, Some(base(&[OWNER, PEER], true)), false)] {
        world.apply(stale);
        let info = &world.groups.sessions[&SESSION];
        assert!(!info.active && !info.closed && !info.text_muted);
        assert!(world.session_roster(SESSION).is_none());
    }
    world.apply(accepted(current, Some(base(&[OWNER, PEER], false)), false));
    let revision = world.session_roster(SESSION).unwrap().revision;
    world.apply(accepted(current, None, true));
    world.apply(accepted(current, Some(base(&[OWNER], true)), false));
    assert!(world.groups.sessions[&SESSION].active && !world.groups.sessions[&SESSION].closed);
    assert!(!world.groups.sessions[&SESSION].text_muted);
    assert_eq!(world.session_roster(SESSION).unwrap().revision, revision);
}

#[test]
fn conference_roster_native_leave_reopen_and_same_account_relogin_reject_old_originals() {
    let mut world = world();
    let old = acceptance(&mut world);
    world.leave_session(SESSION);
    let replacement = acceptance(&mut world);
    assert_ne!(old.incarnation(), replacement.incarnation());
    world.apply(accepted(old, Some(base(&[OWNER, PEER], true)), false));
    world.apply(accepted(old, None, true));
    assert!(!world.groups.sessions[&SESSION].active && !world.groups.sessions[&SESSION].closed);
    world.apply(accepted(replacement, Some(base(&[OWNER, PEER], false)), false));
    assert!(world.session_roster(SESSION).is_some());

    let before_relogin = world.begin_session_initialization(SESSION, false).unwrap();
    world.bind_session_test_login(OWNER);
    world.apply(accepted(before_relogin, None, true));
    world.apply(accepted(before_relogin, Some(base(&[OWNER], true)), false));
    assert!(!world.groups.sessions[&SESSION].closed && !world.groups.sessions[&SESSION].text_muted);
    assert!(world.session_roster(SESSION).is_none());
    let after_relogin = acceptance(&mut world);
    world.apply(accepted(after_relogin, Some(base(&[OWNER, PEER], false)), false));
    assert!(world.session_roster(SESSION).is_some());
}

#[test]
fn conference_roster_native_start_consumes_original_call_witness_then_merges_final_pending_mute_and_recipients() {
    let mut world = world();
    world.start_text_conference(vec![PEER, Uuid::from_u128(4)]);
    let temporary = *world.calls.conferences.keys().next().unwrap();
    let extra = Uuid::from_u128(3);
    world.apply(delta(SESSION, extra, Some("ENTER"), None));
    world.apply(delta(SESSION, OWNER, None, Some(true)));
    world.apply(NetEvent::SessionStarted {
        temp_session: temporary,
        session: SESSION,
        success: true,
        error: String::new(),
        agents: batch(&base(&[OWNER, PEER], false)),
    });
    assert!(!world.calls.conferences.contains_key(&temporary));
    assert!(!world.groups.sessions.contains_key(&temporary));
    assert!(!world.social.has_session(&temporary));
    let snapshot = world.session_roster(SESSION).unwrap();
    assert!(snapshot.participants.contains(&extra));
    assert_eq!(snapshot.self_text_mute, Some(true));
    assert!(world.groups.sessions[&SESSION].text_muted);
}

#[test]
fn conference_roster_native_final_force_close_with_or_without_pending_data_cannot_be_resurrected_by_an_old_base() {
    for pending in [false, true] {
        let mut world = world();
        world.start_text_conference(vec![PEER, Uuid::from_u128(4)]);
        let temporary = *world.calls.conferences.keys().next().unwrap();
        if pending {
            world.apply(delta(SESSION, Uuid::from_u128(3), Some("ENTER"), None));
        }
        world.apply(NetEvent::SessionClosed {
            session: SESSION,
            reason: "synthetic close".into(),
        });
        world.apply(NetEvent::SessionStarted {
            temp_session: temporary,
            session: SESSION,
            success: true,
            error: String::new(),
            agents: batch(&base(&[OWNER, PEER], false)),
        });
        assert!(world.session_roster(SESSION).is_none());
        assert!(!world.groups.sessions.contains_key(&SESSION));
        assert!(!world.groups.sessions[&temporary].active);
        assert!(world.session_start_capture(temporary).is_none());
    }
}

#[test]
fn conference_roster_native_proof_budget_loss_keeps_current_and_new_native_initialization_callbacks_working() {
    let mut world = world();
    let original = acceptance(&mut world);
    for number in 1000..1000 + MAX_TRACKED as u128 {
        world.apply(delta(Uuid::from_u128(number), PEER, Some("ENTER"), None));
    }
    assert!(world.groups.rosters.lost);
    assert!(world.groups.rosters.entries.len() <= MAX_TRACKED);
    world.apply(accepted(original, Some(base(&[OWNER, PEER], true)), false));
    assert!(world.groups.sessions[&SESSION].active);
    assert!(world.groups.sessions[&SESSION].text_muted);
    assert!(world.session_roster(SESSION).is_none());
    world.leave_session(SESSION);
    let new = acceptance(&mut world);
    world.apply(accepted(new, Some(base(&[OWNER, PEER], true)), false));
    assert!(world.groups.sessions[&SESSION].active && world.groups.sessions[&SESSION].text_muted);
    assert!(
        world.session_roster(SESSION).is_none(),
        "new HTTP cookies cannot repair lost history"
    );
}

#[test]
fn conference_roster_native_incoming_serial_exhaustion_does_not_end_the_native_login_scope() {
    let mut world = world();
    let capture = acceptance(&mut world);
    world.shared_chat_incoming = None;
    assert!(world.shared_chat_close_fence().is_none());
    world.apply(accepted(capture, Some(base(&[OWNER, PEER], true)), false));
    assert!(world.groups.sessions[&SESSION].active && world.groups.sessions[&SESSION].text_muted);
    assert!(world.session_roster(SESSION).is_some());
}

#[test]
fn conference_roster_native_proof_saturation_cannot_erase_final_close_and_close_loss_only_blocks_ambiguous_remaps() {
    for overflow_closes in [false, true] {
        let mut world = world();
        world.start_text_conference(vec![PEER, Uuid::from_u128(4)]);
        let temporary = *world.calls.conferences.keys().next().unwrap();
        for number in 1000..1000 + MAX_TRACKED as u128 {
            world.apply(delta(Uuid::from_u128(number), PEER, Some("ENTER"), None));
        }
        let count = if overflow_closes { MAX_TRACKED + 1 } else { 1 };
        for number in 0..count {
            world.apply(NetEvent::SessionClosed {
                session: Uuid::from_u128(5000 + number as u128),
                reason: String::new(),
            });
        }
        let final_session = if overflow_closes { SESSION } else { Uuid::from_u128(5000) };
        world.apply(NetEvent::SessionStarted {
            temp_session: temporary,
            session: final_session,
            success: true,
            error: String::new(),
            agents: batch(&base(&[OWNER, PEER], false)),
        });
        assert!(!world.groups.sessions.contains_key(&final_session));
        assert!(world.session_roster(final_session).is_none());
        let capture = acceptance(&mut world);
        world.apply(accepted(capture, Some(base(&[OWNER, PEER], true)), false));
        assert!(world.groups.sessions[&SESSION].active && world.groups.sessions[&SESSION].text_muted);
        assert!(world.session_roster(SESSION).is_none());
        // A close-history loss only constrains starts already pending at the
        // close. Future native starts can still initialize without proof.
        world.start_text_conference(vec![PEER, Uuid::from_u128(4)]);
        let fresh = *world.calls.conferences.keys().find(|session| **session != temporary).unwrap();
        let fresh_final = Uuid::from_u128(9000);
        world.apply(NetEvent::SessionStarted {
            temp_session: fresh,
            session: fresh_final,
            success: true,
            error: String::new(),
            agents: batch(&base(&[OWNER, PEER], false)),
        });
        assert!(world.groups.sessions[&fresh_final].active);
        assert!(world.session_roster(fresh_final).is_none());
    }
}

#[test]
fn conference_roster_native_proof_saturation_cannot_let_an_older_bootstrap_clear_a_newer_group_moderator_mute() {
    let mut world = world();
    world.groups.groups.push(aurora_net::GroupMembership {
        id: SESSION,
        name: "Synthetic group".into(),
        insignia: Uuid::nil(),
        powers: 0,
        accept_notices: true,
        list_in_profile: true,
        contribution: 0,
    });
    let capture = acceptance(&mut world);
    world.apply(delta(SESSION, OWNER, None, Some(true)));
    for number in 1000..1000 + MAX_TRACKED as u128 {
        world.apply(delta(Uuid::from_u128(number), PEER, Some("ENTER"), None));
    }
    world.apply(accepted(capture, Some(base(&[OWNER, PEER], false)), false));
    assert!(world.groups.sessions[&SESSION].active && world.groups.sessions[&SESSION].text_muted);
    assert!(!world.shared_group_sendable(SESSION));
    assert!(world.session_roster(SESSION).is_none());
    world.apply(delta(SESSION, OWNER, None, Some(false)));
    assert!(!world.groups.sessions[&SESSION].text_muted);
    assert!(
        world.shared_group_sendable(SESSION),
        "a later explicit live unmute remains native authority"
    );
}

#[test]
fn conference_roster_native_proof_refusal_never_fabricates_closes_or_disables_a_later_conference_start() {
    let mut world = world();
    for number in 1000..1000 + MAX_TRACKED as u128 {
        world.apply(delta(Uuid::from_u128(number), PEER, Some("ENTER"), None));
    }
    for number in 2000..2000 + MAX_TRACKED as u128 + 1 {
        let session = Uuid::from_u128(number);
        world.info_mut(session, "Synthetic native session");
        world.begin_session_initialization(session, false).unwrap();
    }
    assert!(world.groups.rosters.closed.is_empty());
    assert!(world.groups.rosters.closed_history_lost_through.is_none());
    world.start_text_conference(vec![PEER, Uuid::from_u128(4)]);
    let temporary = *world.calls.conferences.keys().next().unwrap();
    world.apply(NetEvent::SessionStarted {
        temp_session: temporary,
        session: SESSION,
        success: true,
        error: String::new(),
        agents: batch(&base(&[OWNER, PEER], true)),
    });
    assert!(world.groups.sessions[&SESSION].active && world.groups.sessions[&SESSION].text_muted);
    assert!(world.session_roster(SESSION).is_none());
}

#[test]
fn conference_roster_native_unknown_final_mute_survives_proof_saturation_and_lost_mute_history_never_infers_unmute() {
    for overflow_mutes in [false, true] {
        let mut world = world();
        world.start_text_conference(vec![PEER, Uuid::from_u128(4)]);
        let temporary = *world.calls.conferences.keys().next().unwrap();
        if overflow_mutes {
            for number in 1000..1000 + MAX_TRACKED as u128 {
                world.apply(delta(Uuid::from_u128(number), OWNER, None, Some(true)));
            }
        } else {
            for number in 1000..1000 + MAX_TRACKED as u128 {
                world.apply(delta(Uuid::from_u128(number), PEER, Some("ENTER"), None));
            }
        }
        world.apply(delta(SESSION, OWNER, None, Some(true)));
        world.apply(NetEvent::SessionStarted {
            temp_session: temporary,
            session: SESSION,
            success: true,
            error: String::new(),
            agents: batch(&base(&[OWNER, PEER], false)),
        });
        assert!(world.groups.sessions[&SESSION].active && world.groups.sessions[&SESSION].text_muted);
        assert!(world.session_roster(SESSION).is_none());
        world.apply(delta(SESSION, OWNER, None, Some(false)));
        assert!(!world.groups.sessions[&SESSION].text_muted);
    }
}

#[test]
fn conference_roster_native_member_and_pending_overflow_withhold_proof_without_losing_native_self_mute() {
    let mut world = world();
    let capture = acceptance(&mut world);
    let participants = (1..=MAX_PARTICIPANTS as u128 + 1).map(Uuid::from_u128).collect::<Vec<_>>();
    world.apply(accepted(capture, Some(base(&participants, true)), false));
    assert!(world.groups.sessions[&SESSION].active && world.groups.sessions[&SESSION].text_muted);
    assert!(world.session_roster(SESSION).is_none());

    let mut world = self::world();
    let capture = acceptance(&mut world);
    for _ in 0..=MAX_PENDING {
        world.apply(delta(SESSION, OWNER, None, Some(true)));
    }
    world.apply(accepted(capture, Some(base(&[OWNER, PEER], false)), false));
    assert!(world.groups.sessions[&SESSION].active && world.groups.sessions[&SESSION].text_muted);
    assert!(world.session_roster(SESSION).is_none());
}
