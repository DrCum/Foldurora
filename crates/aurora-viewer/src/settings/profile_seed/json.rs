//! A bounded JSON value reader that rejects duplicate members before any typed
//! projection. Unknown unselected values remain values, never settings defaults.
use super::{Error, MAX_DEPTH, MAX_FILE, MAX_NODES};
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt;

struct Budget(usize);
struct ReadValue<'a> {
    budget: &'a mut Budget,
    depth: usize,
}
impl<'de> DeserializeSeed<'de> for ReadValue<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        if self.depth >= MAX_DEPTH || self.budget.0 == 0 {
            return Err(D::Error::custom("profile settings bound"));
        }
        self.budget.0 -= 1;
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for ReadValue<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded profile settings JSON")
    }
    fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }
    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("invalid number"))
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }
    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(ReadValue {
            budget: &mut *self.budget,
            depth: self.depth + 1,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(A::Error::custom("duplicate profile settings member"));
            }
            let value = map.next_value_seed(ReadValue {
                budget: &mut *self.budget,
                depth: self.depth + 1,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}
pub(super) fn object(bytes: &[u8]) -> Result<Map<String, Value>, Error> {
    if bytes.len() > MAX_FILE {
        return Err(Error::Bound);
    }
    let mut reader = serde_json::Deserializer::from_slice(bytes);
    let value = ReadValue {
        budget: &mut Budget(MAX_NODES),
        depth: 0,
    }
    .deserialize(&mut reader)
    .map_err(|_| Error::Json)?;
    reader.end().map_err(|_| Error::Json)?;
    let Value::Object(map) = value else { return Err(Error::Json) };
    if map.get("settings_version").and_then(Value::as_u64) != Some(u64::from(super::VERSION)) {
        return Err(Error::Version);
    }
    Ok(map)
}
