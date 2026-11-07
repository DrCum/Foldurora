//! Optional fixed-slot quick action VALUE/CODEC. Actual private persistence
//! requires the separately reviewed persistent IO owner/retirement/CAS lane.
//! No filesystem, thread, Registry mutation or native permission exists here.
use super::slots::SlotId;
use aurora_session::MAX_WORKERS;
use serde::{Deserialize, Serialize};

pub(super) const MAX_BYTES: usize = 512;
const VERSION: u16 = 1;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CodecError {
    TooLarge,
    Invalid,
    Version,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Preferences {
    order: Vec<SlotId>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u16,
    order: Vec<SlotId>,
}
impl Preferences {
    pub fn new(slots: impl IntoIterator<Item = SlotId>) -> Result<Self, CodecError> {
        let mut order = Vec::with_capacity(MAX_WORKERS);
        for slot in slots {
            if order.len() == MAX_WORKERS || order.contains(&slot) {
                return Err(CodecError::Invalid);
            }
            order.push(slot);
        }
        Ok(Self { order })
    }
    pub fn order(&self) -> &[SlotId] {
        &self.order
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() > MAX_BYTES {
            return Err(CodecError::TooLarge);
        }
        let stored: Stored = serde_json::from_slice(bytes).map_err(|_| CodecError::Invalid)?;
        if stored.version != VERSION {
            return Err(CodecError::Version);
        }
        Self::new(stored.order)
    }
    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        let bytes = serde_json::to_vec(&Stored {
            version: VERSION,
            order: self.order.clone(),
        })
        .map_err(|_| CodecError::Invalid)?;
        if bytes.len() > MAX_BYTES {
            return Err(CodecError::TooLarge);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_quick_preferences_empty_default_and_exact_order_roundtrip_without_registry_fields() {
        assert!(Preferences::default().order().is_empty());
        let mut slots: Vec<_> = SlotId::all().collect();
        slots.reverse();
        let prefs = Preferences::new(slots.clone()).unwrap();
        let bytes = prefs.encode().unwrap();
        assert!(bytes.len() <= MAX_BYTES);
        assert_eq!(Preferences::decode(&bytes).unwrap(), prefs);
        assert_eq!(prefs.order(), slots.as_slice());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    fn fixed_quick_preferences_reject_duplicate_unknown_or_invalid_slots_and_versions() {
        for bytes in [
            br#"{"version":1,"order":[0,0]}"#.as_slice(),
            br#"{"version":1,"order":[5]}"#,
            br#"{"version":1,"order":[-1]}"#,
            br#"{"version":1,"order":[0],"path":"ignored"}"#,
            br#"{"version":1,"order":null}"#,
            br#"{"order":[0]}"#,
        ] {
            assert!(Preferences::decode(bytes).is_err());
        }
        assert_eq!(Preferences::decode(br#"{"version":2,"order":[]}"#), Err(CodecError::Version));
        let first = SlotId::try_from(0).unwrap();
        assert_eq!(Preferences::new([first, first]), Err(CodecError::Invalid));
        assert_eq!(Preferences::new(SlotId::all().chain([first])), Err(CodecError::Invalid));
    }
    #[test]
    fn fixed_quick_preferences_byte_cap_applies_before_parse_and_does_not_truncate_order() {
        let mut valid = Preferences::new(SlotId::all()).unwrap().encode().unwrap();
        valid.resize(MAX_BYTES, b' ');
        assert_eq!(Preferences::decode(&valid).unwrap().order(), SlotId::all().collect::<Vec<_>>());
        valid.push(b' ');
        assert_eq!(Preferences::decode(&valid), Err(CodecError::TooLarge));
        assert!(Preferences::decode(b"\xff").is_err());
    }
}
