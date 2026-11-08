//! Closed account/grid data only; no session, asset, file path or playback state.
use aurora_session::GridKey;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

pub(crate) const MAX_BOARDS: usize = 16;
pub(crate) const MAX_TILES: usize = 1024;
pub(crate) const MAX_FILE: usize = 4 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Invalid,
    Unsupported,
    Identity,
    Limit,
    Name,
    Missing,
    Duplicate,
    Exhausted,
}
macro_rules! local_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "Uuid", into = "Uuid")]
        pub(crate) struct $name(Uuid);
        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }
        impl TryFrom<Uuid> for $name {
            type Error = &'static str;
            fn try_from(value: Uuid) -> Result<Self, Self::Error> {
                if value.is_nil() {
                    Err("invalid local identifier")
                } else {
                    Ok(Self(value))
                }
            }
        }
        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}
local_id!(BoardId);
local_id!(TileId);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Gesture,
    Sound,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reference {
    pub reference: Uuid,
    pub kind: Kind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Tile {
    pub id: TileId,
    pub item: Reference,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Board {
    pub id: BoardId,
    pub name: String,
    pub revision: u64,
    #[serde(deserialize_with = "tiles")]
    pub tiles: Vec<Tile>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Catalogue {
    version: u32,
    pub account: Uuid,
    pub grid: GridKey,
    pub revision: u64,
    #[serde(deserialize_with = "boards")]
    pub boards: Vec<Board>,
}
fn bounded<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>, const N: usize>(d: D) -> Result<Vec<T>, D::Error> {
    struct Visitor<T, const N: usize>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const N: usize> serde::de::Visitor<'de> for Visitor<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "at most {N} entries")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            if seq.size_hint().is_some_and(|n| n > N) {
                return Err(serde::de::Error::custom("too many entries"));
            }
            let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(N));
            while values.len() < N {
                match seq.next_element()? {
                    Some(v) => values.push(v),
                    None => return Ok(values),
                }
            }
            if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                return Err(serde::de::Error::custom("too many entries"));
            }
            Ok(values)
        }
    }
    d.deserialize_seq(Visitor::<T, N>(std::marker::PhantomData))
}
fn tiles<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Tile>, D::Error> {
    bounded::<D, Tile, MAX_TILES>(d)
}
fn boards<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Board>, D::Error> {
    bounded::<D, Board, MAX_BOARDS>(d)
}
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty() && name == name.trim() && name.len() <= 96 && name.chars().count() <= 64 && !name.chars().any(char::is_control)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Create { id: BoardId, name: String },
    Rename { board: BoardId, name: String },
    Delete { board: BoardId },
    Add { board: BoardId, tile: Tile },
    Remove { board: BoardId, tile: TileId },
}
impl Catalogue {
    pub fn empty(account: Uuid, grid: GridKey) -> Result<Self, Error> {
        if account.is_nil() {
            return Err(Error::Identity);
        }
        Ok(Self {
            version: 1,
            account,
            grid,
            revision: 0,
            boards: Vec::new(),
        })
    }
    pub fn board(&self, id: BoardId) -> Option<&Board> {
        self.boards.iter().find(|b| b.id == id)
    }
    pub fn tile(&self, board: BoardId, tile: TileId) -> Option<&Tile> {
        self.board(board)?.tiles.iter().find(|t| t.id == tile)
    }
    pub fn decode(bytes: &[u8], account: Uuid, grid: GridKey) -> Result<Self, Error> {
        if bytes.is_empty() || bytes.len() > MAX_FILE {
            return Err(Error::Limit);
        }
        let data: Self = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
        data.validate()?;
        if data.account != account || data.grid != grid {
            return Err(Error::Identity);
        }
        Ok(data)
    }
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| Error::Invalid)?;
        if bytes.len() > MAX_FILE {
            return Err(Error::Limit);
        }
        Ok(bytes)
    }
    fn validate(&self) -> Result<(), Error> {
        if self.version != 1 {
            return Err(Error::Unsupported);
        }
        if self.account.is_nil() {
            return Err(Error::Identity);
        }
        if self.boards.len() > MAX_BOARDS {
            return Err(Error::Limit);
        }
        let (mut ids, mut names, mut tile_ids) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        for board in &self.boards {
            if !valid_name(&board.name) {
                return Err(Error::Name);
            }
            if !ids.insert(board.id) || !names.insert(board.name.to_lowercase()) {
                return Err(Error::Duplicate);
            }
            if board.revision == 0 {
                return Err(Error::Invalid);
            }
            if board.tiles.len() > MAX_TILES {
                return Err(Error::Limit);
            }
            let mut references = BTreeSet::new();
            for tile in &board.tiles {
                if tile.item.reference.is_nil() {
                    return Err(Error::Invalid);
                }
                if !tile_ids.insert(tile.id) || !references.insert(tile.item.reference) {
                    return Err(Error::Duplicate);
                }
            }
        }
        Ok(())
    }
    /// Called by the bounded store worker, not on every UI pass. The unchanged
    /// original remains available for byte CAS and exact operation correlation.
    pub fn apply(&self, change: Change) -> Result<Self, Error> {
        let revision = self.revision.checked_add(1).ok_or(Error::Exhausted)?;
        let mut next = self.clone();
        match change {
            Change::Create { id, name } => {
                if next.boards.len() >= MAX_BOARDS {
                    return Err(Error::Limit);
                }
                next.boards.push(Board {
                    id,
                    name,
                    revision: 1,
                    tiles: Vec::new(),
                });
            }
            Change::Delete { board } => {
                let i = next.boards.iter().position(|b| b.id == board).ok_or(Error::Missing)?;
                next.boards.remove(i);
            }
            Change::Rename { board, name } => {
                let b = next.boards.iter_mut().find(|b| b.id == board).ok_or(Error::Missing)?;
                b.revision = b.revision.checked_add(1).ok_or(Error::Exhausted)?;
                b.name = name;
            }
            Change::Add { board, tile } => {
                let b = next.boards.iter_mut().find(|b| b.id == board).ok_or(Error::Missing)?;
                if b.tiles.len() >= MAX_TILES {
                    return Err(Error::Limit);
                }
                b.revision = b.revision.checked_add(1).ok_or(Error::Exhausted)?;
                b.tiles.push(tile);
            }
            Change::Remove { board, tile } => {
                let b = next.boards.iter_mut().find(|b| b.id == board).ok_or(Error::Missing)?;
                let i = b.tiles.iter().position(|t| t.id == tile).ok_or(Error::Missing)?;
                b.revision = b.revision.checked_add(1).ok_or(Error::Exhausted)?;
                b.tiles.remove(i);
            }
        }
        next.revision = revision;
        next.validate()?;
        Ok(next)
    }
}
#[cfg(test)]
mod tests;
