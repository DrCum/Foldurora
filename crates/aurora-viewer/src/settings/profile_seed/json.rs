//! Lossless unselected values with bounded recursive validation. RawValue
//! retains numeric lexemes instead of going through serde_json's f64 fallback.
//! Every raw subtree is still checked for decoded duplicates, depth and nodes.
use super::{Error, MAX_DEPTH, MAX_FILE, MAX_NODES};
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer as _};
use serde_json::value::RawValue;
use std::collections::{BTreeMap, HashSet};
use std::fmt;

pub(super) type Document = BTreeMap<String, Box<RawValue>>;
struct Budget(usize);
fn spend(budget: &mut Budget, depth: usize) -> Result<(), Error> {
    if depth >= MAX_DEPTH || budget.0 == 0 {
        return Err(Error::Json);
    }
    budget.0 -= 1;
    Ok(())
}
struct Check<'a> {
    budget: &'a mut Budget,
    depth: usize,
}
impl<'de> DeserializeSeed<'de> for Check<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        // Refuse before capturing another subtree once admission is exhausted.
        if self.depth >= MAX_DEPTH || self.budget.0 == 0 {
            return Err(D::Error::custom("profile settings bound"));
        }
        let raw = <&RawValue>::deserialize(deserializer)?;
        check(raw, self.budget, self.depth).map_err(|_| D::Error::custom("invalid bounded profile settings"))
    }
}
struct Container<'a> {
    budget: &'a mut Budget,
    depth: usize,
}
impl<'de> Visitor<'de> for Container<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded profile settings container")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let mut keys = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key) {
                return Err(A::Error::custom("duplicate profile settings member"));
            }
            map.next_value_seed(Check {
                budget: &mut *self.budget,
                depth: self.depth + 1,
            })?;
        }
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while seq
            .next_element_seed(Check {
                budget: &mut *self.budget,
                depth: self.depth + 1,
            })?
            .is_some()
        {}
        Ok(())
    }
}
fn check(raw: &RawValue, budget: &mut Budget, depth: usize) -> Result<(), Error> {
    spend(budget, depth)?;
    let mut reader = serde_json::Deserializer::from_str(raw.get());
    let visitor = Container { budget, depth };
    match raw.get().trim_start().as_bytes().first() {
        Some(b'{') => reader.deserialize_map(visitor).map_err(|_| Error::Json)?,
        Some(b'[') => reader.deserialize_seq(visitor).map_err(|_| Error::Json)?,
        // RawValue already validates scalar syntax. Do not deserialize numbers
        // as f64 merely to validate them: that would round unknown integers,
        // precision or exponents. Scalars are still counted above.
        _ => return Ok(()),
    }
    reader.end().map_err(|_| Error::Json)
}
struct Root<'a>(&'a mut Budget);
impl<'de> Visitor<'de> for Root<'_> {
    type Value = Document;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("profile settings document")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Document, A::Error> {
        let mut values = Document::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(A::Error::custom("duplicate profile settings member"));
            }
            if self.0.0 == 0 {
                return Err(A::Error::custom("profile settings bound"));
            }
            let raw = map.next_value::<Box<RawValue>>()?;
            check(&raw, self.0, 1).map_err(|_| A::Error::custom("invalid bounded profile settings"))?;
            values.insert(key, raw);
        }
        Ok(values)
    }
}
// RawValue first scans a complete subtree. Bound lexical nesting BEFORE that
// scan, so an over-deep rejected input cannot multiply its internal skip-stack
// allocations across our recursive checker. Syntax/decoded keys remain Serde's
// responsibility; quoted/escaped delimiters do not contribute to nesting.
fn nesting(bytes: &[u8]) -> Result<(), Error> {
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for &byte in bytes {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > MAX_DEPTH {
                        return Err(Error::Json);
                    }
                }
                b'}' | b']' => {
                    if depth == 0 {
                        return Err(Error::Json);
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
    }
    if quoted || escaped || depth != 0 {
        Err(Error::Json)
    } else {
        Ok(())
    }
}
pub(super) fn object(bytes: &[u8]) -> Result<Document, Error> {
    if bytes.len() > MAX_FILE {
        return Err(Error::Bound);
    }
    nesting(bytes)?;
    let mut budget = Budget(MAX_NODES);
    spend(&mut budget, 0)?;
    let mut reader = serde_json::Deserializer::from_slice(bytes);
    let map = reader.deserialize_map(Root(&mut budget)).map_err(|_| Error::Json)?;
    reader.end().map_err(|_| Error::Json)?;
    let version = map.get("settings_version").and_then(|v| serde_json::from_str::<u32>(v.get()).ok());
    if version != Some(super::VERSION) {
        return Err(Error::Version);
    }
    Ok(map)
}
pub(super) fn members(raw: &RawValue) -> Result<BTreeMap<String, &RawValue>, Error> {
    serde_json::from_str(raw.get()).map_err(|_| Error::Selected)
}
