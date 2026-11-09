//! Non-overlapping UI padding, never a rectangle over the actual world.
use crate::workspace::model::{Arrangement, Background};
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
pub(super) fn regions(
    canvas: egui::Rect,
    world: egui::Rect,
    choice: &Background,
    arrangement: Option<&Arrangement>,
    displays: &[egui::Rect],
) -> Vec<(egui::Rect, [u8; 3])> {
    let padding = outside(canvas, world);
    let mut result = padding.iter().map(|r| (*r, choice.common())).collect::<Vec<_>>();
    if let Some(arrangement) = arrangement.filter(|a| a.len() == displays.len()) {
        for (index, display) in displays.iter().enumerate().filter(|(_, r)| r.is_finite() && r.is_positive()) {
            let color = choice.color(arrangement, index);
            if color == choice.common() {
                continue;
            }
            result.extend(
                padding
                    .iter()
                    .map(|r| r.intersect(*display))
                    .filter(|r| r.is_positive())
                    .map(|r| (r, color)),
            );
        }
    }
    result
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

#[cfg(test)]
mod display_tests {
    use super::*;
    #[test]
    fn workspace_display_background_paint_clips_overrides_and_falls_back_on_unknown_topology() {
        let a = Arrangement::from_physical(&[[0, 0, 100, 100], [100, 0, 200, 100]]).unwrap();
        let choice = Background::Solid { rgb: [1, 2, 3] }.with_display(&a, 0, Some([4, 5, 6])).unwrap();
        let r = |x, w| egui::Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(w, 100.0));
        let canvas = r(0.0, 200.0);
        let world = r(100.0, 100.0);
        let displays = [r(0.0, 100.0), r(100.0, 100.0)];
        let painted = regions(canvas, world, &choice, Some(&a), &displays);
        assert!(painted.iter().any(|(_, c)| *c == [4, 5, 6]));
        for (rect, _) in painted {
            assert!(canvas.contains_rect(rect));
            assert!(!rect.intersect(world).is_positive());
        }
        for arrangement in [None, Arrangement::from_physical(&[[0, 0, 200, 100]]).as_ref()] {
            assert!(
                regions(canvas, world, &choice, arrangement, &displays)
                    .iter()
                    .all(|(_, c)| *c == [1, 2, 3])
            );
        }
        assert!(regions(canvas, canvas, &choice, Some(&a), &displays).is_empty());
    }
}
