use super::*;
use egui::{Event, Modifiers, PointerButton};

#[test]
fn folder_jump_classic_inventory_entry_uses_real_pointer_and_preserves_normal_search() {
    let mut frames = crate::ui::widgets::elide_tests::Frames::new();
    let palette = crate::theme::Theme::default().palette();
    let mut state = InventoryUi {
        search: "ordinary query".into(),
        ..Default::default()
    };
    let mut preferences = InventoryPreferences::default();
    let mut texts = Vec::new();
    for _ in 0..4 {
        texts = frames.run(Vec::new(), |ui| toolbar(ui, &palette, &mut state, &mut preferences));
    }
    let caption = aurora_i18n::tr!("inventory-jump-title");
    let position = texts
        .iter()
        .find(|text| text.source == caption)
        .expect("actual classic entry")
        .rect
        .center();
    for pressed in [true, false] {
        frames.run(
            vec![
                Event::PointerMoved(position),
                Event::PointerButton {
                    pos: position,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
            |ui| toolbar(ui, &palette, &mut state, &mut preferences),
        );
    }
    assert_eq!(state.take_jump_request(), Some(uuid::Uuid::nil()));
    assert_eq!(state.search, "ordinary query");
    state.tab = 4;
    let texts = frames.run(Vec::new(), |ui| toolbar(ui, &palette, &mut state, &mut preferences));
    assert!(
        !texts.iter().any(|text| text.source == caption),
        "unsupported Favorites view has no misleading entry"
    );
}
