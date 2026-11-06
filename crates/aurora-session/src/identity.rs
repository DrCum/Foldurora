use crate::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

macro_rules! opaque_id {
    ($name:ident, $bytes:expr) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; $bytes]);

        impl $name {
            pub fn from_bytes(bytes: [u8; $bytes]) -> Result<Self, Error> {
                if bytes.iter().any(|b| *b != 0) {
                    Ok(Self(bytes))
                } else {
                    Err(Error::Identity)
                }
            }

            pub fn parse_hex(text: &str) -> Result<Self, Error> {
                if text.len() != $bytes * 2 {
                    return Err(Error::Identity);
                }
                let mut bytes = [0; $bytes];
                for (pair, output) in text.as_bytes().chunks_exact(2).zip(bytes.iter_mut()) {
                    let digit = |b| match b {
                        b'0'..=b'9' => Ok(b - b'0'),
                        b'a'..=b'f' => Ok(b - b'a' + 10),
                        _ => Err(Error::Identity),
                    };
                    *output = digit(pair[0])? * 16 + digit(pair[1])?;
                }
                Self::from_bytes(bytes)
            }

            pub fn as_bytes(&self) -> &[u8; $bytes] {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                for byte in self.0 {
                    write!(f, "{byte:02x}")?;
                }
                Ok(())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                Self::parse_hex(&text).map_err(serde::de::Error::custom)
            }
        }
    };
}

opaque_id!(InstallationId, 32);
opaque_id!(HostId, 16);
// Fresh, unpredictable per-process launch identity. Never reuse it for a restart.
opaque_id!(WorkerId, 16);
// An opaque local login generation identifier, never a simulator session token.
opaque_id!(SessionId, 16);
opaque_id!(AccountKey, 32);
opaque_id!(GridKey, 32);
opaque_id!(ConversationId, 16);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerIdentity {
    pub installation: InstallationId,
    pub host: HostId,
    pub worker: WorkerId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionIdentity {
    pub session: SessionId,
    pub account: AccountKey,
    pub grid: GridKey,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub worker: WorkerIdentity,
    pub session: Option<SessionIdentity>,
}

impl Binding {
    pub fn validate(&self) -> Result<(), Error> {
        if self.session.is_some_and(|s| s.generation == 0) {
            Err(Error::Identity)
        } else {
            Ok(())
        }
    }
}
