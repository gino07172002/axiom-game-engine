//! Player settings (docs/plan-player-settings.zh-TW.md S1): typed keys with ranges, a lenient
//! reader for the player's file, strict author defaults, and the ACL `settings default` verb.
use axiom_core::settings::{self, DisplayMode, PlayerSettings, ShadowQuality};
use axiom_core::{error_response, Registry, Request, Store, World};
use serde_json::{json, Value};

#[test]
fn keys_have_types_and_ranges() {
    let mut s = PlayerSettings::default();
    s.set("audio.master", &json!(0.25)).unwrap();
    s.set("mouse.invert_y", &json!(true)).unwrap();
    s.set("display.mode", &json!("borderless")).unwrap();
    s.set("quality.shadows", &json!("high")).unwrap();
    assert_eq!(s.master, 0.25);
    assert!(s.invert_y);
    assert_eq!(s.display, DisplayMode::Borderless);
    assert_eq!(s.shadows, ShadowQuality::High);
    assert_eq!(s.shadows.texels(), 4096);
    assert_eq!(s.number("audio.master"), Some(0.25));
    for (key, value, needle) in [
        ("audio.master", json!(1.5), "0..1"),
        ("audio.master", json!("loud"), "number"),
        ("mouse.sensitivity", json!(0.0), "0.1..5"),
        ("mouse.invert_y", json!(1), "true or false"),
        (
            "display.mode",
            json!("tiny"),
            "window, borderless, fullscreen",
        ),
        ("nope", json!(1), "unknown setting"),
    ] {
        let e = s.clone().set(key, &value).expect_err(key);
        assert!(format!("{e:#}").contains(needle), "{key} {value}: {e:#}");
    }
    // Every key round-trips through the file form.
    let back = PlayerSettings::merged(PlayerSettings::default(), &s.to_json());
    assert_eq!(back, (s, vec![]));
}

#[test]
fn a_damaged_settings_file_keeps_the_defaults_and_reports_why() {
    let base = PlayerSettings::default();
    let (s, problems) = PlayerSettings::merged(
        base,
        &json!({"audio.master": 0.5, "audio.music": 9, "bogus": true}),
    );
    assert_eq!(s.master, 0.5, "good keys still apply");
    assert_eq!(s.music, base.music, "a bad value keeps the default");
    assert_eq!(problems.len(), 2, "{problems:?}");
    let (s, problems) = PlayerSettings::merged(base, &json!([1, 2]));
    assert_eq!(s, base);
    assert_eq!(problems.len(), 1);
}

fn world() -> World {
    let mut w = World::empty("settings");
    w.capabilities.insert("core".into(), 1);
    w
}
fn acl(store: &mut Store, text: &str) -> Result<i64, Value> {
    store
        .execute(Request::Acl {
            text: text.into(),
            preview: false,
            expected_rev: None,
            propose: false,
        })
        .map(|v| v["rev"].as_i64().unwrap_or(-1))
        .map_err(|e| error_response(&e))
}

#[test]
fn acl_sets_author_defaults_and_removing_the_last_one_removes_the_resource() {
    let mut s = Store::memory_with(Registry::core(), world()).unwrap();
    let raw = s.current().unwrap().1;
    acl(&mut s, "settings default audio.master 0.8").unwrap();
    acl(&mut s, "settings default display.mode fullscreen").unwrap();
    let w = s.current().unwrap().1;
    assert_eq!(
        w.resources["game.settings"],
        json!({"defaults": {"audio.master": 0.8, "display.mode": "fullscreen"}})
    );
    let d = settings::defaults(w.resources.get("game.settings")).unwrap();
    assert_eq!((d.master, d.display), (0.8, DisplayMode::Fullscreen));
    let rev = s.current().unwrap().0;
    for (script, path) in [
        (
            "settings default audio.master 2",
            "/resources/game.settings/defaults/audio.master",
        ),
        (
            "settings default nope 1",
            "/resources/game.settings/defaults/nope",
        ),
    ] {
        let e = acl(&mut s, script).expect_err(script);
        let at = e["diagnostics"][0]["path"].as_str().unwrap_or_default();
        assert_eq!(at, path, "{script}: {e}");
        assert_eq!(s.current().unwrap().0, rev, "{script} must not commit");
    }
    let e = acl(&mut s, "settings default audio.master").expect_err("usage");
    assert!(
        e.to_string().contains("settings default KEY VALUE|none"),
        "{e}"
    );
    acl(&mut s, "settings default audio.master none").unwrap();
    acl(&mut s, "settings default display.mode none").unwrap();
    assert_eq!(
        s.current().unwrap().1,
        raw,
        "the resource goes with its last default"
    );
}
