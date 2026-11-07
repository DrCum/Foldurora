use crate::{Error, MAX_WORKERS};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewRate {
    Half,
    #[default]
    One,
    Two,
    Five,
}
impl PreviewRate {
    pub fn half_fps(self) -> u32 {
        match self {
            Self::Half => 1,
            Self::One => 2,
            Self::Two => 4,
            Self::Five => 10,
        }
    }
    pub fn interval_millis(self) -> u64 {
        2000 / self.half_fps() as u64
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewSize {
    #[default]
    Small,
    Medium,
    Large,
}
impl PreviewSize {
    pub fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Small => (320, 180),
            Self::Medium => (480, 270),
            Self::Large => (640, 360),
        }
    }
    pub fn bytes(self) -> u64 {
        let (w, h) = self.dimensions();
        u64::from(w) * u64::from(h) * 4
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreviewDemand {
    pub requested: Option<PreviewRate>,
    pub visible: bool,
    pub inactive: bool,
    pub session_current: bool,
    /// Both remain reserved until the worker acknowledges a lower policy.
    pub acknowledged_half_fps: u32,
    pub pending_half_fps: u32,
}

/// Pure admission, not rendering. Equal supported shares preserve the largest
/// possible total without admitting a new lane over acknowledged/in-flight
/// reservations. None also covers exhausted / unsafe rendering budgets.
pub fn admit_previews(demands: &[PreviewDemand], budget_half_fps: u32) -> Result<Vec<Option<PreviewRate>>, Error> {
    admit_previews_with_active(demands, budget_half_fps, true)
}

/// Each entry is one unique source after the broker deduplicates its views.
/// `has_active` comes from acknowledged controller state, never a window or
/// capture request; it is only capacity arithmetic and grants no rendering.
/// With no Active, four fixed views plus cycling can cover all five sources
/// (`fssessionmonitormodel.h::visibleSources`, Legacy Viewer).
pub fn admit_previews_with_active(
    demands: &[PreviewDemand],
    budget_half_fps: u32,
    has_active: bool,
) -> Result<Vec<Option<PreviewRate>>, Error> {
    if demands.len() > MAX_WORKERS - usize::from(has_active)
        || budget_half_fps > 20
        || demands.iter().any(|d| d.acknowledged_half_fps > 10 || d.pending_half_fps > 10)
    {
        return Err(Error::Capacity);
    }
    let eligible = |d: &PreviewDemand| d.visible && d.inactive && d.session_current && d.requested.is_some();
    let count = demands.iter().filter(|d| eligible(d)).count() as u32;
    let share = budget_half_fps / count.max(1);
    let rates = [PreviewRate::Five, PreviewRate::Two, PreviewRate::One, PreviewRate::Half];
    let desired: Vec<_> = demands
        .iter()
        .map(|d| {
            if !eligible(d) {
                return None;
            }
            rates
                .iter()
                .copied()
                .find(|r| r.half_fps() <= share && d.requested.is_some_and(|target| *r <= target))
        })
        .collect();
    let reservations: Vec<_> = demands.iter().map(|d| d.acknowledged_half_fps.max(d.pending_half_fps)).collect();
    let mut held = reservations.iter().sum::<u32>();
    Ok(desired
        .into_iter()
        .zip(reservations)
        .map(|(target, reserved)| {
            let requested = target.map_or(0, PreviewRate::half_fps);
            if requested <= reserved {
                return target;
            }
            let extra = requested - reserved;
            if extra <= budget_half_fps.saturating_sub(held) {
                held += extra;
                target
            } else {
                None
            }
        })
        .collect())
}
