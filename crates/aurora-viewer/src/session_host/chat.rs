//! Main-thread chat backend. The socket lane owns IO; this module retains only
//! bounded live data and never interprets text as GUI input or a viewer command.

use aurora_session::Label;
use aurora_session::chat::{
    ChatCommand, ChatError, ChatEvent, ChatKey, ChatLine, ChatMessage, ChatPacket, ChatSource, ChatText, Conversation, ConversationInfo,
    MAX_CONVERSATIONS, MAX_RETAINED_LINES, NativeChatPolicy, SendAdmission, SendLedger, SendOutcome, SendReceipt, VERSION,
};
use aurora_session::chat::{MAX_READ_ROWS, MarkReadRequest, ReadMetadata, ReadOutcome, ReadReceiptLedger, ReadReceiptQuery, ReadStateRow};
use aurora_session::chat::{ReviewConversationRequest, ReviewOutcome, ReviewReceiptLedger, ReviewReceiptQuery, ReviewReceiptReply};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

const SCAN_CONVERSATIONS: usize = 4;
pub(crate) const SCAN_LINES: usize = 8;
const POLL_LINES: usize = 3;
pub(crate) const MAX_REPLY: usize = 11;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NativeCursor {
    pub token: u64,
    pub next: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeSnapshot {
    pub title: Label,
    /// Native rule/mute revisions and privacy options, not message contents.
    pub stamp: [u64; 4],
    pub ready: bool,
    pub can_send: bool,
    pub can_read: bool,
    pub initial: NativeCursor,
}

pub(crate) struct NativeLine {
    pub sender: Label,
    pub text: ChatText,
    pub kind: aurora_session::chat::LineKind,
    pub truncated: bool,
}
pub(crate) struct NativePage {
    pub cursor: NativeCursor,
    pub gap: bool,
    pub lines: Vec<NativeLine>,
}

pub(crate) trait NativeChat {
    /// Called again at operation boundaries; must evaluate the current control
    /// lease/account, not return a cached source captured before a long pause.
    fn source(&self) -> Option<ChatSource>;
    fn catalogue(&self) -> Vec<Conversation>;
    fn snapshot(&self, conversation: Conversation) -> Option<NativeSnapshot>;
    fn lines(&self, conversation: Conversation, cursor: NativeCursor, limit: usize) -> NativePage;
    fn submit_literal(&mut self, key: ChatKey, text: &str) -> SendOutcome;
    fn read_metadata(&self, _conversation: Conversation) -> Option<ReadMetadata> {
        None
    }
    fn mark_read(&mut self, _request: MarkReadRequest) -> ReadOutcome {
        ReadOutcome::RejectedNative
    }
    fn review_conversation(&mut self, _request: ReviewConversationRequest) -> ReviewOutcome {
        ReviewOutcome::RejectedNative
    }
}

struct ConversationState {
    snapshot: NativeSnapshot,
    policy: NativeChatPolicy,
    cursor: NativeCursor,
}

#[derive(Default)]
pub(crate) struct WorkerChat {
    source: Option<ChatSource>,
    retired: Option<ChatSource>,
    ledger: Option<SendLedger>,
    reads: Option<ReadReceiptLedger>,
    reviews: Option<ReviewReceiptLedger>,
    read_cache: BTreeMap<Conversation, Option<ReadMetadata>>,
    read_dirty: VecDeque<Conversation>,
    read_fair: usize,
    states: BTreeMap<Conversation, ConversationState>,
    lines: VecDeque<ChatLine>,
    receipt: Option<SendReceipt>,
    next_event: u64,
    next_policy: u64,
    gap_through: u64,
    scan: usize,
    metadata: usize,
    received: u64,
    outgoing: u64,
}

impl WorkerChat {
    pub fn current_source(&self) -> Option<ChatSource> {
        self.source
    }
    pub fn outgoing_sequence(&self) -> u64 {
        self.outgoing
    }

    /// Only for lost authority/logout, not a transient socket disconnect.
    /// Producer/receiver sequences survive; the same retired lease cannot
    /// silently recreate a ledger and replay an uncertain send.
    pub fn clear(&mut self) {
        if let Some(source) = self.source.take() {
            self.retired = Some(source);
        }
        self.ledger = None;
        self.reads = None;
        self.reviews = None;
        self.read_cache.clear();
        self.read_dirty.clear();
        self.read_fair = 0;
        self.states.clear();
        self.lines.clear();
        self.receipt = None;
        self.next_event = 0;
        self.next_policy = 0;
        self.gap_through = 0;
        self.scan = 0;
        self.metadata = 0;
    }

    fn context(&mut self, native: &impl NativeChat) -> Result<ChatSource, ChatError> {
        let Some(source) = native.source() else {
            self.clear();
            return Err(ChatError::Unavailable);
        };
        source.validate()?;
        if self.source == Some(source) {
            return Ok(source);
        }
        if let Some(old) = self.source.or(self.retired) {
            let mut check = SendLedger::new(old)?;
            if !check.reset(source)? {
                return Err(ChatError::Binding);
            }
        }
        self.clear();
        self.source = Some(source);
        self.ledger = Some(SendLedger::new(source)?);
        self.reads = Some(ReadReceiptLedger::new(source));
        self.reviews = Some(ReviewReceiptLedger::new(source)?);
        Ok(source)
    }

    fn event(&mut self) -> Result<u64, ChatError> {
        self.next_event = self.next_event.checked_add(1).ok_or(ChatError::Sequence)?;
        Ok(self.next_event)
    }
    fn gap(&mut self) -> Result<(), ChatError> {
        self.gap_through = self.event()?;
        Ok(())
    }

    fn refresh(&mut self, native: &impl NativeChat, conversation: Conversation) -> Result<(), ChatError> {
        let source = self.context(native)?;
        let snapshot = native.snapshot(conversation).ok_or(ChatError::Unavailable)?;
        if !self.states.contains_key(&conversation) && self.states.len() == MAX_CONVERSATIONS {
            return Err(ChatError::Capacity);
        }
        let read = if snapshot.ready && snapshot.can_read {
            native.read_metadata(conversation)
        } else {
            None
        };
        if self.read_cache.get(&conversation) != Some(&read) {
            self.read_cache.insert(conversation, read);
            if !self.read_dirty.contains(&conversation) {
                self.read_dirty.push_back(conversation);
            }
        }
        let existing = self.states.contains_key(&conversation);
        let changed = self.states.get(&conversation).is_none_or(|state| {
            state.snapshot.stamp != snapshot.stamp
                || state.snapshot.ready != snapshot.ready
                || state.snapshot.can_send != snapshot.can_send
                || state.snapshot.can_read != snapshot.can_read
        });
        if !changed {
            if let Some(state) = self.states.get_mut(&conversation) {
                state.snapshot.title = snapshot.title;
            }
            return Ok(());
        }
        if !existing && self.states.len() == MAX_CONVERSATIONS {
            return Err(ChatError::Capacity);
        }
        self.next_policy = self.next_policy.checked_add(1).ok_or(ChatError::Sequence)?;
        let key = ChatKey { source, conversation };
        let previous = self.lines.len();
        self.lines.retain(|line| line.key != key);
        // The host may still retain a title or text after this worker has
        // evicted its own duplicate. Invalidation cannot depend on retained
        // native lines, nor wait for this conversation's metadata rotation.
        if existing || previous != self.lines.len() {
            self.gap()?;
        }
        let policy = NativeChatPolicy {
            key,
            revision: self.next_policy,
            lease_current: true,
            ready: snapshot.ready,
            can_send: snapshot.can_send,
            can_read: snapshot.can_read,
        };
        let cursor = snapshot.initial;
        self.states.insert(conversation, ConversationState { snapshot, policy, cursor });
        Ok(())
    }

    /// Scans at most four conversations/eight native entries each. Network,
    /// transcript and inventory servicing remain owned by the ordinary viewer.
    pub fn sync(&mut self, native: &impl NativeChat) -> Result<(), ChatError> {
        let source = self.context(native)?;
        let catalogue: Vec<_> = native.catalogue().into_iter().take(MAX_CONVERSATIONS).collect();
        let known: BTreeSet<_> = catalogue.iter().copied().collect();
        self.read_cache.retain(|key, _| known.contains(key));
        self.read_dirty.retain(|key| known.contains(key));
        let previous_states = self.states.len();
        self.states.retain(|conversation, _| known.contains(conversation));
        let before = self.lines.len();
        self.lines.retain(|line| known.contains(&line.key.conversation));
        if previous_states != self.states.len() || before != self.lines.len() {
            self.gap()?;
        }
        for _ in 0..SCAN_CONVERSATIONS.min(catalogue.len()) {
            let conversation = catalogue[self.scan % catalogue.len()];
            self.scan = self.scan.wrapping_add(1);
            self.refresh(native, conversation)?;
            let state = self.states.get(&conversation).ok_or(ChatError::Unavailable)?;
            if !state.policy.ready || !state.policy.can_read {
                continue;
            }
            let policy = state.policy;
            let page = native.lines(conversation, state.cursor, SCAN_LINES);
            if let Some(state) = self.states.get_mut(&conversation) {
                state.cursor = page.cursor;
            }
            if page.gap {
                // Missing/cleared native history is ordinary loss, not a
                // privacy revocation. A skipped event marks consumer history
                // incomplete without purging other conversations or drafts.
                self.event()?;
            }
            for line in page.lines.into_iter().take(SCAN_LINES) {
                let event = self.event()?;
                if self.lines.len() == MAX_RETAINED_LINES {
                    self.lines.pop_front();
                }
                self.lines.push_back(ChatLine {
                    key: policy.key,
                    event,
                    policy_revision: policy.revision,
                    sender: line.sender,
                    text: line.text,
                    kind: line.kind,
                    truncated: line.truncated,
                });
            }
        }
        if self.context(native)? != source {
            return Err(ChatError::Binding);
        }
        Ok(())
    }

    fn packet(&mut self, reply_to: u64, event: ChatEvent) -> Result<ChatPacket, ChatError> {
        self.outgoing = self.outgoing.checked_add(1).ok_or(ChatError::Sequence)?;
        Ok(ChatPacket {
            version: VERSION,
            source: self.source.ok_or(ChatError::Unavailable)?,
            sequence: self.outgoing,
            reply_to,
            message: ChatMessage::Event(event),
        })
    }

    fn require_read_authority(&mut self, native: &impl NativeChat, key: ChatKey) -> Result<(), ChatError> {
        if self.context(native)? != key.source {
            return Err(ChatError::Binding);
        }
        if !matches!(key.conversation, Conversation::Nearby | Conversation::Direct(_))
            || !native
                .catalogue()
                .into_iter()
                .take(MAX_CONVERSATIONS)
                .any(|conversation| conversation == key.conversation)
        {
            return Err(ChatError::Unavailable);
        }
        self.refresh(native, key.conversation)?;
        let policy = self.states.get(&key.conversation).ok_or(ChatError::Unavailable)?.policy;
        if !policy.lease_current || !policy.ready || !policy.can_read {
            return Err(ChatError::Denied);
        }
        if self.context(native)? != key.source {
            return Err(ChatError::Binding);
        }
        Ok(())
    }

    fn read_batch(&mut self, native: &impl NativeChat) -> Result<Vec<ReadStateRow>, ChatError> {
        let keys: Vec<_> = native.catalogue().into_iter().take(MAX_CONVERSATIONS).collect();
        let mut chosen = Vec::new();
        for _ in 0..2 {
            if let Some(key) = self.read_dirty.pop_front() {
                chosen.push(key);
            }
        }
        // Two unconditional fair slots cannot be starved by a hot dirty key.
        for _ in 0..keys.len().min(MAX_READ_ROWS) {
            if chosen.len() == MAX_READ_ROWS || keys.is_empty() {
                break;
            }
            let key = keys[self.read_fair % keys.len()];
            self.read_fair = self.read_fair.wrapping_add(1);
            if !chosen.contains(&key) {
                chosen.push(key);
            }
        }
        let mut rows = Vec::new();
        for key in chosen {
            self.refresh(native, key)?;
            let state = self.states.get(&key).ok_or(ChatError::Unavailable)?;
            rows.push(ReadStateRow {
                conversation: key,
                title: state.snapshot.title.clone(),
                policy_revision: state.policy.revision,
                lease_current: state.policy.lease_current,
                ready: state.policy.ready,
                can_send: state.policy.can_send,
                can_read: state.policy.can_read,
                read: if state.policy.ready && state.policy.can_read {
                    native.read_metadata(key)
                } else {
                    None
                },
            });
        }
        Ok(rows)
    }

    #[cfg(test)]
    pub fn handle(&mut self, native: &mut impl NativeChat, packet: &ChatPacket) -> Result<Vec<ChatPacket>, ChatError> {
        // Unstamped callers cannot grant a delayed GUI effect.
        self.handle_at(native, packet, None, Instant::now())
    }

    pub fn handle_received(
        &mut self,
        native: &mut impl NativeChat,
        packet: &ChatPacket,
        received_at: Instant,
        now: Instant,
    ) -> Result<Vec<ChatPacket>, ChatError> {
        self.handle_at(native, packet, Some(received_at), now)
    }

    fn handle_at(
        &mut self,
        native: &mut impl NativeChat,
        packet: &ChatPacket,
        received_at: Option<Instant>,
        now: Instant,
    ) -> Result<Vec<ChatPacket>, ChatError> {
        packet.validate()?;
        let source = self.context(native)?;
        if packet.source != source {
            return Err(ChatError::Binding);
        }
        let ChatMessage::Command(command) = &packet.message else {
            return Err(ChatError::Invalid);
        };
        if packet.sequence <= self.received {
            return Err(ChatError::Sequence);
        }
        // Reserve sequence capacity before any native mutation, not while
        // constructing its reply afterward. The driver reserves queue space.
        self.outgoing.checked_add(MAX_REPLY as u64).ok_or(ChatError::Sequence)?;
        match command {
            ChatCommand::ReviewConversation(request) => {
                self.require_read_authority(native, request.target.key)?;
                let duplicate = self.reviews.as_mut().ok_or(ChatError::Unavailable)?.reserve(*request)?;
                let receipt = if let Some(receipt) = duplicate {
                    receipt
                } else {
                    let policy = self
                        .states
                        .get(&request.target.key.conversation)
                        .ok_or(ChatError::Unavailable)?
                        .policy;
                    let observed = native.read_metadata(request.target.key.conversation).map(|read| read.observed);
                    let age = received_at.and_then(|received| now.checked_duration_since(received));
                    let outcome = if age.is_none_or(|age| age > Duration::from_secs(2)) {
                        ReviewOutcome::Expired
                    } else if policy.revision != request.target.policy_revision
                        || observed.is_none_or(|current| {
                            current.epoch != request.target.observed.epoch || current.through < request.target.observed.through
                        })
                    {
                        ReviewOutcome::StaleTarget
                    } else if self.context(native)? != source {
                        ReviewOutcome::RejectedNative
                    } else {
                        native.review_conversation(*request)
                    };
                    self.reviews.as_mut().ok_or(ChatError::Unavailable)?.finish(*request, outcome)?
                };
                if self.context(native)? != source {
                    return Err(ChatError::Binding);
                }
                self.received = packet.sequence;
                Ok(vec![self.packet(
                    packet.sequence,
                    ChatEvent::ReviewReceipt(ReviewReceiptReply {
                        query: ReviewReceiptQuery {
                            key: request.target.key,
                            request: request.request,
                        },
                        receipt: Some(receipt),
                    }),
                )?])
            }
            ChatCommand::QueryReviewReceipt(query) => {
                // Receipt recovery is read-only and intentionally works in
                // Standby; it never obtains or waits for an Active grant.
                self.require_read_authority(native, query.key)?;
                let reply = self.reviews.as_ref().ok_or(ChatError::Unavailable)?.query(*query)?;
                if self.context(native)? != source {
                    return Err(ChatError::Binding);
                }
                self.received = packet.sequence;
                Ok(vec![self.packet(packet.sequence, ChatEvent::ReviewReceipt(reply))?])
            }
            ChatCommand::MarkRead(request) => {
                self.require_read_authority(native, request.key)?;
                let duplicate = self.reads.as_mut().ok_or(ChatError::Unavailable)?.reserve(*request)?;
                let receipt = if let Some(receipt) = duplicate {
                    receipt
                } else {
                    let supported = native
                        .catalogue()
                        .into_iter()
                        .take(MAX_CONVERSATIONS)
                        .any(|key| key == request.key.conversation);
                    let outcome = if supported {
                        self.refresh(native, request.key.conversation)?;
                        let policy = self.states.get(&request.key.conversation).ok_or(ChatError::Unavailable)?.policy;
                        if policy.key == request.key
                            && policy.revision == request.policy_revision
                            && policy.lease_current
                            && policy.ready
                            && policy.can_read
                            && self.context(native)? == source
                        {
                            native.mark_read(*request)
                        } else {
                            ReadOutcome::RejectedNative
                        }
                    } else {
                        ReadOutcome::RejectedNative
                    };
                    self.reads.as_mut().ok_or(ChatError::Unavailable)?.finish(*request, outcome)?
                };
                if self.context(native)? != source {
                    return Err(ChatError::Binding);
                }
                self.received = packet.sequence;
                // Prioritize a fresh native observation on the next Poll,
                // independently of the immutable operation receipt.
                self.read_dirty.retain(|key| *key != request.key.conversation);
                self.read_dirty.push_front(request.key.conversation);
                // Fresh metadata follows on Poll; receipt carries no old body.
                Ok(vec![self.packet(
                    packet.sequence,
                    ChatEvent::ReadReceipt(aurora_session::chat::ReadReceiptReply {
                        query: ReadReceiptQuery {
                            key: request.key,
                            request: request.request,
                        },
                        receipt: Some(receipt),
                    }),
                )?])
            }
            ChatCommand::QueryReadReceipt(query) => {
                self.require_read_authority(native, query.key)?;
                let reply = self.reads.as_ref().ok_or(ChatError::Unavailable)?.query(*query)?;
                if self.context(native)? != source {
                    return Err(ChatError::Binding);
                }
                self.received = packet.sequence;
                Ok(vec![self.packet(packet.sequence, ChatEvent::ReadReceipt(reply))?])
            }
            ChatCommand::Send(request) => {
                // Only a currently advertised native conversation can send.
                if !native
                    .catalogue()
                    .into_iter()
                    .take(MAX_CONVERSATIONS)
                    .any(|key| key == request.key.conversation)
                {
                    return Err(ChatError::Unavailable);
                }
                self.refresh(native, request.key.conversation)?;
                let policy = self.states.get(&request.key.conversation).ok_or(ChatError::Unavailable)?.policy;
                let admission = self.ledger.as_mut().ok_or(ChatError::Unavailable)?.admit(request, policy);
                let receipt = match admission {
                    Ok(SendAdmission::Duplicate(receipt)) => receipt,
                    Ok(SendAdmission::SubmitOnce) => {
                        if self.context(native)? != source {
                            return Err(ChatError::Binding);
                        }
                        let outcome = native.submit_literal(request.key, request.text.as_str());
                        self.ledger
                            .as_mut()
                            .ok_or(ChatError::Unavailable)?
                            .finish(request.key, request.request, outcome)?
                    }
                    Err(ChatError::Denied | ChatError::Unavailable | ChatError::StalePolicy | ChatError::Busy) => SendReceipt {
                        key: request.key,
                        request: request.request,
                        outcome: SendOutcome::RejectedNative,
                    },
                    Err(error) => return Err(error),
                };
                if self.context(native)? != source {
                    return Err(ChatError::Binding);
                }
                self.received = packet.sequence;
                self.receipt = Some(receipt);
                Ok(vec![self.packet(packet.sequence, ChatEvent::Receipt(receipt))?])
            }
            ChatCommand::Poll { after_event } => {
                self.sync(native)?;
                let keys: Vec<_> = self.states.keys().copied().collect();
                let metadata = if !keys.is_empty() {
                    let key = keys[self.metadata % keys.len()];
                    self.metadata = self.metadata.wrapping_add(1);
                    self.refresh(native, key)?;
                    Some(key)
                } else {
                    None
                };
                // Recheck current native restrictions on every exported line,
                // including conversations outside this tick's scan rotation.
                // Stage all native refreshes first: any can invalidate host
                // caches, and that reset must precede every new metadata line.
                let after = (*after_event).max(self.gap_through);
                let selected: Vec<_> = self
                    .lines
                    .iter()
                    .filter(|line| line.event > after)
                    .take(POLL_LINES)
                    .cloned()
                    .collect();
                for line in &selected {
                    self.refresh(native, line.key.conversation)?;
                }
                // read_batch may revoke policies. Stage it before Gap and all
                // other data so old private caches are purged first.
                let read_rows = self.read_batch(native)?;
                let mut events = Vec::new();
                let mut advertised = BTreeSet::new();
                if *after_event < self.gap_through {
                    events.push(ChatEvent::Gap {
                        through_event: self.gap_through,
                    });
                }
                if let Some(key) = metadata {
                    let state = self.states.get(&key).ok_or(ChatError::Unavailable)?;
                    events.push(ChatEvent::Conversation(ConversationInfo {
                        policy: state.policy,
                        title: state.snapshot.title.clone(),
                    }));
                    advertised.insert(key);
                }
                let after = (*after_event).max(self.gap_through);
                for line in selected.into_iter().filter(|line| line.event > after) {
                    let current = self.states.get(&line.key.conversation).ok_or(ChatError::Unavailable)?;
                    if current.policy.can_read && current.policy.ready && current.policy.revision == line.policy_revision {
                        // The host must never invent permissions for a line
                        // outside the rotating metadata conversation. Send
                        // its trusted current policy first in the same batch.
                        if advertised.insert(line.key.conversation) {
                            let info = ConversationInfo {
                                policy: current.policy,
                                title: current.snapshot.title.clone(),
                            };
                            events.push(ChatEvent::Conversation(info));
                        }
                        events.push(ChatEvent::Line(line));
                    }
                }
                events.push(ChatEvent::ReadBatch(read_rows));
                if let Some(receipt) = self.receipt {
                    events.push(ChatEvent::Receipt(receipt));
                }
                if self.context(native)? != source {
                    return Err(ChatError::Binding);
                }
                let result = events
                    .into_iter()
                    .map(|event| self.packet(packet.sequence, event))
                    .collect::<Result<Vec<_>, _>>()?;
                self.received = packet.sequence;
                Ok(result)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::chat::{ChatDrafts, ChatHistory, LineKind, MAX_TEXT_BYTES, SendRequest};
    use aurora_session::{
        AccountKey, ConversationId, GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerId, WorkerIdentity,
    };
    use std::cell::Cell;

    fn source(generation: u64) -> ChatSource {
        ChatSource {
            worker: WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).unwrap(),
                host: HostId::from_bytes([2; 16]).unwrap(),
                worker: WorkerId::from_bytes([3; 16]).unwrap(),
            },
            session: SessionIdentity {
                session: SessionId::from_bytes([4; 16]).unwrap(),
                account: AccountKey::from_bytes([5; 32]).unwrap(),
                grid: GridKey::from_bytes([6; 32]).unwrap(),
                generation,
            },
            lease_epoch: 1,
        }
    }

    struct Native {
        source: ChatSource,
        calls: Cell<usize>,
        change: Option<(usize, Option<ChatSource>)>,
        revision: u64,
        readable: bool,
        entries: u64,
        direct_entries: Option<u64>,
        token: u64,
        initial: u64,
        largest_scan: Cell<usize>,
        submits: usize,
        direct: bool,
        snapshots: Cell<usize>,
        change_policy: Option<(usize, u64)>,
    }
    impl Native {
        fn new() -> Self {
            Self {
                source: source(1),
                calls: Cell::new(0),
                change: None,
                revision: 0,
                readable: true,
                entries: 0,
                direct_entries: None,
                token: 1,
                initial: 0,
                largest_scan: Cell::new(0),
                submits: 0,
                direct: false,
                snapshots: Cell::new(0),
                change_policy: None,
            }
        }
    }
    impl NativeChat for Native {
        fn source(&self) -> Option<ChatSource> {
            let call = self.calls.get() + 1;
            self.calls.set(call);
            if let Some((at, replacement)) = self.change
                && call >= at
            {
                return replacement;
            }
            Some(self.source)
        }
        fn catalogue(&self) -> Vec<Conversation> {
            let mut conversations = vec![Conversation::Nearby];
            if self.direct {
                conversations.push(Conversation::Direct(ConversationId::from_bytes([7; 16]).unwrap()));
            }
            conversations
        }
        fn snapshot(&self, conversation: Conversation) -> Option<NativeSnapshot> {
            let call = self.snapshots.get() + 1;
            self.snapshots.set(call);
            let revision = self
                .change_policy
                .filter(|_| matches!(conversation, Conversation::Direct(_)))
                .filter(|(at, _)| call >= *at)
                .map_or(self.revision, |(_, revision)| revision);
            Some(NativeSnapshot {
                title: Label::new("Synthetic nearby").unwrap(),
                stamp: [revision, 0, 0, 0],
                ready: true,
                can_send: true,
                can_read: self.readable,
                initial: NativeCursor {
                    token: self.token,
                    next: self.initial,
                },
            })
        }
        fn lines(&self, conversation: Conversation, cursor: NativeCursor, limit: usize) -> NativePage {
            self.largest_scan.set(self.largest_scan.get().max(limit));
            let entries = if matches!(conversation, Conversation::Direct(_)) {
                self.direct_entries.unwrap_or(self.entries)
            } else {
                self.entries
            };
            if cursor.token != self.token {
                return NativePage {
                    cursor: NativeCursor {
                        token: self.token,
                        next: entries,
                    },
                    gap: true,
                    lines: Vec::new(),
                };
            }
            let end = cursor.next.saturating_add(limit as u64).min(entries);
            let lines = (cursor.next..end)
                .map(|_| NativeLine {
                    sender: Label::new("Synthetic sender").unwrap(),
                    text: ChatText::new("Synthetic live text").unwrap(),
                    kind: LineKind::Incoming,
                    truncated: false,
                })
                .collect();
            NativePage {
                cursor: NativeCursor {
                    token: self.token,
                    next: end,
                },
                gap: false,
                lines,
            }
        }
        fn submit_literal(&mut self, key: ChatKey, _: &str) -> SendOutcome {
            assert_eq!(key.source, self.source);
            self.submits += 1;
            // Models the actual App API: the enqueue result is unavailable.
            SendOutcome::Uncertain
        }
    }

    fn send(source: ChatSource, revision: u64) -> ChatPacket {
        ChatPacket {
            version: VERSION,
            source,
            sequence: 1,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::Send(SendRequest {
                key: ChatKey {
                    source,
                    conversation: Conversation::Nearby,
                },
                request: 1,
                policy_revision: revision,
                text: ChatText::new("Synthetic literal text").unwrap(),
            })),
        }
    }
    fn poll(source: ChatSource, sequence: u64) -> ChatPacket {
        ChatPacket {
            version: VERSION,
            source,
            sequence,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::Poll { after_event: 0 }),
        }
    }

    fn retain_without_reset(history: &mut ChatHistory, policies: &mut BTreeMap<Conversation, NativeChatPolicy>, packets: &[ChatPacket]) {
        for packet in packets {
            match &packet.message {
                ChatMessage::Event(ChatEvent::Conversation(info)) => {
                    history.apply_policy(info.policy).unwrap();
                    policies.insert(info.policy.key.conversation, info.policy);
                }
                ChatMessage::Event(ChatEvent::Line(line)) => {
                    history.push(line.clone(), policies[&line.key.conversation]).unwrap();
                }
                ChatMessage::Event(ChatEvent::Gap { .. }) => panic!("ordinary history loss must not reset consumer policy or drafts"),
                _ => {}
            }
        }
    }

    #[test]
    fn host_chat_backend_rechecks_source_after_admission_before_any_native_mutation() {
        for replacement in [Some(source(2)), None] {
            let mut native = Native::new();
            let mut backend = WorkerChat::default();
            backend.sync(&native).unwrap();
            let revision = backend.states[&Conversation::Nearby].policy.revision;
            native.calls.set(0);
            // initial context, policy refresh, then the pre-submit boundary.
            native.change = Some((3, replacement));
            let error = backend.handle(&mut native, &send(source(1), revision)).unwrap_err();
            assert_eq!(
                error,
                if replacement.is_some() {
                    ChatError::Binding
                } else {
                    ChatError::Unavailable
                }
            );
            assert_eq!(native.submits, 0);
            assert_eq!(backend.received, 0);
            assert_eq!(backend.outgoing_sequence(), 0);
            assert!(backend.lines.is_empty() && backend.receipt.is_none());
            assert_eq!(backend.current_source(), replacement);
        }
    }

    #[test]
    fn host_chat_backend_lost_receipt_and_reconnect_never_resubmit_uncertain_send() {
        let mut native = Native::new();
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        let revision = backend.states[&Conversation::Nearby].policy.revision;
        let mut request = send(source(1), revision);
        let first = backend.handle(&mut native, &request).unwrap();
        request.sequence = 2; // Same request after a lost reply / socket reconnect.
        let duplicate = backend.handle(&mut native, &request).unwrap();
        assert_eq!(native.submits, 1);
        for reply in [&first[0], &duplicate[0]] {
            assert!(matches!(
                reply.message,
                ChatMessage::Event(ChatEvent::Receipt(SendReceipt {
                    outcome: SendOutcome::Uncertain,
                    ..
                }))
            ));
        }
        assert!(duplicate[0].sequence > first[0].sequence);
        backend.clear(); // Authority loss is different from a socket reconnect.
        assert_eq!(backend.sync(&native), Err(ChatError::Binding));
        assert_eq!(native.submits, 1);
        native.source = source(2);
        backend.sync(&native).unwrap();
        let next = backend.handle(&mut native, &poll(source(2), 3)).unwrap();
        assert!(
            next.iter()
                .all(|packet| packet.sequence > duplicate[0].sequence && packet.source == source(2))
        );
    }

    #[test]
    fn host_chat_backend_invalid_source_and_direction_cannot_poison_receiver_sequence() {
        let mut native = Native::new();
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        assert_eq!(backend.handle(&mut native, &poll(source(2), 9000)), Err(ChatError::Binding));
        let invalid = ChatPacket {
            version: VERSION,
            source: source(1),
            sequence: 9000,
            reply_to: 0,
            message: ChatMessage::Event(ChatEvent::Unsupported),
        };
        assert_eq!(backend.handle(&mut native, &invalid), Err(ChatError::Invalid));
        assert_eq!(backend.received, 0);
        let valid = backend.handle(&mut native, &poll(source(1), 1)).unwrap();
        assert!(!valid.is_empty());
        assert_eq!(backend.received, 1);
    }

    #[test]
    fn host_chat_backend_retention_scan_and_batches_are_bounded_and_restrictions_purge_text() {
        let mut native = Native::new();
        native.entries = 1024;
        let mut backend = WorkerChat::default();
        for _ in 0..40 {
            backend.sync(&native).unwrap();
        }
        assert_eq!(backend.lines.len(), MAX_RETAINED_LINES);
        assert_eq!(native.largest_scan.get(), SCAN_LINES);
        let replies = backend.handle(&mut native, &poll(source(1), 1)).unwrap();
        assert!(replies.len() <= 11);
        assert!(
            replies
                .iter()
                .filter(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Line(_))))
                .count()
                <= POLL_LINES
        );
        assert!(replies.iter().all(|packet| aurora_session::chat::encode(packet).is_ok()));
        native.revision += 1;
        native.readable = false;
        native.initial = native.entries;
        let restricted = backend.handle(&mut native, &poll(source(1), 2)).unwrap();
        assert!(backend.lines.is_empty());
        assert!(
            !restricted
                .iter()
                .any(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Line(_))))
        );
        assert!(
            restricted
                .iter()
                .any(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Conversation(ref info)) if !info.policy.can_read))
        );
    }

    #[test]
    fn host_chat_backend_nearby_overflow_preserves_direct_policy_history_and_draft() {
        let mut native = Native::new();
        native.direct = true;
        native.direct_entries = Some(1);
        let mut backend = WorkerChat::default();
        let mut history = ChatHistory::new(source(1)).unwrap();
        let mut policies = BTreeMap::new();
        let first = backend.handle(&mut native, &poll(source(1), 1)).unwrap();
        retain_without_reset(&mut history, &mut policies, &first);
        let direct = Conversation::Direct(ConversationId::from_bytes([7; 16]).unwrap());
        let direct_policy = policies[&direct];
        assert_eq!(history.lines(direct_policy).count(), 1);
        let mut drafts = ChatDrafts::new(source(1)).unwrap();
        let draft = ChatText::new("Synthetic inactive IM draft").unwrap();
        drafts.set(direct_policy.key, draft.clone()).unwrap();

        native.entries = 1024;
        for _ in 0..40 {
            backend.sync(&native).unwrap();
        }
        assert_eq!(backend.lines.len(), MAX_RETAINED_LINES);
        assert!(backend.lines.iter().all(|line| line.key.conversation == Conversation::Nearby));
        assert_eq!(backend.gap_through, 0);
        let mut request = poll(source(1), 2);
        request.message = ChatMessage::Command(ChatCommand::Poll { after_event: 1 });
        let replies = backend.handle(&mut native, &request).unwrap();
        assert!(replies.len() <= 11);
        retain_without_reset(&mut history, &mut policies, &replies);
        assert!(history.has_gap(), "skipped Nearby events still mark incomplete history");
        assert_eq!(policies[&direct], direct_policy);
        assert_eq!(backend.states[&direct].policy, direct_policy);
        assert_eq!(history.lines(direct_policy).count(), 1);
        assert_eq!(drafts.get(direct_policy.key), Some(&draft));
    }

    #[test]
    fn host_chat_backend_native_cursor_reset_marks_loss_without_policy_reset() {
        let mut native = Native::new();
        native.entries = 1;
        let mut backend = WorkerChat::default();
        let mut history = ChatHistory::new(source(1)).unwrap();
        let mut policies = BTreeMap::new();
        let first = backend.handle(&mut native, &poll(source(1), 1)).unwrap();
        retain_without_reset(&mut history, &mut policies, &first);
        let policy = policies[&Conversation::Nearby];
        assert!(!history.has_gap());

        // Mirrors the native adapter's cleared/replaced LineClock token:
        // the page skips to the current end and reports ordinary history loss.
        native.token += 1;
        backend.sync(&native).unwrap();
        assert_eq!(backend.states[&Conversation::Nearby].cursor.token, native.token);
        assert_eq!(backend.gap_through, 0);
        native.entries += 1;
        let mut request = poll(source(1), 2);
        request.message = ChatMessage::Command(ChatCommand::Poll { after_event: 1 });
        let replies = backend.handle(&mut native, &request).unwrap();
        retain_without_reset(&mut history, &mut policies, &replies);
        assert!(history.has_gap());
        assert_eq!(policies[&Conversation::Nearby], policy);
        assert_eq!(history.lines(policy).count(), 2);
    }

    #[test]
    fn host_chat_backend_each_exported_conversation_supplies_native_policy_before_its_lines() {
        let mut native = Native::new();
        native.direct = true;
        native.entries = 8;
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        let mut request = poll(source(1), 1);
        // The rotating metadata begins with Nearby, but the selected retained
        // lines belong to a Direct conversation the host has not seen yet.
        request.message = ChatMessage::Command(ChatCommand::Poll { after_event: 8 });
        let replies = backend.handle(&mut native, &request).unwrap();
        let mut policies = BTreeMap::new();
        let mut lines = 0;
        for reply in replies {
            match reply.message {
                ChatMessage::Event(ChatEvent::Conversation(info)) => {
                    policies.insert(info.policy.key.conversation, info.policy);
                }
                ChatMessage::Event(ChatEvent::Line(line)) => {
                    let policy = policies.get(&line.key.conversation).expect("same-batch native policy before line");
                    assert_eq!(line.key, policy.key);
                    assert_eq!(line.policy_revision, policy.revision);
                    assert!(policy.can_read && policy.ready && policy.lease_current);
                    assert!(matches!(line.key.conversation, Conversation::Direct(_)));
                    lines += 1;
                }
                _ => {}
            }
        }
        assert_eq!(lines, POLL_LINES);
    }

    #[test]
    fn host_chat_backend_policy_change_resets_host_cache_even_after_native_history_is_empty() {
        let mut native = Native::new();
        native.entries = 8;
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        let first = backend.handle(&mut native, &poll(source(1), 1)).unwrap();
        let cached = first
            .iter()
            .find_map(|packet| match &packet.message {
                ChatMessage::Event(ChatEvent::Line(line)) => Some(line.clone()),
                _ => None,
            })
            .expect("host retained a real exported line");
        let mut catch_up = poll(source(1), 2);
        catch_up.message = ChatMessage::Command(ChatCommand::Poll {
            after_event: 8 - POLL_LINES as u64,
        });
        let second = backend.handle(&mut native, &catch_up).unwrap();
        let after_event = second
            .iter()
            .filter_map(|packet| match &packet.message {
                ChatMessage::Event(ChatEvent::Line(line)) => Some(line.event),
                _ => None,
            })
            .max()
            .unwrap();
        assert_eq!(after_event, 8);
        // Model the consumer retaining a line after its producer no longer
        // retains a duplicate. A privacy reset must not depend on that copy.
        backend.lines.clear();
        native.entries = 0;
        native.revision += 1;
        native.readable = false;
        let mut request = poll(source(1), 3);
        request.message = ChatMessage::Command(ChatCommand::Poll { after_event });
        let replies = backend.handle(&mut native, &request).unwrap();
        assert!(backend.lines.is_empty());
        assert!(matches!(replies[0].message, ChatMessage::Event(ChatEvent::Gap { through_event }) if through_event > after_event));
        assert!(matches!(&replies[1].message, ChatMessage::Event(ChatEvent::Conversation(info))
            if info.policy.key == cached.key && info.policy.revision > cached.policy_revision && !info.policy.can_read));
        assert!(
            !replies
                .iter()
                .any(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Line(_))))
        );
        assert!(replies.len() <= 11);
        assert!(replies.iter().all(|packet| aurora_session::chat::encode(packet).is_ok()));
    }

    #[test]
    fn host_chat_backend_closed_conversation_resets_cached_titles_without_native_lines() {
        let mut native = Native::new();
        native.direct = true;
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        assert!(backend.lines.is_empty());
        let first = backend.handle(&mut native, &poll(source(1), 1)).unwrap();
        let second = backend.handle(&mut native, &poll(source(1), 2)).unwrap();
        assert!(
            !first
                .iter()
                .chain(&second)
                .any(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Gap { .. }))),
            "first policy discovery does not invalidate an existing cache"
        );
        assert!(
            second
                .iter()
                .any(|packet| matches!(&packet.message, ChatMessage::Event(ChatEvent::Conversation(info))
            if matches!(info.policy.key.conversation, Conversation::Direct(_))))
        );
        native.direct = false;
        let closed = backend.handle(&mut native, &poll(source(1), 3)).unwrap();
        assert!(matches!(closed[0].message, ChatMessage::Event(ChatEvent::Gap { .. })));
        assert!(closed.iter().all(
            |packet| !matches!(&packet.message, ChatMessage::Event(ChatEvent::Conversation(info))
            if matches!(info.policy.key.conversation, Conversation::Direct(_)))
        ));
    }

    #[test]
    fn host_chat_backend_late_line_preflight_reset_still_precedes_all_metadata() {
        let mut native = Native::new();
        native.direct = true;
        native.entries = 8;
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        native.snapshots.set(0);
        // Two sync snapshots, rotating Nearby metadata, then the selected
        // Direct line's preflight discovers a changed existing policy.
        native.change_policy = Some((4, 1));
        let mut request = poll(source(1), 1);
        request.message = ChatMessage::Command(ChatCommand::Poll { after_event: 8 });
        let replies = backend.handle(&mut native, &request).unwrap();
        assert!(matches!(replies[0].message, ChatMessage::Event(ChatEvent::Gap { through_event }) if through_event > 16));
        assert!(
            replies
                .iter()
                .skip(1)
                .any(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Conversation(_))))
        );
        assert!(
            !replies
                .iter()
                .any(|packet| matches!(packet.message, ChatMessage::Event(ChatEvent::Line(_))))
        );
        assert!(replies.len() <= 11);
    }
    struct ReadCatalogue {
        revision: u64,
        reads: Cell<usize>,
    }
    impl NativeChat for ReadCatalogue {
        fn source(&self) -> Option<ChatSource> {
            Some(source(1))
        }
        fn catalogue(&self) -> Vec<Conversation> {
            std::iter::once(Conversation::Nearby)
                .chain((1..64).map(|i| Conversation::Direct(ConversationId::from_bytes([i; 16]).unwrap())))
                .collect()
        }
        fn snapshot(&self, _: Conversation) -> Option<NativeSnapshot> {
            Some(NativeSnapshot {
                title: Label::new("Synthetic").unwrap(),
                stamp: [0; 4],
                ready: true,
                can_send: true,
                can_read: true,
                initial: NativeCursor::default(),
            })
        }
        fn lines(&self, _: Conversation, cursor: NativeCursor, _: usize) -> NativePage {
            NativePage {
                cursor,
                gap: false,
                lines: Vec::new(),
            }
        }
        fn submit_literal(&mut self, _: ChatKey, _: &str) -> SendOutcome {
            panic!("metadata never sends")
        }
        fn read_metadata(&self, _: Conversation) -> Option<ReadMetadata> {
            self.reads.set(self.reads.get() + 1);
            Some(ReadMetadata {
                unread: 1,
                unread_revision: self.revision,
                observed: aurora_session::chat::ObservedIncoming {
                    epoch: 1,
                    through: self.revision,
                },
                can_mark_read: true,
            })
        }
    }

    #[test]
    fn host_chat_read_metadata_fairness_and_true_maximum_reply_budget_are_bounded() {
        let mut native = ReadCatalogue {
            revision: 1,
            reads: Cell::new(0),
        };
        let mut backend = WorkerChat::default();
        let mut seen = BTreeSet::new();
        for sequence in 1..=32 {
            native.revision += 1; // all conversations continuously dirty
            let packets = backend.handle(&mut native, &poll(source(1), sequence)).unwrap();
            assert!(packets.len() <= MAX_REPLY);
            for packet in packets {
                if let ChatMessage::Event(ChatEvent::ReadBatch(rows)) = packet.message {
                    assert!(rows.len() <= MAX_READ_ROWS);
                    seen.extend(rows.into_iter().map(|row| row.conversation));
                }
            }
            assert!(backend.read_cache.len() <= MAX_CONVERSATIONS && backend.read_dirty.len() <= MAX_CONVERSATIONS);
        }
        assert_eq!(seen.len(), MAX_CONVERSATIONS, "fair cursor survives constant dirty traffic");
        // Populate exactly three distinct line policies, different from the
        // rotating policy, plus a gap and an existing send receipt.
        backend.metadata = 0;
        backend.gap_through = 1;
        backend.lines.clear();
        let keys = native.catalogue();
        for (offset, conversation) in keys.iter().copied().skip(1).take(3).enumerate() {
            backend.refresh(&native, conversation).unwrap();
            let policy = backend.states[&conversation].policy;
            backend.lines.push_back(ChatLine {
                key: policy.key,
                event: offset as u64 + 2,
                policy_revision: policy.revision,
                sender: Label::new("Synthetic").unwrap(),
                text: ChatText::new(&"\"".repeat(MAX_TEXT_BYTES)).unwrap(),
                kind: LineKind::Incoming,
                truncated: false,
            });
        }
        backend.receipt = Some(SendReceipt {
            key: backend.states[&Conversation::Nearby].policy.key,
            request: 1,
            outcome: SendOutcome::Uncertain,
        });
        let packets = backend.handle(&mut native, &poll(source(1), 33)).unwrap();
        assert_eq!(
            packets.len(),
            10,
            "gap1 + policy1 + 3*(policy1+line1) + read batch1 + send receipt1"
        );
        assert!(packets.iter().all(|packet| aurora_session::chat::encode(packet).is_ok()));
        assert!(matches!(packets[0].message, ChatMessage::Event(ChatEvent::Gap { .. })));
        let mut policies = BTreeSet::new();
        for packet in packets {
            match packet.message {
                ChatMessage::Event(ChatEvent::Conversation(info)) => {
                    policies.insert(info.policy.key.conversation);
                }
                ChatMessage::Event(ChatEvent::Line(line)) => assert!(policies.contains(&line.key.conversation)),
                _ => {}
            }
        }
    }

    #[test]
    fn host_chat_read_sequence_capacity_is_reserved_before_native_counter_mutation() {
        let mut native = Native::new();
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        let policy = backend.states[&Conversation::Nearby].policy;
        let request = MarkReadRequest {
            key: policy.key,
            request: 1,
            policy_revision: policy.revision,
            observed: aurora_session::chat::ObservedIncoming { epoch: 1, through: 0 },
            unread_revision: 1,
        };
        backend.outgoing = u64::MAX - MAX_REPLY as u64 + 1;
        let packet = ChatPacket {
            version: VERSION,
            source: source(1),
            sequence: 1,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::MarkRead(request)),
        };
        assert_eq!(backend.handle(&mut native, &packet), Err(ChatError::Sequence));
        assert_eq!(
            backend.reads.as_mut().unwrap().reserve(request),
            Ok(None),
            "capacity rejection occurs before even reserving a mutation identity"
        );
    }

    struct ReviewNative {
        inner: Native,
        read: ReadMetadata,
        opens: Vec<ReviewConversationRequest>,
        outcome: ReviewOutcome,
    }
    impl ReviewNative {
        fn new() -> Self {
            Self {
                inner: Native::new(),
                read: ReadMetadata {
                    unread: 1,
                    unread_revision: 1,
                    observed: aurora_session::chat::ObservedIncoming { epoch: 1, through: 1 },
                    can_mark_read: false,
                },
                opens: Vec::new(),
                outcome: ReviewOutcome::OpenedLocally,
            }
        }
    }
    impl NativeChat for ReviewNative {
        fn source(&self) -> Option<ChatSource> {
            self.inner.source()
        }
        fn catalogue(&self) -> Vec<Conversation> {
            self.inner.catalogue()
        }
        fn snapshot(&self, conversation: Conversation) -> Option<NativeSnapshot> {
            let mut snapshot = self.inner.snapshot(conversation)?;
            snapshot.can_send = false;
            Some(snapshot)
        }
        fn lines(&self, conversation: Conversation, cursor: NativeCursor, limit: usize) -> NativePage {
            self.inner.lines(conversation, cursor, limit)
        }
        fn read_metadata(&self, _: Conversation) -> Option<ReadMetadata> {
            Some(self.read)
        }
        fn submit_literal(&mut self, _: ChatKey, _: &str) -> SendOutcome {
            panic!("Review cannot send")
        }
        fn mark_read(&mut self, _: MarkReadRequest) -> ReadOutcome {
            panic!("Review cannot acknowledge Read")
        }
        fn review_conversation(&mut self, request: ReviewConversationRequest) -> ReviewOutcome {
            self.opens.push(request);
            self.outcome
        }
    }
    fn review_request(backend: &WorkerChat, native: &ReviewNative) -> ReviewConversationRequest {
        let policy = backend.states[&Conversation::Nearby].policy;
        ReviewConversationRequest {
            target: aurora_session::chat::ReviewCapture {
                key: policy.key,
                policy_revision: policy.revision,
                observed: native.read.observed,
            },
            request: 1,
            activation: 7,
        }
    }
    fn review_packet(request: ReviewConversationRequest, sequence: u64) -> ChatPacket {
        ChatPacket {
            version: VERSION,
            source: request.target.key.source,
            sequence,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::ReviewConversation(request)),
        }
    }
    fn review_outcome(packets: &[ChatPacket]) -> ReviewOutcome {
        match &packets[0].message {
            ChatMessage::Event(ChatEvent::ReviewReceipt(reply)) => reply.receipt.unwrap().outcome,
            _ => panic!("typed Review receipt"),
        }
    }

    #[test]
    fn host_review_worker_opens_once_and_receipt_query_never_requires_foreground_or_reopens() {
        let mut native = ReviewNative::new();
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        let request = review_request(&backend, &native);
        let now = Instant::now();
        assert_eq!(
            review_outcome(&backend.handle_received(&mut native, &review_packet(request, 1), now, now).unwrap()),
            ReviewOutcome::OpenedLocally
        );
        native.outcome = ReviewOutcome::NotForeground;
        assert_eq!(
            review_outcome(
                &backend
                    .handle_received(&mut native, &review_packet(request, 2), now, now + Duration::from_secs(9))
                    .unwrap()
            ),
            ReviewOutcome::OpenedLocally
        );
        let packet = ChatPacket {
            sequence: 3,
            message: ChatMessage::Command(ChatCommand::QueryReviewReceipt(ReviewReceiptQuery {
                key: request.target.key,
                request: 1,
            })),
            ..review_packet(request, 1)
        };
        assert_eq!(
            review_outcome(&backend.handle(&mut native, &packet).unwrap()),
            ReviewOutcome::OpenedLocally
        );
        assert_eq!(native.opens, vec![request]);
        assert_eq!(native.inner.submits, 0);
    }

    #[test]
    fn host_review_worker_io_age_and_unstamped_entry_expire_terminally_without_opening() {
        for unstamped in [false, true] {
            let mut native = ReviewNative::new();
            let mut backend = WorkerChat::default();
            backend.sync(&native).unwrap();
            let request = review_request(&backend, &native);
            let now = Instant::now();
            let packet = review_packet(request, 1);
            let result = if unstamped {
                backend.handle(&mut native, &packet)
            } else {
                backend.handle_received(&mut native, &packet, now, now + Duration::from_millis(2001))
            };
            assert_eq!(review_outcome(&result.unwrap()), ReviewOutcome::Expired);
            assert_eq!(backend.received, 1);
            assert_eq!(
                review_outcome(&backend.handle_received(&mut native, &review_packet(request, 2), now, now).unwrap()),
                ReviewOutcome::Expired
            );
            assert!(native.opens.is_empty());
        }
    }

    #[test]
    fn host_review_worker_observation_allows_arrivals_and_native_read_but_rejects_replacement_or_policy_change() {
        for change in 0..5 {
            let mut native = ReviewNative::new();
            let mut backend = WorkerChat::default();
            backend.sync(&native).unwrap();
            let request = review_request(&backend, &native);
            match change {
                0 => {
                    native.read.unread = 0;
                    native.read.unread_revision += 1;
                }
                1 => {
                    native.read.observed.through += 1;
                    native.read.unread += 1;
                    native.read.unread_revision += 1;
                }
                2 => native.read.observed.epoch += 1,
                3 => native.read.observed.through = 0,
                4 => native.inner.revision += 1,
                _ => unreachable!(),
            }
            let now = Instant::now();
            let outcome = review_outcome(&backend.handle_received(&mut native, &review_packet(request, 1), now, now).unwrap());
            assert_eq!(
                outcome,
                if change < 2 {
                    ReviewOutcome::OpenedLocally
                } else {
                    ReviewOutcome::StaleTarget
                }
            );
            assert_eq!(native.opens.len(), usize::from(change < 2));
        }
    }

    #[test]
    fn host_review_worker_reserves_sequence_and_immutable_identity_before_native_effect() {
        let mut native = ReviewNative::new();
        let mut backend = WorkerChat::default();
        backend.sync(&native).unwrap();
        let request = review_request(&backend, &native);
        let now = Instant::now();
        backend.outgoing = u64::MAX - MAX_REPLY as u64 + 1;
        assert_eq!(
            backend.handle_received(&mut native, &review_packet(request, 1), now, now),
            Err(ChatError::Sequence)
        );
        assert!(native.opens.is_empty());
        assert_eq!(backend.received, 0);
        backend.outgoing = 0;
        backend.handle_received(&mut native, &review_packet(request, 1), now, now).unwrap();
        let changed = ReviewConversationRequest { activation: 8, ..request };
        assert_eq!(
            backend.handle_received(&mut native, &review_packet(changed, 2), now, now),
            Err(ChatError::RequestChanged)
        );
        assert_eq!(native.opens.len(), 1);
        assert_eq!(backend.received, 1);
        native.inner.readable = false;
        let query = ChatPacket {
            message: ChatMessage::Command(ChatCommand::QueryReviewReceipt(ReviewReceiptQuery {
                key: request.target.key,
                request: 1,
            })),
            ..review_packet(request, 2)
        };
        assert_eq!(backend.handle(&mut native, &query), Err(ChatError::Denied));
        assert_eq!(native.opens.len(), 1);
    }
}
