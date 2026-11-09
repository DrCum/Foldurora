use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(crate) const LIMIT: usize = 32;
pub(crate) const MAX_FILE: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Inventory,
    MiniMap,
    WorldMap,
    Conversations,
}
impl Role {
    pub(crate) const ALL: [Self; 4] = [Self::Inventory, Self::MiniMap, Self::WorldMap, Self::Conversations];
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Inventory => "inventory",
            Self::MiniMap => "minimap",
            Self::WorldMap => "worldmap",
            Self::Conversations => "conversations",
        }
    }
}

/// Indices are eight fixed perimeter points, not serialized egui identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Geometry {
    pub point: u8,
    pub offset: [f32; 2],
    pub size: [f32; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Viewport {
    pub enabled: bool,
    pub restrict_ui: bool,
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}
impl From<&crate::world_viewport::ViewportSettings> for Viewport {
    fn from(v: &crate::world_viewport::ViewportSettings) -> Self {
        Self {
            enabled: v.enabled,
            restrict_ui: v.restrict_ui,
            left: v.left,
            right: v.right,
            top: v.top,
            bottom: v.bottom,
        }
    }
}
impl From<&Viewport> for crate::world_viewport::ViewportSettings {
    fn from(v: &Viewport) -> Self {
        Self {
            enabled: v.enabled,
            restrict_ui: v.restrict_ui,
            left: v.left,
            right: v.right,
            top: v.top,
            bottom: v.bottom,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bars {
    pub classic: bool,
    pub stretch: bool,
    pub rights_text: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Layout {
    pub geometry: [Option<Geometry>; 4],
    pub viewport: Viewport,
    pub bars: Bars,
}
impl Layout {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        for g in self.geometry.iter().flatten() {
            if g.point >= 8
                || g.offset.iter().any(|v| !v.is_finite() || v.abs() > 32768.0)
                || g.size.iter().any(|v| !v.is_finite() || !(32.0..=16384.0).contains(v))
            {
                return Err("geometry");
            }
        }
        let v = &self.viewport;
        if [v.left, v.right, v.top, v.bottom]
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=95.0).contains(v))
            || v.left + v.right >= 100.0
            || v.top + v.bottom >= 100.0
        {
            return Err("viewport");
        }
        Ok(())
    }
    pub(crate) fn settings(&self, settings: &mut crate::settings::Settings, _hosted: bool) {
        // Relative world placement is independent of native host placement.
        // Workspace preview and cancel must restore it while hosted too.
        settings.world_viewport = (&self.viewport).into();
        settings.classic_bars = self.bars.classic;
        settings.place_bar_stretch = self.bars.stretch;
        settings.place_bar_rights_text = self.bars.rights_text;
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    pub id: Uuid,
    pub name: String,
    pub layout: Layout,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Catalogue {
    pub version: u32,
    pub account: Uuid,
    pub grid: String,
    pub revision: u64,
    pub records: Vec<Record>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub favorites: Vec<Uuid>,
    #[serde(default, skip_serializing_if = "Startup::disabled")]
    pub startup: Startup,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_layout: Option<Layout>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub show_favorites: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Startup {
    #[default]
    Disabled,
    LastArrangement,
    Named {
        workspace: Uuid,
    },
}
impl Startup {
    fn disabled(&self) -> bool {
        *self == Self::Disabled
    }
}
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty() && name == name.trim() && name.len() <= 96 && name.chars().count() <= 64 && !name.chars().any(char::is_control)
}
impl Catalogue {
    pub(crate) fn name_available(&self, name: &str, except: Option<Uuid>) -> bool {
        valid_name(name)
            && !self
                .records
                .iter()
                .any(|r| Some(r.id) != except && r.name.to_lowercase() == name.to_lowercase())
    }
    pub(crate) fn empty(account: Uuid, grid: String) -> Self {
        Self {
            version: 1,
            account,
            grid,
            revision: 1,
            records: Vec::new(),
            favorites: Vec::new(),
            startup: Startup::Disabled,
            last_layout: None,
            show_favorites: false,
        }
    }
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.version != 1
            || self.account.is_nil()
            || aurora_session::GridKey::parse_hex(&self.grid).is_err()
            || self.revision == 0
            || self.records.len() > LIMIT
        {
            return Err("catalogue");
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        for r in &self.records {
            if r.id.is_nil() || !ids.insert(r.id) || !valid_name(&r.name) || !names.insert(r.name.to_lowercase()) {
                return Err("record");
            }
            r.layout.validate()?;
        }
        let mut favorites = std::collections::BTreeSet::new();
        if self.favorites.len() > LIMIT
            || self.favorites.iter().any(|id| !ids.contains(id) || !favorites.insert(*id))
            || matches!(self.startup, Startup::Named { workspace } if !ids.contains(&workspace))
        {
            return Err("workspace reference");
        }
        if let Some(layout) = &self.last_layout {
            layout.validate()?;
        }
        Ok(())
    }
    pub(crate) fn startup_layout(&self) -> Option<(Option<Uuid>, Layout)> {
        match self.startup {
            Startup::Disabled => None,
            Startup::LastArrangement => self.last_layout.clone().map(|layout| (None, layout)),
            Startup::Named { workspace } => self
                .records
                .iter()
                .find(|r| r.id == workspace)
                .map(|r| (Some(r.id), r.layout.clone())),
        }
    }
    pub(crate) fn remove_references(&mut self, id: Uuid) {
        self.favorites.retain(|favorite| *favorite != id);
        if self.startup == (Startup::Named { workspace: id }) {
            self.startup = Startup::Disabled;
        }
    }
    pub(crate) fn decode(bytes: &[u8], account: Uuid, grid: &str) -> Result<Self, &'static str> {
        if bytes.len() > MAX_FILE {
            return Err("size");
        }
        let c: Self = serde_json::from_slice(bytes).map_err(|_| "schema")?;
        c.validate()?;
        if c.account != account || c.grid != grid {
            return Err("identity");
        }
        Ok(c)
    }
    pub(crate) fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|_| "schema")?;
        if bytes.len() > MAX_FILE {
            return Err("size");
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layout() -> Layout {
        Layout {
            geometry: [Some(Geometry {
                point: 7,
                offset: [-32768.0, 32768.0],
                size: [16384.0, 32.0],
            }); 4],
            viewport: Viewport {
                enabled: true,
                restrict_ui: true,
                left: 95.0,
                right: 0.0,
                top: 0.0,
                bottom: 95.0,
            },
            bars: Bars {
                classic: false,
                stretch: true,
                rights_text: false,
            },
        }
    }
    #[test]
    fn workspace_catalogue_maximum_escaped_names_are_bounded_and_round_trip() {
        let mut c = Catalogue::empty(Uuid::new_v4(), "f".repeat(64));
        for n in 0..LIMIT {
            c.records.push(Record {
                id: Uuid::new_v4(),
                name: format!("{n:02}{}", "\\\"".repeat(31)),
                layout: layout(),
            });
        }
        let b = c.encode().unwrap();
        assert!(b.len() < MAX_FILE);
        assert_eq!(Catalogue::decode(&b, c.account, &c.grid).unwrap(), c);
        c.records.push(c.records[0].clone());
        assert!(c.encode().is_err());
    }
    #[test]
    fn workspace_unknown_schema_fields_identity_duplicates_and_nonfinite_geometry_fail_closed() {
        let mut c = Catalogue::empty(Uuid::new_v4(), "a".repeat(64));
        c.records.push(Record {
            id: Uuid::new_v4(),
            name: "Desk".into(),
            layout: layout(),
        });
        let b = c.encode().unwrap();
        let mut v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        v["records"][0]["layout"]["secret"] = "ignored?".into();
        assert!(Catalogue::decode(&serde_json::to_vec(&v).unwrap(), c.account, &c.grid).is_err());
        v = serde_json::from_slice(&b).unwrap();
        v["version"] = 2.into();
        assert!(Catalogue::decode(&serde_json::to_vec(&v).unwrap(), c.account, &c.grid).is_err());
        assert!(Catalogue::decode(&b, Uuid::new_v4(), &c.grid).is_err());
        let mut duplicate = c.records[0].clone();
        duplicate.id = Uuid::new_v4();
        duplicate.name = "desk".into();
        c.records.push(duplicate);
        assert!(c.encode().is_err());
        c.records.pop();
        c.records[0].layout.geometry[0].as_mut().unwrap().offset[0] = f32::NAN;
        assert!(c.validate().is_err());
        assert!(!valid_name(" padded"));
        assert!(!valid_name("line\nbreak"));
        assert!(!valid_name(&"界".repeat(33)));
    }
    #[test]
    fn workspace_preview_settings_save_and_backup_keep_baseline_and_unrelated_changes() {
        let mut s = crate::settings::Settings::default();
        let mut baseline = layout();
        baseline.viewport = (&s.world_viewport).into();
        baseline.bars = Bars {
            classic: s.classic_bars,
            stretch: s.place_bar_stretch,
            rights_text: s.place_bar_rights_text,
        };
        let mut candidate = baseline.clone();
        candidate.viewport.enabled = true;
        candidate.viewport.left = 50.0;
        candidate.bars.classic = !baseline.bars.classic;
        s.workspace_held = Some(baseline.clone());
        candidate.settings(&mut s, false);
        s.show_balance = !s.show_balance;
        let balance = s.show_balance;
        let saved = s.to_saved();
        assert_eq!(saved.world_viewport, (&baseline.viewport).into());
        assert_eq!(saved.classic_bars, baseline.bars.classic);
        assert_eq!(saved.show_balance, balance);
        assert_eq!(s.world_viewport, (&candidate.viewport).into());
        let json = serde_json::to_value(saved).unwrap();
        assert!(json.get("workspace_held").is_none());
        baseline.settings(&mut s, true);
        assert_eq!(
            s.world_viewport,
            (&baseline.viewport).into(),
            "cancel restores relative world placement inside the fitted host"
        );
    }
    #[test]
    fn workspace_rename_accepts_its_own_unchanged_or_case_only_name_but_not_another_record() {
        let mut c = Catalogue::empty(Uuid::new_v4(), "a".repeat(64));
        let id = Uuid::new_v4();
        c.records.push(Record {
            id,
            name: "Desk".into(),
            layout: layout(),
        });
        c.records.push(Record {
            id: Uuid::new_v4(),
            name: "Other".into(),
            layout: layout(),
        });
        assert!(c.name_available("Desk", Some(id)));
        assert!(c.name_available("DESK", Some(id)));
        assert!(!c.name_available("other", Some(id)));
        assert!(!c.name_available("DESK", None));
        c.records[0].name = "DESK".into();
        assert!(c.validate().is_ok());
    }

    #[test]
    fn workspace_legacy_catalogue_defaults_to_no_startup_or_favorites() {
        let c = Catalogue::empty(Uuid::new_v4(), "a".repeat(64));
        let value = serde_json::to_value(&c).unwrap();
        assert!(value.get("startup").is_none());
        assert!(value.get("favorites").is_none());
        assert_eq!(
            Catalogue::decode(&serde_json::to_vec(&value).unwrap(), c.account, &c.grid).unwrap(),
            c
        );
        assert!(c.startup_layout().is_none());
    }

    #[test]
    fn workspace_favorite_and_startup_ids_survive_rename_and_are_retired_on_delete() {
        let mut c = Catalogue::empty(Uuid::new_v4(), "a".repeat(64));
        let id = Uuid::new_v4();
        c.records.push(Record {
            id,
            name: "Desk".into(),
            layout: layout(),
        });
        c.favorites.push(id);
        c.startup = Startup::Named { workspace: id };
        c.records[0].name = "Renamed".into();
        let loaded = Catalogue::decode(&c.encode().unwrap(), c.account, &c.grid).unwrap();
        assert_eq!(loaded.startup_layout(), Some((Some(id), layout())));
        assert_eq!(loaded.favorites, vec![id]);
        c.records.clear();
        assert!(c.validate().is_err());
        c.remove_references(id);
        assert!(c.validate().is_ok());
        assert_eq!(c.startup, Startup::Disabled);
        assert!(c.favorites.is_empty());
    }

    #[test]
    fn workspace_unknown_startup_and_duplicate_or_foreign_favorites_fail_closed() {
        let mut c = Catalogue::empty(Uuid::new_v4(), "a".repeat(64));
        let id = Uuid::new_v4();
        c.records.push(Record {
            id,
            name: "Desk".into(),
            layout: layout(),
        });
        c.favorites = vec![id, id];
        assert!(c.validate().is_err());
        c.favorites = vec![Uuid::new_v4()];
        assert!(c.validate().is_err());
        c.favorites.clear();
        let mut value = serde_json::to_value(&c).unwrap();
        value["startup"] = serde_json::json!({"mode":"named", "workspace":id, "future":true});
        assert!(Catalogue::decode(&serde_json::to_vec(&value).unwrap(), c.account, &c.grid).is_err());
        c.startup = Startup::LastArrangement;
        c.last_layout = Some(layout());
        c.last_layout.as_mut().unwrap().viewport.left = f32::NAN;
        assert!(c.validate().is_err());
    }
}
