//! Native character windows: one isolated world/network owner per process.
//!
//! Discovery is scoped to this executable, and exposes only an opaque saved
//! account/grid identity. The worker owns all file IO and process launches.
//! A focus request names an exact process nonce and session generation, never
//! a PID alone. It cannot log in, send input or issue a simulator command.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, VecDeque};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const VERSION: u32 = 1;
const PERIOD: Duration = Duration::from_millis(250);
const PUBLISH_SECS: u64 = 2;
const STALE_SECS: u64 = 8;
const LAUNCH_SECS: u64 = 30;
const REMOVE_SECS: u64 = 120;
const MAX_FILE: u64 = 2048;
const MAX_ACCOUNTS: usize = 64;
const MAX_COMMANDS: usize = 8;

/// Stable within the saved catalogue, with grid and name independently framed.
/// This is not a credential and never authorizes a login.
pub fn account_id(account: &crate::login_accounts::Account) -> String {
    identity(&account.uri, &account.username)
}

pub fn identity(uri: &str, username: &str) -> String {
    let uri = uri.trim();
    let username = username.trim().to_lowercase();
    let mut hash = Sha256::new();
    hash.update((uri.len() as u64).to_le_bytes());
    hash.update(uri.as_bytes());
    hash.update((username.len() as u64).to_le_bytes());
    hash.update(username.as_bytes());
    hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn installation_id(executable: &Path) -> String {
    Sha256::digest(executable.as_os_str().as_encoded_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    #[default]
    Login,
    Connecting,
    Online,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    version: u32,
    pub instance: Uuid,
    pub pid: u32,
    pub account: String,
    pub phase: Phase,
    generation: u64,
    stamp: u64,
}

impl Presence {
    fn current(&self, now: u64) -> bool {
        self.version == VERSION && !self.instance.is_nil() && self.pid > 0 && valid_id(&self.account) && fresh(self.stamp, now, STALE_SECS)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Focus {
    version: u32,
    instance: Uuid,
    account: String,
    generation: u64,
    stamp: u64,
}

impl Focus {
    fn matches(&self, own: &Presence, now: u64) -> bool {
        self.version == VERSION
            && self.instance == own.instance
            && self.account == own.account
            && self.generation == own.generation
            && fresh(self.stamp, now, STALE_SECS)
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Reservation {
    version: u32,
    account: String,
    nonce: Uuid,
    stamp: u64,
}

fn fresh(stamp: u64, now: u64, max_age: u64) -> bool {
    stamp <= now.saturating_add(2) && now.saturating_sub(stamp) <= max_age
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub peers: Vec<Presence>,
    pub opening: BTreeSet<String>,
    pub catalogue: Option<Vec<crate::login_accounts::Account>>,
    pub catalogue_count: usize,
}

pub enum Notice {
    Opening,
    FocusRequested,
    Busy,
    Failed,
    Demo,
    OtherGrid,
}

enum Command {
    Open(String),
    Focus(Presence),
}

#[derive(Default)]
struct State {
    own: Option<Presence>,
    generation: u64,
    watch_open: bool,
    snapshot: Snapshot,
    commands: VecDeque<Command>,
    notice: Option<Notice>,
    focus: Option<Focus>,
    stop: bool,
}

impl State {
    fn follow(&mut self, instance: Uuid, account: Option<String>, phase: Phase) {
        let changed = match (&self.own, &account) {
            (Some(own), Some(account)) => own.account != *account || own.phase != phase,
            (None, None) => false,
            _ => true,
        };
        if changed {
            self.generation = self.generation.saturating_add(1);
            let generation = self.generation;
            self.own = account.map(|account| Presence {
                version: VERSION,
                instance,
                pid: std::process::id(),
                account,
                phase,
                generation,
                stamp: 0,
            });
        }
    }
}

type Shared = Arc<(Mutex<State>, Condvar)>;

fn locked(shared: &Shared) -> std::sync::MutexGuard<'_, State> {
    shared.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub struct Manager {
    instance: Uuid,
    shared: Shared,
    started: bool,
    /// The CLI selection is consumed once, even if the catalogue is refreshed.
    pub startup_applied: bool,
    last_identity: Option<(String, String, String)>,
}

impl Default for Manager {
    fn default() -> Self {
        Self {
            instance: Uuid::new_v4(),
            shared: Arc::new((Mutex::new(State::default()), Condvar::new())),
            started: false,
            startup_applied: false,
            last_identity: None,
        }
    }
}

impl Manager {
    /// Demo never discovers a real viewer or writes a heartbeat.
    pub fn follow(&mut self, enabled: bool, uri: String, username: &str, phase: Phase) {
        if !enabled {
            return;
        }
        if self
            .last_identity
            .as_ref()
            .is_none_or(|(old_uri, old_name, _)| *old_uri != uri || old_name != username)
        {
            let id = identity(&uri, username);
            self.last_identity = Some((uri, username.to_owned(), id));
        }
        let account = if username.trim().is_empty() {
            None
        } else {
            self.last_identity.as_ref().map(|(_, _, id)| id.clone())
        };
        locked(&self.shared).follow(self.instance, account, phase);
        if !std::mem::replace(&mut self.started, true) {
            let shared = self.shared.clone();
            let instance = self.instance;
            let result = std::thread::Builder::new().name("characters".into()).spawn(move || {
                let Ok(executable) = std::env::current_exe().and_then(std::fs::canonicalize) else {
                    locked(&shared).notice = Some(Notice::Failed);
                    return;
                };
                let scope = installation_id(&executable);
                let dir = crate::logging::logs_dir().with_file_name("characters").join(scope);
                if std::fs::create_dir_all(&dir).is_err() {
                    locked(&shared).notice = Some(Notice::Failed);
                    return;
                }
                run(&dir, &executable, instance, &shared);
            });
            if result.is_err() {
                locked(&self.shared).notice = Some(Notice::Failed);
            }
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        locked(&self.shared).snapshot.clone()
    }

    /// Only catalogue identities need discovery. Direct, bounded lookup avoids
    /// an ever-growing directory of lock files hiding a live character.
    pub fn watch(&self, open: bool) {
        locked(&self.shared).watch_open = open;
    }

    pub fn own_instance(&self) -> Uuid {
        self.instance
    }

    pub fn take_focus(&self) -> bool {
        let mut state = locked(&self.shared);
        state
            .focus
            .take()
            .is_some_and(|request| state.own.as_ref().is_some_and(|own| request.matches(own, now())))
    }

    pub fn take_notice(&self) -> Option<Notice> {
        locked(&self.shared).notice.take()
    }

    pub fn open(&self, account: String) {
        if valid_id(&account) {
            self.queue(Command::Open(account));
        }
    }

    pub fn focus(&self, peer: Presence) {
        allow_foreground(peer.pid);
        self.queue(Command::Focus(peer));
    }

    fn queue(&self, command: Command) {
        let mut state = locked(&self.shared);
        if state.commands.len() < MAX_COMMANDS {
            state.commands.push_back(command);
            self.shared.1.notify_one();
        }
    }

    pub fn leave(&self) {
        locked(&self.shared).stop = true;
        self.shared.1.notify_one();
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        self.leave();
    }
}

#[cfg(windows)]
fn allow_foreground(pid: u32) {
    // SAFETY: the user just selected this live viewer; the API only grants
    // foreground activation to that process. The nonce-bound mailbox is what
    // asks the target to focus, so PID reuse cannot select another character.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
    }
}

#[cfg(not(windows))]
fn allow_foreground(_: u32) {}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn record_path(dir: &Path, account: &str) -> PathBuf {
    dir.join(format!("{account}.character"))
}

fn focus_path(dir: &Path, instance: Uuid) -> PathBuf {
    dir.join(format!("{instance}.focus"))
}

fn reservation_path(dir: &Path, account: &str) -> PathBuf {
    dir.join(format!("{account}.pending"))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_FILE {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_FILE {
        None
    } else {
        serde_json::from_slice(&bytes).ok()
    }
}

fn publish(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    crate::atomic_file::publish(path, &bytes)
}

fn scan(dir: &Path, accounts: &[String], stamp: u64) -> Snapshot {
    let mut snapshot = Snapshot::default();
    let mut seen = BTreeSet::new();
    for account in accounts
        .iter()
        .filter(|id| valid_id(id))
        .filter(|id| seen.insert(id.as_str()))
        .take(MAX_ACCOUNTS)
    {
        let path = record_path(dir, account);
        if let Some(peer) = read_json::<Presence>(&path)
            && peer.account == *account
        {
            if peer.current(stamp) {
                snapshot.peers.push(peer);
            } else if !fresh(peer.stamp, stamp, REMOVE_SECS) {
                let _ = std::fs::remove_file(path);
            }
        }
        // Readiness/nonce ownership retires pending files; a new launch
        // replaces them under its lock. Discovery must not remove a paused
        // cold child's pending file just because its lease expires.
        let path = reservation_path(dir, account);
        if let Some(reservation) = read_json::<Reservation>(&path)
            && reservation.version == VERSION
            && reservation.account == *account
            && !reservation.nonce.is_nil()
            && fresh(reservation.stamp, stamp, LAUNCH_SECS)
        {
            snapshot.opening.insert(reservation.account);
        }
    }
    snapshot.peers.sort_by_key(|peer| peer.pid);
    snapshot
}

fn request_focus(dir: &Path, expected: &Presence, stamp: u64) -> io::Result<()> {
    let current = read_json::<Presence>(&record_path(dir, &expected.account))
        .filter(|current| {
            current.current(stamp)
                && current.instance == expected.instance
                && current.pid == expected.pid
                && current.account == expected.account
                && current.generation == expected.generation
        })
        .ok_or_else(|| io::Error::other("character window changed or closed"))?;
    publish(
        &focus_path(dir, current.instance),
        &Focus {
            version: VERSION,
            instance: current.instance,
            account: current.account,
            generation: current.generation,
            stamp,
        },
    )
}

fn focus_existing_character(dir: &Path, account: &str, stamp: u64) -> io::Result<Option<Notice>> {
    if let Some(peer) = read_json::<Presence>(&record_path(dir, account)).filter(|peer| peer.account == account && peer.current(stamp)) {
        allow_foreground(peer.pid);
        request_focus(dir, &peer, stamp)?;
        return Ok(Some(Notice::FocusRequested));
    }
    Ok(None)
}

/// Observes a launch without creating files, admitting a process or changing
/// the owner's reservation. The avatar gate covers every related account.
fn launch_in_progress(dir: &Path, account: &str) -> io::Result<bool> {
    let lock = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.join(format!("{account}.lock")))
    {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    match lock.try_lock() {
        Ok(()) => Ok(false),
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

fn contended_avatar_feedback(dir: &Path, account: &str, stamp: u64) -> io::Result<Notice> {
    if let Some(notice) = focus_existing_character(dir, account, stamp)? {
        return Ok(notice);
    }
    let pending = read_json::<Reservation>(&reservation_path(dir, account)).is_some_and(|reservation| {
        reservation.version == VERSION
            && reservation.account == account
            && !reservation.nonce.is_nil()
            && fresh(reservation.stamp, stamp, LAUNCH_SECS)
    });
    if pending || launch_in_progress(dir, account)? {
        Ok(Notice::Opening)
    } else {
        // The avatar lock may belong to another grid, or its owner has not
        // published yet. Never infer permission to launch this target.
        Ok(Notice::Busy)
    }
}

/// The command line carries no name, password, grid URI, token or SLURL.
fn launch_arguments(account: &str) -> Vec<String> {
    vec![
        "--saved-account".into(),
        account.into(),
        "--title".into(),
        format!("Character {}", &account[..8]),
    ]
}

fn owns_reservation(expected: &Reservation, current: &Reservation) -> bool {
    current.version == VERSION && current.account == expected.account && current.nonce == expected.nonce
}

fn release_reservation(dir: &Path, expected: &Reservation) {
    let path = reservation_path(dir, &expected.account);
    if read_json::<Reservation>(&path).is_some_and(|current| owns_reservation(expected, &current)) {
        let _ = std::fs::remove_file(path);
    }
}

/// The account lock remains held until startup completes. A slow Vulkan or
/// avatar-library initialization must not expire into a second launch. Only
/// this launch nonce can be refreshed or removed by its supervisor.
fn refresh_reservation(dir: &Path, expected: &Reservation, pid: u32, stamp: u64) -> io::Result<bool> {
    let path = reservation_path(dir, &expected.account);
    let Some(mut current) = read_json::<Reservation>(&path).filter(|current| owns_reservation(expected, current)) else {
        return Ok(false);
    };
    if read_json::<Presence>(&record_path(dir, &expected.account)).is_some_and(|peer| peer.current(stamp) && peer.pid == pid) {
        release_reservation(dir, expected);
        return Ok(false);
    }
    current.stamp = stamp;
    publish(&path, &current)?;
    Ok(true)
}

fn supervise_launch(dir: &Path, reservation: &Reservation, lock: std::fs::File, receiver: std::sync::mpsc::Receiver<std::process::Child>) {
    let Ok(mut child) = receiver.recv() else {
        release_reservation(dir, reservation);
        return;
    };
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                release_reservation(dir, reservation);
                return;
            }
            Ok(None) => {}
            Err(_) => break,
        }
        match refresh_reservation(dir, reservation, child.id(), now()) {
            Ok(false) => break,
            Ok(true) | Err(_) => std::thread::sleep(Duration::from_secs(PUBLISH_SECS)),
        }
    }
    drop(lock);
    // Unix needs a wait after the child closes; this owns no renderer, no
    // network session and no build worker while the character remains open.
    let _ = child.wait();
}

fn open_character(dir: &Path, executable: &Path, account: &str, stamp: u64) -> io::Result<Notice> {
    if !valid_id(account) {
        return Err(io::Error::other("invalid saved character"));
    }
    // A reservation precedes process startup. The lock also serializes two
    // different viewers opening the same account before a heartbeat exists.
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(format!("{account}.lock")))?;
    let held = match lock.try_lock() {
        Ok(()) => true,
        Err(std::fs::TryLockError::WouldBlock) => false,
        Err(std::fs::TryLockError::Error(error)) => return Err(error),
    };
    if let Some(notice) = focus_existing_character(dir, account, stamp)? {
        return Ok(notice);
    }
    if !held {
        // Another launcher or its cold-start supervisor owns this account.
        // A queued duplicate is already opening, including before its pending
        // file appears or after readiness removes it. Do not scan/retire the
        // owner's reservation while its launch lock is held.
        return Ok(Notice::Opening);
    }
    let snapshot = scan(dir, &[account.to_owned()], stamp);
    if snapshot.opening.contains(account) {
        return Ok(Notice::Opening);
    }
    let pending = reservation_path(dir, account);
    let reservation = Reservation {
        version: VERSION,
        account: account.into(),
        nonce: Uuid::new_v4(),
        stamp,
    };
    publish(&pending, &reservation)?;
    // Prepare the supervisor before spawning so a thread-creation failure
    // cannot leave an unobserved live child or an unreaped Unix process.
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let supervisor_dir = dir.to_owned();
    let supervisor_reservation = reservation.clone();
    let supervisor = match std::thread::Builder::new().name("character-start".into()).spawn(move || {
        supervise_launch(&supervisor_dir, &supervisor_reservation, lock, receiver);
    }) {
        Ok(supervisor) => supervisor,
        Err(error) => {
            release_reservation(dir, &reservation);
            return Err(error);
        }
    };
    let launched = std::process::Command::new(executable)
        .args(launch_arguments(account))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let child = match launched {
        Ok(child) => child,
        Err(error) => {
            drop(sender);
            let _ = supervisor.join();
            release_reservation(dir, &reservation);
            return Err(error);
        }
    };
    if let Err(error) = sender.send(child) {
        let mut child = error.0;
        let _ = child.kill();
        let _ = child.wait();
        release_reservation(dir, &reservation);
        return Err(io::Error::other("character supervisor stopped"));
    }
    Ok(Notice::Opening)
}

/// Account data currently uses the avatar UUID alone, including across grids.
/// Do not automate two different grid logins that would write that same store.
fn open_saved_character(dir: &Path, executable: &Path, account: &str, stamp: u64) -> io::Result<Notice> {
    let accounts = crate::login_accounts::load();
    open_catalogue_character(dir, executable, account, stamp, &accounts)
}

fn open_catalogue_character(
    dir: &Path,
    executable: &Path,
    account: &str,
    stamp: u64,
    accounts: &[crate::login_accounts::Account],
) -> io::Result<Notice> {
    let Some(target) = accounts.iter().find(|target| account_id(target) == account) else {
        return Err(io::Error::other("saved character no longer available"));
    };
    let avatar_lock;
    if let Some(avatar) = target.agent.filter(|avatar| !avatar.is_nil()) {
        avatar_lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join(format!("avatar-{avatar}.lock")))?;
        match avatar_lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return contended_avatar_feedback(dir, account, stamp),
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }
        let related: Vec<_> = accounts
            .iter()
            .filter(|candidate| candidate.agent == Some(avatar))
            .take(MAX_ACCOUNTS + 1)
            .map(account_id)
            .collect();
        if related.len() > MAX_ACCOUNTS {
            return Ok(Notice::OtherGrid);
        }
        let snapshot = scan(dir, &related, stamp);
        if snapshot.peers.iter().any(|peer| peer.account != account) || snapshot.opening.iter().any(|opening| opening != account) {
            return Ok(Notice::OtherGrid);
        }
        for related_account in related.iter().filter(|related| related.as_str() != account) {
            if launch_in_progress(dir, related_account)? {
                return Ok(Notice::OtherGrid);
            }
        }
    }
    open_character(dir, executable, account, stamp)
}

pub fn other_grid_open(account: &crate::login_accounts::Account, accounts: &[crate::login_accounts::Account], snapshot: &Snapshot) -> bool {
    account.agent.filter(|avatar| !avatar.is_nil()).is_some_and(|avatar| {
        accounts.iter().any(|other| {
            other.agent == Some(avatar) && other.key() != account.key() && {
                let id = account_id(other);
                snapshot.peers.iter().any(|peer| peer.account == id) || snapshot.opening.contains(&id)
            }
        })
    })
}

fn run(dir: &Path, executable: &Path, instance: Uuid, shared: &Shared) {
    let inbox = focus_path(dir, instance);
    let mut published: Option<Presence> = None;
    let mut scanned = 0;
    let mut catalogue: Option<Vec<crate::login_accounts::Account>> = None;
    let mut catalogue_count = 0;
    loop {
        let (mut own, commands, watching, stop) = {
            let mut state = locked(shared);
            (
                state.own.clone(),
                state.commands.drain(..).collect::<Vec<_>>(),
                state.watch_open,
                state.stop,
            )
        };
        if stop {
            retire(dir, published.as_ref());
            let _ = std::fs::remove_file(&inbox);
            return;
        }
        let stamp = now();
        if let Some(own) = &mut own {
            if published.as_ref().is_none_or(|old| {
                old.account != own.account || old.generation != own.generation || stamp.saturating_sub(old.stamp) >= PUBLISH_SECS
            }) {
                own.stamp = stamp;
                if published.as_ref().is_some_and(|old| old.account != own.account) {
                    retire(dir, published.as_ref());
                }
                if publish(&record_path(dir, &own.account), own).is_ok() {
                    let _ = std::fs::remove_file(reservation_path(dir, &own.account));
                    published = Some(own.clone());
                }
            }
        } else if published.is_some() {
            retire(dir, published.take().as_ref());
        }
        if let Some(request) = read_json::<Focus>(&inbox) {
            let _ = std::fs::remove_file(&inbox);
            if own.as_ref().is_some_and(|own| request.matches(own, stamp)) {
                locked(shared).focus = Some(request);
            }
        }
        for command in commands {
            let notice = match command {
                Command::Open(account) => open_saved_character(dir, executable, &account, stamp),
                Command::Focus(peer) => request_focus(dir, &peer, stamp).map(|()| Notice::FocusRequested),
            }
            .unwrap_or(Notice::Failed);
            locked(shared).notice = Some(notice);
        }
        if stamp != scanned {
            if watching {
                let accounts = crate::login_accounts::load();
                catalogue_count = accounts.len();
                catalogue = Some(accounts.into_iter().take(MAX_ACCOUNTS).collect());
            }
            let mut watched: Vec<_> = catalogue
                .as_ref()
                .map_or_else(Vec::new, |accounts| accounts.iter().map(account_id).collect());
            if watched.is_empty()
                && let Some(own) = own
            {
                watched.insert(0, own.account);
            }
            let mut snapshot = scan(dir, &watched, stamp);
            snapshot.catalogue = catalogue.clone();
            snapshot.catalogue_count = catalogue_count;
            locked(shared).snapshot = snapshot;
            scanned = stamp;
        }
        let state = locked(shared);
        let _ = shared
            .1
            .wait_timeout_while(state, PERIOD, |state| !state.stop && state.commands.is_empty());
    }
}

fn retire(dir: &Path, own: Option<&Presence>) {
    if let Some(own) = own {
        let path = record_path(dir, &own.account);
        if read_json::<Presence>(&path).is_some_and(|current| current.instance == own.instance) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Memory-only presentation: demo actions never reach the process manager.
pub fn demo_snapshot(accounts: &[crate::login_accounts::Account], own: Uuid) -> Snapshot {
    Snapshot {
        peers: accounts
            .iter()
            .take(2)
            .enumerate()
            .map(|(index, account)| Presence {
                version: VERSION,
                instance: if index == 0 { own } else { Uuid::from_u128(2) },
                pid: index as u32 + 1,
                account: account_id(account),
                phase: Phase::Online,
                generation: 1,
                stamp: 0,
            })
            .collect(),
        opening: BTreeSet::new(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(stamp: u64) -> Presence {
        Presence {
            version: VERSION,
            instance: Uuid::new_v4(),
            pid: 42,
            account: identity("https://grid.invalid/login", "synthetic"),
            phase: Phase::Online,
            generation: 3,
            stamp,
        }
    }

    fn temporary() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-characters-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn account_identity_is_grid_scoped_framed_and_case_normalized() {
        assert_eq!(
            identity(" https://a.invalid ", " Synthetic "),
            identity("https://a.invalid", "synthetic")
        );
        assert_ne!(
            identity("https://a.invalid", "synthetic"),
            identity("https://b.invalid", "synthetic")
        );
        assert_ne!(identity("ab", "c"), identity("a", "bc"));
        assert!(valid_id(&identity("grid", "synthetic")));
        for invalid in ["", "../another-character", &"F".repeat(64), &"a".repeat(63)] {
            assert!(!valid_id(invalid));
        }
        assert_ne!(installation_id(Path::new("/Foo/viewer")), installation_id(Path::new("/foo/viewer")));
    }

    #[test]
    fn focus_is_bound_to_instance_account_generation_and_freshness() {
        let own = peer(100);
        let request = Focus {
            version: VERSION,
            instance: own.instance,
            account: own.account.clone(),
            generation: own.generation,
            stamp: 100,
        };
        assert!(request.matches(&own, 101));
        let mut changed = own.clone();
        changed.instance = Uuid::new_v4();
        assert!(!request.matches(&changed, 101), "a reused PID is not the old instance");
        changed = own.clone();
        changed.account = identity("other-grid", "synthetic");
        assert!(!request.matches(&changed, 101));
        changed = own.clone();
        changed.generation += 1;
        assert!(!request.matches(&changed, 101), "a previous login cannot select a new session");
        assert!(!request.matches(&own, 110));
        assert!(!request.matches(&own, 90), "a future clock stamp is rejected");
    }

    #[test]
    fn queued_focus_cannot_cross_an_empty_catalogue_or_new_login_generation() {
        let manager = Manager::default();
        let account = identity("grid", "synthetic");
        let mut state = locked(&manager.shared);
        state.follow(manager.instance, Some(account.clone()), Phase::Online);
        let own = state.own.as_ref().unwrap().clone();
        state.focus = Some(Focus {
            version: VERSION,
            instance: own.instance,
            account: own.account,
            generation: own.generation,
            stamp: now(),
        });
        state.follow(manager.instance, None, Phase::Login);
        state.follow(manager.instance, Some(account), Phase::Online);
        assert!(state.own.as_ref().unwrap().generation > own.generation);
        drop(state);
        assert!(!manager.take_focus());
    }

    #[test]
    fn stale_or_changed_viewers_cannot_receive_a_focus_request() {
        let dir = temporary();
        let original = peer(100);
        publish(&record_path(&dir, &original.account), &original).unwrap();
        assert!(request_focus(&dir, &original, 101).is_ok());
        let request: Focus = read_json(&focus_path(&dir, original.instance)).unwrap();
        assert!(request.matches(&original, 101));
        assert!(request_focus(&dir, &original, 110).is_err());
        let mut changed = original.clone();
        changed.generation += 1;
        publish(&record_path(&dir, &original.account), &changed).unwrap();
        assert!(request_focus(&dir, &original, 101).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn login_and_connecting_windows_prevent_a_duplicate_launch() {
        let dir = temporary();
        for phase in [Phase::Login, Phase::Connecting, Phase::Online] {
            let mut existing = peer(100);
            existing.phase = phase;
            publish(&record_path(&dir, &existing.account), &existing).unwrap();
            assert!(matches!(
                open_character(&dir, Path::new("/never-launch-this-program"), &existing.account, 101),
                Ok(Notice::FocusRequested)
            ));
            std::fs::remove_file(record_path(&dir, &existing.account)).unwrap();
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn launch_reservation_blocks_second_start_and_failure_releases_it() {
        let dir = temporary();
        let account = identity("grid", "synthetic");
        publish(
            &reservation_path(&dir, &account),
            &Reservation {
                version: VERSION,
                account: account.clone(),
                nonce: Uuid::new_v4(),
                stamp: 100,
            },
        )
        .unwrap();
        assert!(matches!(
            open_character(&dir, Path::new("/never-launch-this-program"), &account, 101),
            Ok(Notice::Opening)
        ));
        assert!(open_character(&dir, Path::new("/never-launch-this-program"), &account, 131).is_err());
        assert!(
            !reservation_path(&dir, &account).exists(),
            "failed launch must not hold the character"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn queued_duplicate_opens_are_idempotent_through_cold_start_and_ready_handoff() {
        let dir = temporary();
        let account = identity("grid", "synthetic");
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join(format!("{account}.lock")))
            .unwrap();
        lock.lock().unwrap();
        let executable = Path::new("/never-launch-this-program");
        // Another launcher can hold the account lock before it publishes.
        assert!(matches!(open_character(&dir, executable, &account, 100), Ok(Notice::Opening)));
        assert!(!reservation_path(&dir, &account).exists());
        let reservation = Reservation {
            version: VERSION,
            account: account.clone(),
            nonce: Uuid::new_v4(),
            stamp: 100,
        };
        publish(&reservation_path(&dir, &account), &reservation).unwrap();
        // Model multiple commands drained in one worker tick, followed by a
        // cold child whose owning supervisor has not refreshed its lease yet.
        for stamp in [101, 101, 101, 101, 101, 100 + LAUNCH_SECS + 1] {
            assert!(matches!(open_character(&dir, executable, &account, stamp), Ok(Notice::Opening)));
            let current: Reservation = read_json(&reservation_path(&dir, &account)).unwrap();
            assert_eq!(current.nonce, reservation.nonce);
            assert_eq!(
                current.stamp, reservation.stamp,
                "a duplicate must not mutate or retire another launch"
            );
        }
        let mut ready = peer(100 + LAUNCH_SECS + 2);
        ready.account = account.clone();
        ready.phase = Phase::Login;
        publish(&record_path(&dir, &account), &ready).unwrap();
        release_reservation(&dir, &reservation);
        // A child's first presence can precede its supervisor releasing the
        // account lock. The same request now focuses that exact ready window.
        assert!(matches!(
            open_character(&dir, executable, &account, 100 + LAUNCH_SECS + 2),
            Ok(Notice::FocusRequested)
        ));
        let focus: Focus = read_json(&focus_path(&dir, ready.instance)).unwrap();
        assert!(focus.matches(&ready, 100 + LAUNCH_SECS + 2));
        assert!(!reservation_path(&dir, &account).exists());
        drop(lock);
        assert!(matches!(
            open_character(&dir, executable, &account, 100 + LAUNCH_SECS + 3),
            Ok(Notice::FocusRequested)
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn genuine_launch_and_lock_file_failures_are_not_reported_as_opening() {
        let dir = temporary();
        let account = identity("grid", "synthetic");
        let executable = Path::new("/never-launch-this-program");
        let error = open_character(&dir, executable, &account, 100)
            .err()
            .expect("missing executable must fail");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(!reservation_path(&dir, &account).exists());
        // A bad lock-file path is an IO failure, distinct from another
        // viewer legitimately holding an otherwise usable account lock.
        let bad_account = identity("grid", "synthetic-bad-lock-path");
        let lock_path = dir.join(format!("{bad_account}.lock"));
        std::fs::create_dir(&lock_path).unwrap();
        assert!(open_character(&dir, executable, &bad_account, 101).is_err());
        assert!(!reservation_path(&dir, &bad_account).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn saved_avatar_accounts() -> Vec<crate::login_accounts::Account> {
        (0..2)
            .map(|index| crate::login_accounts::Account {
                username: "synthetic-avatar".into(),
                uri: format!("https://synthetic-grid-{index}.invalid/login"),
                remember_password: false,
                remember_mfa: false,
                start_location: "last".into(),
                start_region: String::new(),
                agent: Some(Uuid::from_u128(42)),
                last_place: None,
            })
            .collect()
    }

    #[test]
    fn avatar_aware_open_reports_contention_without_cross_grid_launch_or_pending_retirement() {
        let dir = temporary();
        let accounts = saved_avatar_accounts();
        let alpha = account_id(&accounts[0]);
        let beta = account_id(&accounts[1]);
        let executable = Path::new("/never-launch-this-program");
        let avatar_lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join(format!("avatar-{}.lock", accounts[0].agent.unwrap())))
            .unwrap();
        avatar_lock.lock().unwrap();
        // This avatar-wide lock alone does not identify the requested grid.
        assert!(matches!(
            open_catalogue_character(&dir, executable, &alpha, 100, &accounts),
            Ok(Notice::Busy)
        ));
        let account_lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join(format!("{alpha}.lock")))
            .unwrap();
        account_lock.lock().unwrap();
        let pending = Reservation {
            version: VERSION,
            account: alpha.clone(),
            nonce: Uuid::new_v4(),
            stamp: 100,
        };
        publish(&reservation_path(&dir, &alpha), &pending).unwrap();
        assert!(matches!(
            open_catalogue_character(&dir, executable, &alpha, 101, &accounts),
            Ok(Notice::Opening)
        ));
        assert!(matches!(
            open_catalogue_character(&dir, executable, &beta, 101, &accounts),
            Ok(Notice::Busy)
        ));
        assert!(!reservation_path(&dir, &beta).exists());
        // A cold child can outlive the crash lease. Neither a contended gate
        // nor related-account discovery may erase its still-owned nonce.
        let late = 100 + LAUNCH_SECS + 1;
        assert!(matches!(
            open_catalogue_character(&dir, executable, &alpha, late, &accounts),
            Ok(Notice::Opening)
        ));
        drop(avatar_lock);
        assert!(matches!(
            open_catalogue_character(&dir, executable, &alpha, late, &accounts),
            Ok(Notice::Opening)
        ));
        let observed: Reservation = read_json(&reservation_path(&dir, &alpha)).unwrap();
        assert_eq!((observed.nonce, observed.stamp), (pending.nonce, pending.stamp));
        // Even with an expired lease, the other grid remains excluded by the
        // exact related launch lock while the avatar gate is owned here.
        assert!(matches!(
            open_catalogue_character(&dir, executable, &beta, late, &accounts),
            Ok(Notice::OtherGrid)
        ));
        assert!(!dir.join(format!("{beta}.lock")).exists());
        assert!(!reservation_path(&dir, &beta).exists());
        let mut ready = peer(late + 1);
        ready.account = alpha.clone();
        ready.phase = Phase::Login;
        publish(&record_path(&dir, &alpha), &ready).unwrap();
        release_reservation(&dir, &pending);
        let avatar_lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(dir.join(format!("avatar-{}.lock", accounts[0].agent.unwrap())))
            .unwrap();
        avatar_lock.lock().unwrap();
        assert!(matches!(
            open_catalogue_character(&dir, executable, &alpha, late + 1, &accounts),
            Ok(Notice::FocusRequested)
        ));
        assert!(matches!(
            open_catalogue_character(&dir, executable, &beta, late + 1, &accounts),
            Ok(Notice::Busy)
        ));
        drop(avatar_lock);
        assert!(matches!(
            open_catalogue_character(&dir, executable, &beta, late + 1, &accounts),
            Ok(Notice::OtherGrid)
        ));
        let focus: Focus = read_json(&focus_path(&dir, ready.instance)).unwrap();
        assert!(focus.matches(&ready, late + 1));
        drop(account_lock);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn avatar_aware_open_keeps_real_spawn_and_gate_file_failures_as_errors() {
        let dir = temporary();
        let bad_dir = temporary();
        let accounts = saved_avatar_accounts();
        let alpha = account_id(&accounts[0]);
        let executable = Path::new("/never-launch-this-program");
        let error = open_catalogue_character(&dir, executable, &alpha, 100, &accounts)
            .err()
            .expect("missing executable must remain an error through the avatar gate");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(!reservation_path(&dir, &alpha).exists());
        std::fs::create_dir(bad_dir.join(format!("avatar-{}.lock", accounts[0].agent.unwrap()))).unwrap();
        assert!(open_catalogue_character(&bad_dir, executable, &alpha, 100, &accounts).is_err());
        assert!(!reservation_path(&bad_dir, &alpha).exists());
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(bad_dir).unwrap();
    }

    #[test]
    fn slow_child_keeps_its_reservation_until_ready_without_claiming_another_launch() {
        let dir = temporary();
        let account = identity("grid", "synthetic");
        let reservation = Reservation {
            version: VERSION,
            account: account.clone(),
            nonce: Uuid::new_v4(),
            stamp: 100,
        };
        publish(&reservation_path(&dir, &account), &reservation).unwrap();
        // The supervisor still owns the OS account lock throughout this
        // simulated cold start, even after the original 30-second lease.
        assert!(refresh_reservation(&dir, &reservation, 123, 100 + LAUNCH_SECS + 1).unwrap());
        assert!(
            scan(&dir, std::slice::from_ref(&account), 100 + LAUNCH_SECS + 2)
                .opening
                .contains(&account)
        );
        assert!(matches!(
            open_character(&dir, Path::new("/never-launch-this-program"), &account, 100 + LAUNCH_SECS + 2),
            Ok(Notice::Opening)
        ));
        let mut ready = peer(100 + LAUNCH_SECS + 3);
        ready.account = account.clone();
        ready.pid = 123;
        publish(&record_path(&dir, &account), &ready).unwrap();
        assert!(!refresh_reservation(&dir, &reservation, 123, 100 + LAUNCH_SECS + 3).unwrap());
        assert!(!reservation_path(&dir, &account).exists());
        let replacement = Reservation {
            nonce: Uuid::new_v4(),
            ..reservation.clone()
        };
        publish(&reservation_path(&dir, &account), &replacement).unwrap();
        assert!(!refresh_reservation(&dir, &reservation, 123, 200).unwrap());
        release_reservation(&dir, &reservation);
        assert_eq!(
            read_json::<Reservation>(&reservation_path(&dir, &account)).unwrap().nonce,
            replacement.nonce
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_launch_lock_and_installation_scope_cannot_be_bypassed() {
        let dir = temporary();
        let other = temporary();
        let existing = peer(100);
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join(format!("{}.lock", existing.account)))
            .unwrap();
        lock.lock().unwrap();
        assert!(matches!(
            open_character(&dir, Path::new("/never-launch-this-program"), &existing.account, 101),
            Ok(Notice::Opening)
        ));
        assert!(!reservation_path(&dir, &existing.account).exists());
        publish(&record_path(&dir, &existing.account), &existing).unwrap();
        assert!(request_focus(&other, &existing, 101).is_err());
        drop(lock);
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(other).unwrap();
    }

    #[test]
    fn discovery_is_bounded_and_ignores_oversized_invalid_stale_and_misnamed_records() {
        let dir = temporary();
        let good = peer(100);
        for index in 0..300 {
            std::fs::write(dir.join(format!("unrelated-{index}.lock")), b"").unwrap();
        }
        publish(&record_path(&dir, &good.account), &good).unwrap();
        let mut stale = peer(1);
        stale.account = identity("stale-grid", "synthetic");
        publish(&record_path(&dir, &stale.account), &stale).unwrap();
        let wrong = identity("wrong-grid", "synthetic");
        publish(&record_path(&dir, &wrong), &good).unwrap();
        let oversized = identity("large-grid", "synthetic");
        std::fs::write(record_path(&dir, &oversized), vec![b'x'; MAX_FILE as usize + 1]).unwrap();
        let ids = vec![good.account.clone(), stale.account.clone(), wrong, oversized];
        assert_eq!(scan(&dir, &ids, 122).peers, Vec::<Presence>::new());
        assert!(!record_path(&dir, &stale.account).exists(), "old process records are retired");
        assert_eq!(scan(&dir, &ids, 101).peers, vec![good]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn launch_arguments_only_select_a_saved_account_and_never_log_in() {
        let account = identity("https://grid.invalid/login", "synthetic");
        let args = launch_arguments(&account);
        assert_eq!(args[0], "--saved-account");
        assert_eq!(args[1], account);
        assert!(
            !args
                .iter()
                .any(|arg| arg.contains("synthetic") || arg.contains("grid.invalid") || arg == "--login")
        );
    }
}
