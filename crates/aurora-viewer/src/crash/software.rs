//! CPU painter for the small recovery window. No graphics adapter, surface,
//! shader, GPU thread or driver is initialized after a rendering crash.
//! Only egui meshes are accepted: the recovery UI has no paint callbacks.

use egui::{
    Color32, ColorImage, TextureId,
    epaint::{ClippedPrimitive, Primitive, Vertex},
};
use std::collections::HashMap;

#[derive(Default)]
pub struct Painter {
    textures: HashMap<TextureId, ColorImage>,
}

impl Painter {
    pub fn paint(
        &mut self,
        pixels: &mut [u32],
        size: [usize; 2],
        primitives: &[ClippedPrimitive],
        delta: &egui::TexturesDelta,
        scale: f32,
        background: Color32,
    ) {
        self.paint_with_private(pixels, size, primitives, delta, scale, background, None);
    }

    /// The private image is borrowed for this call and never enters the texture
    /// cache. Ordinary recovery/font uploads keep their existing semantics.
    #[allow(clippy::too_many_arguments)] // Preserve the existing paint inputs plus the private borrow/gate.
    pub(crate) fn paint_with_private(
        &mut self,
        pixels: &mut [u32],
        size: [usize; 2],
        primitives: &[ClippedPrimitive],
        delta: &egui::TexturesDelta,
        scale: f32,
        background: Color32,
        private: Option<(TextureId, &ColorImage)>,
    ) -> bool {
        self.paint_regions(pixels, size, primitives, delta, scale, background, private, None)
    }

    /// Optional disjoint physical clips avoid rasterizing the host's native
    /// world hole. Texture uploads/frees and private borrowing remain identical.
    /// Clear the whole DIB cheaply so a later full cover never exposes old pixels.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_regions(
        &mut self,
        pixels: &mut [u32],
        size: [usize; 2],
        primitives: &[ClippedPrimitive],
        delta: &egui::TexturesDelta,
        scale: f32,
        background: Color32,
        private: Option<(TextureId, &ColorImage)>,
        regions: Option<&[[usize; 4]]>,
    ) -> bool {
        if private.is_some_and(|(id, _)| {
            !matches!(id, TextureId::User(_)) || self.textures.contains_key(&id) || delta.set.contains_key(&id) || delta.free.contains(&id)
        }) {
            return false;
        }
        for (id, changes) in &delta.set {
            for change in changes {
                let egui::ImageData::Color(image) = &change.image;
                if let Some([x, y]) = change.pos {
                    if let Some(texture) = self.textures.get_mut(id) {
                        for row in 0..image.height() {
                            if y + row >= texture.height() || x + image.width() > texture.width() {
                                continue;
                            }
                            let start = (y + row) * texture.width() + x;
                            texture.pixels[start..start + image.width()]
                                .copy_from_slice(&image.pixels[row * image.width()..(row + 1) * image.width()]);
                        }
                    }
                } else {
                    self.textures.insert(*id, image.as_ref().clone());
                }
            }
        }
        pixels.fill(rgb(background));
        for primitive in primitives {
            let Primitive::Mesh(mesh) = &primitive.primitive else { continue };
            let texture = private
                .filter(|(id, _)| *id == mesh.texture_id)
                .map(|(_, image)| image)
                .or_else(|| self.textures.get(&mesh.texture_id));
            let Some(texture) = texture else { continue };
            if texture.width() == 0 || texture.height() == 0 {
                continue;
            }
            let clip = [
                (primitive.clip_rect.min.x * scale).floor().max(0.0) as usize,
                (primitive.clip_rect.min.y * scale).floor().max(0.0) as usize,
                (primitive.clip_rect.max.x * scale).ceil().max(0.0).min(size[0] as f32) as usize,
                (primitive.clip_rect.max.y * scale).ceil().max(0.0).min(size[1] as f32) as usize,
            ];
            let full = [[0, 0, size[0], size[1]]];
            for region in regions.unwrap_or(&full) {
                let clip = [
                    clip[0].max(region[0]),
                    clip[1].max(region[1]),
                    clip[2].min(region[2]),
                    clip[3].min(region[3]),
                ];
                if clip[0] >= clip[2] || clip[1] >= clip[3] {
                    continue;
                }
                for indices in mesh.indices.as_chunks::<3>().0 {
                    if let (Some(a), Some(b), Some(c)) = (
                        mesh.vertices.get(indices[0] as usize),
                        mesh.vertices.get(indices[1] as usize),
                        mesh.vertices.get(indices[2] as usize),
                    ) {
                        triangle(pixels, size, clip, [*a, *b, *c], texture, scale);
                    }
                }
            }
        }
        for id in &delta.free {
            self.textures.remove(id);
        }
        true
    }
}

fn rgb(color: Color32) -> u32 {
    (u32::from(color.r()) << 16) | (u32::from(color.g()) << 8) | u32::from(color.b())
}

fn edge(a: egui::Pos2, b: egui::Pos2, p: egui::Pos2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

fn top_left(a: egui::Pos2, b: egui::Pos2) -> bool {
    b.y < a.y || (b.y == a.y && b.x > a.x)
}

fn triangle(pixels: &mut [u32], size: [usize; 2], clip: [usize; 4], mut v: [Vertex; 3], texture: &ColorImage, scale: f32) {
    for vertex in &mut v {
        vertex.pos *= scale;
    }
    let mut area = edge(v[0].pos, v[1].pos, v[2].pos);
    if area.abs() < f32::EPSILON {
        return;
    }
    if area < 0.0 {
        v.swap(1, 2);
        area = -area;
    }
    let min_x = v.iter().map(|v| v.pos.x).fold(f32::INFINITY, f32::min).floor().max(clip[0] as f32) as usize;
    let min_y = v.iter().map(|v| v.pos.y).fold(f32::INFINITY, f32::min).floor().max(clip[1] as f32) as usize;
    let max_x = v
        .iter()
        .map(|v| v.pos.x)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .max(0.0)
        .min(clip[2] as f32) as usize;
    let max_y = v
        .iter()
        .map(|v| v.pos.y)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .max(0.0)
        .min(clip[3] as f32) as usize;
    let inclusive = [
        top_left(v[1].pos, v[2].pos),
        top_left(v[2].pos, v[0].pos),
        top_left(v[0].pos, v[1].pos),
    ];
    // Egui's panels and covers use one white texel and a constant colour.
    // Sampling/interpolating those for every pixel dominated large host paints.
    let solid = (v[0].uv == v[1].uv && v[0].uv == v[2].uv && v[0].color == v[1].color && v[0].color == v[2].color).then(|| {
        let texel = sample(texture, v[0].uv.to_vec2());
        std::array::from_fn::<_, 4, _>(|channel| f32::from(v[0].color[channel]) * texel[channel] / 255.0)
    });
    for y in min_y..max_y {
        for x in min_x..max_x {
            let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
            let weights = [
                edge(v[1].pos, v[2].pos, p),
                edge(v[2].pos, v[0].pos, p),
                edge(v[0].pos, v[1].pos, p),
            ];
            if weights.iter().zip(inclusive).any(|(&w, include)| w < 0.0 || (w == 0.0 && !include)) {
                continue;
            }
            let color = solid.unwrap_or_else(|| {
                let w = weights.map(|w| w / area);
                let uv = v[0].uv.to_vec2() * w[0] + v[1].uv.to_vec2() * w[1] + v[2].uv.to_vec2() * w[2];
                let texel = sample(texture, uv);
                std::array::from_fn(|channel| (0..3).map(|i| f32::from(v[i].color[channel]) * w[i]).sum::<f32>() * texel[channel] / 255.0)
            });
            let target = &mut pixels[y * size[0] + x];
            let alpha = color[3] / 255.0;
            let mut blended = 0;
            for (channel, shift) in [16, 8, 0].into_iter().enumerate() {
                let destination = ((*target >> shift) & 255) as f32;
                blended |= ((color[channel] + destination * (1.0 - alpha)).round().clamp(0.0, 255.0) as u32) << shift;
            }
            *target = blended;
        }
    }
}

fn sample(texture: &ColorImage, uv: egui::Vec2) -> [f32; 4] {
    let x = (uv.x * texture.width() as f32 - 0.5).clamp(0.0, texture.width().saturating_sub(1) as f32);
    let y = (uv.y * texture.height() as f32 - 0.5).clamp(0.0, texture.height().saturating_sub(1) as f32);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(texture.width() - 1);
    let y1 = (y0 + 1).min(texture.height() - 1);
    let a = texture[(x0, y0)];
    let b = texture[(x1, y0)];
    let c = texture[(x0, y1)];
    let d = texture[(x1, y1)];
    let (tx, ty) = (x - x.floor(), y - y.floor());
    std::array::from_fn(|i| {
        (f32::from(a[i]) * (1.0 - tx) + f32::from(b[i]) * tx) * (1.0 - ty) + (f32::from(c[i]) * (1.0 - tx) + f32::from(d[i]) * tx) * ty
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_painter_clips_host_hole_but_still_uploads_frees_and_covers_old_pixels() {
        let id = TextureId::Managed(9);
        let mut mesh = egui::Mesh::with_texture(id);
        mesh.add_rect_with_uv(
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(4.0, 4.0)),
            egui::Rect::from_min_max(egui::pos2(0.5, 0.5), egui::pos2(0.5, 0.5)),
            Color32::from_rgba_premultiplied(128, 0, 0, 128),
        );
        let primitives = [ClippedPrimitive {
            clip_rect: egui::Rect::EVERYTHING,
            primitive: Primitive::Mesh(mesh),
        }];
        let mut delta = egui::TexturesDelta::default();
        delta.push(
            id,
            egui::epaint::ImageDelta::full(ColorImage::filled([1, 1], Color32::WHITE), egui::TextureOptions::LINEAR),
        );
        delta.free.insert(id);
        let mut painter = Painter::default();
        let mut clipped = [0xabcdef; 16];
        assert!(painter.paint_regions(
            &mut clipped,
            [4, 4],
            &primitives,
            &delta,
            1.0,
            Color32::BLACK,
            None,
            Some(&[[0, 0, 4, 1], [0, 3, 4, 4], [0, 1, 1, 3], [3, 1, 4, 3]])
        ));
        assert!(!painter.textures.contains_key(&id));
        let mut full = [0; 16];
        painter.paint(&mut full, [4, 4], &primitives, &delta, 1.0, Color32::BLACK);
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(full[y * 4 + x], 0x800000);
                assert_eq!(
                    clipped[y * 4 + x],
                    if (1..3).contains(&x) && (1..3).contains(&y) {
                        0
                    } else {
                        full[y * 4 + x]
                    }
                );
            }
        }
        delta.clear();
    }
    #[test]
    fn shared_triangle_edges_blend_once_and_obey_clip() {
        let texture = ColorImage::filled([1, 1], Color32::WHITE);
        let vertex = |x, y| Vertex {
            pos: egui::pos2(x, y),
            uv: egui::pos2(0.5, 0.5),
            color: Color32::from_rgba_premultiplied(128, 0, 0, 128),
        };
        let mut pixels = [0; 16];
        let v = [vertex(0.0, 0.0), vertex(4.0, 0.0), vertex(4.0, 4.0), vertex(0.0, 4.0)];
        triangle(&mut pixels, [4, 4], [1, 1, 3, 3], [v[0], v[1], v[2]], &texture, 1.0);
        triangle(&mut pixels, [4, 4], [1, 1, 3, 3], [v[0], v[2], v[3]], &texture, 1.0);
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(
                    pixels[y * 4 + x],
                    if (1..3).contains(&x) && (1..3).contains(&y) { 0x800000 } else { 0 }
                );
            }
        }
    }

    #[test]
    fn cpu_painter_ordinary_full_partial_and_free_order_remain_unchanged_with_private_borrow() {
        let mut painter = Painter::default();
        let ordinary = TextureId::Managed(3);
        let private = ColorImage::filled([1, 1], Color32::BLUE);
        let mut delta = egui::TexturesDelta::default();
        delta.push(
            ordinary,
            egui::epaint::ImageDelta::full(ColorImage::filled([2, 2], Color32::RED), egui::TextureOptions::LINEAR),
        );
        let mut pixels = [0; 1];
        assert!(painter.paint_with_private(
            &mut pixels,
            [1, 1],
            &[],
            &delta,
            1.0,
            Color32::BLACK,
            Some((TextureId::User(7), &private))
        ));
        delta.clear();
        assert_eq!(painter.textures[&ordinary].pixels, vec![Color32::RED; 4]);
        assert!(!painter.textures.contains_key(&TextureId::User(7)));
        delta.push(
            ordinary,
            egui::epaint::ImageDelta::partial([1, 0], ColorImage::filled([1, 1], Color32::GREEN), egui::TextureOptions::LINEAR),
        );
        painter.paint(&mut pixels, [1, 1], &[], &delta, 1.0, Color32::BLACK);
        delta.clear();
        assert_eq!(
            painter.textures[&ordinary].pixels,
            [Color32::RED, Color32::GREEN, Color32::RED, Color32::RED]
        );
        let mesh = egui::Mesh::with_texture(ordinary);
        let primitive = ClippedPrimitive {
            clip_rect: egui::Rect::EVERYTHING,
            primitive: Primitive::Mesh(mesh),
        };
        delta.free(ordinary);
        painter.paint(&mut pixels, [1, 1], &[primitive], &delta, 1.0, Color32::BLACK);
        delta.clear();
        assert!(!painter.textures.contains_key(&ordinary));
        delta.push(
            TextureId::User(7),
            egui::epaint::ImageDelta::full(private.clone(), egui::TextureOptions::LINEAR),
        );
        painter.paint(&mut pixels, [1, 1], &[], &delta, 1.0, Color32::BLACK);
        delta.clear();
        assert!(
            !painter.paint_with_private(
                &mut pixels,
                [1, 1],
                &[],
                &delta,
                1.0,
                Color32::BLACK,
                Some((TextureId::User(7), &private))
            ),
            "a previously cached User ID cannot be repurposed as private"
        );
    }
}
