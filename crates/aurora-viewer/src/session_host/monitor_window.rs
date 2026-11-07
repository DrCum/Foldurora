//! Actual Windows owned monitor surface. The manager and canonical SharedChat
//! outlive this disposable native/context owner. No transport, pixels or raw
//! Window handle escapes the sealed private-image facade.

use super::chat_ui::{self, InputMode};
use super::chat_window::{edit_input, edit_prefix};
use super::monitor_edits::close as close_edits;
use super::monitor_input::MonitorKeys;
use super::monitor_manager::MonitorManager;
use super::monitor_model::{Binding, View};
use super::monitor_ui::{Action, Image, MonitorUi};
use super::preview_projection::{PreparedImage, PreviewRead, PreviewReader, ViewNamespace};
use super::private_image::{MonitorSurface, Placement, ToolOptions};
use super::shared_chat::SharedChat;
use super::transport::HostSnapshot;
use crate::theme::Theme;
use crate::ui::fonts;
use std::cell::RefCell;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

const MIN_FRAME: Duration = Duration::from_millis(33);

#[derive(Default)]
pub(super) struct Outcome {
    pub close: bool,
    pub minimized: bool,
    pub action: Option<Action>,
    pub failed: bool,
    pub recreate: bool,
    pub observed_pin: Option<bool>,
    pub placement: Option<Placement>,
}
pub(super) struct MonitorWindow {
    view: View,
    binding: Binding,
    namespace: ViewNamespace,
    surface: MonitorSurface,
    context: egui::Context,
    theme: Theme,
    body: MonitorUi,
    generation: u64,
    staged: Option<Arc<PreparedImage>>,
    // The OS may still show this pair while a newer slot image is pending.
    // Keep its exact admission/guard until the original new frame is presented.
    previous: Option<Arc<PreparedImage>>,
    keys: MonitorKeys,
    focused: bool,
    blocked: bool,
    focus_requested: bool,
    pin_on_show: Option<bool>,
    observed_pin: Option<bool>,
    input_pending: bool,
    title: String,
    restore_placement: Option<Placement>,
    repaint: Option<Instant>,
    last_frame: Option<Instant>,
}
impl MonitorWindow {
    pub fn new(
        event_loop: &ActiveEventLoop,
        owner: &Window,
        manager: &MonitorManager,
        reader: &PreviewReader,
        read: &PreviewRead,
        host: &HostSnapshot,
        placement: Option<Placement>,
    ) -> anyhow::Result<Self> {
        let view = read.view();
        anyhow::ensure!(manager.current_read(view, read), "monitor view is not current");
        let binding = manager
            .model()
            .binding(view)
            .ok_or_else(|| anyhow::anyhow!("monitor binding is absent"))?;
        let context = egui::Context::default();
        let theme = Theme {
            panel_alpha: 255,
            ..Default::default()
        };
        theme.apply(&context, 1.0);
        fonts::apply(&context, fonts::DEFAULT);
        let title = title(view, binding, host);
        let surface = MonitorSurface::owned_tool(
            event_loop,
            &context,
            owner,
            ToolOptions {
                title: title.clone(),
                initial: if view == View::Cycling { [380.0, 452.0] } else { [380.0, 420.0] },
                minimum: if view == View::Cycling { [380.0, 452.0] } else { [320.0, 392.0] },
                background: theme.palette().panel,
            },
            read.namespace().get(),
            reader.ui_budget(),
        )?;
        Ok(Self {
            view,
            binding,
            namespace: read.namespace(),
            surface,
            body: MonitorUi::new(&context, 0, read.namespace().get()),
            generation: 0,
            context,
            theme,
            staged: None,
            previous: None,
            keys: Default::default(),
            focused: false,
            blocked: false,
            focus_requested: false,
            pin_on_show: Some(manager.desired_pin(view)),
            observed_pin: None,
            input_pending: false,
            title,
            restore_placement: placement,
            repaint: Some(Instant::now()),
            last_frame: None,
        })
    }
    pub fn id(&self) -> WindowId {
        self.surface.id()
    }
    pub fn view(&self) -> View {
        self.view
    }
    pub fn focused(&self) -> bool {
        self.focused
    }
    pub fn is_live(&self) -> bool {
        self.surface.is_live()
    }
    pub fn timer_busy(&self) -> bool {
        self.input_pending || self.body.timer_busy()
    }
    pub fn matches(&self, manager: &MonitorManager) -> bool {
        manager.namespace(self.view) == Some(self.namespace)
            && manager.model().binding(self.view).is_some_and(|binding| {
                binding.source == self.binding.source
                    && binding.incarnation == self.binding.incarnation
                    && binding.enabled
                    && binding.visible
            })
    }
    /// Polls revoke old visible image output even when there is no redraw or
    /// native input. A current view may then recreate an image-free owner.
    pub fn retain(&mut self, manager: &MonitorManager, read: &PreviewRead, chat: &mut SharedChat) -> bool {
        if self.matches(manager) && manager.current_read(self.view, read) && self.refresh_image(read) {
            return true;
        }
        self.retire(chat);
        false
    }

    pub fn flush_edits(&mut self, event_loop: &ActiveEventLoop, chat: &mut SharedChat) {
        let view = self.view;
        let binding = self.binding;
        let palette = self.theme.palette();
        let shared = RefCell::new(chat);
        let mut retire = |ctx: &egui::Context, raw| close_edits(ctx, raw, view, binding, &palette, &mut shared.borrow_mut());
        let Ok(mut raw) = self.surface.raw_input(event_loop, &mut retire) else {
            return;
        };
        self.body.observe_ime(&raw.events);
        let composer = shared.borrow().composer_rect_for(view.surface(), &self.context);
        if let Some(prefix) = edit_prefix(&mut raw, composer, egui::Popup::is_any_open(&self.context)) {
            let _ = self.surface.draw(
                prefix,
                |ui, _| {
                    let _ = shared
                        .borrow_mut()
                        .show_bound_for(view.surface(), binding.source.chat(), ui, &palette, InputMode::Closing);
                },
                &mut retire,
            );
        }
        let _ = self.surface.draw(
            edit_input(raw),
            |ui, _| {
                let _ = shared
                    .borrow_mut()
                    .show_bound_for(view.surface(), binding.source.chat(), ui, &palette, InputMode::Closing);
            },
            &mut retire,
        );
    }
    pub fn retire(&mut self, chat: &mut SharedChat) {
        let view = self.view;
        let binding = self.binding;
        let palette = self.theme.palette();
        self.body.cancel_input();
        self.surface
            .retire(&mut |ctx, raw| close_edits(ctx, raw, view, binding, &palette, chat));
        chat.surface_closed(view.surface());
        self.staged = None;
        self.previous = None;
        self.focused = false;
    }
    pub fn request_focus(&mut self) {
        self.focus_requested = true;
        self.repaint = Some(Instant::now());
    }
    pub fn cancel_focus(&mut self, chat: &mut SharedChat) {
        self.body.cancel_input();
        chat.surface_closed(self.view.surface());
        self.context.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
        chat_ui::cancel_input(&self.context);
    }
    pub fn minimized(&mut self, event_loop: &ActiveEventLoop, chat: &mut SharedChat) -> Option<bool> {
        let view = self.view;
        let binding = self.binding;
        let palette = self.theme.palette();
        self.surface
            .minimized(event_loop, &mut |ctx, raw| close_edits(ctx, raw, view, binding, &palette, chat))
            .ok()
    }
    pub fn pin(&mut self, event_loop: &ActiveEventLoop, desired: bool, chat: &mut SharedChat) -> Option<bool> {
        let view = self.view;
        let binding = self.binding;
        let palette = self.theme.palette();
        self.surface
            .pin(event_loop, desired, &mut |ctx, raw| {
                close_edits(ctx, raw, view, binding, &palette, chat)
            })
            .ok()
    }

    /// The parent obtains a fresh read before EVERY event and validates against
    /// the latest manager. Final presentation rechecks the SAME read, not a new
    /// guard wrapped around already-built private output.
    #[allow(clippy::too_many_arguments)]
    pub fn event(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: &WindowEvent,
        manager: &MonitorManager,
        read: &PreviewRead,
        chat: &mut SharedChat,
        host: &HostSnapshot,
        blocked: bool,
    ) -> Outcome {
        let mut outcome = Outcome::default();
        if !self.matches(manager) || !manager.current_read(self.view, read) {
            self.retire(chat);
            outcome.recreate = true;
            return outcome;
        }
        // Image-only revocation is processed before forwarding any native input;
        // a fresh image-free owner can keep independently permitted Chat usable.
        if !self.refresh_image(read) {
            self.retire(chat);
            outcome.recreate = true;
            return outcome;
        }
        if matches!(event, WindowEvent::CloseRequested | WindowEvent::Destroyed) {
            self.retire(chat);
            outcome.close = true;
            return outcome;
        }
        if matches!(event, WindowEvent::Focused(false)) || self.blocked != blocked {
            self.flush_edits(event_loop, chat);
            self.body.cancel_input();
            chat.surface_closed(self.view.surface());
            chat_ui::cancel_input(&self.context);
        }
        self.blocked = blocked;
        match event {
            WindowEvent::Focused(focused) => {
                self.keys.focused(*focused);
                self.focused = *focused;
            }
            WindowEvent::ModifiersChanged(modifiers) => self.keys.modifiers(modifiers.state()),
            _ => {}
        }
        if let WindowEvent::KeyboardInput { event, is_synthetic, .. } = event
            && self.keys.suppress(
                &event.logical_key,
                event.state,
                event.repeat,
                *is_synthetic,
                self.body.command_focused(&self.context),
            )
        {
            return outcome;
        }
        if matches!(
            event,
            WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::KeyboardInput { .. }
                | WindowEvent::Ime(_)
                | WindowEvent::Touch(_)
        ) {
            self.input_pending = true;
        }
        let view = self.view;
        let binding = self.binding;
        let palette = self.theme.palette();
        let shared = RefCell::new(chat);
        let mut flush = |ctx: &egui::Context, raw| close_edits(ctx, raw, view, binding, &palette, &mut shared.borrow_mut());
        let title = title(view, binding, host);
        if title != self.title {
            if self.surface.set_title(event_loop, &title, &mut flush).is_err() {
                outcome.failed = true;
                return outcome;
            }
            self.title = title;
        }
        let id = self.surface.id();
        match self.surface.event(event_loop, id, event, &mut flush) {
            Ok(Some(response)) if response.repaint => self.repaint = Some(Instant::now()),
            Ok(_) => {}
            Err(_) => {
                outcome.failed = true;
                return outcome;
            }
        }
        match self.surface.minimized(event_loop, &mut flush) {
            Ok(true) => {
                self.surface.retire(&mut flush);
                self.staged = None;
                outcome.minimized = true;
                return outcome;
            }
            Ok(false) => {}
            Err(_) => {
                outcome.failed = true;
                return outcome;
            }
        }
        if self.last_frame.is_some()
            && matches!(
                event,
                WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
            )
        {
            match self.surface.placement(event_loop, &mut flush) {
                Ok(placement) => outcome.placement = Some(placement),
                Err(_) => {
                    outcome.failed = true;
                    return outcome;
                }
            }
        }
        let chat = shared.into_inner();
        self.binding = manager.model().binding(view).unwrap_or(binding);
        self.body.sync(view, manager.model(), manager.state(view));
        if matches!(event, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }) {
            self.repaint = Some(Instant::now());
        }
        if matches!(event, WindowEvent::RedrawRequested) {
            if self.last_frame.is_some_and(|last| last.elapsed() < MIN_FRAME) {
                self.repaint = Some(Instant::now());
            } else {
                // Capture is returned even if presentation subsequently fails.
                // Shell will refresh/revalidate before queueing the command.
                let (action, failed) = self.render(event_loop, manager, read, chat, host, blocked);
                outcome.action = action;
                let current = read.view_current() && self.staged.as_ref().is_none_or(|image| image.current());
                outcome.failed = failed && current;
                outcome.recreate = failed && !current;
                outcome.observed_pin = self.observed_pin.take();
            }
        }
        outcome
    }

    fn refresh_image(&mut self, read: &PreviewRead) -> bool {
        if let Some(previous) = &self.previous {
            let Some(retained) = read.refresh_prepared(previous) else {
                return false;
            };
            if retained.revision() != previous.revision() || retained.admitted().stamp() != previous.admitted().stamp() {
                return false;
            }
            self.previous = Some(retained);
        }
        if let Some(old) = &self.staged {
            let Some(retained) = read.refresh_prepared(old) else { return false };
            if retained.revision() != old.revision() || retained.admitted().stamp() != old.admitted().stamp() {
                return false;
            }
            self.staged = Some(retained);
        }
        if let Some(next) = read.prepared() {
            if let Some(old) = &self.staged
                && old.revision() == next.revision()
            {
                if old.admitted().stamp() != next.admitted().stamp() {
                    return false;
                }
                // Only refresh the guard of the exact immutable pixel pair.
                self.staged = Some(next.clone());
                return true;
            }
            if self.surface.stage(self.namespace.get(), next.image()).is_err() {
                return false;
            }
            if self.previous.is_none() {
                self.previous = self.staged.clone();
            }
            self.staged = Some(next.clone());
        }
        self.staged.as_ref().is_none_or(|image| image.current())
    }
    fn render(
        &mut self,
        event_loop: &ActiveEventLoop,
        manager: &MonitorManager,
        read: &PreviewRead,
        chat: &mut SharedChat,
        host: &HostSnapshot,
        blocked: bool,
    ) -> (Option<Action>, bool) {
        if !self.refresh_image(read) {
            self.retire(chat);
            return (None, true);
        }
        let view = self.view;
        let binding = self.binding;
        let state = manager.state(view);
        let palette = self.theme.palette();
        let context = self.context.clone();
        let shared = RefCell::new(chat);
        let mut flush = |ctx: &egui::Context, raw| close_edits(ctx, raw, view, binding, &palette, &mut shared.borrow_mut());
        let Ok(mut raw) = self.surface.raw_input(event_loop, &mut flush) else {
            return (None, true);
        };
        self.input_pending = false;
        let mut suppress_actions = blocked;
        let composer = shared.borrow().composer_rect_for(view.surface(), &context);
        if let Some(prefix) = edit_prefix(&mut raw, composer, egui::Popup::is_any_open(&context)) {
            suppress_actions |= self.body.observe_ime(&prefix.events);
            if self
                .surface
                .draw(
                    prefix,
                    |ui, _| {
                        let _ = shared
                            .borrow_mut()
                            .show_bound_for(view.surface(), binding.source.chat(), ui, &palette, InputMode::Editing);
                    },
                    &mut flush,
                )
                .is_err()
            {
                return (None, true);
            }
        }
        let staged = self.staged.as_ref();
        let body = &mut self.body;
        let generation = &mut self.generation;
        let namespace = self.namespace;
        let mut action = None;
        let frame = self.surface.draw(
            raw,
            |ui, texture| {
                if *generation != texture.generation() {
                    *generation = texture.generation();
                    body.reset_surface(&context, *generation, namespace.get());
                }
                let shown = body.show(
                    ui,
                    &palette,
                    view,
                    manager.model(),
                    &mut shared.borrow_mut(),
                    host,
                    state,
                    staged.map(|image| Image {
                        binding,
                        admitted: image.admitted(),
                        texture,
                    }),
                    read.now(),
                    if suppress_actions { InputMode::Navigating } else { InputMode::Live },
                );
                if !ui.ctx().will_discard() {
                    action = action.take().or(shown);
                }
            },
            &mut flush,
        );
        let Ok(frame) = frame else { return (action, true) };
        let previous = self.previous.as_ref();
        let mut current = || {
            manager.current_read(view, read)
                && staged.is_none_or(|image| image.current() && image.admitted().current(read.now()))
                && previous.is_none_or(|image| image.current() && image.admitted().current(read.now()))
        };
        let result = self.surface.present(event_loop, &context, frame, &mut current, &mut flush);
        if let Ok(delay) = result {
            self.previous = None;
            self.last_frame = Some(Instant::now());
            self.repaint = Instant::now().checked_add(delay);
            if let Some(placement) = self.restore_placement.take()
                && (!read.view_current() || self.surface.request_placement(event_loop, placement, &mut flush).is_err())
            {
                return (action, true);
            }
            if let Some(desired) = self.pin_on_show.take() {
                match self.surface.pin(event_loop, desired, &mut flush) {
                    Ok(actual) => self.observed_pin = Some(actual),
                    Err(_) => return (action, true),
                }
            }
            if self.focus_requested && read.view_current() {
                self.focus_requested = false;
                if self.surface.focus(event_loop, &mut flush).is_err() {
                    return (action, true);
                }
            }
            (action, false)
        } else {
            (action, true)
        }
    }
    pub fn poll(&mut self, event_loop: &ActiveEventLoop, changed: bool, now: Instant, chat: &mut SharedChat) -> Option<Instant> {
        if changed {
            self.repaint = Some(now);
        }
        let due = self.repaint.map(|at| self.last_frame.map_or(at, |last| at.max(last + MIN_FRAME)))?;
        if now < due {
            return Some(due);
        }
        self.repaint = None;
        let view = self.view;
        let binding = self.binding;
        let palette = self.theme.palette();
        let _ = self
            .surface
            .request_redraw(event_loop, &mut |ctx, raw| close_edits(ctx, raw, view, binding, &palette, chat));
        None
    }
}

fn title(view: View, binding: Binding, host: &HostSnapshot) -> String {
    let profile = host
        .slots
        .iter()
        .find(|slot| slot.id == binding.source.slot())
        .map(|slot| slot.label.as_str())
        .unwrap_or_default();
    format!(
        "{} — {}",
        aurora_i18n::tr!(
            "host-monitor-window-title",
            number = super::monitor_manager::index(view) as i64 + 1,
            profile = profile
        ),
        crate::cli::window_title()
    )
}
