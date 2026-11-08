//! FSFloaterGestureBoard native first slice. Presses capture exact model/native
//! witnesses; a release never substitutes a new inventory row, board or source.
use super::widgets::{Floater, Modal};
use crate::{
    app::gesture_board::{Proof, resolve},
    gesture_board::{
        model::{self, BoardId, Catalogue, Change},
        playback::{BoardTicket, Playback, Ticket},
    },
    gestures::Manager,
    theme::Palette,
    world::inventory::{Inventory, actions::PlaybackItem},
};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

#[derive(Clone)]
pub(crate) enum Command {
    Select(BoardId),
    Inventory,
    Play(Ticket, PlaybackItem),
    Stop(BoardId, BoardTicket),
    Add(BoardId, PlaybackItem),
    Edit(Change),
    BeginNew,
    BeginRename(BoardId),
    BeginDelete(BoardId),
}
#[derive(Clone)]
pub(crate) struct Action {
    pub proof: Proof,
    pub revision: u64,
    pub command: Command,
    activation: Option<ActivationControlTicket>,
}
impl Action {
    fn same_action(&self, other: &Self) -> bool {
        self.proof.same_owner(other.proof)
            && self.revision == other.revision
            && match (&self.command, &other.command) {
                (Command::Select(a), Command::Select(b)) => a == b,
                (Command::Inventory, Command::Inventory) | (Command::BeginNew, Command::BeginNew) => true,
                (Command::Play(a, target_a), Command::Play(b, target_b)) => target_a == target_b && a.same_action(*b),
                (Command::Stop(board_a, a), Command::Stop(board_b, b)) => board_a == board_b && a.same_action(*b),
                (Command::Add(a, target_a), Command::Add(b, target_b)) => a == b && target_a == target_b,
                (Command::Edit(a), Command::Edit(b)) => a == b,
                (Command::BeginRename(a), Command::BeginRename(b)) | (Command::BeginDelete(a), Command::BeginDelete(b)) => a == b,
                _ => false,
            }
    }
}
/// Local committed-control correlation only. Construction and consumption stay
/// in this module; it cannot replace App's genuine native/source proof.
#[derive(Clone, Copy, PartialEq)]
struct ActivationControlTicket {
    instance: u64,
    release: u64,
    control: u64,
    frame: u64,
    id: egui::Id,
    rect: egui::Rect,
    clip: egui::Rect,
    layer: egui::LayerId,
    transform: Option<egui::emath::TSTransform>,
    key: Option<egui::Key>,
    position: Option<egui::Pos2>,
}
struct Held {
    id: egui::Id,
    control: u64,
    rect: egui::Rect,
    clip: egui::Rect,
    layer: egui::LayerId,
    transform: Option<egui::emath::TSTransform>,
    action: Action,
    key: Option<egui::Key>,
    press_frame: u64,
    press_index: usize,
}
impl Held {
    fn release(&self, input: &egui::InputState, frame: u64) -> Option<Option<egui::Pos2>> {
        input.events.iter().enumerate().find_map(|(index, event)| {
            if self.press_frame == frame && index <= self.press_index {
                return None;
            }
            match (self.key, event) {
                (
                    None,
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        ..
                    },
                ) => Some(Some(*pos)),
                (Some(expected), egui::Event::Key { key, pressed: false, .. }) if expected == *key => Some(None),
                _ => None,
            }
        })
    }
}
struct Dialog {
    origin: Proof,
    revision: u64,
    board: Option<BoardId>,
    delete: bool,
    name: String,
    created: BoardId,
}
struct Painted {
    control: u64,
    action: Action,
    rect: egui::Rect,
    layer: egui::LayerId,
    transform: Option<egui::emath::TSTransform>,
    clip: egui::Rect,
    focused: bool,
}
const MAX_CONTROLS: usize = model::MAX_TILES * 2 + model::MAX_BOARDS + 16;
pub(crate) struct State {
    instance: u64,
    next_control: u64,
    next_release: u64,
    released: Option<ActivationControlTicket>,
    dialog_completed: Option<u64>,
    owner: Option<Proof>,
    held: Option<Held>,
    dialog: Option<Dialog>,
    last_press: Option<u64>,
    frame: Option<u64>,
    finished_pass: Option<u64>,
    painted: HashMap<egui::Id, Painted>,
    building: HashMap<egui::Id, Painted>,
    composing: bool,
    input_blocked: bool,
    suspended: bool,
    focus: Option<egui::Id>,
    retire_focus: Option<egui::Id>,
}
impl Default for State {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            instance: NEXT
                .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
                .unwrap_or(0),
            next_control: 1,
            next_release: 1,
            released: None,
            dialog_completed: None,
            owner: None,
            held: None,
            dialog: None,
            last_press: None,
            frame: None,
            finished_pass: None,
            painted: HashMap::new(),
            building: HashMap::new(),
            composing: false,
            input_blocked: false,
            suspended: false,
            focus: None,
            retire_focus: None,
        }
    }
}
fn next(counter: &mut u64) -> Option<u64> {
    let current = *counter;
    *counter = current.checked_add(1).unwrap_or(0);
    (current != 0 && *counter != 0).then_some(current)
}
impl State {
    pub(crate) fn cancel(&mut self) {
        self.retire_focus = self
            .focus
            .filter(|id| self.painted.contains_key(id) || self.building.contains_key(id))
            .or(self.retire_focus);
        self.owner = None;
        self.composing = false;
        self.input_blocked = false;
        self.focus = None;
        self.held = None;
        self.dialog = None;
        self.dialog_completed = None;
        self.released = None;
        self.finished_pass = None;
        self.painted.clear();
        self.building.clear();
    }
    fn bind(&mut self, proof: Option<Proof>) {
        if !self.owner.zip(proof).is_some_and(|(a, b)| a.same_owner(b)) {
            self.cancel();
        }
        self.owner = proof;
    }
    fn begin(&mut self, ctx: &egui::Context) {
        let frame = ctx.cumulative_frame_nr();
        if self.frame != Some(frame) {
            if self.dialog_completed.take().is_some() {
                self.dialog = None;
            }
            self.released = None;
            self.painted = std::mem::take(&mut self.building);
            self.frame = Some(frame);
        }
        // Each egui pass replaces this set. The next frame sees the last pass,
        // while a discarded pass cannot overwrite the prior painted witnesses.
        self.building.clear();
        let was_composing = self.composing;
        let blocked = ctx.input(|input| {
            let mut event = false;
            for value in &input.events {
                match value {
                    egui::Event::Ime(ime) => {
                        event = true;
                        match ime {
                            egui::ImeEvent::Preedit { text, .. } => self.composing = !text.is_empty(),
                            egui::ImeEvent::Commit(_) => self.composing = false,
                            #[allow(deprecated)]
                            egui::ImeEvent::Disabled => self.composing = false,
                            _ => {}
                        }
                    }
                    egui::Event::WindowFocused(_) => event = true,
                    _ => {}
                }
            }
            !input.focused || event || was_composing || self.composing
        });
        let focus = ctx.memory(|m| m.focused());
        if let Some(id) = self.retire_focus.take().filter(|id| Some(*id) == focus) {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
        let focus = ctx.memory(|m| m.focused());
        if blocked && let Some(id) = focus.filter(|id| self.painted.contains_key(id)) {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
        let focus = ctx.memory(|m| m.focused());
        if self.focus != focus && self.held.as_ref().is_some_and(|held| held.key.is_some()) {
            self.held = None;
        }
        self.focus = focus;
        self.input_blocked = blocked;
        if blocked {
            self.held = None;
            self.released = None;
        }
    }
    /// Must be called after Context::run_ui returns. A later widget can discard
    /// the pass, change geometry or take modal/focus ownership after show().
    pub(crate) fn take_activation(&mut self, ctx: &egui::Context, action: &Action) -> bool {
        let Some(ticket) = action.activation else { return false };
        if self.released != Some(ticket) {
            return false;
        }
        self.released = None;
        let valid = self.instance != 0
            && ticket.instance == self.instance
            && self.frame == Some(ticket.frame)
            && ticket.frame.checked_add(1) == Some(ctx.cumulative_frame_nr())
            && self.finished_pass.and_then(|pass| pass.checked_add(1)) == Some(ctx.cumulative_pass_nr())
            && !self.input_blocked
            && !self.composing
            && ctx.input(|input| input.focused)
            && !egui::Popup::is_any_open(ctx)
            && ctx.memory(|memory| memory.allows_interaction(ticket.layer))
            && ctx.layer_transform_to_global(ticket.layer) == ticket.transform
            && ctx.viewport(|viewport| {
                // end_pass swaps this exact committed widget set into prev_pass.
                // The local map alone cannot establish that egui kept the widget.
                viewport.prev_pass.widgets.get(ticket.id).is_some_and(|widget| {
                    widget.enabled
                        && widget.visible
                        && widget.rect == ticket.rect
                        && widget.interact_rect == ticket.clip
                        && widget.layer_id == ticket.layer
                })
            })
            && self.building.get(&ticket.id).is_some_and(|painted| {
                painted.control == ticket.control
                    && painted.action.same_action(action)
                    && painted.rect == ticket.rect
                    && painted.clip == ticket.clip
                    && painted.layer == ticket.layer
                    && painted.transform == ticket.transform
                    && painted.clip.is_positive()
                    && painted.rect.intersect(painted.clip).is_positive()
                    && match (ticket.key, ticket.position) {
                        (None, Some(position)) => painted.clip.contains(position) && ctx.layer_id_at(position) == Some(ticket.layer),
                        (Some(_), None) => painted.focused && ctx.memory(|memory| memory.focused() == Some(ticket.id)),
                        _ => false,
                    }
            });
        if self.dialog_completed.take().is_some() && valid {
            self.dialog = None;
        }
        valid
    }
    fn finish(&mut self, ctx: &egui::Context) {
        self.finished_pass = Some(ctx.cumulative_pass_nr());
        // Virtualization/removal may prevent the originating widget from being
        // visited. Its retirement or exact release still consumes the gesture.
        let frame = ctx.cumulative_frame_nr();
        if self
            .held
            .as_ref()
            .is_some_and(|held| !self.building.contains_key(&held.id) || ctx.input(|input| held.release(input, frame).is_some()))
        {
            self.held = None;
        }
        // Remember the actual final focus, never a map iteration's first row.
        // The maps are identity lookups; their iteration order has no UI role.
        let focus = ctx.memory(|memory| memory.focused());
        if self.held.as_ref().is_some_and(|held| held.key.is_some() && focus != Some(held.id)) {
            self.held = None;
        }
        self.focus = focus;
    }
    fn capture(&mut self, response: &egui::Response, make: impl FnOnce() -> Option<Action>) -> Option<Action> {
        let frame = response.ctx.cumulative_frame_nr();
        let (pointer_press, key_press) = response.ctx.input(|input| {
            (
                input.events.iter().enumerate().find_map(|(index, event)| match event {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        ..
                    } => Some((index, *pos)),
                    _ => None,
                }),
                input.events.iter().enumerate().find_map(|(index, event)| match event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } if matches!(key, egui::Key::Enter | egui::Key::Space) && !modifiers.ctrl && !modifiers.alt && !modifiers.command => {
                        Some((index, *key))
                    }
                    _ => None,
                }),
            )
        });
        let interactive = response.ctx.memory(|memory| memory.allows_interaction(response.layer_id)) && !self.suspended;
        let pointer = pointer_press.filter(|(_, pos)| {
            interactive
                && response.interact_rect.contains(*pos)
                && response.ctx.layer_id_at(*pos) == Some(response.layer_id)
                && (response.is_pointer_button_down_on() || response.clicked_by(egui::PointerButton::Primary))
        });
        let key = key_press.filter(|_| interactive && response.has_focus());
        if !self.input_blocked
            && response.enabled()
            && (pointer.is_some() || key.is_some())
            && self.last_press != Some(frame)
            && self.held.is_none()
        {
            self.last_press = Some(frame);
            let (press_index, key) = if let Some((index, _)) = pointer {
                (index, None)
            } else {
                let (index, key) = key?;
                (index, Some(key))
            };
            // A queued press only binds the exact previously painted clipped
            // control. Its actual event position/order, not the final pointer
            // position of an input batch, determines the captured target.
            self.held = self
                .painted
                .get(&response.id)
                .filter(|painted| {
                    painted.rect == response.rect
                        && painted.clip == response.interact_rect
                        && painted.layer == response.layer_id
                        && (key.is_none() || painted.focused)
                })
                .map(|painted| Held {
                    id: response.id,
                    control: painted.control,
                    rect: painted.rect,
                    clip: painted.clip,
                    layer: painted.layer,
                    transform: painted.transform,
                    action: painted.action.clone(),
                    key,
                    press_frame: frame,
                    press_index,
                });
        }
        if self.instance != 0
            && interactive
            && response.enabled()
            && self.building.len() < MAX_CONTROLS
            && let Some(action) = make()
        {
            let control = self
                .painted
                .get(&response.id)
                .filter(|painted| {
                    painted.action.same_action(&action)
                        && painted.rect == response.rect
                        && painted.clip == response.interact_rect
                        && painted.layer == response.layer_id
                        && painted.transform == response.ctx.layer_transform_to_global(response.layer_id)
                })
                .map(|painted| painted.control)
                .or_else(|| next(&mut self.next_control));
            let Some(control) = control else { return None };
            self.building.insert(
                response.id,
                Painted {
                    control,
                    action,
                    rect: response.rect,
                    layer: response.layer_id,
                    transform: response.ctx.layer_transform_to_global(response.layer_id),
                    clip: response.interact_rect,
                    focused: response.has_focus(),
                },
            );
        }
        let released = self
            .held
            .as_ref()
            .filter(|held| held.id == response.id)
            .and_then(|held| response.ctx.input(|input| held.release(input, frame)));
        if let Some(position) = released {
            let held = self.held.take()?;
            let owns_release = position.map_or_else(
                || response.has_focus(),
                |pos| response.interact_rect.contains(pos) && response.ctx.layer_id_at(pos) == Some(response.layer_id),
            );
            if interactive && !self.input_blocked && response.enabled() && owns_release {
                let ticket = ActivationControlTicket {
                    instance: self.instance,
                    release: next(&mut self.next_release)?,
                    control: held.control,
                    frame,
                    id: response.id,
                    rect: held.rect,
                    clip: held.clip,
                    layer: held.layer,
                    transform: held.transform,
                    key: held.key,
                    position,
                };
                let mut action = held.action;
                action.activation = Some(ticket);
                self.released = Some(ticket);
                return Some(action);
            }
        }

        None
    }
}
pub(crate) struct Context<'a> {
    pub data: Option<&'a Catalogue>,
    pub selected: Option<BoardId>,
    pub runtime: &'a Playback,
    pub inventory: &'a Inventory,
    pub manager: &'a Manager,
    pub proof: Option<Proof>,
    pub add: Option<PlaybackItem>,
    pub writable: bool,
    pub status: &'static str,
}
pub(crate) fn show(ctx: &egui::Context, p: &Palette, state: &mut State, open: &mut bool, context: Context<'_>) -> Option<Action> {
    let Context {
        data,
        selected,
        runtime,
        inventory,
        manager,
        proof,
        add,
        writable,
        status,
    } = context;
    state.bind(proof);
    state.begin(ctx);
    state.suspended = state.dialog.is_some();
    let mut output = None;
    Floater::new(
        "gesture_boards",
        aurora_i18n::tr!("board-title"),
        egui::pos2(100.0, 130.0),
        egui::vec2(610.0, 460.0),
    )
    .show(ctx, p, open, |ui| {
        ui.label(aurora_i18n::text(status));
        ui.label(aurora_i18n::tr!("board-help"));
        let Some(data) = data else {
            ui.label(aurora_i18n::tr!("board-no-data"));
            return;
        };
        let action = |command| {
            proof.map(|proof| Action {
                proof,
                revision: data.revision,
                command,
                activation: None,
            })
        };
        egui::ScrollArea::horizontal().id_salt("board_choices").show(ui, |ui| {
            ui.horizontal(|ui| {
                for board in &data.boards {
                    ui.push_id(board.id, |ui| {
                        let response = ui.add_enabled(proof.is_some(), egui::Button::new(&board.name).selected(selected == Some(board.id)));
                        if let Some(a) = state.capture(&response, || action(Command::Select(board.id))) {
                            output = Some(a);
                        }
                    });
                }
            });
        });
        ui.horizontal(|ui| {
            let new = ui.add_enabled(
                writable && proof.is_some() && data.boards.len() < model::MAX_BOARDS,
                egui::Button::new(aurora_i18n::tr!("board-new")),
            );
            if let Some(a) = state.capture(&new, || action(Command::BeginNew)) {
                output = Some(a);
            }
            let rename = ui.add_enabled(
                writable && proof.is_some() && selected.is_some(),
                egui::Button::new(aurora_i18n::tr!("common-rename")),
            );
            if let Some(a) = state.capture(&rename, || selected.and_then(|b| action(Command::BeginRename(b)))) {
                output = Some(a);
            }
            let delete = ui.add_enabled(
                writable && proof.is_some() && selected.is_some(),
                egui::Button::new(aurora_i18n::tr!("common-delete")),
            );
            if let Some(a) = state.capture(&delete, || selected.and_then(|b| action(Command::BeginDelete(b)))) {
                output = Some(a);
            }
        });
        let Some(board) = selected.and_then(|id| data.board(id)) else {
            return;
        };
        ui.horizontal(|ui| {
            let add_button = ui
                .add_enabled(
                    writable && proof.is_some() && add.is_some() && board.tiles.len() < model::MAX_TILES,
                    egui::Button::new(aurora_i18n::tr!("board-add")),
                )
                .on_hover_text(aurora_i18n::tr!("board-selection-help"));
            if let Some(a) = state.capture(&add_button, || add.and_then(|item| action(Command::Add(board.id, item)))) {
                output = Some(a);
            }
            let inv = ui.add_enabled(proof.is_some(), egui::Button::new(aurora_i18n::tr!("common-inventory")));
            if let Some(a) = state.capture(&inv, || action(Command::Inventory)) {
                output = Some(a);
            }
            let stop = ui
                .add_enabled(proof.is_some(), egui::Button::new(aurora_i18n::tr!("board-stop")))
                .on_hover_text(aurora_i18n::tr!("board-sound-help"));
            if let Some(a) = state.capture(&stop, || {
                runtime
                    .capture_board(data, board.id, Instant::now())
                    .ok()
                    .and_then(|t| action(Command::Stop(board.id, t)))
            }) {
                output = Some(a);
            }
        });
        if board.tiles.is_empty() {
            ui.label(aurora_i18n::tr!("board-empty"));
        }
        // Virtualized rows keep all1024 tiles reachable without a frame-sized
        // clone of inventory names, definitions or the saved catalogue.
        let rows = board.tiles.len().div_ceil(3);
        let width = ((ui.available_width() - ui.spacing().item_spacing.x * 2.0) / 3.0).max(100.0);
        egui::ScrollArea::vertical()
            .id_salt("board_tiles")
            .auto_shrink([false, false])
            .show_rows(ui, 82.0, rows, |ui, range| {
                for row in range {
                    ui.horizontal(|ui| {
                        for tile in board.tiles.iter().skip(row * 3).take(3) {
                            ui.push_id(tile.id, |ui| {
                                ui.allocate_ui(egui::vec2(width, 78.0), |ui| {
                                    let target = resolve(inventory, tile.item);
                                    let label = inventory
                                        .items
                                        .get(&tile.item.reference)
                                        .map(|item| item.name.clone())
                                        .unwrap_or_else(|| aurora_i18n::tr!("board-missing").into());
                                    ui.add(egui::Label::new(egui::RichText::new(label).strong()).truncate());
                                    ui.label(aurora_i18n::text(if tile.item.kind == model::Kind::Sound {
                                        "gesture-sound"
                                    } else {
                                        "board-kind-gesture"
                                    }));
                                    ui.horizontal(|ui| {
                                        let playing = target.is_some_and(|item| manager.playback_id(item.item()).is_some());
                                        let loading = target.is_some_and(|item| runtime.pending(item.item()).is_some());
                                        let text = if playing {
                                            aurora_i18n::tr!("common-stop")
                                        } else if loading {
                                            aurora_i18n::tr!("gesture-loading")
                                        } else {
                                            aurora_i18n::tr!("common-play")
                                        };
                                        let play = ui.add_enabled(proof.is_some() && target.is_some(), egui::Button::new(text));
                                        if let Some(a) = state.capture(&play, || {
                                            target.and_then(|item| {
                                                runtime
                                                    .capture(data, board.id, tile.id, item, manager, Instant::now())
                                                    .ok()
                                                    .and_then(|ticket| action(Command::Play(ticket, item)))
                                            })
                                        }) {
                                            output = Some(a);
                                        }
                                        let remove = ui
                                            .add_enabled(writable && proof.is_some(), egui::Button::new(aurora_i18n::tr!("common-remove")));
                                        if let Some(a) = state.capture(&remove, || {
                                            action(Command::Edit(Change::Remove {
                                                board: board.id,
                                                tile: tile.id,
                                            }))
                                        }) {
                                            output = Some(a);
                                        }
                                    });
                                });
                            });
                        }
                    });
                }
            });
    });
    state.suspended = false;
    if !*open {
        state.cancel();
        return None;
    }
    if let Some(Action {
        proof,
        revision,
        command,
        activation,
    }) = output.take()
    {
        let dialog = match command {
            Command::BeginNew => Some((None, false, String::new())),
            Command::BeginRename(id) => data.and_then(|d| d.board(id)).map(|b| (Some(id), false, b.name.clone())),
            Command::BeginDelete(id) => data.and_then(|d| d.board(id)).map(|b| (Some(id), true, b.name.clone())),
            _ => {
                output = Some(Action {
                    proof,
                    revision,
                    command,
                    activation,
                });
                None
            }
        };
        if let Some((board, delete, name)) = dialog {
            state.held = None;
            state.painted.clear();
            state.building.clear();
            state.dialog = Some(Dialog {
                origin: proof,
                revision,
                board,
                delete,
                name,
                created: BoardId::new(),
            });
        }
    }
    // Pull dialog out so text editing and the ordinary held-control state use
    // disjoint borrows; it is restored only while its exact origin still exists.
    if let Some(mut dialog) = state.dialog.take() {
        let valid = proof.is_some_and(|p| p.same_owner(dialog.origin))
            && data.is_some_and(|d| d.revision == dialog.revision && dialog.board.is_none_or(|id| d.board(id).is_some()));
        if valid {
            let mut keep = true;
            Modal::new(egui::Id::unique("board_edit")).show(ctx, |ui| {
                ui.heading(aurora_i18n::text(if dialog.delete {
                    "board-delete-question"
                } else {
                    "board-edit-name"
                }));
                if dialog.delete {
                    ui.label(&dialog.name);
                } else {
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.name)
                            .hint_text(aurora_i18n::tr!("board-name-placeholder"))
                            .char_limit(64),
                    );
                }
                ui.horizontal(|ui| {
                    let enabled = writable && (dialog.delete || model::valid_name(&dialog.name));
                    let confirm = ui.add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("common-save")));
                    if let Some(a) = state.capture(&confirm, || {
                        let command = if dialog.delete {
                            Change::Delete { board: dialog.board? }
                        } else if let Some(board) = dialog.board {
                            Change::Rename {
                                board,
                                name: dialog.name.clone(),
                            }
                        } else {
                            Change::Create {
                                id: dialog.created,
                                name: dialog.name.clone(),
                            }
                        };
                        proof.map(|proof| Action {
                            proof,
                            revision: dialog.revision,
                            command: Command::Edit(command),
                            activation: None,
                        })
                    }) {
                        output = Some(a);
                        // The final pass must still contain this exact Save
                        // control. The post-run consumer closes it; direct UI
                        // callers retire it at the next frame boundary.
                        state.dialog_completed = Some(ctx.cumulative_frame_nr());
                    }
                    if ui.button(aurora_i18n::tr!("common-cancel")).clicked() {
                        keep = false;
                        state.held = None;
                    }
                });
            });
            if keep {
                state.dialog = Some(dialog);
            }
        }
    }
    state.finish(ctx);
    output
}
