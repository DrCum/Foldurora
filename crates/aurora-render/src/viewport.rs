//! A world view inside the native window. All rectangles and picks use
//! physical pixels, independently of the interface's scale or monitor DPI.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldViewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Embed a local world projection into the full output's clip space. UI
/// projections use this; the world GPU targets retain the local projection.
pub fn window_matrix(view: WorldViewport, size: (u32, u32)) -> glam::Mat4 {
    let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
    glam::Mat4::from_cols(
        glam::Vec4::new(view.width as f32 / w, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, view.height as f32 / h, 0.0, 0.0),
        glam::Vec4::Z,
        glam::Vec4::new(
            (2.0 * view.x as f32 + view.width as f32) / w - 1.0,
            1.0 - (2.0 * view.y as f32 + view.height as f32) / h,
            0.0,
            1.0,
        ),
    )
}

impl WorldViewport {
    pub fn full(size: (u32, u32)) -> Self {
        Self {
            x: 0,
            y: 0,
            width: size.0.max(1),
            height: size.1.max(1),
        }
    }

    /// Keep a saved or caller-provided view inside the current output.
    pub fn fitted(self, size: (u32, u32)) -> Self {
        let full = Self::full(size);
        let x = self.x.min(full.width - 1);
        let y = self.y.min(full.height - 1);
        Self {
            x,
            y,
            width: self.width.max(1).min(full.width - x),
            height: self.height.max(1).min(full.height - y),
        }
    }

    /// Preserve a scene's position inside an interface capture. Like egui,
    /// scale both axes by the output/native height ratio, then round edges.
    /// The caller admits only matching canvas proportions (integer rounding
    /// aside); a full output target does not imply a full scene rectangle.
    pub fn scaled(self, source: (u32, u32), output: (u32, u32)) -> Self {
        let source = (source.0.max(1), source.1.max(1));
        let output = (output.0.max(1), output.1.max(1));
        let view = self.fitted(source);
        if view == Self::full(source) {
            return Self::full(output);
        }
        let edge = |value: u32, from: u32, to: u32| {
            ((u64::from(value) * u64::from(to) + u64::from(from) / 2) / u64::from(from)).min(u64::from(u32::MAX)) as u32
        };
        let x = edge(view.x, source.1, output.1);
        let y = edge(view.y, source.1, output.1);
        let right = edge(view.x + view.width, source.1, output.1);
        let bottom = edge(view.y + view.height, source.1, output.1);
        Self {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        }
        .fitted(output)
    }

    /// Margins are fractions in left, right, top, bottom order. Opposite
    /// margins scale together if needed to retain at least 5% of the view.
    pub fn from_margins(size: (u32, u32), margins: [f32; 4]) -> Self {
        let pair = |a: f32, b: f32| {
            let clean = |v: f32| if v.is_finite() { v.clamp(0.0, 0.95) } else { 0.0 };
            let (a, b) = (clean(a), clean(b));
            let scale = if a + b > 0.95 { 0.95 / (a + b) } else { 1.0 };
            (a * scale, b * scale)
        };
        let full = Self::full(size);
        let (left, right) = pair(margins[0], margins[1]);
        let (top, bottom) = pair(margins[2], margins[3]);
        let x = (left * full.width as f32).round() as u32;
        let y = (top * full.height as f32).round() as u32;
        Self {
            x,
            y,
            width: full
                .width
                .saturating_sub(x)
                .saturating_sub((right * full.width as f32).round() as u32),
            height: full
                .height
                .saturating_sub(y)
                .saturating_sub((bottom * full.height as f32).round() as u32),
        }
        .fitted(size)
    }

    pub fn size(self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn rect(self) -> [u32; 4] {
        [self.x, self.y, self.width, self.height]
    }
    pub fn area(self) -> [f32; 4] {
        [
            self.x as f32,
            self.y as f32,
            (self.x + self.width) as f32,
            (self.y + self.height) as f32,
        ]
    }

    pub fn local(self, x: f32, y: f32) -> Option<(f32, f32)> {
        let (x, y) = (x - self.x as f32, y - self.y as f32);
        (x >= 0.0 && y >= 0.0 && x < self.width as f32 && y < self.height as f32).then_some((x, y))
    }

    /// Window pixels to the possibly downscaled scene's texels. An invalid
    /// pick stays invalid instead of being clamped onto the world's edge.
    pub fn pixel(self, pixel: (u32, u32), scene: (u32, u32)) -> (u32, u32) {
        if pixel.0 < self.x || pixel.1 < self.y || pixel.0 - self.x >= self.width || pixel.1 - self.y >= self.height {
            return (u32::MAX, u32::MAX);
        }
        crate::rlva::resolution::pixel((pixel.0 - self.x, pixel.1 - self.y), self.size(), scene)
    }

    pub fn project(self, ndc: [f32; 2]) -> [f32; 2] {
        [
            self.x as f32 + (ndc[0] * 0.5 + 0.5) * self.width as f32,
            self.y as f32 + (0.5 - ndc[1] * 0.5) * self.height as f32,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_photo_scales_asymmetric_world_margins_with_the_whole_canvas() {
        let native = (5000, 3000);
        let output = (2000, 1200);
        let source = WorldViewport {
            x: 1800,
            y: 300,
            width: 2400,
            height: 2100,
        };
        let view = source.scaled(native, output);
        assert_eq!(view.rect(), [720, 120, 960, 840]);
        // The scene and full-canvas overlays must agree at corners and centre.
        // Output dimensions remain the whole workspace, including its margins.
        for p in [[-1.0, 1.0], [0.0, 0.0], [1.0, -1.0], [-0.4, 0.6]] {
            let native_pixel = source.project(p);
            let photo_pixel = view.project(p);
            for axis in 0..2 {
                assert!((photo_pixel[axis] - native_pixel[axis] * 0.4).abs() < 0.001);
            }
        }
        for size in [(2000, 1200), (1, 1), (0, 0)] {
            assert_eq!(WorldViewport::full(native).scaled(native, size), WorldViewport::full(size));
        }
        let corrupt = WorldViewport {
            x: u32::MAX,
            y: u32::MAX,
            width: u32::MAX,
            height: 0,
        };
        let fitted = corrupt.scaled((5000, 3000), (640, 480));
        assert_eq!(fitted.rect(), [639, 479, 1, 1]);
        // This preview has an unavoidable rounded short side. Match egui's
        // height-based transform, rather than independently scaling X.
        let right = WorldViewport {
            x: 1920,
            y: 0,
            width: 960,
            height: 1080,
        };
        assert_eq!(right.scaled((3840, 1080), (719, 202)).rect(), [359, 0, 180, 202]);
        assert_eq!(
            WorldViewport::full((3840, 1080)).scaled((3840, 1080), (719, 202)),
            WorldViewport::full((719, 202))
        );
    }

    #[test]
    fn snapshot_scene_hint_is_bounded_and_only_changes_interface_output() {
        let mut spec = crate::SnapshotSpec {
            width: 2000,
            height: 1200,
            interface: true,
            huds: true,
            photo: (2000, 1200),
            world_viewport: None,
        };
        assert_eq!(spec.output_viewport(), WorldViewport::full((2000, 1200)));
        spec.world_viewport = Some(WorldViewport {
            x: 720,
            y: 120,
            width: 960,
            height: 840,
        });
        assert_eq!(spec.output_viewport().rect(), [720, 120, 960, 840]);
        spec.world_viewport = Some(WorldViewport {
            x: 1999,
            y: u32::MAX,
            width: u32::MAX,
            height: 0,
        });
        assert_eq!(spec.output_viewport().rect(), [1999, 1199, 1, 1]);
        spec.interface = false;
        assert_eq!(
            spec.output_viewport(),
            WorldViewport::full((2000, 1200)),
            "clean exports use all output pixels"
        );
        assert_eq!(
            (spec.width, spec.height),
            spec.photo,
            "scene placement never changes export dimensions"
        );
    }

    #[test]
    fn offset_world_view_maps_picks_and_projection_in_the_same_physical_pixels() {
        let view = WorldViewport {
            x: 1920,
            y: 240,
            width: 2560,
            height: 1440,
        };
        assert_eq!(view.project([0.0, 0.0]), [3200.0, 960.0]);
        assert_eq!(view.local(3200.0, 960.0), Some((1280.0, 720.0)));
        assert_eq!(view.pixel((3200, 960), (1280, 720)), (640, 360));
        assert_eq!(view.pixel((4479, 1679), (1280, 720)), (1279, 719));
        for p in [(1919, 960), (3200, 239), (4480, 960), (3200, 1680)] {
            assert_eq!(view.pixel(p, (1280, 720)), (u32::MAX, u32::MAX));
            assert_eq!(view.local(p.0 as f32, p.1 as f32), None);
        }
        assert_eq!(view.local(f32::NAN, 960.0), None);
    }

    #[test]
    fn label_clip_projection_matches_world_pixels_with_a_nonzero_origin() {
        let size = (5120, 2160);
        let view = WorldViewport {
            x: 1920,
            y: 240,
            width: 2560,
            height: 1440,
        };
        let embed = window_matrix(view, size);
        for [x, y] in [[-1.0, -1.0], [0.0, 0.0], [1.0, 1.0], [0.3, -0.7]] {
            let clip = embed * glam::Vec4::new(x * 2.0, y * 2.0, 0.8, 2.0);
            let window = [
                (clip.x / clip.w * 0.5 + 0.5) * size.0 as f32,
                (0.5 - clip.y / clip.w * 0.5) * size.1 as f32,
            ];
            let local = view.project([x, y]);
            assert!((window[0] - local[0]).abs() < 0.001);
            assert!((window[1] - local[1]).abs() < 0.001);
            assert_eq!((clip.z, clip.w), (0.8, 2.0));
        }
    }

    #[test]
    fn margin_pairs_retain_a_usable_view_and_survive_resize_and_mixed_dpi() {
        for size in [(3840, 1080), (7680, 2160), (1280, 720), (1, 1), (0, 0)] {
            let view = WorldViewport::from_margins(size, [0.0, 0.5, 0.0, 0.0]);
            assert_eq!(view.width, (size.0.max(1) / 2).max(1));
            assert_eq!(view.height, size.1.max(1));
            let tiny = WorldViewport::from_margins(size, [0.9, 0.9, 0.9, 0.9]);
            assert!(tiny.width >= ((size.0 as f32 * 0.05).floor() as u32).max(1));
            assert!(tiny.height >= ((size.1 as f32 * 0.05).floor() as u32).max(1));
            assert!(tiny.x + tiny.width <= size.0.max(1));
            assert!(tiny.y + tiny.height <= size.1.max(1));
        }
        assert_eq!(
            WorldViewport::from_margins((800, 600), [f32::NAN, f32::INFINITY, -1.0, 0.0]),
            WorldViewport::full((800, 600))
        );
    }

    #[test]
    fn a_removed_display_or_corrupt_rectangle_never_places_a_view_outside_the_output() {
        let old = WorldViewport {
            x: u32::MAX,
            y: 1200,
            width: u32::MAX,
            height: 900,
        };
        assert_eq!(
            old.fitted((1920, 1080)),
            WorldViewport {
                x: 1919,
                y: 1079,
                width: 1,
                height: 1
            }
        );
        assert_eq!(WorldViewport::full((0, 0)).size(), (1, 1));
    }
}
