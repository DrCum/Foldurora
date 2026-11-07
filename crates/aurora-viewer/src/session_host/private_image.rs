//! Dormant private CPU image ownership. This is a pixel-payload bound, excluding
//! ordinary fonts, allocator metadata and window-system/compositor storage.
//! No private bytes enter egui's texture manager or the ordinary Painter cache.

mod surface;
#[cfg(test)]
mod tests;

use egui::{Color32, ColorImage, Context, TextureId};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const UI_PIXEL_BYTES: usize = 24 * 1024 * 1024;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Error {
    Invalid,
    Capacity,
    Allocation,
    Exhausted,
    Stale,
    Collision,
    Surface,
}
fn fresh() -> Result<u64, Error> {
    NEXT_ID
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| Error::Exhausted)
}

#[derive(Clone)]
pub(super) struct UiPixelBudget(Arc<Budget>);
struct Budget {
    limit: usize,
    used: AtomicUsize,
}
struct UiPixelCredit {
    budget: UiPixelBudget,
    bytes: usize,
}
impl UiPixelBudget {
    /// Construct once at the host owner, then inject clones into all surfaces.
    pub(super) fn new() -> Self {
        Self(Arc::new(Budget {
            limit: UI_PIXEL_BYTES,
            used: AtomicUsize::new(0),
        }))
    }
    fn reserve(&self, bytes: usize) -> Result<UiPixelCredit, Error> {
        if bytes == 0 {
            return Err(Error::Invalid);
        }
        self.0
            .used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.0.limit)
            })
            .map_err(|_| Error::Capacity)?;
        Ok(UiPixelCredit {
            budget: self.clone(),
            bytes,
        })
    }
    pub(super) fn used(&self) -> usize {
        self.0.used.load(Ordering::Acquire)
    }
}
impl Drop for UiPixelCredit {
    fn drop(&mut self) {
        self.budget.0.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

struct Image {
    image: ColorImage,
    _credit: UiPixelCredit,
}
#[derive(Clone)]
pub(super) struct PrivateImage(Arc<Image>);
impl PrivateImage {
    /// Call only within the image service's guarded pixel callback. This factory
    /// checks storage, not source permission; the owner revalidates before paint.
    pub(super) fn from_rgba(budget: &UiPixelBudget, size: [usize; 2], rgba: &[u8]) -> Result<Self, Error> {
        if size.contains(&0) || size[0] > 640 || size[1] > 360 {
            return Err(Error::Invalid);
        }
        let count = size[0].checked_mul(size[1]).ok_or(Error::Invalid)?;
        let bytes = count.checked_mul(size_of::<Color32>()).ok_or(Error::Invalid)?;
        if size_of::<Color32>() != 4 || rgba.len() != bytes || rgba.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 255) {
            return Err(Error::Invalid);
        }
        let credit = budget.reserve(bytes)?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(count).map_err(|_| Error::Allocation)?;
        // Refuse a different reported Vec capacity before copying private bytes.
        // Allocator rounding outside Vec's reported storage is not this bound.
        if pixels.capacity() != count {
            return Err(Error::Allocation);
        }
        pixels.extend(rgba.as_chunks::<4>().0.iter().map(|p| Color32::from_rgb(p[0], p[1], p[2])));
        Ok(Self(Arc::new(Image {
            image: ColorImage::new(size, pixels),
            _credit: credit,
        })))
    }
}

/// No pixels escape through the UI-facing capability. It borrows the owner, so
/// the body cannot keep it across a slot mutation or surface retirement.
pub(super) struct PrivateTextureView<'a> {
    slot: &'a PrivateImageSlot,
}
impl PrivateTextureView<'_> {
    pub(super) fn context(&self) -> &Context {
        &self.slot.context
    }
    pub(super) fn texture_id(&self) -> TextureId {
        self.slot.texture
    }
    pub(super) fn size(&self) -> [usize; 2] {
        self.slot.current.as_ref().map_or([0, 0], |image| image.0.image.size)
    }
    pub(super) fn generation(&self) -> u64 {
        self.slot.generation
    }
    pub(super) fn binding_generation(&self) -> u64 {
        self.slot.binding
    }
}
/// Move-only, single-presentation local capability. It grants no source access.
struct PrivateFrameTicket {
    context: Context,
    owner: u64,
    binding: u64,
    image: u64,
    frame: u64,
}
/// Output and its local ticket are created together; callers cannot combine
/// stale already-built shapes with a fresh presentation ticket.
pub(super) struct PrivateFrame {
    ticket: PrivateFrameTicket,
    output: egui::FullOutput,
}
impl PrivateFrame {
    fn into_parts(mut self) -> (PrivateFrameTicket, egui::FullOutput) {
        // This sealed move-only frame is consumed once; the copied local fields
        // cannot be used after this method returns. Context clones carry no pixels.
        let ticket = PrivateFrameTicket {
            context: self.ticket.context.clone(),
            owner: self.ticket.owner,
            binding: self.ticket.binding,
            image: self.ticket.image,
            frame: self.ticket.frame,
        };
        (ticket, std::mem::take(&mut self.output))
    }
}
impl Drop for PrivateFrame {
    fn drop(&mut self) {
        // An abandoned/discarded UI pass owns no private pixels. Dispose any
        // ordinary font delta explicitly rather than triggering egui's debug Drop.
        self.output.textures_delta.clear();
    }
}
pub(super) struct PrivateImageSlot {
    budget: UiPixelBudget,
    context: Context,
    generation: u64,
    binding: u64,
    texture: TextureId,
    revision: u64,
    frame: Option<u64>,
    current: Option<PrivateImage>,
    pending: Option<PrivateImage>,
    pending_ui: egui::TexturesDelta,
}
impl PrivateImageSlot {
    pub(super) fn new(context: Context, binding: u64, budget: UiPixelBudget) -> Result<Self, Error> {
        if binding == 0 {
            return Err(Error::Invalid);
        }
        Ok(Self {
            budget,
            context,
            generation: fresh()?,
            binding,
            texture: TextureId::User(fresh()?),
            revision: fresh()?,
            frame: None,
            current: None,
            pending: None,
            pending_ui: Default::default(),
        })
    }
    pub(super) fn stage(&mut self, binding: u64, image: PrivateImage) -> Result<(), Error> {
        if binding != self.binding || binding == 0 {
            return Err(Error::Stale);
        }
        if !Arc::ptr_eq(&self.budget.0, &image.0._credit.budget.0) {
            return Err(Error::Invalid);
        }
        let revision = fresh()?;
        self.pending = Some(image);
        self.revision = revision;
        self.frame = None;
        Ok(())
    }
    fn begin_frame(&mut self) -> Result<(PrivateTextureView<'_>, PrivateFrameTicket), Error> {
        if self.binding == 0 {
            return Err(Error::Stale);
        }
        if let Some(image) = self.pending.take() {
            self.current = Some(image);
        }
        let frame = fresh()?;
        self.frame = Some(frame);
        let ticket = PrivateFrameTicket {
            context: self.context.clone(),
            owner: self.generation,
            binding: self.binding,
            image: self.revision,
            frame,
        };
        Ok((PrivateTextureView { slot: self }, ticket))
    }
    pub(super) fn draw(
        &mut self,
        raw: egui::RawInput,
        mut paint: impl FnMut(&mut egui::Ui, &PrivateTextureView<'_>),
    ) -> Result<PrivateFrame, Error> {
        let (view, ticket) = self.begin_frame()?;
        let mut output = view.context().run_ui(raw, |ui| paint(ui, &view));
        if let Err(error) = self.check_delta(&output.textures_delta) {
            output.textures_delta.clear();
            return Err(error);
        }
        // Keep ordinary atlas uploads with the context even if this frame is
        // abandoned. FullOutput and its local ticket never retain private bytes.
        self.pending_ui.append(std::mem::take(&mut output.textures_delta));
        Ok(PrivateFrame { ticket, output })
    }
    fn accepts(&self, context: &Context, ticket: &PrivateFrameTicket) -> bool {
        *context == self.context
            && ticket.context == self.context
            && ticket.owner == self.generation
            && ticket.binding == self.binding
            && ticket.binding != 0
            && ticket.image == self.revision
            && self.frame == Some(ticket.frame)
    }
    fn consume(&mut self, context: &Context, ticket: PrivateFrameTicket) -> Result<(), Error> {
        if !self.accepts(context, &ticket) {
            return Err(Error::Stale);
        }
        self.frame = None;
        Ok(())
    }
    pub(super) fn retire(&mut self) {
        self.binding = 0;
        self.frame = None;
        self.current = None;
        self.pending = None;
        self.pending_ui.clear();
    }
    fn borrowed(&self) -> Option<(TextureId, &ColorImage)> {
        self.current.as_ref().map(|image| (self.texture, &image.0.image))
    }
    fn check_delta(&self, delta: &egui::TexturesDelta) -> Result<(), Error> {
        if !matches!(self.texture, TextureId::User(_))
            || delta.set.keys().chain(delta.free.iter()).any(|id| matches!(id, TextureId::User(_)))
        {
            return Err(Error::Collision);
        }
        Ok(())
    }
}

impl Drop for PrivateImageSlot {
    fn drop(&mut self) {
        self.pending_ui.clear();
    }
}
