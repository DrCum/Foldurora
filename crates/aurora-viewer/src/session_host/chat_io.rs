//! Dedicated worker chat IO. The control bridge and native backend retain their
//! own lifetimes: a chat socket failure neither blocks control acknowledgments
//! nor clears a native send ledger. No native/UI API, transcript or filesystem
//! operation is called here.
//!
//! Application methods only touch bounded typed queues and the latest current
//! source. The IO thread owns connect, authentication, codec and socket polling.
//! A new source/socket retires queued data from the preceding connection; no
//! mutation or native event is automatically replayed after reconnection.

use super::chat_transport::{LaneError, Permit, WorkerIo};
use aurora_session::WorkerIdentity;
use aurora_session::chat::{ChatMessage, ChatPacket, ChatSource};
use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const CAPACITY: usize = 16;
// A native Poll can emit up to eleven packets. One command per CPU frame keeps
// that response batch within the adapter's separate bounded response queue.
const COMMANDS_PER_CALL: usize = 1;
const PERIOD: Duration = Duration::from_millis(20);
const RETRY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
struct Scope {
    identity: WorkerIdentity,
    lease_epoch: u64,
}

impl Scope {
    fn accepts(self, source: ChatSource) -> bool {
        source.worker == self.identity && source.lease_epoch == self.lease_epoch && source.validate().is_ok()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Connection {
    revision: u64,
    epoch: u64,
}

#[derive(Clone)]
struct Queued {
    connection: Connection,
    packet: ChatPacket,
    received_at: Instant,
}

/// Local IO arrival time travels with the command through every App queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReceivedCommand {
    pub packet: ChatPacket,
    pub received_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceUpdate {
    Unchanged,
    Renewed,
    Changed,
}

struct State {
    scope: Scope,
    source: Option<ChatSource>,
    expires: Option<Instant>,
    revision: u64,
    epoch: u64,
    connected: bool,
    outgoing: VecDeque<Queued>,
    incoming: VecDeque<Queued>,
}

impl State {
    fn new(scope: Scope) -> Self {
        Self {
            scope,
            source: None,
            expires: None,
            revision: 1,
            epoch: 0,
            connected: false,
            outgoing: VecDeque::with_capacity(CAPACITY),
            incoming: VecDeque::with_capacity(CAPACITY),
        }
    }

    fn set_source(&mut self, source: Option<ChatSource>, expires: Option<Instant>, now: Instant) -> SourceUpdate {
        let expired = self.expire(now);
        let source = source.filter(|source| self.scope.accepts(*source) && expires.is_some_and(|deadline| deadline > now));
        let expires = source.and(expires);
        if source == self.source && expires == self.expires {
            return if expired { SourceUpdate::Changed } else { SourceUpdate::Unchanged };
        }
        if source == self.source {
            self.expires = expires;
            return if expired { SourceUpdate::Changed } else { SourceUpdate::Renewed };
        }
        self.replace_source(source, expires);
        SourceUpdate::Changed
    }

    fn replace_source(&mut self, source: Option<ChatSource>, expires: Option<Instant>) {
        self.source = source;
        self.expires = expires;
        self.revision = if self.revision == 0 {
            0
        } else {
            self.revision.checked_add(1).unwrap_or(0)
        };
        self.retire();
        if self.revision == 0 {
            self.source = None;
            self.expires = None;
        }
    }

    fn expire(&mut self, now: Instant) -> bool {
        if self.source.is_some() && self.expires.is_none_or(|deadline| deadline <= now) {
            self.replace_source(None, None);
            true
        } else {
            false
        }
    }

    fn retire(&mut self) {
        self.connected = false;
        self.outgoing.clear();
        self.incoming.clear();
    }

    fn current_revision(&self, revision: u64) -> bool {
        self.revision != 0
            && self.revision == revision
            && self.source.is_some()
            && self.expires.is_some_and(|deadline| deadline > Instant::now())
    }

    fn connection(&self) -> Option<Connection> {
        (self.connected && self.current_revision(self.revision)).then_some(Connection {
            revision: self.revision,
            epoch: self.epoch,
        })
    }

    fn matches(&self, connection: Connection) -> bool {
        self.current_revision(connection.revision) && self.connection() == Some(connection)
    }

    fn connected(&mut self, revision: u64) -> Option<Connection> {
        if !self.current_revision(revision) {
            return None;
        }
        if self.connected {
            return self.connection();
        }
        let Some(epoch) = self.epoch.checked_add(1) else {
            self.revision = 0;
            self.source = None;
            self.expires = None;
            self.retire();
            return None;
        };
        self.retire();
        self.epoch = epoch;
        self.connected = true;
        self.connection()
    }

    fn outgoing(&mut self, packet: &ChatPacket) -> Result<(), LaneError> {
        let connection = self.connection().ok_or(LaneError::NotReady)?;
        if self.outgoing.len() == CAPACITY {
            return Err(LaneError::Busy);
        }
        if Some(packet.source) != self.source || !self.scope.accepts(packet.source) {
            return Err(LaneError::Binding);
        }
        if !matches!(packet.message, ChatMessage::Event(_)) {
            return Err(LaneError::Direction);
        }
        if packet.sequence == 0 {
            return Err(LaneError::Sequence);
        }
        // No codec or producer-sequence mutation occurs on this caller thread.
        self.outgoing.push_back(Queued {
            connection,
            packet: packet.clone(),
            received_at: Instant::now(),
        });
        Ok(())
    }

    fn next_outgoing(&self, connection: Connection) -> Option<Queued> {
        if !self.matches(connection) {
            return None;
        }
        self.outgoing
            .front()
            .filter(|queued| queued.connection == connection && Some(queued.packet.source) == self.source)
            .cloned()
    }

    fn sent(&mut self, queued: &Queued) {
        if self
            .outgoing
            .front()
            .is_some_and(|front| front.connection == queued.connection && front.packet.sequence == queued.packet.sequence)
        {
            self.outgoing.pop_front();
        }
    }

    fn receive_capacity(&self, revision: u64) -> Option<usize> {
        // Zero room still permits write/authentication progress. The socket
        // consumes no chat frames beyond this allowance, so backpressure never
        // drops native mutations or blocks their pending receipts from flushing.
        self.current_revision(revision).then_some(CAPACITY - self.incoming.len())
    }

    #[cfg(test)]
    fn incoming(&mut self, connection: Connection, packets: Vec<ChatPacket>) -> Result<(), LaneError> {
        self.incoming_at(connection, packets, Instant::now())
    }

    fn incoming_at(&mut self, connection: Connection, packets: Vec<ChatPacket>, received_at: Instant) -> Result<(), LaneError> {
        if !self.matches(connection) {
            return Ok(());
        }
        for packet in packets {
            // Exact current source is checked before forwarding, and before
            // the native backend can consume its own receive high-water mark.
            if Some(packet.source) != self.source
                || !self.scope.accepts(packet.source)
                || !matches!(packet.message, ChatMessage::Command(_))
            {
                continue;
            }
            if self.incoming.len() == CAPACITY {
                return Err(LaneError::Busy);
            }
            self.incoming.push_back(Queued {
                connection,
                packet,
                received_at,
            });
        }
        Ok(())
    }

    fn commands(&mut self) -> Vec<ReceivedCommand> {
        let mut commands = Vec::with_capacity(COMMANDS_PER_CALL);
        while commands.len() < COMMANDS_PER_CALL {
            let Some(queued) = self.incoming.pop_front() else { break };
            if self.matches(queued.connection)
                && Some(queued.packet.source) == self.source
                && matches!(queued.packet.message, ChatMessage::Command(_))
            {
                commands.push(ReceivedCommand {
                    packet: queued.packet,
                    received_at: queued.received_at,
                });
            }
        }
        commands
    }

    fn disconnected(&mut self, revision: u64) -> bool {
        if self.revision != revision {
            return false;
        }
        self.retire();
        true
    }
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    ready: AtomicBool,
    stop: AtomicBool,
    clock: Instant,
    deadline: AtomicU64,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn current(&self, revision: u64) -> bool {
        let mut state = self.state();
        if state.expire(Instant::now()) {
            self.ready.store(false, Ordering::Release);
        }
        !self.stop.load(Ordering::Acquire) && state.current_revision(revision)
    }

    fn authenticated(&self, revision: u64) -> Option<Connection> {
        let mut state = self.state();
        if state.expire(Instant::now()) {
            self.ready.store(false, Ordering::Release);
        }
        if self.stop.load(Ordering::Acquire) {
            return None;
        }
        let connection = state.connected(revision);
        if connection.is_some() {
            self.ready.store(true, Ordering::Release);
        } else if state.revision == 0 {
            self.stop.store(true, Ordering::Release);
        }
        connection
    }

    fn disconnected(&self, revision: u64) {
        let mut state = self.state();
        if state.disconnected(revision) {
            self.ready.store(false, Ordering::Release);
        }
    }

    fn wait(&self) {
        let state = self.state();
        if !self.stop.load(Ordering::Acquire) {
            let period = state
                .expires
                .map_or(PERIOD, |deadline| PERIOD.min(deadline.saturating_duration_since(Instant::now())));
            drop(
                self.wake
                    .wait_timeout(state, period)
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
        }
    }
}

/// The backend's send ledger and both packet high-water marks stay outside this
/// channel, across temporary chat socket disconnects and reconnects.
pub(crate) struct WorkerChannel {
    shared: Arc<Shared>,
}

impl WorkerChannel {
    pub fn start(port: u16, permit: Permit) -> io::Result<Self> {
        if port == 0 || permit.lease_epoch == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid worker chat channel endpoint"));
        }
        let scope = Scope {
            identity: permit.identity,
            lease_epoch: permit.lease_epoch,
        };
        let shared = Arc::new(Shared {
            state: Mutex::new(State::new(scope)),
            wake: Condvar::new(),
            ready: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            clock: Instant::now(),
            deadline: AtomicU64::new(0),
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("session-worker-chat".into())
            .spawn(move || run(port, permit, worker))?;
        Ok(Self { shared })
    }

    pub fn ready(&self) -> bool {
        self.shared.ready.load(Ordering::Acquire)
            && !self.shared.stop.load(Ordering::Acquire)
            && self.shared.clock.elapsed().as_nanos() < u128::from(self.shared.deadline.load(Ordering::Acquire))
    }

    /// Only the control bridge's fresh authority supplies this value. Repeating
    /// the same source/deadline is a no-op. Renewal retains the current socket;
    /// None or a missing/expired receipt deadline retires its authority.
    pub fn set_source(&self, source: Option<ChatSource>, expires: Option<Instant>) {
        if self.shared.stop.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.shared.state();
        let update = state.set_source(source, expires, Instant::now());
        if update != SourceUpdate::Unchanged {
            // Readiness must not transiently drop on mutex contention: the
            // native adapter retains pending responses on Busy. Publish the
            // exact receipt deadline on this monotonic atomic clock instead.
            let deadline = state.expires.map_or(0, |deadline| {
                deadline
                    .saturating_duration_since(self.shared.clock)
                    .as_nanos()
                    .min(u128::from(u64::MAX)) as u64
            });
            self.shared.deadline.store(deadline, Ordering::Release);
            if update == SourceUpdate::Changed {
                self.shared.ready.store(false, Ordering::Release);
            }
            if state.revision == 0 {
                self.shared.stop.store(true, Ordering::Release);
            }
            self.shared.wake.notify_one();
        }
    }

    /// One command per CPU frame allows the adapter's complete policy/line/
    /// receipt response batch to fit without dropping a native mutation.
    pub fn commands(&self) -> Vec<ReceivedCommand> {
        if !self.ready() {
            return Vec::new();
        }
        let Ok(mut state) = self.shared.state.try_lock() else {
            return Vec::new();
        };
        if self.shared.stop.load(Ordering::Acquire) {
            return Vec::new();
        }
        if state.expire(Instant::now()) {
            self.shared.ready.store(false, Ordering::Release);
            self.shared.wake.notify_one();
            return Vec::new();
        }
        let commands = state.commands();
        if !commands.is_empty() {
            self.shared.wake.notify_one();
        }
        commands
    }

    pub fn send(&self, packet: &ChatPacket) -> Result<(), LaneError> {
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(LaneError::Closed);
        }
        if !self.ready() {
            return Err(LaneError::NotReady);
        }
        let mut state = self.shared.state.try_lock().map_err(|_| LaneError::Busy)?;
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(LaneError::Closed);
        }
        if state.expire(Instant::now()) {
            self.shared.ready.store(false, Ordering::Release);
            self.shared.wake.notify_one();
            return Err(LaneError::NotReady);
        }
        state.outgoing(packet)?;
        self.shared.wake.notify_one();
        Ok(())
    }
}

impl Drop for WorkerChannel {
    fn drop(&mut self) {
        // No join: an off-thread connect may still be in its one-second timeout.
        self.shared.ready.store(false, Ordering::Release);
        self.shared.stop.store(true, Ordering::Release);
        self.shared.wake.notify_one();
    }
}

struct Retry {
    next: Instant,
}

impl Retry {
    fn new(now: Instant) -> Self {
        Self { next: now }
    }
    fn due(&self, now: Instant) -> bool {
        now >= self.next
    }
    fn attempted(&mut self, now: Instant) {
        self.next = now + RETRY;
    }
    fn failed(&mut self, now: Instant) {
        self.next = self.next.max(now + RETRY);
    }
}

struct Ended(Arc<Shared>);

impl Drop for Ended {
    fn drop(&mut self) {
        self.0.ready.store(false, Ordering::Release);
        self.0.stop.store(true, Ordering::Release);
        self.0.state().retire();
        self.0.wake.notify_all();
    }
}

fn run(port: u16, permit: Permit, shared: Arc<Shared>) {
    let _ended = Ended(shared.clone());
    let mut lane: Option<WorkerIo> = None;
    let mut revision = 0;
    let mut connection = None;
    let mut retry = Retry::new(Instant::now());
    while !shared.stop.load(Ordering::Acquire) {
        let (current, present) = {
            let mut state = shared.state();
            if state.expire(Instant::now()) {
                shared.ready.store(false, Ordering::Release);
            }
            (state.revision, state.source.is_some())
        };
        if current != revision {
            // Drop all previously framed data before using a new source. The
            // native backend/ledger is deliberately not owned or reset here.
            lane = None;
            connection = None;
            revision = current;
        }
        if present && lane.is_none() && retry.due(Instant::now()) {
            retry.attempted(Instant::now());
            if let Ok(socket) = WorkerIo::connect(port, permit.clone())
                && shared.current(revision)
            {
                lane = Some(socket);
            }
            // Recheck stop/source after the potentially slow off-thread connect.
            continue;
        }
        let mut failed = false;
        if let Some(socket) = &mut lane {
            if !shared.current(revision) {
                continue;
            }
            if socket.ready() && connection.is_none() {
                connection = shared.authenticated(revision);
            }
            if let Some(connection) = connection {
                for _ in 0..CAPACITY {
                    let queued = shared.state().next_outgoing(connection);
                    let Some(queued) = queued else { break };
                    if !shared.current(revision) {
                        break;
                    }
                    match socket.queue(&queued.packet) {
                        Ok(()) => shared.state().sent(&queued),
                        Err(LaneError::Busy) => break,
                        Err(_) => {
                            failed = true;
                            break;
                        }
                    }
                }
            }
            let allowance = if !failed && !shared.stop.load(Ordering::Acquire) {
                shared.state().receive_capacity(revision)
            } else {
                None
            };
            if let Some(allowance) = allowance {
                let polled = if allowance == CAPACITY {
                    socket.poll()
                } else {
                    socket.poll_bounded(allowance)
                };
                match polled {
                    Ok(packets) => {
                        let received_at = Instant::now();
                        if socket.ready() && connection.is_none() {
                            connection = shared.authenticated(revision);
                        }
                        if let Some(connection) = connection
                            && !shared.stop.load(Ordering::Acquire)
                            && shared.state().incoming_at(connection, packets, received_at).is_err()
                        {
                            failed = true;
                        }
                    }
                    Err(_) => failed = true,
                }
            }
        }
        if failed {
            // The socket may have delivered some data already. Never stage its
            // unsent events or queued commands onto the replacement connection.
            shared.disconnected(revision);
            lane = None;
            connection = None;
            retry.failed(Instant::now());
        }
        shared.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::chat::{ChatCommand, ChatEvent, VERSION};
    use aurora_session::{AccountKey, GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerId};

    fn source() -> ChatSource {
        ChatSource {
            worker: WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).expect("synthetic installation"),
                host: HostId::from_bytes([2; 16]).expect("synthetic host"),
                worker: WorkerId::from_bytes([3; 16]).expect("synthetic worker"),
            },
            session: SessionIdentity {
                session: SessionId::from_bytes([4; 16]).expect("synthetic session"),
                account: AccountKey::from_bytes([5; 32]).expect("synthetic account"),
                grid: GridKey::from_bytes([6; 32]).expect("synthetic grid"),
                generation: 1,
            },
            lease_epoch: 7,
        }
    }

    fn state() -> (State, ChatSource, Instant, Connection) {
        let source = source();
        let mut state = State::new(Scope {
            identity: source.worker,
            lease_epoch: source.lease_epoch,
        });
        let deadline = Instant::now() + Duration::from_secs(3600);
        assert_eq!(
            state.set_source(Some(source), Some(deadline), Instant::now()),
            SourceUpdate::Changed
        );
        let connection = state.connected(state.revision).expect("synthetic authenticated connection");
        (state, source, deadline, connection)
    }

    fn packet(source: ChatSource, sequence: u64, outgoing: bool) -> ChatPacket {
        ChatPacket {
            version: VERSION,
            source,
            sequence,
            reply_to: 0,
            message: if outgoing {
                ChatMessage::Event(ChatEvent::Gap { through_event: sequence })
            } else {
                ChatMessage::Command(ChatCommand::Poll { after_event: 0 })
            },
        }
    }

    #[test]
    fn worker_chat_source_renewal_keeps_socket_but_rebind_retires_queues() {
        let (mut state, source, deadline, connection) = state();
        state.outgoing(&packet(source, 1, true)).expect("outgoing event");
        state
            .incoming(connection, vec![packet(source, 1, false)])
            .expect("incoming command");
        assert_eq!(
            state.set_source(Some(source), Some(deadline), Instant::now()),
            SourceUpdate::Unchanged
        );
        let renewed = deadline + Duration::from_secs(1);
        assert_eq!(state.set_source(Some(source), Some(renewed), Instant::now()), SourceUpdate::Renewed);
        assert!(state.connection() == Some(connection));
        assert_eq!((state.outgoing.len(), state.incoming.len()), (1, 1));

        let mut next = source;
        next.session.generation += 1;
        assert_eq!(state.set_source(Some(next), Some(renewed), Instant::now()), SourceUpdate::Changed);
        assert!(state.connection().is_none());
        assert!(state.outgoing.is_empty() && state.incoming.is_empty());
        assert!(!state.current_revision(connection.revision));
    }

    #[test]
    fn worker_chat_filters_source_before_forwarding_without_consuming_receiver_sequence() {
        let (mut state, source, _, connection) = state();
        let mut wrong = source;
        wrong.session.generation += 1;
        state
            .incoming(
                connection,
                vec![packet(wrong, u64::MAX, false), packet(source, 2, true), packet(source, 1, false)],
            )
            .expect("bounded input");
        let commands = state.commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].packet.sequence, 1);
        assert_eq!(state.outgoing(&packet(wrong, 1, true)), Err(LaneError::Binding));
        assert_eq!(state.outgoing(&packet(source, 1, false)), Err(LaneError::Direction));
        assert_eq!(state.outgoing(&packet(source, 0, true)), Err(LaneError::Sequence));
    }

    #[test]
    fn worker_chat_busy_retains_front_until_the_socket_accepts_it() {
        let (mut state, source, _, connection) = state();
        for sequence in 1..=CAPACITY as u64 {
            state.outgoing(&packet(source, sequence, true)).expect("bounded queue");
        }
        let next = packet(source, CAPACITY as u64 + 1, true);
        assert_eq!(state.outgoing(&next), Err(LaneError::Busy));
        let front = state.next_outgoing(connection).expect("pending front");
        assert_eq!(front.packet.sequence, 1);
        // A socket Busy leaves the same event available; no sequence advances.
        assert_eq!(state.next_outgoing(connection).expect("retry front").packet.sequence, 1);
        assert_eq!(state.outgoing.len(), CAPACITY);
        state.sent(&front);
        assert_eq!(state.outgoing.len(), CAPACITY - 1);
        state.outgoing(&next).expect("same caller packet available after Busy");
        assert_eq!(state.outgoing.back().expect("last packet").packet.sequence, next.sequence);
    }

    #[test]
    fn worker_chat_full_incoming_queue_still_allows_response_progress_and_drains_one_command() {
        let (mut state, source, _, connection) = state();
        state
            .incoming(
                connection,
                (1..=CAPACITY as u64).map(|sequence| packet(source, sequence, false)).collect(),
            )
            .expect("full incoming queue");
        assert_eq!(state.receive_capacity(connection.revision), Some(0));
        state
            .outgoing(&packet(source, 1, true))
            .expect("receipt can queue behind full input");
        let response = state
            .next_outgoing(connection)
            .expect("response remains flushable with read allowance zero");
        state.sent(&response);
        assert!(state.outgoing.is_empty());
        for sequence in 1..=CAPACITY as u64 {
            let commands = state.commands();
            assert_eq!(commands.len(), 1);
            assert_eq!(commands[0].packet.sequence, sequence);
            assert_eq!(state.receive_capacity(connection.revision), Some(sequence as usize));
        }
        assert!(state.commands().is_empty());
    }

    #[test]
    fn worker_chat_disconnect_drops_unsent_data_and_old_epoch_commands_without_native_replay() {
        let (mut state, source, _, old_connection) = state();
        state.outgoing(&packet(source, 1, true)).expect("old unsent event");
        let old = state.next_outgoing(old_connection).expect("old front");
        state.incoming(old_connection, vec![packet(source, 1, false)]).expect("old command");
        assert!(state.disconnected(old_connection.revision));
        assert!(state.outgoing.is_empty() && state.commands().is_empty());
        let new_connection = state.connected(state.revision).expect("reconnected same source");
        assert!(new_connection.epoch > old_connection.epoch);
        assert!(state.next_outgoing(new_connection).is_none());
        state.outgoing(&packet(source, 2, true)).expect("new producer sequence");
        state.sent(&old);
        assert_eq!(state.next_outgoing(new_connection).expect("new front preserved").packet.sequence, 2);
        state
            .incoming(old_connection, vec![packet(source, u64::MAX, false)])
            .expect("old socket discarded");
        assert!(state.commands().is_empty());
        // Only queue/connection state exists here: the native receipt ledger and
        // both global packet high-water marks are retained by the caller.
    }

    #[test]
    fn worker_chat_expiry_retires_authority_without_an_application_frame() {
        let (mut state, source, deadline, connection) = state();
        state.outgoing(&packet(source, 1, true)).expect("unsent event");
        state.incoming(connection, vec![packet(source, 1, false)]).expect("queued command");
        assert!(state.expire(deadline));
        assert!(state.source.is_none() && state.expires.is_none());
        assert!(state.outgoing.is_empty() && state.incoming.is_empty());
        assert!(state.connection().is_none());
        assert!(!state.expire(deadline + Duration::from_secs(1)));
        assert_eq!(state.outgoing(&packet(source, 2, true)), Err(LaneError::NotReady));
        // A late renewal cannot resurrect the old socket/queued commands.
        assert_eq!(
            state.set_source(
                Some(source),
                Some(deadline + Duration::from_secs(2)),
                deadline + Duration::from_secs(1)
            ),
            SourceUpdate::Changed
        );
        assert!(!state.current_revision(connection.revision));
    }

    #[test]
    fn worker_chat_missing_deadline_wrong_scope_and_counter_exhaustion_fail_closed() {
        let (mut state, source, deadline, _) = state();
        assert_eq!(state.set_source(Some(source), None, Instant::now()), SourceUpdate::Changed);
        assert!(state.source.is_none());
        assert_eq!(state.set_source(None, None, Instant::now()), SourceUpdate::Unchanged);
        let mut foreign = source;
        foreign.lease_epoch += 1;
        assert_eq!(
            state.set_source(Some(foreign), Some(deadline), Instant::now()),
            SourceUpdate::Unchanged
        );
        assert!(state.source.is_none());
        state.revision = u64::MAX;
        state.set_source(Some(source), Some(deadline), Instant::now());
        assert_eq!(state.revision, 0);
        assert!(state.source.is_none());

        let (mut state, _, _, _) = self::state();
        state.retire();
        state.epoch = u64::MAX;
        assert!(state.connected(state.revision).is_none());
        assert_eq!(state.revision, 0);
        assert!(state.source.is_none());
    }

    #[test]
    fn worker_chat_same_call_expiry_and_fresh_source_cannot_reopen_exhausted_authority() {
        let (mut state, source, _, connection) = state();
        state.outgoing(&packet(source, 1, true)).expect("queued old event");
        state
            .incoming(connection, vec![packet(source, 1, false)])
            .expect("queued old command");
        let now = Instant::now();
        state.revision = u64::MAX;
        state.expires = Some(now);
        assert_eq!(state.set_source(Some(source), Some(now + RETRY), now), SourceUpdate::Changed);
        assert_eq!(state.revision, 0);
        assert!(state.source.is_none() && state.expires.is_none());
        assert!(state.outgoing.is_empty() && state.incoming.is_empty());
        assert!(state.connection().is_none() && state.connected(1).is_none());
        state.set_source(Some(source), Some(now + RETRY), now);
        assert_eq!(state.revision, 0);
        assert!(state.source.is_none() && state.expires.is_none());
    }

    #[test]
    fn worker_chat_retry_stays_bounded_across_failure_and_source_changes() {
        let start = Instant::now();
        let mut retry = Retry::new(start);
        assert!(retry.due(start));
        retry.attempted(start);
        assert!(!retry.due(start + RETRY - Duration::from_millis(1)));
        // A source change does not recreate/reset the retry schedule.
        assert!(retry.due(start + RETRY));
        retry.failed(start + Duration::from_millis(250));
        assert!(!retry.due(start + RETRY));
        assert!(retry.due(start + RETRY + Duration::from_millis(250)));
        retry.attempted(start + Duration::from_secs(2));
        assert!(!retry.due(start + Duration::from_millis(2999)));
    }

    #[test]
    fn worker_chat_drop_and_thread_exit_clear_readiness_without_joining_or_native_calls() {
        let (state, source, _, _) = state();
        let shared = Arc::new(Shared {
            state: Mutex::new(state),
            wake: Condvar::new(),
            ready: AtomicBool::new(true),
            stop: AtomicBool::new(false),
            clock: Instant::now(),
            deadline: AtomicU64::new(Duration::from_secs(3600).as_nanos() as u64),
        });
        let channel = WorkerChannel { shared: shared.clone() };
        assert!(channel.ready());
        {
            let _held = shared.state();
            assert!(channel.ready(), "bounded queue contention must not look like a socket disconnect");
            assert_eq!(channel.send(&packet(source, 1, true)), Err(LaneError::Busy));
        }
        drop(channel);
        assert!(!shared.ready.load(Ordering::Acquire));
        assert!(shared.stop.load(Ordering::Acquire));
        drop(Ended(shared.clone()));
        assert!(shared.state().connection().is_none());
    }

    #[test]
    fn worker_chat_ready_deadline_expires_even_if_application_and_io_have_not_ticked() {
        let (state, source, _, _) = state();
        // The IO thread has not yet retired State and its authenticated latch
        // still reads true. The monotonic receipt deadline alone rejects use.
        let shared = Arc::new(Shared {
            state: Mutex::new(state),
            wake: Condvar::new(),
            ready: AtomicBool::new(true),
            stop: AtomicBool::new(false),
            clock: Instant::now() - Duration::from_secs(2),
            deadline: AtomicU64::new(Duration::from_secs(1).as_nanos() as u64),
        });
        let channel = WorkerChannel { shared };
        assert!(!channel.ready());
        assert!(channel.commands().is_empty());
        assert_eq!(channel.send(&packet(source, 1, true)), Err(LaneError::NotReady));
    }

    #[test]
    fn worker_chat_io_arrival_stamp_survives_queue_wait_and_one_command_drain() {
        let (mut state, source, _, connection) = state();
        let received_at = Instant::now();
        state
            .incoming_at(connection, vec![packet(source, 1, false), packet(source, 2, false)], received_at)
            .unwrap();
        let first = state.commands().pop().unwrap();
        assert_eq!(first.received_at, received_at);
        let second = state.commands().pop().unwrap();
        assert_eq!(second.packet.sequence, 2);
        assert_eq!(second.received_at, received_at, "never restamp at App dequeue");
    }
}
