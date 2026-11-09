//! Image placement uses the current padding union, then clips each draw to a
//! non-world strip. No cover/crop mode can extend over the rendered world.
#[cfg(test)]
use crate::workspace::model::Picture;
use crate::workspace::model::{Arrangement, Background, DrawImage, ImageMode, ManagedRef, TextureRef};
use egui::{Rect, Vec2};

pub(crate) struct Region {
    pub clip: Rect,
    pub target: Rect,
    pub image: DrawImage,
}
pub(crate) fn regions(canvas: Rect, world: Rect, choice: &Background, arrangement: Option<&Arrangement>, displays: &[Rect]) -> Vec<Region> {
    if !matches!(choice, Background::Images { .. }) {
        return Vec::new();
    }
    let padding = super::geometry::outside(canvas, world);
    let common = choice.paint(None, None);
    let mut result = Vec::new();
    let mut add = |clips: Vec<Rect>, image: Option<DrawImage>| {
        let Some(image) = image else { return };
        let target = clips.iter().copied().fold(Rect::NOTHING, Rect::union);
        if !target.is_positive() || !target.is_finite() {
            return;
        }
        result.extend(clips.into_iter().map(|clip| Region {
            clip,
            target,
            image: image.clone(),
        }));
    };
    if common.spanning() || arrangement.is_none_or(|a| a.len() != displays.len()) {
        add(padding, common.draw_image());
    } else if let Some(a) = arrangement {
        for (index, display) in displays.iter().enumerate().filter(|(_, r)| r.is_finite() && r.is_positive()) {
            let clips = padding.iter().map(|p| p.intersect(*display)).filter(|r| r.is_positive()).collect();
            add(clips, choice.paint(Some(a), Some(index)).draw_image());
        }
    }
    result
}
pub(crate) fn references(regions: &[Region]) -> Vec<TextureRef> {
    let mut result = Vec::new();
    for r in regions {
        if let DrawImage::Texture(picture) = &r.image
            && !result.contains(&picture.texture)
        {
            result.push(picture.texture.clone());
        }
    }
    result
}
pub(crate) fn local_references(regions: &[Region]) -> Vec<ManagedRef> {
    let mut result = Vec::new();
    for r in regions {
        if let DrawImage::Local(p) = &r.image
            && !result.contains(&p.reference)
        {
            result.push(p.reference.clone());
        }
    }
    result
}
pub(super) fn placement(target: Rect, image: Vec2, mode: ImageMode) -> Option<(Rect, Rect)> {
    if !target.is_positive() || !target.is_finite() || !image.is_finite() || image.x <= 0.0 || image.y <= 0.0 {
        return None;
    }
    let uv = Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
    match mode {
        ImageMode::Stretch => Some((target, uv)),
        ImageMode::Tile => Some((target, Rect::from_min_size(egui::Pos2::ZERO, target.size() / image))),
        _ => {
            let ratio = target.size() / image;
            let scale = match mode {
                ImageMode::Center => 1.0,
                ImageMode::Fit => ratio.min_elem(),
                _ => ratio.max_elem(),
            };
            let dest = Rect::from_center_size(target.center(), image * scale);
            (dest.is_finite() && dest.is_positive()).then_some((dest, uv))
        }
    }
}
pub(super) fn clipped(destination: Rect, uv: Rect, clip: Rect) -> Option<(Rect, Rect)> {
    if !destination.is_finite() || !destination.is_positive() || !uv.is_finite() || !clip.is_finite() {
        return None;
    }
    let area = destination.intersect(clip);
    if !area.is_positive() {
        return None;
    }
    let map = |p: egui::Pos2| uv.min + (p - destination.min) / destination.size() * uv.size();
    Some((area, Rect::from_min_max(map(area.min), map(area.max))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_local_picture_regions_use_separate_managed_keys_and_keep_original_world_clips() {
        let reference = ManagedRef::new(uuid::Uuid::new_v4(), uuid::Uuid::new_v4(), "a".repeat(64)).unwrap();
        let picture = crate::workspace::model::LocalPicture {
            reference: reference.clone(),
            mode: ImageMode::Span,
            brightness: 200,
        };
        let choice = Background::Solid { rgb: [1, 2, 3] }.with_local(None, picture).unwrap();
        let canvas = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 100.0));
        let world = Rect::from_min_size(egui::pos2(100.0, 0.0), egui::vec2(100.0, 100.0));
        let plans = regions(canvas, world, &choice, None, &[]);
        assert_eq!(plans.len(), 1);
        assert!(references(&plans).is_empty());
        assert_eq!(local_references(&plans), vec![reference]);
        assert!(!plans[0].clip.intersect(world).is_positive());
        assert_eq!(plans[0].image.brightness(), 200);
    }
    #[test]
    fn workspace_picture_placement_preserves_aspect_native_size_and_clipped_tile_uv() {
        let target = Rect::from_min_size(egui::pos2(-100.0, 10.0), egui::vec2(200.0, 100.0));
        let image = egui::vec2(100.0, 100.0);
        let (fill, uv) = placement(target, image, ImageMode::Fill).unwrap();
        assert_eq!(fill.size(), egui::vec2(200.0, 200.0));
        let (area, crop) = clipped(fill, uv, target).unwrap();
        assert_eq!(area, target);
        assert_eq!(crop.height(), 0.5);
        assert_eq!(placement(target, image, ImageMode::Fit).unwrap().0.size(), image);
        assert_eq!(placement(target, image, ImageMode::Center).unwrap().0.size(), image);
        assert_eq!(placement(target, image, ImageMode::Stretch).unwrap().0, target);
        let (_, tile) = placement(target, image, ImageMode::Tile).unwrap();
        assert_eq!(tile.size(), egui::vec2(2.0, 1.0));
        assert!(placement(target, Vec2::ZERO, ImageMode::Fill).is_none());
        assert!(placement(Rect::EVERYTHING, image, ImageMode::Fill).is_none());
    }
    #[test]
    fn workspace_picture_regions_keep_span_continuity_and_never_paint_world_or_unknown_geometry() {
        let a = Arrangement::from_physical(&[[0, 0, 100, 100], [100, 0, 200, 100]]).unwrap();
        let r = |x, w| Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(w, 100.0));
        let canvas = r(0.0, 200.0);
        let world = r(70.0, 60.0);
        let picture = Picture {
            texture: TextureRef::new(uuid::Uuid::new_v4(), uuid::Uuid::new_v4(), "a".repeat(64)).unwrap(),
            mode: ImageMode::Span,
            brightness: 255,
        };
        let choice = Background::Solid { rgb: [1, 2, 3] }.with_picture(None, picture).unwrap();
        let regions = regions(canvas, world, &choice, Some(&a), &[r(0.0, 100.0), r(100.0, 100.0)]);
        assert_eq!(regions.len(), 2);
        assert_eq!(references(&regions).len(), 1);
        for region in regions {
            assert_eq!(region.target, canvas);
            assert!(!region.clip.intersect(world).is_positive());
        }
        assert!(super::regions(canvas, Rect::NOTHING, &choice, Some(&a), &[]).is_empty());
        assert!(super::regions(canvas, canvas, &choice, Some(&a), &[]).is_empty());
    }
}
