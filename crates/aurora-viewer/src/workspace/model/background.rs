//! Account-scoped padding colors; no file paths or texture/asset references.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Background {
    Black {},
    Solid { rgb: [u8; 3] },
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
}
