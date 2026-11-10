//! Tuning editor operations (Input Map, Localization, ability events, VFX curves, shader
//! source) and their ACL verbs, with no editor running. The resources owned by runtime
//! capabilities (`action.abilities`, `vfx.library`, `render.shaders`) are checked as edit
//! batches here; the runtime parity tests commit them with the installed registry.
use axiom_core::ops::{self, AbilityEventEdit, InputEdit, LocEdit, LocIssue, VfxEdit};
use axiom_core::{error_response, CoreEdit, Edit, Registry, Request, Store, World};
use serde_json::{json, Value};

fn world() -> World {
    let mut w = World::empty("tuning");
    for c in ["behavior", "ui"] {
        w.capabilities.insert(c.into(), 1);
    }
    w.systems = vec!["behavior.execute".into()];
    w.resources
        .insert("behavior.program".into(), json!({"events":{"go":{}}}));
    w.resources.insert(
        "input.bindings".into(),
        json!({"capture_pointer":false,
            "axes":{"move_x":{"positive":"KeyD","negative":"KeyA"}},
            "buttons":{"jump":"Space","use":"KeyE"}}),
    );
    w.resources.insert(
        "ui.document".into(),
        json!({"locale":"en",
            "catalogs":{"en":{"title":"Title","hello":"Hello"},"ja":{"title":"タイトル","hello":"Hello"}},
            "widgets":[{"kind":"text","id":"title","binding":{"kind":"key","id":"title"}}]}),
    );
    w
}

fn store() -> Store {
    Store::memory_with(Registry::core(), world()).unwrap()
}

fn acl(store: &mut Store, text: &str) -> Result<World, Value> {
    store
        .execute(Request::Acl {
            text: text.into(),
            preview: false,
            expected_rev: None,
            propose: false,
        })
        .map_err(|e| error_response(&e))?;
    Ok(store.current().unwrap().1)
}

fn apply(edits: Vec<Edit>) -> World {
    let mut store = store();
    store
        .execute(Request::Apply {
            expected_rev: None,
            edits,
        })
        .unwrap();
    store.current().unwrap().1
}

fn code(e: &anyhow::Error) -> String {
    error_response(e)["diagnostics"][0]["code"]
        .as_str()
        .unwrap_or("")
        .to_owned()
}

#[test]
fn input_verbs_match_the_ops_and_flag_conflicts() {
    let w = world();
    for (script, edit) in [
        (
            "input button jump KeyW",
            InputEdit::Button {
                action: "jump".into(),
                key: "KeyW".into(),
            },
        ),
        (
            "input button fire MouseLeft",
            InputEdit::Button {
                action: "fire".into(),
                key: "MouseLeft".into(),
            },
        ),
        (
            "input axis move_z KeyW KeyS",
            InputEdit::Axis {
                action: "move_z".into(),
                positive: "KeyW".into(),
                negative: "KeyS".into(),
            },
        ),
        (
            "input remove use",
            InputEdit::Remove {
                action: "use".into(),
            },
        ),
    ] {
        let via_ops = apply(ops::edit_input(&w, &edit).unwrap());
        assert_eq!(acl(&mut store(), script).unwrap(), via_ops, "{script}");
    }
    // Rebinding to the same key is no edit; a button / axis cannot change kind in place.
    assert!(ops::edit_input(
        &w,
        &InputEdit::Button {
            action: "jump".into(),
            key: "Space".into()
        }
    )
    .unwrap()
    .is_empty());
    for (edit, expected) in [
        (
            InputEdit::Button {
                action: "move_x".into(),
                key: "KeyQ".into(),
            },
            "action_kind",
        ),
        (
            InputEdit::Axis {
                action: "jump".into(),
                positive: "KeyQ".into(),
                negative: "KeyZ".into(),
            },
            "action_kind",
        ),
        (
            InputEdit::Remove {
                action: "nope".into(),
            },
            "not_found",
        ),
        (
            InputEdit::Button {
                action: "jump".into(),
                key: "Key W".into(),
            },
            "invalid_value",
        ),
        (
            InputEdit::Button {
                action: "bad name".into(),
                key: "KeyW".into(),
            },
            "invalid_value",
        ),
    ] {
        assert_eq!(
            code(&ops::edit_input(&w, &edit).unwrap_err()),
            expected,
            "{edit:?}"
        );
    }
    let hint = ops::edit_input(
        &w,
        &InputEdit::Button {
            action: "jump".into(),
            key: "W".into(),
        },
    )
    .unwrap_err();
    assert!(
        error_response(&hint)
            .to_string()
            .contains("did you mean KeyW"),
        "{hint:#}"
    );
    // Conflicts: KeyD on a button and an axis half.
    let mut b: axiom_core::InputBindings =
        serde_json::from_value(w.resources["input.bindings"].clone()).unwrap();
    assert!(ops::input_conflicts(&b).is_empty());
    b.buttons.insert("dash".into(), "KeyD".into());
    assert_eq!(
        ops::input_conflicts(&b),
        vec![(
            "KeyD".to_owned(),
            vec!["dash".to_owned(), "move_x+".to_owned()]
        )]
    );
    // Budget: 64 actions.
    let mut big = w.clone();
    let buttons: serde_json::Map<String, Value> =
        (0..62).map(|i| (format!("b{i}"), json!("KeyQ"))).collect();
    big.resources.get_mut("input.bindings").unwrap()["buttons"] = Value::Object(buttons);
    assert!(ops::edit_input(
        &big,
        &InputEdit::Button {
            action: "x".into(),
            key: "KeyX".into()
        }
    )
    .is_ok());
    let mut full = big.clone();
    full.resources.get_mut("input.bindings").unwrap()["buttons"]["b62"] = json!("KeyQ");
    assert_eq!(
        code(
            &ops::edit_input(
                &full,
                &InputEdit::Button {
                    action: "x".into(),
                    key: "KeyX".into()
                }
            )
            .unwrap_err()
        ),
        "budget_exceeded"
    );
    // No input map at all.
    let mut none = w.clone();
    none.resources.remove("input.bindings");
    assert_eq!(
        code(
            &ops::edit_input(
                &none,
                &InputEdit::Remove {
                    action: "jump".into()
                }
            )
            .unwrap_err()
        ),
        "not_found"
    );
}

#[test]
fn localization_verbs_match_the_ops_and_keep_locales_aligned() {
    let w = world();
    for (script, edit) in [
        (
            r#"loc set ja hello "こんにちは""#,
            LocEdit::Set {
                locale: "ja".into(),
                key: "hello".into(),
                text: "こんにちは".into(),
            },
        ),
        (
            r#"loc key add bye "Bye""#,
            LocEdit::AddKey {
                key: "bye".into(),
                text: "Bye".into(),
            },
        ),
        (
            "loc locale add fr from=en",
            LocEdit::AddLocale {
                locale: "fr".into(),
                from: Some("en".into()),
            },
        ),
        (
            "loc locale add zh-TW",
            LocEdit::AddLocale {
                locale: "zh-TW".into(),
                from: None,
            },
        ),
        (
            "loc locale remove ja",
            LocEdit::RemoveLocale {
                locale: "ja".into(),
            },
        ),
        (
            "loc key remove hello",
            LocEdit::RemoveKey {
                key: "hello".into(),
            },
        ),
    ] {
        let via_ops = apply(ops::edit_localization(&w, &edit).unwrap());
        assert_eq!(acl(&mut store(), script).unwrap(), via_ops, "{script}");
    }
    // New key: text in the default locale, empty (missing) elsewhere; the world still compiles.
    let added = acl(&mut store(), "loc key add bye Bye").unwrap();
    assert_eq!(added.resources["ui.document"]["catalogs"]["ja"]["bye"], "");
    assert_eq!(
        added.resources["ui.document"]["catalogs"]["en"]["bye"],
        "Bye"
    );
    let blank = acl(&mut store(), "loc locale add ko").unwrap();
    assert_eq!(
        blank.resources["ui.document"]["catalogs"]["ko"]["title"],
        ""
    );
    // A cell edit is one field write, not the whole document.
    let set = ops::edit_localization(
        &w,
        &LocEdit::Set {
            locale: "ja".into(),
            key: "hello".into(),
            text: "やあ".into(),
        },
    )
    .unwrap();
    assert!(
        matches!(set.as_slice(), [Edit::Core(CoreEdit::SetField { field, .. })] if field == "/catalogs/ja/hello")
    );
    // Unchanged text: no edit.
    assert!(ops::edit_localization(
        &w,
        &LocEdit::Set {
            locale: "en".into(),
            key: "title".into(),
            text: "Title".into()
        }
    )
    .unwrap()
    .is_empty());
    for (edit, expected) in [
        (
            LocEdit::RemoveLocale {
                locale: "en".into(),
            },
            "default_locale",
        ),
        (
            LocEdit::AddKey {
                key: "title".into(),
                text: String::new(),
            },
            "id_taken",
        ),
        (
            LocEdit::AddKey {
                key: "two words".into(),
                text: String::new(),
            },
            "invalid_value",
        ),
        (
            LocEdit::AddLocale {
                locale: "ja".into(),
                from: None,
            },
            "id_taken",
        ),
        (
            LocEdit::AddLocale {
                locale: "de".into(),
                from: Some("xx".into()),
            },
            "not_found",
        ),
        (
            LocEdit::Set {
                locale: "xx".into(),
                key: "title".into(),
                text: "t".into(),
            },
            "not_found",
        ),
        (
            LocEdit::Set {
                locale: "en".into(),
                key: "nope".into(),
                text: "t".into(),
            },
            "not_found",
        ),
        (
            LocEdit::Set {
                locale: "en".into(),
                key: "title".into(),
                text: "x".repeat(8193),
            },
            "invalid_value",
        ),
    ] {
        assert_eq!(
            code(&ops::edit_localization(&w, &edit).unwrap_err()),
            expected,
            "{edit:?}"
        );
    }
    // A key still used by a widget: the commit is refused with the widget path.
    let refused = acl(&mut store(), "loc key remove title").unwrap_err();
    assert!(refused.to_string().contains("title"), "{refused}");
    // Warning rule shared with the UI Editor.
    assert_eq!(
        ops::loc_issue("k", "", Some("x"), false),
        Some(LocIssue::Missing)
    );
    assert_eq!(
        ops::loc_issue("k", "k", Some("x"), true),
        Some(LocIssue::Missing)
    );
    assert_eq!(
        ops::loc_issue("k", "x", Some("x"), false),
        Some(LocIssue::Untranslated)
    );
    assert_eq!(ops::loc_issue("k", "x", Some("x"), true), None);
    assert_eq!(ops::loc_issue("k", "y", Some("x"), false), None);
}

fn runtime_world() -> World {
    let mut w = world();
    w.resources.insert(
        "audio.library".into(),
        json!({"cues":{"pop":{"frequency":440,"duration":0.1},"chime":{"frequency":880,"duration":0.2}}}),
    );
    w.resources.insert(
        "action.abilities".into(),
        json!({"abilities":{"wave":{"kind":"attack","clip":"wave","duration":190,"events":[{"tick":10,"sound":"chime"}]}}}),
    );
    w.resources.insert(
        "vfx.library".into(),
        json!({"emitters":{"fx/a":{"size":0.2,"end_size":0.05,"color":[1,0,0,1],"end_color":[0,0,1,0]},"fx/bare":{}}}),
    );
    w.resources.insert(
        "render.shaders".into(),
        json!({"shaders":{"glow":{"wgsl":"fn surface(s: SurfaceInput) -> vec4<f32> { return s.color; }"}},"materials":{}}),
    );
    w
}

/// The edits an ACL statement compiles to (the editor's `*.edit` commands run the same).
fn compiled(w: &World, statement: &str) -> Vec<Value> {
    axiom_core::acl::tuning_command_edits(w, statement)
        .unwrap()
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap())
        .collect()
}

fn of(edits: Vec<Edit>) -> Vec<Value> {
    edits
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap())
        .collect()
}

#[test]
fn ability_vfx_and_shader_verbs_compile_to_the_ops() {
    let w = runtime_world();
    let event = |e: AbilityEventEdit| of(ops::edit_ability_event(&w, "wave", &e).unwrap());
    assert_eq!(
        compiled(&w, "ability wave event add 40 pop"),
        event(AbilityEventEdit::Add {
            tick: 40,
            sound: "pop".into()
        })
    );
    assert_eq!(
        compiled(&w, "ability wave event move 0 120"),
        event(AbilityEventEdit::Move {
            index: 0,
            tick: 120
        })
    );
    assert_eq!(
        compiled(&w, "ability wave event sound 0 pop"),
        event(AbilityEventEdit::Sound {
            index: 0,
            sound: "pop".into()
        })
    );
    assert_eq!(
        compiled(&w, "ability wave event remove 0"),
        event(AbilityEventEdit::Remove { index: 0 })
    );
    // The whole events array is written at the ability.
    assert_eq!(
        event(AbilityEventEdit::Add {
            tick: 40,
            sound: "pop".into()
        })[0]["value"],
        json!([{"tick":10,"sound":"chime"},{"tick":40,"sound":"pop"}])
    );
    assert!(
        ops::edit_ability_event(&w, "wave", &AbilityEventEdit::Move { index: 0, tick: 10 })
            .unwrap()
            .is_empty()
    );
    for (edit, expected) in [
        (
            AbilityEventEdit::Add {
                tick: 190,
                sound: "pop".into(),
            },
            "invalid_value",
        ),
        (
            AbilityEventEdit::Add {
                tick: 1,
                sound: "boom".into(),
            },
            "missing_cue",
        ),
        (AbilityEventEdit::Move { index: 3, tick: 1 }, "not_found"),
        (AbilityEventEdit::Remove { index: 1 }, "not_found"),
    ] {
        assert_eq!(
            code(&ops::edit_ability_event(&w, "wave", &edit).unwrap_err()),
            expected,
            "{edit:?}"
        );
    }
    assert_eq!(
        code(
            &ops::edit_ability_event(&w, "nope", &AbilityEventEdit::Remove { index: 0 })
                .unwrap_err()
        ),
        "not_found"
    );
    let mut full = w.clone();
    full.resources.get_mut("action.abilities").unwrap()["abilities"]["wave"]["events"] = json!((0
        ..16)
        .map(|i| json!({"tick":i,"sound":"pop"}))
        .collect::<Vec<_>>());
    assert_eq!(
        code(
            &ops::edit_ability_event(
                &full,
                "wave",
                &AbilityEventEdit::Add {
                    tick: 50,
                    sound: "pop".into()
                }
            )
            .unwrap_err()
        ),
        "budget_exceeded"
    );

    // VFX: two field writes per edit, values rounded to 4 decimals; unchanged ends skipped.
    let size = of(ops::edit_vfx(
        &w,
        "fx/a",
        &VfxEdit::Size {
            start: 0.35,
            end: 0.05,
        },
    )
    .unwrap());
    assert_eq!(compiled(&w, "vfx fx/a size 0.35 0.05"), size);
    assert_eq!(size.len(), 1);
    assert_eq!(size[0]["field"], "/emitters/fx~1a/size");
    let color = compiled(&w, "vfx fx/a color 0.2,0.4,0.6,1 0,0,0,0");
    assert_eq!(
        color,
        of(ops::edit_vfx(
            &w,
            "fx/a",
            &VfxEdit::Color {
                start: [0.2, 0.4, 0.6, 1.0],
                end: [0.0; 4]
            }
        )
        .unwrap())
    );
    assert_eq!(color[0]["value"], json!([0.2, 0.4, 0.6, 1.0]));
    // An authored 1 equals 1.0: no edit.
    let mut ints = w.clone();
    ints.resources.get_mut("vfx.library").unwrap()["emitters"]["fx/a"]["size"] = json!(1);
    assert!(ops::edit_vfx(
        &ints,
        "fx/a",
        &VfxEdit::Size {
            start: 1.0,
            end: 0.05
        }
    )
    .unwrap()
    .is_empty());
    // An emitter with defaults gets explicit fields.
    assert_eq!(
        of(ops::edit_vfx(
            &w,
            "fx/bare",
            &VfxEdit::Size {
                start: 0.1,
                end: 0.0
            }
        )
        .unwrap())
        .len(),
        2
    );
    for edit in [
        VfxEdit::Size {
            start: 0.0,
            end: 0.1,
        },
        VfxEdit::Size {
            start: 0.1,
            end: 5.0,
        },
        VfxEdit::Color {
            start: [1.5, 0.0, 0.0, 1.0],
            end: [0.0; 4],
        },
        VfxEdit::Size {
            start: f64::NAN,
            end: 0.1,
        },
    ] {
        assert_eq!(
            code(&ops::edit_vfx(&w, "fx/a", &edit).unwrap_err()),
            "invalid_value",
            "{edit:?}"
        );
    }
    assert_eq!(
        code(
            &ops::edit_vfx(
                &w,
                "fx/none",
                &VfxEdit::Size {
                    start: 0.1,
                    end: 0.1
                }
            )
            .unwrap_err()
        ),
        "not_found"
    );

    // Shader source: JSON-string token with new lines, one field write.
    let src = "fn surface(s: SurfaceInput) -> vec4<f32> {\n  return vec4<f32>(1.0);\n}";
    let script = format!("shader glow source {}", serde_json::to_string(src).unwrap());
    let shader = compiled(&w, &script);
    assert_eq!(shader, of(ops::set_shader_source(&w, "glow", src).unwrap()));
    assert_eq!(shader[0]["field"], "/shaders/glow/wgsl");
    assert_eq!(shader[0]["value"], src);
    for (id, wgsl, expected) in [
        ("lit", "x", "builtin_shader"),
        ("nope", "x", "not_found"),
        ("glow", "", "invalid_value"),
    ] {
        assert_eq!(
            code(&ops::set_shader_source(&w, id, wgsl).unwrap_err()),
            expected,
            "{id}"
        );
    }
    // Syntax errors are structured.
    for bad in [
        "input button jump",
        "loc set en title",
        "ability wave event add x pop",
        "vfx fx/a color 1,0,0 0,0,0,0",
        "shader glow",
    ] {
        assert!(
            axiom_core::acl::tuning_command_edits(&w, bad).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn vfx_curve_and_gradient_verbs_write_whole_lists_and_remove_them() {
    let w = runtime_world();
    let edit = |e: &VfxEdit| of(ops::edit_vfx(&w, "fx/a", e).unwrap());
    let curve = VfxEdit::SizeCurve {
        keys: vec![[0.0, 0.1], [0.25, 0.4], [1.0, 0.0]],
    };
    let set = compiled(&w, "vfx fx/a size_curve 0:0.1 0.25:0.4 1:0");
    assert_eq!(set, edit(&curve));
    assert_eq!(set[0]["field"], "/emitters/fx~1a/size_curve");
    assert_eq!(
        set[0]["value"],
        json!([{"t":0.0,"size":0.1},{"t":0.25,"size":0.4},{"t":1.0,"size":0.0}])
    );
    let gradient = VfxEdit::ColorGradient {
        stops: vec![
            (0.0, [1.0, 0.0, 0.0, 1.0]),
            (0.5, [0.0, 1.0, 0.0, 1.0]),
            (1.0, [0.0, 0.0, 1.0, 0.0]),
        ],
    };
    assert_eq!(
        compiled(&w, "vfx fx/a gradient 0:1,0,0,1 0.5:0,1,0,1 1:0,0,1,0"),
        edit(&gradient)
    );
    // Same list again (authored ints) = no edit; `none` rewrites the emitter without it.
    let mut curved = w.clone();
    curved.resources.get_mut("vfx.library").unwrap()["emitters"]["fx/a"]["size_curve"] =
        json!([{"t":0,"size":0.1},{"t":0.25,"size":0.4},{"t":1,"size":0}]);
    assert!(ops::edit_vfx(&curved, "fx/a", &curve).unwrap().is_empty());
    let none = axiom_core::acl::tuning_command_edits(&curved, "vfx fx/a size_curve none").unwrap();
    let none = of(none);
    assert_eq!(none[0]["field"], "/emitters/fx~1a");
    assert!(none[0]["value"].get("size_curve").is_none());
    assert_eq!(none[0]["value"]["size"], 0.2);
    assert!(compiled(&w, "vfx fx/a gradient none").is_empty());
    // Bounds: 2..8 keys, strictly increasing t in 0..1, size 0..4, channels 0..1.
    let nine: Vec<[f64; 2]> = (0..9).map(|i| [i as f64 / 8.0, 0.1]).collect();
    for bad in [
        VfxEdit::SizeCurve {
            keys: vec![[0.0, 0.1]],
        },
        VfxEdit::SizeCurve { keys: nine },
        VfxEdit::SizeCurve {
            keys: vec![[0.5, 0.1], [0.5, 0.2]],
        },
        // Distinct as typed, one time once rounded to 4 decimals.
        VfxEdit::SizeCurve {
            keys: vec![[0.00001, 0.1], [0.00004, 0.2]],
        },
        VfxEdit::SizeCurve {
            keys: vec![[0.0, 0.1], [1.2, 0.2]],
        },
        VfxEdit::SizeCurve {
            keys: vec![[0.0, 0.1], [1.0, 4.5]],
        },
        VfxEdit::ColorGradient {
            stops: vec![(0.0, [1.0; 4]), (1.0, [2.0, 0.0, 0.0, 0.0])],
        },
        VfxEdit::ColorGradient {
            stops: vec![(1.0, [1.0; 4]), (0.0, [1.0; 4])],
        },
    ] {
        assert_eq!(
            code(&ops::edit_vfx(&w, "fx/a", &bad).unwrap_err()),
            "invalid_value",
            "{bad:?}"
        );
    }
    for bad in [
        "vfx fx/a size_curve 0-0.1 1:0",
        "vfx fx/a gradient 0:1,0,0 1:0,0,0,0",
        "vfx fx/a size_curve",
    ] {
        assert!(
            axiom_core::acl::tuning_command_edits(&w, bad).is_err(),
            "{bad}"
        );
    }
}
