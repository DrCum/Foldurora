//! Explicit captures of the production body through the software CPU painter.
//! Every identity, unread observation and transcript sentinel is synthetic.

use super::tests::{direct, host};
use super::*;
use crate::crash::software::Painter;
use crate::theme::Theme;
use crate::ui::fonts;
use aurora_session::chat::{ChatLine, ChatText, LineKind};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;

const PRIVATE: &str = "SYNTHETIC_TRANSCRIPT_MUST_NOT_APPEAR";
const PRIVATE_ERROR: &str = "SYNTHETIC_PRIVATE_PATH_MUST_NOT_APPEAR";
const SELECTED_TITLE: &str = "Synthetic selected conversation";

fn collect(shape: &egui::Shape, clip: Rect, texts: &mut Vec<(String, bool)>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, clip, texts);
            }
        }
        egui::Shape::Text(text) => {
            let bounds = Rect::from_min_size(text.pos, text.galley.size());
            if clip.intersects(bounds) && !text.galley.text().is_empty() {
                texts.push((text.galley.text().to_owned(), !text.galley.elided && clip.contains_rect(bounds)));
            }
        }
        egui::Shape::Callback(_) => panic!("Attention must remain CPU mesh-only"),
        _ => {}
    }
}

struct Fixture {
    body: AttentionUi,
    model: Attention,
    host: HostSnapshot,
    context: egui::Context,
    painter: Painter,
    pixels: Vec<u32>,
    texts: Vec<(String, bool)>,
    logical: [u32; 2],
    scale: f32,
    frame: u64,
}

impl Fixture {
    fn new(logical: [u32; 2], scale: f32) -> Self {
        let mut host = host();
        let chat = Arc::make_mut(&mut host.chat[0]);
        let info = chat
            .conversations
            .iter_mut()
            .find(|info| info.policy.key.conversation == direct())
            .unwrap();
        info.title = aurora_session::Label::new(SELECTED_TITLE).unwrap();
        chat.lines.push(ChatLine {
            key: info.policy.key,
            event: 1,
            policy_revision: info.policy.revision,
            sender: aurora_session::Label::new("Synthetic sender").unwrap(),
            text: ChatText::new(PRIVATE).unwrap(),
            kind: LineKind::Incoming,
            truncated: false,
        });
        let mut model = Attention::default();
        model.sync(&host, &host.attention_preferences.preferences);
        model.select(model.rows()[0].key);
        let context = egui::Context::default();
        Theme {
            panel_alpha: 255,
            ..Default::default()
        }
        .apply(&context, 1.0);
        context.all_styles_mut(|style| style.animation_time = 0.0);
        fonts::apply(&context, fonts::DEFAULT);
        Self {
            body: AttentionUi::default(),
            model,
            host,
            context,
            painter: Painter::default(),
            pixels: vec![0; (logical[0] as f32 * scale).round() as usize * (logical[1] as f32 * scale).round() as usize],
            texts: Vec::new(),
            logical,
            scale,
            frame: 0,
        }
    }

    fn show(&mut self) {
        self.events(Vec::new());
    }

    fn events(&mut self, events: Vec<Event>) {
        self.model.sync(&self.host, &self.host.attention_preferences.preferences);
        self.frame += 1;
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(self.logical[0] as f32, self.logical[1] as f32));
        let mut raw = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(self.frame as f64 / 60.0),
            focused: true,
            events,
            ..Default::default()
        };
        raw.viewports.entry(raw.viewport_id).or_default().native_pixels_per_point = Some(self.scale);
        let palette = Theme {
            panel_alpha: 255,
            ..Default::default()
        }
        .palette();
        let output = self.context.run_ui(raw, |ui| {
            assert!(self.body.show(ui, &palette, &mut self.model, &self.host, InputMode::Live).is_none());
        });
        assert!((output.pixels_per_point - self.scale).abs() < 0.001);
        self.texts.clear();
        for shape in &output.shapes {
            collect(&shape.shape, shape.clip_rect.intersect(screen), &mut self.texts);
        }
        let primitives = self.context.tessellate(output.shapes, output.pixels_per_point);
        assert!(
            primitives
                .iter()
                .all(|primitive| matches!(&primitive.primitive, egui::epaint::Primitive::Mesh(_)))
        );
        self.painter.paint(
            &mut self.pixels,
            self.logical.map(|size| (size as f32 * self.scale).round() as usize),
            &primitives,
            &output.textures_delta,
            output.pixels_per_point,
            palette.panel,
        );
    }

    fn toggle_filters(&mut self) {
        for _ in 0..3 {
            self.show();
        }
        let rect = self
            .context
            .data(|data| data.get_temp::<Rect>(Id::unique("attention_filter_toggle").with("rect")))
            .unwrap();
        for pressed in [true, false] {
            self.events(vec![
                Event::PointerMoved(rect.center()),
                Event::PointerButton {
                    pos: rect.center(),
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ]);
        }
    }

    fn scroll_filters_to_bottom(&mut self) {
        for _ in 0..3 {
            let clip = self
                .context
                .data(|data| data.get_temp::<Rect>(Id::unique("attention_filter_scroll")))
                .unwrap();
            self.events(vec![
                Event::PointerMoved(clip.center()),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                    delta: vec2(0.0, -1000.0),
                    modifiers: Modifiers::NONE,
                },
            ]);
            self.show();
        }
    }

    fn capture(&mut self, output: &Path, name: &str, required: &[&str], forbidden: &[&str]) -> Value {
        for _ in 0..4 {
            self.show();
        }
        let missing: Vec<_> = required
            .iter()
            .filter(|required| !self.texts.iter().any(|(text, complete)| text.contains(**required) && *complete))
            .collect();
        let leaked: Vec<_> = [PRIVATE, PRIVATE_ERROR]
            .into_iter()
            .chain(forbidden.iter().copied())
            .filter(|forbidden| self.texts.iter().any(|(text, _)| text.contains(*forbidden)))
            .collect();
        let physical = self.logical.map(|size| (size as f32 * self.scale).round() as u32);
        let rgba: Vec<_> = self
            .pixels
            .iter()
            .flat_map(|pixel| [(*pixel >> 16) as u8, (*pixel >> 8) as u8, *pixel as u8, 255])
            .collect();
        image::save_buffer(
            output.join(format!("{name}.png")),
            &rgba,
            physical[0],
            physical[1],
            image::ColorType::Rgba8,
        )
        .unwrap();
        let record = json!({
            "image": format!("{name}.png"), "locale": aurora_i18n::language(), "logical_size": self.logical,
            "physical_size": physical, "pixels_per_point": self.scale, "selected_key": self.model.selected(),
            "reminder_count": self.model.rows().len(), "published_preferences": self.host.attention_preferences.preferences,
            "filters_open": self.body.filters_open,
            "filter_viewport": self.body.filters_open.then(|| self.context.data(|data| data.get_temp::<Rect>(Id::unique("attention_filter_scroll"))).map(|rect| [rect.min.x, rect.min.y, rect.max.x, rect.max.y])),
            "filter_content_size": self.body.filters_open.then(|| self.context.data(|data| data.get_temp::<egui::Vec2>(Id::unique("attention_filter_content"))).map(|size| [size.x, size.y])),
            "missing_or_clipped_required_text": missing, "privacy_leaks": leaked,
            "painted_text": self.texts.iter().map(|(text, complete)| json!({"text": text, "fully_visible": complete})).collect::<Vec<_>>(),
        });
        std::fs::write(output.join(format!("{name}.json")), serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        assert!(missing.is_empty(), "{name}: missing/clipped text: {missing:?}");
        assert!(leaked.is_empty(), "{name}: private/revoked text: {leaked:?}");
        record
    }
}

fn layout(output: &Path, name: &str, logical: [u32; 2], scale: f32) -> Vec<Value> {
    let mut f = Fixture::new(logical, scale);
    let identity = aurora_i18n::tr!("host-chat-identity", account = "Synthetic Resident", grid = "Synthetic Grid");
    let mut records = vec![f.capture(
        output,
        &format!("{name}-01-selected"),
        &[
            aurora_i18n::tr!("host-attention-help"),
            aurora_i18n::tr!("host-attention-filters"),
            aurora_i18n::tr!("host-attention-open"),
            aurora_i18n::tr!("host-attention-dismiss"),
            &identity,
            SELECTED_TITLE,
            &aurora_i18n::tr!("host-attention-unread", count = 7_i64),
        ],
        &[],
    )];
    f.toggle_filters();
    let profiles: Vec<_> = SlotId::all()
        .map(|slot| {
            format!(
                "{} · Same profile",
                aurora_i18n::tr!("host-profile-default", number = i64::from(slot.number()))
            )
        })
        .collect();
    let mut top_required = vec![
        aurora_i18n::tr!("host-attention-filters"),
        aurora_i18n::tr!("host-attention-profiles"),
        aurora_i18n::tr!("host-attention-open"),
        aurora_i18n::tr!("host-attention-dismiss"),
        identity.as_str(),
    ];
    top_required.extend(profiles[..4].iter().map(String::as_str));
    records.push(f.capture(output, &format!("{name}-07-expanded-filters"), &top_required, &[]));
    // Narrow/localized captions can legitimately require scrolling. Capture
    // both ends through real wheel events, requiring every profile and both
    // categories fully visible at least once across these two scenes.
    f.scroll_filters_to_bottom();
    records.push(f.capture(
        output,
        &format!("{name}-08-expanded-filters-bottom"),
        &[
            &profiles[4],
            aurora_i18n::tr!("host-attention-nearby"),
            aurora_i18n::tr!("host-attention-direct"),
            aurora_i18n::tr!("host-attention-open"),
            aurora_i18n::tr!("host-attention-dismiss"),
            &identity,
        ],
        &[],
    ));
    f.toggle_filters();
    let desired = Preferences {
        nearby: true,
        ..Preferences::default()
    };
    f.body.queued(42, desired);
    records.push(f.capture(
        output,
        &format!("{name}-02-saving"),
        &[aurora_i18n::tr!("host-attention-saving"), &identity],
        &[],
    ));
    f.host.attention_preferences.operation = Some(super::super::attention_prefs::PreferenceOperation {
        ticket: 42,
        preferences: desired,
        error: Some(PRIVATE_ERROR.into()),
    });
    f.host.attention_preferences.error = Some(PRIVATE_ERROR.into());
    records.push(f.capture(
        output,
        &format!("{name}-03-save-failed"),
        &[aurora_i18n::tr!("host-attention-save-failed"), &identity],
        &[],
    ));
    // This exact selected source is revoked while the surface is closed.
    f.host.chat.remove(0);
    f.host.workers.remove(0);
    f.model.sync(&f.host, &f.host.attention_preferences.preferences);
    f.body.cancel_gestures();
    records.push(f.capture(
        output,
        &format!("{name}-04-selected-unavailable"),
        &[aurora_i18n::tr!("host-attention-unavailable")],
        &[SELECTED_TITLE],
    ));
    f.host.chat.clear();
    f.host.workers.clear();
    records.push(f.capture(
        output,
        &format!("{name}-05-empty"),
        &[aurora_i18n::tr!("host-attention-empty")],
        &[SELECTED_TITLE],
    ));
    f.host.attention_preferences = PreferenceState::default();
    f.host.attention_preferences.error = Some(PRIVATE_ERROR.into());
    records.push(f.capture(
        output,
        &format!("{name}-06-load-failed"),
        &[aurora_i18n::tr!("host-attention-load-failed")],
        &[SELECTED_TITLE, "Synthetic Resident"],
    ));
    records
}

fn short_layout(output: &Path) -> Vec<Value> {
    let mut f = Fixture::new([380, 420], 1.0);
    let mut records = vec![f.capture(
        output,
        "short-01-selected",
        &[
            aurora_i18n::tr!("host-attention-filters"),
            aurora_i18n::tr!("host-attention-open"),
            aurora_i18n::tr!("host-attention-dismiss"),
            SELECTED_TITLE,
        ],
        &[],
    )];
    f.toggle_filters();
    records.push(f.capture(
        output,
        "short-02-expanded-filters",
        &[
            aurora_i18n::tr!("host-attention-filters"),
            aurora_i18n::tr!("host-attention-profiles"),
            aurora_i18n::tr!("host-attention-open"),
            aurora_i18n::tr!("host-attention-dismiss"),
            SELECTED_TITLE,
        ],
        &[],
    ));
    records
}

#[test]
#[ignore = "explicit synthetic CPU capture; run alone with AURORA_ATTENTION_VISUAL_DIR and AURORA_ATTENTION_VISUAL_LANGUAGE"]
fn attention_cpu_visual_fixture() {
    let language = std::env::var("AURORA_ATTENTION_VISUAL_LANGUAGE").expect("choose en, fr or ja in an isolated test process");
    assert!(["en", "fr", "ja"].contains(&language.as_str()));
    let output = std::path::PathBuf::from(std::env::var_os("AURORA_ATTENTION_VISUAL_DIR").expect("use an empty disposable directory"));
    assert!(
        !output.exists() || std::fs::read_dir(&output).unwrap().next().is_none(),
        "preserve earlier captures"
    );
    std::fs::create_dir_all(&output).unwrap();
    struct RestoreLanguage(String);
    impl Drop for RestoreLanguage {
        fn drop(&mut self) {
            aurora_i18n::select(&self.0, None);
        }
    }
    let _restore = RestoreLanguage(aurora_i18n::language().to_owned());
    aurora_i18n::select(&language, None);
    let fallback = fonts::fallback::wait();
    if language == "ja" {
        assert!(
            fallback.iter().any(|face| face.has('あ') && face.has('界')),
            "Japanese capture requires CJK fallback fonts"
        );
    }
    let mut captures = layout(&output, "normal", [540, 580], 1.0);
    captures.extend(layout(&output, "narrow-150pct", [380, 580], 1.5));
    captures.extend(short_layout(&output));
    let manifest = json!({"fixture": "production AttentionUi + SoftwareWindow CPU painter, synthetic data", "locale": language, "native_window_acceptance": false, "network_or_grid": false, "no_transcript_or_draft_copy": true, "captures": captures});
    std::fs::write(output.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}
