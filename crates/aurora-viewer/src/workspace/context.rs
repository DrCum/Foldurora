// SPDX-License-Identifier: LGPL-2.1-or-later
//! Closed snapshots of existing native workspace context controls. Source:
//! Folderstorm fsworkspacecontext.h at abbe3cb0ca66ee2484f277219f6a27706fab7fc6.
//! Native Rust types and ranges remain authoritative. Backgrounds have no
//! native renderer here and are deliberately absent from this model.
use crate::{camera::CameraPreset, settings::Settings};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! graphics {
    ($($field:ident:$ty:ty),+ $(,)?)=>{
        #[derive(Clone,Debug,PartialEq,Serialize,Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(crate) struct Graphics {$(pub $field:$ty,)+pub depth_of_field:bool,pub active:Option<String>}
        impl Graphics {
            pub fn capture(s:&Settings)->Self {Self {$($field:s.$field,)+depth_of_field:s.dof.enabled,active:s.graphics_saved_active.clone()}}
            pub fn write(&self,s:&mut Settings) {$(s.$field=self.$field;)+s.shadows=s.shadow_quality>0;s.dof.enabled=self.depth_of_field;s.graphics_saved_active=self.active.clone();}
            pub fn preserve_external(&mut self,current:&Self,expected:&Self) {
                $(if current.$field!=expected.$field {self.$field=current.$field;})+
                if current.depth_of_field!=expected.depth_of_field {self.depth_of_field=current.depth_of_field;}
                if current.active!=expected.active {self.active=current.active.clone();}
            }
            pub fn validate(&self)->bool {
                let mut s=Settings::default();self.write(&mut s);let s=s.sanitized();
                let mut normalized=Self::capture(&s);normalized.active=self.active.clone();
                normalized==*self && self.active.as_ref().is_none_or(|n|crate::graphics_presets::saved::valid_name(n))
            }
        }
    }
}
graphics! {draw_distance:f32,shadow_quality:u8,ssao:u8,water_reflections:u8,reflection_probes:u8,probe_count:u32,max_avatars:u32,max_complexity:u32,max_particles:u32,lod_factor:f32,glow:bool,fps_cap:bool,fps_limit:u32}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Camera {
    pub preset: CameraPreset,
    pub camera_offset: [f32; 3],
    pub focus_offset: [f32; 3],
    pub offset_scale: f32,
    pub fov: f32,
    pub zoom_time: f32,
    pub smoothing: f32,
    pub track_focus_object: bool,
    pub zoom: f32,
}
impl Camera {
    pub fn capture(s: &Settings, zoom: f32) -> Self {
        let c = &s.camera;
        Self {
            preset: c.preset,
            camera_offset: c.camera_offset,
            focus_offset: c.focus_offset,
            offset_scale: c.offset_scale,
            fov: c.fov,
            zoom_time: c.zoom_time,
            smoothing: c.smoothing,
            track_focus_object: c.track_focus_object,
            zoom,
        }
    }
    pub fn write(&self, s: &mut Settings) {
        let c = &mut s.camera;
        c.preset = self.preset;
        c.camera_offset = self.camera_offset;
        c.focus_offset = self.focus_offset;
        c.offset_scale = self.offset_scale;
        c.fov = self.fov;
        c.zoom_time = self.zoom_time;
        c.smoothing = self.smoothing;
        c.track_focus_object = self.track_focus_object;
    }
    pub fn preserve_external(&mut self, current: &Self, expected: &Self) {
        macro_rules! keep {($($f:ident),+)=>{$(if current.$f!=expected.$f {self.$f=current.$f;})+};}
        keep!(
            preset,
            camera_offset,
            focus_offset,
            offset_scale,
            fov,
            zoom_time,
            smoothing,
            track_focus_object,
            zoom
        );
    }
    pub fn validate(&self) -> bool {
        let mut s = Settings::default();
        self.write(&mut s);
        s.camera = s.camera.sanitized();
        Self::capture(&s, self.zoom) == *self
            && self.zoom.is_finite()
            && (0.0..=1.0).contains(&self.zoom)
            && self
                .camera_offset
                .iter()
                .chain(&self.focus_offset)
                .all(|v| v.is_finite() && v.abs() <= 128.)
            && crate::camera::FOV_RANGE.contains(&self.fov)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Hud {
    pub item: Uuid,
    pub name: String,
    pub point: u8,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Context {
    pub graphics: Option<Graphics>,
    pub camera: Option<Camera>,
    pub huds: Vec<Hud>,
}
impl Context {
    pub fn validate(&self) -> bool {
        let mut ids = std::collections::BTreeSet::new();
        self.graphics.as_ref().is_none_or(Graphics::validate)
            && self.camera.as_ref().is_none_or(Camera::validate)
            && self.huds.len() <= 64
            && self.huds.iter().all(|h| {
                !h.item.is_nil()
                    && ids.insert(h.item)
                    && (31..=38).contains(&h.point)
                    && !h.name.is_empty()
                    && h.name.len() <= 96
                    && !h.name.chars().any(char::is_control)
            })
    }
    pub fn capture_settings(s: &Settings, like: &Self, zoom: f32) -> Self {
        Self {
            graphics: like.graphics.as_ref().map(|_| Graphics::capture(s)),
            camera: like.camera.as_ref().map(|_| Camera::capture(s, zoom)),
            huds: Vec::new(),
        }
    }
    pub fn write(&self, s: &mut Settings) {
        if let Some(g) = &self.graphics {
            g.write(s);
        }
        if let Some(c) = &self.camera {
            c.write(s);
        }
    }
    pub fn preserve_external(&mut self, current: &Self, expected: &Self) {
        if let (Some(before), Some(current), Some(expected)) = (&mut self.graphics, &current.graphics, &expected.graphics) {
            before.preserve_external(current, expected);
        }
        if let (Some(before), Some(current), Some(expected)) = (&mut self.camera, &current.camera, &expected.camera) {
            before.preserve_external(current, expected);
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Capture {
    pub graphics: bool,
    pub camera: bool,
    pub huds: bool,
}
/// Separate unsaved context layers let a restricted camera retain its original
/// saved values after a partial runtime cleanup. No unrelated setting enters.
#[derive(Clone, Debug)]
pub(crate) struct Held {
    pub before: Context,
    pub expected: Context,
}
impl Held {
    pub fn project(&self, s: &mut Settings) {
        let zoom = self.expected.camera.as_ref().map_or(0., |c| c.zoom);
        let current = Context::capture_settings(s, &self.expected, zoom);
        let mut before = self.before.clone();
        before.preserve_external(&current, &self.expected);
        before.write(s);
    }
    pub fn restore(self, s: &mut Settings, camera_allowed: bool) -> Option<Self> {
        let zoom = self.expected.camera.as_ref().map_or(0., |c| c.zoom);
        let current = Context::capture_settings(s, &self.expected, zoom);
        let mut before = self.before.clone();
        before.preserve_external(&current, &self.expected);
        if let Some(g) = &before.graphics {
            g.write(s);
        }
        if camera_allowed {
            if let Some(c) = &before.camera {
                c.write(s);
            }
            None
        } else {
            before.graphics = None;
            let mut expected = self.expected;
            expected.graphics = None;
            before.camera.as_ref()?;
            Some(Self { before, expected })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_saved_projection_and_cleanup_preserve_unrelated_and_manual_fields() {
        let mut s = Settings::default();
        let before = Context {
            graphics: Some(Graphics::capture(&s)),
            camera: Some(Camera::capture(&s, 0.5)),
            ..Default::default()
        };
        let mut expected = before.clone();
        expected.graphics.as_mut().unwrap().draw_distance = 256.;
        expected.camera.as_mut().unwrap().fov = 1.5;
        expected.write(&mut s);
        s.mouse_sensitivity = 3.;
        s.fps_limit = 77;
        let held = Held { before, expected };
        let mut saved = s.clone();
        held.project(&mut saved);
        assert_eq!(saved.draw_distance, 96.);
        assert_eq!(saved.camera.fov, crate::camera::DEFAULT_FOV);
        assert_eq!(saved.mouse_sensitivity, 3.);
        assert_eq!(saved.fps_limit, 77);
        assert_eq!(s.draw_distance, 256.);
        let residual = held.restore(&mut s, false).unwrap();
        assert_eq!(s.draw_distance, 96.);
        assert_eq!(s.camera.fov, 1.5);
        assert!(residual.restore(&mut s, true).is_none());
        assert_eq!(s.camera.fov, crate::camera::DEFAULT_FOV);
    }
    #[test]
    fn context_rejects_unknown_fields_and_invalid_hud_points() {
        assert!(serde_json::from_str::<Context>(r#"{"credentials":{}}"#).is_err());
        let c = Context {
            huds: vec![Hud {
                item: Uuid::new_v4(),
                name: "HUD".into(),
                point: 1,
            }],
            ..Default::default()
        };
        assert!(!c.validate());
    }
}
