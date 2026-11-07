//! Explicit production SharedChat + CPU-painter captures. All identities,
//! policies, receipts and text are synthetic; no native window or IO is used.

use super::super::chat_broker::{ReadOperation, ReadStatus};
use super::tests::{host, source};
use super::*;
use crate::crash::software::Painter;
use crate::theme::Theme;
use crate::ui::fonts;
use aurora_session::chat::{Conversation, ObservedIncoming, ReadMetadata, ReadOutcome, SendOutcome, SendReceipt};
use egui::{Event, PointerButton, Pos2, Rect};
use serde_json::{Value, json};
use std::path::Path;

const PRIVATE_DRAFT: &str = "SYNTHETIC_PRIVATE_DRAFT_MUST_BE_PURGED";

fn collect_text(shape: &egui::Shape, clip: Rect, texts: &mut Vec<(String, bool)>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, clip, texts);
            }
        }
        egui::Shape::Text(text) => {
            let bounds = Rect::from_min_size(text.pos, text.galley.size());
            if clip.intersects(bounds) && !text.galley.text().is_empty() {
                texts.push((text.galley.text().to_owned(), !text.galley.elided && clip.contains_rect(bounds)));
            }
        }
        egui::Shape::Callback(_) => panic!("shared chat must remain CPU mesh-only"),
        _ => {}
    }
}

struct Fixture {
    model: SharedChat,
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
        Arc::make_mut(&mut host.chat[0]).reads.insert(
            Conversation::Nearby,
            ReadMetadata {
                unread: 7,
                unread_revision: 1,
                observed: ObservedIncoming { epoch: 1, through: 7 },
                can_mark_read: true,
            },
        );
        let mut model = SharedChat::default();
        model.sync(&host, false);
        model.select(source(1));
        let mut fixture = Self {
            model,
            host,
            context: egui::Context::default(),
            painter: Painter::default(),
            pixels: vec![0; (logical[0] as f32 * scale).round() as usize * (logical[1] as f32 * scale).round() as usize],
            texts: Vec::new(),
            logical,
            scale,
            frame: 0,
        };
        fixture.prepare_context();
        fixture
    }

    fn prepare_context(&mut self) {
        let theme = Theme {
            panel_alpha: 255,
            ..Default::default()
        };
        theme.apply(&self.context, 1.0);
        self.context.all_styles_mut(|style| style.animation_time = 0.0);
        fonts::apply(&self.context, fonts::DEFAULT);
    }

    fn show(&mut self, events: Vec<Event>) -> Option<SendDraft> {
        self.model.sync(&self.host, false);
        self.frame += 1;
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(self.logical[0] as f32, self.logical[1] as f32));
        let mut raw = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(self.frame as f64 / 60.0),
            events,
            focused: true,
            ..Default::default()
        };
        raw.viewports.entry(raw.viewport_id).or_default().native_pixels_per_point = Some(self.scale);
        let palette = Theme {
            panel_alpha: 255,
            ..Default::default()
        }
        .palette();
        let mut action = None;
        let output = self.context.run_ui(raw, |ui| {
            action = action.take().or(self.model.show(ui, &palette, InputMode::Live));
        });
        assert!((output.pixels_per_point - self.scale).abs() < 0.001);
        self.texts.clear();
        for shape in &output.shapes {
            collect_text(&shape.shape, shape.clip_rect.intersect(screen), &mut self.texts);
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
        action
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            assert!(self.show(Vec::new()).is_none());
        }
    }

    fn click(&mut self, id: Id) -> Option<SendDraft> {
        self.settle();
        let rect = self
            .context
            .data(|data| data.get_temp::<Rect>(id))
            .expect("actual shared-chat widget");
        let mut result = None;
        for pressed in [true, false] {
            let action = self.show(vec![
                Event::PointerMoved(rect.center()),
                Event::PointerButton {
                    pos: rect.center(),
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            result = result.or(action);
        }
        result
    }

    fn type_text(&mut self, text: &str) {
        assert!(self.click(Id::unique("host_chat_input_rect")).is_none());
        assert!(self.show(vec![Event::Text(text.to_owned())]).is_none());
    }

    fn capture(&mut self, output: &Path, name: &str, required: &[&str], forbidden: &[&str]) -> Value {
        self.settle();
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(self.logical[0] as f32, self.logical[1] as f32));
        let combo = self
            .context
            .data(|data| data.get_temp::<Rect>(Id::unique("shared_chat_combo_rect")))
            .unwrap();
        let popup_id = self
            .context
            .data(|data| data.get_temp::<Id>(Id::unique("shared_chat_popup")))
            .unwrap();
        let popup =
            egui::Popup::is_id_open(&self.context, popup_id).then(|| self.context.memory(|memory| memory.area_rect(popup_id)).unwrap());
        let missing: Vec<_> = required
            .iter()
            .filter(|required| !self.texts.iter().any(|(text, complete)| text.as_str() == **required && *complete))
            .collect();
        let leaked: Vec<_> = forbidden
            .iter()
            .filter(|forbidden| self.texts.iter().any(|(text, _)| text.contains(**forbidden)))
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
            "physical_size": physical, "pixels_per_point": self.scale, "selected_source": self.model.selected,
            "retained_sources": self.model.states.len(), "source_selector": [combo.min.x, combo.min.y, combo.max.x, combo.max.y],
            "popup": popup.map(|rect| [rect.min.x, rect.min.y, rect.max.x, rect.max.y]),
            "missing_or_clipped_required_text": missing, "revoked_text": leaked,
            "painted_text": self.texts.iter().map(|(text, complete)| json!({"text": text, "fully_visible": complete})).collect::<Vec<_>>(),
        });
        std::fs::write(output.join(format!("{name}.json")), serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        assert!(
            canvas.expand(1.0).contains_rect(combo),
            "source selector must fit the actual native canvas"
        );
        if let Some(popup) = popup {
            assert!(canvas.expand(1.0).contains_rect(popup), "actual popup outer frame must fit");
        }
        assert!(missing.is_empty(), "{name}: missing/clipped labels: {missing:?}");
        assert!(leaked.is_empty(), "{name}: revoked text was painted: {leaked:?}");
        record
    }
}

fn layout(output: &Path, name: &str, logical: [u32; 2], scale: f32) -> Vec<Value> {
    let mut fixture = Fixture::new(logical, scale);
    let identity = aurora_i18n::tr!("host-chat-identity", account = "Same Resident", grid = "Same Grid");
    let mut records = vec![fixture.capture(
        output,
        &format!("{name}-01-selected-identity"),
        &[
            aurora_i18n::tr!("host-chat-send-as"),
            &identity,
            aurora_i18n::tr!("host-chat-compose-help"),
        ],
        &[],
    )];
    fixture.click(Id::unique("shared_chat_combo_rect"));
    records.push(fixture.capture(
        output,
        &format!("{name}-02-five-duplicate-labels"),
        &[aurora_i18n::tr!("host-chat-send-as"), &identity],
        &[],
    ));
    egui::Popup::close_all(&fixture.context);
    fixture.type_text("Synthetic explicit pending send");
    let sent = fixture
        .click(Id::unique("host_chat_send_rect"))
        .expect("one explicit typed intent; no transport is installed");
    fixture.model.queued(&sent, 42);
    Arc::make_mut(&mut fixture.host.chat[0]).receipts.push(SendReceipt {
        key: sent.key,
        request: 42,
        outcome: SendOutcome::Pending,
    });
    records.push(fixture.capture(
        output,
        &format!("{name}-03-pending"),
        &[&identity, aurora_i18n::tr!("host-chat-queued")],
        &[],
    ));
    Arc::make_mut(&mut fixture.host.chat[0]).receipts[0].outcome = SendOutcome::AcceptedLocally;
    fixture.settle();
    fixture.type_text(PRIVATE_DRAFT);
    assert!(fixture.click(Id::unique("host_chat_read_rect")).is_none());
    let read = fixture
        .model
        .take_read_action()
        .expect("explicit captured Read; no native transport installed");
    fixture.model.queued_read(read, 80);
    Arc::make_mut(&mut fixture.host.chat[0]).read_operations.push(ReadOperation {
        request: read.request(80),
        status: ReadStatus::Pending,
    });
    records.push(fixture.capture(
        output,
        &format!("{name}-06-read-pending"),
        &[&identity, aurora_i18n::tr!("host-chat-read-pending")],
        &[],
    ));
    let chat = Arc::make_mut(&mut fixture.host.chat[0]);
    chat.read_operations[0].status = ReadStatus::Completed(ReadOutcome::StaleObservation);
    chat.reads.insert(
        Conversation::Nearby,
        ReadMetadata {
            unread: 8,
            unread_revision: 2,
            observed: ObservedIncoming { epoch: 1, through: 8 },
            can_mark_read: true,
        },
    );
    records.push(fixture.capture(
        output,
        &format!("{name}-07-read-stale"),
        &[&identity, aurora_i18n::tr!("host-chat-read-stale")],
        &[],
    ));
    assert!(fixture.click(Id::unique("host_chat_read_rect")).is_none());
    let read = fixture
        .model
        .take_read_action()
        .expect("new explicit action after checking current observation");
    fixture.model.queued_read(read, 81);
    Arc::make_mut(&mut fixture.host.chat[0]).read_operations.push(ReadOperation {
        request: read.request(81),
        status: ReadStatus::Uncertain,
    });
    records.push(fixture.capture(
        output,
        &format!("{name}-08-read-uncertain"),
        &[&identity, aurora_i18n::tr!("host-chat-read-uncertain")],
        &[],
    ));
    let chat = Arc::make_mut(&mut fixture.host.chat[0]);
    chat.conversations[0].policy.revision += 1;
    chat.conversations[0].policy.can_send = false;
    chat.conversations[0].policy.can_read = false;
    fixture.model.sync(&fixture.host, false); // closed/minimized: no UI pass
    fixture.context = egui::Context::default();
    fixture.painter = Painter::default();
    fixture.prepare_context();
    fixture.model.cancel_gestures();
    records.push(fixture.capture(
        output,
        &format!("{name}-04-policy-reopened"),
        &[&identity, aurora_i18n::tr!("host-chat-restricted")],
        &[PRIVATE_DRAFT],
    ));
    fixture.host.chat.remove(0);
    fixture.host.workers.remove(0);
    fixture.model.sync(&fixture.host, false);
    records.push(fixture.capture(
        output,
        &format!("{name}-05-selected-unavailable"),
        &[aurora_i18n::tr!("host-chat-source-unavailable")],
        &[PRIVATE_DRAFT],
    ));
    records
}

#[test]
#[ignore = "explicit synthetic CPU capture; run alone with AURORA_SHARED_CHAT_VISUAL_DIR and AURORA_SHARED_CHAT_VISUAL_LANGUAGE"]
fn shared_chat_cpu_visual_fixture() {
    let language = std::env::var("AURORA_SHARED_CHAT_VISUAL_LANGUAGE").expect("choose en, fr or ja in an isolated test process");
    assert!(["en", "fr", "ja"].contains(&language.as_str()));
    let output = std::path::PathBuf::from(std::env::var_os("AURORA_SHARED_CHAT_VISUAL_DIR").expect("use an empty disposable directory"));
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
    let mut captures = layout(&output, "normal", [680, 680], 1.0);
    captures.extend(layout(&output, "narrow-150pct", [380, 680], 1.5));
    let manifest = json!({"fixture": "production SharedChat + SoftwareWindow CPU painter, synthetic data", "locale": language,
        "native_window_acceptance": false, "network_or_grid": false, "captures": captures});
    std::fs::write(output.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}
