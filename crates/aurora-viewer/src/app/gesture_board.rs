//! FSGestureBoardController / FSFloaterGestureBoard native consumer (Legacy
//! Viewer abbe3cb0ca66ee2484f277219f6a27706fab7fc6). Only inventory references
//! persist. UI observations never substitute for a genuine managed action scope.
use super::{App, Screen};
use crate::gesture_board::{
    model::{self, BoardId, Catalogue, Change, Kind, Reference, Tile, TileId},
    playback::{self, Facts, Managed, Playback, Ready, Ticket},
    store,
};
use crate::session_host::{
    profile::{NativeDirectory, NativeStoreCancellation, NativeStoreKind},
    worker::action::NativeActionScope,
};
use crate::ui::gesture_board::{self as view, Action, Command};
use crate::world::inventory::actions::{self, PlaybackItem, PlaybackKind};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use uuid::Uuid;

/// Borrowed from the actual Gfx during draw_ui's temporary take. This is only
/// window/device availability; genuine managed NativeActionScope stays separate.
#[derive(Clone, Copy)]
pub(super) struct Presentation<'a> {
    renderer: &'a aurora_render::Renderer,
    window: &'a winit::window::Window,
}
impl<'a> Presentation<'a> {
    pub(super) fn new(renderer: &'a aurora_render::Renderer, window: &'a winit::window::Window) -> Self {
        Self { renderer, window }
    }
}
fn available<T>(owned: Option<&T>, borrowed: Option<&T>, current: impl FnOnce(&T) -> bool) -> bool {
    owned.or(borrowed).is_some_and(current)
}

/// Captured by actual App observation only. Retaining this value does not renew
/// the original native lease or the pointer/key press's thirty-second lifetime.
#[derive(Clone, Copy)]
pub(crate) struct Proof {
    instance: u64,
    epoch: u64,
    window: u64,
    actions: u64,
    local: u64,
    facts: Facts,
    native: Option<NativeActionScope>,
    at: Instant,
}
impl Proof {
    pub(crate) fn same_owner(self, other: Self) -> bool {
        self.instance == other.instance
            && self.epoch == other.epoch
            && self.window == other.window
            && self.actions == other.actions
            && self.local == other.local
            && self.facts == other.facts
    }
}
fn advance(value: u64) -> u64 {
    if value == 0 { 0 } else { value.checked_add(1).unwrap_or(0) }
}
struct Job {
    operation: u64,
    cancellation: NativeStoreCancellation,
    initial: Option<Proof>,
}
pub(crate) struct State {
    instance: u64,
    epoch: u64,
    window: u64,
    actions: u64,
    local: u64,
    open: bool,
    facts: Option<Facts>,
    runtime: Playback,
    ui: view::State,
    lane: Option<store::Lane>,
    snapshot: Option<store::Snapshot>,
    job: Option<Job>,
    next_operation: u64,
    loaded: bool,
    selected: Option<BoardId>,
    pending: BTreeMap<Uuid, (Ticket, Proof, PlaybackItem)>,
    status: &'static str,
}
impl Default for State {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            instance: NEXT
                .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
                .unwrap_or(0),
            epoch: 0,
            window: 1,
            actions: 0,
            local: 1,
            open: false,
            facts: None,
            runtime: Playback::default(),
            ui: Default::default(),
            lane: None,
            snapshot: None,
            job: None,
            next_operation: 1,
            loaded: false,
            selected: None,
            pending: BTreeMap::new(),
            status: "board-loading",
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel_job();
    }
}
impl State {
    fn cancel_job(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancellation.cancel();
        }
    }
    fn cancel_play_actions(&mut self) {
        self.local = advance(self.local);
        let _ = self.runtime.invalidate_actions();
        self.pending.clear();
        self.ui.cancel();
    }
    fn cancel_actions(&mut self) {
        self.cancel_play_actions();
        self.cancel_job();
    }

    fn proof(&self, native: Option<NativeActionScope>, now: Instant) -> Option<Proof> {
        Some(Proof {
            instance: self.instance,
            epoch: self.epoch,
            window: self.window,
            actions: self.actions,
            local: self.local,
            facts: self.facts?,
            native,
            at: now,
        })
    }
    fn reconcile(&mut self, app: &App, presentation: Option<Presentation<'_>>) -> Option<Proof> {
        let observation = app.board_observation(self.facts);
        let facts = observation.map(|o| o.0);
        let identity_changed = !self.facts.zip(facts).is_some_and(|(a, b)| a.same_source(b));
        if self.facts != facts || self.epoch != app.board_epoch {
            self.cancel_actions();
            if identity_changed || self.epoch != app.board_epoch {
                self.snapshot = None;
                self.loaded = false;
                self.selected = None;
                self.ui = Default::default();
            }
            let _ = self.runtime.bind(facts);
            self.facts = facts;
            self.epoch = app.board_epoch;
        }
        if self.actions != app.board_actions {
            self.cancel_actions();
            self.actions = app.board_actions;
        }
        if self.open != app.panels.gesture_board {
            self.open = app.panels.gesture_board;
            self.window = advance(self.window);
            self.cancel_actions();
            let _ = self.runtime.window(self.open);
            if !self.open {
                self.snapshot = None;
                self.loaded = false;
            }
        }
        self.runtime.expire(Instant::now());
        self.pending
            .retain(|item, (ticket, _, _)| self.runtime.pending(*item) == Some(*ticket));
        if self.instance == 0
            || self.local == 0
            || self.window == 0
            || self.epoch == 0
            || self.actions == 0
            || !self.open
            || !app.board_effect_ready(presentation)
        {
            return None;
        }
        let (_, native) = observation?;
        if app.board_managed && native.is_none() {
            return None;
        }
        self.proof(native, Instant::now())
    }
    fn current(&self, app: &App, proof: Proof, presentation: Option<Presentation<'_>>) -> bool {
        let Some((facts, _)) = app.board_observation(self.facts) else {
            return false;
        };
        self.instance != 0
            && self.local != 0
            && self.open
            && app.panels.gesture_board
            && self.epoch == app.board_epoch
            && self.actions == app.board_actions
            && self.proof(proof.native, proof.at).is_some_and(|p| p.same_owner(proof))
            && facts == proof.facts
            && app.board_effect_ready(presentation)
            && Instant::now()
                .checked_duration_since(proof.at)
                .is_some_and(|age| age < Duration::from_secs(30))
            && if app.board_managed {
                proof.native.is_some_and(|scope| {
                    app.host_session.as_ref().is_some_and(|b| {
                        b.native_action_current(scope)
                            && scope.generation() == proof.facts.logical_generation
                            && Instant::now() < scope.deadline()
                    })
                })
            } else {
                proof.native.is_none() && app.host_session.is_none()
            }
    }
    fn owner(&self) -> Option<store::Owner> {
        Some(store::Owner {
            instance: self.instance,
            epoch: self.epoch,
            facts: self.facts?,
        })
    }
    fn queue(&mut self, app: &App, proof: Proof, change: Option<Change>, initial: bool, presentation: Option<Presentation<'_>>) -> bool {
        if !self.current(app, proof, presentation) || self.job.is_some() {
            return false;
        }
        let Some(next) = self.next_operation.checked_add(1) else {
            self.status = "board-unavailable";
            return false;
        };
        let Some(owner) = self.owner() else {
            return false;
        };
        let cancellation = NativeStoreCancellation::new();
        if app.demo {
            // Offline fixtures never read/write a real profile or play on load.
            let result = match (&self.snapshot, change) {
                (Some(old), Some(edit)) => old.data.apply(edit),
                (_, None) => Catalogue::empty(owner.facts.agent, owner.facts.grid),
                _ => return false,
            };
            if let Ok(data) = result {
                self.snapshot = Some(store::Snapshot::memory(data));
                self.loaded = true;
                self.status = "board-ready";
                self.select_existing();
                return true;
            }
            self.status = "board-save-failed";
            return false;
        }
        let Ok(directory) = NativeDirectory::select(NativeStoreKind::Boards, owner.facts.agent, &owner.facts.grid.to_string()) else {
            self.status = "board-unavailable";
            self.loaded = true;
            return false;
        };
        if self.lane.is_none() {
            self.lane = store::Lane::new().ok();
        }
        let edit = change.map(|change| (self.snapshot.as_ref().and_then(store::Snapshot::original), change));
        let request = store::Request {
            owner,
            operation: self.next_operation,
            directory,
            cancellation: cancellation.clone(),
            edit,
        };
        if !self.lane.as_ref().is_some_and(|lane| lane.try_submit(request)) {
            self.status = "board-busy";
            return false;
        }
        self.job = Some(Job {
            operation: self.next_operation,
            cancellation,
            initial: initial.then_some(proof),
        });
        self.next_operation = next;
        self.status = "board-loading";
        true
    }
    fn select_existing(&mut self) {
        if self
            .selected
            .is_none_or(|id| self.snapshot.as_ref().is_none_or(|s| s.data.board(id).is_none()))
        {
            self.selected = self.snapshot.as_ref().and_then(|s| s.data.boards.first().map(|b| b.id));
        }
    }
    fn take_job(&mut self, owner: store::Owner, operation: u64) -> Option<Job> {
        if Some(owner) != self.owner()
            || !self
                .job
                .as_ref()
                .is_some_and(|job| job.operation == operation && !job.cancellation.is_cancelled())
        {
            return None;
        }
        self.job.take()
    }
    fn poll(&mut self, app: &App, presentation: Option<Presentation<'_>>) {
        let result = match self.lane.as_ref().map(store::Lane::poll) {
            Some(Ok(Some(reply))) => reply,
            Some(Err(())) => {
                self.cancel_job();
                self.loaded = true;
                self.status = "board-unavailable";
                return;
            }
            _ => return,
        };
        let Some(job) = self.take_job(result.owner, result.operation) else {
            return;
        };
        self.loaded = true;
        match result.result {
            Ok(snapshot) => {
                self.snapshot = Some(snapshot);
                self.status = "board-ready";
                self.select_existing();
                // Only a still-current explicit first Open may create the first
                // empty board. A later heartbeat cannot renew that intent.
                if let Some(proof) = job.initial.filter(|p| self.current(app, *p, presentation))
                    && self.snapshot.as_ref().is_some_and(|s| s.original().is_none())
                {
                    self.queue(
                        app,
                        proof,
                        Some(Change::Create {
                            id: BoardId::new(),
                            name: aurora_i18n::tr!("board-default-name").into(),
                        }),
                        false,
                        presentation,
                    );
                }
            }
            Err(store::Error::Cancelled) => self.status = "board-cancelled",
            Err(store::Error::Conflict) => self.status = "board-conflict",
            Err(store::Error::Model(_)) => {
                if job.initial.is_some() {
                    self.snapshot = None;
                }
                self.status = "board-invalid";
            }
            Err(_) => self.status = "board-save-failed",
        }
    }
    fn ready(&mut self, app: &mut App, proof: Proof, target: PlaybackItem, ready: Ready, presentation: Option<Presentation<'_>>) {
        if !self.current(app, proof, presentation) || !current_target(&app.world.inventory, target) || app.world.rlva.has("sendgesture") {
            return;
        }
        app.gesture_prefetch(ready.definition());
        if !self.current(app, proof, presentation) || !current_target(&app.world.inventory, target) {
            return;
        }
        let Some(data) = self.snapshot.as_ref().map(|s| s.data.clone()) else {
            return;
        };
        if self
            .runtime
            .start(ready, &data, target, &mut app.gesture_state.manager, Instant::now())
            .is_ok()
        {
            self.status = "board-ready";
            // Ownership has already been installed before native effect callbacks.
            app.tick_gestures();
        }
    }
    fn complete(&mut self, app: &mut App, presentation: Option<Presentation<'_>>) {
        let work: Vec<_> = self.pending.values().copied().collect();
        for (ticket, proof, target) in work {
            if !self.current(app, proof, presentation)
                || !current_target(&app.world.inventory, target)
                || app.world.rlva.has("sendgesture")
                || app.gesture_state.board_cache_failed(target)
            {
                self.status = "board-unavailable";
                self.runtime.cancel_pending(ticket);
                self.pending.remove(&ticket.item());
                continue;
            }
            let Some(data) = self.snapshot.as_ref().map(|s| s.data.clone()) else {
                break;
            };
            match self
                .runtime
                .prepare_pending(ticket, &data, target, &app.gesture_state.manager, Instant::now())
            {
                Ok(ready) => {
                    self.pending.remove(&ticket.item());
                    self.ready(app, proof, target, ready, presentation);
                }
                Err(playback::Error::Loading) => {}
                Err(_) => {
                    self.pending.remove(&ticket.item());
                    self.status = "board-unavailable";
                }
            }
        }
    }
    fn dispatch(&mut self, app: &mut App, action: Action, presentation: Option<Presentation<'_>>) {
        if !self.current(app, action.proof, presentation) {
            self.status = "board-stale";
            return;
        }
        let Some(data) = self.snapshot.as_ref().map(|s| s.data.clone()) else {
            return;
        };
        if action.revision != data.revision {
            self.status = "board-stale";
            return;
        }
        match action.command {
            Command::BeginNew | Command::BeginRename(_) | Command::BeginDelete(_) => {}
            Command::Select(id) => {
                if data.board(id).is_some() && self.selected != Some(id) {
                    self.cancel_play_actions();
                    self.selected = Some(id);
                }
            }
            Command::Inventory => app.panels.inventory = true,
            Command::Play(ticket, target) => {
                if !current_target(&app.world.inventory, target) {
                    self.status = "board-stale";
                    return;
                }
                if target.kind() == PlaybackKind::Sound {
                    if self.runtime.validate_sound(ticket, &data, target, Instant::now()).is_ok()
                        && self.current(app, action.proof, presentation)
                    {
                        app.apply_inventory_action(crate::ui::inventory::InvAction::TriggerSound(target.asset()));
                    }
                    return;
                }
                if app.world.rlva.has("sendgesture") {
                    self.status = "board-blocked";
                    return;
                }
                match self
                    .runtime
                    .click(ticket, &data, target, &mut app.gesture_state.manager, Instant::now())
                {
                    Ok(playback::Click::Ready(ready)) => self.ready(app, action.proof, target, ready, presentation),
                    Ok(playback::Click::Stopped(effects)) => app.gesture_effects(effects),
                    Ok(playback::Click::NeedsCache(ticket)) => {
                        if app.board_cache_request(target) {
                            self.pending.insert(ticket.item(), (ticket, action.proof, target));
                            self.status = "board-loading";
                        } else {
                            self.runtime.cancel_pending(ticket);
                            self.status = "board-busy";
                        }
                    }
                    Ok(playback::Click::AlreadyLoading) => {}
                    Err(_) => self.status = "board-unavailable",
                }
            }
            Command::Stop(board, ticket) => {
                if let Ok(effects) = self.stop_board(board, ticket, &data, &mut app.gesture_state.manager, Instant::now()) {
                    app.gesture_effects(effects);
                }
            }
            Command::Add(board, target) => {
                if self.job.is_some() || !current_target(&app.world.inventory, target) {
                    self.status = "board-stale";
                    return;
                }
                let Some(b) = data.board(board) else {
                    return;
                };
                // Reference aliases must not create duplicate resolved tiles.
                if duplicate(&app.world.inventory, b, target) {
                    self.status = "board-duplicate";
                    return;
                }
                let tile = Tile {
                    id: TileId::new(),
                    item: Reference {
                        reference: target.reference(),
                        kind: if target.kind() == PlaybackKind::Gesture {
                            Kind::Gesture
                        } else {
                            Kind::Sound
                        },
                    },
                };
                self.edit(app, action.proof, &data, Some(board), Change::Add { board, tile }, presentation);
            }
            Command::Edit(change) => {
                let affected = match &change {
                    Change::Create { .. } => None,
                    Change::Rename { board, .. } | Change::Delete { board } | Change::Remove { board, .. } => Some(*board),
                    Change::Add { .. } => return,
                };
                self.edit(app, action.proof, &data, affected, change, presentation);
            }
        }
    }
    fn stop_board(
        &mut self,
        board: BoardId,
        ticket: playback::BoardTicket,
        data: &Catalogue,
        manager: &mut crate::gestures::Manager,
        now: Instant,
    ) -> Result<Vec<crate::gestures::Effect>, playback::Error> {
        if board != ticket.board() {
            return Err(playback::Error::Stale);
        }
        let effects = self.runtime.stop_board(ticket, data, manager, now)?;
        self.pending.retain(|_, (t, _, _)| t.board() != board);
        Ok(effects)
    }
    fn edit(
        &mut self,
        app: &App,
        proof: Proof,
        data: &Catalogue,
        affected: Option<BoardId>,
        change: Change,
        presentation: Option<Presentation<'_>>,
    ) {
        if self.job.is_some() {
            return;
        }
        // PRE-edit catalogue: deletion must not prune a Ready latch before it is
        // revoked. Failed/queued saves never restore a canceled play intent.
        if let Some(board) = affected {
            if self.runtime.cancel_board(board, data).is_err() {
                return;
            }
            self.pending.retain(|_, (ticket, _, _)| ticket.board() != board);
        }
        self.queue(app, proof, Some(change), false, presentation);
    }
}
pub(crate) fn resolve(inv: &crate::world::inventory::Inventory, item: Reference) -> Option<PlaybackItem> {
    actions::resolve_playback_item(
        inv,
        item.reference,
        if item.kind == Kind::Gesture {
            PlaybackKind::Gesture
        } else {
            PlaybackKind::Sound
        },
    )
    .ok()
}
impl App {
    pub(super) fn revoke_board_actions(&mut self) {
        self.board_actions = advance(self.board_actions);
        if let Some(state) = &mut self.board_state {
            state.cancel_actions();
        }
    }
    pub(super) fn reset_boards(&mut self) {
        self.revoke_board_actions();
        self.board_epoch = advance(self.board_epoch);
        self.board_state = None;
        self.panels.gesture_board = false;
    }
    fn board_effect_ready(&self, presentation: Option<Presentation<'_>>) -> bool {
        matches!(self.screen, Screen::World)
            && available(self.gfx.as_ref().map(|g| &g.renderer), presentation.map(|p| p.renderer), |r| {
                !r.failed() && !r.device_lost()
            })
            && presentation.is_none_or(|p| p.window.inner_size().width != 0 && p.window.inner_size().height != 0)
            && !self.exit_prompt.open
            && !self.login_form.busy
            && self.options_ui.capture.is_none()
            && self.world.main_region.and_then(|h| self.world.region_offset(h)).is_some()
    }
    fn board_observation(&self, previous: Option<Facts>) -> Option<(Facts, Option<NativeActionScope>)> {
        if !matches!(self.screen, Screen::World)
            || self.board_epoch == 0
            || self.world.agent_id.is_nil()
            || self.world.inventory.owner != self.world.agent_id
        {
            return None;
        }
        let login = self.world.login.as_ref()?.session_id;
        if login.is_nil() {
            return None;
        }
        let grid = crate::session_host::transport::grid_key(&self.settings.login_uri()).ok()?;
        let (logical_generation, managed, native) = if self.board_managed {
            let bridge = self.host_session.as_ref()?;
            let source = super::host_chat::source(self, bridge)?;
            let native = bridge.native_action_scope().filter(|s| s.source() == source);
            let activation = native
                .map(|s| s.activation())
                .or_else(|| previous.and_then(|f| f.managed.filter(|m| m.source == source).map(|m| m.activation)))?;
            (source.session.generation, Some(Managed { source, activation }), native)
        } else {
            if self.host_session.is_some() {
                return None;
            }
            (self.board_epoch, None, None)
        };
        Some((
            Facts {
                agent: self.world.agent_id,
                login,
                grid,
                logical_generation,
                profile_generation: self.board_epoch,
                managed,
            },
            native,
        ))
    }
    fn with_boards(&mut self, presentation: Option<Presentation<'_>>, body: impl FnOnce(&mut State, &mut App)) {
        let mut state = self.board_state.take().unwrap_or_default();
        body(&mut state, self);
        // A native callback may have reset the source while state was borrowed.
        // Never reinstall that old source's private state over a replacement.
        if self.board_state.is_none() && state.epoch == self.board_epoch {
            state.reconcile(self, presentation);
            self.board_state = Some(state);
        }
    }
    pub(super) fn tick_boards(&mut self) {
        self.tick_boards_presented(None);
    }
    fn tick_boards_presented(&mut self, presentation: Option<Presentation<'_>>) {
        if self.board_state.is_none() && !self.panels.gesture_board {
            return;
        }
        self.with_boards(presentation, |state, app| {
            let proof = state.reconcile(app, presentation);
            state.poll(app, presentation);
            if !state.loaded
                && state.job.is_none()
                && let Some(proof) = proof
            {
                if state.queue(app, proof, None, true, presentation) && app.demo {
                    state.queue(
                        app,
                        proof,
                        Some(Change::Create {
                            id: BoardId::new(),
                            name: aurora_i18n::tr!("board-default-name").into(),
                        }),
                        false,
                        presentation,
                    );
                }
            }
            state.complete(app, presentation);
        });
    }
    pub(super) fn show_boards(&mut self, ctx: &egui::Context, p: &crate::theme::Palette, frame: Presentation<'_>) {
        if !self.panels.gesture_board {
            return;
        }
        let presentation = Some(frame);
        self.tick_boards_presented(presentation);
        self.with_boards(presentation, |state, app| {
            let proof = state.reconcile(app, presentation);
            let data = state.snapshot.as_ref().map(|s| s.data.clone());
            let mut open = app.panels.gesture_board;
            let selection = if app.inventory_ui.selection.len() == 1 {
                app.inventory_ui.selection.iter().next().copied()
            } else {
                None
            };
            let add = selection.and_then(|id| {
                [PlaybackKind::Gesture, PlaybackKind::Sound]
                    .into_iter()
                    .find_map(|kind| actions::resolve_playback_item(&app.world.inventory, id, kind).ok())
            });
            let action = view::show(
                ctx,
                p,
                &mut state.ui,
                &mut open,
                view::Context {
                    data: data.as_deref(),
                    selected: state.selected,
                    runtime: &state.runtime,
                    inventory: &app.world.inventory,
                    manager: &app.gesture_state.manager,
                    proof,
                    add,
                    writable: state.job.is_none(),
                    status: state.status,
                },
            );
            app.panels.gesture_board = open;
            if !open {
                state.reconcile(app, presentation);
                return;
            }
            if let Some(action) = action {
                state.dispatch(app, action, presentation);
            }
        });
    }
}

fn current_target(inventory: &crate::world::inventory::Inventory, target: PlaybackItem) -> bool {
    actions::resolve_playback_item(inventory, target.reference(), target.kind()) == Ok(target)
}
fn duplicate(inventory: &crate::world::inventory::Inventory, board: &model::Board, target: PlaybackItem) -> bool {
    board
        .tiles
        .iter()
        .any(|tile| resolve(inventory, tile.item).is_some_and(|old| old.item() == target.item()))
}
#[cfg(test)]
mod tests;
