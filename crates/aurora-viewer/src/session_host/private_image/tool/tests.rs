use super::*;
use egui::{ColorImage, FullOutput, TextureId};
use std::cell::RefCell;

#[derive(Clone, Copy)]
enum Event {
    Text(&'static str),
    Placement(Placement),
    Pin(bool),
    Minimized(bool),
    PanicTitle,
    FailPin,
}
#[derive(Default)]
struct Trace {
    calls: Vec<&'static str>,
    input: Vec<egui::Event>,
    placement: Option<Placement>,
    requested: Option<Placement>,
    pinned: bool,
    minimized: bool,
    panic_title: bool,
    fail_pin: bool,
    minimize_on_show_entry: bool,
    panic_minimized: bool,
    drop_credits: Vec<usize>,
    colors: Vec<u32>,
}
struct Fake {
    trace: Rc<RefCell<Trace>>,
    budget: super::super::UiPixelBudget,
    pixels: Vec<u32>,
    painter: crate::crash::software::Painter,
}
impl Fake {
    fn new(trace: Rc<RefCell<Trace>>, budget: super::super::UiPixelBudget) -> Self {
        trace.borrow_mut().placement = Some(Placement {
            outer_position: [-100, 20],
            client_size: [4, 4],
            maximized: false,
        });
        Self {
            trace,
            budget,
            pixels: Vec::new(),
            painter: Default::default(),
        }
    }
}
impl OwnedSurface for Fake {
    type PresentContext = ();
    fn minimized(&self) -> bool {
        let trace = self.trace.borrow();
        assert!(!trace.panic_minimized, "synthetic native minimize query panic");
        trace.minimized
    }
    fn size(&self) -> [u32; 2] {
        self.trace.borrow().placement.unwrap().client_size
    }
    fn input(&mut self) -> RawInput {
        let mut trace = self.trace.borrow_mut();
        trace.calls.push("input");
        RawInput {
            events: std::mem::take(&mut trace.input),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(4.0, 4.0))),
            ..Default::default()
        }
    }
    fn hide(&mut self) {
        self.trace.borrow_mut().calls.push("hide");
    }
    fn paint(
        &mut self,
        _: &(),
        context: &Context,
        mut output: FullOutput,
        size: [u32; 2],
        private: Option<(TextureId, &ColorImage)>,
        current: &mut dyn FnMut() -> bool,
    ) -> Result<Duration, Error> {
        let count = size[0] as usize * size[1] as usize;
        if self.pixels.len() != count {
            assert!(
                self.budget.used() >= (self.pixels.len() + count) * 4,
                "physical old+new storage is credited before allocation"
            );
            self.pixels = vec![0; count];
        }
        let meshes = context.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
        assert!(self.painter.paint_with_private(
            &mut self.pixels,
            size.map(|side| side as usize),
            &meshes,
            &output.textures_delta,
            output.pixels_per_point,
            Color32::BLACK,
            private
        ));
        output.textures_delta.clear();
        if !current() {
            return Err(Error::Stale);
        }
        let color = self.pixels[0];
        assert!(self.pixels.iter().all(|pixel| *pixel == color));
        let mut trace = self.trace.borrow_mut();
        trace.colors.push(color);
        trace.calls.push("paint");
        Ok(Duration::ZERO)
    }
}
impl ToolNative for Fake {
    type Id = u64;
    type Event = Event;
    type Response = bool;
    fn id(&self) -> u64 {
        7
    }
    fn event(&mut self, _: &(), event: &Event) -> Result<bool, Error> {
        let mut trace = self.trace.borrow_mut();
        trace.calls.push("event");
        match *event {
            Event::Text(value) => trace.input.push(egui::Event::Text(value.into())),
            Event::Placement(value) => trace.placement = Some(value),
            Event::Pin(value) => trace.pinned = value,
            Event::Minimized(value) => trace.minimized = value,
            Event::PanicTitle => trace.panic_title = true,
            Event::FailPin => trace.fail_pin = true,
        }
        Ok(true)
    }
    fn show(&mut self, _: &()) -> Result<(), Error> {
        let mut trace = self.trace.borrow_mut();
        if trace.minimize_on_show_entry {
            trace.minimized = true;
        }
        if trace.minimized {
            return Err(Error::Stale);
        }
        trace.calls.push("show");
        Ok(())
    }
    fn request_redraw(&mut self, _: &()) -> Result<(), Error> {
        self.trace.borrow_mut().calls.push("redraw");
        Ok(())
    }
    fn focused(&self, _: &()) -> Result<bool, Error> {
        Ok(self.trace.borrow().calls.contains(&"focus"))
    }
    fn focus(&mut self, _: &()) -> Result<(), Error> {
        self.trace.borrow_mut().calls.push("focus");
        Ok(())
    }
    fn set_title(&mut self, _: &(), _: &str) -> Result<(), Error> {
        assert!(!self.trace.borrow().panic_title, "synthetic native title panic");
        self.trace.borrow_mut().calls.push("title");
        Ok(())
    }
    fn pin(&mut self, _: &(), _: bool) -> Result<bool, Error> {
        let mut trace = self.trace.borrow_mut();
        trace.calls.push("pin");
        if trace.fail_pin {
            return Err(Error::Surface);
        }
        Ok(trace.pinned)
    }
    fn placement(&self, _: &()) -> Result<Placement, Error> {
        Ok(self.trace.borrow().placement.unwrap())
    }
    fn request_placement(&mut self, _: &(), placement: Placement) -> Result<(), Error> {
        let mut trace = self.trace.borrow_mut();
        trace.requested = Some(placement);
        trace.calls.push("placement_request");
        Ok(())
    }
}
impl Drop for Fake {
    fn drop(&mut self) {
        self.pixels = Vec::new();
        let mut trace = self.trace.borrow_mut();
        trace.calls.push("drop");
        trace.drop_credits.push(self.budget.used());
    }
}
fn fixture() -> (Tool<Fake>, Context, super::super::UiPixelBudget, Rc<RefCell<Trace>>) {
    let budget = super::super::UiPixelBudget::new();
    let context = Context::default();
    let slot = PrivateImageSlot::new(context.clone(), 11, budget.clone()).unwrap();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let native = Fake::new(trace.clone(), budget.clone());
    let mut tool = Tool::new(native, slot);
    tool.stage(11, PrivateImage::from_rgba(&budget, [2, 2], &[255, 0, 0, 255].repeat(4)).unwrap())
        .unwrap();
    (tool, context, budget, trace)
}
fn draw(tool: &mut Tool<Fake>) -> PrivateFrame {
    let size = tool.owner.native.as_ref().unwrap().size();
    tool.draw(
        RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size[0] as f32, size[1] as f32),
            )),
            ..Default::default()
        },
        |ui, view| {
            ui.painter().image(
                view.texture_id(),
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0] as f32, size[1] as f32)),
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        },
        &mut |_, _| {},
    )
    .unwrap()
}
fn show(tool: &mut Tool<Fake>, context: &Context) {
    let sealed = draw(tool);
    tool.present(&(), context, sealed, &mut || true, &mut |_, _| {}).unwrap();
}

#[test]
fn private_tool_routes_only_owned_native_events_and_shows_only_current_original_frame() {
    let (mut tool, context, budget, trace) = fixture();
    assert!(tool.event(&(), 8, &Event::Text("wrong owner"), &mut |_, _| {}).unwrap().is_none());
    assert!(trace.borrow().calls.is_empty());
    tool.request_redraw(&(), &mut |_, _| {}).unwrap();
    assert_eq!(trace.borrow().calls, ["redraw"]);
    assert_eq!(tool.event(&(), 7, &Event::Text("delivered"), &mut |_, _| {}).unwrap(), Some(true));
    let input = tool.raw_input(&mut |_, _| {}).unwrap();
    assert!(matches!(input.events.as_slice(),[egui::Event::Text(text)] if text=="delivered"));
    assert_eq!(tool.focus(&(), &mut |_, _| {}), Err(Error::Stale));
    assert_eq!(
        tool.request_placement(
            &(),
            Placement {
                outer_position: [0, 0],
                client_size: [8, 8],
                maximized: true
            },
            &mut |_, _| {}
        ),
        Err(Error::Stale)
    );
    assert!(
        !trace.borrow().calls.contains(&"placement_request"),
        "placement/restore must not show an unpainted owner"
    );
    assert!(!trace.borrow().calls.contains(&"focus"));
    let old = draw(&mut tool);
    let fresh = draw(&mut tool);
    assert_eq!(tool.present(&(), &context, old, &mut || true, &mut |_, _| {}), Err(Error::Stale));
    assert!(!trace.borrow().calls.contains(&"show"));
    tool.present(&(), &context, fresh, &mut || true, &mut |_, _| {}).unwrap();
    assert_eq!(trace.borrow().colors, [0xff0000]);
    assert_eq!(budget.used(), 16 + 64);
    let unchanged = draw(&mut tool);
    assert_eq!(
        tool.stage(12, PrivateImage::from_rgba(&budget, [2, 2], &[0, 0, 255, 255].repeat(4)).unwrap()),
        Err(Error::Stale)
    );
    let foreign = super::super::UiPixelBudget::new();
    assert_eq!(
        tool.stage(11, PrivateImage::from_rgba(&foreign, [2, 2], &[0, 0, 255, 255].repeat(4)).unwrap()),
        Err(Error::Invalid)
    );
    assert_eq!(foreign.used(), 0);
    tool.present(&(), &context, unchanged, &mut || true, &mut |_, _| {}).unwrap();
    assert_eq!(
        budget.used(),
        80,
        "rejected late/wrong-pool work cannot replace or retire the live owner"
    );
    let old = draw(&mut tool);
    tool.stage(11, PrivateImage::from_rgba(&budget, [2, 2], &[0, 255, 0, 255].repeat(4)).unwrap())
        .unwrap();
    assert_eq!(tool.present(&(), &context, old, &mut || true, &mut |_, _| {}), Err(Error::Stale));
    show(&mut tool, &context);
    assert_eq!(trace.borrow().colors, [0xff0000, 0xff0000, 0x00ff00]);
    assert_eq!(
        trace.borrow().calls.iter().filter(|call| **call == "show").count(),
        1,
        "same-binding updates never re-show/focus the window"
    );
}

#[test]
fn private_tool_requests_keep_observed_pin_placement_and_delivered_retirement_order() {
    let (mut tool, context, budget, trace) = fixture();
    show(&mut tool, &context);
    let old = tool.placement(&(), &mut |_, _| {}).unwrap();
    let desired = Placement {
        outer_position: [-1920, -120],
        client_size: [8, 4],
        maximized: false,
    };
    tool.request_placement(&(), desired, &mut |_, _| {}).unwrap();
    assert_eq!(
        tool.placement(&(), &mut |_, _| {}).unwrap(),
        old,
        "request is not observed placement"
    );
    assert_eq!(budget.used(), 80, "request cannot release old physical storage");
    assert!(
        !tool.pin(&(), true, &mut |_, _| {}).unwrap(),
        "return observed pin, not requested hint"
    );
    tool.event(&(), 7, &Event::Pin(true), &mut |_, _| {}).unwrap();
    assert!(tool.pin(&(), true, &mut |_, _| {}).unwrap());
    assert!(
        !tool.focused(&(), &mut |_, _| {}).unwrap(),
        "pin does not focus the tool or a World"
    );
    tool.event(&(), 7, &Event::Placement(desired), &mut |_, _| {}).unwrap();
    show(&mut tool, &context);
    assert_eq!(budget.used(), 16 + 128);
    tool.event(&(), 7, &Event::Text("old delivered prefix"), &mut |_, _| {}).unwrap();
    trace.borrow_mut().calls.clear();
    tool.retire(&mut |old_context, input| {
        assert_eq!(*old_context, context);
        assert!(matches!(input.events.as_slice(),[egui::Event::Text(text)] if text=="old delivered prefix"));
        trace.borrow_mut().calls.push("flush");
    });
    assert_eq!(trace.borrow().calls, ["hide", "input", "flush", "drop"]);
    assert_eq!(
        trace.borrow().drop_credits,
        [128],
        "actual bitmap drops before its physical reservation"
    );
    assert_eq!(budget.used(), 0);
    assert_eq!(tool.focus(&(), &mut |_, _| {}), Err(Error::Stale));
    assert_eq!(
        tool.stage(11, PrivateImage::from_rgba(&budget, [2, 2], &[0, 0, 0, 255].repeat(4)).unwrap()),
        Err(Error::Stale)
    );
    assert_eq!(budget.used(), 0);
}

#[test]
fn private_tool_first_show_rechecks_permission_and_command_errors_retire_owned_storage() {
    for reject_at in [3, 4] {
        let (mut tool, context, budget, trace) = fixture();
        let sealed = draw(&mut tool);
        let mut checks = 0;
        assert_eq!(
            tool.present(
                &(),
                &context,
                sealed,
                &mut || {
                    checks += 1;
                    checks < reject_at
                },
                &mut |_, _| {}
            ),
            Err(Error::Stale)
        );
        assert_eq!(checks, reject_at);
        assert_eq!(
            trace.borrow().calls.iter().filter(|call| **call == "show").count(),
            usize::from(reject_at == 4)
        );
        assert_eq!(trace.borrow().drop_credits, [64]);
        assert_eq!(budget.used(), 0);
    }
    let (mut tool, context, budget, trace) = fixture();
    show(&mut tool, &context);
    tool.event(&(), 7, &Event::FailPin, &mut |_, _| {}).unwrap();
    assert_eq!(tool.pin(&(), true, &mut |_, _| {}), Err(Error::Surface));
    assert_eq!(trace.borrow().drop_credits, [64]);
    assert_eq!(budget.used(), 0);
}

#[test]
fn private_tool_command_and_draw_unwind_remove_retained_native_owner_before_releasing_credits() {
    for native_panic in [false, true] {
        let (mut tool, context, budget, trace) = fixture();
        show(&mut tool, &context);
        if native_panic {
            tool.event(&(), 7, &Event::PanicTitle, &mut |_, _| {}).unwrap();
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut flush = |_: &Context, _: RawInput| trace.borrow_mut().calls.push("flush");
            if native_panic {
                let _ = tool.set_title(&(), "Synthetic monitor", &mut flush);
            } else {
                let _ = tool.draw(RawInput::default(), |_, _| panic!("synthetic body panic"), &mut flush);
            }
        }));
        assert!(result.is_err());
        assert!(!tool.live());
        let calls = &trace.borrow().calls;
        assert_eq!(&calls[calls.len() - 4..], ["hide", "input", "flush", "drop"]);
        assert_eq!(trace.borrow().drop_credits, [64]);
        assert_eq!(budget.used(), 0);
    }
}

#[test]
fn private_tool_minimized_frame_cannot_show_or_revive_focus_and_stays_charged_until_retired() {
    let (mut tool, context, budget, trace) = fixture();
    show(&mut tool, &context);
    tool.event(&(), 7, &Event::Minimized(true), &mut |_, _| {}).unwrap();
    assert!(tool.minimized(&mut |_, _| {}).unwrap());
    let sealed = draw(&mut tool);
    tool.present(&(), &context, sealed, &mut || true, &mut |_, _| {}).unwrap();
    assert!(!tool.live());
    assert_eq!(budget.used(), 64);
    assert_eq!(trace.borrow().calls.iter().filter(|call| **call == "show").count(), 1);
    assert_eq!(tool.focus(&(), &mut |_, _| {}), Err(Error::Stale));
    tool.retire(&mut |_, _| {});
    assert_eq!(trace.borrow().drop_credits, [64]);
    assert_eq!(budget.used(), 0);
}

#[cfg(windows)]
#[test]
fn private_tool_windows_facade_is_owned_and_all_native_calls_use_per_call_event_loop() {
    use crate::session_host::private_image::MonitorSurface;
    fn draw_signature(
        surface: &mut MonitorSurface,
        raw: RawInput,
        flush: &mut dyn FnMut(&Context, RawInput),
    ) -> Result<PrivateFrame, Error> {
        surface.draw(raw, |_, _| {}, flush)
    }
    let _ = draw_signature;
    fn owned<T: 'static>() {}
    owned::<MonitorSurface>();
    let _ = MonitorSurface::owned_tool;
    let _ = MonitorSurface::id;
    let _ = MonitorSurface::event;
    let _ = MonitorSurface::raw_input;
    let _ = MonitorSurface::request_redraw;
    let _ = MonitorSurface::focused;
    let _ = MonitorSurface::minimized;
    let _ = MonitorSurface::focus;
    let _ = MonitorSurface::set_title;
    let _ = MonitorSurface::pin;
    let _ = MonitorSurface::placement;
    let _ = MonitorSurface::request_placement;
    let _ = MonitorSurface::stage;
    let _ = MonitorSurface::present;
    let _ = MonitorSurface::retire;
}

#[test]
fn private_tool_first_show_rechecks_minimized_after_paint_without_revoking_source_permission() {
    for minimize_at in [2, 3, usize::MAX] {
        let (mut tool, context, budget, trace) = fixture();
        trace.borrow_mut().minimize_on_show_entry = minimize_at == usize::MAX;
        let sealed = draw(&mut tool);
        let mut checks = 0;
        let mut current = || {
            checks += 1;
            if checks == minimize_at {
                trace.borrow_mut().minimized = true;
            }
            true // source/retention permission remains valid throughout
        };
        assert_eq!(tool.present(&(), &context, sealed, &mut current, &mut |_, _| {}), Err(Error::Stale));
        assert!(checks >= 2, "actual paint completed before the late minimize");
        assert!(trace.borrow().calls.contains(&"paint"));
        assert!(
            !trace.borrow().calls.contains(&"show"),
            "a late native minimize cannot restore/show this owner"
        );
        assert_eq!(
            trace.borrow().drop_credits,
            [64],
            "actual DIB destroyed before physical credit release"
        );
        assert_eq!(budget.used(), 0);
        assert!(!tool.live());
    }
    let (mut tool, context, budget, trace) = fixture();
    let sealed = draw(&mut tool);
    let mut checks = 0;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = tool.present(
            &(),
            &context,
            sealed,
            &mut || {
                checks += 1;
                if checks == 3 {
                    trace.borrow_mut().panic_minimized = true;
                }
                true
            },
            &mut |_, _| {},
        );
    }));
    assert!(result.is_err(), "late native query failure resumes the original unwind");
    assert_eq!(checks, 3);
    assert!(trace.borrow().calls.contains(&"paint"));
    assert!(!trace.borrow().calls.contains(&"show"));
    assert_eq!(trace.borrow().drop_credits, [64]);
    assert_eq!(budget.used(), 0);
    assert!(!tool.live(), "a caught late query panic retains no native bitmap");
}
