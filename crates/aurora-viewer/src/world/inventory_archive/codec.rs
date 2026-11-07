use super::{Error, MAX_BYTES, MAX_FLAGS, Source};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

const VERSION: u32 = 1;
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Document {
    pub(super) account: Uuid,
    pub(super) grid: [u8; 32],
    pub(super) publication: u64,
    pub(super) folders: Arc<BTreeSet<Uuid>>,
}
impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ArchiveDocument([redacted])")
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u32,
    account: Uuid,
    grid: [u8; 32],
    publication: u64,
    #[serde(deserialize_with = "folders")]
    folders: BTreeSet<Uuid>,
}
#[derive(Serialize)]
struct Writing<'a> {
    version: u32,
    account: Uuid,
    grid: [u8; 32],
    publication: u64,
    folders: &'a BTreeSet<Uuid>,
}

fn folders<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<BTreeSet<Uuid>, D::Error> {
    struct Folders;
    impl<'de> serde::de::Visitor<'de> for Folders {
        type Value = BTreeSet<Uuid>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("bounded unique nonnil folder UUIDs")
        }
        fn visit_seq<S: serde::de::SeqAccess<'de>>(self, mut seq: S) -> Result<Self::Value, S::Error> {
            let mut folders = BTreeSet::new();
            while let Some(id) = seq.next_element::<Uuid>()? {
                if folders.len() >= MAX_FLAGS || id.is_nil() || !folders.insert(id) {
                    return Err(serde::de::Error::custom("invalid archive folders"));
                }
            }
            Ok(folders)
        }
    }
    deserializer.deserialize_seq(Folders)
}
impl Document {
    pub(super) fn empty(source: Source) -> Self {
        Self {
            account: source.account,
            grid: source.grid.bytes(),
            publication: 0,
            folders: Arc::new(BTreeSet::new()),
        }
    }
    pub(super) fn matches(&self, source: Source) -> bool {
        self.account == source.account && self.grid == source.grid.bytes()
    }
    pub(super) fn valid_published(&self) -> bool {
        !self.account.is_nil() && self.publication != 0 && self.folders.len() <= MAX_FLAGS && !self.folders.iter().any(Uuid::is_nil)
    }
    pub(crate) fn publication(&self) -> u64 {
        self.publication
    }
    pub(crate) fn decode(bytes: &[u8], source: Source) -> Result<Self, Error> {
        if bytes.len() > MAX_BYTES {
            return Err(Error::Capacity);
        }
        let stored: Stored = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
        if stored.version != VERSION || stored.publication == 0 {
            return Err(Error::Invalid);
        }
        let document = Self {
            account: stored.account,
            grid: stored.grid,
            publication: stored.publication,
            folders: Arc::new(stored.folders),
        };
        if !document.matches(source) {
            return Err(Error::Scope);
        }
        Ok(document)
    }
    pub(crate) fn encode(&self) -> Result<Vec<u8>, Error> {
        if !self.valid_published() {
            return Err(Error::Invalid);
        }
        let bytes = serde_json::to_vec(&Writing {
            version: VERSION,
            account: self.account,
            grid: self.grid,
            publication: self.publication,
            folders: &self.folders,
        })
        .map_err(|_| Error::Invalid)?;
        if bytes.len() > MAX_BYTES {
            return Err(Error::Capacity);
        }
        Ok(bytes)
    }
}
