//! One disposable owned CPU surface. Shell retains the canonical shared-chat
//! model when this window closes, is minimized or cannot be created.

use super::chat_ui::InputMode;
use super::shared_chat::SharedChat;
use super::software::SoftwareWindow;
use super::transport::HostController;
use crate::theme::Theme;
use crate::ui::fonts;
use std::time::{Duration, Instant};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

const MIN_FRAME: Duration = Duration::from_millis(33);

pub(super) struct ChatWindow {
    surface: SoftwareWindow,
    context: egui::Context,
    theme: Theme,
    repaint: Option<Instant>,
    last_frame: Option<Instant>,
    blocked: bool,
    pending_textures: egui::TexturesDelta,
}

impl ChatWindow {
    pub fn new(event_loop: &ActiveEventLoop, owner: &Window) -> anyhow::Result<Self> {
        let context = egui::Context::default();
        let theme = Theme {
            panel_alpha: 255,
            ..Default::default()
        };
        theme.apply(&context, 1.0);
        fonts::apply(&context, fonts::DEFAULT);
        // Embedded icons belong to the host's texture space. This independent
        // CPU context uses text and font meshes only, just as the old popout.
        let title = aurora_i18n::tr!("host-chat-shared-title", viewer = crate::cli::window_title());
        let surface = SoftwareWindow::owned_tool(event_loop, &context, owner, title, [680.0, 680.0], [380.0, 400.0])?;
        surface.window.set_visible(true);
        surface.window.request_redraw();
        Ok(Self {
            surface,
            context,
            theme,
            repaint: Some(Instant::now()),
            last_frame: None,
            blocked: false,
            pending_textures: Default::default(),
        })
    }

    pub fn id(&self) -> WindowId {
        self.surface.window.id()
    }

    pub fn focused(&self) -> bool {
        self.surface.window.has_focus()
    }

    pub fn focus(&mut self) {
        super::chat_ui::cancel_input(&self.context);
        self.surface.window.set_minimized(false);
        self.surface.window.focus_window();
        self.repaint = Some(Instant::now());
        self.surface.window.request_redraw();
    }

    /// Apply only edits already delivered to the old focused composer. Check
    /// current authority first; this pass emits no sends or platform output.
    pub fn flush_edits(&mut self, controller: &HostController, model: &mut SharedChat) {
        model.sync(&controller.snapshot(), false);
        let mut raw = self.surface.raw_input();
        if let Some(edits) = edit_prefix(
            &mut raw,
            model.composer_rect(&self.context),
            egui::Popup::is_any_open(&self.context),
        ) {
            self.apply_edits(edits, model);
        }
        self.apply_edits(edit_input(raw), model);
    }

    fn apply_edits(&mut self, raw: egui::RawInput, model: &mut SharedChat) {
        let palette = self.theme.palette();
        let mut output = self.context.run_ui(raw, |ui| {
            let _ = model.show(ui, &palette, InputMode::Closing);
        });
        // A navigation flush reuses this context. Keep any new font/image
        // uploads until the following visible pass instead of losing them.
        self.pending_textures.append(std::mem::take(&mut output.textures_delta));
    }

    /// Close disposes only this surface. The caller keeps the shared model.
    pub fn event(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: &WindowEvent,
        controller: &HostController,
        model: &mut SharedChat,
        blocked: bool,
    ) -> anyhow::Result<bool> {
        if matches!(event, WindowEvent::CloseRequested) {
            self.flush_edits(controller, model);
            model.cancel_gestures();
            return Ok(true);
        }
        if matches!(event, WindowEvent::Destroyed) {
            model.cancel_gestures();
            return Ok(true);
        }
        if matches!(event, WindowEvent::Focused(false)) {
            self.flush_edits(controller, model);
            model.cancel_gestures();
            super::chat_ui::cancel_input(&self.context);
        }
        if self.surface.event(event).repaint {
            self.repaint = Some(Instant::now());
        }
        if matches!(event, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }) {
            self.repaint = Some(Instant::now());
        }
        if matches!(event, WindowEvent::RedrawRequested) {
            if self.last_frame.is_some_and(|last| last.elapsed() < MIN_FRAME) {
                self.repaint = Some(Instant::now());
            } else {
                self.render(event_loop, controller, model, blocked)?;
            }
        }
        Ok(false)
    }

    pub fn poll(&mut self, changed: bool, blocked: bool, now: Instant) -> Option<Instant> {
        if changed || self.blocked != blocked {
            self.repaint = Some(now);
        }
        self.blocked = blocked;
        let due = self
            .repaint
            .map(|repaint| self.last_frame.map_or(repaint, |last| repaint.max(last + MIN_FRAME)))?;
        if now >= due {
            self.repaint = None;
            self.surface.window.request_redraw();
            None
        } else {
            Some(due)
        }
    }

    fn render(
        &mut self,
        event_loop: &ActiveEventLoop,
        controller: &HostController,
        model: &mut SharedChat,
        blocked: bool,
    ) -> anyhow::Result<()> {
        let size = self.surface.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        model.sync(&controller.snapshot(), false);
        let mut raw = self.surface.raw_input();
        let mut suppress_send = blocked;
        if let Some(edits) = edit_prefix(
            &mut raw,
            model.composer_rect(&self.context),
            egui::Popup::is_any_open(&self.context),
        ) {
            suppress_send |= edits.events.iter().any(|event| matches!(event, egui::Event::Ime(_)));
            self.apply_edits(edits, model);
        }
        let context = self.context.clone();
        let palette = self.theme.palette();
        let mut action = None;
        let mut output = context.run_ui(raw, |ui| {
            let mode = if suppress_send { InputMode::Navigating } else { InputMode::Live };
            let shown = model.show(ui, &palette, mode);
            action = action.take().or(shown);
        });
        self.pending_textures.append(std::mem::take(&mut output.textures_delta));
        output.textures_delta = std::mem::take(&mut self.pending_textures);
        let delay = self.surface.present(event_loop, &context, output, palette.panel)?;
        self.last_frame = Some(Instant::now());
        self.repaint = Instant::now().checked_add(delay);
        if let Some(draft) = action {
            match controller.send_chat(draft.key, draft.policy_revision, draft.text.clone()) {
                Ok(request) => model.queued(&draft, request),
                Err(error) => model.queue_failed(&draft, error),
            }
            self.repaint = Some(Instant::now());
        }
        Ok(())
    }
}

/// Drain delivered editing input before close/focus loss, without turning a
/// pending click, Enter or unfinished IME candidate into a Send action.
pub(super) fn edit_input(mut raw: egui::RawInput) -> egui::RawInput {
    raw.focused = true;
    raw.events.retain(|event| match event {
        egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::ModifiersChanged(_) => true,
        egui::Event::Key { key, .. } => !matches!(key, egui::Key::Enter | egui::Key::Tab | egui::Key::Escape),
        egui::Event::Ime(egui::ImeEvent::Commit(_) | egui::ImeEvent::DeleteSurrounding { .. }) => true,
        _ => false,
    });
    raw
}

/// Winit can deliver text/paste before a navigation press in the same redraw
/// batch. Drain that ordered prefix through the old focused editor before the
/// pointer changes focus; the remaining raw input still performs navigation.
pub(super) fn edit_prefix(raw: &mut egui::RawInput, composer: Option<egui::Rect>, popup_open: bool) -> Option<egui::RawInput> {
    let navigation = raw.events.iter().position(|event| match event {
        egui::Event::PointerButton { pos, pressed: true, .. } => composer.is_none_or(|rect| !rect.contains(*pos)),
        egui::Event::Key {
            key: egui::Key::Tab | egui::Key::Escape,
            pressed: true,
            ..
        }
        | egui::Event::WindowFocused(_) => true,
        egui::Event::Key {
            key: egui::Key::Enter,
            pressed: true,
            ..
        } => popup_open,
        _ => false,
    });
    let mut index = 0;
    raw.events.retain(|event| {
        let ambiguous = navigation.is_some_and(|navigation| index > navigation)
            && (matches!(
                event,
                egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Cut | egui::Event::Ime(_)
            ) || matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::Enter,
                    pressed: true,
                    ..
                }
            ));
        index += 1;
        !ambiguous
    });
    if composer.is_none() {
        return None;
    }
    let through = raw
        .events
        .iter()
        .position(|event| {
            matches!(
                event,
                egui::Event::PointerButton { pressed: true, .. }
                    | egui::Event::Key {
                        key: egui::Key::Enter | egui::Key::Tab | egui::Key::Escape,
                        pressed: true,
                        ..
                    }
                    | egui::Event::WindowFocused(_)
                    | egui::Event::Ime(egui::ImeEvent::Preedit { .. })
            )
        })
        .unwrap_or(raw.events.len());
    let mut edits = Vec::new();
    let mut index = 0;
    raw.events.retain(|event| {
        let editable = index < through
            && !matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::Enter | egui::Key::Tab | egui::Key::Escape,
                    ..
                }
            )
            && matches!(
                event,
                egui::Event::Text(_)
                    | egui::Event::Paste(_)
                    | egui::Event::Key { .. }
                    | egui::Event::Ime(egui::ImeEvent::Commit(_) | egui::ImeEvent::DeleteSurrounding { .. })
            );
        index += 1;
        if editable {
            edits.push(event.clone());
        }
        !editable
    });
    if edits.is_empty() {
        return None;
    }
    let mut input = raw.clone();
    input.events = edits;
    Some(edit_input(input))
}
