//! Private owner ordering. Only the dormant test module can construct this path.
//! The future Shell supplies delivered-edit flush and actual source authority.

use super::{Error, PrivateFrame, PrivateImageSlot, UiPixelBudget, UiPixelCredit};
#[cfg(windows)]
use egui::Color32;
use egui::{ColorImage, Context, FullOutput, RawInput, TextureId};
use std::time::Duration;

pub(super) trait OwnedSurface {
    type PresentContext: ?Sized;
    fn minimized(&self) -> bool;
    fn size(&self) -> [u32; 2];
    fn input(&mut self) -> RawInput;
    fn hide(&mut self);
    fn paint(
        &mut self,
        presentation: &Self::PresentContext,
        context: &Context,
        output: FullOutput,
        size: [u32; 2],
        private: Option<(TextureId, &ColorImage)>,
        current: &mut dyn FnMut() -> bool,
    ) -> Result<Duration, Error>;
}

pub(super) struct PrivateSoftwareSurface<S: OwnedSurface> {
    pub(super) native: Option<S>,
    pub(super) slot: PrivateImageSlot,
    budget: UiPixelBudget,
    bitmap: Option<([u32; 2], UiPixelCredit)>,
    replacement: Option<([u32; 2], UiPixelCredit)>,
}

// Field destruction order is deliberate: the actual native bitmap is destroyed
// before either physical reservation, including if hide/input/flush unwinds.
struct Retiring<S> {
    native: Option<S>,
    _bitmap: Option<([u32; 2], UiPixelCredit)>,
    _replacement: Option<([u32; 2], UiPixelCredit)>,
}
struct PendingOutput(FullOutput);
impl Drop for PendingOutput {
    fn drop(&mut self) {
        self.0.textures_delta.clear();
    }
}
impl<S: OwnedSurface> PrivateSoftwareSurface<S> {
    pub(super) fn new(native: S, slot: PrivateImageSlot) -> Self {
        let budget = slot.budget.clone();
        Self {
            native: Some(native),
            slot,
            budget,
            bitmap: None,
            replacement: None,
        }
    }
    fn take_retiring(&mut self) -> Retiring<S> {
        Retiring {
            native: self.native.take(),
            _bitmap: self.bitmap.take(),
            _replacement: self.replacement.take(),
        }
    }
    pub(super) fn retire(&mut self, flush: &mut dyn FnMut(&Context, RawInput)) {
        let mut retiring = self.take_retiring();
        self.slot.retire();
        if let Some(native) = &mut retiring.native {
            native.hide();
            // The parent validates old captured chat authority and applies only
            // delivered old-focused edits in Closing mode, never Send/Read.
            flush(&self.slot.context, native.input());
        }
        // Retiring also enforces this order if any callback above panics.
        drop(retiring);
    }
    pub(super) fn present(
        &mut self,
        presentation: &S::PresentContext,
        context: &Context,
        frame: PrivateFrame,
        current: &mut dyn FnMut() -> bool,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<Duration, Error> {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.present_inner(presentation, context, frame, current, flush)
        }));
        match result {
            Ok(result) => result,
            Err(panic) => {
                // No conversion to an ordinary error: the original failure is
                // resumed after removing all native/pixel ownership from self.
                self.retire(flush);
                std::panic::resume_unwind(panic)
            }
        }
    }
    fn present_inner(
        &mut self,
        presentation: &S::PresentContext,
        context: &Context,
        frame: PrivateFrame,
        current: &mut dyn FnMut() -> bool,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<Duration, Error> {
        let (ticket, output) = frame.into_parts();
        let mut output = PendingOutput(output);
        // Old callbacks/output cannot tear down an unrelated current surface.
        if !self.slot.accepts(context, &ticket) {
            // Rejected first-pass geometry must not lose its font upload while
            // egui's manager believes that atlas was delivered. Private User
            // uploads never enter this mailbox; foreign contexts are discarded.
            if *context == self.slot.context
                && ticket.context == self.slot.context
                && self.native.is_some()
                && self.slot.binding != 0
                && self.slot.check_delta(&output.0.textures_delta).is_ok()
            {
                self.slot.pending_ui.append(std::mem::take(&mut output.0.textures_delta));
            }
            output.0.textures_delta.clear();
            return Err(Error::Stale);
        }
        if let Err(error) = self.slot.validate_frame(context, &ticket, &output.0.textures_delta, current) {
            output.0.textures_delta.clear();
            self.retire(flush);
            return Err(error);
        }
        self.slot.consume(context, ticket)?;
        let Some(native) = &mut self.native else {
            output.0.textures_delta.clear();
            return Err(Error::Surface);
        };
        let size = if native.minimized() { [0, 0] } else { native.size() };
        if size.contains(&0) {
            output.0.textures_delta.clear();
            self.slot.retire();
            native.hide();
            flush(&self.slot.context, native.input());
            // No resize: the old DIB still exists and remains fully charged.
            // Restoration requires retiring/recreating this private owner.
            return Ok(Duration::from_millis(250));
        }
        self.replacement = if self.bitmap.as_ref().is_some_and(|(old, _)| *old == size) {
            None
        } else {
            let bytes = bitmap_bytes(size);
            match bytes.and_then(|bytes| self.budget.reserve(bytes)) {
                Ok(credit) => Some((size, credit)),
                Err(error) => {
                    output.0.textures_delta.clear();
                    self.retire(flush);
                    return Err(error);
                }
            }
        };
        self.slot.pending_ui.append(std::mem::take(&mut output.0.textures_delta));
        output.0.textures_delta = std::mem::take(&mut self.slot.pending_ui);
        let result = native.paint(
            presentation,
            context,
            std::mem::take(&mut output.0),
            size,
            self.slot.borrowed(),
            current,
        );
        match result {
            Ok(delay) => {
                if let Some(replacement) = self.replacement.take() {
                    self.bitmap = Some(replacement);
                }
                Ok(delay)
            }
            Err(error) => {
                // Both owner-held reservations survive until native Drop.
                self.retire(flush);
                Err(error)
            }
        }
    }
}
impl<S: OwnedSurface> Drop for PrivateSoftwareSurface<S> {
    fn drop(&mut self) {
        let mut retiring = self.take_retiring();
        self.slot.retire();
        if let Some(native) = &mut retiring.native {
            native.hide();
        }
        drop(retiring);
    }
}
pub(super) fn bitmap_bytes(size: [u32; 2]) -> Result<usize, Error> {
    if size.contains(&0) || size.into_iter().any(|edge| i32::try_from(edge).is_err()) {
        return Err(Error::Invalid);
    }
    usize::try_from(size[0])
        .ok()
        .and_then(|w| w.checked_mul(usize::try_from(size[1]).ok()?))
        .and_then(|pixels| pixels.checked_mul(size_of::<u32>()))
        .ok_or(Error::Invalid)
}

// This adapter owns SoftwareWindow; it never exposes its Arc<Window>. The module
// is cfg(test), and construction is not wired to Shell or any image endpoint.
#[cfg(windows)]
pub(super) struct WindowsSurface {
    native: super::super::software::SoftwareWindow,
    background: Color32,
}
#[cfg(windows)]
impl WindowsSurface {
    pub(super) fn owned_tool(
        event_loop: &winit::event_loop::ActiveEventLoop,
        context: &Context,
        owner: &winit::window::Window,
    ) -> anyhow::Result<Self> {
        Self::configured_tool(
            event_loop,
            context,
            owner,
            super::ToolOptions {
                title: String::new(),
                initial: [380.0, 290.0],
                minimum: [280.0, 120.0],
                background: Color32::BLACK,
            },
        )
    }
    fn owned_hwnd(&self) -> Result<windows_sys::Win32::Foundation::HWND, Error> {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let RawWindowHandle::Win32(handle) = self.native.window.window_handle().map_err(|_| Error::Surface)?.as_raw() else {
            return Err(Error::Surface);
        };
        Ok(handle.hwnd.get() as windows_sys::Win32::Foundation::HWND)
    }
    pub(super) fn configured_tool(
        event_loop: &winit::event_loop::ActiveEventLoop,
        context: &Context,
        owner: &winit::window::Window,
        options: super::ToolOptions,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            native: super::super::software::SoftwareWindow::owned_tool(
                event_loop,
                context,
                owner,
                options.title,
                options.initial,
                options.minimum,
            )?,
            background: options.background,
        })
    }
}
#[cfg(windows)]
impl OwnedSurface for WindowsSurface {
    type PresentContext = winit::event_loop::ActiveEventLoop;
    fn minimized(&self) -> bool {
        self.native.window.is_minimized() != Some(false)
    }
    fn size(&self) -> [u32; 2] {
        let size = self.native.window.inner_size();
        [size.width, size.height]
    }
    fn input(&mut self) -> RawInput {
        self.native.raw_input()
    }
    fn hide(&mut self) {
        self.native.window.set_visible(false);
    }
    fn paint(
        &mut self,
        presentation: &Self::PresentContext,
        context: &Context,
        output: FullOutput,
        size: [u32; 2],
        private: Option<(TextureId, &ColorImage)>,
        current: &mut dyn FnMut() -> bool,
    ) -> Result<Duration, Error> {
        self.native
            .present_private(presentation, context, output, self.background, size, private, current)
            .map_err(|_| Error::Surface)
    }
}

#[cfg(windows)]
impl super::tool::ToolNative for WindowsSurface {
    type Id = winit::window::WindowId;
    type Event = winit::event::WindowEvent;
    type Response = egui_winit::EventResponse;
    fn id(&self) -> Self::Id {
        self.native.window.id()
    }
    fn event(&mut self, _context: &Self::PresentContext, event: &Self::Event) -> Result<Self::Response, Error> {
        Ok(self.native.event(event))
    }
    fn show(&mut self, _context: &Self::PresentContext) -> Result<(), Error> {
        if self.native.window.is_minimized() != Some(false) {
            return Err(Error::Stale);
        }
        self.native.window.set_visible(true);
        if self.native.window.is_minimized() != Some(false) {
            return Err(Error::Stale);
        }
        Ok(())
    }
    fn request_redraw(&mut self, _context: &Self::PresentContext) -> Result<(), Error> {
        self.native.window.request_redraw();
        Ok(())
    }
    fn focused(&self, _context: &Self::PresentContext) -> Result<bool, Error> {
        Ok(self.native.window.has_focus())
    }
    fn focus(&mut self, _context: &Self::PresentContext) -> Result<(), Error> {
        self.native.window.focus_window();
        Ok(())
    }
    fn set_title(&mut self, _context: &Self::PresentContext, title: &str) -> Result<(), Error> {
        self.native.window.set_title(title);
        Ok(())
    }
    fn pin(&mut self, _context: &Self::PresentContext, pinned: bool) -> Result<bool, Error> {
        use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetWindowLongPtrW, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
            SetWindowPos, WS_EX_TOPMOST,
        };
        let hwnd = self.owned_hwnd()?;
        // SAFETY: only this live owned tool HWND is changed on its GUI thread;
        // NOACTIVATE preserves current focus. No foreign/native worker is used.
        unsafe {
            if SetWindowPos(
                hwnd,
                if pinned { HWND_TOPMOST } else { HWND_NOTOPMOST },
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOOWNERZORDER,
            ) == 0
            {
                return Err(Error::Surface);
            }
            SetLastError(0);
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            if style == 0 && GetLastError() != 0 {
                return Err(Error::Surface);
            }
            Ok(style as u32 & WS_EX_TOPMOST != 0)
        }
    }
    fn placement(&self, _context: &Self::PresentContext) -> Result<super::Placement, Error> {
        let pos = self.native.window.outer_position().map_err(|_| Error::Surface)?;
        let size = self.native.window.inner_size();
        let hwnd = self.owned_hwnd()?;
        // SAFETY: this owned live tool is queried on its GUI thread. IsZoomed
        // reads actual native state, independent of winit's requested flags.
        let maximized = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsZoomed(hwnd) != 0 };
        Ok(super::Placement {
            outer_position: [pos.x, pos.y],
            client_size: [size.width, size.height],
            maximized,
        })
    }
    fn request_placement(&mut self, _context: &Self::PresentContext, placement: super::Placement) -> Result<(), Error> {
        self.native.window.set_maximized(false);
        self.native.window.set_outer_position(winit::dpi::PhysicalPosition::new(
            placement.outer_position[0],
            placement.outer_position[1],
        ));
        let _ = self
            .native
            .window
            .request_inner_size(winit::dpi::PhysicalSize::new(placement.client_size[0], placement.client_size[1]));
        if placement.maximized {
            self.native.window.set_maximized(true);
        }
        Ok(())
    }
}
