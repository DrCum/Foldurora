//! Bounded local-image decoder/normalizer. No private store or native dialog.
use super::model::{Image, MAX_SIDE};
use std::{
    io::{Cursor, Read},
    path::Path,
};
use uuid::Uuid;

pub(crate) struct Decoded {
    pub id: Uuid,
    pub original: [u32; 2],
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Cancelled,
    Invalid,
    Unavailable,
    Full,
}
pub(crate) fn decode(image: &Image) -> Result<Decoded, Failure> {
    let mut reader = image::ImageReader::with_format(Cursor::new(image.png.bytes()), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(4 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader.decode().map_err(|_| Failure::Invalid)?.to_rgba8();
    if rgba.width() == 0 || rgba.height() == 0 || rgba.width() > MAX_SIDE || rgba.height() > MAX_SIDE {
        return Err(Failure::Invalid);
    }
    Ok(Decoded {
        id: image.id,
        original: image.original,
        size: [rgba.width(), rgba.height()],
        rgba: rgba.into_raw(),
    })
}
pub(crate) fn normalize(path: &Path) -> Result<Image, Failure> {
    let format = path
        .extension()
        .and_then(|s| s.to_str())
        .and_then(image::ImageFormat::from_extension)
        .filter(|f| {
            matches!(
                f,
                image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Bmp | image::ImageFormat::Tga
            )
        })
        .ok_or(Failure::Invalid)?;
    let file = std::fs::File::open(path).map_err(|_| Failure::Unavailable)?;
    let meta = file.metadata().map_err(|_| Failure::Unavailable)?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > 32 * 1024 * 1024 {
        return Err(Failure::Invalid);
    }
    let mut bytes = Vec::new();
    file.take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Unavailable)?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(Failure::Invalid);
    }
    normalize_bytes(&bytes, format)
}
fn normalize_bytes(bytes: &[u8], format: image::ImageFormat) -> Result<Image, Failure> {
    let original = image::ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|_| Failure::Invalid)?;
    if original.0 == 0
        || original.1 == 0
        || original.0 > 8192
        || original.1 > 8192
        || u64::from(original.0) * u64::from(original.1) > 32 * 1024 * 1024
    {
        return Err(Failure::Invalid);
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let normalized = reader
        .decode()
        .map_err(|_| Failure::Invalid)?
        .thumbnail(MAX_SIDE, MAX_SIDE)
        .to_rgba8();
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(normalized)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| Failure::Invalid)?;
    Ok(Image {
        id: Uuid::new_v4(),
        original: [original.0, original.1],
        png: super::model::Png::new(png.into_inner()).map_err(|_| Failure::Invalid)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_local_image_normalizer_preserves_original_shape_and_bounded_pixels_without_a_file_or_dialog() {
        let source = image::RgbaImage::from_pixel(512, 128, image::Rgba([50, 100, 200, 255]));
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(source)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let managed = normalize_bytes(bytes.get_ref(), image::ImageFormat::Png).unwrap();
        assert_eq!(managed.original, [512, 128]);
        assert!(!managed.id.is_nil());
        let ready = decode(&managed).unwrap();
        assert_eq!(ready.size, [256, 64]);
        assert_eq!(ready.original, [512, 128]);
        assert_eq!(ready.rgba.len(), 256 * 64 * 4);
        assert!(ready.rgba.as_chunks::<4>().0.iter().all(|p| *p == [50, 100, 200, 255]));
        assert!(normalize_bytes(b"not an image", image::ImageFormat::Png).is_err());
    }
    #[test]
    fn workspace_local_image_decoder_rejects_corrupt_png_even_when_the_structural_envelope_accepts_its_header() {
        let source = image::RgbaImage::from_pixel(4, 4, image::Rgba([50, 100, 200, 255]));
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(source)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let mut managed = normalize_bytes(bytes.get_ref(), image::ImageFormat::Png).unwrap();
        let mut corrupt = managed.png.bytes().to_vec();
        let at = corrupt.windows(4).position(|w| w == b"IDAT").unwrap() + 4;
        corrupt[at] ^= 1;
        managed.png = super::super::model::Png::new(corrupt).unwrap();
        assert!(decode(&managed).is_err(), "the native worker must not trust a valid header alone");
    }
}
