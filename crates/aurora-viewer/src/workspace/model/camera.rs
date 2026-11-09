//! Folderstorm FSWorkspaceContext avatar-relative controls, using Aurora ranges.
//! No object/world anchor, photo lens, environment, movement or XR state is saved.
use crate::camera::{CameraPreset, CameraSettings};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CameraContext {
    version: u8,
    preset: CameraPreset,
    camera_offset: [f32; 3],
    focus_offset: [f32; 3],
    offset_scale: f32,
    fov: f32,
    zoom_time: f32,
    smoothing: f32,
    track_focus_object: bool,
    pub zoom: f32,
}
impl CameraContext {
    pub(crate) fn capture(s: &CameraSettings, zoom: f32) -> Self {
        Self {
            version: 1,
            preset: s.preset,
            camera_offset: s.camera_offset,
            focus_offset: s.focus_offset,
            offset_scale: s.offset_scale,
            fov: s.fov,
            zoom_time: s.zoom_time,
            smoothing: s.smoothing,
            track_focus_object: s.track_focus_object,
            zoom,
        }
    }
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let range = |v: f32, lo: f32, hi: f32| v.is_finite() && (lo..=hi).contains(&v);
        if self.version != 1
            || self
                .camera_offset
                .iter()
                .chain(&self.focus_offset)
                .any(|v| !range(*v, -128.0, 128.0))
            || self.camera_offset.iter().map(|v| v * v).sum::<f32>() < 0.0001
            || !range(self.offset_scale, 0.5, 3.0)
            || !range(self.fov, 5.0f32.to_radians(), 175.0f32.to_radians())
            || !range(self.zoom_time, 0.0, 4.0)
            || !range(self.smoothing, 0.0, 9.0)
            || !range(self.zoom, 0.0, 1.0)
        {
            return Err("camera context");
        }
        Ok(())
    }
    pub(crate) fn matches(&self, other: &Self) -> bool {
        let mut expected = self.clone();
        expected.zoom = other.zoom;
        expected == *other && (self.zoom - other.zoom).abs() <= 0.0001
    }
    /// Pure save projection. Live application must use the App's current-source
    /// and RLVa guards; this function never changes camera mode or world state.
    pub(crate) fn preferences(&self, s: &mut CameraSettings) {
        if self.validate().is_err() {
            return;
        }
        s.preset = self.preset;
        s.camera_offset = self.camera_offset;
        s.focus_offset = self.focus_offset;
        s.offset_scale = self.offset_scale;
        s.fov = self.fov;
        s.zoom_time = self.zoom_time;
        s.smoothing = self.smoothing;
        s.track_focus_object = self.track_focus_object;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_camera_snapshot_is_bounded_and_refuses_unknown_or_future_data() {
        let c = CameraContext::capture(&CameraSettings::default(), 0.5);
        assert!(c.validate().is_ok());
        let mut future = c.clone();
        future.version = 2;
        assert!(future.validate().is_err());
        for zoom in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            let mut invalid = c.clone();
            invalid.zoom = zoom;
            assert!(invalid.validate().is_err());
        }
        let mut invalid = c.clone();
        invalid.camera_offset = [0.0; 3];
        assert!(invalid.validate().is_err());
        let mut value = serde_json::to_value(c).unwrap();
        value["environment"] = "excluded".into();
        assert!(serde_json::from_value::<CameraContext>(value).is_err());
    }
    #[test]
    fn workspace_camera_preferences_preserve_unrelated_mouselook_and_constraint_choices() {
        let mut s = CameraSettings::default();
        let c = CameraContext::capture(&s, 0.7);
        s.apply_preset(CameraPreset::Front);
        s.disable_constraints = true;
        s.enable_mouselook = false;
        s.vr_view = crate::camera::VrView::DesktopCamera;
        c.preferences(&mut s);
        assert_eq!(s.preset, CameraPreset::Rear);
        assert!(s.disable_constraints);
        assert!(!s.enable_mouselook);
        assert_eq!(s.vr_view, crate::camera::VrView::DesktopCamera);
        assert_eq!(CameraContext::capture(&s, 0.7), c);
    }
    #[test]
    fn workspace_camera_comparison_tolerates_only_native_zoom_rounding() {
        let original = CameraContext::capture(&CameraSettings::default(), 0.4);
        let mut live = original.clone();
        live.zoom += f32::EPSILON;
        assert!(original.matches(&live));
        live.zoom += 0.01;
        assert!(!original.matches(&live));
        live.zoom = original.zoom;
        live.fov += 0.01;
        assert!(!original.matches(&live));
    }
}
