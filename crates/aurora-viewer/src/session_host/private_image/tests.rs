use super::surface::{OwnedSurface, PrivateSoftwareSurface};
use super::*;
use egui::{FullOutput, RawInput};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

fn image(budget: &UiPixelBudget, color: [u8; 4]) -> PrivateImage {
    PrivateImage::from_rgba(budget, [2, 2], &color.repeat(4)).unwrap()
}
fn slot(budget: &UiPixelBudget) -> PrivateImageSlot {
    let mut slot = PrivateImageSlot::new(Context::default(), 1, budget.clone()).unwrap();
    slot.stage(1, image(budget, [255, 0, 0, 255])).unwrap();
    slot
}
fn frame(slot: &mut PrivateImageSlot) -> (Context, PrivateFrame) {
    let context = slot.context.clone();
    let frame = slot
        .draw(
            RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(4.0, 4.0))),
                ..Default::default()
            },
            |ui, view| {
                ui.painter().image(
                    view.texture_id(),
                    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(4.0, 4.0)),
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            },
        )
        .unwrap();
    (context, frame)
}

#[test]
fn private_cpu_images_share_one_pool_and_credit_follows_actual_allocation() {
    let budget = UiPixelBudget::new();
    let first = image(&budget, [12, 34, 56, 255]);
    assert_eq!(budget.used(), first.0.image.pixels.capacity() * size_of::<Color32>());
    let clone = first.clone();
    assert!(Arc::ptr_eq(&first.0, &clone.0));
    drop(first);
    assert_eq!(budget.used(), 16);
    drop(clone);
    assert_eq!(budget.used(), 0);
    // Five surface owners share a single allocation ceiling, not five pools.
    let mut credits = Vec::new();
    for owner in (0..5).map(|_| budget.clone()) {
        credits.push(owner.reserve(4 * 1024 * 1024).unwrap());
    }
    assert_eq!(budget.used(), 20 * 1024 * 1024);
    assert!(budget.reserve(5 * 1024 * 1024).is_err());
    assert!(budget.reserve(usize::MAX).is_err());
    assert!(budget.reserve(0).is_err());
    drop(credits);
    assert_eq!(budget.used(), 0);
    for (size, bytes) in [
        ([0, 1], vec![]),
        ([641, 1], vec![]),
        ([2, 2], vec![0; 15]),
        ([1, 1], vec![1, 2, 3, 0]),
    ] {
        assert!(PrivateImage::from_rgba(&budget, size, &bytes).is_err());
        assert_eq!(budget.used(), 0);
    }
}

#[test]
fn private_cpu_slot_coalesces_one_pending_and_old_callbacks_do_not_mutate_new_owner() {
    let budget = UiPixelBudget::new();
    let mut slot = slot(&budget);
    let (view, old) = slot.begin_frame().unwrap();
    assert_eq!(view.size(), [2, 2]);
    assert_eq!(view.binding_generation(), 1);
    let generation = view.generation();
    let id = view.texture_id();
    let context = view.context().clone();
    assert!(matches!(id, TextureId::User(_)));
    let abandoned = slot.draw(RawInput::default(), |_, _| {}).unwrap();
    drop(abandoned);
    assert!(slot.pending_ui.set.keys().all(|id| matches!(id, TextureId::Managed(_))));
    let foreign = UiPixelBudget::new();
    assert_eq!(slot.stage(1, image(&foreign, [0, 0, 0, 255])), Err(Error::Invalid));
    assert_eq!(foreign.used(), 0);
    assert_eq!(budget.used(), 16);
    slot.stage(1, image(&budget, [0, 255, 0, 255])).unwrap();
    assert_eq!(budget.used(), 32);
    slot.stage(1, image(&budget, [0, 0, 255, 255])).unwrap();
    assert_eq!(budget.used(), 32, "superseded pending allocation drops immediately");
    assert_eq!(slot.consume(&context, old), Err(Error::Stale));
    let (view, ticket) = slot.begin_frame().unwrap();
    assert_eq!(view.generation(), generation);
    assert_eq!(view.texture_id(), id);
    assert_eq!(budget.used(), 16);
    assert_eq!(slot.consume(&Context::default(), ticket), Err(Error::Stale));
    let (_, ticket) = slot.begin_frame().unwrap();
    slot.consume(&context, ticket).unwrap();
    assert!(slot.frame.is_none());
    slot.retire();
    assert_eq!(budget.used(), 0);
    assert!(slot.begin_frame().is_err());
    let replacement = PrivateImageSlot::new(context, 1, budget.clone()).unwrap();
    assert_ne!(replacement.generation, generation);
    assert_ne!(replacement.texture, id);
    assert!(slot.stage(1, image(&budget, [0, 0, 0, 255])).is_err());
    assert_eq!(budget.used(), 0);
}

#[test]
fn private_cpu_painter_borrows_real_pixels_without_cache_or_texture_manager_entries() {
    let budget = UiPixelBudget::new();
    let mut slot = slot(&budget);
    let (context, frame) = frame(&mut slot);
    assert!(!frame.output().shapes.is_empty());
    let (ticket, mut output) = frame.into_parts();
    assert!(!output.textures_delta.set.contains_key(&slot.texture));
    assert!(context.tex_manager().read().meta(slot.texture).is_none());
    output.textures_delta.append(std::mem::take(&mut slot.pending_ui));
    let primitives = context.tessellate(output.shapes, output.pixels_per_point);
    let mut painter = crate::crash::software::Painter::default();
    let pointer = slot.current.as_ref().unwrap().0.image.pixels.as_ptr();
    let mut pixels = vec![0; 16];
    assert!(painter.paint_with_private(
        &mut pixels,
        [4, 4],
        &primitives,
        &output.textures_delta,
        1.0,
        Color32::BLACK,
        slot.borrowed()
    ));
    assert!(pixels.iter().all(|pixel| *pixel == 0xff0000));
    assert_eq!(pointer, slot.current.as_ref().unwrap().0.image.pixels.as_ptr());
    assert_eq!(budget.used(), 16);
    output.textures_delta.clear();
    slot.consume(&context, ticket).unwrap();
    slot.retire();
    painter.paint(
        &mut pixels,
        [4, 4],
        &primitives,
        &egui::TexturesDelta::default(),
        1.0,
        Color32::BLACK,
    );
    assert!(pixels.iter().all(|pixel| *pixel == 0), "private image was never cached");
    assert_eq!(budget.used(), 0);
}

#[test]
fn private_cpu_texture_namespace_rejects_managed_and_delta_collisions() {
    let image = ColorImage::filled([1, 1], Color32::RED);
    let mut painter = crate::crash::software::Painter::default();
    let mut pixels = [0; 1];
    let mut delta = egui::TexturesDelta::default();
    let user = TextureId::User(123);
    delta.push(user, egui::epaint::ImageDelta::full(image.clone(), egui::TextureOptions::LINEAR));
    assert!(!painter.paint_with_private(&mut pixels, [1, 1], &[], &delta, 1.0, Color32::BLACK, Some((user, &image))));
    delta.clear();
    assert!(!painter.paint_with_private(
        &mut pixels,
        [1, 1],
        &[],
        &delta,
        1.0,
        Color32::BLACK,
        Some((TextureId::Managed(2), &image))
    ));
    delta.free(user);
    assert!(!painter.paint_with_private(&mut pixels, [1, 1], &[], &delta, 1.0, Color32::BLACK, Some((user, &image))));
    delta.clear();
}

#[derive(Default)]
struct Trace {
    events: Vec<&'static str>,
    observed: Vec<usize>,
    last_uniform_color: Option<u32>,
}
struct FakeSurface {
    size: [u32; 2],
    pixels: Vec<u32>,
    budget: UiPixelBudget,
    trace: Rc<RefCell<Trace>>,
    painter: crate::crash::software::Painter,
    fail: bool,
}
impl FakeSurface {
    fn new(budget: UiPixelBudget, trace: Rc<RefCell<Trace>>) -> Self {
        Self {
            size: [4, 4],
            pixels: Vec::new(),
            budget,
            trace,
            painter: Default::default(),
            fail: false,
        }
    }
}
impl OwnedSurface for FakeSurface {
    fn size(&self) -> [u32; 2] {
        self.size
    }
    fn input(&mut self) -> RawInput {
        self.trace.borrow_mut().events.push("input");
        RawInput {
            events: vec![egui::Event::Text("delivered-prefix".into())],
            ..Default::default()
        }
    }
    fn hide(&mut self) {
        self.trace.borrow_mut().events.push("hide");
    }
    fn paint(
        &mut self,
        context: &Context,
        mut output: FullOutput,
        size: [u32; 2],
        private: Option<(TextureId, &ColorImage)>,
        current: &mut dyn FnMut() -> bool,
    ) -> Result<Duration, Error> {
        assert_eq!(size, self.size);
        if self.fail {
            output.textures_delta.clear();
            return Err(Error::Surface);
        }
        let count = size[0] as usize * size[1] as usize;
        if self.pixels.len() != count {
            // Exercise real old+new physical pixel allocations under the same
            // production reservation helper, then release the old allocation.
            assert!(self.budget.used() >= (self.pixels.len() + count) * 4);
            let replacement = vec![0; count];
            self.trace.borrow_mut().events.push("allocate");
            self.pixels = replacement;
        }
        let primitives = context.tessellate(output.shapes, output.pixels_per_point);
        assert!(self.painter.paint_with_private(
            &mut self.pixels,
            [size[0] as usize, size[1] as usize],
            &primitives,
            &output.textures_delta,
            1.0,
            Color32::BLACK,
            private
        ));
        output.textures_delta.clear();
        if !current() {
            return Err(Error::Stale);
        }
        self.trace.borrow_mut().events.push("present");
        self.trace.borrow_mut().last_uniform_color = self
            .pixels
            .first()
            .copied()
            .filter(|color| self.pixels.iter().all(|pixel| pixel == color));
        Ok(Duration::ZERO)
    }
}
impl Drop for FakeSurface {
    fn drop(&mut self) {
        self.pixels.clear();
        self.pixels.shrink_to_fit();
        self.trace.borrow_mut().events.push("drop");
        self.trace.borrow_mut().observed.push(self.budget.used());
    }
}
fn flush(trace: &Rc<RefCell<Trace>>, raw: RawInput) {
    assert!(matches!(raw.events.as_slice(), [egui::Event::Text(text)] if text == "delivered-prefix"));
    trace.borrow_mut().events.push("flush");
}

#[test]
fn private_cpu_surface_charges_physical_resize_overlap_and_keeps_minimized_bitmap_credit() {
    let budget = UiPixelBudget::new();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let mut owner = PrivateSoftwareSurface::new(FakeSurface::new(budget.clone(), trace.clone()), slot(&budget));
    let (context, frame) = frame(&mut owner.slot);
    owner
        .present(&context, frame, &mut || true, &mut |_, raw| flush(&trace, raw))
        .unwrap();
    assert_eq!(budget.used(), 16 + 4 * 4 * 4);
    assert_eq!(trace.borrow().last_uniform_color, Some(0xff0000));
    owner.native.as_mut().unwrap().size = [8, 8]; // physical DPI/native resize, not source image dimensions
    let (context, frame) = frame(&mut owner.slot);
    owner
        .present(&context, frame, &mut || true, &mut |_, raw| flush(&trace, raw))
        .unwrap();
    assert_eq!(budget.used(), 16 + 8 * 8 * 4);
    owner.native.as_mut().unwrap().size = [0, 0];
    let (context, frame) = frame(&mut owner.slot);
    owner
        .present(&context, frame, &mut || true, &mut |_, raw| flush(&trace, raw))
        .unwrap();
    assert_eq!(budget.used(), 8 * 8 * 4, "minimized DIB still physically exists");
    assert!(owner.slot.begin_frame().is_err(), "restore needs a fresh safe owner");
    owner.retire(&mut |_, raw| flush(&trace, raw));
    assert_eq!(budget.used(), 0);
    assert_eq!(trace.borrow().observed, [256]);
    assert!(trace.borrow().events.ends_with(&["hide", "input", "flush", "drop"]));
}

#[test]
fn private_cpu_surface_stale_frame_cannot_draw_or_close_newer_image_and_failures_drop_before_credit() {
    for final_denial in [false, true] {
        let budget = UiPixelBudget::new();
        let trace = Rc::new(RefCell::new(Trace::default()));
        let mut owner = PrivateSoftwareSurface::new(FakeSurface::new(budget.clone(), trace.clone()), slot(&budget));
        let (context, mut old) = frame(&mut owner.slot);
        let ordinary = TextureId::Managed(999);
        old.output.textures_delta.push(
            ordinary,
            egui::epaint::ImageDelta::full(ColorImage::filled([1, 1], Color32::WHITE), egui::TextureOptions::LINEAR),
        );
        owner.slot.stage(1, image(&budget, [0, 255, 0, 255])).unwrap();
        assert_eq!(
            owner.present(&context, old, &mut || true, &mut |_, raw| flush(&trace, raw)),
            Err(Error::Stale)
        );
        assert!(trace.borrow().events.is_empty());
        assert!(
            owner.slot.pending_ui.set.contains_key(&ordinary),
            "same-context ordinary font upload survives discarded geometry"
        );
        let (context, frame) = frame(&mut owner.slot);
        owner
            .present(&context, frame, &mut || true, &mut |_, raw| flush(&trace, raw))
            .unwrap();
        assert_eq!(trace.borrow().last_uniform_color, Some(0x00ff00));
        owner.native.as_mut().unwrap().fail = !final_denial;
        if final_denial {
            owner.native.as_mut().unwrap().size = [8, 8];
        }
        let (context, frame) = frame(&mut owner.slot);
        let mut checks = 0;
        let result = owner.present(
            &context,
            frame,
            &mut || {
                checks += 1;
                checks == 1
            },
            &mut |_, raw| flush(&trace, raw),
        );
        assert_eq!(result, Err(if final_denial { Error::Stale } else { Error::Surface }));
        assert!(owner.native.is_none());
        assert_eq!(
            trace.borrow().observed,
            [if final_denial { 64 + 256 } else { 64 }],
            "old/new DIB credits stay live through failed resized presentation and actual bitmap drop"
        );
        assert_eq!(
            trace.borrow().events.iter().filter(|event| **event == "present").count(),
            1,
            "failure never publishes a second frame"
        );
        assert_eq!(budget.used(), 0);
        assert!(trace.borrow().events.ends_with(&["hide", "input", "flush", "drop"]));
    }
}

#[test]
fn private_cpu_surface_budget_exhaustion_refuses_resize_without_unaccounted_bitmap() {
    let budget = UiPixelBudget::new();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let mut owner = PrivateSoftwareSurface::new(FakeSurface::new(budget.clone(), trace.clone()), slot(&budget));
    let (context, frame) = frame(&mut owner.slot);
    owner
        .present(&context, frame, &mut || true, &mut |_, raw| flush(&trace, raw))
        .unwrap();
    let held = budget.reserve(UI_PIXEL_BYTES - budget.used() - 64).unwrap();
    owner.native.as_mut().unwrap().size = [8, 8];
    let (context, frame) = frame(&mut owner.slot);
    assert_eq!(
        owner.present(&context, frame, &mut || true, &mut |_, raw| flush(&trace, raw)),
        Err(Error::Capacity)
    );
    assert_eq!(trace.borrow().events.iter().filter(|event| **event == "allocate").count(), 1);
    assert!(owner.native.is_none());
    assert_eq!(budget.used(), held.bytes);
    drop(held);
    assert_eq!(budget.used(), 0);
    assert!(surface::bitmap_bytes([u32::MAX, 1]).is_err());
}

#[cfg(windows)]
#[test]
fn private_cpu_windows_adapter_is_explicitly_owned_and_dormant() {
    // Compile the real adapter/hook on Windows without creating a native window.
    let constructor = surface::WindowsSurface::owned_tool;
    let _ = constructor;
}
