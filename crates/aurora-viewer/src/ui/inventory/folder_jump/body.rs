//! The same bounded chooser body used by the ordinary Inventory floater and
//! actual-egui fixtures. Navigation only leaves through a captured exact ticket.

use super::model::{Model, Phase, Policy, Source, Ticket};
use crate::theme::Palette;
use crate::ui::widgets;
use crate::world::inventory::Inventory;
use egui::{Id, Key, LayerId, Pos2, Rect, RichText};

#[derive(Clone, Copy)]
enum Intent {
    Select(Ticket),
    Open(Ticket),
}

#[cfg(test)]
mod tests;

impl Intent {
    fn ticket(self) -> Ticket {
        match self {
            Self::Select(ticket) | Self::Open(ticket) => ticket,
        }
    }
}

#[derive(Clone, Copy)]
struct Control {
    widget: Id,
    rect: Rect,
    layer: LayerId,
    pointer: Option<Intent>,
    enter: Option<Intent>,
    space: Option<Intent>,
    up: Option<Ticket>,
    down: Option<Ticket>,
}

impl Control {
    fn key(self, key: Key) -> Option<Intent> {
        match key {
            Key::Enter => self.enter,
            Key::Space => self.space,
            Key::ArrowUp => self.up.map(Intent::Select),
            Key::ArrowDown => self.down.map(Intent::Select),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Edge {
    Pointer { pos: Pos2, down: bool },
    Key { key: Key, down: bool, repeat: bool, plain: bool },
}

#[derive(Clone, Copy)]
enum Trigger {
    Pointer,
    Key(Key),
    Accessibility,
}

#[derive(Clone, Copy)]
struct Gesture {
    control: Control,
    intent: Intent,
    trigger: Trigger,
}

#[derive(Clone, Copy)]
enum Release {
    Pointer(Pos2),
    Key(Key),
    Accessibility,
}

pub(super) enum Action {
    Open(Ticket),
    Close,
}

#[derive(Default)]
pub(super) struct Body {
    model: Model,
    query: String,
    source: Option<Source>,
    controls: Vec<Control>,
    gesture: Option<Gesture>,
    pending_release: Option<(Gesture, Release)>,
    input_frame: Option<u64>,
    work_frame: Option<u64>,
    ime: bool,
    focus_query: bool,
    query_widget: Option<Id>,
    reveal: Option<uuid::Uuid>,
    /// Native adapter fences physical Enter/Space across main-window refocus.
    blocked_keys: [bool; 2],
}

impl Body {
    pub(super) fn source(&self) -> Option<Source> {
        self.model.source()
    }

    pub(super) fn validate(&self, ticket: Ticket, inventory: &Inventory, source: Option<Source>, policy: Policy) -> bool {
        self.model.validate(ticket, inventory, source, policy)
    }

    pub(super) fn observe(&mut self, inventory: &Inventory, source: Option<Source>, policy: Policy) {
        self.model.sync(inventory, source, policy);
        let source = self.model.source();
        if source != self.source {
            self.query.clear();
            self.controls.clear();
            self.gesture = None;
            self.pending_release = None;
            self.ime = false;
            self.source = source;
            self.focus_query = source.is_some();
        }
        self.controls.retain(|control| {
            [
                control.pointer,
                control.enter,
                control.space,
                control.up.map(Intent::Select),
                control.down.map(Intent::Select),
            ]
            .into_iter()
            .flatten()
            .any(|intent| self.model.validate(intent.ticket(), inventory, source, policy))
        });
        if self
            .gesture
            .is_some_and(|gesture| !self.model.validate(gesture.intent.ticket(), inventory, source, policy))
        {
            self.gesture = None;
        }
    }

    pub(super) fn cancel_input(&mut self) {
        self.model.cancel_input();
        self.controls.clear();
        self.gesture = None;
        self.pending_release = None;
        self.input_frame = None;
        self.ime = false;
    }

    pub(super) fn block_key(&mut self, key: Key, blocked: bool) {
        let index = match key {
            Key::Enter => 0,
            Key::Space => 1,
            _ => return,
        };
        self.blocked_keys[index] = blocked;
        if blocked {
            self.gesture = None;
        }
    }

    fn key_allowed(&self, key: Key) -> bool {
        match key {
            Key::Enter => !self.blocked_keys[0],
            Key::Space => !self.blocked_keys[1],
            _ => true,
        }
    }

    fn capture(&mut self, ui: &egui::Ui, inventory: &Inventory, source: Option<Source>, policy: Policy) -> Option<(Gesture, Release)> {
        let frame = ui.ctx().cumulative_frame_nr();
        if self.input_frame == Some(frame) {
            return self.pending_release;
        }
        self.input_frame = Some(frame);
        self.pending_release = None;
        let focus = ui.ctx().memory(|memory| memory.focused());
        // Snapshot only small event metadata. Context access below is OUTSIDE
        // input(): egui 0.37 takes a write lock while that closure runs.
        let (edges, overflow, ime_seen, terminal, composing, focused) = ui.input(|input| {
            let mut edges = Vec::new();
            let mut ime_seen = false;
            let mut terminal = false;
            let mut composing = self.ime;
            let mut overflow = false;
            for (index, event) in input.events.iter().enumerate() {
                let edge = match event {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        ..
                    } => Some(Edge::Pointer { pos: *pos, down: *pressed }),
                    egui::Event::Key {
                        key,
                        pressed,
                        repeat,
                        modifiers,
                        ..
                    } if matches!(key, Key::Enter | Key::Space | Key::ArrowUp | Key::ArrowDown) => Some(Edge::Key {
                        key: *key,
                        down: *pressed,
                        repeat: *repeat,
                        plain: modifiers.is_none(),
                    }),
                    egui::Event::Ime(event) => {
                        ime_seen = true;
                        match event {
                            egui::ImeEvent::Preedit { text, .. } => {
                                composing = !text.is_empty();
                                terminal |= text.is_empty();
                            }
                            egui::ImeEvent::Commit(_) | egui::ImeEvent::Disabled => {
                                composing = false;
                                terminal = true;
                            }
                            egui::ImeEvent::Enabled => {}
                        }
                        None
                    }
                    _ => None,
                };
                if let Some(edge) = edge {
                    if edges.len() < 128 {
                        edges.push((index, edge));
                    } else {
                        overflow = true;
                    }
                }
            }
            (edges, overflow, ime_seen, terminal, composing, input.focused)
        });
        self.ime = composing;
        if terminal {
            self.focus_query = false;
        }
        if !ui.is_enabled() || !focused || overflow || ime_seen || self.ime {
            self.gesture = None;
            if ime_seen || self.ime {
                // The query keeps its genuine Commit/Text events. Confirmation
                // keys cannot make the single-line field lose focus first.
                ui.input_mut(|input| {
                    input.events.retain(|event| {
                        !matches!(
                            event,
                            egui::Event::Key {
                                key: Key::Enter | Key::Space,
                                ..
                            }
                        )
                    })
                });
            }
            return None;
        }
        let valid = |intent: Intent| self.model.validate(intent.ticket(), inventory, source, policy);
        let mut released = None;
        let mut consumed = Vec::new();
        for (index, edge) in edges {
            match edge {
                Edge::Pointer { pos, down: true } => {
                    if self.gesture.is_some_and(|gesture| matches!(gesture.trigger, Trigger::Key(_))) {
                        self.gesture = None;
                    }
                    if self.gesture.is_none()
                        && let Some(control) = self.controls.iter().rev().find(|control| {
                            control.pointer.is_some_and(valid)
                                && control.rect.contains(pos)
                                && ui.ctx().layer_id_at(pos) == Some(control.layer)
                        })
                    {
                        self.gesture = control.pointer.map(|intent| Gesture {
                            control: *control,
                            intent,
                            trigger: Trigger::Pointer,
                        });
                    }
                }
                Edge::Key {
                    key,
                    down: true,
                    repeat: false,
                    plain: true,
                } if self.key_allowed(key) => {
                    if self.gesture.is_none()
                        && let Some(control) = self
                            .controls
                            .iter()
                            .rev()
                            .find(|control| focus == Some(control.widget) && control.key(key).is_some_and(valid))
                    {
                        self.gesture = control.key(key).map(|intent| Gesture {
                            control: *control,
                            intent,
                            trigger: Trigger::Key(key),
                        });
                        consumed.push(index);
                    }
                }
                Edge::Pointer { pos, down: false } => {
                    if let Some(gesture) = self.gesture.filter(|gesture| matches!(gesture.trigger, Trigger::Pointer)) {
                        self.gesture = None;
                        released.get_or_insert((gesture, Release::Pointer(pos)));
                    }
                }
                Edge::Key { key, down: false, .. } => {
                    if let Some(gesture) = self
                        .gesture
                        .filter(|gesture| matches!(gesture.trigger, Trigger::Key(held) if held == key))
                    {
                        self.gesture = None;
                        released.get_or_insert((gesture, Release::Key(key)));
                        consumed.push(index);
                    }
                }
                _ => {}
            }
        }
        if !consumed.is_empty() {
            ui.input_mut(|input| {
                let mut index = 0;
                input.events.retain(|_| {
                    let keep = !consumed.contains(&index);
                    index += 1;
                    keep
                });
            });
        }
        if released.is_none() && self.gesture.is_none() {
            for control in &self.controls {
                if focus == Some(control.widget)
                    && control.enter.is_some_and(valid)
                    && ui.input(|input| input.has_accesskit_action_request(control.widget, egui::accesskit::Action::Click))
                    && let Some(intent) = control.enter
                {
                    released = Some((
                        Gesture {
                            control: *control,
                            intent,
                            trigger: Trigger::Accessibility,
                        },
                        Release::Accessibility,
                    ));
                    break;
                }
            }
        }
        self.pending_release = released;
        released
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        inventory: &Inventory,
        source: Option<Source>,
        policy: Policy,
    ) -> Option<Action> {
        ui.push_id(self.model.widget_namespace(), |ui| {
            self.show_inner(ui, palette, inventory, source, policy)
        })
        .inner
    }

    fn show_inner(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        inventory: &Inventory,
        source: Option<Source>,
        policy: Policy,
    ) -> Option<Action> {
        // Keep ownership before pruning: terminal IME must retire its focus
        // even if an Inventory revision just invalidated that control ticket.
        let owned_focus = ui
            .ctx()
            .memory(|memory| memory.focused())
            .filter(|focus| self.controls.iter().any(|control| control.widget == *focus));
        let terminal_ime = ui.input(|input| {
            input.events.iter().any(|event| {
                matches!(event, egui::Event::Ime(egui::ImeEvent::Commit(_) | egui::ImeEvent::Disabled))
                    || matches!(event, egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) if text.is_empty())
            })
        });
        if terminal_ime && let Some(focus) = owned_focus.filter(|focus| Some(*focus) != self.query_widget) {
            ui.ctx().memory_mut(|memory| memory.surrender_focus(focus));
        }
        self.observe(inventory, source, policy);
        let frame = ui.ctx().cumulative_frame_nr();
        if self.work_frame != Some(frame) {
            self.work_frame = Some(frame);
            self.model.advance(inventory);
        }
        let live =
            ui.is_enabled() && ui.ctx().memory(|memory| memory.is_above_modal_layer(ui.layer_id())) && !egui::Popup::is_any_open(ui.ctx());
        if !live {
            self.cancel_input();
        }
        let released = self.capture(ui, inventory, source, policy);
        let mut painting = Vec::with_capacity(64);
        let field = ui
            .push_id("folder_jump_query", |ui| {
                // Same themed TextEdit as widgets::search_field, with a character
                // limit so a paste/IME commit cannot retain an unbounded query.
                ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text(aurora_i18n::tr!("inventory-jump-query"))
                        .desired_width(ui.available_width())
                        .char_limit(256),
                )
            })
            .inner;
        self.query_widget = Some(field.id);
        if terminal_ime && field.has_focus() {
            // Commit reached the actual TextEdit above exactly once. Retire
            // only afterward, so its later confirmation Enter cannot Open.
            ui.ctx().memory_mut(|memory| memory.surrender_focus(field.id));
            self.focus_query = false;
        }
        if field.changed() {
            self.model.set_query(&self.query);
        }
        if self.focus_query && !terminal_ime {
            field.request_focus();
            self.focus_query = false;
        }
        let navigation = if live {
            (self.model.capture_navigation(true), self.model.capture_navigation(false))
        } else {
            (None, None)
        };
        let selected = live.then(|| self.model.capture_selected()).flatten();
        let query_control = Control {
            widget: field.id,
            rect: field.interact_rect.intersect(ui.clip_rect()),
            layer: field.layer_id,
            pointer: None,
            enter: selected.map(Intent::Open),
            space: None,
            up: navigation.0,
            down: navigation.1,
        };
        painting.push(query_control);
        let status = match self.model.phase() {
            Phase::Indexing | Phase::Matching => aurora_i18n::tr!("inventory-jump-scanning").to_owned(),
            Phase::InvalidQuery => aurora_i18n::tr!("inventory-jump-query-limit").to_owned(),
            Phase::Unavailable | Phase::Exhausted => aurora_i18n::tr!("inventory-jump-unavailable").to_owned(),
            Phase::Ready if self.model.limited() => aurora_i18n::tr!("inventory-jump-limited").to_owned(),
            Phase::Ready if self.model.matches() > 60 => aurora_i18n::tr!("inventory-jump-first", count = 60),
            Phase::Ready if self.model.matches() == 0 => aurora_i18n::tr!("inventory-jump-empty").to_owned(),
            Phase::Ready => aurora_i18n::tr!("inventory-jump-count", count = self.model.matches()),
        };
        ui.label(RichText::new(status).small().color(palette.muted));
        let (mut list, footer) = widgets::above_footer(ui);
        egui::ScrollArea::vertical()
            .id_salt("folder_jump_results")
            .auto_shrink([false, false])
            .show(&mut list, |ui| {
                for row in self.model.rows() {
                    let caption = format!(
                        "{}\n{}",
                        row.name,
                        if row.parent_path.is_empty() {
                            aurora_i18n::tr!("common-my-inventory")
                        } else {
                            row.parent_path
                        }
                    );
                    let response = ui
                        .push_id(row.widget, |ui| {
                            ui.add(
                                egui::Button::new(caption)
                                    .selected(self.model.selected() == Some(row.folder))
                                    .truncate()
                                    .min_size(egui::vec2(ui.available_width(), 0.0)),
                            )
                        })
                        .inner;
                    if self.reveal == Some(row.folder) {
                        response.scroll_to_me(Some(egui::Align::Center));
                        self.reveal = None;
                    }
                    let ticket = live.then(|| self.model.capture(row.folder)).flatten();
                    painting.push(Control {
                        widget: response.id,
                        rect: response.interact_rect.intersect(ui.clip_rect()),
                        layer: response.layer_id,
                        pointer: ticket.map(Intent::Select),
                        enter: ticket.map(Intent::Open),
                        space: ticket.map(Intent::Select),
                        up: navigation.0,
                        down: navigation.1,
                    });
                }
            });
        let mut close = false;
        footer.show(ui, |ui| {
            ui.horizontal(|ui| {
                let width = ((ui.available_width() - ui.spacing().item_spacing.x) * 0.5).max(0.0);
                let response = ui
                    .add_enabled_ui(selected.is_some(), |ui| {
                        widgets::primary_button_sized(ui, palette, aurora_i18n::tr!("common-open"), egui::vec2(width, 30.0))
                    })
                    .inner;
                painting.push(Control {
                    widget: response.id,
                    rect: response.interact_rect.intersect(ui.clip_rect()),
                    layer: response.layer_id,
                    pointer: selected.map(Intent::Open),
                    enter: selected.map(Intent::Open),
                    space: selected.map(Intent::Open),
                    up: None,
                    down: None,
                });
                close = widgets::flat_button_sized(ui, palette, aurora_i18n::tr!("common-cancel"), egui::vec2(width, 30.0)).clicked();
            });
        });
        if ui.ctx().will_discard() {
            return None;
        }
        if close || (!self.ime && !terminal_ime && ui.input(|input| input.key_pressed(Key::Escape))) {
            self.cancel_input();
            return Some(Action::Close);
        }
        let activated = released
            .filter(|(gesture, release)| {
                self.model.validate(gesture.intent.ticket(), inventory, source, policy)
                    && painting.iter().any(|control| {
                        control.widget == gesture.control.widget
                            && match release {
                                Release::Pointer(pos) => control.rect.contains(*pos) && ui.ctx().layer_id_at(*pos) == Some(control.layer),
                                Release::Key(key) => {
                                    ui.ctx().memory(|memory| memory.focused()) == Some(control.widget) && self.key_allowed(*key)
                                }
                                Release::Accessibility => ui.ctx().memory(|memory| memory.focused()) == Some(control.widget),
                            }
                    })
            })
            .map(|(gesture, _)| (gesture.intent, gesture.trigger));
        self.controls = painting;
        self.pending_release = None;
        match activated {
            Some((Intent::Open(ticket), _)) => Some(Action::Open(ticket)),
            Some((Intent::Select(ticket), trigger)) => {
                if self.model.select(ticket, inventory, source, policy) {
                    if matches!(trigger, Trigger::Key(Key::ArrowUp | Key::ArrowDown)) {
                        self.reveal = Some(ticket.folder());
                        if let Some(control) = self.controls.iter().find(|control| {
                            matches!(control.pointer,
                                Some(Intent::Select(row)) if row.folder() == ticket.folder()
                            )
                        }) {
                            ui.ctx().memory_mut(|memory| memory.request_focus(control.widget));
                        }
                    }
                    ui.ctx().request_repaint();
                }
                None
            }
            None => None,
        }
    }
}
