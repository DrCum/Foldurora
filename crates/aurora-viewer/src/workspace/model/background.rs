//! Account-scoped padding colors. Display keys describe geometry, never names,
//! native monitor identifiers or file paths. Optional texture references are
//! private metadata; native permission and privacy proofs are required to use them.
use serde::{Deserialize, Serialize};
mod displays;
pub(crate) use displays::{Arrangement, DisplayColors, MAX_DISPLAYS};
mod images;
pub(crate) use images::{DisplayPaints, ImageMode, Paint, Picture, TextureRef};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Background {
    Black {},
    Solid { rgb: [u8; 3] },
    Displays { common: [u8; 3], rules: DisplayColors },
    Images { common: Paint, rules: Option<DisplayPaints> },
}
impl Default for Background {
    fn default() -> Self {
        Self::Black {}
    }
}
impl Background {
    pub(crate) fn is_black(&self) -> bool {
        matches!(self, Self::Black { .. })
    }
    pub(crate) fn common(&self) -> [u8; 3] {
        match self {
            Self::Black {} => [0; 3],
            Self::Solid { rgb } => *rgb,
            Self::Displays { common, .. } => *common,
            Self::Images { common, .. } => common.fallback(),
        }
    }
    pub(crate) fn color(&self, current: &Arrangement, index: usize) -> [u8; 3] {
        match self {
            Self::Displays { rules, .. } if rules.arrangement() == current => rules.color(index).unwrap_or_else(|| self.common()),
            Self::Images { .. } => self.paint(Some(current), Some(index)).fallback(),
            _ => self.common(),
        }
    }
    pub(crate) fn with_common(&self, rgb: [u8; 3]) -> Self {
        match self {
            Self::Displays { rules, .. } => Self::Displays {
                common: rgb,
                rules: rules.clone(),
            },
            Self::Images { rules, .. } => Self::Images {
                common: Paint::Color { rgb },
                rules: rules.clone(),
            },
            _ => Self::Solid { rgb },
        }
    }
    pub(crate) fn with_display(&self, current: &Arrangement, index: usize, rgb: Option<[u8; 3]>) -> Option<Self> {
        if let Self::Images { common, rules } = self {
            if common.spanning() {
                return None;
            }
            let mut rules = rules
                .as_ref()
                .filter(|r| r.arrangement() == current)
                .cloned()
                .unwrap_or_else(|| DisplayPaints::empty(current.clone()));
            rules.set(index, rgb.map(|rgb| Paint::Color { rgb }))?;
            return Some(Self::Images {
                common: common.clone(),
                rules: Some(rules),
            });
        }
        let mut rules = match self {
            Self::Displays { rules, .. } if rules.arrangement() == current => rules.clone(),
            _ => DisplayColors::empty(current.clone()),
        };
        rules.set(index, rgb)?;
        Some(Self::Displays {
            common: self.common(),
            rules,
        })
    }
    pub(crate) fn arrangement(&self) -> Option<&Arrangement> {
        match self {
            Self::Displays { rules, .. } => Some(rules.arrangement()),
            Self::Images { rules, .. } => rules.as_ref().map(DisplayPaints::arrangement),
            _ => None,
        }
    }
    pub(crate) fn paint(&self, current: Option<&Arrangement>, index: Option<usize>) -> Paint {
        match self {
            Self::Images { common, rules } => {
                if !common.spanning()
                    && let (Some(a), Some(i), Some(r)) = (current, index, rules)
                    && r.arrangement() == a
                    && let Some(paint) = r.paint(i)
                {
                    return paint.clone();
                }
                common.clone()
            }
            _ => Paint::Color {
                rgb: current.zip(index).map_or_else(|| self.common(), |(a, i)| self.color(a, i)),
            },
        }
    }
    pub(crate) fn with_picture(&self, target: Option<(&Arrangement, usize)>, picture: Picture) -> Option<Self> {
        if target.is_some() && picture.mode == ImageMode::Span {
            return None;
        }
        let (mut common, mut rules) = match self {
            Self::Images { common, rules } => (common.clone(), rules.clone()),
            Self::Displays { common, rules } => {
                let mut paints = DisplayPaints::empty(rules.arrangement().clone());
                for i in 0..rules.arrangement().len() {
                    paints.set(i, rules.color(i).map(|rgb| Paint::Color { rgb }))?;
                }
                (Paint::Color { rgb: *common }, Some(paints))
            }
            _ => (Paint::Color { rgb: self.common() }, None),
        };
        let fallback = self.paint(target.map(|t| t.0), target.map(|t| t.1)).fallback();
        let paint = Paint::Texture { picture, fallback };
        match target {
            None => common = paint,
            Some((a, i)) => {
                let mut slots = rules
                    .filter(|r| r.arrangement() == a)
                    .unwrap_or_else(|| DisplayPaints::empty(a.clone()));
                slots.set(i, Some(paint))?;
                rules = Some(slots);
            }
        }
        Some(Self::Images { common, rules })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_background_choices_roundtrip_without_asset_or_path_fields() {
        for choice in [Background::Black {}, Background::Solid { rgb: [10, 20, 30] }] {
            let bytes = serde_json::to_vec(&choice).unwrap();
            assert_eq!(serde_json::from_slice::<Background>(&bytes).unwrap(), choice);
        }
    }
    #[test]
    fn workspace_background_unknown_kind_extra_fields_and_invalid_channels_are_refused() {
        for text in [
            r#"{"kind":"file","path":"private"}"#,
            r#"{"kind":"black","path":"private"}"#,
            r#"{"kind":"solid","rgb":[256,0,0]}"#,
            r#"{"kind":"solid","rgb":[-1,0,0]}"#,
            r#"{"kind":"solid","rgb":[1.5,0,0]}"#,
            r#"{"kind":"solid","rgb":[0,0]}"#,
        ] {
            assert!(serde_json::from_str::<Background>(text).is_err(), "{text}");
        }
    }
    #[test]
    fn workspace_display_background_roundtrip_preserves_common_and_only_matches_original_arrangement() {
        let two = Arrangement::from_physical(&[[0, 0, 100, 100], [100, 0, 200, 100]]).unwrap();
        let one = Arrangement::from_physical(&[[0, 0, 100, 100]]).unwrap();
        let common = Background::Solid { rgb: [1, 2, 3] };
        let choice = common.with_display(&two, 1, Some([7, 8, 9])).unwrap();
        let bytes = serde_json::to_vec(&choice).unwrap();
        assert_eq!(serde_json::from_slice::<Background>(&bytes).unwrap(), choice);
        assert_eq!(choice.color(&two, 0), common.common());
        assert_eq!(choice.color(&two, 1), [7, 8, 9]);
        assert_eq!(choice.color(&one, 0), common.common());
        assert_eq!(choice.color(&one, 1), common.common());
        assert!(choice.with_display(&two, two.len(), Some([0; 3])).is_none());
        let reset = choice.with_display(&two, 1, None).unwrap();
        assert_eq!(reset.color(&two, 1), common.common());
        let updated = choice.with_common([4, 5, 6]);
        assert_eq!(updated.color(&two, 0), [4, 5, 6]);
        assert_eq!(updated.color(&two, 1), [7, 8, 9]);
        let changed_topology = choice.with_display(&one, 0, Some([10, 11, 12])).unwrap();
        assert_eq!(changed_topology.common(), common.common());
        assert_eq!(changed_topology.color(&two, 1), common.common());
        assert_eq!(changed_topology.color(&one, 0), [10, 11, 12]);
    }
    #[test]
    fn workspace_picture_upgrade_retains_display_colors_original_grid_and_scoped_removal() {
        let a = Arrangement::from_physical(&[[0, 0, 100, 100], [100, 0, 200, 100]]).unwrap();
        let colors = Background::Solid { rgb: [1, 2, 3] }.with_display(&a, 1, Some([4, 5, 6])).unwrap();
        let picture = Picture {
            texture: TextureRef::new(uuid::Uuid::new_v4(), uuid::Uuid::new_v4(), "a".repeat(64)).unwrap(),
            mode: ImageMode::Fill,
            brightness: 180,
        };
        let image = colors.with_picture(None, picture.clone()).unwrap();
        assert_eq!(image.paint(None, None).picture(), Some(&picture));
        assert!(image.paint(Some(&a), Some(1)).picture().is_none());
        assert_eq!(image.color(&a, 1), [4, 5, 6]);
        let restored = serde_json::from_slice::<Background>(&serde_json::to_vec(&image).unwrap()).unwrap();
        assert_eq!(restored, image);
        let cleared = image.with_common([7, 8, 9]);
        assert!(cleared.paint(None, None).picture().is_none());
        assert_eq!(cleared.color(&a, 1), [4, 5, 6]);
        let span = image
            .with_picture(
                None,
                Picture {
                    mode: ImageMode::Span,
                    ..picture
                },
            )
            .unwrap();
        assert!(span.paint(Some(&a), Some(1)).spanning());
        assert!(span.with_display(&a, 1, None).is_none());
    }
}
