//! Optional workspace padding, below windows and outside the actual scene only.
use crate::workspace::model::{Arrangement, Background};
mod geometry;
pub(crate) fn paint(
    ctx: &egui::Context,
    canvas: egui::Rect,
    world: egui::Rect,
    choice: &Background,
    arrangement: Option<&Arrangement>,
    displays: &[egui::Rect],
) {
    if choice.is_black() {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::background()).with_clip_rect(canvas);
    for (rect, rgb) in geometry::regions(canvas, world, choice, arrangement, displays) {
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
    }
}
