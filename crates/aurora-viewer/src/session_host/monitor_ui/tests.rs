use super::*;
use crate::session_host::monitor_model::Source;
use crate::session_host::slots::{SlotId, SlotLabel, SlotSummary};
use crate::session_host::transport::{HostChatSnapshot, WorkerSummary};
use aurora_session::chat::{ChatKey, ChatSource, Conversation, ConversationInfo, NativeChatPolicy, ObservedIncoming, ReadMetadata};
use aurora_session::{
    AccountKey, ConversationId, GridKey, HostId, InstallationId, Label, Phase, SessionId, SessionIdentity, WorkerId, WorkerIdentity,
    WorkerStatus,
};
use egui::{Event, Modifiers, PointerButton, Pos2, RawInput};
use std::sync::Arc;
pub(super) fn source(worker: u8) -> ChatSource {
    ChatSource {
        worker: WorkerIdentity {
            installation: InstallationId::from_bytes([1; 32]).unwrap(),
            host: HostId::from_bytes([2; 16]).unwrap(),
            worker: WorkerId::from_bytes([worker; 16]).unwrap(),
        },
        session: SessionIdentity {
            session: SessionId::from_bytes([worker; 16]).unwrap(),
            account: AccountKey::from_bytes([worker; 32]).unwrap(),
            grid: GridKey::from_bytes([6; 32]).unwrap(),
            generation: 1,
        },
        lease_epoch: 7,
    }
}

pub(super) fn direct() -> Conversation {
    Conversation::Direct(ConversationId::from_bytes([11; 16]).unwrap())
}

pub(super) fn host() -> HostSnapshot {
    let mut snapshot = HostSnapshot {
        attention_preferences: super::super::attention_prefs::PreferenceState {
            preferences: super::super::attention_prefs::Preferences::default(),
            writable: true,
            error: None,
            operation: None,
        },
        ..Default::default()
    };
    for (index, slot) in SlotId::all().enumerate() {
        let source = source(index as u8 + 1);
        snapshot.slots.push(SlotSummary {
            id: slot,
            label: SlotLabel::new("Same profile").unwrap(),
            available: false,
        });
        snapshot.workers.push(WorkerSummary {
            slot: Some(slot),
            binding: source.binding(),
            reachable: true,
            label: "Fallback profile".into(),
            grid: "Fallback grid".into(),
            status: Some(WorkerStatus {
                phase: Phase::Online,
                name: Label::new("Synthetic Resident").unwrap(),
                grid_label: Label::new("Synthetic Grid").unwrap(),
                native_window: index as u64 + 1,
                keyboard_focused: false,
                blocked: false,
                last_frame: None,
            }),
        });
        snapshot.chat.push(Arc::new(HostChatSnapshot {
            source,
            authenticated: true,
            live: true,
            conversations: [Conversation::Nearby, direct()]
                .into_iter()
                .enumerate()
                .map(|(index, conversation)| ConversationInfo {
                    title: Label::new("Synthetic conversation").unwrap(),
                    policy: NativeChatPolicy {
                        key: ChatKey { source, conversation },
                        revision: index as u64 + 1,
                        lease_current: true,
                        ready: true,
                        can_read: true,
                        can_send: true,
                    },
                })
                .collect(),
            reads: [Conversation::Nearby, direct()]
                .into_iter()
                .map(|conversation| {
                    (
                        conversation,
                        ReadMetadata {
                            unread: 7,
                            unread_revision: 1,
                            observed: ObservedIncoming { epoch: 1, through: 7 },
                            can_mark_read: true,
                        },
                    )
                })
                .collect(),
            lines: Vec::new(),
            receipts: Vec::new(),
            read_operations: Vec::new(),
            gap: false,
        }));
    }
    snapshot
}

pub(super) struct Frames {
    pub contexts: [egui::Context; 3],
    pub bodies: [MonitorUi; 2],
    pub model: MonitorSet,
    pub chat: SharedChat,
    pub host: HostSnapshot,
    pub states: [State; 2],
    pub frame: u64,
    pub active: usize,
    pub logical: [f32; 2],
    pub scale: f32,
    pub now: u64,
    pub text: Vec<String>,
    pub output: Option<egui::FullOutput>,
}
impl Frames {
    pub fn new(logical: [f32; 2], scale: f32) -> Self {
        let contexts = std::array::from_fn(|_| {
            let context = egui::Context::default();
            crate::theme::Theme {
                panel_alpha: 255,
                ..Default::default()
            }
            .apply(&context, 1.0);
            context.all_styles_mut(|style| style.animation_time = 0.0);
            context
        });
        let mut model = MonitorSet::new().unwrap();
        let sources: Vec<_> = (1..=5)
            .map(|number| Source::new(SlotId::try_from(number - 1).unwrap(), source(number), true).unwrap())
            .collect();
        model
            .sync(
                &sources,
                std::array::from_fn(|i| SlotId::try_from(i as u8).unwrap()),
                Some(sources[4].preview()),
            )
            .unwrap();
        model.open_fixed(sources[0]).unwrap();
        model.open_cycler(0).unwrap();
        model.pause().unwrap();
        let bodies = [MonitorUi::new(&contexts[1], 0, 0), MonitorUi::new(&contexts[2], 0, 0)];
        let mut chat = SharedChat::default();
        let host = host();
        chat.sync(&host, false);
        for surface in [chat_ui::ChatSurface::Shared, View::Fixed1.surface(), View::Cycling.surface()] {
            chat.select_for(surface, source(1));
        }
        let mut result = Self {
            contexts,
            bodies,
            model,
            chat,
            host,
            states: [State {
                chat: true,
                ..Default::default()
            }; 2],
            frame: 0,
            active: 1,
            logical,
            scale,
            now: 10,
            text: Vec::new(),
            output: None,
        };
        for index in 0..3 {
            result.settle(index);
        }
        result
    }
    pub fn surface(index: usize) -> chat_ui::ChatSurface {
        [chat_ui::ChatSurface::Shared, View::Fixed1.surface(), View::Cycling.surface()][index]
    }
    pub fn view(index: usize) -> View {
        [View::Fixed1, View::Cycling][index - 1]
    }
    pub fn show_image(&mut self, index: usize, events: Vec<Event>, image: Option<Image<'_>>, mode: InputMode) -> Option<Action> {
        self.chat.sync(&self.host, false);
        self.frame += 1;
        let modifiers = events
            .iter()
            .rev()
            .find_map(|event| match event {
                Event::Key { modifiers, .. } | Event::PointerButton { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let mut raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(self.logical[0], self.logical[1]))),
            time: Some(self.frame as f64 / 60.0),
            focused: self.active == index,
            events: std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect(),
            ..Default::default()
        };
        raw.viewports.entry(raw.viewport_id).or_default().native_pixels_per_point = Some(self.scale);
        let context = self.contexts[index].clone();
        let palette = crate::theme::Theme::default().palette();
        let mut action = None;
        let output = context.run_ui(raw, |ui| {
            let result = if index == 0 {
                let send = self.chat.show_for(Self::surface(index), ui, &palette, mode);
                let read = self.chat.take_read_action_for(Self::surface(index));
                send.map(Action::Send).or_else(|| read.map(Action::Read))
            } else {
                let view = Self::view(index);
                self.bodies[index - 1].show(
                    ui,
                    &palette,
                    view,
                    &self.model,
                    &mut self.chat,
                    &self.host,
                    self.states[index - 1],
                    image.as_ref().map(|image| Image {
                        binding: image.binding,
                        admitted: image.admitted,
                        texture: image.texture,
                    }),
                    self.now,
                    mode,
                )
            };
            action = action.take().or(result);
        });
        self.text.clear();
        fn collect(shape: &egui::Shape, texts: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => texts.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, texts);
                    }
                }
                egui::Shape::Callback(_) => panic!("CPU monitor must use meshes only"),
                _ => {}
            }
        }
        for shape in &output.shapes {
            collect(&shape.shape, &mut self.text);
        }
        self.output = Some(output);
        action
    }
    pub fn show(&mut self, index: usize, events: Vec<Event>) -> Option<Action> {
        self.show_image(index, events, None, InputMode::Live)
    }
    pub fn settle(&mut self, index: usize) {
        for _ in 0..4 {
            assert!(self.show(index, Vec::new()).is_none());
        }
    }
    pub fn rect(&mut self, index: usize, id: Id) -> Rect {
        self.settle(index);
        self.contexts[index]
            .data(|data| data.get_temp::<Rect>(id))
            .expect("actual production widget rectangle")
    }
    pub fn pointer(&mut self, index: usize, at: Pos2, pressed: bool) -> Option<Action> {
        self.show(
            index,
            vec![
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        )
    }
    pub fn click(&mut self, index: usize, id: Id) -> Option<Action> {
        let at = self.rect(index, id).center();
        assert!(self.pointer(index, at, true).is_none());
        self.pointer(index, at, false)
    }
    pub fn focus(&mut self, index: usize) {
        if self.active != index {
            let previous = self.active;
            self.active = index;
            assert!(self.show(previous, vec![Event::WindowFocused(false)]).is_none());
            assert!(self.show(index, vec![Event::WindowFocused(true)]).is_none());
        }
        self.settle(index);
    }
    pub fn composer(&mut self, index: usize) {
        self.focus(index);
        assert!(self.click(index, Id::unique("host_chat_input_rect")).is_none());
    }
    pub fn type_text(&mut self, index: usize, text: &str) {
        self.composer(index);
        assert!(self.show(index, vec![Event::Text(text.into())]).is_none());
        self.settle(index);
    }
    pub fn send(&mut self, index: usize) -> SendDraft {
        self.focus(index);
        let Some(Action::Send(send)) = self.click(index, Id::unique("host_chat_send_rect")) else {
            panic!("real canonical Send click")
        };
        send
    }
    pub fn has(&self, key: &'static str) -> bool {
        self.text.iter().any(|text| text.contains(aurora_i18n::text(key)))
    }
    pub fn sync_sources(&mut self) {
        let sources: Vec<_> = self
            .host
            .chat
            .iter()
            .filter_map(|chat| {
                let worker = self.host.workers.iter().find(|worker| worker.binding == chat.source.binding())?;
                Some(Source::new(worker.slot?, chat.source, chat.authenticated && chat.live).unwrap())
            })
            .collect();
        let active = sources.last().map(|source| source.preview());
        self.model
            .sync(&sources, std::array::from_fn(|i| SlotId::try_from(i as u8).unwrap()), active)
            .unwrap();
    }
}
fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers,
    }
}
fn command_z() -> Modifiers {
    Modifiers {
        ctrl: true,
        command: true,
        ..Modifiers::NONE
    }
}

#[test]
fn monitor_body_captures_exact_binding_and_settings_base_without_optimistic_acknowledgement() {
    let mut f = Frames::new([440.0, 560.0], 1.0);
    f.states[0].chat = false;
    let view = View::Fixed1;
    let old = f.model.binding(view).unwrap();
    let at = f.rect(1, control_id(view, "exchange")).center();
    assert!(f.pointer(1, at, true).is_none());
    f.model.close(view).unwrap();
    f.model
        .open_fixed(Source::new(SlotId::try_from(1).unwrap(), source(2), true).unwrap())
        .unwrap();
    assert!(f.pointer(1, at, false).is_none(), "release cannot exchange the replacement binding");
    assert_ne!(f.model.binding(view).unwrap().incarnation, old.incarnation);
    let old_pixels = f.rect(1, control_id(view, "exchange")).center();
    f.model.close(view).unwrap();
    f.model
        .open_fixed(Source::new(SlotId::try_from(2).unwrap(), source(3), true).unwrap())
        .unwrap();
    // Native input was queued against the old painted binding before the first
    // redraw can show this replacement; current geometry may not capture it.
    assert!(f.pointer(1, old_pixels, true).is_none());
    assert!(f.pointer(1, old_pixels, false).is_none());
    f.settle(1);
    assert!(
        matches!(f.click(1,control_id(view,"exchange")),Some(Action::Control{command:Command::Exchange(ticket),..}) if ticket.source().chat()==source(3))
    );
    assert!(f.click(1, control_id(view, "options")).is_none());
    f.settle(1);
    let Some(Action::Control {
        command: Command::Settings { base, desired, .. },
        ..
    }) = f.click(1, control_id(view, "medium"))
    else {
        panic!("visible published setting action")
    };
    assert_eq!(base.accepted.size, PreviewSize::Small);
    assert_eq!(desired.size, PreviewSize::Medium);
    assert_eq!(f.states[0].accepted.size, PreviewSize::Small);
    f.states[0].revision += 1;
    f.states[0].request = Some((77, desired));
    f.settle(1);
    assert!(f.has("host-monitor-settings-pending"));
    assert!(f.click(1, control_id(view, "medium")).is_none());
    f.states[0].revision += 1;
    f.states[0].request = None;
    f.states[0].failed = true;
    f.settle(1);
    assert!(f.has("host-monitor-settings-failed"));
    assert_eq!(f.states[0].accepted.size, PreviewSize::Small);
}

#[test]
fn monitor_body_dispatch_retains_all_original_guards_and_only_explicit_read_metadata() {
    let mut f = Frames::new([440.0, 560.0], 1.0);
    let view = View::Fixed1;
    let expected_ticket = f.model.capture_view(view).unwrap();
    let expected_binding = f.model.binding(view).unwrap();
    let expected_state = f.states[0];
    let expected_playing = f.model.playing();
    let expected_interval = f.model.interval();
    let Some(Action::Control {
        view: captured_view,
        ticket,
        binding,
        base,
        playing,
        interval,
        command: Command::Exchange(strong),
    }) = f.click(1, control_id(view, "exchange"))
    else {
        panic!("fresh pointer gesture captures the complete Exchange action")
    };
    assert_eq!(captured_view, view);
    assert_eq!(ticket, expected_ticket);
    assert_eq!(binding, expected_binding);
    assert_eq!(base, expected_state);
    assert_eq!(playing, expected_playing);
    assert_eq!(interval, expected_interval);
    assert_eq!(strong.source().chat(), source(1));
    assert!(f.model.validate_view(ticket) && f.model.can_exchange(strong));
    assert_eq!(f.model.binding(view), Some(expected_binding), "capture does not perform Exchange");

    f.model.close(view).unwrap();
    f.model
        .open_fixed(Source::new(SlotId::try_from(1).unwrap(), source(2), true).unwrap())
        .unwrap();
    assert!(!f.model.validate_view(ticket));
    assert!(!f.model.can_exchange(strong));
    assert_ne!(f.model.binding(view), Some(binding));
    assert_eq!(
        binding.source.chat(),
        source(1),
        "old dispatch is not retargeted to the replacement"
    );

    let mut r = Frames::new([440.0, 560.0], 1.0);
    r.type_text(1, "Draft remains local across explicit Read");
    let key = ChatKey {
        source: source(1),
        conversation: Conversation::Nearby,
    };
    let expected_read = *r.host.chat[0].reads.get(&key.conversation).unwrap();
    let expected_policy = r.host.chat[0]
        .conversations
        .iter()
        .find(|info| info.policy.key == key)
        .unwrap()
        .policy;
    assert!(
        r.chat.take_read_action_for(view.surface()).is_none(),
        "drawing and editing do not request Read"
    );
    let Some(Action::Read(read)) = r.click(1, Id::unique("host_chat_read_rect")) else {
        panic!("fresh explicit Read gesture returns its original canonical observation")
    };
    assert_eq!(read.key, key);
    assert_eq!(read.policy_revision, expected_policy.revision);
    assert_eq!(read.metadata, expected_read);
    assert!(
        r.chat.take_read_action_for(view.surface()).is_none(),
        "the body transfers the Read action once"
    );
    r.settle(1);
    assert!(!r.has("host-chat-read-pending"), "returning an action does not queue an operation");
    assert_eq!(r.host.chat[0].reads.get(&key.conversation), Some(&expected_read));
    assert_eq!(r.send(1).text.as_str(), "Draft remains local across explicit Read");
}

#[test]
fn monitor_body_held_options_and_navigation_never_borrow_changed_source_or_published_base() {
    let mut f = Frames::new([440.0, 592.0], 1.0);
    f.focus(2);
    f.states[1].chat = false;
    assert!(f.click(2, control_id(View::Cycling, "options")).is_none());
    let at = f.rect(2, control_id(View::Cycling, "medium")).center();
    assert!(f.pointer(2, at, true).is_none());
    f.states[1].revision += 1;
    f.states[1].accepted.size = PreviewSize::Large;
    assert!(f.pointer(2, at, false).is_none());
    f.bodies[1].cancel_input();
    f.settle(2);
    let at = f.rect(2, control_id(View::Cycling, "next")).center();
    let target = f.model.capture_navigation(Direction::Next).unwrap().target();
    assert!(f.pointer(2, at, true).is_none());
    let mut replacement = target.chat();
    replacement.lease_epoch += 1;
    let chat = Arc::make_mut(&mut f.host.chat[1]);
    chat.source = replacement;
    for info in &mut chat.conversations {
        info.policy.key.source = replacement;
    }
    f.host.workers[1].binding = replacement.binding();
    f.sync_sources();
    assert!(f.pointer(2, at, false).is_none());
    assert_eq!(f.model.binding(View::Cycling).unwrap().source.chat(), source(1));
}

#[test]
fn monitor_body_playing_cycle_requires_pause_then_fresh_composer_focus_before_typing_or_read() {
    let mut f = Frames::new([440.0, 592.0], 1.0);
    f.focus(2);
    f.type_text(2, "Canonical retained draft");
    f.model.play(100).unwrap();
    assert!(
        f.show(
            2,
            vec![Event::Text("must not append".into()), key(Key::Enter, true, Modifiers::NONE)]
        )
        .is_none()
    );
    f.settle(2);
    assert!(f.has("host-monitor-pause-chat"));
    assert!(f.click(2, Id::unique("host_chat_read_rect")).is_none());
    let Some(Action::Control {
        command: Command::Pause, ..
    }) = f.click(2, control_id(View::Cycling, "play"))
    else {
        panic!("explicit Pause")
    };
    f.model.pause().unwrap();
    assert!(f.show(2, vec![Event::Text("queued before new focus".into())]).is_none());
    f.settle(2);
    f.composer(2);
    assert!(f.show(2, vec![Event::Text(" + fresh".into())]).is_none());
    assert_eq!(f.send(2).text.as_str(), "Canonical retained draft + fresh");
}

#[test]
fn monitor_body_three_contexts_share_canonical_draft_undo_and_keep_mismatched_source_inert() {
    let mut f = Frames::new([680.0, 680.0], 1.0);
    f.type_text(0, "Canonical base");
    f.frame += 120;
    f.type_text(1, " plus monitor edit");
    f.composer(2);
    assert!(
        f.show(2, vec![key(Key::Z, true, command_z()), key(Key::Z, false, command_z())])
            .is_none()
    );
    assert_eq!(
        f.send(2).text.as_str(),
        "Canonical base",
        "third context owns the same canonical Undo"
    );
    assert_eq!(f.send(0).text.as_str(), "Canonical base");
    f.chat.select_for(View::Fixed1.surface(), source(2)); // Parent-controlled navigation, deliberately mismatched image binding.
    f.focus(1);
    assert!(
        f.show(1, vec![Event::Text("wrong sender".into()), key(Key::Enter, true, Modifiers::NONE)])
            .is_none()
    );
    assert_eq!(
        f.chat.selected_source_for(View::Fixed1.surface()),
        Some(source(2)),
        "body never selects a fallback sender"
    );
    assert!(f.chat.take_read_action_for(View::Fixed1.surface()).is_none());
    assert!(f.has("host-chat-source-unavailable"));
    f.chat.select_for(View::Fixed1.surface(), source(1));
    f.settle(1);
    assert_eq!(f.send(1).text.as_str(), "Canonical base");
    f.composer(1);
    let redo = Modifiers {
        shift: true,
        ..command_z()
    };
    assert!(f.show(1, vec![key(Key::Z, true, redo), key(Key::Z, false, redo)]).is_none());
    assert_eq!(
        f.send(0).text.as_str(),
        "Canonical base plus monitor edit",
        "source-bound mismatch did not replace the shared Undo/Redo history"
    );
}

#[test]
fn monitor_body_compact_layout_and_actual_controls_fit_narrow_short_and_scaled_contexts() {
    for (logical, scale, index) in [([380.0, 420.0], 1.0, 1), ([320.0, 392.0], 1.0, 1), ([380.0, 452.0], 1.5, 2)] {
        let mut f = Frames::new(logical, scale);
        f.host.slots[0].label = SlotLabel::new("Long synthetic saved profile name with distinct account identity").unwrap();
        f.host.workers[0].status.as_mut().unwrap().name = Label::new("Long synthetic resident account display name").unwrap();
        f.focus(index);
        f.settle(index);
        let view = Frames::view(index);
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(logical[0], logical[1]));
        for name in ["exchange", "chat", "pin", "options"] {
            let rect = f.rect(index, control_id(view, name));
            assert!(
                rect.is_positive() && canvas.contains_rect(rect),
                "actual control fits {logical:?}: {name} {rect:?}"
            );
        }
        let composer = f.rect(index, Id::unique("host_chat_input_rect"));
        let send = f.rect(index, Id::unique("host_chat_send_rect"));
        assert!(
            canvas.contains_rect(composer) && canvas.contains_rect(send),
            "canonical composer/actions remain reachable {logical:?}"
        );
        assert!(f.click(index, control_id(view, "options")).is_none());
        f.settle(index);
        let rect = f.rect(index, control_id(view, "small"));
        assert!(canvas.contains_rect(rect));
        assert!(matches!(
            f.click(index, control_id(view, "small")),
            Some(Action::Control {
                command: Command::Settings { .. },
                ..
            })
        ));
    }
}

#[test]
fn monitor_body_batched_press_release_uses_original_pixels_and_ime_cannot_arm_commands() {
    let mut f = Frames::new([440.0, 560.0], 1.0);
    f.states[0].chat = false;
    let a = f.rect(1, control_id(View::Fixed1, "exchange")).center();
    let b = f.rect(1, control_id(View::Fixed1, "pin")).center();
    assert!(
        f.show(
            1,
            vec![
                Event::PointerMoved(a),
                Event::PointerButton {
                    pos: a,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE
                },
                Event::PointerMoved(b),
                Event::PointerButton {
                    pos: b,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE
                }
            ]
        )
        .is_none()
    );
    let widget = f.bodies[0]
        .controls
        .iter()
        .find(|control| matches!(control.intent.command, Command::Exchange(_)))
        .unwrap()
        .widget;
    f.contexts[1].memory_mut(|memory| memory.request_focus(widget));
    assert!(
        f.show(
            1,
            vec![
                Event::Ime(egui::ImeEvent::Preedit {
                    text: "あ".into(),
                    active_range_chars: None
                }),
                key(Key::Space, true, Modifiers::NONE)
            ]
        )
        .is_none()
    );
    assert!(f.show(1, vec![key(Key::Space, false, Modifiers::NONE)]).is_none());
    assert!(f.show(1, vec![Event::Ime(egui::ImeEvent::Commit("あ".into()))]).is_none());
    f.settle(1);
}

pub(super) struct AdmittedFixture {
    pub admitted: AdmittedImage,
    pub images: aurora_session::preview_image::SourceImages,
    pub policy: aurora_session::preview_image::PreviewPolicy,
    pub rgba: Vec<u8>,
    pub size: [usize; 2],
}
impl AdmittedFixture {
    pub fn new(source: Source) -> Self {
        use aurora_session::preview_image::*;
        let budget = ImageBudget::new(HOST_IMAGE_BYTES).unwrap();
        let mut images = SourceImages::new(source.preview(), budget.clone()).unwrap();
        let epochs = PreviewEpochs::new(1, 1, 1, 1, 1).unwrap();
        let policy = PreviewPolicy::new(
            source.preview(),
            epochs,
            PreviewAccess {
                ready: true,
                active: true,
                activation: 1,
                capture_active: true,
                retain_last_active: true,
            },
        )
        .unwrap();
        images.sync(policy, 0).unwrap();
        let geometry = PreviewGeometry::new([640, 360], [0, 0, 640, 360], PreviewSize::Small).unwrap();
        let claim = images.request(1, 1, geometry, 0).unwrap();
        let stamp = PreviewStamp::new(claim, 1, 1, 1, geometry).unwrap();
        let admitted = images.admit(stamp, budget.reserve(geometry).unwrap(), 5).unwrap();
        let policy = PreviewPolicy::new(
            source.preview(),
            epochs,
            PreviewAccess {
                ready: true,
                active: false,
                activation: 1,
                capture_active: false,
                retain_last_active: true,
            },
        )
        .unwrap();
        images.sync(policy, 5).unwrap();
        let size = geometry.size().map(|side| side as usize);
        let rgba = (0..size[1])
            .flat_map(|y| {
                (0..size[0]).flat_map(move |x| {
                    if (x / 20 + y / 20) % 2 == 0 {
                        [48, 96, 144, 255]
                    } else {
                        [144, 96, 48, 255]
                    }
                })
            })
            .collect();
        Self {
            admitted,
            images,
            policy,
            rgba,
            size,
        }
    }
    pub fn observe_policy(&mut self, received_at: u64) {
        self.images.sync_observed(self.policy, received_at, received_at).unwrap();
    }
}

pub(super) fn draw_private(
    f: &mut Frames,
    index: usize,
    slot: &mut super::super::private_image::PrivateImageSlot,
    admitted: &AdmittedImage,
) -> super::super::private_image::PrivateFrame {
    draw_private_events(f, index, slot, admitted, Vec::new())
}

pub(super) fn draw_private_events(
    f: &mut Frames,
    index: usize,
    slot: &mut super::super::private_image::PrivateImageSlot,
    admitted: &AdmittedImage,
    events: Vec<Event>,
) -> super::super::private_image::PrivateFrame {
    f.frame += 1;
    f.chat.sync(&f.host, false);
    let mut raw = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(f.logical[0], f.logical[1]))),
        time: Some(f.frame as f64 / 60.0),
        focused: f.active == index,
        events,
        ..Default::default()
    };
    raw.viewports.entry(raw.viewport_id).or_default().native_pixels_per_point = Some(f.scale);
    let palette = crate::theme::Theme::default().palette();
    let view = Frames::view(index);
    let frame = slot
        .draw(raw, |ui, texture| {
            let body = &mut f.bodies[index - 1];
            if body.generation == 0 {
                body.reset_surface(ui.ctx(), texture.generation(), texture.binding_generation());
            }
            let image = f.model.binding(view).map(|binding| Image {
                binding,
                admitted,
                texture,
            });
            assert!(
                body.show(
                    ui,
                    &palette,
                    view,
                    &f.model,
                    &mut f.chat,
                    &f.host,
                    f.states[index - 1],
                    image,
                    f.now,
                    InputMode::Live
                )
                .is_none()
            );
        })
        .unwrap();
    f.text.clear();
    fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(shape) => text.push(shape.galley.text().to_owned()),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, text);
                }
            }
            _ => {}
        }
    }
    for shape in &frame.output().shapes {
        collect(&shape.shape, &mut f.text);
    }
    frame
}

#[test]
fn monitor_body_draws_only_current_admitted_source_and_actual_texture_namespace_without_rejuvenating_age() {
    use super::super::private_image::{PrivateImage, PrivateImageSlot, UiPixelBudget};
    let mut f = Frames::new([440.0, 360.0], 1.0);
    f.states[0].chat = false;
    let source = f.model.binding(View::Fixed1).unwrap().source;
    let mut admitted = AdmittedFixture::new(source);
    let budget = UiPixelBudget::new();
    let mut slot = PrivateImageSlot::new(f.contexts[1].clone(), 7, budget.clone()).unwrap();
    slot.stage(7, PrivateImage::from_rgba(&budget, admitted.size, &admitted.rgba).unwrap())
        .unwrap();
    let frame = draw_private(&mut f, 1, &mut slot, &admitted.admitted);
    let primitives = f.contexts[1].tessellate(frame.output().shapes.clone(), frame.output().pixels_per_point);
    assert!(primitives.iter().any(|primitive| matches!(&primitive.primitive, egui::epaint::Primitive::Mesh(mesh) if matches!(mesh.texture_id, egui::TextureId::User(_)))));
    let canvas = f.contexts[1]
        .data(|data| data.get_temp::<Rect>(control_id(View::Fixed1, "image")))
        .unwrap();
    let fit = f.contexts[1]
        .data(|data| data.get_temp::<Rect>(control_id(View::Fixed1, "image_fitted")))
        .unwrap();
    assert!(canvas.contains_rect(fit) && fit.is_positive());
    assert!((fit.width() / fit.height() - 16.0 / 9.0).abs() < 0.01);
    drop(frame);
    let original_age = admitted.admitted.age();
    // Each synthetic receipt arrives before the previous authority expires.
    // Repainting or rereading the original receipt cannot bridge a five-second gap.
    for received_at in (505..=5005).step_by(500) {
        admitted.observe_policy(received_at);
        assert!(admitted.admitted.current(received_at));
        assert_eq!(admitted.admitted.age(), original_age);
    }
    f.now = 5010;
    let frame = draw_private(&mut f, 1, &mut slot, &admitted.admitted);
    assert!(
        f.text
            .iter()
            .any(|text| text.contains(&aurora_i18n::tr!("host-monitor-last-active", youngest = 5_i64, oldest = 6_i64))),
        "same admitted image keeps original request-correlated age"
    );
    drop(frame);
    // Metadata stays current, but a foreign surface namespace may not lend its texture.
    f.bodies[0].generation += 1;
    let frame = draw_private(&mut f, 1, &mut slot, &admitted.admitted);
    assert!(f.has("host-monitor-no-image"));
    let primitives = f.contexts[1].tessellate(frame.output().shapes.clone(), frame.output().pixels_per_point);
    assert!(!primitives.iter().any(|primitive| matches!(&primitive.primitive, egui::epaint::Primitive::Mesh(mesh) if matches!(mesh.texture_id, egui::TextureId::User(_)))));
    drop(frame);
    f.bodies[0].generation -= 1;
    f.bodies[0].binding_generation += 1;
    let frame = draw_private(&mut f, 1, &mut slot, &admitted.admitted);
    assert!(f.has("host-monitor-no-image"), "actual owner binding namespace must match");
    drop(frame);
    f.bodies[0].binding_generation -= 1;
    f.bodies[0].context = f.contexts[0].clone();
    let frame = draw_private(&mut f, 1, &mut slot, &admitted.admitted);
    assert!(f.has("host-monitor-no-image"), "another egui context cannot lend a private texture");
    drop(frame);
    f.bodies[0].context = f.contexts[1].clone();
    f.model.close(View::Fixed1).unwrap();
    f.model
        .open_fixed(Source::new(SlotId::try_from(1).unwrap(), self::source(2), true).unwrap())
        .unwrap();
    let frame = draw_private(&mut f, 1, &mut slot, &admitted.admitted);
    assert!(
        f.has("host-monitor-no-image"),
        "old admitted source cannot paint a replacement binding"
    );
    drop(frame);
    slot.retire();
    assert_eq!(budget.used(), 0, "fixture owner releases the actual credited private pixels");
}

#[test]
fn monitor_bound_facade_clears_latent_ui_read_on_mismatch_and_preserves_queued_operation_and_canonical_draft() {
    for queued in [false, true] {
        let mut f = Frames::new([680.0, 680.0], 1.0);
        f.type_text(1, "Canonical bound draft");
        let context = f.contexts[1].clone();
        let palette = crate::theme::Theme::default().palette();
        let surface = View::Fixed1.surface();
        let bound = |f: &mut Frames, expected, events| {
            f.frame += 1;
            f.chat.sync(&f.host, false);
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(680.0, 680.0))),
                time: Some(f.frame as f64 / 60.0),
                focused: true,
                events,
                ..Default::default()
            };
            let mut output = context.run_ui(raw, |ui| {
                assert!(f.chat.show_bound_for(surface, expected, ui, &palette, InputMode::Live).is_none());
            });
            output.textures_delta.clear();
        };
        for _ in 0..3 {
            bound(&mut f, source(1), Vec::new());
        }
        let at = context
            .data(|data| data.get_temp::<Rect>(Id::unique("host_chat_read_rect")))
            .unwrap()
            .center();
        for pressed in [true, false] {
            bound(
                &mut f,
                source(1),
                vec![
                    Event::PointerMoved(at),
                    Event::PointerButton {
                        pos: at,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ],
            );
        }
        if queued {
            let read = f.chat.take_read_action_for(surface).expect("fresh explicit source-owned Read");
            f.chat.queued_read(read, 77);
        }
        bound(&mut f, source(2), Vec::new());
        assert_eq!(
            f.chat.selected_source_for(surface),
            Some(source(1)),
            "mismatch cannot select another source"
        );
        assert!(
            f.chat.take_read_action_for(surface).is_none(),
            "stale UI Read cannot leak into a later pass"
        );
        for _ in 0..3 {
            bound(&mut f, source(1), Vec::new());
        }
        f.settle(1);
        if queued {
            assert!(f.has("host-chat-read-pending"), "canonical queued operation is not discarded");
        }
        assert_eq!(f.send(1).text.as_str(), "Canonical bound draft");
    }
}

#[test]
fn monitor_body_canonical_ime_commits_once_and_late_composition_cannot_cross_cycling_source() {
    let mut f = Frames::new([680.0, 680.0], 1.0);
    f.type_text(1, "Committed prefix");
    let editor_focus = f.contexts[1]
        .memory(|memory| memory.focused())
        .expect("real canonical editor focus");
    assert!(
        f.show(
            1,
            vec![Event::Ime(egui::ImeEvent::Preedit {
                text: "候補".into(),
                active_range_chars: None
            })]
        )
        .is_none()
    );
    assert_eq!(
        f.contexts[1].memory(|memory| memory.focused()),
        Some(editor_focus),
        "toolbar IME fence preserves canonical TextEdit focus"
    );
    f.focus(2);
    assert_eq!(
        f.send(2).text.as_str(),
        "Committed prefix",
        "uncommitted candidate stays with its source-owned editor"
    );
    f.composer(2);
    assert!(
        f.show(
            2,
            vec![Event::Ime(egui::ImeEvent::Preedit {
                text: "候補".into(),
                active_range_chars: None
            })]
        )
        .is_none()
    );
    assert!(f.show(2, vec![Event::Ime(egui::ImeEvent::Commit("確定".into()))]).is_none());
    assert_eq!(
        f.send(0).text.as_str(),
        "Committed prefix確定",
        "true Commit is canonical across three contexts"
    );
    f.composer(2);
    assert!(
        f.show(
            2,
            vec![Event::Ime(egui::ImeEvent::Preedit {
                text: "古い候補".into(),
                active_range_chars: None
            })]
        )
        .is_none()
    );
    let next = f.model.capture_navigation(Direction::Next).unwrap();
    f.model.navigate(next).unwrap(); // Parent already delivered old edits; source transition remains outside body.
    f.chat.select_for(View::Cycling.surface(), source(2));
    assert!(
        f.show(
            2,
            vec![
                Event::Ime(egui::ImeEvent::Commit("old commit".into())),
                Event::Text("queued text".into())
            ]
        )
        .is_none()
    );
    f.settle(2);
    f.type_text(2, "Fresh second source");
    let send = f.send(2);
    assert_eq!(send.key.source, source(2));
    assert_eq!(send.text.as_str(), "Fresh second source");
    assert_eq!(f.send(0).text.as_str(), "Committed prefix確定");
}

#[test]
fn monitor_body_blockers_invalidate_held_local_controls_and_unavailable_cycle_can_recover_locally() {
    let mut f = Frames::new([440.0, 592.0], 1.0);
    f.focus(2);
    f.states[1].chat = false;
    let view = View::Cycling;
    let at = f.rect(2, control_id(view, "play")).center();
    assert!(f.pointer(2, at, true).is_none());
    f.model.set_blocked(true).unwrap();
    f.settle(2);
    f.model.set_blocked(false).unwrap();
    assert!(f.pointer(2, at, false).is_none(), "unblocking never revives the held local action");
    f.host.chat.remove(0);
    f.host.workers.remove(0);
    f.sync_sources();
    f.settle(2);
    assert!(!f.model.binding(view).unwrap().enabled);
    assert!(
        matches!(
            f.click(2, control_id(view, "play")),
            Some(Action::Control {
                command: Command::Play,
                ..
            })
        ),
        "unavailable paused view still permits explicit local recovery"
    );
    assert!(f.click(2, control_id(view, "options")).is_none());
    let scroll = f.rect(2, control_id(view, "options_scroll"));
    for _ in 0..3 {
        assert!(
            f.show(
                2,
                vec![
                    Event::PointerMoved(scroll.center()),
                    Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        source: egui::MouseWheelSource::Wheel,
                        delta: vec2(0.0, -1000.0),
                        modifiers: Modifiers::NONE,
                    }
                ]
            )
            .is_none()
        );
        f.settle(2);
    }
    let button = f.rect(2, control_id(view, "apply_interval"));
    assert!(
        scroll.contains(button.center()),
        "actual interval control scrolls into its viewport"
    );
    assert!(matches!(
        f.click(2, control_id(view, "apply_interval")),
        Some(Action::Control {
            command: Command::Interval(10),
            ..
        })
    ));
    assert!(
        f.model.capture(view).is_none(),
        "local recovery grants no source/native/chat authority"
    );
}

#[test]
fn monitor_body_accessibility_uses_committed_exact_binding_and_edit_prefix_keeps_canonical_owner() {
    let mut f = Frames::new([680.0, 680.0], 1.0);
    f.type_text(1, "Before flush");
    assert!(
        f.show_image(1, vec![Event::Text(" + delivered".into())], None, InputMode::Editing)
            .is_none()
    );
    f.settle(1);
    assert_eq!(f.send(0).text.as_str(), "Before flush + delivered");
    f.focus(1);
    let control = *f.bodies[0]
        .controls
        .iter()
        .find(|control| matches!(control.intent.command, Command::Exchange(_)))
        .unwrap();
    f.contexts[1].memory_mut(|memory| memory.request_focus(control.widget));
    f.settle(1);
    let click = || {
        Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: control.widget.accesskit_id(),
            data: None,
        })
    };
    assert!(
        matches!(f.show(1,vec![click()]),Some(Action::Control{command:Command::Exchange(ticket),..}) if ticket==match control.intent.command{Command::Exchange(ticket)=>ticket,_=>unreachable!()})
    );
    f.model.set_blocked(true).unwrap();
    assert!(f.show(1, vec![click()]).is_none());
    f.model.set_blocked(false).unwrap();
    assert!(
        f.show(1, vec![click()]).is_none(),
        "a revoked painted accessibility identity cannot revive before a fresh paint"
    );
    f.settle(1);
    assert_eq!(f.send(2).text.as_str(), "Before flush + delivered");
}

#[test]
fn monitor_native_retirement_flush_keeps_delivered_edit_prefix_and_undo_on_original_canonical_source() {
    let mut f = Frames::new([680.0, 680.0], 1.0);
    f.type_text(1, "Before retirement");
    // type_text already focused the real editor. Another rapid mouse click can
    // select its last word; explicitly establish the append caret with a key.
    assert!(
        f.show(1, vec![key(Key::End, true, Modifiers::NONE), key(Key::End, false, Modifiers::NONE)])
            .is_none()
    );
    let view = View::Fixed1;
    let old = f.model.binding(view).unwrap();
    let context = f.contexts[1].clone();
    f.frame += 120;
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(680.0, 680.0))),
        time: Some(f.frame as f64 / 60.0),
        focused: true,
        events: vec![
            Event::Text(" + delivered".into()),
            Event::PointerButton {
                pos: Pos2::ZERO,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
            Event::Text(" forbidden tail".into()),
            Event::Ime(egui::ImeEvent::Commit(" forbidden commit".into())),
            key(Key::Enter, true, Modifiers::NONE),
        ],
        ..Default::default()
    };
    super::super::monitor_edits::close(&context, raw, view, old, &crate::theme::Theme::default().palette(), &mut f.chat);
    assert!(
        f.chat.take_read_action_for(view.surface()).is_none(),
        "Closing cannot leave a later Read intent"
    );
    f.model.close(view).unwrap();
    f.model
        .open_fixed(Source::new(SlotId::try_from(1).unwrap(), source(2), true).unwrap())
        .unwrap();
    f.chat.select_for(view.surface(), source(2));
    f.settle(1);
    assert_eq!(
        f.send(0).text.as_str(),
        "Before retirement + delivered",
        "prefix stayed with the old source after its owner dropped"
    );
    f.composer(2);
    assert!(
        f.show(2, vec![key(Key::Z, true, command_z()), key(Key::Z, false, command_z())])
            .is_none()
    );
    assert_eq!(
        f.send(0).text.as_str(),
        "Before retirement",
        "third context retains the same canonical Undo transaction"
    );
    f.type_text(1, "Replacement draft");
    assert_eq!(f.send(1).key.source, source(2));
    assert_eq!(f.send(1).text.as_str(), "Replacement draft");
}

#[test]
fn monitor_native_drained_ime_prefix_retires_toolbar_confirmation_but_preserves_canonical_commit_once() {
    let mut f = Frames::new([680.0, 680.0], 1.0);
    f.settle(1);
    let exchange = *f.bodies[0]
        .controls
        .iter()
        .find(|control| matches!(control.intent.command, Command::Exchange(_)))
        .unwrap();
    f.contexts[1].memory_mut(|memory| memory.request_focus(exchange.widget));
    assert!(f.bodies[0].command_focused(&f.contexts[1]));
    assert!(
        f.show(
            1,
            vec![Event::Ime(egui::ImeEvent::Preedit {
                text: "candidate".into(),
                active_range_chars: None
            })]
        )
        .is_none()
    );
    // Native edit-prefix handling observes the ORIGINAL terminal event before
    // the canonical Editing pass consumes it. It never replays Commit to body.
    let prefix = vec![Event::Ime(egui::ImeEvent::Commit("candidate".into()))];
    assert!(f.bodies[0].observe_ime(&prefix));
    assert!(!f.bodies[0].command_focused(&f.contexts[1]));
    assert!(f.show(1, vec![key(Key::Enter, true, Modifiers::NONE)]).is_none());
    assert!(f.show(1, vec![key(Key::Enter, false, Modifiers::NONE)]).is_none());
    f.composer(1);
    let focus = f.contexts[1].memory(|memory| memory.focused()).unwrap();
    assert!(!f.bodies[0].command_focused(&f.contexts[1]));
    assert!(
        f.show(
            1,
            vec![Event::Ime(egui::ImeEvent::Preedit {
                text: "accepted".into(),
                active_range_chars: None
            })]
        )
        .is_none()
    );
    let prefix = vec![Event::Ime(egui::ImeEvent::Commit("accepted".into()))];
    assert!(f.bodies[0].observe_ime(&prefix));
    assert_eq!(f.contexts[1].memory(|memory| memory.focused()), Some(focus));
    assert!(f.show_image(1, prefix, None, InputMode::Editing).is_none());
    assert!(
        f.show_image(1, vec![key(Key::Enter, true, Modifiers::NONE)], None, InputMode::Navigating)
            .is_none()
    );
    assert!(f.show(1, vec![key(Key::Enter, false, Modifiers::NONE)]).is_none());
    assert_eq!(
        f.send(0).text.as_str(),
        "accepted",
        "original Commit reached the canonical source exactly once"
    );
    f.focus(1);
    f.settle(1);
    f.contexts[1].memory_mut(|memory| memory.request_focus(exchange.widget));
    assert!(f.show(1, vec![key(Key::Enter, true, Modifiers::NONE)]).is_none());
    assert!(
        matches!(
            f.show(1, vec![key(Key::Enter, false, Modifiers::NONE)]),
            Some(Action::Control {
                command: Command::Exchange(_),
                ..
            })
        ),
        "fresh explicit command focus remains usable"
    );
}

#[test]
fn monitor_body_ime_terminal_retires_only_owned_command_focus_before_later_confirmation_keys() {
    fn focus_exchange(f: &mut Frames) -> Painted {
        f.settle(1);
        let control = *f.bodies[0]
            .controls
            .iter()
            .find(|control| matches!(control.intent.command, Command::Exchange(_)))
            .unwrap();
        f.contexts[1].memory_mut(|memory| memory.request_focus(control.widget));
        f.settle(1);
        assert_eq!(f.contexts[1].memory(|memory| memory.focused()), Some(control.widget));
        control
    }
    fn fresh_exchange(f: &mut Frames) {
        f.settle(1);
        let control = *f.bodies[0]
            .controls
            .iter()
            .find(|control| matches!(control.intent.command, Command::Exchange(_)))
            .unwrap();
        for _ in 0..12 {
            if f.contexts[1].memory(|memory| memory.focused()) == Some(control.widget) {
                break;
            }
            assert!(
                f.show(1, vec![key(Key::Tab, true, Modifiers::NONE), key(Key::Tab, false, Modifiers::NONE)])
                    .is_none()
            );
            f.settle(1);
        }
        assert_eq!(
            f.contexts[1].memory(|memory| memory.focused()),
            Some(control.widget),
            "fresh explicit Tab navigation restores command focus"
        );
        assert!(f.show(1, vec![key(Key::Enter, true, Modifiers::NONE)]).is_none());
        assert!(
            matches!(f.show(1,vec![key(Key::Enter,false,Modifiers::NONE)]),Some(Action::Control{command:Command::Exchange(ticket),..}) if ticket.source().chat()==control.intent.binding.source.chat())
        );
    }
    let mut f = Frames::new([440.0, 560.0], 1.0);
    f.states[0].chat = false;
    focus_exchange(&mut f);
    assert!(
        f.show(
            1,
            vec![Event::Ime(egui::ImeEvent::Preedit {
                text: "候補".into(),
                active_range_chars: None
            })]
        )
        .is_none()
    );
    assert!(f.contexts[1].memory(|memory| memory.focused()).is_none());
    assert!(f.show(1, vec![Event::Ime(egui::ImeEvent::Commit("確定".into()))]).is_none());
    assert!(
        f.show(1, vec![key(Key::Enter, true, Modifiers::NONE)]).is_none(),
        "confirmation may follow Commit in a later frame"
    );
    assert!(f.show(1, vec![key(Key::Enter, false, Modifiers::NONE)]).is_none());
    fresh_exchange(&mut f);

    for key_first in [false, true] {
        #[allow(deprecated)]
        let terminals = [
            egui::ImeEvent::Commit("確定".into()),
            egui::ImeEvent::Preedit {
                text: String::new(),
                active_range_chars: None,
            },
            egui::ImeEvent::Disabled,
        ];
        for terminal in terminals {
            let mut f = Frames::new([440.0, 560.0], 1.0);
            f.states[0].chat = false;
            focus_exchange(&mut f);
            // The published base/input can change before this render. Its old
            // ticket is pruned, but the old owned focus identity still retires.
            f.states[0].revision += 1;
            f.states[0].failed = true;
            f.model.cancel_input().unwrap();
            let terminal = Event::Ime(terminal);
            let down = key(Key::Enter, true, Modifiers::NONE);
            let events = if key_first { vec![down, terminal] } else { vec![terminal, down] };
            assert!(
                f.show(1, events).is_none(),
                "same-frame terminal/key order cannot activate a new published base"
            );
            assert!(f.contexts[1].memory(|memory| memory.focused()).is_none());
            assert!(f.show(1, vec![key(Key::Enter, false, Modifiers::NONE)]).is_none());
            fresh_exchange(&mut f);
        }
    }
}
