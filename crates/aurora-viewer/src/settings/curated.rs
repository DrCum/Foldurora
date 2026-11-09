// SPDX-License-Identifier: LGPL-2.1-or-later
//! Closed native preference controls, after Folderstorm fspreferencesmodel.h
//! at abbe3cb0ca66ee2484f277219f6a27706fab7fc6. Only these scalar fields can
//! enter the assistant DTO or its temporary saved-layer projection.
use super::Settings;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(usize)]
pub(crate) enum Id {
    LetterKeysMove,
    ArrowKeysMove,
    TapHoldRun,
    QuickJump,
    MouseSensitivity,
    InvertMouse,
    SelectionBeam,
    LookAtTargets,
    UiScale,
    CameraFov,
    CameraOffsetScale,
    DrawDistance,
    GraphicsQuality,
    MasterVolume,
    EffectsVolume,
    MuteAudio,
}
impl Id {
    pub const ALL: [Self; 16] = [
        Self::LetterKeysMove,
        Self::ArrowKeysMove,
        Self::TapHoldRun,
        Self::QuickJump,
        Self::MouseSensitivity,
        Self::InvertMouse,
        Self::SelectionBeam,
        Self::LookAtTargets,
        Self::UiScale,
        Self::CameraFov,
        Self::CameraOffsetScale,
        Self::DrawDistance,
        Self::GraphicsQuality,
        Self::MasterVolume,
        Self::EffectsVolume,
        Self::MuteAudio,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::LetterKeysMove => "letter_keys_move",
            Self::ArrowKeysMove => "arrow_keys_move",
            Self::TapHoldRun => "tap_hold_run",
            Self::QuickJump => "quick_jump",
            Self::MouseSensitivity => "mouse_sensitivity",
            Self::InvertMouse => "invert_mouse",
            Self::SelectionBeam => "selection_beam",
            Self::LookAtTargets => "look_at_targets",
            Self::UiScale => "ui_scale",
            Self::CameraFov => "camera_fov",
            Self::CameraOffsetScale => "camera_offset_scale",
            Self::DrawDistance => "draw_distance",
            Self::GraphicsQuality => "graphics_quality",
            Self::MasterVolume => "master_volume",
            Self::EffectsVolume => "effects_volume",
            Self::MuteAudio => "mute_audio",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.key() == s)
    }
    pub fn writable(self) -> bool {
        (self as usize) < 12
    }
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::UiScale | Self::CameraFov | Self::CameraOffsetScale | Self::DrawDistance | Self::GraphicsQuality
        )
    }
    pub fn camera(self) -> bool {
        matches!(self, Self::CameraFov | Self::CameraOffsetScale)
    }
    pub fn kind(self) -> &'static str {
        match self {
            Self::MouseSensitivity
            | Self::UiScale
            | Self::CameraFov
            | Self::CameraOffsetScale
            | Self::DrawDistance
            | Self::MasterVolume
            | Self::EffectsVolume => "number",
            Self::LookAtTargets | Self::GraphicsQuality => "integer",
            _ => "boolean",
        }
    }
    pub fn bounds(self) -> Option<(f64, f64)> {
        match self {
            Self::MouseSensitivity => Some((0.1, 5.)),
            Self::UiScale => Some((0.75, 2.)),
            Self::CameraFov => Some((0.17, 2.97)),
            Self::CameraOffsetScale => Some((0.5, 3.)),
            Self::DrawDistance => Some((32., 512.)),
            Self::LookAtTargets => Some((0., 1.)),
            Self::GraphicsQuality => Some((0., 6.)),
            Self::MasterVolume | Self::EffectsVolume => Some((0., 1.)),
            _ => None,
        }
    }
    pub fn input(self, v: &serde_json::Value) -> Result<Scalar, &'static str> {
        if !self.writable() {
            return Err("preference_read_only");
        }
        match self.kind() {
            "boolean" => v.as_bool().map(Scalar::Boolean).ok_or("invalid_preference_type"),
            "integer" => v
                .as_i64()
                .filter(|n| matches!(*n, 0 | 1))
                .map(|n| Scalar::Integer(n as i32))
                .ok_or("invalid_preference_type_or_range"),
            _ => {
                let (lo, hi) = self.bounds().ok_or("invalid_preference")?;
                let n = v
                    .as_f64()
                    .filter(|n| n.is_finite() && (lo..=hi).contains(n))
                    .ok_or("invalid_preference_type_or_range")?;
                Ok(Scalar::Number(n as f32 as f64))
            }
        }
    }
    pub fn write(self, s: &mut Settings, v: Scalar) -> bool {
        match (self, v) {
            (Self::LetterKeysMove, Scalar::Boolean(v)) => s.letter_keys_move = v,
            (Self::ArrowKeysMove, Scalar::Boolean(v)) => s.arrow_keys_always_move = v,
            (Self::TapHoldRun, Scalar::Boolean(v)) => s.tap_hold_run = v,
            (Self::QuickJump, Scalar::Boolean(v)) => s.quickjump = v,
            (Self::MouseSensitivity, Scalar::Number(v)) => s.mouse_sensitivity = v as f32,
            (Self::InvertMouse, Scalar::Boolean(v)) => s.invert_mouse = v,
            (Self::SelectionBeam, Scalar::Boolean(v)) => s.selection_beam = v,
            (Self::LookAtTargets, Scalar::Integer(v)) => s.lookat.show_targets = v == 1,
            (Self::UiScale, Scalar::Number(v)) => s.ui_scale = v as f32,
            (Self::CameraFov, Scalar::Number(v)) => s.camera.fov = v as f32,
            (Self::CameraOffsetScale, Scalar::Number(v)) => s.camera.offset_scale = v as f32,
            (Self::DrawDistance, Scalar::Number(v)) => s.draw_distance = v as f32,
            _ => return false,
        }
        true
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum Scalar {
    Boolean(bool),
    Integer(i32),
    Number(f64),
    Unknown,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Values([Scalar; 16]);
impl Values {
    pub fn capture(s: &Settings) -> Self {
        use Scalar::{Boolean as B, Integer as I, Number as N};
        let quality = crate::graphics_presets::Preset::matching(s)
            .map(|p| match p {
                crate::graphics_presets::Preset::Potato => 0,
                crate::graphics_presets::Preset::Low => 1,
                crate::graphics_presets::Preset::Medium => 3,
                crate::graphics_presets::Preset::High => 5,
                crate::graphics_presets::Preset::Deluxe => 6,
            })
            .map(I)
            .unwrap_or(Scalar::Unknown);
        Self([
            B(s.letter_keys_move),
            B(s.arrow_keys_always_move),
            B(s.tap_hold_run),
            B(s.quickjump),
            N(s.mouse_sensitivity as f64),
            B(s.invert_mouse),
            B(s.selection_beam),
            I(i32::from(s.lookat.show_targets)),
            N(s.ui_scale as f64),
            N(s.camera.fov as f64),
            N(s.camera.offset_scale as f64),
            N(s.draw_distance as f64),
            quality,
            N(s.audio.volume[0] as f64),
            N(s.audio.volume[3] as f64),
            B(s.audio.muted[0]),
        ])
    }
    pub fn get(&self, id: Id) -> Scalar {
        self.0[id as usize]
    }
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Saved(Arc<Mutex<Option<Values>>>);
impl Saved {
    pub fn get(&self, s: &Settings) -> Values {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        guard.get_or_insert_with(|| Values::capture(s)).clone()
    }
    pub fn record(&self, s: &Settings) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(Values::capture(s));
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Field {
    pub before: Scalar,
    pub saved: Scalar,
    pub expected: Scalar,
    pub external: bool,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Held(pub BTreeMap<Id, Field>, pub Option<Option<String>>);
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct Cleanup {
    pub restored: Vec<&'static str>,
    pub skipped: Vec<&'static str>,
}
impl Held {
    pub fn observe(&mut self, current: &Values) {
        for (id, f) in &mut self.0 {
            if current.get(*id) != f.expected {
                f.external = true;
            }
        }
    }
    pub fn project(&self, s: &mut Settings) {
        let current = Values::capture(s);
        for (id, f) in &self.0 {
            if !f.external && current.get(*id) == f.expected {
                id.write(s, f.saved);
            }
        }
        if self
            .0
            .get(&Id::DrawDistance)
            .is_some_and(|f| !f.external && current.get(Id::DrawDistance) == f.expected)
            && s.graphics_saved_active.is_none()
            && let Some(active) = &self.1
        {
            s.graphics_saved_active = active.clone();
        }
    }
    pub fn restore(mut self, s: &mut Settings, allowed: impl Fn(Id) -> bool) -> (Option<Self>, Cleanup) {
        self.observe(&Values::capture(s));
        let graphics_owned = self
            .0
            .get(&Id::DrawDistance)
            .is_some_and(|f| !f.external && allowed(Id::DrawDistance));
        let mut cleanup = Cleanup::default();
        self.0.retain(|id, f| {
            if f.external {
                cleanup.skipped.push(id.key());
                return false;
            }
            if !allowed(*id) {
                cleanup.skipped.push(id.key());
                return true;
            }
            id.write(s, f.before);
            cleanup.restored.push(id.key());
            false
        });
        if graphics_owned
            && s.graphics_saved_active.is_none()
            && let Some(active) = self.1.take()
        {
            s.graphics_saved_active = active;
        }
        ((!self.0.is_empty()).then_some(self), cleanup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn preview(s: &mut Settings) -> Held {
        let before = Values::capture(s);
        s.curated_saved.record(s);
        let mut hold = Held::default();
        for (id, value) in [(Id::UiScale, Scalar::Number(1.5)), (Id::MouseSensitivity, Scalar::Number(2.))] {
            hold.0.insert(
                id,
                Field {
                    before: before.get(id),
                    saved: before.get(id),
                    expected: value,
                    external: false,
                },
            );
            assert!(id.write(s, value));
        }
        hold
    }
    #[test]
    fn preview_saved_projection_and_backup_preserve_baseline_and_unrelated_edits() {
        let mut s = Settings::default();
        s.curated_held = Some(preview(&mut s));
        s.show_balance = !s.show_balance;
        let saved = s.to_saved();
        assert_eq!(saved.ui_scale, 1.);
        assert_eq!(saved.mouse_sensitivity, 1.);
        assert_eq!(saved.show_balance, s.show_balance);
        assert_eq!(s.ui_scale, 1.5);
        let backup = super::super::backup::Backup::new(&s, None).to_bytes().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&backup).unwrap();
        assert_eq!(v["settings"]["ui_scale"], 1.);
        assert!(v["settings"].get("curated_held").is_none());
    }
    #[test]
    fn cleanup_preserves_manual_edits_and_keeps_blocked_drafts_out_of_saved_layer() {
        let mut s = Settings::default();
        let hold = preview(&mut s);
        s.mouse_sensitivity = 3.;
        let (residual, result) = hold.restore(&mut s, |id| id != Id::UiScale);
        assert_eq!(s.mouse_sensitivity, 3.);
        assert_eq!(s.ui_scale, 1.5);
        assert_eq!(result.skipped.len(), 2);
        s.curated_held = residual;
        assert_eq!(s.to_saved().ui_scale, 1.);
        assert_eq!(s.to_saved().mouse_sensitivity, 3.);
        let hold = s.curated_held.take().unwrap();
        let (residual, result) = hold.restore(&mut s, |_| true);
        assert!(residual.is_none());
        assert_eq!(result.restored, vec!["ui_scale"]);
        assert_eq!(s.ui_scale, 1.);
    }
    #[test]
    fn public_control_types_and_native_bounds_are_closed() {
        assert!(Id::parse("local_assistant").is_none());
        assert!(Id::MouseSensitivity.input(&serde_json::json!(8)).is_err());
        assert!(Id::DrawDistance.input(&serde_json::json!(1024)).is_err());
        assert!(Id::LookAtTargets.input(&serde_json::json!(true)).is_err());
        assert!(Id::TapHoldRun.input(&serde_json::json!("true")).is_err());
        assert!(Id::MasterVolume.input(&serde_json::json!(0.5)).is_err());
        assert_eq!(Id::ALL.iter().filter(|id| id.writable()).count(), 12);
    }
}
