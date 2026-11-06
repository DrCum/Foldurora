//! Explicit, synthetic CPU captures of the production popout body. No native
//! window, controller, socket, profile, world or graphics adapter is created.

use super::*;
use crate::crash::software::Painter;
use crate::theme::Theme;
use crate::ui::fonts;
use aurora_session::chat::{ChatLine, ConversationInfo, LineKind};
use aurora_session::{
    AccountKey, ConversationId, GridKey, HostId, InstallationId, Label, SessionId, SessionIdentity, WorkerId, WorkerIdentity,
};
use egui::{Event, PointerButton, Pos2, Rect};
use serde_json::{Value, json};
use std::path::Path;

const ACCOUNT: &str = "Synthetic Alpha";
const GRID: &str = "Synthetic Grid";
const RESIDENT: &str = "Synthetic Resident";
const NEARBY_SENDER: &str = "Synthetic Nearby Sender";
const DIRECT_TEXT: &str = "Synthetic inactive-character IM. A literal /ao off stays ordinary text.";
const SOURCE_DRAFT: &str = "SYNTHETIC_SOURCE_DRAFT_MUST_BE_PURGED";
const PRIVACY_DRAFT: &str = "SYNTHETIC_PRIVACY_DRAFT_MUST_BE_PURGED";
const REPLACEMENT_TEXT: &str = "SYNTHETIC_REPLACEMENT_TEXT_MUST_NOT_BE_DISPLAYED";

fn source() -> ChatSource {
    ChatSource {
        worker: WorkerIdentity {
            installation: InstallationId::from_bytes([1; 32]).unwrap(),
            host: HostId::from_bytes([2; 16]).unwrap(),
            worker: WorkerId::from_bytes([3; 16]).unwrap(),
        },
        session: SessionIdentity {
            session: SessionId::from_bytes([4; 16]).unwrap(),
            account: AccountKey::from_bytes([5; 32]).unwrap(),
            grid: GridKey::from_bytes([6; 32]).unwrap(),
            generation: 7,
        },
        lease_epoch: 8,
    }
}

fn direct() -> Conversation {
    Conversation::Direct(ConversationId::from_bytes([11; 16]).unwrap())
}

fn snapshot(source: ChatSource) -> HostChatSnapshot {
    let nearby = aurora_i18n::tr!("common-nearby-chat");
    let conversations: Vec<_> = [(Conversation::Nearby, nearby), (direct(), RESIDENT)]
        .into_iter()
        .enumerate()
        .map(|(index, (conversation, title))| ConversationInfo {
            policy: NativeChatPolicy {
                key: ChatKey { source, conversation },
                revision: index as u64 + 1,
                lease_current: true,
                ready: true,
                can_send: true,
                can_read: true,
            },
            title: Label::new(title).unwrap(),
        })
        .collect();
    let lines = [
        (
            0,
            NEARBY_SENDER,
            "Synthetic nearby: Bonjour, こんにちは. This is offline fixture content.",
        ),
        (1, RESIDENT, DIRECT_TEXT),
    ]
    .into_iter()
    .enumerate()
    .map(|(event, (index, sender, text))| ChatLine {
        key: conversations[index].policy.key,
        event: event as u64 + 1,
        policy_revision: conversations[index].policy.revision,
        sender: Label::new(sender).unwrap(),
        text: ChatText::new(text).unwrap(),
        kind: LineKind::Incoming,
        truncated: false,
    })
    .collect();
    HostChatSnapshot {
        source,
        authenticated: true,
        live: true,
        conversations,
        lines,
        receipts: Vec::new(),
        gap: false,
    }
}

struct PaintedText {
    text: String,
    complete: bool,
}

fn visible_text(shape: &egui::Shape, clip: Rect, texts: &mut Vec<PaintedText>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                visible_text(shape, clip, texts);
            }
        }
        egui::Shape::Text(text) => {
            let bounds = Rect::from_min_size(text.pos, text.galley.size());
            if clip.intersects(bounds) && !text.galley.text().is_empty() {
                texts.push(PaintedText {
                    text: text.galley.text().to_owned(),
                    complete: !text.galley.elided && clip.contains_rect(bounds),
                });
            }
        }
        egui::Shape::Callback(_) => panic!("the CPU chat fixture must not require a graphics callback"),
        _ => {}
    }
}

struct Frames {
    context: egui::Context,
    painter: Painter,
    theme: Theme,
    chat: ChatUi,
    logical: [u32; 2],
    scale: f32,
    frame: u64,
    pixels: Vec<u32>,
    texts: Vec<PaintedText>,
}

impl Frames {
    fn new(logical: [u32; 2], scale: f32) -> Self {
        let context = egui::Context::default();
        let theme = Theme {
            panel_alpha: 255,
            ..Default::default()
        };
        theme.apply(&context, 1.0);
        context.all_styles_mut(|style| style.animation_time = 0.0);
        fonts::apply(&context, fonts::DEFAULT);
        let native = logical.map(|size| (size as f32 * scale).round() as usize);
        Self {
            context,
            painter: Painter::default(),
            theme,
            chat: ChatUi::new(source()),
            logical,
            scale,
            frame: 0,
            pixels: vec![0; native[0] * native[1]],
            texts: Vec::new(),
        }
    }

    fn show(&mut self, snapshot: &HostChatSnapshot, events: Vec<Event>) -> Option<SendDraft> {
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
            let shown = self.chat.show(ui, &palette, Some(snapshot), ACCOUNT, GRID);
            action = action.take().or(shown);
        });
        assert!((output.pixels_per_point - self.scale).abs() < 0.001);
        self.texts.clear();
        for shape in &output.shapes {
            visible_text(&shape.shape, shape.clip_rect.intersect(screen), &mut self.texts);
        }
        let primitives = self.context.tessellate(output.shapes, output.pixels_per_point);
        assert!(
            primitives
                .iter()
                .all(|primitive| matches!(primitive.primitive, egui::epaint::Primitive::Mesh(_)))
        );
        // Same painter, texture uploads and native 0x00RRGGBB pixel format as
        // SoftwareWindow::present, without requiring an OS window or display.
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

    fn settle(&mut self, snapshot: &HostChatSnapshot) {
        for _ in 0..3 {
            assert!(self.show(snapshot, Vec::new()).is_none());
        }
    }

    fn click(&mut self, snapshot: &HostChatSnapshot, id: Id) -> Option<SendDraft> {
        self.settle(snapshot);
        let rect = self
            .context
            .data(|data| data.get_temp::<Rect>(id))
            .expect("actual chat widget was laid out");
        assert!(self.show(snapshot, vec![Event::PointerMoved(rect.center())]).is_none());
        let mut action = None;
        for pressed in [true, false] {
            let shown = self.show(
                snapshot,
                vec![Event::PointerButton {
                    pos: rect.center(),
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert!(action.is_none() || shown.is_none(), "one click cannot produce two sends");
            action = action.or(shown);
        }
        action
    }

    fn select_direct(&mut self, snapshot: &HostChatSnapshot) {
        assert!(self.click(snapshot, Id::unique("host_chat_combo_rect")).is_none());
        assert!(self.click(snapshot, Id::unique(("host_chat_pick", direct()))).is_none());
        self.settle(snapshot);
        assert_eq!(self.chat.key(), snapshot.conversations[1].policy.key);
    }

    fn type_text(&mut self, snapshot: &HostChatSnapshot, text: &str) {
        assert!(self.click(snapshot, Id::unique("host_chat_input_rect")).is_none());
        assert!(self.show(snapshot, vec![Event::Text(text.to_owned())]).is_none());
        assert_eq!(self.chat.draft, text);
    }

    fn capture(
        &mut self,
        output: &Path,
        name: &str,
        snapshot: &HostChatSnapshot,
        title: &str,
        senders: &[&str],
        forbidden: &[&str],
    ) -> Value {
        self.settle(snapshot);
        assert_eq!(
            self.chat.source(),
            source(),
            "the immutable account/grid/session must never follow another snapshot"
        );
        let identity = aurora_i18n::tr!("host-chat-identity", account = ACCOUNT, grid = GRID);
        let expected_history: Vec<_> = self
            .chat
            .policy
            .into_iter()
            .filter(|policy| policy.can_read && policy.ready && policy.lease_current)
            .flat_map(|policy| {
                snapshot
                    .lines
                    .iter()
                    .filter(move |line| line.key == policy.key && line.policy_revision == policy.revision)
            })
            .map(|line| line.text.as_str())
            .collect();
        let receipt_text = self.chat.receipt.map(|receipt| outcome_text(receipt.outcome));
        let warning_text = self.chat.warning.map(warning_text);
        let draft_text = (!self.chat.draft.is_empty()).then_some(self.chat.draft.as_str());
        let missing_text: Vec<_> = std::iter::once(identity.as_str())
            .chain(std::iter::once(title))
            .chain(senders.iter().copied())
            .chain(expected_history.iter().copied())
            .chain(receipt_text)
            .chain(warning_text)
            .chain(draft_text)
            .filter(|required| !self.texts.iter().any(|text| text.text == *required && text.complete))
            .collect();
        let leaked_text: Vec<_> = forbidden
            .iter()
            .filter(|forbidden| self.texts.iter().any(|text| text.text.contains(**forbidden)))
            .collect();
        let panel = self.theme.palette().panel;
        let background = (u32::from(panel.r()) << 16) | (u32::from(panel.g()) << 8) | u32::from(panel.b());
        let ink_pixels = self.pixels.iter().filter(|pixel| **pixel != background).count();
        let size = self.logical.map(|size| (size as f32 * self.scale).round() as u32);
        let rgba: Vec<_> = self
            .pixels
            .iter()
            .flat_map(|pixel| [(*pixel >> 16) as u8, (*pixel >> 8) as u8, *pixel as u8, 255])
            .collect();
        let image = format!("{name}.png");
        image::save_buffer(output.join(&image), &rgba, size[0], size[1], image::ColorType::Rgba8).unwrap();
        let result = json!({
            "image": image,
            "locale": aurora_i18n::language(),
            "logical_size": self.logical,
            "physical_size": size,
            "pixels_per_point": self.scale,
            "immutable_source": self.chat.source(),
            "snapshot_source": snapshot.source,
            "account": ACCOUNT,
            "grid": GRID,
            "selected_conversation": self.chat.conversation,
            "expected_identity": identity,
            "expected_conversation_title": title,
            "expected_visible_senders": senders,
            "expected_history_text": expected_history,
            "expected_receipt_text": receipt_text,
            "expected_warning_text": warning_text,
            "expected_draft_text": draft_text,
            "forbidden_text": forbidden,
            "draft_empty": self.chat.draft.is_empty(),
            "available": self.chat.available,
            "policy": self.chat.policy,
            "receipt": self.chat.receipt,
            "ink_pixels": ink_pixels,
            "missing_or_clipped_required_text": missing_text,
            "unexpected_revoked_text": leaked_text,
            "painted_text": self.texts.iter().map(|text| json!({"text": text.text, "fully_visible": text.complete})).collect::<Vec<_>>(),
        });
        std::fs::write(output.join(format!("{name}.json")), serde_json::to_vec_pretty(&result).unwrap()).unwrap();
        // Keep the actual failing pixels and semantic record for inspection;
        // passing a nonblank image alone is never visual acceptance.
        assert!(
            missing_text.is_empty(),
            "{name}: required identity/title/sender/text is missing or clipped: {missing_text:?}"
        );
        assert!(
            leaked_text.is_empty(),
            "{name}: revoked or retargeted text was painted: {leaked_text:?}"
        );
        assert!(ink_pixels > 100, "{name}: capture must contain actual painted UI pixels");
        result
    }
}

fn capture_layout(output: &Path, name: &str, logical: [u32; 2], scale: f32) -> Vec<Value> {
    let mut records = Vec::new();
    let mut snapshot = snapshot(source());
    let mut frames = Frames::new(logical, scale);
    let nearby = snapshot.conversations[0].title.as_str().to_owned();
    records.push(frames.capture(
        output,
        &format!("{name}-01-nearby"),
        &snapshot,
        &nearby,
        &[NEARBY_SENDER],
        &[DIRECT_TEXT],
    ));
    frames.select_direct(&snapshot);
    frames.type_text(&snapshot, "Synthetic literal draft: /ao off · Bonjour · こんにちは");
    records.push(frames.capture(
        output,
        &format!("{name}-02-inactive-im-draft"),
        &snapshot,
        RESIDENT,
        &[RESIDENT],
        &[NEARBY_SENDER],
    ));
    let sent = frames
        .click(&snapshot, Id::unique("host_chat_send_rect"))
        .expect("explicit fixture click emits one typed action");
    assert_eq!(sent.key, snapshot.conversations[1].policy.key);
    assert_eq!(sent.policy_revision, snapshot.conversations[1].policy.revision);
    assert_eq!(sent.text.as_str(), "Synthetic literal draft: /ao off · Bonjour · こんにちは");
    frames.chat.queued(&sent, 42);
    snapshot.receipts.push(SendReceipt {
        key: sent.key,
        request: 42,
        outcome: SendOutcome::Pending,
    });
    assert!(frames.chat.draft.is_empty());
    records.push(frames.capture(output, &format!("{name}-03-queued"), &snapshot, RESIDENT, &[RESIDENT], &[]));
    assert_eq!(frames.chat.receipt.unwrap().outcome, SendOutcome::Pending);
    assert!(frames.click(&snapshot, Id::unique("host_chat_send_rect")).is_none());
    snapshot.receipts[0].outcome = SendOutcome::Uncertain;
    records.push(frames.capture(output, &format!("{name}-04-uncertain"), &snapshot, RESIDENT, &[RESIDENT], &[]));
    assert_eq!(frames.chat.receipt.unwrap().outcome, SendOutcome::Uncertain);

    frames.type_text(&snapshot, SOURCE_DRAFT);
    let mut replacement = source();
    replacement.session.generation += 1;
    replacement.session.account = AccountKey::from_bytes([55; 32]).unwrap();
    let mut replacement_snapshot = self::snapshot(replacement);
    replacement_snapshot.lines[0].text = ChatText::new(REPLACEMENT_TEXT).unwrap();
    frames.settle(&replacement_snapshot);
    assert!(frames.chat.draft.is_empty() && !frames.chat.available);
    records.push(frames.capture(
        output,
        &format!("{name}-05-source-replaced"),
        &replacement_snapshot,
        aurora_i18n::tr!("host-chat-no-conversation"),
        &[],
        &[SOURCE_DRAFT, REPLACEMENT_TEXT, DIRECT_TEXT, RESIDENT, NEARBY_SENDER],
    ));

    // Independent timeline: never revive a retired source to stage the next
    // picture. The revoked policy is applied before the first capture frame.
    let mut privacy = Frames::new(logical, scale);
    let mut restricted = self::snapshot(source());
    privacy.select_direct(&restricted);
    privacy.type_text(&restricted, PRIVACY_DRAFT);
    restricted.conversations[1].policy.revision += 1;
    restricted.conversations[1].policy.can_send = false;
    restricted.conversations[1].policy.can_read = false;
    restricted.conversations[1].title = Label::new(aurora_i18n::tr!("rlva-blocked")).unwrap();
    restricted.lines.retain(|line| line.key.conversation != direct());
    restricted.gap = true;
    privacy.settle(&restricted);
    assert!(privacy.chat.draft.is_empty());
    assert!(privacy.chat.policy.is_some_and(|policy| !policy.can_send && !policy.can_read));
    assert!(privacy.click(&restricted, Id::unique("host_chat_send_rect")).is_none());
    records.push(privacy.capture(
        output,
        &format!("{name}-06-privacy-revoked"),
        &restricted,
        aurora_i18n::tr!("rlva-blocked"),
        &[],
        &[PRIVACY_DRAFT, DIRECT_TEXT, RESIDENT, NEARBY_SENDER],
    ));
    records
}

#[test]
#[ignore = "explicit CPU visual capture; run alone with AURORA_HOST_CHAT_VISUAL_DIR and AURORA_HOST_CHAT_VISUAL_LANGUAGE"]
fn host_chat_cpu_visual_fixture() {
    let language = std::env::var("AURORA_HOST_CHAT_VISUAL_LANGUAGE").expect("select exactly en, fr or ja for this isolated test process");
    assert!(["en", "fr", "ja"].contains(&language.as_str()));
    let output =
        std::path::PathBuf::from(std::env::var_os("AURORA_HOST_CHAT_VISUAL_DIR").expect("set an empty disposable capture directory"));
    assert!(
        !output.exists() || std::fs::read_dir(&output).unwrap().next().is_none(),
        "preserve prior captures; choose an empty directory"
    );
    std::fs::create_dir_all(&output).unwrap();
    let previous_language = aurora_i18n::language().to_owned();
    struct RestoreLanguage(String);
    impl Drop for RestoreLanguage {
        fn drop(&mut self) {
            aurora_i18n::select(&self.0, None);
        }
    }
    let _restore = RestoreLanguage(previous_language);
    aurora_i18n::select(&language, None);
    assert_eq!(aurora_i18n::language(), language);
    let fallback = fonts::fallback::wait();
    if language == "ja" {
        assert!(
            fallback.iter().any(|face| face.has('あ') && face.has('界')),
            "Japanese visual acceptance requires an installed CJK fallback font"
        );
    }
    let mut records = capture_layout(&output, "normal", [640, 640], 1.0);
    records.extend(capture_layout(&output, "narrow-150pct", [380, 640], 1.5));
    assert_eq!(records.len(), 12);
    let manifest = json!({
        "fixture": "actual ChatUi + SoftwareWindow CPU Painter; synthetic data only",
        "locale": language,
        "native_window_acceptance": false,
        "network_or_grid": false,
        "captures": records,
    });
    std::fs::write(output.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    println!("CPU chat visual captures: {}", output.display());
}
