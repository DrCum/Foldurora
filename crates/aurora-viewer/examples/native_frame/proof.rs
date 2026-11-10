use aurora_session::PhysicalRect;
use std::{
    cell::{Cell, RefCell},
    io::{self, BufRead, Write},
    process::{Child, ChildStdin, Command, Stdio},
    rc::Rc,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, SystemInformation::GetTickCount64},
    UI::{Input::KeyboardAndMouse::IsWindowEnabled, WindowsAndMessaging::*},
};

#[path = "../../src/session_host/frame_guard.rs"]
mod guard;
use guard::{Guard, WindowIdentity, WorkerOrder};

const NOTICE: u32 = WM_APP + 7;
const BLUE: u32 = 0x00da_7631;
const GREEN: u32 = 0x0065_c246;
const HOST_COLOR: u32 = 0x003d_312d;
const TIMER: usize = 7;
const FOCUS_WAIT_MS: u64 = 250;
const FOCUS_PAUSE_CUTOFF_MS: u64 = 900;
const TRACE_LIMIT: usize = 256;
const CLASS: &[u16] = &[
    70, 111, 108, 100, 117, 114, 111, 114, 97, 70, 114, 97, 109, 101, 80, 114, 111, 111, 102, 0,
];

#[link(name = "user32")]
unsafe extern "system" {
    fn SetProcessDpiAwarenessContext(context: isize) -> i32;
    fn GetThreadDpiAwarenessContext() -> isize;
    fn AreDpiAwarenessContextsEqual(first: isize, second: isize) -> i32;
}
fn dpi() -> io::Result<()> {
    // SAFETY: set only this new synthetic process's DPI mode before HWNDs;
    // an existing manifest choice is acceptable only if it is the same v2 mode.
    unsafe {
        SetProcessDpiAwarenessContext(-4);
        require(
            AreDpiAwarenessContextsEqual(GetThreadDpiAwarenessContext(), -4) != 0,
            "per-monitor-v2 DPI awareness before native window creation",
        )
    }
}

fn fail(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
fn own(hwnd: HWND) -> WindowIdentity {
    WindowIdentity {
        hwnd: hwnd as u64,
        pid: std::process::id(),
    }
}
fn require(condition: bool, label: &str) -> io::Result<()> {
    if !condition {
        return Err(fail(label));
    }
    println!("PASS {label}");
    Ok(())
}

// Only native identity/order/focus metadata. No titles, text, profile data or
// foreign-window mutation. Capture in bounded memory so diagnostic printing
// cannot block the UI during the timed pause/cover checks.
struct FocusObservation {
    stage: &'static str,
    tick: u64,
    returned: Option<i32>,
    foreground: u64,
    pid: u32,
    thread: u32,
    caller_thread: u32,
    host_thread: u32,
    gui_available: bool,
    active: u64,
    focused: u64,
    host_previous: u64,
    host_next: u64,
    host_last_popup: u64,
    host_enabled: bool,
}

struct FocusTrace {
    host: HWND,
    enabled: Cell<bool>,
    dropped: Cell<usize>,
    observations: RefCell<Vec<FocusObservation>>,
}

impl FocusTrace {
    fn new(host: HWND) -> Self {
        Self {
            host,
            enabled: Cell::new(false),
            dropped: Cell::new(0),
            observations: RefCell::new(Vec::with_capacity(TRACE_LIMIT)),
        }
    }

    fn record(&self, stage: &'static str, returned: Option<i32>) {
        if !self.enabled.get() {
            return;
        }
        let Ok(mut observations) = self.observations.try_borrow_mut() else {
            self.dropped.set(self.dropped.get().saturating_add(1));
            return;
        };
        if observations.len() == TRACE_LIMIT {
            self.dropped.set(self.dropped.get().saturating_add(1));
            return;
        }
        // SAFETY: read-only native metadata queries. GetGUIThreadInfo does not
        // send a synchronous request to the deliberately paused worker.
        unsafe {
            let foreground = GetForegroundWindow();
            let mut pid = 0;
            let thread = GetWindowThreadProcessId(foreground, &mut pid);
            let mut gui = GUITHREADINFO {
                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            let gui_available = thread != 0 && GetGUIThreadInfo(thread, &mut gui) != 0;
            observations.push(FocusObservation {
                stage,
                tick: GetTickCount64(),
                returned,
                foreground: foreground as u64,
                pid,
                thread,
                gui_available,
                caller_thread: windows_sys::Win32::System::Threading::GetCurrentThreadId(),
                host_thread: GetWindowThreadProcessId(self.host, std::ptr::null_mut()),
                active: if gui_available { gui.hwndActive as u64 } else { 0 },
                focused: if gui_available { gui.hwndFocus as u64 } else { 0 },
                host_previous: GetWindow(self.host, GW_HWNDPREV) as u64,
                host_next: GetWindow(self.host, GW_HWNDNEXT) as u64,
                host_last_popup: GetLastActivePopup(self.host) as u64,
                host_enabled: IsWindowEnabled(self.host) != 0,
            });
        }
    }

    fn dump(&self) {
        self.enabled.set(false);
        for row in self.observations.borrow().iter() {
            println!(
                "OBSERVED focus stage={} tick={} sfw_return={:?} foreground={} pid={} thread={} caller_thread={} host_thread={} gui_available={} active={} focused={} host_previous={} host_next={} host_last_popup={} host_enabled={}",
                row.stage,
                row.tick,
                row.returned,
                row.foreground,
                row.pid,
                row.thread,
                row.caller_thread,
                row.host_thread,
                row.gui_available,
                row.active,
                row.focused,
                row.host_previous,
                row.host_next,
                row.host_last_popup,
                row.host_enabled
            );
        }
        println!("OBSERVED focus_trace_dropped={}", self.dropped.get());
    }
}

// No injected input. Native key/button observations only occur if a person
// actually interacts with this owned proof window.
unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_PAINT {
        let mut paint = PAINTSTRUCT::default();
        unsafe {
            let dc = BeginPaint(hwnd, &mut paint);
            let color = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as u32;
            let brush = CreateSolidBrush(color);
            if !dc.is_null() && !brush.is_null() {
                FillRect(dc, &paint.rcPaint, brush);
            }
            if !brush.is_null() {
                DeleteObject(brush);
            }
            EndPaint(hwnd, &paint);
        }
        return 0;
    }
    if message == WM_CLOSE {
        unsafe {
            DestroyWindow(hwnd);
        }
        return 0;
    }
    if message == WM_DESTROY {
        // Popups are explicitly destroyed after the main loop, not closed by
        // synthetic clicks. Only the main top-level has a timer.
        unsafe {
            if KillTimer(hwnd, TIMER) != 0 {
                PostQuitMessage(0);
            }
        }
        return 0;
    }
    if matches!(message, WM_LBUTTONDOWN | WM_KEYDOWN | WM_MOUSEWHEEL) {
        println!("OBSERVED native-input hwnd={} message={message} wparam={wparam}", hwnd as u64);
        let _ = io::stdout().flush();
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn create(title: &str, style: u32, owner: HWND, rect: PhysicalRect, color: u32) -> io::Result<HWND> {
    rect.validate().map_err(io::Error::other)?;
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: module,
            lpszClassName: CLASS.as_ptr(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: GetStockObject(BLACK_BRUSH) as _,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 && windows_sys::Win32::Foundation::GetLastError() != 1410 {
            return Err(io::Error::last_os_error());
        }
        let hwnd = CreateWindowExW(
            if style & WS_POPUP != 0 { WS_EX_TOOLWINDOW } else { 0 },
            CLASS.as_ptr(),
            wide(title).as_ptr(),
            style,
            rect.x,
            rect.y,
            rect.width as i32,
            rect.height as i32,
            owner,
            std::ptr::null_mut(),
            module,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err(io::Error::last_os_error());
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, color as isize);
        Ok(hwnd)
    }
}

fn paint(hwnd: HWND) -> io::Result<()> {
    unsafe {
        if InvalidateRect(hwnd, std::ptr::null(), 1) == 0 || UpdateWindow(hwnd) == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn rect(text: &str) -> io::Result<PhysicalRect> {
    let values = text
        .split_whitespace()
        .map(str::parse::<i64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(io::Error::other)?;
    if values.len() != 4 {
        return Err(fail("expected four physical coordinates"));
    }
    let result = PhysicalRect {
        x: values[0].try_into().map_err(io::Error::other)?,
        y: values[1].try_into().map_err(io::Error::other)?,
        width: values[2].try_into().map_err(io::Error::other)?,
        height: values[3].try_into().map_err(io::Error::other)?,
    };
    result.validate().map_err(io::Error::other)?;
    Ok(result)
}

fn worker(host: WindowIdentity, marker: usize) -> io::Result<()> {
    let window = create(
        "Synthetic worker [AUTOMATION - DO NOT TOUCH]",
        WS_POPUP,
        std::ptr::null_mut(),
        PhysicalRect {
            x: 50,
            y: 50,
            width: 400,
            height: 300,
        },
        BLUE,
    )?;
    let mut order = Some(WorkerOrder::new(window, host, marker)?);
    require(
        WorkerOrder::new(window, host, marker).is_err(),
        "duplicate worker registration refused without replacing constraint",
    )?;
    let (send, receive) = mpsc::channel();
    let notification = window as usize;
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else {
                break;
            };
            if send.send(line).is_err() {
                return;
            }
            // Our own process's message pump only, never a foreign HWND.
            unsafe {
                PostMessageW(notification as HWND, NOTICE, 0, 0);
            }
        }
        let _ = send.send("exit".into());
        unsafe {
            PostMessageW(notification as HWND, NOTICE, 0, 0);
        }
    });
    println!("ready {} {}", window as u64, std::process::id());
    io::stdout().flush()?;
    let mut message = MSG::default();
    loop {
        let status = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
        if status == -1 {
            return Err(io::Error::last_os_error());
        }
        if status == 0 {
            break;
        }
        if message.message == NOTICE {
            while let Ok(command) = receive.try_recv() {
                if let Some(args) = command.strip_prefix("fit ") {
                    let area = rect(args)?;
                    guard::worker_place(window, host, marker, area, true)?;
                    paint(window)?;
                    let current = guard::bounds(own(window))?;
                    println!("placed {} {} {} {}", current.x, current.y, current.width, current.height);
                } else if command == "focus" {
                    unsafe {
                        SetForegroundWindow(window);
                    }
                    require(
                        unsafe { GetForegroundWindow() } == window,
                        "OS accepted worker foreground request (refusal is inconclusive)",
                    )?;
                    println!("focused");
                } else if command == "pause" {
                    // This explicit synthetic focus transaction already
                    // observed this worker as foreground. Give the exact
                    // controller PID ordinary Windows eligibility before the
                    // pause. Never use ASFW_ANY, input injection or a policy
                    // override. Eligibility is not proof activation succeeded.
                    guard::verified(host)?;
                    require(guard::marker(host)? == marker, "exact host registration before foreground handoff")?;
                    require(
                        unsafe { GetForegroundWindow() } == window,
                        "actual foreground worker makes controller handoff",
                    )?;
                    // SAFETY: the configured exact controller was verified
                    // above; this actual foreground process may grant it
                    // ordinary activation eligibility for this explicit step.
                    let allowed = unsafe { AllowSetForegroundWindow(host.pid) };
                    let allowed_error = (allowed == 0).then(io::Error::last_os_error);
                    println!(
                        "OBSERVED controller_foreground_handoff pid={} returned={allowed} error={allowed_error:?}",
                        host.pid
                    );
                    require(allowed != 0, "foreground worker permitted only its exact controller PID")?;
                    println!("paused {}", unsafe { GetTickCount64() });
                    io::stdout().flush()?;
                    // Bounded deliberate own-UI stall, never another process's
                    // SuspendThread or a system/driver failure simulation.
                    std::thread::sleep(Duration::from_millis(1800));
                    println!("resumed");
                } else if command == "hide" {
                    unsafe {
                        ShowWindow(window, SW_HIDE);
                    }
                    println!("hidden");
                } else if command == "invalid" {
                    let area = guard::bounds(own(window))?;
                    require(
                        guard::worker_place(
                            window,
                            WindowIdentity {
                                pid: host.pid.wrapping_add(1),
                                ..host
                            },
                            marker,
                            area,
                            false,
                        )
                        .is_err(),
                        "stale PID refused",
                    )?;
                    require(
                        guard::worker_place(window, host, marker.wrapping_add(1), area, false).is_err(),
                        "stale registration refused",
                    )?;
                    println!("invalid-refused");
                } else if command == "detach" {
                    // Only this synthetic control channel requests detach. The
                    // real broker/grant/GPU protocol is deliberately not used.
                    order.as_ref().ok_or_else(|| fail("constraint already detached"))?.detach()?;
                    order = None;
                    unsafe {
                        ShowWindow(window, SW_HIDE);
                    }
                    println!("detached-hidden");
                } else if command == "exit" {
                    unsafe {
                        DestroyWindow(window);
                    }
                    return Ok(());
                } else {
                    return Err(fail("unknown synthetic command"));
                }
                io::stdout().flush()?;
            }
        } else {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    drop(order);
    Ok(())
}

fn other_app(area: PhysicalRect, next_pid: u32) -> io::Result<()> {
    let window = create(
        "Synthetic foreground app [AUTOMATION - DO NOT TOUCH]",
        WS_OVERLAPPEDWINDOW,
        std::ptr::null_mut(),
        area,
        GREEN,
    )?;
    let (send, receive) = mpsc::channel();
    let notify = window as usize;
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else {
                break;
            };
            if send.send(line).is_err() {
                return;
            }
            unsafe {
                PostMessageW(notify as HWND, NOTICE, 0, 0);
            }
        }
        let _ = send.send("exit".into());
        unsafe {
            PostMessageW(notify as HWND, NOTICE, 0, 0);
        }
    });
    unsafe {
        ShowWindow(window, SW_SHOWNORMAL);
        SetForegroundWindow(window);
    }
    paint(window)?;
    println!("outside {} {}", window as u64, std::process::id());
    io::stdout().flush()?;
    let mut message = MSG::default();
    loop {
        let status = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
        if status == -1 {
            return Err(io::Error::last_os_error());
        }
        if status == 0 {
            return Err(fail("outside application closed unexpectedly"));
        }
        if message.message == NOTICE && receive.try_iter().any(|command| command == "exit") {
            // Ordinary explicit foreground handoff to the proof's known worker;
            // no simulated keys, thread attachment, ACL or global policy change.
            unsafe {
                AllowSetForegroundWindow(next_pid);
                DestroyWindow(window);
            }
            println!("outside-hidden");
            io::stdout().flush()?;
            return Ok(());
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn spawn(args: &[String], rows: Arc<Mutex<Vec<String>>>, notify: HWND) -> io::Result<(ChildOwner, std::thread::JoinHandle<()>)> {
    let mut child = Command::new(std::env::current_exe()?)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let output = child.stdout.take().ok_or_else(|| fail("missing synthetic stdout"))?;
    let child = ChildOwner {
        input: child.stdin.take(),
        child,
        exit_requested: false,
    };
    let notify = notify as usize;
    let reader = std::thread::spawn(move || {
        for line in io::BufReader::new(output).lines() {
            let Ok(line) = line else {
                break;
            };
            println!("synthetic: {line}");
            if let Ok(mut rows) = rows.lock() {
                rows.push(line);
            }
            unsafe {
                PostMessageW(notify as HWND, NOTICE, 0, 0);
            }
        }
    });
    Ok((child, reader))
}

struct ChildOwner {
    child: Child,
    input: Option<ChildStdin>,
    exit_requested: bool,
}
impl ChildOwner {
    fn send(&mut self, command: &str) -> io::Result<()> {
        let input = self.input.as_mut().ok_or_else(|| fail("worker input closed"))?;
        writeln!(input, "{command}")?;
        input.flush()?;
        if command == "exit" {
            self.exit_requested = true;
        }
        Ok(())
    }
    fn finish(&mut self) -> io::Result<()> {
        if !self.exit_requested {
            self.send("exit")?;
        }
        self.input = None;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait()? {
                return require(
                    status.success(),
                    &format!("synthetic process {} exited normally: {status}", self.child.id()),
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(fail("synthetic child cleanup timed out; prerequisite did not pass"))
    }
}
impl Drop for ChildOwner {
    fn drop(&mut self) {
        if !self.exit_requested {
            let _ = self.send("exit");
        }
        self.input = None;
        // Only the child spawned for this proof, never a viewer/foreign PID.
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        eprintln!(
            "proof child did not exit; terminating only owned synthetic child {}",
            self.child.id()
        );
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn visible_pixel(point: windows_sys::Win32::Foundation::POINT, expected_window: HWND, expected_color: u32, label: &str) -> io::Result<()> {
    unsafe {
        require(WindowFromPoint(point) == expected_window, &format!("{label}: native hit target"))?;
        let dc = GetDC(std::ptr::null_mut());
        if dc.is_null() {
            return Err(io::Error::last_os_error());
        }
        let color = GetPixel(dc, point.x, point.y);
        ReleaseDC(std::ptr::null_mut(), dc);
        require(
            color == expected_color,
            &format!("{label}: desktop pixel color={color:08x} expected={expected_color:08x}"),
        )?;
    }
    Ok(())
}

fn target(host: HWND) -> io::Result<PhysicalRect> {
    let outer = guard::client(own(host))?;
    let result = PhysicalRect {
        x: outer.x + 24,
        y: outer.y + 72,
        width: outer.width.checked_sub(48).ok_or_else(|| fail("host too narrow"))?,
        height: outer.height.checked_sub(96).ok_or_else(|| fail("host too short"))?,
    };
    result.validate().map_err(io::Error::other)?;
    Ok(result)
}

fn fit(child: &mut ChildOwner, guard: &Guard, area: PhysicalRect) -> io::Result<()> {
    guard.target(area)?;
    child.send(&format!("fit {} {} {} {}", area.x, area.y, area.width, area.height))
}

fn controller() -> io::Result<()> {
    let host = create(
        "Foldurora native-frame proof [AUTOMATION - DO NOT TOUCH]",
        WS_OVERLAPPEDWINDOW,
        std::ptr::null_mut(),
        PhysicalRect {
            x: 120,
            y: 100,
            width: 900,
            height: 650,
        },
        HOST_COLOR,
    )?;
    let popup = create(
        "Synthetic popup [AUTOMATION - DO NOT TOUCH]",
        WS_POPUP | WS_BORDER,
        host,
        PhysicalRect {
            x: 220,
            y: 245,
            width: 280,
            height: 120,
        },
        GREEN,
    )?;
    let popup_id = own(popup);
    let authorized_until = Rc::new(Cell::new(None::<Instant>));
    let authority = authorized_until.clone();
    let trace = Rc::new(FocusTrace::new(host));
    let guard_trace = trace.clone();
    let guard = Guard::new(
        host as u64,
        Rc::new(move || authority.get().is_some_and(|until| Instant::now() < until)),
        Rc::new(move |anchor| {
            guard::verified(popup_id)?;
            guard::show_above(popup, anchor, None)
        }),
        Rc::new(move |stage| guard_trace.record(stage, None)),
    )?;
    let marker = guard::marker(own(host))?;
    unsafe {
        ShowWindow(host, SW_SHOWNORMAL);
        if SetTimer(host, TIMER, 20, None) == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    paint(host)?;
    let rows = Arc::new(Mutex::new(Vec::<String>::new()));
    let (mut child, reader) = spawn(
        &[
            "--worker".into(),
            (host as u64).to_string(),
            std::process::id().to_string(),
            marker.to_string(),
        ],
        rows.clone(),
        host,
    )?;
    let mut other = None::<ChildOwner>;
    let mut other_reader = None;
    println!(
        "OBSERVED owned_controller host={} popup={} pid={}",
        host as u64,
        popup as u64,
        std::process::id()
    );
    let result = (|| {
        let start = Instant::now();
        let mut worker_id = None;
        let mut expected = target(host)?;
        let mut phase = 0;
        let mut due = Instant::now();
        let mut mask_started = None;
        let mut mask_checked = false;
        let mut minimized_checked = false;
        let mut pause_tick = 0;
        let mut focus_deadline = None;
        let mut focus_return = 0;
        let mut old_area = expected;
        let mut outside_window = None;
        let mut message = MSG::default();
        loop {
            let status = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
            if status == -1 {
                return Err(io::Error::last_os_error());
            }
            if status == 0 {
                return Err(fail("proof closed before its gates completed"));
            }
            if start.elapsed() > Duration::from_secs(20) {
                return Err(fail("proof deadline expired"));
            }
            let lines = std::mem::take(&mut *rows.lock().map_err(|_| fail("proof reader poisoned"))?);
            for line in lines {
                if let Some(values) = line.strip_prefix("ready ") {
                    let values: Vec<_> = values.split_whitespace().collect();
                    if values.len() != 2 {
                        return Err(fail("invalid worker identity"));
                    }
                    let candidate = WindowIdentity {
                        hwnd: values[0].parse().map_err(io::Error::other)?,
                        pid: values[1].parse().map_err(io::Error::other)?,
                    };
                    require(candidate.pid == child.child.id(), "exact independently spawned worker PID")?;
                    guard::verified(candidate)?;
                    worker_id = Some(candidate);
                    fit(&mut child, &guard, expected)?;
                } else if let Some(area) = line.strip_prefix("placed ") {
                    require(rect(area)? == expected, "exact submitted native target")?;
                    require(
                        guard.covered(),
                        "shown target remains covered before synthetic acknowledgement is admitted",
                    )?;
                    phase = if phase == 0 { 9 } else { 10 };
                    due = Instant::now() + Duration::from_millis(150);
                } else if let Some(handle) = line.strip_prefix("outside ") {
                    let values: Vec<_> = handle.split_whitespace().collect();
                    if values.len() != 2 {
                        return Err(fail("invalid outside process identity"));
                    }
                    let other_hwnd: u64 = values[0].parse().map_err(io::Error::other)?;
                    let pid: u32 = values[1].parse().map_err(io::Error::other)?;
                    require(
                        other.as_ref().is_some_and(|owner| owner.child.id() == pid) && pid != child.child.id(),
                        "outside app has a distinct owned process",
                    )?;
                    guard::verified(WindowIdentity { hwnd: other_hwnd, pid })?;
                    require(
                        unsafe { GetForegroundWindow() } == other_hwnd as HWND,
                        "OS accepted ordinary outside foreground request (refusal is inconclusive)",
                    )?;
                    guard.release(worker_id.unwrap(), expected)?;
                    outside_window = Some(other_hwnd as HWND);
                    phase = 8;
                    due = Instant::now() + Duration::from_millis(250);
                } else if line == "outside-hidden" {
                    child.send("focus")?;
                } else if line == "focused" {
                    phase = 2;
                    child.send("pause")?;
                } else if let Some(tick) = line.strip_prefix("paused ") {
                    pause_tick = tick.parse::<u64>().map_err(io::Error::other)?;
                    require(
                        unsafe { GetTickCount64() }.saturating_sub(pause_tick) < 500,
                        "worker pause receipt is fresh",
                    )?;
                    // This does not contact or wait for the blocked worker.
                    let worker = worker_id.unwrap();
                    let expected_client = guard::client(own(host))?;
                    require(
                        guard::input_open(own(host), marker, worker, expected, expected_client),
                        "unblocked exact worker-focus input baseline",
                    )?;
                    let before = Instant::now();
                    authorized_until.set(None);
                    guard.close()?;
                    require(
                        unsafe { GetForegroundWindow() } == worker.hwnd as HWND,
                        "cover does not steal worker keyboard focus",
                    )?;
                    require(
                        !guard::input_open(own(host), marker, worker, expected, expected_client),
                        "cover alone closes input with focus and geometry unchanged",
                    )?;
                    require(
                        before.elapsed() < Duration::from_millis(200),
                        "opaque native cover calls return within 200ms while worker paused",
                    )?;
                    println!(
                        "OBSERVED native_cover_call_us={} (not a compositor fence)",
                        before.elapsed().as_micros()
                    );
                    authorized_until.set(Some(Instant::now() + Duration::from_secs(5)));
                    guard.release(worker, expected)?;
                    require(
                        guard::input_open(own(host), marker, worker, expected, expected_client),
                        "same exact worker input reopens after synthetic authority returns",
                    )?;
                    // One request only. Keep pumping this controller's own
                    // messages to observe completion, never retry/force focus.
                    let requested = unsafe { GetTickCount64() };
                    let deadline = requested
                        .checked_add(FOCUS_WAIT_MS)
                        .ok_or_else(|| fail("foreground deadline overflow"))?
                        .min(
                            pause_tick
                                .checked_add(FOCUS_PAUSE_CUTOFF_MS)
                                .ok_or_else(|| fail("pause deadline overflow"))?,
                        );
                    focus_deadline = Some(deadline);
                    trace.enabled.set(true);
                    trace.record("host-request-before", None);
                    if requested < pause_tick || unsafe { GetTickCount64() } >= deadline {
                        trace.record("host-request-expired", None);
                        return Err(fail("INCONCLUSIVE: activation deadline expired before the single host request"));
                    }
                    // SAFETY: request activation only for this event thread's
                    // own host. The actual foreground worker handed back
                    // eligibility; no input injection or queue attachment.
                    focus_return = unsafe { SetForegroundWindow(host) };
                    trace.record("host-request-after", Some(focus_return));
                    phase = 11;
                } else if line == "resumed" {
                    require(
                        mask_checked && guard.covered() && unsafe { GetTickCount64() }.saturating_sub(pause_tick) >= 1700,
                        "cover remained independent of paused event loop",
                    )?;
                    require(
                        minimized_checked && unsafe { IsIconic(host) } == 0,
                        "minimize and restore both completed before worker resumed",
                    )?;
                    expected = target(host)?;
                    fit(&mut child, &guard, expected)?;
                    phase = 4;
                } else if line == "invalid-refused" {
                    phase = 6;
                    due = Instant::now() + Duration::from_millis(200);
                } else if line == "detached-hidden" {
                    require(
                        unsafe { IsWindowVisible(worker_id.unwrap().hwnd as HWND) } == 0,
                        "worker hides itself before detach retirement",
                    )?;
                    println!(
                        "NOT RUN egui surface/input routing; real GPU/session grants; grid; hosted recovery; visual native-frame acceptance"
                    );
                    return Ok(());
                }
            }
            if phase == 11 {
                let now = unsafe { GetTickCount64() };
                if now < pause_tick || focus_deadline.is_none_or(|deadline| now >= deadline) {
                    trace.record("host-foreground-timeout", Some(focus_return));
                    return Err(fail(format!(
                        "INCONCLUSIVE: host foreground was not confirmed before the live-pause deadline; SetForegroundWindow returned {focus_return}"
                    )));
                }
                if focus_return != 0 && unsafe { GetForegroundWindow() } == host {
                    trace.record("host-foreground-confirmed", Some(focus_return));
                    require(
                        unsafe { GetForegroundWindow() } == host,
                        "OS accepted explicit host foreground request (refusal is inconclusive)",
                    )?;
                    require(
                        guard.exposed(worker_id.unwrap(), expected),
                        "host activation keeps paused acknowledged worker above host",
                    )?;
                    require(
                        focus_deadline.is_some_and(|deadline| unsafe { GetTickCount64() } < deadline),
                        "host foreground and guarded exposure confirmed within the live-pause deadline",
                    )?;
                    old_area = expected;
                    let second_cover = Instant::now();
                    authorized_until.set(None);
                    guard.close()?;
                    require(guard.covered(), "revocation covers without worker response")?;
                    require(
                        second_cover.elapsed() < Duration::from_millis(200),
                        "second cover call returns within bound",
                    )?;
                    println!("OBSERVED second_cover_native_call_us={}", second_cover.elapsed().as_micros());
                    mask_started = Some(Instant::now());
                    phase = 3;
                    let host_rect = guard::bounds(own(host))?;
                    unsafe {
                        if SetWindowPos(
                            host,
                            std::ptr::null_mut(),
                            host_rect.x + host_rect.width as i32 / 2 + 80,
                            host_rect.y + 29,
                            host_rect.width as i32 + 40,
                            host_rect.height as i32 + 20,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        ) == 0
                        {
                            return Err(io::Error::last_os_error());
                        }
                    }
                    expected = target(host)?;
                    guard.target(expected)?;
                    unsafe {
                        ShowWindow(host, SW_MINIMIZE);
                    }
                    require(
                        unsafe { IsIconic(host) } != 0 && guard.covered(),
                        "minimize retains the unowned guard while worker paused",
                    )?;
                }
            }
            if Instant::now() >= due {
                if phase == 9 || phase == 10 {
                    // A worker has shown the target, but the controller still
                    // withholds its synthetic acknowledgement for this interval.
                    let point = windows_sys::Win32::Foundation::POINT {
                        x: expected.x + 10,
                        y: expected.y + expected.height as i32 - 10,
                    };
                    unsafe {
                        let hit = WindowFromPoint(point);
                        let mut pid = 0;
                        require(
                            GetWindowThreadProcessId(hit, &mut pid) != 0 && pid == std::process::id(),
                            "shown pending target is blocked by controller-owned native hit target",
                        )?;
                        let dc = GetDC(std::ptr::null_mut());
                        if dc.is_null() {
                            return Err(io::Error::last_os_error());
                        }
                        let color = GetPixel(dc, point.x, point.y);
                        ReleaseDC(std::ptr::null_mut(), dc);
                        require(
                            color == HOST_COLOR || color == 0,
                            "shown pending target remains opaque before release",
                        )?;
                    }
                    authorized_until.set(Some(Instant::now() + Duration::from_secs(5)));
                    guard.release(worker_id.unwrap(), expected)?;
                    require(
                        guard.exposed(worker_id.unwrap(), expected) && !guard.covered(),
                        "exact synthetic acknowledgement exposes world",
                    )?;
                    unsafe {
                        require(
                            GetWindow(worker_id.unwrap().hwnd as HWND, GW_OWNER).is_null(),
                            "worker remains independently unowned",
                        )?;
                        for window in [host, popup, worker_id.unwrap().hwnd as HWND] {
                            require(
                                GetWindowLongPtrW(window, GWL_EXSTYLE) & WS_EX_TOPMOST as isize == 0,
                                "no global topmost presentation",
                            )?;
                        }
                    }
                    phase = if phase == 9 { 1 } else { 5 };
                    due = Instant::now() + Duration::from_millis(350);
                } else if phase == 1 {
                    let point = windows_sys::Win32::Foundation::POINT {
                        x: expected.x + expected.width as i32 - 20,
                        y: expected.y + expected.height as i32 - 20,
                    };
                    visible_pixel(point, worker_id.unwrap().hwnd as HWND, BLUE, "exposed world outside popup")?;
                    let popup_rect = guard::bounds(popup_id)?;
                    visible_pixel(
                        windows_sys::Win32::Foundation::POINT {
                            x: popup_rect.x + 20,
                            y: popup_rect.y + 20,
                        },
                        popup,
                        GREEN,
                        "popup above world",
                    )?;
                    let (outside, read) = spawn(
                        &[
                            "--other".into(),
                            expected.x.to_string(),
                            expected.y.to_string(),
                            expected.width.to_string(),
                            expected.height.to_string(),
                            child.child.id().to_string(),
                        ],
                        rows.clone(),
                        host,
                    )?;
                    other = Some(outside);
                    other_reader = Some(read);
                    phase = 7;
                } else if phase == 8 {
                    let outside = outside_window.ok_or_else(|| fail("outside identity missing"))?;
                    let area = guard::client(WindowIdentity {
                        hwnd: outside as u64,
                        pid: other.as_ref().ok_or_else(|| fail("outside process missing"))?.child.id(),
                    })?;
                    visible_pixel(
                        windows_sys::Win32::Foundation::POINT {
                            x: area.x + 30,
                            y: area.y + 30,
                        },
                        outside,
                        GREEN,
                        "another foreground app stays above workspace",
                    )?;
                    other.as_mut().ok_or_else(|| fail("outside process missing"))?.send("exit")?;
                    phase = 7;
                } else if phase == 5 {
                    child.send("invalid")?;
                    phase = 7;
                } else if phase == 6 {
                    authorized_until.set(None);
                    guard.close()?;
                    require(
                        !guard::input_open(own(host), marker, worker_id.unwrap(), expected, guard::client(own(host))?),
                        "covered world refuses current native input",
                    )?;
                    child.send("detach")?;
                    phase = 7;
                }
            }
            if phase == 3 && !mask_checked && mask_started.is_some_and(|at| at.elapsed() >= Duration::from_millis(150)) {
                require(
                    unsafe { GetTickCount64() }.saturating_sub(pause_tick) < 1600,
                    "cover sampled before the worker can leave its bounded pause",
                )?;
                let point = windows_sys::Win32::Foundation::POINT {
                    x: old_area.x + 10,
                    y: old_area.y + old_area.height as i32 - 10,
                };
                unsafe {
                    let hit = WindowFromPoint(point);
                    let mut pid = 0;
                    require(
                        GetWindowThreadProcessId(hit, &mut pid) != 0 && pid == std::process::id() && hit != host && hit != popup,
                        "old world is covered by controller-owned HWND while worker stalled",
                    )?;
                    let dc = GetDC(std::ptr::null_mut());
                    if dc.is_null() {
                        return Err(io::Error::last_os_error());
                    }
                    let color = GetPixel(dc, point.x, point.y);
                    ReleaseDC(std::ptr::null_mut(), dc);
                    require(color == 0, "old world has opaque cover pixels while worker stalled")?;
                }
                require(
                    unsafe { GetTickCount64() }.saturating_sub(pause_tick) < 1600,
                    "old-world cover pixel sample completed before the worker can resume",
                )?;
                if !minimized_checked {
                    require(unsafe { IsIconic(host) } != 0, "first old-world sample is while minimized")?;
                    unsafe {
                        ShowWindow(host, SW_RESTORE);
                    }
                    require(
                        unsafe { IsIconic(host) } == 0 && guard.covered(),
                        "host restored while worker is still paused and covered",
                    )?;
                    require(
                        unsafe { GetTickCount64() }.saturating_sub(pause_tick) < 1600,
                        "restore completed before the worker can resume",
                    )?;
                    minimized_checked = true;
                    mask_started = Some(Instant::now());
                } else {
                    let outer = guard::bounds(own(host))?;
                    require(
                        point.x < outer.x
                            || point.x >= outer.x + outer.width as i32
                            || point.y < outer.y
                            || point.y >= outer.y + outer.height as i32,
                        "restored old-world cover sample is outside current host outer bounds",
                    )?;
                    require(
                        unsafe { GetTickCount64() }.saturating_sub(pause_tick) < 1600,
                        "restored cover pixel sample completed before the worker can resume",
                    )?;
                    mask_checked = true;
                }
            }
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    })();
    authorized_until.set(None);
    let _ = guard.close();
    let cleanup = (|| {
        if let Some(other) = &mut other {
            other.finish()?;
        }
        child.finish()
    })();
    drop(other);
    if let Some(reader) = other_reader {
        let _ = reader.join();
    }
    drop(child);
    let _ = reader.join();
    // Covers remain until this synthetic child has exited. This is neither a
    // production GPU-quiescence receipt nor authority to refund preview credit.
    unsafe {
        if IsWindow(popup) != 0 {
            DestroyWindow(popup);
        }
        if IsWindow(host) != 0 {
            DestroyWindow(host);
        }
    }
    trace.dump();
    drop(guard);
    result?;
    require(trace.dropped.get() == 0, "bounded native focus trace retained all observations")?;
    cleanup?;
    println!("PASS synthetic placement/activation/cover sequence and checked child cleanup; production path remains disabled");
    Ok(())
}

pub fn run() -> io::Result<()> {
    dpi()?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--worker") {
        if args.len() != 4 {
            return Err(fail("invalid worker invocation"));
        }
        worker(
            WindowIdentity {
                hwnd: args[1].parse().map_err(io::Error::other)?,
                pid: args[2].parse().map_err(io::Error::other)?,
            },
            args[3].parse().map_err(io::Error::other)?,
        )
    } else if args.first().is_some_and(|arg| arg == "--other") {
        if args.len() != 6 {
            return Err(fail("invalid outside application invocation"));
        }
        other_app(rect(&args[1..5].join(" "))?, args[5].parse().map_err(io::Error::other)?)
    } else if args.is_empty() {
        controller()
    } else {
        Err(fail("usage: host_native_frame"))
    }
}
