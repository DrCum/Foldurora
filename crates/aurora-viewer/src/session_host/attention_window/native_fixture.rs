//! Private ignored X11 fixture: real AttentionWindow input/rendering, synthetic
//! body-free authority only. No HostController, persistence, workers or World.

use super::*;
use crate::session_host::attention_prefs::{PreferenceOperation, Preferences};
use crate::session_host::attention_ui::native_fixture_host;
use crate::session_host::slots::SlotId;
use serde::Deserialize;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event_loop::{ControlFlow, EventLoop};
use winit::platform::x11::EventLoopBuilderExtX11;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

const MAX_STATE_BYTES: usize = 512 * 1024;
const MAX_AUTHORITIES: usize = 64;
const MAX_AUTHORITY_BYTES: usize = 1536;
const OBSERVATION_FIELDS: [&str; 8] = [
    "selected_row_authority",
    "open_ticket_authority",
    "open_paint_frame",
    "flags",
    "prior_focus_losses",
    "prior_intents",
    "cached_controls",
    "save_authority",
];
const OBSERVATION_FLAGS: [&str; 20] = [
    "selected_valid",
    "open_focused",
    "native_window_focused",
    "blocked",
    "modifiers_none",
    "writable",
    "stopped",
    "save_pending",
    "ime_composing",
    "popup_open",
    "gesture",
    "open_live_valid",
    "open_ready",
    "open_painted_valid",
    "open_painted_selected",
    "open_painted_live_enabled",
    "eligible",
    "filters_open",
    "save_failed",
    "open_painted_present",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Control {
    serial: u64,
    phase: String,
    command: Command,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Observe,
    Block { blocked: bool },
    ReplaceSource { slot: u8 },
    RevisePolicy { slot: u8 },
    Incoming { slot: u8 },
    CompletePreferences { success: bool },
    Reopen,
    Finish,
}

fn private(path: &Path, directory: bool) {
    let metadata = path.symlink_metadata().unwrap();
    // SAFETY: geteuid only reads this process's effective user ID.
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
        _ => panic!("fixture requires a separately owned X11 server"),
    }
}

struct Fixture {
    directory: PathBuf,
    title: String,
    snapshot: HostSnapshot,
    model: Attention,
    body: AttentionUi,
    owner: Option<Window>,
    attention: Option<AttentionWindow>,
    started: Instant,
    serial: u64,
    phase: String,
    blocked: bool,
    modifiers: ModifiersState,
    events: Vec<Value>,
    awaiting_activation_paint: Vec<usize>,
    observation_authorities: RefCell<Vec<Value>>,
    intents: Vec<Value>,
    pending_preferences: Option<(u64, Preferences)>,
    preference_ticket: u64,
    focus_losses: usize,
    closes: usize,
    reopens: usize,
    finished: bool,
}

impl Fixture {
    fn new(directory: PathBuf) -> Self {
        let snapshot = native_fixture_host();
        // Mapping is always slot 1..5; Nearby stays excluded by real defaults.
        assert_eq!(snapshot.chat.len(), 5);
        let mut fixture = Self {
            title: format!("Test attention-native-os [AUTOMATION - DO NOT TOUCH] {}", std::process::id()),
            directory,
            snapshot,
            model: Attention::default(),
            body: AttentionUi::default(),
            owner: None,
            attention: None,
            started: Instant::now(),
            serial: 0,
            phase: "startup".into(),
            blocked: true,
            modifiers: ModifiersState::empty(),
            events: Vec::new(),
            awaiting_activation_paint: Vec::new(),
            observation_authorities: RefCell::new(Vec::new()),
            intents: Vec::new(),
            pending_preferences: None,
            preference_ticket: 0,
            focus_losses: 0,
            closes: 0,
            reopens: 0,
            finished: false,
        };
        fixture.sync();
        fixture
    }

    fn authority(&self, kind: &str, value: Option<Value>) -> Option<usize> {
        let value = value.filter(|value| !value.is_null())?;
        let authority = json!({"kind": kind, "value": value});
        assert!(serde_json::to_vec(&authority).unwrap().len() <= MAX_AUTHORITY_BYTES);
        let mut authorities = self.observation_authorities.borrow_mut();
        if let Some(index) = authorities.iter().position(|existing| *existing == authority) {
            return Some(index);
        }
        assert!(authorities.len() < MAX_AUTHORITIES, "bounded native authority trace exhausted");
        let index = authorities.len();
        authorities.push(authority);
        Some(index)
    }

    fn sync(&mut self) {
        self.model.sync(&self.snapshot, &self.snapshot.attention_preferences.preferences);
        self.body.sync(&self.model, &self.snapshot.attention_preferences);
        // These synthetic observations must never acquire native mutations.
        assert!(
            self.snapshot
                .chat
                .iter()
                .all(|chat| chat.lines.is_empty() && chat.receipts.is_empty() && chat.read_operations.is_empty())
        );
    }

    fn open(&mut self, event_loop: &ActiveEventLoop) {
        assert!(self.attention.is_none());
        let window = AttentionWindow::new(event_loop, self.owner.as_ref().unwrap()).unwrap();
        window.surface.window.set_title(&self.title);
        self.attention = Some(window);
    }

    fn observation(&self) -> Value {
        let window = self.attention.as_ref();
        let focused = window.is_some_and(AttentionWindow::focused);
        let selected = self.model.selected();
        let selected_valid = selected.is_some_and(|key| self.model.capture(key).is_some());
        let selected_row = selected.map(|key| {
            self.model.rows().iter().find(|row| row.key == key).map_or_else(
                || json!({"key": key, "policy": null, "read": null, "incarnation": null}),
                |row| json!({"key": row.key, "policy": row.policy, "read": row.read, "incarnation": row.id()}),
            )
        });
        let open_focused = window.is_some_and(|window| {
            let id = egui::Id::unique("attention_open");
            let widget = window.context.data(|data| data.get_temp::<egui::Id>(id.with("widget")));
            widget.is_some() && widget == window.context.memory(|memory| memory.focused())
        });
        let ui = self.body.native_fixture_status(&self.model, &self.snapshot.attention_preferences);
        let writable = self.snapshot.attention_preferences.writable;
        let stopped = self.snapshot.stopped;
        let popup = window.is_some_and(|window| egui::Popup::is_any_open(&window.context));
        let eligible = focused
            && open_focused
            && selected_valid
            && !self.blocked
            && writable
            && !stopped
            && ui["save"].is_null()
            && ui["open_ready"] == true
            && ui["ime_composing"] == false
            && self.modifiers.is_empty()
            && !popup;
        let flags = [
            selected_valid,
            open_focused,
            focused,
            self.blocked,
            self.modifiers.is_empty(),
            writable,
            stopped,
            !ui["save"].is_null(),
            ui["ime_composing"] == true,
            popup,
            ui["gesture"] == true,
            ui["open_live_valid"] == true,
            ui["open_ready"] == true,
            ui["open_painted"]["valid"] == true,
            ui["open_painted"]["selected"] == true,
            ui["open_painted"]["live_enabled"] == true,
            eligible,
            ui["filters_open"] == true,
            ui["save_failed"] == true,
            !ui["open_painted"].is_null(),
        ]
        .into_iter()
        .enumerate()
        .fold(0u32, |mask, (bit, on)| mask | (u32::from(on) << bit));
        json!([
            self.authority("selected_row", selected_row),
            self.authority("open_ticket", ui["open_painted"].get("ticket").cloned()),
            ui["open_painted"]["frame"],
            flags,
            self.focus_losses,
            self.intents.len(),
            ui["controls"],
            self.authority("save", Some(ui["save"].clone()))
        ])
    }

    fn state(&self) -> Value {
        let attention = self.attention.as_ref().map(|window| {
            let context = &window.context;
            let control = |id: egui::Id| {
                let rect = context
                    .data(|data| data.get_temp::<egui::Rect>(id.with("rect")))
                    .map(|rect| [rect.min.x, rect.min.y, rect.max.x, rect.max.y]);
                let widget = context.data(|data| data.get_temp::<egui::Id>(id.with("widget")));
                json!({"rect": rect, "focused": widget.is_some() && widget == context.memory(|memory| memory.focused())})
            };
            let rows: Vec<_> = self
                .model
                .rows()
                .iter()
                .map(|row| {
                    let id = chat_ui::source_id(row.key.source).with(("attention_row", row.key.conversation, row.id()));
                    json!({"key": row.key, "incarnation": row.id(), "slot": u8::from(row.slot),
                    "policy": row.policy, "read": row.read, "control": control(id)})
                })
                .collect();
            let slots: Vec<_> = SlotId::all()
                .map(|slot| {
                    json!({"slot": u8::from(slot),
                "control": control(egui::Id::unique(("attention_slot", slot)))})
                })
                .collect();
            json!({"window": window_id(&window.surface.window), "title": self.title,
                "client_origin": window.surface.window.inner_position().ok().map(|p| [p.x, p.y]),
                "pixels_per_point": context.pixels_per_point(), "focused": window.focused(),
                "frame": context.cumulative_frame_nr(), "painted": window.last_frame.is_some(),
                "popup_open": egui::Popup::is_any_open(context), "rows": rows, "slots": slots,
                "controls": {"open": control(egui::Id::unique("attention_open")),
                    "dismiss": control(egui::Id::unique("attention_dismiss")),
                    "previous": control(egui::Id::unique(("attention_action", "previous"))),
                    "next": control(egui::Id::unique(("attention_action", "next"))),
                    "filters": control(egui::Id::unique("attention_filter_toggle")),
                    "nearby": control(egui::Id::unique(("attention_kind", "nearby"))),
                    "direct": control(egui::Id::unique(("attention_kind", "direct")))}})
        });
        let reads: Vec<_> = self
            .snapshot
            .chat
            .iter()
            .map(|chat| {
                let reads: Vec<_> = chat
                    .reads
                    .iter()
                    .map(|(conversation, read)| json!({"conversation": conversation, "read": read}))
                    .collect();
                json!({"source": chat.source, "reads": reads,
                "read_operations": chat.read_operations.len(), "receipts": chat.receipts.len()})
            })
            .collect();
        let rows: Vec<_> = self
            .model
            .rows()
            .iter()
            .map(|row| {
                json!({"key": row.key,
            "incarnation": row.id(), "policy": row.policy, "read": row.read})
            })
            .collect();
        json!({"fixture": "synthetic policy / native Linux AttentionWindow CPU input only",
            "source_version": aurora_version::version(), "pid": std::process::id(), "phase": self.phase, "serial": self.serial,
            "display": std::env::var("DISPLAY").ok(), "xauthority": std::env::var("XAUTHORITY").ok(),
            "owner": self.owner.as_ref().map(|window| json!({"window": window_id(window), "title": format!("{} owner", self.title)})),
            "attention": attention, "selected": self.model.selected(), "rows": rows,
            "preferences": self.snapshot.attention_preferences.preferences,
            "ui": self.body.native_fixture_status(&self.model, &self.snapshot.attention_preferences),
            "blocked": self.blocked, "events": self.events, "intents": self.intents, "native_reads": reads,
            "observation_fields": OBSERVATION_FIELDS, "observation_flags": OBSERVATION_FLAGS,
            "observation_authorities": self.observation_authorities.borrow().clone(),
            "focus_losses": self.focus_losses, "closes": self.closes, "reopens": self.reopens, "finished": self.finished,
            "preference_io": false, "host_controller": false, "authentication_acceptance": false,
            "grid_or_world_submit": false, "ime_acceptance": false})
    }

    fn publish(&self) {
        let bytes = serde_json::to_vec(&self.state()).unwrap();
        assert!(bytes.len() < MAX_STATE_BYTES);
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

    fn action(&mut self, action: Action) {
        assert!(self.intents.len() < 64);
        let before = self.snapshot.chat.iter().map(|chat| chat.reads.clone()).collect::<Vec<_>>();
        let intent = match action {
            Action::OpenChat(ticket) => {
                let captured = ticket.key();
                let row = self.model.rows().iter().find(|row| row.key == captured).unwrap();
                let incarnation = row.id();
                let capture = self.authority("open_ticket", Some(ticket.native_fixture_status()));
                let opened = self.model.open_chat(ticket);
                assert_eq!(opened, Some(captured));
                json!({"kind": "open_chat", "capture": capture, "incarnation": incarnation, "validated": true})
            }
            Action::Dismiss(ticket) => {
                let captured = ticket.key();
                let row = self.model.rows().iter().find(|row| row.key == captured).unwrap();
                let incarnation = row.id();
                let capture = self.authority("open_ticket", Some(ticket.native_fixture_status()));
                assert!(self.model.dismiss(ticket));
                json!({"kind": "dismiss", "capture": capture, "incarnation": incarnation, "local_only": true})
            }
            Action::Preferences(preferences) => {
                assert!(self.pending_preferences.is_none());
                self.preference_ticket += 1;
                self.pending_preferences = Some((self.preference_ticket, preferences));
                self.body.queued(self.preference_ticket, preferences);
                json!({"kind": "preferences", "ticket": self.preference_ticket, "preferences": preferences,
                    "queued_only": true, "published": self.snapshot.attention_preferences.preferences})
            }
        };
        assert_eq!(before, self.snapshot.chat.iter().map(|chat| chat.reads.clone()).collect::<Vec<_>>());
        self.intents.push(json!({"phase": self.phase, "intent": intent}));
        self.sync();
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
        assert!(
            control.phase.len() <= 64
                && control
                    .phase
                    .chars()
                    .all(|c| (c.is_ascii_graphic() || c == ' ') && !matches!(c, '"' | '\\'))
        );
        self.serial = control.serial;
        self.phase = control.phase;
        match control.command {
            Command::Observe => {}
            Command::Block { blocked } => self.blocked = blocked,
            Command::ReplaceSource { slot } | Command::RevisePolicy { slot } | Command::Incoming { slot } => {
                assert!((1..=5).contains(&slot));
                let index = usize::from(slot - 1);
                let chat = Arc::make_mut(&mut self.snapshot.chat[index]);
                match control.command {
                    Command::ReplaceSource { .. } => {
                        chat.source.session.generation += 1;
                        for info in &mut chat.conversations {
                            info.policy.key.source = chat.source;
                        }
                        self.snapshot.workers[index].binding = chat.source.binding();
                    }
                    Command::RevisePolicy { .. } => {
                        for info in &mut chat.conversations {
                            info.policy.revision += 1;
                        }
                    }
                    Command::Incoming { .. } => {
                        for read in chat.reads.values_mut() {
                            read.unread += 1;
                            read.unread_revision += 1;
                            read.observed.through += 1;
                        }
                    }
                    _ => unreachable!(),
                }
            }
            Command::CompletePreferences { success } => {
                let (ticket, preferences) = self.pending_preferences.take().expect("an actual queued preference intent");
                let state = &mut self.snapshot.attention_preferences;
                let error = (!success).then(|| "Synthetic save failure".to_owned());
                state.operation = Some(PreferenceOperation {
                    ticket,
                    preferences,
                    error: error.clone(),
                });
                state.error = error;
                if success {
                    state.preferences = preferences;
                }
            }
            Command::Reopen => {
                self.open(event_loop);
                self.reopens += 1;
            }
            Command::Finish => {
                // Coverage assertions belong to the external driver and its
                // preserved receipts; a normal exit is never itself acceptance.
                self.finished = true;
                self.publish();
                event_loop.exit();
            }
        }
        self.sync();
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
        self.sync();
        let is_attention = self.attention.as_ref().is_some_and(|window| window.id() == id);
        let activation = is_attention
            && matches!(&event, WindowEvent::KeyboardInput { event, .. }
            if matches!(event.logical_key, Key::Named(NamedKey::Enter | NamedKey::Space)));
        let paint_before = matches!(event, WindowEvent::RedrawRequested).then(|| self.observation());
        let prior_frame = self.attention.as_ref().and_then(|window| window.last_frame);
        let data = match &event {
            WindowEvent::KeyboardInput { event, is_synthetic, .. } => Some(json!({"kind": "key",
                "key": format!("{:?}", event.logical_key), "state": format!("{:?}", event.state),
                "repeat": event.repeat, "synthetic": is_synthetic, "before": self.observation()})),
            WindowEvent::Focused(focused) => {
                if is_attention && !focused {
                    self.focus_losses += 1;
                }
                Some(json!({"kind": "focus", "focused": focused}))
            }
            WindowEvent::MouseInput { state, button, .. } => Some(json!({"kind": "mouse",
                "state": format!("{state:?}"), "button": format!("{button:?}")})),
            WindowEvent::ModifiersChanged(modifiers) => {
                if is_attention {
                    self.modifiers = modifiers.state();
                }
                Some(json!({"kind": "modifiers", "value": format!("{:?}", modifiers.state())}))
            }
            WindowEvent::CloseRequested => Some(json!({"kind": "close"})),
            _ => None,
        };
        if is_attention {
            let (close, action) = self
                .attention
                .as_mut()
                .unwrap()
                .event(event_loop, &event, &mut self.model, &mut self.body, &self.snapshot, self.blocked)
                .unwrap();
            if close {
                for index in std::mem::take(&mut self.awaiting_activation_paint) {
                    self.events[index]["paint_effect"] = json!({"cancelled": "surface_closed"});
                }
                self.attention = None;
                self.closes += 1;
            }
            if let Some(action) = action {
                self.action(action);
            }
        } else if matches!(event, WindowEvent::CloseRequested) {
            panic!("driver closed the owner instead of AttentionWindow");
        }
        if let Some(data) = data {
            assert!(self.events.len() < 512);
            let index = self.events.len();
            self.events
                .push(json!({"phase": self.phase, "attention": is_attention, "event": data,
                "after": self.observation()}));
            if activation {
                self.awaiting_activation_paint.push(index);
            }
        }
        if is_attention && paint_before.is_some() && self.attention.as_ref().and_then(|window| window.last_frame) != prior_frame {
            let after = self.observation();
            let window = self.attention.as_ref().unwrap();
            let frame = window.context.cumulative_frame_nr();
            let native_window = window_id(&window.surface.window);
            for index in std::mem::take(&mut self.awaiting_activation_paint) {
                self.events[index]["paint_effect"] = json!({"frame": frame, "before": paint_before,
                    "after": after, "phase": self.phase, "window": native_window});
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        assert!(self.started.elapsed() < Duration::from_secs(420), "owned native fixture timed out");
        self.control(event_loop);
        self.sync();
        let now = Instant::now();
        if let Some(window) = &mut self.attention {
            window.poll(true, self.blocked, &mut self.body, now);
        }
        self.publish();
        event_loop.set_control_flow(ControlFlow::WaitUntil(now + Duration::from_millis(50)));
    }
}

#[test]
#[ignore = "explicit native X11 CPU input; private display/XDG/output and external XTest driver required"]
fn attention_native_os_fixture() {
    let directory = PathBuf::from(std::env::var_os("AURORA_ATTENTION_NATIVE_DIR").expect("private batch required"));
    assert!(directory.is_absolute() && directory.canonicalize().unwrap() == directory);
    private(&directory, true);
    assert!(
        !directory.join("state.json").exists() && !directory.join("control.json").exists(),
        "preserve preceding evidence"
    );
    let display = std::env::var("DISPLAY").unwrap();
    assert_eq!(std::env::var("AURORA_ATTENTION_NATIVE_DISPLAY").unwrap(), display);
    assert!(display.starts_with(':') && !display[1..].is_empty() && display[1..].chars().all(|c| c.is_ascii_digit()));
    for name in [
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_RUNTIME_DIR",
        "XAUTHORITY",
    ] {
        let path = PathBuf::from(std::env::var_os(name).expect("private fixture environment required"));
        assert!(path.parent() == Some(directory.as_path()) && path.canonicalize().unwrap() == path);
        private(&path, name != "XAUTHORITY");
    }
    let _ = crate::cli::get();
    aurora_i18n::select("en", None);
    fonts::fallback::wait();
    let mut fixture = Fixture::new(directory);
    let event_loop = EventLoop::builder().with_x11().with_any_thread(true).build().unwrap();
    event_loop.run_app(&mut fixture).unwrap();
    assert!(
        fixture.finished,
        "only explicit finish completes the fixture; driver receipts decide acceptance"
    );
}

#[test]
fn attention_native_trace_budget_covers_512_fully_annotated_key_events() {
    let mut fixture = Fixture::new(PathBuf::new());
    fixture.phase = "p".repeat(64);
    let key = fixture.model.rows()[0].key;
    fixture.model.select(key);
    let ticket = fixture.model.capture(key).unwrap();
    assert_eq!(fixture.authority("open_ticket", Some(ticket.native_fixture_status())), Some(0));
    assert_eq!(fixture.authority("open_ticket", Some(ticket.native_fixture_status())), Some(0));
    // Fill every authority slot to its enforced byte ceiling, rather than
    // relying on the small number of captures in the acceptance sequence.
    for index in 1..MAX_AUTHORITIES {
        let mut value = json!({"index": index, "padding": ""});
        let overhead = serde_json::to_vec(&json!({"kind": "budget", "value": value})).unwrap().len();
        value["padding"] = json!("x".repeat(MAX_AUTHORITY_BYTES - overhead));
        fixture.authority("budget", Some(value));
    }
    let observation = json!([63, 63, u64::MAX, (1u32 << OBSERVATION_FLAGS.len()) - 1, 512, 64, 32, 63]);
    for _ in 0..512 {
        fixture.events.push(json!({"phase": fixture.phase, "attention": true,
            "event": {"kind": "key", "key": "Named(Enter)", "state": "Released", "repeat": false,
                "synthetic": false, "before": observation}, "after": observation,
            "paint_effect": {"frame": u64::MAX, "window": u64::MAX, "phase": fixture.phase,
                "before": observation, "after": observation}}));
    }
    for _ in 0..64 {
        let preferences = Preferences {
            include_slots: [false; 5],
            nearby: false,
            direct: false,
            oldest_first: false,
        };
        fixture
            .intents
            .push(json!({"phase": fixture.phase, "intent": {"kind": "preferences",
            "ticket": u64::MAX, "preferences": preferences, "queued_only": true, "published": preferences}}));
    }
    let mut state = fixture.state();
    // No native window is created. Reserve 32 KiB for its five/ten row probes,
    // slots, controls, geometry and current UI state, above the real closed
    // fixture's model/native-read metadata already included by state().
    state["attention"] = json!({"native_window_metadata_reserve": "x".repeat(32 * 1024)});
    let bytes = serde_json::to_vec(&state).unwrap().len();
    assert!(bytes < MAX_STATE_BYTES - 48 * 1024, "512-event state needs {bytes} bytes");
}

#[test]
fn attention_native_trace_keeps_exact_identity_for_a_missing_selection() {
    let mut fixture = Fixture::new(PathBuf::new());
    let mut missing = fixture.model.rows()[0].key;
    missing.source.session.generation += 1;
    assert!(!fixture.model.select(missing));
    let observation = fixture.observation();
    let index = observation[0].as_u64().unwrap() as usize;
    let authorities = fixture.observation_authorities.borrow();
    assert_eq!(authorities[index]["kind"], "selected_row");
    assert_eq!(authorities[index]["value"]["key"], json!(missing));
    assert!(authorities[index]["value"]["policy"].is_null());
    let flags = observation[3].as_u64().unwrap();
    assert_eq!(flags & 1, 0, "missing selection cannot be eligible/current");
    assert!(observation[1].is_null());
}
