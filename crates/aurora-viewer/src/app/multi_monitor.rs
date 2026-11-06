//! World viewport state shared by rendering, HUD layout and input gating.
use super::*;

impl App {
    /// Synthetic display layouts for offline demo captures; never saved.
    pub(super) fn demo_world_viewport(&mut self) {
        let Ok(mode) = std::env::var("AURORA_DEMO_VIEWPORT") else { return };
        match mode.as_str() {
            "left" => self.settings.world_viewport.half(false),
            "right" => self.settings.world_viewport.half(true),
            "inset" | "scaled" => {
                self.settings.world_viewport = crate::world_viewport::ViewportSettings {
                    enabled: true,
                    left: 25.0,
                    right: 10.0,
                    top: 15.0,
                    bottom: 20.0,
                };
                if mode == "scaled" {
                    self.settings.render_scale_percent = 75;
                }
            }
            _ => {}
        }
    }

    pub(super) fn world_viewport_for_size(&self, size: (u32, u32), vr: bool) -> aurora_render::WorldViewport {
        if !matches!(self.screen, Screen::World)
            || self.camera.mouselook()
            || self.interface.hidden
            || self.panels.snapshot
            || self.snapshots.capturing.is_some()
            || vr
        {
            return aurora_render::WorldViewport::full(size);
        }
        self.settings.world_viewport.viewport(size)
    }

    pub(super) fn world_pointer_inside(&self) -> bool {
        self.world_viewport
            .is_none_or(|view| view.local(self.cursor_pos.0, self.cursor_pos.1).is_some())
    }
}
