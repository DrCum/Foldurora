//! Viewer-side host bridge. Bootstrap is validated before profile writes;
//! window/input effects are applied only on this viewer's own event thread.

use super::transport::{self, WorkerConfig, WorkerIo};
use aurora_session::{AccountKey, Command, Event, GridKey, Message, RejectReason, WorkerEffects, WorkerGuard, WorkerMode, WorkerStatus};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

struct Startup {
    config: WorkerConfig,
    directory: PathBuf,
}
static STARTUP: OnceLock<Startup> = OnceLock::new();

pub(crate) fn initialize_startup(path: Option<&Path>) -> anyhow::Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    let cli = crate::cli::get();
    anyhow::ensure!(
        !cli.vr && cli.url.is_none() && cli.grid.is_none(),
        "host workers cannot override their bound login context"
    );
    let (config, directory) = transport::read_config(path)?;
    anyhow::ensure!(
        cli.saved_account.as_deref() == config.expected_account.as_ref().map(ToString::to_string).as_deref(),
        "worker account selection differs from its private configuration"
    );
    // A bootstrap belongs to one process launch only. Claim it before even
    // activating its profile, so a replay cannot write into a live profile.
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let claim = options.open(directory.join("consumed"))?;
    drop(claim);
    super::profile::activate(
        &config.identity.installation.to_string(),
        uuid::Uuid::from_bytes(*config.session.as_bytes()),
        config.generation,
    )?;
    STARTUP
        .set(Startup { config, directory })
        .map_err(|_| anyhow::anyhow!("worker bootstrap already initialized"))?;
    Ok(())
}

pub(crate) struct WorkerBridge {
    config: WorkerConfig,
    io: WorkerIo,
    guard: WorkerGuard,
    since: Instant,
    detached: bool,
    generation: u64,
    previous_account: Option<(AccountKey, GridKey)>,
    presentation: NativePresentation,
}

impl WorkerBridge {
    pub fn from_startup() -> anyhow::Result<Option<Self>> {
        let Some(startup) = STARTUP.get() else {
            return Ok(None);
        };
        let io = WorkerIo::start(startup.config.clone(), startup.directory.clone())?;
        let guard = WorkerGuard::new(startup.config.identity, startup.config.lease_epoch, 0)?;
        Ok(Some(Self {
            config: startup.config.clone(),
            io,
            guard,
            since: Instant::now(),
            detached: false,
            generation: startup.config.generation - 1,
            previous_account: None,
            presentation: Default::default(),
        }))
    }

    pub fn tick(&mut self, mut status: WorkerStatus, account: Option<(AccountKey, GridKey)>) -> WorkerEffects {
        if self.detached {
            return WorkerEffects::default();
        }
        if account != self.previous_account {
            if account.is_some() {
                let Some(generation) = self.generation.checked_add(1) else {
                    self.detached = true;
                    self.io.stop();
                    return WorkerEffects {
                        release_input: true,
                        restore_window: true,
                        ..Default::default()
                    };
                };
                self.generation = generation;
            }
            self.previous_account = account;
        }
        let session = account.map(|(account, grid)| {
            let mut binding = self.config.session_binding(account, grid);
            binding.generation = self.generation;
            binding
        });
        let mismatch = account.is_some_and(|(account, grid)| {
            self.config.expected_account.is_some_and(|expected| expected != account)
                || self.config.expected_grid.is_some_and(|expected| expected != grid)
        });
        if mismatch || self.io.failed() {
            self.detached = true;
            self.io.stop();
            return WorkerEffects {
                release_input: true,
                restore_window: true,
                ..Default::default()
            };
        }
        let mut effects = self.guard.update_session(session).unwrap_or_default();
        let now = self.since.elapsed().as_millis().min(u64::MAX as u128) as u64;
        for envelope in self.io.commands() {
            let Message::Command(command) = &envelope.message else {
                continue;
            };
            // No native adoption can be claimed until the renderer exposes
            // an ordered GPU completion fence. Reject before changing mode
            // or styles; an ordinary manual-login window remains reachable.
            if command.session_bound() {
                if envelope.binding == self.guard.binding() && envelope.lease_epoch == self.config.lease_epoch {
                    self.io.reply(
                        self.guard.binding(),
                        envelope.sequence,
                        Event::Rejected {
                            activation: command.activation(),
                            reason: RejectReason::Unsupported,
                        },
                    );
                }
                continue;
            }
            if let Ok(transition) = self.guard.on_command(&envelope, now) {
                merge_effects(&mut effects, transition.effects);
                if let Some(reply) = transition.reply {
                    if !self.io.reply(self.guard.binding(), reply.reply_to, reply.event) {
                        self.detached = true;
                    }
                } else if matches!(command, Command::Heartbeat { .. }) {
                    self.io
                        .reply(self.guard.binding(), envelope.sequence, Event::Status(status.clone()));
                }
            }
        }
        merge_effects(&mut effects, self.guard.tick(now));
        if self.guard.mode() == WorkerMode::Detached || self.detached {
            self.detached = true;
            effects.release_input = true;
            effects.restore_window = true;
            self.io.stop();
        }
        // No proof is inferred from the CPU-side queued-present statistic.
        status.last_frame = None;
        self.io.status(self.guard.binding(), status);
        effects
    }

    pub fn restore(&mut self, window: &winit::window::Window) {
        self.presentation.restore(window);
    }
}

fn merge_effects(target: &mut WorkerEffects, next: WorkerEffects) {
    target.release_input |= next.release_input;
    target.quiesce |= next.quiesce;
    target.prepare |= next.prepare;
    target.grant_input |= next.grant_input;
    target.restore_window |= next.restore_window;
    target.apply_viewport |= next.apply_viewport;
}

pub(crate) fn native_window(window: &winit::window::Window) -> u64 {
    #[cfg(windows)]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            return handle.hwnd.get() as u64;
        }
    }
    let _ = window;
    0
}

/// Worker-owned top-level window styles, following `FSSessionWorker::embed`
/// (`indra/newview/fssessionworker.cpp`, pinned abbe3cb0, lines 588/640).
/// This deliberately never reparents a foreign window. It stays dormant
/// until a completed prepared-frame acknowledgment is available.
#[derive(Default)]
struct NativePresentation {
    #[cfg(windows)]
    saved: Option<native::Saved>,
}

impl NativePresentation {
    fn restore(&mut self, window: &winit::window::Window) {
        #[cfg(windows)]
        if let Some(saved) = self.saved.take() {
            native::restore(window, saved);
        }
        let _ = window;
    }
}

#[cfg(windows)]
mod native {
    use windows_sys::Win32::{
        Foundation::{HWND, RECT},
        UI::WindowsAndMessaging::{
            GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetWindow, GetWindowLongPtrW, GetWindowRect, IsIconic, IsZoomed, SWP_FRAMECHANGED,
            SWP_NOACTIVATE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW, WS_THICKFRAME,
        },
    };
    use winit::{platform::windows::WindowExtWindows, window::Window};

    pub(super) struct Saved {
        style: isize,
        extended: isize,
        rect: RECT,
        maximized: bool,
        minimized: bool,
    }

    #[allow(dead_code)]
    pub(super) fn fit(window: &Window, viewport: aurora_session::HostedViewport) -> std::io::Result<Saved> {
        viewport.validate().map_err(std::io::Error::other)?;
        let hwnd = super::native_window(window) as HWND;
        if hwnd.is_null() {
            return Err(std::io::Error::other("worker has no native window"));
        }
        // SAFETY: hwnd is this viewer's own winit window on its event thread.
        // Read-only owner check prevents treating a dialog as a world window.
        unsafe {
            if !GetWindow(hwnd, GW_OWNER).is_null() {
                return Err(std::io::Error::other("worker world window has a native owner"));
            }
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut rect) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let saved = Saved {
                style: GetWindowLongPtrW(hwnd, GWL_STYLE),
                extended: GetWindowLongPtrW(hwnd, GWL_EXSTYLE),
                rect,
                maximized: IsZoomed(hwnd) != 0,
                minimized: IsIconic(hwnd) != 0,
            };
            window.set_maximized(false);
            window.set_minimized(false);
            window.set_decorations(false);
            window.set_skip_taskbar(true);
            SetWindowLongPtrW(hwnd, GWL_STYLE, saved.style & !((WS_CAPTION | WS_THICKFRAME) as isize));
            SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                (saved.extended & !(WS_EX_APPWINDOW as isize)) | WS_EX_TOOLWINDOW as isize,
            );
            let rect = viewport.rect;
            if SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                rect.x,
                rect.y,
                rect.width as i32,
                rect.height as i32,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOZORDER,
            ) == 0
            {
                restore(window, saved);
                return Err(std::io::Error::last_os_error());
            }
            Ok(saved)
        }
    }

    pub(super) fn restore(window: &Window, saved: Saved) {
        let hwnd = super::native_window(window) as HWND;
        if hwnd.is_null() {
            return;
        }
        window.set_decorations(saved.style & WS_CAPTION as isize != 0);
        window.set_skip_taskbar(false);
        // SAFETY: only styles and saved geometry of this process's own window
        // are restored. No foreign owner, parent, input or process is changed.
        unsafe {
            SetWindowLongPtrW(hwnd, GWL_STYLE, saved.style);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, saved.extended);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                saved.rect.left,
                saved.rect.top,
                saved.rect.right - saved.rect.left,
                saved.rect.bottom - saved.rect.top,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
        window.set_maximized(saved.maximized);
        // A detached worker must be reachable, including after host failure.
        let _ = saved.minimized;
        window.set_minimized(false);
        window.set_visible(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_effects_always_preserve_input_release_and_window_restoration() {
        let mut effects = WorkerEffects {
            quiesce: true,
            ..Default::default()
        };
        merge_effects(
            &mut effects,
            WorkerEffects {
                release_input: true,
                restore_window: true,
                ..Default::default()
            },
        );
        assert!(effects.quiesce && effects.release_input && effects.restore_window);
    }
}
