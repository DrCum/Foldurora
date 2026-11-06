//! Owned CPU-only chat popouts. These never create a Vulkan context, forward
//! input to a worker world, or participate in its activation transaction.

use super::chat_ui::{self, ChatUi};
use super::software::SoftwareWindow;
use super::transport::{HostChatSnapshot, HostController, HostSnapshot, WorkerSummary};
use crate::theme::Theme;
use crate::ui::fonts;
use aurora_session::chat::ChatSource;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

const MIN_FRAME: Duration = Duration::from_millis(33);

pub(super) struct ChatWindow {
    surface: SoftwareWindow,
    context: egui::Context,
    theme: Theme,
    ui: ChatUi,
    snapshot: Option<Arc<HostChatSnapshot>>,
    account: String,
    grid: String,
    repaint: Option<Instant>,
    last_frame: Option<Instant>,
}

impl ChatWindow {
    pub fn new(
        event_loop: &ActiveEventLoop,
        owner: &Window,
        worker: &WorkerSummary,
        snapshot: Arc<HostChatSnapshot>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            worker.binding == snapshot.source.binding() && chat_ui::eligible(&snapshot),
            "chat source is not available"
        );
        let context = egui::Context::default();
        let theme = Theme {
            panel_alpha: 255,
            ..Default::default()
        };
        theme.apply(&context, 1.0);
        fonts::apply(&context, fonts::DEFAULT);
        // No Icons::load here: its process-global texture handles belong to
        // the host context. This view uses font meshes and text-only widgets.
        let account = worker
            .status
            .as_ref()
            .map(|status| status.name.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(&worker.label)
            .to_owned();
        let grid = worker
            .status
            .as_ref()
            .map(|status| status.grid_label.as_str())
            .filter(|grid| !grid.is_empty())
            .unwrap_or(&worker.grid)
            .to_owned();
        let title = aurora_i18n::tr!(
            "host-chat-title",
            account = account.as_str(),
            grid = grid.as_str(),
            viewer = crate::cli::window_title()
        );
        let surface = SoftwareWindow::owned_tool(event_loop, &context, owner, title, [640.0, 640.0], [380.0, 360.0])?;
        let mut ui = ChatUi::new(snapshot.source);
        ui.sync(Some(&snapshot));
        surface.window.set_visible(true);
        surface.window.request_redraw();
        Ok(Self {
            surface,
            context,
            theme,
            ui,
            snapshot: Some(snapshot),
            account,
            grid,
            repaint: Some(Instant::now()),
            last_frame: None,
        })
    }

    pub fn id(&self) -> WindowId {
        self.surface.window.id()
    }
    pub fn source(&self) -> ChatSource {
        self.ui.source()
    }
    pub fn focused(&self) -> bool {
        self.surface.window.has_focus()
    }
    pub fn focus(&self) {
        self.surface.window.focus_window();
    }

    /// Returns true only when this popout should close. Close never shuts
    /// down a worker, sends chat or changes the active world.
    pub fn event(&mut self, event_loop: &ActiveEventLoop, event: &WindowEvent, controller: &HostController) -> anyhow::Result<bool> {
        if matches!(event, WindowEvent::CloseRequested | WindowEvent::Destroyed) {
            return Ok(true);
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
                self.render(event_loop, controller)?;
            }
        }
        Ok(false)
    }

    pub fn poll(&mut self, snapshot: &HostSnapshot, closing: bool, now: Instant) -> Option<Instant> {
        let current = (!closing && !snapshot.stopped && snapshot.error.is_none())
            .then(|| {
                snapshot
                    .chat
                    .iter()
                    .find(|chat| chat.source == self.source() && chat_ui::eligible(chat))
                    .cloned()
            })
            .flatten();
        let changed = match (&self.snapshot, &current) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        self.ui.sync(current.as_deref());
        self.snapshot = current;
        if changed {
            self.repaint = Some(now);
        }
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

    fn render(&mut self, event_loop: &ActiveEventLoop, controller: &HostController) -> anyhow::Result<()> {
        let size = self.surface.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        let raw = self.surface.raw_input();
        let context = self.context.clone();
        let p = self.theme.palette();
        let mut action = None;
        let output = context.run_ui(raw, |ui| {
            let shown = self.ui.show(ui, &p, self.snapshot.as_deref(), &self.account, &self.grid);
            action = action.take().or(shown);
        });
        let delay = self.surface.present(event_loop, &context, output, p.panel)?;
        self.last_frame = Some(Instant::now());
        self.repaint = Instant::now().checked_add(delay);
        if let Some(draft) = action {
            match controller.send_chat(draft.key, draft.policy_revision, draft.text.clone()) {
                Ok(request) => self.ui.queued(&draft, request),
                Err(error) => self.ui.queue_failed(error),
            }
            self.repaint = Some(Instant::now());
        }
        Ok(())
    }
}
