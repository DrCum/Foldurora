//! Bounded account/profile/grid-local Favorite menu names. These are display
//! metadata only: no Inventory mutation, login authority or simulator request.
//! Behavior port of LLFavoritesBarCtrl::rebuildFolderEntries / doToSelected
//! (indra/newview/llfavoritesbar.cpp, FSLandmarkFavoriteFolders, LGPL 2.1).
use super::{Inventory, LoginGrid};
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use uuid::Uuid;

pub const MAX_ASSIGNMENTS: usize = 1024;
pub const MAX_BYTES: usize = 128 * 1024;
const VERSION: u32 = 1;
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);
// IO threads only. A new owner's load cannot overtake a prior owner's atomic
// publication. Revocation skips old work that has not entered publication yet.
static IO_OWNER: Mutex<()> = Mutex::new(());

// Bound real worker lifetimes, including canceled workers stuck in OS IO or
// waiting for IO_OWNER. All ordinary Favorites instances share this pool, so
// replacing the entire App/World model cannot mint fresh worker capacity.
const MAX_WORKERS: usize = 2;
static WORKERS: OnceLock<WorkerPool> = OnceLock::new();
#[derive(Clone)]
pub(super) struct WorkerPool(Arc<WorkerSlots>);
struct WorkerSlots {
    limit: usize,
    used: AtomicUsize,
    #[cfg(test)]
    starts: AtomicUsize,
    #[cfg(test)]
    peak: AtomicUsize,
}
impl Default for WorkerPool {
    fn default() -> Self {
        WORKERS
            .get_or_init(|| {
                Self(Arc::new(WorkerSlots {
                    limit: MAX_WORKERS,
                    used: AtomicUsize::new(0),
                    #[cfg(test)]
                    starts: AtomicUsize::new(0),
                    #[cfg(test)]
                    peak: AtomicUsize::new(0),
                }))
            })
            .clone()
    }
}
impl WorkerPool {
    #[cfg(test)]
    fn isolated(limit: usize) -> Self {
        Self(Arc::new(WorkerSlots {
            limit,
            used: AtomicUsize::new(0),
            #[cfg(test)]
            starts: AtomicUsize::new(0),
            #[cfg(test)]
            peak: AtomicUsize::new(0),
        }))
    }
    fn admit(&self) -> Option<WorkerPermit> {
        let used = self
            .0
            .used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| (used < self.0.limit).then(|| used + 1))
            .ok()?;
        #[cfg(test)]
        self.0.peak.fetch_max(used + 1, Ordering::Relaxed);
        #[cfg(not(test))]
        let _ = used;
        Some(WorkerPermit(self.0.clone()))
    }
}
struct WorkerPermit(Arc<WorkerSlots>);
impl Drop for WorkerPermit {
    fn drop(&mut self) {
        self.0.used.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GroupScope {
    account: Uuid,
    session: Uuid,
    grid: LoginGrid,
}
impl GroupScope {
    pub fn new(account: Uuid, session: Uuid, grid: LoginGrid) -> Option<Self> {
        (!account.is_nil() && !session.is_nil()).then_some(Self { account, session, grid })
    }
    fn current(self, account: Uuid, session: Uuid, inventory: &Inventory) -> bool {
        self.account == account && self.session == session && inventory.owner == account
    }
    fn path(self, directory: &Path) -> PathBuf {
        let hex = self.grid.namespace();
        directory.join(format!("favorites_groups_{hex}.json"))
    }
}
impl std::fmt::Debug for GroupScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GroupScope([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct GroupEditTicket {
    instance: u64,
    revision: u64,
    scope: GroupScope,
    item: Uuid,
    previous: Option<String>,
}
impl std::fmt::Debug for GroupEditTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GroupEditTicket([redacted])")
    }
}
impl GroupEditTicket {
    pub fn previous(&self) -> &str {
        self.previous.as_deref().unwrap_or("")
    }
    pub fn namespace(&self) -> (u64, u64, Uuid) {
        (self.instance, self.revision, self.item)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GroupStatus {
    #[default]
    Unavailable,
    Loading,
    Ready,
    Saving,
    LoadFailed,
    SaveFailed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupError {
    Unavailable,
    Busy,
    Stale,
    Invalid,
    Capacity,
    Exhausted,
}

type Assignments = BTreeMap<Uuid, String>;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    account: Uuid,
    grid: [u8; 32],
    publication: u64,
    #[serde(deserialize_with = "read_assignments")]
    assignments: Assignments,
}
fn canonical(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.trim() == name && !name.chars().any(char::is_control)
}
fn input_name(name: &str) -> Result<Option<String>, GroupError> {
    let name = name.trim();
    if name.chars().any(char::is_control) {
        return Err(GroupError::Invalid);
    }
    let mut end = name.len().min(64);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    let name = name[..end].trim_end();
    Ok((!name.is_empty()).then(|| name.to_owned()))
}
fn read_assignments<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Assignments, D::Error> {
    struct Rows;
    impl<'de> serde::de::Visitor<'de> for Rows {
        type Value = Assignments;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("bounded unique Favorite names")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Assignments, M::Error> {
            let mut rows = Assignments::new();
            while let Some((item, name)) = map.next_entry::<Uuid, String>()? {
                if rows.len() >= MAX_ASSIGNMENTS || item.is_nil() || !canonical(&name) || rows.insert(item, name).is_some() {
                    return Err(serde::de::Error::custom("invalid Favorite names"));
                }
            }
            Ok(rows)
        }
    }
    deserializer.deserialize_map(Rows)
}
/// Follow only this account's actual Inventory ancestry; never a row label or
/// a cached rank. Bounded against malformed cycles, without an O(n) list scan
/// for every view ticket.
pub(super) fn current_favorite(inventory: &Inventory, item: Uuid) -> bool {
    let Some(item) = inventory
        .items
        .get(&item)
        .filter(|item| item.asset_type == super::ASSET_LANDMARK && item.owner == inventory.owner && !item.owner.is_nil())
    else {
        return false;
    };
    if inventory.root.is_nil() {
        return false;
    }
    let mut at = item.parent;
    let mut favorite = false;
    let mut seen = std::collections::HashSet::new();
    while !at.is_nil() && seen.len() < 4096 && seen.insert(at) {
        if at == inventory.root {
            return favorite;
        }
        let Some(folder) = inventory.folders.get(&at).filter(|folder| !folder.library) else {
            return false;
        };
        favorite |= folder.info.type_default == super::FOLDER_FAVORITES;
        at = folder.info.parent;
    }
    false
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Favorite names unavailable")
}
fn validate_leaf(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES as u64 {
                return Err(invalid());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err(invalid());
                } // reparse point
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
struct Snapshot {
    publication: u64,
    rows: Assignments,
}
fn read(path: &Path, scope: GroupScope) -> io::Result<Snapshot> {
    validate_leaf(path)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    } // OPEN_REPARSE_POINT
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Snapshot {
                publication: 0,
                rows: Assignments::new(),
            });
        }
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        return Err(invalid());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(invalid());
        }
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid());
    }
    let doc: Document = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if doc.version != VERSION || doc.account != scope.account || doc.grid != scope.grid.bytes() || doc.publication == 0 {
        return Err(invalid());
    }
    Ok(Snapshot {
        publication: doc.publication,
        rows: doc.assignments,
    })
}

/// Stable lock inode, independent of atomic replacement of the data file.
/// Only IO threads call this. Contention is an outcome, never a GUI wait.
fn write_lock(path: &Path) -> io::Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock_path = path.with_extension("lock");
    validate_leaf(&lock_path)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let file = options.open(&lock_path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() != 0 {
        return Err(invalid());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(invalid());
        }
    }
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(io::Error::new(io::ErrorKind::WouldBlock, "Favorite names busy")),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}
fn publish(path: &Path, scope: GroupScope, rows: &Assignments, publication: u64, alive: &AtomicBool) -> io::Result<bool> {
    if publication == 0 || rows.len() > MAX_ASSIGNMENTS || rows.iter().any(|(id, name)| id.is_nil() || !canonical(name)) {
        return Err(invalid());
    }
    validate_leaf(path)?;
    let bytes = serde_json::to_vec(&Document {
        version: VERSION,
        account: scope.account,
        grid: scope.grid.bytes(),
        publication,
        assignments: rows.clone(),
    })
    .map_err(|_| invalid())?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::atomic_file::write_if(path, &bytes, || alive.load(Ordering::Acquire))
}
struct Save {
    operation: u64,
    expected: Assignments,
    expected_publication: u64,
    publication: u64,
    rows: Assignments,
}
enum Reply {
    Loaded(io::Result<Snapshot>),
    Saved(u64, io::Result<bool>),
}
struct Owner {
    requests: Sender<Save>,
    replies: Receiver<Reply>,
    alive: Arc<AtomicBool>,
}
impl Owner {
    fn start(path: &Path, scope: GroupScope, workers: &WorkerPool) -> io::Result<Option<Self>> {
        // Admission precedes even channel allocation. Logical cancellation
        // never refunds it; only the actual worker's RAII exit does.
        let Some(permit) = workers.admit() else { return Ok(None) };
        let path = path.to_owned();
        let (requests, incoming) = crossbeam_channel::bounded::<Save>(1);
        let (outgoing, replies) = crossbeam_channel::bounded(1);
        let alive = Arc::new(AtomicBool::new(true));
        let worker_alive = alive.clone();
        #[cfg(test)]
        workers.0.starts.fetch_add(1, Ordering::Relaxed);
        let run = move || {
            let loaded = {
                let _owner = IO_OWNER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if !worker_alive.load(Ordering::Acquire) {
                    return;
                }
                read(&path, scope)
            };
            if outgoing.send(Reply::Loaded(loaded)).is_err() {
                return;
            }
            while let Ok(save) = incoming.recv() {
                let result = {
                    let _owner = IO_OWNER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if !worker_alive.load(Ordering::Acquire) {
                        return;
                    }
                    // Read/compare/replace under a nonblocking OS file lock,
                    // shared by cooperating standalone/profile processes.
                    (|| {
                        let _lease = write_lock(&path)?;
                        let current = read(&path, scope)?;
                        if current.publication != save.expected_publication || current.rows != save.expected {
                            return Err(invalid());
                        }
                        publish(&path, scope, &save.rows, save.publication, &worker_alive)
                    })()
                };
                if outgoing.send(Reply::Saved(save.operation, result)).is_err() {
                    return;
                }
            }
            // Make this an owning FnOnce: its captured channels and IO state
            // retire before the surrounding worker permit on every exit.
            drop((path, incoming, outgoing, worker_alive));
        };
        std::thread::Builder::new().name("aurora-favorite-names".into()).spawn(move || {
            let _permit = permit;
            // FnOnce owns the worker's channels/path/IO state; those drop
            // before the outer permit, including an unwinding IO failure.
            run();
        })?;
        Ok(Some(Self { requests, replies, alive }))
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    } // No GUI join/wait.
}
struct Pending {
    operation: u64,
    publication: u64,
    rows: Assignments,
}
pub(super) struct Groups {
    instance: u64,
    scope: GroupScope,
    path: PathBuf,
    workers: WorkerPool,
    owner: Option<Owner>,
    rows: Assignments,
    publication: u64,
    pending: Option<Pending>,
    next: u64,
    status: GroupStatus,
}
impl Groups {
    pub fn open(scope: GroupScope, directory: PathBuf, workers: WorkerPool) -> Result<Self, GroupError> {
        let instance = NEXT_INSTANCE
            .try_update(Ordering::AcqRel, Ordering::Acquire, |next| next.checked_add(1))
            .map_err(|_| GroupError::Exhausted)?;
        let path = scope.path(&directory);
        let owner = Owner::start(&path, scope, &workers).map_err(|_| GroupError::Unavailable)?;
        Ok(Self {
            instance,
            scope,
            path,
            workers,
            owner,
            rows: Assignments::new(),
            publication: 0,
            pending: None,
            next: 1,
            status: GroupStatus::Loading,
        })
    }
    pub fn poll(&mut self) {
        if self.owner.is_none() && self.status == GroupStatus::Loading {
            match Owner::start(&self.path, self.scope, &self.workers) {
                Ok(owner) => self.owner = owner,
                Err(_) => self.status = GroupStatus::LoadFailed,
            }
        }
        let Some(owner) = &self.owner else { return };
        match owner.replies.try_recv() {
            Ok(Reply::Loaded(Ok(rows))) if self.status == GroupStatus::Loading => {
                self.publication = rows.publication;
                self.rows = rows.rows;
                self.status = GroupStatus::Ready;
            }
            Ok(Reply::Loaded(Err(_))) => {
                self.status = GroupStatus::LoadFailed;
                self.owner = None;
            }
            Ok(Reply::Saved(operation, result)) => {
                let Some(pending) = self.pending.take() else { return };
                if pending.operation != operation {
                    self.status = GroupStatus::LoadFailed;
                    self.owner = None;
                    return;
                }
                match result {
                    Ok(true) => {
                        self.publication = pending.publication;
                        self.rows = pending.rows;
                        self.status = GroupStatus::Ready;
                    }
                    _ => self.status = GroupStatus::SaveFailed,
                }
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.status = GroupStatus::LoadFailed;
                self.owner = None;
                self.pending = None;
            }
            _ => {}
        }
    }
    pub fn status(&self) -> GroupStatus {
        self.status
    }
    pub fn name_for(&self, account: Uuid, session: Uuid, inventory: &Inventory, item: Uuid) -> Option<&str> {
        if !self.scope.current(account, session, inventory) || !current_favorite(inventory, item) {
            return None;
        }
        self.rows.get(&item).map(String::as_str)
    }
    pub fn ticket(&self, account: Uuid, session: Uuid, inventory: &Inventory, item: Uuid) -> Option<GroupEditTicket> {
        if !matches!(self.status, GroupStatus::Ready | GroupStatus::SaveFailed)
            || !self.scope.current(account, session, inventory)
            || !current_favorite(inventory, item)
        {
            return None;
        }
        Some(GroupEditTicket {
            instance: self.instance,
            revision: self.next,
            scope: self.scope,
            item,
            previous: self.rows.get(&item).cloned(),
        })
    }
    pub fn assign(
        &mut self,
        ticket: GroupEditTicket,
        account: Uuid,
        session: Uuid,
        inventory: &Inventory,
        name: &str,
    ) -> Result<(), GroupError> {
        if self.pending.is_some() {
            return Err(GroupError::Busy);
        }
        if self.ticket(account, session, inventory, ticket.item).as_ref() != Some(&ticket) {
            return Err(GroupError::Stale);
        }
        let name = input_name(name)?;
        let mut rows = self.rows.clone();
        if let Some(name) = name {
            rows.insert(ticket.item, name);
        } else {
            rows.remove(&ticket.item);
        }
        if rows.len() > MAX_ASSIGNMENTS {
            return Err(GroupError::Capacity);
        }
        let Some(next) = self.next.checked_add(1) else {
            self.status = GroupStatus::LoadFailed;
            self.owner = None;
            return Err(GroupError::Exhausted);
        };
        let Some(publication) = self.publication.checked_add(1) else {
            self.status = GroupStatus::LoadFailed;
            self.owner = None;
            return Err(GroupError::Exhausted);
        };
        let operation = self.next;
        let Some(owner) = &self.owner else {
            return Err(GroupError::Unavailable);
        };
        // Even rejected IO admission consumes this editor incarnation. A
        // retry must capture the current source/base in a freshly opened menu.
        self.next = next;
        match owner.requests.try_send(Save {
            operation,
            expected: self.rows.clone(),
            expected_publication: self.publication,
            publication,
            rows: rows.clone(),
        }) {
            Ok(()) => {}
            Err(crossbeam_channel::TrySendError::Full(_)) => {
                self.status = GroupStatus::SaveFailed;
                return Err(GroupError::Busy);
            }
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                self.status = GroupStatus::LoadFailed;
                self.owner = None;
                return Err(GroupError::Unavailable);
            }
        }
        self.pending = Some(Pending {
            operation,
            publication,
            rows,
        });
        self.status = GroupStatus::Saving;
        Ok(())
    }
}

#[cfg(test)]
#[path = "groups_tests.rs"]
pub(crate) mod tests;
