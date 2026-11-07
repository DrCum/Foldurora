//! Disposable owned CPU surface; Shell retains reminder and preference state.

use super::attention::Attention;
use super::attention_ui::{Action, AttentionUi};
use super::chat_ui::{self, InputMode};
use super::chat_window::enter_release::EnterRelease;
use super::software::SoftwareWindow;
use super::transport::HostSnapshot;
use crate::theme::Theme;
use crate::ui::fonts;
use std::time::{Duration, Instant};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

const MIN_FRAME: Duration = Duration::from_millis(33);

/// Button activation has no multiline exception. Reuse the proven native
/// release fence independently for Return and Space; either key can activate a
/// focused control, including after winit resets its repeat tracking on focus.
#[derive(Default)]
struct ButtonRelease {
    enter: EnterRelease,
    space: EnterRelease,
}

impl ButtonRelease {
    fn focused(&mut self, focused: bool) {
        self.enter.focused(focused);
        self.space.focused(focused);
    }

    fn suppress(&mut self, key: &Key, state: ElementState, repeat: bool, synthetic: bool) -> bool {
        let fence = match key {
            Key::Named(NamedKey::Enter) => &mut self.enter,
            Key::Named(NamedKey::Space) => &mut self.space,
            _ => return false,
        };
        // Both independent fences operate on the logical activation-key
        // family. Ignore Shift here: Attention contains no multiline editor.
        let held = fence.suppress(&Key::Named(NamedKey::Enter), state, repeat, synthetic, ModifiersState::empty());
        // A synthetic focus-loss release is not a button activation either.
        synthetic || held
    }
}

pub(super) struct AttentionWindow {
    surface: SoftwareWindow,
    context: egui::Context,
    theme: Theme,
    repaint: Option<Instant>,
    last_frame: Option<Instant>,
    blocked: bool,
    keys: ButtonRelease,
}

impl AttentionWindow {
    pub fn new(event_loop: &ActiveEventLoop, owner: &Window) -> anyhow::Result<Self> {
        let context = egui::Context::default();
        let theme = Theme {
            panel_alpha: 255,
            ..Default::default()
        };
        theme.apply(&context, 1.0);
        fonts::apply(&context, fonts::DEFAULT);
        let title = format!("{} — {}", aurora_i18n::tr!("host-attention-title"), crate::cli::window_title());
        let surface = SoftwareWindow::owned_tool(event_loop, &context, owner, title, [540.0, 580.0], [380.0, 340.0])?;
        surface.window.set_visible(true);
        surface.window.request_redraw();
        Ok(Self {
            surface,
            context,
            theme,
            repaint: Some(Instant::now()),
            last_frame: None,
            blocked: false,
            keys: Default::default(),
        })
    }

    pub fn id(&self) -> WindowId {
        self.surface.window.id()
    }

    pub fn focused(&self) -> bool {
        self.surface.window.has_focus()
    }

    pub fn focus(&mut self, ui: &mut AttentionUi) {
        ui.cancel_gestures();
        chat_ui::cancel_input(&self.context);
        self.surface.window.set_minimized(false);
        self.surface.window.focus_window();
        self.repaint = Some(Instant::now());
        self.surface.window.request_redraw();
    }

    pub fn event(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: &WindowEvent,
        model: &mut Attention,
        ui: &mut AttentionUi,
        snapshot: &HostSnapshot,
        blocked: bool,
    ) -> anyhow::Result<(bool, Option<Action>)> {
        if matches!(event, WindowEvent::CloseRequested | WindowEvent::Destroyed) {
            ui.surface_failed();
            return Ok((true, None));
        }
        if let WindowEvent::Focused(focused) = event {
            self.keys.focused(*focused);
        }
        if let WindowEvent::KeyboardInput { event, is_synthetic, .. } = event
            && self.keys.suppress(&event.logical_key, event.state, event.repeat, *is_synthetic)
        {
            return Ok((false, None));
        }
        if matches!(event, WindowEvent::Focused(false)) {
            ui.cancel_gestures();
            chat_ui::cancel_input(&self.context);
        }
        if blocked != self.blocked {
            ui.cancel_gestures();
            chat_ui::cancel_input(&self.context);
            self.blocked = blocked;
        }
        if self.surface.event(event).repaint {
            self.repaint = Some(Instant::now());
        }
        if matches!(event, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }) {
            self.repaint = Some(Instant::now());
        }
        let mut action = None;
        if matches!(event, WindowEvent::RedrawRequested) {
            if self.last_frame.is_some_and(|last| last.elapsed() < MIN_FRAME) {
                self.repaint = Some(Instant::now());
            } else {
                action = self.render(event_loop, model, ui, snapshot, blocked)?;
            }
        }
        Ok((false, action))
    }

    pub fn poll(&mut self, changed: bool, blocked: bool, ui: &mut AttentionUi, now: Instant) -> Option<Instant> {
        if changed || self.blocked != blocked {
            self.repaint = Some(now);
        }
        if self.blocked != blocked {
            ui.cancel_gestures();
            chat_ui::cancel_input(&self.context);
        }
        self.blocked = blocked;
        let due = self.repaint.map(|at| self.last_frame.map_or(at, |last| at.max(last + MIN_FRAME)))?;
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
        model: &mut Attention,
        body: &mut AttentionUi,
        snapshot: &HostSnapshot,
        blocked: bool,
    ) -> anyhow::Result<Option<Action>> {
        let size = self.surface.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(None);
        }
        let context = self.context.clone();
        let palette = self.theme.palette();
        let mode = if blocked { InputMode::Navigating } else { InputMode::Live };
        let mut action = None;
        let output = context.run_ui(self.surface.raw_input(), |ui| {
            let shown = body.show(ui, &palette, model, snapshot, mode);
            if mode == InputMode::Live {
                action = action.take().or(shown);
            }
        });
        let delay = match self.surface.present(event_loop, &context, output, palette.panel) {
            Ok(delay) => delay,
            Err(error) => {
                body.surface_failed();
                return Err(error);
            }
        };
        let now = Instant::now();
        self.last_frame = Some(now);
        self.repaint = now.checked_add(delay);
        Ok(action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attention_native_button_refocus_fences_return_and_space_until_real_release() {
        for key in [NamedKey::Enter, NamedKey::Space] {
            for snapshot_before_focus in [false, true] {
                let mut keys = ButtonRelease::default();
                keys.focused(true);
                assert!(!keys.suppress(&Key::Named(key), ElementState::Pressed, false, false));
                assert!(
                    keys.suppress(&Key::Named(key), ElementState::Released, false, true),
                    "focus snapshot up must not activate"
                );
                keys.focused(false);
                if !snapshot_before_focus {
                    keys.focused(true);
                }
                assert!(keys.suppress(&Key::Named(key), ElementState::Pressed, false, true));
                if snapshot_before_focus {
                    keys.focused(true);
                }
                assert!(keys.suppress(&Key::Named(key), ElementState::Pressed, false, false));
                assert!(keys.suppress(&Key::Named(key), ElementState::Pressed, true, false));
                assert!(!keys.suppress(&Key::Named(key), ElementState::Released, false, false));
                assert!(!keys.suppress(&Key::Named(key), ElementState::Pressed, false, false));
            }
        }
    }

    #[test]
    fn attention_native_button_release_does_not_rearm_the_other_held_key() {
        let mut keys = ButtonRelease::default();
        keys.focused(true);
        for key in [NamedKey::Enter, NamedKey::Space] {
            assert!(!keys.suppress(&Key::Named(key), ElementState::Pressed, false, false));
        }
        assert!(!keys.suppress(&Key::Named(NamedKey::Enter), ElementState::Released, false, false));
        assert!(keys.suppress(&Key::Named(NamedKey::Space), ElementState::Pressed, false, false));
        assert!(!keys.suppress(&Key::Named(NamedKey::Enter), ElementState::Pressed, false, false));
        assert!(!keys.suppress(&Key::Character("a".into()), ElementState::Released, false, false));
        assert!(keys.suppress(&Key::Named(NamedKey::Space), ElementState::Pressed, false, false));
    }
}
