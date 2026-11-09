//! Regression checks for the independent Aurora hover/control ports.
use super::*;

fn picture_switch(shapes: &[egui::Shape]) -> egui::Pos2 {
    let label = shapes
        .iter()
        .find(|shape| matches!(shape, egui::Shape::Text(t) if t.galley.text() == aurora_i18n::tr!("inventory-hover-pictures")))
        .expect("the picture row stays visible")
        .visual_bounding_rect();
    shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Rect(r)
                if r.rect.size() == crate::ui::widgets::SWITCH_SIZE
                    && r.rect.left() > label.right()
                    && (r.rect.center().y - label.center().y).abs() < 2.0 =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .min_by(|a, b| a.left().total_cmp(&b.left()))
        .expect("the enabled picture switch")
        .center()
}

#[test]
fn inventory_pictures_preference_can_be_clicked_with_text_tooltips_off() {
    for text in [false, true] {
        let mut frames = Frames::new();
        let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
        s.inventory.show_hover_tooltips = text;
        let view = (Page::Inventory, 0, 700.0);
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = preference_shapes(&mut frames, &mut s, &mut st, &mut r, view, Vec::new());
        }
        for events in click(picture_switch(&shapes)) {
            preference_shapes(&mut frames, &mut s, &mut st, &mut r, view, events);
        }
        assert!(s.inventory.thumbnail_tooltips, "pictures can be enabled independently");
        assert_eq!(s.inventory.show_hover_tooltips, text, "the neighboring switch stays unchanged");
    }
}

/// Representative opaque CPU Preferences images. Native fonts/glass and
/// source/account switching are separate acceptance checks.
#[test]
fn hover_controls_preferences_capture() {
    let Some(path) = std::env::var_os("AURORA_CAPTURE") else { return };
    let path = std::path::Path::new(&path);
    for (name, query) in [
        ("friends-only", aurora_i18n::tr!("toolbar-command-friends-only")),
        ("inventory-pictures", aurora_i18n::tr!("inventory-hover-pictures")),
    ] {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme {
            panel_alpha: 255,
            ..Default::default()
        };
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let icons = Icons::load(&ctx, None);
        let mut s = Settings::default();
        s.inventory.thumbnail_tooltips = true;
        let mut st = OptionsUi {
            query: query.to_owned(),
            friends_only: Some(true),
            friends_only_keep: Some(false),
            ..Default::default()
        };
        let mut painter = crate::crash::software::Painter::default();
        let mut pixels = vec![0; 1280 * 900];
        for frame in 0..12 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |ui| {
                    let mut open = true;
                    show(ui.ctx(), &p, &icons, &mut s, &mut st, &mut open);
                },
            );
            let primitives = if frame == 11 {
                ctx.tessellate(output.shapes.clone(), output.pixels_per_point)
            } else {
                Vec::new()
            };
            assert!(primitives.iter().all(|p| matches!(p.primitive, egui::epaint::Primitive::Mesh(_))));
            painter.paint(
                &mut pixels,
                [1280, 900],
                &primitives,
                &output.textures_delta,
                output.pixels_per_point,
                p.panel,
            );
            output.textures_delta.clear();
        }
        let rgba: Vec<u8> = pixels
            .iter()
            .flat_map(|p| [(*p >> 16) as u8, (*p >> 8) as u8, *p as u8, 255])
            .collect();
        image::save_buffer(
            path.with_file_name(format!("{name}.png")),
            &rgba,
            1280,
            900,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
}
