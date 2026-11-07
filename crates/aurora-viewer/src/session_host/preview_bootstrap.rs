//! Optional private bootstrap DTO only. No endpoint, thread or image producer
//! starts here; absent preview preserves the previous worker launch schema.
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct PreviewSecret(Arc<[u8; 32]>);
impl std::fmt::Debug for PreviewSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreviewSecret([redacted])")
    }
}
impl PreviewSecret {
    fn from_bytes(bytes: [u8; 32]) -> Result<Self, &'static str> {
        if bytes.iter().all(|b| *b == 0) {
            return Err("invalid private preview key");
        }
        Ok(Self(Arc::new(bytes)))
    }
    #[cfg(test)]
    pub(super) fn synthetic(bytes: [u8; 32]) -> Result<Self, &'static str> {
        Self::from_bytes(bytes)
    }
    pub fn random() -> Result<Self, &'static str> {
        use ring::rand::SecureRandom;
        let mut bytes = [0; 32];
        ring::rand::SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| "preview randomness unavailable")?;
        Self::from_bytes(bytes)
    }
    pub(super) fn context(&self) -> ring::hmac::Context {
        ring::hmac::Context::with_key(&ring::hmac::Key::new(ring::hmac::HMAC_SHA256, self.0.as_ref()))
    }
    pub(super) fn sign(&self, input: &[u8]) -> ring::hmac::Tag {
        ring::hmac::sign(&ring::hmac::Key::new(ring::hmac::HMAC_SHA256, self.0.as_ref()), input)
    }
    pub(super) fn verifies(&self, input: &[u8], tag: &[u8]) -> bool {
        ring::hmac::verify(&ring::hmac::Key::new(ring::hmac::HMAC_SHA256, self.0.as_ref()), input, tag).is_ok()
    }
}
impl Serialize for PreviewSecret {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut text = [0u8; 64];
        for (byte, pair) in self.0.iter().zip(text.as_chunks_mut::<2>().0) {
            pair[0] = HEX[usize::from(*byte >> 4)];
            pair[1] = HEX[usize::from(*byte & 15)];
        }
        let text = std::str::from_utf8(&text).map_err(|_| serde::ser::Error::custom("invalid private preview key"))?;
        serializer.serialize_str(text)
    }
}
impl<'de> Deserialize<'de> for PreviewSecret {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeyVisitor;
        impl serde::de::Visitor<'_> for KeyVisitor {
            type Value = PreviewSecret;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a private preview key")
            }
            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Self::Value, E> {
                if text.len() != 64 {
                    return Err(E::custom("invalid private preview key"));
                }
                let mut bytes = [0; 32];
                for (pair, out) in text.as_bytes().as_chunks::<2>().0.iter().zip(&mut bytes) {
                    let digit = |b| match b {
                        b'0'..=b'9' => Some(b - b'0'),
                        b'a'..=b'f' => Some(b - b'a' + 10),
                        _ => None,
                    };
                    let (Some(a), Some(b)) = (digit(pair[0]), digit(pair[1])) else {
                        return Err(E::custom("invalid private preview key"));
                    };
                    *out = (a << 4) | b;
                }
                PreviewSecret::from_bytes(bytes).map_err(E::custom)
            }
        }
        deserializer.deserialize_str(KeyVisitor)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewBootstrap {
    pub port: u16,
    pub key: PreviewSecret,
}
