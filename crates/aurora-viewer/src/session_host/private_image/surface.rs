//! Private owner ordering. Only the dormant test module can construct this path.
//! The future Shell supplies delivered-edit flush and actual source authority.

use super::{Error, PrivateFrame, PrivateImageSlot, UiPixelBudget, UiPixelCredit};
#[cfg(windows)]
use egui::Color32;
use egui::{ColorImage, Context, FullOutput, RawInput, TextureId};
use std::time::Duration;

pub(super) trait OwnedSurface {
    fn size(&self) -> [u32; 2];
    fn input(&mut self) -> RawInput;
    fn hide(&mut self);
    fn paint(
        &mut self,
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
}
impl<S: OwnedSurface> PrivateSoftwareSurface<S> {
    pub(super) fn new(native: S, slot: PrivateImageSlot) -> Self {
        let budget = slot.budget.clone();
        Self {
            native: Some(native),
            slot,
            budget,
            bitmap: None,
        }
    }
    pub(super) fn retire(&mut self, flush: &mut dyn FnMut(&Context, RawInput)) {
        self.slot.retire();
        if let Some(mut native) = self.native.take() {
            // The parent validates old captured chat authority and applies only
            // delivered old-focused edits in Closing mode, never Send/Read.
            native.hide();
            flush(&self.slot.context, native.input());
            drop(native); // Actual Surface/DIB before its physical byte credit.
        }
        self.bitmap = None;
    }
    pub(super) fn present(
        &mut self,
        context: &Context,
        frame: PrivateFrame,
        current: &mut dyn FnMut() -> bool,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<Duration, Error> {
        let (ticket, mut output) = frame.into_parts();
        // Old callbacks/output cannot tear down an unrelated current surface.
        if !self.slot.accepts(context, &ticket) {
            // Rejected first-pass geometry must not lose its font upload while
            // egui's manager believes that atlas was delivered. Private User
            // uploads never enter this mailbox; foreign contexts are discarded.
            if *context == self.slot.context
                && ticket.context == self.slot.context
                && self.native.is_some()
                && self.slot.binding != 0
                && self.slot.check_delta(&output.textures_delta).is_ok()
            {
                self.slot.pending_ui.append(std::mem::take(&mut output.textures_delta));
            }
            output.textures_delta.clear();
            return Err(Error::Stale);
        }
        if let Err(error) = self.slot.validate_frame(context, &ticket, &output.textures_delta, current) {
            output.textures_delta.clear();
            self.retire(flush);
            return Err(error);
        }
        self.slot.consume(context, ticket)?;
        let Some(native) = &mut self.native else {
            output.textures_delta.clear();
            return Err(Error::Surface);
        };
        let size = native.size();
        if size.contains(&0) {
            output.textures_delta.clear();
            self.slot.retire();
            native.hide();
            flush(&self.slot.context, native.input());
            // No resize: the old DIB still exists and remains fully charged.
            // Restoration requires retiring/recreating this private owner.
            return Ok(Duration::from_millis(250));
        }
        let replacement = if self.bitmap.as_ref().is_some_and(|(old, _)| *old == size) {
            None
        } else {
            let bytes = bitmap_bytes(size);
            match bytes.and_then(|bytes| self.budget.reserve(bytes)) {
                Ok(credit) => Some((size, credit)),
                Err(error) => {
                    output.textures_delta.clear();
                    self.retire(flush);
                    return Err(error);
                }
            }
        };
        self.slot.pending_ui.append(std::mem::take(&mut output.textures_delta));
        output.textures_delta = std::mem::take(&mut self.slot.pending_ui);
        let result = native.paint(context, output, size, self.slot.borrowed(), current);
        match result {
            Ok(delay) => {
                if let Some(replacement) = replacement {
                    self.bitmap = Some(replacement);
                }
                Ok(delay)
            }
            Err(error) => {
                // replacement credit remains live until after native is dropped.
                self.retire(flush);
                drop(replacement);
                Err(error)
            }
        }
    }
}
impl<S: OwnedSurface> Drop for PrivateSoftwareSurface<S> {
    fn drop(&mut self) {
        self.slot.retire();
        if let Some(mut native) = self.native.take() {
            native.hide();
            drop(native);
        }
        self.bitmap = None;
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
pub(super) struct WindowsSurface<'a> {
    native: super::super::software::SoftwareWindow,
    event_loop: &'a winit::event_loop::ActiveEventLoop,
    background: Color32,
}
#[cfg(windows)]
impl<'a> WindowsSurface<'a> {
    pub(super) fn owned_tool(
        event_loop: &'a winit::event_loop::ActiveEventLoop,
        context: &Context,
        owner: &winit::window::Window,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            native: super::super::software::SoftwareWindow::owned_tool(
                event_loop,
                context,
                owner,
                String::new(),
                [380.0, 290.0],
                [280.0, 120.0],
            )?,
            event_loop,
            background: Color32::BLACK,
        })
    }
}
#[cfg(windows)]
impl OwnedSurface for WindowsSurface<'_> {
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
        context: &Context,
        output: FullOutput,
        size: [u32; 2],
        private: Option<(TextureId, &ColorImage)>,
        current: &mut dyn FnMut() -> bool,
    ) -> Result<Duration, Error> {
        self.native
            .present_private(self.event_loop, context, output, self.background, size, private, current)
            .map_err(|_| Error::Surface)
    }
}
