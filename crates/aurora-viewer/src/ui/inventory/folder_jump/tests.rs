use super::*;
use crate::keybinds::{Action, Binding, Input, KeyBindings, Mods};
use egui::{Event, Id, Modifiers, PointerButton, vec2};
use winit::event::ElementState;
use winit::keyboard::KeyCode;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

#[test]
fn folder_jump_shortcut_keeps_configured_held_and_derived_passwall_bindings_and_exact_modifiers() {
    let mut bindings = KeyBindings::default();
    let input = Input::key(KeyCode::KeyJ);
    let ctrl = Mods {
        ctrl: true,
        ..Default::default()
    };
    assert!(shortcut::available(&bindings, &input, ctrl));
    assert!(
        !shortcut::available(&bindings, &input, Mods { shift: true, ..ctrl }),
        "Ctrl+Shift+J stays Shared Environment"
    );
    assert!(!shortcut::available(&bindings, &input, Mods::default()));
    bindings.set(Action::Import, 0, Some(Binding::captured(input.clone(), ctrl)));
    assert!(
        !shortcut::available(&bindings, &input, ctrl),
        "explicit custom Ctrl+J owns its action"
    );
    let mut bindings = KeyBindings::default();
    bindings.set(Action::Forward, 0, Some(Binding::captured(input.clone(), Mods::default())));
    assert!(
        !shortcut::available(&bindings, &input, ctrl),
        "held movement binding is not a menu-action fallback"
    );
    let mut bindings = KeyBindings::default();
    bindings.set(Action::Up, 0, Some(Binding::captured(input.clone(), Mods::default())));
    assert!(bindings.pass_wall_triggered(&input, ctrl));
    assert!(!shortcut::available(&bindings, &input, ctrl));
}

#[test]
fn folder_jump_full_owner_gate_rejects_specialized_partial_favorites_and_unpainted_or_hidden_owners() {
    let ctx = egui::Context::default();
    let mut main = InventoryUi::default();
    assert!(supported(&main));
    assert!(
        !main.jump_owner_visible(&ctx, true, Uuid::nil()),
        "open flag alone is not a displayed owner"
    );
    main.was_open = true;
    assert!(main.jump_owner_visible(&ctx, true, Uuid::nil()));
    assert!(!main.jump_owner_visible(&ctx, false, Uuid::nil()));
    main.tab = 4;
    assert!(!supported(&main));
    main.tab = 0;
    main.scope = id(20);
    assert!(!supported(&main));
    main.scope = Uuid::nil();
    main.ui_id = super::super::listings::PANEL;
    assert!(!supported(&main));
    main.ui_id = Uuid::new_v4();
    main.full_inventory = true;
    assert!(supported(&main));
    main.ui_id = Uuid::new_v4();
    assert!(supported(&main), "every genuine full owner can use Folder Jump");
    main.full_inventory = false;
    assert!(!supported(&main), "an arbitrary owner ID alone is insufficient");
    main.ui_id = Uuid::nil();
    crate::ui::widgets::hide_windows(&ctx, true, false);
    assert!(!main.jump_owner_visible(&ctx, true, Uuid::nil()));
}

#[test]
fn folder_jump_shortcut_uses_actual_inventory_focus_then_pointer_and_expires_hidden_claims() {
    let mut frames = crate::ui::widgets::elide_tests::Frames::new();
    let palette = crate::theme::Theme::default().palette();
    let (main_field, other_field) = (Id::unique("folder_jump_main_field"), Id::unique("folder_jump_other_field"));
    let mut text = [String::new(), String::new()];
    let mut render = |ctx: &egui::Context, draw_main: bool| {
        if draw_main {
            widgets::Floater::new(
                "folder_jump_owner_fixture",
                "Synthetic Inventory",
                egui::pos2(40.0, 40.0),
                vec2(300.0, 300.0),
            )
            .show(ctx, &palette, &mut true, |ui| {
                shortcut::claim(ui, Uuid::nil());
                ui.add(egui::TextEdit::singleline(&mut text[0]).id(main_field));
            });
        }
        widgets::Floater::new(
            "folder_jump_elsewhere_fixture",
            "Synthetic other field",
            egui::pos2(600.0, 40.0),
            vec2(300.0, 300.0),
        )
        .show(ctx, &palette, &mut true, |ui| {
            ui.add(egui::TextEdit::singleline(&mut text[1]).id(other_field));
        });
    };
    for _ in 0..4 {
        frames.run(Vec::new(), |ui| render(ui.ctx(), true));
    }
    let main = frames.ctx.read_response(main_field).expect("actual main field").rect.center();
    let other = frames.ctx.read_response(other_field).expect("actual other field").rect.center();
    let pointer = |position, pressed| {
        vec![
            Event::PointerMoved(position),
            Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ]
    };
    frames.run(pointer(main, true), |ui| render(ui.ctx(), true));
    frames.run(pointer(main, false), |ui| render(ui.ctx(), true));
    assert_eq!(shortcut::owner(&frames.ctx), Some(Uuid::nil()));
    frames.run(pointer(other, true), |ui| render(ui.ctx(), true));
    frames.run(pointer(other, false), |ui| render(ui.ctx(), true));
    frames.run(vec![Event::PointerMoved(main)], |ui| render(ui.ctx(), true));
    assert_eq!(
        shortcut::owner(&frames.ctx),
        None,
        "another editor keeps its focus despite pointer over Inventory"
    );
    let focus = frames.ctx.memory(|memory| memory.focused());
    if let Some(focus) = focus {
        frames.ctx.memory_mut(|memory| memory.surrender_focus(focus));
    }
    frames.run(vec![Event::PointerMoved(main)], |ui| render(ui.ctx(), true));
    assert_eq!(
        shortcut::owner(&frames.ctx),
        Some(Uuid::nil()),
        "pointer fallback requires keyboard nowhere"
    );
    for _ in 0..3 {
        frames.run(Vec::new(), |ui| render(ui.ctx(), false));
    }
    assert_eq!(shortcut::owner(&frames.ctx), None);
}

#[test]
fn folder_jump_native_keys_observe_both_space_forms_and_snapshot_orders_while_no_command_is_eligible() {
    for key in [
        Key::Named(NamedKey::Enter),
        Key::Named(NamedKey::Space),
        Key::Character(" ".into()),
        Key::Character("j".into()),
        Key::Character("J".into()),
    ] {
        for before_focus in [false, true] {
            let mut chooser = Chooser::default();
            chooser.event(&WindowEvent::Focused(true));
            assert!(!chooser.key_edge(&key, ElementState::Pressed, false, false));
            chooser.key_edge(&key, ElementState::Released, false, true);
            chooser.event(&WindowEvent::Focused(false));
            if before_focus {
                assert!(chooser.key_edge(&key, ElementState::Pressed, false, true));
            }
            chooser.event(&WindowEvent::Focused(true));
            if !before_focus {
                assert!(chooser.key_edge(&key, ElementState::Pressed, false, true));
            }
            assert!(
                chooser.key_edge(&key, ElementState::Pressed, false, false),
                "actual first refocus edge may have repeat=false"
            );
            assert!(chooser.key_edge(&key, ElementState::Pressed, true, false));
            assert!(chooser.key_edge(&key, ElementState::Pressed, true, false));
            chooser.close();
            assert!(
                chooser.key_edge(&key, ElementState::Pressed, false, false),
                "close/reopen cannot erase observational held proof"
            );
            assert!(!chooser.key_edge(&key, ElementState::Released, false, false));
            assert!(!chooser.key_edge(&key, ElementState::Pressed, false, false));
        }
    }
}
