//! Synthetic CPU visuals of the dormant production body. Original sealed
//! PrivateFrames are consumed by the surface owner's guarded fixture painter.
use super::tests::{AdmittedFixture, Frames, draw_private_events};
use super::*;
use crate::crash::software::Painter;
use crate::session_host::private_image::{PrivateImage, PrivateImageSlot, UiPixelBudget};
use egui::{Event, Modifiers, PointerButton, Pos2};
use serde_json::{Value, json};
use std::path::Path;

const HIDDEN: &str = "SYNTHETIC_CANONICAL_DRAFT";
fn text(shape: &egui::Shape, clip: Rect, output: &mut Vec<(String, bool)>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                text(shape, clip, output);
            }
        }
        egui::Shape::Text(shape) => {
            let bounds = Rect::from_min_size(shape.pos, shape.galley.size());
            if clip.intersects(bounds) && !shape.galley.text().is_empty() {
                output.push((shape.galley.text().to_owned(), !shape.galley.elided && clip.contains_rect(bounds)));
            }
        }
        egui::Shape::Callback(_) => panic!("CPU monitor fixture cannot execute callbacks"),
        _ => {}
    }
}
struct Fixture {
    f: Frames,
    index: usize,
    admitted: AdmittedFixture,
    slot: PrivateImageSlot,
    painter: Painter,
    pixels: Vec<u32>,
    texts: Vec<(String, bool)>,
    physical: [usize; 2],
    scale: f32,
}
impl Fixture {
    fn new(logical: [f32; 2], scale: f32, index: usize) -> Self {
        let mut f = Frames::new(logical, scale);
        f.focus(index);
        f.states[index - 1].chat = false;
        for context in &f.contexts {
            crate::ui::fonts::apply(context, crate::ui::fonts::DEFAULT);
        }
        let admitted = AdmittedFixture::new(f.model.binding(Frames::view(index)).unwrap().source);
        let budget = UiPixelBudget::new();
        let mut slot = PrivateImageSlot::new(f.contexts[index].clone(), 7, budget.clone()).unwrap();
        slot.stage(7, PrivateImage::from_rgba(&budget, admitted.size, &admitted.rgba).unwrap())
            .unwrap();
        let physical = logical.map(|side| (side * scale).round() as usize);
        Self {
            f,
            index,
            admitted,
            slot,
            painter: Painter::default(),
            pixels: vec![0; physical[0] * physical[1]],
            texts: Vec::new(),
            physical,
            scale,
        }
    }
    fn events(&mut self, events: Vec<Event>) {
        let frame = draw_private_events(&mut self.f, self.index, &mut self.slot, &self.admitted.admitted, events);
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(self.f.logical[0], self.f.logical[1]));
        self.texts.clear();
        assert!((frame.output().pixels_per_point - self.scale).abs() < 0.001);
        for shape in &frame.output().shapes {
            text(&shape.shape, shape.clip_rect.intersect(screen), &mut self.texts);
        }
        let context = self.f.contexts[self.index].clone();
        let now = self.f.now;
        let admitted = &self.admitted.admitted;
        self.slot
            .paint_fixture(
                &context,
                frame,
                &mut self.painter,
                &mut self.pixels,
                self.physical,
                crate::theme::Theme::default().palette().panel,
                &mut || admitted.current(now),
            )
            .unwrap();
    }
    fn settle(&mut self) {
        for _ in 0..4 {
            self.events(Vec::new());
        }
    }
    fn type_text(&mut self, value: &str) {
        self.settle();
        let at = self.f.contexts[self.index]
            .data(|data| data.get_temp::<Rect>(Id::unique("host_chat_input_rect")))
            .unwrap()
            .center();
        for pressed in [true, false] {
            self.events(vec![
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ]);
        }
        self.events(vec![Event::Text(value.into())]);
    }
    fn click_options(&mut self) {
        self.settle();
        let view = Frames::view(self.index);
        let expected = !self.f.bodies[self.index - 1].options_open;
        let name = if expected { "options" } else { "close_options" };
        let at = self.f.contexts[self.index]
            .data(|data| data.get_temp::<Rect>(control_id(view, name)))
            .unwrap()
            .center();
        for pressed in [true, false] {
            self.events(vec![
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ]);
        }
        self.settle();
        assert_eq!(self.f.bodies[self.index - 1].options_open, expected);
    }
    fn scroll_options(&mut self) {
        let view = Frames::view(self.index);
        let at = self.f.contexts[self.index]
            .data(|data| data.get_temp::<Rect>(control_id(view, "options_scroll")))
            .unwrap()
            .center();
        for _ in 0..3 {
            self.events(vec![
                Event::PointerMoved(at),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                    delta: vec2(0.0, -1000.0),
                    modifiers: Modifiers::NONE,
                },
            ]);
            self.settle();
        }
    }
    fn capture(&mut self, dir: &Path, name: &str, keys: &[&'static str], draft_visible: bool) -> Value {
        self.settle();
        let mut required: Vec<_> = keys.iter().map(|key| aurora_i18n::text(key).to_owned()).collect();
        if self
            .f
            .model
            .binding(Frames::view(self.index))
            .is_some_and(|binding| binding.enabled && binding.visible)
        {
            required.push("Synthetic Resident".to_owned());
            required.push("Synthetic Grid".to_owned());
            required.push("Same profile".to_owned());
            let (youngest, oldest) = self.admitted.admitted.age().bounds(self.f.now).unwrap();
            required.push(aurora_i18n::tr!(
                "host-monitor-last-active",
                youngest = (youngest / 1000) as i64,
                oldest = oldest.div_ceil(1000) as i64
            ));
        }
        let missing: Vec<_> = required
            .iter()
            .filter(|required| {
                !self
                    .texts
                    .iter()
                    .any(|(text, complete)| *complete && text.contains(required.as_str()))
            })
            .collect();
        let leaks: Vec<_> = self
            .texts
            .iter()
            .filter(|(text, _)| !draft_visible && text.contains(HIDDEN))
            .collect();
        let rgba: Vec<_> = self
            .pixels
            .iter()
            .flat_map(|pixel| [(*pixel >> 16) as u8, (*pixel >> 8) as u8, *pixel as u8, 255])
            .collect();
        image::save_buffer(
            dir.join(format!("{name}.png")),
            &rgba,
            self.physical[0] as u32,
            self.physical[1] as u32,
            image::ColorType::Rgba8,
        )
        .unwrap();
        let record = json!({
            "image":format!("{name}.png"), "language":aurora_i18n::language(), "logical_size":self.f.logical,
            "physical_size":self.physical,"pixels_per_point":self.scale,"view":format!("{:?}",Frames::view(self.index)),
            "binding":format!("{:?}",self.f.model.binding(Frames::view(self.index))),"published_state":format!("{:?}",self.f.states[self.index-1]),
            "synthetic":true,"native":false,"authenticated_io":false,"image_admission":"actual pure SourceImages, synthetic policy/stamp/pixels",
            "sealed_original_frame_consumed":true,"private_pixel_bytes_in_body":0,
            "texture_owner_generation":self.f.bodies[self.index-1].generation,"binding_namespace":self.f.bodies[self.index-1].binding_generation,
            "admitted_stamp":format!("{:?}",self.admitted.admitted.stamp()),"age_bounds_ms":self.admitted.admitted.age().bounds(self.f.now).unwrap(),
            "missing_or_clipped_required_text":missing,"hidden_draft_leaks":leaks,
            "painted_text":self.texts.iter().map(|(text,complete)|json!({"text":text,"fully_visible":complete})).collect::<Vec<_>>()
        });
        std::fs::write(dir.join(format!("{name}.json")), serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        assert!(missing.is_empty(), "{name}: missing or clipped {missing:?}");
        assert!(leaks.is_empty(), "{name}: hidden canonical draft leaked");
        record
    }
}
fn layout(dir: &Path, name: &str, logical: [f32; 2], scale: f32) -> Vec<Value> {
    let mut f = Fixture::new(logical, scale, 1);
    let mut records = vec![f.capture(
        dir,
        &format!("{name}-01-image"),
        &[
            "host-monitor-exchange",
            "host-monitor-chat",
            "host-monitor-pin",
            "host-monitor-options",
        ],
        false,
    )];
    // Use real canonical editor input, never a second draft in the body.
    f.f.states[0].chat = true;
    f.f.states[0].revision += 1;
    f.type_text(HIDDEN);
    records.push(f.capture(dir, &format!("{name}-02-chat"), &["host-chat-send", "host-monitor-exchange"], true));
    assert!(f.texts.iter().any(|(text, _)| text.contains(HIDDEN)));
    f.f.states[0].chat = false;
    f.f.states[0].revision += 1;
    f.click_options();
    records.push(f.capture(
        dir,
        &format!("{name}-03-options-top"),
        &["host-monitor-sampling-help", "host-monitor-small", "host-monitor-medium"],
        false,
    ));
    f.scroll_options();
    records.push(f.capture(dir, &format!("{name}-04-options-bottom"), &["host-monitor-rate-five"], false));
    f.click_options();
    f.f.states[0].revision += 1;
    f.f.states[0].request = Some((
        77,
        Options {
            size: PreviewSize::Medium,
            ..Default::default()
        },
    ));
    records.push(f.capture(
        dir,
        &format!("{name}-05-request-pending"),
        &["host-monitor-settings-pending"],
        false,
    ));
    f.f.states[0].revision += 1;
    f.f.states[0].request = None;
    f.f.states[0].failed = true;
    records.push(f.capture(dir, &format!("{name}-06-request-failed"), &["host-monitor-settings-failed"], false));
    f.f.host.chat.remove(0);
    f.f.host.workers.remove(0);
    f.f.sync_sources();
    records.push(f.capture(
        dir,
        &format!("{name}-07-unavailable"),
        &["host-monitor-no-image", "host-monitor-unavailable"],
        false,
    ));
    let mut cycle = Fixture::new(logical, scale, 2);
    cycle.f.states[1].chat = true;
    cycle.f.model.play(100).unwrap();
    records.push(cycle.capture(
        dir,
        &format!("{name}-08-playing"),
        &["host-monitor-pause", "host-monitor-pause-chat"],
        false,
    ));
    records
}

#[test]
#[ignore = "explicit synthetic CPU visuals: set AURORA_MONITOR_VISUAL_DIR and AURORA_MONITOR_VISUAL_LANGUAGE"]
fn monitor_body_cpu_visual_fixture() {
    let dir = std::env::var_os("AURORA_MONITOR_VISUAL_DIR").expect("explicit output directory");
    let dir = Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();
    assert!(std::fs::read_dir(dir).unwrap().next().is_none(), "fresh output directory required");
    let language = std::env::var("AURORA_MONITOR_VISUAL_LANGUAGE").expect("explicit isolated language");
    assert!(matches!(language.as_str(), "en" | "fr" | "ja"));
    struct Restore(String);
    impl Drop for Restore {
        fn drop(&mut self) {
            aurora_i18n::select(&self.0, None);
        }
    }
    let _restore = Restore(aurora_i18n::language().to_owned());
    aurora_i18n::select(&language, None);
    let fallback = crate::ui::fonts::fallback::wait();
    if language == "ja" {
        assert!(
            fallback.iter().any(|face| face.has('あ') && face.has('界')),
            "Japanese captures require CJK fallback fonts"
        );
    }
    let mut records = layout(dir, "normal", [540.0, 580.0], 1.0);
    records.extend(layout(dir, "narrow", [380.0, 452.0], 1.5));
    records.extend(layout(dir, "short", [380.0, 420.0], 1.0));
    assert_eq!(records.len(), 24);
    std::fs::write(dir.join("manifest.json"),serde_json::to_vec_pretty(&json!({"scope":"production cfg(test)-only body, synthetic CPU image/chat; no native, IO, Windows, IME or GPU acceptance", "captures":records})).unwrap()).unwrap();
}
