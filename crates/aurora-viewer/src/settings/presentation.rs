//! Native presentation normalization shared by ordinary settings and the
//! selective profile-copy validator. These are the existing native ranges.
use super::{BLOOM_STRENGTH, FONT_SCALE_MAX, FONT_SCALE_MIN};

pub(super) fn draw_distance(value: f32) -> f32 {
    value.clamp(32.0, 512.0)
}
pub(super) fn exposure(value: f32) -> f32 {
    value.clamp(0.25, 4.0)
}
pub(super) fn sharpen(value: f32) -> f32 {
    if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.4 }
}
pub(super) fn bloom_strength(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.05, 1.0)
    } else {
        BLOOM_STRENGTH
    }
}
pub(super) fn lod_factor(value: f32) -> f32 {
    value.clamp(0.5, 4.0)
}
pub(super) fn ui_scale(value: f32) -> f32 {
    value.clamp(0.75, 2.0)
}
pub(super) fn texture_budget_mb(value: u32) -> u32 {
    value.clamp(256, 32768)
}
pub(super) fn font_scale(value: f32) -> f32 {
    value.clamp(FONT_SCALE_MIN, FONT_SCALE_MAX)
}
pub(super) fn chat_font_scale(value: f32) -> f32 {
    if value.is_finite() { font_scale(value) } else { 1.0 }
}
pub(super) fn shadow_quality(value: u8) -> u8 {
    value.min(4)
}
pub(super) fn antialiasing(value: u8) -> u8 {
    value.min(6)
}
pub(super) fn render_scale_percent(value: u32) -> u32 {
    value.clamp(50, 100)
}
