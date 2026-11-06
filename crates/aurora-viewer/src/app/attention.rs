//! What `world/attention.rs` records, decided and done: the taskbar flash
//! (LLWindowWin32::flashIcon, indra/llwindow/llwindowwin32.cpp) and the
//! system notification (GrowlManager::performNotification / shouldNotify,
//! indra/newview/growlmanager.cpp, originally LGPL 2.1), and the click on a
//! notification, which brings the window back to what it told.

use super::App;
use crate::keywords::KeywordSettings;
use crate::settings::Settings;
use crate::world::attention::{Cause, Notice, Open};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// GROWL_THROTTLE_TIME: a second notification with the same title within
/// this time is dropped.
const THROTTLE: Duration = Duration::from_secs(1);
/// GROWL_THROTTLE_CLEANUP_PERIOD.
const THROTTLE_CLEANUP: Duration = Duration::from_secs(300);
/// GROWL_MAX_BODY_LENGTH (characters here, bytes there).
const MAX_BODY: usize = 255;
/// The notifications whose click is still remembered.
const KEEP_OPENS: usize = 64;

#[derive(Default)]
pub struct State {
    /// Started with the first notification (the setting off: never).
    notifier: Option<crate::system_notify::Notifier>,
    throttle: Throttle,
    next_id: u64,
    /// What a click on each notification shown opens.
    opens: VecDeque<(u64, Open)>,
    /// A flash asked during the frame, done once the window is back.
    flash: bool,
    /// The window is not the foreground window, or is minimized, as
    /// Windows says between frames (LLWindowWin32::flashIcon asks
    /// GetForegroundWindow): the focus events can miss a minimize.
    behind: bool,
    /// The last caption belongs to one native window. GPU recovery creates
    /// another window even when its account and unread count stay unchanged.
    title: Option<(winit::window::WindowId, String)>,
}

/// The titles told lately (GrowlManager::mTitleTimers).
#[derive(Default)]
struct Throttle {
    seen: HashMap<String, Instant>,
    cleaned: Option<Instant>,
}

impl Throttle {
    /// The title may be told now. A repeat within the second is dropped and
    /// starts the second again, as performNotification does.
    fn allow(&mut self, title: &str, now: Instant) -> bool {
        if self.cleaned.is_none_or(|t| now.duration_since(t) >= THROTTLE_CLEANUP) {
            self.seen.clear();
            self.cleaned = Some(now);
        }
        let repeat = self.seen.get(title).is_some_and(|t| now.duration_since(*t) < THROTTLE);
        self.seen.insert(title.to_owned(), now);
        !repeat
    }
}

/// Whether a cause flashes the taskbar, by the settings
/// (processNewMessage: FSFlashOnMessage, FSFlashOnObjectIM for the objects'
/// messages; LLScriptFloaterManager: FSFlashOnScriptDialog).
pub fn flashes(cause: Cause, s: &Settings) -> bool {
    match cause {
        Cause::Message | Cause::Mention => s.flash_on_message,
        Cause::ObjectIm => s.flash_on_message && s.flash_on_object_im,
        Cause::ScriptDialog => s.flash_on_script_dialog,
        // FSKeywords::notify plays its sound and tells Growl, no flash
        Cause::Keyword | Cause::Notice => false,
    }
}

/// The keyword sound to play (PlayModeUISndFSKeywordSound,
/// UISndFSKeywordSound), under the "Interface sounds" switch.
pub fn keyword_sound(s: &Settings, k: &KeywordSettings) -> Option<uuid::Uuid> {
    (s.audio.ui_sounds && k.play_sound && !k.sound.is_nil()).then_some(k.sound)
}

/// Whether the system is told now (GrowlManager::shouldNotify): turned on,
/// never in do not disturb, and only while the window is not in front
/// unless FSGrowlWhenActive. `forced`: turned on by AURORA_DEMO_ATTENTION.
pub fn notifies(s: &Settings, forced: bool, in_front: bool, dnd: bool) -> bool {
    (s.system_notifications || forced) && !dnd && (s.system_notifications_when_active || !in_front)
}

/// LLChicletBar's counter: at most 99, then "99+" (max_displayed_count).
const MAX_TITLE_COUNT: usize = 99;
/// LLStringUtil::truncate(window_title, 255), in characters here.
const MAX_TITLE: usize = 255;

/// The window title with the unread IMs before it, "(3) Aurora Viewer…"
/// (FSShowMessageCountInWindowTitle,
/// LLIMWellChiclet::updateApplicationWindowTitle, llchiclet.cpp).
pub fn titled(title: &str, unread: usize, on: bool) -> String {
    if !on || unread == 0 {
        return title.to_owned();
    }
    let full = if unread > MAX_TITLE_COUNT {
        format!("({MAX_TITLE_COUNT}+) {title}")
    } else {
        format!("({unread}) {title}")
    };
    full.chars().take(MAX_TITLE).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptionPhase {
    Login,
    Connecting,
    Online,
}

impl CaptionPhase {
    fn for_screen(screen: &super::Screen, busy: bool) -> Self {
        match screen {
            super::Screen::World => Self::Online,
            super::Screen::Loading { .. } => Self::Connecting,
            super::Screen::Login if busy => Self::Connecting,
            super::Screen::Login => Self::Login,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Login => aurora_i18n::tr!("character-window-login"),
            Self::Connecting => aurora_i18n::tr!("character-window-connecting"),
            Self::Online => aurora_i18n::tr!("character-window-online"),
        }
    }
}

/// Explicit titles identify automation/custom runs. Only the exact opaque
/// label generated by Characters is replaced; its CLI/log/IPC label stays put.
fn automatic_caption(title: Option<&str>, saved_account: Option<&str>) -> bool {
    title.is_none_or(|title| {
        saved_account
            .filter(|id| crate::characters::valid_id(id))
            .is_some_and(|id| title == format!("Character {}", &id[..8]))
    })
}

/// Account/catalogue text must not inject a second line or direction overrides
/// into the OS caption. Bound both the work and the displayed Unicode text.
fn caption_text(text: &str, limit: usize) -> String {
    let clean: String = text
        .chars()
        .take(512)
        .filter(|c| {
            (!c.is_control() || c.is_whitespace())
                && !matches!(*c, '\u{200b}' | '\u{200e}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
        })
        .collect();
    let flat = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = flat.chars();
    let mut result: String = chars.by_ref().take(limit).collect();
    if limit > 0 && chars.next().is_some() {
        result.pop();
        result.push('…');
    }
    result
}

fn caption_grid(uri: &str, grids: &[crate::grids::Grid]) -> String {
    const LIMIT: usize = 48;
    match crate::login_accounts::grid(uri) {
        crate::settings::GridChoice::SecondLife => aurora_i18n::tr!("character-window-main-grid").into(),
        crate::settings::GridChoice::SecondLifeBeta => aurora_i18n::tr!("character-switch-beta-grid").into(),
        crate::settings::GridChoice::Custom => {
            // Paths/schemes can select different grids on the same host,
            // and friendly labels need not be unique. An opaque URI-only
            // tag distinguishes them without exposing endpoint details.
            let identity = crate::characters::identity(uri, "");
            let tag = &identity[..8];
            let budget = LIMIT - 3 - tag.len();
            let label = grids
                .iter()
                .find(|grid| grid.login.trim() == uri.trim())
                // A pasted endpoint is not a display label. In particular,
                // never echo its userinfo, path, query or fragment.
                .filter(|grid| !grid.name.contains(['/', '\\', '@', '?', '#', ':']))
                .map(|grid| caption_text(&grid.name, budget))
                .filter(|label| !label.is_empty())
                .or_else(|| {
                    reqwest::Url::parse(uri.trim())
                        .ok()
                        .filter(|url| matches!(url.scheme(), "http" | "https"))
                        .and_then(|url| {
                            let host = url.host_str()?;
                            let port = url.port().map_or_else(String::new, |port| format!(":{port}"));
                            Some(format!("{}{port}", caption_text(host, budget - port.len())))
                        })
                })
                .unwrap_or_else(|| caption_text(aurora_i18n::tr!("character-switch-custom-grid"), budget));
            format!("{label} · {tag}")
        }
    }
}

fn account_caption(viewer: &str, phase: CaptionPhase, account: Option<(&str, &str)>) -> String {
    let account = account
        .map(|(name, grid)| (caption_text(name, 64), caption_text(grid, 48)))
        .filter(|(name, _)| !name.is_empty());
    match account {
        Some((name, grid)) => aurora_i18n::tr!(
            "character-window-title",
            account = name.as_str(),
            grid = grid.as_str(),
            phase = phase.label(),
            viewer = viewer,
        ),
        None => aurora_i18n::tr!("character-window-login-title", phase = phase.label(), viewer = viewer),
    }
}

fn selected_caption(viewer: &str, phase: CaptionPhase, ready: bool, adding: bool, username: &str, grid: &str) -> String {
    // A child must not show the profile inherited from the global settings
    // before its exact selection completes. An Add draft is not that profile.
    let selected = ready && (!adding || phase != CaptionPhase::Login);
    account_caption(viewer, phase, selected.then_some((username, grid)))
}

fn decorated_caption(base: &str, unread: usize, on: bool, paused: bool) -> String {
    let title = titled(base, unread, on);
    if paused {
        aurora_i18n::tr!("busy-pause-window-title", title = title.as_str())
    } else {
        title
    }
}

fn caption_changed<Id: PartialEq>(shown: &mut Option<(Id, String)>, window: Id, want: &str) -> bool {
    if shown
        .as_ref()
        .is_some_and(|(old_window, title)| *old_window == window && title == want)
    {
        return false;
    }
    *shown = Some((window, want.to_owned()));
    true
}

/// What a notification opens, without its session (for the log).
fn kind_of(open: Open) -> &'static str {
    match open {
        Open::Conversation(_) => "conversation",
        Open::NearbyChat => "nearby chat",
        Open::Notifications => "notifications",
        Open::Window => "window",
    }
}

/// The body as the system shows it: one line of plain text, cut at
/// GROWL_MAX_BODY_LENGTH.
fn body_text(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(MAX_BODY) {
        Some((i, _)) => format!("{}…", &flat[..i]),
        None => flat,
    }
}

impl App {
    /// Each frame in world: what the world asked for since the last frame.
    pub(super) fn update_attention(&mut self) {
        let mut keyword_sound = None;
        for a in std::mem::take(&mut self.world.attention) {
            if self.attention.behind && flashes(a.cause, &self.settings) {
                self.flash_taskbar();
            }
            if a.cause == Cause::Keyword {
                // once a frame, however many lines arrived together
                keyword_sound = self::keyword_sound(&self.settings, &self.keyword_settings());
            }
            if let Some(notice) = a.notice {
                self.tell_system(notice);
            }
        }
        if let Some(id) = keyword_sound {
            log::debug!(target: "ui_sound", "UISndFSKeywordSound ({id})");
            self.scene.sounds.want_kept(id);
            self.scene.sounds.play_ui(self.audio_engine.as_ref(), id);
        }
    }

    /// The keyword alerts in force: AURORA_DEMO_KEYWORDS's for the run
    /// (never saved), else the preferences'.
    pub(super) fn keyword_settings(&self) -> KeywordSettings {
        match crate::demo::keywords::settings() {
            Some(k) if self.demo => k,
            _ => self.settings.keywords.clone(),
        }
    }

    /// One system notification, when the settings, the focus, do not
    /// disturb and the throttle let it through.
    pub(super) fn tell_system(&mut self, notice: Notice) {
        let forced = self.demo && crate::demo::attention::mode().is_some();
        if !notifies(&self.settings, forced, !self.attention.behind, self.world.status.dnd) {
            return;
        }
        let world = &self.world;
        let name = |id: &uuid::Uuid| world.social.avatar_names.complete(id);
        // a label still waiting for its name keeps the address: the
        // notification is not drawn again once it comes (#947)
        let app = |url: &str, link: &crate::slapp::AppLink| match crate::ui::chat::app_label(world, url, link) {
            label if label == "…" => crate::slapp::unescape(url),
            label => label,
        };
        let title = body_text(&crate::ui::chat::plain_text(&notice.title, name, app));
        if title.is_empty() {
            return;
        }
        if !self.attention.throttle.allow(&title, Instant::now()) {
            log::info!("system notification dropped by the throttle");
            return;
        }
        let body = body_text(&crate::ui::chat::plain_text(&notice.body, name, app));
        log::info!("system notification shown ({})", kind_of(notice.open));
        let st = &mut self.attention;
        st.next_id += 1;
        let id = st.next_id;
        st.opens.push_back((id, notice.open));
        if st.opens.len() > KEEP_OPENS {
            st.opens.pop_front();
        }
        st.notifier
            .get_or_insert_with(crate::system_notify::Notifier::start)
            .show(crate::system_notify::Toast { id, title, body });
    }

    /// Between frames, the window at hand (the frame holds it): whether it
    /// is in front, the flash asked for, and the clicks on the notifications.
    pub(super) fn attention_window(&mut self) {
        if let Some(g) = &self.gfx {
            self.attention.behind = !window_in_front(&g.window);
        }
        self.unread_title();
        if std::mem::take(&mut self.attention.flash)
            && let Some(g) = &self.gfx
        {
            #[cfg(windows)]
            {
                log::info!("taskbar flash");
                flash_window(&g.window);
            }
            #[cfg(not(windows))]
            if flashes_here(g.window.is_minimized()) {
                log::info!("taskbar flash");
                g.window
                    .request_user_attention(Some(winit::window::UserAttentionType::Informational));
            } else {
                log::debug!("taskbar flash skipped: the window is not minimized");
            }
        }
        self.attention_clicks();
    }

    /// One caption source for initial creation and subsequent updates. Keep
    /// the selected login separate from its opaque process/logging identity.
    pub(super) fn native_window_title(&self) -> String {
        let cli = crate::cli::get();
        let in_world = matches!(self.screen, super::Screen::World);
        let on = self.settings.unread_ims_in_window_title || (self.demo && crate::demo::im_announce::forced());
        let unread = if in_world { self.world.conversations_badge() } else { 0 };
        let base = if automatic_caption(cli.title.as_deref(), cli.saved_account.as_deref()) {
            let phase = CaptionPhase::for_screen(&self.screen, self.login_form.busy);
            let ready = cli.saved_account.is_none() || self.characters.startup_applied;
            let uri = self.settings.login_uri();
            let grid = caption_grid(&uri, &self.settings.grids);
            let viewer = aurora_i18n::tr!("cli-aurora-viewer", version = aurora_version::version());
            let viewer = crate::cli::profile_label(env!("AURORA_BUILD_PROFILE"))
                .map_or_else(|| viewer.clone(), |profile| format!("{viewer} ({profile})"));
            selected_caption(&viewer, phase, ready, self.login_form.adding, &self.settings.username, &grid)
        } else {
            crate::cli::window_title()
        };
        decorated_caption(&base, unread, on, self.busy_pause.paused())
    }

    /// Counted like the Conversations button (IMs, group chats, conferences),
    /// in world only; native calls happen only on changes or a new window.
    fn unread_title(&mut self) {
        let Some(g) = &self.gfx else {
            return;
        };
        let want = self.native_window_title();
        if caption_changed(&mut self.attention.title, g.window.id(), &want) {
            g.window.set_title(&want);
        }
    }

    /// A notification clicked: the window comes back (restored, in front)
    /// on its conversation, the nearby chat or the bell's list.
    fn attention_clicks(&mut self) {
        let Some(notifier) = &self.attention.notifier else {
            return;
        };
        let mut clicked = Vec::new();
        while let Some(id) = notifier.clicked() {
            clicked.push(id);
        }
        for id in clicked {
            let open = self.attention.opens.iter().find(|(i, _)| *i == id).map(|(_, o)| *o);
            if let Some(g) = &self.gfx {
                g.window.set_minimized(false);
                g.window.focus_window();
            }
            if !matches!(self.screen, super::Screen::World) {
                continue;
            }
            match open {
                Some(Open::Conversation(session)) => {
                    if !self.world.rlva_can_start_im(session) {
                        continue;
                    }
                    self.world.social.session_mut(session);
                    self.world.social.focus_im = Some(session);
                    crate::ui::notifications::open_conversations(&self.egui_ctx, &mut self.panels.chat);
                }
                Some(Open::NearbyChat) => {
                    self.world.social.focus_im = Some(uuid::Uuid::nil());
                    crate::ui::notifications::open_conversations(&self.egui_ctx, &mut self.panels.chat);
                }
                Some(Open::Notifications) => self.notif_ui.open = true,
                Some(Open::Window) | None => {}
            }
        }
    }

    /// LLWindow::flashIcon(5.f): the taskbar button flashes ten times in
    /// five seconds, then stays lit until the window is activated
    /// (LLWindowWin32::flashIcon: FLASHW_TRAY, ICON_FLASH_TIME 0.5 s).
    pub(super) fn flash_taskbar(&mut self) {
        self.attention.flash = true;
    }

    /// The window is not in front (another window has the foreground, or
    /// it is minimized).
    pub(super) fn window_behind(&self) -> bool {
        self.attention.behind
    }
}

/// The window is the foreground window and not minimized.
#[cfg(windows)]
fn window_in_front(window: &winit::window::Window) -> bool {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsIconic};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return true;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return true;
    };
    let hwnd = handle.hwnd.get() as HWND;
    // SAFETY: two read-only window manager queries on the window this
    // thread created.
    unsafe { GetForegroundWindow() == hwnd && IsIconic(hwnd) == 0 }
}

#[cfg(not(windows))]
fn window_in_front(window: &winit::window::Window) -> bool {
    window.has_focus() && !window.is_minimized().unwrap_or(false)
}

/// Whether the flash asked for (the window behind) is done, by what the
/// window says of being minimized. macOS bounces the icon whenever
/// (LLWindowMacOSX::flashIcon). Linux flashes only a minimized window
/// (LLWindowSDL::flashIcon, indra/llwindow/llwindowsdl.cpp): on Wayland a
/// window cannot know it (`None`), so it never flashes there. Wayland's
/// attention request is an xdg_activation one, and a compositor that grants
/// it switches to the viewer's workspace and focuses it.
#[cfg(not(windows))]
fn flashes_here(minimized: Option<bool>) -> bool {
    cfg!(target_os = "macos") || minimized == Some(true)
}

#[cfg(windows)]
fn flash_window(window: &winit::window::Window) {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FLASHW_TRAY, FLASHWINFO, FlashWindowEx};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let info = FLASHWINFO {
        cbSize: size_of::<FLASHWINFO>() as u32,
        hwnd: handle.hwnd.get() as HWND,
        dwFlags: FLASHW_TRAY,
        uCount: 10,
        dwTimeout: 500,
    };
    // SAFETY: a window manager call on the window this thread created,
    // with a filled FLASHWINFO of the right size.
    unsafe {
        FlashWindowEx(&info);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_repeated_within_a_second_is_dropped() {
        let mut t = Throttle::default();
        let now = Instant::now();
        assert!(t.allow("Jane", now));
        assert!(!t.allow("Jane", now + Duration::from_millis(900)));
        // the drop started the second again
        assert!(!t.allow("Jane", now + Duration::from_millis(1800)));
        assert!(t.allow("Jane", now + Duration::from_millis(2900)));
        assert!(t.allow("Tom", now + Duration::from_millis(2900)));
    }

    #[test]
    fn the_flash_follows_its_settings() {
        let mut s = Settings::default();
        assert!(flashes(Cause::Message, &s) && flashes(Cause::Mention, &s) && flashes(Cause::ObjectIm, &s));
        assert!(!flashes(Cause::ScriptDialog, &s) && !flashes(Cause::Notice, &s) && !flashes(Cause::Keyword, &s));
        s.flash_on_message = false;
        assert!(!flashes(Cause::ObjectIm, &s) && !flashes(Cause::Mention, &s));
        s.flash_on_script_dialog = true;
        assert!(flashes(Cause::ScriptDialog, &s));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_flashes_only_a_minimized_window() {
        assert!(flashes_here(Some(true)));
        assert!(!flashes_here(Some(false)), "behind but not minimized");
        assert!(!flashes_here(None), "Wayland: never, its request would focus the window");
    }

    #[test]
    fn the_keyword_sound_needs_its_switch_and_the_interface_sounds() {
        let mut s = Settings::default();
        let mut k = KeywordSettings::default();
        assert_eq!(keyword_sound(&s, &k), None, "off by default, like PlayModeUISndFSKeywordSound");
        k.play_sound = true;
        assert_eq!(keyword_sound(&s, &k), Some(crate::keywords::DEFAULT_SOUND));
        s.audio.ui_sounds = false;
        assert_eq!(keyword_sound(&s, &k), None);
        s.audio.ui_sounds = true;
        k.sound = uuid::Uuid::nil();
        assert_eq!(keyword_sound(&s, &k), None);
    }

    #[test]
    fn the_system_hears_only_in_the_background_and_never_in_do_not_disturb() {
        let mut s = Settings::default();
        assert!(!notifies(&s, false, false, false), "off by default, like FSEnableGrowl");
        assert!(notifies(&s, true, false, false), "the demo turns them on");
        s.system_notifications = true;
        assert!(notifies(&s, false, false, false));
        assert!(!notifies(&s, false, true, false));
        assert!(!notifies(&s, false, false, true));
        s.system_notifications_when_active = true;
        assert!(notifies(&s, false, true, false));
        assert!(!notifies(&s, false, true, true));
    }

    #[test]
    fn the_body_is_one_line_cut_at_255_characters() {
        assert_eq!(body_text("a\n b\tc"), "a b c");
        let long = "é".repeat(300);
        let cut = body_text(&long);
        assert_eq!(cut.chars().count(), MAX_BODY + 1);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn the_title_counts_the_unread_ims_up_to_99_plus() {
        let t = "Aurora Viewer 0.8 (Dev)";
        assert_eq!(titled(t, 3, false), t, "off by default");
        assert_eq!(titled(t, 0, true), t);
        assert_eq!(titled(t, 3, true), "(3) Aurora Viewer 0.8 (Dev)");
        assert_eq!(titled(t, 99, true), "(99) Aurora Viewer 0.8 (Dev)");
        assert_eq!(titled(t, 100, true), "(99+) Aurora Viewer 0.8 (Dev)");
        assert_eq!(titled(&"é".repeat(300), 1, true).chars().count(), MAX_TITLE);
    }

    #[test]
    fn custom_and_automation_captions_keep_their_cli_identity() {
        let id = crate::characters::identity("https://grid.invalid/login", "synthetic");
        assert!(automatic_caption(None, None));
        assert!(automatic_caption(Some(&format!("Character {}", &id[..8])), Some(&id)));
        for title in ["Test titles [AUTOMATION - DO NOT TOUCH]", "My custom session", "Character 12345678"] {
            assert!(!automatic_caption(Some(title), None));
            assert!(!automatic_caption(Some(title), Some(&id)));
        }
        assert!(!automatic_caption(Some("Character 12345678"), Some("invalid")));
        let explicit = "Aurora Viewer 0.8 - Test titles [AUTOMATION - DO NOT TOUCH] (Dev)";
        assert_eq!(decorated_caption(explicit, 0, false, false), explicit);
        assert_eq!(decorated_caption(explicit, 2, true, false), format!("(2) {explicit}"));
    }

    #[test]
    fn character_caption_tracks_selection_login_logout_and_missing_targets() {
        use super::super::Screen;
        let viewer = "Aurora Viewer 0.8 (Dev)";
        let login = CaptionPhase::for_screen(&Screen::Login, false);
        let connecting = CaptionPhase::for_screen(&Screen::Login, true);
        let loading = CaptionPhase::for_screen(&Screen::Loading { since: Instant::now() }, false);
        let online = CaptionPhase::for_screen(&Screen::World, false);
        assert_eq!(
            (login, connecting, loading, online),
            (
                CaptionPhase::Login,
                CaptionPhase::Connecting,
                CaptionPhase::Connecting,
                CaptionPhase::Online
            )
        );
        let alpha = selected_caption(viewer, login, true, false, "Synthetic Alpha", "Grid A");
        let beta = selected_caption(viewer, login, true, false, "Synthetic Beta", "Grid A");
        assert!(alpha.contains("Synthetic Alpha") && !alpha.contains("Synthetic Beta"));
        assert!(beta.contains("Synthetic Beta") && !beta.contains("Synthetic Alpha"));
        let waiting = selected_caption(viewer, connecting, true, false, "Synthetic Alpha", "Grid A");
        let active = selected_caption(viewer, online, true, false, "Synthetic Alpha", "Grid A");
        assert!(waiting.contains(connecting.label()) && active.contains(online.label()));
        assert_ne!(waiting, active);
        assert_ne!(active, alpha, "logout returns the selected profile to its manual login caption");
        assert_ne!(alpha, selected_caption(viewer, login, true, false, "Synthetic Alpha", "Grid B"));
        for empty in [
            selected_caption(viewer, login, false, false, "Inherited profile", "Inherited grid"),
            selected_caption(viewer, login, true, true, "Unsubmitted Add draft", "Draft grid"),
            selected_caption(viewer, login, true, false, "", "Previous grid"),
        ] {
            assert_eq!(empty, account_caption(viewer, login, None));
            assert!(!empty.contains("profile") && !empty.contains("grid") && !empty.contains("draft"));
        }
        assert!(selected_caption(viewer, connecting, true, true, "Synthetic New", "Grid A").contains("Synthetic New"));
        assert!(alpha.ends_with(viewer), "retain version and build profile at the end");
    }

    #[test]
    fn caption_identity_is_bounded_one_line_unicode_without_direction_spoofing() {
        assert_eq!(
            caption_text("  Élodie\n雪\t👩\u{200d}🔧\0\u{202e}\u{2066}  ", 64),
            "Élodie 雪 👩\u{200d}🔧"
        );
        let long = caption_text(&"雪".repeat(1000), 64);
        assert_eq!(long.chars().count(), 64);
        assert!(long.ends_with('…'));
        assert_eq!(caption_text("text", 0), "");
        let base = account_caption(
            "Aurora Viewer 0.8 (Dev)",
            CaptionPhase::Online,
            Some((&"雪".repeat(1000), &"é".repeat(1000))),
        );
        let titled = decorated_caption(&base, 150, true, true);
        assert!(titled.chars().count() <= MAX_TITLE);
        assert!(titled.contains("(99+)"));
        assert!(titled.contains("Aurora Viewer 0.8 (Dev)"));
        let unread_title = super::titled(&base, 150, true);
        assert_eq!(titled, aurora_i18n::tr!("busy-pause-window-title", title = unread_title.as_str()));
    }

    #[test]
    fn caption_grid_uses_short_labels_or_hosts_and_never_endpoint_secrets() {
        let uri = "https://synthetic-user:synthetic-password@grid.invalid:8002/private-path?synthetic-query=1#synthetic-fragment";
        let host = caption_grid(uri, &[]);
        assert!(host.starts_with("grid.invalid:8002 · "));
        let mut grid = crate::grids::Grid {
            login: uri.into(),
            name: "  Grille 雪\nBleue  ".into(),
            ..Default::default()
        };
        assert!(caption_grid(uri, std::slice::from_ref(&grid)).starts_with("Grille 雪 Bleue · "));
        for invalid_name in [
            uri,
            "name?synthetic-query",
            "name#synthetic-fragment",
            "name/private-path",
            "synthetic-user@grid.invalid",
        ] {
            grid.name = invalid_name.into();
            assert_eq!(caption_grid(uri, std::slice::from_ref(&grid)), host);
        }
        let invalid = caption_grid("not a URL synthetic-private-value", &[]);
        assert!(invalid.starts_with(aurora_i18n::tr!("character-switch-custom-grid")));
        assert!(!invalid.contains("synthetic-private-value"));
        assert_ne!(
            caption_grid(aurora_net::login::SL_MAIN_GRID, &[]),
            caption_grid(aurora_net::login::SL_BETA_GRID, &[])
        );
        let long_host = format!("https://{}.invalid:8003/private-path", "a".repeat(55));
        let label = caption_grid(&long_host, &[]);
        assert!(label.chars().count() <= 48 && label.contains(":8003 · "));
    }

    #[test]
    fn custom_grid_captions_distinguish_exact_endpoints_and_duplicate_friendly_labels() {
        let uris = [
            "https://synthetic-user:synthetic-password@grid.invalid/login-a?synthetic-query=1#synthetic-fragment",
            "https://synthetic-user:synthetic-password@grid.invalid/login-b?synthetic-query=1#synthetic-fragment",
            "http://synthetic-user:synthetic-password@grid.invalid/login-a?synthetic-query=1#synthetic-fragment",
        ];
        let mut grids: Vec<_> = uris
            .iter()
            .map(|uri| crate::grids::Grid {
                login: (*uri).into(),
                name: "Synthetic shared grid".into(),
                ..Default::default()
            })
            .collect();
        for friendly in [false, true] {
            let catalogue = if friendly { grids.as_slice() } else { &[] };
            let labels: Vec<_> = uris.iter().map(|uri| caption_grid(uri, catalogue)).collect();
            let unique: std::collections::BTreeSet<_> = labels.iter().collect();
            assert_eq!(
                unique.len(),
                uris.len(),
                "same host and duplicate names must retain exact grid identity"
            );
            let captions: std::collections::BTreeSet<_> = labels
                .iter()
                .map(|label| account_caption("Aurora Viewer 0.8 (Dev)", CaptionPhase::Login, Some(("Synthetic Alpha", label))))
                .collect();
            assert_eq!(
                captions.len(),
                uris.len(),
                "the same login name remains distinguishable across grids"
            );
            for (uri, label) in uris.iter().zip(labels) {
                assert!(label.chars().count() <= 48);
                assert!(label.starts_with(if friendly {
                    "Synthetic shared grid · "
                } else {
                    "grid.invalid · "
                }));
                let (_, tag) = label.rsplit_once(" · ").unwrap();
                assert_eq!(tag.len(), 8);
                assert!(tag.bytes().all(|c| c.is_ascii_hexdigit()));
                assert_eq!(
                    label,
                    caption_grid(&format!(" {uri} "), catalogue),
                    "identity trims only endpoint whitespace"
                );
                for hidden in [
                    "synthetic-user",
                    "synthetic-password",
                    "login-a",
                    "login-b",
                    "synthetic-query",
                    "synthetic-fragment",
                    "://",
                    "?",
                    "#",
                    "@",
                ] {
                    assert!(!label.contains(hidden), "endpoint details must not enter the caption: {label}");
                }
            }
        }
        grids[0].name = "雪".repeat(1000);
        let label = caption_grid(uris[0], &grids);
        assert_eq!(label.chars().count(), 48);
        assert!(label.starts_with('雪') && label.contains("… · "));
        let (_, tag) = label.rsplit_once(" · ").unwrap();
        assert_eq!(tag.len(), 8, "bounded labels must retain the distinguishing tag");
        assert_eq!(
            caption_grid(aurora_net::login::SL_MAIN_GRID, &[]),
            aurora_i18n::tr!("character-window-main-grid")
        );
        assert_eq!(
            caption_grid(aurora_net::login::SL_BETA_GRID, &[]),
            aurora_i18n::tr!("character-switch-beta-grid")
        );
    }

    #[test]
    fn native_caption_is_reapplied_to_a_recreated_window_without_other_changes() {
        let mut shown = None;
        let title = account_caption("Aurora Viewer 0.8 (Dev)", CaptionPhase::Online, Some(("Synthetic Alpha", "Grid A")));
        assert!(caption_changed(&mut shown, 1u8, &title));
        assert!(!caption_changed(&mut shown, 1u8, &title));
        assert!(
            caption_changed(&mut shown, 2u8, &title),
            "the replacement window needs its own native caption update"
        );
        assert!(!caption_changed(&mut shown, 2u8, &title));
        assert!(
            caption_changed(&mut shown, 2u8, &format!("(1) {title}")),
            "unread indicators remain part of the same writer"
        );
    }

    #[test]
    fn character_caption_templates_resolve_in_every_supported_language() {
        for language in ["en", "fr", "ja"] {
            let phase = aurora_i18n::text_in(language, "character-window-online");
            let title = aurora_i18n::tr_in!(
                language,
                "character-window-title",
                account = "Synthetic 雪",
                grid = "Grid A",
                phase = phase,
                viewer = "Aurora Viewer 0.8 (Dev)",
            );
            for expected in ["Synthetic 雪", "Grid A", phase, "Aurora Viewer 0.8 (Dev)"] {
                assert!(title.contains(expected), "{language} caption: {title}");
            }
            let empty = aurora_i18n::tr_in!(
                language,
                "character-window-login-title",
                phase = phase,
                viewer = "Aurora Viewer 0.8 (Dev)"
            );
            assert!(empty.contains(phase) && empty.ends_with("Aurora Viewer 0.8 (Dev)"));
        }
    }
}
