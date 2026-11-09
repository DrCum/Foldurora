//! Full-inventory instances through real egui frames, including their owners
//! after closing and the large-inventory cost of several simultaneous views.

use super::*;
use egui::{Event, Id, Pos2, Rect};

struct Fixture {
    ctx: egui::Context,
    world: World,
    st: InventoryUi,
    prefs: InventoryPreferences,
    classic: bool,
    open: bool,
    frame: u64,
}

impl Fixture {
    fn new(classic: bool) -> Self {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = crate::demo::DEMO_AGENT;
        world.inventory.root = Uuid::from_u128(1);
        world.inventory.owner = world.agent_id;
        world.inventory.created_folders(vec![aurora_net::inventory::InvFolder {
            id: world.inventory.root,
            name: "Inventory".into(),
            type_default: 8,
            ..Default::default()
        }]);
        world.inventory.folders.get_mut(&world.inventory.root).unwrap().state = FetchState::Fetched;
        crate::world::appearance::seed_demo(&mut world.inventory, world.agent_id);
        crate::world::inventory::demo::seed(&mut world.inventory, world.agent_id);
        let mut f = Self {
            ctx,
            world,
            st: InventoryUi::default(),
            prefs: InventoryPreferences::default(),
            classic,
            open: true,
            frame: 0,
        };
        f.settle();
        f
    }

    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.frame += 1;
        headless_output(self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0))),
                time: Some(self.frame as f64 / 60.0),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                show(
                    ui.ctx(),
                    &crate::theme::Theme::default().palette(),
                    &Icons::default(),
                    &mut self.world,
                    &mut self.st,
                    &mut self.prefs,
                    &mut self.open,
                    false,
                    &Default::default(),
                    &Default::default(),
                    &Default::default(),
                    &Default::default(),
                    self.classic,
                );
            },
        ))
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            self.frame(Vec::new());
        }
    }

    fn add(&mut self) -> Uuid {
        self.st.open_additional_window(&self.ctx);
        self.st.windows.last().expect("new window").id
    }

    fn rect(&self, id: Uuid) -> Rect {
        self.ctx.memory(|m| m.area_rect(Id::unique(floater_id(id))).expect("drawn window"))
    }

    fn click_new(&mut self) {
        // The main control may be covered by a cascaded window.
        self.ctx
            .move_to_top(egui::LayerId::new(egui::Order::Middle, Id::unique("inventory")));
        self.settle();
        let pos = self
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::unique("inventory_additional_button")))
            .unwrap()
            .center();
        for pressed in [true, false] {
            self.frame(vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]);
        }
        self.settle();
    }
}

#[test]
fn inventory_new_window_button_creates_instances_in_both_styles() {
    for classic in [false, true] {
        let mut f = Fixture::new(classic);
        for count in 1..=3 {
            f.click_new();
            assert_eq!(f.st.windows.len(), count, "classic={classic}");
            assert!(
                f.st.windows
                    .iter()
                    .all(|w| w.additional && w.open && w.root.is_nil() && w.state.scope.is_nil())
            );
        }
        let ids: HashSet<_> = f.st.windows.iter().map(|w| w.id).collect();
        assert_eq!(ids.len(), 3);
        assert!(!ids.contains(&Uuid::nil()));
        assert!(f.st.windows.iter().all(|w| w.state.ui_id == w.id));

        // The extras still have their tabs, but only the primary owns Received
        // and the create-window control (including when it is closed).
        f.open = false;
        f.ctx.data_mut(|d| d.remove::<Rect>(Id::unique("inventory_additional_button")));
        let output = f.frame(Vec::new());
        fn contains(shape: &egui::Shape, text: &str) -> bool {
            match shape {
                egui::Shape::Text(t) => t.galley.text() == text,
                egui::Shape::Vec(shapes) => shapes.iter().any(|s| contains(s, text)),
                _ => false,
            }
        }
        let received = if classic {
            aurora_i18n::tr!("ui-inventory-received-purchases")
        } else {
            aurora_i18n::tr!("inventory-tab-received")
        };
        assert!(!output.shapes.iter().any(|s| contains(&s.shape, received)));
        assert!(
            f.ctx
                .data(|d| d.get_temp::<Rect>(Id::unique("inventory_additional_button")))
                .is_none()
        );
        f.add();
        f.settle();
        assert_eq!(f.st.windows.len(), 4, "the shortcut's action also works with the primary closed");
    }
}

#[test]
fn inventory_many_windows_keep_independent_state_and_lifetimes() {
    for classic in [false, true] {
        for count in [3, 8, 16] {
            let mut f = Fixture::new(classic);
            for _ in 1..count {
                f.add();
            }
            f.settle();
            for (i, w) in f.st.windows.iter_mut().enumerate() {
                let item = Uuid::from_u128(8100 + (i % 3) as u128);
                w.state.show_original(&f.world.inventory, item);
                w.state.search = format!("search-{i}");
                w.state.filters.permissions = rules::COPY;
                w.state.plain.set_open(Uuid::from_u128(8000), i % 2 == 0);
            }
            f.settle();
            let id = f.add();
            f.settle();
            f.frame(vec![Event::Text("new search".into())]);
            assert_eq!(f.st.windows.last().unwrap().state.search, "new search", "new search owns focus");
            for (i, w) in f.st.windows[..count - 1].iter().enumerate() {
                assert_eq!(w.state.search, format!("search-{i}"));
                assert_eq!(w.state.selected, Some(Uuid::from_u128(8100 + (i % 3) as u128)));
                assert_eq!(w.state.filters.permissions, rules::COPY);
                assert_eq!(w.state.plain.is_open(Uuid::from_u128(8000)), i % 2 == 0);
            }
            assert_eq!(f.st.windows.last().unwrap().state.filters, f.prefs.filter_defaults);
            let before = f.rect(id);
            widgets::minimize_floater(&f.ctx, &floater_id(id));
            f.settle();
            f.add();
            f.settle();
            assert!(widgets::floater_minimized(&f.ctx, &floater_id(id)), "new means new, not restore");
            widgets::expand_floater(&f.ctx, &floater_id(id));
            f.settle();
            assert!((f.rect(id).size() - before.size()).length() < 2.0);
            let middle = f.st.windows[count / 2].id;
            widgets::close_floater(&f.ctx, &floater_id(middle));
            f.settle();
            assert!(!f.st.windows.iter().any(|w| w.id == middle));
            assert!(f.st.windows.iter().any(|w| w.id == id && w.open));
            // Same close-all operation used by @showinv. Closing the main
            // alone above did not close any of the extra views.
            f.st.close_additional_windows();
            f.settle();
            assert!(f.st.windows.is_empty(), "closed views and their caches are dropped");
            f.add();
            f.settle();
            assert!(f.st.windows[0].state.search.is_empty());
            assert!(!widgets::floater_minimized(&f.ctx, &floater_id(f.st.windows[0].id)));
        }
    }
}

#[test]
fn inventory_new_windows_cascade_without_moving_existing_views() {
    for classic in [false, true] {
        let mut f = Fixture::new(classic);
        let first = f.add();
        f.settle();
        let main = f.ctx.memory(|m| m.area_rect(Id::unique("inventory")).unwrap());
        let a = f.rect(first);
        assert!(a.right() < main.left(), "the first extra opens beside the main");
        let second = f.add();
        f.settle();
        let b = f.rect(second);
        assert!((b.min - a.min - Vec2::splat(32.0)).length() < 2.0);
        assert_eq!(f.rect(first), a);
        widgets::minimize_floater(&f.ctx, &floater_id(second));
        f.settle();
        let third = f.add();
        f.settle();
        assert!((f.rect(third).min - a.min - Vec2::splat(32.0)).length() < 2.0);
        for _ in 4..=16 {
            f.add();
        }
        f.settle();
        let work = crate::ui::layout::work_area(&f.ctx);
        for w in &f.st.windows {
            assert!(work.expand(1.0).contains_rect(f.rect(w.id)), "window {} stays reachable", w.id);
        }
    }
}

#[test]
fn inventory_closed_windows_keep_pending_documents_and_confirmations_separate() {
    let mut f = Fixture::new(true);
    let first = f.add();
    let second = f.add();
    f.add();
    f.settle();
    let request = Uuid::new_v4();
    let note = &mut f.st.windows[0].state;
    note.preview = Some(Uuid::from_u128(8103));
    note.load_document(7, true, &Ok(Vec::new()));
    note.preview_text = Some(Ok("Retained draft".into()));
    note.document_changed();
    note.start_content_save(request, 7, "Retained draft".into(), true).unwrap();
    f.st.windows[1].state.dialog = Some(EditDialog::Delete(vec![Uuid::from_u128(8100)], false));
    f.st.close_additional_windows();
    f.settle();
    assert_eq!(f.st.windows.len(), 2);
    assert!(f.st.windows.iter().all(|w| !w.open));
    let newest = f.add();
    assert_ne!(newest, first);
    assert_ne!(newest, second);
    assert!(!f.st.windows.last_mut().unwrap().state.finish_content_save(request, None));
    let note = &mut f.st.windows.iter_mut().find(|w| w.id == first).unwrap().state;
    assert!(note.save_pending && note.preview_dirty);
    assert!(note.finish_content_save(request, None));
    f.settle();
    assert!(!f.st.windows.iter().any(|w| w.id == first));
    assert!(f.st.windows.iter().any(|w| w.id == second && w.state.dialog.is_some()));
    f.st.windows.iter_mut().find(|w| w.id == second).unwrap().state.dialog = None;
    f.settle();
    assert_eq!(f.st.windows.len(), 1);
    assert_eq!(f.st.windows[0].id, newest);
}

/// Diagnostic, not a timing assertion: run alone with --ignored --nocapture
/// --test-threads=1. RSS includes allocator-retained pages, not just live caches.
#[test]
#[ignore = "large-inventory timing and memory diagnostic"]
fn inventory_multi_window_profile() {
    use std::time::Instant;
    fn rss_mib() -> u64 {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
        let mut system = System::new();
        let pid = sysinfo::get_current_pid().unwrap();
        system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing().with_memory());
        system.process(pid).unwrap().memory() / (1024 * 1024)
    }
    for classic in [true, false] {
        for items in [30_000, 200_000] {
            let mut f = Fixture::new(classic);
            let expanded = crate::world::inventory::demo::seed_big(&mut f.world.inventory, f.world.agent_id, items);
            f.st.open_folders(expanded.iter().copied());
            f.settle();
            assert!(f.st.plain.rows.len() > 1000, "the benchmark must draw a populated inventory");
            for count in [1, 3, 8, 16] {
                let start = Instant::now();
                while f.st.windows.len() + 1 < count {
                    f.add();
                    f.st.windows.last_mut().unwrap().state.open_folders(expanded.iter().copied());
                }
                f.frame(Vec::new());
                let opening = start.elapsed().as_secs_f64() * 1000.0;
                f.settle();
                assert!(f.st.windows.iter().all(|w| w.state.plain.rows.len() > 1000));
                let start = Instant::now();
                for _ in 0..10 {
                    f.frame(Vec::new());
                }
                let idle = start.elapsed().as_secs_f64() * 100.0;
                let mut update = 0.0;
                for n in 0..3 {
                    let mut item = f.world.inventory.items[&crate::world::inventory::demo::BIG_LINKED].clone();
                    item.desc = format!("Update {n}");
                    f.world.inventory.server_updated(Vec::new(), vec![item]);
                    let start = Instant::now();
                    f.frame(Vec::new());
                    update += start.elapsed().as_secs_f64() * 1000.0 / 3.0;
                }
                println!(
                    "inventory windows={count} items={items} classic={classic} opening_ms={opening:.2} idle_ms={idle:.2} update_ms={update:.2} rss_mib={}",
                    rss_mib()
                );
            }
            f.st.close_additional_windows();
            f.settle();
            assert!(f.st.windows.is_empty());
            println!("inventory closed extras items={items} classic={classic} rss_mib={}", rss_mib());
        }
    }
}
