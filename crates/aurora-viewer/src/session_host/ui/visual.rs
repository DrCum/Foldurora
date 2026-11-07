//! Explicit synthetic CPU captures of the production compact HostUi.
//! No native window, socket, profile, account file, world or GPU is created.

use super::*;
use crate::crash::software::Painter;
use crate::theme::Theme;
use crate::ui::{fonts, icons::Icons};
use aurora_session::chat::{ChatKey, ChatSource, Conversation, ConversationInfo, NativeChatPolicy};
use aurora_session::{
    Binding, FrameProof, GridKey, HostId, InstallationId, Label, SessionId, SessionIdentity, WorkerIdentity, WorkerStatus,
};
use egui::{Event, PointerButton, Pos2};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;

fn fixture_snapshot() -> HostSnapshot {
    let mut snapshot = HostSnapshot {
        can_activate: true,
        ..Default::default()
    };
    for (index, slot) in SlotId::all().enumerate() {
        let byte = index as u8;
        let name = format!("Synthetic Resident {}", index + 1);
        let binding = Binding {
            worker: WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).unwrap(),
                host: HostId::from_bytes([2; 16]).unwrap(),
                worker: WorkerId::from_bytes([10 + byte; 16]).unwrap(),
            },
            session: Some(SessionIdentity {
                session: SessionId::from_bytes([20 + byte; 16]).unwrap(),
                account: AccountKey::from_bytes([30 + byte; 32]).unwrap(),
                grid: GridKey::from_bytes([40 + byte; 32]).unwrap(),
                generation: 1,
            }),
        };
        snapshot.slots.push(SlotSummary {
            id: slot,
            label: SlotLabel::new(match index {
                0 => "Synthetic Alpha",
                1 => "Synthetic Beta",
                2 => "Synthetic Gamma",
                3 => "Synthetic Delta",
                _ => "WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW",
            })
            .unwrap(),
            available: false,
        });
        snapshot.workers.push(WorkerSummary {
            slot: Some(slot),
            binding,
            status: Some(WorkerStatus {
                phase: Phase::Online,
                name: Label::new(&name).unwrap(),
                grid_label: Label::new("Synthetic Grid").unwrap(),
                native_window: 100 + u64::from(byte), // Data only: never passed to a native API.
                keyboard_focused: index == 0,
                last_frame: Some(FrameProof {
                    device_epoch: 1,
                    surface_epoch: 1,
                    frame: 1,
                }),
                ..Default::default()
            }),
            reachable: true,
            label: name,
            grid: "Synthetic Grid".into(),
        });
        let source = ChatSource {
            worker: binding.worker,
            session: binding.session.unwrap(),
            lease_epoch: 1,
        };
        snapshot.chat.push(Arc::new(super::super::transport::HostChatSnapshot {
            source,
            authenticated: true,
            live: true,
            conversations: vec![ConversationInfo {
                policy: NativeChatPolicy {
                    key: ChatKey {
                        source,
                        conversation: Conversation::Nearby,
                    },
                    revision: 1,
                    lease_current: true,
                    ready: true,
                    can_read: true,
                    can_send: true,
                },
                title: Label::new("Synthetic Nearby").unwrap(),
            }],
            lines: Vec::new(),
            receipts: Vec::new(),
            gap: false,
        }));
    }
    snapshot.active = Some(snapshot.workers[0].binding.worker.worker);
    snapshot
}

fn rect_json(rect: Rect) -> Value {
    json!({"min": [rect.left(), rect.top()], "max": [rect.right(), rect.bottom()], "size": [rect.width(), rect.height()]})
}

struct Frames {
    context: egui::Context,
    painter: Painter,
    theme: Theme,
    _icons: Icons,
    host: HostUi,
    hosted: WorkerId,
    logical: [u32; 2],
    scale: f32,
    frame: u64,
    pixels: Vec<u32>,
    world: Rect,
    painted_text: Vec<Value>,
}

fn collect_text(shape: &egui::Shape, clip: Rect, text: &mut Vec<Value>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, clip, text);
            }
        }
        egui::Shape::Text(shape) => {
            let bounds = Rect::from_min_size(shape.pos, shape.galley.size());
            if clip.intersects(bounds) && !shape.galley.text().is_empty() {
                text.push(json!({
                    "text": shape.galley.text(),
                    "bounds": rect_json(bounds),
                    "clip": rect_json(clip),
                    "fully_visible": !shape.galley.elided && clip.contains_rect(bounds),
                }));
            }
        }
        egui::Shape::Callback(_) => panic!("the compact controller fixture must remain CPU mesh-only"),
        _ => {}
    }
}

impl Frames {
    fn new(snapshot: &HostSnapshot, logical: [u32; 2], scale: f32) -> Self {
        let context = egui::Context::default();
        let theme = Theme {
            panel_alpha: 255,
            ..Default::default()
        };
        theme.apply(&context, 1.0);
        context.all_styles_mut(|style| {
            style.animation_time = 0.0;
            style.visuals.popup_shadow = egui::epaint::Shadow::NONE;
        });
        fonts::apply(&context, fonts::DEFAULT);
        let icons = Icons::load(&context, None); // Embedded icons only; no skin path is read.
        let native = logical.map(|size| (size as f32 * scale).round() as usize);
        Self {
            context,
            painter: Painter::default(),
            theme,
            _icons: icons,
            host: HostUi::default(),
            hosted: snapshot.active.unwrap(),
            logical,
            scale,
            frame: 0,
            pixels: vec![0; native[0] * native[1]],
            world: Rect::NOTHING,
            painted_text: Vec::new(),
        }
    }

    fn show(&mut self, snapshot: &HostSnapshot, events: Vec<Event>) -> Option<Action> {
        self.frame += 1;
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(self.logical[0] as f32, self.logical[1] as f32));
        let mut raw = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(self.frame as f64 / 60.0),
            events,
            ..Default::default()
        };
        raw.viewports.entry(raw.viewport_id).or_default().native_pixels_per_point = Some(self.scale);
        let palette = self.theme.palette();
        let mut action = None;
        let output = self.context.run_ui(raw, |ui| {
            let shown = self.host.show(ui, &palette, snapshot, true, Some(self.hosted), false);
            self.world = shown.world;
            action = action.take().or(shown.action);
        });
        assert!((output.pixels_per_point - self.scale).abs() < 0.001);
        self.painted_text.clear();
        for shape in &output.shapes {
            collect_text(&shape.shape, shape.clip_rect.intersect(screen), &mut self.painted_text);
        }
        let primitives = self.context.tessellate(output.shapes, output.pixels_per_point);
        assert!(
            primitives
                .iter()
                .all(|primitive| matches!(primitive.primitive, egui::epaint::Primitive::Mesh(_)))
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

    fn settle(&mut self, snapshot: &HostSnapshot) {
        for _ in 0..4 {
            assert!(self.show(snapshot, Vec::new()).is_none());
        }
    }

    fn rect(&self, id: Id) -> Rect {
        self.context
            .data(|data| data.get_temp::<Rect>(id))
            .expect("actual production widget must be laid out")
    }

    fn click(&mut self, snapshot: &HostSnapshot, id: Id) -> Option<Action> {
        self.settle(snapshot);
        let at = self.rect(id).center();
        assert!(self.show(snapshot, vec![Event::PointerMoved(at)]).is_none());
        let mut action = None;
        for pressed in [true, false] {
            let next = self.show(
                snapshot,
                vec![Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert!(
                action.is_none() || next.is_none(),
                "one explicit click produces at most one typed action"
            );
            action = action.or(next);
        }
        action
    }

    fn capture(&mut self, directory: &Path, name: &str, snapshot: &HostSnapshot) -> Value {
        self.settle(snapshot);
        let canvas = self.context.content_rect();
        let tolerance = 1.0 / self.scale;
        assert!(self.world.is_positive() && canvas.expand(tolerance).contains_rect(self.world));
        assert!(self.world.top() - canvas.top() <= 40.0, "controller remains one compact row");
        if !self.host.management_open {
            assert!((self.world.left() - canvas.left()).abs() <= tolerance);
            assert!((self.world.right() - canvas.right()).abs() <= tolerance);
        }
        let controls: Vec<_> = [
            "host_slot_combo",
            "host_slot_open",
            "host_strip_activate",
            "host_strip_chat",
            "host_manage",
            "host_request_close",
        ]
        .into_iter()
        .map(|key| {
            let rect = self.rect(Id::unique(key));
            assert!(canvas.expand(tolerance).contains_rect(rect), "{key} escapes the controller canvas");
            assert!(rect.bottom() <= self.world.top() + tolerance, "{key} overlaps the world");
            json!({"key": key, "rect": rect_json(rect)})
        })
        .collect();
        let popup = self
            .context
            .data(|data| data.get_temp::<Id>(Id::unique("host_slot_popup_area")))
            .filter(|id| egui::Popup::is_id_open(&self.context, *id))
            .map(|id| self.context.memory(|memory| memory.area_rect(id)).expect("actual popup outer area"));
        if let Some(popup) = popup {
            assert!(canvas.expand(tolerance).contains_rect(popup));
            assert!(popup.width() <= self.rect(Id::unique("host_slot_combo")).width() + tolerance);
            assert!(
                popup.intersects(self.world),
                "popup intentionally overlays the synthetic world rectangle"
            );
        }
        let overlay_id = if self.host.rename.is_some() {
            Some(Id::unique("host_profile_rename"))
        } else if self.host.close_prompt {
            Some(Id::unique("session_host_close"))
        } else {
            None
        };
        let modal = overlay_id.map(|id| self.context.memory(|memory| memory.area_rect(id)).expect("actual modal outer area"));
        if let Some(modal) = modal {
            assert!(canvas.expand(tolerance).contains_rect(modal));
            assert!(modal.intersects(self.world));
        }
        let tabs: Vec<_> = snapshot.workers.iter().map(|worker| {
            let id = worker.binding.worker.worker;
            let active = self.context.data(|data| data.get_temp::<bool>(Id::unique(("host_tab_active", id))))
                .expect("actual tab active state");
            assert_eq!(active, id == self.hosted, "active highlight follows only the acknowledged hosted worker");
            json!({"binding": worker.binding, "slot": worker.slot, "active": active, "rect": rect_json(self.rect(Id::unique(("host_tab", id))))})
        }).collect();
        let size = self.logical.map(|size| (size as f32 * self.scale).round() as u32);
        let rgba: Vec<_> = self
            .pixels
            .iter()
            .flat_map(|pixel| [(*pixel >> 16) as u8, (*pixel >> 8) as u8, *pixel as u8, 255])
            .collect();
        let image = format!("{name}.png");
        image::save_buffer(directory.join(&image), &rgba, size[0], size[1], image::ColorType::Rgba8).unwrap();
        let record = json!({
            "image": image, "locale": aurora_i18n::language(), "synthetic_only": true, "native_acceptance": false,
            "logical_size": self.logical, "physical_size": size, "pixels_per_point": self.scale,
            "canvas": rect_json(canvas), "world": rect_json(self.world), "management_open": self.host.management_open,
            "modal_open": self.host.modal_open(), "hosted_worker": self.hosted, "selected_worker": self.host.selected,
            "selected_slot": self.host.selected_slot, "controls": controls, "tabs": tabs,
            "tabs_clip": rect_json(self.rect(Id::unique("host_tabs_clip"))),
            "profile_popup_outer": popup.map(rect_json), "modal_outer": modal.map(rect_json), "painted_text": self.painted_text,
        });
        std::fs::write(directory.join(format!("{name}.json")), serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        record
    }
}

fn wide_scenes(directory: &Path, snapshot: &HostSnapshot) -> Vec<Value> {
    let mut frames = Frames::new(snapshot, [1240, 800], 1.0);
    let mut records = vec![frames.capture(directory, "01-wide-five-workers", snapshot)];
    assert!(frames.click(snapshot, Id::unique("host_manage")).is_none());
    assert!(frames.host.management_open);
    records.push(frames.capture(directory, "02-management-open", snapshot));
    assert!(frames.click(snapshot, Id::unique("host_manage")).is_none());
    assert!(!frames.host.management_open);
    assert!(frames.click(snapshot, Id::unique("host_slot_combo")).is_none());
    records.push(frames.capture(directory, "03-profile-dropdown", snapshot));
    let long_label_worker = &snapshot.workers[4];
    assert_eq!(
        frames.click(snapshot, Id::unique(("host_slot_pick", long_label_worker.slot.unwrap()))),
        Some(Action::Activate(long_label_worker.binding.worker.worker))
    );
    frames.settle(snapshot);
    assert_eq!(frames.host.selected, Some(long_label_worker.binding.worker.worker));
    assert_eq!(
        frames.hosted, snapshot.workers[0].binding.worker.worker,
        "typed request alone is never an activation acknowledgement"
    );
    assert!(frames.click(snapshot, Id::unique("host_manage")).is_none());
    assert!(frames.host.management_open);
    assert!(frames.click(snapshot, Id::unique("host_slot_rename")).is_none());
    assert!(frames.host.rename.is_some());
    records.push(frames.capture(directory, "04-profile-rename-dialog", snapshot));
    assert!(frames.click(snapshot, Id::unique("host_slot_cancel")).is_none());
    assert!(!frames.host.modal_open());
    assert!(frames.click(snapshot, Id::unique("host_manage")).is_none());
    assert!(!frames.host.management_open);
    assert!(frames.click(snapshot, Id::unique("host_request_close")).is_none());
    assert!(frames.host.close_prompt);
    records.push(frames.capture(directory, "05-close-confirmation", snapshot));
    assert!(frames.click(snapshot, Id::unique("host_cancel_close")).is_none());
    assert!(!frames.host.modal_open());
    records
}

#[test]
#[ignore = "explicit isolated CPU visual capture; set AURORA_COMPACT_HOST_VISUAL_DIR and AURORA_COMPACT_HOST_VISUAL_LANGUAGE"]
fn compact_host_cpu_visual_fixture() {
    let language = std::env::var("AURORA_COMPACT_HOST_VISUAL_LANGUAGE").expect("select exactly en, fr or ja in this isolated test process");
    assert!(["en", "fr", "ja"].contains(&language.as_str()));
    let directory =
        std::path::PathBuf::from(std::env::var_os("AURORA_COMPACT_HOST_VISUAL_DIR").expect("set an empty disposable capture directory"));
    assert!(
        !directory.exists() || std::fs::read_dir(&directory).unwrap().next().is_none(),
        "preserve earlier captures; use an empty directory"
    );
    std::fs::create_dir_all(&directory).unwrap();
    struct RestoreLanguage(String);
    impl Drop for RestoreLanguage {
        fn drop(&mut self) {
            aurora_i18n::select(&self.0, None);
        }
    }
    let _restore = RestoreLanguage(aurora_i18n::language().to_owned());
    aurora_i18n::select(&language, None);
    assert_eq!(aurora_i18n::language(), language);
    let fallback = fonts::fallback::wait();
    if language == "ja" {
        assert!(
            fallback.iter().any(|face| face.has('あ') && face.has('界')),
            "Japanese captures require an installed CJK fallback font"
        );
    }
    let snapshot = fixture_snapshot();
    let mut records = wide_scenes(&directory, &snapshot);
    // Finish/drop the wide context before loading the next context's global
    // embedded icons. Never mix two contexts' icon texture handles.
    let mut narrow = Frames::new(&snapshot, [540, 680], 1.5);
    records.push(narrow.capture(&directory, "06-narrow-150pct-five-workers", &snapshot));
    assert!(narrow.click(&snapshot, Id::unique("host_slot_combo")).is_none());
    records.push(narrow.capture(&directory, "07-narrow-150pct-profile-dropdown", &snapshot));
    let manifest = json!({
        "fixture": "production HostUi + SoftwareWindow CPU Painter + embedded icons; synthetic roster only",
        "locale": language, "native_window_acceptance": false, "native_embedding_or_input_acceptance": false,
        "renderer_or_gpu": false, "network_or_grid": false, "profiles_or_account_files": false,
        "actions": "real widget clicks return typed intents only; no transport or native dispatch",
        "captures": records,
    });
    std::fs::write(directory.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    println!("CPU compact-host visual captures: {}", directory.display());
}
