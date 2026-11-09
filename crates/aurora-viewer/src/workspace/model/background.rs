//! Account-scoped padding colors. Display keys describe geometry, never names,
//! native monitor identifiers, file paths or texture/asset references.
use serde::{Deserialize, Serialize};
mod displays;
pub(crate) use displays::{Arrangement, DisplayColors, MAX_DISPLAYS};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Background {
    Black {},
    Solid { rgb: [u8; 3] },
    Displays { common: [u8; 3], rules: DisplayColors },
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
        }
    }
    pub(crate) fn color(&self, current: &Arrangement, index: usize) -> [u8; 3] {
        match self {
            Self::Displays { rules, .. } if rules.arrangement() == current => rules.color(index).unwrap_or_else(|| self.common()),
            _ => self.common(),
        }
    }
    pub(crate) fn with_common(&self, rgb: [u8; 3]) -> Self {
        match self {
            Self::Displays { rules, .. } => Self::Displays {
                common: rgb,
                rules: rules.clone(),
            },
            _ => Self::Solid { rgb },
        }
    }
    pub(crate) fn with_display(&self, current: &Arrangement, index: usize, rgb: Option<[u8; 3]>) -> Option<Self> {
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
}
