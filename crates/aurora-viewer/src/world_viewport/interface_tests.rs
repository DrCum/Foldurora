use super::*;
use crate::ui::{layout, menu, widgets};
use egui::{Id, Pos2, Rect, Vec2, pos2, vec2};

const INVENTORY: &str = "viewport_inventory_fixture";
const SETTINGS: &str = "viewport_settings_fixture";
const WORLD_MAP: &str = "viewport_world_map_fixture";
const MINI_MAP: &str = "viewport_mini_map_fixture";
const CAMERA: &str = "viewport_camera_fixture";
const PHOTO: &str = "viewport_photo_fixture";
const ORDINARY: [&str; 4] = [INVENTORY, SETTINGS, WORLD_MAP, MINI_MAP];

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

struct Interface {
    ctx: egui::Context,
    settings: ViewportSettings,
    dpi: f32,
    frame: usize,
    // Only the migration test uses the old f504 global canvas contract.
    legacy_canvas: bool,
    wide: bool,
    locked_map: bool,
    menus: bool,
    buttons: [Rect; 4],
    clicks: [usize; 4],
    top: Rect,
    bottom: Rect,
    toolbar_popup: Rect,
    ordinary_popup: Rect,
}

impl Interface {
    const NATIVE: (u32, u32) = (3200, 2000);

    fn new(dpi: f32, zoom: f32) -> Self {
        Self::with_wide(dpi, zoom, false)
    }

    fn with_wide(dpi: f32, zoom: f32, wide: bool) -> Self {
        let ctx = egui::Context::default();
        ctx.set_zoom_factor(zoom);
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        let mut this = Self {
            ctx,
            settings: ViewportSettings {
                restrict_ui: false,
                ..inset()
            },
            dpi,
            frame: 0,
            legacy_canvas: false,
            wide,
            locked_map: false,
            menus: false,
            buttons: [Rect::NOTHING; 4],
            clicks: [0; 4],
            top: Rect::NOTHING,
            bottom: Rect::NOTHING,
            toolbar_popup: Rect::NOTHING,
            ordinary_popup: Rect::NOTHING,
        };
        this.frames(5);
        this
    }

    fn native_rect(&self) -> Rect {
        Rect::from_min_size(pos2(7.0, 11.0), vec2(3200.0, 2000.0) / (self.dpi * self.ctx.zoom_factor()))
    }

    fn bounds(&self) -> Rect {
        self.settings
            .toolbar_rect(Self::NATIVE, self.native_rect())
            .unwrap_or_else(|| self.native_rect())
    }

    fn window(&self, id: &str) -> Rect {
        self.ctx.memory(|memory| memory.area_rect(Id::unique(id))).expect("fixture window")
    }

    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            self.show(Vec::new(), true);
        }
    }

    fn restart(&mut self) {
        let saved = ron::to_string(&self.ctx.memory(|memory| memory.clone())).expect("saved layout");
        let ctx = egui::Context::default();
        ctx.set_zoom_factor(self.ctx.zoom_factor());
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        ctx.memory_mut(|memory| *memory = ron::from_str(&saved).expect("restored layout"));
        self.ctx = ctx;
    }

    fn show(&mut self, events: Vec<egui::Event>, has_screen: bool) {
        self.frame += 1;
        let native = self.native_rect();
        let chrome = self.settings.toolbar_rect(Self::NATIVE, native);
        let screen = if self.legacy_canvas { chrome.unwrap_or(native) } else { native };
        let mut raw = egui::RawInput {
            screen_rect: has_screen.then_some(screen),
            time: Some(self.frame as f64 / 60.0),
            events,
            ..Default::default()
        };
        let desktop = Rect::from_min_size(pos2(-1920.0, 80.0), native.size());
        let info = raw.viewports.entry(raw.viewport_id).or_default();
        info.native_pixels_per_point = Some(self.dpi);
        info.inner_rect = Some(desktop);
        let p = crate::theme::Theme::default().palette();
        let mut clicked = [false; 4];
        let mut output = self.ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx().clone();
            layout::begin_canvas(&ctx, self.legacy_canvas && chrome.is_some());
            layout::set_toolbar_canvas(&ctx, if self.legacy_canvas { None } else { chrome });
            assert_eq!(ctx.input(|input| input.viewport().inner_rect), Some(desktop));
            let room = ctx.content_rect();
            let toolbar = if self.legacy_canvas { room } else { layout::toolbar_rect(&ctx) };
            let mut bars = ui.new_child(egui::UiBuilder::new().max_rect(toolbar));
            bars.set_clip_rect(toolbar);
            self.top = egui::Panel::top("viewport_fixture_top")
                .exact_size(24.0)
                .show(&mut bars, |ui| {
                    layout::toolbar_layer(ui.ctx(), ui.layer_id());
                    let response = ui.button("Toolbar menu");
                    if self.menus {
                        menu::dropdown_held(&response, &p, true, None, |ui| {
                            ui.set_min_size(vec2(130.0, 70.0));
                            ui.label("Toolbar item");
                            self.toolbar_popup = ui.min_rect();
                        });
                    }
                })
                .response
                .rect;
            self.bottom = egui::Panel::bottom("viewport_fixture_bottom")
                .exact_size(32.0)
                .show(&mut bars, |ui| ui.label("Toolbar"))
                .response
                .rect;
            // Islands keep ordinary windows on the native workspace even
            // when the bars themselves have a smaller placement rectangle.
            layout::set_work_area(&ctx, room);
            layout::set_move_area(&ctx, room);
            layout::set_window_edges(&ctx, layout::Edges::OPEN);
            layout::frame(&ctx, &p, false);
            for (index, id) in ORDINARY.into_iter().enumerate() {
                let size = if self.wide && (index == 0 || index == 3) {
                    vec2(2600.0, 600.0)
                } else {
                    vec2(240.0, 170.0)
                };
                let position = native.min + vec2(20.0, 90.0 + index as f32 * 220.0);
                let body = |ui: &mut egui::Ui| {
                    let response = ui.button("Activate ordinary window");
                    self.buttons[index] = response.rect;
                    clicked[index] |= response.clicked();
                    if self.menus && index == 0 {
                        menu::dropdown_held(&response, &p, true, None, |ui| {
                            ui.set_min_size(vec2(130.0, 70.0));
                            ui.label("Inventory item");
                            self.ordinary_popup = ui.min_rect();
                        });
                    }
                };
                if id == MINI_MAP {
                    widgets::bare_window(
                        &ctx,
                        &p,
                        id,
                        "Mini-map",
                        egui::Frame::NONE,
                        Rect::from_min_size(position, size),
                        vec2(100.0, 80.0),
                        self.locked_map,
                        body,
                    );
                } else {
                    widgets::Floater::new(id, id, position, size)
                        .allow_partial_outside()
                        .silent()
                        .show(&ctx, &p, &mut true, body);
                }
            }
            for (index, id) in [CAMERA, PHOTO].into_iter().enumerate() {
                let position = native.min + vec2(30.0, native.height() * (0.55 + index as f32 * 0.2));
                widgets::Floater::new(id, id, position, vec2(300.0, 220.0))
                    .viewport_control()
                    .allow_partial_outside()
                    .silent()
                    .show(&ctx, &p, &mut true, |ui| {
                        let _ = ui.button("Camera/snapshot action");
                        ui.label("Controls stay with the world; output size is independent.");
                    });
            }
        });
        for (count, clicked) in self.clicks.iter_mut().zip(clicked) {
            *count += usize::from(clicked);
        }
        output.textures_delta.clear();
    }

    fn click(&mut self, pos: Pos2) {
        self.show(
            vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE), egui::Event::PointerMoved(pos)],
            true,
        );
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

    fn drag(&mut self, start: Pos2, target: Pos2) {
        self.show(
            vec![
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                egui::Event::PointerMoved(start),
            ],
            true,
        );
        self.show(
            vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            true,
        );
        for _ in 0..3 {
            self.show(vec![egui::Event::PointerMoved(target)], true);
        }
        self.show(
            vec![egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            true,
        );
        self.frames(5);
    }
}

#[test]
fn toolbar_bounds_preserve_offset_native_coordinates_at_fractional_dpi_and_zoom() {
    let settings = inset();
    for dpi in [1.0, 1.25, 1.5, 2.0] {
        for zoom in [0.8, 1.0, 1.25] {
            let scale = dpi * zoom;
            let screen = Rect::from_min_size(pos2(7.0, 11.0), vec2(3200.0, 2000.0) / scale);
            let expected = Rect::from_min_size(screen.min + vec2(800.0, 300.0) / scale, vec2(2080.0, 1300.0) / scale);
            close_rect(
                settings.toolbar_rect(Interface::NATIVE, screen).expect("toolbar bounds"),
                expected,
                0.001,
            );
        }
    }
}

#[test]
fn selective_viewport_keeps_real_bars_and_camera_controls_inside_with_ordinary_windows_outside() {
    for (dpi, zoom) in [(1.0, 1.0), (1.25, 1.2), (1.5, 0.8), (2.0, 1.25)] {
        let mut f = Interface::new(dpi, zoom);
        f.settings.restrict_ui = true;
        f.frames(6);
        let bounds = f.bounds().expand(1.0);
        close_rect(f.ctx.content_rect(), f.native_rect(), 1.0);
        assert!(layout::restricted_canvas(&f.ctx).is_none());
        for rect in [f.top, f.bottom, f.window(CAMERA), f.window(PHOTO)] {
            assert!(bounds.contains_rect(rect), "{dpi}/{zoom}: {rect:?} outside {bounds:?}");
        }
        for id in ORDINARY {
            let rect = f.window(id);
            assert!(f.native_rect().expand(1.0).contains_rect(rect), "{id}: {rect:?}");
            assert!(
                rect.left() < bounds.left() - 5.0,
                "ordinary {id} was moved into {bounds:?}: {rect:?}"
            );
        }
    }
}

#[test]
fn selective_viewport_keeps_inventory_settings_and_maps_clickable_outside_the_world() {
    let mut f = Interface::new(1.25, 1.2);
    f.settings.restrict_ui = true;
    f.frames(6);
    for (index, id) in ORDINARY.into_iter().enumerate() {
        let point = f.buttons[index].center();
        assert!(!f.bounds().contains(point), "ordinary fixture button must be outside the world");
        f.click(point);
        assert_eq!(f.clicks[index], 1, "{id} lost native-workspace pointer input");
        assert_eq!(f.ctx.input(|input| input.pointer.latest_pos()), Some(point));
    }
}

#[test]
fn selective_viewport_toolbar_popups_are_bounded_while_inventory_popups_keep_the_native_workspace() {
    let mut f = Interface::new(1.25, 1.2);
    f.settings.restrict_ui = true;
    f.menus = true;
    f.frames(8);
    assert!(f.toolbar_popup.is_positive(), "the real toolbar dropdown must open");
    assert!(
        f.bounds().expand(1.0).contains_rect(f.toolbar_popup),
        "{:?} outside {:?}",
        f.toolbar_popup,
        f.bounds()
    );
    assert!(f.ordinary_popup.is_positive(), "the real Inventory dropdown must open");
    assert!(f.native_rect().expand(1.0).contains_rect(f.ordinary_popup));
    assert!(
        f.ordinary_popup.left() < f.bounds().left() - 5.0,
        "ordinary popup was trapped inside the world"
    );
}

#[test]
fn selective_viewport_toggling_restores_controls_without_moving_ordinary_saved_windows() {
    let mut f = Interface::new(1.0, 1.0);
    let ordinary = ORDINARY.map(|id| f.window(id));
    let controls = [f.window(CAMERA), f.window(PHOTO)];
    for enabled in [true, false, true, false] {
        f.settings.restrict_ui = enabled;
        f.frames(6);
        for (id, expected) in ORDINARY.into_iter().zip(ordinary) {
            close_rect(f.window(id), expected, 1.0);
        }
        if enabled {
            for id in [CAMERA, PHOTO] {
                assert!(f.bounds().expand(1.0).contains_rect(f.window(id)));
            }
        } else {
            for (id, expected) in [CAMERA, PHOTO].into_iter().zip(controls) {
                close_rect(f.window(id), expected, 1.0);
            }
        }
    }
    f.show(Vec::new(), false);
    close_rect(f.ctx.content_rect(), f.native_rect(), 1.0);
}

#[test]
fn selective_viewport_saved_memory_restart_preserves_both_scopes_and_ordinary_geometry() {
    for enabled in [false, true] {
        let mut f = Interface::new(1.25, 1.0);
        f.settings.restrict_ui = enabled;
        widgets::move_floater(&f.ctx, SETTINGS, pos2(25.0, 410.0));
        widgets::size_floater(&f.ctx, SETTINGS, vec2(450.0, 260.0));
        f.frames(6);
        let ordinary = ORDINARY.map(|id| f.window(id));
        let controls = [f.window(CAMERA), f.window(PHOTO)];
        f.restart();
        f.frames(6);
        for (id, expected) in ORDINARY.into_iter().zip(ordinary) {
            close_rect(f.window(id), expected, 1.0);
        }
        for (id, expected) in [CAMERA, PHOTO].into_iter().zip(controls) {
            close_rect(f.window(id), expected, 1.0);
        }
    }
}

#[test]
fn selective_viewport_restores_legacy_wide_backups_after_restart_and_minimized_expansion() {
    for minimized in [false, true] {
        let mut f = Interface::with_wide(1.0, 1.0, true);
        f.locked_map = minimized;
        f.frames(6);
        let wide = [f.window(INVENTORY), f.window(MINI_MAP)];
        assert!(wide[0].width() > 2500.0);
        assert!(wide[1].width() > 2500.0);
        f.settings.restrict_ui = true;
        f.legacy_canvas = true;
        f.frames(6);
        for id in [INVENTORY, MINI_MAP] {
            assert!(f.window(id).width() < 2300.0);
            assert!(
                f.ctx
                    .data_mut(|data| data.get_persisted::<Vec2>(Id::unique(id).with("viewport_unrestricted_size")))
                    .is_some()
            );
        }
        if minimized {
            widgets::minimize_floater(&f.ctx, INVENTORY);
            f.frames(4);
        }
        f.restart();
        f.legacy_canvas = false;
        f.frames(6);
        if minimized {
            assert!(widgets::floater_minimized(&f.ctx, INVENTORY));
            widgets::expand_floater(&f.ctx, INVENTORY);
            f.frames(6);
        }
        for (id, expected) in [INVENTORY, MINI_MAP].into_iter().zip(wide) {
            close_rect(f.window(id), expected, 1.0);
        }
    }
}

#[test]
fn selective_viewport_control_drag_and_resize_stay_bounded_when_ordinary_edges_are_open() {
    let mut f = Interface::new(1.0, 1.0);
    f.settings.restrict_ui = true;
    f.frames(6);
    assert_eq!(layout::edges_for(&f.ctx, Id::unique(INVENTORY)), layout::Edges::OPEN);
    let before = f.window(CAMERA);
    let start = before.min + vec2(70.0, 15.0);
    f.drag(start, f.native_rect().min + vec2(30.0, 30.0));
    assert!(f.bounds().expand(1.0).contains_rect(f.window(CAMERA)));
    let before = f.window(PHOTO);
    let start = before.right_bottom() - vec2(2.0, 2.0);
    f.drag(start, f.native_rect().right_bottom() - vec2(2.0, 2.0));
    let resized = f.window(PHOTO);
    assert!(
        resized.width() > before.width() + 20.0 || resized.height() > before.height() + 20.0,
        "resize must actually occur: {before:?} -> {resized:?}"
    );
    assert!(f.bounds().expand(1.0).contains_rect(resized));
}

#[test]
fn selective_viewport_small_controls_do_not_fall_back_to_the_native_workspace() {
    let mut f = Interface::new(1.0, 1.0);
    f.settings = ViewportSettings {
        enabled: true,
        restrict_ui: true,
        left: 60.0,
        right: 30.0,
        top: 30.0,
        bottom: 55.0,
    };
    f.frames(8);
    assert!(f.bounds().width() < 640.0 && f.bounds().height() < 400.0);
    for id in [CAMERA, PHOTO] {
        assert!(
            f.bounds().expand(1.0).contains_rect(f.window(id)),
            "{id}: {:?} outside {:?}",
            f.window(id),
            f.bounds()
        );
    }
    close_rect(f.ctx.content_rect(), f.native_rect(), 1.0);
    f.click(f.buttons[1].center());
    assert_eq!(f.clicks[1], 1, "Settings remains usable outside the small viewport");
}

#[test]
fn viewport_snapshot_output_and_desktop_mouselook_are_independent_of_control_placement() {
    let native = Interface::NATIVE;
    let screen = Rect::from_min_size(pos2(7.0, 11.0), vec2(2560.0, 1600.0));
    for restricted in [false, true] {
        let settings = ViewportSettings {
            restrict_ui: restricted,
            ..inset()
        };
        let world = settings.viewport(native);
        let controls = settings.toolbar_rect(native, screen);
        assert_ne!(world, aurora_render::WorldViewport::full(native));
        assert_eq!(settings.render_viewport(native, true, false, false, false), world);
        assert_eq!(settings.render_viewport(native, true, true, false, false), world);
        for (mouselook, hidden, photo, vr) in [
            (false, false, true, false),
            (true, true, true, false),
            (true, true, false, true),
            (false, true, false, false),
        ] {
            assert_eq!(
                settings.render_viewport(native, mouselook, hidden, photo, vr),
                aurora_render::WorldViewport::full(native)
            );
            assert_eq!(
                settings.toolbar_rect(native, screen),
                controls,
                "output policy cannot change placement"
            );
        }
    }
}
