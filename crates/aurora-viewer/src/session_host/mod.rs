//! Opt-in native session controller. Worlds, logins and GPU ownership remain
//! in separately isolated workers; the host paints only its CPU interface.

mod attention;
mod attention_prefs;
mod attention_ui;
mod attention_window;
pub(crate) mod chat;
pub(crate) mod chat_broker;
pub(crate) mod chat_io;
pub(crate) mod chat_transport;
mod chat_ui;
mod chat_window;
// Pure monitor bindings only; no production surface or image service enabled.
#[cfg(test)]
mod monitor_model;
mod native;
mod preview_bootstrap;
pub(crate) mod profile;
mod review;
// Unused prerequisite only; native IO/bootstrap/authority integration is absent.
#[cfg(test)]
mod preview_transport;
// Dormant CPU private-image lifetime prerequisite; no native monitor caller.
#[cfg(test)]
mod private_image;
mod shared_chat;
pub(crate) mod slots;
mod software;
pub(crate) mod startup;
pub(crate) mod transport;
mod ui;
pub(crate) mod worker;

use crate::theme::Theme;
use crate::ui::{fonts, icons::Icons};
use software::SoftwareWindow;
use std::sync::Arc;
use std::time::{Duration, Instant};
use transport::{HostController, HostSnapshot};
use ui::{Action, HostUi, Notice};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

const POLL: Duration = Duration::from_millis(100);
const MIN_FRAME: Duration = Duration::from_millis(33);
const CLOSE_GRACE: Duration = Duration::from_secs(3);

pub fn run() -> anyhow::Result<()> {
    aurora_i18n::select("default", crate::cli::get().language.as_deref());
    let event_loop = EventLoop::new()?;
    let context = egui::Context::default();
    // The software shell has no world behind its surfaces to blur.
    let theme = Theme {
        panel_alpha: 255,
        ..Default::default()
    };
    theme.apply(&context, 1.0);
    // This CPU-owned HWND cannot alpha-blend a shadow over the separate worker
    // window. Opaque framed popup islands preserve the surrounding live world.
    context.all_styles_mut(|style| style.visuals.popup_shadow = egui::epaint::Shadow::NONE);
    fonts::fallback::wait();
    fonts::apply(&context, fonts::DEFAULT);
    let icons = Icons::load(&context, None);
    let controller = HostController::start()?;
    let snapshot = controller.snapshot();
    let now = Instant::now();
    let mut app = Shell {
        context,
        theme,
        _icons: icons,
        controller,
        snapshot,
        ui: HostUi::default(),
        surface: None,
        chat_window: None,
        shared_chat: shared_chat::SharedChat::default(),
        chat_changed: true,
        attention: attention::Attention::default(),
        attention_ui: attention_ui::AttentionUi::default(),
        attention_window: None,
        attention_changed: true,
        error: None,
        next_poll: now,
        repaint: Some(now),
        last_frame: None,
        closing: None,
        notice_at: None,
        world: egui::Rect::NOTHING,
        region: native::Region::default(),
        overlays: native::Overlays::default(),
        transition: PresentationGate::default(),
        requested_viewport: None,
        park_requested: false,
        layout_dirty: true,
        geometry_dirty: false,
    };
    log::info!("session host: CPU controller starting; worker worlds remain independent");
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.error {
        anyhow::bail!(error);
    }
    Ok(())
}

struct Shell {
    context: egui::Context,
    theme: Theme,
    // Retain the embedded icon textures for the lifetime of the CPU painter.
    _icons: Icons,
    controller: HostController,
    snapshot: Arc<HostSnapshot>,
    ui: HostUi,
    surface: Option<SoftwareWindow>,
    chat_window: Option<chat_window::ChatWindow>,
    shared_chat: shared_chat::SharedChat,
    chat_changed: bool,
    attention: attention::Attention,
    attention_ui: attention_ui::AttentionUi,
    attention_window: Option<attention_window::AttentionWindow>,
    attention_changed: bool,
    error: Option<String>,
    next_poll: Instant,
    repaint: Option<Instant>,
    last_frame: Option<Instant>,
    closing: Option<Instant>,
    notice_at: Option<Instant>,
    world: egui::Rect,
    region: native::Region,
    overlays: native::Overlays,
    transition: PresentationGate,
    requested_viewport: Option<aurora_session::HostedViewport>,
    park_requested: bool,
    layout_dirty: bool,
    geometry_dirty: bool,
}

impl Shell {
    fn request_repaint(&mut self) {
        self.repaint = Some(Instant::now());
    }

    fn action(&mut self, event_loop: &ActiveEventLoop, action: Action, viewport: Option<aurora_session::HostedViewport>) {
        if self.closing.is_some() {
            return;
        }
        let result = match action.clone() {
            Action::OpenChat(source) => {
                self.open_chat(event_loop, source);
                return;
            }
            Action::OpenAttention => {
                self.open_attention(event_loop);
                return;
            }
            Action::OpenLogin(account, slot) => self.controller.open_manual_login(account, slot),
            Action::RenameSlot(slot, label) => self.controller.rename_slot(slot, label),
            Action::Activate(worker) => match viewport {
                Some(viewport) => self.controller.activate(worker, viewport),
                None => Err(transport::HostError::Unsupported),
            },
            Action::Detach(worker) => self.controller.detach(worker),
            Action::Close => {
                self.cancel_attention_review();
                self.chat_window = None;
                self.shared_chat.clear();
                self.attention_window = None;
                self.attention_ui.cancel_gestures();
                self.attention.clear();
                self.close_region();
                self.controller.shutdown();
                self.closing = Some(Instant::now());
                self.request_repaint();
                return;
            }
        };
        if let Ok(request) = result {
            let affects_active = match action {
                Action::OpenLogin(_, _) | Action::Activate(_) => true,
                // The broker's pending row can name the old renderer while
                // the requested target is also a transaction participant.
                Action::Detach(worker) => self.snapshot.active == Some(worker) || self.snapshot.pending.is_some(),
                Action::OpenChat(_) | Action::OpenAttention | Action::RenameSlot(_, _) | Action::Close => false,
            };
            if affects_active {
                // The IO thread has not seen the intent yet. Do not reopen
                // the old hole against a pre-intent snapshot in that interval.
                self.transition.sent(request);
                self.close_region();
            }
        }
        self.ui.notice = Some(match result {
            Ok(_) => Notice::Queued,
            Err(error) => Notice::Error(error),
        });
        self.notice_at = Some(Instant::now());
        self.request_repaint();
    }

    fn open_chat(&mut self, event_loop: &ActiveEventLoop, source: aurora_session::chat::ChatSource) {
        self.shared_chat.sync(&self.controller.snapshot(), self.closing.is_some());
        if let Some(window) = &mut self.chat_window {
            // Native text can be queued after the preceding redraw. Keep it
            // with the old sender before this explicit navigation changes it.
            window.flush_edits(&self.controller, &mut self.shared_chat);
            self.shared_chat.select(source);
            self.shared_chat.cancel_gestures();
            window.focus();
            return;
        }
        self.shared_chat.select(source);
        let result = (|| {
            let owner = self.surface.as_ref().ok_or_else(|| anyhow::anyhow!("host window is unavailable"))?;
            chat_window::ChatWindow::new(event_loop, &owner.window)
        })();
        match result {
            Ok(window) => self.chat_window = Some(window),
            Err(error) => {
                log::warn!("session host: could not open owned chat window: {error}");
                self.ui.notice = Some(Notice::Error(transport::HostError::Unavailable));
                self.notice_at = Some(Instant::now());
                self.request_repaint();
            }
        }
    }

    fn sync_attention(&mut self) {
        if self.closing.is_some() || self.snapshot.stopped {
            self.attention.clear();
        } else {
            // Pending/invalid loads carry disabled preferences. A transient save
            // error retains the last published filters and must remain usable.
            self.attention
                .sync(&self.snapshot, &self.snapshot.attention_preferences.preferences);
        }
        self.attention_ui.sync(&self.attention, &self.snapshot.attention_preferences);
        if self.attention_ui.sync_review(&self.snapshot) {
            self.attention_changed = true;
            self.request_repaint();
        }
        self.ui.attention_count = self.attention.rows().len();
    }

    fn refresh_attention(&mut self) {
        let snapshot = self.controller.snapshot();
        let changed = !ui::same_snapshot(&snapshot, &self.snapshot);
        self.snapshot = snapshot;
        self.chat_changed |= self.shared_chat.sync(&self.snapshot, self.closing.is_some());
        self.sync_attention();
        self.attention_changed |= changed;
        if changed {
            self.request_repaint();
        }
    }

    fn open_attention(&mut self, event_loop: &ActiveEventLoop) {
        self.refresh_attention();
        if let Some(window) = &mut self.attention_window {
            window.focus(&mut self.attention_ui);
            return;
        }
        self.attention_ui.cancel_gestures();
        let result = self
            .surface
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("host window is unavailable"))
            .and_then(|owner| attention_window::AttentionWindow::new(event_loop, &owner.window));
        match result {
            Ok(window) => self.attention_window = Some(window),
            Err(error) => {
                log::warn!("session host: could not open owned Attention window: {error}");
                self.ui.notice = Some(Notice::Error(transport::HostError::Unavailable));
                self.notice_at = Some(Instant::now());
                self.request_repaint();
            }
        }
    }

    fn cancel_attention_review(&mut self) {
        if let Some(request) = self.attention_ui.review_to_cancel() {
            let queued = self.controller.cancel_review(request).is_ok();
            self.attention_ui.review_cancelled(request, queued);
            self.attention_changed = true;
        }
    }

    fn attention_action(&mut self, event_loop: &ActiveEventLoop, action: attention_ui::Action) {
        self.refresh_attention();
        if self.closing.is_some() || self.snapshot.stopped || self.ui.modal_open() {
            self.attention_ui.surface_failed();
            self.attention_changed = true;
            self.request_repaint();
            return;
        }
        match action {
            attention_ui::Action::Select(ticket) => {
                if self.attention.validate(ticket) {
                    self.cancel_attention_review();
                    self.attention.select(ticket.key());
                }
            }
            attention_ui::Action::Review(ticket) => {
                let target = self.attention.review_capture(ticket);
                let viewport = self.surface.as_ref().and_then(|surface| {
                    review_viewport(
                        &self.snapshot,
                        surface.viewport(self.world, self.context.pixels_per_point()),
                        surface.window.is_visible() == Some(true) && surface.window.is_minimized() == Some(false),
                        self.layout_dirty || self.geometry_dirty || self.transition.waiting() || self.attention_ui.review_busy(),
                    )
                });
                if let (Some(target), Some(viewport)) = (target, viewport) {
                    match self.controller.review_conversation(target, viewport) {
                        Ok(request) => {
                            self.attention_ui.review_queued(request, target);
                            // Admission is not an activation acknowledgment.
                            // Cover immediately before an old snapshot can paint.
                            self.transition.sent(request);
                            self.close_region();
                        }
                        Err(_) => self.attention_ui.review_failed(),
                    }
                } else {
                    self.attention_ui.review_failed();
                }
            }
            attention_ui::Action::CancelReview(request) => {
                if self.attention_ui.review_to_cancel() == Some(request) {
                    self.cancel_attention_review();
                }
            }
            attention_ui::Action::OpenChat(ticket) => {
                if !self.attention.validate(ticket) {
                    return;
                }
                self.cancel_attention_review();
                // Drain edits already delivered to the old composer before the
                // explicit source/conversation change; this pass cannot send.
                if let Some(window) = &mut self.chat_window {
                    window.flush_edits(&self.controller, &mut self.shared_chat);
                }
                self.refresh_attention();
                if let Some(key) = self.attention.open_chat(ticket)
                    && self.shared_chat.select_key(key)
                {
                    self.open_chat(event_loop, key.source);
                }
            }
            attention_ui::Action::Dismiss(ticket) => {
                if self.attention.validate(ticket) {
                    self.cancel_attention_review();
                }
                if self.attention.dismiss(ticket) {
                    self.ui.attention_count = self.attention.rows().len();
                }
            }
            attention_ui::Action::Preferences(preferences) => match self.controller.save_attention_preferences(preferences) {
                Ok(ticket) => self.attention_ui.queued(ticket, preferences),
                Err(_) => self.attention_ui.queue_failed(),
            },
        }
        self.ui.attention_count = self.attention.rows().len();
        self.attention_changed = true;
        self.request_repaint();
    }

    fn render(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let Some(surface) = &self.surface else { return Ok(()) };
        let size = surface.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        let native_viewport = (!self.layout_dirty)
            .then(|| surface.viewport(self.world, self.context.pixels_per_point()))
            .flatten();
        let hosted = self.confirmed_active(native_viewport);
        let native_ready = native_viewport.is_some();
        let Some(surface) = &mut self.surface else { return Ok(()) };
        let raw = surface.raw_input();
        let context = self.context.clone();
        let p = self.theme.palette();
        let mut action = None;
        let mut world = self.world;
        let mut overlays = native::Overlays::default();
        let output = context.run_ui(raw, |ui| {
            let shown = self.ui.show(ui, &p, &self.snapshot, native_ready, hosted, self.closing.is_some());
            // A discarded egui pass must not duplicate a queued action.
            action = action.take().or(shown.action);
            world = shown.world;
            overlays = native::Overlays::collect(ui.ctx(), world);
        });
        self.world = world;
        self.overlays = overlays;
        self.layout_dirty = false;
        let viewport = surface.viewport(world, context.pixels_per_point());
        let delay = surface.present(event_loop, &context, output, p.panel)?;
        let now = Instant::now();
        self.last_frame = Some(now);
        self.repaint = now.checked_add(delay);
        if let Some(action) = action {
            self.action(event_loop, action, viewport);
        }
        self.sync_native()?;
        Ok(())
    }

    fn poll(&mut self, now: Instant) {
        if now < self.next_poll {
            return;
        }
        self.next_poll = now + POLL;
        let snapshot = self.controller.snapshot();
        let changed = !ui::same_snapshot(&snapshot, &self.snapshot);
        if changed {
            self.repaint = Some(now);
        }
        self.snapshot = snapshot;
        self.chat_changed |= self.shared_chat.sync(&self.snapshot, self.closing.is_some());
        self.sync_attention();
        self.attention_changed |= changed;
        if self
            .notice_at
            .is_some_and(|at| now.saturating_duration_since(at) >= Duration::from_secs(4))
        {
            self.ui.notice = None;
            self.notice_at = None;
            self.repaint = Some(now);
        }
    }

    fn confirmed_active(&self, viewport: Option<aurora_session::HostedViewport>) -> Option<aurora_session::WorkerId> {
        presented_worker(
            &self.snapshot,
            viewport,
            self.closing.is_some() || self.ui.modal_open() || self.transition.waiting() || self.geometry_dirty,
        )
    }

    fn close_region(&mut self) {
        if let Some(surface) = &self.surface
            && let Err(error) = self.region.close(&surface.window)
        {
            // Destroying this owned window during shutdown also removes its
            // region. Do not keep an inaccessible cutout after a native error.
            log::warn!("session host: failed to restore controller region: {error}");
            self.controller.shutdown();
            self.closing.get_or_insert_with(Instant::now);
        }
    }

    fn sync_native(&mut self) -> anyhow::Result<()> {
        self.transition.observe(self.snapshot.completed_request);
        let Some(surface) = &self.surface else { return Ok(()) };
        let worker_focused = self.snapshot.active.is_some_and(|id| {
            self.snapshot.workers.iter().any(|worker| {
                worker.binding.worker.worker == id
                    && worker.reachable
                    && worker.binding.session.is_some()
                    && worker.status.as_ref().is_some_and(|status| status.keyboard_focused)
            })
        });
        self.controller.set_workspace_foreground(
            surface.window.has_focus()
                || worker_focused
                || self.chat_window.as_ref().is_some_and(|window| window.focused())
                || self.attention_window.as_ref().is_some_and(|window| window.focused()),
        );
        let viewport = surface.viewport(self.world, self.context.pixels_per_point());
        let visible = surface.window.is_visible() == Some(true) && surface.window.is_minimized() == Some(false);
        // A native resize is closed immediately, then waits for one real egui
        // layout before submitting the new world rectangle. Minimizing cannot
        // wait for that frame, since no surface will be presented while hidden.
        if self.layout_dirty && visible {
            self.region.close(&surface.window)?;
            return Ok(());
        }
        let confirmed = self.confirmed_active(viewport);
        self.region.set(
            &surface.window,
            confirmed.and(viewport),
            self.world,
            self.context.pixels_per_point(),
            &self.overlays,
        )?;
        if self.closing.is_some()
            || self.snapshot.stopped
            || self.snapshot.error.is_some()
            || (!self.geometry_dirty && (self.snapshot.pending.is_some() || self.transition.waiting()))
        {
            return Ok(());
        }
        if self.snapshot.active.is_none() && self.snapshot.pending.is_none() {
            self.geometry_dirty = false;
            self.park_requested = false;
            self.requested_viewport = None;
            return Ok(());
        }
        match viewport {
            Some(viewport) => {
                self.park_requested = false;
                if (self.geometry_dirty || self.snapshot.active_viewport != Some(viewport))
                    && self.requested_viewport != Some(viewport)
                    && let Ok(request) = self.controller.update_viewport(viewport)
                {
                    self.geometry_dirty = false;
                    self.requested_viewport = Some(viewport);
                    self.transition.sent(request);
                }
            }
            None => {
                self.requested_viewport = None;
                if !self.park_requested
                    && let Ok(request) = self.controller.park_active()
                {
                    self.park_requested = true;
                    self.transition.sent(request);
                }
            }
        }
        Ok(())
    }
}

/// Admission needs current native geometry, not the old acknowledged hole.
/// The broker independently verifies the captured source and exact new grant.
fn review_viewport(
    snapshot: &HostSnapshot,
    viewport: Option<aurora_session::HostedViewport>,
    visible: bool,
    blocked: bool,
) -> Option<aurora_session::HostedViewport> {
    if blocked || !visible || snapshot.stopped || !snapshot.can_activate || snapshot.pending.is_some() || snapshot.error.is_some() {
        return None;
    }
    viewport.filter(|viewport| viewport.validate().is_ok())
}

fn presented_worker(
    snapshot: &HostSnapshot,
    viewport: Option<aurora_session::HostedViewport>,
    covered: bool,
) -> Option<aurora_session::WorkerId> {
    if covered
        || snapshot.stopped
        || snapshot.error.is_some()
        || !snapshot.can_activate
        || snapshot.pending.is_some()
        || viewport.is_none_or(|viewport| viewport.validate().is_err())
        || snapshot.active_viewport != viewport
    {
        return None;
    }
    snapshot.active.filter(|id| {
        snapshot
            .workers
            .iter()
            .any(|worker| worker.binding.worker.worker == *id && ui::presentation_ready(worker))
    })
}

/// A queued local intent is not an acknowledgment. Keep the prior world
/// covered until the broker has processed that exact request, including an
/// idempotent/no-op or rejected request that legitimately retains its old grant.
#[derive(Default)]
struct PresentationGate {
    request: Option<u64>,
}

impl PresentationGate {
    fn sent(&mut self, request: u64) {
        self.request = Some(self.request.map_or(request, |previous| previous.max(request)));
    }

    fn observe(&mut self, completed: u64) {
        if self.request.is_some_and(|request| completed >= request) {
            self.request = None;
        }
    }

    fn waiting(&self) -> bool {
        self.request.is_some()
    }
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }
        let title = format!("{} — {}", aurora_i18n::tr!("host-title"), crate::cli::window_title());
        match SoftwareWindow::new(event_loop, &self.context, title) {
            Ok(surface) => {
                surface.window.request_redraw();
                self.surface = Some(surface);
                self.request_repaint();
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.controller.shutdown();
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.attention_window.as_ref().is_some_and(|window| window.id() == id) {
            self.refresh_attention();
            let blocked = self.ui.modal_open() || self.closing.is_some() || self.snapshot.stopped;
            let Some(window) = self.attention_window.as_mut() else { return };
            let result = window.event(
                event_loop,
                &event,
                &mut self.attention,
                &mut self.attention_ui,
                &self.snapshot,
                blocked,
            );
            match result {
                Ok((closed, action)) => {
                    if closed {
                        self.cancel_attention_review();
                        self.attention_window = None;
                    } else if let Some(action) = action {
                        self.attention_action(event_loop, action);
                    }
                }
                Err(error) => {
                    log::warn!("session host: owned Attention surface closed after native error: {error}");
                    self.cancel_attention_review();
                    self.attention_ui.surface_failed();
                    self.attention_window = None;
                    self.ui.notice = Some(Notice::Error(transport::HostError::Unavailable));
                    self.notice_at = Some(Instant::now());
                    self.request_repaint();
                }
            }
            return;
        }
        if let Some(window) = self.chat_window.as_mut().filter(|window| window.id() == id) {
            match window.event(event_loop, &event, &self.controller, &mut self.shared_chat, self.ui.modal_open()) {
                Ok(false) => {}
                Ok(true) => {
                    self.chat_window = None;
                }
                Err(error) => {
                    log::warn!("session host: owned chat surface closed after native error: {error}");
                    self.shared_chat.cancel_gestures();
                    self.chat_window = None;
                    self.ui.notice = Some(Notice::Error(transport::HostError::Unavailable));
                    self.notice_at = Some(Instant::now());
                    self.request_repaint();
                }
            }
            return;
        }
        let Some(surface) = &mut self.surface else { return };
        if surface.window.id() != id {
            return;
        }
        if matches!(event, WindowEvent::CloseRequested) {
            // Closing the host is distinct from quitting any running viewer.
            self.ui.close_prompt = true;
            self.close_region();
            self.request_repaint();
            return;
        }
        let response = surface.event(&event);
        if matches!(event, WindowEvent::Focused(false)) {
            // A world-hole click goes to the independent worker, outside
            // egui's event stream. Native focus loss dismisses the old menu
            // without capturing the worker's mouse or changing its grant.
            egui::Popup::close_all(&self.context);
            self.overlays = native::Overlays::default();
            self.request_repaint();
        }
        if matches!(event, WindowEvent::CursorLeft { .. }) {
            // egui-winit queued PointerGone; paint once to remove any hover
            // island even when the worker now receives pointer movement.
            self.request_repaint();
        }
        if response.repaint {
            self.request_repaint();
        }
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } | WindowEvent::Moved(_)
        ) {
            self.close_region();
            self.geometry_dirty = true;
            self.requested_viewport = None;
            if matches!(event, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }) {
                self.layout_dirty = true;
            }
            self.request_repaint();
            // Windows title-bar dragging runs a modal native loop. Submit
            // fresh movement geometry here; about_to_wait may run only after
            // the drag ends. Resize still waits for the real egui layout.
            if let Err(error) = self.sync_native() {
                self.error = Some(error.to_string());
                self.controller.shutdown();
                event_loop.exit();
                return;
            }
        }
        if matches!(event, WindowEvent::RedrawRequested) {
            if self.last_frame.is_some_and(|last| last.elapsed() < MIN_FRAME) {
                self.request_repaint();
            } else if let Err(error) = self.render(event_loop) {
                self.error = Some(error.to_string());
                self.close_region();
                self.controller.shutdown();
                event_loop.exit();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let native_poll = now >= self.next_poll;
        self.poll(now);
        if native_poll && let Err(error) = self.sync_native() {
            self.error = Some(error.to_string());
            self.close_region();
            self.controller.shutdown();
            event_loop.exit();
            return;
        }
        if self
            .closing
            .is_some_and(|at| self.controller.stopped() || now.saturating_duration_since(at) >= CLOSE_GRACE)
        {
            // A nonresponsive bridge is not permission to kill its viewers.
            // Their worker lease restores ordinary reachable windows.
            event_loop.exit();
            return;
        }
        let mut next = self.next_poll;
        if let Some(window) = &mut self.chat_window
            && let Some(repaint) = window.poll(std::mem::take(&mut self.chat_changed), self.ui.modal_open(), now)
        {
            next = next.min(repaint);
        }
        if let Some(window) = &mut self.attention_window
            && let Some(repaint) = window.poll(
                std::mem::take(&mut self.attention_changed),
                self.ui.modal_open() || self.closing.is_some() || self.snapshot.stopped,
                &mut self.attention_ui,
                now,
            )
        {
            next = next.min(repaint);
        }
        if let Some(repaint) = self.repaint {
            let due = self.last_frame.map_or(repaint, |last| repaint.max(last + MIN_FRAME));
            if now >= due {
                self.repaint = None;
                if let Some(surface) = &self.surface {
                    surface.window.request_redraw();
                }
            } else {
                next = next.min(due);
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(next));
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.cancel_attention_review();
        self.close_region();
        self.controller.shutdown();
        // Drop clipboard/surface while the platform connection still lives.
        self.chat_window = None;
        self.shared_chat.clear();
        self.attention_window = None;
        self.attention_ui.cancel_gestures();
        self.attention.clear();
        self.surface = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_admission_needs_visible_current_native_geometry_without_competing_handoff() {
        use aurora_session::{HostedViewport, PhysicalRect};
        let viewport = HostedViewport {
            host_window: 7,
            rect: PhysicalRect {
                x: -1600,
                y: 80,
                width: 1000,
                height: 700,
            },
        };
        let mut snapshot = HostSnapshot {
            can_activate: true,
            ..Default::default()
        };
        assert_eq!(review_viewport(&snapshot, Some(viewport), true, false), Some(viewport));
        assert!(review_viewport(&snapshot, Some(viewport), false, false).is_none());
        assert!(review_viewport(&snapshot, Some(viewport), true, true).is_none());
        assert!(review_viewport(&snapshot, None, true, false).is_none());
        assert!(
            review_viewport(
                &snapshot,
                Some(HostedViewport {
                    host_window: 0,
                    ..viewport
                }),
                true,
                false
            )
            .is_none()
        );
        snapshot.pending = Some(aurora_session::WorkerId::from_bytes([2; 16]).unwrap());
        assert!(review_viewport(&snapshot, Some(viewport), true, false).is_none());
        snapshot.pending = None;
        snapshot.can_activate = false;
        assert!(review_viewport(&snapshot, Some(viewport), true, false).is_none());
        snapshot.can_activate = true;
        snapshot.stopped = true;
        assert!(review_viewport(&snapshot, Some(viewport), true, false).is_none());
    }

    #[test]
    fn queued_local_intent_cannot_reopen_previous_acknowledged_world() {
        let mut gate = PresentationGate::default();
        gate.sent(12);
        gate.observe(11);
        assert!(gate.waiting());
        gate.observe(12);
        assert!(!gate.waiting());
    }

    #[test]
    fn idempotent_activation_and_duplicate_open_resolve_from_exact_broker_receipt() {
        let mut gate = PresentationGate::default();
        for request in [12, 13] {
            gate.sent(request);
            gate.observe(request - 1);
            assert!(gate.waiting(), "an unchanged old grant cannot acknowledge a local request");
            gate.observe(request);
            assert!(!gate.waiting(), "broker no-op receipt does not require a viewport change");
        }
    }

    #[test]
    fn overlapping_local_intents_wait_for_the_latest_exact_fifo_receipt() {
        let mut gate = PresentationGate::default();
        gate.sent(12);
        gate.sent(13);
        gate.observe(12);
        assert!(gate.waiting());
        gate.observe(13);
        assert!(!gate.waiting());
    }
}
