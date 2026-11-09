//! Non-overlapping UI padding, never a rectangle over the actual world.
use egui::Rect;
pub(super) fn outside(canvas: Rect, world: Rect) -> Vec<Rect> {
    if !canvas.is_finite() || !world.is_finite() || !canvas.is_positive() || !world.is_positive() {
        return Vec::new();
    }
    let world = canvas.intersect(world);
    if !world.is_positive() {
        return Vec::new();
    }
    [
        Rect::from_min_max(canvas.min, egui::pos2(canvas.max.x, world.min.y)),
        Rect::from_min_max(egui::pos2(canvas.min.x, world.max.y), canvas.max),
        Rect::from_min_max(egui::pos2(canvas.min.x, world.min.y), egui::pos2(world.min.x, world.max.y)),
        Rect::from_min_max(egui::pos2(world.max.x, world.min.y), egui::pos2(canvas.max.x, world.max.y)),
    ]
    .into_iter()
    .filter(|r| r.is_positive())
    .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_background_padding_partitions_canvas_without_world_or_corner_overlap() {
        let canvas = Rect::from_min_max(egui::pos2(-100.0, -40.0), egui::pos2(800.0, 600.0));
        let world = Rect::from_min_max(egui::pos2(200.0, 50.0), egui::pos2(700.0, 500.0));
        let rects = outside(canvas, world);
        assert_eq!(rects.len(), 4);
        for (i, r) in rects.iter().enumerate() {
            assert!(canvas.contains_rect(*r));
            assert!(!r.intersect(world).is_positive());
            for other in &rects[i + 1..] {
                assert!(!r.intersect(*other).is_positive());
            }
        }
        assert_eq!(rects.iter().map(|r| r.area()).sum::<f32>(), canvas.area() - world.area());
    }
    #[test]
    fn workspace_background_full_or_unknown_world_never_fills_over_scene() {
        let canvas = Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(100.0, 100.0));
        assert!(outside(canvas, canvas).is_empty());
        assert!(outside(canvas, Rect::NOTHING).is_empty());
        assert!(outside(canvas, Rect::EVERYTHING).is_empty());
        assert!(outside(canvas, canvas.translate(egui::vec2(200.0, 0.0))).is_empty());
        assert_eq!(outside(canvas, Rect::from_min_max(egui::pos2(50.0, 0.0), canvas.max)).len(), 1);
    }
}
