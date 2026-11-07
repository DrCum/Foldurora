//! The app's side of Help › About Aurora Viewer (`ui/about.rs`): the
//! information LLFloaterAbout::getInfo gathers (indra/newview/
//! llfloaterabout.cpp, originally LGPL 2.1) — the viewer, the computer,
//! the graphics, where we are and the link — read from the app while the
//! window is open. The computer is read in the background
//! (`system_info.rs`) when the window opens, then again every few seconds
//! for the viewer's memory. The region and the position follow RLVa's
//! @showloc, as the Legacy Viewer's AboutPositionRLVShowLoc. Pinned Folderstorm
//! abbe3cb0 adds an explicit per-open/source/region location disclosure choice;
//! both visible details and the copied report honor it.

use super::App;
use crate::system_info::{Machine, Reading};
use crate::ui::about::{AboutUi, AboutView, Row, Section};
use crate::{settings::Settings, world::World};
use aurora_render::Renderer;
use std::time::{Duration, Instant};

/// The viewer's memory is read again this often while the window is open.
const MACHINE_EVERY: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct About {
    pub ui: AboutUi,
    machine: Option<Machine>,
    reading: Option<(Reading, Instant)>,
    location: Option<LocationContext>,
}

/// Small source facts only; no location text, simulator address, credentials or
/// persisted consent. Unknown/unready facts cannot retain a previous choice.
#[derive(Clone, Copy, PartialEq, Eq)]
struct LocationContext {
    agent: uuid::Uuid,
    session: uuid::Uuid,
    region: Option<(u64, Option<uuid::Uuid>)>,
    privacy: (bool, u64, bool),
    ready: bool,
}
impl LocationContext {
    fn observe(world: &World, live: bool) -> Self {
        let session = world.login.as_ref().map_or(uuid::Uuid::nil(), |login| login.session_id);
        Self {
            agent: world.agent_id,
            session,
            region: world
                .main()
                .map(|region| (region.handle, region.info.as_ref().map(|info| info.region_id))),
            privacy: (world.rlva.enabled(), world.rlva.revision(), world.rlva.has("showloc")),
            ready: live
                && !world.agent_id.is_nil()
                && !session.is_nil()
                && world.login.as_ref().is_some_and(|login| login.agent_id == world.agent_id)
                && world.main().is_some()
                && world.movement_complete
                && !world.teleporting,
        }
    }
    fn allowed(self) -> bool {
        self.ready && !self.privacy.2
    }
}
impl About {
    fn observe_location(&mut self, context: LocationContext) {
        self.ui.location_context(context.allowed(), self.location != Some(context));
        self.location = Some(context);
    }
}

impl App {
    /// CPU-only observation after EACH actual event application, including
    /// events consumed by World. An A->B->A drain cannot retain old consent
    /// merely because no About paint occurred between the boundaries.
    pub(super) fn observe_about_location(&mut self) {
        if !self.panels.about {
            self.about.location = None;
            self.about.ui.closed();
            return;
        }
        let live = matches!(self.screen, super::Screen::World) && !self.login_form.busy;
        self.about.observe_location(LocationContext::observe(&self.world, live));
    }

    /// The window, drawn with the interface.
    pub(super) fn about_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette, renderer: &Renderer) {
        self.observe_about_location();
        if !self.panels.about {
            // read afresh at the next opening
            self.about.machine = None;
            self.about.reading = None;
            return;
        }
        let live = matches!(self.screen, super::Screen::World) && !self.login_form.busy;
        if let Some((reading, _)) = &self.about.reading
            && let Some(machine) = reading.get()
        {
            self.about.machine = Some(machine.clone());
        }
        let due = self
            .about
            .reading
            .as_ref()
            .is_none_or(|(reading, at)| reading.get().is_some() && at.elapsed() >= MACHINE_EVERY);
        if due {
            self.about.reading = Some((Reading::start(), Instant::now()));
        }
        if self.about.machine.is_none() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let view = AboutView {
            sections: self.about_sections(renderer),
            location: location_section(&self.world, &self.settings, self.demo, live, self.about.ui.includes_location()),
            logo: self.logo.as_ref(),
        };
        self.about.ui.show(ctx, p, &view, &mut self.panels.about);
    }

    fn about_sections(&self, renderer: &Renderer) -> Vec<Section> {
        let s = &self.settings;
        let gpu = &renderer.info;
        let mut sections = Vec::new();
        let (w, h) = renderer.size();
        sections.push(Section {
            icon: "app-window-filled",
            title: "about-section-viewer",
            rows: vec![
                Row::text("about-commit", aurora_version::commit()),
                Row::text(
                    "about-build",
                    format!("{} ({})", crate::ui::about::build_label(), env!("AURORA_BUILD_PROFILE")),
                ),
                Row::text("about-language", aurora_i18n::language()),
                Row::text("about-window-size", format!("{w} × {h} px")),
                Row::text("about-ui-scale", format!("{:.0} %", s.ui_scale * 100.0)),
                if s.rlva_enabled {
                    Row::text("about-rlva", crate::world::rlva::VERSION)
                } else {
                    Row::key("about-rlva", "about-off")
                },
            ],
        });
        sections.push(Section {
            icon: "device-desktop-filled",
            title: "about-section-computer",
            rows: match &self.about.machine {
                Some(m) => vec![
                    Row::text("about-os", m.os.clone()),
                    Row::text("about-cpu", m.cpu.clone()),
                    Row::text("about-threads", m.threads.to_string()),
                    Row::text("about-memory", m.memory_text()),
                ],
                None => vec![Row::key("about-os", "about-reading")],
            },
        });
        let aa = ["none", "FXAA", "MSAA 2x", "MSAA 4x", "MSAA 8x", "TAA", "SMAA"]
            .get(s.antialiasing as usize)
            .copied()
            .unwrap_or("?");
        sections.push(Section {
            icon: "photo-filled",
            title: "about-section-graphics",
            rows: vec![
                Row::text(
                    "about-gpu",
                    format!("{} ({})", gpu.name, if gpu.integrated { "integrated" } else { "discrete" }),
                ),
                Row::text("about-graphics-api", gpu.backend.clone()),
                Row::text("about-driver", gpu.driver.clone()),
                match gpu.vram_mb {
                    Some(mb) => Row::text("about-vram", format!("{mb} MB")),
                    None => Row::key("about-vram", "about-unknown"),
                },
                Row::text(
                    "about-texture-budget",
                    format!("{} MB", s.texture_budget(gpu.vram_mb, gpu.integrated)),
                ),
                Row::text(
                    "about-display",
                    format!(
                        "{:.0} Hz, {}",
                        self.screen_rate.hz(),
                        super::present_mode_name(renderer.present_mode())
                    ),
                ),
                Row::text(
                    "about-render-settings",
                    format!(
                        "draw distance {:.0} m, LOD {}, shadows {}, {aa}",
                        s.draw_distance,
                        s.lod_factor,
                        if s.shadows { "on" } else { "off" }
                    ),
                ),
            ],
        });
        if self.world.main().is_some() {
            let ping = self.net.stats.ping_ms.load(std::sync::atomic::Ordering::Relaxed);
            let loss = crate::net_profile::loss_percent(&self.net_profile.last);
            sections.push(Section {
                icon: "antenna-bars-5",
                title: "about-section-network",
                rows: vec![
                    Row::text("about-ping", format!("{ping} ms")),
                    Row::text("about-packet-loss", format!("{loss:.1} %")),
                    Row::text("about-bandwidth", format!("{} kbit/s", s.bandwidth_kbps)),
                ],
            });
        }
        sections
    }
}

/// Construct sensitive rows only after CURRENT actual facts and consent pass.
/// UI independently filters this optional card again after checkbox input.
fn location_section(world: &World, settings: &Settings, demo: bool, live: bool, include: bool) -> Option<Section> {
    if !include || !LocationContext::observe(world, live).allowed() {
        return None;
    }
    let region = world.main()?;
    let at = world.agent.position;
    // A Hypergrid visit uses the current region's grid metadata, as before.
    let grid = crate::grids::current_label(settings, world.opensim_extras())
        .filter(|_| !demo)
        .unwrap_or_else(|| crate::world::chat_log::grid_name(&settings.grid, &settings.custom_login_uri, demo));
    Some(Section {
        icon: "map-pin-filled",
        title: "about-section-location",
        rows: vec![
            Row::text("about-grid", grid),
            Row::text("about-region", region.name.clone()),
            Row::text("about-position", format!("{:.1}, {:.1}, {:.1}", at.x, at.y, at.z)),
            match world.estate.versions.get(&region.handle) {
                Some(version) => Row::text("about-simulator", version.clone()),
                None => Row::key("about-simulator", "about-unknown"),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{scene::avatar::AvatarLibrary, theme::Theme, ui::about};
    use egui::{Event, PointerButton, Pos2, Rect, Vec2};
    use std::sync::Arc;
    use uuid::Uuid;

    const PLACE: &str = "Synthetic About region";
    const SIM: &str = "Synthetic About simulator";

    fn world() -> World {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        Arc::make_mut(world.login.as_mut().unwrap()).session_id = Uuid::from_u128(101);
        world.movement_complete = true;
        world.teleporting = false;
        world.rlva_configure(true);
        let handle = world.main_region.unwrap();
        world.regions.get_mut(&handle).unwrap().name = PLACE.into();
        world.estate.versions.insert(handle, SIM.into());
        assert!(LocationContext::observe(&world, true).allowed());
        world
    }
    fn restriction(world: &mut World, text: &str) {
        assert!(world.rlva.receive(&aurora_net::ChatMessage {
            from_name: "Synthetic About restriction".into(),
            source_id: Uuid::from_u128(102),
            owner_id: world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: text.into(),
            ..Default::default()
        }));
    }

    #[test]
    fn about_location_section_requires_current_actual_login_region_readiness_and_rlva_permission() {
        let mut world = world();
        let settings = Settings::default();
        assert!(location_section(&world, &settings, true, true, false).is_none());
        let section = location_section(&world, &settings, true, true, true).unwrap();
        assert_eq!(
            section.rows.iter().map(|row| row.label).collect::<Vec<_>>(),
            vec!["about-grid", "about-region", "about-position", "about-simulator"]
        );
        let text = about::copy_text([&section]);
        assert!(text.contains(PLACE) && text.contains(SIM));
        assert!(location_section(&world, &settings, true, false, true).is_none());
        world.teleporting = true;
        assert!(location_section(&world, &settings, true, true, true).is_none());
        world.teleporting = false;
        world.movement_complete = false;
        assert!(location_section(&world, &settings, true, true, true).is_none());
        world.movement_complete = true;
        let region = world.main_region.take();
        assert!(location_section(&world, &settings, true, true, true).is_none());
        world.main_region = region;
        let session = world.login.take();
        assert!(location_section(&world, &settings, true, true, true).is_none());
        world.login = session;
        Arc::make_mut(world.login.as_mut().unwrap()).agent_id = Uuid::from_u128(103);
        assert!(location_section(&world, &settings, true, true, true).is_none());
        Arc::make_mut(world.login.as_mut().unwrap()).agent_id = world.agent_id;
        restriction(&mut world, "@showloc=n");
        assert!(
            location_section(&world, &settings, true, true, true).is_none(),
            "opt-in never overrides native privacy"
        );
    }

    fn text_rect(shape: &egui::Shape, text: &str) -> Option<Rect> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text && !t.galley.elided => Some(Rect::from_min_size(t.pos, t.galley.size())),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_rect(shape, text)),
            _ => None,
        }
    }
    fn shown(output: &egui::FullOutput, text: &str) -> bool {
        output.shapes.iter().any(|shape| text_rect(&shape.shape, text).is_some())
    }
    fn copied(output: &egui::FullOutput) -> Vec<&str> {
        output
            .platform_output
            .commands
            .iter()
            .filter_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }
    fn pointer(at: Pos2, pressed: bool) -> Vec<Event> {
        vec![
            Event::PointerMoved(at),
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ]
    }

    struct Harness {
        ctx: egui::Context,
        palette: crate::theme::Palette,
        about: About,
        world: World,
        settings: Settings,
        open: bool,
        time: f64,
    }
    impl Harness {
        fn new(zoom: f32) -> Self {
            let ctx = egui::Context::default();
            let theme = Theme::default();
            theme.apply(&ctx, 1.0);
            crate::ui::fonts::apply(&ctx, crate::ui::fonts::DEFAULT);
            ctx.set_zoom_factor(zoom);
            crate::ui::widgets::place_floater(&ctx, about::WINDOW, egui::pos2(30.0, 30.0), Vec2::new(700.0, 850.0));
            let mut h = Self {
                ctx,
                palette: theme.palette(),
                about: About::default(),
                world: world(),
                settings: Settings::default(),
                open: true,
                time: 0.0,
            };
            for _ in 0..4 {
                h.frame(Vec::new());
            }
            h
        }
        fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
            self.time += 0.1;
            let output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1500.0, 1400.0))),
                    time: Some(self.time),
                    events,
                    ..Default::default()
                },
                |_| {
                    if self.open {
                        self.about.observe_location(LocationContext::observe(&self.world, true));
                    } else {
                        self.about.location = None;
                        self.about.ui.closed();
                    }
                    let view = AboutView {
                        sections: vec![Section {
                            icon: "device-desktop-filled",
                            title: "about-section-computer",
                            rows: vec![Row::text("about-cpu", "Synthetic CPU")],
                        }],
                        location: location_section(&self.world, &self.settings, true, true, self.about.ui.includes_location()),
                        logo: None,
                    };
                    self.about.ui.show(&self.ctx, &self.palette, &view, &mut self.open);
                },
            );
            crate::ui::login::test_output(output)
        }
        fn position(&mut self, label: &str) -> Pos2 {
            let output = self.frame(Vec::new());
            output
                .shapes
                .iter()
                .find_map(|shape| text_rect(&shape.shape, label))
                .unwrap_or_else(|| panic!("missing About control: {label}"))
                .center()
        }
        fn click(&mut self, label: &str) -> egui::FullOutput {
            let at = self.position(label);
            self.frame(pointer(at, true));
            self.frame(pointer(at, false))
        }
        fn copy(&mut self) -> egui::FullOutput {
            // Copied marks use wall Instant, not the fake egui clock. Click
            // the actual footer in either state without sleeping in a test.
            let output = self.frame(Vec::new());
            let at = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    text_rect(&shape.shape, aurora_i18n::tr!("about-copy"))
                        .or_else(|| text_rect(&shape.shape, aurora_i18n::tr!("about-copied")))
                })
                .map(|rect| rect.center())
                .max_by(|a, b| a.y.total_cmp(&b.y))
                .unwrap();
            self.frame(pointer(at, true));
            self.frame(pointer(at, false))
        }
    }

    #[test]
    fn about_real_egui_location_opt_in_gates_display_row_copy_and_full_report_including_same_pass_uncheck() {
        for zoom in [1.0, 1.5] {
            let mut h = Harness::new(zoom);
            assert!(!h.about.ui.includes_location());
            assert!(!shown(&h.frame(Vec::new()), PLACE));
            let output = h.copy();
            let text = copied(&output);
            assert_eq!(text.len(), 1);
            assert!(text[0].contains("Synthetic CPU") && !text[0].contains(PLACE) && !text[0].contains(SIM));
            h.click(aurora_i18n::tr!("about-include-location"));
            assert!(h.about.ui.includes_location());
            let output = h.frame(Vec::new());
            assert!(shown(&output, PLACE) && shown(&output, SIM));
            let row_copy = h.click(PLACE);
            assert_eq!(copied(&row_copy), vec![PLACE]);
            let output = h.copy();
            assert!(copied(&output)[0].contains(PLACE) && copied(&output)[0].contains(SIM));
            // On release this frame's view still contains a prebuilt location
            // card. Production UI must filter it AFTER processing the uncheck.
            let output = h.click(aurora_i18n::tr!("about-include-location"));
            assert!(!h.about.ui.includes_location());
            assert!(!shown(&output, PLACE) && !shown(&output, SIM));
            let output = h.copy();
            assert!(!copied(&output)[0].contains(PLACE) && !copied(&output)[0].contains(SIM));
        }
    }

    #[test]
    fn about_real_egui_source_change_retires_a_held_opt_in_click_and_reopen_requires_new_consent() {
        let mut h = Harness::new(1.0);
        let at = h.position(aurora_i18n::tr!("about-include-location"));
        h.frame(pointer(at, true));
        Arc::make_mut(h.world.login.as_mut().unwrap()).session_id = Uuid::from_u128(104);
        let output = h.frame(pointer(at, false));
        assert!(!h.about.ui.includes_location() && !shown(&output, PLACE));
        h.frame(Vec::new());
        assert!(!h.about.ui.includes_location(), "old release cannot become new-source consent");
        h.click(aurora_i18n::tr!("about-include-location"));
        assert!(h.about.ui.includes_location());
        h.open = false;
        h.frame(Vec::new());
        h.about.ui.open(&h.ctx, about::Tab::Info, &mut h.open);
        let output = h.frame(Vec::new());
        assert!(!h.about.ui.includes_location() && !shown(&output, PLACE));
        h.frame(Vec::new());
        h.click(aurora_i18n::tr!("about-include-location"));
        assert!(h.about.ui.includes_location());
        h.about.ui.open(&h.ctx, about::Tab::Info, &mut h.open);
        assert!(
            !h.about.ui.includes_location(),
            "explicit About reopen also resets an already visible window"
        );
    }

    #[test]
    fn about_real_egui_actual_region_readiness_and_rlva_changes_reset_without_revival() {
        let mut h = Harness::new(1.0);
        h.click(aurora_i18n::tr!("about-include-location"));
        assert!(h.about.ui.includes_location());
        let handle = h.world.main_region.unwrap();
        Arc::make_mut(h.world.regions.get_mut(&handle).unwrap().info.as_mut().unwrap()).region_id = Uuid::from_u128(105);
        let output = h.frame(Vec::new());
        assert!(!h.about.ui.includes_location() && !shown(&output, PLACE));
        h.frame(Vec::new());
        h.click(aurora_i18n::tr!("about-include-location"));
        assert!(h.about.ui.includes_location());
        let region = h.world.main_region.take();
        let output = h.frame(Vec::new());
        assert!(!h.about.ui.includes_location() && !shown(&output, PLACE));
        h.world.main_region = region;
        h.frame(Vec::new());
        h.frame(Vec::new());
        assert!(!h.about.ui.includes_location());
        h.click(aurora_i18n::tr!("about-include-location"));
        assert!(h.about.ui.includes_location());
        restriction(&mut h.world, "@showloc=n");
        let output = h.frame(Vec::new());
        assert!(!h.about.ui.includes_location() && !shown(&output, SIM));
        restriction(&mut h.world, "@showloc=y");
        h.frame(Vec::new());
        h.frame(Vec::new());
        assert!(!h.about.ui.includes_location());
        h.click(aurora_i18n::tr!("about-include-location"));
        assert!(h.about.ui.includes_location());
        h.world.teleporting = true;
        h.frame(Vec::new());
        assert!(!h.about.ui.includes_location());
        h.world.teleporting = false;
        h.frame(Vec::new());
        h.frame(Vec::new());
        assert!(!h.about.ui.includes_location());
    }
    #[test]
    fn about_actual_bridge_receive_same_drain_region_roundtrip_cannot_preserve_opt_in() {
        let mut h = Harness::new(1.0);
        h.click(aurora_i18n::tr!("about-include-location"));
        assert!(h.about.ui.includes_location());
        let original = LocationContext::observe(&h.world, true);
        let handle = h.world.main_region.unwrap();
        let other = handle.checked_add(256u64 << 32).unwrap();
        let mut bridge = crate::world::bridge::Bridge::default();
        // Exactly the real receive->World application path; no draw between
        // events. MainRegionChanged is consumed by World (returns None), so an
        // on_app_event-only observer would miss this boundary entirely.
        for (index, event) in [
            aurora_net::NetEvent::MainRegionChanged { handle: other },
            aurora_net::NetEvent::MainRegionChanged { handle },
            aurora_net::NetEvent::AgentMovementComplete {
                handle,
                position: glam::Vec3::new(128.0, 128.0, 25.0),
                look_at: glam::Vec3::X,
            },
        ]
        .into_iter()
        .enumerate()
        {
            let now = Instant::now();
            let app_event = bridge.receive(&mut h.world, event, now, now);
            if index < 2 {
                assert!(app_event.is_none(), "World consumes the region boundary");
            }
            h.about.observe_location(LocationContext::observe(&h.world, true));
        }
        assert!(
            LocationContext::observe(&h.world, true) == original,
            "final source facts match: only per-event observation sees the roundtrip"
        );
        assert!(!h.about.ui.includes_location());
        let output = h.frame(Vec::new());
        assert!(!shown(&output, PLACE) && !shown(&output, SIM));
        let output = h.copy();
        assert!(!copied(&output)[0].contains(PLACE) && !copied(&output)[0].contains(SIM));
    }
}
