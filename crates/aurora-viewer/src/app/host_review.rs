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
        self.review_input.begin();
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
}

impl ReviewInput {
    fn begin(&mut self) {
        self.blocked = self.held;
        self.first_pass = true;
        self.armed = true;
    }

    pub fn observe(&mut self, event: &winit::event::WindowEvent) -> bool {
        use winit::{
            event::{ElementState, WindowEvent},
            keyboard::{Key, NamedKey},
        };
        if let WindowEvent::Focused(focused) = event {
            self.focused(*focused);
        }
        let WindowEvent::KeyboardInput { event, is_synthetic, .. } = event else {
            return false;
        };
        if event.logical_key != Key::Named(NamedKey::Enter) {
            return false;
        }
        self.key(event.state == ElementState::Pressed, *is_synthetic, event.repeat)
    }

    fn focused(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        if focused {
            self.held = self.before_focus;
            self.blocked &= self.held;
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
            }
            return false;
        }
        let was_held = std::mem::replace(&mut self.held, true);
        if !self.focused {
            self.before_focus = true;
        }
        if self.armed && (synthetic || repeat) {
            self.blocked = true;
        } else if self.armed && !was_held {
            self.armed = false;
        }
        !synthetic && self.blocked
    }

    pub fn filter(&mut self, raw: &mut egui::RawInput) {
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
        assert!(!input.key(true, false, false));
    }
}
