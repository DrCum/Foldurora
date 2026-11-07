//! Body-free board intent and exact native-run ownership. This never changes
//! app/gestures::Load.play/active/open flags or performs native IO itself.
use super::model::{BoardId, Catalogue, Kind, MAX_BOARDS, MAX_TILES, TileId};
use crate::gestures::{Effect, Manager, PlaybackId};
use crate::world::inventory::actions::{PlaybackItem, PlaybackKind};
use aurora_assets::gesture::Gesture;
use aurora_session::{GridKey, chat::ChatSource};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use uuid::Uuid;
const MAX_PENDING: usize = 32;
const DEADLINE: Duration = Duration::from_secs(30);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Unavailable,
    Stale,
    Missing,
    Busy,
    Loading,
    Blocked,
    Exhausted,
}
/// Correlation data only, never native permission. The actual App keeps and
/// rechecks WorkerBridge's genuine NativeActionScope separately at each effect.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Managed {
    pub source: ChatSource,
    pub activation: u64,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Facts {
    pub agent: Uuid,
    pub login: Uuid,
    pub grid: GridKey,
    pub logical_generation: u64,
    pub profile_generation: u64,
    pub managed: Option<Managed>,
}
impl Facts {
    fn valid(self) -> bool {
        !self.agent.is_nil()
            && !self.login.is_nil()
            && self.logical_generation != 0
            && self.profile_generation != 0
            && self.managed.is_none_or(|m| {
                m.activation != 0 && m.source.session.grid == self.grid && m.source.session.generation == self.logical_generation
            })
    }
    fn same_source(self, other: Self) -> bool {
        self.agent == other.agent
            && self.login == other.login
            && self.grid == other.grid
            && self.logical_generation == other.logical_generation
            && self.profile_generation == other.profile_generation
            && self.managed.map(|m| m.source) == other.managed.map(|m| m.source)
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Intent {
    Start,
    Stop(PlaybackId),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ticket {
    instance: u64,
    scope: u64,
    window: u64,
    action: u64,
    catalogue: u64,
    board_revision: u64,
    board: BoardId,
    tile: TileId,
    target: PlaybackItem,
    intent: Intent,
    captured_at: Instant,
}
impl Ticket {
    pub fn board(self) -> BoardId {
        self.board
    }
    pub fn item(self) -> Uuid {
        self.target.item()
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct BoardTicket {
    instance: u64,
    scope: u64,
    window: u64,
    action: u64,
    catalogue: u64,
    board_revision: u64,
    board: BoardId,
    captured_at: Instant,
}
pub(crate) struct Ready {
    ticket: Ticket,
    definition: Gesture,
}
impl Ready {
    pub fn definition(&self) -> &Gesture {
        &self.definition
    }
}
pub(crate) enum Click {
    Ready(Ready),
    NeedsCache(Ticket),
    AlreadyLoading,
    Stopped(Vec<Effect>),
}
struct Owned {
    board: BoardId,
    id: PlaybackId,
}
pub(crate) struct Playback {
    instance: u64,
    scope: u64,
    window: u64,
    action: u64,
    open: bool,
    facts: Option<Facts>,
    pending: BTreeMap<Uuid, Ticket>,
    owned: BTreeMap<Uuid, Owned>,
}
impl Default for Playback {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let instance = NEXT
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .unwrap_or(0);
        Self {
            instance,
            scope: 1,
            window: 1,
            action: 1,
            open: false,
            facts: None,
            pending: BTreeMap::new(),
            owned: BTreeMap::new(),
        }
    }
}
impl Playback {
    fn advance(counter: &mut u64) -> Result<(), Error> {
        *counter = counter.checked_add(1).ok_or(Error::Exhausted)?;
        Ok(())
    }
    fn fail_closed(&mut self) {
        self.instance = 0;
        self.facts = None;
        self.pending.clear();
        self.owned.clear();
    }
    pub fn bind(&mut self, facts: Option<Facts>) -> Result<(), Error> {
        let facts = facts.filter(|f| f.valid());
        if self.facts == facts {
            return Ok(());
        }
        if let Err(e) = Self::advance(&mut self.scope) {
            self.fail_closed();
            return Err(e);
        }
        if !self.facts.zip(facts).is_some_and(|(a, b)| a.same_source(b)) {
            self.owned.clear();
        }
        self.pending.clear();
        self.facts = facts;
        Ok(())
    }
    pub fn window(&mut self, open: bool) -> Result<(), Error> {
        if self.open == open {
            return Ok(());
        }
        if let Err(e) = Self::advance(&mut self.window) {
            self.fail_closed();
            return Err(e);
        }
        self.open = open;
        self.pending.clear();
        Ok(())
    }
    pub fn invalidate_actions(&mut self) -> Result<(), Error> {
        if let Err(e) = Self::advance(&mut self.action) {
            self.fail_closed();
            return Err(e);
        }
        self.pending.clear();
        Ok(())
    }
    fn current(&self, data: &Catalogue) -> Result<(), Error> {
        let f = self.facts.ok_or(Error::Unavailable)?;
        if self.instance == 0 || !self.open || data.account != f.agent || data.grid != f.grid {
            return Err(Error::Unavailable);
        }
        Ok(())
    }
    pub fn capture(
        &self,
        data: &Catalogue,
        board: BoardId,
        tile: TileId,
        target: PlaybackItem,
        manager: &Manager,
        now: Instant,
    ) -> Result<Ticket, Error> {
        self.current(data)?;
        let b = data.board(board).ok_or(Error::Missing)?;
        let t = data.tile(board, tile).ok_or(Error::Missing)?;
        let kind = match t.item.kind {
            Kind::Gesture => PlaybackKind::Gesture,
            Kind::Sound => PlaybackKind::Sound,
        };
        if t.item.reference != target.reference() || kind != target.kind() {
            return Err(Error::Stale);
        }
        let intent = if target.kind() == PlaybackKind::Gesture {
            manager.playback_id(target.item()).map_or(Intent::Start, Intent::Stop)
        } else {
            Intent::Start
        };
        Ok(Ticket {
            instance: self.instance,
            scope: self.scope,
            window: self.window,
            action: self.action,
            catalogue: data.revision,
            board_revision: b.revision,
            board,
            tile,
            target,
            intent,
            captured_at: now,
        })
    }
    fn validate(&self, t: Ticket, data: &Catalogue, target: PlaybackItem, now: Instant, pending: bool) -> Result<(), Error> {
        self.current(data)?;
        if t.instance != self.instance
            || t.scope != self.scope
            || t.window != self.window
            || t.action != self.action
            || target != t.target
            || (!pending && data.revision != t.catalogue)
            || now.checked_duration_since(t.captured_at).is_none_or(|age| age >= DEADLINE)
        {
            return Err(Error::Stale);
        }
        let b = data.board(t.board).ok_or(Error::Stale)?;
        let tile = data.tile(t.board, t.tile).ok_or(Error::Stale)?;
        if b.revision != t.board_revision
            || tile.item.reference != target.reference()
            || (tile.item.kind == Kind::Gesture) != (target.kind() == PlaybackKind::Gesture)
        {
            return Err(Error::Stale);
        }
        Ok(())
    }
    pub fn validate_sound(&self, t: Ticket, data: &Catalogue, target: PlaybackItem, now: Instant) -> Result<(), Error> {
        self.validate(t, data, target, now, false)?;
        if target.kind() != PlaybackKind::Sound || t.intent != Intent::Start {
            return Err(Error::Stale);
        }
        Ok(())
    }
    fn ready(&self, t: Ticket, manager: &Manager) -> Result<Ready, Error> {
        if !manager.enabled {
            return Err(Error::Blocked);
        }
        if manager.playback_id(t.item()).is_some() {
            return Err(Error::Stale);
        }
        let entry = manager
            .entries
            .get(&t.item())
            .filter(|e| e.asset == t.target.asset())
            .ok_or(Error::Loading)?;
        Ok(Ready {
            ticket: t,
            definition: entry.definition.clone(),
        })
    }
    pub fn click(
        &mut self,
        t: Ticket,
        data: &Catalogue,
        target: PlaybackItem,
        manager: &mut Manager,
        now: Instant,
    ) -> Result<Click, Error> {
        self.validate(t, data, target, now, false)?;
        if target.kind() != PlaybackKind::Gesture {
            return Err(Error::Stale);
        }
        if let Intent::Stop(id) = t.intent {
            if manager.playback_id(t.item()) != Some(id) {
                return Err(Error::Stale);
            }
            self.pending.remove(&t.item());
            self.owned.remove(&t.item());
            return Ok(Click::Stopped(manager.stop_exact(t.item(), id)));
        }
        if self.pending.contains_key(&t.item()) {
            return Ok(Click::AlreadyLoading);
        }
        match self.ready(t, manager) {
            Ok(ready) => Ok(Click::Ready(ready)),
            Err(Error::Loading) => {
                if self.pending.len() >= MAX_PENDING {
                    return Err(Error::Busy);
                }
                self.pending.insert(t.item(), t);
                Ok(Click::NeedsCache(t))
            }
            Err(e) => Err(e),
        }
    }
    pub fn pending(&self, item: Uuid) -> Option<Ticket> {
        self.pending.get(&item).copied()
    }
    pub fn cancel_pending(&mut self, t: Ticket) -> bool {
        if self.pending.get(&t.item()) != Some(&t) {
            return false;
        }
        self.pending.remove(&t.item());
        true
    }
    pub fn cancel_board(&mut self, board: BoardId) {
        self.pending.retain(|_, t| t.board != board);
    }
    pub fn expire(&mut self, now: Instant) {
        self.pending
            .retain(|_, t| now.checked_duration_since(t.captured_at).is_some_and(|age| age < DEADLINE));
    }
    pub fn prepare_pending(
        &mut self,
        t: Ticket,
        data: &Catalogue,
        target: PlaybackItem,
        manager: &Manager,
        now: Instant,
    ) -> Result<Ready, Error> {
        if self.pending.get(&t.item()) != Some(&t) {
            return Err(Error::Stale);
        }
        if let Err(e) = self.validate(t, data, target, now, true) {
            self.cancel_pending(t);
            return Err(e);
        }
        let result = self.ready(t, manager);
        if !matches!(result, Err(Error::Loading)) {
            self.cancel_pending(t);
        }
        result
    }
    /// App preloads Ready.definition through its existing native asset path,
    /// rechecks genuine NativeActionScope, then commits. Ownership is recorded
    /// BEFORE App tick/effect callbacks, never by copying someone else's run ID.
    pub fn start(
        &mut self,
        ready: Ready,
        data: &Catalogue,
        target: PlaybackItem,
        manager: &mut Manager,
        now: Instant,
    ) -> Result<(), Error> {
        self.validate(ready.ticket, data, target, now, true)?;
        if !manager.enabled {
            return Err(Error::Blocked);
        }
        if manager.playback_id(target.item()).is_some() {
            return Err(Error::Stale);
        }
        self.owned.retain(|item, owned| manager.playback_id(*item) == Some(owned.id));
        if self.owned.len() >= MAX_BOARDS * MAX_TILES {
            return Err(Error::Busy);
        }
        manager.play(target.item(), ready.definition, None);
        let id = manager.playback_id(target.item()).ok_or(Error::Blocked)?;
        self.owned.insert(
            target.item(),
            Owned {
                board: ready.ticket.board,
                id,
            },
        );
        Ok(())
    }
    pub fn capture_board(&self, data: &Catalogue, board: BoardId, now: Instant) -> Result<BoardTicket, Error> {
        self.current(data)?;
        let revision = data.board(board).ok_or(Error::Missing)?.revision;
        Ok(BoardTicket {
            instance: self.instance,
            scope: self.scope,
            window: self.window,
            action: self.action,
            catalogue: data.revision,
            board_revision: revision,
            board,
            captured_at: now,
        })
    }
    /// Exact source/window/action snapshot, not a current dropdown or tile row.
    /// A board whose last tile was removed may still stop its own running work.
    pub fn stop_board(&mut self, from: BoardTicket, data: &Catalogue, manager: &mut Manager, now: Instant) -> Result<Vec<Effect>, Error> {
        self.current(data)?;
        if from.instance != self.instance
            || from.scope != self.scope
            || from.window != self.window
            || from.action != self.action
            || now.checked_duration_since(from.captured_at).is_none_or(|age| age >= DEADLINE)
            || from.catalogue != data.revision
            || data.board(from.board).is_none_or(|b| b.revision != from.board_revision)
        {
            return Err(Error::Stale);
        }
        self.cancel_board(from.board);
        let items: Vec<_> = self
            .owned
            .iter()
            .filter(|(_, owned)| owned.board == from.board)
            .map(|(item, owned)| (*item, owned.id))
            .collect();
        let mut effects = Vec::new();
        for (item, id) in items {
            self.owned.remove(&item);
            effects.extend(manager.stop_exact(item, id));
        }
        Ok(effects)
    }
}
#[cfg(test)]
mod tests;
