use super::*;
use crate::world::favorites::{GroupFixture, group_fixture};
use egui::{Event, PointerButton};

fn text_rect(shape: &egui::Shape, text: &str) -> Option<Rect> {
    match shape {
        egui::Shape::Text(text_shape) if text_shape.galley.text() == text && !text_shape.galley.elided => {
            Some(Rect::from_min_size(text_shape.pos, text_shape.galley.size()))
        }
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_rect(shape, text)),
        _ => None,
    }
}
fn visible(output: &egui::FullOutput, text: &str) -> bool {
    output.shapes.iter().any(|shape| text_rect(&shape.shape, text).is_some())
}
fn pointer(position: Pos2, button: PointerButton, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(position),
        Event::PointerButton {
            pos: position,
            button,
            pressed,
            modifiers: Default::default(),
        },
    ]
}
struct Harness {
    ctx: egui::Context,
    palette: Palette,
    icons: Icons,
    data: GroupFixture,
    mode: usize,
    time: f64,
}
impl Harness {
    fn new(mode: usize, zoom: f32) -> Self {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::DEFAULT);
        ctx.set_zoom_factor(zoom);
        let icons = Icons::load(&ctx, None);
        let mut data = group_fixture();
        data.save(10, "Trips");
        data.save(12, "Trips");
        let mut h = Self {
            ctx,
            palette: theme.palette(),
            icons,
            data,
            mode,
            time: 0.0,
        };
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        h
    }
    fn view(&mut self) -> View {
        let ids = self.data.favorites.list(&self.data.inventory).to_vec();
        View {
            list: ids
                .into_iter()
                .map(|id| {
                    let item = &self.data.inventory.items[&id];
                    Favorite {
                        item: id,
                        asset: item.asset_id,
                        name: item.name.clone(),
                        group: self
                            .data
                            .favorites
                            .group_name(self.data.account, self.data.session, &self.data.inventory, id)
                            .map(str::to_owned),
                        group_ticket: self
                            .data
                            .favorites
                            .group_ticket(self.data.account, self.data.session, &self.data.inventory, id),
                        region: format!("Synthetic region {}", id.as_u128()),
                        rating: "",
                        global: Some(DVec3::new(1.0, 2.0, 3.0)),
                        slurl: Some("https://example.invalid/landmark".into()),
                        thumbnail: None,
                    }
                })
                .collect(),
            access: [FavoritesAccess::Menu, FavoritesAccess::Row, FavoritesAccess::Dock][self.mode],
            group_status: self.data.favorites.group_status(),
            ..Default::default()
        }
    }
    fn frame(&mut self, events: Vec<Event>) -> (egui::FullOutput, Option<FavAction>) {
        self.time += 0.1;
        let view = self.view();
        let mut action = None;
        let output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1500.0, 1200.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |_| {
                if self.mode == 1 {
                    if let (_, Some(BarAction::Favorite(found))) = row(
                        &self.ctx,
                        &self.palette,
                        &self.icons,
                        &view,
                        Rect::from_min_size(egui::pos2(20.0, 40.0), Vec2::new(900.0, 30.0)),
                    ) {
                        action = Some(found);
                    }
                } else {
                    egui::CentralPanel::default().show(&self.ctx, |ui| {
                        if self.mode == 0 {
                            menu_body(ui, &self.palette, &self.icons, &view, &mut action);
                        } else {
                            grid_body(ui, &self.palette, &self.icons, &view, &mut action);
                        }
                    });
                }
            },
        );
        (crate::ui::login::test_output(output), action)
    }
    fn position(&mut self, text: &str) -> Pos2 {
        let (output, _) = self.frame(Vec::new());
        output
            .shapes
            .iter()
            .find_map(|shape| text_rect(&shape.shape, text))
            .unwrap_or_else(|| panic!("missing Favorite control: {text}"))
            .center()
    }
    fn click(&mut self, text: &str, button: PointerButton) -> (egui::FullOutput, Option<FavAction>) {
        let position = self.position(text);
        self.frame(pointer(position, button, true));
        self.frame(pointer(position, button, false))
    }
    fn open_folder(&mut self) {
        assert!(self.click("Trips", PointerButton::Primary).1.is_none());
        let mut output = self.frame(Vec::new()).0;
        for _ in 0..5 {
            if visible(&output, "First landmark") && visible(&output, "Last landmark") {
                break;
            }
            output = self.frame(Vec::new()).0;
        }
        assert!(visible(&output, "First landmark") && visible(&output, "Last landmark"));
    }
    fn open_editor(&mut self) {
        self.open_folder();
        assert!(self.click("First landmark", PointerButton::Secondary).1.is_none());
        self.click(aurora_i18n::tr!("favorites-folder-set"), PointerButton::Primary);
        self.frame(Vec::new());
    }
    fn edit_name(&mut self, text: &str) {
        let editor = self
            .ctx
            .data(|data| data.get_temp::<GroupEditor>(Id::unique("favorite_folder_editor")))
            .unwrap();
        let input = Id::unique(("favorite_group_input", editor.ticket.as_ref().map(GroupEditTicket::namespace)));
        let position = self.ctx.read_response(input).expect("real group TextEdit").rect.center();
        self.frame(pointer(position, PointerButton::Primary, true));
        self.frame(pointer(position, PointerButton::Primary, false));
        let command = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        self.frame(vec![
            Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: command,
            },
            Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: command,
            },
            Event::Key {
                key: egui::Key::Backspace,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            },
            Event::Key {
                key: egui::Key::Backspace,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Default::default(),
            },
            Event::Text(text.into()),
        ]);
    }
}

#[test]
fn favorites_group_projection_keeps_first_original_rank_and_member_order_through_filters() {
    let mut h = Harness::new(0, 1.0);
    let view = h.view();
    let entries = grouped_entries(&view, "");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].rank(), 0);
    assert_eq!(entries[1].rank(), 1);
    let Entry::Folder { name, members, .. } = &entries[0] else {
        panic!("folder entry")
    };
    assert_eq!(*name, "Trips");
    assert_eq!(
        members.iter().map(|(rank, favorite)| (*rank, favorite.item)).collect::<Vec<_>>(),
        vec![(0, Uuid::from_u128(10)), (2, Uuid::from_u128(12))]
    );
    let matched = grouped_entries(&view, "Last landmark");
    let Entry::Folder { rank, members, .. } = &matched[0] else {
        panic!("filtered folder")
    };
    assert_eq!(*rank, 0, "folder does not jump to its first filtered member");
    assert_eq!(members[0].0, 2);
    assert_eq!(grouped_entries(&view, "TRIPS")[0].label(), "Trips");
    assert!(grouped_entries(&view, "absent").is_empty());
}

#[test]
fn favorites_real_egui_grouped_end_drop_uses_full_original_list_not_last_heading_rank() {
    let mut h = Harness::new(0, 1.0);
    let view = h.view();
    let entries = grouped_entries(&view, "");
    // Presentation ranks are [0, 1], but the folder also owns flat item 2.
    let rects = entries
        .iter()
        .enumerate()
        .map(|(row, entry)| {
            (
                entry.rank(),
                Rect::from_min_size(egui::pos2(20.0, 30.0 + row as f32 * 40.0), Vec2::new(340.0, 40.0)),
            )
        })
        .collect::<Vec<_>>();
    let position = egui::pos2(80.0, 130.0);
    h.frame(pointer(position, PointerButton::Primary, true));
    egui::DragAndDrop::set_payload(
        &h.ctx,
        Dragged {
            target: drag::Target::Landmark(Uuid::from_u128(10)),
            favorite: Some(Uuid::from_u128(10)),
            name: "First landmark".into(),
            thumbnail: None,
        },
    );
    let mut action = None;
    let _ = h.ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))),
            time: Some(h.time + 0.1),
            events: pointer(position, PointerButton::Primary, false),
            ..Default::default()
        },
        |_| {
            egui::CentralPanel::default().show(&h.ctx, |ui| {
                action = drop_on(
                    ui,
                    &h.palette,
                    Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(390.0, 150.0)),
                    &rects,
                    true,
                    view.list.len(),
                );
            });
        },
    );
    assert_eq!(action, Some(FavAction::Move(Uuid::from_u128(10), 3)));
    if let Some(FavAction::Move(item, rank)) = action {
        h.data.favorites.rearrange(&h.data.inventory, item, rank);
    }
    assert_eq!(h.data.favorites.list(&h.data.inventory).last(), Some(&Uuid::from_u128(10)));
    assert_eq!(h.data.name(10), Some("Trips"));
    assert_eq!(h.data.name(12), Some("Trips"));
}

#[test]
fn favorites_real_egui_folder_buttons_in_all_layouts_open_exact_member_actions_without_teleport() {
    for zoom in [1.0, 1.5] {
        for mode in 0..3 {
            let mut h = Harness::new(mode, zoom);
            let (output, _) = h.frame(Vec::new());
            assert!(visible(&output, "Trips") && visible(&output, "Middle landmark"));
            assert!(!visible(&output, "First landmark") && !visible(&output, "Last landmark"));
            h.open_folder();
            let (_, action) = h.click("Last landmark", PointerButton::Primary);
            assert_eq!(action, Some(FavAction::Teleport(Uuid::from_u128(112))));
        }
    }
}
#[test]
fn favorites_real_egui_captured_name_apply_and_empty_ungroup_preserve_inventory_and_order() {
    let mut h = Harness::new(0, 1.0);
    let order = h.data.favorites.list(&h.data.inventory).to_vec();
    let original = h.data.inventory.items[&Uuid::from_u128(10)].clone();
    h.open_editor();
    h.edit_name("New menu");
    let (_, action) = h.click(aurora_i18n::tr!("common-apply"), PointerButton::Primary);
    let Some(FavAction::AssignGroup { ticket, name }) = action else {
        panic!("typed captured group action")
    };
    assert_eq!(name, "New menu");
    h.data
        .favorites
        .assign_group(ticket, h.data.account, h.data.session, &h.data.inventory, &name)
        .unwrap();
    h.data.settle();
    assert_eq!(h.data.name(10), Some("New menu"));
    assert_eq!(h.data.name(12), Some("Trips"));
    assert_eq!(h.data.favorites.list(&h.data.inventory), order.as_slice());
    assert_eq!(h.data.inventory.items[&Uuid::from_u128(10)], original);
    let mut empty = Harness::new(0, 1.0);
    empty.open_editor();
    empty.edit_name("");
    let (_, action) = empty.click(aurora_i18n::tr!("common-apply"), PointerButton::Primary);
    let Some(FavAction::AssignGroup { ticket, name }) = action else {
        panic!("typed empty assignment")
    };
    assert!(name.is_empty());
    empty
        .data
        .favorites
        .assign_group(ticket, empty.data.account, empty.data.session, &empty.data.inventory, &name)
        .unwrap();
    empty.data.settle();
    assert_eq!(empty.data.name(10), None);
    assert_eq!(empty.data.favorites.list(&empty.data.inventory), order.as_slice());
    let view = empty.view();
    let entries = grouped_entries(&view, "");
    assert!(matches!(entries[0], Entry::Item(0, _)));
    assert_eq!(entries[2].label(), "Trips");
}
#[test]
fn favorites_real_egui_held_apply_cannot_substitute_a_replaced_source_or_previous_assignment() {
    for replace_source in [true, false] {
        let mut h = Harness::new(0, 1.0);
        h.open_editor();
        h.edit_name("Never apply elsewhere");
        let position = h.position(aurora_i18n::tr!("common-apply"));
        h.frame(pointer(position, PointerButton::Primary, true));
        if replace_source {
            h.data.replace_source(Uuid::from_u128(999));
        } else {
            h.data.save(10, "Changed assignment");
        }
        let (_, action) = h.frame(pointer(position, PointerButton::Primary, false));
        assert!(action.is_none());
        assert_eq!(h.data.name(10), Some(if replace_source { "Trips" } else { "Changed assignment" }));
        assert_eq!(h.data.name(12), Some("Trips"));
    }
}
#[test]
fn favorites_real_egui_cancel_emits_no_assignment_and_leaves_published_name() {
    let mut h = Harness::new(0, 1.0);
    h.open_editor();
    h.edit_name("Discarded");
    let (_, action) = h.click(aurora_i18n::tr!("common-cancel"), PointerButton::Primary);
    assert!(action.is_none());
    assert_eq!(h.data.name(10), Some("Trips"));
}
