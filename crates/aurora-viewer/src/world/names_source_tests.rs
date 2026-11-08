//! Source/destination isolation regressions needed by the host contact chooser.
use super::*;
use crate::world::World;
use crate::world::account_metadata_origin::{GridDigest, Pending, request};
use aurora_net::{NameSource, NetEvent};
use std::sync::Arc;

const ME: Uuid = Uuid::from_u128(0xc011_0001);
const PEER: Uuid = Uuid::from_u128(0xc011_0002);

fn world() -> World {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.bind_session_test_login(ME);
    world
}

fn display(source: NameSource, text: &str) -> NetEvent {
    NetEvent::DisplayNames {
        source,
        names: vec![AvatarNameData {
            id: PEER,
            display_name: text.into(),
            username: "peer.resident".into(),
            ..Default::default()
        }],
        bad_ids: vec![],
        max_age: Some(60),
    }
}

#[test]
fn name_source_reused_wire_identifiers_do_not_accept_an_old_login_object() {
    let mut w = world();
    let old_login = w.login.as_ref().unwrap().clone();
    let old = NameSource::from_login(&old_login);
    // An actual new login object, deliberately reusing every server field.
    w.login = Some(Arc::new((*old_login).clone()));
    let current = NameSource::from_login(w.login.as_ref().unwrap());
    w.apply(display(current, "Current name"));
    w.social.session_mut(PEER).input = "Unsent draft".into();
    w.social.ims[0].unread = 7;

    w.apply(display(old.clone(), "Wrong account name"));
    w.apply(NetEvent::Names {
        source: old.clone(),
        names: vec![(PEER, "Wrong".into(), "Resident".into())],
    });
    w.apply(NetEvent::DisplayNameUpdate {
        source: old,
        name: AvatarNameData {
            id: PEER,
            display_name: "Wrong update".into(),
            ..Default::default()
        },
        old_display_name: "Current name".into(),
    });
    assert_eq!(w.social.avatar_names.raw(&PEER).unwrap().display_name, "Current name");
    assert!(!w.social.names.contains_key(&PEER));
    assert_eq!((w.social.ims[0].input.as_str(), w.social.ims[0].unread), ("Unsent draft", 7));
    assert_eq!(w.social.focus_im, None);
    assert!(w.take_social_commands().is_empty());
}

#[test]
fn name_source_stale_failure_and_display_dialog_reply_have_no_replacement_side_effect() {
    let mut w = world();
    let old = NameSource::from_login(w.login.as_ref().unwrap());
    w.bind_session_test_login(ME);
    w.social.avatar_names.want(&PEER);
    assert_eq!(w.social.avatar_names.take_asks(), vec![PEER]);
    w.apply(NetEvent::DisplayNamesFailed {
        source: old.clone(),
        ids: vec![PEER],
    });
    assert!(w.social.avatar_names.legacy_asks.is_empty());
    assert!(w.social.avatar_names.pending.contains_key(&PEER));
    assert!(
        w.apply(NetEvent::SetDisplayNameReply {
            source: old,
            status: 403,
            reason: "Previous account".into(),
            content: Llsd::Undef,
        })
        .is_none()
    );

    let source = NameSource::from_login(w.login.as_ref().unwrap());
    w.apply(NetEvent::DisplayNamesFailed {
        source: source.clone(),
        ids: vec![PEER],
    });
    assert_eq!(w.social.avatar_names.legacy_asks, vec![PEER]);
    assert!(matches!(
        w.apply(NetEvent::SetDisplayNameReply {
            source,
            status: 200,
            reason: "OK".into(),
            content: Llsd::Undef,
        }),
        Some(NetEvent::SetDisplayNameReply { status: 200, .. })
    ));
}

#[test]
fn name_source_after_reset_cannot_repopulate_cache_and_current_legacy_still_resolves() {
    let mut w = world();
    let old = NameSource::from_login(w.login.as_ref().unwrap());
    w.reset();
    w.apply(display(old, "Old cached name"));
    assert!(w.social.avatar_names.raw(&PEER).is_none());
    w.bind_session_test_login(ME);
    w.social.avatar_names.legacy_protocol = true;
    w.apply(NetEvent::Names {
        source: NameSource::from_login(w.login.as_ref().unwrap()),
        names: vec![(PEER, "Peer".into(), "Resident".into())],
    });
    assert_eq!(w.social.names[&PEER], "Peer");
    assert_eq!(w.social.avatar_names.raw(&PEER).unwrap().legacy_first, "Peer");
}

#[test]
fn name_source_cache_custom_grid_punctuation_and_uri_components_do_not_collide() {
    let grids = [
        "https://grid-a.example/login",
        "https://grid.a.example/login",
        "https://grid.a.example/login?grid=one",
        "https://grid.a.example/login?grid=two",
        "https://grid.a.example/Login",
        "https://grid.a.example:8443/login",
    ];
    let files: HashSet<_> = grids.iter().map(|uri| cache_file(GridDigest::from_uri(uri).unwrap())).collect();
    assert_eq!(files.len(), grids.len());
    assert!(files.iter().all(|file| file.len() == 86 && !file.contains("example")));
    assert_eq!(
        cache_file(GridDigest::from_uri(&format!(" {} ", grids[0])).unwrap()),
        cache_file(GridDigest::from_uri(grids[0]).unwrap())
    );
    assert_eq!(
        cache_file(GridDigest::from_uri(aurora_net::login::SL_MAIN_GRID).unwrap()),
        "avatar_name_cache.xml"
    );
    assert_eq!(
        cache_file(GridDigest::from_uri(aurora_net::login::SL_BETA_GRID).unwrap()),
        "avatar_name_cache.aditi.xml"
    );
}

#[test]
fn name_source_diagnostics_never_print_the_retained_login() {
    let w = world();
    let source = NameSource::from_login(w.login.as_ref().unwrap());
    assert_eq!(format!("{source:?}"), "NameSource(..)");
}

#[test]
fn name_source_cache_uses_consumed_original_request_and_refuses_ambiguous_submission() {
    let mut pending = Pending::default();
    let mut first = request("https://first.example.invalid/login");
    pending.submitted(&first);
    first.login_uri = "https://edited.example.invalid/login".into();
    let file = pending.completed().map(cache_file).unwrap();
    assert_eq!(
        file,
        cache_file(GridDigest::from_uri("https://first.example.invalid/login").unwrap())
    );
    assert!(pending.completed().map(cache_file).is_none());
    pending.submitted(&first);
    pending.submitted(&request("https://ignored.example.invalid/login"));
    assert!(pending.completed().map(cache_file).is_none());
    pending.failed();
    pending.submitted(&request(aurora_net::login::SL_MAIN_GRID));
    assert_eq!(pending.completed().map(cache_file).as_deref(), Some("avatar_name_cache.xml"));
}

#[test]
fn name_source_alternate_demo_captures_the_actual_login_before_producing_a_reply() {
    let mut w = world();
    let login = w.login.as_ref().unwrap().clone();
    for event in crate::demo::demo_reply_for_login(&aurora_net::NetCommand::RequestDisplayNames(vec![PEER]), &login) {
        w.apply(event);
    }
    assert!(w.social.avatar_names.raw(&PEER).is_some());
    // A later synthetic login cannot consume a previously produced reply.
    let old_reply = crate::demo::demo_reply_for_login(&aurora_net::NetCommand::RequestNames(vec![PEER]), &login);
    w.bind_session_test_login(ME);
    for event in old_reply {
        w.apply(event);
    }
    assert!(!w.social.names.contains_key(&PEER));
}
