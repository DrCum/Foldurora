//! Opt-in component captures of the common CPU host surfaces, not native/grid evidence.
use super::*;

#[test]
#[ignore = "explicit synthetic CPU capture; set AURORA_CONFERENCE_VISUAL_DIR to a new empty directory"]
fn conference_host_attention_and_composer_cpu_capture() {
    let directory = std::path::PathBuf::from(std::env::var_os("AURORA_CONFERENCE_VISUAL_DIR").expect("set capture directory"));
    assert!(
        !directory.exists() || std::fs::read_dir(&directory).unwrap().next().is_none(),
        "preserve previous captures"
    );
    std::fs::create_dir_all(&directory).unwrap();
    let mut host = host();
    host.chat.truncate(1);
    let snapshot = Arc::make_mut(&mut host.chat[0]);
    let conference = Conversation::Conference(ConversationId::from_bytes([12; 16]).unwrap());
    let mut info = snapshot.conversations[1].clone();
    info.policy.key.conversation = conference;
    info.title = Label::new("Synthetic Conference").unwrap();
    snapshot.conversations.push(info);
    snapshot.reads.insert(conference, snapshot.reads[&direct()]);
    snapshot.closes.targets.insert(conference, 22);
    let mut chat = crate::session_host::chat_ui::ChatUi::new(snapshot.source);
    assert!(chat.select_existing(snapshot, conference));
    let mut model = Attention::default();
    model.sync(&host, &host.attention_preferences.preferences);
    let mut attention = AttentionUi {
        filters_open: true,
        ..Default::default()
    };
    for (name, show_attention) in [("attention-conference", true), ("shared-conference", false)] {
        let context = egui::Context::default();
        let theme = crate::theme::Theme {
            panel_alpha: 255,
            ..Default::default()
        };
        theme.apply(&context, 1.0);
        context.all_styles_mut(|style| style.animation_time = 0.0);
        crate::ui::fonts::apply(&context, crate::ui::fonts::DEFAULT);
        let _icons = crate::ui::icons::Icons::load(&context, None);
        let mut painter = crate::crash::software::Painter::default();
        let mut pixels = vec![0; 640 * 640];
        for frame in 0..4 {
            let output = context.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 640.0))),
                    time: Some(f64::from(frame) / 60.0),
                    ..Default::default()
                },
                |ui| {
                    if show_attention {
                        assert!(attention.show(ui, &theme.palette(), &mut model, &host, InputMode::Live).is_none());
                    } else {
                        assert!(
                            chat.show(ui, &theme.palette(), Some(&host.chat[0]), "Synthetic Alpha", "Synthetic Grid")
                                .is_none()
                        );
                    }
                },
            );
            let primitives = context.tessellate(output.shapes, output.pixels_per_point);
            assert!(
                primitives
                    .iter()
                    .all(|primitive| matches!(primitive.primitive, egui::epaint::Primitive::Mesh(_)))
            );
            painter.paint(
                &mut pixels,
                [640, 640],
                &primitives,
                &output.textures_delta,
                output.pixels_per_point,
                theme.palette().panel,
            );
        }
        let rgba: Vec<_> = pixels
            .iter()
            .flat_map(|pixel| [(*pixel >> 16) as u8, (*pixel >> 8) as u8, *pixel as u8, 255])
            .collect();
        image::save_buffer(directory.join(format!("{name}.png")), &rgba, 640, 640, image::ColorType::Rgba8).unwrap();
    }
    std::fs::write(directory.join("scope.txt"), "Synthetic common CPU host Attention/Chat UI only. Same host surface for both viewer styles. No native window, GPU, grid, profile, socket or human acceptance.\n").unwrap();
}
