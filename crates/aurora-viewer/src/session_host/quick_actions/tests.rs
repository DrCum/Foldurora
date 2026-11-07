//! Metadata/correlation fixtures only, never actual native permission.
use super::*;
use aurora_session::{AccountKey, GridKey, SessionId, SessionIdentity, WorkerId, WorkerIdentity};

fn installation() -> InstallationId {
    InstallationId::from_bytes([1; 32]).unwrap()
}
fn host() -> HostId {
    HostId::from_bytes([2; 16]).unwrap()
}
fn slot(value: u8) -> SlotId {
    SlotId::try_from(value).unwrap()
}
fn running(phase: Phase) -> Facts {
    Facts {
        occupant: Some(Occupant {
            binding: Binding {
                worker: WorkerIdentity {
                    installation: installation(),
                    host: host(),
                    worker: WorkerId::from_bytes([3; 16]).unwrap(),
                },
                session: (phase == Phase::Online).then(|| SessionIdentity {
                    session: SessionId::from_bytes([4; 16]).unwrap(),
                    account: AccountKey::from_bytes([5; 32]).unwrap(),
                    grid: GridKey::from_bytes([6; 32]).unwrap(),
                    generation: 1,
                }),
            },
            lease_epoch: 7,
            phase,
            window: Some(WindowTag { app: 1, window: 1 }),
            current: true,
            managed: true,
            ordinary_window: phase != Phase::Online,
            blocked: false,
        }),
        ..Facts::default()
    }
}
#[test]
fn fixed_quick_policy_maps_empty_starting_online_and_ordinary_without_detached_or_lost_fallback() {
    let empty = Facts {
        available: true,
        ..Facts::default()
    };
    assert_eq!(choice(empty, false), Choice::OpenLogin);
    assert_eq!(choice(Facts { reserved: true, ..empty }, false), Choice::Opening);
    assert_eq!(choice(Facts::default(), false), Choice::Unavailable);
    for phase in [Phase::Login, Phase::Connecting, Phase::Online, Phase::Disconnected] {
        let mut facts = running(phase);
        assert_eq!(
            choice(facts, false),
            if phase == Phase::Online {
                Choice::Activate
            } else {
                Choice::ShowViewer
            }
        );
        assert_eq!(choice(facts, true), Choice::Unavailable);
        facts.occupant.as_mut().unwrap().blocked = true;
        assert_eq!(choice(facts, false), Choice::Unavailable);
        facts.occupant.as_mut().unwrap().blocked = false;
        facts.occupant.as_mut().unwrap().managed = false;
        assert_eq!(
            choice(facts, false),
            Choice::Unavailable,
            "detached cannot become raw HWND fallback"
        );
        facts.occupant.as_mut().unwrap().managed = true;
        facts.occupant.as_mut().unwrap().current = false;
        assert_eq!(choice(facts, false), Choice::Unavailable, "lost control is not a new login");
    }
    let mut facts = running(Phase::Login);
    facts.occupant.as_mut().unwrap().window = None;
    assert_eq!(choice(facts, false), Choice::Opening);
    facts.occupant.as_mut().unwrap().ordinary_window = false;
    assert_eq!(choice(facts, false), Choice::Unavailable);
}
#[test]
fn fixed_quick_captured_slot_and_action_ignore_card_order_or_management_selection() {
    let mut tracker = Tracker::new(installation(), host());
    for id in SlotId::all() {
        tracker.observe(id, running(Phase::Online)).unwrap();
    }
    let tickets: Vec<_> = SlotId::all().map(|id| tracker.capture(id).unwrap()).collect();
    for ticket in tickets.into_iter().rev() {
        assert_eq!(ticket.action(), Choice::Activate);
        assert_eq!(ticket.expected(), running(Phase::Online));
        assert!(tracker.current(ticket));
        assert!(tracker.current(tracker.capture(ticket.slot()).unwrap()));
    }
}
#[test]
fn fixed_quick_empty_opening_empty_and_global_busy_aba_never_revive_old_presses() {
    let mut tracker = Tracker::new(installation(), host());
    let empty = Facts {
        available: true,
        ..Facts::default()
    };
    tracker.observe(slot(0), empty).unwrap();
    let held = tracker.capture(slot(0)).unwrap();
    tracker
        .observe(
            slot(0),
            Facts {
                reserved: true,
                ..Facts::default()
            },
        )
        .unwrap();
    assert!(tracker.capture(slot(0)).is_none());
    tracker.observe(slot(0), empty).unwrap();
    assert!(!tracker.current(held));
    let held = tracker.capture(slot(0)).unwrap();
    tracker.set_busy(true).unwrap();
    tracker.set_busy(false).unwrap();
    assert!(!tracker.current(held));
    let held = tracker.capture(slot(0)).unwrap();
    tracker.boundary().unwrap(); // Real unpublished lifecycle boundary, even same latest facts.
    assert!(!tracker.current(held));
    let held = tracker.capture(slot(0)).unwrap();
    assert_eq!(tracker.observe(slot(0), empty), Ok(false));
    assert!(tracker.current(held), "unchanged heartbeat metadata is not a new action");
}
#[test]
fn fixed_quick_app_window_observation_and_full_session_replacement_reject_old_target() {
    let mut tracker = Tracker::new(installation(), host());
    let mut facts = running(Phase::Login);
    tracker.observe(slot(0), facts).unwrap();
    let held = tracker.capture(slot(0)).unwrap();
    facts.occupant.as_mut().unwrap().window = Some(WindowTag { app: 1, window: 2 });
    tracker.observe(slot(0), facts).unwrap();
    assert!(!tracker.current(held));
    let held = tracker.capture(slot(0)).unwrap();
    facts.occupant.as_mut().unwrap().window = Some(WindowTag { app: 2, window: 1 });
    tracker.observe(slot(0), facts).unwrap();
    assert!(!tracker.current(held));
    tracker.observe(slot(0), running(Phase::Online)).unwrap();
    let held = tracker.capture(slot(0)).unwrap();
    let mut next = running(Phase::Online);
    next.occupant.as_mut().unwrap().binding.session.as_mut().unwrap().generation += 1;
    tracker.observe(slot(0), next).unwrap();
    assert!(!tracker.current(held));
    assert_eq!(tracker.capture(slot(0)).unwrap().action(), Choice::Activate);
}
#[test]
fn fixed_quick_wrong_controller_zero_observation_and_exhaustion_fail_closed_without_retarget() {
    let mut tracker = Tracker::new(installation(), host());
    tracker.observe(slot(0), running(Phase::Online)).unwrap();
    let held = tracker.capture(slot(0)).unwrap();
    let mut invalid = running(Phase::Online);
    invalid.occupant.as_mut().unwrap().binding.worker.host = HostId::from_bytes([9; 16]).unwrap();
    assert_eq!(tracker.observe(slot(0), invalid), Err(Error::Invalid));
    assert!(!tracker.current(held));
    let mut invalid = running(Phase::Online);
    invalid.occupant.as_mut().unwrap().binding.worker.installation = InstallationId::from_bytes([8; 32]).unwrap();
    assert_eq!(tracker.observe(slot(0), invalid), Err(Error::Invalid));
    let mut invalid = running(Phase::Login);
    invalid.occupant.as_mut().unwrap().lease_epoch = 0;
    assert_eq!(tracker.observe(slot(0), invalid), Err(Error::Invalid));
    let mut invalid = running(Phase::Login);
    invalid.occupant.as_mut().unwrap().window = Some(WindowTag { app: 0, window: 1 });
    assert_eq!(tracker.observe(slot(0), invalid), Err(Error::Invalid));
    let mut invalid = running(Phase::Online);
    invalid.occupant.as_mut().unwrap().binding.session.as_mut().unwrap().generation = 0;
    assert_eq!(tracker.observe(slot(0), invalid), Err(Error::Invalid));
    tracker.observe(slot(0), running(Phase::Online)).unwrap();
    let held = tracker.capture(slot(0)).unwrap();
    let mut replacement = Tracker::new(installation(), host());
    replacement.observe(slot(0), running(Phase::Online)).unwrap();
    assert!(!replacement.current(held));
    tracker.serial = u64::MAX - 1;
    assert_eq!(tracker.boundary(), Err(Error::Exhausted));
    assert!(!tracker.current(held));
    assert_eq!(tracker.observe(slot(0), running(Phase::Online)), Err(Error::Exhausted));
}
