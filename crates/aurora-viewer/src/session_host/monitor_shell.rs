//! Actual Windows Shell routing; all image/proof IO remains in HostController's
//! separately owned projection. No uncredited native backend is selected.

use super::Shell;
use super::monitor_manager::index;
use super::monitor_model::{Binding, View};
use super::monitor_ui::{Action as MonitorAction, Command, State};
use super::monitor_window::MonitorWindow;
use super::preview_projection::{PreviewError, PreviewRead};
use super::shared_chat::SharedChat;
use super::slots::SlotId;
use super::ui::Notice;
use aurora_session::preview_image::PreviewSource;
use std::time::{Duration, Instant};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::WindowId;

fn flush_view(
    windows: &mut [Option<MonitorWindow>; 5],
    chat: &mut SharedChat,
    event_loop: &ActiveEventLoop,
    view: View,
    _binding: Binding,
    _state: State,
) {
    if let Some(window) = &mut windows[index(view)] {
        window.flush_edits(event_loop, chat);
    }
}

impl Shell {
    fn publish_mcp_monitors(&self, authority: &aurora_mcp::authority::Authority) -> Result<(), aurora_mcp::authority::AuthorityError> {
        use aurora_mcp::authority::{MonitorProjection, MonitorSnapshot};
        let profiles = authority.profile_access()?;
        let mut monitors = Vec::new();
        for view in View::ALL {
            let Some(binding) = self.monitors.model().binding(view) else {
                continue;
            };
            let source = binding.source.chat().binding();
            let Some(profile) = profiles.iter().find(|profile| profile.ticket.binding == Some(source)) else {
                return Err(aurora_mcp::authority::AuthorityError::StaleSource);
            };
            monitors.push(MonitorProjection {
                monitor: (index(view) + 1) as u8,
                source: profile.ticket,
                incarnation: binding.incarnation,
                visible: binding.visible,
                enabled: binding.enabled,
            });
        }
        let eligible = self
            .ui
            .monitor_sources
            .iter()
            .filter(|(_, source)| Some(source.worker().worker) != self.snapshot.active)
            .filter_map(|(_, source)| {
                profiles
                    .iter()
                    .find(|profile| {
                        profile.ticket.binding
                            == Some(aurora_session::Binding {
                                worker: source.worker(),
                                session: Some(source.session()),
                            })
                    })
                    .map(|profile| profile.ticket)
            })
            .collect();
        authority.publish_monitors(MonitorSnapshot {
            eligible,
            monitors,
            playing: self.monitors.model().playing(),
            interval_seconds: self.monitors.model().interval(),
        })?;
        Ok(())
    }

    fn mcp_monitor_actions(&mut self, event_loop: &ActiveEventLoop, blocked: bool) {
        use aurora_mcp::{
            authority::ActionResult,
            wire::{HostAction, PermissionClass},
        };
        let Some(authority) = self.snapshot.mcp.authority.clone() else {
            return;
        };
        if self.publish_mcp_monitors(&authority).is_err() || blocked {
            return;
        }
        for queued in authority
            .ready_actions()
            .unwrap_or_default()
            .into_iter()
            .filter(|action| action.action.class() == PermissionClass::Monitors)
            .take(16)
        {
            let _ = authority.execute_action(&queued.request_id, |action| {
                let result = (|| -> Result<(), PreviewError> {
                    let now = self.monitor_time();
                    let cycle_scope: Vec<_> = self
                        .ui
                        .monitor_sources
                        .iter()
                        .filter(|(_, source)| {
                            Some(source.worker().worker) != self.snapshot.active
                                && action.profiles.iter().any(|ticket| {
                                    ticket.binding
                                        == Some(aurora_session::Binding {
                                            worker: source.worker(),
                                            session: Some(source.session()),
                                        })
                                })
                        })
                        .map(|(_, source)| *source)
                        .collect();
                    match &action.action {
                        HostAction::MonitorOpen => {
                            let ticket = action.profiles.first().ok_or(PreviewError::Unavailable)?;
                            let source = self
                                .ui
                                .monitor_sources
                                .iter()
                                .find(|(slot, source)| {
                                    slot.number() == ticket.profile.number()
                                        && ticket.binding
                                            == Some(aurora_session::Binding {
                                                worker: source.worker(),
                                                session: Some(source.session()),
                                            })
                                })
                                .map(|(_, source)| *source)
                                .ok_or(PreviewError::Stale)?;
                            let windows = &mut self.monitor_windows;
                            let chat = &mut self.shared_chat;
                            self.monitors.open(source, &mut |view, binding, state| {
                                flush_view(windows, chat, event_loop, view, binding, state)
                            })?;
                        }
                        HostAction::MonitorClose { monitor } => {
                            let view = *View::ALL
                                .get(usize::from(*monitor).saturating_sub(1))
                                .ok_or(PreviewError::Invalid)?;
                            let windows = &mut self.monitor_windows;
                            let chat = &mut self.shared_chat;
                            self.monitors.close(view, &mut |view, binding, state| {
                                flush_view(windows, chat, event_loop, view, binding, state)
                            })?;
                            if let Some(mut window) = self.monitor_windows[index(view)].take() {
                                window.retire(&mut self.shared_chat);
                            }
                        }
                        HostAction::MonitorsOpenAll => {
                            let sources: Vec<_> = self
                                .ui
                                .monitor_sources
                                .iter()
                                .filter(|(_, source)| {
                                    action.profiles.iter().any(|ticket| {
                                        ticket.binding
                                            == Some(aurora_session::Binding {
                                                worker: source.worker(),
                                                session: Some(source.session()),
                                            })
                                    })
                                })
                                .map(|(_, source)| *source)
                                .collect();
                            let windows = &mut self.monitor_windows;
                            let chat = &mut self.shared_chat;
                            for source in sources {
                                if Some(source.worker().worker) == self.snapshot.active {
                                    continue;
                                }
                                self.monitors.open(source, &mut |view, binding, state| {
                                    flush_view(windows, chat, event_loop, view, binding, state)
                                })?;
                            }
                        }
                        HostAction::MonitorsArrange => {
                            let monitor = self
                                .surface
                                .as_ref()
                                .and_then(|surface| surface.window.current_monitor())
                                .ok_or(PreviewError::Unavailable)?;
                            let origin = monitor.position();
                            let size = monitor.size();
                            for (index, window) in self.monitor_windows.iter_mut().flatten().enumerate() {
                                let x = i64::from(origin.x) + i64::from(size.width) * (index % 2) as i64 / 2;
                                let y = i64::from(origin.y) + i64::from(size.height) * (index / 2) as i64 / 3;
                                window
                                    .arrange(
                                        event_loop,
                                        [
                                            i32::try_from(x).map_err(|_| PreviewError::Invalid)?,
                                            i32::try_from(y).map_err(|_| PreviewError::Invalid)?,
                                        ],
                                        &mut self.shared_chat,
                                    )
                                    .map_err(|_| PreviewError::Unavailable)?;
                            }
                        }
                        HostAction::MonitorCycleConfigure { interval_seconds, playing } => {
                            let windows = &mut self.monitor_windows;
                            let chat = &mut self.shared_chat;
                            self.monitors.mcp_cycle(
                                &cycle_scope,
                                Some(*interval_seconds),
                                Some(*playing),
                                false,
                                now,
                                &mut |view, binding, state| flush_view(windows, chat, event_loop, view, binding, state),
                            )?;
                        }
                        HostAction::MonitorCyclePause | HostAction::MonitorCycleNext => {
                            let windows = &mut self.monitor_windows;
                            let chat = &mut self.shared_chat;
                            self.monitors.mcp_cycle(
                                &cycle_scope,
                                None,
                                Some(false),
                                matches!(action.action, HostAction::MonitorCycleNext),
                                now,
                                &mut |view, binding, state| flush_view(windows, chat, event_loop, view, binding, state),
                            )?;
                        }
                        _ => return Err(PreviewError::Unsupported),
                    }
                    self.publish_monitors(event_loop)?;
                    Ok(())
                })();
                match result {
                    Ok(()) => ActionResult::Submitted,
                    Err(_) => ActionResult::RejectedNative,
                }
            });
            // The first action changes the canonical set; following approvals
            // must observe/reapprove that new set rather than reuse old targets.
            if self.publish_mcp_monitors(&authority).is_err() {
                break;
            }
        }
    }
    fn monitor_time(&self) -> u64 {
        self.monitor_started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }
    fn monitor_error(&mut self, error: PreviewError) {
        if error == PreviewError::Exhausted {
            self.monitors.fail_closed();
        }
        log::debug!("session host: monitor operation unavailable: {error:?}");
        self.ui.notice = Some(Notice::MonitorUnavailable);
        self.notice_at = Some(Instant::now());
        self.request_repaint();
    }
    fn monitor_read(&mut self, view: View) -> Option<PreviewRead> {
        let namespace = self.monitors.namespace(view);
        self.monitor_reads.retain_namespace(view, namespace);
        let namespace = namespace?;
        match self.monitor_reader.try_read(view, namespace) {
            Ok(read) if self.monitors.current_read(view, &read) => {
                self.monitor_reads.remember(&read);
                Some(read)
            }
            Err(PreviewError::Busy | PreviewError::Stale) => self
                .monitor_reads
                .fallback(view, namespace)
                .filter(|read| self.monitors.current_read(view, read)),
            _ => None,
        }
    }
    fn publish_monitors(&mut self, event_loop: &ActiveEventLoop) -> Result<(), PreviewError> {
        let mut failure = None;
        for view in View::ALL {
            let at = index(view);
            if self.monitor_windows[at].as_ref().is_some_and(|window| !window.is_live()) {
                let windows = &mut self.monitor_windows;
                let chat = &mut self.shared_chat;
                if let Err(error) = self.monitors.hide(view, &mut |view, binding, state| {
                    flush_view(windows, chat, event_loop, view, binding, state)
                }) {
                    if error == PreviewError::Exhausted {
                        self.monitors.fail_closed();
                    }
                    failure = Some(error);
                }
                if let Some(mut window) = self.monitor_windows[at].take() {
                    window.retire(&mut self.shared_chat);
                }
                self.monitor_placements[at] = None;
                self.monitor_error(PreviewError::Unavailable);
            }
        }
        let result = self.monitors.publish(&self.controller);
        // The synchronous fence advances on ALL queue outcomes. Purge old OS
        // output now, including unchanged views waiting for the new full set.
        for view in View::ALL {
            let at = index(view);
            let read = self.monitor_read(view);
            let keep = match (self.monitor_windows[at].as_mut(), read.as_ref()) {
                (Some(window), Some(read)) => window.retain(&self.monitors, read, &mut self.shared_chat),
                _ => false,
            };
            if !keep && let Some(mut window) = self.monitor_windows[at].take() {
                window.retire(&mut self.shared_chat);
            }
        }
        result.and(failure.map_or(Ok(()), Err))
    }
    /// Current full source catalogue is real Broker evidence, independent of
    /// optional image bootstrap and independently governed canonical Chat.
    fn refresh_monitors(&mut self, event_loop: &ActiveEventLoop) -> Result<(), PreviewError> {
        self.snapshot = self.controller.snapshot();
        self.chat_changed |= self.shared_chat.sync(&self.snapshot, self.closing.is_some());
        let status = self.monitor_reader.try_status()?;
        let old_sources = self.ui.monitor_sources.clone();
        let windows = &mut self.monitor_windows;
        let chat = &mut self.shared_chat;
        self.monitors.sync(
            &status,
            std::array::from_fn(|slot| SlotId::try_from(slot as u8).expect("bounded stable slot")),
            &mut |view, binding, state| flush_view(windows, chat, event_loop, view, binding, state),
        )?;
        self.ui.monitor_sources = status
            .sources()
            .iter()
            .filter(|source| source.available)
            .map(|source| (source.slot, source.source))
            .collect();
        self.ui.monitor_cycle_ready = status
            .sources()
            .iter()
            .any(|source| source.available && Some(source.source) != status.active());
        let recovery = self.monitors.recovery()?;
        let recovery_changed = self.ui.monitor_recovery != recovery;
        self.ui.monitor_recovery = recovery;
        if old_sources != self.ui.monitor_sources || recovery_changed {
            self.request_repaint();
        }
        Ok(())
    }
    pub(super) fn open_monitor(&mut self, event_loop: &ActiveEventLoop, source: Option<PreviewSource>) {
        self.ui.notice = Some(Notice::MonitorStarting);
        self.notice_at = None;
        self.request_repaint();
        let result = (|| {
            self.refresh_monitors(event_loop)?;
            let now = self.monitor_time();
            let windows = &mut self.monitor_windows;
            let chat = &mut self.shared_chat;
            let mut flush = |view, binding, state| flush_view(windows, chat, event_loop, view, binding, state);
            let view = if let Some(source) = source {
                self.monitors.open(source, &mut flush)?
            } else {
                self.monitors.open_cycling(now, &mut flush)?
            };
            self.publish_monitors(event_loop)?;
            self.monitor_focus = self.monitors.namespace(view).map(|namespace| (view, namespace));
            self.sync_monitor_windows(event_loop);
            Ok(())
        })();
        if let Err(error) = result {
            self.monitor_error(error);
        }
        if let Err(error) = self.publish_monitors(event_loop) {
            self.monitor_error(error);
        }
    }
    pub(super) fn resume_monitor(&mut self, event_loop: &ActiveEventLoop, recovery: super::monitor_manager::Recovery) {
        self.ui.notice = Some(Notice::MonitorStarting);
        self.notice_at = None;
        self.request_repaint();
        let result = (|| {
            self.refresh_monitors(event_loop)?;
            let now = self.monitor_time();
            let windows = &mut self.monitor_windows;
            let chat = &mut self.shared_chat;
            if !self.monitors.resume(recovery, now, &mut |view, binding, state| {
                flush_view(windows, chat, event_loop, view, binding, state)
            })? {
                return Ok(());
            }
            self.publish_monitors(event_loop)?;
            self.monitor_focus = self.monitors.namespace(View::Cycling).map(|namespace| (View::Cycling, namespace));
            self.sync_monitor_windows(event_loop);
            Ok(())
        })();
        if let Err(error) = result {
            self.monitor_error(error);
        }
        if let Err(error) = self.publish_monitors(event_loop) {
            self.monitor_error(error);
        }
    }

    fn sync_monitor_windows(&mut self, event_loop: &ActiveEventLoop) {
        for view in View::ALL {
            let at = index(view);
            let read = self.monitor_read(view);
            let current = read.as_ref().is_some_and(|read| self.monitors.current_read(view, read));
            let keep = current
                && self.monitor_windows[at]
                    .as_ref()
                    .is_some_and(|window| window.matches(&self.monitors));
            if !keep && let Some(mut window) = self.monitor_windows[at].take() {
                window.retire(&mut self.shared_chat);
            }
            if current && self.monitor_windows[at].is_none() && self.closing.is_none() {
                let Some(owner) = &self.surface else { continue };
                let Some(read) = &read else { continue };
                let Some(binding) = self.monitors.model().binding(view) else {
                    continue;
                };
                // All old native input is already drained before this selection.
                // There is no fallback to another source or canonical surface.
                self.shared_chat.select_for(view.surface(), binding.source.chat());
                let _ = self.monitors.observe_pin(view, false);
                match MonitorWindow::new(
                    event_loop,
                    &owner.window,
                    &self.monitors,
                    &self.monitor_reader,
                    read,
                    &self.snapshot,
                    self.monitor_placements[at],
                ) {
                    Ok(window) => self.monitor_windows[at] = Some(window),
                    Err(error) => {
                        log::warn!("session host: could not create owned monitor: {error}");
                        self.monitor_placements[at] = None;
                        self.monitor_error(PreviewError::Unavailable);
                        let windows = &mut self.monitor_windows;
                        let chat = &mut self.shared_chat;
                        let _ = self.monitors.hide(view, &mut |view, binding, state| {
                            flush_view(windows, chat, event_loop, view, binding, state)
                        });
                    }
                }
            }
            if self.monitor_focus == self.monitors.namespace(view).map(|namespace| (view, namespace))
                && let Some(window) = &mut self.monitor_windows[at]
            {
                window.request_focus();
                self.monitor_focus = None;
            }
            if let (Some(window), Some(read)) = (&mut self.monitor_windows[at], &read) {
                let blocked = self.ui.modal_open() || self.closing.is_some() || self.snapshot.stopped;
                let outcome = window.first_frame(event_loop, &self.monitors, read, &mut self.shared_chat, &self.snapshot, blocked);
                self.monitor_outcome(event_loop, view, outcome);
            }
        }
        // Constructor/native failure leaves no visible subscriber. Stop its
        // cache/demand immediately; recovery requires an explicit Open.
        if let Err(error) = self.publish_monitors(event_loop) {
            self.monitor_error(error);
        }
    }

    pub(super) fn monitor_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: &WindowEvent) -> bool {
        let Some(view) = self
            .monitor_windows
            .iter()
            .flatten()
            .find(|window| window.id() == id)
            .map(MonitorWindow::view)
        else {
            return false;
        };
        let at = index(view);
        // Closing/minimizing an owned view is local Stop, independent of a
        // contended/expired preview read. Never recreate a window the user just
        // closed merely because its image/control publication was unavailable.
        self.shared_chat.sync(&self.controller.snapshot(), self.closing.is_some());
        let close = matches!(event, WindowEvent::CloseRequested | WindowEvent::Destroyed);
        let minimized = matches!(event, WindowEvent::Resized(_))
            && self.monitor_windows[at]
                .as_mut()
                .and_then(|window| window.minimized(event_loop, &mut self.shared_chat))
                == Some(true);
        if close || minimized {
            let windows = &mut self.monitor_windows;
            let chat = &mut self.shared_chat;
            let mut flush = |view, binding, state| flush_view(windows, chat, event_loop, view, binding, state);
            let result = if close {
                self.monitors.close(view, &mut flush)
            } else {
                self.monitors.hide(view, &mut flush)
            };
            if let Err(error) = result {
                self.monitor_error(error);
            }
            if let Some(mut window) = self.monitor_windows[at].take() {
                window.retire(&mut self.shared_chat);
            }
            if let Err(error) = self.publish_monitors(event_loop) {
                self.monitor_error(error);
            }
            return true;
        }
        if let Err(error) = self.refresh_monitors(event_loop)
            && !matches!(error, PreviewError::Busy | PreviewError::Stale)
        {
            if let Some(mut window) = self.monitor_windows[at].take() {
                window.retire(&mut self.shared_chat);
            }
            self.monitor_error(error);
            if let Err(error) = self.publish_monitors(event_loop) {
                self.monitor_error(error);
            }
            return true;
        }
        let Some(read) = self.monitor_read(view) else {
            if let Some(mut window) = self.monitor_windows[at].take() {
                window.retire(&mut self.shared_chat);
            }
            if let Err(error) = self.publish_monitors(event_loop) {
                self.monitor_error(error);
            }
            return true;
        };
        let blocked = self.ui.modal_open() || self.closing.is_some() || self.snapshot.stopped;
        let Some(window) = &mut self.monitor_windows[at] else { return true };
        let outcome = window.event(
            event_loop,
            event,
            &self.monitors,
            &read,
            &mut self.shared_chat,
            &self.snapshot,
            blocked,
        );
        self.monitor_outcome(event_loop, view, outcome);
        true
    }

    fn monitor_outcome(&mut self, event_loop: &ActiveEventLoop, view: View, outcome: super::monitor_window::Outcome) {
        let at = index(view);
        if outcome.first_presented && matches!(self.ui.notice, Some(Notice::MonitorStarting)) {
            self.ui.notice = None;
            self.notice_at = None;
            self.request_repaint();
        }
        if outcome.close || outcome.minimized || outcome.failed {
            let windows = &mut self.monitor_windows;
            let chat = &mut self.shared_chat;
            let mut flush = |view, binding, state| flush_view(windows, chat, event_loop, view, binding, state);
            let result = if outcome.close {
                self.monitors.close(view, &mut flush)
            } else {
                self.monitors.hide(view, &mut flush)
            };
            if let Err(error) = result {
                self.monitor_error(error);
            }
        }
        if outcome.failed {
            self.monitor_placements[at] = None;
        } else if let Some(placement) = outcome.placement {
            self.monitor_placements[at] = Some(placement);
        }
        if outcome.failed || outcome.recreate || outcome.close || outcome.minimized {
            if let Some(mut window) = self.monitor_windows[at].take() {
                window.retire(&mut self.shared_chat);
            }
        } else if let Some(actual) = outcome.observed_pin {
            let _ = self.monitors.observe_pin(view, actual);
        }
        if outcome.failed {
            self.monitor_error(PreviewError::Unavailable);
        }
        // A captured action is independent of native presentation success. It
        // receives fresh guards and local validation before any queue admission.
        if let Some(action) = outcome.action {
            self.monitor_action(event_loop, view, action);
        }
        if let Err(error) = self.publish_monitors(event_loop) {
            self.monitor_error(error);
        }
    }

    fn monitor_action(&mut self, event_loop: &ActiveEventLoop, view: View, action: MonitorAction) {
        let refreshed = self.refresh_monitors(event_loop);
        let roster_current = refreshed.is_ok();
        if refreshed.is_err_and(|error| !matches!(error, PreviewError::Busy | PreviewError::Stale))
            || self.closing.is_some()
            || self.ui.modal_open()
            || self.snapshot.stopped
        {
            return;
        }
        let Some(read) = self.monitor_read(view) else { return };
        if !read.view_current() {
            return;
        }
        let Some(binding) = self.monitors.model().binding(view) else {
            return;
        };
        match action {
            MonitorAction::Contact(contact) => {
                if contact.source() != binding.source.chat() || (view == View::Cycling && self.monitors.model().playing()) {
                    return;
                }
                match contact {
                    super::chat_ui::ContactAction::Refresh(source) => {
                        let _ = self.controller.refresh_contacts(source);
                    }
                    super::chat_ui::ContactAction::Search { source, revision, query } => {
                        let _ = self.controller.search_contacts(source, revision, query);
                    }
                    super::chat_ui::ContactAction::Open(draft) => match self.controller.open_direct(draft.request) {
                        Ok(ticket) => self.shared_chat.queued_open(draft, ticket),
                        Err(_) => self.shared_chat.open_queue_failed(draft),
                    },
                }
            }
            MonitorAction::Send(draft) => {
                if draft.key.source != binding.source.chat() || (view == View::Cycling && self.monitors.model().playing()) {
                    return;
                }
                match self.controller.send_chat(draft.key, draft.policy_revision, draft.text.clone()) {
                    Ok(request) => self.shared_chat.queued(&draft, request),
                    Err(error) => self.shared_chat.queue_failed(&draft, error),
                }
            }
            MonitorAction::Close(draft) => {
                if draft.capture.key.source != binding.source.chat() || (view == View::Cycling && self.monitors.model().playing()) {
                    return;
                }
                match self.controller.close_chat(draft.request(0)) {
                    Ok(ticket) => self.shared_chat.queued_close(draft, ticket),
                    Err(error) => self.shared_chat.close_queue_failed(draft, error),
                }
            }
            MonitorAction::Read(draft) => {
                if draft.key.source != binding.source.chat() || (view == View::Cycling && self.monitors.model().playing()) {
                    return;
                }
                match self.controller.mark_chat_read(draft.request(0)) {
                    Ok(request) => self.shared_chat.queued_read(draft, request),
                    Err(_) => self.shared_chat.read_queue_failed(draft),
                }
            }
            action @ MonitorAction::Control { .. } => {
                if !self.monitors.validate(&action) {
                    return;
                }
                let MonitorAction::Control { command, .. } = &action else { return };
                if matches!(command, Command::Navigate(_) | Command::Play | Command::Exchange(_)) && !roster_current {
                    return;
                }
                if matches!(command, Command::Settings { .. } | Command::Exchange(_)) {
                    // Never upgrade a view-only fallback to backend authority.
                    // A separately acquired genuine current command read is
                    // required, and Controller checks it again before enqueue.
                    let Some(namespace) = self.monitors.namespace(view) else { return };
                    let Ok(command_read) = self.monitor_reader.try_read(view, namespace) else {
                        return;
                    };
                    if !self.monitors.current_read(view, &command_read) || !command_read.commands_current() {
                        return;
                    }
                }
                let mut observed_pin = None;
                if let MonitorAction::Control {
                    command: Command::Pin(_, desired),
                    ..
                } = &action
                {
                    observed_pin = self.monitor_windows[index(view)]
                        .as_mut()
                        .and_then(|window| window.pin(event_loop, *desired, &mut self.shared_chat));
                }
                let viewport = self.surface.as_ref().and_then(|surface| {
                    super::review_viewport(
                        &self.snapshot,
                        surface.viewport(self.world, self.context.pixels_per_point()),
                        surface.window.is_visible() == Some(true) && surface.window.is_minimized() == Some(false),
                        self.layout_dirty || self.geometry_dirty || self.transition.waiting() || self.attention_ui.review_busy(),
                    )
                });
                let exchange = matches!(
                    &action,
                    MonitorAction::Control {
                        command: Command::Exchange(_),
                        ..
                    }
                );
                let chat_transition = matches!(
                    &action,
                    MonitorAction::Control {
                        command: Command::Chat(_, _),
                        ..
                    }
                );
                let now = self.monitor_time();
                let windows = &mut self.monitor_windows;
                let chat = &mut self.shared_chat;
                let result = self.monitors.control(
                    &action,
                    &self.controller,
                    viewport,
                    now,
                    observed_pin,
                    &mut |view, binding, state| flush_view(windows, chat, event_loop, view, binding, state),
                );
                if exchange && result == Ok(true) {
                    // Host frontdoor already closes capture synchronously. Cover
                    // the old world until fresh exact Coordinator geometry wins.
                    self.close_region();
                    if let Some(request) = self.monitors.exchange_request() {
                        self.transition.sent(request);
                    }
                }
                if let Err(error) = result {
                    self.monitor_error(error);
                }
                if chat_transition
                    && result == Ok(true)
                    && let Some(window) = &mut self.monitor_windows[index(view)]
                {
                    window.cancel_focus(&mut self.shared_chat);
                }
                if let Err(error) = self.publish_monitors(event_loop) {
                    self.monitor_error(error);
                }
                self.sync_monitor_windows(event_loop);
            }
        }
        self.chat_changed = true;
    }

    pub(super) fn poll_monitors(&mut self, event_loop: &ActiveEventLoop, now: Instant, changed: bool) -> Option<Instant> {
        let refreshed = self.refresh_monitors(event_loop);
        let fresh = refreshed.is_ok();
        if let Err(error) = refreshed {
            self.ui.monitor_sources.clear();
            self.ui.monitor_cycle_ready = false;
            self.ui.monitor_recovery = None;
            if !matches!(error, PreviewError::Busy | PreviewError::Stale) {
                self.monitor_error(error);
            }
        }
        let blocked = self.ui.modal_open() || self.closing.is_some() || self.snapshot.stopped;
        let input_busy = !fresh
            || self.monitor_windows[index(View::Cycling)]
                .as_ref()
                .is_some_and(MonitorWindow::timer_busy);
        if let Some(scope) = self.monitors.model().cycle_scope() {
            let current = self
                .snapshot
                .mcp
                .authority
                .as_ref()
                .and_then(|authority| {
                    let global = authority.policy().ok()?.enabled
                        && authority.permission_levels().ok()?.get(aurora_mcp::wire::PermissionClass::Monitors)
                            != aurora_mcp::authority::AuthorityLevel::Never;
                    let profiles = authority.profile_access().ok()?;
                    Some(
                        global
                            && scope.iter().all(|source| {
                                profiles.iter().any(|profile| {
                                    profile.ticket.binding
                                        == Some(aurora_session::Binding {
                                            worker: source.worker(),
                                            session: Some(source.session()),
                                        })
                                        && profile.permissions.is_some_and(|permissions| {
                                            permissions.get(aurora_mcp::wire::PermissionClass::Monitors)
                                                != aurora_mcp::authority::AuthorityLevel::Never
                                        })
                                })
                            }),
                    )
                })
                .unwrap_or(false);
            if !current {
                let _ = self.monitors.pause_mcp_cycle();
            }
        }
        let time = self.monitor_time();
        let windows = &mut self.monitor_windows;
        let chat = &mut self.shared_chat;
        let result = self.monitors.tick(time, blocked, input_busy, &mut |view, binding, state| {
            flush_view(windows, chat, event_loop, view, binding, state)
        });
        if let Err(error) = result {
            self.monitor_error(error);
        }
        if let Err(error) = self.publish_monitors(event_loop) {
            self.monitor_error(error);
        }
        self.sync_monitor_windows(event_loop);
        let mut next = None;
        for window in self.monitor_windows.iter_mut().flatten() {
            if let Some(due) = window.poll(event_loop, changed, now, &mut self.shared_chat) {
                next = Some(next.map_or(due, |old: Instant| old.min(due)));
            }
        }
        self.mcp_monitor_actions(event_loop, blocked || !fresh);
        if let Err(error) = self.publish_monitors(event_loop) {
            self.monitor_error(error);
        }
        // Timers remain CPU local and bounded; never busy-spin on contended IO.
        next.or_else(|| now.checked_add(Duration::from_millis(100)))
    }
    pub(super) fn hide_monitors(&mut self, event_loop: &ActiveEventLoop) {
        for view in View::ALL {
            let windows = &mut self.monitor_windows;
            let chat = &mut self.shared_chat;
            let _ = self.monitors.hide(view, &mut |view, binding, state| {
                flush_view(windows, chat, event_loop, view, binding, state)
            });
        }
        let _ = self.publish_monitors(event_loop);
        self.retire_monitor_windows();
    }
    pub(super) fn retire_monitor_windows(&mut self) {
        self.monitor_focus = None;
        self.monitor_reads.clear();
        for entry in &mut self.monitor_windows {
            if let Some(mut window) = entry.take() {
                window.retire(&mut self.shared_chat);
            }
        }
    }
}
