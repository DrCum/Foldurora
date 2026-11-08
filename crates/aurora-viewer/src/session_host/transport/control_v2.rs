//! Actual bounded V2 worker IO owner. Production version selection remains V1
//! until the joined native/IO consumer is independently reviewed. The state
//! below is used by worker_loop and its byte-publication test adapter alike.
use super::{CLOSE_WAIT, HEARTBEAT, PERIOD, WorkerConfig, publish, read_bytes};
use aurora_session::control_v2::{self as wire, *};
use aurora_session::show_viewer::{ShowViewerOutcome, ShowViewerReceipt, ShowViewerTarget};
use aurora_session::{Binding, Error, WorkerIdentity, WorkerStatus};
use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const DOWNSTREAM: usize = 15;
const REPLIES: usize = 16;
pub(crate) const COMMAND: &str = "control-v2.json";
pub(crate) const INGRESS: &str = "control-ingress-v2.json";
pub(crate) const REPLY: &str = "reply-v2.json";
pub(crate) const ACK: &str = "reply-ack-v2.json";
pub(crate) const STATUS: &str = "status-v2.json";

struct Credits {
    used: AtomicUsize,
    limit: usize,
}
impl Credits {
    fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            used: AtomicUsize::new(0),
            limit,
        })
    }
    fn acquire(self: &Arc<Self>) -> Option<Credit> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| (used < self.limit).then_some(used + 1))
            .ok()?;
        Some(Credit(self.clone()))
    }
}
struct Credit(Arc<Credits>);
impl Drop for Credit {
    fn drop(&mut self) {
        self.0.used.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Ownership moves IO→channel→App FIFO head. Its credit is released only after
/// admission/terminal cancellation, not merely when App removes it from channel.
pub(crate) struct ReceivedControl {
    pub envelope: ControlV2Envelope,
    pub received_at: Instant,
    pub bytes: Arc<[u8]>,
    pub digest: [u8; 32],
    _credit: Credit,
}
impl ReceivedControl {
    pub fn integrity(&self) -> bool {
        self.bytes.len() <= MAX_CONTROL_V2_BYTES && <[u8; 32]>::from(Sha256::digest(&*self.bytes)) == self.digest
    }
    pub fn predates_ready(&self, ready: Instant) -> bool {
        self.received_at < ready
    }
    pub fn received_ms(&self, origin: Instant, now: Instant) -> Result<u64, Error> {
        if self.received_at > now {
            return Err(Error::Stale);
        }
        let elapsed = self.received_at.checked_duration_since(origin).ok_or(Error::Stale)?;
        elapsed.as_millis().try_into().map_err(|_| Error::Sequence)
    }
}
struct Staged {
    envelope: ControlV2Envelope,
    received_at: Instant,
    bytes: Arc<[u8]>,
    digest: [u8; 32],
}

struct ReplySlot {
    original: ControlV2Envelope,
    terminal: OnceLock<ControlV2Event>,
    effect_started: AtomicBool,
    _credit: Credit,
}
impl ReplySlot {
    fn fallback(&self) -> ControlV2Event {
        match &self.original.message {
            ControlV2Message::Command(ControlV2Command::ShowViewer(request)) if self.effect_started.load(Ordering::Acquire) => {
                ControlV2Event::ShowViewerReceipt(ShowViewerReceipt {
                    request: *request,
                    outcome: ShowViewerOutcome::EffectUncertain,
                })
            }
            ControlV2Message::Command(command) => ControlV2Event::ControlUnavailable { command: command.clone() },
            ControlV2Message::Event(_) => unreachable!("reply slots are reserved only for validated commands"),
        }
    }
}
/// Nonclonable original reply ownership. Native work must retain this value;
/// dropping it cannot free/reuse the physical slot before IO retires its reply.
pub(crate) struct ReplyReservation {
    slot: Arc<ReplySlot>,
}
impl ReplyReservation {
    pub fn original(&self) -> &ControlV2Envelope {
        &self.slot.original
    }
    pub fn mark_effect_started(&self) {
        self.slot.effect_started.store(true, Ordering::Release);
    }
    pub fn effect_started(&self) -> bool {
        self.slot.effect_started.load(Ordering::Acquire)
    }
    pub fn complete(self, event: ControlV2Event) -> Result<(), Error> {
        let original = &self.slot.original;
        let reply = ControlV2Envelope {
            version: CONTROL_V2_VERSION,
            binding: original.binding,
            lease_epoch: original.lease_epoch,
            sequence: 1,
            reply_to: original.sequence,
            message: ControlV2Message::Event(event.clone()),
        };
        if !wire::matches_reply(original, &reply) {
            return Err(Error::Binding);
        }
        self.slot.terminal.set(event).map_err(|_| Error::Stale)
    }
}
impl Drop for ReplyReservation {
    fn drop(&mut self) {
        if self.slot.terminal.get().is_none() {
            let _ = self.slot.terminal.set(self.slot.fallback());
        }
    }
}

#[derive(Clone)]
struct Status {
    binding: Binding,
    status: WorkerStatus,
    ordinary: Option<ShowViewerTarget>,
}
pub(crate) struct WorkerEndpoint {
    commands: Receiver<ReceivedControl>,
    reservations: Sender<Arc<ReplySlot>>,
    reply_credits: Arc<Credits>,
    status: Arc<Mutex<Option<Status>>>,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    worker: WorkerIdentity,
    lease_epoch: u64,
    pub origin: Instant,
}
impl WorkerEndpoint {
    pub fn start(config: WorkerConfig, directory: PathBuf, origin: Instant) -> io::Result<Self> {
        let (commands_tx, commands) = bounded(DOWNSTREAM);
        let (reservations, replies_rx) = bounded(REPLIES);
        let reply_credits = Credits::new(REPLIES);
        let status = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));
        let worker = config.identity;
        let lease_epoch = config.lease_epoch;
        let state = WorkerState::new(worker, lease_epoch, commands_tx, replies_rx, reply_credits.clone(), origin);
        let (thread_status, thread_stop, thread_failed, thread_exited) = (status.clone(), stop.clone(), failed.clone(), exited.clone());
        std::thread::Builder::new().name("session-worker-v2-io".into()).spawn(move || {
            struct Exit {
                failed: Arc<AtomicBool>,
                exited: Arc<AtomicBool>,
            }
            impl Drop for Exit {
                fn drop(&mut self) {
                    self.failed.store(true, Ordering::Release);
                    self.exited.store(true, Ordering::Release);
                }
            }
            let _exit = Exit {
                failed: thread_failed.clone(),
                exited: thread_exited,
            };
            worker_loop(state, &directory, thread_status, &thread_stop, &thread_failed);
        })?;
        Ok(Self {
            commands,
            reservations,
            reply_credits,
            status,
            stop,
            failed,
            exited,
            worker,
            lease_epoch,
            origin,
        })
    }
    pub fn next_command(&self) -> Option<ReceivedControl> {
        self.commands.try_recv().ok()
    }
    pub fn reserve(&self, original: &ControlV2Envelope) -> Result<ReplyReservation, Error> {
        original.validate()?;
        if self.failed() || original.binding.worker != self.worker || original.lease_epoch != self.lease_epoch {
            return Err(Error::Unavailable);
        }
        if !matches!(original.message, ControlV2Message::Command(_)) {
            return Err(Error::Invalid);
        }
        let credit = self.reply_credits.acquire().ok_or(Error::Capacity)?;
        let slot = Arc::new(ReplySlot {
            original: original.clone(),
            terminal: OnceLock::new(),
            effect_started: AtomicBool::new(false),
            _credit: credit,
        });
        self.reservations.try_send(slot.clone()).map_err(|error| match error {
            TrySendError::Full(_) => Error::Capacity,
            TrySendError::Disconnected(_) => Error::Unavailable,
        })?;
        Ok(ReplyReservation { slot })
    }
    pub fn status(&self, binding: Binding, status: WorkerStatus, ordinary: Option<ShowViewerTarget>) {
        if let Ok(mut latest) = self.status.try_lock() {
            *latest = Some(Status { binding, status, ordinary });
        }
    }
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire) || self.stop.load(Ordering::Acquire) || self.exited.load(Ordering::Acquire)
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Drop for WorkerEndpoint {
    fn drop(&mut self) {
        self.stop();
    }
}

struct RetainedReply {
    slot: Arc<ReplySlot>,
    envelope: Option<ControlV2Envelope>,
    bytes: Option<Vec<u8>>,
    published: bool,
}
/// This is the actual worker_loop state, including physical reply credits.
/// Tests substitute only byte input/publication; admission and ownership are
/// identical to the real private-file adapter below.
struct WorkerState {
    worker: WorkerIdentity,
    lease_epoch: u64,
    commands: Sender<ReceivedControl>,
    downstream: Arc<Credits>,
    reservations: Receiver<Arc<ReplySlot>>,
    reply_credits: Arc<Credits>,
    stage: Option<Staged>,
    transferred: u64,
    ingress: Option<(ControlIngressAckV2, Vec<u8>)>,
    ingress_dirty: bool,
    replies: VecDeque<RetainedReply>,
    sequence: u64,
    next_status: Instant,
}
impl WorkerState {
    fn new(
        worker: WorkerIdentity,
        lease_epoch: u64,
        commands: Sender<ReceivedControl>,
        reservations: Receiver<Arc<ReplySlot>>,
        reply_credits: Arc<Credits>,
        now: Instant,
    ) -> Self {
        Self {
            worker,
            lease_epoch,
            commands,
            downstream: Credits::new(DOWNSTREAM),
            reservations,
            reply_credits,
            stage: None,
            transferred: 0,
            ingress: None,
            ingress_dirty: false,
            replies: VecDeque::with_capacity(REPLIES),
            sequence: 0,
            next_status: now,
        }
    }
    fn sequence(&mut self) -> io::Result<u64> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| io::Error::other("V2 event sequence exhausted"))?;
        Ok(self.sequence)
    }
    fn decode_command(&mut self, bytes: Vec<u8>, now: Instant) {
        if self.stage.is_some() {
            return;
        }
        let Ok(envelope) = wire::decode_control(&bytes) else { return };
        if envelope.binding.worker != self.worker
            || envelope.lease_epoch != self.lease_epoch
            || envelope.sequence <= self.transferred
            || !matches!(envelope.message, ControlV2Message::Command(_))
        {
            return;
        }
        let digest = Sha256::digest(&bytes).into();
        self.stage = Some(Staged {
            envelope,
            received_at: now,
            bytes: bytes.into(),
            digest,
        });
    }
    fn transfer(&mut self) -> io::Result<()> {
        if self.stage.is_none() {
            return Ok(());
        }
        let Some(credit) = self.downstream.acquire() else { return Ok(()) };
        let Some(stage) = self.stage.take() else { return Ok(()) };
        let receipt = ControlIngressAckV2 {
            version: CONTROL_V2_VERSION,
            binding: stage.envelope.binding,
            lease_epoch: self.lease_epoch,
            command_sequence: stage.envelope.sequence,
            command_digest: stage.digest,
        };
        let command = ReceivedControl {
            envelope: stage.envelope,
            received_at: stage.received_at,
            bytes: stage.bytes,
            digest: stage.digest,
            _credit: credit,
        };
        match self.commands.try_send(command) {
            Ok(()) => {
                self.transferred = receipt.command_sequence;
                let bytes = wire::encode_ingress_ack(&receipt).map_err(io::Error::other)?;
                self.ingress = Some((receipt, bytes));
                self.ingress_dirty = true;
            }
            Err(TrySendError::Full(command)) => {
                self.stage = Some(Staged {
                    envelope: command.envelope,
                    received_at: command.received_at,
                    bytes: command.bytes,
                    digest: command.digest,
                });
            }
            Err(TrySendError::Disconnected(_)) => return Err(io::Error::other("V2 App command owner closed")),
        }
        Ok(())
    }
    fn acknowledge(&mut self, bytes: Option<&[u8]>) {
        let Some(ack) = bytes.and_then(|bytes| wire::decode_reply_ack(bytes).ok()) else {
            return;
        };
        if self.replies.front().is_some_and(|reply| {
            reply.published
                && reply
                    .envelope
                    .as_ref()
                    .is_some_and(|envelope| ack.matches_published(envelope, self.worker, self.lease_epoch))
        }) {
            let _ = self.replies.pop_front();
        }
    }
    fn step(
        &mut self,
        ack: Option<&[u8]>,
        command: Option<Vec<u8>>,
        now: Instant,
        status: Option<Status>,
        mut publish: impl FnMut(&str, &[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        // Receipt retirement always precedes admission, even with16 blocked
        // commands/replies. It cannot call Guard or refresh original stamps.
        self.acknowledge(ack);
        while self.replies.len() < REPLIES {
            match self.reservations.try_recv() {
                Ok(slot) => self.replies.push_back(RetainedReply {
                    slot,
                    envelope: None,
                    bytes: None,
                    published: false,
                }),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        if let Some(event) = self
            .replies
            .front()
            .filter(|reply| reply.envelope.is_none())
            .and_then(|reply| reply.slot.terminal.get())
            .cloned()
        {
            let sequence = self.sequence()?;
            if let Some(reply) = self.replies.front_mut() {
                let original = &reply.slot.original;
                let envelope = ControlV2Envelope {
                    version: CONTROL_V2_VERSION,
                    binding: original.binding,
                    lease_epoch: original.lease_epoch,
                    sequence,
                    reply_to: original.sequence,
                    message: ControlV2Message::Event(event),
                };
                let bytes = wire::encode_control(&envelope).map_err(io::Error::other)?;
                reply.envelope = Some(envelope);
                reply.bytes = Some(bytes);
            }
        }
        if let Some(reply) = self.replies.front_mut()
            && !reply.published
            && let Some(bytes) = &reply.bytes
        {
            publish(REPLY, bytes)?;
            reply.published = true;
        }
        if let Some(bytes) = command {
            self.decode_command(bytes, now);
        }
        self.transfer()?;
        if self.ingress_dirty
            && let Some((_, bytes)) = &self.ingress
        {
            publish(INGRESS, bytes)?;
            self.ingress_dirty = false;
        }
        if now >= self.next_status && self.reply_credits.used.load(Ordering::Acquire) == 0 {
            if let Some(status) = status {
                let envelope = ControlV2Envelope {
                    version: CONTROL_V2_VERSION,
                    binding: status.binding,
                    lease_epoch: self.lease_epoch,
                    sequence: self.sequence()?,
                    reply_to: 0,
                    message: ControlV2Message::Event(ControlV2Event::Status {
                        status: status.status,
                        ordinary: status.ordinary,
                    }),
                };
                publish(STATUS, &wire::encode_control(&envelope).map_err(io::Error::other)?)?;
            }
            self.next_status = now
                .checked_add(HEARTBEAT)
                .ok_or_else(|| io::Error::other("V2 status clock exhausted"))?;
        }
        Ok(())
    }
}

fn worker_loop(mut state: WorkerState, directory: &Path, status: Arc<Mutex<Option<Status>>>, stop: &AtomicBool, failed: &AtomicBool) {
    let mut closing = None;
    loop {
        let now = Instant::now();
        if stop.load(Ordering::Acquire) && closing.is_none() {
            closing = Some(now.checked_add(CLOSE_WAIT).unwrap_or(now));
        }
        // The independent ACK file remains readable while all other owners
        // are full/staged or the service is draining. No channel wait here.
        let ack = read_bytes(&directory.join(ACK), MAX_CONTROL_ACK_BYTES).ok();
        let command = (closing.is_none() && state.stage.is_none())
            .then(|| read_bytes(&directory.join(COMMAND), MAX_CONTROL_V2_BYTES).ok())
            .flatten();
        let latest = status.try_lock().ok().and_then(|status| status.clone());
        if let Err(error) = state.step(ack.as_deref(), command, now, latest, |name, bytes| {
            publish(&directory.join(name), bytes)
        }) {
            if !failed.swap(true, Ordering::AcqRel) {
                log::warn!("V2 worker control became unavailable: {error}");
            }
            if closing.is_none() {
                closing = Some(now.checked_add(CLOSE_WAIT).unwrap_or(now));
            }
        }
        if closing.is_some_and(|deadline| now >= deadline || state.reply_credits.used.load(Ordering::Acquire) == 0) {
            break;
        }
        std::thread::sleep(PERIOD);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Delivery {
    Queued,
    Published,
    Transferred,
}
struct HostCommand {
    envelope: ControlV2Envelope,
    bytes: Vec<u8>,
    digest: [u8; 32],
    captured_at: Instant,
    delivery: Delivery,
}
/// Actual Child writer/correlation owner:16 total accepted commands, not16
/// plus an uncharged published command. Transport transfer is independent of
/// terminal native completion, so delayed Prepare cannot hold the file slot.
pub(crate) struct HostControl {
    worker: WorkerIdentity,
    lease_epoch: u64,
    operations: VecDeque<HostCommand>,
    pending: Option<u64>,
    highest: u64,
    received_reply: u64,
    ack: Option<(ControlV2Ack, Vec<u8>)>,
    receipts: VecDeque<ControlV2Envelope>,
}
impl HostControl {
    pub fn new(worker: WorkerIdentity, lease_epoch: u64) -> Self {
        Self {
            worker,
            lease_epoch,
            operations: VecDeque::with_capacity(REPLIES),
            pending: None,
            highest: 0,
            received_reply: 0,
            ack: None,
            receipts: VecDeque::with_capacity(REPLIES),
        }
    }
    pub fn enqueue(&mut self, envelope: ControlV2Envelope, captured_at: Instant) -> io::Result<()> {
        envelope.validate().map_err(io::Error::other)?;
        if envelope.binding.worker != self.worker
            || envelope.lease_epoch != self.lease_epoch
            || !matches!(envelope.message, ControlV2Message::Command(_))
        {
            return Err(io::Error::other("V2 writer scope differs"));
        }
        if envelope.sequence <= self.highest {
            return Err(io::Error::other("V2 writer sequence is stale"));
        }
        if self.operations.len() == REPLIES {
            return Err(io::Error::other("V2 writer is full"));
        }
        let bytes = wire::encode_control(&envelope).map_err(io::Error::other)?;
        let digest = Sha256::digest(&bytes).into();
        self.highest = envelope.sequence;
        self.operations.push_back(HostCommand {
            envelope,
            bytes,
            digest,
            captured_at,
            delivery: Delivery::Queued,
        });
        Ok(())
    }
    pub fn ingress(&mut self, receipt: &ControlIngressAckV2) -> bool {
        let Some(sequence) = self.pending else { return false };
        let Some(operation) = self.operations.iter_mut().find(|op| op.envelope.sequence == sequence) else {
            return false;
        };
        if operation.delivery != Delivery::Published
            || !receipt.matches_ingress(&operation.envelope, operation.digest, self.worker, self.lease_epoch)
        {
            return false;
        }
        operation.delivery = Delivery::Transferred;
        self.pending = None;
        true
    }
    /// Returns an exact copied native/metadata reply once. Retirement ACK is
    /// independent of whether the current Coordinator still accepts its old
    /// source: a source change cannot strand physical publication ownership.
    pub fn reply(&mut self, envelope: &ControlV2Envelope) -> Option<ControlV2Envelope> {
        if envelope.sequence <= self.received_reply || envelope.validate().is_err() {
            return None;
        }
        let index = self.operations.iter().position(|op| wire::matches_reply(&op.envelope, envelope))?;
        let operation = &self.operations[index];
        // Only an explicit transfer receipt releases the command-file owner.
        // A terminal reply arriving first stays in WorkerState's retained file
        // until the next poll consumes that distinct ingress receipt.
        if operation.delivery != Delivery::Transferred {
            return None;
        }
        let ack = ControlV2Ack {
            version: CONTROL_V2_VERSION,
            reply: ReplyAckV2 {
                binding: envelope.binding,
                lease_epoch: envelope.lease_epoch,
                event_sequence: envelope.sequence,
                reply_to: envelope.reply_to,
            },
        };
        let bytes = wire::encode_reply_ack(&ack).ok()?;
        // The next validated published reply proves the previous exact ACK
        // retired; WorkerState cannot publish it otherwise. No status can do so.
        self.ack = Some((ack, bytes));
        self.received_reply = envelope.sequence;
        if self.receipts.len() == REPLIES {
            let _ = self.receipts.pop_front();
        }
        self.receipts.push_back(envelope.clone());
        let _ = self.operations.remove(index);
        Some(envelope.clone())
    }
    pub fn flush(&mut self, directory: &Path, now: Instant) -> io::Result<Option<aurora_session::show_viewer::ShowViewerRequest>> {
        self.flush_retirement(directory)?;
        self.flush_command(now, |bytes| publish(&directory.join(COMMAND), bytes))
    }
    pub fn flush_retirement(&mut self, directory: &Path) -> io::Result<()> {
        if let Some((_, bytes)) = &self.ack {
            publish(&directory.join(ACK), bytes)?;
        }
        if let Ok(bytes) = read_bytes(&directory.join(INGRESS), MAX_CONTROL_ACK_BYTES)
            && let Ok(receipt) = wire::decode_ingress_ack(&bytes)
        {
            self.ingress(&receipt);
        }
        Ok(())
    }
    fn flush_command(
        &mut self,
        now: Instant,
        mut publish: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<Option<aurora_session::show_viewer::ShowViewerRequest>> {
        if self.pending.is_none() {
            let Some(index) = self.operations.iter().position(|op| op.delivery == Delivery::Queued) else {
                return Ok(None);
            };
            let operation = &self.operations[index];
            if let ControlV2Message::Command(ControlV2Command::ShowViewer(request)) = operation.envelope.message {
                let Some(age) = now.checked_duration_since(operation.captured_at) else {
                    return Err(io::Error::other("V2 host capture is in the future"));
                };
                if age >= Duration::from_millis(SHOW_VIEWER_MILLIS) {
                    let request = request;
                    let _ = self.operations.remove(index);
                    return Ok(Some(request));
                }
            }
            let operation = &mut self.operations[index];
            operation.delivery = Delivery::Published;
            self.pending = Some(operation.envelope.sequence);
        }
        if let Some(operation) = self.operations.iter().find(|op| Some(op.envelope.sequence) == self.pending) {
            // Even an ambiguous first publication failure retains these exact
            // bytes/scope. It never re-encodes or renews Host/worker ingress.
            publish(&operation.bytes)?;
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
