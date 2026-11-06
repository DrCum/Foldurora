//! Logging to the console and to `aurora.log` in the logs directory (the
//! previous run is kept as `aurora.previous.log`), so a session can be
//! shared after a test even when the viewer was started without a console.
//! Capability URLs are secrets for the session: their id is masked.
//!
//! The lines reach the system as they are logged, and the disk once per
//! second (`start_flusher`): a PC that freezes and is reset keeps the log up
//! to the second before. Left to the system, ext4 brought the whole log
//! back empty after a reset in the first half minute of a session (#1809).

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Time between two flushes of the log files to the disk.
const FLUSH_PERIOD: Duration = Duration::from_secs(1);

/// Records written to the log files, for the flusher to know there is
/// something to flush.
static RECORDS: AtomicU64 = AtomicU64::new(0);

pub fn logs_dir() -> PathBuf {
    if let Some(test) = aurora_updater::test_environment() {
        return test.root.join("logs");
    }
    directories::ProjectDirs::from("org", "Aurora", "AuroraViewer")
        .map(|d| d.data_local_dir().join("logs"))
        .unwrap_or_else(|| PathBuf::from("logs"))
}

pub fn log_path() -> PathBuf {
    logs_dir().join(format!("{}.log", log_stem()))
}

/// The offline demo logs apart, so it never rotates a live session's log;
/// `--title` adds the window name (one log per test window) and
/// AURORA_LOG_NAME picks any other name.
fn log_stem() -> String {
    if let Some(name) = std::env::var_os("AURORA_LOG_NAME").filter(|n| !n.is_empty()) {
        return name.to_string_lossy().into_owned();
    }
    let base = if std::env::var_os("AURORA_DEMO").is_some() {
        "aurora-demo"
    } else {
        "aurora"
    };
    ordinary_stem(base, crate::cli::title_slug())
}

fn ordinary_stem(base: &str, title: Option<String>) -> String {
    match title {
        Some(slug) => format!("{base}-{slug}"),
        None => format!("{base}-{}", std::process::id()),
    }
}

fn host_stem(pid: u32, nonce: uuid::Uuid) -> String {
    format!("aurora-session-host-{pid}-{}", nonce.simple())
}

/// Show the logs directory in the system file manager.
pub fn open_logs_dir() {
    let dir = logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Err(e) = open_directory(&dir) {
        log::warn!("cannot open {}: {e}", dir.display());
    }
}

/// Show a directory in the system file manager (also the transcripts folder).
pub fn open_directory(path: &Path) -> std::io::Result<()> {
    crate::open_path::open(path)
}

/// Mask the id of capability URLs (".../cap/<uuid>").
fn redact(line: &str) -> std::borrow::Cow<'_, str> {
    if !line.contains("/cap/") {
        return line.into();
    }
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find("/cap/") {
        out.push_str(&rest[..i + 5]);
        rest = &rest[i + 5..];
        let end = rest.find(|c: char| !(c.is_ascii_hexdigit() || c == '-')).unwrap_or(rest.len());
        if end > 0 {
            out.push('…');
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.into()
}

thread_local! {
    /// What this thread logs while a capture runs (`capture`).
    static CAPTURE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` and return, besides its result, the lines it logged on this
/// thread without their timestamp and target (the diagnostic copied to the
/// clipboard, scene/describe.rs). env_logger writes in the calling thread.
pub fn capture<R>(f: impl FnOnce() -> R) -> (R, String) {
    CAPTURE.with(|c| *c.borrow_mut() = Some(String::new()));
    let result = f();
    let text = CAPTURE.with(|c| c.borrow_mut().take()).unwrap_or_default();
    (result, strip_headers(&text))
}

/// Log lines without env_logger's header
/// ("[2026-10-20T12:00:00.000Z INFO  aurora_viewer::scene] text").
fn strip_headers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let line = match line.strip_prefix('[').and_then(|l| l.split_once("] ")) {
            Some((_, rest)) => rest,
            None => line,
        };
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// A line the Dev Toolbox keeps in memory: J3 and L3 show the last warnings,
/// J5 the recent output of a diagnostic, without reading the log file.
#[derive(Debug, Clone)]
pub struct Recent {
    pub at: std::time::Instant,
    /// "HH:MM:SS" of the record's header (UTC, as in the log).
    pub clock: String,
    pub level: log::Level,
    pub text: String,
}

const WARNINGS_KEPT: usize = 200;
const TOOL_LINES_KEPT: usize = 600;

/// The diagnostics whose output J5 copies, by the start of their lines.
pub const TOOL_PREFIXES: &[(&str, &str)] = &[
    ("P3", "slow sync:"),
    ("P4", "still check:"),
    ("P5", "sync check:"),
    ("P6", "perf summary:"),
    ("P6", "net summary:"),
    ("O9", "sculpt watch:"),
    ("M1", "media objects:"),
];

static WARNINGS: std::sync::Mutex<std::collections::VecDeque<Recent>> = std::sync::Mutex::new(std::collections::VecDeque::new());
static TOOL_LINES: std::sync::Mutex<std::collections::VecDeque<(&'static str, Recent)>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());

/// The header of a record: (clock, level, the message after the header).
fn parse_record(text: &str) -> Option<(&str, log::Level, &str)> {
    let rest = text.strip_prefix('[')?;
    let (header, message) = rest.split_once("] ")?;
    let mut parts = header.split_whitespace();
    let stamp = parts.next()?;
    let level = parts.next()?.parse().ok()?;
    let clock = stamp.split_once('T').map_or(stamp, |(_, t)| t.get(..8).unwrap_or(t));
    Some((clock, level, message.trim_end()))
}

/// Keep what the Dev Toolbox shows from one record.
fn remember(text: &str) {
    let Some((clock, level, message)) = parse_record(text) else {
        return;
    };
    let recent = || Recent {
        at: std::time::Instant::now(),
        clock: clock.to_owned(),
        level,
        text: message.to_owned(),
    };
    if level <= log::Level::Warn
        && let Ok(mut ring) = WARNINGS.lock()
    {
        if ring.len() == WARNINGS_KEPT {
            ring.pop_front();
        }
        ring.push_back(recent());
    }
    if let Some((tool, _)) = TOOL_PREFIXES.iter().find(|(_, prefix)| message.starts_with(prefix))
        && let Ok(mut ring) = TOOL_LINES.lock()
    {
        if ring.len() == TOOL_LINES_KEPT {
            ring.pop_front();
        }
        ring.push_back((tool, recent()));
    }
}

/// The last `n` warnings and errors, the oldest first.
pub fn recent_warnings(n: usize) -> Vec<Recent> {
    WARNINGS
        .lock()
        .map(|ring| ring.iter().skip(ring.len().saturating_sub(n)).cloned().collect())
        .unwrap_or_default()
}

/// The lines the diagnostics of [`TOOL_PREFIXES`] wrote in the last `since`.
pub fn tool_lines(since: std::time::Duration) -> Vec<(&'static str, Recent)> {
    let now = std::time::Instant::now();
    TOOL_LINES
        .lock()
        .map(|ring| ring.iter().filter(|(_, r)| now.duration_since(r.at) <= since).cloned().collect())
        .unwrap_or_default()
}

/// Console + file writer.
struct Tee {
    file: Option<std::fs::File>,
    session_file: Option<std::fs::File>,
    host: Option<Arc<HostState>>,
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        let text = redact(&text);
        CAPTURE.with(|c| {
            if let Some(s) = c.borrow_mut().as_mut() {
                s.push_str(&text);
            }
        });
        remember(&text);
        let _ = std::io::stderr().write_all(text.as_bytes());
        if let Some(f) = &mut self.file {
            let written = f.write_all(text.as_bytes());
            if let (Some(host), Err(error)) = (&self.host, written) {
                host.failed(error.kind());
                return Err(error);
            }
            crate::disk_io::written(crate::disk_io::Origin::Logs, text.len());
        }
        if let Some(f) = &mut self.session_file {
            let _ = f.write_all(text.as_bytes());
            crate::disk_io::written(crate::disk_io::Origin::Logs, text.len());
        }
        RECORDS.fetch_add(1, Ordering::Relaxed);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(f) = &mut self.file {
            let flushed = f.flush();
            if let (Some(host), Err(error)) = (&self.host, flushed) {
                host.failed(error.kind());
                return Err(error);
            }
        }
        if let Some(f) = &mut self.session_file {
            let _ = f.flush();
        }
        if self.host.is_some() {
            // FreeConsole deliberately removes this sink. Only the checked
            // durable file determines whether host logging remains usable.
            let _ = std::io::stderr().flush();
            Ok(())
        } else {
            std::io::stderr().flush()
        }
    }
}

#[derive(Default)]
struct HostState {
    error: Mutex<Option<io::ErrorKind>>,
    stop: AtomicBool,
}

impl HostState {
    fn failed(&self, error: io::ErrorKind) {
        self.error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_or_insert(error);
    }

    fn result(&self) -> io::Result<()> {
        match *self.error.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
            Some(kind) => Err(io::Error::new(kind, "session host log could not persist every record")),
            None => Ok(()),
        }
    }
}

/// Retained by the host until its startup/shutdown result has been logged and
/// explicitly flushed. All methods that sync belong outside the UI loop.
pub(crate) struct HostLog {
    path: PathBuf,
    file: std::fs::File,
    state: Arc<HostState>,
}

impl HostLog {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn flush(&self) -> io::Result<()> {
        log::logger().flush();
        self.sync()
    }

    fn sync(&self) -> io::Result<()> {
        self.file.sync_data()?;
        self.state.result()
    }
}

impl Drop for HostLog {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::Release);
    }
}

trait DurableLog: Write {
    fn sync_record(&mut self) -> io::Result<()>;
}

impl DurableLog for std::fs::File {
    fn sync_record(&mut self) -> io::Result<()> {
        self.sync_data()
    }
}

fn initial_host_record(file: &mut impl DurableLog, record: &[u8]) -> io::Result<()> {
    file.write_all(record)?;
    file.flush()?;
    file.sync_record()
}

struct HostFiles {
    path: PathBuf,
    file: std::fs::File,
    logger: std::fs::File,
    flusher: std::fs::File,
}

fn prepare_host_log(directory: &Path, pid: u32, nonce: uuid::Uuid) -> io::Result<HostFiles> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join(format!("{}.log", host_stem(pid, nonce)));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
    let logger = file.try_clone()?;
    let flusher = file.try_clone()?;
    let record = format!(
        "Aurora session host {} starting ({} {}, {} allocator), log: {}\n",
        aurora_version::version(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        crate::allocator::name(),
        path.display()
    );
    // This cannot be filtered out with RUST_LOG: the startup record reaches
    // durable storage before logger installation or console detachment.
    initial_host_record(&mut file, record.as_bytes())?;
    #[cfg(unix)]
    std::fs::File::open(directory)?.sync_all()?;
    Ok(HostFiles {
        path,
        file,
        logger,
        flusher,
    })
}

/// Host-only checked startup. Failures leave console attachment to the caller;
/// ordinary viewer logging, file names, rotation and crash sessions are intact.
pub(crate) fn init_host() -> io::Result<HostLog> {
    let HostFiles {
        path,
        file,
        logger,
        flusher,
    } = prepare_host_log(&logs_dir(), std::process::id(), uuid::Uuid::new_v4())?;
    let state = Arc::new(HostState::default());
    let guard = HostLog {
        path,
        file,
        state: state.clone(),
    };
    spawn_flusher(vec![flusher], Some(state.clone()))?;
    logger_builder(Tee {
        file: Some(logger),
        session_file: None,
        host: Some(state),
    })
    .try_init()
    .map_err(|_| io::Error::new(io::ErrorKind::AlreadyExists, "a process logger is already installed"))?;
    install_panic_hook();
    Ok(guard)
}

/// Flush `files` (handles of the log files) to the disk once per
/// `FLUSH_PERIOD` when lines were logged since the last time, off the
/// threads that log: a flush waits for the disk.
fn start_flusher(files: Vec<std::fs::File>) {
    if let Err(e) = spawn_flusher(files, None) {
        eprintln!("log files not flushed to the disk: {e}");
    }
}

fn spawn_flusher(files: Vec<std::fs::File>, host: Option<Arc<HostState>>) -> io::Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let spawned = std::thread::Builder::new().name("aurora-log-flush".into()).spawn(move || {
        let mut flushed = 0;
        loop {
            std::thread::sleep(FLUSH_PERIOD);
            if host.as_ref().is_some_and(|host| host.stop.load(Ordering::Acquire)) {
                break;
            }
            let records = RECORDS.load(Ordering::Relaxed);
            if records != flushed {
                flushed = records;
                for file in &files {
                    if let Err(error) = file.sync_data()
                        && let Some(host) = &host
                    {
                        host.failed(error.kind());
                    }
                }
            }
        }
    });
    spawned.map(|_| ())
}

pub fn init() {
    let dir = logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = log_path();
    let _ = std::fs::rename(&path, dir.join(format!("{}.previous.log", log_stem())));
    let file = std::fs::File::create(&path).ok();
    let session_file = crate::crash::session_log();
    start_flusher(
        [&file, &session_file]
            .into_iter()
            .flatten()
            .filter_map(|f| f.try_clone().ok())
            .collect(),
    );
    logger_builder(Tee {
        file,
        session_file,
        host: None,
    })
    .init();
    install_panic_hook();
    log::info!(
        "Aurora Viewer {} starting ({} {}, {} allocator), log: {}",
        aurora_version::version(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        crate::allocator::name(),
        path.display()
    );
}

fn logger_builder(tee: Tee) -> env_logger::Builder {
    // `pulseaudio` off: the PulseAudio client logs `ERROR Reactor error:
    // Client disconnected` each time cpal closes a stream (every output or
    // capture device switch). Real stream failures reach aurora-audio through
    // cpal's error callback, which reopens the device.
    let mut builder = env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn,reqwest=warn,symphonia=warn,pulseaudio=off,zbus=warn"),
    );
    builder.format_timestamp_millis().target(env_logger::Target::Pipe(Box::new(tee)));
    builder
}

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        // wgpu's panic for a device lost during a wait, caught by the
        // renderer and taken as the loss (aurora-render device_lost.rs);
        // any other panic there is reported
        if aurora_render::in_guarded_poll() && info.payload_as_str().is_some_and(aurora_render::is_loss_panic) {
            log::warn!("wgpu: {info}");
            return;
        }
        let bt = std::backtrace::Backtrace::force_capture();
        crate::crash::panic(info, &bt);
        log::error!("panic: {info}\n{bt}");
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_host_logs() -> PathBuf {
        std::env::temp_dir().join(format!("aurora-host-log-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn host_log_names_do_not_change_ordinary_demo_title_or_pid_stems() {
        assert_eq!(ordinary_stem("aurora", None), format!("aurora-{}", std::process::id()));
        assert_eq!(ordinary_stem("aurora-demo", None), format!("aurora-demo-{}", std::process::id()));
        assert_eq!(ordinary_stem("aurora", Some("test-view".into())), "aurora-test-view");
        assert_eq!(ordinary_stem("aurora-demo", Some("test-view".into())), "aurora-demo-test-view");
        let first = host_stem(42, uuid::Uuid::from_u128(1));
        assert_eq!(first, "aurora-session-host-42-00000000000000000000000000000001");
        assert_ne!(
            first,
            host_stem(42, uuid::Uuid::from_u128(2)),
            "PID reuse cannot rotate or replace an earlier host log"
        );
        assert_ne!(first, host_stem(43, uuid::Uuid::from_u128(1)));
    }

    #[test]
    fn host_log_preparation_reports_directory_and_file_failures_without_overwriting() {
        let directory = temporary_host_logs();
        std::fs::create_dir_all(&directory).expect("private test directory");
        let blocked = directory.join("file-instead-of-directory");
        std::fs::write(&blocked, b"keep").expect("path blocker");
        assert!(prepare_host_log(&blocked.join("logs"), 42, uuid::Uuid::from_u128(1)).is_err());
        assert_eq!(std::fs::read(&blocked).expect("unchanged blocker"), b"keep");
        let path = directory.join(format!("{}.log", host_stem(42, uuid::Uuid::from_u128(2))));
        std::fs::write(&path, b"previous host log").expect("existing file");
        assert!(prepare_host_log(&directory, 42, uuid::Uuid::from_u128(2)).is_err());
        assert_eq!(std::fs::read(&path).expect("unchanged earlier log"), b"previous host log");
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn host_startup_requires_successful_write_flush_and_sync_before_installation() {
        struct Injected {
            failure: Option<&'static str>,
            operations: Vec<&'static str>,
        }
        impl Injected {
            fn operation(&mut self, operation: &'static str) -> io::Result<()> {
                self.operations.push(operation);
                if self.failure == Some(operation) {
                    Err(io::ErrorKind::PermissionDenied.into())
                } else {
                    Ok(())
                }
            }
        }
        impl Write for Injected {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.operation("write")?;
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                self.operation("flush")
            }
        }
        impl DurableLog for Injected {
            fn sync_record(&mut self) -> io::Result<()> {
                self.operation("sync")
            }
        }
        for (failure, expected) in [
            ("write", vec!["write"]),
            ("flush", vec!["write", "flush"]),
            ("sync", vec!["write", "flush", "sync"]),
        ] {
            let mut writer = Injected {
                failure: Some(failure),
                operations: Vec::new(),
            };
            let mut may_install_or_detach = false;
            let result = initial_host_record(&mut writer, b"synthetic startup\n").map(|()| may_install_or_detach = true);
            assert_eq!(result.expect_err("startup must fail").kind(), io::ErrorKind::PermissionDenied);
            assert!(!may_install_or_detach);
            assert_eq!(writer.operations, expected);
        }
        let mut writer = Injected {
            failure: None,
            operations: Vec::new(),
        };
        initial_host_record(&mut writer, b"synthetic startup\n").expect("checked durable record");
        assert_eq!(writer.operations, ["write", "flush", "sync"]);
    }

    #[test]
    fn host_log_has_a_durable_start_record_and_checked_terminal_flush() {
        let directory = temporary_host_logs();
        std::fs::create_dir_all(&directory).expect("private test directory");
        let ordinary = directory.join("aurora-42.log");
        std::fs::write(&ordinary, b"ordinary viewer log").expect("ordinary fixture");
        let HostFiles {
            path,
            file,
            mut logger,
            flusher,
        } = prepare_host_log(&directory, 42, uuid::Uuid::from_u128(1)).expect("durable preparation");
        let initial = std::fs::read_to_string(&path).expect("durable startup record");
        assert!(initial.starts_with("Aurora session host "));
        assert!(initial.contains(" starting ("));
        assert!(initial.ends_with('\n'));
        writeln!(logger, "synthetic controller startup failure").expect("checked logger clone");
        logger.flush().expect("checked writer flush");
        let state = Arc::new(HostState::default());
        let guard = HostLog {
            path: path.clone(),
            file,
            state: state.clone(),
        };
        assert_eq!(guard.path(), path);
        guard.sync().expect("checked terminal sync");
        assert_eq!(
            std::fs::read_to_string(&path).expect("whole log"),
            format!("{initial}synthetic controller startup failure\n")
        );
        assert_eq!(std::fs::read(&ordinary).expect("ordinary log unchanged"), b"ordinary viewer log");
        state.failed(io::ErrorKind::WriteZero);
        state.failed(io::ErrorKind::PermissionDenied);
        assert_eq!(
            guard.sync().expect_err("earlier lost records stay visible").kind(),
            io::ErrorKind::WriteZero
        );
        drop(guard);
        assert!(state.stop.load(Ordering::Acquire));
        drop(logger);
        drop(flusher);
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn record_headers_give_clock_and_level() {
        let (clock, level, message) =
            parse_record("[2026-10-20T02:34:46.538Z WARN  aurora_viewer::scene::textures] texture memory: pressure\n").unwrap();
        assert_eq!(clock, "02:34:46");
        assert_eq!(level, log::Level::Warn);
        assert_eq!(message, "texture memory: pressure");
        assert!(parse_record("  continuation line").is_none());
    }

    #[test]
    fn captured_lines_lose_their_header() {
        let text = "[2026-10-20T12:00:00.000Z INFO  aurora_viewer::scene::describe] attachment '?' point 40\n  prim [0 tex] x\n";
        assert_eq!(strip_headers(text), "attachment '?' point 40\n  prim [0 tex] x\n");
    }

    #[test]
    fn masks_capability_ids() {
        assert_eq!(
            redact(
                "error sending request for url (https://sim1.agni.lindenlab.com:12043/cap/0b6c6a8e-1f0d-4d3e-9a1b-2c3d4e5f6a7b): timeout"
            ),
            "error sending request for url (https://sim1.agni.lindenlab.com:12043/cap/…): timeout"
        );
        assert_eq!(redact("plain line"), "plain line");
    }
}
