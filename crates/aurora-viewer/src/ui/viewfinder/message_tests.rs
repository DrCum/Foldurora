use super::*;

/// The rows of the shown text `wanted` in this frame's shapes.
fn rows(output: &egui::FullOutput, wanted: &str) -> usize {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == wanted => Some(text.galley.rows.len()),
            _ => None,
        })
        .expect("message visible")
}

#[test]
fn a_long_message_after_a_short_one_stays_on_one_line() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let p = theme.palette();
    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0));
    let mut time = 0.0;
    let mut frame = |text: &str| {
        time += 0.1;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                time: Some(time),
                ..Default::default()
            },
            |ui| message(ui.ctx(), &p, screen, text, false),
        );
        output.textures_delta.clear();
        output
    };
    // the area learns the short message's width first
    for _ in 0..3 {
        frame("AF-S");
    }
    let long = "Interface hidden · I or the button at the bottom right shows it again";
    frame(long);
    assert_eq!(rows(&frame(long), long), 1);
}

/// Real egui hit testing and gesture state around the production shutter.
struct ShutterFixture {
    ctx: egui::Context,
    viewfinder: Viewfinder,
    frame: usize,
    center: Pos2,
}

impl ShutterFixture {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut fixture = Self {
            ctx,
            viewfinder: Viewfinder::default(),
            frame: 0,
            center: Pos2::ZERO,
        };
        for _ in 0..3 {
            assert!(fixture.show(Vec::new(), false).is_empty());
        }
        fixture
    }

    fn button(&self, pressed: bool, position: Pos2) -> egui::Event {
        egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn show(&mut self, events: Vec<egui::Event>, repeat_pass: bool) -> Vec<Out> {
        self.frame += 1;
        let mut actions = Vec::new();
        let palette = crate::theme::Theme::default().palette();
        let mut passes = 0;
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 480.0))),
                time: Some(self.frame as f64 / 60.0),
                events,
                ..Default::default()
            },
            |ui| {
                passes += 1;
                self.viewfinder.shutter_button(ui, &palette, &mut actions);
                if repeat_pass && ui.ctx().current_pass_index() == 0 {
                    ui.ctx().request_discard("exercise repeated shutter layout");
                }
            },
        );
        self.center = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Circle(circle) if (circle.radius - 33.0).abs() < 0.01 => Some(circle.center),
                _ => None,
            })
            .expect("production shutter background");
        output.textures_delta.clear();
        if repeat_pass {
            assert!(passes >= 2, "the gesture must run through repeated egui passes");
        }
        actions
    }
}

#[test]
fn actual_shutter_same_frame_click_focuses_and_releases_once() {
    let mut f = ShutterFixture::new();
    // No preceding hover at the shutter: motion, down and up share one frame.
    let center = f.center;
    let actions = f.show(
        vec![egui::Event::PointerMoved(center), f.button(true, center), f.button(false, center)],
        true,
    );
    assert_eq!(actions, vec![Out::HalfPress, Out::Release]);
    assert!(!f.viewfinder.mouse_down);
    assert!(f.show(Vec::new(), true).is_empty(), "the completed click must not replay");
}

#[test]
fn actual_shutter_held_press_and_release_emit_one_action_each() {
    let mut f = ShutterFixture::new();
    let center = f.center;
    assert_eq!(
        f.show(vec![egui::Event::PointerMoved(center), f.button(true, center)], true),
        vec![Out::HalfPress]
    );
    assert!(f.viewfinder.mouse_down);
    for _ in 0..3 {
        assert!(f.show(Vec::new(), true).is_empty());
    }
    assert_eq!(f.show(vec![f.button(false, center)], true), vec![Out::Release]);
    assert!(!f.viewfinder.mouse_down);
    assert!(f.show(Vec::new(), true).is_empty());
}

#[test]
fn actual_shutter_release_off_button_cancels_and_next_click_still_works() {
    let mut f = ShutterFixture::new();
    let center = f.center;
    let outside = Pos2::new(300.0, 300.0);
    assert_eq!(
        f.show(vec![egui::Event::PointerMoved(center), f.button(true, center)], true),
        vec![Out::HalfPress]
    );
    assert!(f.show(vec![egui::Event::PointerMoved(outside)], true).is_empty());
    assert_eq!(f.show(vec![f.button(false, outside)], true), vec![Out::Cancel]);
    assert!(!f.viewfinder.mouse_down);
    assert_eq!(
        f.show(
            vec![egui::Event::PointerMoved(center), f.button(true, center), f.button(false, center)],
            true,
        ),
        vec![Out::HalfPress, Out::Release]
    );
}

#[test]
fn actual_shutter_press_elsewhere_then_release_over_button_is_ignored() {
    let mut f = ShutterFixture::new();
    let outside = Pos2::new(300.0, 300.0);
    assert!(
        f.show(vec![egui::Event::PointerMoved(outside), f.button(true, outside)], true)
            .is_empty()
    );
    assert!(
        f.show(vec![egui::Event::PointerMoved(f.center), f.button(false, f.center)], true)
            .is_empty()
    );
    assert!(!f.viewfinder.mouse_down);
}

#[test]
fn actual_shutter_secondary_button_does_not_focus_or_take_a_photo() {
    let mut f = ShutterFixture::new();
    let center = f.center;
    f.show(vec![egui::Event::PointerMoved(center)], false);
    for pressed in [true, false] {
        assert!(
            f.show(
                vec![egui::Event::PointerButton {
                    pos: center,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
                true,
            )
            .is_empty()
        );
    }
    assert!(!f.viewfinder.mouse_down);
}
