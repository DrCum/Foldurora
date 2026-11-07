//! Ignored native Linux CPU-window input fixture. Policies/receipts are synthetic;
//! the real ChatWindow, egui-winit and software painter process only OS input.

use super::*;
use crate::session_host::shared_chat::native_fixture_host;
use crate::session_host::transport::native_fixture::{Inbox, Intent, controller};
use aurora_session::chat::{Conversation, ObservedIncoming, ReadMetadata};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event_loop::{ControlFlow, EventLoop};
use winit::platform::x11::EventLoopBuilderExtX11;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Control {
    serial: u64,
    phase: String,
    blocked: Option<bool>,
    complete: bool,
    reopen: bool,
    replace_source: Option<u8>,
    finish: bool,
}

fn private(path: &Path, directory: bool) {
    let metadata = path.symlink_metadata().unwrap();
    // SAFETY: geteuid reads the current process's effective user ID only.
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    assert!(!metadata.file_type().is_symlink());
    assert_eq!(metadata.is_dir(), directory);
    assert!(directory || metadata.is_file() && metadata.nlink() == 1);
    assert_eq!(metadata.mode() & 0o777, if directory { 0o700 } else { 0o600 });
}

fn window_id(window: &Window) -> u64 {
    match window.window_handle().unwrap().as_raw() {
        RawWindowHandle::Xlib(handle) => handle.window,
        RawWindowHandle::Xcb(handle) => u64::from(handle.window.get()),
        _ => panic!("native fixture requires its isolated X11 server"),
    }
}

struct Fixture {
    directory: PathBuf,
    title: String,
    controller: HostController,
    inbox: Inbox,
    model: SharedChat,
    owner: Option<Window>,
    chat: Option<ChatWindow>,
    started: Instant,
    serial: u64,
    phase: String,
    blocked: bool,
    events: Vec<Value>,
    intents: Vec<Intent>,
    focused_repeats_before_intent: usize,
    focus_losses: usize,
    closes: usize,
    reopens: usize,
    finished: bool,
}

impl Fixture {
    fn open(&mut self, event_loop: &ActiveEventLoop) {
        assert!(self.chat.is_none());
        let chat = ChatWindow::new(event_loop, self.owner.as_ref().unwrap()).unwrap();
        chat.surface.window.set_title(&self.title);
        self.chat = Some(chat);
    }

    fn repeat_observation(&self, is_chat: bool) -> Value {
        let model = self.model.native_fixture_status(self.chat.as_ref().map(|chat| &chat.context));
        let source = model["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["key"]["source"] == model["selected"]);
        let sendable = source.is_some_and(|source| {
            ["focused", "editable", "available", "can_send", "draft_nonempty", "literal_ready"]
                .iter()
                .all(|name| source[*name] == true)
                && source["send_pending"] == false
        });
        let focused = self.chat.as_ref().is_some_and(ChatWindow::focused);
        let shift = self.chat.as_ref().is_some_and(|chat| chat.native_modifiers.shift_key());
        json!({"selected": model["selected"], "selected_source": source,
            "captured_key": source.map(|source| &source["key"]), "native_window_focused": focused,
            "chat_event": is_chat, "blocked": self.blocked, "prior_focus_losses": self.focus_losses,
            "prior_intents": self.intents.len(), "shift_down": shift,
            "eligible": is_chat && focused && !self.blocked && self.focus_losses > 0 && self.intents.is_empty() && !shift && sendable})
    }

    fn publish(&self) {
        let chat = self.chat.as_ref().map(|chat| {
            let context = &chat.context;
            let rect = |name| context.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(name)))
                .map(|rect| [rect.min.x, rect.min.y, rect.max.x, rect.max.y]);
            let sources: Vec<_> = self.controller.snapshot().chat.iter().map(|source| {
                let id = egui::Id::unique(("shared_chat_pick", crate::session_host::chat_ui::source_id(source.source)));
                json!({"source": source.source, "rect": context.data(|data| data.get_temp::<egui::Rect>(id))
                    .map(|rect| [rect.min.x, rect.min.y, rect.max.x, rect.max.y])})
            }).collect();
            let conversations: Vec<_> = self.controller.snapshot().chat[0].conversations.iter().map(|info| {
                let id = egui::Id::unique(("host_chat_pick", info.policy.key.conversation));
                json!({"conversation": info.policy.key.conversation, "rect": context.data(|data| data.get_temp::<egui::Rect>(id))
                    .map(|rect| [rect.min.x, rect.min.y, rect.max.x, rect.max.y])})
            }).collect();
            json!({"window": window_id(&chat.surface.window), "title": self.title,
                "client_origin": chat.surface.window.inner_position().ok().map(|p| [p.x, p.y]),
                "pixels_per_point": context.pixels_per_point(), "focused": chat.focused(),
                "minimized": chat.surface.window.is_minimized(), "composer_focused": self.model.composer_rect(context).is_some(),
                "popup_open": egui::Popup::is_any_open(context), "sources": sources, "conversations": conversations,
                "rects": {"input": rect("host_chat_input_rect"), "send": rect("host_chat_send_rect"),
                    "read": rect("host_chat_read_rect"), "source": rect("shared_chat_combo_rect"), "conversation": rect("host_chat_combo_rect")}})
        });
        let state = json!({"fixture": "synthetic policy data / native Linux ChatWindow CPU input only",
            "source_version": aurora_version::version(), "pid": std::process::id(), "phase": self.phase, "serial": self.serial,
            "display": std::env::var("DISPLAY").unwrap(), "xauthority": std::env::var("XAUTHORITY").unwrap(),
            "owner": self.owner.as_ref().map(|window| json!({"window": window_id(window), "title": format!("{} owner", self.title)})),
            "chat": chat, "blocked": self.blocked, "events": self.events, "intents": self.intents,
            "model": self.model.native_fixture_status(self.chat.as_ref().map(|chat| &chat.context)),
            "focused_repeats_before_intent": self.focused_repeats_before_intent, "focus_losses": self.focus_losses,
            "closes": self.closes, "reopens": self.reopens, "finished": self.finished,
            "synthetic_completion": "RejectedNative only; no native delivery or unread mutation",
            "authentication_acceptance": false, "grid_or_world_submit": false, "ime_acceptance": false});
        let bytes = serde_json::to_vec_pretty(&state).unwrap();
        assert!(bytes.len() < 256 * 1024);
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.directory.join("state.json.next"))
            .unwrap();
        output.write_all(&bytes).unwrap();
        drop(output);
        std::fs::rename(self.directory.join("state.json.next"), self.directory.join("state.json")).unwrap();
    }

    fn control(&mut self, event_loop: &ActiveEventLoop) {
        let path = self.directory.join("control.json");
        if !path.exists() {
            return;
        }
        private(&path, false);
        assert!(path.metadata().unwrap().len() <= 4096);
        let control: Control = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        if control.serial <= self.serial {
            return;
        }
        assert!(control.phase.len() <= 64 && control.phase.chars().all(|c| c.is_ascii_graphic() || c == ' '));
        self.serial = control.serial;
        self.phase = control.phase;
        if let Some(blocked) = control.blocked {
            self.blocked = blocked;
        }
        if control.complete {
            self.inbox.complete(&self.intents);
        }
        if let Some(number) = control.replace_source {
            self.inbox.replace_source(number);
        }
        if control.reopen {
            self.open(event_loop);
            self.reopens += 1;
        }
        if control.finish {
            assert!(
                self.focused_repeats_before_intent > 0,
                "observe an OS repeat in the focused composer before any intent"
            );
            assert!(self.focus_losses > 0 && self.closes > 0 && self.reopens > 0);
            assert!(self.intents.iter().filter(|intent| matches!(intent, Intent::Send { .. })).count() >= 3);
            assert!(self.intents.iter().any(|intent| matches!(intent, Intent::Read { .. })));
            self.finished = true;
            self.publish();
            event_loop.exit();
        }
    }
}

impl ApplicationHandler for Fixture {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        assert!(self.owner.is_none());
        self.owner = Some(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(format!("{} owner", self.title))
                        .with_inner_size(winit::dpi::LogicalSize::new(320.0, 180.0)),
                )
                .unwrap(),
        );
        self.open(event_loop);
        self.publish();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let is_chat = self.chat.as_ref().is_some_and(|chat| chat.id() == id);
        let data = match &event {
            WindowEvent::KeyboardInput { event, is_synthetic, .. } => {
                let observation = (event.repeat && event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter))
                    .then(|| self.repeat_observation(is_chat));
                if observation.as_ref().is_some_and(|observation| observation["eligible"] == true)
                    && event.state == ElementState::Pressed
                    && !*is_synthetic
                {
                    self.focused_repeats_before_intent += 1;
                }
                Some(
                    json!({"kind": "key", "key": format!("{:?}", event.logical_key), "state": format!("{:?}", event.state),
                    "repeat": event.repeat, "synthetic": is_synthetic, "repeat_observation": observation}),
                )
            }
            WindowEvent::ModifiersChanged(modifiers) => Some(json!({"kind": "modifiers", "value": format!("{:?}", modifiers.state())})),
            WindowEvent::Focused(focused) => {
                if is_chat && !focused {
                    self.focus_losses += 1;
                }
                Some(json!({"kind": "focus", "focused": focused}))
            }
            WindowEvent::MouseInput { state, button, .. } => {
                Some(json!({"kind": "mouse", "state": format!("{state:?}"), "button": format!("{button:?}")}))
            }
            WindowEvent::CloseRequested => Some(json!({"kind": "close"})),
            WindowEvent::Ime(event) => Some(json!({"kind": "ime", "value": format!("{event:?}")})),
            _ => None,
        };
        if let Some(data) = data {
            assert!(self.events.len() < 512);
            self.events.push(json!({"phase": self.phase, "chat": is_chat, "event": data}));
        }
        if is_chat {
            if self
                .chat
                .as_mut()
                .unwrap()
                .event(event_loop, &event, &self.controller, &mut self.model, self.blocked)
                .unwrap()
            {
                self.chat = None;
                self.closes += 1;
            }
            self.intents.extend(self.inbox.drain());
            assert!(self.intents.len() <= 64);
        } else if matches!(event, WindowEvent::CloseRequested) {
            panic!("driver closed the owner instead of the ChatWindow");
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        assert!(self.started.elapsed() < Duration::from_secs(420), "owned native fixture timed out");
        self.control(event_loop);
        self.model.sync(&self.controller.snapshot(), false);
        let now = Instant::now();
        if let Some(chat) = &mut self.chat {
            chat.poll(true, self.blocked, now);
        }
        self.publish();
        event_loop.set_control_flow(ControlFlow::WaitUntil(now + Duration::from_millis(50)));
    }
}

#[test]
#[ignore = "explicit native X11 CPU input; private display/XDG/output guards and external XTest driver required"]
fn shared_chat_native_os_fixture() {
    let directory = PathBuf::from(std::env::var_os("AURORA_SHARED_CHAT_NATIVE_DIR").expect("private fixture batch required"));
    assert!(directory.is_absolute() && directory.canonicalize().unwrap() == directory);
    private(&directory, true);
    assert!(
        !directory.join("state.json").exists() && !directory.join("control.json").exists(),
        "preserve preceding evidence"
    );
    let display = std::env::var("DISPLAY").unwrap();
    assert_eq!(std::env::var("AURORA_SHARED_CHAT_NATIVE_DISPLAY").unwrap(), display);
    assert!(display.starts_with(':') && display[1..].chars().all(|c| c.is_ascii_digit()));
    for name in [
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_RUNTIME_DIR",
        "XAUTHORITY",
    ] {
        let path = PathBuf::from(std::env::var_os(name).expect("private fixture environment required"));
        assert!(path.starts_with(&directory) && path.canonicalize().unwrap() == path);
        private(&path, name != "XAUTHORITY");
    }
    let _ = crate::cli::get();
    aurora_i18n::select("en", None);
    fonts::fallback::wait();
    let mut host = native_fixture_host();
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
    model.select(host.chat[0].source);
    let (controller, inbox) = controller(host);
    let mut fixture = Fixture {
        title: format!("Test shared-chat-native-os [AUTOMATION - DO NOT TOUCH] {}", std::process::id()),
        directory,
        controller,
        inbox,
        model,
        owner: None,
        chat: None,
        started: Instant::now(),
        serial: 0,
        phase: "startup".into(),
        blocked: true,
        events: Vec::new(),
        intents: Vec::new(),
        focused_repeats_before_intent: 0,
        focus_losses: 0,
        closes: 0,
        reopens: 0,
        finished: false,
    };
    let event_loop = EventLoop::builder().with_x11().with_any_thread(true).build().unwrap();
    event_loop.run_app(&mut fixture).unwrap();
    assert!(fixture.finished, "only an explicit checked finish completes the fixture");
}
