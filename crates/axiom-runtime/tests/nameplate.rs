//! Nameplate theming (docs/plan-player-settings.zh-TW.md N1): a plate's label can come from the
//! catalogs and follow the locale, and its `class` / `style` must name theme entries (the error
//! names the entity).
use axiom_core::gameplay::RuntimeCommand;
use axiom_core::{ContentPackage, World};
use axiom_runtime::{installed_registry, nameplate, ActionMap, WorldRuntime};
use serde_json::json;

fn town() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/axiom-town"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}

/// The town with its first nameplate replaced by `plate`; returns the entity id too.
fn with_plate(plate: serde_json::Value) -> (World, String) {
    let mut w = town();
    let e = w
        .entities
        .iter_mut()
        .find(|e| e.components.contains_key("ui.nameplate"))
        .expect("the town has nameplates");
    e.components.insert("ui.nameplate".into(), plate);
    let id = e.id.clone();
    (w, id)
}

#[test]
fn a_keyed_plate_follows_the_locale_and_falls_back_to_its_text() {
    let mut w = town();
    let doc = w.resources.get_mut("ui.document").unwrap();
    doc["catalogs"]["zh-TW"]["plate_lumi"] = json!("露米");
    doc["catalogs"]["en"]["plate_lumi"] = json!("Lumi");
    let mut r = WorldRuntime::new(w, 1, installed_registry()).unwrap();
    r.step(&ActionMap::default());
    let plate: nameplate::Nameplate =
        serde_json::from_value(json!({"key":"plate_lumi","text":"fallback"})).unwrap();
    let label = |r: &WorldRuntime| nameplate::label(r.state.gameplay.as_ref(), &plate).into_owned();
    let first = label(&r);
    assert!(first == "露米" || first == "Lumi", "{first}");
    for (locale, want) in [("en", "Lumi"), ("zh-TW", "露米")] {
        r.command(&RuntimeCommand::Locale {
            locale: locale.into(),
        })
        .unwrap();
        assert_eq!(label(&r), want);
    }
    // A key no catalog has: the plate's own text.
    let missing: nameplate::Nameplate =
        serde_json::from_value(json!({"key":"plate_nobody","text":"Nobody"})).unwrap();
    assert_eq!(
        nameplate::label(r.state.gameplay.as_ref(), &missing),
        "Nobody"
    );
}

#[test]
fn plate_classes_and_styles_must_be_in_the_theme() {
    let (w, id) = with_plate(json!({"text":"A","class":"no_such_class"}));
    let e = WorldRuntime::new(w, 1, installed_registry())
        .err()
        .expect("unknown class is refused");
    let msg = format!("{e:#}");
    assert!(
        msg.contains(&format!("/entities/{id}/components/ui.nameplate/class")),
        "{msg}"
    );
    let (w, _) = with_plate(json!({"text":"A","style":"no_such_style"}));
    let msg = format!(
        "{:#}",
        WorldRuntime::new(w, 1, installed_registry()).err().unwrap()
    );
    assert!(msg.contains("ui.nameplate/style"), "{msg}");
    // Neither text nor key.
    let (w, _) = with_plate(json!({"height":1.5}));
    assert!(WorldRuntime::new(w, 1, installed_registry()).is_err());
    // Existing theme entries are accepted.
    let (w, _) = with_plate(json!({"key":"plate_x","class":"speaker","style":"dialog"}));
    WorldRuntime::new(w, 1, installed_registry()).unwrap();
}
