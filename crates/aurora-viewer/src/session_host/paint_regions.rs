//! Physical CPU paint clips for the native host cutout. Rectangles are disjoint
//! so popup overlap never blends a translucent pixel twice. Round exactly like
//! native::protected_shape (outward islands, inward world).
pub(super) fn visible_regions(
    size: [usize; 2],
    world: egui::Rect,
    scale: f32,
    rects: &[egui::Rect],
    cover_all: bool,
) -> Option<Vec<[usize; 4]>> {
    if cover_all || !world.is_finite() || !scale.is_finite() || scale <= 0.0 || rects.len() > 8 {
        return None;
    }
    let edge = |value: f64, limit: usize| value.max(0.0).min(limit as f64) as usize;
    let scaled = |value: f32| f64::from(value) * f64::from(scale);
    let hole = [
        edge(scaled(world.left()).ceil(), size[0]),
        edge(scaled(world.top()).ceil(), size[1]),
        edge(scaled(world.right()).floor(), size[0]),
        edge(scaled(world.bottom()).floor(), size[1]),
    ];
    if hole[0] >= hole[2] || hole[1] >= hole[3] {
        return None;
    }
    let mut regions = subtract([0, 0, size[0], size[1]], hole);
    for rect in rects {
        if !rect.is_finite() {
            return None;
        }
        let island = [
            edge(scaled(rect.left()).floor() - 1.0, size[0]).max(hole[0]),
            edge(scaled(rect.top()).floor() - 1.0, size[1]).max(hole[1]),
            edge(scaled(rect.right()).ceil() + 1.0, size[0]).min(hole[2]),
            edge(scaled(rect.bottom()).ceil() + 1.0, size[1]).min(hole[3]),
        ];
        if island[0] >= island[2] || island[1] >= island[3] {
            continue;
        }
        let mut uncovered = vec![island];
        for existing in &regions {
            uncovered = uncovered.into_iter().flat_map(|rect| subtract(rect, *existing)).collect();
        }
        regions.extend(uncovered);
    }
    Some(regions)
}

fn subtract(rect: [usize; 4], cut: [usize; 4]) -> Vec<[usize; 4]> {
    let overlap = [rect[0].max(cut[0]), rect[1].max(cut[1]), rect[2].min(cut[2]), rect[3].min(cut[3])];
    if overlap[0] >= overlap[2] || overlap[1] >= overlap[3] {
        return vec![rect];
    }
    [
        [rect[0], rect[1], rect[2], overlap[1]],
        [rect[0], overlap[3], rect[2], rect[3]],
        [rect[0], overlap[1], overlap[0], overlap[3]],
        [overlap[2], overlap[1], rect[2], overlap[3]],
    ]
    .into_iter()
    .filter(|r| r[0] < r[2] && r[1] < r[3])
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct TestOverlays {
        rects: Vec<egui::Rect>,
        cover_all: bool,
    }
    impl TestOverlays {
        fn paint_regions(&self, size: [usize; 2], world: egui::Rect, scale: f32) -> Option<Vec<[usize; 4]>> {
            visible_regions(size, world, scale, &self.rects, self.cover_all)
        }
    }
    #[test]
    fn host_paint_regions_cover_popup_union_once_and_skip_only_the_inward_world_hole() {
        let overlays = TestOverlays {
            rects: vec![
                egui::Rect::from_min_max(egui::pos2(3.1, 2.0), egui::pos2(5.9, 5.0)),
                egui::Rect::from_min_max(egui::pos2(5.0, 3.0), egui::pos2(8.0, 6.0)),
            ],
            cover_all: false,
        };
        let world = egui::Rect::from_min_max(egui::pos2(2.2, 1.2), egui::pos2(9.8, 7.8));
        let regions = overlays.paint_regions([15, 12], world, 1.5).unwrap();
        let mut counts = [[0; 15]; 12];
        for [left, top, right, bottom] in regions {
            for row in &mut counts[top..bottom] {
                for cell in &mut row[left..right] {
                    *cell += 1;
                }
            }
        }
        for (y, row) in counts.iter().enumerate() {
            for (x, count) in row.iter().enumerate() {
                let chrome = !(4..14).contains(&x) || !(2..11).contains(&y);
                let popup = ((3..10).contains(&x) && (2..9).contains(&y)) || ((6..13).contains(&x) && (3..10).contains(&y));
                assert_eq!(*count, u8::from(chrome || popup), "pixel {x},{y}");
            }
        }
        let fallback = TestOverlays {
            cover_all: true,
            ..Default::default()
        };
        assert!(fallback.paint_regions([15, 12], world, 1.5).is_none());
    }
}
