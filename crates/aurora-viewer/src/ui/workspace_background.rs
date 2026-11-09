//! Optional workspace padding, below windows and outside the actual scene only.
use crate::workspace::model::{Arrangement, Background};
mod geometry;
pub(crate) mod pictures;
pub(crate) struct PictureImage {
    pub texture: egui::TextureHandle,
    pub original: [u32; 2],
}
pub(crate) type Images = std::collections::HashMap<crate::workspace::model::TextureRef, PictureImage>;
pub(crate) type LocalImages = std::collections::HashMap<crate::workspace::model::ManagedRef, PictureImage>;
pub(crate) fn paint(
    ctx: &egui::Context,
    canvas: egui::Rect,
    world: egui::Rect,
    choice: &Background,
    arrangement: Option<&Arrangement>,
    displays: &[egui::Rect],
    images: Option<&Images>,
    local_images: Option<&LocalImages>,
) {
    if choice.is_black() {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::background()).with_clip_rect(canvas);
    for (rect, rgb) in geometry::regions(canvas, world, choice, arrangement, displays) {
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
    }
    for region in pictures::regions(canvas, world, choice, arrangement, displays) {
        let image = match &region.image {
            crate::workspace::model::DrawImage::Texture(p) => images.and_then(|m| m.get(&p.texture)),
            crate::workspace::model::DrawImage::Local(p) => local_images.and_then(|m| m.get(&p.reference)),
        };
        let Some(image) = image else {
            continue;
        };
        let size = egui::vec2(image.original[0] as f32, image.original[1] as f32) / ctx.pixels_per_point();
        let Some((dest, uv)) = pictures::placement(region.target, size, region.image.mode()) else {
            continue;
        };
        let Some((area, uv)) = pictures::clipped(dest, uv, region.clip) else {
            continue;
        };
        painter.image(image.texture.id(), area, uv, egui::Color32::from_gray(region.image.brightness()));
    }
}
