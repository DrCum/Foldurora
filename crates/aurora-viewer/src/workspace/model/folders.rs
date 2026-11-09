//! Private expanded-folder references, not names, items or fetch instructions.
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub(crate) const MAX_EXPANDED: usize = 256;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<Uuid>", into = "Vec<Uuid>")]
pub(crate) struct ExpandedFolders(Vec<Uuid>);
impl TryFrom<Vec<Uuid>> for ExpandedFolders {
    type Error = &'static str;
    fn try_from(mut folders: Vec<Uuid>) -> Result<Self, Self::Error> {
        if folders.len() > MAX_EXPANDED || folders.iter().any(Uuid::is_nil) {
            return Err("expanded folders");
        }
        folders.sort_unstable();
        if folders.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("duplicate expanded folder");
        }
        Ok(Self(folders))
    }
}
impl From<ExpandedFolders> for Vec<Uuid> {
    fn from(folders: ExpandedFolders) -> Self {
        folders.0
    }
}
impl ExpandedFolders {
    pub(crate) fn refs(&self) -> &[Uuid] {
        &self.0
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_expanded_folder_references_are_bounded_unique_and_canonical() {
        assert!(ExpandedFolders::try_from(vec![Uuid::nil()]).is_err());
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        assert!(ExpandedFolders::try_from(vec![a, a]).is_err());
        assert!(ExpandedFolders::try_from((1..=257).map(Uuid::from_u128).collect::<Vec<_>>()).is_err());
        let folders = ExpandedFolders::try_from(vec![b, a]).unwrap();
        assert_eq!(folders.refs(), &[a, b]);
        assert_eq!(
            serde_json::from_slice::<ExpandedFolders>(&serde_json::to_vec(&folders).unwrap()).unwrap(),
            folders
        );
    }
    #[test]
    fn workspace_expanded_folder_schema_accepts_only_uuid_references_and_explicit_empty() {
        for text in [r#"["private/path"]"#, r#"[{"item":"private"}]"#, r#"{"folders":[]}"#, r#"[null]"#] {
            assert!(serde_json::from_str::<ExpandedFolders>(text).is_err());
        }
        assert_eq!(serde_json::from_str::<ExpandedFolders>("[]").unwrap(), ExpandedFolders::default());
    }
}
