//! A native layout hint, never permission to capture or a prepared-frame proof.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "LayoutFields", into = "LayoutFields")]
pub struct PreviewLayout {
    source: PreviewSource,
    epochs: PreviewEpochs,
    activation: u64,
    demand: u64,
    device_epoch: u64,
    source_size: [u32; 2],
    source_rect: [u32; 4],
}
impl std::fmt::Debug for PreviewLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreviewLayout([redacted])")
    }
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutFields {
    source: PreviewSource,
    epochs: PreviewEpochs,
    activation: u64,
    demand: u64,
    device_epoch: u64,
    source_size: [u32; 2],
    source_rect: [u32; 4],
}
impl PreviewLayout {
    pub fn new(
        source: PreviewSource,
        epochs: PreviewEpochs,
        activation: u64,
        demand: u64,
        device_epoch: u64,
        source_size: [u32; 2],
        source_rect: [u32; 4],
    ) -> Result<Self, ImageError> {
        let [x, y, width, height] = source_rect;
        if activation == 0
            || demand == 0
            || device_epoch == 0
            || source_size.contains(&0)
            || width == 0
            || height == 0
            || x.checked_add(width).is_none_or(|right| right > source_size[0])
            || y.checked_add(height).is_none_or(|bottom| bottom > source_size[1])
        {
            return Err(ImageError::Invalid);
        }
        Ok(Self {
            source,
            epochs,
            activation,
            demand,
            device_epoch,
            source_size,
            source_rect,
        })
    }
    pub fn source(self) -> PreviewSource {
        self.source
    }
    pub fn epochs(self) -> PreviewEpochs {
        self.epochs
    }
    pub fn activation(self) -> u64 {
        self.activation
    }
    pub fn demand(self) -> u64 {
        self.demand
    }
    pub fn device_epoch(self) -> u64 {
        self.device_epoch
    }
    pub fn source_size(self) -> [u32; 2] {
        self.source_size
    }
    pub fn source_rect(self) -> [u32; 4] {
        self.source_rect
    }
    pub fn geometry(self, cap: PreviewSize) -> Result<PreviewGeometry, ImageError> {
        PreviewGeometry::new(self.source_size, self.source_rect, cap)
    }
    pub fn matches_policy(self, policy: PreviewPolicy) -> bool {
        self.source == policy.source()
            && self.epochs == policy.epochs()
            && self.activation == policy.access().activation
            && policy.access().capture_active
    }
}
impl TryFrom<LayoutFields> for PreviewLayout {
    type Error = &'static str;
    fn try_from(value: LayoutFields) -> Result<Self, Self::Error> {
        Self::new(
            value.source,
            value.epochs,
            value.activation,
            value.demand,
            value.device_epoch,
            value.source_size,
            value.source_rect,
        )
        .map_err(|_| "invalid preview layout")
    }
}
impl From<PreviewLayout> for LayoutFields {
    fn from(value: PreviewLayout) -> Self {
        Self {
            source: value.source,
            epochs: value.epochs,
            activation: value.activation,
            demand: value.demand,
            device_epoch: value.device_epoch,
            source_size: value.source_size,
            source_rect: value.source_rect,
        }
    }
}
