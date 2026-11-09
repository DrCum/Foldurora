//! Strict structural envelope. Native workers also decode each PNG with bounded
//! image limits before accepting a library; a PNG header alone is not validation.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(crate) const MAX_FILE: usize = 32 * 1024 * 1024;
pub(crate) const MAX_IMAGES: usize = 128;
pub(crate) const MAX_PNG: usize = 512 * 1024;
pub(crate) const MAX_SIDE: u32 = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct Png(Vec<u8>);
impl Png {
    pub(crate) fn new(bytes: Vec<u8>) -> Result<Self, &'static str> {
        if bytes.len() < 45
            || bytes.len() > MAX_PNG
            || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
            || bytes[8..12] != 13u32.to_be_bytes()
            || &bytes[12..16] != b"IHDR"
        {
            return Err("managed PNG envelope");
        }
        let width = u32::from_be_bytes(bytes[16..20].try_into().expect("checked length"));
        let height = u32::from_be_bytes(bytes[20..24].try_into().expect("checked length"));
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err("managed PNG dimensions");
        }
        Ok(Self(bytes))
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.0
    }
}
impl TryFrom<String> for Png {
    type Error = &'static str;
    fn try_from(encoded: String) -> Result<Self, Self::Error> {
        if encoded.len() > MAX_PNG.div_ceil(3) * 4 {
            return Err("managed PNG encoding size");
        }
        Self::new(STANDARD.decode(encoded).map_err(|_| "managed PNG encoding")?)
    }
}
impl From<Png> for String {
    fn from(png: Png) -> Self {
        STANDARD.encode(png.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Image {
    pub id: Uuid,
    pub original: [u32; 2],
    pub png: Png,
}
impl Image {
    fn valid(&self) -> bool {
        !self.id.is_nil()
            && self.original.into_iter().all(|n| n > 0 && n <= 8192)
            && u64::from(self.original[0]) * u64::from(self.original[1]) <= 32 * 1024 * 1024
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Library {
    pub version: u8,
    pub account: Uuid,
    pub grid: String,
    pub images: Vec<Image>,
}
impl Library {
    pub(crate) fn empty(account: Uuid, grid: String) -> Self {
        Self {
            version: 1,
            account,
            grid,
            images: Vec::new(),
        }
    }
    fn valid(&self, account: Uuid, grid: &str) -> bool {
        self.version == 1
            && !account.is_nil()
            && self.account == account
            && self.grid == grid
            && grid.len() == 64
            && grid.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            && self.images.len() <= MAX_IMAGES
            && self.images.iter().all(Image::valid)
            && self.images.windows(2).all(|pair| pair[0].id < pair[1].id)
    }
    pub(crate) fn decode(bytes: &[u8], account: Uuid, grid: &str) -> Result<Self, &'static str> {
        if bytes.len() > MAX_FILE {
            return Err("managed image library size");
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| "managed image library schema")?;
        if !value.valid(account, grid) {
            return Err("managed image library identity or entries");
        }
        Ok(value)
    }
    pub(crate) fn encode(&self) -> Result<Vec<u8>, &'static str> {
        if !self.valid(self.account, &self.grid) {
            return Err("managed image library identity or entries");
        }
        let bytes = serde_json::to_vec(self).map_err(|_| "managed image library encoding")?;
        if bytes.len() > MAX_FILE {
            return Err("managed image library size");
        }
        Ok(bytes)
    }
    pub(crate) fn insert(&mut self, image: Image) -> Result<(), &'static str> {
        if self.images.len() >= MAX_IMAGES || !image.valid() || self.images.iter().any(|i| i.id == image.id) {
            return Err("managed image library full or immutable ID reused");
        }
        // Validate aggregate bytes before changing the caller's original library.
        let mut next = self.clone();
        next.images.push(image);
        next.images.sort_by_key(|i| i.id);
        next.encode()?;
        *self = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Image {
        // A real one-pixel PNG. Decoder/CRC tests belong to the worker, not this
        // envelope-only test; production must never skip the bounded decoder.
        let bytes = STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNoaGj4DwAFhAKAjM1mJgAAAABJRU5ErkJggg==")
            .unwrap();
        Image {
            id: Uuid::from_u128(10),
            original: [1, 1],
            png: Png::new(bytes).unwrap(),
        }
    }
    #[test]
    fn workspace_local_image_envelope_refuses_paths_bad_base64_and_oversized_pixel_headers() {
        for text in ["file:///private", "not base64", ""] {
            assert!(Png::try_from(text.to_owned()).is_err());
        }
        let mut bytes = sample().png.0;
        bytes[16..20].copy_from_slice(&257u32.to_be_bytes());
        assert!(Png::new(bytes).is_err());
        assert!(Png::new(vec![0; MAX_PNG + 1]).is_err());
        let mut image = serde_json::to_value(sample()).unwrap();
        image["path"] = "private".into();
        assert!(serde_json::from_value::<Image>(image).is_err());
    }
    #[test]
    fn workspace_local_image_library_identity_immutability_and_failed_insert_preserve_original() {
        let account = Uuid::new_v4();
        let grid = "a".repeat(64);
        let mut lib = Library::empty(account, grid.clone());
        lib.insert(sample()).unwrap();
        let original = lib.encode().unwrap();
        assert_eq!(Library::decode(&original, account, &grid).unwrap(), lib);
        assert!(Library::decode(&original, Uuid::new_v4(), &grid).is_err());
        assert!(Library::decode(&original, account, &"b".repeat(64)).is_err());
        assert!(lib.insert(sample()).is_err());
        assert_eq!(lib.encode().unwrap(), original);
        let mut invalid = sample();
        invalid.id = Uuid::from_u128(11);
        invalid.original = [8192, 8192];
        assert!(lib.insert(invalid).is_err());
        assert_eq!(lib.encode().unwrap(), original);
        let mut wire = serde_json::to_value(lib).unwrap();
        wire["filename"] = "private".into();
        assert!(Library::decode(&serde_json::to_vec(&wire).unwrap(), account, &grid).is_err());
    }
}
