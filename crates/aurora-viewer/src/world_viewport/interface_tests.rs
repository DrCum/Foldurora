use super::*;
use crate::ui::{layout, widgets};
use egui::{Id, Pos2, Rect, Vec2, pos2, vec2};

const WINDOW: &str = "viewport_interface_fixture";

fn inset() -> ViewportSettings {
    ViewportSettings {
        enabled: true,
        restrict_ui: true,
        left: 25.0,
        right: 10.0,
        top: 15.0,
        bottom: 20.0,
    }
}

fn close_rect(actual: Rect, expected: Rect, tolerance: f32) {
    assert!(
        (actual.min - expected.min).abs().max_elem() <= tolerance,
        "{actual:?} != {expected:?}"
    );
    assert!(
        (actual.max - expected.max).abs().max_elem() <= tolerance,
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn interface_canvas_uses_current_native_dpi_and_zoom_with_both_origins() {
    let settings = inset();
    let native = (3200, 2000);
    for dpi in [1.0, 1.25, 1.5, 2.0] {
        for zoom in [0.8, 1.0, 1.25] {
            let ppp = dpi * zoom;
            // The integration normally starts at zero. An offset makes an
            // accidental replacement with viewport-local coordinates visible.
            let screen = Rect::from_min_size(pos2(7.0, 11.0), vec2(3200.0, 2000.0) / ppp);
            let rect = settings.interface_rect(settings.viewport(native), native, screen).unwrap();
            let expected = Rect::from_min_size(screen.min + vec2(800.0, 300.0) / ppp, vec2(2080.0, 1300.0) / ppp);
            close_rect(rect, expected, 0.001);
        }
    }
}

#[test]
fn interface_canvas_minimum_uses_logical_units_and_keeps_reset_controls_reachable() {
    let settings = inset();
    let native = (1920, 1080);
    let view = aurora_render::WorldViewport {
        x: 1000,
        y: 200,
        width: 800,
        height: 500,
    };
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0) / 1.25);
    let rect = settings.interface_rect(view, native, screen).expect("640 × 400 logical points");
    close_rect(rect, Rect::from_min_size(pos2(800.0, 160.0), MIN_INTERFACE_SIZE), 0.001);
    for small in [
        aurora_render::WorldViewport { width: 799, ..view },
        aurora_render::WorldViewport { height: 499, ..view },
    ] {
        assert_eq!(settings.interface_rect(small, native, screen), None);
    }
    assert!(settings.restrict_ui, "fallback does not overwrite the saved preference");
}

#[test]
fn interface_canvas_is_optional_and_full_frame_modes_use_the_native_canvas() {
    let mut settings = inset();
    let native = (1600, 1000);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0));
    let view = settings.viewport(native);
    assert!(settings.interface_rect(view, native, screen).is_some());
    assert_eq!(
        settings.interface_rect(aurora_render::WorldViewport::full(native), native, screen),
        None
    );
    settings.restrict_ui = false;
    assert_eq!(settings.interface_rect(view, native, screen), None);
    settings.restrict_ui = true;
    settings.enabled = false;
    assert_eq!(settings.interface_rect(view, native, screen), None);
    settings.enabled = true;
    assert_eq!(settings.interface_rect(view, (0, 0), screen), None);
    for invalid in [Rect::NOTHING, Rect::from_min_size(Pos2::ZERO, vec2(0.0, 0.0))] {
        assert_eq!(settings.interface_rect(view, native, invalid), None);
    }
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Floater,
    Bare,
    LockedBare,
}

struct Interface {
    ctx: egui::Context,
    settings: ViewportSettings,
    dpi: f32,
    frame: usize,
    full_frame: bool,
    world_bars: bool,
    classic_room: bool,
    aspect: Option<(f32, egui::Vec2)>,
    kind: Kind,
    default_size: Vec2,
    popup: bool,
    button: Rect,
    top: Rect,
    bottom: Rect,
    popup_rect: Rect,
    clicks: usize,
}

impl Interface {
    const NATIVE: (u32, u32) = (3200, 2000);

    fn new(dpi: f32, zoom: f32) -> Self {
        Self::with_kind(dpi, zoom, Kind::Floater, vec2(260.0, 180.0))
    }

    fn with_kind(dpi: f32, zoom: f32, kind: Kind, default_size: Vec2) -> Self {
        let ctx = egui::Context::default();
        ctx.set_zoom_factor(zoom);
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        let mut this = Self {
            ctx,
            settings: inset(),
            dpi,
            frame: 0,
            full_frame: false,
            world_bars: true,
            classic_room: false,
            aspect: None,
            kind,
            default_size,
            popup: false,
            button: Rect::NOTHING,
            top: Rect::NOTHING,
            bottom: Rect::NOTHING,
            popup_rect: Rect::NOTHING,
            clicks: 0,
        };
        // Adopt the native scale and pending zoom before changing canvases,
        // just as an existing viewer does when its setting is toggled.
        this.settings.restrict_ui = false;
        this.frames(4);
        this
    }

    fn window(&self) -> Rect {
        self.ctx
            .memory(|memory| memory.area_rect(Id::unique(WINDOW)))
            .expect("fixture window")
    }

    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            self.show(Vec::new(), true);
        }
    }

    fn show(&mut self, events: Vec<egui::Event>, has_screen: bool) {
        self.frame += 1;
        let ppp = self.dpi * self.ctx.zoom_factor();
        let native_rect = Rect::from_min_size(Pos2::ZERO, vec2(3200.0, 2000.0) / ppp);
        let view = if self.full_frame {
            aurora_render::WorldViewport::full(Self::NATIVE)
        } else {
            self.settings.viewport(Self::NATIVE)
        };
        let requested = self.settings.enabled && self.settings.restrict_ui && !self.full_frame;
        let mut raw = egui::RawInput {
            screen_rect: has_screen.then_some(native_rect),
            time: Some(self.frame as f64 / 60.0),
            events,
            ..Default::default()
        };
        // Native metadata describes the actual OS window, independently of
        // the interface canvas. A cropped UI must leave it alone.
        let desktop_rect = Rect::from_min_size(pos2(-1920.0, 80.0), native_rect.size());
        let info = raw.viewports.entry(raw.viewport_id).or_default();
        info.native_pixels_per_point = Some(self.dpi);
        info.inner_rect = Some(desktop_rect);
        let interface_rect = raw
            .screen_rect
            .and_then(|screen| self.settings.interface_rect(view, Self::NATIVE, screen));
        let restricted = requested && (interface_rect.is_some() || !has_screen && layout::restricted_canvas(&self.ctx).is_some());
        if let Some(rect) = interface_rect {
            raw.screen_rect = Some(rect);
        }
        let p = crate::theme::Theme::default().palette();
        let mut clicked = false;
        let mut output = self.ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx().clone();
            if has_screen && (requested || layout::restricted_canvas(&ctx).is_some()) {
                let native = Rect::from_min_size(Pos2::ZERO, vec2(3200.0, 2000.0) / ctx.pixels_per_point());
                let expected = self.settings.interface_rect(view, Self::NATIVE, native).unwrap_or(native);
                if !layout::canvas_matches(&ctx, expected) {
                    ctx.request_repaint();
                    return;
                }
            }
            layout::begin_canvas(&ctx, restricted);
            assert_eq!(ctx.input(|input| input.viewport().inner_rect), Some(desktop_rect));
            if self.world_bars {
                self.top = egui::Panel::top("viewport_interface_top")
                    .exact_size(24.0)
                    .show(ui, |ui| ui.label("Menu"))
                    .response
                    .rect;
                self.bottom = egui::Panel::bottom("viewport_interface_bottom")
                    .exact_size(32.0)
                    .show(ui, |ui| ui.label("Toolbar and chat"))
                    .response
                    .rect;
                let room = if self.classic_room {
                    ctx.content_rect().shrink2(vec2(40.0, 30.0))
                } else {
                    ui.available_rect_before_wrap()
                };
                // Production bars report both rooms every World frame: the
                // islands allow movement across the full canvas, while the
                // classic skin keeps windows between its bars.
                layout::set_work_area(&ctx, room);
                layout::set_move_area(&ctx, if self.classic_room { room } else { ctx.content_rect() });
                layout::set_window_edges(&ctx, layout::Edges::OPEN);
                layout::frame(&ctx, &p, false);
            }
            let body = |ui: &mut egui::Ui| {
                let button = ui.button("Activate");
                self.button = button.rect;
                clicked |= button.clicked();
            };
            match self.kind {
                Kind::Floater => {
                    let mut floater = widgets::Floater::new(WINDOW, "Viewport fixture", pos2(16.0, 90.0), self.default_size)
                        .allow_partial_outside()
                        .silent();
                    if let Some((ratio, chrome)) = self.aspect {
                        floater = floater.keep_aspect(ratio, chrome);
                    }
                    floater.show(&ctx, &p, &mut true, body);
                }
                Kind::Bare | Kind::LockedBare => widgets::bare_window(
                    &ctx,
                    &p,
                    WINDOW,
                    "Map",
                    egui::Frame::NONE,
                    Rect::from_min_size(pos2(16.0, 90.0), self.default_size),
                    vec2(100.0, 80.0),
                    matches!(self.kind, Kind::LockedBare),
                    body,
                ),
            }
            if self.popup {
                let anchor = ctx.content_rect().right_bottom() - vec2(2.0, 2.0);
                if let Some(shown) = egui::Popup::new(Id::unique("viewport_interface_popup"), ctx, anchor, ui.layer_id()).show(|ui| {
                    ui.set_min_size(vec2(160.0, 100.0));
                    ui.label("Menu item");
                }) {
                    self.popup_rect = shown.response.rect;
                }
            }
        });
        self.clicks += usize::from(clicked);
        if restricted {
            let canvas = self.ctx.content_rect().expand(0.01);
            for shape in &output.shapes {
                assert!(
                    canvas.contains_rect(shape.clip_rect),
                    "paint escaped {canvas:?}: {:?}",
                    shape.clip_rect
                );
            }
        }
        output.textures_delta.clear();
    }

    fn click(&mut self, pos: Pos2) {
        self.show(vec![egui::Event::PointerMoved(pos)], true);
        for pressed in [true, false] {
            self.show(
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
                true,
            );
        }
    }

    fn drag_to(&mut self, start: Pos2, target: Pos2) {
        self.show(vec![egui::Event::PointerMoved(start)], true);
        self.show(
            vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            true,
        );
        self.show(vec![egui::Event::PointerMoved(target)], true);
        self.show(vec![egui::Event::PointerMoved(target)], true);
        self.show(
            vec![egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            true,
        );
        self.frames(4);
    }
}

#[test]
fn interface_egui_anchors_chrome_and_popups_inside_an_offset_fractional_dpi_canvas() {
    for (dpi, zoom) in [(1.0, 1.0), (1.25, 1.2), (1.5, 0.8), (2.0, 1.25)] {
        let mut f = Interface::new(dpi, zoom);
        f.settings.restrict_ui = true;
        f.popup = true;
        f.frames(5);
        let canvas = f.ctx.content_rect();
        let ppp = dpi * zoom;
        close_rect(
            canvas,
            Rect::from_min_size(pos2(800.0, 300.0) / ppp, vec2(2080.0, 1300.0) / ppp),
            1.0,
        );
        assert!(canvas.expand(1.0).contains_rect(f.top));
        assert!(canvas.expand(1.0).contains_rect(f.bottom));
        assert!((f.bottom.center().x - canvas.center().x).abs() < 1.0);
        assert!(canvas.expand(1.0).contains_rect(f.window()), "{:?}", f.window());
        assert!(canvas.expand(1.0).contains_rect(f.popup_rect), "{:?}", f.popup_rect);
        assert_eq!(layout::edges_for(&f.ctx, Id::unique(WINDOW)), layout::Edges::default());
    }
}

#[test]
fn interface_egui_keeps_absolute_pointer_input_and_does_not_own_the_old_panel_margin() {
    let mut f = Interface::new(1.5, 1.25);
    let old_button = f.button.center();
    f.settings.restrict_ui = true;
    f.frames(5);
    let button = f.button.center();
    assert!(f.ctx.content_rect().contains(button));
    assert!(!f.ctx.content_rect().contains(old_button));
    f.click(old_button);
    assert_eq!(f.clicks, 0);
    assert!(
        !f.ctx.is_pointer_over_egui(),
        "the unused monitor remains outside the interface hit regions"
    );
    f.click(button);
    assert_eq!(f.clicks, 1);
    assert_eq!(f.ctx.input(|input| input.pointer.latest_pos()), Some(button));
}

#[test]
fn interface_egui_toggling_and_temporary_full_frames_keep_saved_window_anchors() {
    let mut f = Interface::new(1.0, 1.0);
    let wide = f.window();
    f.settings.restrict_ui = true;
    f.frames(5);
    let restricted = f.window();
    f.full_frame = true;
    f.frames(5);
    close_rect(f.window(), wide, 1.0);
    f.full_frame = false;
    f.frames(5);
    close_rect(f.window(), restricted, 1.0);
    // A minimized native window supplies no new screen rect. Its last
    // valid canvas and anchor placement must survive unchanged.
    f.show(Vec::new(), false);
    close_rect(f.window(), restricted, 1.0);
    f.settings.restrict_ui = false;
    f.frames(5);
    close_rect(f.window(), wide, 1.0);
    assert_eq!(layout::edges_for(&f.ctx, Id::unique(WINDOW)), layout::Edges::OPEN);
}

#[test]
fn interface_login_floaters_without_a_bars_pass_keep_their_original_canvas_insets() {
    let mut f = Interface::with_kind(1.0, 1.0, Kind::Floater, vec2(2600.0, 900.0));
    // Login has neither World's bar geometry reporting nor layout::frame.
    // Start with fresh egui memory so its initial room is the default one.
    f.ctx = egui::Context::default();
    f.world_bars = false;
    f.frames(4);
    let wide = f.window();
    f.settings.restrict_ui = true;
    f.frames(5);
    assert!(f.ctx.content_rect().expand(1.0).contains_rect(f.window()));
    f.settings.restrict_ui = false;
    f.frames(5);
    close_rect(f.window(), wide, 1.0);
}

#[test]
fn interface_egui_relayout_cancels_a_release_against_the_previous_canvas() {
    let mut f = Interface::new(1.25, 1.0);
    let old = f.button.center();
    f.show(vec![egui::Event::PointerMoved(old)], true);
    f.show(
        vec![egui::Event::PointerButton {
            pos: old,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
        true,
    );
    f.settings.restrict_ui = true;
    f.show(
        vec![egui::Event::PointerButton {
            pos: old,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
        true,
    );
    assert_eq!(f.clicks, 0, "the release belongs to a rectangle that is no longer shown there");
    f.frames(3);
    f.click(f.button.center());
    assert_eq!(f.clicks, 1);
}

#[test]
fn interface_egui_pending_zoom_does_not_record_a_stale_startup_or_viewport_canvas() {
    let mut f = Interface::new(1.25, 1.0);
    f.ctx = egui::Context::default();
    f.ctx.set_zoom_factor(1.3);
    f.settings.restrict_ui = true;
    f.frames(6);
    assert!(f.ctx.content_rect().expand(1.0).contains_rect(f.window()));
    f.ctx.set_zoom_factor(2.0);
    f.settings.half(true);
    f.frames(6);
    let expected = Rect::from_min_size(pos2(640.0, 0.0), vec2(640.0, 800.0));
    close_rect(f.ctx.content_rect(), expected, 1.0);
    assert!(expected.expand(1.0).contains_rect(f.window()));
    // Increasing zoom across the safety threshold restores a usable native
    // interface area, rather than recording a tiny clamped layout.
    f.ctx.set_zoom_factor(2.5);
    f.frames(6);
    assert!(layout::restricted_canvas(&f.ctx).is_none());
    close_rect(f.ctx.content_rect(), Rect::from_min_size(Pos2::ZERO, vec2(1024.0, 640.0)), 1.0);
}

#[test]
fn interface_narrow_canvas_falls_back_without_shrinking_saved_floaters() {
    let mut f = Interface::new(1.5, 1.0);
    let wide = f.window();
    f.settings = ViewportSettings {
        enabled: true,
        restrict_ui: true,
        left: 95.0,
        right: 95.0,
        top: 95.0,
        bottom: 95.0,
    };
    f.frames(5);
    let canvas = f.ctx.content_rect();
    close_rect(canvas, Rect::from_min_size(Pos2::ZERO, vec2(3200.0, 2000.0) / 1.5), 1.0);
    assert!(layout::restricted_canvas(&f.ctx).is_none());
    close_rect(f.window(), wide, 1.0);
    for area in [layout::work_area(&f.ctx), layout::move_area(&f.ctx)] {
        assert!(area.is_finite() && area.is_positive());
        assert!(canvas.contains_rect(area), "{canvas:?}: {area:?}");
    }
    f.click(f.button.center());
    assert_eq!(f.clicks, 1, "native-canvas controls remain usable");
    f.settings = inset();
    f.frames(5);
    assert!(
        layout::restricted_canvas(&f.ctx).is_some(),
        "restriction resumes when the viewport is usable"
    );
}

#[test]
fn interface_automatically_shrunk_floaters_restore_their_wide_size_after_restart() {
    let mut f = Interface::new(1.0, 1.0);
    widgets::size_floater(&f.ctx, WINDOW, vec2(2600.0, 900.0));
    f.frames(4);
    let wide = f.window();
    assert!(wide.width() > 2500.0);
    f.settings.restrict_ui = true;
    f.frames(5);
    assert!(f.window().width() < wide.width());
    assert!(f.ctx.content_rect().expand(1.0).contains_rect(f.window()));
    let saved = ron::to_string(&f.ctx.memory(|memory| memory.clone())).expect("saved layout");
    let next = egui::Context::default();
    next.memory_mut(|memory| *memory = ron::from_str(&saved).expect("restored layout"));
    f.ctx = next;
    f.frames(5);
    f.settings.restrict_ui = false;
    f.frames(5);
    close_rect(f.window(), wide, 1.0);
}

#[test]
fn interface_expanding_a_previously_minimized_floater_retains_its_wide_geometry() {
    let mut f = Interface::with_kind(1.0, 1.0, Kind::Floater, vec2(2600.0, 900.0));
    let wide = f.window();
    widgets::minimize_floater(&f.ctx, WINDOW);
    f.frames(4);
    f.settings.restrict_ui = true;
    f.frames(5);
    assert!(widgets::floater_minimized(&f.ctx, WINDOW));
    widgets::expand_floater(&f.ctx, WINDOW);
    f.frames(5);
    assert!(f.window().width() < wide.width());
    f.settings.restrict_ui = false;
    f.frames(5);
    close_rect(f.window(), wide, 1.0);
}

#[test]
fn interface_expanding_on_the_wide_canvas_restores_size_and_right_anchor_after_minimizing() {
    let mut f = Interface::with_kind(1.0, 1.0, Kind::Floater, vec2(2600.0, 900.0));
    widgets::move_floater(&f.ctx, WINDOW, pos2(584.0, 90.0));
    f.frames(4);
    let wide = f.window();
    f.settings.restrict_ui = true;
    f.frames(5);
    widgets::minimize_floater(&f.ctx, WINDOW);
    f.frames(4);
    f.settings.restrict_ui = false;
    f.frames(5);
    assert!(widgets::floater_minimized(&f.ctx, WINDOW));
    widgets::expand_floater(&f.ctx, WINDOW);
    f.frames(5);
    close_rect(f.window(), wide, 1.0);
}

#[test]
fn interface_bare_maps_restore_their_wide_geometry_including_locked_maps() {
    for kind in [Kind::Bare, Kind::LockedBare] {
        let mut f = Interface::with_kind(1.0, 1.0, kind, vec2(2600.0, 900.0));
        let wide = f.window();
        f.settings.restrict_ui = true;
        f.frames(5);
        assert!(f.window().width() < wide.width(), "{kind:?}");
        assert!(f.ctx.content_rect().expand(1.0).contains_rect(f.window()));
        f.settings.restrict_ui = false;
        f.frames(5);
        close_rect(f.window(), wide, 1.0);
    }
}

#[test]
fn interface_manual_resize_replaces_the_retained_wide_floater_or_map_size() {
    for kind in [Kind::Floater, Kind::Bare] {
        let mut f = Interface::with_kind(1.0, 1.0, kind, vec2(2600.0, 900.0));
        let wide = f.window();
        f.settings.restrict_ui = true;
        f.frames(5);
        let before = f.window();
        let start = before.right_bottom() - vec2(2.0, 2.0);
        f.drag_to(start, start - vec2(600.0, 200.0));
        let resized = f.window();
        assert!(
            resized.width() < before.width() - 400.0,
            "the manual {kind:?} resize must occur: {before:?} → {resized:?}"
        );
        f.settings.restrict_ui = false;
        f.frames(5);
        assert!((f.window().size() - resized.size()).abs().max_elem() <= 1.0, "{kind:?}");
        assert!(f.window().width() < wide.width());
    }
}

#[test]
fn interface_aspect_fit_preserves_body_ratio_and_chrome_when_one_axis_overflows() {
    let ctx = egui::Context::default();
    let room = Rect::from_min_size(pos2(320.0, 120.0), vec2(640.0, 720.0));
    let ratio = 16.0 / 9.0;
    let chrome = vec2(20.0, 52.0);
    let requested = Rect::from_min_size(pos2(280.0, 90.0), vec2(1000.0, 980.0 / ratio + chrome.y));
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(room),
            ..Default::default()
        },
        |ui| {
            layout::begin_canvas(ui.ctx(), true);
            layout::set_move_area(ui.ctx(), room);
            let fit = layout::fit_aspect_window(ui.ctx(), requested, ratio, chrome);
            assert!(room.contains_rect(fit));
            let body = fit.size() - chrome;
            assert!((body.x / body.y - ratio).abs() < 0.0001);
            assert_eq!(fit.width(), 640.0);
        },
    );
    output.textures_delta.clear();
}

#[test]
fn interface_aspect_floater_drag_stays_in_the_offset_bar_room_without_distortion() {
    let mut f = Interface::new(1.0, 1.0);
    f.settings = ViewportSettings {
        enabled: true,
        restrict_ui: true,
        left: 50.0,
        right: 30.0,
        top: 10.0,
        bottom: 54.0,
    };
    f.classic_room = true;
    let ratio = 16.0 / 9.0;
    let chrome = vec2(0.0, 52.0);
    f.aspect = Some((ratio, chrome));
    f.frames(5);
    widgets::size_floater(&f.ctx, WINDOW, vec2(360.0, 360.0 / ratio + chrome.y));
    f.frames(4);
    let start = f.window().right_bottom() - vec2(2.0, 2.0);
    let target = start + vec2(1000.0, 900.0);
    f.show(vec![egui::Event::PointerMoved(start)], true);
    f.show(
        vec![egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
        true,
    );
    f.show(vec![egui::Event::PointerMoved(target)], true);
    f.show(vec![egui::Event::PointerMoved(target)], true);
    f.show(
        vec![egui::Event::PointerButton {
            pos: target,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
        true,
    );
    f.frames(4);
    let rect = f.window();
    assert!(rect.width() > 500.0, "the edge drag must actually resize the floater: {rect:?}");
    assert!(layout::move_area(&f.ctx).expand(1.0).contains_rect(rect));
    let body = rect.size() - chrome;
    assert!((body.x - body.y * ratio).abs() <= 1.0, "{rect:?}: body={body:?}");
}
