//! Optional workspace padding, below windows and outside the actual scene only.
use crate::workspace::model::Background;
mod geometry;
pub(crate) fn paint(ctx: &egui::Context, canvas: egui::Rect, world: egui::Rect, choice: Background) {
    let Background::Solid { rgb: [red, green, blue] } = choice else {
        return;
    };
    let color = egui::Color32::from_rgb(red, green, blue);
    let painter = ctx.layer_painter(egui::LayerId::background()).with_clip_rect(canvas);
    for rect in geometry::outside(canvas, world) {
        painter.rect_filled(rect, 0.0, color);
    }
}
