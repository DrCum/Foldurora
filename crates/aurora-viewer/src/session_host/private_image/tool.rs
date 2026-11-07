//! Sealed tool-window facade. No native Window/Arc, pixel slice or reconstructed
//! frame ticket escapes. Parent still owns source/service/native permission.
use super::surface::{OwnedSurface, PrivateSoftwareSurface};
use super::{Error, PrivateFrame, PrivateImage, PrivateImageSlot, PrivateTextureView};
use egui::{Color32, Context, RawInput};
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::Duration;

/// Title is already localized by the caller; these are logical window units.
#[derive(Clone, Debug)]
pub(in crate::session_host) struct ToolOptions {
    pub title: String,
    pub initial: [f64; 2],
    pub minimum: [f64; 2],
    pub background: Color32,
}
impl ToolOptions {
    fn validate(&self) -> Result<(), Error> {
        if self.title.len() > 4096
            || self.title.chars().any(char::is_control)
            || self
                .initial
                .into_iter()
                .chain(self.minimum)
                .any(|side| !side.is_finite() || side < 1.0 || side > f64::from(i32::MAX))
            || (0..2).any(|side| self.minimum[side] > self.initial[side])
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

/// Actual physical client size and outer position, never a source identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::session_host) struct Placement {
    pub outer_position: [i32; 2],
    pub client_size: [u32; 2],
    pub maximized: bool,
}
impl Placement {
    fn validate(self) -> Result<(), Error> {
        super::surface::bitmap_bytes(self.client_size).map(|_| ())
    }
}

// Kept private to private_image; fake/native adapters share the exact facade.
pub(super) trait ToolNative: OwnedSurface {
    type Id: Copy + Eq;
    type Event;
    type Response;
    fn id(&self) -> Self::Id;
    fn event(&mut self, context: &Self::PresentContext, event: &Self::Event) -> Result<Self::Response, Error>;
    fn show(&mut self, context: &Self::PresentContext) -> Result<(), Error>;
    fn request_redraw(&mut self, context: &Self::PresentContext) -> Result<(), Error>;
    fn focused(&self, context: &Self::PresentContext) -> Result<bool, Error>;
    fn focus(&mut self, context: &Self::PresentContext) -> Result<(), Error>;
    fn set_title(&mut self, context: &Self::PresentContext, title: &str) -> Result<(), Error>;
    fn pin(&mut self, context: &Self::PresentContext, pinned: bool) -> Result<bool, Error>;
    fn placement(&self, context: &Self::PresentContext) -> Result<Placement, Error>;
    fn request_placement(&mut self, context: &Self::PresentContext, placement: Placement) -> Result<(), Error>;
}
struct Tool<S: ToolNative> {
    owner: PrivateSoftwareSurface<S>,
    id: S::Id,
    presented: bool,
    // Native operation and destruction stay on the owning GUI thread.
    _thread: PhantomData<Rc<()>>,
}
impl<S: ToolNative> Tool<S> {
    fn new(native: S, slot: PrivateImageSlot) -> Self {
        let id = native.id();
        Self {
            owner: PrivateSoftwareSurface::new(native, slot),
            id,
            presented: false,
            _thread: PhantomData,
        }
    }
    fn live(&self) -> bool {
        self.owner.native.is_some() && self.owner.slot.binding != 0
    }
    fn operation<R>(
        &mut self,
        ready: bool,
        flush: &mut dyn FnMut(&Context, RawInput),
        operation: impl FnOnce(&mut S) -> Result<R, Error>,
    ) -> Result<R, Error> {
        if !self.live() || (ready && !self.presented) {
            return Err(Error::Stale);
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            operation(self.owner.native.as_mut().expect("live native owner"))
        }));
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => {
                self.retire(flush);
                Err(error)
            }
            Err(panic) => {
                self.retire(flush);
                std::panic::resume_unwind(panic)
            }
        }
    }
    fn event(
        &mut self,
        presentation: &S::PresentContext,
        id: S::Id,
        event: &S::Event,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<Option<S::Response>, Error> {
        if id != self.id {
            return Ok(None);
        }
        self.operation(false, flush, |native| native.event(presentation, event)).map(Some)
    }
    fn request_redraw(&mut self, presentation: &S::PresentContext, flush: &mut dyn FnMut(&Context, RawInput)) -> Result<(), Error> {
        self.operation(false, flush, |native| native.request_redraw(presentation))
    }
    fn raw_input(&mut self, flush: &mut dyn FnMut(&Context, RawInput)) -> Result<RawInput, Error> {
        self.operation(false, flush, |native| Ok(native.input()))
    }
    fn minimized(&mut self, flush: &mut dyn FnMut(&Context, RawInput)) -> Result<bool, Error> {
        self.operation(false, flush, |native| Ok(native.minimized()))
    }
    fn focused(&mut self, presentation: &S::PresentContext, flush: &mut dyn FnMut(&Context, RawInput)) -> Result<bool, Error> {
        self.operation(false, flush, |native| native.focused(presentation))
    }
    fn focus(&mut self, presentation: &S::PresentContext, flush: &mut dyn FnMut(&Context, RawInput)) -> Result<(), Error> {
        self.operation(true, flush, |native| {
            if native.minimized() {
                return Err(Error::Stale);
            }
            native.focus(presentation)
        })
    }
    fn set_title(&mut self, presentation: &S::PresentContext, title: &str, flush: &mut dyn FnMut(&Context, RawInput)) -> Result<(), Error> {
        if title.len() > 4096 || title.chars().any(char::is_control) {
            return Err(Error::Invalid);
        }
        self.operation(false, flush, |native| native.set_title(presentation, title))
    }
    fn pin(&mut self, presentation: &S::PresentContext, pinned: bool, flush: &mut dyn FnMut(&Context, RawInput)) -> Result<bool, Error> {
        self.operation(true, flush, |native| native.pin(presentation, pinned))
    }
    fn placement(&mut self, presentation: &S::PresentContext, flush: &mut dyn FnMut(&Context, RawInput)) -> Result<Placement, Error> {
        self.operation(false, flush, |native| {
            if native.minimized() {
                return Err(Error::Stale);
            }
            native.placement(presentation)
        })
    }
    fn request_placement(
        &mut self,
        presentation: &S::PresentContext,
        placement: Placement,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<(), Error> {
        placement.validate()?;
        // winit's maximize/restore and even placement paths may call ShowWindow.
        // They cannot run before the first guarded frame or revive minimization.
        self.operation(true, flush, |native| {
            if native.minimized() {
                return Err(Error::Stale);
            }
            native.request_placement(presentation, placement)
        })
    }
    fn stage(&mut self, binding: u64, image: PrivateImage) -> Result<(), Error> {
        if !self.live() {
            return Err(Error::Stale);
        }
        self.owner.slot.stage(binding, image)
    }
    fn draw(
        &mut self,
        raw: RawInput,
        paint: impl FnMut(&mut egui::Ui, &PrivateTextureView<'_>),
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<PrivateFrame, Error> {
        if !self.live() {
            return Err(Error::Stale);
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.owner.slot.draw(raw, paint)));
        match result {
            Ok(Ok(frame)) => Ok(frame),
            Ok(Err(error)) => {
                self.retire(flush);
                Err(error)
            }
            Err(panic) => {
                self.retire(flush);
                std::panic::resume_unwind(panic)
            }
        }
    }
    fn present(
        &mut self,
        presentation: &S::PresentContext,
        context: &Context,
        frame: PrivateFrame,
        current: &mut dyn FnMut() -> bool,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<Duration, Error> {
        if !self.live() {
            return Err(Error::Stale);
        }
        // Keep the existing stale-output behavior: an old frame must not tear
        // down the newer valid owner. Its generic present alone decides that.
        let delay = self.owner.present(presentation, context, frame, current, flush)?;
        if !self.live() {
            self.presented = false;
            return Ok(delay);
        }
        if !self.presented {
            self.operation(false, flush, |native| {
                if !current() || native.minimized() {
                    return Err(Error::Stale);
                }
                native.show(presentation)?;
                if !current() || native.minimized() {
                    return Err(Error::Stale);
                }
                Ok(())
            })?;
            self.presented = true;
        }
        Ok(delay)
    }
    fn retire(&mut self, flush: &mut dyn FnMut(&Context, RawInput)) {
        self.presented = false;
        self.owner.retire(flush);
    }
}

/// Windows-only until another backend has an actual bitmap allocation bound.
#[cfg(windows)]
pub(in crate::session_host) struct MonitorSurface {
    tool: Tool<super::surface::WindowsSurface>,
}
#[cfg(windows)]
impl MonitorSurface {
    pub(in crate::session_host) fn owned_tool(
        event_loop: &winit::event_loop::ActiveEventLoop,
        context: &Context,
        owner: &winit::window::Window,
        options: ToolOptions,
        binding: u64,
        budget: super::UiPixelBudget,
    ) -> anyhow::Result<Self> {
        options
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid private tool options: {error:?}"))?;
        let slot = PrivateImageSlot::new(context.clone(), binding, budget)
            .map_err(|error| anyhow::anyhow!("private tool allocation: {error:?}"))?;
        let native = super::surface::WindowsSurface::configured_tool(event_loop, context, owner, options)?;
        Ok(Self {
            tool: Tool::new(native, slot),
        })
    }
    pub(in crate::session_host) fn id(&self) -> winit::window::WindowId {
        self.tool.id
    }
    /// Lifetime observation only; actual view/image permission remains external.
    pub(in crate::session_host) fn is_live(&self) -> bool {
        self.tool.live()
    }
    pub(in crate::session_host) fn event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        id: winit::window::WindowId,
        event: &winit::event::WindowEvent,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<Option<egui_winit::EventResponse>, Error> {
        self.tool.event(event_loop, id, event, flush)
    }
    pub(in crate::session_host) fn request_redraw(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<(), Error> {
        self.tool.request_redraw(event_loop, flush)
    }
    pub(in crate::session_host) fn raw_input(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<RawInput, Error> {
        self.tool.raw_input(flush)
    }
    pub(in crate::session_host) fn focused(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<bool, Error> {
        self.tool.focused(event_loop, flush)
    }
    pub(in crate::session_host) fn minimized(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<bool, Error> {
        self.tool.minimized(flush)
    }
    /// Never revives a minimized/retired owner; caller recreates it first.
    pub(in crate::session_host) fn focus(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<(), Error> {
        self.tool.focus(event_loop, flush)
    }
    pub(in crate::session_host) fn set_title(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        title: &str,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<(), Error> {
        self.tool.set_title(event_loop, title, flush)
    }
    /// Returns actual observed topmost state, not the requested hint.
    pub(in crate::session_host) fn pin(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        pinned: bool,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<bool, Error> {
        self.tool.pin(event_loop, pinned, flush)
    }
    pub(in crate::session_host) fn placement(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<Placement, Error> {
        self.tool.placement(event_loop, flush)
    }
    /// Request only, after a guarded first frame and while not minimized.
    /// Publish actual placement from a later query/native event.
    pub(in crate::session_host) fn request_placement(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        placement: Placement,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<(), Error> {
        self.tool.request_placement(event_loop, placement, flush)
    }
    pub(in crate::session_host) fn stage(&mut self, binding: u64, image: PrivateImage) -> Result<(), Error> {
        self.tool.stage(binding, image)
    }
    pub(in crate::session_host) fn draw(
        &mut self,
        raw: RawInput,
        paint: impl FnMut(&mut egui::Ui, &PrivateTextureView<'_>),
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<PrivateFrame, Error> {
        self.tool.draw(raw, paint, flush)
    }
    pub(in crate::session_host) fn present(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        context: &Context,
        frame: PrivateFrame,
        current: &mut dyn FnMut() -> bool,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<Duration, Error> {
        self.tool.present(event_loop, context, frame, current, flush)
    }
    pub(in crate::session_host) fn retire(&mut self, flush: &mut dyn FnMut(&Context, RawInput)) {
        self.tool.retire(flush)
    }
}

#[cfg(test)]
mod tests;
