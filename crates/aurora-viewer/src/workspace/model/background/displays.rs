//! Bounded, origin-independent display arrangement. Same geometry at another
//! desktop origin matches; missing/rearranged displays fall back to common.
use serde::{Deserialize, Serialize};
pub(crate) const MAX_DISPLAYS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Vec<[u16; 4]>", into = "Vec<[u16; 4]>")]
pub(crate) struct Arrangement(Vec<[u16; 4]>);
impl TryFrom<Vec<[u16; 4]>> for Arrangement {
    type Error = &'static str;
    fn try_from(rects: Vec<[u16; 4]>) -> Result<Self, Self::Error> {
        if rects.is_empty()
            || rects.len() > MAX_DISPLAYS
            || rects.iter().any(|[l, t, r, b]| l >= r || t >= b || *r > 1000 || *b > 1000)
            || rects.windows(2).any(|w| w[0] >= w[1])
            || rects.iter().map(|r| r[0]).min() != Some(0)
            || rects.iter().map(|r| r[1]).min() != Some(0)
            || rects.iter().map(|r| r[2]).max() != Some(1000)
            || rects.iter().map(|r| r[3]).max() != Some(1000)
        {
            return Err("display arrangement");
        }
        Ok(Self(rects))
    }
}
impl From<Arrangement> for Vec<[u16; 4]> {
    fn from(value: Arrangement) -> Self {
        value.0
    }
}
impl Arrangement {
    /// Caller supplies sorted, deduplicated physical rectangles. Reject excess
    /// or invalid displays rather than silently assigning colors to a prefix.
    pub(crate) fn from_physical(rects: &[[i64; 4]]) -> Option<Self> {
        if rects.is_empty()
            || rects.len() > MAX_DISPLAYS
            || rects
                .iter()
                .any(|r| r[0] >= r[2] || r[1] >= r[3] || r.iter().any(|v| v.unsigned_abs() > (1u64 << 34)))
            || rects.windows(2).any(|w| w[0] >= w[1])
        {
            return None;
        }
        let left = rects.iter().map(|r| r[0]).min()?;
        let top = rects.iter().map(|r| r[1]).min()?;
        let width = rects.iter().map(|r| r[2]).max()? - left;
        let height = rects.iter().map(|r| r[3]).max()? - top;
        let normalized = rects
            .iter()
            .map(|r| {
                std::array::from_fn(|i| {
                    let (origin, extent) = if i % 2 == 0 { (left, width) } else { (top, height) };
                    (((r[i] - origin) as f64 * 1000.0 / extent as f64).round()) as u16
                })
            })
            .collect::<Vec<_>>();
        Self::try_from(normalized).ok()
    }
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Wire", into = "Wire")]
pub(crate) struct DisplayColors {
    arrangement: Arrangement,
    colors: Vec<Option<[u8; 3]>>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    arrangement: Arrangement,
    colors: Vec<Option<[u8; 3]>>,
}
impl TryFrom<Wire> for DisplayColors {
    type Error = &'static str;
    fn try_from(w: Wire) -> Result<Self, Self::Error> {
        if w.colors.len() != w.arrangement.len() {
            return Err("display color count");
        }
        Ok(Self {
            arrangement: w.arrangement,
            colors: w.colors,
        })
    }
}
impl From<DisplayColors> for Wire {
    fn from(v: DisplayColors) -> Self {
        Self {
            arrangement: v.arrangement,
            colors: v.colors,
        }
    }
}
impl DisplayColors {
    pub(crate) fn empty(arrangement: Arrangement) -> Self {
        Self {
            colors: vec![None; arrangement.len()],
            arrangement,
        }
    }
    pub(crate) fn arrangement(&self) -> &Arrangement {
        &self.arrangement
    }
    pub(crate) fn color(&self, index: usize) -> Option<[u8; 3]> {
        self.colors.get(index).copied().flatten()
    }
    pub(crate) fn set(&mut self, index: usize, rgb: Option<[u8; 3]>) -> Option<()> {
        *self.colors.get_mut(index)? = rgb;
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_display_background_geometry_ignores_origin_and_order_requires_canonical_bounds() {
        let rects = [[-1920, 0, 0, 1080], [0, 0, 1920, 1080]];
        let a = Arrangement::from_physical(&rects).unwrap();
        let translated = rects.map(|r| [r[0] + 800, r[1] - 40, r[2] + 800, r[3] - 40]);
        assert_eq!(a, Arrangement::from_physical(&translated).unwrap());
        assert_ne!(a, Arrangement::from_physical(&[[0, 0, 1920, 1080]]).unwrap());
        assert!(Arrangement::from_physical(&[rects[1], rects[0]]).is_none());
        assert!(Arrangement::from_physical(&[rects[0], rects[0]]).is_none());
        assert!(Arrangement::from_physical(&[[0, 0, 0, 1080]]).is_none());
        assert!(Arrangement::from_physical(&[[i64::MIN, 0, i64::MAX, 1]]).is_none());
    }
    #[test]
    fn workspace_display_background_codec_refuses_excess_collapsed_unknown_and_mismatched_colors() {
        for text in [
            "[]",
            "[[0,0,0,1000]]",
            "[[0,0,1001,1000]]",
            "[[1,0,1000,1000]]",
            "[[0,0,1000,1000],[0,0,1000,1000]]",
        ] {
            assert!(serde_json::from_str::<Arrangement>(text).is_err(), "{text}");
        }
        let too_many = vec![[0, 0, 1000, 1000]; MAX_DISPLAYS + 1];
        assert!(Arrangement::try_from(too_many).is_err());
        for text in [
            r#"{"arrangement":[[0,0,1000,1000]],"colors":[]}"#,
            r#"{"arrangement":[[0,0,1000,1000]],"colors":[null],"name":"private"}"#,
        ] {
            assert!(serde_json::from_str::<DisplayColors>(text).is_err(), "{text}");
        }
    }
}
