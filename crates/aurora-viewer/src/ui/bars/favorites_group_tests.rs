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
    without_tickets: bool,
    member_panel: bool,
    rlva: crate::world::rlva::Rlva,
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
            without_tickets: false,
            member_panel: false,
            rlva: Default::default(),
        };
        for _ in 0..4 {
            h.frame(Vec::new());
        }
        if mode == 0 {
            let button = h
                .ctx
                .read_response(Id::unique("bars_place_favorites"))
                .expect("real F1 Favorites button");
            let position = button.rect.center();
            assert!(h.frame(pointer(position, PointerButton::Primary, true)).1.is_none());
            assert!(h.frame(pointer(position, PointerButton::Primary, false)).1.is_none());
            for _ in 0..4 {
                h.frame(Vec::new());
            }
        }
        h
    }
    fn members() -> Self {
        let mut h = Self::new(2, 1.0);
        h.member_panel = true;
        h.without_tickets = true;
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
                        source: Some(
                            self.data
                                .favorites
                                .context_source(self.data.account, self.data.session, &self.data.inventory),
                        ),
                        item: id,
                        asset: item.asset_id,
                        name: item.name.clone(),
                        group: self
                            .data
                            .favorites
                            .group_name(self.data.account, self.data.session, &self.data.inventory, id)
                            .map(str::to_owned),
                        group_ticket: (!self.without_tickets)
                            .then(|| {
                                self.data
                                    .favorites
                                    .group_ticket(self.data.account, self.data.session, &self.data.inventory, id)
                            })
                            .flatten(),
                        region: format!("Synthetic region {}", id.as_u128()),
                        rating: "",
                        global: Some(DVec3::new(1.0, 2.0, 3.0)),
                        slurl: (!self.rlva.has("showloc")).then(|| "https://example.invalid/landmark".into()),
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
            |root_ui| {
                if self.member_panel {
                    egui::CentralPanel::default().show(root_ui, |ui| {
                        let members = view.list.iter().enumerate().collect::<Vec<_>>();
                        folder_contents(ui, &self.palette, &self.icons, &members, None, false, &mut action);
                    });
                } else if self.mode == 1 {
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
                    egui::CentralPanel::default().show(root_ui, |ui| {
                        if self.mode == 0 {
                            if let Some(BarAction::Favorite(found)) = place_button(
                                ui,
                                &self.palette,
                                &self.icons,
                                Rect::from_min_size(egui::pos2(20.0, 20.0), Vec2::new(PLACE_BUTTON_W, 30.0)),
                                &view,
                            ) {
                                action = Some(found);
                            }
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
        let painted = || {
            output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .take(48)
                .collect::<Vec<_>>()
        };
        output
            .shapes
            .iter()
            .find_map(|shape| text_rect(&shape.shape, text))
            .unwrap_or_else(|| panic!("missing Favorite control: {text}; painted: {:?}", painted()))
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
    fn open_member_context(&mut self) -> GroupEditor {
        assert!(self.click("First landmark", PointerButton::Secondary).1.is_none());
        for _ in 0..4 {
            self.frame(Vec::new());
        }
        self.ctx
            .data(|data| data.get_temp::<GroupEditor>(Id::unique("favorite_folder_editor")))
            .expect("actual member context")
    }
    fn open_editor(&mut self) {
        self.open_folder();
        let popup = if self.mode == 0 {
            let button = self.ctx.read_response(Id::unique("bars_place_favorites")).unwrap();
            egui::Popup::default_response_id(&button)
        } else {
            let view = self.view();
            let first = &view.list[0];
            let id = Id::unique((
                if self.mode == 1 {
                    "bars_favorites_folder_pill"
                } else {
                    "bars_favorites_folder_tile"
                },
                first.group.as_deref().unwrap(),
                first.group_ticket.as_ref().map(GroupEditTicket::namespace),
            ));
            egui::Popup::default_response_id(&self.ctx.read_response(id).expect("actual folder parent"))
        };
        assert!(self.click("First landmark", PointerButton::Secondary).1.is_none());
        assert!(
            egui::Popup::is_id_open(&self.ctx, popup),
            "member context keeps its actual Favorites owner"
        );
        self.click(aurora_i18n::tr!("favorites-folder-set"), PointerButton::Primary);
        self.frame(Vec::new());
        assert!(
            egui::Popup::is_id_open(&self.ctx, popup),
            "nested name editor keeps the actual Favorites dropdown"
        );
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
    let mut h = Harness::new(1, 1.0);
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
        |root_ui| {
            egui::CentralPanel::default().show(root_ui, |ui| {
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
    for mode in 0..3 {
        let mut h = Harness::new(mode, 1.0);
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
        let mut empty = Harness::new(mode, 1.0);
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

#[test]
fn favorites_real_egui_native_context_without_group_ticket_keeps_original_action_table() {
    for (label, command) in [
        (aurora_i18n::tr!("common-teleport-3"), ContextCommand::Teleport),
        (
            aurora_i18n::tr!("common-show-on-map"),
            ContextCommand::ShowOnMap(DVec3::new(1.0, 2.0, 3.0)),
        ),
        (aurora_i18n::tr!("favorites-view-edit"), ContextCommand::Edit),
        (
            aurora_i18n::tr!("common-copy-slurl"),
            ContextCommand::CopySlurl("https://example.invalid/landmark".into()),
        ),
    ] {
        let mut h = Harness::members();
        let editor = h.open_member_context();
        assert!(editor.ticket.is_none(), "metadata availability is not native context authority");
        let (_, action) = h.click(label, PointerButton::Primary);
        assert_eq!(action, context_command(&editor.favorite, command));
        assert!(h.data.favorites.context_current(
            editor.favorite.source.as_ref().unwrap(),
            h.data.account,
            h.data.session,
            &h.data.inventory,
            editor.favorite.item,
            editor.favorite.asset,
        ));
    }
}

#[test]
fn favorites_real_egui_held_native_context_without_ticket_cannot_follow_reorder_or_source_replacement() {
    for change in 0..6 {
        let mut h = Harness::members();
        let editor = h.open_member_context();
        assert!(editor.ticket.is_none());
        let owner_rect = h.ctx.read_response(editor.owner).unwrap().rect;
        let position = h.position(aurora_i18n::tr!("common-teleport-3"));
        assert!(h.frame(pointer(position, PointerButton::Primary, true)).1.is_none());
        match change {
            0 => h.data.favorites.rearrange(&h.data.inventory, Uuid::from_u128(10), 3),
            1 => h.data.replace_source(Uuid::from_u128(999)),
            2 => {
                let old = std::mem::take(&mut h.data.inventory);
                h.data.inventory.owner = old.owner;
                h.data.inventory.root = old.root;
                h.data.inventory.folders = old.folders;
                h.data.inventory.items = old.items;
                h.data.inventory.generation = old.generation;
            }
            3 => h.data.favorites = crate::world::favorites::Favorites::default(),
            4 => h.data.account = Uuid::from_u128(998),
            5 => h.data.inventory.items.get_mut(&Uuid::from_u128(10)).unwrap().asset_id = Uuid::from_u128(997),
            _ => unreachable!(),
        }
        let (output, action) = h.frame(pointer(position, PointerButton::Primary, false));
        assert!(action.is_none(), "held native action followed replacement {change}");
        assert!(
            h.ctx
                .data(|data| data.get_temp::<GroupEditor>(Id::unique("favorite_folder_editor")))
                .is_none()
        );
        assert!(!visible(&output, aurora_i18n::tr!("common-teleport-3")));
        if change == 0 {
            assert_eq!(h.view().list[0].item, Uuid::from_u128(11));
            assert_eq!(
                h.ctx.read_response(editor.owner).unwrap().rect,
                owner_rect,
                "the reused response is still a visible row, so cancellation needs item authority"
            );
        }
    }
}

#[test]
fn favorites_real_egui_fully_scrolled_member_retires_context_even_while_row_is_still_visited() {
    let mut h = Harness::members();
    let template = h.data.inventory.items[&Uuid::from_u128(12)].clone();
    for id in 20..44 {
        let mut item = template.clone();
        item.id = Uuid::from_u128(id);
        item.name = format!("Extra landmark {id}");
        item.created_at = 0;
        h.data.inventory.items.insert(item.id, item);
        h.data
            .inventory
            .folders
            .get_mut(&Uuid::from_u128(2))
            .unwrap()
            .items
            .push(Uuid::from_u128(id));
    }
    h.data.inventory.generation += 1;
    for _ in 0..4 {
        h.frame(Vec::new());
    }
    let editor = h.open_member_context();
    let command_pos = h.position(aurora_i18n::tr!("common-teleport-3"));
    assert!(h.frame(pointer(command_pos, PointerButton::Primary, true)).1.is_none());
    let before = h.ctx.read_response(editor.owner).unwrap().rect;
    let at = egui::pos2(before.left() + 2.0, before.top() + ROW_H * 5.0);
    let mut output = h
        .frame(vec![
            Event::PointerMoved(at),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -2000.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: Default::default(),
            },
        ])
        .0;
    for _ in 0..12 {
        output = h.frame(Vec::new()).0;
    }
    let after = h
        .ctx
        .read_response(editor.owner)
        .expect("ScrollArea still visits the clipped owner");
    assert!(
        after.rect.bottom() < before.top(),
        "real ScrollArea must fully scroll the owner away"
    );
    assert!(
        h.ctx
            .data(|data| data.get_temp::<GroupEditor>(Id::unique("favorite_folder_editor")))
            .is_none()
    );
    assert!(!visible(&output, aurora_i18n::tr!("common-teleport-3")));
    assert!(h.frame(pointer(command_pos, PointerButton::Primary, false)).1.is_none());
}

#[test]
fn favorites_context_dispatch_rechecks_real_source_and_item_even_without_group_metadata() {
    let mut h = Harness::members();
    h.data.favorites = crate::world::favorites::Favorites::default();
    let item = Uuid::from_u128(10);
    let asset = h.data.inventory.items[&item].asset_id;
    let source = h.data.favorites.context_source(h.data.account, h.data.session, &h.data.inventory);
    let current = |h: &Harness| {
        h.data
            .favorites
            .context_current(&source, h.data.account, h.data.session, &h.data.inventory, item, asset)
    };
    assert!(
        h.data
            .favorites
            .group_ticket(h.data.account, h.data.session, &h.data.inventory, item)
            .is_none()
    );
    assert!(current(&h));
    h.data.favorites.rearrange(&h.data.inventory, item, 3);
    assert!(current(&h), "reorder never changes the captured item authority at final dispatch");
    for field in 0..6 {
        let original = h.data.inventory.items[&item].clone();
        let owner = h.data.inventory.owner;
        let root = h.data.inventory.root;
        match field {
            0 => h.data.inventory.items.get_mut(&item).unwrap().asset_id = Uuid::from_u128(900),
            1 => h.data.inventory.items.get_mut(&item).unwrap().asset_type = 0,
            2 => h.data.inventory.items.get_mut(&item).unwrap().parent = root,
            3 => h.data.inventory.items.get_mut(&item).unwrap().owner = Uuid::from_u128(901),
            4 => h.data.inventory.owner = Uuid::from_u128(902),
            5 => {
                h.data.inventory.items.remove(&item);
            }
            _ => unreachable!(),
        }
        assert!(!current(&h), "final native dispatch accepted stale item field {field}");
        h.data.inventory.items.insert(item, original);
        h.data.inventory.owner = owner;
        assert!(current(&h));
    }
    let session = h.data.session;
    h.data.session = Uuid::from_u128(903);
    assert!(!current(&h));
    h.data.session = session;
    let old_inventory_instance = h.data.inventory.favorite_context_instance.clone();
    h.data.inventory.favorite_context_instance = std::sync::Arc::new(());
    assert!(
        !current(&h),
        "identical Inventory contents in a new model cannot adopt a captured context"
    );
    h.data.inventory.favorite_context_instance = old_inventory_instance;
    assert!(current(&h));
    let old_favorites = std::mem::take(&mut h.data.favorites);
    assert!(!current(&h), "a fresh Favorites model cannot adopt a captured context");
    h.data.favorites = old_favorites;
    assert!(current(&h));
    h.data.favorites.bind_groups(None, std::path::PathBuf::new());
    assert!(
        !current(&h),
        "source rebinding retires contexts even when group metadata is unavailable"
    );
}

#[test]
fn favorites_real_egui_cached_context_slurl_respects_actual_showloc_added_before_copy_dispatch() {
    use aurora_net::{ChatMessage, ChatSourceType, ChatType};
    for restrict_after_capture in [false, true] {
        let mut h = Harness::members();
        let mut rlva = crate::world::rlva::Rlva::default();
        rlva.configure(true);
        assert!(!rlva.has("showloc"));
        let editor = h.open_member_context();
        let original_slurl = editor.favorite.slurl.clone().unwrap();
        let (output, action) = h.click(aurora_i18n::tr!("common-copy-slurl"), PointerButton::Primary);
        assert!(
            !output
                .platform_output
                .commands
                .iter()
                .any(|command| matches!(command, egui::OutputCommand::CopyText(_)))
        );
        let Some(FavAction::Context {
            source,
            item,
            asset,
            command: ContextCommand::CopySlurl(cached),
        }) = action
        else {
            panic!("actual captured member Copy SLURL command")
        };
        assert_eq!(cached, original_slurl);
        assert!(
            h.data
                .favorites
                .context_current(&source, h.data.account, h.data.session, &h.data.inventory, item, asset)
        );
        if restrict_after_capture {
            assert!(rlva.receive(&ChatMessage {
                from_name: "Synthetic restriction issuer".into(),
                source_id: Uuid::from_u128(700),
                owner_id: h.data.account,
                source_type: ChatSourceType::Object,
                chat_type: ChatType::OwnerSay,
                position: glam::Vec3::ZERO,
                message: "@showloc=n".into(),
            }));
            assert!(rlva.has("showloc"), "apply the actual current restriction after context capture");
        }
        // Invoke the same effect boundary as App, with the current RLVa model
        // and the old context string. Only normal unrestricted flow may copy.
        crate::app::App::favorite_copy_slurl(&h.ctx, &rlva, cached);
        let output = h.frame(Vec::new()).0;
        assert_eq!(
            output
                .platform_output
                .commands
                .iter()
                .any(|command| matches!(command, egui::OutputCommand::CopyText(text) if text == &original_slurl)),
            !restrict_after_capture,
        );
        if restrict_after_capture {
            assert!(
                !output
                    .platform_output
                    .commands
                    .iter()
                    .any(|command| matches!(command, egui::OutputCommand::CopyText(_)))
            );
        }
        let mut live = Harness::members();
        live.rlva.configure(true);
        let captured = live.open_member_context();
        assert!(captured.favorite.slurl.is_some());
        let position = live.position(aurora_i18n::tr!("common-copy-slurl"));
        let copy_row = |h: &Harness| {
            let layer = h.ctx.layer_id_at(position).expect("actual member context layer");
            h.ctx.viewport(|viewport| {
                viewport
                    .prev_pass
                    .widgets
                    .get_layer(layer)
                    .find(|widget| widget.rect.contains(position) && widget.rect.height() < 40.0 && widget.rect.width() > 150.0)
                    .copied()
                    .expect("actual Copy SLURL menu row")
            })
        };
        assert!(copy_row(&live).sense.click, "the original available URL has an enabled row");
        if restrict_after_capture {
            assert!(live.rlva.receive(&ChatMessage {
                from_name: "Synthetic restriction issuer".into(),
                source_id: Uuid::from_u128(700),
                owner_id: live.data.account,
                source_type: ChatSourceType::Object,
                chat_type: ChatType::OwnerSay,
                position: glam::Vec3::ZERO,
                message: "@showloc=n".into(),
            }));
        }
        live.frame(Vec::new());
        assert_eq!(live.view().list[0].slurl.is_none(), restrict_after_capture);
        assert_eq!(
            copy_row(&live).sense.click,
            !restrict_after_capture,
            "the open context follows current availability"
        );
        let retained = live
            .ctx
            .data(|data| data.get_temp::<GroupEditor>(Id::unique("favorite_folder_editor")))
            .unwrap();
        assert_eq!(
            retained.favorite, captured.favorite,
            "availability never substitutes the captured source or payload"
        );
        let (_, action) = live.click(aurora_i18n::tr!("common-copy-slurl"), PointerButton::Primary);
        if restrict_after_capture {
            assert!(action.is_none(), "a currently hidden location cannot activate cached Copy SLURL");
        } else {
            assert_eq!(
                action,
                context_command(&captured.favorite, ContextCommand::CopySlurl(original_slurl))
            );
        }
    }
}
