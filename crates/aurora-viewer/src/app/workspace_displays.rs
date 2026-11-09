//! Read-only, bounded native display sampling for workspace padding. Separate
//! from window recovery: sampling may never move, resize, dock or focus a window.
use crate::workspace::model::{Arrangement, MAX_DISPLAYS};
use std::time::{Duration, Instant};
use winit::window::{Window, WindowId};

#[derive(Default)]
pub(super) struct Displays {
    sample: Option<(WindowId, Instant, f32)>,
    arrangement: Option<Arrangement>,
    rectangles: Vec<egui::Rect>,
}
impl Displays {
    pub(super) fn prepare(&mut self, window: &Window, now: Instant, scale: f32, needed: bool) {
        if !needed {
            *self = Self::default();
            return;
        }
        if self.sample.is_some_and(|(id, time, old_scale)| {
            id == window.id() && old_scale == scale && now.saturating_duration_since(time) < Duration::from_millis(250)
        }) {
            return;
        }
        self.sample = Some((window.id(), now, scale));
        self.arrangement = None;
        self.rectangles.clear();
        if !scale.is_finite() || scale <= 0.0 {
            return;
        }
        let Ok(origin) = window.inner_position() else { return };
        let mut bounds = window
            .available_monitors()
            .take(MAX_DISPLAYS + 1)
            .map(|m| {
                let (p, s) = (m.position(), m.size());
                let (x, y) = (i64::from(p.x), i64::from(p.y));
                [x, y, x + i64::from(s.width), y + i64::from(s.height)]
            })
            .collect::<Vec<_>>();
        // Refuse excess before deduplication as well: the query is bounded and
        // must not accidentally turn a partial topology into a valid one.
        if bounds.len() > MAX_DISPLAYS {
            return;
        }
        bounds.sort_unstable();
        bounds.dedup();
        let Some(arrangement) = Arrangement::from_physical(&bounds) else {
            return;
        };
        self.rectangles = bounds
            .iter()
            .map(|r| {
                let point = |x: i64, y: i64| egui::pos2((x - i64::from(origin.x)) as f32 / scale, (y - i64::from(origin.y)) as f32 / scale);
                egui::Rect::from_min_max(point(r[0], r[1]), point(r[2], r[3]))
            })
            .collect();
        self.arrangement = Some(arrangement);
    }
    pub(super) fn arrangement(&self) -> Option<&Arrangement> {
        self.arrangement.as_ref()
    }
    pub(super) fn refresh(&mut self, window: &Window, scale: f32) {
        self.sample = None;
        self.prepare(window, Instant::now(), scale, true);
    }
    pub(super) fn rectangles(&self) -> &[egui::Rect] {
        &self.rectangles
    }
}
