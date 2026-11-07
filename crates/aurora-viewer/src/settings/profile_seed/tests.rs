use super::*;

fn document(mut fields: Value) -> Vec<u8> {
    fields.as_object_mut().unwrap().insert("settings_version".into(), VERSION.into());
    serde_json::to_vec(&fields).unwrap()
}
fn families(bits: u8) -> Families {
    Families::new(bits).unwrap()
}
fn all() -> Families {
    families(15)
}
fn patched(seed: &ProfileSeed, target: Option<&[u8]>) -> Value {
    serde_json::from_slice(&seed.patch(target).unwrap().0).unwrap()
}
#[test]
fn profile_seed_selection_mask_is_closed_and_no_empty_projection_is_admitted() {
    for bits in [0, 16, 31, 128, 255] {
        assert_eq!(Families::new(bits), Err(Error::Families));
    }
    for bits in 1..16 {
        let chosen = families(bits);
        for family in Family::ALL {
            assert_eq!(chosen.contains(family), bits & (1 << family as u8) != 0);
        }
    }
}
#[test]
fn profile_seed_native_saved_defaults_are_accepted_without_copying_source_identity_or_unselected_data() {
    let mut source = serde_json::to_value(Settings::default()).unwrap();
    for key in [
        "username",
        "mfa_hash",
        "custom_login_uri",
        "cache_location",
        "chat_log_location",
        "browser_home_page",
        "future_unselected_secret",
    ] {
        source[key] = "SOURCE-SENSITIVE-NEVER-COPY".into();
    }
    source["audio"] = serde_json::json!({"input_device":"SOURCE-DEVICE"});
    let seed = ProfileSeed::read(&document(source), all()).unwrap();
    let target = document(
        serde_json::json!({"username":"TARGET-IDENTITY","future_unselected_secret":{"nested":[1,true,"TARGET-OPAQUE"]},"audio":{"input_device":"TARGET-DEVICE"}}),
    );
    let output = patched(&seed, Some(&target));
    assert_eq!(output["username"], "TARGET-IDENTITY");
    assert_eq!(
        output["future_unselected_secret"],
        serde_json::json!({"nested":[1,true,"TARGET-OPAQUE"]})
    );
    assert_eq!(output["audio"], serde_json::json!({"input_device":"TARGET-DEVICE"}));
    let rendered = serde_json::to_string(&output).unwrap();
    assert!(!rendered.contains("SOURCE-SENSITIVE") && !rendered.contains("SOURCE-DEVICE"));
    assert_eq!(seed.summary().len(), 4);
    assert!(seed.summary().iter().all(|s| s.defaulted == 0));
    assert!(!format!("{seed:?}").contains("TARGET-OPAQUE"));
}
#[test]
fn profile_seed_missing_selected_source_fields_use_current_native_defaults_not_destination_values() {
    let seed = ProfileSeed::read(&document(serde_json::json!({})), all()).unwrap();
    let target = document(
        serde_json::json!({"draw_distance":512.0,"ui_scale":2.0,"colors":{"tag_me":[1,2,3,4]},"keybinds":{"map":[]},"unselected":{"keep":"exact value"}}),
    );
    let result = patched(&seed, Some(&target));
    let native = Settings::default();
    assert_eq!(result["draw_distance"], serde_json::to_value(native.draw_distance).unwrap());
    assert_eq!(result["ui_scale"], serde_json::to_value(native.ui_scale).unwrap());
    assert_eq!(result["colors"], serde_json::to_value(native.colors).unwrap());
    assert_eq!(result["keybinds"], serde_json::to_value(native.keybinds).unwrap());
    assert_eq!(result["unselected"], serde_json::json!({"keep":"exact value"}));
    assert!(seed.summary().iter().all(|summary| summary.defaulted == summary.fields));
    let absent = patched(&seed, None);
    assert_eq!(absent["settings_version"], VERSION);
    assert!(absent.get("username").is_none());
}
#[test]
fn profile_seed_each_family_changes_only_its_explicit_fields_and_never_guesses_a_prefix() {
    let source = document(
        serde_json::json!({"draw_distance":256.0,"ui_scale":1.5,"colors":{"tag_me":[1,2,3,4]},"keybinds":{"map":[["People",[{"input":{"Key":"KeyQ"}},null]]]},"RenderSecret":"SOURCE-SECRET","classic_future_security":true}),
    );
    let target = document(
        serde_json::json!({"draw_distance":96.0,"ui_scale":1.0,"colors":{"unknown_unselected":"TARGET"},"keybinds":{"unsupported_unselected":true},"RenderSecret":"TARGET-SECRET","classic_future_security":false}),
    );
    for (family, bit) in [(Family::Graphics, 1), (Family::Interface, 2)] {
        let seed = ProfileSeed::read(&source, families(bit)).unwrap();
        let result = patched(&seed, Some(&target));
        assert_eq!(result["RenderSecret"], "TARGET-SECRET");
        assert_eq!(result["classic_future_security"], false);
        assert_eq!(result["colors"], serde_json::json!({"unknown_unselected":"TARGET"}));
        assert_eq!(result["keybinds"], serde_json::json!({"unsupported_unselected":true}));
        assert_eq!(result["draw_distance"], if family == Family::Graphics { 256.0 } else { 96.0 });
        assert_eq!(result["ui_scale"], if family == Family::Interface { 1.5 } else { 1.0 });
    }
    let target = document(serde_json::json!({"draw_distance":96.0,"ui_scale":1.0,"unselected":["kept"]}));
    for bit in [4, 8] {
        let result = patched(&ProfileSeed::read(&source, families(bit)).unwrap(), Some(&target));
        assert_eq!(result["draw_distance"], 96.0);
        assert_eq!(result["ui_scale"], 1.0);
        assert_eq!(result["unselected"], serde_json::json!(["kept"]));
    }
}
#[test]
fn profile_seed_colors_pin_the_complete_current_byte_channel_allowlist_and_refuse_future_members() {
    let seed = ProfileSeed::read(
        &document(serde_json::json!({"colors":{"tag_me":[0,1,254,255],"relation_names":false}})),
        families(4),
    )
    .unwrap();
    let result = patched(&seed, None);
    assert_eq!(result["colors"]["tag_me"], serde_json::json!([0, 1, 254, 255]));
    assert_eq!(result["colors"]["relation_names"], false);
    assert_eq!(result["colors"].as_object().unwrap().len(), 37);
    assert_eq!(
        result["colors"]["chat_owner"],
        serde_json::to_value(ColorSettings::default().chat_owner).unwrap()
    );
    for bad in [
        serde_json::json!({"future":true}),
        serde_json::json!({"tag_me":[0,1,2,256]}),
        serde_json::json!({"tag_me":[1,2,3]}),
        serde_json::json!({"tag_me":[0.5,1,2,3]}),
        serde_json::json!({"relation_names":"yes"}),
    ] {
        let bytes = document(serde_json::json!({"colors":bad}));
        assert!(matches!(ProfileSeed::read(&bytes, families(4)), Err(Error::Selected)));
        assert!(matches!(seed.patch(Some(&bytes)), Err(Error::Selected)));
    }
}
#[test]
fn profile_seed_invalid_selected_graphics_and_interface_values_refuse_instead_of_silent_native_clamping() {
    for (key, value, bits) in [
        ("draw_distance", serde_json::json!(31.0), 1),
        ("draw_distance", serde_json::json!(513.0), 1),
        ("exposure", serde_json::json!(0.1), 1),
        ("sharpen", serde_json::json!(2.0), 1),
        ("bloom_strength", serde_json::json!(0.0), 1),
        ("lod_factor", serde_json::json!(8.0), 1),
        ("texture_budget_mb", serde_json::json!(255), 1),
        ("max_texture_resolution", serde_json::json!(1025), 1),
        ("shadow_quality", serde_json::json!(5), 1),
        ("antialiasing", serde_json::json!(7), 1),
        ("render_scale_percent", serde_json::json!(101), 1),
        ("vsync", serde_json::json!("yes"), 1),
        ("ui_scale", serde_json::json!(0.5), 2),
        ("font_scale", serde_json::json!(0.0), 2),
        ("chat_font_scale", serde_json::json!(10.0), 2),
        ("font", serde_json::json!("bad\nfont"), 2),
        ("font", serde_json::json!("x".repeat(129)), 2),
        ("chat_style", serde_json::json!("FutureStyle"), 2),
        ("favorites_access", serde_json::json!("FutureAccess"), 2),
    ] {
        let mut value_map = serde_json::json!({});
        value_map[key] = value;
        let bytes = document(value_map);
        assert!(matches!(ProfileSeed::read(&bytes, families(bits)), Err(Error::Selected)), "{key}");
        let seed = ProfileSeed::read(&document(serde_json::json!({})), families(bits)).unwrap();
        assert!(matches!(seed.patch(Some(&bytes)), Err(Error::Selected)), "{key}");
    }
}
#[test]
fn profile_seed_current_native_range_boundaries_remain_valid() {
    for (distance, exposure, sharpen, bloom, lod, budget, shadow, aa, scale, ui, font) in [
        (32.0, 0.25, 0.0, 0.05, 0.5, 256, 0, 0, 50, 0.75, super::super::FONT_SCALE_MIN),
        (512.0, 4.0, 1.0, 1.0, 4.0, 32768, 4, 6, 100, 2.0, super::super::FONT_SCALE_MAX),
    ] {
        let bytes = document(
            serde_json::json!({"draw_distance":distance,"exposure":exposure,"sharpen":sharpen,"bloom_strength":bloom,"lod_factor":lod,"texture_budget_mb":budget,"shadow_quality":shadow,"antialiasing":aa,"render_scale_percent":scale,"ui_scale":ui,"font_scale":font,"chat_font_scale":font,"max_texture_resolution":2048}),
        );
        let result = patched(&ProfileSeed::read(&bytes, families(3)).unwrap(), None);
        assert_eq!(result["draw_distance"], distance);
        assert_eq!(result["ui_scale"], ui);
    }
}
#[test]
fn profile_seed_binding_selection_uses_native_actions_two_slots_defaults_and_refuses_unknown_or_duplicate_identity() {
    let source = document(serde_json::json!({"keybinds":{"map":[["People",[{"input":{"Key":"KeyQ"},"ctrl":true},null]]]}}));
    let seed = ProfileSeed::read(&source, families(8)).unwrap();
    let result = patched(&seed, None);
    let native: crate::keybinds::KeyBindings = serde_json::from_value(result["keybinds"].clone()).unwrap();
    assert_eq!(
        native.triggered(
            &Input::key(winit::keyboard::KeyCode::KeyQ),
            crate::keybinds::Mods {
                ctrl: true,
                ..Default::default()
            }
        ),
        Some(Action::People)
    );
    assert_eq!(native.get(Action::PushToTalk), Settings::default().keybinds.get(Action::PushToTalk));
    for bad in [
        serde_json::json!({"map":[["People",[null,null]],["People",[null,null]]]}),
        serde_json::json!({"map":[["FutureAction",[null,null]]]}),
        serde_json::json!({"map":[["People",[{"input":{"Touch":"Future"}},null]]]}),
        serde_json::json!({"map":[["People",[{"input":{"Mouse":"FutureMouse9"}},null]]]}),
        serde_json::json!({"map":[["People",[{"input":{"Key":"FutureKey9"}},null]]]}),
        serde_json::json!({"map":[["People",[{"input":{"Key":"KeyQ"},"future":true},null]]]}),
        serde_json::json!({"map":[["People",[null,null,null]]]}),
        serde_json::json!({"map":[],"future":true}),
    ] {
        let bytes = document(serde_json::json!({"keybinds":bad}));
        assert!(matches!(ProfileSeed::read(&bytes, families(8)), Err(Error::Selected)));
        assert!(matches!(seed.patch(Some(&bytes)), Err(Error::Selected)));
    }
}
#[test]
fn profile_seed_only_supported_integer_version_and_complete_json_are_read_without_repair() {
    for raw in [
        b"{}".as_slice(),
        b"[]",
        b"null",
        b"{\"settings_version\":31}",
        b"{\"settings_version\":33}",
        b"{\"settings_version\":32.0}",
        b"{\"settings_version\":32} true",
        b"{\"settings_version\":32,\"draw_distance\":NaN}",
    ] {
        assert!(ProfileSeed::read(raw, all()).is_err());
    }
    let seed = ProfileSeed::read(&document(serde_json::json!({})), all()).unwrap();
    assert!(matches!(
        seed.patch(Some(b"{\"settings_version\":33,\"unknown\":\"preserve original\"}")),
        Err(Error::Version)
    ));
}
#[test]
fn profile_seed_duplicate_members_are_rejected_even_when_escaped_or_inside_unselected_values() {
    for raw in [
        r#"{"settings_version":32,"settings_version":32}"#,
        r#"{"settings_version":32,"colors":{},"\u0063olors":{}}"#,
        r#"{"settings_version":32,"unselected":{"item":1,"item":2}}"#,
    ] {
        assert!(matches!(ProfileSeed::read(raw.as_bytes(), all()), Err(Error::Json)));
    }
}
#[test]
fn profile_seed_document_node_and_depth_budgets_reject_excess_before_projection() {
    let mut deep = Value::Null;
    for _ in 0..MAX_DEPTH - 2 {
        deep = Value::Array(vec![deep]);
    }
    let accepted = document(serde_json::json!({"unselected":deep}));
    assert!(ProfileSeed::read(&accepted, families(1)).is_ok());
    let mut value: Value = serde_json::from_slice(&accepted).unwrap();
    value["unselected"] = Value::Array(vec![value["unselected"].take()]);
    assert!(matches!(ProfileSeed::read(&document(value), families(1)), Err(Error::Json)));
    let accepted = document(serde_json::json!({"unselected":vec![Value::Null;MAX_NODES-3]}));
    assert!(ProfileSeed::read(&accepted, families(1)).is_ok());
    let rejected = document(serde_json::json!({"unselected":vec![Value::Null;MAX_NODES-2]}));
    assert!(matches!(ProfileSeed::read(&rejected, families(1)), Err(Error::Json)));
}
#[test]
fn profile_seed_input_output_and_maximum_escaping_are_bounded_without_exposing_source_values_in_debug() {
    let oversized = vec![b' '; MAX_FILE + 1];
    assert!(matches!(ProfileSeed::read(&oversized, all()), Err(Error::Bound)));
    let seed = ProfileSeed::read(&document(serde_json::json!({"font":"\\\"".repeat(64)})), all()).unwrap();
    let output = seed.patch(None).unwrap().0;
    assert!(output.len() < MAX_PAYLOAD && output.capacity() <= MAX_FILE);
    let read: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(read["font"], "\\\"".repeat(64));
    assert!(!format!("{seed:?}").contains("font"));
    let fixed = document(serde_json::json!({"unselected":""})).len();
    let exact = document(serde_json::json!({"unselected":"x".repeat(MAX_FILE-fixed)}));
    assert_eq!(exact.len(), MAX_FILE);
    let graphics = ProfileSeed::read(&document(serde_json::json!({})), families(1)).unwrap();
    assert!(matches!(graphics.patch(Some(&exact)), Err(Error::Bound)));
    let value = serde_json::json!("x".repeat(MAX_PAYLOAD));
    assert!(matches!(bounded_bytes(&value, MAX_PAYLOAD), Err(Error::Bound)));
}

#[test]
fn profile_seed_old_japanese_mouse_bindings_use_native_tokens_without_copying_or_selecting_language() {
    for (alias, button) in [
        ("中央", winit::event::MouseButton::Middle),
        ("戻る", winit::event::MouseButton::Back),
        ("前進", winit::event::MouseButton::Forward),
    ] {
        let source = document(serde_json::json!({"language":"ja","keybinds":{"map":[["People",[{"input":{"Mouse":alias}},null]]]}}));
        let target = document(serde_json::json!({"language":"en"}));
        let result = patched(&ProfileSeed::read(&source, families(8)).unwrap(), Some(&target));
        assert_eq!(result["language"], "en");
        let native: crate::keybinds::KeyBindings = serde_json::from_value(result["keybinds"].clone()).unwrap();
        assert_eq!(
            native.triggered(&Input::mouse(button).unwrap(), crate::keybinds::Mods::default()),
            Some(Action::People)
        );
        assert!(!serde_json::to_string(&result["keybinds"]).unwrap().contains(alias));
    }
}
#[test]
fn profile_seed_full_action_catalogue_two_slots_and_maximum_font_escaping_fit_the_typed_payload_bound() {
    let rows: Vec<_> = crate::keybinds::sections()
        .iter()
        .flat_map(|(_, actions)| {
            actions.iter().map(|(action, _)| {
                serde_json::json!([action,[
                    {"input":{"Key":"NumpadMemorySubtract"},"ctrl":true,"shift":true,"alt":true},
                    {"input":{"Mouse":"Other65535"},"ctrl":true,"shift":true,"alt":true}
                ]])
            })
        })
        .collect();
    let count = rows.len();
    let bytes = document(serde_json::json!({"font":"\\\"".repeat(64),"keybinds":{"map":rows}}));
    let seed = ProfileSeed::read(&bytes, all()).unwrap();
    let (output, summary) = seed.patch(None).unwrap();
    assert!(output.len() < MAX_PAYLOAD);
    let value: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["keybinds"]["map"].as_array().unwrap().len(), count);
    assert_eq!(summary.iter().find(|s| s.family == Family::Bindings).unwrap().defaulted, 0);
    let actual: crate::keybinds::KeyBindings = serde_json::from_value(value["keybinds"].clone()).unwrap();
    assert!(actual.map.iter().all(|(_, bindings)| bindings.iter().all(Option::is_some)));
    assert_eq!(actual.map.len(), count);
    assert!(count > 50, "exercise the complete existing native action table");
}

#[test]
fn profile_seed_output_allocation_stays_capped_after_one_large_unselected_string_and_later_fields() {
    let value = serde_json::json!({"a_unselected":"x".repeat(MAX_FILE*3/4),"z_later":true});
    let output = bounded_bytes(&value, MAX_FILE).unwrap();
    assert!(output.len() < MAX_FILE);
    assert!(
        output.capacity() <= MAX_FILE,
        "large first write cannot cause an oversized geometric allocation"
    );
    assert_eq!(serde_json::from_slice::<Value>(&output).unwrap(), value);
}
