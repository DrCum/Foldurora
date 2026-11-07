//! Private, bounded local transport. The controller and viewer only exchange
//! typed identities and state; all disk work and child launches stay off-frame.
//!
//! One outstanding command per worker preserves acknowledgments. Status is a
//! separate latest-value slot. The fresh WorkerId is the boot nonce; neither a
//! PID nor a window handle can authorize a command.

pub(crate) use super::chat_broker::HostChatSnapshot;
use super::review::{self, Grant as ReviewGrant, Status as ReviewStatus};
use super::slots::{Registry, SlotId, SlotLabel, SlotSummary};
use aurora_session::chat::{ChatKey, ChatSource, ChatText, MarkReadRequest, ReviewCapture};
use aurora_session::{
    AccountKey, Binding, Command, Coordinator, Effect, Envelope, Event, GridKey, HostId, HostNotice, HostedViewport, InstallationId,
    MAX_WORKERS, Message, PROTOCOL_VERSION, Phase, SessionId, SessionIdentity, WorkerId, WorkerIdentity, WorkerStatus,
};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

pub(crate) mod preview_control;
#[cfg(any(windows, test))]
use super::preview_projection::{
    self, BootstrapAdmission, BrokerProjection, PreviewError, PreviewReader, RequestKind, ViewDescriptor, ViewNamespace, ViewSetRequest,
};
// The joined Windows frontend captures ordinary Active frames only. Inactive
// rendering remains unavailable; an empty visible set creates no demand.
#[cfg(any(windows, test))]
const PREVIEW_CAPABILITY_JOINED: bool = cfg!(windows);

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
    pub slot: Option<SlotId>,
    pub binding: Binding,
    pub status: Option<WorkerStatus>,
    pub reachable: bool,
    pub label: String,
    pub grid: String,
}

#[derive(Clone, Default)]
pub(crate) struct HostSnapshot {
    pub slots: Vec<SlotSummary>,
    pub registry_error: Option<String>,
    pub attention_preferences: super::attention_prefs::PreferenceState,
    pub accounts: Vec<SavedAccount>,
    pub workers: Vec<WorkerSummary>,
    pub opening: Vec<AccountKey>,
    pub stopped: bool,
    pub error: Option<String>,
    /// No native adoption until a completed prepared frame can be proven.
    pub can_activate: bool,
    pub active: Option<WorkerId>,
    pub pending: Option<WorkerId>,
    /// Published only after the exact current transaction's grant is accepted.
    pub active_viewport: Option<HostedViewport>,
    pub completed_request: u64,
    pub chat: Vec<Arc<HostChatSnapshot>>,
    pub review_operations: Vec<review::Operation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostError {
    Busy,
    Unsupported,
    Stopped,
    Unavailable,
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "session controller: {self:?}")
    }
}
impl std::error::Error for HostError {}

enum HostIntent {
    #[cfg(any(windows, test))]
    Preview(RequestKind, preview_projection::OperationSlot),
    Open(Option<AccountKey>, Option<SlotId>),
    RenameSlot(SlotId, SlotLabel),
    SaveAttentionPreferences(super::attention_prefs::Preferences),
    Activate(WorkerId, HostedViewport),
    Viewport(HostedViewport),
    Park,
    Detach(WorkerId),
    ChatRead(MarkReadRequest),
    ReviewConversation(ReviewCapture, HostedViewport),
    CancelReview(u64),
    ChatSend {
        key: ChatKey,
        policy_revision: u64,
        text: ChatText,
    },
}

impl HostIntent {
    fn controls_presentation(&self) -> bool {
        match self {
            #[cfg(any(windows, test))]
            Self::Preview(_, _) => false,
            // Review reports its own body-free outcome. Its local failures
            // cannot cover an acknowledged world or clear a control error.
            Self::ChatSend { .. }
            | Self::ChatRead(_)
            | Self::ReviewConversation(_, _)
            | Self::CancelReview(_)
            | Self::RenameSlot(_, _)
            | Self::SaveAttentionPreferences(_) => false,
            Self::Open(_, _) | Self::Activate(_, _) | Self::Viewport(_) | Self::Park | Self::Detach(_) => true,
        }
    }
}

fn request_result(
    error: &mut Option<String>,
    controls_presentation: bool,
    registry_error: Option<&mut Option<String>>,
    result: anyhow::Result<()>,
) {
    if let Some(notice) = registry_error {
        *notice = result.err().map(|error| error.to_string());
    } else if controls_presentation {
        *error = result.err().map(|error| error.to_string());
    }
}

struct HostRequest {
    ticket: u64,
    intent: HostIntent,
    #[cfg(any(windows, test))]
    _preview_slot: Option<preview_control::CaptureQueueSlot>,
}

struct RequestSender {
    sender: Sender<HostRequest>,
    next: u64,
}

impl RequestSender {
    #[cfg(any(test, not(windows)))]
    fn send(&mut self, intent: HostIntent) -> Result<u64, HostError> {
        self.send_scoped(
            intent,
            #[cfg(any(windows, test))]
            None,
        )
    }
    fn send_scoped(
        &mut self,
        intent: HostIntent,
        #[cfg(any(windows, test))] preview_slot: Option<preview_control::CaptureQueueSlot>,
    ) -> Result<u64, HostError> {
        let ticket = self.next;
        let next = ticket.checked_add(1).ok_or(HostError::Busy)?;
        self.sender
            .try_send(HostRequest {
                ticket,
                intent,
                #[cfg(any(windows, test))]
                _preview_slot: preview_slot,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => HostError::Busy,
                TrySendError::Disconnected(_) => HostError::Stopped,
            })?;
        self.next = next;
        Ok(ticket)
    }
}

pub(crate) struct HostController {
    #[cfg(any(windows, test))]
    preview_frontdoor: Arc<preview_control::Frontdoor>,
    #[cfg(any(windows, test))]
    preview_reader: PreviewReader,
    requests: Mutex<RequestSender>,
    snapshot: Arc<Mutex<Arc<HostSnapshot>>>,
    stop: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    foreground: Arc<AtomicBool>,
}

/// In-process synthetic data and an intent inbox, exclusively for the ignored
/// native CPU-window test. No broker thread, profile, socket or worker starts.
#[cfg(all(test, target_os = "linux"))]
pub(super) mod native_fixture {
    use super::*;
    use crate::session_host::chat_broker::{ReadOperation, ReadStatus};
    use aurora_session::chat::{ReadOutcome, SendOutcome, SendReceipt};

    #[derive(Clone, Debug, Serialize)]
    pub(crate) enum Intent {
        Send {
            ticket: u64,
            key: ChatKey,
            policy_revision: u64,
            text: ChatText,
        },
        Read {
            ticket: u64,
            request: MarkReadRequest,
        },
    }

    pub(crate) struct Inbox {
        receiver: Receiver<HostRequest>,
        snapshot: Arc<Mutex<Arc<HostSnapshot>>>,
    }

    pub(crate) fn controller(snapshot: HostSnapshot) -> (HostController, Inbox) {
        let (sender, receiver) = bounded(MAX_QUEUE);
        let snapshot = Arc::new(Mutex::new(Arc::new(snapshot)));
        let preview_frontdoor = Arc::new(preview_control::Frontdoor::default());
        let preview_reader = PreviewReader::new(preview_frontdoor.clone());
        let controller = HostController {
            preview_frontdoor,
            preview_reader,
            requests: Mutex::new(RequestSender { sender, next: 1 }),
            snapshot: snapshot.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            stopped: Arc::new(AtomicBool::new(false)),
            foreground: Arc::new(AtomicBool::new(false)),
        };
        (controller, Inbox { receiver, snapshot })
    }

    impl Inbox {
        pub(crate) fn drain(&self) -> Vec<Intent> {
            self.receiver
                .try_iter()
                .map(|request| match request.intent {
                    HostIntent::ChatSend {
                        key,
                        policy_revision,
                        text,
                    } => Intent::Send {
                        ticket: request.ticket,
                        key,
                        policy_revision,
                        text,
                    },
                    HostIntent::ChatRead(mut read) => {
                        read.request = request.ticket;
                        Intent::Read {
                            ticket: request.ticket,
                            request: read,
                        }
                    }
                    _ => panic!("native chat fixture must never activate, open, detach or manage a worker"),
                })
                .collect()
        }

        /// Synthetic terminal rejection only; never execute or replay an intent.
        pub(crate) fn complete(&self, intents: &[Intent]) {
            let mut shared = self.snapshot.lock().unwrap();
            let snapshot = Arc::make_mut(&mut shared);
            for intent in intents {
                let (key, ticket) = match intent {
                    Intent::Send { key, ticket, .. } => (*key, *ticket),
                    Intent::Read { request, ticket } => (request.key, *ticket),
                };
                let Some(chat) = snapshot.chat.iter_mut().find(|chat| chat.source == key.source) else {
                    continue;
                };
                let chat = Arc::make_mut(chat);
                match intent {
                    Intent::Send { .. } if !chat.receipts.iter().any(|receipt| receipt.request == ticket) => {
                        chat.receipts.push(SendReceipt {
                            key,
                            request: ticket,
                            outcome: SendOutcome::RejectedNative,
                        });
                    }
                    Intent::Read { request, .. } if !chat.read_operations.iter().any(|operation| operation.request == *request) => {
                        chat.read_operations.push(ReadOperation {
                            request: *request,
                            status: ReadStatus::Completed(ReadOutcome::RejectedNative),
                        });
                    }
                    _ => {}
                }
            }
        }

        pub(crate) fn replace_source(&self, number: u8) {
            assert!((1..=5).contains(&number));
            let mut shared = self.snapshot.lock().unwrap();
            let snapshot = Arc::make_mut(&mut shared);
            let chat = Arc::make_mut(&mut snapshot.chat[usize::from(number - 1)]);
            let old = chat.source;
            chat.source.session.generation = old.session.generation.checked_add(1).unwrap();
            for info in &mut chat.conversations {
                info.policy.key.source = chat.source;
            }
            chat.receipts.clear();
            chat.read_operations.clear();
            for read in chat.reads.values_mut() {
                read.observed.epoch = read.observed.epoch.checked_add(1).unwrap();
            }
            snapshot
                .workers
                .iter_mut()
                .find(|worker| worker.binding == old.binding())
                .unwrap()
                .binding = chat.source.binding();
        }
    }
}

impl HostController {
    pub fn start() -> anyhow::Result<Self> {
        let (tx, rx) = bounded(MAX_QUEUE);
        let snapshot = Arc::new(Mutex::new(Arc::new(HostSnapshot::default())));
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let foreground = Arc::new(AtomicBool::new(false));
        let shared = snapshot.clone();
        let ending = stop.clone();
        let ended = stopped.clone();
        let focus = foreground.clone();
        #[cfg(any(windows, test))]
        let preview_frontdoor = Arc::new(preview_control::Frontdoor::default());
        #[cfg(any(windows, test))]
        let preview_reader = PreviewReader::new(preview_frontdoor.clone());
        #[cfg(any(windows, test))]
        let preview_reads = preview_reader.clone();
        #[cfg(any(windows, test))]
        let preview_fences = preview_frontdoor.clone();
        std::thread::Builder::new().name("session-host-io".into()).spawn(move || {
            if let Err(error) = host_loop(
                &rx,
                &shared,
                &ending,
                &focus,
                #[cfg(any(windows, test))]
                preview_fences,
                #[cfg(any(windows, test))]
                preview_reads,
            ) {
                let previous = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
                let mut next = (*previous).clone();
                next.error = Some(error.to_string());
                next.stopped = true;
                *shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(next);
            }
            ended.store(true, Ordering::Release);
        })?;
        Ok(Self {
            #[cfg(any(windows, test))]
            preview_frontdoor,
            #[cfg(any(windows, test))]
            preview_reader,
            requests: Mutex::new(RequestSender { sender: tx, next: 1 }),
            snapshot,
            stop,
            stopped,
            foreground,
        })
    }

    pub fn snapshot(&self) -> Arc<HostSnapshot> {
        self.snapshot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    pub fn open_manual_login(&self, account: Option<AccountKey>, slot: Option<SlotId>) -> Result<u64, HostError> {
        self.send(HostIntent::Open(account, slot))
    }

    pub fn rename_slot(&self, slot: SlotId, label: SlotLabel) -> Result<u64, HostError> {
        self.send(HostIntent::RenameSlot(slot, label))
    }

    pub fn save_attention_preferences(&self, preferences: super::attention_prefs::Preferences) -> Result<u64, HostError> {
        self.send(HostIntent::SaveAttentionPreferences(preferences))
    }

    pub fn activate(&self, worker: WorkerId, viewport: HostedViewport) -> Result<u64, HostError> {
        if !cfg!(windows) || viewport.validate().is_err() {
            return Err(HostError::Unsupported);
        }
        self.send(HostIntent::Activate(worker, viewport))
    }

    pub fn update_viewport(&self, viewport: HostedViewport) -> Result<u64, HostError> {
        if !cfg!(windows) || viewport.validate().is_err() {
            return Err(HostError::Unsupported);
        }
        self.send(HostIntent::Viewport(viewport))
    }

    pub fn park_active(&self) -> Result<u64, HostError> {
        self.send(HostIntent::Park)
    }

    /// Logical workspace foreground is independent of native keyboard focus.
    /// Repeated focus events replace one value instead of growing an IO queue.
    pub fn set_workspace_foreground(&self, foreground: bool) {
        self.foreground.store(foreground, Ordering::Release);
    }

    pub fn detach(&self, worker: WorkerId) -> Result<u64, HostError> {
        self.send(HostIntent::Detach(worker))
    }

    pub fn send_chat(&self, key: ChatKey, policy_revision: u64, text: ChatText) -> Result<u64, HostError> {
        let snapshot = self.snapshot();
        let allowed = snapshot.chat.iter().any(|chat| {
            chat.source == key.source
                && chat.authenticated
                && chat.live
                && chat.conversations.iter().any(|info| {
                    info.policy.key == key
                        && info.policy.revision == policy_revision
                        && info.policy.lease_current
                        && info.policy.ready
                        && info.policy.can_send
                })
        });
        if !allowed || key.validate().is_err() || text.as_str().trim().is_empty() {
            return Err(HostError::Unavailable);
        }
        self.send(HostIntent::ChatSend {
            key,
            policy_revision,
            text,
        })
    }

    pub fn mark_chat_read(&self, request: MarkReadRequest) -> Result<u64, HostError> {
        let snapshot = self.snapshot();
        let allowed = snapshot.chat.iter().any(|chat| {
            chat.source == request.key.source
                && chat.authenticated
                && chat.live
                && chat.conversations.iter().any(|info| {
                    info.policy.key == request.key
                        && info.policy.revision == request.policy_revision
                        && info.policy.lease_current
                        && info.policy.ready
                        && info.policy.can_read
                })
                && chat.reads.get(&request.key.conversation).is_some_and(|read| {
                    read.can_mark_read && read.observed == request.observed && read.unread_revision == request.unread_revision
                })
        });
        if !allowed {
            return Err(HostError::Unavailable);
        }
        // The broker assigns the operation ticket after FIFO admission.
        self.send(HostIntent::ChatRead(request))
    }

    /// Explicit native UI action. Shell closes its presentation gate on the
    /// returned FIFO ticket, just as for Activate; completion lives separately
    /// in review_operations, never in completed_request or active WorkerId.
    pub fn review_conversation(&self, target: ReviewCapture, viewport: HostedViewport) -> Result<u64, HostError> {
        if !cfg!(windows) || viewport.validate().is_err() {
            return Err(HostError::Unsupported);
        }
        let snapshot = self.snapshot();
        let bound = snapshot.workers.iter().any(|worker| {
            worker.binding == target.key.source.binding()
                && worker.reachable
                && worker
                    .status
                    .as_ref()
                    .is_some_and(|status| status.phase == Phase::Online && !status.blocked && status.native_window != 0)
        });
        if !bound || !snapshot.chat.iter().any(|chat| chat.permits_review(target)) {
            return Err(HostError::Unavailable);
        }
        self.send(HostIntent::ReviewConversation(target, viewport))
    }

    /// Cancels only this captured operation. Once native admission happened,
    /// cancellation means query-only uncertainty, not a promise to undo UI.
    pub fn cancel_review(&self, request: u64) -> Result<u64, HostError> {
        if request == 0 {
            return Err(HostError::Unavailable);
        }
        self.send(HostIntent::CancelReview(request))
    }

    fn send(&self, intent: HostIntent) -> Result<u64, HostError> {
        #[cfg(any(windows, test))]
        let capture = intent.controls_presentation() || matches!(intent, HostIntent::ReviewConversation(_, _) | HostIntent::Preview(_, _));
        #[cfg(any(windows, test))]
        let _attempt = capture.then(|| self.preview_frontdoor.begin_capture().ok()).flatten();
        #[cfg(any(windows, test))]
        let slot = if capture {
            // Preview-only retirement precedes contention/FIFO admission; an
            // image failure never changes the ordinary request result.
            self.preview_frontdoor.queue_slot().ok()
        } else {
            None
        };
        if self.stop.load(Ordering::Acquire) || self.stopped() {
            return Err(HostError::Stopped);
        }
        // The short try-lock keeps tickets in the channel's actual FIFO order
        // even if callers other than the shell enqueue an intent concurrently.
        let mut requests = self.requests.try_lock().map_err(|_| HostError::Busy)?;
        #[cfg(any(windows, test))]
        {
            requests.send_scoped(intent, slot)
        }
        #[cfg(not(any(windows, test)))]
        {
            requests.send(intent)
        }
    }

    /// Nonblocking: the IO thread requests detach, then retires after its
    /// bounded acknowledgment period. It never kills or logs out a worker.
    pub fn shutdown(&self) {
        #[cfg(any(windows, test))]
        self.preview_frontdoor.retire();
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
pub(super) struct ChatBootstrap {
    pub port: u16,
    pub key: super::chat_transport::ChatSecret,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkerConfig {
    pub version: u16,
    pub identity: WorkerIdentity,
    pub session: SessionId,
    pub generation: u64,
    /// Missing in preserved pre-slot bootstraps: keep their generated profile.
    #[serde(default)]
    pub slot: Option<SlotId>,
    pub expected_account: Option<AccountKey>,
    pub expected_grid: Option<GridKey>,
    pub lease_epoch: u64,
    pub host_pid: u32,
    #[serde(default)]
    pub chat: Option<ChatBootstrap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<super::preview_bootstrap::PreviewBootstrap>,
}

impl WorkerConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.version == PROTOCOL_VERSION && self.generation != 0 && self.lease_epoch != 0 && self.host_pid != 0,
            "invalid worker configuration"
        );
        anyhow::ensure!(
            self.chat.as_ref().is_none_or(|chat| chat.port != 0),
            "invalid private chat endpoint"
        );
        anyhow::ensure!(
            self.preview.as_ref().is_none_or(|preview| preview.port != 0),
            "invalid private preview endpoint"
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

    fn encode_for_launch(&mut self) -> anyhow::Result<Vec<u8>> {
        self.encode_with_limit(MAX_CONFIG)
    }
    fn encode_with_limit(&mut self, limit: usize) -> anyhow::Result<Vec<u8>> {
        self.validate()?;
        let mut bytes = serde_json::to_vec(self)?;
        if bytes.len() > limit && self.preview.take().is_some() {
            bytes = serde_json::to_vec(self)?;
        }
        anyhow::ensure!(bytes.len() <= limit, "worker configuration exceeds limit");
        Ok(bytes)
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

pub(super) fn read_json<T: DeserializeOwned>(path: &Path, limit: usize) -> io::Result<T> {
    serde_json::from_slice(&read_bytes(path, limit)?).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "malformed session file"))
}

fn publish(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::other("missing session directory"))?;
    super::profile::private_dir(parent)?;
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = super::profile::create_private_file(&temporary)?;
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

struct PendingCommand {
    envelope: Envelope,
    published: bool,
}

/// The command file is a stop-and-wait slot, never a latest-state mailbox.
/// Heartbeats have their own coalesced value and cannot erase a control step.
#[derive(Default)]
struct Outbox {
    pending: Option<PendingCommand>,
    controls: VecDeque<Envelope>,
    heartbeat: Option<Envelope>,
    highest: u64,
}

impl Outbox {
    fn enqueue(&mut self, envelope: Envelope) -> io::Result<()> {
        envelope.validate().map_err(io::Error::other)?;
        if envelope.sequence <= self.highest {
            return Err(io::Error::other("worker outbox sequence is stale"));
        }
        let Message::Command(command) = &envelope.message else {
            return Err(io::Error::other("worker outbox requires a command"));
        };
        if matches!(command, Command::Heartbeat { .. }) {
            if self
                .pending
                .as_ref()
                .is_some_and(|pending| matches!(pending.envelope.message, Message::Command(Command::Heartbeat { .. })))
            {
                self.pending = None;
            }
            self.highest = envelope.sequence;
            self.heartbeat = Some(envelope);
            return Ok(());
        }
        if matches!(command, Command::Revoke { .. } | Command::Detach) {
            // Rollback must revoke even when an earlier Prepare/Grant may
            // already have arrived. The worker rejects old activation epochs.
            self.pending = None;
            self.controls.clear();
            self.heartbeat = None;
        } else if self
            .pending
            .as_ref()
            .is_some_and(|pending| matches!(pending.envelope.message, Message::Command(Command::Heartbeat { .. })))
        {
            self.pending = None;
        }
        if self.controls.len() + usize::from(self.pending.is_some()) >= MAX_QUEUE {
            return Err(io::Error::other("worker command queue is full"));
        }
        self.highest = envelope.sequence;
        self.controls.push_back(envelope);
        Ok(())
    }

    fn next(&mut self) -> Option<Envelope> {
        if self.pending.is_none() {
            let envelope = self.controls.pop_front().or_else(|| self.heartbeat.take())?;
            if self
                .heartbeat
                .as_ref()
                .is_some_and(|heartbeat| heartbeat.sequence <= envelope.sequence)
            {
                self.heartbeat = None;
            }
            self.pending = Some(PendingCommand {
                envelope,
                published: false,
            });
        }
        self.pending
            .as_ref()
            .filter(|pending| !pending.published)
            .map(|pending| pending.envelope.clone())
    }

    fn published(&mut self, sequence: u64) {
        if let Some(pending) = &mut self.pending
            && pending.envelope.sequence == sequence
        {
            pending.published = true;
        }
    }

    fn acknowledge(&mut self, reply: &Envelope) -> bool {
        let Some(pending) = &self.pending else { return false };
        if !matches_acknowledgment(&pending.envelope, reply) {
            return false;
        }
        self.pending = None;
        true
    }
}

fn matches_acknowledgment(command: &Envelope, reply: &Envelope) -> bool {
    let (Message::Command(command_message), Message::Event(event)) = (&command.message, &reply.message) else {
        return false;
    };
    if reply.reply_to != command.sequence
        || reply.binding.worker != command.binding.worker
        || reply.lease_epoch != command.lease_epoch
        || (command_message.session_bound() && reply.binding != command.binding)
    {
        return false;
    }
    if matches!(event, Event::Rejected { .. }) {
        return event.activation() == command_message.activation();
    }
    match (command_message, event) {
        (Command::Heartbeat { .. } | Command::SetViewport { .. }, Event::Status(_)) | (Command::Detach, Event::Detached) => true,
        (
            Command::Revoke { activation },
            Event::Quiesced {
                activation: acknowledged, ..
            },
        )
        | (
            Command::Prepare { activation, .. },
            Event::FrameReady {
                activation: acknowledged, ..
            },
        )
        | (Command::Grant { activation }, Event::Granted { activation: acknowledged }) => activation == acknowledged,
        _ => false,
    }
}

struct Child {
    #[cfg(any(windows, test))]
    preview: preview_control::ChildPreview,
    config: WorkerConfig,
    directory: PathBuf,
    process: std::process::Child,
    binding: Binding,
    status: Option<WorkerStatus>,
    received: u64,
    last_seen: Option<Instant>,
    outbox: Outbox,
    prepared: Option<(Binding, u64, Option<HostedViewport>)>,
    granted: Option<(Binding, u64)>,
    detached: bool,
    exited: bool,
    reservation: Option<crate::characters::HostLaunchPermit>,
    next_reservation: Instant,
    label: String,
    grid: String,
    chat: super::chat_broker::Peer,
}

impl Child {
    fn accepts(&self, envelope: &Envelope) -> bool {
        self.config.accepts(envelope)
    }

    fn poll_process(&mut self) -> io::Result<bool> {
        if self.process.try_wait()?.is_some() {
            #[cfg(any(windows, test))]
            self.preview.retire();
            self.exited = true;
            self.detached = true;
            self.reservation = None;
            return Ok(true);
        }
        if Instant::now() >= self.next_reservation {
            if let Some(reservation) = &mut self.reservation
                && !reservation.refresh(self.process.id())?
            {
                self.reservation = None;
            }
            self.next_reservation = Instant::now() + HEARTBEAT;
        }
        Ok(false)
    }

    fn incoming(&self) -> Vec<Envelope> {
        // Read status FIRST. WorkerIo publishes replies before its status slot;
        // reading reply afterwards closes the cross-file publication race.
        let status = read_envelope(&self.directory.join("status.json")).ok();
        let reply = read_envelope(&self.directory.join("reply.json"));
        // A transiently unreadable reply file must not let a newer periodic
        // status consume the global sequence ahead of its control ack.
        let control_waiting = self
            .outbox
            .pending
            .as_ref()
            .is_some_and(|pending| !matches!(pending.envelope.message, Message::Command(Command::Heartbeat { .. })));
        let status = status.filter(|_| !control_waiting || reply.is_ok());
        ordered_incoming(status, reply.ok(), self.received)
            .into_iter()
            .filter(|envelope| self.accepts(envelope))
            .collect()
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(envelope) = self.outbox.next() {
            publish_envelope(&self.directory.join("command.json"), &envelope)?;
            self.outbox.published(envelope.sequence);
        }
        Ok(())
    }
}

fn ordered_incoming(status: Option<Envelope>, reply: Option<Envelope>, received: u64) -> Vec<Envelope> {
    let mut incoming: Vec<_> = status
        .into_iter()
        .chain(reply)
        .filter(|envelope| envelope.sequence > received)
        .collect();
    incoming.sort_by_key(|envelope| envelope.sequence);
    incoming.dedup_by_key(|envelope| envelope.sequence);
    incoming
}

#[cfg(test)]
fn command_binding(mut binding: Binding, command: &Command) -> Binding {
    if !command.session_bound() {
        binding.session = None;
    }
    binding
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenGate {
    Queued,
    Parking(WorkerId),
    Ready,
}

impl OpenGate {
    fn parked(&mut self, worker: WorkerId) {
        if *self == Self::Parking(worker) {
            *self = Self::Ready;
        }
    }

    fn can_launch(self, active: Option<WorkerId>, busy: bool) -> bool {
        self == Self::Ready && active.is_none() && !busy
    }
}

struct PendingOpen {
    slot: SlotId,
    account: Option<AccountKey>,
    gate: OpenGate,
    reservation: Option<crate::characters::HostLaunchPermit>,
}

#[derive(Clone, Copy)]
struct ActiveViewport {
    worker: WorkerId,
    binding: Binding,
    viewport: HostedViewport,
    activation: u64,
}

fn exact_review_grant(
    active: Option<ActiveViewport>,
    worker: Option<WorkerId>,
    viewport: Option<HostedViewport>,
    busy: bool,
    current_binding: Option<Binding>,
    granted: Option<(Binding, u64)>,
) -> Option<ReviewGrant> {
    let active = active?;
    (!busy
        && worker == Some(active.worker)
        && current_binding == Some(active.binding)
        && viewport == Some(active.viewport)
        && active.activation != 0
        && granted == Some((active.binding, active.activation)))
    .then_some(ReviewGrant {
        binding: active.binding,
        activation: active.activation,
        viewport: active.viewport,
    })
}

fn review_activation(effects: &[Effect]) -> Option<u64> {
    effects.iter().find_map(|effect| match effect {
        Effect::Send(Envelope {
            message: Message::Command(Command::Revoke { activation } | Command::Prepare { activation, .. }),
            ..
        }) if *activation != 0 => Some(*activation),
        _ => None,
    })
}

struct Broker {
    #[cfg(any(windows, test))]
    preview_frontdoor: Arc<preview_control::Frontdoor>,
    #[cfg(any(windows, test))]
    preview_projection: BrokerProjection,
    registry: Registry,
    registry_error: Option<String>,
    slot_available: [bool; MAX_WORKERS],
    next_slot_scan: Instant,
    coordinator: Coordinator,
    installation: InstallationId,
    host: HostId,
    children: Vec<Child>,
    opens: VecDeque<PendingOpen>,
    viewport: Option<HostedViewport>,
    viewport_dirty: bool,
    park_requested: bool,
    active_viewport: Option<ActiveViewport>,
    pending: Option<WorkerId>,
    error: Option<String>,
    completed_request: u64,
    chat_listener: Option<super::chat_transport::Listener>,
    reviews: review::Transactions,
}

impl Broker {
    fn accepted_viewport(&self, worker: WorkerId) -> Option<ActiveViewport> {
        let child = self.children.iter().find(|child| child.config.identity.worker == worker)?;
        let (binding, activation, viewport) = child.prepared?;
        let viewport = viewport?;
        (binding == child.binding && child.granted == Some((binding, activation)) && self.viewport == Some(viewport)).then_some(
            ActiveViewport {
                worker,
                binding,
                viewport,
                activation,
            },
        )
    }

    fn review_child(&self, target: ReviewCapture) -> Option<usize> {
        let source = target.key.source;
        let live = self.coordinator.workers().any(|entry| {
            entry.live
                && entry.binding == source.binding()
                && entry.status.phase == Phase::Online
                && !entry.status.blocked
                && entry.status.native_window != 0
        });
        if !live {
            return None;
        }
        self.children.iter().position(|child| {
            child.config.identity == source.worker
                && child.config.lease_epoch == source.lease_epoch
                && child.binding == source.binding()
                && !child.detached
                && !child.exited
                && child.last_seen.is_some_and(|seen| seen.elapsed() <= REACHABLE)
        })
    }

    fn review_grant(&self, target: ReviewCapture) -> Option<ReviewGrant> {
        let child = &self.children[self.review_child(target)?];
        exact_review_grant(
            self.active_viewport,
            self.coordinator.active(),
            self.viewport,
            self.coordinator.busy(),
            Some(child.binding),
            child.granted,
        )
    }

    fn begin_review(&mut self, target: ReviewCapture, viewport: HostedViewport, ticket: u64, now: u64) -> anyhow::Result<()> {
        if !self.reviews.begin(target, ticket, viewport, now) {
            return Ok(());
        }
        if self.coordinator.busy() || !self.opens.is_empty() || self.park_requested || self.viewport_dirty {
            self.reviews.interrupt(ReviewStatus::Busy);
            return Ok(());
        }
        let Some(index) = self
            .review_child(target)
            .filter(|index| self.children[*index].chat.review_allowed(target))
        else {
            self.reviews.interrupt(ReviewStatus::Unavailable);
            return Ok(());
        };
        for child in &mut self.children {
            child.chat.watch_review(None);
        }
        self.children[index].chat.watch_review(Some(target));
        let effects = match self.coordinator.activate(target.key.source.worker.worker, Some(viewport), now) {
            Ok(effects) => effects,
            Err(error) => {
                self.reviews.interrupt(if error == aurora_session::Error::Busy {
                    ReviewStatus::Busy
                } else {
                    ReviewStatus::Unavailable
                });
                return Ok(());
            }
        };
        if effects.is_empty() {
            // Coordinator's same-active shortcut is not itself a grant proof.
            // Require the retained exact native placement and accepted child
            // Granted receipt without issuing an extra GPU transaction.
            if let Some(grant) = self.review_grant(target).filter(|grant| grant.viewport == viewport) {
                self.reviews.expect_activation(grant.activation);
                self.reviews.granted(grant);
            } else {
                self.reviews.interrupt(ReviewStatus::Unavailable);
            }
        } else if let Some(activation) = review_activation(&effects) {
            self.reviews.expect_activation(activation);
        } else {
            self.reviews.interrupt(ReviewStatus::Unavailable);
        }
        self.viewport = Some(viewport);
        self.viewport_dirty = false;
        self.effects(effects)?;
        Ok(())
    }

    fn poll_review(&mut self, now: u64) {
        let Some(target) = self.reviews.target() else { return };
        let index = self.review_child(target);
        let current = index.is_some_and(|index| {
            let peer = &self.children[index].chat;
            if self.reviews.waiting() {
                peer.watched_review_current(target)
            } else {
                peer.review_allowed(target)
            }
        });
        self.reviews.tick(now, current);
        if !current {
            self.reviews.interrupt(ReviewStatus::Unavailable);
        }
        if self.reviews.waiting() {
            let Some(grant) = self.review_grant(target) else { return };
            let Some(request) = self.reviews.dispatch(grant) else { return };
            let Some(index) = index else { return };
            match self.children[index].chat.admit_review(request, Instant::now()) {
                Ok(()) => self.reviews.queued(request),
                Err(error) => self.reviews.interrupt(if error == HostError::Busy {
                    ReviewStatus::Busy
                } else {
                    ReviewStatus::Unavailable
                }),
            }
        }
        if let Some(request) = self.reviews.full_request() {
            let delivery = index.and_then(|index| self.children[index].chat.review_delivery(request));
            self.reviews.delivery(delivery);
        }
    }

    fn effects(&mut self, effects: Vec<Effect>) -> io::Result<()> {
        for effect in effects {
            match effect {
                Effect::Send(envelope) => {
                    if let Message::Command(
                        Command::Revoke { activation } | Command::Prepare { activation, .. } | Command::Grant { activation },
                    ) = &envelope.message
                    {
                        #[cfg(any(windows, test))]
                        {
                            let _ = self.preview_frontdoor.capture_intent();
                            self.preview_projection.control_activation(*activation);
                        }
                        self.reviews.control_activation(*activation);
                    }
                    let worker = envelope.binding.worker.worker;
                    let child = self
                        .children
                        .iter_mut()
                        .find(|child| child.config.identity.worker == worker)
                        .ok_or_else(|| io::Error::other("session worker is no longer owned"))?;
                    match &envelope.message {
                        Message::Command(Command::Revoke { .. }) => {
                            child.granted = None;
                            self.active_viewport = None;
                            self.pending = Some(worker);
                        }
                        Message::Command(Command::Prepare { activation, viewport }) => {
                            child.granted = None;
                            self.active_viewport = None;
                            self.pending = Some(worker);
                            child.prepared = Some((envelope.binding, *activation, *viewport));
                        }
                        Message::Command(Command::Grant { .. }) => self.pending = Some(worker),
                        Message::Command(Command::Detach) => {
                            #[cfg(any(windows, test))]
                            {
                                child.preview.retire();
                                self.preview_projection.exchange_lost();
                            }
                            self.reviews.interrupt(ReviewStatus::Unavailable);
                            child.chat.set_control(None, false);
                            child.granted = None;
                            if self.active_viewport.is_some_and(|active| active.worker == worker) {
                                self.active_viewport = None;
                            }
                        }
                        _ => {}
                    }
                    child.outbox.enqueue(envelope)?;
                }
                Effect::Notice(HostNotice::Switched(worker)) => {
                    self.error = None;
                    self.pending = None;
                    self.active_viewport = self.accepted_viewport(worker);
                    if let Some(active) = self.active_viewport {
                        self.reviews.granted(ReviewGrant {
                            binding: active.binding,
                            activation: active.activation,
                            viewport: active.viewport,
                        });
                        #[cfg(any(windows, test))]
                        if let Some(source) = self.preview_view_sources(Instant::now()).into_iter().find(|source| {
                            source.source().worker() == active.binding.worker && Some(source.source().session()) == active.binding.session
                        }) {
                            self.preview_projection
                                .exchange_granted(&source, active.activation, active.viewport, false);
                        }
                    } else {
                        self.reviews.interrupt(ReviewStatus::Unavailable);
                    }
                }
                Effect::Notice(HostNotice::RolledBack(worker)) => {
                    self.reviews.interrupt(ReviewStatus::Cancelled);
                    self.error = None;
                    self.pending = None;
                    self.active_viewport = self.accepted_viewport(worker);
                    #[cfg(any(windows, test))]
                    if let Some(active) = self.active_viewport
                        && let Some(source) = self.preview_view_sources(Instant::now()).into_iter().find(|source| {
                            source.source().worker() == active.binding.worker && Some(source.source().session()) == active.binding.session
                        })
                    {
                        self.preview_projection
                            .exchange_granted(&source, active.activation, active.viewport, true);
                    }
                }
                Effect::Notice(HostNotice::Parked(worker)) => {
                    self.reviews.interrupt(ReviewStatus::Cancelled);
                    self.error = None;
                    self.pending = None;
                    self.active_viewport = None;
                    if let Some(open) = self.opens.front_mut() {
                        open.gate.parked(worker);
                    }
                }
                Effect::Notice(HostNotice::Failed | HostNotice::Detached | HostNotice::Expired(_) | HostNotice::Blocked(_)) => {
                    #[cfg(any(windows, test))]
                    {
                        let _ = self.preview_frontdoor.capture_intent();
                        self.preview_projection.exchange_lost();
                    }
                    self.reviews.interrupt(ReviewStatus::Unavailable);
                    self.pending = None;
                    self.active_viewport = None;
                    self.opens.clear();
                }
            }
        }
        Ok(())
    }

    fn intent(&mut self, intent: HostIntent, ticket: u64, now: u64) -> anyhow::Result<()> {
        #[cfg(any(windows, test))]
        if intent.controls_presentation() || matches!(intent, HostIntent::ReviewConversation(_, _)) {
            self.preview_projection.external_intent();
        }
        if matches!(
            intent,
            HostIntent::Open(_, _) | HostIntent::Activate(_, _) | HostIntent::Viewport(_) | HostIntent::Park | HostIntent::Detach(_)
        ) {
            self.reviews.interrupt(ReviewStatus::Cancelled);
        }
        match intent {
            #[cfg(any(windows, test))]
            HostIntent::Preview(kind, slot) => self.preview_projection.queue(ticket, kind, slot),
            HostIntent::Open(account, requested) => {
                if self.opens.len() >= MAX_QUEUE {
                    anyhow::bail!(HostError::Busy);
                }
                if !self
                    .opens
                    .iter()
                    .any(|open| account.is_some() && open.account == account || requested == Some(open.slot))
                {
                    let slot = self.choose_slot(requested)?;
                    self.opens.push_back(PendingOpen {
                        slot,
                        account,
                        gate: OpenGate::Queued,
                        reservation: None,
                    });
                }
            }
            HostIntent::RenameSlot(slot, label) => {
                self.registry.rename(slot, label)?;
            }
            HostIntent::SaveAttentionPreferences(preferences) => {
                self.registry.save_attention_preferences(ticket, preferences);
            }
            HostIntent::ReviewConversation(target, viewport) => {
                self.begin_review(target, viewport, ticket, now)?;
            }
            HostIntent::CancelReview(request) => self.reviews.cancel(request),
            HostIntent::Activate(worker, viewport) => {
                let effects = self.coordinator.activate(worker, Some(viewport), now)?;
                self.viewport = Some(viewport);
                self.viewport_dirty = false;
                self.effects(effects)?;
            }
            HostIntent::Viewport(viewport) => {
                // Explicit native geometry events also invalidate a placement
                // that happens to return to the original desktop rectangle.
                self.viewport = Some(viewport);
                self.viewport_dirty = true;
                self.active_viewport = None;
            }
            HostIntent::Park => {
                self.active_viewport = None;
                self.viewport = None;
                self.viewport_dirty = false;
                self.park_requested = true;
            }
            HostIntent::Detach(worker) => {
                let effects = self.coordinator.detach(worker)?;
                self.effects(effects)?;
            }
            HostIntent::ChatRead(mut request) => {
                request.request = ticket;
                let key = request.key;
                let live = self
                    .coordinator
                    .workers()
                    .any(|entry| entry.live && entry.binding == key.source.binding() && entry.status.phase == Phase::Online);
                let child = self
                    .children
                    .iter_mut()
                    .find(|child| child.config.identity == key.source.worker && child.config.lease_epoch == key.source.lease_epoch)
                    .ok_or(HostError::Unavailable)?;
                if !live || child.detached || child.exited || child.last_seen.is_none_or(|seen| seen.elapsed() > REACHABLE) {
                    child.chat.set_control(None, false);
                    anyhow::bail!(HostError::Unavailable);
                }
                // The UI already owns this FIFO ticket. The peer publishes a
                // terminal local rejection if native enqueue is no longer valid.
                child.chat.admit_read(request, Instant::now())?;
            }
            HostIntent::ChatSend {
                key,
                policy_revision,
                text,
            } => {
                let live = self
                    .coordinator
                    .workers()
                    .any(|entry| entry.live && entry.binding == key.source.binding() && entry.status.phase == Phase::Online);
                let child = self
                    .children
                    .iter_mut()
                    .find(|child| child.config.identity == key.source.worker && child.config.lease_epoch == key.source.lease_epoch)
                    .ok_or(HostError::Unavailable)?;
                if !live || child.detached || child.exited || child.last_seen.is_none_or(|seen| seen.elapsed() > REACHABLE) {
                    child.chat.set_control(None, false);
                    anyhow::bail!(HostError::Unavailable);
                }
                child.chat.send(key, policy_revision, text, ticket, Instant::now())?;
            }
        }
        Ok(())
    }

    fn poll(&mut self, now: u64) -> anyhow::Result<()> {
        // Keep the process handles in this broker until exit has actually been
        // observed; a detached ordinary window is never a kill permission.
        for index in 0..self.children.len() {
            if !self.children[index].exited && self.children[index].poll_process()? {
                #[cfg(any(windows, test))]
                self.preview_projection.exited(self.children[index].config.identity);
                let effects = self.coordinator.remove(self.children[index].config.identity.worker);
                self.effects(effects)?;
            }
            if self.children[index].exited || self.children[index].detached {
                continue;
            }
            let incoming = self.children[index].incoming();
            for envelope in incoming {
                let observation = self.coordinator.observe(&envelope, now);
                // Coordinator already marked a detached entry non-live when
                // emitting Detach. Its exact correlated native ack still lets
                // the IO broker retire delivery, never grant new authority.
                let detach_ack = matches!(envelope.message, Message::Event(Event::Detached))
                    && self.children[index]
                        .outbox
                        .pending
                        .as_ref()
                        .is_some_and(|pending| matches_acknowledgment(&pending.envelope, &envelope));
                let effects = match observation {
                    Ok(effects) => effects,
                    Err(_) if detach_ack => Vec::new(),
                    Err(_) => continue,
                };
                let child = &mut self.children[index];
                child.received = envelope.sequence;
                child.last_seen = Some(Instant::now());
                let acknowledged = child.outbox.acknowledge(&envelope);
                if acknowledged && let Message::Event(Event::Granted { activation }) = &envelope.message {
                    child.granted = Some((envelope.binding, *activation));
                }
                if matches!(envelope.message, Message::Event(Event::Detached)) {
                    child.detached = true;
                }
                if let Some(entry) = self
                    .coordinator
                    .workers()
                    .find(|entry| entry.binding.worker.worker == child.config.identity.worker)
                {
                    #[cfg(any(windows, test))]
                    if child.status.as_ref().is_some_and(|old| {
                        old.native_window != entry.status.native_window
                            || old.blocked != entry.status.blocked
                            || old.phase != entry.status.phase
                    }) {
                        let _ = self.preview_frontdoor.capture_intent();
                    }
                    #[cfg(any(windows, test))]
                    if child.binding != entry.binding || !entry.live || entry.status.phase != Phase::Online {
                        child.preview.retire();
                    }
                    child.binding = entry.binding;
                    child.status = Some(entry.status.clone());
                }
                self.effects(effects)?;
            }
        }
        self.children.retain(|child| !child.exited);
        let effects = self.coordinator.tick(now);
        self.effects(effects)?;
        self.poll_chat();
        self.poll_review(now);
        Ok(())
    }

    fn poll_chat(&mut self) {
        let now = Instant::now();
        for child in &mut self.children {
            let entry = self
                .coordinator
                .workers()
                .find(|entry| entry.binding.worker == child.config.identity);
            let live = !child.detached
                && !child.exited
                && child.last_seen.is_some_and(|seen| seen.elapsed() <= REACHABLE)
                && entry.is_some_and(|entry| entry.live && entry.status.phase == Phase::Online);
            let source = entry
                .filter(|_| live)
                .and_then(|entry| ChatSource::try_from((entry.binding, child.config.lease_epoch)).ok());
            child.chat.set_control(source, live);
            child.chat.poll(now);
        }
        let Some(listener) = &mut self.chat_listener else {
            return;
        };
        let permits: Vec<_> = self
            .children
            .iter()
            .filter(|child| child.chat.accepting())
            .filter_map(|child| {
                child.config.chat.as_ref().map(|chat| super::chat_transport::Permit {
                    identity: child.config.identity,
                    lease_epoch: child.config.lease_epoch,
                    secret: chat.key.clone(),
                })
            })
            .collect();
        match listener.poll(&permits) {
            Ok(Some(io)) => {
                if let Some(child) = self
                    .children
                    .iter_mut()
                    .find(|child| child.config.identity == io.identity() && child.config.lease_epoch == io.lease_epoch())
                {
                    let _ = child.chat.attach(io);
                }
            }
            Ok(None) => {}
            Err(_) => log::warn!("session chat listener unavailable; control ownership is unchanged"),
        }
    }

    fn reconcile(&mut self, accounts: &[(SavedAccount, crate::login_accounts::Account)], now: u64) -> anyhow::Result<()> {
        if self.viewport_dirty
            && !self.park_requested
            && self.opens.is_empty()
            && let (Some(worker), Some(viewport)) = (self.coordinator.active().or(self.pending), self.viewport)
        {
            match self.coordinator.refresh_viewport(worker, viewport, now) {
                Ok(effects) => {
                    self.viewport_dirty = false;
                    self.effects(effects)?;
                }
                Err(aurora_session::Error::Busy) => {}
                Err(error) => return Err(error.into()),
            }
        }
        if self.coordinator.busy() {
            return Ok(());
        }
        if self.park_requested {
            self.park_requested = false;
            let effects = self.coordinator.park_active(now)?;
            self.effects(effects)?;
            if self.coordinator.busy() {
                return Ok(());
            }
        }
        if let Some(open) = self.opens.front() {
            if matches!(open.gate, OpenGate::Parking(_)) {
                // Only the exact accepted Quiesced -> Parked transition above
                // can change this gate to Ready, never a timeout/empty active.
                self.opens.clear();
                anyhow::bail!("session worker was not acknowledged parked");
            }
            self.open_next(accounts, now)?;
            if self.coordinator.busy() || !self.opens.is_empty() {
                return Ok(());
            }
        }
        Ok(())
    }

    fn choose_slot(&self, requested: Option<SlotId>) -> anyhow::Result<SlotId> {
        for slot in SlotId::all().filter(|slot| requested.is_none_or(|wanted| wanted == *slot)) {
            if !self.opens.iter().any(|open| open.slot == slot)
                && !self.children.iter().any(|child| !child.exited && child.config.slot == Some(slot))
                && self.registry.available(slot)?
            {
                return Ok(slot);
            }
        }
        anyhow::bail!(aurora_i18n::tr!("host-profile-occupied-help"))
    }

    fn scan_slots(&mut self) {
        if Instant::now() < self.next_slot_scan {
            return;
        }
        self.next_slot_scan = Instant::now() + HEARTBEAT;
        for slot in SlotId::all() {
            // Inaccessible/invalid private paths stay unavailable. Surface a
            // refusal without repairing them or scanning ordinary profiles.
            let available = match self.registry.available(slot) {
                Ok(available) => available,
                Err(error) => {
                    self.registry_error = Some(error.to_string());
                    false
                }
            };
            self.slot_available[usize::from(u8::from(slot))] = available;
        }
    }

    fn open_next(&mut self, accounts: &[(SavedAccount, crate::login_accounts::Account)], now: u64) -> anyhow::Result<()> {
        let Some(mut open) = self.opens.pop_front() else { return Ok(()) };
        let account = open.account.and_then(|key| accounts.iter().find(|(saved, _)| saved.key == key));
        anyhow::ensure!(
            open.account.is_none() || account.is_some(),
            "saved worker account is no longer available"
        );
        if open.gate == OpenGate::Queued {
            if let Some((saved, _)) = account {
                let executable = std::env::current_exe()?.canonicalize()?;
                match crate::characters::reserve_host_launch(&executable, &saved.key.to_string())? {
                    crate::characters::HostLaunchReservation::Permit(permit) => open.reservation = Some(permit),
                    crate::characters::HostLaunchReservation::Existing(notice) => {
                        if matches!(
                            notice,
                            crate::characters::Notice::OtherGrid | crate::characters::Notice::Busy | crate::characters::Notice::Failed
                        ) {
                            anyhow::bail!("saved worker launch is reserved by another character window");
                        }
                        return Ok(());
                    }
                }
            }
            anyhow::ensure!(self.children.len() < MAX_WORKERS, "session worker capacity reached");
            let active = self.coordinator.active();
            let effects = self.coordinator.park_active(now)?;
            open.gate = active.map_or(OpenGate::Ready, OpenGate::Parking);
            self.opens.push_front(open);
            self.effects(effects)?;
            return Ok(());
        }
        anyhow::ensure!(
            open.gate.can_launch(self.coordinator.active(), self.coordinator.busy()),
            "worker launch requires an acknowledged parked renderer"
        );
        // Recheck after the GPU park, immediately before process creation.
        // The child takes the lifetime lock and validates the controller stamp
        // before exposing profile paths to settings/cache initialization.
        self.choose_slot(Some(open.slot))?;
        let mut config = new_config(
            self.installation,
            self.host,
            account,
            self.chat_listener.as_ref().map(|listener| listener.port()),
        )?;
        #[cfg(any(windows, test))]
        match self.preview_projection.bootstrap() {
            BootstrapAdmission::Pending => {
                self.opens.push_front(open);
                return Ok(());
            }
            BootstrapAdmission::Ready(bootstrap) => config.preview = Some(bootstrap),
            BootstrapAdmission::Absent => {}
        }
        config.slot = Some(open.slot);
        self.coordinator.register(config.identity.worker, config.lease_epoch, now)?;
        let worker = config.identity.worker;
        match launch(config, account, open.reservation) {
            Ok(child) => self.children.push(child),
            Err(error) => {
                let effects = self.coordinator.remove(worker);
                self.effects(effects)?;
                return Err(error);
            }
        }
        Ok(())
    }

    fn fail_closed(&mut self, error: &dyn std::fmt::Display) {
        self.reviews.interrupt(ReviewStatus::Unavailable);
        self.error = Some(error.to_string());
        self.active_viewport = None;
        self.opens.clear();
        let effects = self.coordinator.detach_all();
        if let Err(error) = self.effects(effects) {
            log::warn!("session host detach could not be queued: {error}");
        }
    }

    fn snapshot(&self, accounts: &[(SavedAccount, crate::login_accounts::Account)], stopped: bool) -> HostSnapshot {
        let workers: Vec<_> = self
            .children
            .iter()
            .filter(|child| !child.detached)
            .map(|child| WorkerSummary {
                slot: child.config.slot,
                binding: child.binding,
                status: child.status.clone(),
                reachable: child.last_seen.is_some_and(|seen| seen.elapsed() <= REACHABLE),
                label: child.label.clone(),
                grid: child.grid.clone(),
            })
            .collect();
        let can_activate = cfg!(windows)
            && !stopped
            && workers.iter().any(|worker| {
                worker.reachable
                    && worker.binding.session.is_some()
                    && worker
                        .status
                        .as_ref()
                        .is_some_and(|status| status.phase == Phase::Online && status.native_window != 0 && !status.blocked)
            });
        let active_viewport = self
            .active_viewport
            .filter(|active| {
                !self.coordinator.busy()
                    && {
                        #[cfg(any(windows, test))]
                        {
                            !self.preview_projection.exchange_waiting()
                        }
                        #[cfg(not(any(windows, test)))]
                        {
                            true
                        }
                    }
                    && self.coordinator.active() == Some(active.worker)
                    && self.viewport == Some(active.viewport)
                    && workers.iter().any(|worker| worker.reachable && worker.binding == active.binding)
                    && self
                        .children
                        .iter()
                        .any(|child| child.binding == active.binding && child.granted == Some((active.binding, active.activation)))
            })
            .map(|active| active.viewport);
        let mut opening: Vec<_> = self.opens.iter().filter_map(|open| open.account).collect();
        opening.extend(
            self.children
                .iter()
                .filter(|child| !child.detached && child.status.is_none())
                .filter_map(|child| child.config.expected_account),
        );
        opening.sort();
        opening.dedup();
        HostSnapshot {
            registry_error: self.registry_error.clone(),
            attention_preferences: self.registry.attention_preferences().clone(),
            slots: SlotId::all()
                .map(|id| SlotSummary {
                    id,
                    label: self.registry.label(id).clone(),
                    available: self.slot_available[usize::from(u8::from(id))]
                        && !self.opens.iter().any(|open| open.slot == id)
                        && !self.children.iter().any(|child| !child.exited && child.config.slot == Some(id)),
                })
                .collect(),
            accounts: accounts.iter().map(|(saved, _)| saved.clone()).collect(),
            workers,
            opening,
            stopped,
            error: self.error.clone(),
            can_activate,
            active: self.coordinator.active(),
            pending: self.coordinator.busy().then_some(self.pending).flatten(),
            active_viewport,
            completed_request: self.completed_request,
            chat: self.children.iter().filter_map(|child| child.chat.snapshot()).collect(),
            review_operations: self.reviews.operations(),
        }
    }
}

fn host_loop(
    rx: &Receiver<HostRequest>,
    shared: &Arc<Mutex<Arc<HostSnapshot>>>,
    stop: &AtomicBool,
    foreground: &AtomicBool,
    #[cfg(any(windows, test))] preview_frontdoor: Arc<preview_control::Frontdoor>,
    #[cfg(any(windows, test))] preview_reader: PreviewReader,
) -> anyhow::Result<()> {
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
    let registry = Registry::open(installation, host)?;
    let mut broker = Broker {
        #[cfg(any(windows, test))]
        preview_frontdoor,
        #[cfg(any(windows, test))]
        preview_projection: BrokerProjection::new(preview_reader, installation, host, PREVIEW_CAPABILITY_JOINED),
        registry,
        registry_error: None,
        slot_available: [false; MAX_WORKERS],
        next_slot_scan: Instant::now(),
        coordinator: Coordinator::new(installation, host),
        installation,
        host,
        children: Vec::new(),
        opens: VecDeque::new(),
        viewport: None,
        viewport_dirty: false,
        park_requested: false,
        active_viewport: None,
        pending: None,
        error: None,
        completed_request: 0,
        reviews: review::Transactions::default(),
        chat_listener: match super::chat_transport::Listener::bind() {
            Ok(listener) => Some(listener),
            Err(_) => {
                log::warn!("session chat listener could not start; native chat remains unavailable");
                None
            }
        },
    };
    // Prepare a process-only reaper before the first child can be spawned.
    // After shutdown it refreshes cold launch permits and observes exits;
    // it owns no GPU, grid session or controller lease and never kills.
    let (reap, receiver) = bounded(MAX_WORKERS);
    std::thread::Builder::new()
        .name("session-host-reap".into())
        .spawn(move || reap_children(receiver))?;
    let began = Instant::now();
    let mut closing = None;
    loop {
        let now = began.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        broker.coordinator.set_workspace_foreground(foreground.load(Ordering::Acquire));
        if stop.load(Ordering::Acquire) && closing.is_none() {
            #[cfg(any(windows, test))]
            broker.preview_projection.stop();
            broker.reviews.interrupt(ReviewStatus::Cancelled);
            closing = Some(Instant::now() + CLOSE_WAIT);
            broker.opens.clear();
            let effects = broker.coordinator.detach_all();
            if let Err(failure) = broker.effects(effects) {
                broker.error = Some(failure.to_string());
            }
        }
        broker.scan_slots();
        for request in rx.try_iter().take(MAX_QUEUE) {
            if closing.is_some() {
                break;
            }
            let controls_presentation = request.intent.controls_presentation();
            let registry_request = matches!(request.intent, HostIntent::RenameSlot(_, _));
            let result = broker.intent(request.intent, request.ticket, now);
            // Metadata and chat errors cannot cover the acknowledged world or
            // clear an unrelated presentation/control failure.
            request_result(
                &mut broker.error,
                controls_presentation,
                registry_request.then_some(&mut broker.registry_error),
                result,
            );
            broker.completed_request = request.ticket;
        }
        if let Err(failure) = broker.poll(now) {
            broker.fail_closed(&failure);
        }
        if closing.is_none()
            && let Err(failure) = broker.reconcile(&accounts, now)
        {
            broker.error = Some(failure.to_string());
            broker.opens.clear();
        }
        #[cfg(any(windows, test))]
        broker.poll_previews(now);
        for child in &mut broker.children {
            if !child.detached
                && let Err(failure) = child.flush()
            {
                broker.error = Some(failure.to_string());
            }
        }
        let stopped = closing.is_some_and(|deadline| Instant::now() >= deadline || broker.children.iter().all(|child| child.detached));
        *shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(broker.snapshot(&accounts, stopped));
        if stopped {
            for child in broker.children.drain(..) {
                if let Err(error) = reap.send(child) {
                    // The prepared reaper unexpectedly ended. Keep ownership
                    // here and reap normally; never terminate the viewer.
                    let mut child = error.0;
                    let _ = child.process.wait();
                }
            }
            break;
        }
        std::thread::sleep(PERIOD);
    }
    Ok(())
}

fn new_config(
    installation: InstallationId,
    host: HostId,
    account: Option<&(SavedAccount, crate::login_accounts::Account)>,
    chat_port: Option<u16>,
) -> anyhow::Result<WorkerConfig> {
    let config = WorkerConfig {
        version: PROTOCOL_VERSION,
        identity: WorkerIdentity {
            installation,
            host,
            worker: WorkerId::from_bytes(*uuid::Uuid::new_v4().as_bytes())?,
        },
        session: SessionId::from_bytes(*uuid::Uuid::new_v4().as_bytes())?,
        generation: 1,
        slot: None,
        expected_account: account.map(|(saved, _)| saved.key),
        expected_grid: account.map(|(_, account)| grid_key(&account.uri)).transpose()?,
        lease_epoch: 1,
        host_pid: std::process::id(),
        chat: chat_port
            .map(|port| super::chat_transport::ChatSecret::random().map(|key| ChatBootstrap { port, key }))
            .transpose()?,
        preview: None,
    };
    config.validate()?;
    Ok(config)
}

fn launch(
    mut config: WorkerConfig,
    account: Option<&(SavedAccount, crate::login_accounts::Account)>,
    reservation: Option<crate::characters::HostLaunchPermit>,
) -> anyhow::Result<Child> {
    let directory = config.directory()?;
    super::profile::private_dir(&directory)?;
    let path = directory.join("bootstrap.json");
    let encoded = config.encode_for_launch()?;
    publish(&path, &encoded)?;
    let mut command = std::process::Command::new(std::env::current_exe()?.canonicalize()?);
    command.arg("--host-worker-config").arg(&path);
    if let Some(key) = config.expected_account {
        command.arg("--saved-account").arg(key.to_string());
    }
    if let Some(language) = crate::cli::get().language.as_deref() {
        command.arg("--language").arg(language);
    }
    super::startup::prepare_worker(&mut command);
    // Inherited environment is unchanged; no login values, password arguments,
    // user input injection or credential-store access occur in the controller.
    let process = command.spawn()?;
    let binding = Binding {
        worker: config.identity,
        session: None,
    };
    let chat = super::chat_broker::Peer::new(config.identity, config.lease_epoch);
    Ok(Child {
        #[cfg(any(windows, test))]
        preview: preview_control::ChildPreview::new(),
        config,
        directory,
        process,
        binding,
        status: None,
        received: 0,
        last_seen: None,
        outbox: Outbox::default(),
        prepared: None,
        granted: None,
        detached: false,
        exited: false,
        reservation,
        next_reservation: Instant::now(),
        label: account.map_or_else(String::new, |(saved, _)| saved.label.clone()),
        grid: account.map_or_else(String::new, |(saved, _)| saved.grid.clone()),
        chat,
    })
}

fn reap_children(receiver: Receiver<Child>) {
    let mut children: Vec<Child> = Vec::new();
    let mut disconnected = false;
    loop {
        if !disconnected {
            match receiver.recv_timeout(HEARTBEAT) {
                Ok(child) => children.push(child),
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => disconnected = true,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            }
        } else {
            std::thread::sleep(HEARTBEAT);
        }
        for child in &mut children {
            if let Err(error) = child.poll_process() {
                log::warn!("session child exit/reservation observation failed: {error}");
            }
        }
        children.retain(|child| !child.exited);
        if disconnected && children.is_empty() {
            return;
        }
    }
}

#[cfg(test)]
mod review_tests;

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
            slot: None,
            expected_account: Some(AccountKey::from_bytes([4; 32]).expect("account")),
            expected_grid: Some(GridKey::from_bytes([5; 32]).expect("grid")),
            lease_epoch: 7,
            host_pid: std::process::id(),
            chat: None,
            preview: None,
        }
    }

    #[test]
    fn preview_bootstrap_optional_absence_and_two_private_keys_fit_existing_limit() {
        let mut base = config();
        let encoded = base.encode_for_launch().unwrap();
        assert!(
            !serde_json::from_slice::<serde_json::Value>(&encoded)
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("preview")
        );
        assert!(serde_json::from_slice::<WorkerConfig>(&encoded).unwrap().preview.is_none());
        base.chat = Some(ChatBootstrap {
            port: 12345,
            key: serde_json::from_str(&format!("\"{}\"", "07".repeat(32))).unwrap(),
        });
        base.preview = Some(super::super::preview_bootstrap::PreviewBootstrap {
            port: 54321,
            key: super::super::preview_bootstrap::PreviewSecret::synthetic([9; 32]).unwrap(),
        });
        base.generation = u64::MAX;
        base.lease_epoch = u64::MAX;
        let encoded = base.encode_for_launch().unwrap();
        assert!(encoded.len() <= MAX_CONFIG);
        assert!(serde_json::from_slice::<WorkerConfig>(&encoded).unwrap().preview.is_some());
        assert!(!format!("{base:?}").contains(&"09".repeat(32)));
        base.preview.as_mut().unwrap().port = 0;
        assert!(base.validate().is_err());
        assert!(serde_json::from_str::<super::super::preview_bootstrap::PreviewSecret>(&format!("\"{}\"", "00".repeat(32))).is_err());
    }

    #[test]
    fn preview_bootstrap_optional_overflow_drops_only_preview_and_invalid_base_still_refuses() {
        let mut base = config();
        let limit = base.encode_for_launch().unwrap().len();
        base.preview = Some(super::super::preview_bootstrap::PreviewBootstrap {
            port: 12345,
            key: super::super::preview_bootstrap::PreviewSecret::synthetic([7; 32]).unwrap(),
        });
        assert!(serde_json::to_vec(&base).unwrap().len() > limit);
        let bytes = base.encode_with_limit(limit).unwrap();
        assert!(base.preview.is_none());
        assert!(serde_json::from_slice::<WorkerConfig>(&bytes).unwrap().preview.is_none());
        assert!(base.encode_with_limit(limit - 1).is_err());
        base.generation = 0;
        assert!(base.encode_for_launch().is_err());
    }

    #[test]
    fn reopening_a_persistent_slot_never_reuses_worker_session_or_chat_authority() {
        let installation = current_installation().unwrap();
        let host = HostId::from_bytes(*uuid::Uuid::new_v4().as_bytes()).unwrap();
        let slot = SlotId::try_from(2).unwrap();
        let mut first = new_config(installation, host, None, Some(45123)).unwrap();
        let mut second = new_config(installation, host, None, Some(45123)).unwrap();
        first.slot = Some(slot);
        second.slot = Some(slot);
        for config in [&mut first, &mut second] {
            config.expected_account = Some(AccountKey::from_bytes([4; 32]).unwrap());
            config.expected_grid = Some(GridKey::from_bytes([5; 32]).unwrap());
        }
        assert_ne!(first.identity.worker, second.identity.worker);
        assert_ne!(first.session, second.session);
        assert_ne!(first.directory().unwrap(), second.directory().unwrap());
        let first_status = status(&first);
        let second_status = status(&second);
        assert!(!second.accepts(&first_status));
        assert_ne!(
            ChatSource::try_from((first_status.binding, first.lease_epoch)).unwrap(),
            ChatSource::try_from((second_status.binding, second.lease_epoch)).unwrap(),
            "slot identity cannot retain an earlier login's chat drafts or receipts"
        );
        let mut old = serde_json::to_value(&first).unwrap();
        old.as_object_mut().unwrap().remove("slot");
        assert!(serde_json::from_value::<WorkerConfig>(old).unwrap().slot.is_none());
        let mut invalid = serde_json::to_value(&second).unwrap();
        invalid["slot"] = serde_json::json!(MAX_WORKERS);
        assert!(serde_json::from_value::<WorkerConfig>(invalid).is_err());
    }

    #[test]
    fn broker_reserves_exact_slots_and_failed_bootstrap_preparation_releases_the_open_request() {
        // A synthetic installation deliberately fails new_config validation
        // before process creation. No user catalogue or viewer is accessed.
        let installation = InstallationId::from_bytes(
            [*uuid::Uuid::new_v4().as_bytes(), *uuid::Uuid::new_v4().as_bytes()]
                .concat()
                .try_into()
                .unwrap(),
        )
        .unwrap();
        let host = HostId::from_bytes(*uuid::Uuid::new_v4().as_bytes()).unwrap();
        let preview_frontdoor = Arc::new(preview_control::Frontdoor::default());
        let preview_reader = PreviewReader::new(preview_frontdoor.clone());
        let mut broker = Broker {
            preview_frontdoor,
            preview_projection: BrokerProjection::new(preview_reader, installation, host, false),
            registry: Registry::open(installation, host).unwrap(),
            registry_error: None,
            slot_available: [true; MAX_WORKERS],
            next_slot_scan: Instant::now(),
            coordinator: Coordinator::new(installation, host),
            installation,
            host,
            children: Vec::new(),
            opens: VecDeque::new(),
            viewport: None,
            viewport_dirty: false,
            park_requested: false,
            active_viewport: None,
            pending: None,
            error: None,
            completed_request: 0,
            chat_listener: None,
            reviews: review::Transactions::default(),
        };
        let slot = SlotId::try_from(1).unwrap();
        broker.intent(HostIntent::Open(None, Some(slot)), 1, 0).unwrap();
        broker.intent(HostIntent::Open(None, Some(slot)), 2, 0).unwrap();
        assert_eq!(broker.opens.len(), 1, "repeated clicks cannot launch two writers in one slot");
        broker.open_next(&[], 0).unwrap();
        assert!(broker.open_next(&[], 0).is_err(), "foreign installation fails before launch");
        assert!(broker.opens.is_empty());
        assert!(broker.children.is_empty());
        assert_eq!(broker.coordinator.workers().count(), 0);
        assert_eq!(
            broker.choose_slot(Some(slot)).unwrap(),
            slot,
            "failed preparation does not consume the slot"
        );
        let root = super::super::profile::slot_root(&installation.to_string(), slot).unwrap();
        let lock = super::super::profile::profile_lock(&root).unwrap();
        assert!(
            broker.intent(HostIntent::Open(None, Some(slot)), 3, 0).is_err(),
            "occupied profile is refused before parking/spawn"
        );
        assert!(broker.opens.is_empty());
        drop(lock);
        for _ in 0..MAX_WORKERS {
            broker.intent(HostIntent::Open(None, None), 4, 0).unwrap();
        }
        assert_eq!(broker.opens.len(), MAX_WORKERS);
        assert!(broker.intent(HostIntent::Open(None, None), 5, 0).is_err());

        // Preference IO is a separate metadata operation: a failed save has a
        // terminal exact-ticket outcome and survives unrelated host requests.
        let preference_path = super::super::profile::host_root()
            .unwrap()
            .join("registries")
            .join(installation.to_string())
            .join("attention-preferences.json");
        super::super::profile::private_dir(&preference_path).unwrap();
        let attempted = super::super::attention_prefs::Preferences {
            nearby: true,
            ..Default::default()
        };
        let intent = HostIntent::SaveAttentionPreferences(attempted);
        assert!(!intent.controls_presentation());
        broker.intent(intent, 6, 0).unwrap();
        let failed = broker.snapshot(&[], false).attention_preferences;
        assert_eq!(failed.preferences, super::super::attention_prefs::Preferences::default());
        let operation = failed.operation.as_ref().unwrap();
        assert_eq!(operation.ticket, 6);
        assert_eq!(operation.preferences, attempted);
        assert!(operation.error.is_some());
        broker
            .intent(
                HostIntent::RenameSlot(slot, SlotLabel::new("Synthetic changed label").unwrap()),
                7,
                0,
            )
            .unwrap();
        broker.intent(HostIntent::Park, 8, 0).unwrap();
        assert_eq!(broker.snapshot(&[], false).attention_preferences, failed);
        std::fs::remove_dir(preference_path).unwrap();
    }

    #[test]
    fn chat_request_failures_do_not_cover_the_active_world_or_clear_control_errors() {
        let config = config();
        let source = ChatSource::try_from((status(&config).binding, config.lease_epoch)).unwrap();
        let intent = HostIntent::ChatSend {
            key: ChatKey {
                source,
                conversation: aurora_session::chat::Conversation::Nearby,
            },
            policy_revision: 1,
            text: ChatText::new("synthetic literal").unwrap(),
        };
        let mut error = None;
        request_result(&mut error, intent.controls_presentation(), None, Err(HostError::Unavailable.into()));
        assert!(error.is_none());
        error = Some("existing control failure".into());
        request_result(&mut error, intent.controls_presentation(), None, Ok(()));
        assert_eq!(error.as_deref(), Some("existing control failure"));
        request_result(&mut error, HostIntent::Park.controls_presentation(), None, Ok(()));
        assert!(error.is_none());
    }

    #[test]
    fn rename_success_and_failure_report_metadata_without_changing_control_error() {
        let intent = HostIntent::RenameSlot(SlotId::try_from(0).unwrap(), SlotLabel::new("Synthetic label").unwrap());
        assert!(!intent.controls_presentation());
        let mut control_error = Some("retained presentation failure".into());
        let mut metadata_error = Some("previous label failure".into());
        request_result(
            &mut control_error,
            intent.controls_presentation(),
            Some(&mut metadata_error),
            Ok(()),
        );
        assert_eq!(control_error.as_deref(), Some("retained presentation failure"));
        assert!(metadata_error.is_none());
        request_result(
            &mut control_error,
            intent.controls_presentation(),
            Some(&mut metadata_error),
            Err(anyhow::anyhow!("synthetic label persistence failure")),
        );
        assert_eq!(control_error.as_deref(), Some("retained presentation failure"));
        assert_eq!(metadata_error.as_deref(), Some("synthetic label persistence failure"));
    }

    #[test]
    fn private_chat_bootstrap_key_is_redacted_and_old_manual_bootstraps_remain_valid() {
        let mut config = config();
        let mut old = serde_json::to_value(&config).unwrap();
        old.as_object_mut().unwrap().remove("chat");
        let restored: WorkerConfig = serde_json::from_value(old).unwrap();
        assert!(restored.chat.is_none());
        restored.validate().unwrap();
        let key: super::super::chat_transport::ChatSecret = serde_json::from_str(&format!("\"{}\"", "07".repeat(32))).unwrap();
        config.chat = Some(ChatBootstrap { port: 12345, key });
        config.validate().unwrap();
        assert!(!format!("{config:?}").contains(&"07".repeat(32)));
        let encoded = serde_json::to_vec(&config).unwrap();
        assert!(encoded.len() < MAX_CONFIG);
        let restored: WorkerConfig = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored.chat.as_ref().unwrap().port, 12345);
        config.chat.as_mut().unwrap().port = 0;
        assert!(config.validate().is_err());
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

    fn command(config: &WorkerConfig, sequence: u64, command: Command) -> Envelope {
        Envelope {
            version: PROTOCOL_VERSION,
            binding: command_binding(status(config).binding, &command),
            lease_epoch: config.lease_epoch,
            sequence,
            reply_to: 0,
            message: Message::Command(command),
        }
    }

    fn event(config: &WorkerConfig, sequence: u64, reply_to: u64, event: Event) -> Envelope {
        Envelope {
            sequence,
            reply_to,
            message: Message::Event(event),
            ..status(config)
        }
    }

    fn frame_ready() -> Event {
        Event::FrameReady {
            activation: 1,
            boundary: aurora_session::PrepareBoundary {
                device_epoch: 1,
                surface_epoch: 1,
                after_frame: 0,
            },
            frame: aurora_session::FrameProof {
                device_epoch: 1,
                surface_epoch: 1,
                frame: 1,
            },
        }
    }

    #[test]
    fn session_outbox_never_overwrites_unacknowledged_control_with_heartbeats() {
        let config = config();
        let mut outbox = Outbox::default();
        outbox
            .enqueue(command(
                &config,
                1,
                Command::Prepare {
                    activation: 1,
                    viewport: None,
                },
            ))
            .expect("prepare");
        assert_eq!(outbox.next().expect("first control").sequence, 1);
        outbox.published(1);
        for sequence in [2, 3] {
            outbox
                .enqueue(command(
                    &config,
                    sequence,
                    Command::Heartbeat {
                        workspace_foreground: sequence == 3,
                    },
                ))
                .expect("coalesced heartbeat");
        }
        assert!(outbox.next().is_none(), "the Prepare slot remains unchanged until its exact ack");
        assert!(!outbox.acknowledge(&event(&config, 9, 2, frame_ready())));
        let mut foreign = event(&config, 9, 1, frame_ready());
        foreign.binding.session.as_mut().expect("session").generation += 1;
        assert!(!outbox.acknowledge(&foreign), "a later login cannot acknowledge an earlier Prepare");
        assert!(outbox.acknowledge(&event(&config, 10, 1, frame_ready())));
        let latest = outbox.next().expect("latest heartbeat");
        assert_eq!(latest.sequence, 3);
        assert!(matches!(
            latest.message,
            Message::Command(Command::Heartbeat {
                workspace_foreground: true
            })
        ));
        outbox.published(3);
        outbox
            .enqueue(command(&config, 4, Command::Grant { activation: 1 }))
            .expect("control supersedes heartbeat");
        assert_eq!(outbox.next().expect("grant").sequence, 4);
    }

    #[test]
    fn session_outbox_safety_commands_supersede_uncertain_grants_without_replaying_them() {
        let config = config();
        let mut outbox = Outbox::default();
        outbox
            .enqueue(command(&config, 1, Command::Grant { activation: 1 }))
            .expect("grant");
        assert_eq!(outbox.next().expect("grant").sequence, 1);
        outbox.published(1);
        outbox
            .enqueue(command(
                &config,
                2,
                Command::Heartbeat {
                    workspace_foreground: false,
                },
            ))
            .expect("heartbeat");
        outbox
            .enqueue(command(&config, 3, Command::Revoke { activation: 2 }))
            .expect("rollback revoke");
        assert_eq!(outbox.next().expect("revoke").sequence, 3);
        outbox.published(3);
        assert!(!outbox.acknowledge(&event(&config, 10, 1, Event::Granted { activation: 1 })));
        outbox
            .enqueue(command(&config, 4, Command::Detach))
            .expect("detach supersedes uncertain revoke");
        assert_eq!(outbox.next().expect("detach").sequence, 4);
        assert!(!outbox.acknowledge(&event(&config, 11, 3, Event::Detached)));
        assert!(outbox.acknowledge(&event(&config, 12, 4, Event::Detached)));
        assert!(outbox.next().is_none());
    }

    #[test]
    fn session_outbox_capacity_and_publication_retries_are_bounded() {
        let config = config();
        let mut outbox = Outbox::default();
        for sequence in 1..=MAX_QUEUE as u64 {
            outbox
                .enqueue(command(
                    &config,
                    sequence,
                    Command::Prepare {
                        activation: 1,
                        viewport: None,
                    },
                ))
                .expect("bounded controls");
        }
        assert!(outbox.enqueue(command(&config, 17, Command::Grant { activation: 1 })).is_err());
        let first = outbox.next().expect("first attempt");
        assert_eq!(
            outbox.next(),
            Some(first.clone()),
            "failed publication retries the identical envelope"
        );
        outbox.published(first.sequence);
        assert!(outbox.next().is_none());
        outbox
            .enqueue(command(&config, 18, Command::Revoke { activation: 2 }))
            .expect("safety bypasses full control queue");
        assert_eq!(outbox.next().expect("only revoke").sequence, 18);
        assert!(
            outbox.enqueue(command(&config, 17, Command::Detach)).is_err(),
            "stale safety traffic cannot erase a newer command"
        );
        assert_eq!(outbox.controls.len(), 0);
    }

    #[test]
    fn session_controller_receipts_follow_actual_fifo_without_consuming_failed_sends() {
        let (sender, receiver) = bounded(1);
        let mut requests = RequestSender { sender, next: 1 };
        assert_eq!(requests.send(HostIntent::Park), Ok(1));
        assert_eq!(requests.send(HostIntent::Open(None, None)), Err(HostError::Busy));
        assert_eq!(receiver.try_recv().expect("first intent").ticket, 1);
        assert_eq!(requests.send(HostIntent::Open(None, None)), Ok(2));
        assert_eq!(receiver.try_recv().expect("next intent").ticket, 2);
        drop(receiver);
        assert_eq!(requests.send(HostIntent::Park), Err(HostError::Stopped));
        assert_eq!(requests.next, 3);
    }

    #[test]
    fn session_launch_gate_requires_the_exact_parked_worker_and_a_quiet_coordinator() {
        let worker = WorkerId::from_bytes([1; 16]).expect("worker");
        let other = WorkerId::from_bytes([2; 16]).expect("other worker");
        let mut gate = OpenGate::Parking(worker);
        assert!(!gate.can_launch(None, false), "empty active after failure is not an ack");
        gate.parked(other);
        assert_eq!(gate, OpenGate::Parking(worker));
        gate.parked(worker);
        assert!(!gate.can_launch(Some(worker), false));
        assert!(!gate.can_launch(None, true));
        assert!(gate.can_launch(None, false));
    }

    #[test]
    fn session_inbox_orders_park_ack_before_later_status_using_the_shared_wire_sequence() {
        use aurora_session::{FrameProof, GpuProgress, PrepareBoundary, WorkerGuard};
        let config = config();
        let mut coordinator = Coordinator::new(config.identity.installation, config.identity.host);
        coordinator
            .register(config.identity.worker, config.lease_epoch, 0)
            .expect("register");
        let mut online = status(&config);
        online.message = Message::Event(Event::Status(WorkerStatus {
            phase: Phase::Online,
            native_window: 1,
            ..Default::default()
        }));
        coordinator.observe(&online, 1).expect("online");
        let mut guard = WorkerGuard::new(config.identity, config.lease_epoch, 0).expect("guard");
        guard.update_session(online.binding.session).expect("exact login");
        let only_send = |effects: Vec<Effect>| {
            effects
                .into_iter()
                .find_map(|effect| match effect {
                    Effect::Send(envelope) => Some(envelope),
                    _ => None,
                })
                .expect("control effect")
        };
        let prepare = only_send(coordinator.activate(config.identity.worker, None, 2).expect("activate"));
        guard.on_command(&prepare, 2).expect("prepare command");
        guard
            .prepare_boundary(
                prepare.sequence,
                PrepareBoundary {
                    device_epoch: 1,
                    surface_epoch: 1,
                    after_frame: 0,
                },
            )
            .expect("boundary");
        let ready = guard
            .frame_ready(
                prepare.sequence,
                FrameProof {
                    device_epoch: 1,
                    surface_epoch: 1,
                    frame: 1,
                },
            )
            .expect("completed frame")
            .expect("reply");
        let grant = only_send(
            coordinator
                .observe(&event(&config, 2, ready.reply_to, ready.event), 3)
                .expect("prepared"),
        );
        let granted = guard.on_command(&grant, 4).expect("grant").reply.expect("grant ack");
        assert!(
            coordinator
                .observe(&event(&config, 3, granted.reply_to, granted.event), 5)
                .expect("granted")
                .iter()
                .any(|effect| matches!(effect, Effect::Notice(HostNotice::Switched(_))))
        );
        let revoke = only_send(coordinator.park_active(6).expect("park before launch"));
        guard.on_command(&revoke, 6).expect("revoke");
        let parked = guard
            .quiesced(
                revoke.sequence,
                GpuProgress {
                    device_epoch: 1,
                    submitted: 1,
                    completed: 1,
                },
            )
            .expect("GPU idle")
            .expect("park ack");
        let acknowledgment = event(&config, 4, parked.reply_to, parked.event);
        let periodic = event(
            &config,
            5,
            0,
            Event::Status(WorkerStatus {
                phase: Phase::Online,
                native_window: 1,
                ..Default::default()
            }),
        );
        let incoming = ordered_incoming(Some(periodic), Some(acknowledgment), 3);
        assert_eq!(incoming.iter().map(|envelope| envelope.sequence).collect::<Vec<_>>(), [4, 5]);
        let mut gate = OpenGate::Parking(config.identity.worker);
        for envelope in incoming {
            for effect in coordinator.observe(&envelope, 7).expect("ordered event") {
                if let Effect::Notice(HostNotice::Parked(worker)) = effect {
                    gate.parked(worker);
                }
            }
        }
        assert!(gate.can_launch(coordinator.active(), coordinator.busy()));
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
