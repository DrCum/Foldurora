use super::super::Favorites;
use super::*;
use crate::world::inventory::{FetchState, Folder};
use aurora_net::inventory::{InvFolder, InvItem};
use std::time::{Duration, Instant};

pub(crate) fn request(uri: &str) -> aurora_net::LoginRequest {
    aurora_net::LoginRequest {
        login_uri: uri.into(),
        username: String::new(),
        password: String::new(),
        start: aurora_net::StartLocation::Last,
        agree_to_tos: false,
        read_critical: false,
        mfa_token: String::new(),
        mfa_hash: String::new(),
        mac: String::new(),
        id0: String::new(),
        last_exec: aurora_net::login::LastExec::default(),
        platform: aurora_net::login::Platform::default(),
    }
}
fn scope(uri: &str) -> GroupScope {
    GroupScope::new(Uuid::from_u128(101), Uuid::from_u128(102), LoginGrid::from_uri(uri).unwrap()).unwrap()
}
fn inventory(account: Uuid) -> Inventory {
    let mut inv = Inventory {
        owner: account,
        root: Uuid::from_u128(1),
        ..Default::default()
    };
    for (id, parent, kind, children, items) in [
        (1, 0, 8, vec![Uuid::from_u128(2)], vec![]),
        (
            2,
            1,
            super::super::FOLDER_FAVORITES,
            vec![],
            vec![Uuid::from_u128(10), Uuid::from_u128(11), Uuid::from_u128(12)],
        ),
    ] {
        inv.folders.insert(
            Uuid::from_u128(id),
            Folder {
                info: InvFolder {
                    id: Uuid::from_u128(id),
                    parent: Uuid::from_u128(parent),
                    type_default: kind,
                    ..Default::default()
                },
                children,
                items,
                state: FetchState::Fetched,
                library: false,
            },
        );
    }
    for (id, name) in [(10, "First landmark"), (11, "Middle landmark"), (12, "Last landmark")] {
        inv.items.insert(
            Uuid::from_u128(id),
            InvItem {
                id: Uuid::from_u128(id),
                parent: Uuid::from_u128(2),
                owner: account,
                name: name.into(),
                asset_id: Uuid::from_u128(id + 100),
                asset_type: super::super::ASSET_LANDMARK,
                created_at: 100 - id as i64,
                ..Default::default()
            },
        );
    }
    inv.generation = 1;
    inv
}
pub(crate) struct Fixture {
    pub favorites: Favorites,
    pub inventory: Inventory,
    pub account: Uuid,
    pub session: Uuid,
    root: PathBuf,
    source: GroupScope,
}
impl Fixture {
    pub fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while matches!(self.favorites.group_status(), GroupStatus::Loading | GroupStatus::Saving) {
            self.favorites.list(&self.inventory);
            assert!(Instant::now() < deadline, "Favorite metadata owner did not answer");
            std::thread::yield_now();
        }
    }
    pub fn ticket(&self, item: u128) -> GroupEditTicket {
        self.favorites
            .group_ticket(self.account, self.session, &self.inventory, Uuid::from_u128(item))
            .unwrap()
    }
    pub fn save(&mut self, item: u128, name: &str) {
        let ticket = self.ticket(item);
        self.favorites
            .assign_group(ticket, self.account, self.session, &self.inventory, name)
            .unwrap();
        self.settle();
        assert_eq!(self.favorites.group_status(), GroupStatus::Ready);
    }
    pub fn name(&self, item: u128) -> Option<&str> {
        self.favorites
            .group_name(self.account, self.session, &self.inventory, Uuid::from_u128(item))
    }
    pub fn replace_source(&mut self, session: Uuid) {
        self.session = session;
        self.source.session = session;
        self.favorites.bind_groups(Some(self.source), self.root.clone());
        self.settle();
    }
    fn path(&self) -> PathBuf {
        self.source.path(&self.root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let alive = self
            .favorites
            .groups
            .as_ref()
            .and_then(|groups| groups.owner.as_ref())
            .map(|owner| owner.alive.clone());
        self.favorites = Favorites::default();
        if let Some(alive) = alive {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Arc::strong_count(&alive) > 1 && Instant::now() < deadline {
                std::thread::yield_now();
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
pub(crate) fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!("aurora-favorite-groups-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let source = scope("https://grid.example.invalid/login");
    // Same admission/owner/poll path, isolated capacity so the ordinary
    // parallel test suite cannot make unrelated fixture loads starve.
    let mut favorites = Favorites {
        group_workers: WorkerPool::isolated(MAX_WORKERS),
        ..Default::default()
    };
    favorites.capture_login_grid(&request("https://grid.example.invalid/login"));
    let grid = favorites.take_login_grid().unwrap();
    assert!(grid == source.grid);
    favorites.bind_groups(GroupScope::new(source.account, source.session, grid), root.clone());
    let mut f = Fixture {
        favorites,
        inventory: inventory(source.account),
        account: source.account,
        session: source.session,
        root,
        source,
    };
    f.settle();
    assert_eq!(f.favorites.group_status(), GroupStatus::Ready);
    f
}

#[test]
fn favorite_group_actual_request_digest_is_consumed_once_and_later_selection_cannot_redirect_it() {
    let mut pending = PendingLoginGrid::default();
    let mut actual = request("https://one.example.invalid/login");
    pending.submitted(&actual);
    actual.login_uri = "https://later-selection.example.invalid/login".into();
    assert!(pending.completed() == LoginGrid::from_uri("https://one.example.invalid/login"));
    assert!(pending.completed().is_none());
    assert!(
        GroupScope::new(
            Uuid::nil(),
            Uuid::from_u128(2),
            LoginGrid::from_uri("https://one.example.invalid").unwrap()
        )
        .is_none()
    );
}
#[test]
fn favorite_group_ignored_later_request_stays_ambiguous_and_failure_retry_captures_its_own_grid() {
    let mut pending = PendingLoginGrid::default();
    // Actual typed command/request values, no fake login server or claim of
    // native authentication. The conservative local proof cannot know whether
    // a later Login is serialized or ignored by the existing session owner.
    for command in [
        aurora_net::NetCommand::Login(Box::new(request("https://one.example.invalid/login"))),
        aurora_net::NetCommand::Login(Box::new(request("https://two.example.invalid/login"))),
    ] {
        if let aurora_net::NetCommand::Login(request) = command {
            pending.submitted(&request);
        }
    }
    pending.submitted(&request("https://one.example.invalid/login"));
    assert!(pending.completed().is_none(), "a later request never clears ambiguity");
    pending.submitted(&request("https://failed.example.invalid/login"));
    pending.failed(); // Production LoginFailed does this BEFORE early retry.
    pending.submitted(&request("https://retry.example.invalid/login"));
    assert!(pending.completed() == LoginGrid::from_uri("https://retry.example.invalid/login"));
}
#[test]
fn favorite_group_missing_and_ambiguous_login_receipts_never_start_metadata_io() {
    let mut favorites = Favorites::default();
    assert!(favorites.take_login_grid().is_none());
    favorites.bind_groups(None, PathBuf::from("unused-synthetic-directory"));
    assert!(favorites.groups.is_none());
    assert_eq!(favorites.group_status(), GroupStatus::Unavailable);
}
#[test]
fn favorite_group_missing_defaults_do_not_write_and_roundtrip_keeps_old_order_bytes() {
    let mut f = fixture();
    assert!(!f.path().exists());
    assert!(!f.path().with_extension("lock").exists());
    let order = f.root.join(super::super::FILE);
    std::fs::write(&order, "synthetic old order bytes").unwrap();
    f.save(10, "  Outings  ");
    assert_eq!(f.name(10), Some("Outings"));
    let stored = read(&f.path(), f.source).unwrap();
    assert_eq!(stored.publication, 1);
    assert_eq!(stored.rows.get(&Uuid::from_u128(10)).map(String::as_str), Some("Outings"));
    assert_eq!(std::fs::read_to_string(order).unwrap(), "synthetic old order bytes");
    f.favorites.bind_groups(Some(f.source), f.root.clone());
    f.settle();
    assert_eq!(f.name(10), Some("Outings"));
    f.save(10, "  ");
    assert!(f.name(10).is_none());
    assert_eq!(
        f.favorites.list(&f.inventory),
        [Uuid::from_u128(10), Uuid::from_u128(11), Uuid::from_u128(12)]
    );
}
#[test]
fn favorite_group_name_input_preserves_utf8_bounds_and_saved_document_is_strict() {
    let name = "界".repeat(40);
    let canonical = input_name(&format!("  {name}  ")).unwrap().unwrap();
    assert_eq!(canonical.len(), 63);
    assert_eq!(canonical, "界".repeat(21));
    assert!(input_name("a\nb").is_err());
    assert!(input_name("\0").is_err());
    let mut f = fixture();
    f.save(10, &name);
    assert_eq!(f.name(10).unwrap().len(), 63);
    let duplicate = format!(
        r#"{{"version":1,"account":"{}","grid":{},"publication":1,"assignments":{{"{}":"a","{}":"b"}}}}"#,
        f.account,
        serde_json::to_string(&f.source.grid.0).unwrap(),
        Uuid::from_u128(10),
        Uuid::from_u128(10)
    );
    assert!(serde_json::from_str::<Document>(&duplicate).is_err());
}
#[test]
fn favorite_group_edit_revalidates_actual_source_item_membership_and_previous_assignment() {
    let mut f = fixture();
    f.save(10, "First");
    let original = f.ticket(10);
    for field in 0..7 {
        let mut invalid = original.clone();
        match field {
            0 => invalid.scope.account = Uuid::from_u128(999),
            1 => invalid.scope.session = Uuid::from_u128(999),
            2 => invalid.scope.grid = LoginGrid::from_uri("https://other.example.invalid").unwrap(),
            3 => invalid.instance += 1,
            4 => invalid.revision += 1,
            5 => invalid.item = Uuid::from_u128(999),
            6 => invalid.previous = Some("Changed".into()),
            _ => unreachable!(),
        }
        assert_eq!(
            f.favorites.assign_group(invalid, f.account, f.session, &f.inventory, "Wrong"),
            Err(GroupError::Stale)
        );
        assert_eq!(f.name(10), Some("First"));
    }
    f.inventory.items.get_mut(&Uuid::from_u128(10)).unwrap().parent = f.inventory.root;
    assert_eq!(
        f.favorites
            .assign_group(original.clone(), f.account, f.session, &f.inventory, "Wrong"),
        Err(GroupError::Stale)
    );
    f.inventory.items.get_mut(&Uuid::from_u128(10)).unwrap().parent = Uuid::from_u128(2);
    f.inventory.items.get_mut(&Uuid::from_u128(10)).unwrap().owner = Uuid::from_u128(999);
    assert_eq!(
        f.favorites.assign_group(original, f.account, f.session, &f.inventory, "Wrong"),
        Err(GroupError::Stale)
    );
}
#[test]
fn favorite_group_reorder_and_duplicate_labels_cannot_retarget_a_captured_item() {
    let mut f = fixture();
    let ticket = f.ticket(10);
    f.inventory.items.get_mut(&Uuid::from_u128(11)).unwrap().name = "First landmark".into();
    f.inventory.generation += 1;
    f.favorites.rearrange(&f.inventory, Uuid::from_u128(10), 99);
    f.favorites
        .assign_group(ticket, f.account, f.session, &f.inventory, "Captured")
        .unwrap();
    f.settle();
    assert_eq!(f.name(10), Some("Captured"));
    assert_eq!(f.name(11), None);
    assert_eq!(f.favorites.list(&f.inventory).last(), Some(&Uuid::from_u128(10)));
}
#[test]
fn favorite_group_old_editor_cannot_reapply_after_failed_save_or_new_model() {
    let mut f = fixture();
    let ticket = f.ticket(10);
    let lease = write_lock(&f.path()).unwrap();
    f.favorites
        .assign_group(ticket.clone(), f.account, f.session, &f.inventory, "Busy")
        .unwrap();
    f.settle();
    assert_eq!(f.favorites.group_status(), GroupStatus::SaveFailed);
    assert!(f.name(10).is_none() && !f.path().exists());
    assert_eq!(
        f.favorites
            .assign_group(ticket.clone(), f.account, f.session, &f.inventory, "Replay"),
        Err(GroupError::Stale)
    );
    drop(lease);
    f.save(10, "Fresh");
    f.favorites.bind_groups(Some(f.source), f.root.clone());
    f.settle();
    assert_eq!(
        f.favorites.assign_group(ticket, f.account, f.session, &f.inventory, "Old model"),
        Err(GroupError::Stale)
    );
    assert_eq!(f.name(10), Some("Fresh"));
}
#[test]
fn favorite_group_os_lock_is_nonblocking_and_foreign_publication_cas_preserves_newer_file() {
    let mut f = fixture();
    f.save(10, "A");
    let mut other = Favorites {
        group_workers: f.favorites.group_workers.clone(),
        ..Default::default()
    };
    other.bind_groups(Some(f.source), f.root.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    while other.group_status() == GroupStatus::Loading {
        other.list(&f.inventory);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let old = other.group_ticket(f.account, f.session, &f.inventory, Uuid::from_u128(10)).unwrap();
    f.save(10, "B");
    f.save(10, "A"); // Same CONTENT again; publication revision still changed.
    let bytes = std::fs::read(f.path()).unwrap();
    other.assign_group(old, f.account, f.session, &f.inventory, "Stale writer").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while other.group_status() == GroupStatus::Saving {
        other.list(&f.inventory);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(other.group_status(), GroupStatus::SaveFailed);
    assert_eq!(std::fs::read(f.path()).unwrap(), bytes);
    let lease = write_lock(&f.path()).unwrap();
    assert!(matches!(write_lock(&f.path()), Err(error) if error.kind() == io::ErrorKind::WouldBlock));
    drop(lease);
}
#[test]
fn favorite_group_account_grid_profile_namespaces_are_separate_and_old_owner_is_canceled_before_new_load() {
    let mut f = fixture();
    f.save(10, "Original");
    let other_grid = scope("https://another-grid.example.invalid/login");
    assert_ne!(f.source.path(&f.root), other_grid.path(&f.root));
    assert!(read(&other_grid.path(&f.root), other_grid).unwrap().rows.is_empty());
    assert!(
        read(&f.source.path(&f.root.join("another-profile")), f.source)
            .unwrap()
            .rows
            .is_empty()
    );
    let lease = IO_OWNER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let ticket = f.ticket(10);
    f.favorites
        .assign_group(ticket, f.account, f.session, &f.inventory, "Canceled old owner")
        .unwrap();
    f.favorites.bind_groups(Some(f.source), f.root.clone());
    drop(lease);
    f.settle();
    assert_eq!(f.name(10), Some("Original"));
    assert_eq!(read(&f.path(), f.source).unwrap().publication, 1);
}
#[test]
fn favorite_group_malformed_newer_wrong_scope_and_oversize_loads_never_rewrite() {
    let mut f = fixture();
    let path = f.path();
    let good = serde_json::to_value(Document {
        version: VERSION,
        account: f.account,
        grid: f.source.grid.0,
        publication: 1,
        assignments: Assignments::new(),
    })
    .unwrap();
    for case in 0..8 {
        let mut doc = good.clone();
        match case {
            0 => doc["version"] = 99.into(),
            1 => doc["account"] = Uuid::from_u128(999).to_string().into(),
            2 => doc["publication"] = 0.into(),
            3 => doc["unknown"] = true.into(),
            6 => doc["grid"] = serde_json::to_value([0u8; 32]).unwrap(),
            7 => doc["assignments"] = serde_json::to_value(Assignments::from([(Uuid::from_u128(10), " malformed ".into())])).unwrap(),
            _ => {}
        }
        let bytes = match case {
            4 => b"malformed".to_vec(),
            5 => vec![b' '; MAX_BYTES + 1],
            _ => serde_json::to_vec(&doc).unwrap(),
        };
        std::fs::write(&path, &bytes).unwrap();
        f.favorites.bind_groups(Some(f.source), f.root.clone());
        f.settle();
        assert_eq!(f.favorites.group_status(), GroupStatus::LoadFailed);
        assert!(
            f.favorites
                .group_ticket(f.account, f.session, &f.inventory, Uuid::from_u128(10))
                .is_none()
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
#[cfg(unix)]
#[test]
fn favorite_group_symlink_data_or_lock_is_not_followed_or_replaced() {
    use std::os::unix::fs::symlink;
    let f = fixture();
    let target = f.root.join("synthetic-other-file");
    std::fs::write(&target, "unchanged").unwrap();
    symlink(&target, f.path()).unwrap();
    assert!(read(&f.path(), f.source).is_err());
    assert!(publish(&f.path(), f.source, &Assignments::new(), 1, &AtomicBool::new(true)).is_err());
    assert!(std::fs::symlink_metadata(f.path()).unwrap().file_type().is_symlink());
    std::fs::remove_file(f.path()).unwrap();
    symlink(&target, f.path().with_extension("lock")).unwrap();
    assert!(write_lock(&f.path()).is_err());
    assert_eq!(std::fs::read_to_string(target).unwrap(), "unchanged");
}
#[test]
fn favorite_group_canceled_atomic_publication_and_assignment_capacity_keep_published_state() {
    let mut f = fixture();
    f.save(10, "Published");
    let before = std::fs::read(f.path()).unwrap();
    assert!(!publish(&f.path(), f.source, &Assignments::new(), 2, &AtomicBool::new(false)).unwrap());
    assert_eq!(std::fs::read(f.path()).unwrap(), before);
    let groups = f.favorites.groups.as_mut().unwrap();
    groups.rows = (0..MAX_ASSIGNMENTS)
        .map(|n| (Uuid::from_u128(1000 + n as u128), "Name".into()))
        .collect();
    let ticket = groups.ticket(f.account, f.session, &f.inventory, Uuid::from_u128(10)).unwrap();
    assert_eq!(
        groups.assign(ticket, f.account, f.session, &f.inventory, "Overflow"),
        Err(GroupError::Capacity)
    );
    assert_eq!(std::fs::read(f.path()).unwrap(), before);
}

#[test]
fn favorite_group_publication_exhaustion_is_readonly_without_rewriting_existing_bytes() {
    let mut f = fixture();
    let bytes = serde_json::to_vec(&Document {
        version: VERSION,
        account: f.account,
        grid: f.source.grid.0,
        publication: u64::MAX,
        assignments: Assignments::new(),
    })
    .unwrap();
    std::fs::write(f.path(), &bytes).unwrap();
    f.favorites.bind_groups(Some(f.source), f.root.clone());
    f.settle();
    let ticket = f.ticket(10);
    assert_eq!(
        f.favorites.assign_group(ticket, f.account, f.session, &f.inventory, "Never wrap"),
        Err(GroupError::Exhausted)
    );
    assert_eq!(f.favorites.group_status(), GroupStatus::LoadFailed);
    assert!(
        f.favorites
            .group_ticket(f.account, f.session, &f.inventory, Uuid::from_u128(10))
            .is_none()
    );
    assert_eq!(std::fs::read(f.path()).unwrap(), bytes);
}

#[test]
fn favorite_group_one_pending_save_rejects_second_edit_and_consumes_old_tickets() {
    let mut f = fixture();
    let first = f.ticket(10);
    let second = f.ticket(11);
    // Hold the actual IO owner, so the queued request cannot complete before
    // the second GUI-side admission. GUI methods never acquire this mutex.
    let owner = IO_OWNER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f.favorites
        .assign_group(first, f.account, f.session, &f.inventory, "First")
        .unwrap();
    assert_eq!(f.favorites.group_status(), GroupStatus::Saving);
    assert!(f.name(10).is_none(), "pending names are not published optimistically");
    assert_eq!(
        f.favorites
            .assign_group(second.clone(), f.account, f.session, &f.inventory, "Second"),
        Err(GroupError::Busy)
    );
    drop(owner);
    f.settle();
    assert_eq!(f.name(10), Some("First"));
    assert_eq!(f.name(11), None);
    assert_eq!(
        f.favorites.assign_group(second, f.account, f.session, &f.inventory, "Old editor"),
        Err(GroupError::Stale)
    );
    f.save(11, "Fresh editor");
    assert_eq!(f.name(11), Some("Fresh editor"));
}

#[test]
fn favorite_group_default_replacement_models_share_the_process_worker_pool() {
    let original = Favorites::default();
    assert_eq!(original.group_workers.0.limit, 2);
    for _ in 0..256 {
        let replacement = Favorites::default();
        assert!(Arc::ptr_eq(&original.group_workers.0, &replacement.group_workers.0));
        assert!(replacement.groups.is_none());
    }
}

#[test]
fn favorite_group_blocked_worker_resets_bound_real_starts_until_actual_exit_then_retry_current_source() {
    let mut f = fixture();
    f.save(10, "Published original source");
    let original_path = f.path();
    let original_bytes = std::fs::read(&original_path).unwrap();
    let pool = f.favorites.group_workers.clone();
    assert_eq!(pool.0.starts.load(Ordering::Acquire), 1);
    let first_alive = f.favorites.groups.as_ref().unwrap().owner.as_ref().unwrap().alive.clone();
    let io = IO_OWNER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let ticket = f.ticket(10);
    f.favorites
        .assign_group(ticket, f.account, f.session, &f.inventory, "Never publish canceled work")
        .unwrap();
    assert_eq!(f.name(10), Some("Published original source"));
    let mut next = scope("https://replacement.example.invalid/login");
    f.favorites.bind_groups(Some(next), f.root.clone());
    let second_alive = f.favorites.groups.as_ref().unwrap().owner.as_ref().unwrap().alive.clone();
    // Both admitted OS workers are unable to finish while IO_OWNER is held.
    // Logical cancel/source replacement must not refund either actual slot.
    assert_eq!(pool.0.used.load(Ordering::Acquire), 2);
    for session in 1000..1512 {
        next.session = Uuid::from_u128(session);
        if session % 2 == 0 {
            f.favorites = Favorites {
                group_workers: pool.clone(),
                ..Default::default()
            };
        }
        f.favorites.bind_groups(Some(next), f.root.clone());
        f.favorites.list(&f.inventory); // Production try-only retry while busy.
        let current = f.favorites.groups.as_ref().unwrap();
        assert_eq!(current.scope, next);
        assert!(current.owner.is_none() && current.rows.is_empty() && current.pending.is_none());
        assert_eq!(f.favorites.group_status(), GroupStatus::Loading);
        assert!(
            f.favorites
                .group_name(f.account, next.session, &f.inventory, Uuid::from_u128(10))
                .is_none()
        );
        assert!(
            f.favorites
                .group_ticket(f.account, next.session, &f.inventory, Uuid::from_u128(10))
                .is_none()
        );
        assert_eq!(pool.0.starts.load(Ordering::Acquire), 2);
        assert_eq!(pool.0.used.load(Ordering::Acquire), 2);
        assert_eq!(pool.0.peak.load(Ordering::Acquire), 2);
    }
    assert!(!first_alive.load(Ordering::Acquire) && !second_alive.load(Ordering::Acquire));
    drop(io);
    let deadline = Instant::now() + Duration::from_secs(5);
    while pool.0.used.load(Ordering::Acquire) != 0 {
        assert!(Instant::now() < deadline, "canceled OS workers did not exit");
        std::thread::yield_now();
    }
    f.source = next;
    f.session = next.session;
    f.settle(); // Current pending source alone may now acquire/retry once.
    assert_eq!(f.favorites.group_status(), GroupStatus::Ready);
    assert_eq!(pool.0.starts.load(Ordering::Acquire), 3);
    assert_eq!(pool.0.used.load(Ordering::Acquire), 1);
    assert_eq!(pool.0.peak.load(Ordering::Acquire), 2);
    assert!(f.name(10).is_none());
    assert!(
        f.favorites
            .group_ticket(f.account, f.session, &f.inventory, Uuid::from_u128(10))
            .is_some()
    );
    assert!(!f.path().exists());
    assert_eq!(std::fs::read(original_path).unwrap(), original_bytes);
    f.favorites.bind_groups(None, f.root.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    while pool.0.used.load(Ordering::Acquire) != 0 {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(f.favorites.group_status(), GroupStatus::Unavailable);
}

#[test]
fn favorite_group_worker_admission_rolls_back_unstarted_closure_and_unwind() {
    let pool = WorkerPool::isolated(1);
    let permit = pool.admit().unwrap();
    assert!(pool.admit().is_none());
    // A spawn failure drops the same owned closure without running it.
    let unstarted = move || drop(permit);
    drop(unstarted);
    assert_eq!(pool.0.used.load(Ordering::Acquire), 0);
    let permit = pool.admit().unwrap();
    assert!(
        std::panic::catch_unwind(move || {
            let _permit = permit;
            panic!("synthetic worker unwind");
        })
        .is_err()
    );
    assert_eq!(pool.0.used.load(Ordering::Acquire), 0);
    assert!(pool.admit().is_some());
}
