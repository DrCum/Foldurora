//! The controller owns a CPU egui surface, never a world Renderer or adapter.
//! Reuse the recovery painter without its crash/report workflow.

use aurora_session::HostedViewport;
#[cfg(any(windows, test))]
use aurora_session::PhysicalRect;
use egui::{Color32, Context, RawInput, Rect};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, OwnedDisplayHandle};
use winit::window::Window;

pub(super) struct SoftwareWindow {
    pub window: Arc<Window>,
    input: egui_winit::State,
    surface: softbuffer::Surface<OwnedDisplayHandle, Arc<Window>>,
    painter: crate::crash::software::Painter,
    pending: egui::TexturesDelta,
}

impl SoftwareWindow {
    pub fn new(event_loop: &ActiveEventLoop, context: &Context, title: String) -> anyhow::Result<Self> {
        let window = Arc::new(
            event_loop.create_window(
                crate::cli::with_desktop_id(Window::default_attributes())
                    .with_title(title)
                    .with_inner_size(winit::dpi::LogicalSize::new(1240.0, 800.0))
                    .with_min_inner_size(winit::dpi::LogicalSize::new(760.0, 520.0)),
            )?,
        );
        let display = softbuffer::Context::new(event_loop.owned_display_handle()).map_err(|error| anyhow::anyhow!("{error}"))?;
        let surface = softbuffer::Surface::new(&display, window.clone()).map_err(|error| anyhow::anyhow!("{error}"))?;
        let input = egui_winit::State::new(
            context.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        Ok(Self {
            window,
            input,
            surface,
            painter: Default::default(),
            pending: Default::default(),
        })
    }

    pub fn event(&mut self, event: &WindowEvent) -> egui_winit::EventResponse {
        self.input.on_window_event(&self.window, event)
    }

    pub fn raw_input(&mut self) -> RawInput {
        self.input.take_egui_input(&self.window)
    }

    /// Zero-sized minimized surfaces neither allocate nor overwrite layout.
    pub fn present(
        &mut self,
        event_loop: &ActiveEventLoop,
        context: &Context,
        mut output: egui::FullOutput,
        background: Color32,
    ) -> anyhow::Result<Duration> {
        self.pending.append(std::mem::take(&mut output.textures_delta));
        let delay = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(Duration::MAX, |output| output.repaint_delay);
        self.input
            .handle_platform_output_with_event_loop(&self.window, event_loop, output.platform_output);
        let size = self.window.inner_size();
        let (Some(width), Some(height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return Ok(Duration::from_millis(250));
        };
        let primitives = context.tessellate(output.shapes, output.pixels_per_point);
        self.surface.resize(width, height).map_err(|error| anyhow::anyhow!("{error}"))?;
        let mut pixels = self.surface.buffer_mut().map_err(|error| anyhow::anyhow!("{error}"))?;
        self.painter.paint(
            &mut pixels,
            [size.width as usize, size.height as usize],
            &primitives,
            &self.pending,
            output.pixels_per_point,
            background,
        );
        self.pending.clear();
        pixels.present().map_err(|error| anyhow::anyhow!("{error}"))?;
        Ok(delay)
    }

    /// Only an owned, visible Windows HWND can be a hosting target. Other
    /// platforms retain ordinary independent worker windows at this stage.
    pub fn viewport(&self, area: Rect, pixels_per_point: f32) -> Option<HostedViewport> {
        #[cfg(windows)]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if self.window.is_visible() != Some(true) || self.window.is_minimized() != Some(false) {
                return None;
            }
            let RawWindowHandle::Win32(handle) = self.window.window_handle().ok()?.as_raw() else {
                return None;
            };
            let origin = self.window.inner_position().ok()?;
            let size = self.window.inner_size();
            physical_viewport(
                handle.hwnd.get() as u64,
                (origin.x, origin.y),
                [size.width, size.height],
                area,
                pixels_per_point,
            )
        }
        #[cfg(not(windows))]
        {
            let _ = (area, pixels_per_point);
            None
        }
    }
}

impl Drop for SoftwareWindow {
    fn drop(&mut self) {
        // A minimized/error surface may still hold an unapplied font upload.
        self.pending.clear();
    }
}

/// Absolute desktop pixels, including negative display origins. Round inward
/// so a fractional-DPI worker cannot cover the controller's last chrome pixel.
#[cfg(any(windows, test))]
pub(super) fn physical_viewport(host_window: u64, origin: (i32, i32), native: [u32; 2], area: Rect, scale: f32) -> Option<HostedViewport> {
    if host_window == 0 || native.contains(&0) || !area.is_finite() || !area.is_positive() || !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let edge = |value: f32, limit: u32, start: bool| {
        let value = f64::from(value) * f64::from(scale);
        let value = if start { value.ceil() } else { value.floor() };
        value.clamp(0.0, f64::from(limit)) as u32
    };
    let (left, top) = (edge(area.left(), native[0], true), edge(area.top(), native[1], true));
    let (right, bottom) = (edge(area.right(), native[0], false), edge(area.bottom(), native[1], false));
    let rect = PhysicalRect {
        x: i32::try_from(i64::from(origin.0) + i64::from(left)).ok()?,
        y: i32::try_from(i64::from(origin.1) + i64::from(top)).ok()?,
        width: right.checked_sub(left)?,
        height: bottom.checked_sub(top)?,
    };
    let viewport = HostedViewport { host_window, rect };
    viewport.validate().ok()?;
    Some(viewport)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_viewport_uses_absolute_negative_origins_and_fractional_dpi() {
        let area = Rect::from_min_max(egui::pos2(240.25, 52.25), egui::pos2(1000.75, 700.75));
        let viewport = physical_viewport(7, (-1920, -120), [1600, 1200], area, 1.5).unwrap();
        assert_eq!(viewport.host_window, 7);
        assert_eq!(
            viewport.rect,
            PhysicalRect {
                x: -1559,
                y: -41,
                width: 1140,
                height: 972
            }
        );
        assert!(f64::from(viewport.rect.x - -1920) >= f64::from(area.left()) * 1.5);
        assert!(f64::from(viewport.rect.x - -1920 + viewport.rect.width as i32) <= f64::from(area.right()) * 1.5);
    }

    #[test]
    fn controller_viewport_clips_to_client_and_refuses_unknown_empty_or_overflow_geometry() {
        let full = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let clipped = physical_viewport(7, (100, 200), [800, 600], full.expand(80.0), 1.0).unwrap();
        assert_eq!(
            clipped.rect,
            PhysicalRect {
                x: 100,
                y: 200,
                width: 800,
                height: 600
            }
        );
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(physical_viewport(7, (0, 0), [800, 600], full, scale).is_none());
        }
        assert!(physical_viewport(0, (0, 0), [800, 600], full, 1.0).is_none());
        assert!(physical_viewport(7, (0, 0), [0, 600], full, 1.0).is_none());
        assert!(physical_viewport(7, (i32::MAX, 0), [800, 600], full, 1.0).is_none());
        assert!(physical_viewport(7, (0, 0), [800, 600], Rect::NOTHING, 1.0).is_none());
        assert!(physical_viewport(7, (0, 0), [800, 600], full.translate(egui::vec2(1000.0, 0.0)), 1.0).is_none());
    }
}
