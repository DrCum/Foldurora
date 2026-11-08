use super::*;
use crate::world::inventory_archive::{controller::testing as controller_testing, visibility::CpuTurn};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn world(flags: &[Uuid]) -> World {
    let mut world = crate::world::inventory_archive::tests::native_fixture_world();
    world.inventory.folders.get_mut(&id(8)).unwrap().library = true;
    for (parent, children) in [(1, vec![2, 4, 5]), (2, vec![3]), (5, vec![6]), (7, vec![8])] {
        world.inventory.folders.get_mut(&id(parent)).unwrap().children = children.into_iter().map(id).collect();
    }
    for (folder, name) in [
        (2, "Archive target"),
        (3, "Archived child"),
        (4, "Visible outfit"),
        (5, "Trash"),
        (6, "Recovery folder"),
    ] {
        world.inventory.folders.get_mut(&id(folder)).unwrap().info.name = name.into();
    }
    world.inventory_archive = Some(controller_testing::native_loaded(&world, flags));
    let mut turn = CpuTurn::default();
    let mut state = State::default();
    for _ in 0..256 {
        turn.advance();
        world.pump_inventory_archive(&mut turn);
        if state.read(&world).unwrap().status() == VisibilityStatus::Ready {
            return world;
        }
    }
    panic!("actual bounded native fixture projection did not publish");
}
fn order(world: &World, binding: Option<VisibilityBinding>, exempt: bool) -> SourceOrder {
    SourceOrder {
        generation: world.inventory.generation,
        root: world.inventory.root,
        agent: world.agent_id,
        inventory: world.inventory_archive_instance,
        profile: crate::session_host::profile::root(),
        login: world
            .login
            .as_ref()
            .map(|login| (login.agent_id, login.session_id, login.inventory_root)),
        binding,
        exempt,
    }
}

#[test]
fn inventory_archive_ui_actual_cache_always_folders_and_ancestor_closure_cannot_resurrect_hidden_rows() {
    let world = world(&[id(2)]);
    let mut state = State::default();
    let instance = state.instance();
    let read = state.read(&world).unwrap();
    let gate = State::ordinary(instance, Some(&read), world.inventory_archive_instance);
    let mut cache = view::ViewCache::default();
    let prefs = InventoryPreferences {
        search_trash: true,
        search_library: true,
        search_outfits: true,
        ..Default::default()
    };
    let filters = view::Filters {
        always_folders: true,
        types: 0,
        ..Default::default()
    };
    cache.search_gated(
        &world.inventory,
        world.inventory.root,
        "",
        &prefs,
        &filters,
        0,
        None,
        &Facts::default(),
        world.agent_id,
        view::now(),
        gate,
    );
    for hidden in [id(2), id(3), id(9)] {
        assert!(!cache.results.contains(&hidden));
        assert!(!cache.result_folders().any(|parent| parent == hidden));
    }
    assert!(cache.results.contains(&id(4)));
    assert!(cache.results.contains(&id(6)), "Trash recovery is visible");
    assert!(cache.results.iter().all(|id| gate.passes(*id)));
    let mut tree = view::TreeState::default();
    tree.set_all(&cache, world.inventory.root, 0, true);
    tree.prepare(&cache, world.inventory.root, 0, true);
    assert!(tree.rows.iter().all(|row| gate.passes(row.id)));
}

#[test]
fn inventory_archive_ui_sidebar_and_recent_received_counters_invalidate_include_without_inventory_change_or_throttle() {
    let mut world = world(&[id(2)]);
    let now = view::now();
    world.inventory.items.get_mut(&id(9)).unwrap().created_at = now - 20;
    world.inventory.items.get_mut(&id(9)).unwrap().favorite = true;
    let mut visible = world.inventory.items[&id(9)].clone();
    visible.id = id(10);
    visible.parent = id(4);
    visible.favorite = false;
    world.inventory.items.insert(id(10), visible);
    let generation = world.inventory.generation;
    let mut state = State::default();
    let instance = state.instance();
    let prefs = InventoryPreferences {
        last_logout: now - 1000,
        search_outfits: true,
        ..Default::default()
    };
    let mut counts = aurora::Counts::default();
    let mut view = view::ViewCache::default();
    let mut sidebar = aurora::Sidebar::default();
    {
        let read = state.read(&world).unwrap();
        let gate = State::ordinary(instance, Some(&read), world.inventory_archive_instance);
        counts.update_gated(&world.inventory, &prefs, 0, gate);
        assert_eq!(counts.recent, 1);
        assert_eq!(counts.favorites, 1);
        view.prepare_gated(&world.inventory, prefs.sort, gate);
        sidebar.fixture_prepare(&view, &world.inventory, world.inventory.root, true);
        assert!(!sidebar.fixture_ids().contains(&id(2)));
    }
    assert!(
        world
            .inventory_archive
            .as_ref()
            .unwrap()
            .set_include(&world, &mut state.policy, true)
    );
    let read = state.read(&world).unwrap();
    let gate = State::ordinary(instance, Some(&read), world.inventory_archive_instance);
    counts.update_gated(&world.inventory, &prefs, 0, gate);
    assert_eq!(counts.recent, 2);
    assert_eq!(counts.favorites, 1);
    view.prepare_gated(&world.inventory, prefs.sort, gate);
    sidebar.fixture_prepare(&view, &world.inventory, world.inventory.root, true);
    assert!(sidebar.fixture_ids().contains(&id(2)));
    assert_eq!(world.inventory.generation, generation);
}

#[test]
fn inventory_archive_ui_browse_demands_skip_hidden_branches_and_share_sixteen_source_bound_admissions() {
    let mut world = world(&[id(2)]);
    let mut state = State::default();
    let instance = state.instance();
    let mut demands = Demands::new(&world.inventory);
    let source;
    {
        let read = state.read(&world).unwrap();
        let gate = State::ordinary(instance, Some(&read), world.inventory_archive_instance);
        let mut browse = Browse {
            inventory: &world.inventory,
            gate,
            ordinary: gate,
            demands: &mut demands,
        };
        assert!(!browse.request_tree(world.inventory.root));
        source = order(&world, read.binding(), false);
    }
    demands.apply(&mut world, &mut state, source);
    assert!(world.inventory.queue.iter().any(|(id, _)| *id == world.inventory.root));
    assert!(!world.inventory.queue.iter().any(|(folder, _)| [id(2), id(3)].contains(folder)));
    world.inventory.queue.clear();
    world.inventory.folders.get_mut(&id(1)).unwrap().state = FetchState::Unknown;
    for n in 20..60 {
        world.inventory.folders.insert(
            id(n),
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: id(n),
                    parent: world.inventory.root,
                    type_default: -1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Unknown,
                library: false,
            },
        );
        world.inventory.folders.get_mut(&id(1)).unwrap().children.push(id(n));
    }
    world.inventory.generation += 1;
    assert!(
        world
            .inventory_archive
            .as_ref()
            .unwrap()
            .set_include(&world, &mut state.policy, true)
    );
    let mut demands = Demands::new(&world.inventory);
    let source;
    {
        let read = state.read(&world).unwrap();
        let gate = State::ordinary(instance, Some(&read), world.inventory_archive_instance);
        for _ in 0..3 {
            let mut browse = Browse {
                inventory: &world.inventory,
                gate,
                ordinary: gate,
                demands: &mut demands,
            };
            browse.request_tree(world.inventory.root);
        }
        source = order(&world, read.binding(), false);
        assert_eq!(demands.pending.len(), 16);
        assert_eq!(demands.left, 0);
    }
    demands.apply(&mut world, &mut state, source);
    assert_eq!(world.inventory.queue.len(), 16);
}

#[test]
fn inventory_archive_ui_deferred_demand_is_discarded_after_actual_privacy_aba() {
    let mut world = world(&[]);
    world.rlva.configure(true);
    let mut state = State::default();
    let instance = state.instance();
    let mut demands = Demands::new(&world.inventory);
    let source;
    {
        let read = state.read(&world).unwrap();
        let gate = State::ordinary(instance, Some(&read), world.inventory_archive_instance);
        let mut browse = Browse {
            inventory: &world.inventory,
            gate,
            ordinary: gate,
            demands: &mut demands,
        };
        browse.request(id(4));
        source = order(&world, read.binding(), false);
    }
    let message = |text: &str| aurora_net::ChatMessage {
        from_name: "Synthetic scope".into(),
        source_id: id(200),
        owner_id: world.agent_id,
        source_type: aurora_net::ChatSourceType::Object,
        chat_type: aurora_net::ChatType::OwnerSay,
        position: glam::Vec3::ZERO,
        message: text.into(),
    };
    let deny = message("@showinv=n");
    let allow = message("@showinv=y");
    assert!(world.rlva.receive(&deny));
    world.observe_inventory_archive();
    assert!(world.rlva.receive(&allow));
    world.observe_inventory_archive();
    demands.apply(&mut world, &mut state, source);
    assert!(world.inventory.queue.is_empty());
    assert!(demands.pending.is_empty());
}

#[test]
fn inventory_archive_ui_hidden_rename_selection_reveal_hover_and_caches_cancel_without_finishing_edit() {
    let world = world(&[id(2)]);
    let mut owner = State::default();
    let instance = owner.instance();
    let read = owner.read(&world).unwrap();
    let gate = State::ordinary(instance, Some(&read), world.inventory_archive_instance);
    let mut state = InventoryUi {
        selected: Some(id(2)),
        anchor: Some(id(3)),
        rename: Some(InlineRename {
            id: id(2),
            name: "Do not commit".into(),
            focus: true,
        }),
        reveal: Some(id(3)),
        navigate_to: Some(id(2)),
        ..Default::default()
    };
    state.selection.extend([id(2), id(3), id(4)]);
    state.reveal_path.extend([id(1), id(2), id(3)]);
    state.wanted_images.insert(id(900));
    prune(&egui::Context::default(), &mut state, gate);
    let mut actions = Vec::new();
    finish_hidden_rename(&world.inventory, &mut state, &mut actions);
    assert!(state.rename.is_none());
    assert!(actions.is_empty());
    assert_eq!(state.selection, HashSet::from([id(4)]));
    assert!(state.selected.is_none());
    assert!(state.anchor.is_none());
    assert!(state.reveal.is_none());
    assert!(state.navigate_to.is_none());
    assert!(state.wanted_images.is_empty());
    assert_eq!(world.inventory.folders[&id(2)].info.name, "Archive target");
}

#[test]
fn inventory_archive_ui_ordinary_none_denies_while_explicit_exemptions_are_named_and_view_owners_independent() {
    let world = world(&[id(2)]);
    let mut left = State::default();
    let mut right = State::default();
    let left_id = left.instance();
    let right_id = right.instance();
    assert_ne!(left_id, right_id);
    let mut exhausted = State {
        instance: None,
        policy: OwnerPolicy::default(),
    };
    assert!(exhausted.read(&world).is_none(), "owner incarnation exhaustion denies admission");
    assert!(!State::ordinary(left_id, None, world.inventory_archive_instance).passes(id(2)));
    assert!(
        world
            .inventory_archive
            .as_ref()
            .unwrap()
            .set_include(&world, &mut left.policy, true)
    );
    let read = left.read(&world).unwrap();
    assert!(State::ordinary(left_id, Some(&read), world.inventory_archive_instance).passes(id(2)));
    let read = right.read(&world).unwrap();
    assert!(!State::ordinary(right_id, Some(&read), world.inventory_archive_instance).passes(id(2)));
    for reason in [
        Exemption::Worn,
        Exemption::Favorites,
        Exemption::Partial,
        Exemption::Picker,
        Exemption::Marketplace,
    ] {
        assert!(Gate::Exempt(reason).passes(id(2)));
    }
}

#[test]
fn inventory_archive_ui_operation_and_canonical_browse_loading_complete_independently() {
    let mut world = world(&[id(2)]);
    for folder in world.inventory.folders.values_mut() {
        folder.state = FetchState::Fetched;
    }
    world.inventory.folders.get_mut(&id(8)).unwrap().state = FetchState::Unknown;
    let mut owner = State::default();
    let instance = owner.instance();
    let mut ui = InventoryUi {
        search: "Library".into(),
        ..Default::default()
    };
    let prefs = InventoryPreferences {
        search_library: true,
        ..Default::default()
    };
    ui.begin_replace_links(id(2));
    let mut demands = Demands::new(&world.inventory);
    {
        let read = owner.read(&world).unwrap();
        let gate = State::ordinary(instance, Some(&read), world.inventory_archive_instance);
        assert!(!gate.passes(id(2)));
        assert!(gate.passes(id(8)));
        let mut browse = Browse {
            inventory: &world.inventory,
            gate,
            ordinary: gate,
            demands: &mut demands,
        };
        super::super::request_search_visible(&mut browse, world.inventory.root, &mut ui, &prefs);
    }
    assert!(ui.browse_fetch && ui.operation_fetch && ui.fetch_all);
    assert_eq!(demands.pending, vec![id(8)], "only the visible Library browse is pending");
    assert_eq!(demands.left, 15, "same sixteen source-bound browse admissions");
    super::super::fetch_whole_inventory(&mut world.inventory, &mut ui);
    assert!(!ui.operation_fetch, "personal operation completes even while search is active");
    assert!(
        ui.browse_fetch && ui.fetch_all,
        "operation completion retains the Library browse owner"
    );
    assert!(
        world.inventory.queue.is_empty(),
        "operation adds no request to the complete personal tree"
    );

    world.inventory.folders.get_mut(&id(8)).unwrap().state = FetchState::Fetched;
    world.inventory.folders.get_mut(&id(2)).unwrap().state = FetchState::Unknown;
    ui.set_operation_fetch(true);
    ui.fetch_poll = None;
    let mut completed = Demands::new(&world.inventory);
    {
        let read = owner.read(&world).unwrap();
        let gate = State::ordinary(instance, Some(&read), world.inventory_archive_instance);
        let mut browse = Browse {
            inventory: &world.inventory,
            gate,
            ordinary: gate,
            demands: &mut completed,
        };
        super::super::request_search_visible(&mut browse, world.inventory.root, &mut ui, &prefs);
    }
    assert!(!ui.browse_fetch, "hidden unknown branch cannot keep ordinary browse loading");
    assert!(
        ui.operation_fetch && ui.fetch_all,
        "browse completion retains the independent operation owner"
    );
    assert!(completed.pending.is_empty(), "ordinary browse never requests the archived branch");
    world.inventory.folders.get_mut(&id(2)).unwrap().state = FetchState::Fetched;
    super::super::fetch_whole_inventory(&mut world.inventory, &mut ui);
    assert!(!ui.operation_fetch && !ui.browse_fetch && !ui.fetch_all);
    assert!(world.inventory.queue.is_empty());
}

fn presentation_key() -> egui::Id {
    egui::Id::unique("archive_fixture_presentation")
}
fn presentation_snapshot() -> egui::Id {
    egui::Id::unique("archive_fixture_presentation_snapshot")
}
pub(super) fn presentation(ui: &mut egui::Ui) {
    match ui.ctx().data(|data| data.get_temp::<u8>(presentation_key()).unwrap_or(0)) {
        1 => ui.disable(),
        2 => ui.set_invisible(),
        _ => {}
    }
}
pub(super) fn record_presentation(ui: &egui::Ui, response: &egui::Response) {
    let actual = ui.ctx().viewport(|viewport| viewport.this_pass.widgets.get(response.id).copied());
    if let Some(actual) = actual {
        ui.ctx().data_mut(|data| data.insert_temp(presentation_snapshot(), actual));
    }
}

struct Frames {
    frames: crate::ui::widgets::elide_tests::Frames,
    world: World,
    state: InventoryUi,
    prefs: InventoryPreferences,
    proposals: Vec<Proposal>,
    discard: bool,
    classic: bool,
    covered: Option<egui::Pos2>,
    late: authority_tests::Late,
    cover_original: Option<Control>,
}
impl Frames {
    fn new() -> Self {
        Self::style(true)
    }
    fn style(classic: bool) -> Self {
        let frames = crate::ui::widgets::elide_tests::Frames::new();
        crate::theme::Theme::default().apply(&frames.ctx, 1.0);
        Self {
            frames,
            world: world(&[]),
            state: InventoryUi::default(),
            prefs: InventoryPreferences::default(),
            proposals: Vec::new(),
            discard: false,
            classic,
            covered: None,
            late: authority_tests::Late::None,
            cover_original: None,
        }
    }
    fn run(&mut self, events: Vec<egui::Event>) -> Vec<crate::ui::widgets::elide_tests::Text> {
        let palette = crate::theme::Theme::default().palette();
        let before = self
            .state
            .archive_context
            .activation
            .control
            .or(self.state.archive_include_context.activation.control);
        let mode = match self.late {
            authority_tests::Late::Disable => 1_u8,
            authority_tests::Late::Hide => 2,
            _ => 0,
        };
        self.frames.ctx.data_mut(|data| {
            data.insert_temp(presentation_key(), mode);
            data.remove::<egui::WidgetRect>(presentation_snapshot());
        });
        let Self {
            frames,
            world,
            state,
            prefs,
            proposals,
            discard,
            classic,
            covered,
            late,
            cover_original,
        } = self;
        let texts = frames.run(events, |ui| {
            let actions = crate::ui::inventory::show(
                ui.ctx(),
                &palette,
                &Icons::default(),
                world,
                state,
                prefs,
                &mut true,
                false,
                &Default::default(),
                &Default::default(),
                &Default::default(),
                &Default::default(),
                *classic,
            );
            for action in actions {
                if let InvAction::Archive(proposal) = action
                    && !proposals.iter().any(|old| old.same(&proposal))
                {
                    proposals.push(*proposal);
                }
            }
            if let Some(proposal) = proposals.last() {
                let control = match proposal.intent {
                    Intent::Edit { .. } => state.archive_context.activation.control,
                    Intent::Include { .. } => state.archive_include_context.activation.control,
                };
                if let Some(control) = control {
                    late.apply(ui, control);
                }
            }
            if let Some(control) = *cover_original {
                authority_tests::Late::VisualCover.apply(ui, control);
            }
            if let Some(position) = *covered {
                egui::Area::new(egui::Id::unique("archive_later_cover"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(position - egui::vec2(20.0, 20.0))
                    .show(ui.ctx(), |ui| {
                        ui.allocate_exact_size(egui::vec2(100.0, 100.0), egui::Sense::click());
                    });
            }
            if *discard && ui.ctx().current_pass_index() == 0 {
                ui.ctx().request_discard("synthetic later widget discarded Archive release paint");
            }
        });
        if let Some(position) = *covered {
            let ctx = &frames.ctx;
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("archive_later_cover"));
            let widgets = ctx.viewport(|viewport| viewport.prev_pass.widgets.get_layer(layer).take(4).copied().collect::<Vec<_>>());
            eprintln!(
                "ARCHIVE_LATE_AREA position={position:?} current_area={:?} visible={} widgets={widgets:?} center_layer={:?}",
                egui::AreaState::load(ctx, layer.id).map(|area| area.rect()),
                ctx.memory(|memory| memory.areas().is_visible(&layer)),
                ctx.layer_id_at(position)
            );
            if let Some(proposal) = proposals.last() {
                let rect = egui::AreaState::load(ctx, layer.id).unwrap().rect();
                assert!(rect.contains(position) && rect.intersect(proposal.control.rect).is_positive());
                assert!(
                    widgets
                        .iter()
                        .any(|widget| widget.enabled && widget.visible && widget.interact_rect.contains(position))
                );
                let order = ctx.memory(|memory| memory.layer_ids().take(32).collect::<Vec<_>>());
                assert!(
                    order.iter().position(|id| *id == layer).unwrap() > order.iter().position(|id| *id == proposal.control.layer).unwrap()
                );
                assert_eq!(
                    ctx.layer_id_at(position),
                    Some(layer),
                    "actual visible covering Area is above the original control"
                );
            }
        }
        if mode != 0 {
            let actual = frames
                .ctx
                .data(|data| data.get_temp::<egui::WidgetRect>(presentation_snapshot()))
                .expect("actual targeted presentation row");
            let original = before.expect("original control before targeted presentation turnover");
            assert_eq!(actual.id, original.widget);
            assert_eq!(actual.layer_id, original.layer);
            assert_eq!(actual.rect, original.registered.rect);
            if mode == 1 {
                assert!(!actual.enabled);
            } else {
                assert!(!actual.visible);
            }
        }
        if let authority_tests::Late::Transform(transform) = *late {
            let control = state
                .archive_context
                .activation
                .control
                .or(state.archive_include_context.activation.control)
                .expect("actual original layer before final transform");
            // Write the actual public current-transform store AFTER the whole
            // UI run. Pinned set_transform_layer compares NaN to identity and
            // would panic before the consumer under test could reject it.
            frames.ctx.memory_mut(|memory| {
                memory.to_global.insert(control.layer, transform);
            });
            let actual = frames.ctx.layer_transform_to_global(control.layer).unwrap();
            assert_eq!(actual.scaling.to_bits(), transform.scaling.to_bits());
            assert_eq!(actual.translation.x.to_bits(), transform.translation.x.to_bits());
            assert_eq!(actual.translation.y.to_bits(), transform.translation.y.to_bits());
        }
        texts
    }
    fn pointer(&mut self, pos: egui::Pos2, button: egui::PointerButton, pressed: bool) {
        self.run(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
    }
    fn warm_late_cover(&mut self) {
        // Egui first sizes a new Area invisibly. Warm this SAME real Area
        // away from the original control, then remove it through normal UI.
        let position = egui::pos2(64.0, 64.0);
        self.covered = Some(position);
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("archive_later_cover"));
        for _ in 0..8 {
            self.run(Vec::new());
            let visible = self.frames.ctx.viewport(|viewport| {
                viewport
                    .prev_pass
                    .widgets
                    .get_layer(layer)
                    .any(|widget| widget.enabled && widget.visible && widget.interact_rect.contains(position))
            });
            if visible && self.frames.ctx.memory(|memory| memory.areas().is_visible(&layer)) {
                self.covered = None;
                self.run(Vec::new());
                assert!(!self.frames.ctx.memory(|memory| memory.areas().is_visible(&layer)));
                return;
            }
        }
        panic!("actual late-cover Area did not finish visible sizing before original input");
    }

    fn open_context(&mut self) -> egui::Pos2 {
        self.state.show_original(&self.world.inventory, id(2));
        let mut texts = Vec::new();
        for _ in 0..8 {
            texts = self.run(Vec::new());
        }
        let target = texts
            .iter()
            .find(|text| text.source == "Archive target")
            .expect("actual ordinary folder row")
            .rect
            .center();
        self.pointer(target, egui::PointerButton::Secondary, true);
        self.pointer(target, egui::PointerButton::Secondary, false);
        self.settled(false).rect.center()
    }
    fn settled(&mut self, include: bool) -> Control {
        let mut previous: Option<Control> = None;
        for _ in 0..64 {
            self.run(Vec::new());
            let current = if include {
                self.state.archive_include_context.activation.control
            } else {
                self.state.archive_context.activation.control
            };
            if let Some(control) = current {
                if control.committed(&self.frames.ctx)
                    && previous.is_some_and(|previous| previous.same_widget(control) && previous.structure == control.structure)
                {
                    return control;
                }
                previous = Some(control);
            }
        }
        let ctx = &self.frames.ctx;
        eprintln!(
            "ARCHIVE_SETTLE include={include} frame={} pass={} context_target={:?} ticket={} seen={:?} source_key={} popup_open={} focus={:?}",
            ctx.cumulative_frame_nr(),
            ctx.cumulative_pass_nr(),
            self.state.archive_context.target,
            self.state.archive_context.ticket.is_some(),
            self.state.archive_context.seen,
            self.state.archive_key.is_some(),
            egui::Popup::is_any_open(ctx),
            ctx.memory(|memory| memory.focused())
        );
        if let Some(control) = previous {
            eprintln!(
                "ARCHIVE_CONTROL widget={:?} layer={:?} frame={} pass={} rect={:?} registered={:?} clip={:?} captured_transform={:?} current_transform={:?} geometry={} registry={} committed={} center_layer={:?} owner_structure={:?}",
                control.widget,
                control.layer,
                control.frame,
                control.pass,
                control.rect,
                control.registered,
                control.paint_clip,
                control.transform,
                ctx.layer_transform_to_global(control.layer),
                control.geometry(ctx),
                control.registry(ctx, true),
                control.committed(ctx),
                ctx.layer_id_at(control.rect.center()),
                control.structure
            );
            let detail = ctx.viewport(|viewport| {
                format!(
                    "final_widget={:?} final_structure={:?}\n{}",
                    viewport.prev_pass.widgets.get(control.widget),
                    structure::Witness::capture(&viewport.prev_pass.widgets, &control),
                    control.structure_origin.diagnostic(&viewport.prev_pass.widgets, &control)
                )
            });
            eprintln!("ARCHIVE_FINAL {detail}");
            let layers = ctx.memory(|memory| {
                memory
                    .layer_ids()
                    .take(16)
                    .map(|layer| (layer, memory.areas().is_visible(&layer)))
                    .collect::<Vec<_>>()
            });
            for (layer, visible) in layers {
                eprintln!(
                    "ARCHIVE_LAYER {layer:?} visible={visible} area={:?} transform={:?}",
                    egui::AreaState::load(ctx, layer.id).map(|area| area.rect()),
                    ctx.layer_transform_to_global(layer)
                );
            }
        }
        panic!("actual captured Archive context control did not settle");
    }
}

#[test]
fn inventory_archive_ui_real_context_only_posts_original_proposal_and_never_attempts_metadata_io_in_paint() {
    for classic in [true, false] {
        let mut frames = Frames::style(classic);
        let position = frames.open_context();
        controller_testing::reset_assignments();
        frames.pointer(position, egui::PointerButton::Primary, true);
        frames.pointer(position, egui::PointerButton::Primary, false);
        assert_eq!(controller_testing::assignments(), 0, "real paint only posts metadata intent");
        assert_eq!(frames.proposals.len(), 1);
        let proposal = frames.proposals.pop().unwrap();
        assert!(
            frames
                .state
                .archive_proposal_current(&frames.frames.ctx, true, &frames.prefs, &proposal)
        );
        let state = frames.state.archive_owner_mut(proposal.owner).unwrap();
        let instance = state.instance();
        let read = state.read(&frames.world);
        let gate = State::ordinary(instance, read.as_ref(), frames.world.inventory_archive_instance);
        assert!(proposal.current_gate(gate));
        // This codec/load-ticket fixture owns no Worker. The post-context data
        // boundary is attempted exactly once and cannot claim native IO success.
        assert_eq!(
            proposal.dispatch(&mut frames.world),
            Err(crate::world::inventory_archive::Error::Unavailable)
        );
        assert_eq!(controller_testing::assignments(), 1);
    }
}

#[test]
fn inventory_archive_ui_original_context_press_cannot_retarget_after_source_generation_or_discarded_release_pass() {
    let mut frames = Frames::new();
    let position = frames.open_context();
    controller_testing::reset_assignments();
    frames.pointer(position, egui::PointerButton::Primary, true);
    frames.world.inventory.generation += 1;
    frames.pointer(position, egui::PointerButton::Primary, false);
    assert!(frames.proposals.is_empty());
    assert_eq!(controller_testing::assignments(), 0);
    let mut frames = Frames::new();
    let position = frames.open_context();
    frames.pointer(position, egui::PointerButton::Primary, true);
    frames.discard = true;
    frames.pointer(position, egui::PointerButton::Primary, false);
    assert!(frames.proposals.iter().all(|proposal| !frames.state.archive_proposal_current(
        &frames.frames.ctx,
        true,
        &frames.prefs,
        proposal
    )));
    assert_eq!(controller_testing::assignments(), 0);
}

#[test]
fn inventory_archive_ui_include_only_changes_original_owner_policy_after_whole_context_and_rejects_discard() {
    fn include(frames: &mut Frames) -> egui::Pos2 {
        let mut texts = Vec::new();
        for _ in 0..4 {
            texts = frames.run(Vec::new());
        }
        let visibility = texts
            .iter()
            .find(|text| text.source == aurora_i18n::tr!("inventory-archive-visibility"))
            .expect("actual Classic Visibility submenu")
            .rect
            .center();
        frames.pointer(visibility, egui::PointerButton::Primary, true);
        frames.pointer(visibility, egui::PointerButton::Primary, false);
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = frames.run(Vec::new());
        }
        texts
            .iter()
            .find(|text| text.source == aurora_i18n::tr!("inventory-archive-include"))
            .expect("actual Include control")
            .rect
            .center()
    }
    let mut frames = Frames::new();
    frames.world = world(&[id(2)]);
    let position = include(&mut frames);
    controller_testing::reset_assignments();
    frames.pointer(position, egui::PointerButton::Primary, true);
    frames.pointer(position, egui::PointerButton::Primary, false);
    assert_eq!(controller_testing::assignments(), 0);
    assert!(
        !frames.state.archive.as_ref().unwrap().policy.included(),
        "paint never mutates Include"
    );
    assert_eq!(frames.proposals.len(), 1);
    let proposal = frames.proposals.pop().unwrap();
    assert!(
        frames
            .state
            .archive_proposal_current(&frames.frames.ctx, true, &frames.prefs, &proposal)
    );
    let state = frames.state.archive_owner_mut(proposal.owner).unwrap();
    let instance = state.instance();
    let read = state.read(&frames.world);
    let gate = State::ordinary(instance, read.as_ref(), frames.world.inventory_archive_instance);
    assert!(!gate.passes(id(2)));
    let desired = proposal
        .include_desired(read.as_ref(), gate)
        .expect("same original policy/source/publication");
    assert!(state.assign_include(&frames.world, desired));
    assert!(state.policy.included());
    assert_eq!(controller_testing::assignments(), 0, "local Include cannot dispatch metadata IO");
    frames.run(Vec::new());
    assert!(frames.state.view.visible_folder(id(2)));
    let mut frames = Frames::new();
    frames.world = world(&[id(2)]);
    let position = include(&mut frames);
    frames.pointer(position, egui::PointerButton::Primary, true);
    frames.discard = true;
    frames.pointer(position, egui::PointerButton::Primary, false);
    assert!(!frames.state.archive.as_ref().unwrap().policy.included());
    assert!(frames.proposals.iter().all(|proposal| !frames.state.archive_proposal_current(
        &frames.frames.ctx,
        true,
        &frames.prefs,
        proposal
    )));
}

#[test]
fn inventory_archive_ui_late_cover_and_bounded_input_overflow_cannot_apply_original_context_release() {
    controller_testing::reset_assignments();
    let mut frames = Frames::new();
    frames.warm_late_cover();
    let position = frames.open_context();
    frames.pointer(position, egui::PointerButton::Primary, true);
    frames.covered = Some(position);
    frames.pointer(position, egui::PointerButton::Primary, false);
    assert!(frames.proposals.iter().all(|proposal| !frames.state.archive_proposal_current(
        &frames.frames.ctx,
        true,
        &frames.prefs,
        proposal
    )));
    assert_eq!(controller_testing::assignments(), 0);
    let mut frames = Frames::new();
    let position = frames.open_context();
    frames.pointer(position, egui::PointerButton::Primary, true);
    let mut events = vec![egui::Event::PointerMoved(position); 129];
    events.push(egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    frames.run(events);
    assert!(frames.proposals.is_empty());
    assert_eq!(controller_testing::assignments(), 0);
}

mod authority_tests;
