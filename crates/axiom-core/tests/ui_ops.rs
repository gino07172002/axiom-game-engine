//! `ui.document` operations (UI Editor) and their ACL `ui` verbs, with no editor running.
use axiom_core::gameplay::{
    CompiledGameplay, Region, RuntimeCommand, TextStyle, UiEffect, UiEffectKind, UiTrigger,
};
use axiom_core::{error_response, ops, Edit, Registry, Request, Store, World};
use serde_json::{json, Value};

fn hud() -> World {
    let mut w = World::empty("hud");
    for c in ["behavior", "ui"] {
        w.capabilities.insert(c.into(), 1);
    }
    w.systems = vec!["behavior.execute".into()];
    w.resources.insert(
        "behavior.program".into(),
        json!({"variables":{"coins":3,"quest":"q_none","ready":false},
            "events":{"buy":{},"thanks":{},"tagged":{"fields":{"who":"text"}}}}),
    );
    w.resources.insert(
        "ui.document".into(),
        json!({"locale":"zh-TW",
            "catalogs":{"zh-TW":{"title":"標題","q_none":"任務：無","buy":"購買"},"en":{"title":"Title","q_none":"Quest: none","buy":"Buy"}},
            "regions":{"title":"top_left","coins":"top_left"},
            "widgets":[
                {"kind":"text","id":"title","binding":{"kind":"key","id":"title"}},
                {"kind":"text","id":"coins","binding":{"kind":"variable","id":"coins"}},
                {"kind":"text","id":"quest","binding":{"kind":"localized_variable","id":"quest"}},
                {"kind":"button","id":"buy","text":"buy","command":{"kind":"emit","event":"buy"}},
                {"kind":"progress","id":"bar","variable":"coins","max":10}]}),
    );
    w
}

fn store() -> Store {
    Store::memory_with(Registry::core(), hud()).unwrap()
}

fn acl(store: &mut Store, text: &str) -> Result<String, Value> {
    store
        .execute(Request::Acl {
            text: text.into(),
            preview: false,
            expected_rev: None,
            propose: false,
        })
        .map(|v| v["text"].as_str().unwrap().to_owned())
        .map_err(|e| error_response(&e))
}

fn code(error: &Value) -> &str {
    error["diagnostics"][0]["code"].as_str().unwrap_or("")
}

fn apply(edits: Vec<Edit>) -> Result<World, Value> {
    let mut store = store();
    store
        .execute(Request::Apply {
            expected_rev: None,
            edits,
        })
        .map_err(|e| error_response(&e))?;
    Ok(store.current().unwrap().1)
}

fn via_acl(script: &str) -> World {
    let mut s = store();
    acl(&mut s, script).unwrap_or_else(|e| panic!("{script}: {e}"));
    s.current().unwrap().1
}

fn doc(w: &World) -> &Value {
    &w.resources["ui.document"]
}

#[test]
fn ui_verbs_compile_to_the_shared_ops() {
    let w = hud();
    let effect = {
        let mut e = UiEffect::new("coins.roll", "coins", UiEffectKind::NumberRoll);
        e.ticks = Some(90);
        e
    };
    let cases: Vec<(&str, Vec<Edit>)> = vec![
        (
            "ui place coins top_right",
            ops::set_ui_region(&w, "coins", Some(Region::TopRight)).unwrap(),
        ),
        (
            "ui place quest bottom",
            ops::set_ui_region(&w, "quest", Some(Region::Bottom)).unwrap(),
        ),
        (
            "ui style coins value",
            ops::set_ui_style(&w, "coins", Some(TextStyle::Value)).unwrap(),
        ),
        (
            "ui style bar label",
            ops::set_ui_style(&w, "bar", Some(TextStyle::Label)).unwrap(),
        ),
        (
            "ui button buy emit thanks",
            ops::set_ui_button_command(
                &w,
                "buy",
                &RuntimeCommand::Emit {
                    event: "thanks".into(),
                    fields: Default::default(),
                },
            )
            .unwrap(),
        ),
        (
            "ui button buy locale en",
            ops::set_ui_button_command(
                &w,
                "buy",
                &RuntimeCommand::Locale {
                    locale: "en".into(),
                },
            )
            .unwrap(),
        ),
        (
            "ui effect add coins.roll coins number_roll ticks=90",
            ops::add_ui_effect(&w, &Registry::core(), &effect).unwrap(),
        ),
        (
            r#"set widgets[id=title] /binding/id "q_none""#,
            ops::set_ui_widget_fields(&w, "title", &[("/binding/id".into(), json!("q_none"))])
                .unwrap(),
        ),
    ];
    for (script, edits) in cases {
        assert_eq!(apply(edits).unwrap(), via_acl(script), "{script}");
    }
    // Effect edits and removal on a world that has the effect.
    let with = via_acl("ui effect add coins.roll coins number_roll ticks=90");
    let mut edited = effect.clone();
    edited.ticks = Some(30);
    edited.from = Some(1.5);
    let mut s = Store::memory_with(Registry::core(), with.clone()).unwrap();
    acl(&mut s, "ui effect set coins.roll ticks=30 from=1.5").unwrap();
    let mut t = Store::memory_with(Registry::core(), with.clone()).unwrap();
    t.execute(Request::Apply {
        expected_rev: None,
        edits: ops::set_ui_effect(&with, &edited).unwrap(),
    })
    .unwrap();
    assert_eq!(s.current().unwrap().1, t.current().unwrap().1);
    acl(&mut s, "ui effect remove coins.roll").unwrap();
    // Removing the last effect leaves the document as it was (no empty `effects` list).
    assert_eq!(doc(&s.current().unwrap().1), doc(&hud()));
    // Unplacing the last widget of a map removes the map key the typed form omits.
    let unplaced = via_acl("ui place title none\nui place coins none");
    assert!(doc(&unplaced).get("regions").is_none());
    // Already so: nothing to commit.
    let mut s = store();
    assert!(acl(&mut s, "ui place title top_left")
        .unwrap()
        .starts_with("unchanged"));
}

#[test]
fn ui_verbs_report_structured_errors() {
    let mut s = store();
    for (script, expected) in [
        ("ui place nope top_left", "not_found"),
        ("ui place coins sideways", "acl_syntax"),
        ("ui style buy value", "invalid_value"),
        ("ui style coins loud", "acl_syntax"),
        ("ui button coins emit buy", "invalid_value"),
        ("ui button buy emit tagged", "invalid_value"),
        ("ui effect remove nope", "not_found"),
        ("ui effect set nope ticks=3", "not_found"),
        ("ui effect add a nope fade_in", "not_found"),
        ("ui effect add a coins wobble", "acl_syntax"),
        ("ui effect add a coins fade_in speed=3", "acl_syntax"),
        ("ui frobnicate", "acl_syntax"),
    ] {
        let e = acl(&mut s, script).expect_err(script);
        assert_eq!(code(&e), expected, "{script}: {e}");
    }
    acl(&mut s, "ui effect add a coins fade_in").unwrap();
    let e = acl(&mut s, "ui effect add a coins pulse").unwrap_err();
    assert_eq!(code(&e), "id_taken", "{e}");
    // Compile-time checks of the effect itself are reported by the commit.
    for (script, needle) in [
        (
            "ui effect add b title pulse",
            "on: change needs a widget that shows a variable",
        ),
        (
            "ui effect add b quest number_roll on=show",
            "number_roll needs a widget showing a number",
        ),
        (
            "ui effect add b coins slide_in amount=9000",
            "slide_in amount must be 0..2000",
        ),
        (
            "ui effect add b coins fade_in amount=1",
            "fade_in takes no amount",
        ),
        (
            "ui effect add b coins fade_in ticks=0",
            "ticks must be 1..1200",
        ),
        (
            "ui effect add b coins shake on=event",
            "`event` goes with on: event",
        ),
        (
            "ui effect add b coins shake event=nope",
            "missing reference nope",
        ),
        (
            "ui effect add b coins fade_in on=when",
            "on: when needs a `when` condition",
        ),
        (
            "ui effect add b coins pulse direction=up",
            "`direction` applies to slides only",
        ),
        (
            "ui effect add b coins pulse from=3",
            "`from` applies to number_roll only",
        ),
        ("ui effect add b bar fade_out", "on: hide never fires"),
    ] {
        let e = acl(&mut s, script).expect_err(script).to_string();
        assert!(e.contains(needle), "{script}: {e}");
    }
}

#[test]
fn effects_compile_with_defaults_and_indices() {
    let w = via_acl(
        "ui effect add t title fade_in\n\
         ui effect add c coins number_roll\n\
         ui effect add q quest pulse\n\
         ui effect add s coins shake event=thanks amount=12\n\
         ui effect add w bar scale_in on=when when={\"kind\":\"ge\",\"left\":{\"kind\":\"variable\",\"id\":\"coins\"},\"right\":{\"kind\":\"literal\",\"value\":5}}",
    );
    let g = CompiledGameplay::compile(&w).unwrap();
    let e: Vec<_> = g
        .effects
        .iter()
        .map(|e| (e.id.as_str(), e.widget, e.ticks))
        .collect();
    assert_eq!(
        e,
        [
            ("t", 0, 30),
            ("c", 1, 60),
            ("q", 2, 30),
            ("s", 1, 30),
            ("w", 4, 30)
        ]
    );
    let s = &g.effects[3];
    assert_eq!(s.amount, 12.0);
    assert!(matches!(
        s.trigger,
        axiom_core::gameplay::EffectTrigger::Event(i) if g.event_ids[i] == "thanks"
    ));
    assert_eq!(g.effects[1].variable, Some(0));
    // `event=E` alone means on: event; the stored JSON stays minimal.
    let stored = &doc(&w)["effects"][3];
    assert_eq!(
        stored,
        &json!({"id":"s","widget":"coins","kind":"shake","on":"event","event":"thanks","amount":12.0})
    );
    let parsed: UiEffect = serde_json::from_value(stored.clone()).unwrap();
    assert_eq!(parsed.trigger(), UiTrigger::Event);
    // At most 64 definitions per document.
    let many: Vec<Value> = (0..65)
        .map(|i| json!({"id":format!("e{i}"),"widget":"title","kind":"fade_in"}))
        .collect();
    let mut big = hud();
    big.resources.get_mut("ui.document").unwrap()["effects"] = json!(many);
    let e = format!("{:#}", CompiledGameplay::compile(&big).unwrap_err());
    assert!(e.contains("at most 64 UI effects"), "{e}");
}

#[test]
fn shipped_documents_are_unchanged_by_the_effects_field() {
    // Content without effects serializes exactly as before (no `effects` key appears).
    for pack in ["axiom-town", "adv", "street"] {
        let path = format!(
            "{}/../../content/{pack}/world.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let world: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let Some(raw) = world["resources"].get("ui.document") else {
            continue;
        };
        let typed: axiom_core::gameplay::UiDocument = serde_json::from_value(raw.clone()).unwrap();
        assert!(typed.effects.is_empty());
        assert!(
            serde_json::to_value(&typed)
                .unwrap()
                .get("effects")
                .is_none(),
            "{pack}"
        );
    }
}
