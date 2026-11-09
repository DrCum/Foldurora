//! Isolated CPU fixtures of production widgets. No App, native window or IO owner.
use crate::crash::software::Painter;
use crate::theme::{Palette, Theme};
use crate::ui::{fonts, icons::Icons};
use egui::{Event, Pos2, Rect};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(crate) struct Environment {
    pub directory: PathBuf,
    previous_language: String,
}
impl Environment {
    pub fn new(scene: &str) -> Self {
        let language = std::env::var("AURORA_MCP_VISUAL_LANGUAGE").expect("select en, fr or ja for this isolated process");
        assert!(["en", "fr", "ja"].contains(&language.as_str()));
        let directory =
            PathBuf::from(std::env::var_os("AURORA_MCP_VISUAL_DIR").expect("set a fresh disposable CPU capture directory")).join(scene);
        assert!(
            !directory.exists() || std::fs::read_dir(&directory).unwrap().next().is_none(),
            "preserve previous captures"
        );
        std::fs::create_dir_all(&directory).unwrap();
        let previous_language = aurora_i18n::language().to_owned();
        aurora_i18n::select(&language, None);
        let fallback = fonts::fallback::wait();
        if language == "ja" {
            assert!(
                fallback.iter().any(|face| face.has('あ') && face.has('界')),
                "Japanese fixtures need a CJK fallback font"
            );
        }
        Self {
            directory,
            previous_language,
        }
    }
}
impl Drop for Environment {
    fn drop(&mut self) {
        aurora_i18n::select(&self.previous_language, None);
    }
}

pub(crate) fn rect_json(rect: Rect) -> Value {
    json!({"min":[rect.left(),rect.top()],"max":[rect.right(),rect.bottom()]})
}

pub(crate) struct Frames {
    pub context: egui::Context,
    pub palette: Palette,
    pub icons: Icons,
    painter: Painter,
    size: [u32; 2],
    frame: u64,
    pixels: Vec<u32>,
    pub text: Vec<Value>,
}
fn collect(shape: &egui::Shape, clip: Rect, text: &mut Vec<Value>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, clip, text);
            }
        }
        egui::Shape::Text(shape) => {
            let bounds = Rect::from_min_size(shape.pos, shape.galley.size());
            if clip.intersects(bounds) && !shape.galley.text().is_empty() {
                let tail = shape.galley.text().find("MCP_FULL_TAIL_SENTINEL").map(|byte| {
                    let start = shape.galley.text()[..byte].chars().count();
                    let end = start + "MCP_FULL_TAIL_SENTINEL".chars().count();
                    let a = shape.galley.pos_from_cursor(egui::text::CCursor::new(start));
                    let b = shape.galley.pos_from_cursor(egui::text::CCursor::new(end));
                    clip.contains_rect(a.union(b).translate(shape.pos.to_vec2()))
                });
                text.push(json!({"text":shape.galley.text(),"bounds":rect_json(bounds),"clip":rect_json(clip),"fully_visible":!shape.galley.elided && clip.contains_rect(bounds),"review_tail_visible":tail}));
            }
        }
        egui::Shape::Callback(_) => panic!("native permission fixtures must remain CPU mesh-only"),
        _ => {}
    }
}
impl Frames {
    pub fn new(size: [u32; 2]) -> Self {
        let context = egui::Context::default();
        let theme = Theme {
            panel_alpha: 255,
            ..Default::default()
        };
        theme.apply(&context, 1.0);
        context.all_styles_mut(|style| {
            style.animation_time = 0.;
            style.visuals.popup_shadow = egui::epaint::Shadow::NONE;
        });
        fonts::apply(&context, fonts::DEFAULT);
        let icons = Icons::load(&context, None);
        Self {
            context,
            palette: theme.palette(),
            icons,
            painter: Painter::default(),
            size,
            frame: 0,
            pixels: vec![0; size[0] as usize * size[1] as usize],
            text: Vec::new(),
        }
    }
    pub fn draw<R>(&mut self, events: Vec<Event>, mut render: impl FnMut(&mut egui::Ui, &Palette, &Icons) -> R) -> R {
        self.frame += 1;
        let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(self.size[0] as f32, self.size[1] as f32));
        let mut result = None;
        let mut output = self.context.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                time: Some(self.frame as f64 / 10.),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| {
                result = Some(render(ui, &self.palette, &self.icons));
            },
        );
        self.text.clear();
        for shape in &output.shapes {
            collect(&shape.shape, shape.clip_rect.intersect(screen), &mut self.text);
        }
        let primitives = self.context.tessellate(output.shapes, output.pixels_per_point);
        assert!(primitives.iter().all(|p| matches!(p.primitive, egui::epaint::Primitive::Mesh(_))));
        self.painter.paint(
            &mut self.pixels,
            self.size.map(|x| x as usize),
            &primitives,
            &output.textures_delta,
            output.pixels_per_point,
            self.palette.panel,
        );
        output.textures_delta.clear();
        result.unwrap()
    }
    pub fn rect(&self, id: egui::Id) -> Rect {
        self.context
            .data(|data| data.get_temp(id))
            .expect("production widget must be laid out")
    }
    pub fn has_text(&self, needle: &str) -> bool {
        self.text.iter().any(|t| t["text"].as_str().is_some_and(|s| s.contains(needle)))
    }
    pub fn text_center(&self, label: &str) -> Pos2 {
        let text = self
            .text
            .iter()
            .find(|t| t["text"] == label && t["fully_visible"] == true)
            .expect("native button label must be fully visible");
        let min = &text["bounds"]["min"];
        let max = &text["bounds"]["max"];
        egui::pos2(
            ((min[0].as_f64().unwrap() + max[0].as_f64().unwrap()) / 2.) as f32,
            ((min[1].as_f64().unwrap() + max[1].as_f64().unwrap()) / 2.) as f32,
        )
    }
    pub fn capture(&self, directory: &Path, name: &str, state: Value) {
        let rgba: Vec<_> = self
            .pixels
            .iter()
            .flat_map(|p| [(*p >> 16) as u8, (*p >> 8) as u8, *p as u8, 255])
            .collect();
        image::save_buffer(
            directory.join(format!("{name}.png")),
            &rgba,
            self.size[0],
            self.size[1],
            image::ColorType::Rgba8,
        )
        .unwrap();
        let record = json!({"image":format!("{name}.png"),"synthetic_only":true,"production_widgets":true,"renderer":"CPU Painter","native_window_acceptance":false,"network_or_grid":false,"profiles_or_account_files":false,"locale":aurora_i18n::language(),"size":self.size,"painted_text":self.text,"state":state});
        std::fs::write(directory.join(format!("{name}.json")), serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    }
}
pub(crate) fn pointer(at: Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(at),
        Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}
pub(crate) fn enter(pressed: bool) -> Vec<Event> {
    vec![Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]
}
