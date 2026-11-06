//! Windows presentation modifies only the controller's owned HWND. The
//! separately acknowledged worker remains an unowned ordinary top-level window.

use aurora_session::HostedViewport;
#[cfg(any(windows, test))]
use aurora_session::PhysicalRect;
use winit::window::Window;

#[derive(Default)]
pub(super) struct Region {
    #[cfg(windows)]
    applied: Option<Shape>,
}

impl Region {
    pub fn close(&mut self, window: &Window) -> anyhow::Result<bool> {
        self.set(window, None, egui::Rect::NOTHING, 1.0)
    }

    /// `viewport` is present only after the broker's exact native/GPU grant
    /// agrees with current local geometry. Removing the region never revokes
    /// or fabricates the worker's render/input ownership.
    pub fn set(&mut self, window: &Window, viewport: Option<HostedViewport>, world: egui::Rect, scale: f32) -> anyhow::Result<bool> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, RGN_DIFF, SetWindowRgn};
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let RawWindowHandle::Win32(handle) = window.window_handle()?.as_raw() else {
                anyhow::bail!("controller has no Win32 window");
            };
            let hwnd = handle.hwnd.get() as windows_sys::Win32::Foundation::HWND;
            let shape = viewport
                .map(|viewport| -> anyhow::Result<Option<Shape>> {
                    let (outer, client) = geometry(hwnd)?;
                    Ok(current_shape(handle.hwnd.get() as u64, outer, client, viewport, world, scale))
                })
                .transpose()?
                .flatten();
            if shape == self.applied {
                return Ok(false);
            }
            // SAFETY: the HWND is this controller's live owned window. No
            // foreign parent, owner, styles or global z-order are modified.
            unsafe {
                if let Some(shape) = shape {
                    let full = CreateRectRgn(0, 0, shape.width, shape.height);
                    let cut = CreateRectRgn(shape.hole.x, shape.hole.y, shape.right(), shape.bottom());
                    if full.is_null() || cut.is_null() {
                        if !full.is_null() {
                            DeleteObject(full);
                        }
                        if !cut.is_null() {
                            DeleteObject(cut);
                        }
                        anyhow::bail!("could not allocate controller window region");
                    }
                    let combined = CombineRgn(full, full, cut, RGN_DIFF);
                    DeleteObject(cut);
                    if combined == 0 || SetWindowRgn(hwnd, full, 1) == 0 {
                        // Ownership transfers to Windows only on success.
                        DeleteObject(full);
                        anyhow::bail!("could not apply controller window region");
                    }
                } else if SetWindowRgn(hwnd, std::ptr::null_mut(), 1) == 0 {
                    anyhow::bail!("could not restore controller window region");
                }
            }
            self.applied = shape;
            Ok(true)
        }
        #[cfg(not(windows))]
        {
            let _ = (window, viewport, world, scale);
            Ok(false)
        }
    }
}

#[cfg(windows)]
fn geometry(hwnd: windows_sys::Win32::Foundation::HWND) -> anyhow::Result<(PhysicalRect, PhysicalRect)> {
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect};
    let mut outer = RECT::default();
    let mut client = RECT::default();
    let mut origin = POINT::default();
    // SAFETY: own live HWND and valid writable stack structures. All queried
    // rectangles use the viewer's existing per-monitor-aware native DPI mode.
    unsafe {
        anyhow::ensure!(
            GetWindowRect(hwnd, &mut outer) != 0 && GetClientRect(hwnd, &mut client) != 0 && ClientToScreen(hwnd, &mut origin) != 0,
            "could not read controller native geometry"
        );
    }
    let rectangle = |x, y, right: i32, bottom: i32| -> anyhow::Result<PhysicalRect> {
        let rect = PhysicalRect {
            x,
            y,
            width: u32::try_from(i64::from(right) - i64::from(x))?,
            height: u32::try_from(i64::from(bottom) - i64::from(y))?,
        };
        rect.validate()?;
        Ok(rect)
    };
    let outer = rectangle(outer.left, outer.top, outer.right, outer.bottom)?;
    let client = PhysicalRect {
        x: origin.x,
        y: origin.y,
        width: u32::try_from(i64::from(client.right) - i64::from(client.left))?,
        height: u32::try_from(i64::from(client.bottom) - i64::from(client.top))?,
    };
    client.validate()?;
    Ok((outer, client))
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shape {
    host_window: u64,
    width: i32,
    height: i32,
    hole: PhysicalRect,
}

#[cfg(any(windows, test))]
impl Shape {
    fn right(self) -> i32 {
        self.hole.x + self.hole.width as i32
    }

    fn bottom(self) -> i32 {
        self.hole.y + self.hole.height as i32
    }
}

/// Window regions include native decorations. Convert the acknowledged
/// desktop rectangle to outer-window coordinates without cutting the caption,
/// borders or any pixel outside the current native client.
#[cfg(any(windows, test))]
fn hole_shape(host_window: u64, outer: PhysicalRect, client: PhysicalRect, viewport: HostedViewport) -> Option<Shape> {
    outer.validate().ok()?;
    client.validate().ok()?;
    viewport.validate().ok()?;
    let contains = |a: PhysicalRect, b: PhysicalRect| {
        b.x >= a.x
            && b.y >= a.y
            && i64::from(b.x) + i64::from(b.width) <= i64::from(a.x) + i64::from(a.width)
            && i64::from(b.y) + i64::from(b.height) <= i64::from(a.y) + i64::from(a.height)
    };
    if host_window != viewport.host_window || !contains(outer, client) || !contains(client, viewport.rect) {
        return None;
    }
    let shape = Shape {
        host_window,
        width: i32::try_from(outer.width).ok()?,
        height: i32::try_from(outer.height).ok()?,
        hole: PhysicalRect {
            x: i32::try_from(i64::from(viewport.rect.x) - i64::from(outer.x)).ok()?,
            y: i32::try_from(i64::from(viewport.rect.y) - i64::from(outer.y)).ok()?,
            width: viewport.rect.width,
            height: viewport.rect.height,
        },
    };
    shape.hole.validate().ok()?;
    Some(shape)
}

/// Recheck current native client coordinates at the final region operation:
/// merely containing the old viewport is insufficient after a small move.
#[cfg(any(windows, test))]
fn current_shape(
    host_window: u64,
    outer: PhysicalRect,
    client: PhysicalRect,
    viewport: HostedViewport,
    world: egui::Rect,
    scale: f32,
) -> Option<Shape> {
    let current = super::software::physical_viewport(host_window, (client.x, client.y), [client.width, client.height], world, scale)?;
    if current != viewport {
        return None;
    }
    hole_shape(host_window, outer, client, viewport)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: u32, height: u32) -> PhysicalRect {
        PhysicalRect { x, y, width, height }
    }

    #[test]
    fn host_cutout_preserves_decorations_at_negative_desktop_origins() {
        let outer = rect(-1930, -160, 1620, 1240);
        let client = rect(-1920, -120, 1600, 1190);
        let viewport = HostedViewport {
            host_window: 7,
            rect: rect(-1559, -41, 1140, 972),
        };
        let shape = hole_shape(7, outer, client, viewport).unwrap();
        assert_eq!(shape.width, 1620);
        assert_eq!(shape.height, 1240);
        assert_eq!(shape.hole, rect(371, 119, 1140, 972));
        assert_eq!(shape.right(), 1511);
        assert_eq!(shape.bottom(), 1091);
    }

    #[test]
    fn host_cutout_refuses_old_hwnd_outside_client_and_invalid_native_bounds() {
        let outer = rect(100, 100, 820, 640);
        let client = rect(110, 130, 800, 600);
        let viewport = HostedViewport {
            host_window: 7,
            rect: client,
        };
        assert!(hole_shape(8, outer, client, viewport).is_none());
        assert!(hole_shape(7, client, outer, viewport).is_none());
        assert!(hole_shape(7, outer, client, HostedViewport { rect: outer, ..viewport }).is_none());
        assert!(
            hole_shape(
                7,
                outer,
                client,
                HostedViewport {
                    rect: rect(110, 130, 0, 600),
                    ..viewport
                }
            )
            .is_none()
        );
        assert!(hole_shape(7, rect(i32::MAX, 0, 820, 640), client, viewport).is_none());
    }

    #[test]
    fn host_cutout_rechecks_native_origin_and_dpi_instead_of_only_containment() {
        let outer = rect(-1930, -160, 1620, 1240);
        let client = rect(-1920, -120, 1600, 1190);
        let world = egui::Rect::from_min_max(egui::pos2(240.25, 52.25), egui::pos2(1000.75, 700.75));
        let viewport =
            super::super::software::physical_viewport(7, (client.x, client.y), [client.width, client.height], world, 1.5).unwrap();
        assert!(current_shape(7, outer, client, viewport, world, 1.5).is_some());
        let moved_outer = rect(outer.x + 10, outer.y, outer.width, outer.height);
        let moved_client = rect(client.x + 10, client.y, client.width, client.height);
        assert!(
            hole_shape(7, moved_outer, moved_client, viewport).is_some(),
            "old rectangle still fits the moved client"
        );
        assert!(current_shape(7, moved_outer, moved_client, viewport, world, 1.5).is_none());
        assert!(current_shape(7, outer, client, viewport, world, 1.25).is_none());
    }
}
