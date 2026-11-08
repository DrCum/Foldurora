//! Authored synthetic-only regressions. Root owns all execution, filesystem
//! fixtures and any native default acquisition; these tests are unrun by author.
use super::*;
use crate::session_host::{profile, slots::Registry};
use std::io::Write;
use std::sync::Mutex;

static WORKER_TEST: Mutex<()> = Mutex::new(());

fn slot(index: u8) -> SlotId {
    SlotId::try_from(index).unwrap()
}
fn selection() -> Selection {
    Selection {
        instance: Instance::fresh().unwrap(),
        incarnation: 1,
        target: slot(0),
        source: Source::Slot(slot(1)),
        families: Families::new(2).unwrap(),
    }
}
fn identities() -> (InstallationId, HostId) {
    let bytes = [*uuid::Uuid::new_v4().as_bytes(), *uuid::Uuid::new_v4().as_bytes()].concat();
    (
        InstallationId::from_bytes(bytes.try_into().unwrap()).unwrap(),
        HostId::from_bytes(*uuid::Uuid::new_v4().as_bytes()).unwrap(),
    )
}
struct Fixture {
    installation: InstallationId,
    registry: Registry,
    inspect: Inspect,
}
impl Fixture {
    fn new() -> Self {
        let (installation, host) = identities();
        let registry = Registry::open(installation, host).unwrap();
        let inspect = Inspect {
            capture: Capture {
                installation,
                controller: host,
                manager: Instance::fresh().unwrap(),
                operation: 1,
                selection: selection(),
            },
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let fixture = Self {
            installation,
            registry,
            inspect,
        };
        fixture.write(
            slot(1),
            br#"{"settings_version":32,"ui_scale":1.25,"excluded_source":"never copied"}"#,
        );
        fixture
    }
    fn path(&self, slot: SlotId) -> std::path::PathBuf {
        profile::slot_root(&self.installation.to_string(), slot)
            .unwrap()
            .join("config/settings.json")
    }
    fn write(&self, slot: SlotId, bytes: &[u8]) {
        let root = profile::slot_root(&self.installation.to_string(), slot).unwrap();
        profile::private_dir(&root).unwrap();
        profile::private_dir(&root.join("config")).unwrap();
        let path = self.path(slot);
        if path.exists() {
            std::fs::remove_file(&path).unwrap();
        }
        let mut file = profile::create_private_file(&path).unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
    }
    fn prepared(&self) -> io::Prepared {
        io::Prepared::inspect(&self.inspect).unwrap_or_else(|outcome| panic!("synthetic prepare: {outcome:?}"))
    }
    fn bytes(&self, slot: SlotId) -> Option<Vec<u8>> {
        match std::fs::read(self.path(slot)) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("synthetic read: {error}"),
        }
    }
}

#[test]
fn profile_copy_capture_identity_and_selection_incarnation_never_alias() {
    let first = selection();
    let mut second = first;
    second.incarnation += 1;
    assert_ne!(first, second);
    assert_ne!(Instance::fresh(), Instance::fresh());
    second.source = Source::Slot(second.target);
    assert!(!second.valid());
    second.source = Source::MainSaved;
    assert!(second.valid());
}

fn synthetic_lane() -> (Lane, Sender<Reply>, Receiver<Command>) {
    let (sender, commands) = bounded(1);
    let (replies, receiver) = bounded(1);
    let mut lane = Lane::default();
    lane.worker = Some(Worker {
        sender,
        replies: receiver,
        _thread: std::thread::spawn(|| {}),
    });
    (lane, replies, commands)
}

#[test]
fn profile_copy_real_lane_reserves_both_slots_and_cancel_requires_original_request() {
    let (mut lane, replies, commands) = synthetic_lane();
    let (installation, host) = identities();
    let selection = selection();
    lane.inspect(installation, host, 7, selection).unwrap();
    let Command::Inspect(inspect) = commands.try_recv().unwrap() else {
        panic!("Inspect")
    };
    assert!(lane.reserved(slot(0)) && lane.reserved(slot(1)));
    lane.cancel_request(selection, 8);
    assert!(!inspect.cancelled.load(Ordering::Acquire));
    let mut changed = selection;
    changed.incarnation += 1;
    lane.cancel_request(changed, 7);
    assert!(!inspect.cancelled.load(Ordering::Acquire));
    lane.cancel_request(selection, 7);
    assert!(inspect.cancelled.load(Ordering::Acquire));
    assert!(lane.reserved(slot(0)), "logical cancel does not release reservation");
    replies
        .try_send(Reply::Terminal(inspect.capture, Outcome::CancelledBeforeCommit))
        .unwrap();
    lane.poll();
    assert!(!lane.reserved(slot(0)) && !lane.reserved(slot(1)));
}

#[test]
fn profile_copy_real_lane_ignores_late_receipts_and_rejects_stale_apply() {
    let (mut lane, replies, commands) = synthetic_lane();
    let (installation, host) = identities();
    lane.inspect(installation, host, 3, selection()).unwrap();
    let Command::Inspect(inspect) = commands.try_recv().unwrap() else {
        panic!("Inspect")
    };
    let mut old = inspect.capture;
    old.selection.incarnation += 1;
    replies.try_send(Reply::Terminal(old, Outcome::Applied)).unwrap();
    lane.poll();
    assert!(lane.reserved(slot(0)));
    replies.try_send(Reply::Prepared(inspect.capture, vec![])).unwrap();
    lane.poll();
    assert_eq!(lane.apply(old), Err(Failure::Invalid));
    assert!(commands.is_empty());
    lane.apply(inspect.capture).unwrap();
    assert!(matches!(commands.try_recv().unwrap(), Command::Apply(capture) if capture == inspect.capture));
}

#[test]
fn profile_copy_lost_terminal_receipt_is_uncertain_and_never_replays() {
    let (mut lane, replies, commands) = synthetic_lane();
    let (installation, host) = identities();
    lane.inspect(installation, host, 4, selection()).unwrap();
    let _ = commands.try_recv().unwrap();
    drop(replies);
    lane.poll();
    assert!(matches!(lane.state().unwrap().phase, Phase::Terminal(Outcome::Uncertain)));
    assert!(!lane.reserved(slot(0)));
    assert!(commands.is_empty());
    assert!(lane.worker.is_some(), "no implicit new worker/replay on disconnect");
}

#[test]
fn profile_copy_512_resets_cannot_refund_the_actual_retiring_worker_permit() {
    let _serial = WORKER_TEST.lock().unwrap();
    let permit = PhysicalPermit::acquire().unwrap();
    let (release, blocked) = bounded::<()>(1);
    let (finished, retired) = bounded(1);
    let thread = std::thread::spawn(move || {
        let _permit = permit;
        blocked.recv().unwrap();
        finished.send(()).unwrap();
    });
    for _ in 0..512 {
        let mut reset = Lane::default();
        assert!(matches!(reset.worker(), Err(Failure::Busy)));
        assert!(reset.worker.is_none(), "no channel/thread allocation while physically occupied");
    }
    release.try_send(()).unwrap();
    retired.recv_timeout(Duration::from_secs(2)).unwrap();
    thread.join().unwrap();
    assert!(PhysicalPermit::acquire().is_some());
}

#[test]
fn profile_copy_actual_worker_spawn_failure_drops_physical_permit_without_start_or_replay() {
    let _serial = WORKER_TEST.lock().unwrap();
    let mut lane = Lane::default();
    assert!(matches!(
        lane.worker_with(|_job| Err(std::io::Error::other("synthetic spawn failure"))),
        Err(Failure::Spawn)
    ));
    assert!(lane.worker.is_none() && lane.state().is_none() && lane.active.is_none());
    assert!(PhysicalPermit::acquire().is_some(), "failed spawn has no running worker to charge");
}

#[test]
fn profile_copy_inspection_retains_real_source_and_target_lifetime_locks_until_drop() {
    let fixture = Fixture::new();
    let prepared = fixture.prepared();
    for slot in [slot(0), slot(1)] {
        assert!(!profile::slot_available(&fixture.installation.to_string(), slot).unwrap());
    }
    drop(prepared);
    for slot in [slot(0), slot(1)] {
        assert!(profile::slot_available(&fixture.installation.to_string(), slot).unwrap());
    }
    assert!(fixture.bytes(slot(0)).is_none());
}

#[test]
fn profile_copy_target_and_source_locked_by_active_or_detached_owner_refuse_without_writing() {
    for busy in [slot(0), slot(1)] {
        let fixture = Fixture::new();
        fixture.write(slot(0), br#"{"settings_version":32,"ui_scale":1.0}"#);
        let original = fixture.bytes(slot(0));
        let root = profile::slot_root(&fixture.installation.to_string(), busy).unwrap();
        let _owner = profile::profile_lock(&root).unwrap();
        assert!(matches!(
            io::Prepared::inspect(&fixture.inspect),
            Err(Outcome::Refused(Failure::Io))
        ));
        assert_eq!(fixture.bytes(slot(0)), original);
    }
}

#[test]
fn profile_copy_original_target_cas_preserves_changed_or_newly_created_target_bytes() {
    for initially_present in [false, true] {
        let fixture = Fixture::new();
        if initially_present {
            fixture.write(slot(0), br#"{"settings_version":32,"ui_scale":1.0}"#);
        }
        let prepared = fixture.prepared();
        let changed = br#"{"settings_version":32,"ui_scale":1.1,"sentinel":"external change"}"#;
        fixture.write(slot(0), changed);
        assert_eq!(prepared.apply(&fixture.inspect, Instant::now() + CONFIRMATION), Outcome::Changed);
        assert_eq!(fixture.bytes(slot(0)).as_deref(), Some(changed.as_slice()));
    }
}

#[test]
fn profile_copy_source_cas_requires_new_inspect_and_preserves_original_target() {
    let fixture = Fixture::new();
    fixture.write(slot(0), br#"{"settings_version":32,"ui_scale":1.0}"#);
    let original = fixture.bytes(slot(0));
    let prepared = fixture.prepared();
    fixture.write(slot(1), br#"{"settings_version":32,"ui_scale":1.5}"#);
    assert_eq!(prepared.apply(&fixture.inspect, Instant::now() + CONFIRMATION), Outcome::Changed);
    assert_eq!(fixture.bytes(slot(0)), original);
}

#[test]
fn profile_copy_cancel_and_original_expiry_before_commit_never_write() {
    for cancel in [false, true] {
        let fixture = Fixture::new();
        let prepared = fixture.prepared();
        let deadline = if cancel { Instant::now() + CONFIRMATION } else { Instant::now() };
        fixture.inspect.cancelled.store(cancel, Ordering::Release);
        assert_eq!(
            prepared.apply(&fixture.inspect, deadline),
            if cancel { Outcome::CancelledBeforeCommit } else { Outcome::Expired }
        );
        assert!(fixture.bytes(slot(0)).is_none());
    }
}

#[test]
fn profile_copy_apply_changes_only_selected_native_fields_without_copying_source_unknowns() {
    let fixture = Fixture::new();
    fixture.write(
        slot(0),
        br#"{"settings_version":32,"ui_scale":1.0,"unknown_target":{"n":123456789012345678901234567890},"draw_distance":200.0}"#,
    );
    let prepared = fixture.prepared();
    assert!(prepared.summary.iter().any(|summary| summary.changed > 0 && summary.defaulted > 0));
    assert_eq!(prepared.apply(&fixture.inspect, Instant::now() + CONFIRMATION), Outcome::Applied);
    let bytes = String::from_utf8(fixture.bytes(slot(0)).unwrap()).unwrap();
    assert!(bytes.contains("123456789012345678901234567890"));
    assert!(bytes.contains("\"draw_distance\":200.0"));
    assert!(!bytes.contains("excluded_source") && !bytes.contains("never copied"));
    assert_eq!(serde_json::from_str::<serde_json::Value>(&bytes).unwrap()["ui_scale"], 1.25);
}

#[test]
fn profile_copy_malformed_selected_newer_and_missing_source_refuse_original_bytes() {
    for source in [
        Some(br#"{"settings_version":33}"#.as_slice()),
        Some(br#"{"settings_version":32,"ui_scale":0}"#.as_slice()),
        Some(br#"{"settings_version":32,"ui_scale":1,"ui_scale":2}"#.as_slice()),
        Some(b"malformed".as_slice()),
        None,
    ] {
        let fixture = Fixture::new();
        fixture.write(slot(0), b"original target untouched on source error");
        match source {
            Some(bytes) => fixture.write(slot(1), bytes),
            None => std::fs::remove_file(fixture.path(slot(1))).unwrap(),
        }
        assert!(io::Prepared::inspect(&fixture.inspect).is_err());
        assert_eq!(
            fixture.bytes(slot(0)).as_deref(),
            Some(b"original target untouched on source error".as_slice())
        );
    }
}

#[test]
fn profile_copy_controller_replacement_refuses_prepared_original_target() {
    let fixture = Fixture::new();
    let prepared = fixture.prepared();
    let installation = fixture.installation;
    let inspect = fixture.inspect;
    drop(fixture.registry);
    let replacement = HostId::from_bytes(*uuid::Uuid::new_v4().as_bytes()).unwrap();
    let _new_registry = Registry::open(installation, replacement).unwrap();
    assert_eq!(
        prepared.apply(&inspect, Instant::now() + CONFIRMATION),
        Outcome::Refused(Failure::Stopped)
    );
    let target = profile::slot_root(&installation.to_string(), slot(0))
        .unwrap()
        .join("config/settings.json");
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn profile_copy_symlink_hardlink_public_file_and_replaced_lifetime_lock_refuse_without_repair() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for kind in 0..4 {
        let fixture = Fixture::new();
        fixture.write(slot(0), br#"{"settings_version":32,"ui_scale":1}"#);
        let original = fixture.bytes(slot(0));
        let prepared = fixture.prepared();
        match kind {
            0 => {
                let path = fixture.path(slot(0));
                std::fs::remove_file(&path).unwrap();
                symlink(fixture.path(slot(1)), path).unwrap();
            }
            1 => std::fs::hard_link(fixture.path(slot(0)), fixture.path(slot(0)).with_extension("hardlink")).unwrap(),
            2 => std::fs::set_permissions(fixture.path(slot(0)), std::fs::Permissions::from_mode(0o666)).unwrap(),
            3 => {
                let root = profile::slot_root(&fixture.installation.to_string(), slot(0)).unwrap();
                std::fs::remove_file(root.join("profile.lock")).unwrap();
                drop(profile::create_private_file(&root.join("profile.lock")).unwrap());
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            prepared.apply(&fixture.inspect, Instant::now() + CONFIRMATION),
            Outcome::Refused(Failure::Io)
        ));
        if kind != 0 {
            assert_eq!(fixture.bytes(slot(0)), original);
        }
        if kind == 2 {
            assert_eq!(
                std::fs::metadata(fixture.path(slot(0))).unwrap().permissions().mode() & 0o777,
                0o666
            );
        }
    }
}
