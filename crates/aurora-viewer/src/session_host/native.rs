//! Windows presentation modifies only the controller's owned HWND. The
//! separately acknowledged worker remains an unowned ordinary top-level window.

use aurora_session::HostedViewport;
#[cfg(any(windows, test))]
use aurora_session::PhysicalRect;
use winit::window::Window;

const MAX_OVERLAYS: usize = 8;

/// Only live CPU popup pixels cover the acknowledged world. This is collected
/// in each UI pass, before egui drains its paint lists; remembered closed Areas
/// must not leave an invisible native hit target over a worker.
#[derive(Default)]
pub(super) struct Overlays {
    rects: Vec<egui::Rect>,
    cover_all: bool,
}

impl Overlays {
    pub fn collect(context: &egui::Context, world: egui::Rect) -> Self {
        let areas = context.memory(|memory| {
            memory
                .areas()
                .visible_layer_ids()
                .into_iter()
                .filter(|layer| matches!(layer.order, egui::Order::Foreground | egui::Order::Tooltip))
                .filter_map(|layer| Some((layer, memory.area_rect(layer.id)?)))
                .collect::<Vec<_>>()
        });
        let mut result = Self::default();
        for (layer, area) in areas {
            let painted = context.graphics(|graphics| {
                graphics.get(layer).map_or(egui::Rect::NOTHING, |list| {
                    list.all_entries().fold(egui::Rect::NOTHING, |bounds, entry| {
                        let shape = entry.transform_after_tessellation * entry.shape.visual_bounding_rect();
                        bounds.union(shape.intersect(entry.clip_rect))
                    })
                })
            });
            if !painted.is_positive() {
                continue;
            }
            let transform = context.layer_transform_to_global(layer).unwrap_or_default();
            let rect = (transform * painted.union(area)).intersect(world);
            if !rect.is_finite() {
                result.cover_all = true;
                break;
            }
            if rect.is_positive() {
                result.rects.push(rect);
            }
            if result.rects.len() > MAX_OVERLAYS {
                // A bounded conservative fallback, never a silently clipped
                // menu whose pixels or input fall through to the world.
                result.rects.clear();
                result.cover_all = true;
                break;
            }
        }
        result.rects.sort_by(|a, b| {
            a.left()
                .total_cmp(&b.left())
                .then(a.top().total_cmp(&b.top()))
                .then(a.right().total_cmp(&b.right()))
                .then(a.bottom().total_cmp(&b.bottom()))
        });
        result.rects.dedup();
        result
    }
}

#[derive(Default)]
pub(super) struct Region {
    #[cfg(windows)]
    applied: Option<Applied>,
}

impl Region {
    pub fn close(&mut self, window: &Window) -> anyhow::Result<bool> {
        self.set(window, None, egui::Rect::NOTHING, 1.0, &Overlays::default())
    }

    /// `viewport` is present only after the broker's exact native/GPU grant
    /// agrees with current local geometry. Removing the region never revokes
    /// or fabricates the worker's render/input ownership.
    pub fn set(
        &mut self,
        window: &Window,
        viewport: Option<HostedViewport>,
        world: egui::Rect,
        scale: f32,
        overlays: &Overlays,
    ) -> anyhow::Result<bool> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, RGN_DIFF, RGN_OR, SetWindowRgn};
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let RawWindowHandle::Win32(handle) = window.window_handle()?.as_raw() else {
                anyhow::bail!("controller has no Win32 window");
            };
            let hwnd = handle.hwnd.get() as windows_sys::Win32::Foundation::HWND;
            let shape = viewport
                .map(|viewport| -> anyhow::Result<Option<Applied>> {
                    let (outer, client) = geometry(hwnd)?;
                    Ok(current_shape(handle.hwnd.get() as u64, outer, client, viewport, world, scale)
                        .and_then(|shape| protected_shape(shape, outer, client, scale, overlays)))
                })
                .transpose()?
                .flatten();
            if shape == self.applied {
                return Ok(false);
            }
            // SAFETY: the HWND is this controller's live owned window. No
            // foreign parent, owner, styles or global z-order are modified.
            unsafe {
                if let Some(applied) = &shape {
                    let shape = applied.shape;
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
                    let mut combined = CombineRgn(full, full, cut, RGN_DIFF);
                    DeleteObject(cut);
                    for overlay in &applied.overlays {
                        if combined == 0 {
                            break;
                        }
                        let island = CreateRectRgn(
                            overlay.x,
                            overlay.y,
                            overlay.x + overlay.width as i32,
                            overlay.y + overlay.height as i32,
                        );
                        if island.is_null() {
                            combined = 0;
                            break;
                        }
                        combined = CombineRgn(full, full, island, RGN_OR);
                        DeleteObject(island);
                    }
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
            let _ = (window, viewport, world, scale, overlays);
            Ok(false)
        }
    }
}

#[cfg(any(windows, test))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct Applied {
    shape: Shape,
    overlays: Vec<PhysicalRect>,
}

/// Popup coverage changes only this owned HWND's shape, never the worker's
/// acknowledged viewport or render/input grant. Round outward for hit targets;
/// the world hole itself continues to use the inward-rounded exact grant.
#[cfg(any(windows, test))]
fn protected_shape(shape: Shape, outer: PhysicalRect, client: PhysicalRect, scale: f32, overlays: &Overlays) -> Option<Applied> {
    if overlays.cover_all || !scale.is_finite() || scale <= 0.0 || overlays.rects.len() > MAX_OVERLAYS {
        return None;
    }
    let edge = |value: f32, limit: u32, start: bool| {
        let value = f64::from(value) * f64::from(scale);
        let value = if start { value.floor() - 1.0 } else { value.ceil() + 1.0 };
        value.clamp(0.0, f64::from(limit)) as i64
    };
    let mut islands = Vec::with_capacity(overlays.rects.len());
    for rect in &overlays.rects {
        if !rect.is_finite() || !rect.is_positive() {
            return None;
        }
        let left = edge(rect.left(), client.width, true);
        let top = edge(rect.top(), client.height, true);
        let right = edge(rect.right(), client.width, false);
        let bottom = edge(rect.bottom(), client.height, false);
        let island = PhysicalRect {
            x: i32::try_from(i64::from(client.x) - i64::from(outer.x) + left).ok()?,
            y: i32::try_from(i64::from(client.y) - i64::from(outer.y) + top).ok()?,
            width: u32::try_from(right - left).ok()?,
            height: u32::try_from(bottom - top).ok()?,
        };
        island.validate().ok()?;
        islands.push(island);
    }
    Some(Applied { shape, overlays: islands })
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

    #[test]
    fn host_popup_islands_round_outward_without_changing_the_exact_world_or_caption() {
        let outer = rect(-1928, -152, 1616, 1270);
        let client = rect(-1920, -120, 1600, 1200);
        let world = egui::Rect::from_min_max(egui::pos2(0.0, 38.0), egui::pos2(1066.6667, 800.0));
        let viewport = super::super::software::physical_viewport(7, (client.x, client.y), [1600, 1200], world, 1.5).unwrap();
        let shape = current_shape(7, outer, client, viewport, world, 1.5).unwrap();
        let overlays = Overlays {
            rects: vec![egui::Rect::from_min_max(egui::pos2(100.2, 40.1), egui::pos2(260.3, 210.4))],
            cover_all: false,
        };
        let applied = protected_shape(shape, outer, client, 1.5, &overlays).unwrap();
        assert_eq!(applied.shape, shape, "popup coverage cannot replace the acknowledged world");
        assert_eq!(applied.overlays, vec![rect(157, 91, 243, 258)]);
        let popup = applied.overlays[0];
        assert!(popup.y >= client.y - outer.y, "native caption remains outside popup coverage");
        assert!(popup.x + popup.width as i32 <= applied.shape.width);
        assert!(popup.y + popup.height as i32 <= applied.shape.height);

        let clipped = Overlays {
            rects: vec![egui::Rect::from_min_max(egui::pos2(-100.0, -100.0), egui::pos2(2000.0, 2000.0))],
            cover_all: false,
        };
        assert_eq!(
            protected_shape(shape, outer, client, 1.5, &clipped).unwrap().overlays,
            vec![rect(8, 32, 1600, 1200)],
            "coverage stays inside this owned native client"
        );
        for invalid in [
            Overlays {
                rects: Vec::new(),
                cover_all: true,
            },
            Overlays {
                rects: vec![world; MAX_OVERLAYS + 1],
                cover_all: false,
            },
            Overlays {
                rects: vec![egui::Rect::NAN],
                cover_all: false,
            },
        ] {
            assert!(protected_shape(shape, outer, client, 1.5, &invalid).is_none());
        }
        assert!(protected_shape(shape, outer, client, 0.0, &overlays).is_none());
    }

    #[test]
    fn host_popup_islands_follow_real_combo_paint_and_disappear_when_it_closes() {
        let context = egui::Context::default();
        context.all_styles_mut(|style| style.visuals.popup_shadow = egui::epaint::Shadow::NONE);
        let size = egui::vec2(800.0, 600.0);
        let world = egui::Rect::from_min_max(egui::pos2(0.0, 38.0), size.to_pos2());
        let mut frame = 0;
        let mut selected = 0;
        let mut show = |events| {
            frame += 1;
            let mut overlays = Overlays::default();
            let mut button = egui::Rect::NOTHING;
            let _ = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    time: Some(f64::from(frame) / 60.0),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::Panel::top("strip").exact_size(38.0).show(ui, |ui| {
                        button = egui::ComboBox::from_id_salt("profiles")
                            .selected_text("Character 1")
                            .width(200.0)
                            .show_ui(ui, |ui| {
                                for index in 0..5 {
                                    ui.selectable_value(&mut selected, index, format!("Character {}", index + 1));
                                }
                            })
                            .response
                            .rect;
                    });
                    overlays = Overlays::collect(ui.ctx(), world);
                },
            );
            (button, overlays)
        };
        let mut button = egui::Rect::NOTHING;
        for _ in 0..4 {
            (button, _) = show(Vec::new());
        }
        for pressed in [true, false] {
            show(vec![
                egui::Event::PointerMoved(button.center()),
                egui::Event::PointerButton {
                    pos: button.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        let mut visible = false;
        for _ in 0..6 {
            let (_, overlays) = show(Vec::new());
            assert!(!overlays.cover_all);
            visible |= !overlays.rects.is_empty();
            for rect in overlays.rects {
                assert!(world.contains_rect(rect));
                assert!(
                    rect.width() < world.width() / 2.0,
                    "a compact popup must preserve the surrounding live world"
                );
            }
        }
        assert!(visible, "real popup paint below the strip needs a native island");
        egui::Popup::close_all(&context);
        let (_, closed) = show(vec![egui::Event::PointerGone]);
        assert!(
            closed.rects.is_empty(),
            "remembered closed Area is not current paint or input coverage"
        );
        assert!(!closed.cover_all);
    }

    #[test]
    fn host_overlay_collection_covers_live_tooltips_and_bounds_excess_area_count() {
        let context = egui::Context::default();
        let world = egui::Rect::from_min_max(egui::pos2(0.0, 38.0), egui::pos2(800.0, 600.0));
        let mut frame = 0;
        let mut show = |count: usize, order| {
            frame += 1;
            let mut overlays = Overlays::default();
            let _ = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                    time: Some(f64::from(frame) / 60.0),
                    ..Default::default()
                },
                |ui| {
                    for index in 0..count {
                        egui::Area::new(egui::Id::unique(("owned_overlay", index)))
                            .order(order)
                            .fixed_pos(egui::pos2(10.0 + index as f32 * 50.0, 80.0))
                            .show(ui.ctx(), |ui| {
                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                    ui.label("Owned hint");
                                });
                            });
                    }
                    overlays = Overlays::collect(ui.ctx(), world);
                },
            );
            overlays
        };
        for _ in 0..4 {
            show(1, egui::Order::Tooltip);
        }
        let tooltip = show(1, egui::Order::Tooltip);
        assert_eq!(tooltip.rects.len(), 1);
        assert!(!tooltip.cover_all);
        assert!(show(0, egui::Order::Tooltip).rects.is_empty());
        for _ in 0..4 {
            show(MAX_OVERLAYS + 1, egui::Order::Foreground);
        }
        let excess = show(MAX_OVERLAYS + 1, egui::Order::Foreground);
        assert!(
            excess.cover_all,
            "excess overlays conservatively close coverage instead of truncating a menu"
        );
        assert!(excess.rects.is_empty());
    }

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
