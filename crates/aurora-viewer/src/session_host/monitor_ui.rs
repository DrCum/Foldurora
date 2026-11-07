//! Test-only compact monitor body following the pinned Legacy monitor window
//! (tools/session-host/main.cpp, abbe3cb0ca66ee2484f277219f6a27706fab7fc6).
//! Pixels and their credited/native/service fences belong to the future surface
//! owner. This body never loads/registers/copies pixels, opens an endpoint, or
//! applies a source transition. Parent flushes old delivered edits before it
//! changes bindings; every canonical chat pass names its exact ChatSurface.

use super::chat_ui::{self, InputMode, ReadDraft, SendDraft};
use super::monitor_model::{Binding, Direction, MonitorSet, Navigation, Options, Ticket, View};
use super::private_image::PrivateTextureView;
use super::shared_chat::SharedChat;
use super::transport::HostSnapshot;
use crate::theme::Palette;
use crate::ui::widgets;
use aurora_session::preview_image::AdmittedImage;
use aurora_session::{PreviewRate, PreviewSize};
use egui::{Id, Key, Rect, RichText, vec2};

/// Immutable published settings from the owner, never optimistic checkmarks.
/// The owner changes revision on every request/receipt/rebind, without reuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct State {
    pub revision: u64,
    pub chat: bool,
    pub pinned: bool,
    pub accepted: Options,
    pub request: Option<(u64, Options)>,
    pub failed: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            revision: 1,
            chat: false,
            pinned: false,
            accepted: Options::default(),
            request: None,
            failed: false,
        }
    }
}

/// Owner attests that this private texture contains exactly these admitted
/// pixels and retains all conversion/painter credits. Actual service/view/native
/// guards and erasure of old texture/OS output are independent prerequisites.
pub(super) struct Image<'a> {
    pub binding: Binding,
    pub admitted: &'a AdmittedImage,
    pub texture: &'a PrivateTextureView<'a>,
}
impl Image<'_> {
    fn current(&self, binding: Binding, body: &MonitorUi, context: &egui::Context, now: u64) -> bool {
        let size = self.admitted.stamp().geometry().size().map(|side| side as usize);
        self.binding == binding
            && binding.enabled
            && binding.visible
            && self.admitted.stamp().claim().source() == binding.source.preview()
            && self.texture.context() == context
            && body.context == *context
            && body.generation != 0
            && self.texture.generation() == body.generation
            && body.binding_generation != 0
            && self.texture.binding_generation() == body.binding_generation
            && self.texture.size() == size
            && self.admitted.current(now)
    }
}

// A bounded painted-control set keeps complete captured proofs inline and Copy,
// avoiding per-paint allocations or a weaker reconstructed navigation target.
#[expect(
    clippy::large_enum_variant,
    reason = "bounded Copy captures retain the original immutable navigation proof inline"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Command {
    Exchange(Ticket),
    Chat(Ticket, bool),
    Pin(Ticket, bool),
    Settings { ticket: Ticket, base: State, desired: Options },
    Navigate(Navigation),
    Pause,
    Play,
    Interval(u32),
}
/// Parent revalidates the captured local ticket, binding, published base,
/// playing/interval observations and any strong command ticket after its latest
/// sync, then flushes before applying changes. These values confer no authority.
// One transient dispatch carries the complete original view/source/base guards;
// this value contains neither queued pixel data nor an operation ledger.
#[expect(
    clippy::large_enum_variant,
    reason = "one ephemeral action retains all original guards together for parent revalidation"
)]
#[derive(Debug)]
pub(super) enum Action {
    Control {
        view: View,
        ticket: Ticket,
        binding: Binding,
        base: State,
        playing: bool,
        interval: u32,
        command: Command,
    },
    Send(SendDraft),
    Read(ReadDraft),
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Intent {
    view_ticket: Ticket,
    binding: Binding,
    state: State,
    playing: bool,
    interval: u32,
    command: Command,
}
impl Intent {
    fn valid(self, view: View, model: &MonitorSet, state: State) -> bool {
        if !model.validate_view(self.view_ticket)
            || model.binding(view) != Some(self.binding)
            || self.state != state
            || self.playing != model.playing()
            || self.interval != model.interval()
        {
            return false;
        }
        match self.command {
            Command::Navigate(navigation) => model.validate_navigation(navigation),
            Command::Exchange(ticket) => model.can_exchange(ticket),
            Command::Chat(ticket, _) | Command::Pin(ticket, _) => model.validate(ticket),
            Command::Settings { ticket, .. } => state.request.is_none() && model.validate(ticket),
            Command::Pause | Command::Play | Command::Interval(_) => view == View::Cycling,
        }
    }
}
#[derive(Clone, Copy)]
struct Painted {
    id: Id,
    widget: Id,
    rect: Rect,
    layer: egui::LayerId,
    intent: Intent,
}
#[derive(Clone, Copy)]
enum Trigger {
    Pointer,
    Key(Key),
    Access,
}
#[derive(Clone, Copy)]
struct Gesture {
    control: Painted,
    trigger: Trigger,
    frame: u64,
    press_index: usize,
}

pub(super) struct MonitorUi {
    context: egui::Context,
    generation: u64,
    binding_generation: u64,
    controls: Vec<Painted>,
    painting: Vec<Painted>,
    // Non-private command/presentation widget identities, committed with paint.
    // Separate from valid action tickets so state revocation cannot hide focus.
    focus_controls: Vec<Id>,
    focus_painting: Vec<Id>,
    gesture: Option<Gesture>,
    input_frame: Option<u64>,
    observed: Option<(Binding, bool)>,
    transition_frame: Option<u64>,
    ime: bool,
    options_open: bool,
    interval_text: String,
    interval_seen: u32,
}
impl MonitorUi {
    pub fn new(context: &egui::Context, generation: u64, binding_generation: u64) -> Self {
        Self {
            context: context.clone(),
            generation,
            binding_generation,
            controls: Vec::new(),
            painting: Vec::new(),
            focus_controls: Vec::new(),
            focus_painting: Vec::new(),
            gesture: None,
            input_frame: None,
            observed: None,
            transition_frame: None,
            ime: false,
            options_open: false,
            interval_text: "10".into(),
            interval_seen: 10,
        }
    }
    pub fn reset_surface(&mut self, context: &egui::Context, generation: u64, binding_generation: u64) {
        self.context = context.clone();
        self.generation = generation;
        self.binding_generation = binding_generation;
        self.cancel_input();
        self.observed = None;
    }
    pub fn cancel_input(&mut self) {
        self.controls.clear();
        self.painting.clear();
        self.focus_controls.clear();
        self.focus_painting.clear();
        self.gesture = None;
        self.input_frame = None;
        self.ime = false;
        self.options_open = false;
    }
    /// Call after every model/state sync even when hidden. This removes stale
    /// captured identities; it does not erase a surface's external pixel cache.
    pub fn sync(&mut self, view: View, model: &MonitorSet, state: State) {
        self.controls.retain(|control| control.intent.valid(view, model, state));
        if self
            .gesture
            .is_some_and(|gesture| !gesture.control.intent.valid(view, model, state))
        {
            self.gesture = None;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        view: View,
        model: &MonitorSet,
        chat: &mut SharedChat,
        host: &HostSnapshot,
        state: State,
        image: Option<Image<'_>>,
        now: u64,
        mode: InputMode,
    ) -> Option<Action> {
        self.sync(view, model, state);
        let frame = ui.ctx().cumulative_frame_nr();
        let binding = model.binding(view);
        if binding.map(|binding| (binding, model.playing())) != self.observed {
            self.cancel_input();
            self.observed = binding.map(|binding| (binding, model.playing()));
            self.transition_frame = Some(frame);
            chat.surface_closed(view.surface());
            chat_ui::cancel_input(ui.ctx());
            ui.ctx().memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
        if self.interval_seen != model.interval() {
            self.interval_seen = model.interval();
            self.interval_text = model.interval().to_string();
            self.gesture = None;
        }
        let focused = ui.input(|input| input.focused);
        let focus_changed = ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(_))));
        let was_ime = self.ime;
        let ime_event = ui.input(|input| {
            let mut seen = false;
            for event in &input.events {
                if let egui::Event::Ime(event) = event {
                    seen = true;
                    match event {
                        egui::ImeEvent::Preedit { text, .. } => self.ime = !text.is_empty(),
                        egui::ImeEvent::Commit(_) => self.ime = false,
                        #[allow(deprecated)]
                        egui::ImeEvent::Disabled => self.ime = false,
                        _ => {}
                    }
                }
            }
            seen
        });
        if ime_event {
            // OS confirmation keys may arrive after Commit/Disabled in another
            // frame. Retire only Monitor command focus at the IME boundary;
            // canonical TextEdit keeps its source-owned composition and focus.
            self.gesture = None;
            ui.ctx().memory_mut(|memory| {
                if let Some(id) = memory.focused().filter(|id| self.focus_controls.contains(id)) {
                    memory.surrender_focus(id);
                }
            });
        }
        let surface_live = state.revision != 0
            && mode == InputMode::Live
            && focused
            && !focus_changed
            && self.transition_frame != Some(frame)
            && self.context == *ui.ctx()
            && model.capture_view(view).is_some();
        // Composition suppresses toolbar commands, not canonical editor input.
        // ChatUi owns the source-qualified preedit/Commit/Undo transaction.
        let live = surface_live && !was_ime && !self.ime && !ime_event && !egui::Popup::is_any_open(ui.ctx());
        if !focused || focus_changed || mode == InputMode::Closing {
            self.cancel_input();
        }
        if !live {
            self.gesture = None;
        }
        self.capture(ui, view, model, state, live);
        self.painting.clear();
        self.focus_painting.clear();
        let mut command = None;
        let ticket = model.capture(view);
        egui::Panel::top(Id::unique(("monitor_top", view.surface().index())))
            .frame(widgets::overlay_frame(ui.ctx(), p))
            .show(ui, |ui| {
                if let Some(binding) = binding {
                    let profile = host
                        .slots
                        .iter()
                        .find(|slot| slot.id == binding.source.slot())
                        .map(|slot| slot.label.as_str())
                        .unwrap_or("");
                    let number = aurora_i18n::tr!("host-profile-default", number = i64::from(binding.source.slot().number()));
                    let profile_caption = if profile.is_empty() {
                        number
                    } else {
                        format!("{number} · {profile}")
                    };
                    ui.add(egui::Label::new(&profile_caption).truncate())
                        .on_hover_text(&profile_caption);
                    if let Some(status) = host
                        .workers
                        .iter()
                        .find(|worker| worker.binding == binding.source.chat().binding())
                        .and_then(|worker| worker.status.as_ref())
                    {
                        let identity = aurora_i18n::tr!(
                            "host-chat-identity",
                            account = status.name.as_str(),
                            grid = status.grid_label.as_str()
                        );
                        ui.add(egui::Label::new(RichText::new(&identity).small()).truncate())
                            .on_hover_text(&identity);
                    }
                }
                let current = binding
                    .and_then(|binding| image.as_ref().filter(|image| image.current(binding, self, ui.ctx(), now)))
                    .and_then(|image| image.admitted.age().bounds(now).ok().map(|age| (image, age)));
                let caption = current.map_or_else(
                    || {
                        aurora_i18n::text(if binding.is_some_and(|binding| binding.enabled) {
                            "host-monitor-read-only"
                        } else {
                            "host-monitor-unavailable"
                        })
                        .to_owned()
                    },
                    |(_, (youngest, oldest))| {
                        aurora_i18n::tr!(
                            "host-monitor-last-active",
                            youngest = (youngest / 1000) as i64,
                            oldest = oldest.div_ceil(1000) as i64
                        )
                    },
                );
                let width = ((ui.available_width() - 3.0 * ui.spacing().item_spacing.x) / 4.0).max(1.0);
                // Measure each button against its full cell, never the remaining
                // width of a horizontal row. Reserve footer/chat before pixels.
                let toolbar_height = [
                    "host-monitor-exchange",
                    "host-monitor-chat",
                    "host-monitor-pin",
                    "host-monitor-options",
                ]
                .into_iter()
                .map(|key| {
                    wrapped_height(
                        ui,
                        aurora_i18n::text(key),
                        (width - 2.0 * ui.spacing().button_padding.x).max(1.0),
                        egui::TextStyle::Button,
                    ) + 2.0 * ui.spacing().button_padding.y
                })
                .fold(26.0_f32, f32::max);
                let mut footer = wrapped_height(ui, &caption, ui.available_width(), egui::TextStyle::Body) + toolbar_height;
                let mut rows = 3;
                if view == View::Cycling {
                    footer += 32.0;
                    rows += 1;
                    if model.playing() && state.chat {
                        footer += wrapped_height(
                            ui,
                            aurora_i18n::text("host-monitor-pause-chat"),
                            ui.available_width(),
                            egui::TextStyle::Body,
                        );
                        rows += 1;
                    }
                }
                if state.request.is_some() || state.failed {
                    footer += wrapped_height(
                        ui,
                        aurora_i18n::text(if state.request.is_some() {
                            "host-monitor-settings-pending"
                        } else {
                            "host-monitor-settings-failed"
                        }),
                        ui.available_width(),
                        egui::TextStyle::Body,
                    );
                    rows += 1;
                }
                let reserved = footer + rows as f32 * ui.spacing().item_spacing.y + 16.0 + if state.chat { 260.0 } else { 0.0 };
                let height =
                    (ui.ctx().content_rect().bottom() - ui.cursor().top() - reserved).clamp(16.0, if state.chat { 180.0 } else { 300.0 });
                let (rect, image_response) = ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::click());
                if live && image_response.secondary_clicked() {
                    self.options_open = true;
                    self.gesture = None;
                }
                #[cfg(test)]
                ui.ctx().data_mut(|data| data.insert_temp(control_id(view, "image"), rect));
                if let Some((image, _)) = current {
                    let size = image.admitted.stamp().geometry().size();
                    let fit = (rect.width() / size[0] as f32).min(rect.height() / size[1] as f32);
                    let fitted = Rect::from_center_size(rect.center(), vec2(size[0] as f32 * fit, size[1] as f32 * fit));
                    #[cfg(test)]
                    ui.ctx().data_mut(|data| data.insert_temp(control_id(view, "image_fitted"), fitted));
                    // theme-guard: content — admitted image colors are preserved without theme tint.
                    ui.painter().image(
                        image.texture.texture_id(),
                        fitted,
                        Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                } else {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        aurora_i18n::tr!("host-monitor-no-image"),
                        egui::TextStyle::Body.resolve(ui.style()),
                        p.muted,
                    );
                }
                ui.label(caption);
                ui.horizontal(|ui| {
                    for (name, label, intent, enabled) in [
                        (
                            "exchange",
                            aurora_i18n::tr!("host-monitor-exchange"),
                            ticket.map(Command::Exchange),
                            ticket.is_some_and(|ticket| model.can_exchange(ticket)),
                        ),
                        (
                            "chat",
                            aurora_i18n::tr!("host-monitor-chat"),
                            ticket.map(|ticket| Command::Chat(ticket, !state.chat)),
                            ticket.is_some(),
                        ),
                        (
                            "pin",
                            aurora_i18n::tr!("host-monitor-pin"),
                            ticket.map(|ticket| Command::Pin(ticket, !state.pinned)),
                            ticket.is_some(),
                        ),
                    ] {
                        let response = ui
                            .push_id(widget_id(view, binding, name), |ui| {
                                ui.add_enabled_ui(live && enabled, |ui| {
                                    ui.add_sized(
                                        vec2(width, toolbar_height),
                                        egui::Button::new(label)
                                            .wrap()
                                            .selected(match name {
                                                "chat" => state.chat,
                                                "pin" => state.pinned,
                                                _ => false,
                                            })
                                            .min_size(vec2(width, toolbar_height)),
                                    )
                                })
                                .inner
                            })
                            .inner;
                        let response = if name == "pin" {
                            response.on_hover_text(aurora_i18n::tr!("host-monitor-pin-help"))
                        } else {
                            response
                        };
                        #[cfg(test)]
                        ui.ctx().data_mut(|data| data.insert_temp(control_id(view, name), response.rect));
                        self.control(ui, response, view, binding, state, model, intent, &mut command);
                    }
                    let response = ui
                        .push_id(widget_id(view, binding, "options"), |ui| {
                            ui.add_enabled_ui(live, |ui| {
                                ui.add_sized(
                                    vec2(width, toolbar_height),
                                    egui::Button::new(aurora_i18n::tr!("host-monitor-options"))
                                        .wrap()
                                        .selected(self.options_open)
                                        .min_size(vec2(width, toolbar_height)),
                                )
                            })
                            .inner
                        })
                        .inner;
                    #[cfg(test)]
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(control_id(view, "options"), response.rect));
                    self.record_control_focus(response.id);
                    if response.clicked() {
                        self.options_open = !self.options_open;
                        self.gesture = None;
                    }
                });
                if view == View::Cycling {
                    ui.horizontal(|ui| {
                        for (name, label, intent) in [
                            (
                                "previous",
                                aurora_i18n::tr!("host-monitor-previous"),
                                model.capture_navigation(Direction::Previous).map(Command::Navigate),
                            ),
                            (
                                "play",
                                if model.playing() {
                                    aurora_i18n::tr!("host-monitor-pause")
                                } else {
                                    aurora_i18n::tr!("host-monitor-play")
                                },
                                Some(if model.playing() { Command::Pause } else { Command::Play }),
                            ),
                            (
                                "next",
                                aurora_i18n::tr!("host-monitor-next"),
                                model.capture_navigation(Direction::Next).map(Command::Navigate),
                            ),
                        ] {
                            let response = ui
                                .push_id(widget_id(view, binding, name), |ui| {
                                    ui.add_enabled(live && intent.is_some(), egui::Button::new(label))
                                })
                                .inner;
                            #[cfg(test)]
                            ui.ctx().data_mut(|data| data.insert_temp(control_id(view, name), response.rect));
                            self.control(ui, response, view, binding, state, model, intent, &mut command);
                        }
                    });
                    if model.playing() && state.chat {
                        ui.label(aurora_i18n::tr!("host-monitor-pause-chat"));
                    }
                }
                if state.request.is_some() {
                    ui.label(aurora_i18n::tr!("host-monitor-settings-pending"));
                } else if state.failed {
                    ui.label(RichText::new(aurora_i18n::tr!("host-monitor-settings-failed")).color(p.warn));
                }
            });
        let passive = !surface_live || (view == View::Cycling && model.playing()) || command.is_some();
        let mut send = None;
        if state.chat
            && let Some(binding) = binding.filter(|binding| binding.enabled)
        {
            let flushing = matches!(mode, InputMode::Editing | InputMode::Closing);
            if passive && !flushing {
                chat.surface_closed(view.surface());
            }
            ui.add_enabled_ui(!passive || flushing, |ui| {
                send = chat.show_bound_for(
                    view.surface(),
                    binding.source.chat(),
                    ui,
                    p,
                    if flushing {
                        mode
                    } else if passive {
                        InputMode::Navigating
                    } else {
                        mode
                    },
                );
            });
        } else {
            chat.surface_closed(view.surface());
            egui::CentralPanel::default()
                .frame(widgets::overlay_frame(ui.ctx(), p))
                .show(ui, |_| {});
        }
        let read = chat.take_read_action_for(view.surface()); // Never leave a latent UI Read behind a disabled body.
        if self.options_open {
            self.options(ui, p, view, binding, state, model, live, &mut command);
        }
        if ui.ctx().will_discard() {
            return None;
        }
        if mode != InputMode::Live {
            self.painting.clear();
            self.focus_painting.clear();
            return None;
        }
        self.controls = std::mem::take(&mut self.painting);
        self.focus_controls = std::mem::take(&mut self.focus_painting);
        if egui::Popup::is_any_open(ui.ctx()) {
            self.gesture = None;
            command = None;
        }
        if let Some(intent) = command.filter(|intent: &Intent| intent.valid(view, model, state)) {
            self.gesture = None;
            return Some(Action::Control {
                view,
                ticket: intent.view_ticket,
                binding: intent.binding,
                base: intent.state,
                playing: intent.playing,
                interval: intent.interval,
                command: intent.command,
            });
        }
        if self.released(ui) {
            self.gesture = None;
        }
        if !passive && mode == InputMode::Live {
            if let Some(send) = send {
                return Some(Action::Send(send));
            }
            if let Some(read) = read {
                return Some(Action::Read(read));
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn options(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        view: View,
        binding: Option<Binding>,
        state: State,
        model: &MonitorSet,
        live: bool,
        command: &mut Option<Intent>,
    ) {
        let mut close = live && ui.input(|input| input.key_pressed(Key::Escape));
        let width = ui.ctx().content_rect().width().min(320.0) - 16.0;
        let pos = ui.ctx().content_rect().left_top() + vec2(8.0, 38.0);
        egui::Area::new(widget_id(view, binding, "options_area"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .constrain_to(ui.ctx().content_rect())
            .show(ui.ctx(), |ui| {
                widgets::overlay_frame(ui.ctx(), p).show(ui, |ui| {
                    ui.set_width(width.max(1.0));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(aurora_i18n::tr!("host-monitor-options")).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let response = ui.add_enabled(live, egui::Button::new(aurora_i18n::tr!("common-close")));
                            ui.ctx()
                                .data_mut(|data| data.insert_temp(control_id(view, "close_options"), response.rect));
                            self.record_control_focus(response.id);
                            close |= response.clicked();
                        });
                    });
                    ui.label(aurora_i18n::tr!("host-monitor-sampling-help"));
                    let scroll = egui::ScrollArea::vertical()
                        .id_salt("monitor_options_rows")
                        .max_height(160.0)
                        .min_scrolled_height(100.0)
                        .show(ui, |ui| {
                            ui.add_enabled_ui(live && state.request.is_none(), |ui| {
                                for (name, label, desired, selected) in [
                                    (
                                        "small",
                                        aurora_i18n::tr!("host-monitor-small"),
                                        Options {
                                            size: PreviewSize::Small,
                                            ..state.accepted
                                        },
                                        state.accepted.size == PreviewSize::Small,
                                    ),
                                    (
                                        "medium",
                                        aurora_i18n::tr!("host-monitor-medium"),
                                        Options {
                                            size: PreviewSize::Medium,
                                            ..state.accepted
                                        },
                                        state.accepted.size == PreviewSize::Medium,
                                    ),
                                    (
                                        "large",
                                        aurora_i18n::tr!("host-monitor-large"),
                                        Options {
                                            size: PreviewSize::Large,
                                            ..state.accepted
                                        },
                                        state.accepted.size == PreviewSize::Large,
                                    ),
                                    (
                                        "half",
                                        aurora_i18n::tr!("host-monitor-rate-half"),
                                        Options {
                                            rate: PreviewRate::Half,
                                            ..state.accepted
                                        },
                                        state.accepted.rate == PreviewRate::Half,
                                    ),
                                    (
                                        "one",
                                        aurora_i18n::tr!("host-monitor-rate-one"),
                                        Options {
                                            rate: PreviewRate::One,
                                            ..state.accepted
                                        },
                                        state.accepted.rate == PreviewRate::One,
                                    ),
                                    (
                                        "two",
                                        aurora_i18n::tr!("host-monitor-rate-two"),
                                        Options {
                                            rate: PreviewRate::Two,
                                            ..state.accepted
                                        },
                                        state.accepted.rate == PreviewRate::Two,
                                    ),
                                    (
                                        "five",
                                        aurora_i18n::tr!("host-monitor-rate-five"),
                                        Options {
                                            rate: PreviewRate::Five,
                                            ..state.accepted
                                        },
                                        state.accepted.rate == PreviewRate::Five,
                                    ),
                                ] {
                                    let response = ui
                                        .push_id(widget_id(view, binding, name), |ui| {
                                            ui.add_enabled_ui(model.capture(view).is_some(), |ui| widgets::radio(ui, p, selected, label))
                                                .inner
                                        })
                                        .inner;
                                    let intent = model.capture(view).map(|ticket| Command::Settings {
                                        ticket,
                                        base: state,
                                        desired,
                                    });
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|data| data.insert_temp(control_id(view, name), response.rect));
                                    self.control(ui, response, view, binding, state, model, intent, command);
                                }
                                if view == View::Cycling {
                                    ui.label(aurora_i18n::tr!("host-monitor-interval"));
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.interval_text)
                                            .char_limit(4)
                                            .desired_width(60.0),
                                    );
                                    let seconds = self
                                        .interval_text
                                        .parse::<u32>()
                                        .ok()
                                        .filter(|seconds| (2..=3600).contains(seconds));
                                    let response = ui.add_enabled(
                                        seconds.is_some(),
                                        egui::Button::new(aurora_i18n::tr!("host-monitor-apply-interval")),
                                    );
                                    ui.ctx()
                                        .data_mut(|data| data.insert_temp(control_id(view, "apply_interval"), response.rect));
                                    self.control(ui, response, view, binding, state, model, seconds.map(Command::Interval), command);
                                }
                            });
                        });
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(control_id(view, "options_scroll"), scroll.inner_rect));
                });
            });
        if close {
            self.cancel_input();
            *command = None;
        }
    }

    fn record_control_focus(&mut self, id: Id) {
        assert!(self.focus_painting.len() < 24);
        self.focus_painting.push(id);
    }

    #[allow(clippy::too_many_arguments)]
    fn control(
        &mut self,
        ui: &egui::Ui,
        response: egui::Response,
        view: View,
        binding: Option<Binding>,
        state: State,
        model: &MonitorSet,
        command: Option<Command>,
        activated: &mut Option<Intent>,
    ) {
        self.record_control_focus(response.id);
        let Some(binding) = binding else { return };
        let Some(command) = command.filter(|_| response.enabled()) else {
            return;
        };
        let Some(view_ticket) = model.capture_view(view) else { return };
        let intent = Intent {
            view_ticket,
            binding,
            state,
            playing: model.playing(),
            interval: model.interval(),
            command,
        };
        let painted = Painted {
            id: response.id,
            widget: response.id,
            rect: response.interact_rect.intersect(ui.clip_rect()),
            layer: response.layer_id,
            intent,
        };
        assert!(self.painting.len() < 24);
        self.painting.push(painted);
        if let Some(gesture) = self.gesture
            && gesture.control.id == response.id
            && activated.is_none()
        {
            let frame = ui.ctx().cumulative_frame_nr();
            let after = |index| gesture.frame != frame || index > gesture.press_index;
            let released = match gesture.trigger {
                Trigger::Pointer => ui
                    .input(|input| {
                        input.events.iter().enumerate().find_map(|(index, event)| match event {
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed: false,
                                ..
                            } if after(index) => Some(*pos),
                            _ => None,
                        })
                    })
                    .is_some_and(|pos| painted.rect.contains(pos) && ui.ctx().layer_id_at(pos) == Some(painted.layer)),
                Trigger::Key(key) => {
                    response.has_focus()
                        && ui.input(|input| {
                            input.events.iter().enumerate().any(|(index, event)|
                    matches!(event, egui::Event::Key { key: released, pressed: false, .. } if *released == key && after(index)))
                        })
                }
                Trigger::Access => {
                    response.has_focus()
                        && ui.input(|input| input.has_accesskit_action_request(response.id, egui::accesskit::Action::Click))
                }
            };
            if released {
                *activated = Some(gesture.control.intent);
            }
        }
    }
    fn released(&self, ui: &egui::Ui) -> bool {
        let Some(gesture) = self.gesture else { return false };
        if matches!(gesture.trigger, Trigger::Access) {
            return true;
        }
        let frame = ui.ctx().cumulative_frame_nr();
        ui.input(|input| {
            input.events.iter().enumerate().any(|(index, event)| {
                (gesture.frame != frame || index > gesture.press_index)
                    && match (gesture.trigger, event) {
                        (
                            Trigger::Pointer,
                            egui::Event::PointerButton {
                                button: egui::PointerButton::Primary,
                                pressed: false,
                                ..
                            },
                        ) => true,
                        (
                            Trigger::Key(key),
                            egui::Event::Key {
                                key: released,
                                pressed: false,
                                ..
                            },
                        ) => key == *released,
                        _ => false,
                    }
            })
        })
    }
    fn capture(&mut self, ui: &egui::Ui, view: View, model: &MonitorSet, state: State, live: bool) {
        let frame = ui.ctx().cumulative_frame_nr();
        if self.input_frame == Some(frame) {
            return;
        }
        self.input_frame = Some(frame);
        if !live || self.gesture.is_some() {
            return;
        }
        let focus = ui.ctx().memory(|memory| memory.focused());
        // Context::input takes a write lock. Resolve exact pointer layers before
        // entering its closure; copy only press coordinates, never text/IME data.
        let mut pointer_layers = ui.input(|input| {
            input
                .events
                .iter()
                .enumerate()
                .filter_map(|(index, event)| match event {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        ..
                    } => Some((index, *pos, None)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        for (_, pos, layer) in &mut pointer_layers {
            *layer = ui.ctx().layer_id_at(*pos);
        }
        let captured = ui.input(|input| {
            input.events.iter().enumerate().find_map(|(index, event)| match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } => self
                    .controls
                    .iter()
                    .rev()
                    .find(|control| {
                        control.rect.contains(*pos)
                            && pointer_layers
                                .iter()
                                .find(|(event_index, _, _)| *event_index == index)
                                .and_then(|(_, _, layer)| *layer)
                                == Some(control.layer)
                    })
                    .map(|control| (index, Trigger::Pointer, *control)),
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } if matches!(key, Key::Enter | Key::Space) && modifiers.is_none() => self
                    .controls
                    .iter()
                    .find(|control| Some(control.widget) == focus)
                    .map(|control| (index, Trigger::Key(*key), *control)),
                egui::Event::AccessKitActionRequest(_) => self
                    .controls
                    .iter()
                    .find(|control| {
                        Some(control.widget) == focus && input.has_accesskit_action_request(control.widget, egui::accesskit::Action::Click)
                    })
                    .map(|control| (index, Trigger::Access, *control)),
                _ => None,
            })
        });
        if let Some((press_index, trigger, control)) = captured.filter(|(_, _, control)| control.intent.valid(view, model, state)) {
            self.gesture = Some(Gesture {
                control,
                trigger,
                frame,
                press_index,
            });
        }
    }
}

fn wrapped_height(ui: &egui::Ui, text: &str, width: f32, style: egui::TextStyle) -> f32 {
    ui.painter()
        .layout(text.to_owned(), style.resolve(ui.style()), ui.visuals().text_color(), width)
        .size()
        .y
}

fn widget_id(view: View, binding: Option<Binding>, name: &str) -> Id {
    Id::unique((
        "monitor",
        view.surface().index(),
        binding.map(|binding| (chat_ui::source_id(binding.source.chat()), binding.incarnation)),
        name,
    ))
}

fn control_id(view: View, name: &str) -> Id {
    Id::unique(("monitor_control", view.surface().index(), name))
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod visual;
