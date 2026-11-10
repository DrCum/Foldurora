//! Experimental native-frame presentation primitives. Not wired into the
//! production host or worker before the prerequisite Windows proof. No session/GPU/input ownership is
//! created here: the caller must retain the ordinary exact broker receipts.
//!
//! Guard HWNDs belong to the controller process but are deliberately unowned:
//! Windows hides native owned popups on minimize before a Winit notification.
//! A guard instead remains over retained worker pixels until explicit release.
//! This module is also consumed by the two-process native proof example.

use aurora_session::PhysicalRect;
use std::cell::Cell;
use std::io;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{
        BLACK_BRUSH, BeginPaint, ClientToScreen, EndPaint, FillRect, GetDC, GetStockObject, InvalidateRect, PAINTSTRUCT, ReleaseDC,
        UpdateWindow,
    },
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::WindowsAndMessaging::*,
};

const BLOCKED: &[u16] = &[
    70, 111, 108, 100, 117, 114, 111, 114, 97, 46, 70, 114, 97, 109, 101, 46, 66, 108, 111, 99, 107, 101, 100, 46, 51, 0,
];
const MARKER: &[u16] = &[
    70, 111, 108, 100, 117, 114, 111, 114, 97, 46, 70, 114, 97, 109, 101, 46, 79, 119, 110, 101, 114, 46, 51, 0,
];
const OLD_COVER: &[u16] = &[70, 83, 79, 108, 100, 71, 117, 97, 114, 100, 51, 0];
const TARGET_COVER: &[u16] = &[70, 83, 84, 97, 114, 103, 101, 116, 71, 117, 97, 114, 100, 51, 0];
const GENERATION: &[u16] = &[70, 83, 71, 117, 97, 114, 100, 71, 101, 110, 51, 0];
const PAINTED: &[u16] = &[70, 83, 71, 117, 97, 114, 100, 80, 97, 105, 110, 116, 51, 0];
const EPOCH: &[u16] = &[70, 83, 72, 111, 115, 116, 69, 112, 111, 99, 104, 51, 0];
const CLASS: &[u16] = &[
    70, 111, 108, 100, 117, 114, 111, 114, 97, 71, 117, 97, 114, 100, 67, 111, 118, 101, 114, 86, 51, 0,
];
const SUBCLASS_ID: usize = 0x4653_3301;
// No native pointer is an incarnation token. Monotonic per-process identities
// prevent same-process HWND/allocation reuse from reviving an old registration.
// The production lease/binding is still required independently.
static NEXT_REGISTRATION: AtomicUsize = AtomicUsize::new(1);

type SubclassProc = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, callback: SubclassProc, id: usize, data: usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, callback: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowIdentity {
    pub hwnd: u64,
    pub pid: u32,
}

fn error(message: &'static str) -> io::Error {
    io::Error::other(message)
}

pub(crate) fn verified(window: WindowIdentity) -> io::Result<HWND> {
    let hwnd = window.hwnd as HWND;
    let mut pid = 0;
    // SAFETY: read-only native identity query; no foreign window is modified.
    if window.hwnd != hwnd as usize as u64
        || window.hwnd == 0
        || window.pid == 0
        || unsafe { IsWindow(hwnd) == 0 || GetWindowThreadProcessId(hwnd, &mut pid) == 0 }
        || pid != window.pid
    {
        return Err(error("native window identity changed"));
    }
    Ok(hwnd)
}

pub(crate) fn bounds(window: WindowIdentity) -> io::Result<PhysicalRect> {
    let hwnd = verified(window)?;
    let mut rect = RECT::default();
    // SAFETY: read-only query into a valid stack rectangle.
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = PhysicalRect {
        x: rect.left,
        y: rect.top,
        width: u32::try_from(i64::from(rect.right) - i64::from(rect.left)).map_err(io::Error::other)?,
        height: u32::try_from(i64::from(rect.bottom) - i64::from(rect.top)).map_err(io::Error::other)?,
    };
    result.validate().map_err(io::Error::other)?;
    verified(window)?;
    Ok(result)
}

pub(crate) fn client(window: WindowIdentity) -> io::Result<PhysicalRect> {
    let hwnd = verified(window)?;
    let mut rect = RECT::default();
    let mut origin = POINT::default();
    // SAFETY: read-only query into valid stack objects, using the process DPI mode.
    if unsafe { GetClientRect(hwnd, &mut rect) == 0 || ClientToScreen(hwnd, &mut origin) == 0 } {
        return Err(io::Error::last_os_error());
    }
    let result = PhysicalRect {
        x: origin.x,
        y: origin.y,
        width: u32::try_from(rect.right - rect.left).map_err(io::Error::other)?,
        height: u32::try_from(rect.bottom - rect.top).map_err(io::Error::other)?,
    };
    result.validate().map_err(io::Error::other)?;
    verified(window)?;
    Ok(result)
}

/// Folderstorm fssessionworker.cpp::fitSurface (abbe3cb0), strengthened with
/// exact native identity and failure checks. Caller changes only its own HWND.
/// A topmost predecessor is a boundary, never permission to join that band.
pub(crate) fn above(own: HWND, anchor: WindowIdentity, rect: Option<PhysicalRect>) -> io::Result<()> {
    place_above(own, anchor, rect, false)
}

pub(crate) fn show_above(own: HWND, anchor: WindowIdentity, rect: Option<PhysicalRect>) -> io::Result<()> {
    place_above(own, anchor, rect, true)
}

fn place_above(own: HWND, anchor: WindowIdentity, rect: Option<PhysicalRect>, show: bool) -> io::Result<()> {
    verified(WindowIdentity {
        hwnd: own as u64,
        pid: std::process::id(),
    })?;
    // SAFETY: modifications are confined to this actual event thread.
    if unsafe { GetWindowThreadProcessId(own, std::ptr::null_mut()) != GetCurrentThreadId() } {
        return Err(error("presentation window belongs to another event thread"));
    }
    let anchor_hwnd = verified(anchor)?;
    if own == anchor_hwnd {
        return Err(error("native placement cannot anchor itself"));
    }
    // SAFETY: query the foreign anchor and its neighbor; move only our owned
    // HWND, without activation, parenting, thread attachment or TOPMOST.
    unsafe {
        if GetWindowLongPtrW(own, GWL_EXSTYLE) & WS_EX_TOPMOST as isize != 0 {
            return Err(error("presentation window unexpectedly became topmost"));
        }
        let predecessor = GetWindow(anchor_hwnd, GW_HWNDPREV);
        let ordered = predecessor == own;
        let insert = if predecessor.is_null() || GetWindowLongPtrW(predecessor, GWL_EXSTYLE) & WS_EX_TOPMOST as isize != 0 {
            HWND_TOP
        } else {
            predecessor
        };
        if ordered && rect.is_none() && (!show || IsWindowVisible(own) != 0) {
            return Ok(());
        }
        let mut flags = SWP_NOACTIVATE | SWP_NOOWNERZORDER | if show { SWP_SHOWWINDOW } else { 0 };
        if ordered {
            flags |= SWP_NOZORDER;
        }
        let (x, y, width, height) = if let Some(rect) = rect {
            rect.validate().map_err(io::Error::other)?;
            (rect.x, rect.y, rect.width as i32, rect.height as i32)
        } else {
            flags |= SWP_NOMOVE | SWP_NOSIZE;
            (0, 0, 0, 0)
        };
        verified(anchor)?;
        if SetWindowPos(own, insert, x, y, width, height, flags) == 0 {
            return Err(io::Error::last_os_error());
        }
        verified(anchor)?;
        if GetWindow(own, GW_HWNDNEXT) != anchor_hwnd {
            return Err(error("native relative placement was not retained"));
        }
    }
    Ok(())
}

fn below(own: HWND, anchor: WindowIdentity, rect: Option<PhysicalRect>, show: bool) -> io::Result<()> {
    verified(WindowIdentity {
        hwnd: own as u64,
        pid: std::process::id(),
    })?;
    let anchor = verified(anchor)?;
    // SAFETY: query native order; change only our current event-thread HWND.
    unsafe {
        if own == anchor
            || GetWindowThreadProcessId(own, std::ptr::null_mut()) != GetCurrentThreadId()
            || GetWindowLongPtrW(own, GWL_EXSTYLE) & WS_EX_TOPMOST as isize != 0
            || GetWindowLongPtrW(anchor, GWL_EXSTYLE) & WS_EX_TOPMOST as isize != 0
        {
            return Err(error("invalid non-topmost presentation order"));
        }
        if rect.is_none() && GetWindow(own, GW_HWNDPREV) == anchor && (!show || IsWindowVisible(own) != 0) {
            return Ok(());
        }
        let mut flags = SWP_NOACTIVATE | SWP_NOOWNERZORDER | if show { SWP_SHOWWINDOW } else { 0 };
        let (x, y, width, height) = if let Some(rect) = rect {
            rect.validate().map_err(io::Error::other)?;
            (rect.x, rect.y, rect.width as i32, rect.height as i32)
        } else {
            flags |= SWP_NOMOVE | SWP_NOSIZE;
            (0, 0, 0, 0)
        };
        if SetWindowPos(own, anchor, x, y, width, height, flags) == 0 {
            return Err(io::Error::last_os_error());
        }
        if GetWindow(own, GW_HWNDPREV) != anchor {
            return Err(error("native guarded order was not retained"));
        }
    }
    Ok(())
}

pub(crate) fn marker(host: WindowIdentity) -> io::Result<usize> {
    let hwnd = verified(host)?;
    // SAFETY: query native metadata on the configured controller only.
    let marker = unsafe { GetPropW(hwnd, MARKER.as_ptr()) } as usize;
    if marker == 0 {
        return Err(error("rectangular-host capability unavailable"));
    }
    Ok(marker)
}

fn contains(outer: PhysicalRect, inner: PhysicalRect) -> bool {
    i64::from(inner.x) >= i64::from(outer.x)
        && i64::from(inner.y) >= i64::from(outer.y)
        && i64::from(inner.x) + i64::from(inner.width) <= i64::from(outer.x) + i64::from(outer.width)
        && i64::from(inner.y) + i64::from(inner.height) <= i64::from(outer.y) + i64::from(outer.height)
}

/// One immutable insertion anchor for fit, translation, show and restacking.
/// Hiding masks never invalidates this anchor. BLOCKED is an input/visibility
/// gate, never a choice between competing worker insertion policies.
pub(crate) fn worker_place(
    own: HWND,
    host: WindowIdentity,
    expected_marker: usize,
    rect: PhysicalRect,
    show: bool,
) -> io::Result<PhysicalRect> {
    rect.validate().map_err(io::Error::other)?;
    let host_rect = client(host)?;
    let anchor = worker_anchor(host, expected_marker)?;
    below(own, anchor, Some(rect), show)?;
    if worker_anchor(host, expected_marker)? != anchor {
        return Err(error("native host changed during placement"));
    }
    Ok(host_rect)
}

fn worker_anchor(host: WindowIdentity, expected_marker: usize) -> io::Result<WindowIdentity> {
    let hwnd = verified(host)?;
    if marker(host)? != expected_marker {
        return Err(error("native host registration changed"));
    }
    // SAFETY: query only scalar properties of the exact registration. An
    // arbitrary foreign pointer is never dereferenced.
    let anchor = WindowIdentity {
        hwnd: unsafe { GetPropW(hwnd, OLD_COVER.as_ptr()) } as u64,
        pid: host.pid,
    };
    let cover = verified(anchor)?;
    if unsafe { GetPropW(cover, MARKER.as_ptr()) } as usize != expected_marker {
        return Err(error("native insertion anchor changed"));
    }
    Ok(anchor)
}

/// Observes relative order without changing any window. A bounded unknown
/// result refuses presentation, rather than moving to a global topmost band.
fn is_above(upper: HWND, lower: HWND) -> io::Result<bool> {
    // SAFETY: read-only traversal of the desktop's native Z-order.
    unsafe {
        let mut current = GetTopWindow(std::ptr::null_mut());
        for _ in 0..4096 {
            if current.is_null() {
                break;
            }
            if current == upper {
                return Ok(true);
            }
            if current == lower {
                return Ok(false);
            }
            current = GetWindow(current, GW_HWNDNEXT);
        }
    }
    Err(error("native order could not be established"))
}

/// Raise only. Moving a persistent anchor down to an old worker could expose
/// a newer worker, so retaining its current higher position is mandatory.
fn raise_above(own: HWND, anchor: WindowIdentity, rect: Option<PhysicalRect>, show: bool) -> io::Result<()> {
    let anchor_hwnd = verified(anchor)?;
    verified(WindowIdentity {
        hwnd: own as u64,
        pid: std::process::id(),
    })?;
    if !is_above(own, anchor_hwnd)? {
        return place_above(own, anchor, rect, show);
    }
    let mut flags = SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOZORDER;
    if show {
        flags |= SWP_SHOWWINDOW;
    }
    let (x, y, width, height) = match rect {
        Some(rect) => {
            rect.validate().map_err(io::Error::other)?;
            (rect.x, rect.y, rect.width as i32, rect.height as i32)
        }
        None => {
            flags |= SWP_NOMOVE | SWP_NOSIZE;
            (0, 0, 0, 0)
        }
    };
    // SAFETY: this event thread changes only its own nonactivating window.
    unsafe {
        if GetWindowThreadProcessId(own, std::ptr::null_mut()) != GetCurrentThreadId()
            || GetWindowLongPtrW(own, GWL_EXSTYLE) & WS_EX_TOPMOST as isize != 0
        {
            return Err(error("invalid anchor owner or topmost band"));
        }
        if SetWindowPos(own, std::ptr::null_mut(), x, y, width, height, flags) == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    if !is_above(own, verified(anchor)?)? {
        return Err(error("anchor was lowered"));
    }
    Ok(())
}

const WORKER_SUBCLASS: usize = 0x4653_3302;
const WORKER_MARKER: &[u16] = &[70, 83, 87, 111, 114, 107, 101, 114, 71, 117, 97, 114, 100, 51, 0];

struct WorkerState {
    own: WindowIdentity,
    host: WindowIdentity,
    marker: usize,
    attached: Cell<bool>,
}

/// Only the worker's own event thread installs this constraint. All native
/// show/activation requests, including OS activation, keep this same anchor.
/// Drop it on the authenticated detach path BEFORE restoring ordinary styles.
pub(crate) struct WorkerOrder(Rc<WorkerState>);

impl WorkerOrder {
    pub(crate) fn new(own: HWND, host: WindowIdentity, marker: usize) -> io::Result<Self> {
        worker_anchor(host, marker)?;
        let identity = WindowIdentity {
            hwnd: own as u64,
            pid: std::process::id(),
        };
        verified(identity)?;
        // SAFETY: subclass only this actual worker UI thread's own HWND.
        if unsafe { GetWindowThreadProcessId(own, std::ptr::null_mut()) != GetCurrentThreadId() } {
            return Err(error("worker constraint requires its native event thread"));
        }
        if unsafe { !GetPropW(own, WORKER_MARKER.as_ptr()).is_null() } {
            return Err(error("worker already has a native order constraint"));
        }
        let state = Rc::new(WorkerState {
            own: identity,
            host,
            marker,
            attached: Cell::new(false),
        });
        let pointer = Rc::into_raw(state.clone()) as usize;
        // SAFETY: retain the registration until removal or WM_NCDESTROY.
        if unsafe { SetPropW(own, WORKER_MARKER.as_ptr(), pointer as _) } == 0 {
            unsafe {
                drop(Rc::from_raw(pointer as *const WorkerState));
            }
            return Err(io::Error::last_os_error());
        }
        if unsafe { SetWindowSubclass(own, Some(worker_proc), WORKER_SUBCLASS, pointer) } == 0 {
            unsafe {
                RemovePropW(own, WORKER_MARKER.as_ptr());
            }
            unsafe {
                drop(Rc::from_raw(pointer as *const WorkerState));
            }
            return Err(io::Error::last_os_error());
        }
        state.attached.set(true);
        Ok(Self(state))
    }
    /// Ordinary-window restoration must not proceed if the exact native hook
    /// could not be removed. Drop alone is deliberately not a success receipt.
    pub(crate) fn detach(&self) -> io::Result<()> {
        let state = self.0.clone();
        if !state.attached.get() {
            return Ok(());
        }
        let own = verified(state.own)?;
        unsafe {
            if GetWindowThreadProcessId(own, std::ptr::null_mut()) != GetCurrentThreadId()
                || !std::ptr::eq(GetPropW(own, WORKER_MARKER.as_ptr()).cast_const(), Rc::as_ptr(&state).cast())
            {
                return Err(error("worker native registration changed before detach"));
            }
            if RemoveWindowSubclass(own, Some(worker_proc), WORKER_SUBCLASS) == 0 {
                return Err(io::Error::last_os_error());
            }
            state.attached.set(false);
            RemovePropW(own, WORKER_MARKER.as_ptr());
            drop(Rc::from_raw(Rc::as_ptr(&state)));
        }
        Ok(())
    }
}

unsafe extern "system" fn worker_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _: usize, data: usize) -> LRESULT {
    let pointer = data as *const WorkerState;
    // SAFETY: registration owns a strong reference for the whole callback.
    unsafe {
        Rc::increment_strong_count(pointer);
    }
    let state = unsafe { Rc::from_raw(pointer) };
    if message == WM_WINDOWPOSCHANGING && lparam != 0 {
        let position = unsafe { &mut *(lparam as *mut WINDOWPOS) };
        if position.flags & SWP_NOZORDER == 0 || position.flags & SWP_SHOWWINDOW != 0 {
            match worker_anchor(state.host, state.marker) {
                Ok(anchor) => {
                    position.hwndInsertAfter = anchor.hwnd as HWND;
                    position.flags &= !SWP_NOZORDER;
                    position.flags |= SWP_NOOWNERZORDER;
                }
                Err(_) => {
                    // No authority to order against a reused/missing host.
                    // Hide only this owned worker and refuse a pending show.
                    position.flags = (position.flags & !SWP_SHOWWINDOW) | SWP_HIDEWINDOW | SWP_NOZORDER | SWP_NOACTIVATE;
                }
            }
        }
    }
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    if message == WM_NCDESTROY && state.attached.replace(false) {
        unsafe {
            if GetPropW(hwnd, WORKER_MARKER.as_ptr()) as usize == data {
                RemovePropW(hwnd, WORKER_MARKER.as_ptr());
            }
            drop(Rc::from_raw(pointer));
        }
    }
    result
}

impl Drop for WorkerOrder {
    fn drop(&mut self) {
        let _ = self.detach();
    }
}

fn union(a: PhysicalRect, b: PhysicalRect) -> io::Result<PhysicalRect> {
    let left = a.x.min(b.x);
    let top = a.y.min(b.y);
    let right = (i64::from(a.x) + i64::from(a.width)).max(i64::from(b.x) + i64::from(b.width));
    let bottom = (i64::from(a.y) + i64::from(a.height)).max(i64::from(b.y) + i64::from(b.height));
    let result = PhysicalRect {
        x: left,
        y: top,
        width: u32::try_from(right - i64::from(left)).map_err(io::Error::other)?,
        height: u32::try_from(bottom - i64::from(top)).map_err(io::Error::other)?,
    };
    result.validate().map_err(io::Error::other)?;
    Ok(result)
}

pub(crate) struct Guard(Rc<State>);

struct State {
    host: WindowIdentity,
    registration: usize,
    covers: [HWND; 2],
    worker: Cell<Option<WindowIdentity>>,
    retained: Cell<Option<PhysicalRect>>,
    targets: Cell<Option<PhysicalRect>>,
    attached: Cell<bool>,
    arming: Cell<bool>,
    failed: Cell<bool>,
    epoch: Cell<usize>,
    generation: Cell<usize>,
    exposing: Cell<bool>,
    may_expose: Cell<bool>,
    authority: Rc<dyn Fn() -> bool>,
    surfaces: Rc<dyn Fn(WindowIdentity) -> io::Result<()>>,
    // Optional caller-owned observation sink. It must not mutate native state,
    // wait, or reenter this guard. The offline proof uses bounded memory only.
    trace: Rc<dyn Fn(&'static str)>,
}

impl Guard {
    /// Must run on the host's actual native event thread, before adoption.
    pub(crate) fn new(
        host: u64,
        authority: Rc<dyn Fn() -> bool>,
        surfaces: Rc<dyn Fn(WindowIdentity) -> io::Result<()>>,
        trace: Rc<dyn Fn(&'static str)>,
    ) -> io::Result<Self> {
        let host = WindowIdentity {
            hwnd: host,
            pid: std::process::id(),
        };
        let hwnd = verified(host)?;
        let registration = NEXT_REGISTRATION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| value.checked_add(1))
            .map_err(|_| error("native registration identity exhausted"))?;
        // SAFETY: subclass and cover state belong to the actual host UI thread.
        if unsafe { GetWindowThreadProcessId(hwnd, std::ptr::null_mut()) != GetCurrentThreadId() } {
            return Err(error("native guard must be created on the host event thread"));
        }
        // SAFETY: read only; never replace an existing live guard's callback.
        if unsafe { !GetPropW(hwnd, MARKER.as_ptr()).is_null() } {
            return Err(error("host already has a native presentation guard"));
        }
        // SAFETY: register our own class and create hidden windows in this
        // process/thread. No foreign owner/parent or application region.
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            let class = WNDCLASSW {
                lpfnWndProc: Some(cover_proc),
                hInstance: module,
                lpszClassName: CLASS.as_ptr(),
                hbrBackground: GetStockObject(BLACK_BRUSH) as _,
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 && windows_sys::Win32::Foundation::GetLastError() != 1410 {
                return Err(io::Error::last_os_error());
            }
            let create = || {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    CLASS.as_ptr(),
                    CLASS.as_ptr(),
                    WS_POPUP,
                    0,
                    0,
                    1,
                    1,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    module,
                    std::ptr::null(),
                )
            };
            let first = create();
            if first.is_null() {
                return Err(io::Error::last_os_error());
            }
            let second = create();
            if second.is_null() {
                DestroyWindow(first);
                return Err(io::Error::last_os_error());
            }
            if let Err(error) = above(first, host, None).and_then(|()| {
                above(
                    second,
                    WindowIdentity {
                        hwnd: first as u64,
                        pid: host.pid,
                    },
                    None,
                )
            }) {
                DestroyWindow(second);
                DestroyWindow(first);
                return Err(error);
            }
            let state = Rc::new(State {
                host,
                registration,
                covers: [first, second],
                worker: Cell::new(None),
                retained: Cell::new(None),
                targets: Cell::new(None),
                attached: Cell::new(false),
                arming: Cell::new(false),
                failed: Cell::new(false),
                epoch: Cell::new(0),
                generation: Cell::new(0),
                exposing: Cell::new(false),
                may_expose: Cell::new(false),
                authority,
                surfaces,
                trace,
            });
            let pointer = Rc::into_raw(state.clone()) as usize;
            if SetPropW(first, MARKER.as_ptr(), registration as _) == 0
                || SetPropW(second, MARKER.as_ptr(), registration as _) == 0
                || SetPropW(hwnd, OLD_COVER.as_ptr(), first as _) == 0
                || SetPropW(hwnd, TARGET_COVER.as_ptr(), second as _) == 0
                || SetPropW(hwnd, MARKER.as_ptr(), registration as _) == 0
                || SetPropW(hwnd, BLOCKED.as_ptr(), 1usize as _) == 0
                || SetWindowSubclass(hwnd, Some(host_proc), SUBCLASS_ID, pointer) == 0
            {
                let error = io::Error::last_os_error();
                RemovePropW(hwnd, MARKER.as_ptr());
                RemovePropW(hwnd, BLOCKED.as_ptr());
                RemovePropW(hwnd, OLD_COVER.as_ptr());
                RemovePropW(hwnd, TARGET_COVER.as_ptr());
                drop(Rc::from_raw(pointer as *const State));
                return Err(error);
            }
            state.attached.set(true);
            Ok(Self(state))
        }
    }

    pub(crate) fn target(&self, rect: PhysicalRect) -> io::Result<()> {
        let state = self.0.clone();
        state.arm()?;
        let target = rect
            .validate()
            .map_err(io::Error::other)
            .and_then(|()| state.targets.get().map_or(Ok(rect), |old| union(old, rect)));
        let target = match target {
            Ok(target) => target,
            Err(error) => {
                state.failed.set(true);
                return Err(error);
            }
        };
        state.targets.set(Some(target));
        state.arm()
    }

    pub(crate) fn close(&self) -> io::Result<()> {
        self.0.clone().arm()
    }

    pub(crate) fn covered(&self) -> bool {
        self.0.verify_host().is_err() || unsafe { !GetPropW(self.0.host.hwnd as HWND, BLOCKED.as_ptr()).is_null() }
    }

    pub(crate) fn exposed(&self, worker: WindowIdentity, expected: PhysicalRect) -> bool {
        !self.covered()
            && self.0.worker.get() == Some(worker)
            && bounds(worker).ok() == Some(expected)
            && unsafe { GetWindow(self.0.host.hwnd as HWND, GW_HWNDPREV) == worker.hwnd as HWND }
    }

    /// Only after the caller's exact current native/GPU grant and committed
    /// popup paint agree. Native geometry alone never authorizes this call.
    pub(crate) fn release(&self, worker: WindowIdentity, expected: PhysicalRect) -> io::Result<()> {
        let state = self.0.clone();
        if !state.targets.get().is_some_and(|target| contains(target, expected))
            && !(state.worker.get() == Some(worker) && state.retained.get() == Some(expected))
        {
            state.failed.set(true);
            let _ = state.arm();
            return Err(error("worker has no protected target rectangle"));
        }
        // Keep every prior rectangle until matching acknowledgement. A target
        // union is retired only after this exact exposure succeeds.
        let old_worker = state.worker.replace(Some(worker));
        let old_rect = state.retained.replace(Some(expected));
        state.may_expose.set(true);
        let result = state.expose(worker, expected);
        if result.is_err() {
            // targets still protect the newly submitted position; retain old
            // coverage as well. Never interpret placement as GPU quiescence.
            state.worker.set(old_worker);
            state.retained.set(old_rect);
            state.failed.set(true);
            let _ = state.arm();
        } else {
            state.targets.set(None);
        }
        result
    }
}

impl State {
    fn verify_host(&self) -> io::Result<HWND> {
        let hwnd = verified(self.host)?;
        // SAFETY: require this exact registration, not just same-process reuse.
        if unsafe { GetPropW(hwnd, MARKER.as_ptr()) } as usize != self.registration {
            return Err(error("native host registration changed"));
        }
        Ok(hwnd)
    }

    fn expose(&self, worker: WindowIdentity, expected: PhysicalRect) -> io::Result<()> {
        (self.trace)("expose-entry");
        let host = self.verify_host()?;
        if self.failed.get()
            || !self.may_expose.get()
            || !(self.authority)()
            || bounds(worker)? != expected
            || !contains(client(self.host)?, expected)
            || !is_above(verified(worker_anchor(self.host, self.registration)?)?, verified(worker)?)?
            || unsafe { IsWindowVisible(host) == 0 || IsIconic(host) != 0 || IsWindowVisible(worker.hwnd as HWND) == 0 }
        {
            (self.trace)("expose-refused");
            return Err(error("cover release has no current presentation authority"));
        }
        if self.exposing.replace(true) {
            (self.trace)("expose-reentered");
            return Err(error("native exposure reentered"));
        }
        let generation = self.generation.get();
        let result = (|| {
            // Lower only our own rectangular host. Never send the worker a
            // synchronous request: a suspended worker cannot block the cover.
            below(host, worker, None, false)?;
            (self.surfaces)(WindowIdentity {
                hwnd: self.covers[1] as u64,
                pid: self.host.pid,
            })?;
            if bounds(worker)? != expected || !(self.authority)() || generation != self.generation.get() {
                return Err(error("native presentation changed during exposure"));
            }
            let epoch = self.epoch.get().checked_add(1).ok_or_else(|| error("native boundary exhausted"))?;
            unsafe {
                if SetPropW(host, EPOCH.as_ptr(), epoch as _) == 0 {
                    return Err(io::Error::last_os_error());
                }
                for cover in self.covers {
                    ShowWindow(cover, SW_HIDE);
                    if IsWindowVisible(cover) != 0 || generation != self.generation.get() || !(self.authority)() {
                        return Err(error("cover changed while retiring"));
                    }
                }
                if RemovePropW(host, BLOCKED.as_ptr()).is_null() && self.epoch.get() == 0 {
                    return Err(error("initial native barrier was missing"));
                }
            }
            if generation != self.generation.get() || !(self.authority)() || bounds(worker)? != expected {
                return Err(error("native barrier was rearmed during exposure"));
            }
            self.epoch.set(epoch);
            Ok(())
        })();
        self.exposing.set(false);
        (self.trace)(if result.is_ok() { "expose-ok" } else { "expose-error" });
        result
    }

    fn arm(&self) -> io::Result<()> {
        self.may_expose.set(false);
        let host = self.verify_host()?;
        let generation = self
            .generation
            .get()
            .checked_add(1)
            .ok_or_else(|| error("cover generation exhausted"))?;
        self.generation.set(generation);
        // SAFETY: set a blocking-only property on this owned host before any
        // move/minimize/hide reaches its original window procedure.
        if unsafe { SetPropW(host, BLOCKED.as_ptr(), 1usize as _) == 0 || SetPropW(host, GENERATION.as_ptr(), generation as _) == 0 } {
            return Err(io::Error::last_os_error());
        }
        if self.arming.replace(true) {
            return Err(error("native cover reentered"));
        }
        let result = (|| {
            let mut highest = self.host;
            if let Some(worker) = self.worker.get() {
                let old = match (self.retained.get(), bounds(worker).ok()) {
                    (Some(retained), Some(observed)) => union(retained, observed)?,
                    (Some(retained), None) => retained,
                    (None, Some(observed)) => observed,
                    (None, None) => return Err(error("retained worker bounds missing")),
                };
                self.retained.set(Some(old));
                let anchor = if verified(worker).is_ok() { worker } else { self.host };
                self.show_cover(self.covers[0], old, anchor)?;
                highest = WindowIdentity {
                    hwnd: self.covers[0] as u64,
                    pid: self.host.pid,
                };
            }
            if let Some(targets) = self.targets.get() {
                self.show_cover(self.covers[1], targets, highest)?;
                highest = WindowIdentity {
                    hwnd: self.covers[1] as u64,
                    pid: self.host.pid,
                };
            }
            if self.retained.get().is_some() || self.targets.get().is_some() {
                // Raise only our host above the masks; the immutable insertion
                // anchor stays above every constrained managed worker.
                raise_above(host, highest, None, false)?;
            }
            Ok(())
        })();
        self.arming.set(false);
        if result.is_err() {
            self.failed.set(true);
        }
        result
    }

    fn show_cover(&self, cover: HWND, rect: PhysicalRect, anchor: WindowIdentity) -> io::Result<()> {
        // Never allocate a desktop-sized DIB: this GDI class paints opaque
        // black synchronously. Old coverage stays while target coverage grows.
        let own = WindowIdentity {
            hwnd: cover as u64,
            pid: self.host.pid,
        };
        // SAFETY: avoid repainting identical retained masks on idle polls.
        if bounds(own).ok() == Some(rect) && unsafe { IsWindowVisible(cover) != 0 && !GetPropW(cover, PAINTED.as_ptr()).is_null() } {
            return raise_above(cover, anchor, None, true);
        }
        raise_above(cover, anchor, Some(rect), false)?;
        // SAFETY: repaint/show our own nonactivating, non-topmost guard only.
        unsafe {
            let dc = GetDC(cover);
            if dc.is_null() {
                return Err(io::Error::last_os_error());
            }
            let painted = SendMessageW(cover, WM_ERASEBKGND, dc as usize, 0) != 0;
            ReleaseDC(cover, dc);
            if !painted || InvalidateRect(cover, std::ptr::null(), 1) == 0 {
                return Err(error("native opaque guard could not initialize"));
            }
            // Showing and relative ordering must be one native operation:
            // ShowWindow followed by reordering could flash above another app.
            raise_above(cover, anchor, None, true)?;
            if UpdateWindow(cover) == 0 || GetPropW(cover, PAINTED.as_ptr()).is_null() || IsWindowVisible(cover) == 0 {
                return Err(error("native opaque guard paint failed"));
            }
        }
        raise_above(cover, anchor, None, false)
    }
}

/// Current cover observation only. This does not authenticate a queued OS
/// message or replace the existing source/lease/input-release/IME pipeline.
pub(crate) fn input_open(
    host: WindowIdentity,
    expected_marker: usize,
    own: WindowIdentity,
    expected: PhysicalRect,
    expected_client: PhysicalRect,
) -> bool {
    let Ok(hwnd) = verified(host) else {
        return false;
    };
    if expected_marker == 0
        || marker(host).ok() != Some(expected_marker)
        || client(host).ok() != Some(expected_client)
        || !contains(expected_client, expected)
        || verified(own).is_err()
        || bounds(own).ok() != Some(expected)
    {
        return false;
    }
    unsafe {
        let registration = GetPropW(hwnd, MARKER.as_ptr()) as usize;
        let epoch = GetPropW(hwnd, EPOCH.as_ptr()) as usize;
        registration == expected_marker
            && epoch != 0
            && GetPropW(hwnd, BLOCKED.as_ptr()).is_null()
            && IsWindowVisible(hwnd) != 0
            && IsIconic(hwnd) == 0
            && IsWindowVisible(own.hwnd as HWND) != 0
            && GetForegroundWindow() == own.hwnd as HWND
            && GetPropW(hwnd, MARKER.as_ptr()) as usize == registration
            && GetPropW(hwnd, EPOCH.as_ptr()) as usize == epoch
            && GetPropW(hwnd, BLOCKED.as_ptr()).is_null()
    }
}

unsafe extern "system" fn host_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _: usize, data: usize) -> LRESULT {
    // SAFETY: the subclass owns a strong Rc until removal/WM_NCDESTROY.
    // Retain one for this entire callback, including reentrant original-proc
    // calls and any controller teardown they trigger.
    let pointer = data as *const State;
    unsafe {
        Rc::increment_strong_count(pointer);
    }
    let state = unsafe { Rc::from_raw(pointer) };
    match message {
        WM_ACTIVATE => (state.trace)("host-activate-before"),
        WM_WINDOWPOSCHANGED => (state.trace)("host-position-before"),
        _ => {}
    }
    let geometry = message == WM_WINDOWPOSCHANGING
        && lparam != 0
        && unsafe {
            let position = &*(lparam as *const WINDOWPOS);
            position.flags & (SWP_NOMOVE | SWP_NOSIZE) != (SWP_NOMOVE | SWP_NOSIZE) || position.flags & SWP_HIDEWINDOW != 0
        };
    let changing =
        geometry || message == WM_SHOWWINDOW && wparam == 0 || message == WM_SYSCOMMAND && (wparam as u32 & 0xfff0) == SC_MINIMIZE;
    if changing && (state.arming.get() || state.arm().is_err()) {
        // A sent geometry change during arming may be constrained, but it must
        // not tear down an otherwise healthy guard or replay a queued gesture.
        if message == WM_SYSCOMMAND {
            return 0;
        }
        if message == WM_WINDOWPOSCHANGING && lparam != 0 {
            // SAFETY: Windows supplies the writable WINDOWPOS for our host.
            let pos = unsafe { &mut *(lparam as *mut WINDOWPOS) };
            pos.flags = (pos.flags & !(SWP_HIDEWINDOW | SWP_SHOWWINDOW)) | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
        }
    }
    // SAFETY: preserve Winit's original procedure and standard subclass chain.
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    // Native activation can raise the host while the worker is suspended.
    // Use only this UI thread's cached, nonblocking exact-authority callback;
    // otherwise retain the cover. Geometry invalidation already cleared it.
    if matches!(message, WM_WINDOWPOSCHANGED | WM_ACTIVATE)
        && !state.arming.get()
        && !state.exposing.get()
        && state.may_expose.get()
        && let (Some(worker), Some(rect)) = (state.worker.get(), state.retained.get())
        && (bounds(worker).ok() != Some(rect) || !(state.authority)() || !is_above(worker.hwnd as HWND, hwnd).unwrap_or(false))
        && state.expose(worker, rect).is_err()
    {
        let _ = state.arm();
    }
    match message {
        WM_ACTIVATE => (state.trace)("host-activate-after"),
        WM_WINDOWPOSCHANGED => (state.trace)("host-position-after"),
        _ => {}
    }
    if message == WM_NCDESTROY && state.attached.replace(false) {
        // SAFETY: retire the registration's one strong reference only after
        // the original procedure returns; this callback still retains its own.
        unsafe {
            drop(Rc::from_raw(pointer));
        }
    }
    result
}

unsafe extern "system" fn cover_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_ERASEBKGND {
        let mut rect = RECT::default();
        // SAFETY: paint only our cover client with the class's opaque brush.
        let success = unsafe { GetClientRect(hwnd, &mut rect) != 0 && FillRect(wparam as _, &rect, GetStockObject(BLACK_BRUSH) as _) != 0 };
        return isize::from(success);
    }
    if message == WM_PAINT {
        let mut paint = PAINTSTRUCT::default();
        // SAFETY: standard balanced painting for this process's guard class.
        unsafe {
            let dc = BeginPaint(hwnd, &mut paint);
            let filled = !dc.is_null() && FillRect(dc, &paint.rcPaint, GetStockObject(BLACK_BRUSH) as _) != 0;
            let ended = EndPaint(hwnd, &paint) != 0;
            if filled && ended {
                SetPropW(hwnd, PAINTED.as_ptr(), 1usize as _);
            } else {
                RemovePropW(hwnd, PAINTED.as_ptr());
            }
        }
        return 0;
    }
    // SAFETY: default native processing of this owned guard's messages.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let state = self.0.clone();
        // Retain opaque protection and the registration reference while a
        // worker can still be visible. WM_NCDESTROY finally retires it. Dropping
        // a Rust owner/error path is not an authenticated hide receipt.
        if (state.worker.get().is_some() || state.targets.get().is_some()) && state.verify_host().is_ok() {
            let _ = state.arm();
            return;
        }
        // SAFETY: unhook only our exact registration. Active callbacks and
        // operations retain independent Rcs, so synchronous reentry cannot
        // free their State. On failed unhook leave the registration alive.
        unsafe {
            if state.attached.get()
                && state.verify_host().is_ok()
                && RemoveWindowSubclass(state.host.hwnd as HWND, Some(host_proc), SUBCLASS_ID) != 0
            {
                state.attached.set(false);
                drop(Rc::from_raw(Rc::as_ptr(&state)));
                RemovePropW(state.host.hwnd as HWND, MARKER.as_ptr());
                RemovePropW(state.host.hwnd as HWND, EPOCH.as_ptr());
                RemovePropW(state.host.hwnd as HWND, GENERATION.as_ptr());
                RemovePropW(state.host.hwnd as HWND, OLD_COVER.as_ptr());
                RemovePropW(state.host.hwnd as HWND, TARGET_COVER.as_ptr());
                RemovePropW(state.host.hwnd as HWND, BLOCKED.as_ptr());
            }
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        for cover in self.covers {
            if !cover.is_null() {
                // SAFETY: guard HWNDs were created on this process/thread.
                unsafe {
                    DestroyWindow(cover);
                }
            }
        }
    }
}
