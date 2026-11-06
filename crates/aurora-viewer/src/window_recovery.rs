//! Gentle recovery after a display disappears or its desktop origin changes.
//!
//! LLWindowWin32's WM_DISPLAYCHANGE dispatches handleDisplayChanged
//! (indra/llwindow/llwindowwin32.cpp). Unlike startup restore, a live viewer
//! leaves every partly visible spanning layout alone. It only recovers a
//! wholly inaccessible window after both topology and geometry settle.

use std::time::{Duration, Instant};

pub const POLL: Duration = Duration::from_secs(1);
pub const GRACE: Duration = Duration::from_secs(3);
#[cfg(any(target_os = "linux", test))]
pub const INPUT_MAX_AGE: Duration = Duration::from_millis(250);

/// A native reply authorizes only the request that produced it. Time spent
/// queued or blocked on the server counts against freshness, too.
#[cfg(any(target_os = "linux", test))]
pub fn pointer_reply(request: (u64, Instant), reply: (u64, Instant, Option<bool>), now: Instant) -> Option<bool> {
    if request.0 != reply.0
        || now.checked_duration_since(request.1)? > INPUT_MAX_AGE
        || reply.1.checked_duration_since(request.1)? > INPUT_MAX_AGE
        || now.checked_duration_since(reply.1)? > INPUT_MAX_AGE
    {
        return None;
    }
    reply.2
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Display {
    pub bounds: [i64; 4],
    scale_milli: u32,
}

impl Display {
    pub fn new(position: (i32, i32), size: (u32, u32), scale: f64) -> Self {
        let (x, y) = (i64::from(position.0), i64::from(position.1));
        Self {
            bounds: [x, y, x + i64::from(size.0), y + i64::from(size.1)],
            scale_milli: if scale.is_finite() && scale > 0.0 {
                (scale * 1000.0).round() as u32
            } else {
                1000
            },
        }
    }

    fn valid(self) -> bool {
        let [left, top, right, bottom] = self.bounds;
        right > left && bottom > top
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    /// Outer origin and both sizes are physical pixels, including negatives.
    pub position: (i32, i32),
    pub outer: (u32, u32),
    pub inner: (u32, u32),
}

impl Geometry {
    fn valid(self) -> bool {
        self.outer.0 > 0 && self.outer.1 > 0 && self.inner.0 > 0 && self.inner.1 > 0
    }

    fn bounds(self) -> [i64; 4] {
        let (x, y) = (i64::from(self.position.0), i64::from(self.position.1));
        [x, y, x + i64::from(self.outer.0), y + i64::from(self.outer.1)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub position: (i32, i32),
    /// None preserves a window that already fits the surviving display.
    pub inner_size: Option<(u32, u32)>,
}

/// No bounding-desktop shortcut: disconnected displays can leave a hole.
pub fn visible(window: Geometry, displays: &[Display]) -> bool {
    if !window.valid() {
        return false;
    }
    let [left, top, right, bottom] = window.bounds();
    displays.iter().any(|d| {
        let [dl, dt, dr, db] = d.bounds;
        d.valid() && left < dr && right > dl && top < db && bottom > dt
    })
}

fn plan(window: Geometry, displays: &[Display]) -> Option<Plan> {
    if !window.valid() || visible(window, displays) {
        return None;
    }
    let [left, top, right, bottom] = window.bounds();
    let target = displays.iter().min_by(|a, b| {
        let distance = |d: &Display| {
            let [dl, dt, dr, db] = d.bounds;
            let x = (dl - right).max(left - dr).max(0) as f64;
            let y = (dt - bottom).max(top - db).max(0) as f64;
            x * x + y * y
        };
        distance(a).total_cmp(&distance(b))
    })?;
    let [left, top, right, bottom] = target.bounds;
    let (width, height) = ((right - left) as u32, (bottom - top) as u32);
    let resize = window.outer.0 > width || window.outer.1 > height;
    // The startup restore also uses 80% of the surviving display. Work in
    // physical pixels here so a DPI transition cannot move the target twice.
    let outer = if resize {
        (
            ((u64::from(width) * 4 / 5) as u32).max(1),
            ((u64::from(height) * 4 / 5) as u32).max(1),
        )
    } else {
        window.outer
    };
    let decoration = (
        window.outer.0.saturating_sub(window.inner.0),
        window.outer.1.saturating_sub(window.inner.1),
    );
    let coordinate = |value: i64| value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    Some(Plan {
        position: (
            coordinate(left + (i64::from(width) - i64::from(outer.0)) / 2),
            coordinate(top + (i64::from(height) - i64::from(outer.1)) / 2),
        ),
        inner_size: resize.then(|| {
            (
                outer.0.saturating_sub(decoration.0).max(1),
                outer.1.saturating_sub(decoration.1).max(1),
            )
        }),
    })
}

struct Pending {
    since: Instant,
    geometry: Option<Geometry>,
}

#[derive(Default)]
pub struct Recovery {
    initialized: bool,
    displays: Vec<Display>,
    checked: Option<Instant>,
    pending: Option<Pending>,
}

impl Recovery {
    pub fn due(&self, now: Instant) -> bool {
        self.checked.is_none_or(|t| now.saturating_duration_since(t) >= POLL)
    }

    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }

    /// An excluded window or unknown native state restarts the grace period.
    /// The first valid topology is only a baseline, never a reason to move.
    pub fn observe(&mut self, now: Instant, displays: &[Display], geometry: Option<Geometry>, blocked: bool) -> Option<Plan> {
        self.checked = Some(now);
        let mut displays: Vec<_> = displays.iter().copied().filter(|d| d.valid()).collect();
        displays.sort_unstable();
        displays.dedup();
        if displays.is_empty() {
            if let Some(pending) = &mut self.pending {
                pending.since = now;
                pending.geometry = None;
            }
            return None;
        }
        let geometry = geometry.filter(|g| g.valid());
        if self.displays != displays {
            self.displays = displays;
            if self.initialized {
                self.pending = Some(Pending {
                    since: now,
                    geometry: None,
                });
            }
            self.initialized = true;
        }
        if geometry.is_some_and(|g| visible(g, &self.displays)) {
            // A window manager or the user has already made it reachable.
            // Never retain permission for a later, unrelated manual move.
            self.pending = None;
            return None;
        }
        let pending = self.pending.as_mut()?;
        if blocked || geometry.is_none() {
            pending.since = now;
            pending.geometry = None;
            return None;
        }
        if pending.geometry != geometry {
            pending.since = now;
            pending.geometry = geometry;
            return None;
        }
        if now.saturating_duration_since(pending.since) < GRACE {
            return None;
        }
        self.pending = None;
        plan(geometry?, &self.displays)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(x: i32, y: i32, width: u32, height: u32) -> Display {
        Display::new((x, y), (width, height), 1.0)
    }

    fn window(x: i32, y: i32, width: u32, height: u32) -> Geometry {
        Geometry {
            position: (x, y),
            outer: (width, height),
            inner: (width.saturating_sub(16), height.saturating_sub(40)),
        }
    }

    #[test]
    fn live_recovery_waits_for_a_changed_topology_and_settled_geometry_then_moves_once() {
        let mut recovery = Recovery::default();
        let t = Instant::now();
        let dual = [display(0, 0, 1920, 1080), display(1920, 0, 1920, 1080)];
        let lost = Some(window(2100, 100, 1280, 800));
        assert!(recovery.observe(t, &dual, lost, false).is_none());
        assert!(recovery.observe(t + GRACE * 2, &dual, lost, false).is_none());
        assert!(!recovery.pending());
        let changed = t + GRACE * 3;
        assert!(recovery.observe(changed, &dual[..1], lost, false).is_none());
        assert!(
            recovery
                .observe(changed + GRACE - Duration::from_millis(1), &dual[..1], lost, false)
                .is_none()
        );
        assert_eq!(
            recovery.observe(changed + GRACE, &dual[..1], lost, false),
            Some(Plan {
                position: (320, 140),
                inner_size: None
            })
        );
        assert!(recovery.observe(changed + GRACE * 2, &dual[..1], lost, false).is_none());
        assert!(!recovery.pending());
    }

    #[test]
    fn dragging_or_excluded_states_require_a_fresh_grace_period_before_recovery() {
        for blocked_for in [1, 4, 20] {
            let mut recovery = Recovery::default();
            let t = Instant::now();
            let dual = [display(0, 0, 1920, 1080), display(1920, 0, 1920, 1080)];
            let lost = Some(window(2100, 100, 1280, 800));
            recovery.observe(t, &dual, lost, false);
            recovery.observe(t + POLL, &dual[..1], lost, false);
            let end = t + POLL + GRACE * blocked_for;
            assert!(recovery.observe(end, &dual[..1], lost, true).is_none());
            assert!(recovery.pending());
            assert!(recovery.observe(end + POLL, &dual[..1], lost, false).is_none());
            assert!(
                recovery
                    .observe(end + POLL + GRACE - Duration::from_millis(1), &dual[..1], lost, false)
                    .is_none()
            );
            assert!(recovery.observe(end + POLL + GRACE, &dual[..1], lost, false).is_some());
        }
    }

    #[test]
    fn moving_geometry_and_further_dpi_or_origin_changes_restart_the_wait() {
        let mut recovery = Recovery::default();
        let t = Instant::now();
        let dual = [display(0, 0, 1920, 1080), display(1920, 0, 1920, 1080)];
        let lost = Some(window(2100, 100, 1280, 800));
        recovery.observe(t, &dual, lost, false);
        recovery.observe(t + POLL, &dual[..1], lost, false);
        let moved = Some(window(2101, 101, 1280, 800));
        assert!(recovery.observe(t + GRACE, &dual[..1], moved, false).is_none());
        let changed = [Display::new((0, -100), (1920, 1080), 1.25)];
        assert!(recovery.observe(t + GRACE * 2, &changed, moved, false).is_none());
        assert!(
            recovery
                .observe(t + GRACE * 3 - Duration::from_millis(1), &changed, moved, false)
                .is_none()
        );
        assert!(recovery.observe(t + GRACE * 3, &changed, moved, false).is_some());
    }

    #[test]
    fn every_partly_visible_span_is_preserved_including_negative_origins_and_one_pixel() {
        let displays = [display(-1920, -200, 1920, 1080), display(0, 0, 1920, 1080)];
        for geometry in [
            window(-3000, -199, 1081, 800),
            window(1919, 400, 3840, 1080),
            window(-1920, -200, 5760, 1080),
        ] {
            assert!(plan(geometry, &displays).is_none());
        }
        // Touching an edge without any visible pixel does not count.
        assert!(plan(window(1920, 400, 1280, 800), &displays).is_some());
    }

    #[test]
    fn recovery_uses_real_monitor_rectangles_and_nearest_survivor_not_the_desktop_box() {
        let displays = [display(-1920, -1080, 1920, 1080), display(0, 0, 1920, 1080)];
        let geometry = window(200, -900, 800, 600);
        assert_eq!(
            plan(geometry, &displays),
            Some(Plan {
                position: (-1360, -840),
                inner_size: None
            })
        );
        let negative = [display(-1920, 0, 1920, 1080)];
        assert_eq!(
            plan(window(100, 100, 1280, 800), &negative),
            Some(Plan {
                position: (-1600, 140),
                inner_size: None
            })
        );
    }

    #[test]
    fn oversized_recovery_uses_eighty_percent_and_preserves_native_decoration_allowance() {
        let target = [Display::new((-2560, 0), (2560, 1440), 1.5)];
        assert_eq!(
            plan(window(0, 100, 3840, 2160), &target),
            Some(Plan {
                position: (-2304, 144),
                inner_size: Some((2032, 1112))
            })
        );
    }

    #[test]
    fn missing_native_geometry_or_displays_fail_closed_and_never_cause_a_late_snap() {
        let mut recovery = Recovery::default();
        let t = Instant::now();
        let dual = [display(0, 0, 1920, 1080), display(1920, 0, 1920, 1080)];
        let lost = Some(window(2100, 100, 1280, 800));
        recovery.observe(t, &dual, lost, false);
        recovery.observe(t + POLL, &dual[..1], lost, false);
        assert!(recovery.observe(t + GRACE * 2, &[], lost, false).is_none());
        assert!(recovery.observe(t + GRACE * 3, &dual[..1], None, false).is_none());
        assert!(recovery.observe(t + GRACE * 4, &dual[..1], lost, false).is_none());
        let returned = Some(window(100, 100, 1280, 800));
        assert!(recovery.observe(t + GRACE * 5, &dual[..1], returned, false).is_none());
        assert!(recovery.observe(t + GRACE * 6, &dual[..1], returned, false).is_none());
        assert!(!recovery.pending());
        assert!(recovery.observe(t + GRACE * 7, &dual[..1], lost, false).is_none());
    }

    #[test]
    fn enumeration_order_duplicates_and_invalid_displays_do_not_arm_recovery() {
        let mut recovery = Recovery::default();
        let t = Instant::now();
        let a = display(0, 0, 1920, 1080);
        let b = display(1920, 0, 1920, 1080);
        recovery.observe(t, &[a, b], Some(window(5000, 0, 800, 600)), false);
        assert!(!recovery.due(t + POLL - Duration::from_millis(1)));
        assert!(recovery.due(t + POLL));
        recovery.observe(t + GRACE, &[b, a, a, display(0, 0, 0, 0)], Some(window(5000, 0, 800, 600)), false);
        assert!(!recovery.pending());
        assert!(!recovery.due(t));
    }

    #[test]
    fn old_pointer_replies_cannot_authorize_new_requests_or_a_slow_native_query() {
        let t = Instant::now();
        let sampled = t + Duration::from_millis(20);
        assert_eq!(pointer_reply((4, t), (4, sampled, Some(false)), sampled), Some(false));
        assert_eq!(pointer_reply((4, t), (4, sampled, Some(true)), sampled), Some(true));
        assert_eq!(pointer_reply((5, t), (4, sampled, Some(false)), sampled), None);
        assert_eq!(pointer_reply((4, t), (4, sampled, None), sampled), None);
        let slow = t + INPUT_MAX_AGE + Duration::from_millis(1);
        assert_eq!(pointer_reply((4, t), (4, slow, Some(false)), slow), None);
        let delayed = t + Duration::from_millis(200);
        assert_eq!(
            pointer_reply((4, t), (4, delayed, Some(false)), delayed + Duration::from_millis(200)),
            None
        );
        assert_eq!(pointer_reply((4, t), (4, sampled, Some(false)), t + INPUT_MAX_AGE), Some(false));
        assert_eq!(
            pointer_reply(
                (4, t),
                (4, sampled, Some(false)),
                sampled + INPUT_MAX_AGE + Duration::from_millis(1)
            ),
            None
        );
        assert_eq!(pointer_reply((4, sampled), (4, t, Some(false)), sampled), None);
        assert_eq!(pointer_reply((4, t), (4, sampled, Some(false)), t), None);
    }

    #[test]
    fn a_visible_layout_clears_pending_immediately_even_while_input_is_blocked() {
        let mut recovery = Recovery::default();
        let t = Instant::now();
        let dual = [display(0, 0, 1920, 1080), display(1920, 0, 1920, 1080)];
        let lost = Some(window(2100, 100, 1280, 800));
        recovery.observe(t, &dual, lost, false);
        recovery.observe(t + POLL, &dual[..1], lost, false);
        assert!(recovery.pending());
        recovery.observe(t + POLL * 2, &dual[..1], Some(window(1919, 0, 1280, 800)), true);
        assert!(!recovery.pending());
        assert!(recovery.observe(t + GRACE * 4, &dual[..1], lost, false).is_none());
    }

    #[test]
    fn a_topology_change_during_a_temporary_native_state_survives_until_normal_restore() {
        let mut recovery = Recovery::default();
        let t = Instant::now();
        let dual = [display(0, 0, 1920, 1080), display(1920, 0, 1920, 1080)];
        let lost_normal = Some(window(2100, 100, 1280, 800));
        recovery.observe(t, &dual, lost_normal, false);
        // A visible fullscreen/maximized/VR window, a minimized/hidden
        // window and unknown native state all withhold the normal rect.
        assert!(recovery.observe(t + POLL, &dual[..1], None, true).is_none());
        assert!(recovery.pending());
        let restored = t + GRACE * 10;
        assert!(recovery.observe(restored, &dual[..1], None, true).is_none());
        assert!(
            recovery.pending(),
            "visible temporary geometry cannot disarm normal-window recovery"
        );
        assert!(recovery.observe(restored + POLL, &dual[..1], lost_normal, false).is_none());
        assert!(
            recovery
                .observe(restored + POLL + GRACE - Duration::from_millis(1), &dual[..1], lost_normal, false)
                .is_none()
        );
        assert_eq!(
            recovery.observe(restored + POLL + GRACE, &dual[..1], lost_normal, false),
            Some(Plan {
                position: (320, 140),
                inner_size: None
            })
        );
    }
}
