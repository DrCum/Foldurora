//! Synchronous App-thread native review. No deferred World focus marker, read
//! command, session creation, literal send or notification action is involved.

use super::{App, host_chat};
use crate::session_host::worker::WorkerBridge;
use aurora_session::chat::{ReviewConversationRequest, ReviewOutcome};

impl App {
    pub(super) fn review_existing_conversation(&mut self, bridge: &WorkerBridge, request: ReviewConversationRequest) -> ReviewOutcome {
        if request.validate().is_err()
            || host_chat::source(self, bridge) != Some(request.target.key.source)
            || self.exit_prompt.open
            || self.login_form.busy
            || self.options_ui.capture.is_some()
        {
            return ReviewOutcome::RejectedNative;
        }
        let conversation = match existing_target(&self.world, request, true) {
            Ok(conversation) => conversation,
            Err(outcome) => return outcome,
        };
        let Some(gfx) = &self.gfx else {
            return ReviewOutcome::RejectedNative;
        };
        // No await or deferred work between current native authority and the
        // exact show-only effect. Foreground failure is terminal, never queued.
        if let Err(outcome) = bridge.review_authorized(request.target.key.source, request.activation, &gfx.window) {
            return outcome;
        }
        // HWND/foreground checks may outlive the control receipt. Re-evaluate
        // the actual login source and fresh grant at the effect boundary.
        if host_chat::source(self, bridge) != Some(request.target.key.source)
            || !bridge.review_still_current(request.target.key.source, request.activation)
        {
            return ReviewOutcome::RejectedNative;
        }
        self.review_input.begin();
        self.alt_codes.reset();
        egui::Popup::close_all(&self.egui_ctx);
        self.egui_ctx.stop_dragging();
        self.egui_ctx.input_mut(|input| input.pointer = Default::default());
        // Retire an earlier deferred presentation request so a subsequent draw
        // cannot overwrite this synchronous exact selection.
        self.world.social.focus_im = None;
        self.chat_ui.review_existing(&self.egui_ctx, conversation, &mut self.panels.chat);
        gfx.window.focus_window();
        gfx.window.request_redraw();
        ReviewOutcome::OpenedLocally
    }
}

fn existing_target(
    world: &crate::world::World,
    request: ReviewConversationRequest,
    ui_ready: bool,
) -> Result<Option<uuid::Uuid>, ReviewOutcome> {
    let native = host_chat::snapshot(world, request.target.key.conversation, ui_ready).ok_or(ReviewOutcome::StaleTarget)?;
    if !native.ready || !native.can_read {
        return Err(ReviewOutcome::RejectedNative);
    }
    let read = host_chat::read_metadata(world, request.target.key.conversation, ui_ready).ok_or(ReviewOutcome::StaleTarget)?;
    if read.observed.epoch != request.target.observed.epoch || read.observed.through < request.target.observed.through {
        return Err(ReviewOutcome::StaleTarget);
    }
    host_chat::native_read_key(request.target.key.conversation).ok_or(ReviewOutcome::RejectedNative)
}

/// Native Enter is observed before egui-winit ignores focus snapshots. A
/// review cannot turn a key held during activation into a new composer Send.
#[derive(Default)]
pub(super) struct ReviewInput {
    held: bool,
    blocked: bool,
    first_pass: bool,
    armed: bool,
    focused: bool,
    before_focus: bool,
    ime_composing: bool,
    ime_quarantined: bool,
    ime_enter_quarantined: bool,
}

impl ReviewInput {
    fn begin(&mut self) {
        self.ime_enter_quarantined |= self.ime_composing || self.ime_quarantined;
        self.blocked = self.held || self.ime_enter_quarantined;
        self.first_pass = true;
        self.armed = true;
        self.ime_quarantined |= self.ime_composing;
        // Commit may precede its confirming key-down. Preserve that key fence
        // through terminal IME events and focus snapshots until physical up.
        // A canceled old composition conservatively consumes the first Enter.
    }

    pub fn observe(&mut self, event: &winit::event::WindowEvent) -> bool {
        use winit::{
            event::{ElementState, WindowEvent},
            keyboard::{Key, NamedKey},
        };
        if let WindowEvent::Focused(focused) = event {
            self.focused(*focused);
        }
        if let WindowEvent::Ime(ime) = event {
            return self.ime(ime);
        }
        let WindowEvent::KeyboardInput { event, is_synthetic, .. } = event else {
            return false;
        };
        if event.logical_key != Key::Named(NamedKey::Enter) {
            return false;
        }
        self.key(event.state == ElementState::Pressed, *is_synthetic, event.repeat)
    }

    fn ime(&mut self, ime: &winit::event::Ime) -> bool {
        use winit::event::Ime;
        let suppress = self.ime_quarantined;
        match ime {
            // An empty Preedit can precede Commit, so only an explicit terminal
            // event releases a composition captured before Review.
            Ime::Preedit(..) => self.ime_composing = true,
            Ime::Commit(_) | Ime::Disabled => {
                self.ime_composing = false;
                self.ime_quarantined = false;
            }
            Ime::Enabled => {}
        }
        suppress
    }

    fn focused(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        if focused {
            self.held = self.before_focus;
            self.blocked = (self.blocked && self.held) || self.ime_enter_quarantined;
        }
        self.before_focus = false;
    }

    fn key(&mut self, pressed: bool, synthetic: bool, repeat: bool) -> bool {
        if !pressed {
            if !synthetic {
                self.held = false;
                self.blocked = false;
                self.armed = false;
                self.before_focus = false;
                self.ime_enter_quarantined = false;
            }
            return false;
        }
        let was_held = std::mem::replace(&mut self.held, true);
        if !self.focused {
            self.before_focus = true;
        }
        if self.first_pass || self.ime_quarantined || self.ime_enter_quarantined {
            // This press will be rejected by the new-target frame fence. Its
            // later OS repeats must remain rejected until a physical release.
            self.blocked = true;
            self.armed = true;
        } else if self.armed && (synthetic || repeat) {
            self.blocked = true;
        } else if self.armed && !was_held {
            self.armed = false;
        }
        !synthetic && self.blocked
    }

    /// The final App input boundary, after egui-winit, native Alt codes and
    /// ordinary queued input assembly, immediately before run_ui.
    pub fn finish_frame_input(&mut self, raw: &mut egui::RawInput, alt_text: &mut Vec<String>) {
        raw.events.extend(alt_text.drain(..).map(egui::Event::Text));
        self.filter(raw);
    }

    fn filter(&mut self, raw: &mut egui::RawInput) {
        if std::mem::take(&mut self.first_pass) {
            // Pending input began before the explicit target change. Do not
            // interpret it against the newly selected native conversation.
            raw.events.retain(|event| {
                !matches!(
                    event,
                    egui::Event::Key { pressed: true, .. }
                        | egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Cut
                        | egui::Event::Ime(_)
                        | egui::Event::PointerButton { .. }
                )
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native_editor_frame(
        ctx: &egui::Context,
        input: &mut ReviewInput,
        alt_text: &mut Vec<String>,
        field: &'static str,
        draft: &mut String,
        events: Vec<egui::Event>,
    ) {
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 300.0))),
            time: Some(ctx.cumulative_pass_nr() as f64 / 60.0),
            focused: true,
            events,
            ..Default::default()
        };
        // Use the exact final App assembly boundary, including the separate
        // native Alt-code queue, before the existing egui editor sees input.
        input.finish_frame_input(&mut raw, alt_text);
        let mut output = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.add(egui::TextEdit::singleline(draft).id(egui::Id::unique(field)))
                    .request_focus();
            });
        });
        output.textures_delta.clear();
    }

    #[test]
    fn native_review_final_app_input_assembly_quarantines_queued_alt_code_text_before_new_editor() {
        use winit::keyboard::KeyCode;
        let ctx = egui::Context::default();
        let mut input = ReviewInput::default();
        let mut draft = "New target draft".to_owned();
        let mut alt_text = Vec::new();
        native_editor_frame(&ctx, &mut input, &mut alt_text, "review_alt_target", &mut draft, Vec::new());
        let mut alt_codes = crate::alt_codes::AltCodes::default();
        alt_codes.alt_pressed();
        assert!(alt_codes.key_pressed(KeyCode::Numpad3, false));
        alt_text.push(alt_codes.alt_released(None).unwrap());
        input.begin();
        native_editor_frame(&ctx, &mut input, &mut alt_text, "review_alt_target", &mut draft, Vec::new());
        assert!(alt_text.is_empty());
        assert_eq!(draft, "New target draft", "pre-review Alt text cannot bypass the final input fence");
        alt_codes.alt_pressed();
        assert!(alt_codes.key_pressed(KeyCode::Numpad3, false));
        alt_codes.reset(); // the production Review effect also retires unfinished Alt sequences
        assert!(alt_codes.alt_released(Some("old release")).is_none());
        alt_codes.alt_pressed();
        assert!(alt_codes.key_pressed(KeyCode::Numpad3, false));
        alt_text.push(alt_codes.alt_released(None).unwrap());
        native_editor_frame(&ctx, &mut input, &mut alt_text, "review_alt_target", &mut draft, Vec::new());
        assert!(draft.contains('♥'), "a fresh native Alt sequence remains an ordinary edit");
    }

    #[test]
    fn native_review_old_ime_composition_cannot_commit_across_later_frames_into_new_editor() {
        use winit::event::{Ime, WindowEvent};
        let ctx = egui::Context::default();
        let mut input = ReviewInput::default();
        let mut alt_text = Vec::new();
        let mut old = "Old draft".to_owned();
        let mut target = "New draft".to_owned();
        native_editor_frame(&ctx, &mut input, &mut alt_text, "review_ime_old", &mut old, Vec::new());
        assert!(!input.observe(&WindowEvent::Ime(Ime::Preedit("未確定".into(), None))));
        native_editor_frame(
            &ctx,
            &mut input,
            &mut alt_text,
            "review_ime_old",
            &mut old,
            vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "未確定".into(),
                active_range_chars: None,
            })],
        );
        let parked = old.clone();
        input.begin();
        native_editor_frame(&ctx, &mut input, &mut alt_text, "review_ime_target", &mut target, Vec::new());
        assert!(
            input.observe(&WindowEvent::Ime(Ime::Preedit(String::new(), None))),
            "empty Preedit is not proof the old Commit ended"
        );
        native_editor_frame(&ctx, &mut input, &mut alt_text, "review_ime_target", &mut target, Vec::new());
        assert!(
            input.key(true, false, false),
            "an old composition's confirmation Enter cannot send the new draft"
        );
        assert!(input.observe(&WindowEvent::Ime(Ime::Commit("old candidate".into()))));
        native_editor_frame(&ctx, &mut input, &mut alt_text, "review_ime_target", &mut target, Vec::new());
        assert_eq!(target, "New draft");
        assert_eq!(old, parked, "Review and the old terminal event never change parked text");
        input.key(false, false, false);
        assert!(!input.observe(&WindowEvent::Ime(Ime::Preedit("新".into(), None))));
        native_editor_frame(
            &ctx,
            &mut input,
            &mut alt_text,
            "review_ime_target",
            &mut target,
            vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "新".into(),
                active_range_chars: None,
            })],
        );
        assert!(!input.observe(&WindowEvent::Ime(Ime::Commit("新".into()))));
        native_editor_frame(
            &ctx,
            &mut input,
            &mut alt_text,
            "review_ime_target",
            &mut target,
            vec![egui::Event::Ime(egui::ImeEvent::Commit("新".into()))],
        );
        assert!(target.contains('新'), "a fresh composition after old termination remains usable");
    }

    fn native_world() -> crate::world::World {
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = uuid::Uuid::from_u128(99);
        world.mutes.state = crate::world::mutes::LoadState::Loaded;
        world.social.session_mut(uuid::Uuid::from_u128(55));
        let current: Vec<_> = [None, Some(uuid::Uuid::from_u128(55))]
            .into_iter()
            .map(|key| {
                let (token, raw, badge) = world.read_counters(key).unwrap();
                (key, token, raw, badge)
            })
            .collect();
        world.read_ledger.reconcile(&current);
        world
    }

    fn capture(world: &crate::world::World, conversation: aurora_session::chat::Conversation) -> ReviewConversationRequest {
        use aurora_session::*;
        let source = aurora_session::chat::ChatSource {
            worker: WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).unwrap(),
                host: HostId::from_bytes([2; 16]).unwrap(),
                worker: WorkerId::from_bytes([3; 16]).unwrap(),
            },
            session: SessionIdentity {
                session: SessionId::from_bytes([4; 16]).unwrap(),
                account: AccountKey::from_bytes([5; 32]).unwrap(),
                grid: GridKey::from_bytes([6; 32]).unwrap(),
                generation: 1,
            },
            lease_epoch: 1,
        };
        ReviewConversationRequest {
            target: aurora_session::chat::ReviewCapture {
                key: aurora_session::chat::ChatKey { source, conversation },
                policy_revision: 1,
                observed: host_chat::read_metadata(world, conversation, true).unwrap().observed,
            },
            request: 1,
            activation: 7,
        }
    }

    #[test]
    fn group_attention_native_review_requires_existing_initialized_group_and_fresh_permissions_without_mutation() {
        use aurora_session::{ConversationId, chat::Conversation};
        let group = uuid::Uuid::from_u128(55);
        let conversation = Conversation::Group(ConversationId::from_bytes(*group.as_bytes()).unwrap());
        for fault in 0..8 {
            let mut world = native_world();
            world.groups.groups.push(aurora_net::GroupMembership {
                id: group,
                name: "Synthetic group".into(),
                insignia: uuid::Uuid::nil(),
                powers: 0,
                accept_notices: true,
                list_in_profile: false,
                contribution: 0,
            });
            let mut info = crate::world::groups::SessionInfo::default();
            info.group = true;
            info.active = true;
            info.text_muted = true; // A moderator send restriction does not prohibit show-only review.
            world.groups.sessions.insert(group, info);
            world.social.session_mut(group).unread = 3;
            let request = capture(&world, conversation);
            request.validate().unwrap();
            let before = world.read_counters(Some(group));
            let sessions = world.social.ims.len();
            assert_eq!(existing_target(&world, request, true), Ok(Some(group)));
            assert_eq!(world.read_counters(Some(group)), before);
            match fault {
                0 => world.groups.groups.clear(),
                1 => world.groups.sessions.get_mut(&group).unwrap().closed = true,
                2 => world.groups.sessions.get_mut(&group).unwrap().active = false,
                3 => world.groups.sessions.get_mut(&group).unwrap().group = false,
                4 => world.mutes.state = crate::world::mutes::LoadState::Initial,
                5 => world.social.ims.retain(|im| im.other != group),
                6 => world.groups.mute_all_groups = true,
                7 => world.agent_id = uuid::Uuid::nil(),
                _ => unreachable!(),
            }
            assert!(existing_target(&world, request, true).is_err(), "fault {fault}");
            assert!(world.social.ims.len() <= sessions, "review never creates a missing native session");
            assert!(world.groups.out.is_empty());
            assert!(world.social.focus_im.is_none());
            if fault != 5 {
                assert_eq!(world.read_counters(Some(group)), before, "review never marks read or leaves");
            }
        }
    }

    #[test]
    fn conference_attention_native_review_uses_existing_known_session_without_read_send_or_join() {
        use aurora_session::{ConversationId, chat::Conversation};
        let mut world = native_world();
        world.bind_session_test_login(uuid::Uuid::from_u128(99));
        world.start_text_conference(vec![uuid::Uuid::from_u128(55), uuid::Uuid::from_u128(56)]);
        let temporary = *world.groups.sessions.keys().next().unwrap();
        let session = uuid::Uuid::from_u128(600);
        let body = aurora_llsd::llsd_map! { "agent_info" => aurora_llsd::llsd_map! {
            uuid::Uuid::from_u128(99).to_string() => aurora_llsd::llsd_map! { "mutes" => aurora_llsd::llsd_map! { "text" => true } },
            uuid::Uuid::from_u128(55).to_string() => aurora_llsd::llsd_map! {},
        } };
        world.apply(aurora_net::NetEvent::SessionStarted {
            temp_session: temporary,
            session,
            success: true,
            error: String::new(),
            agents: aurora_net::social::parse_session_agent_batch(&body, aurora_net::social::MAX_SESSION_ROSTER_ENTRIES),
        });
        world.groups.take_commands();
        let (token, raw, badge) = world.read_counters(Some(session)).unwrap();
        world.read_ledger.reconcile(&[(Some(session), token, raw, badge)]);
        let conversation = Conversation::Conference(ConversationId::from_bytes(*session.as_bytes()).unwrap());
        let request = capture(&world, conversation);
        let before = world.read_counters(Some(session));
        let count = world.social.ims.len();
        assert_eq!(existing_target(&world, request, true), Ok(Some(session)));
        assert_eq!(world.read_counters(Some(session)), before);
        assert_eq!(world.social.ims.len(), count);
        assert!(world.groups.out.is_empty());
        world.apply(aurora_net::NetEvent::SessionClosed {
            session,
            reason: String::new(),
        });
        assert!(existing_target(&world, request, true).is_err());
    }

    #[test]
    fn native_review_target_reuses_read_policy_without_send_permission_or_explicit_read_mutation() {
        use aurora_session::chat::Conversation;
        let mut world = native_world();
        let request = capture(&world, Conversation::Nearby);
        let policy = host_chat::snapshot(&world, Conversation::Nearby, true).unwrap();
        assert!(policy.can_read && !policy.can_send, "no region enables native read-only review");
        let before = host_chat::read_metadata(&world, Conversation::Nearby, true).unwrap();
        assert_eq!(existing_target(&world, request, true), Ok(None));
        assert_eq!(host_chat::read_metadata(&world, Conversation::Nearby, true), Some(before));
        assert_eq!(existing_target(&world, request, false), Err(ReviewOutcome::RejectedNative));
        world.agent_id = uuid::Uuid::nil();
        assert_eq!(existing_target(&world, request, true), Err(ReviewOutcome::RejectedNative));
    }

    #[test]
    fn native_review_target_rejects_missing_direct_recreated_history_and_privacy_without_creating_a_session() {
        use aurora_session::{ConversationId, chat::Conversation};
        let mut world = native_world();
        let peer = uuid::Uuid::from_u128(55);
        let direct = Conversation::Direct(ConversationId::from_bytes(*peer.as_bytes()).unwrap());
        let request = capture(&world, direct);
        assert_eq!(existing_target(&world, request, true), Ok(Some(peer)));
        let missing = Conversation::Direct(ConversationId::from_bytes([77; 16]).unwrap());
        assert_eq!(
            existing_target(
                &world,
                ReviewConversationRequest {
                    target: aurora_session::chat::ReviewCapture {
                        key: aurora_session::chat::ChatKey {
                            conversation: missing,
                            ..request.target.key
                        },
                        ..request.target
                    },
                    ..request
                },
                true
            ),
            Err(ReviewOutcome::StaleTarget)
        );
        for conversation in [
            Conversation::Group(ConversationId::from_bytes(*peer.as_bytes()).unwrap()),
            Conversation::Conference(ConversationId::from_bytes(*peer.as_bytes()).unwrap()),
        ] {
            assert_eq!(
                existing_target(
                    &world,
                    ReviewConversationRequest {
                        target: aurora_session::chat::ReviewCapture {
                            key: aurora_session::chat::ChatKey {
                                conversation,
                                ..request.target.key
                            },
                            ..request.target
                        },
                        ..request
                    },
                    true
                ),
                Err(ReviewOutcome::StaleTarget)
            );
        }
        let count = world.social.ims.len();
        world.social.session_mut(peer).clock.token += 1;
        let (token, raw, badge) = world.read_counters(Some(peer)).unwrap();
        world.read_ledger.reconcile(&[(Some(peer), token, raw, badge)]);
        assert_eq!(existing_target(&world, request, true), Err(ReviewOutcome::StaleTarget));
        assert_eq!(world.social.ims.len(), count);
        world.rlva.configure(true);
        world.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Synthetic controller".into(),
            source_id: uuid::Uuid::from_u128(77),
            owner_id: world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: "@recvim=n".into(),
        }));
        assert_eq!(existing_target(&world, request, true), Err(ReviewOutcome::RejectedNative));
        assert_eq!(world.social.ims.len(), count);
    }

    #[test]
    fn native_review_enter_requires_real_release_and_never_rearms_on_synthetic_focus_release() {
        let mut input = ReviewInput::default();
        input.focused(true);
        assert!(!input.key(true, false, false));
        input.begin();
        assert!(!input.key(false, true, false));
        assert!(!input.key(true, true, false));
        assert!(input.key(true, false, false));
        input.filter(&mut egui::RawInput::default());
        assert!(input.key(true, false, true));
        assert!(!input.key(false, false, false));
        assert!(!input.key(true, false, false));
    }

    #[test]
    fn native_review_first_pass_drops_queued_send_edges_and_typing_without_touching_a_draft() {
        let mut input = ReviewInput::default();
        input.begin();
        let mut raw = egui::RawInput {
            events: vec![
                egui::Event::Text("old target".into()),
                egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        };
        input.filter(&mut raw);
        assert!(raw.events.is_empty());
        raw.events.push(egui::Event::Text("fresh edit".into()));
        input.filter(&mut raw);
        assert_eq!(raw.events.len(), 1);
    }

    #[test]
    fn native_review_focus_snapshots_preserve_held_enter_but_release_outside_allows_a_fresh_press() {
        for snapshot_before_focus in [false, true] {
            let mut input = ReviewInput::default();
            input.focused(true);
            input.key(true, false, false);
            input.key(false, true, false);
            input.focused(false);
            input.begin();
            if snapshot_before_focus {
                input.key(true, true, false);
            }
            input.focused(true);
            if !snapshot_before_focus {
                input.key(true, true, false);
            }
            input.filter(&mut egui::RawInput::default());
            assert!(input.key(true, false, false));
            input.key(false, false, false);
            assert!(!input.key(true, false, false));
        }
        let mut input = ReviewInput::default();
        input.focused(true);
        input.key(true, false, false);
        input.focused(false);
        input.begin();
        input.focused(true); // no Enter in native focus snapshot
        input.filter(&mut egui::RawInput::default());
        assert!(!input.key(true, false, false));
    }

    #[test]
    fn native_review_enter_discarded_before_first_draw_stays_fenced_until_real_release() {
        let mut input = ReviewInput::default();
        input.focused(true);
        input.begin(); // Enter was physically up at the Review boundary
        assert!(input.key(true, false, false));
        input.finish_frame_input(&mut egui::RawInput::default(), &mut Vec::new());
        assert!(
            input.key(true, false, false),
            "repeat=false cannot rescue the discarded first press"
        );
        assert!(input.key(true, false, true));
        assert!(!input.key(false, true, false));
        assert!(input.key(true, false, false));
        assert!(!input.key(false, false, false));
        assert!(!input.key(true, false, false));
    }

    #[test]
    fn native_review_old_ime_confirmation_enter_is_fenced_in_either_terminal_event_order() {
        use winit::event::{Ime, WindowEvent};
        for commit_first in [false, true] {
            let mut input = ReviewInput::default();
            input.focused(true);
            assert!(!input.observe(&WindowEvent::Ime(Ime::Preedit("old candidate".into(), None))));
            input.begin();
            input.finish_frame_input(&mut egui::RawInput::default(), &mut Vec::new());
            if commit_first {
                assert!(input.observe(&WindowEvent::Ime(Ime::Commit("old candidate".into()))));
            }
            input.focused(false);
            input.focused(true); // native review focus cannot rearm the old confirmation key
            assert!(input.key(true, false, false));
            if !commit_first {
                assert!(input.observe(&WindowEvent::Ime(Ime::Commit("old candidate".into()))));
            }
            assert!(input.key(true, false, false));
            assert!(input.key(true, false, true));
            assert!(!input.key(false, false, false));
            assert!(!input.key(true, false, false), "physical release permits the next explicit send");
        }
    }
}
