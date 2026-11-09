//! Private inventory item references for the Folderstorm additive HUD context.
//! These are not asset IDs, names, paths or a permission to attach anything.
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub(crate) const MAX_HUDS: usize = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HudRef {
    pub item: Uuid,
    pub point: u8,
}
impl HudRef {
    pub(crate) fn valid(self) -> bool {
        // The reference schema's eight HUD points. Native avatar-library HUD
        // membership and all item/point rights must still be checked live.
        !self.item.is_nil() && (31..=38).contains(&self.point)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<HudRef>", into = "Vec<HudRef>")]
pub(crate) struct HudSet(Vec<HudRef>);
impl TryFrom<Vec<HudRef>> for HudSet {
    type Error = &'static str;
    fn try_from(refs: Vec<HudRef>) -> Result<Self, Self::Error> {
        let mut items = std::collections::BTreeSet::new();
        if refs.len() > MAX_HUDS || refs.iter().any(|r| !r.valid() || !items.insert(r.item)) {
            return Err("HUD references");
        }
        Ok(Self(refs))
    }
}
impl From<HudSet> for Vec<HudRef> {
    fn from(set: HudSet) -> Self {
        set.0
    }
}
impl HudSet {
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn refs(&self) -> &[HudRef] {
        &self.0
    }
    pub(crate) fn contains(&self, reference: HudRef) -> bool {
        self.0.contains(&reference)
    }
    pub(crate) fn is_subset_of(&self, current: &Self) -> bool {
        self.0.iter().all(|r| current.contains(*r))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn reference(item: u128, point: u8) -> HudRef {
        HudRef {
            item: Uuid::from_u128(item),
            point,
        }
    }
    #[test]
    fn workspace_hud_references_are_bounded_unique_original_items_at_hud_points() {
        for invalid in [
            vec![reference(0, 31)],
            vec![reference(1, 0)],
            vec![reference(1, 30)],
            vec![reference(1, 39)],
            vec![reference(1, 31), reference(1, 32)],
        ] {
            assert!(HudSet::try_from(invalid).is_err());
        }
        assert!(HudSet::try_from((1..=65).map(|item| reference(item, 31)).collect::<Vec<_>>()).is_err());
        let set = HudSet::try_from(vec![reference(1, 31), reference(2, 38)]).unwrap();
        assert_eq!(set.refs().len(), 2);
        let bytes = serde_json::to_vec(&set).unwrap();
        assert_eq!(serde_json::from_slice::<HudSet>(&bytes).unwrap(), set);
    }
    #[test]
    fn workspace_hud_reference_schema_refuses_asset_payloads_unknown_fields_and_future_points() {
        let item = Uuid::from_u128(1).to_string();
        let text = format!(r#"[{{"item":"{item}","point":31,"asset":"never copied"}}]"#);
        assert!(serde_json::from_str::<HudSet>(&text).is_err());
        let text = format!(r#"[{{"item":"{item}","point":39}}]"#);
        assert!(serde_json::from_str::<HudSet>(&text).is_err());
    }
    #[test]
    fn workspace_hud_matching_keeps_extra_worn_items_and_never_changes_points() {
        let requested = HudSet::try_from(vec![reference(1, 31)]).unwrap();
        let current = HudSet::try_from(vec![reference(1, 31), reference(2, 38)]).unwrap();
        assert!(requested.is_subset_of(&current));
        assert!(!current.is_subset_of(&requested));
        assert!(!requested.is_subset_of(&HudSet::try_from(vec![reference(1, 32)]).unwrap()));
        assert!(HudSet::default().is_subset_of(&current));
    }
}
