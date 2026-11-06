//! Saved world-only canvas within a display-spanning native window.
//! Expected layout behavior follows LLViewerWindow::updateWorldViewRect
//! (indra/newview/llviewerwindow.cpp); rendering uses Aurora's own targets.

use serde::{Deserialize, Serialize};

#[cfg(test)]
mod interface_tests;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewportSettings {
    pub enabled: bool,
    /// Keep chrome and camera/photo controls in the world rectangle;
    /// ordinary windows retain the full native workspace.
    pub restrict_ui: bool,
    /// Percentages of the native canvas, independent of monitor DPI.
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl ViewportSettings {
    /// A fitted worker's native window already is the host's world area.
    /// Apply this same effective policy to rendering, controls and cursor
    /// capture without overwriting the saved standalone arrangement.
    pub fn for_hosted_worker(&self, fitted: bool) -> Self {
        if fitted { Self::default() } else { self.clone() }
    }

    pub fn render_viewport(
        &self,
        size: (u32, u32),
        mouselook: bool,
        interface_hidden: bool,
        snapshot: bool,
        vr: bool,
    ) -> aurora_render::WorldViewport {
        if snapshot || vr || (interface_hidden && !mouselook) {
            aurora_render::WorldViewport::full(size)
        } else {
            self.viewport(size)
        }
    }

    pub fn viewport(&self, size: (u32, u32)) -> aurora_render::WorldViewport {
        if !self.enabled {
            return aurora_render::WorldViewport::full(size);
        }
        aurora_render::WorldViewport::from_margins(size, [self.left / 100.0, self.right / 100.0, self.top / 100.0, self.bottom / 100.0])
    }

    pub fn half(&mut self, right: bool) {
        *self = Self {
            enabled: true,
            restrict_ui: self.restrict_ui,
            left: if right { 50.0 } else { 0.0 },
            right: if right { 0.0 } else { 50.0 },
            top: 0.0,
            bottom: 0.0,
        };
    }

    /// Use the integration's logical canvas to include this frame's native
    /// DPI and interface zoom, rather than the previous egui pass's scale.
    /// Pointer positions stay in the native canvas's absolute coordinates.
    pub fn toolbar_rect(&self, native: (u32, u32), screen: egui::Rect) -> Option<egui::Rect> {
        // Placement deliberately uses the configured view. Snapshot output,
        // mouselook and capture can render full-frame without moving controls.
        let view = self.viewport(native);
        if !self.enabled
            || !self.restrict_ui
            || native.0 == 0
            || native.1 == 0
            || !screen.is_finite()
            || !screen.is_positive()
            || view == aurora_render::WorldViewport::full(native)
        {
            return None;
        }
        let view = view.fitted(native);
        let scale = screen.size() / egui::vec2(native.0 as f32, native.1 as f32);
        let min = screen.min + egui::vec2(view.x as f32, view.y as f32) * scale;
        let rect = egui::Rect::from_min_size(min, egui::vec2(view.width as f32, view.height as f32) * scale);
        rect.is_positive().then_some(rect)
    }
}

/// A saved spanning canvas must fit the connected desktop when restored.
/// Positions can be negative; the saved window dimensions are logical and
/// must be compared with display bounds in physical pixels at the new DPI.
pub fn span_fits_displays(size: (u32, u32), scale: f64, position: Option<(i32, i32)>, displays: &[[i64; 4]]) -> bool {
    let bounds = displays
        .iter()
        .copied()
        .filter(|d| d[2] > d[0] && d[3] > d[1])
        .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
    let Some(bounds) = bounds else { return true };
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let (width, height) = ((f64::from(size.0) * scale).round(), (f64::from(size.1) * scale).round());
    if width > (bounds[2] - bounds[0]) as f64 + 1.0 || height > (bounds[3] - bounds[1]) as f64 + 1.0 {
        return false;
    }
    position.is_none_or(|(x, y)| {
        // Saved origins include native decorations; tolerate a frame border.
        let border = 32.0;
        f64::from(x) >= bounds[0] as f64 - border
            && f64::from(y) >= bounds[1] as f64 - border
            && f64::from(x) + width <= bounds[2] as f64 + border
            && f64::from(y) + height <= bounds[3] as f64 + border
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitted_workers_share_full_render_and_toolbar_geometry_until_detached() {
        let settings = ViewportSettings {
            enabled: true,
            restrict_ui: true,
            left: 25.0,
            right: 10.0,
            top: 15.0,
            bottom: 20.0,
        };
        let size = (3200, 2000);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
        let standalone = settings.for_hosted_worker(false);
        let hosted = settings.for_hosted_worker(true);
        assert_eq!(
            standalone.render_viewport(size, true, true, false, false).rect(),
            [800, 300, 2080, 1300]
        );
        assert!(standalone.toolbar_rect(size, screen).is_some());
        for mouselook in [false, true] {
            assert_eq!(
                hosted.render_viewport(size, mouselook, false, false, false),
                aurora_render::WorldViewport::full(size)
            );
            assert!(hosted.toolbar_rect(size, screen).is_none());
        }
        let detached = settings.for_hosted_worker(false);
        assert_eq!(detached, standalone);
        assert_eq!(settings, standalone, "hosting never rewrites the saved arrangement");
    }

    #[test]
    fn old_settings_default_to_full_canvas_and_presets_keep_the_other_half_for_panels() {
        let mut settings: ViewportSettings = serde_json::from_str("{}").unwrap();
        assert!(!settings.enabled);
        assert!(!settings.restrict_ui);
        assert_eq!(settings.viewport((3840, 1080)), aurora_render::WorldViewport::full((3840, 1080)));
        settings.half(false);
        assert_eq!(settings.viewport((3840, 1080)).rect(), [0, 0, 1920, 1080]);
        settings.half(true);
        assert_eq!(settings.viewport((3840, 1080)).rect(), [1920, 0, 1920, 1080]);
        settings.restrict_ui = true;
        settings.half(false);
        assert!(settings.restrict_ui, "placement presets retain the interface preference");
        let saved = serde_json::to_string(&settings).expect("saved viewport");
        assert_eq!(serde_json::from_str::<ViewportSettings>(&saved).unwrap(), settings);
        settings.half(true);
        settings.left = f32::NAN;
        settings.top = 500.0;
        settings.bottom = 500.0;
        let view = settings.viewport((1280, 720));
        assert_eq!(view.width, 1280);
        assert!(view.height >= 36);
    }

    #[test]
    fn saved_spans_survive_connected_displays_and_recover_after_removal_or_dpi_changes() {
        let dual = [[-1920, 0, 0, 1080], [0, 0, 2560, 1440]];
        assert!(span_fits_displays((4480, 1440), 1.0, Some((-1920, 0)), &dual));
        assert!(span_fits_displays((2987, 960), 1.5, Some((-1920, 0)), &dual));
        assert!(!span_fits_displays((4480, 1440), 1.0, Some((-1920, 0)), &dual[1..]));
        assert!(!span_fits_displays((4480, 1440), 1.5, Some((-1920, 0)), &dual));
        assert!(span_fits_displays(
            (3200, 2104),
            1.0,
            Some((-1280, -1024)),
            &[[-1280, -1024, 0, 0], [0, 0, 1920, 1080]]
        ));
        assert!(span_fits_displays((800, 600), f64::NAN, Some((0, 0)), &[[0, 0, 1920, 1080]]));
        assert!(span_fits_displays((4480, 1440), 1.0, None, &[]));
        let right = [[0, 0, 1920, 1080], [1920, 0, 3840, 1080]];
        let left = [[-1920, 0, 0, 1080], [0, 0, 1920, 1080]];
        assert!(span_fits_displays((3840, 1080), 1.0, Some((0, 0)), &right));
        assert!(!span_fits_displays((3840, 1080), 1.0, Some((0, 0)), &left));
        assert!(span_fits_displays((3840, 1080), 1.0, Some((-1920, 0)), &left));
    }
}
