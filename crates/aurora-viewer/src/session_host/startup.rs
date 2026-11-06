//! Controller-only diagnostics and console lifetime. Ordinary viewer startup
//! retains its existing logging, crash recovery and inherited console behavior.

use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

pub(crate) fn run() -> anyhow::Result<()> {
    run_checked(crate::logging::init_host, detach_console, super::run, report_failure)
}

trait Diagnostics {
    fn path(&self) -> &Path;
    fn flush(&self) -> io::Result<()>;
}

impl Diagnostics for crate::logging::HostLog {
    fn path(&self) -> &Path {
        crate::logging::HostLog::path(self)
    }

    fn flush(&self) -> io::Result<()> {
        crate::logging::HostLog::flush(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailureStage {
    Log,
    Console,
    Run,
    Flush,
}

/// Keep initialization and failure ordering testable without installing a
/// global logger, creating native windows, or detaching a test runner's console.
fn run_checked<L: Diagnostics>(
    initialize: impl FnOnce() -> io::Result<L>,
    detach: impl FnOnce() -> io::Result<()>,
    run: impl FnOnce() -> anyhow::Result<()>,
    mut report: impl FnMut(FailureStage, &str, Option<&Path>),
) -> anyhow::Result<()> {
    let diagnostics = match initialize() {
        Ok(diagnostics) => diagnostics,
        Err(error) => {
            report(FailureStage::Log, &error.to_string(), None);
            return Err(error.into());
        }
    };
    if let Err(error) = detach() {
        log::error!("session host console detachment failed: {error}");
        let detail = match diagnostics.flush() {
            Ok(()) => error.to_string(),
            Err(flush) => format!("{error}; diagnostic log flush failed: {flush}"),
        };
        report(FailureStage::Console, &detail, Some(diagnostics.path()));
        return Err(error.into());
    }
    let result = run();
    match &result {
        Ok(()) => log::info!("session host: controller stopped normally"),
        Err(error) => log::error!("session host startup / event loop error: {error:#}"),
    }
    match (result, diagnostics.flush()) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => {
            report(FailureStage::Run, &format!("{error:#}"), Some(diagnostics.path()));
            Err(error)
        }
        (Ok(()), Err(error)) => {
            report(FailureStage::Flush, &error.to_string(), Some(diagnostics.path()));
            Err(error.into())
        }
        (Err(error), Err(flush)) => {
            let detail = format!("{error:#}; diagnostic log flush failed: {flush}");
            report(FailureStage::Run, &detail, Some(diagnostics.path()));
            Err(error.context(format!("diagnostic log flush failed: {flush}")))
        }
    }
}

fn detach_console() -> io::Result<()> {
    #[cfg(windows)]
    {
        // SAFETY: FreeConsole detaches only this calling process. It neither
        // hides nor closes another process's window; shared consoles remain
        // attached to their other processes. A process without a console also
        // succeeds. https://learn.microsoft.com/windows/console/freeconsole
        if unsafe { windows_sys::Win32::System::Console::FreeConsole() } == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// A detached console-subsystem parent would otherwise give its debug worker a
/// new console. Apply only to explicitly managed workers, never ordinary viewer
/// launches. CREATE_NO_WINDOW is ignored for a GUI-subsystem release binary.
pub(super) fn prepare_worker(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(worker_creation_flags());
    }
    #[cfg(not(windows))]
    let _ = command;
}

#[cfg(any(windows, test))]
fn worker_creation_flags() -> u32 {
    // https://learn.microsoft.com/windows/win32/procthread/process-creation-flags
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    CREATE_NO_WINDOW
}

fn report_failure(stage: FailureStage, detail: &str, path: Option<&Path>) {
    let path = path.map_or_else(String::new, |path| path.display().to_string());
    let body = match stage {
        FailureStage::Log => aurora_i18n::tr!("host-start-log-failed", error = detail),
        FailureStage::Console => aurora_i18n::tr!("host-start-console-failed", error = detail, path = path),
        FailureStage::Run => aurora_i18n::tr!("host-start-run-failed", error = detail, path = path),
        FailureStage::Flush => aurora_i18n::tr!("host-start-flush-failed", error = detail, path = path),
    };
    // A detached console can leave an invalid stderr handle. Reporting must
    // still reach this process's native dialog instead of panicking here.
    let _ = writeln!(io::stderr(), "{body}");
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW};
        let wide = |value: &str| value.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let title = wide(aurora_i18n::tr!("host-start-error-title"));
        let body = wide(&body);
        // SAFETY: both strings are terminated and live throughout this call.
        // This process owns the error dialog; no foreign console is touched.
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                body.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    type Events = Rc<RefCell<Vec<&'static str>>>;

    struct Log {
        events: Events,
        fail_flush: bool,
    }

    impl Diagnostics for Log {
        fn path(&self) -> &Path {
            Path::new("synthetic-host.log")
        }

        fn flush(&self) -> io::Result<()> {
            self.events.borrow_mut().push("flush");
            if self.fail_flush {
                Err(io::ErrorKind::WriteZero.into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn host_log_failure_is_visible_before_any_console_detach_or_shell_start() {
        let events = Events::default();
        let result = run_checked::<Log>(
            || {
                events.borrow_mut().push("log-failed");
                Err(io::ErrorKind::PermissionDenied.into())
            },
            || {
                events.borrow_mut().push("detach");
                Ok(())
            },
            || {
                events.borrow_mut().push("run");
                Ok(())
            },
            |stage, _, path| {
                assert_eq!(stage, FailureStage::Log);
                assert!(path.is_none());
                events.borrow_mut().push("visible");
            },
        );
        assert!(result.is_err());
        assert_eq!(*events.borrow(), ["log-failed", "visible"]);
    }

    #[test]
    fn host_console_detaches_only_after_durable_log_and_flushes_on_normal_close() {
        let events = Events::default();
        run_checked(
            || {
                events.borrow_mut().push("durable-log");
                Ok(Log {
                    events: events.clone(),
                    fail_flush: false,
                })
            },
            || {
                events.borrow_mut().push("detach");
                Ok(())
            },
            || {
                events.borrow_mut().push("run");
                Ok(())
            },
            |_, _, _| events.borrow_mut().push("visible"),
        )
        .expect("checked startup");
        assert_eq!(*events.borrow(), ["durable-log", "detach", "run", "flush"]);
    }

    #[test]
    fn host_console_failure_stays_visible_and_does_not_start_the_shell() {
        let events = Events::default();
        assert!(
            run_checked(
                || Ok(Log {
                    events: events.clone(),
                    fail_flush: false
                }),
                || {
                    events.borrow_mut().push("detach-failed");
                    Err(io::ErrorKind::PermissionDenied.into())
                },
                || {
                    events.borrow_mut().push("run");
                    Ok(())
                },
                |stage, _, path| {
                    assert_eq!(stage, FailureStage::Console);
                    assert_eq!(path, Some(Path::new("synthetic-host.log")));
                    events.borrow_mut().push("visible");
                },
            )
            .is_err()
        );
        assert_eq!(*events.borrow(), ["detach-failed", "flush", "visible"]);
    }

    #[test]
    fn detached_host_run_and_log_failures_are_reported_after_checked_flush() {
        for run_fails in [false, true] {
            let events = Events::default();
            assert!(
                run_checked(
                    || Ok(Log {
                        events: events.clone(),
                        fail_flush: true
                    }),
                    || Ok(()),
                    || if run_fails {
                        anyhow::bail!("synthetic shell failure")
                    } else {
                        Ok(())
                    },
                    |stage, detail, _| {
                        assert_eq!(stage, if run_fails { FailureStage::Run } else { FailureStage::Flush });
                        assert!(!detail.is_empty());
                        events.borrow_mut().push("visible");
                    },
                )
                .is_err()
            );
            assert_eq!(*events.borrow(), ["flush", "visible"]);
        }
    }

    #[test]
    fn managed_worker_creation_flags_suppress_console_without_allocating_or_detaching_another() {
        assert_eq!(worker_creation_flags(), 0x0800_0000);
        assert_eq!(
            worker_creation_flags() & (0x10 | 0x08),
            0,
            "no CREATE_NEW_CONSOLE or DETACHED_PROCESS"
        );
        #[cfg(windows)]
        assert_eq!(worker_creation_flags(), windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
}
