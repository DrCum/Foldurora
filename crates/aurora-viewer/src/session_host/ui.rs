//! The controller shows acknowledged local sessions only. Chat is offered
//! only for its authenticated live native backend; logins remain manual.

use super::transport::{HostError, HostSnapshot, WorkerSummary};
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind};
use aurora_session::{AccountKey, MAX_WORKERS, Phase, WorkerId};
use egui::{Id, Rect, RichText, TextEdit, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    OpenLogin(Option<AccountKey>),
    Activate(WorkerId),
    Detach(WorkerId),
    OpenChat(aurora_session::chat::ChatSource),
    Close,
}

#[derive(Clone, Copy)]
pub(super) enum Notice {
    Queued,
    Error(HostError),
}

#[derive(Default)]
pub(super) struct HostUi {
    selected: Option<WorkerId>,
    search: String,
    pub close_prompt: bool,
    pub notice: Option<Notice>,
}

pub(super) struct Output {
    pub action: Option<Action>,
    pub world: Rect,
}

impl HostUi {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        snapshot: &HostSnapshot,
        native_ready: bool,
        hosted: Option<WorkerId>,
        closing: bool,
    ) -> Output {
        let mut action = None;
        let ctx = ui.ctx().clone();
        let frame = widgets::overlay_frame(&ctx, p);
        egui::Panel::top("session_host_header")
            .exact_size(58.0)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    widgets::glyph(ui, "users-group", p.violet_pale, 24.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(aurora_i18n::tr!("host-title")).strong().size(18.0));
                        ui.label(RichText::new(aurora_i18n::tr!("host-manual-login-help")).small().color(p.muted));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let close = widgets::action_button(ui, p, "x", aurora_i18n::tr!("host-close"), ActionKind::Flat, vec2(0.0, 30.0));
                        #[cfg(test)]
                        ctx.data_mut(|data| data.insert_temp(Id::unique("host_request_close"), close.rect));
                        if close.clicked() {
                            self.close_prompt = true;
                        }
                    });
                });
            });
        let side_width = (ctx.content_rect().width() * 0.3).clamp(260.0, 340.0);
        egui::Panel::left("session_host_roster")
            .exact_size(side_width)
            .frame(frame)
            .show(ui, |ui| {
                ui.add(
                    TextEdit::singleline(&mut self.search)
                        .hint_text(aurora_i18n::tr!("host-search"))
                        .char_limit(128),
                );
                ui.add_space(8.0);
                if ui
                    .add_enabled_ui(!closing && !snapshot.stopped, |ui| {
                        widgets::action_button(
                            ui,
                            p,
                            "login-2",
                            aurora_i18n::tr!("host-open-login"),
                            ActionKind::Primary,
                            vec2(ui.available_width(), 32.0),
                        )
                    })
                    .inner
                    .clicked()
                {
                    action = Some(Action::OpenLogin(None));
                }
                ui.add_space(10.0);
                let query = self.search.trim().to_lowercase();
                egui::ScrollArea::vertical().id_salt("session_host_rows").show(ui, |ui| {
                    widgets::organization_header(ui, p, aurora_i18n::tr!("host-running-sessions"));
                    let mut count = 0;
                    for worker in snapshot
                        .workers
                        .iter()
                        .take(MAX_WORKERS)
                        .filter(|worker| matches(display_name(worker), display_grid(worker), &query))
                    {
                        count += 1;
                        let id = worker.binding.worker.worker;
                        ui.push_id(id.to_string(), |ui| {
                            let label = display_name(worker);
                            let selected = self.selected == Some(id);
                            if widgets::navigation(ui, p, "user-filled", label, selected).clicked() {
                                self.selected = Some(id);
                            }
                            ui.label(RichText::new(display_grid(worker)).small().color(p.muted));
                            ui.label(
                                RichText::new(status(worker))
                                    .small()
                                    .color(if worker.reachable { p.muted } else { p.warn }),
                            );
                            if hosted == Some(id) {
                                ui.label(RichText::new(aurora_i18n::tr!("host-hosted")).small().color(p.success));
                            }
                            if snapshot.pending == Some(id) {
                                ui.label(RichText::new(aurora_i18n::tr!("host-switching")).small().color(p.violet_pale));
                            }
                            if worker.status.as_ref().is_some_and(|status| status.keyboard_focused) {
                                ui.label(RichText::new(aurora_i18n::tr!("host-keyboard-focus")).small().color(p.teal));
                            }
                            ui.horizontal_wrapped(|ui| {
                                let ready = !closing
                                    && !snapshot.stopped
                                    && snapshot.can_activate
                                    && snapshot.pending.is_none()
                                    && hosted != Some(id)
                                    && native_ready
                                    && activation_ready(worker);
                                let activate = ui
                                    .add_enabled_ui(ready, |ui| {
                                        widgets::action_button(
                                            ui,
                                            p,
                                            "arrows-exchange",
                                            aurora_i18n::tr!("host-activate"),
                                            ActionKind::Flat,
                                            vec2(0.0, 28.0),
                                        )
                                    })
                                    .inner;
                                #[cfg(test)]
                                ctx.data_mut(|data| data.insert_temp(Id::unique(("host_activate", id)), activate.rect));
                                if activate.clicked() {
                                    action = Some(Action::Activate(id));
                                }
                                if let Some(chat) = snapshot
                                    .chat
                                    .iter()
                                    .find(|chat| chat.source.binding() == worker.binding && super::chat_ui::eligible(chat))
                                {
                                    let response = widgets::action_button_if(
                                        ui,
                                        p,
                                        "messages-filled",
                                        aurora_i18n::tr!("host-chat-open"),
                                        ActionKind::Flat,
                                        vec2(0.0, 28.0),
                                        !closing && !snapshot.stopped,
                                    );
                                    #[cfg(test)]
                                    ctx.data_mut(|data| data.insert_temp(Id::unique(("host_chat_open", id)), response.rect));
                                    if response.clicked() {
                                        action = Some(Action::OpenChat(chat.source));
                                    }
                                }
                                let detach = ui
                                    .add_enabled_ui(!closing && !snapshot.stopped, |ui| {
                                        widgets::action_button(
                                            ui,
                                            p,
                                            "external-link",
                                            aurora_i18n::tr!("host-detach"),
                                            ActionKind::Flat,
                                            vec2(0.0, 28.0),
                                        )
                                    })
                                    .inner
                                    .on_hover_text(aurora_i18n::tr!("host-detach-help"));
                                #[cfg(test)]
                                ctx.data_mut(|data| data.insert_temp(Id::unique(("host_detach", id)), detach.rect));
                                if detach.clicked() {
                                    action = Some(Action::Detach(id));
                                }
                            });
                            ui.add_space(12.0);
                        });
                    }
                    if count == 0 {
                        ui.label(RichText::new(aurora_i18n::tr!("host-no-sessions")).color(p.muted));
                    }
                    ui.add_space(10.0);
                    widgets::organization_header(ui, p, aurora_i18n::tr!("host-saved-characters"));
                    for account in snapshot
                        .accounts
                        .iter()
                        .take(64)
                        .filter(|account| matches(&account.label, &account.grid, &query))
                    {
                        ui.push_id(account.key.to_string(), |ui| {
                            ui.label(RichText::new(&account.label).strong());
                            ui.label(RichText::new(&account.grid).small().color(p.muted));
                            let opening = snapshot.opening.contains(&account.key);
                            let response = ui
                                .add_enabled_ui(!closing && !snapshot.stopped && !opening, |ui| {
                                    widgets::action_button(
                                        ui,
                                        p,
                                        "login-2",
                                        aurora_i18n::tr!("host-open-login"),
                                        ActionKind::Flat,
                                        vec2(0.0, 28.0),
                                    )
                                })
                                .inner;
                            #[cfg(test)]
                            ctx.data_mut(|data| data.insert_temp(Id::unique(("host_open", account.key)), response.rect));
                            if response.clicked() {
                                action = Some(Action::OpenLogin(Some(account.key)));
                            }
                            if opening {
                                ui.label(RichText::new(aurora_i18n::tr!("host-opening")).small().color(p.muted));
                            }
                            ui.add_space(12.0);
                        });
                    }
                    if snapshot.accounts.is_empty() {
                        ui.label(RichText::new(aurora_i18n::tr!("host-no-saved-characters")).color(p.muted));
                    }
                });
            });
        let world = ui.available_rect_before_wrap();
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.22).max(10.0));
                widgets::glyph(ui, "device-desktop", p.violet_pale, 40.0);
                ui.add_space(12.0);
                let active = hosted.and_then(|id| snapshot.workers.iter().find(|worker| worker.binding.worker.worker == id));
                ui.label(
                    RichText::new(active.map_or(aurora_i18n::tr!("host-no-hosted-session"), display_name))
                        .strong()
                        .size(20.0),
                );
                ui.add_space(8.0);
                ui.label(if !cfg!(windows) {
                    aurora_i18n::tr!("host-platform-help")
                } else if !snapshot.can_activate {
                    aurora_i18n::tr!("host-unavailable-help")
                } else {
                    aurora_i18n::tr!("host-activate-help")
                });
                if closing {
                    ui.add_space(12.0);
                    ui.label(aurora_i18n::tr!("host-closing"));
                }
                if let Some(notice) = self.notice {
                    ui.add_space(10.0);
                    ui.label(RichText::new(notice_text(notice)).color(p.muted));
                }
                if let Some(error) = &snapshot.error {
                    ui.add_space(12.0);
                    ui.label(RichText::new(error).color(p.danger));
                }
            });
        });
        if self.close_prompt && !closing {
            let dialog = widgets::Modal::new(Id::unique("session_host_close")).show(&ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(RichText::new(aurora_i18n::tr!("host-close-question")).strong());
                ui.label(aurora_i18n::tr!("host-close-help"));
                ui.add_space(12.0);
                ui.horizontal_wrapped(|ui| {
                    let cancel = ui.button(aurora_i18n::tr!("common-cancel"));
                    #[cfg(test)]
                    ctx.data_mut(|data| data.insert_temp(Id::unique("host_cancel_close"), cancel.rect));
                    if cancel.clicked() {
                        self.close_prompt = false;
                    }
                    let confirm = widgets::action_button(
                        ui,
                        p,
                        "external-link",
                        aurora_i18n::tr!("host-close-confirm"),
                        ActionKind::Primary,
                        vec2(0.0, 30.0),
                    );
                    #[cfg(test)]
                    ctx.data_mut(|data| data.insert_temp(Id::unique("host_confirm_close"), confirm.rect));
                    if confirm.clicked() {
                        self.close_prompt = false;
                        action = Some(Action::Close);
                    }
                });
            });
            if dialog.should_close() {
                self.close_prompt = false;
            }
        }
        Output { action, world }
    }
}

fn matches(label: &str, grid: &str, query: &str) -> bool {
    query.is_empty() || label.to_lowercase().contains(query) || grid.to_lowercase().contains(query)
}

/// The IO thread may publish a fresh Arc for unchanged receipts. Only visible
/// changes should repaint the full software surface, including on a large host.
pub(super) fn same_snapshot(a: &HostSnapshot, b: &HostSnapshot) -> bool {
    a.stopped == b.stopped
        && a.error == b.error
        && a.can_activate == b.can_activate
        && a.active == b.active
        && a.active_viewport == b.active_viewport
        && a.completed_request == b.completed_request
        && a.pending == b.pending
        && a.opening == b.opening
        && a.chat.len() == b.chat.len()
        && a.chat.iter().zip(&b.chat).all(|(a, b)| std::sync::Arc::ptr_eq(a, b))
        && a.accounts.len() == b.accounts.len()
        && a.accounts
            .iter()
            .zip(&b.accounts)
            .all(|(a, b)| a.key == b.key && a.label == b.label && a.grid == b.grid)
        && a.workers.len() == b.workers.len()
        && a.workers.iter().zip(&b.workers).all(|(a, b)| {
            let same_status = match (&a.status, &b.status) {
                (Some(a), Some(b)) => {
                    a.phase == b.phase
                        && a.name == b.name
                        && a.grid_label == b.grid_label
                        && a.native_window == b.native_window
                        && a.keyboard_focused == b.keyboard_focused
                        && a.blocked == b.blocked
                        && a.last_frame.is_some_and(|frame| frame.validate().is_ok())
                            == b.last_frame.is_some_and(|frame| frame.validate().is_ok())
                }
                (None, None) => true,
                _ => false,
            };
            a.binding == b.binding && same_status && a.reachable == b.reachable && a.label == b.label && a.grid == b.grid
        })
}

fn display_name(worker: &WorkerSummary) -> &str {
    let label = worker
        .status
        .as_ref()
        .map(|status| status.name.as_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(&worker.label);
    if label.trim().is_empty() {
        aurora_i18n::tr!("host-unnamed-session")
    } else {
        label
    }
}

fn display_grid(worker: &WorkerSummary) -> &str {
    worker
        .status
        .as_ref()
        .map(|status| status.grid_label.as_str())
        .filter(|grid| !grid.trim().is_empty())
        .unwrap_or(&worker.grid)
}

fn activation_ready(worker: &WorkerSummary) -> bool {
    worker.reachable
        && worker.binding.session.is_some()
        && worker
            .status
            .as_ref()
            .is_some_and(|status| status.phase == Phase::Online && !status.blocked && status.native_window != 0)
}

pub(super) fn presentation_ready(worker: &WorkerSummary) -> bool {
    activation_ready(worker)
        && worker
            .status
            .as_ref()
            .is_some_and(|status| status.last_frame.is_some_and(|frame| frame.validate().is_ok()))
}

fn status(worker: &WorkerSummary) -> &'static str {
    if !worker.reachable {
        return aurora_i18n::tr!("host-unreachable");
    }
    let Some(status) = &worker.status else {
        return aurora_i18n::tr!("host-starting");
    };
    match status.phase {
        Phase::Login => aurora_i18n::tr!("host-login"),
        Phase::Connecting => aurora_i18n::tr!("host-connecting"),
        Phase::Online if status.blocked => aurora_i18n::tr!("host-worker-blocked"),
        Phase::Online => aurora_i18n::tr!("host-online"),
        Phase::Disconnected => aurora_i18n::tr!("host-disconnected"),
    }
}

fn notice_text(notice: Notice) -> &'static str {
    match notice {
        Notice::Queued => aurora_i18n::tr!("host-request-queued"),
        Notice::Error(HostError::Busy) => aurora_i18n::tr!("host-busy"),
        Notice::Error(HostError::Unsupported) => aurora_i18n::tr!("host-unavailable-help"),
        Notice::Error(HostError::Stopped) => aurora_i18n::tr!("host-stopped"),
        Notice::Error(HostError::Unavailable) => aurora_i18n::tr!("host-chat-unavailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::transport::SavedAccount;
    use super::*;
    use aurora_session::{
        Binding, FrameProof, GridKey, HostId, InstallationId, Label, SessionId, SessionIdentity, WorkerIdentity, WorkerStatus,
    };

    fn snapshot() -> HostSnapshot {
        let account = AccountKey::from_bytes([5; 32]).unwrap();
        HostSnapshot {
            accounts: vec![SavedAccount {
                key: account,
                label: "Demo One".into(),
                grid: "Example Grid".into(),
            }],
            workers: vec![WorkerSummary {
                binding: Binding {
                    worker: WorkerIdentity {
                        installation: InstallationId::from_bytes([1; 32]).unwrap(),
                        host: HostId::from_bytes([2; 16]).unwrap(),
                        worker: WorkerId::from_bytes([3; 16]).unwrap(),
                    },
                    session: Some(SessionIdentity {
                        session: SessionId::from_bytes([4; 16]).unwrap(),
                        account,
                        grid: GridKey::from_bytes([6; 32]).unwrap(),
                        generation: 1,
                    }),
                },
                status: Some(WorkerStatus {
                    phase: Phase::Online,
                    name: Label::new("Demo One").unwrap(),
                    grid_label: Label::new("Example Grid").unwrap(),
                    native_window: 7,
                    last_frame: Some(FrameProof {
                        device_epoch: 1,
                        surface_epoch: 1,
                        frame: 1,
                    }),
                    ..Default::default()
                }),
                reachable: true,
                label: "Demo One".into(),
                grid: "Example Grid".into(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn controller_activation_requires_live_bound_unblocked_online_session_and_presentation_requires_completed_frame() {
        let base = snapshot();
        let mut worker = base.workers[0].clone();
        assert!(activation_ready(&worker));
        assert!(presentation_ready(&worker));
        worker.reachable = false;
        assert!(!activation_ready(&worker));
        worker = base.workers[0].clone();
        worker.binding.session = None;
        assert!(!activation_ready(&worker));
        for phase in [Phase::Login, Phase::Connecting, Phase::Disconnected] {
            worker = base.workers[0].clone();
            worker.status.as_mut().unwrap().phase = phase;
            assert!(!activation_ready(&worker));
        }
        worker = base.workers[0].clone();
        worker.status.as_mut().unwrap().blocked = true;
        assert!(!activation_ready(&worker));
        worker.status.as_mut().unwrap().blocked = false;
        worker.status.as_mut().unwrap().native_window = 0;
        assert!(!activation_ready(&worker));
        worker.status.as_mut().unwrap().native_window = 7;
        worker.status.as_mut().unwrap().last_frame = None;
        assert!(
            activation_ready(&worker),
            "first activation must be able to request its first prepared frame"
        );
        assert!(
            !presentation_ready(&worker),
            "a prepared/completed frame is still required before showing the world"
        );
    }

    struct Frames {
        ctx: egui::Context,
        ui: HostUi,
        frame: u64,
    }

    impl Frames {
        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::theme::Theme {
                panel_alpha: 255,
                ..Default::default()
            }
            .apply(&ctx, 1.0);
            Self {
                ctx,
                ui: HostUi::default(),
                frame: 0,
            }
        }

        fn show(&mut self, snapshot: &HostSnapshot, events: Vec<egui::Event>, native_ready: bool) -> Option<Action> {
            self.frame += 1;
            let p = crate::theme::Theme {
                panel_alpha: 255,
                ..Default::default()
            }
            .palette();
            let mut action = None;
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1240.0, 800.0))),
                    time: Some(self.frame as f64 / 60.0),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let shown = self.ui.show(ui, &p, snapshot, native_ready, None, false);
                    action = action.or(shown.action);
                    assert!(shown.world.left() >= 260.0 && shown.world.top() >= 58.0);
                },
            );
            assert!(
                output.shapes.iter().all(|shape| !matches!(&shape.shape, egui::Shape::Callback(_))),
                "software-only UI must use meshes"
            );
            output.textures_delta.clear();
            action
        }

        fn click(&mut self, snapshot: &HostSnapshot, id: Id, native_ready: bool) -> Option<Action> {
            for _ in 0..4 {
                self.show(snapshot, Vec::new(), native_ready);
            }
            let rect = self.ctx.data(|data| data.get_temp::<Rect>(id)).expect("action shown");
            self.show(snapshot, vec![egui::Event::PointerMoved(rect.center())], native_ready);
            let mut action = None;
            for pressed in [true, false] {
                action = action.or(self.show(
                    snapshot,
                    vec![egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    native_ready,
                ));
            }
            action
        }
    }

    #[test]
    fn controller_buttons_issue_exact_typed_roster_actions_and_gate_unsupported_activation() {
        let mut snapshot = snapshot();
        let worker = snapshot.workers[0].binding.worker.worker;
        let account = snapshot.accounts[0].key;
        let mut frames = Frames::new();
        assert_eq!(frames.click(&snapshot, Id::unique(("host_activate", worker)), true), None);
        snapshot.can_activate = true;
        assert_eq!(frames.click(&snapshot, Id::unique(("host_activate", worker)), false), None);
        snapshot.workers[0].status.as_mut().unwrap().last_frame = None;
        assert_eq!(
            frames.click(&snapshot, Id::unique(("host_activate", worker)), true),
            Some(Action::Activate(worker))
        );
        assert_eq!(
            frames.click(&snapshot, Id::unique(("host_detach", worker)), true),
            Some(Action::Detach(worker))
        );
        assert_eq!(
            frames.click(&snapshot, Id::unique(("host_open", account)), true),
            Some(Action::OpenLogin(Some(account)))
        );
        snapshot.opening.push(account);
        assert_eq!(frames.click(&snapshot, Id::unique(("host_open", account)), true), None);
    }

    #[test]
    fn host_chat_roster_button_requires_authenticated_live_exact_source_and_never_activates_world() {
        use super::super::transport::HostChatSnapshot;
        use aurora_session::chat::{ChatKey, ChatSource, Conversation, ConversationInfo, NativeChatPolicy};
        use std::sync::Arc;
        let mut snapshot = snapshot();
        let worker = snapshot.workers[0].binding.worker.worker;
        let source = ChatSource::try_from((snapshot.workers[0].binding, 1)).unwrap();
        let mut chat = HostChatSnapshot {
            source,
            authenticated: false,
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
                title: Label::new("Synthetic nearby").unwrap(),
            }],
            lines: Vec::new(),
            receipts: Vec::new(),
            gap: false,
        };
        snapshot.chat.push(Arc::new(chat.clone()));
        let mut frames = Frames::new();
        frames.show(&snapshot, Vec::new(), true);
        assert!(
            frames
                .ctx
                .data(|data| data.get_temp::<Rect>(Id::unique(("host_chat_open", worker))))
                .is_none()
        );
        chat.authenticated = true;
        snapshot.chat[0] = Arc::new(chat.clone());
        assert_eq!(
            frames.click(&snapshot, Id::unique(("host_chat_open", worker)), true),
            Some(Action::OpenChat(source))
        );
        assert_eq!(snapshot.active, None);
        chat.source.session.generation += 1;
        chat.conversations[0].policy.key.source = chat.source;
        snapshot.chat[0] = Arc::new(chat);
        let mut frames = Frames::new();
        frames.show(&snapshot, Vec::new(), true);
        assert!(
            frames
                .ctx
                .data(|data| data.get_temp::<Rect>(Id::unique(("host_chat_open", worker))))
                .is_none(),
            "new session metadata cannot authorize the old worker binding"
        );
    }

    #[test]
    fn controller_search_matches_display_names_and_grids_without_changing_identity() {
        assert!(matches("Demo One", "Example Grid", "example"));
        assert!(matches("Demo One", "Example Grid", "one"));
        assert!(!matches("Demo One", "Example Grid", "missing"));
    }

    #[test]
    fn controller_generic_login_adopts_acknowledged_character_and_grid_metadata() {
        let mut snapshot = snapshot();
        let worker = &mut snapshot.workers[0];
        worker.label.clear();
        worker.grid.clear();
        let identity = worker.binding;
        assert_eq!(display_name(worker), "Demo One");
        assert_eq!(display_grid(worker), "Example Grid");
        assert!(matches(display_name(worker), display_grid(worker), "demo"));
        assert!(matches(display_name(worker), display_grid(worker), "example"));
        assert_eq!(worker.binding, identity, "labels never substitute the protocol binding");
        worker.status.as_mut().unwrap().phase = Phase::Login;
        worker.status.as_mut().unwrap().blocked = true;
        assert_eq!(status(worker), aurora_i18n::tr!("host-login"));
        worker.status.as_mut().unwrap().name = Label::new("").unwrap();
        worker.status.as_mut().unwrap().grid_label = Label::new("").unwrap();
        worker.label = "Saved Character".into();
        worker.grid = "Saved Grid".into();
        assert_eq!(display_name(worker), "Saved Character");
        assert_eq!(display_grid(worker), "Saved Grid");
    }

    #[test]
    fn host_presentation_requires_exact_native_ack_beyond_activation_eligibility() {
        use aurora_session::{HostedViewport, PhysicalRect};
        let mut snapshot = snapshot();
        let worker = snapshot.workers[0].binding.worker.worker;
        let viewport = HostedViewport {
            host_window: 7,
            rect: PhysicalRect {
                x: -1600,
                y: 80,
                width: 1000,
                height: 700,
            },
        };
        snapshot.can_activate = true;
        snapshot.active = Some(worker);
        snapshot.active_viewport = Some(viewport);
        let presented = super::super::presented_worker;
        assert_eq!(presented(&snapshot, Some(viewport), false), Some(worker));
        assert_eq!(
            presented(&snapshot, Some(viewport), true),
            None,
            "modal/local intent covers the world"
        );
        assert_eq!(presented(&snapshot, None, false), None);
        let moved = HostedViewport {
            rect: PhysicalRect {
                x: viewport.rect.x + 10,
                ..viewport.rect
            },
            ..viewport
        };
        assert_eq!(
            presented(&snapshot, Some(moved), false),
            None,
            "a live move needs a new exact acknowledgment"
        );
        snapshot.pending = Some(worker);
        assert_eq!(presented(&snapshot, Some(viewport), false), None);
        snapshot.pending = None;
        snapshot.active_viewport = Some(moved);
        assert_eq!(presented(&snapshot, Some(moved), false), Some(worker));
        snapshot.workers[0].status.as_mut().unwrap().last_frame = None;
        assert!(activation_ready(&snapshot.workers[0]));
        assert_eq!(
            presented(&snapshot, Some(moved), false),
            None,
            "no completed frame means no world cutout"
        );
    }

    #[test]
    fn controller_header_close_click_keeps_its_modal_open_across_following_passes() {
        let snapshot = snapshot();
        let mut frames = Frames::new();
        assert_eq!(frames.click(&snapshot, Id::unique("host_request_close"), true), None);
        assert!(frames.ui.close_prompt, "the opening pointer release must not dismiss the new modal");
        for _ in 0..4 {
            assert_eq!(frames.show(&snapshot, Vec::new(), true), None);
            assert!(
                frames.ui.close_prompt,
                "the close confirmation must survive discarded/following UI passes"
            );
            assert!(
                frames
                    .ctx
                    .data(|data| data.get_temp::<Rect>(Id::unique("host_confirm_close")))
                    .is_some()
            );
        }
    }

    #[test]
    fn controller_close_requires_confirmation_and_cancel_keeps_sessions_untouched() {
        let snapshot = snapshot();
        let mut frames = Frames::new();
        frames.ui.close_prompt = true;
        assert_eq!(frames.show(&snapshot, Vec::new(), true), None);
        assert_eq!(frames.click(&snapshot, Id::unique("host_cancel_close"), true), None);
        assert!(!frames.ui.close_prompt);
        frames.ui.close_prompt = true;
        assert_eq!(frames.click(&snapshot, Id::unique("host_confirm_close"), true), Some(Action::Close));
        assert!(!frames.ui.close_prompt);
    }

    #[test]
    fn controller_unchanged_receipts_sleep_and_visible_status_changes_repaint() {
        let a = snapshot();
        assert!(same_snapshot(&a, &a.clone()));
        let mut b = a.clone();
        b.workers[0].status.as_mut().unwrap().last_frame.as_mut().unwrap().frame += 1;
        assert!(same_snapshot(&a, &b), "progress receipts do not animate the roster");
        b = a.clone();
        b.workers[0].reachable = false;
        assert!(!same_snapshot(&a, &b));
        b = a.clone();
        b.workers[0].status.as_mut().unwrap().keyboard_focused = true;
        assert!(!same_snapshot(&a, &b));
        b = a.clone();
        b.pending = Some(b.workers[0].binding.worker.worker);
        assert!(!same_snapshot(&a, &b));
        b = a.clone();
        b.accounts[0].grid = "Different Grid".into();
        assert!(!same_snapshot(&a, &b));
    }
}
