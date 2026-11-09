//! The one native standard additional Inventory window. No dynamic UI IDs.
use super::{ExpandedFolders, Geometry};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InventoryWindow {
    pub visible: bool,
    #[serde(default)]
    pub minimized: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<Geometry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<ExpandedFolders>,
}
impl InventoryWindow {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.geometry.is_some_and(|g| !g.valid())
            || self.folder.is_some_and(|id| id.is_nil())
            || (!self.visible && (self.minimized || self.folder.is_some() || self.expanded.is_some()))
            || (self.minimized && (self.folder.is_some() || self.expanded.is_some()))
        {
            return Err("additional Inventory window");
        }
        Ok(())
    }
    pub(crate) fn has_folders(&self) -> bool {
        self.folder.is_some() || self.expanded.is_some()
    }
    pub(crate) fn baseline_for(&self, target: &Self) -> Self {
        let mut baseline = self.clone();
        if target.geometry.is_none() {
            baseline.geometry = None;
        }
        if target.folder.is_none() {
            baseline.folder = None;
        }
        if target.expanded.is_none() {
            baseline.expanded = None;
        }
        baseline
    }
    pub(crate) fn matches(&self, live: &Self) -> bool {
        self.visible == live.visible
            && self.minimized == live.minimized
            && self.geometry.zip(live.geometry).is_none_or(|(a, b)| a == b)
            && self.folder.zip(live.folder).is_none_or(|(a, b)| a == b)
            && self.expanded.as_ref().zip(live.expanded.as_ref()).is_none_or(|(a, b)| a == b)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn closed() -> InventoryWindow {
        InventoryWindow {
            visible: false,
            minimized: false,
            geometry: None,
            folder: None,
            expanded: None,
        }
    }
    #[test]
    fn workspace_additional_inventory_schema_refuses_hidden_navigation_and_invalid_geometry() {
        let original = closed();
        assert!(original.validate().is_ok());
        let mut value = original.clone();
        value.folder = Some(Uuid::new_v4());
        assert!(value.validate().is_err());
        value.visible = true;
        assert!(value.validate().is_ok());
        value.minimized = true;
        assert!(value.validate().is_err());
        value.folder = None;
        assert!(value.validate().is_ok());
        value.geometry = Some(Geometry {
            point: 8,
            offset: [0.0; 2],
            size: [320.0, 240.0],
        });
        assert!(value.validate().is_err());
        value.geometry.as_mut().unwrap().point = 0;
        value.geometry.as_mut().unwrap().offset[0] = f32::NAN;
        assert!(value.validate().is_err());
        assert!(serde_json::from_str::<InventoryWindow>(r#"{"visible":false,"owner":"arbitrary"}"#).is_err());
    }
    #[test]
    fn workspace_additional_inventory_scope_preserves_absence_and_original_context() {
        let mut current = closed();
        current.visible = true;
        current.folder = Some(Uuid::new_v4());
        current.expanded = Some(ExpandedFolders::default());
        let target = closed();
        let baseline = current.baseline_for(&target);
        assert!(baseline.visible);
        assert!(baseline.folder.is_none() && baseline.expanded.is_none());
        assert!(!target.matches(&current));
        assert!(current.matches(&current));
        let bytes = serde_json::to_vec(&current).unwrap();
        assert_eq!(serde_json::from_slice::<InventoryWindow>(&bytes).unwrap(), current);
    }
}
