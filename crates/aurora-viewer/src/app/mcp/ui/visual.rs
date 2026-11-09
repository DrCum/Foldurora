//! Actual Local Assistant controls with in-memory authority and a disconnected IO owner.
use super::*;
use crate::ui::test_cpu::{Environment, Frames, enter, pointer, rect_json};
use aurora_mcp::assistant::permissions::Configuration;
use aurora_mcp::assistant::runtime::ViewerRequest;
use aurora_session::{GridKey, InstallationId};

fn binding() -> Binding {
    Binding {
        worker: WorkerIdentity {
            installation: InstallationId::from_bytes([1; 32]).unwrap(),
            host: HostId::from_bytes([2; 16]).unwrap(),
            worker: WorkerId::from_bytes([3; 16]).unwrap(),
        },
        session: Some(SessionIdentity {
            session: SessionId::from_bytes([4; 16]).unwrap(),
            account: AccountKey::from_bytes([5; 32]).unwrap(),
            grid: GridKey::from_bytes([6; 32]).unwrap(),
            generation: 1,
        }),
    }
}
fn draw(frames: &mut Frames, state: &mut Assistant, config: &mut Configuration, events: Vec<egui::Event>) {
    state.runtime.observe(state.binding, true);
    assert!(
        !frames.draw(events, |ui, p, _| show(
            ui.ctx(),
            p,
            state,
            config,
            "Synthetic Resident · Synthetic Grid"
        )),
        "review clicks must not change policy"
    );
}
// Report the actual production scroll clip independently from layout bounds.
// Partial cards are not silently promoted to fully visible native evidence.
fn review_actions(frames: &Frames, id: &str) -> Value {
    let clip = frames.rect(egui::Id::unique(("assistant_action_clip", id)));
    let deny = frames.rect(egui::Id::unique(("assistant_deny", id)));
    let allow = frames.rect(egui::Id::unique(("assistant_allow", id)));
    json!({
        "clip":rect_json(clip),
        "deny":rect_json(deny),"deny_visible":rect_json(deny.intersect(clip)),
        "deny_fully_visible":clip.contains_rect(deny),"deny_center_visible":clip.contains(deny.center()),
        "allow":rect_json(allow),"allow_visible":rect_json(allow.intersect(clip)),
        "allow_fully_visible":clip.contains_rect(allow),"allow_center_visible":clip.contains(allow.center()),
    })
}
fn poll(state: &Assistant, endpoint: HostId, id: &str) -> Value {
    state.runtime.request(
        1,
        ViewerRequest::OperationStatus {
            viewer_session: endpoint,
            request_id: id.into(),
        },
    )
}
#[test]
#[ignore = "explicit isolated CPU capture; set AURORA_MCP_VISUAL_DIR and AURORA_MCP_VISUAL_LANGUAGE"]
fn character_mcp_cpu_visual_fixture() {
    let env = Environment::new("character");
    let mut frames = Frames::new([1240, 900]);
    let mut state = Assistant::with_io(io::Owner::inert());
    state.open = true;
    let mut config = Configuration::default();
    for _ in 0..4 {
        draw(&mut frames, &mut state, &mut config, vec![]);
    }
    assert!(!state.runtime.snapshot().enabled);
    assert!(state.io.snapshot().installation.is_none());
    frames.capture(
        &env.directory,
        "01-default-disabled",
        json!({"enabled":false,"binding":false,"io":"inert disconnected owner"}),
    );
    let source = binding();
    let endpoint = HostId::from_bytes([7; 16]).unwrap();
    state.binding = Some(source);
    state.runtime.observe(Some(source), true);
    config.permissions.set(Class::ChatSendNearby, Level::Ask).unwrap();
    state.runtime.configure(config.clone()).unwrap();
    state.runtime.capabilities([Tool::ChatSend]);
    state.runtime.enable(source, endpoint).unwrap();
    let text = format!("{}MCP_FULL_TAIL_SENTINEL", "Frozen literal text; ".repeat(44));
    assert!(text.len() <= 1023);
    let arguments =
        json!({"conversation_id":"synthetic-nearby-generation-1","topic":"nearby","text":text,"client_request_id":"synthetic-exact-once"})
            .as_object()
            .unwrap()
            .clone();
    let mut receipts = Vec::new();
    for (index, allow) in [false, true].into_iter().enumerate() {
        state.runtime.observe(Some(source), true);
        let reply = state.runtime.request(
            1,
            ViewerRequest::Invoke {
                viewer_session: endpoint,
                tool: Tool::ChatSend,
                arguments: arguments.clone(),
                request_id: None,
            },
        );
        let id = reply["request_id"].as_str().unwrap().to_owned();
        state.runtime.prepared(&id, &[Class::ChatSendNearby], false).unwrap();
        state.evidence.insert(
            id.clone(),
            json!({"recipient":"Synthetic Nearby","permission":"chat_send_nearby","native_revision":4,"same_source":true}),
        );
        for _ in 0..4 {
            draw(&mut frames, &mut state, &mut config, vec![]);
        }
        let deny = frames.rect(egui::Id::unique(("assistant_deny", &id)));
        let allow_rect = frames.rect(egui::Id::unique(("assistant_allow", &id)));
        assert!(deny.left() < allow_rect.left(), "Deny precedes Allow once");
        assert!(deny.bottom() < 750., "pending request actions remain inside the opened floater");
        for pressed in [true, false] {
            draw(&mut frames, &mut state, &mut config, enter(pressed));
        }
        assert_eq!(poll(&state, endpoint, &id)["status"], "pending_approval");
        assert!(state.runtime.ready().is_empty());
        let header = frames.rect(egui::Id::unique(("assistant_evidence", &id)));
        for pressed in [true, false] {
            draw(&mut frames, &mut state, &mut config, pointer(header.center(), pressed));
        }
        for _ in 0..4 {
            draw(&mut frames, &mut state, &mut config, vec![]);
        }
        assert!(frames.has_text("Synthetic Nearby"), "actual native evidence is reviewable");
        let scroll: egui::Id = frames
            .context
            .data(|data| data.get_temp(egui::Id::unique(("assistant_arguments_scroll", &id))))
            .unwrap();
        let arguments_rect = frames.rect(egui::Id::unique(("assistant_arguments_rect", &id)));
        let content: egui::Vec2 = frames
            .context
            .data(|data| data.get_temp(egui::Id::unique(("assistant_arguments_content", &id))))
            .unwrap();
        let viewport: egui::Vec2 = frames
            .context
            .data(|data| data.get_temp(egui::Id::unique(("assistant_arguments_viewport", &id))))
            .unwrap();
        assert!(
            arguments_rect.is_positive(),
            "the actual argument clip must be reachable by the pointer"
        );
        frames.capture(&env.directory, &format!("0{}-before-scroll", index + 2), json!({"actual_clip":rect_json(arguments_rect), "content":[content.x,content.y], "viewport":[viewport.x,viewport.y], "pending":true,"review_actions":review_actions(&frames,&id)}));
        draw(
            &mut frames,
            &mut state,
            &mut config,
            vec![
                egui::Event::PointerMoved(arguments_rect.center()),
                egui::Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(-1_000_000., -1_000_000.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        for _ in 0..12 {
            draw(&mut frames, &mut state, &mut config, vec![]);
        }
        let offset = egui::scroll_area::State::load(&frames.context, scroll).unwrap().unclamped_offset();
        frames.capture(&env.directory, &format!("0{}-after-scroll", index + 2), json!({"actual_clip":rect_json(arguments_rect), "content":[content.x,content.y], "viewport":[viewport.x,viewport.y], "offset":[offset.x,offset.y], "pending":true,"review_actions":review_actions(&frames,&id)}));
        // The shared Label may wrap: reviewability is independent of the axis.
        // The real pointer and wheel must bring every tail glyph into the clip.
        assert!(
            frames.text.iter().any(|text| text["review_tail_visible"] == true),
            "the complete frozen tail must be visibly reviewable after scrolling"
        );
        frames.capture(&env.directory,&format!("0{}-frozen-review-tail",index+2),json!({"request_id":id,"frozen_text_bytes":text.len(),"full_tail_visible":true,"evidence_expanded":true,"deny":rect_json(frames.rect(egui::Id::unique(("assistant_deny",&id)))),"allow":rect_json(frames.rect(egui::Id::unique(("assistant_allow",&id)))),"no_default_enter":true,"review_actions":review_actions(&frames,&id)}));
        let key = if allow { "assistant_allow" } else { "assistant_deny" };
        let at = frames.rect(egui::Id::unique((key, &id))).center();
        for pressed in [true, false] {
            draw(&mut frames, &mut state, &mut config, pointer(at, pressed));
        }
        let result = poll(&state, endpoint, &id);
        if allow {
            let ready = state.runtime.ready();
            assert_eq!(ready.len(), 1);
            assert_eq!(ready[0].request_id, id);
            assert_eq!(ready[0].arguments, arguments, "UI resolution cannot edit frozen arguments");
        } else {
            assert_eq!(result["error_code"], "permission_denied");
            assert!(state.runtime.ready().is_empty());
        }
        receipts.push(json!({"request_id":id,"explicit_pointer_allow":allow,"receipt":result,"native_execution":false}));
    }
    assert!(state.io.snapshot().installation.is_none());
    std::fs::write(
        env.directory.join("pointer-outcomes.json"),
        serde_json::to_vec_pretty(&receipts).unwrap(),
    )
    .unwrap();
}
