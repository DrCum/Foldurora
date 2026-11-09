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
    /// Hosting determines the outer client canvas, not the world inside it.
    /// Render, controls and cursor capture share the character's margins in
    /// both modes; fitting never rewrites the saved standalone arrangement.
    pub fn for_hosted_worker(&self, _fitted: bool) -> Self {
        self.clone()
    }

    pub fn render_viewport(
        &self,
        size: (u32, u32),
        mouselook: bool,
        interface_hidden: bool,
        capture: bool,
        vr: bool,
    ) -> aurora_render::WorldViewport {
        if capture || vr || (interface_hidden && !mouselook) {
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

/// Live camera geometry, frozen by a photo job separately from its output
/// dimensions. This is ephemeral and never changes a saved viewport setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhotoScene {
    pub native: (u32, u32),
    pub view: aurora_render::WorldViewport,
}

/// Accept every aspect a nonzero u32 pixel rectangle can have. A stylistic
/// minimum would distort a valid narrow scene or photo; values outside the
/// pixel domain (including nonfinite/nonpositive inputs) use a square fallback.
pub(crate) fn pixel_aspect(aspect: f32) -> f32 {
    let largest = u32::MAX as f32;
    if aspect.is_finite() && aspect >= 1.0 / largest && aspect <= largest {
        aspect
    } else {
        1.0
    }
}

impl PhotoScene {
    pub fn new(native: (u32, u32), view: aurora_render::WorldViewport) -> Self {
        let native = (native.0.max(1), native.1.max(1));
        Self {
            native,
            view: view.fitted(native),
        }
    }

    pub fn aspect(self) -> f32 {
        self.view.width as f32 / self.view.height as f32
    }

    /// A rounded preview target still frames the selected full photo. With
    /// the interface, preserve the live scene inside the full output canvas.
    pub fn capture_aspect(self, photo: (u32, u32), interface: bool) -> f32 {
        if interface {
            self.aspect()
        } else {
            pixel_aspect(photo.0 as f32 / photo.1.max(1) as f32)
        }
    }

    pub fn output_viewport(self, output: (u32, u32)) -> aurora_render::WorldViewport {
        self.view.scaled(self.native, output)
    }

    pub fn logical_rect(self, canvas: egui::Rect) -> egui::Rect {
        let scale = canvas.size() / egui::vec2(self.native.0 as f32, self.native.1 as f32);
        egui::Rect::from_min_size(
            canvas.min + egui::vec2(self.view.x as f32, self.view.y as f32) * scale,
            egui::vec2(self.view.width as f32, self.view.height as f32) * scale,
        )
    }

    /// Scene-normalized AF point to absolute native pixels. Renderer picking
    /// continues to receive native coordinates, including the viewport origin.
    pub fn point(self, point: Option<egui::Pos2>) -> (f32, f32) {
        let point = point.filter(|p| p.is_finite()).unwrap_or(egui::pos2(0.5, 0.5));
        (
            self.view.x as f32 + (point.x.clamp(0.0, 1.0) * self.view.width as f32).min(self.view.width as f32 - 0.5),
            self.view.y as f32 + (point.y.clamp(0.0, 1.0) * self.view.height as f32).min(self.view.height as f32 - 0.5),
        )
    }

    pub fn normalized_point(self, point: (f32, f32)) -> Option<egui::Pos2> {
        let (x, y) = self.view.local(point.0, point.1)?;
        Some(egui::pos2(x / self.view.width as f32, y / self.view.height as f32))
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
    fn fitted_workers_keep_character_world_and_toolbar_geometry_after_detach() {
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
                standalone.viewport(size)
            );
            assert_eq!(hosted.toolbar_rect(size, screen), standalone.toolbar_rect(size, screen));
        }
        let detached = settings.for_hosted_worker(false);
        assert_eq!(detached, standalone);
        assert_eq!(settings, standalone, "hosting never rewrites the saved arrangement");
    }

    #[test]
    fn live_photo_scene_is_independent_of_control_restriction_and_keeps_explicit_overrides() {
        let mut settings = ViewportSettings {
            enabled: true,
            restrict_ui: false,
            left: 40.0,
            right: 10.0,
            top: 10.0,
            bottom: 20.0,
        };
        let native = (5000, 3000);
        let scene = |settings: &ViewportSettings, mouselook, hidden, vr| {
            PhotoScene::new(native, settings.render_viewport(native, mouselook, hidden, false, vr))
        };
        let bounded = scene(&settings, false, false, false);
        assert_eq!(bounded.view.rect(), [2000, 300, 2500, 2100]);
        settings.restrict_ui = true;
        assert_eq!(scene(&settings, false, false, false), bounded);
        assert_eq!(scene(&settings, true, true, false), bounded, "desktop mouselook retains the scene");
        assert_eq!(scene(&settings.for_hosted_worker(true), false, false, false), bounded);
        for full in [
            scene(&settings, false, true, false),
            scene(&settings, false, false, true),
            scene(&ViewportSettings::default(), false, false, false),
        ] {
            assert_eq!(full.view, aurora_render::WorldViewport::full(native));
        }
        assert_eq!(
            settings.render_viewport(native, false, false, true, false),
            aurora_render::WorldViewport::full(native),
            "a clean output render target is independently full-sized"
        );
        assert_eq!(settings.viewport(native), bounded.view, "temporary policies never rewrite margins");
    }

    #[test]
    fn photo_focus_uses_absolute_native_pixels_and_logical_scene_offsets_at_any_ui_scale() {
        let source = PhotoScene::new(
            (5000, 3000),
            aurora_render::WorldViewport {
                x: 2000,
                y: 300,
                width: 2500,
                height: 2100,
            },
        );
        assert_eq!(source.point(None), (3250.0, 1350.0));
        assert_eq!(source.point(Some(egui::pos2(f32::NAN, 0.2))), source.point(None));
        assert_eq!(source.point(Some(egui::pos2(1.0, 1.0))), (4499.5, 2399.5));
        for native in [(2000.0, 300.0), (3250.0, 1350.0), (4499.0, 2399.0)] {
            let normalized = source.normalized_point(native).unwrap();
            let reconstructed = source.point(Some(normalized));
            assert!((native.0 - reconstructed.0).abs() < 0.001 && (native.1 - reconstructed.1).abs() < 0.001);
            for ppp in [1.0, 1.5, 2.5] {
                let origin = egui::pos2(17.0, 29.0);
                let canvas = egui::Rect::from_min_size(origin, egui::vec2(5000.0, 3000.0) / ppp);
                let scene = source.logical_rect(canvas);
                let marker = scene.min + normalized.to_vec2() * scene.size();
                let expected = origin + egui::vec2(native.0, native.1) / ppp;
                assert!(
                    marker.distance(expected) < 0.001,
                    "AF marker and renderer pick share the world origin"
                );
                assert!(scene.min.distance(origin + egui::vec2(2000.0, 300.0) / ppp) < 0.001);
            }
        }
        for outside in [
            (1999.0, 1350.0),
            (4500.0, 1350.0),
            (3250.0, 299.0),
            (3250.0, 2400.0),
            (f32::NAN, 1350.0),
        ] {
            assert!(
                source.normalized_point(outside).is_none(),
                "workspace input cannot become a world AF point"
            );
        }
        let output = source.output_viewport((2000, 1200));
        assert_eq!(output.rect(), [800, 120, 1000, 840]);
        assert_eq!(
            source.native,
            (5000, 3000),
            "export size cannot change frozen source aspect or picks"
        );
        assert!((source.aspect() - output.width as f32 / output.height as f32).abs() < 1e-6);
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
