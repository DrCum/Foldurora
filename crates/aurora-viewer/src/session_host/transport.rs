//! Private, bounded local transport. The controller and viewer only exchange
//! typed identities and state; all disk work and child launches stay off-frame.
//!
//! One outstanding command per worker preserves acknowledgments. Status is a
//! separate latest-value slot. The fresh WorkerId is the boot nonce; neither a
//! PID nor a window handle can authorize a command.

use aurora_session::{
    AccountKey, Binding, Command, Envelope, Event, GridKey, HostId, HostedViewport, InstallationId, MAX_WORKERS, Message, PROTOCOL_VERSION,
    SessionId, SessionIdentity, WorkerId, WorkerIdentity, WorkerStatus,
};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

const PERIOD: Duration = Duration::from_millis(100);
const HEARTBEAT: Duration = Duration::from_secs(1);
const REACHABLE: Duration = Duration::from_secs(5);
const CLOSE_WAIT: Duration = Duration::from_secs(2);
const MAX_CONFIG: usize = 4096;
const MAX_QUEUE: usize = 16;

#[derive(Clone)]
pub(crate) struct SavedAccount {
    pub key: AccountKey,
    pub label: String,
    pub grid: String,
}

#[derive(Clone)]
pub(crate) struct WorkerSummary {
    pub binding: Binding,
    pub status: Option<WorkerStatus>,
    pub reachable: bool,
    pub label: String,
    pub grid: String,
}

#[derive(Clone, Default)]
pub(crate) struct HostSnapshot {
    pub accounts: Vec<SavedAccount>,
    pub workers: Vec<WorkerSummary>,
    pub opening: Vec<AccountKey>,
    pub stopped: bool,
    pub error: Option<String>,
    /// No native adoption until a completed prepared frame can be proven.
    pub can_activate: bool,
    pub active: Option<WorkerId>,
    pub pending: Option<WorkerId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostError {
    Busy,
    Unsupported,
    Stopped,
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "session controller: {self:?}")
    }
}
impl std::error::Error for HostError {}

enum HostIntent {
    Open(Option<AccountKey>),
    Detach(WorkerId),
}

pub(crate) struct HostController {
    tx: Sender<HostIntent>,
    snapshot: Arc<Mutex<Arc<HostSnapshot>>>,
    stop: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

impl HostController {
    pub fn start() -> anyhow::Result<Self> {
        let (tx, rx) = bounded(MAX_QUEUE);
        let snapshot = Arc::new(Mutex::new(Arc::new(HostSnapshot::default())));
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let shared = snapshot.clone();
        let ending = stop.clone();
        let ended = stopped.clone();
        std::thread::Builder::new().name("session-host-io".into()).spawn(move || {
            if let Err(error) = host_loop(&rx, &shared, &ending) {
                let previous = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
                let mut next = (*previous).clone();
                next.error = Some(error.to_string());
                next.stopped = true;
                *shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(next);
            }
            ended.store(true, Ordering::Release);
        })?;
        Ok(Self {
            tx,
            snapshot,
            stop,
            stopped,
        })
    }

    pub fn snapshot(&self) -> Arc<HostSnapshot> {
        self.snapshot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    pub fn open_manual_login(&self, account: Option<AccountKey>) -> Result<(), HostError> {
        self.send(HostIntent::Open(account))
    }

    /// This checkpoint must never imply that queued `stats.presented` is a
    /// completed GPU frame. The shell disables this action via can_activate.
    pub fn activate(&self, _worker: WorkerId, _viewport: HostedViewport) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }

    pub fn detach(&self, worker: WorkerId) -> Result<(), HostError> {
        self.send(HostIntent::Detach(worker))
    }

    fn send(&self, intent: HostIntent) -> Result<(), HostError> {
        if self.stop.load(Ordering::Acquire) || self.stopped() {
            return Err(HostError::Stopped);
        }
        self.tx.try_send(intent).map_err(|error| match error {
            TrySendError::Full(_) => HostError::Busy,
            TrySendError::Disconnected(_) => HostError::Stopped,
        })
    }

    /// Nonblocking: the IO thread requests detach, then retires after its
    /// bounded acknowledgment period. It never kills or logs out a worker.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Release);
    }
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
}

impl Drop for HostController {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkerConfig {
    pub version: u16,
    pub identity: WorkerIdentity,
    pub session: SessionId,
    pub generation: u64,
    pub expected_account: Option<AccountKey>,
    pub expected_grid: Option<GridKey>,
    pub lease_epoch: u64,
}

impl WorkerConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.version == PROTOCOL_VERSION && self.generation != 0 && self.lease_epoch != 0,
            "invalid worker configuration"
        );
        anyhow::ensure!(
            self.expected_account.is_some() == self.expected_grid.is_some(),
            "incomplete worker account binding"
        );
        anyhow::ensure!(
            self.identity.installation == current_installation()?,
            "worker belongs to another installation"
        );
        Ok(())
    }

    pub fn directory(&self) -> io::Result<PathBuf> {
        Ok(super::profile::host_root()?
            .join("channels")
            .join(self.identity.installation.to_string())
            .join(self.identity.host.to_string())
            .join(self.identity.worker.to_string()))
    }

    pub fn session_binding(&self, account: AccountKey, grid: GridKey) -> SessionIdentity {
        SessionIdentity {
            session: self.session,
            account,
            grid,
            generation: self.generation,
        }
    }

    fn accepts(&self, envelope: &Envelope) -> bool {
        envelope.binding.worker == self.identity
            && envelope.lease_epoch == self.lease_epoch
            && envelope.binding.session.is_none_or(|session| {
                session.session == self.session
                    && session.generation >= self.generation
                    && self.expected_account.is_none_or(|key| key == session.account)
                    && self.expected_grid.is_none_or(|key| key == session.grid)
            })
    }
}

pub(super) fn current_installation() -> anyhow::Result<InstallationId> {
    let executable = std::env::current_exe()?.canonicalize()?;
    Ok(InstallationId::from_bytes(
        Sha256::digest(executable.as_os_str().as_encoded_bytes()).into(),
    )?)
}

pub(crate) fn grid_key(uri: &str) -> Result<GridKey, aurora_session::Error> {
    GridKey::from_bytes(Sha256::digest(uri.trim().as_bytes()).into())
}

/// Reads cannot follow symlinks/reparse points, grow without limit, or accept
/// unknown fields. The private parent and file ACL are checked on every read.
fn read_bytes(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    super::profile::validate_private_file(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "session file exceeds its bound"));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "session file grew beyond its bound"));
    }
    Ok(bytes)
}

pub(super) fn read_config(path: &Path) -> anyhow::Result<(WorkerConfig, PathBuf)> {
    anyhow::ensure!(path.is_absolute(), "worker configuration must be an absolute private path");
    let config: WorkerConfig = read_json(path, MAX_CONFIG)?;
    config.validate()?;
    let directory = config.directory()?;
    anyhow::ensure!(
        path == directory.join("bootstrap.json"),
        "worker configuration path does not match its binding"
    );
    super::profile::private_dir(&directory)?;
    Ok((config, directory))
}

fn read_json<T: DeserializeOwned>(path: &Path, limit: usize) -> io::Result<T> {
    serde_json::from_slice(&read_bytes(path, limit)?).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "malformed session file"))
}

fn publish(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::other("missing session directory"))?;
    super::profile::private_dir(parent)?;
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        drop(file);
        super::profile::validate_private_file(&temporary)?;
        // Reuse the existing atomic publisher's Windows sharing retry, but
        // retain this private temporary file's access mode on replacement.
        let mut attempt = 0;
        loop {
            match std::fs::rename(&temporary, path) {
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied && attempt < 5 => {
                    std::thread::sleep(Duration::from_millis(1 << attempt));
                    attempt += 1;
                }
                result => return result,
            }
        }
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn publish_envelope(path: &Path, envelope: &Envelope) -> io::Result<()> {
    let bytes = aurora_session::encode(envelope).map_err(io::Error::other)?;
    publish(path, &bytes)
}

fn read_envelope(path: &Path) -> io::Result<Envelope> {
    aurora_session::decode(&read_bytes(path, aurora_session::MAX_MESSAGE_BYTES)?).map_err(io::Error::other)
}

pub(super) struct WorkerIo {
    commands: Receiver<Envelope>,
    replies: Sender<(Binding, u64, Event)>,
    status: Arc<Mutex<Option<(Binding, WorkerStatus)>>>,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
}

impl WorkerIo {
    pub fn start(config: WorkerConfig, directory: PathBuf) -> anyhow::Result<Self> {
        let (commands_tx, commands) = bounded(MAX_QUEUE);
        let (replies, replies_rx) = bounded(MAX_QUEUE);
        let status = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let latest = status.clone();
        let ending = stop.clone();
        let failure = failed.clone();
        std::thread::Builder::new().name("session-worker-io".into()).spawn(move || {
            if let Err(error) = worker_loop(config, &directory, commands_tx, replies_rx, latest, &ending) {
                log::warn!("session worker transport stopped: {error}");
                failure.store(true, Ordering::Release);
            }
        })?;
        Ok(Self {
            commands,
            replies,
            status,
            stop,
            failed,
        })
    }
    pub fn commands(&self) -> impl Iterator<Item = Envelope> + '_ {
        self.commands.try_iter().take(MAX_QUEUE)
    }
    pub fn reply(&self, binding: Binding, reply_to: u64, event: Event) -> bool {
        self.replies.try_send((binding, reply_to, event)).is_ok()
    }
    pub fn status(&self, binding: Binding, status: WorkerStatus) {
        *self.status.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((binding, status));
    }
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Drop for WorkerIo {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn worker_loop(
    config: WorkerConfig,
    directory: &Path,
    commands: Sender<Envelope>,
    replies: Receiver<(Binding, u64, Event)>,
    status: Arc<Mutex<Option<(Binding, WorkerStatus)>>>,
    stop: &AtomicBool,
) -> io::Result<()> {
    super::profile::private_dir(directory)?;
    let mut received = 0;
    let mut sent = 0u64;
    let mut next_status = Instant::now();
    loop {
        let ending = stop.load(Ordering::Acquire);
        if !ending
            && let Ok(envelope) = read_envelope(&directory.join("command.json"))
            && envelope.binding.worker == config.identity
            && envelope.lease_epoch == config.lease_epoch
            && envelope.sequence > received
            && matches!(envelope.message, Message::Command(_))
        {
            let sequence = envelope.sequence;
            if commands.try_send(envelope).is_ok() {
                received = sequence;
            }
        }
        for (binding, reply_to, event) in replies.try_iter().take(MAX_QUEUE) {
            sent = sent.checked_add(1).ok_or_else(|| io::Error::other("session sequence exhausted"))?;
            publish_envelope(
                &directory.join("reply.json"),
                &Envelope {
                    version: PROTOCOL_VERSION,
                    binding,
                    lease_epoch: config.lease_epoch,
                    sequence: sent,
                    reply_to,
                    message: Message::Event(event),
                },
            )?;
        }
        if ending {
            break;
        }
        if Instant::now() >= next_status {
            let latest = status.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
            if let Some((binding, status)) = latest {
                sent = sent.checked_add(1).ok_or_else(|| io::Error::other("session sequence exhausted"))?;
                publish_envelope(
                    &directory.join("status.json"),
                    &Envelope {
                        version: PROTOCOL_VERSION,
                        binding,
                        lease_epoch: config.lease_epoch,
                        sequence: sent,
                        reply_to: 0,
                        message: Message::Event(Event::Status(status)),
                    },
                )?;
            }
            next_status = Instant::now() + HEARTBEAT;
        }
        std::thread::sleep(PERIOD);
    }
    Ok(())
}

struct Child {
    config: WorkerConfig,
    directory: PathBuf,
    process: std::process::Child,
    binding: Binding,
    status: Option<WorkerStatus>,
    last_status: u64,
    last_reply: u64,
    last_seen: Option<Instant>,
    sequence: u64,
    pending: Option<(u64, bool)>,
    next_heartbeat: Instant,
    detach: bool,
    detached: bool,
    label: String,
    grid: String,
}

impl Child {
    fn send(&mut self, command: Command) -> io::Result<()> {
        let detach = matches!(command, Command::Detach);
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| io::Error::other("session sequence exhausted"))?;
        let envelope = Envelope {
            version: PROTOCOL_VERSION,
            // Login can change between publication and receipt. Lease/Detach
            // bind the worker boot identity, never yesterday's login session.
            binding: command_binding(self.binding, &command),
            lease_epoch: self.config.lease_epoch,
            sequence: self.sequence,
            reply_to: 0,
            message: Message::Command(command),
        };
        publish_envelope(&self.directory.join("command.json"), &envelope)?;
        self.pending = Some((self.sequence, detach));
        Ok(())
    }

    fn accepts(&self, envelope: &Envelope) -> bool {
        self.config.accepts(envelope)
    }

    fn poll(&mut self) -> io::Result<()> {
        if self.process.try_wait()?.is_some() {
            self.detached = true;
            return Ok(());
        }
        if let Ok(envelope) = read_envelope(&self.directory.join("status.json"))
            && self.accepts(&envelope)
            && envelope.sequence > self.last_status
            && envelope.reply_to == 0
            && let Message::Event(Event::Status(status)) = envelope.message
        {
            self.last_status = envelope.sequence;
            self.last_seen = Some(Instant::now());
            self.binding = envelope.binding;
            self.status = Some(status);
        }
        if let Ok(envelope) = read_envelope(&self.directory.join("reply.json"))
            && self.accepts(&envelope)
            && envelope.sequence > self.last_reply
            && self.pending.is_some_and(|(sequence, _)| sequence == envelope.reply_to)
            && let Message::Event(event) = envelope.message
            && self.pending.is_some_and(|(_, detach)| {
                if detach {
                    matches!(event, Event::Detached)
                } else {
                    matches!(event, Event::Status(_))
                }
            })
        {
            self.last_reply = envelope.sequence;
            self.last_seen = Some(Instant::now());
            self.pending = None;
            self.detached |= matches!(event, Event::Detached);
        }
        if self.detached {
            return Ok(());
        }
        // Detach is worker-bound, so it remains valid across login changes.
        // It may supersede a heartbeat that lost its reply during startup.
        if self.detach {
            if !self.pending.is_some_and(|(_, detach)| detach) {
                self.send(Command::Detach)?;
                self.next_heartbeat = Instant::now() + HEARTBEAT;
            }
        } else if self.pending.is_none() && Instant::now() >= self.next_heartbeat {
            self.send(Command::Heartbeat {
                workspace_foreground: true,
            })?;
            self.next_heartbeat = Instant::now() + HEARTBEAT;
        }
        Ok(())
    }
}

fn command_binding(mut binding: Binding, command: &Command) -> Binding {
    if !command.session_bound() {
        binding.session = None;
    }
    binding
}

fn host_loop(rx: &Receiver<HostIntent>, shared: &Arc<Mutex<Arc<HostSnapshot>>>, stop: &AtomicBool) -> anyhow::Result<()> {
    anyhow::ensure!(
        aurora_updater::test_environment().is_none(),
        "production session hosting is unavailable in an update fixture"
    );
    let installation = current_installation()?;
    let host = HostId::from_bytes(*uuid::Uuid::new_v4().as_bytes())?;
    let mut accounts = Vec::new();
    for account in crate::login_accounts::load().into_iter().take(64) {
        if let Ok(key) = AccountKey::parse_hex(&crate::characters::account_id(&account)) {
            accounts.push((
                SavedAccount {
                    key,
                    label: account.username.clone(),
                    grid: crate::app::host_grid_label(&account.uri, &[]),
                },
                account,
            ));
        }
    }
    let mut children: Vec<Child> = Vec::new();
    let mut closing = None;
    let mut error = None;
    loop {
        if stop.load(Ordering::Acquire) && closing.is_none() {
            closing = Some(Instant::now() + CLOSE_WAIT);
            for child in &mut children {
                child.detach = true;
            }
        }
        for intent in rx.try_iter().take(MAX_QUEUE) {
            if closing.is_some() {
                break;
            }
            match intent {
                HostIntent::Open(key) => {
                    if children.len() >= MAX_WORKERS
                        || key.is_some_and(|key| {
                            children
                                .iter()
                                .any(|child| !child.detached && child.config.expected_account == Some(key))
                        })
                    {
                        continue;
                    }
                    let account = key.and_then(|key| accounts.iter().find(|(saved, _)| saved.key == key));
                    if key.is_some() && account.is_none() {
                        continue;
                    }
                    match launch(installation, host, account) {
                        Ok(child) => children.push(child),
                        Err(failure) => error = Some(failure.to_string()),
                    }
                }
                HostIntent::Detach(worker) => {
                    if let Some(child) = children.iter_mut().find(|child| child.config.identity.worker == worker) {
                        child.detach = true;
                    }
                }
            }
        }
        for child in &mut children {
            if !child.detached
                && let Err(failure) = child.poll()
            {
                error = Some(failure.to_string());
            }
        }
        let stopped = closing.is_some_and(|deadline| Instant::now() >= deadline || children.iter().all(|child| child.detached));
        let workers = children
            .iter()
            .filter(|child| !child.detached)
            .map(|child| WorkerSummary {
                binding: child.binding,
                status: child.status.clone(),
                reachable: child.last_seen.is_some_and(|seen| seen.elapsed() <= REACHABLE),
                label: child.label.clone(),
                grid: child.grid.clone(),
            })
            .collect();
        let opening = children
            .iter()
            .filter(|child| !child.detached && child.status.is_none())
            .filter_map(|child| child.config.expected_account)
            .collect();
        *shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(HostSnapshot {
            accounts: accounts.iter().map(|(saved, _)| saved.clone()).collect(),
            workers,
            opening,
            stopped,
            error: error.clone(),
            can_activate: false,
            active: None,
            pending: None,
        });
        if stopped {
            break;
        }
        std::thread::sleep(PERIOD);
    }
    Ok(())
}

fn launch(
    installation: InstallationId,
    host: HostId,
    account: Option<&(SavedAccount, crate::login_accounts::Account)>,
) -> anyhow::Result<Child> {
    let config = WorkerConfig {
        version: PROTOCOL_VERSION,
        identity: WorkerIdentity {
            installation,
            host,
            worker: WorkerId::from_bytes(*uuid::Uuid::new_v4().as_bytes())?,
        },
        session: SessionId::from_bytes(*uuid::Uuid::new_v4().as_bytes())?,
        generation: 1,
        expected_account: account.map(|(saved, _)| saved.key),
        expected_grid: account.map(|(_, account)| grid_key(&account.uri)).transpose()?,
        lease_epoch: 1,
    };
    config.validate()?;
    let directory = config.directory()?;
    super::profile::private_dir(&directory)?;
    let path = directory.join("bootstrap.json");
    publish(&path, &serde_json::to_vec(&config)?)?;
    let mut command = std::process::Command::new(std::env::current_exe()?.canonicalize()?);
    command.arg("--host-worker-config").arg(&path);
    if let Some(key) = config.expected_account {
        command.arg("--saved-account").arg(key.to_string());
    }
    if let Some(language) = crate::cli::get().language.as_deref() {
        command.arg("--language").arg(language);
    }
    // Inherited environment is unchanged; no login values, password arguments,
    // user input injection or credential-store access occur in the controller.
    let process = command.spawn()?;
    let binding = Binding {
        worker: config.identity,
        session: None,
    };
    Ok(Child {
        config,
        directory,
        process,
        binding,
        status: None,
        last_status: 0,
        last_reply: 0,
        last_seen: None,
        sequence: 0,
        pending: None,
        next_heartbeat: Instant::now(),
        detach: false,
        detached: false,
        label: account.map_or_else(String::new, |(saved, _)| saved.label.clone()),
        grid: account.map_or_else(String::new, |(saved, _)| saved.grid.clone()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> WorkerConfig {
        WorkerConfig {
            version: PROTOCOL_VERSION,
            identity: WorkerIdentity {
                installation: current_installation().expect("test executable"),
                host: HostId::from_bytes([1; 16]).expect("host"),
                worker: WorkerId::from_bytes(*uuid::Uuid::new_v4().as_bytes()).expect("worker"),
            },
            session: SessionId::from_bytes([3; 16]).expect("local session"),
            generation: 1,
            expected_account: Some(AccountKey::from_bytes([4; 32]).expect("account")),
            expected_grid: Some(GridKey::from_bytes([5; 32]).expect("grid")),
            lease_epoch: 7,
        }
    }

    fn status(config: &WorkerConfig) -> Envelope {
        Envelope {
            version: PROTOCOL_VERSION,
            binding: Binding {
                worker: config.identity,
                session: Some(config.session_binding(config.expected_account.expect("account"), config.expected_grid.expect("grid"))),
            },
            lease_epoch: config.lease_epoch,
            sequence: 1,
            reply_to: 0,
            message: Message::Event(Event::Status(WorkerStatus::default())),
        }
    }

    #[test]
    fn session_transport_rejects_foreign_host_nonce_account_grid_generation_and_lease() {
        let config = config();
        let original = status(&config);
        assert!(config.accepts(&original));
        let mut foreign = original.clone();
        foreign.binding.worker.host = HostId::from_bytes([9; 16]).expect("other host");
        assert!(!config.accepts(&foreign));
        foreign = original.clone();
        foreign.binding.worker.worker = WorkerId::from_bytes([9; 16]).expect("other launch");
        assert!(!config.accepts(&foreign));
        foreign = original.clone();
        foreign.binding.session.as_mut().expect("session").account = AccountKey::from_bytes([9; 32]).expect("other account");
        assert!(!config.accepts(&foreign));
        foreign = original.clone();
        foreign.binding.session.as_mut().expect("session").grid = GridKey::from_bytes([9; 32]).expect("other grid");
        assert!(!config.accepts(&foreign));
        foreign = original.clone();
        foreign.binding.session.as_mut().expect("session").generation = 0;
        assert!(!config.accepts(&foreign));
        foreign = original;
        foreign.lease_epoch += 1;
        assert!(!config.accepts(&foreign));
    }

    #[test]
    fn session_bootstrap_accepts_only_its_private_bound_location_and_known_fields() {
        let config = config();
        let directory = config.directory().expect("private test directory");
        super::super::profile::private_dir(&directory).expect("private test directory");
        let path = directory.join("bootstrap.json");
        publish(&path, &serde_json::to_vec(&config).expect("serialize")).expect("publish");
        assert!(read_config(&path).is_ok());
        let wrong = directory.join("other.json");
        publish(&wrong, &serde_json::to_vec(&config).expect("serialize")).expect("publish");
        assert!(read_config(&wrong).is_err());
        let mut value = serde_json::to_value(&config).expect("serialize");
        value["unrecognized"] = true.into();
        publish(&path, &serde_json::to_vec(&value).expect("serialize")).expect("publish");
        assert!(read_config(&path).is_err());
        publish(&path, &vec![b' '; MAX_CONFIG + 1]).expect("oversized fixture");
        assert!(read_config(&path).is_err());
        std::fs::remove_dir_all(directory).expect("remove own fixture");
    }

    #[test]
    fn session_worker_transport_keeps_commands_bounded_and_does_not_renew_replays() {
        let config = config();
        let directory = config.directory().expect("private test directory");
        super::super::profile::private_dir(&directory).expect("private test directory");
        let io = WorkerIo::start(config.clone(), directory.clone()).expect("start IO");
        let envelope = Envelope {
            version: PROTOCOL_VERSION,
            binding: Binding {
                worker: config.identity,
                session: None,
            },
            lease_epoch: config.lease_epoch,
            sequence: 1,
            reply_to: 0,
            message: Message::Command(Command::Heartbeat {
                workspace_foreground: false,
            }),
        };
        publish_envelope(&directory.join("command.json"), &envelope).expect("publish command");
        let received = io.commands.recv_timeout(Duration::from_secs(3)).expect("bounded worker receipt");
        assert_eq!(received, envelope);
        assert!(
            io.commands.recv_timeout(PERIOD * 3).is_err(),
            "same command must not renew the lease"
        );
        io.stop();
        // IO shutdown is deliberately asynchronous; its thread performs no
        // further writes after this lease-only fixture has been stopped.
    }

    #[test]
    fn session_worker_lease_and_detach_survive_logout_before_command_receipt() {
        let config = config();
        let online = status(&config).binding;
        let mut guard = aurora_session::WorkerGuard::new(config.identity, config.lease_epoch, 0).expect("guard");
        guard.update_session(online.session).expect("online");
        let heartbeat = Command::Heartbeat {
            workspace_foreground: true,
        };
        let mut envelope = Envelope {
            version: PROTOCOL_VERSION,
            binding: command_binding(online, &heartbeat),
            lease_epoch: config.lease_epoch,
            sequence: 1,
            reply_to: 0,
            message: Message::Command(heartbeat),
        };
        guard.update_session(None).expect("logout before receipt");
        assert!(guard.on_command(&envelope, 100).is_ok());
        envelope.sequence += 1;
        assert!(guard.on_command(&envelope, 200).is_ok(), "subsequent heartbeat continues");
        envelope.sequence += 1;
        envelope.binding = command_binding(online, &Command::Detach);
        envelope.message = Message::Command(Command::Detach);
        let reply = guard.on_command(&envelope, 300).expect("detach across logout").reply.expect("ack");
        assert_eq!(reply.event, Event::Detached);
    }
}
