//! Private texture references and bounded display choices, never file paths.
use super::Arrangement;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ImageMode {
    #[default]
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
    Span,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Reference", into = "Reference")]
pub(crate) struct TextureRef {
    pub item: Uuid,
    pub asset: Uuid,
    pub grid: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    item: Uuid,
    asset: Uuid,
    grid: String,
}
impl TryFrom<Reference> for TextureRef {
    type Error = &'static str;
    fn try_from(r: Reference) -> Result<Self, Self::Error> {
        if r.item.is_nil()
            || r.asset.is_nil()
            || r.grid.len() != 64
            || !r.grid.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err("workspace texture reference");
        }
        Ok(Self {
            item: r.item,
            asset: r.asset,
            grid: r.grid,
        })
    }
}
impl From<TextureRef> for Reference {
    fn from(r: TextureRef) -> Self {
        Self {
            item: r.item,
            asset: r.asset,
            grid: r.grid,
        }
    }
}
impl TextureRef {
    pub(crate) fn new(item: Uuid, asset: Uuid, grid: String) -> Result<Self, &'static str> {
        Reference { item, asset, grid }.try_into()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Picture {
    pub texture: TextureRef,
    pub mode: ImageMode,
    pub brightness: u8,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "ManagedWire", into = "ManagedWire")]
pub(crate) struct ManagedRef {
    pub image: Uuid,
    pub account: Uuid,
    pub grid: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedWire {
    image: Uuid,
    account: Uuid,
    grid: String,
}
impl TryFrom<ManagedWire> for ManagedRef {
    type Error = &'static str;
    fn try_from(w: ManagedWire) -> Result<Self, Self::Error> {
        if w.image.is_nil()
            || w.account.is_nil()
            || w.grid.len() != 64
            || !w.grid.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err("managed image reference");
        }
        Ok(Self {
            image: w.image,
            account: w.account,
            grid: w.grid,
        })
    }
}
impl From<ManagedRef> for ManagedWire {
    fn from(r: ManagedRef) -> Self {
        Self {
            image: r.image,
            account: r.account,
            grid: r.grid,
        }
    }
}
impl ManagedRef {
    pub(crate) fn new(image: Uuid, account: Uuid, grid: String) -> Result<Self, &'static str> {
        ManagedWire { image, account, grid }.try_into()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalPicture {
    pub reference: ManagedRef,
    pub mode: ImageMode,
    pub brightness: u8,
}
#[derive(Clone)]
pub(crate) enum DrawImage {
    Texture(Picture),
    Local(LocalPicture),
}
impl DrawImage {
    pub(crate) fn mode(&self) -> ImageMode {
        match self {
            Self::Texture(p) => p.mode,
            Self::Local(p) => p.mode,
        }
    }
    pub(crate) fn brightness(&self) -> u8 {
        match self {
            Self::Texture(p) => p.brightness,
            Self::Local(p) => p.brightness,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Paint {
    Color { rgb: [u8; 3] },
    Texture { picture: Picture, fallback: [u8; 3] },
    Local { picture: LocalPicture, fallback: [u8; 3] },
}
impl Paint {
    pub(crate) fn fallback(&self) -> [u8; 3] {
        match self {
            Self::Color { rgb } => *rgb,
            Self::Texture { fallback, .. } | Self::Local { fallback, .. } => *fallback,
        }
    }
    pub(crate) fn picture(&self) -> Option<&Picture> {
        match self {
            Self::Texture { picture, .. } => Some(picture),
            _ => None,
        }
    }
    pub(crate) fn spanning(&self) -> bool {
        self.draw_image().is_some_and(|p| p.mode() == ImageMode::Span)
    }
    pub(crate) fn draw_image(&self) -> Option<DrawImage> {
        match self {
            Self::Texture { picture, .. } => Some(DrawImage::Texture(picture.clone())),
            Self::Local { picture, .. } => Some(DrawImage::Local(picture.clone())),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DisplayWire", into = "DisplayWire")]
pub(crate) struct DisplayPaints {
    arrangement: Arrangement,
    paints: Vec<Option<Paint>>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DisplayWire {
    arrangement: Arrangement,
    paints: Vec<Option<Paint>>,
}
impl TryFrom<DisplayWire> for DisplayPaints {
    type Error = &'static str;
    fn try_from(w: DisplayWire) -> Result<Self, Self::Error> {
        if w.arrangement.len() != w.paints.len() || w.paints.iter().flatten().any(Paint::spanning) {
            return Err("workspace display paints");
        }
        Ok(Self {
            arrangement: w.arrangement,
            paints: w.paints,
        })
    }
}
impl From<DisplayPaints> for DisplayWire {
    fn from(r: DisplayPaints) -> Self {
        Self {
            arrangement: r.arrangement,
            paints: r.paints,
        }
    }
}
impl DisplayPaints {
    pub(crate) fn empty(arrangement: Arrangement) -> Self {
        Self {
            paints: vec![None; arrangement.len()],
            arrangement,
        }
    }
    pub(crate) fn arrangement(&self) -> &Arrangement {
        &self.arrangement
    }
    pub(crate) fn paint(&self, index: usize) -> Option<&Paint> {
        self.paints.get(index)?.as_ref()
    }
    pub(crate) fn set(&mut self, index: usize, paint: Option<Paint>) -> Option<()> {
        if paint.as_ref().is_some_and(Paint::spanning) {
            return None;
        }
        *self.paints.get_mut(index)? = paint;
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_local_picture_reference_is_profile_metadata_and_preserves_the_inventory_picture_wire_shape() {
        let account = Uuid::new_v4();
        let grid = "a".repeat(64);
        let reference = ManagedRef::new(Uuid::new_v4(), account, grid.clone()).unwrap();
        let local = Paint::Local {
            picture: LocalPicture {
                reference: reference.clone(),
                mode: ImageMode::Fit,
                brightness: 200,
            },
            fallback: [1, 2, 3],
        };
        let bytes = serde_json::to_vec(&local).unwrap();
        assert_eq!(serde_json::from_slice::<Paint>(&bytes).unwrap(), local);
        assert!(ManagedRef::new(Uuid::nil(), account, grid.clone()).is_err());
        assert!(ManagedRef::new(reference.image, Uuid::nil(), grid.clone()).is_err());
        assert!(ManagedRef::new(reference.image, account, "file:///private".into()).is_err());
        let mut invalid = serde_json::to_value(reference).unwrap();
        invalid["path"] = "private".into();
        assert!(serde_json::from_value::<ManagedRef>(invalid).is_err());
        let texture = TextureRef::new(Uuid::new_v4(), Uuid::new_v4(), grid).unwrap();
        let legacy = serde_json::to_value(Picture {
            texture,
            mode: ImageMode::Fill,
            brightness: 255,
        })
        .unwrap();
        assert!(legacy.get("texture").is_some());
        assert!(legacy.get("reference").is_none());
        assert_eq!(legacy.as_object().unwrap().len(), 3);
    }
    #[test]
    fn workspace_picture_references_refuse_paths_nil_and_non_native_grid_keys() {
        assert!(TextureRef::new(Uuid::nil(), Uuid::new_v4(), "a".repeat(64)).is_err());
        assert!(TextureRef::new(Uuid::new_v4(), Uuid::nil(), "a".repeat(64)).is_err());
        for grid in ["file://private".into(), "A".repeat(64), "x".repeat(64), "a".repeat(63)] {
            assert!(TextureRef::new(Uuid::new_v4(), Uuid::new_v4(), grid).is_err());
        }
        let r = TextureRef::new(Uuid::new_v4(), Uuid::new_v4(), "a".repeat(64)).unwrap();
        let mut wire = serde_json::to_value(r).unwrap();
        wire["path"] = "private".into();
        assert!(serde_json::from_value::<TextureRef>(wire).is_err());
    }
    #[test]
    fn workspace_picture_display_codec_refuses_span_overrides_and_preserves_common_fallback() {
        let a = Arrangement::from_physical(&[[0, 0, 100, 100]]).unwrap();
        let mut slots = DisplayPaints::empty(a);
        let picture = Picture {
            texture: TextureRef::new(Uuid::new_v4(), Uuid::new_v4(), "a".repeat(64)).unwrap(),
            mode: ImageMode::Span,
            brightness: 255,
        };
        assert!(
            slots
                .set(
                    0,
                    Some(Paint::Texture {
                        picture: picture.clone(),
                        fallback: [1, 2, 3]
                    })
                )
                .is_none()
        );
        let p = Paint::Texture {
            picture: Picture {
                mode: ImageMode::Fit,
                ..picture
            },
            fallback: [1, 2, 3],
        };
        slots.set(0, Some(p.clone())).unwrap();
        assert_eq!(slots.paint(0), Some(&p));
        let bytes = serde_json::to_vec(&slots).unwrap();
        assert_eq!(serde_json::from_slice::<DisplayPaints>(&bytes).unwrap(), slots);
        let mut invalid = serde_json::to_value(slots).unwrap();
        invalid["paints"][0]["picture"]["mode"] = "span".into();
        assert!(serde_json::from_value::<DisplayPaints>(invalid).is_err());
    }
}
