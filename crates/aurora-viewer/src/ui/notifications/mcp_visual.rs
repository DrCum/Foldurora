//! Full native notification-review routes, including their read-state behavior.
use super::*;
use crate::ui::test_cpu::{Environment, Frames, enter, pointer};
use crate::world::notification_history::History;
use serde_json::json;
use std::sync::Arc;

#[test]
#[ignore = "explicit isolated CPU capture; set AURORA_MCP_VISUAL_DIR and AURORA_MCP_VISUAL_LANGUAGE"]
fn notification_review_mcp_cpu_visual_fixture() {
    let env = Environment::new("notification-review");
    for aurora in [false, true] {
        let mut frames = Frames::new([1240, 900]);
        let world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let mut emoji = crate::ui::emoji::Emoji::load();
        let mut history = History::default();
        let mut list = Notifications::default();
        let unrelated = list.push(
            Kind::Info,
            "Unrelated untouched notification",
            "Unrelated private body must remain hidden",
            Data::None,
        );
        let selected = list.push(
            Kind::Friendship,
            "Synthetic selected review",
            "Reviewing this exact offer must not answer it.",
            Data::Friend {
                from: Uuid::from_u128(10),
                tx: Uuid::from_u128(11),
                offline: false,
            },
        );
        list.aurora = aurora;
        let mut nui = NotifUi {
            open: true,
            review: Some(selected),
            anchor: Some(egui::pos2(24., 60.)),
            bell: Some(egui::Rect::from_min_size(egui::pos2(1170., 8.), egui::vec2(36., 36.))),
            ..Default::default()
        };
        nui.center.tab = crate::ui::notification_center::Tab::ToDo;
        let pics = HashMap::new();
        let images = HashMap::new();
        let mut draw = |frames: &mut Frames, list: &mut Notifications, nui: &mut NotifUi, events| {
            frames.draw(events, |ui, p, icons| {
                let mut env = ToastEnv {
                    pics: &pics,
                    images: &images,
                    emoji: &mut emoji,
                    hidden: false,
                    message_seconds: 30.,
                    restart: None,
                    presence_at: crate::settings::PresencePosition::TopRight,
                    classic_bars: false,
                };
                if aurora {
                    crate::ui::notification_center::show(ui.ctx(), p, icons, &world, list, &mut history, nui, &mut env, false)
                } else {
                    show(ui.ctx(), p, icons, &world, list, &mut history, nui, &mut env, false)
                }
            })
        };
        for _ in 0..8 {
            let (answers, asks) = draw(&mut frames, &mut list, &mut nui, vec![]);
            assert!(answers.is_empty() && asks.is_empty());
        }
        assert!(frames.has_text("Synthetic selected review"));
        assert!(!frames.has_text("Unrelated untouched notification"));
        assert!(!frames.has_text("Unrelated private body"));
        assert_eq!(list.unread(), 2, "opening exact review must not mark any notification read");
        assert!(list.list.iter().all(|n| !n.read));
        assert_eq!(nui.review, Some(selected));
        for pressed in [true, false] {
            let (answers, asks) = draw(&mut frames, &mut list, &mut nui, enter(pressed));
            assert!(answers.is_empty() && asks.is_empty(), "Enter does not answer review");
        }
        let style = if aurora { "aurora" } else { "legacy" };
        frames.capture(&env.directory,&format!("{style}-exact-review"),json!({"style":style,"review_id":selected,"unrelated_id":unrelated,"unread":list.unread(),"opening_answers":0,"no_default_enter":true}));
        let at = frames.text_center(aurora_i18n::tr!("ui-notifications-accept"));
        let mut answers = Vec::new();
        for pressed in [true, false] {
            let (next, asks) = draw(&mut frames, &mut list, &mut nui, pointer(at, pressed));
            assert!(asks.is_empty());
            answers.extend(next);
        }
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].0, selected);
        assert!(matches!(answers[0].1, Response::Accept));
        assert_eq!(list.unread(), 2, "native typed response is not dispatched by this fixture");
        assert_eq!(list.list.iter().map(|n| n.id).collect::<Vec<_>>(), vec![unrelated, selected]);
        nui.review = Some(u64::MAX);
        for _ in 0..4 {
            let (answers, asks) = draw(&mut frames, &mut list, &mut nui, vec![]);
            assert!(answers.is_empty() && asks.is_empty());
        }
        assert!(!frames.has_text("Unrelated untouched notification"));
        assert!(!frames.has_text("Synthetic selected review"));
        assert_eq!(list.unread(), 2);
        frames.capture(
            &env.directory,
            &format!("{style}-missing-review"),
            json!({"style":style,"review_id":u64::MAX,"unread":2,"unrelated_visible":false}),
        );
        nui.open = false;
        let _ = draw(&mut frames, &mut list, &mut nui, vec![]);
        assert_eq!(nui.review, None);
        std::fs::write(
            env.directory.join(format!("{style}-pointer-outcome.json")),
            serde_json::to_vec_pretty(&json!({"selected":selected,"response":"Accept","native_dispatch":false,"unrelated_unread":true}))
                .unwrap(),
        )
        .unwrap();
    }
}
