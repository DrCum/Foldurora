//! Workspace quality snapshots reuse the native rendering-only preset allowlist.
//! No hardware, pacing, account, environment or credential settings are copied.
use crate::{graphics_presets::saved::Saved, settings::Settings};
use serde::{Deserialize, Serialize};

mod codec;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
struct Quality(String, Option<Saved>);

impl TryFrom<String> for Quality {
    type Error = &'static str;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        if text.len() > codec::MAX_BYTES {
            return Err("graphics size");
        }
        let input: serde_json::Value = serde_json::from_str(&text).map_err(|_| "graphics schema")?;
        let saved: Saved = serde_json::from_value(input.clone()).map_err(|_| "graphics schema")?;
        let typed = serde_json::to_value(&saved).map_err(|_| "graphics schema")?;
        if !codec::known_fields(&input, &typed) {
            return Err("graphics fields");
        }
        // Use the same numeric/range normalization as loading a native preset.
        let mut settings = Settings::default();
        saved.apply(&mut settings);
        let values = Saved::read(&settings);
        let normalized = serde_json::to_string(&values).map_err(|_| "graphics schema")?;
        if normalized.len() > codec::MAX_BYTES {
            return Err("graphics size");
        }
        // Persist only a readable canonical snapshot, including when native
        // normalization encounters malformed/nonfinite external numbers.
        let checked: Saved = serde_json::from_str(&normalized).map_err(|_| "graphics values")?;
        Ok(Self(normalized, Some(checked)))
    }
}
impl From<Quality> for String {
    fn from(value: Quality) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Graphics {
    version: u8,
    values: Quality,
    selected: Option<String>,
}
impl Graphics {
    pub(crate) fn capture(settings: &Settings) -> Self {
        let text = serde_json::to_string(&Saved::capture(settings)).unwrap_or_default();
        Self {
            version: 1,
            // Invalid live values make Layout::validate refuse the save; they
            // never silently drop an explicitly selected capture group.
            values: Quality::try_from(text).unwrap_or_else(|_| Quality(String::new(), None)),
            selected: crate::graphics_presets::saved::selected(settings)
                .filter(|name| name.len() <= 256 && crate::graphics_presets::saved::valid_name(name))
                .map(str::to_owned),
        }
    }
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.version != 1
            || self
                .selected
                .as_ref()
                .is_some_and(|name| name.len() > 256 || !crate::graphics_presets::saved::valid_name(name))
        {
            return Err("graphics snapshot");
        }
        self.values.1.as_ref().map(|_| ()).ok_or("graphics snapshot")
    }
    pub(crate) fn apply(&self, settings: &mut Settings) {
        if self.validate().is_err() {
            return;
        }
        let Some(values) = self.values.1.as_ref() else {
            return;
        };
        values.apply(settings);
        let current = Saved::read(settings);
        settings.graphics_saved_active = self
            .selected
            .clone()
            .filter(|name| settings.graphics_saved.get(name) == Some(&current));
    }
    pub(crate) fn matches_settings(&self, settings: &Settings) -> bool {
        let Some(values) = self.values.1.as_ref() else {
            return false;
        };
        let selected = self
            .selected
            .as_deref()
            .filter(|name| settings.graphics_saved.get(*name) == Some(values));
        Saved::capture(settings) == *values && crate::graphics_presets::saved::selected(settings) == selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_graphics_snapshot_restores_only_native_quality_and_current_profile_identity() {
        let mut settings = Settings::default();
        crate::graphics_presets::Preset::High.apply(&mut settings);
        settings.graphics_saved.insert("Own profile".into(), Saved::capture(&settings));
        settings.graphics_saved_active = Some("Own profile".into());
        let snapshot = Graphics::capture(&settings);
        let quality = Saved::read(&settings);
        crate::graphics_presets::Preset::Potato.apply(&mut settings);
        settings.fps_limit = 73;
        settings.vsync = false;
        settings.show_balance = false;
        snapshot.apply(&mut settings);
        assert_eq!(Saved::read(&settings), quality);
        assert_eq!(settings.graphics_saved_active.as_deref(), Some("Own profile"));
        assert_eq!((settings.fps_limit, settings.vsync, settings.show_balance), (73, false, false));
        settings.graphics_saved.clear();
        snapshot.apply(&mut settings);
        assert!(settings.graphics_saved_active.is_none());
    }
    #[test]
    fn workspace_graphics_snapshot_refuses_unknown_fields_oversize_and_future_versions() {
        let snapshot = Graphics::capture(&Settings::default());
        let mut value = serde_json::to_value(&snapshot).unwrap();
        let mut quality: serde_json::Value = serde_json::from_str(value["values"].as_str().unwrap()).unwrap();
        quality["credentials"] = "never imported".into();
        value["values"] = serde_json::to_string(&quality).unwrap().into();
        assert!(serde_json::from_value::<Graphics>(value).is_err());
        assert!(Quality::try_from(" ".repeat(codec::MAX_BYTES + 1)).is_err());
        let mut invalid = snapshot;
        invalid.version = 99;
        assert!(invalid.validate().is_err());
    }
}
