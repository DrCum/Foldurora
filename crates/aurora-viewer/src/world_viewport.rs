//! Saved world-only canvas within a display-spanning native window.
//! Expected layout behavior follows LLViewerWindow::updateWorldViewRect
//! (indra/newview/llviewerwindow.cpp); rendering uses Aurora's own targets.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewportSettings {
    pub enabled: bool,
    /// Percentages of the native canvas, independent of monitor DPI.
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl ViewportSettings {
    pub fn viewport(&self, size: (u32, u32)) -> aurora_render::WorldViewport {
        if !self.enabled {
            return aurora_render::WorldViewport::full(size);
        }
        aurora_render::WorldViewport::from_margins(size, [self.left / 100.0, self.right / 100.0, self.top / 100.0, self.bottom / 100.0])
    }

    pub fn half(&mut self, right: bool) {
        *self = Self {
            enabled: true,
            left: if right { 50.0 } else { 0.0 },
            right: if right { 0.0 } else { 50.0 },
            top: 0.0,
            bottom: 0.0,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_default_to_full_canvas_and_presets_keep_the_other_half_for_panels() {
        let mut settings: ViewportSettings = serde_json::from_str("{}").unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings.viewport((3840, 1080)), aurora_render::WorldViewport::full((3840, 1080)));
        settings.half(false);
        assert_eq!(settings.viewport((3840, 1080)).rect(), [0, 0, 1920, 1080]);
        settings.half(true);
        assert_eq!(settings.viewport((3840, 1080)).rect(), [1920, 0, 1920, 1080]);
        settings.left = f32::NAN;
        settings.top = 500.0;
        settings.bottom = 500.0;
        let view = settings.viewport((1280, 720));
        assert_eq!(view.width, 1280);
        assert!(view.height >= 36);
    }
}
