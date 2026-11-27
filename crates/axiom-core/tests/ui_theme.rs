//! ui.document v2, plan M1a: theme frames, panel styles and anchored panels through the
//! ACL `ui frame|panelstyle|panel` verbs (the editor commands compile to the same verbs).
use axiom_core::gameplay::UiDocument;
use axiom_core::{error_response, Registry, Request, Store, World};
use serde_json::{json, Value};

fn hud() -> World {
    let mut w = World::empty("hud");
    for c in ["behavior", "ui"] {
        w.capabilities.insert(c.into(), 1);
    }
    w.systems = vec!["behavior.execute".into()];
    w.resources.insert(
        "behavior.program".into(),
        json!({"variables":{"coins":3,"quest":"q_none"},"events":{"buy":{}}}),
    );
    w.resources.insert(
        "ui.document".into(),
        json!({"locale":"en",
            "catalogs":{"en":{"title":"Title","q_none":"Quest: none","buy":"Buy"}},
            "images":{"parchment":"ui/parchment.png","brass":"ui/brass.png"},
            "regions":{"title":"top_left"},
            "widgets":[
                {"kind":"text","id":"title","binding":{"kind":"key","id":"title"}},
                {"kind":"text","id":"quest","binding":{"kind":"localized_variable","id":"quest"}},
                {"kind":"text","id":"coins","binding":{"kind":"variable","id":"coins"}},
                {"kind":"button","id":"buy","text":"buy","command":{"kind":"emit","event":"buy"}}]}),
    );
    w
}
fn store() -> Store {
    Store::memory_with(Registry::core(), hud()).unwrap()
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
fn doc(store: &Store) -> Value {
    store.current().unwrap().1.resources["ui.document"].clone()
}
fn rev(store: &Store) -> i64 {
    store.current().unwrap().0
}

#[test]
fn acl_builds_a_framed_panel_from_a_theme() {
    let mut s = store();
    acl(&mut s, "ui frame scroll parchment 24 24 24 24").unwrap();
    acl(
        &mut s,
        "ui panelstyle quest_box fill=10141Ccc corner=6 padding=18 frame=scroll",
    )
    .unwrap();
    acl(
        &mut s,
        "ui panel tracker top_left at=12,64 width=320 style=quest_box widgets=quest,coins",
    )
    .unwrap();
    let d = doc(&s);
    assert_eq!(
        d["theme"]["frames"]["scroll"],
        json!({"image":"parchment","slice":[24,24,24,24]})
    );
    assert_eq!(
        d["theme"]["panel"]["quest_box"],
        json!({"fill":[16,20,28,204],"corner":6.0,"padding":18.0,"frame":"scroll"})
    );
    assert_eq!(
        d["panels"],
        json!([{"id":"tracker","anchor":"top_left","offset":[12.0,64.0],"width":320.0,
            "style":"quest_box","children":["quest","coins"]}])
    );
    // The typed document reads back and the world compiles with it.
    let typed: UiDocument = serde_json::from_value(d).unwrap();
    assert_eq!(typed.panels[0].children, ["quest", "coins"]);
    // Editing a panel keeps its position in the list (z order); the same edit is a no-op.
    acl(&mut s, "ui panel extra bottom widgets=buy").unwrap();
    acl(
        &mut s,
        "ui panel tracker top_left at=12,80 width=320 style=quest_box widgets=quest,coins",
    )
    .unwrap();
    assert_eq!(doc(&s)["panels"][0]["id"], "tracker");
    assert_eq!(doc(&s)["panels"][0]["offset"], json!([12.0, 80.0]));
    let before = rev(&s);
    acl(&mut s, "ui frame scroll parchment 24 24 24 24").unwrap();
    assert_eq!(rev(&s), before, "an unchanged frame commits nothing");
}

#[test]
fn invalid_themes_and_panels_are_refused_with_the_field_path() {
    let mut s = store();
    acl(&mut s, "ui frame scroll parchment 24 24 24 24").unwrap();
    acl(&mut s, "ui panelstyle box frame=scroll").unwrap();
    acl(&mut s, "ui panel tracker top_left widgets=quest").unwrap();
    let start = rev(&s);
    for (script, path) in [
        ("ui frame f2 missing 8 8 8 8", "/theme/frames/f2/image"),
        ("ui frame f2 parchment 8 8 8 4000", "/theme/frames/f2/slice"),
        ("ui panelstyle b2 frame=nope", "/theme/panel/b2/frame"),
        ("ui panelstyle b2 corner=900", "/theme/panel/b2/corner"),
        ("ui frame scroll none", "/theme/panel/box/frame"),
        ("ui panel p2 top widgets=ghost", "/panels/1/children/0"),
        ("ui panel p2 top widgets=title", "/panels/1/children/0"),
        ("ui panel p2 top widgets=quest", "/panels/1/children/0"),
        ("ui panel p2 top style=nope", "/panels/1/style"),
        ("ui panel p2 top width=2", "/panels/1/width"),
        ("ui panel nope none", "/panels"),
    ] {
        let e = acl(&mut s, script).expect_err(script);
        let at = e["diagnostics"][0]["path"].as_str().unwrap_or_default();
        assert!(
            at.ends_with(path),
            "{script}: path {at} (want …{path}): {e}"
        );
        assert_eq!(rev(&s), start, "{script} must not commit");
    }
    for (script, needle) in [
        ("ui panelstyle b3 fill=zz", "RRGGBB"),
        ("ui panelstyle b3 shine=1", "unknown panel style option"),
        ("ui panel p3 sideways", "region"),
        ("ui panel p3 top at=1", "X,Y"),
        ("ui frame f3 parchment 1 2 3", "use: ui frame"),
    ] {
        let e = acl(&mut s, script).expect_err(script);
        assert!(e.to_string().contains(needle), "{script}: {e}");
    }
    assert_eq!(rev(&s), start);
}

#[test]
fn documents_without_a_theme_keep_their_exact_form() {
    // No theme / panels: the typed document serializes neither key.
    let raw = hud().resources["ui.document"].clone();
    let typed: UiDocument = serde_json::from_value(raw.clone()).unwrap();
    let back = serde_json::to_value(&typed).unwrap();
    assert!(
        back.get("theme").is_none() && back.get("panels").is_none(),
        "{back}"
    );
    // Adding and then removing everything returns the authored JSON byte for byte.
    let mut s = store();
    acl(&mut s, "ui frame scroll parchment 24 24 24 24").unwrap();
    acl(&mut s, "ui panelstyle box frame=scroll").unwrap();
    acl(&mut s, "ui panel tracker top_left style=box widgets=quest").unwrap();
    acl(&mut s, "ui panel tracker none").unwrap();
    acl(&mut s, "ui panelstyle box none").unwrap();
    acl(&mut s, "ui frame scroll none").unwrap();
    assert_eq!(doc(&s), raw);
}

#[test]
fn text_and_button_classes_assign_to_widgets_and_reject_mismatches() {
    let mut s = store();
    acl(&mut s, "ui frame plate brass 12 12 12 12 center=tile").unwrap();
    acl(&mut s, "ui frame edge brass 12 12 12 12 center=hidden").unwrap();
    acl(
        &mut s,
        "ui textclass gold size=20 color=E9C478 shadow=000000AA",
    )
    .unwrap();
    acl(&mut s, "ui buttonclass chip size=15 padding=8 normal.frame=plate normal.text=F3E6C8 hover.frame=edge pressed.fill=200000FF disabled.text=8A8A8A").unwrap();
    acl(&mut s, "ui class title gold").unwrap();
    acl(&mut s, "ui class buy chip").unwrap();
    let d = doc(&s);
    assert_eq!(d["theme"]["frames"]["plate"]["center"], "tile");
    assert_eq!(d["theme"]["frames"]["edge"]["center"], "hidden");
    assert_eq!(
        d["theme"]["text"]["gold"],
        json!({"size":20.0,"color":[233,196,120,255],"shadow":[0,0,0,170]})
    );
    assert_eq!(
        d["theme"]["button"]["chip"],
        json!({"size":15.0,"padding":8.0,
            "normal":{"frame":"plate","text":[243,230,200,255]},
            "hover":{"frame":"edge"},
            "pressed":{"fill":[32,0,0,255]},
            "disabled":{"text":[138,138,138,255]}})
    );
    assert_eq!(d["classes"], json!({"title":"gold","buy":"chip"}));
    let start = rev(&s);
    for (script, path) in [
        ("ui class quest chip", "/classes/quest"), // a text widget needs a text class
        ("ui class buy gold", "/classes/buy"),     // a button needs a button class
        ("ui class quest nope", "/classes/quest"),
        ("ui textclass big size=400", "/theme/text/big/size"),
        (
            "ui buttonclass b2 hover.frame=nope",
            "/theme/button/b2/hover/frame",
        ),
        ("ui buttonclass b2 padding=99", "/theme/button/b2/padding"),
        ("ui frame plate none", "/theme/button/chip/normal/frame"),
        ("ui textclass gold none", "/classes/title"),
    ] {
        let e = acl(&mut s, script).expect_err(script);
        let at = e["diagnostics"][0]["path"].as_str().unwrap_or_default();
        assert!(
            at.ends_with(path),
            "{script}: path {at} (want …{path}): {e}"
        );
        assert_eq!(rev(&s), start, "{script} must not commit");
    }
    for (script, needle) in [
        ("ui buttonclass b3 glow.frame=plate", "unknown button state"),
        (
            "ui buttonclass b3 normal.shine=1",
            "unknown button look field",
        ),
        ("ui buttonclass b3 shine=1", "unknown button class option"),
        ("ui textclass t3 weight=bold", "unknown text class option"),
        (
            "ui frame f9 brass 1 1 1 1 center=spiral",
            "stretch, tile or hidden",
        ),
        ("ui frame f9 brass 1 1 1 1 middle=tile", "unknown option"),
    ] {
        let e = acl(&mut s, script).expect_err(script);
        assert!(e.to_string().contains(needle), "{script}: {e}");
    }
    // A widget with a legacy text style cannot also take a class.
    let mut w = hud();
    w.resources.get_mut("ui.document").unwrap()["styles"] = json!({"quest":"value"});
    let mut s2 = Store::memory_with(Registry::core(), w).unwrap();
    acl(&mut s2, "ui textclass gold size=20").unwrap();
    let e = acl(&mut s2, "ui class quest gold").expect_err("style and class");
    assert!(e.to_string().contains("styles or classes"), "{e}");
    // Removing every class assignment and class returns the document to its exact form.
    acl(&mut s, "ui class title none").unwrap();
    acl(&mut s, "ui class buy none").unwrap();
    acl(&mut s, "ui buttonclass chip none").unwrap();
    acl(&mut s, "ui textclass gold none").unwrap();
    acl(&mut s, "ui frame plate none").unwrap();
    acl(&mut s, "ui frame edge none").unwrap();
    assert_eq!(doc(&s), hud().resources["ui.document"]);
}

/// Plan M3: panel sizes in px or %, layouts, nesting, z and the reference screen.
#[test]
fn panel_layouts_nest_and_the_reference_screen_round_trips() {
    let mut s = store();
    let raw = doc(&s);
    acl(&mut s, "ui screen 1920 1080 scale=fit").unwrap();
    acl(&mut s, "ui panel coins_box top widgets=coins").unwrap();
    acl(
        &mut s,
        "ui panel shop top_right width=40% height=300 layout=grid columns=3 gap=6 z=2 widgets=quest,coins_box,buy",
    )
    .unwrap();
    let d = doc(&s);
    assert_eq!(
        d["theme"],
        json!({"reference": [1920.0, 1080.0], "scale": "fit"})
    );
    let shop = &d["panels"][1];
    assert_eq!(
        shop,
        &json!({"id": "shop", "anchor": "top_right", "width": "40%", "height": 300.0,
            "layout": "grid", "columns": 3, "gap": 6.0, "z": 2,
            "children": ["quest", "coins_box", "buy"]})
    );
    let typed: UiDocument = serde_json::from_value(d.clone()).unwrap();
    assert_eq!(
        typed.panels[1].width,
        Some(axiom_core::ui_theme::Length::Percent(40.0))
    );
    assert_eq!(
        axiom_core::ui_theme::panel_depths(&typed.panels),
        vec![2, 1],
        "coins_box sits inside shop"
    );
    let theme = typed.theme.as_ref().unwrap();
    assert!((theme.scale_factor([1280.0, 720.0]) - 720.0 / 1080.0).abs() < 1e-6);
    assert!(
        (theme.scale_factor([2560.0, 1080.0]) - 1.0).abs() < 1e-6,
        "fit: the smaller ratio"
    );

    let start = rev(&s);
    for (script, path) in [
        ("ui panel p9 top width=0%", "/panels/2/width"),
        ("ui panel p9 top height=5000", "/panels/2/height"),
        ("ui panel p9 top layout=row columns=2", "/panels/2/columns"),
        (
            "ui panel p9 top layout=grid columns=40",
            "/panels/2/columns",
        ),
        ("ui panel p9 top gap=500", "/panels/2/gap"),
        ("ui panel p9 top z=999", "/panels/2/z"),
        ("ui panel buy top", "/panels/2/id"),
        ("ui panel p9 top widgets=p9", "/panels/2/children/0"),
        ("ui panel p9 top widgets=coins_box", "/panels/2/children/0"),
        // shop <- coins_box <- shop: a cycle.
        ("ui panel coins_box top widgets=coins,shop", "/panels/0"),
        ("ui screen 100 100", "/theme/reference"),
    ] {
        let e = acl(&mut s, script).expect_err(script);
        let at = e["diagnostics"][0]["path"].as_str().unwrap_or_default();
        assert!(
            at.ends_with(path),
            "{script}: path {at} (want …{path}): {e}"
        );
        assert_eq!(rev(&s), start, "{script} must not commit");
    }
    for (script, needle) in [
        ("ui panel p9 top width=wide", "percentage"),
        (
            "ui panel p9 top layout=spiral",
            "column, row, grid or stack",
        ),
        ("ui screen 1920 1080 scale=zoom", "fit_height"),
        ("ui screen 1920", "use: ui screen"),
    ] {
        let e = acl(&mut s, script).expect_err(script);
        assert!(e.to_string().contains(needle), "{script}: {e}");
    }
    // Five deep is refused (four is the limit).
    acl(&mut s, "ui panel shop none").unwrap();
    acl(&mut s, "ui panel coins_box none").unwrap();
    acl(&mut s, "ui panel d4 top widgets=coins").unwrap();
    acl(&mut s, "ui panel d3 top widgets=d4").unwrap();
    acl(&mut s, "ui panel d2 top widgets=d3").unwrap();
    acl(&mut s, "ui panel d1 top widgets=d2").unwrap();
    let e = acl(&mut s, "ui panel d0 top widgets=d1").expect_err("depth 5");
    assert!(e.to_string().contains("nested 5 deep"), "{e}");
    // Removing everything restores the authored document byte for byte.
    for id in ["d1", "d2", "d3", "d4"] {
        acl(&mut s, &format!("ui panel {id} none")).unwrap();
    }
    acl(&mut s, "ui screen none").unwrap();
    assert_eq!(doc(&s), raw);
}

/// Plan M4: content fonts with an accepted licence, used by text and button classes.
#[test]
fn content_fonts_need_an_open_licence_and_known_ids() {
    let mut s = store();
    let raw = doc(&s);
    acl(&mut s, "ui font pixel ui/pixel.ttf license=MIT").unwrap();
    acl(&mut s, "ui textclass big size=30 font=pixel").unwrap();
    acl(&mut s, "ui buttonclass chip font=pixel").unwrap();
    let d = doc(&s);
    assert_eq!(
        d["theme"]["fonts"],
        json!({"pixel": {"path": "ui/pixel.ttf", "license": "MIT"}})
    );
    assert_eq!(d["theme"]["text"]["big"]["font"], "pixel");
    assert_eq!(d["theme"]["button"]["chip"]["font"], "pixel");
    let start = rev(&s);
    for (script, path) in [
        (
            "ui font f2 ui/x.ttf license=Proprietary",
            "/theme/fonts/f2/license",
        ),
        ("ui font f2 ui/x.png license=MIT", "/theme/fonts/f2/path"),
        ("ui textclass t2 font=nope", "/theme/text/t2/font"),
        ("ui buttonclass b2 font=nope", "/theme/button/b2/font"),
        ("ui font pixel none", "/theme/text/big/font"),
    ] {
        let e = acl(&mut s, script).expect_err(script);
        let at = e["diagnostics"][0]["path"].as_str().unwrap_or_default();
        assert!(
            at.ends_with(path),
            "{script}: path {at} (want …{path}): {e}"
        );
        assert_eq!(rev(&s), start, "{script} must not commit");
    }
    // A quoted path (spaces) is decoded, not kept with its quotes.
    acl(&mut s, r#"ui font spaced "ui/My Fonts/a.ttf" license=MIT"#).unwrap();
    assert_eq!(
        doc(&s)["theme"]["fonts"]["spaced"]["path"],
        "ui/My Fonts/a.ttf"
    );
    acl(&mut s, "ui font spaced none").unwrap();
    for (script, needle) in [
        ("ui font f2 ui/x.ttf", "license="),
        (
            "ui font f2 ui/x.ttf size=3 license=MIT",
            "unknown font option",
        ),
    ] {
        let e = acl(&mut s, script).expect_err(script);
        assert!(e.to_string().contains(needle), "{script}: {e}");
    }
    for k in 2..=4 {
        acl(&mut s, &format!("ui font f{k} ui/f{k}.otf license=OFL-1.1")).unwrap();
    }
    let e = acl(&mut s, "ui font f5 ui/f5.ttf license=OFL-1.1").expect_err("5 fonts");
    assert!(e.to_string().contains("at most 4 fonts"), "{e}");
    for k in 2..=4 {
        acl(&mut s, &format!("ui font f{k} none")).unwrap();
    }
    acl(&mut s, "ui buttonclass chip none").unwrap();
    acl(&mut s, "ui textclass big none").unwrap();
    acl(&mut s, "ui font pixel none").unwrap();
    assert_eq!(doc(&s), raw);
}

/// Plan M5: templates expand at compile time into ordinary widgets and panels.
#[test]
fn templates_expand_into_widgets_panels_and_classes() {
    use axiom_core::gameplay::UiDocument;
    let mut s = store();
    let raw = doc(&s);
    acl(&mut s, "ui textclass big size=30").unwrap();
    acl(&mut s, "ui panel shelf top layout=row").unwrap();
    acl(
        &mut s,
        r#"ui template card {"params":["k"],"widgets":[{"kind":"text","id":"card_{k}","binding":{"kind":"key","id":"{k}"}},{"kind":"text","id":"card_{k}_n","binding":{"kind":"variable","id":"coins"}}],"panel":{"id":"card_{k}_box","anchor":"top","layout":"row"},"classes":{"card_{k}":"big"}}"#,
    )
    .unwrap();
    acl(
        &mut s,
        r#"ui use cards card into=shelf count=coins [{"k":"title"},{"k":"buy"}]"#,
    )
    .unwrap();
    let typed: UiDocument = serde_json::from_value(doc(&s)).unwrap();
    let out = axiom_core::ui_template::expand(&typed).unwrap();
    assert!(out.templates.is_empty() && out.uses.is_empty());
    let ids: Vec<&str> = out.widgets.iter().map(|w| w.id()).collect();
    assert_eq!(
        ids,
        [
            "title",
            "quest",
            "coins",
            "buy",
            "card_title",
            "card_title_n",
            "card_buy",
            "card_buy_n"
        ],
        "instances after the authored widgets, in order"
    );
    let shelf = out.panels.iter().find(|p| p.id == "shelf").unwrap();
    assert_eq!(shelf.children, ["card_title_box", "card_buy_box"]);
    let card = out.panels.iter().find(|p| p.id == "card_buy_box").unwrap();
    assert_eq!(card.children, ["card_buy", "card_buy_n"]);
    assert_eq!(out.classes["card_title"], "big");
    // count=coins: instance 1 is shown while coins > 1 (coins >= 2).
    let when = serde_json::to_value(&out.widgets[6]).unwrap()["when"].clone();
    assert_eq!(
        when,
        json!({"kind":"ge","left":{"kind":"variable","id":"coins"},"right":{"kind":"literal","value":2}})
    );
    // The whole world still compiles (keys exist, widgets are unique, budgets hold).
    let world = s.current().unwrap().1;
    axiom_core::gameplay::CompiledGameplay::compile(&world).unwrap();

    let start = rev(&s);
    for (script, path) in [
        (r#"ui use u2 nope [{"k":"title"}]"#, "/uses/1/template"),
        (r#"ui use u2 card [{}]"#, "/uses/1/instances/0"),
        (
            r#"ui use u2 card [{"k":"title","x":"y"}]"#,
            "/uses/1/instances/0",
        ),
        (r#"ui use u2 card [{"k":"a!b"}]"#, "/uses/1/instances/0/k"),
        (
            r#"ui use u2 card into=ghost [{"k":"quest"}]"#,
            "/uses/1/into",
        ),
        (r#"ui use cards2 card [{"k":"title"}]"#, "/panels/3/id"),
        ("ui template card none", "/uses/0/template"),
        (
            r#"ui template t2 {"widgets":[{"kind":"text","id":"x","binding":{"kind":"key","id":"title"}}],"panel":{"id":"p","anchor":"top","children":["x"]}}"#,
            "/templates/t2/panel/children",
        ),
        (
            r#"ui template t2 {"params":["i"],"widgets":[{"kind":"text","id":"x{i}","binding":{"kind":"key","id":"title"}}]}"#,
            "/templates/t2/params",
        ),
        (r#"ui template t2 {"widgets":[]}"#, "/templates/t2/widgets"),
    ] {
        let e = acl(&mut s, script).expect_err(script);
        let at = e["diagnostics"][0]["path"].as_str().unwrap_or_default();
        assert!(
            at.ends_with(path),
            "{script}: path {at} (want …{path}): {e}"
        );
        assert_eq!(rev(&s), start, "{script} must not commit");
    }
    // 64 widgets after expansion at most: 4 authored + 4 now + 2 x 30 more is too many.
    let many: Vec<_> = (0..30).map(|n| json!({"k": format!("k{n}")})).collect();
    let e = acl(
        &mut s,
        &format!("ui use u3 card into=shelf {}", json!(many)),
    )
    .expect_err("over the widget budget");
    assert!(e.to_string().contains("after expansion"), "{e}");
    assert_eq!(rev(&s), start);
    for script in [
        "ui use cards none",
        "ui template card none",
        "ui panel shelf none",
        "ui textclass big none",
    ] {
        acl(&mut s, script).unwrap();
    }
    assert_eq!(doc(&s), raw);
}

/// Plan M6: pages of root panels, opened and closed by typed commands.
#[test]
fn pages_show_root_panels_and_buttons_open_and_close_them() {
    let mut s = store();
    let raw = doc(&s);
    acl(&mut s, "ui panel menu center widgets=buy").unwrap();
    acl(&mut s, "ui panel inner top widgets=coins").unwrap();
    acl(&mut s, "ui panel outer top widgets=inner").unwrap();
    acl(&mut s, "ui page shop panels=menu pause=on escape=on").unwrap();
    assert_eq!(
        doc(&s)["pages"],
        json!({"shop": {"panels": ["menu"], "pause": true, "escape": true}}),
        "modal is the default and not written"
    );
    acl(&mut s, "ui button buy close shop").unwrap();
    assert_eq!(
        doc(&s)["widgets"][3]["command"],
        json!({"kind": "close", "page": "shop"})
    );
    acl(&mut s, "ui button buy close").unwrap();
    assert_eq!(doc(&s)["widgets"][3]["command"], json!({"kind": "close"}));
    acl(&mut s, "ui button buy open shop").unwrap();
    let start = rev(&s);
    for (script, path) in [
        ("ui page p2 panels=ghost", "/pages/p2/panels/0"),
        ("ui page p2 panels=inner", "/pages/p2/panels/0"),
        ("ui page p2 panels=menu", "/pages/shop/panels/0"),
        ("ui page p2 panels=outer escape=on", "/pages"),
        ("ui page p2 modal=off", "/pages/p2/panels"),
    ] {
        let e = acl(&mut s, script).expect_err(script);
        let at = e["diagnostics"][0]["path"].as_str().unwrap_or_default();
        assert!(
            at.ends_with(path),
            "{script}: path {at} (want …{path}): {e}"
        );
        assert_eq!(rev(&s), start, "{script} must not commit");
    }
    let e = acl(&mut s, "ui page p2 panels=outer pause=maybe").expect_err("flag");
    assert!(e.to_string().contains("on or off"), "{e}");
    // The command names a page the document has (checked when the world compiles).
    let world = s.current().unwrap().1;
    axiom_core::gameplay::CompiledGameplay::compile(&world).unwrap();
    for script in [
        r#"set widgets[id=buy] /command {"kind":"emit","event":"buy"}"#,
        "ui page shop none",
        "ui panel outer none",
        "ui panel inner none",
        "ui panel menu none",
    ] {
        acl(&mut s, script).unwrap();
    }
    assert_eq!(doc(&s), raw);
}

#[test]
fn nameplate_theme_sets_distances_and_defaults_and_checks_them() {
    let mut s = store();
    acl(&mut s, "ui textclass tag size=16 color=FFE0A0").unwrap();
    acl(
        &mut s,
        "ui panelstyle tag_box fill=10141Ccc corner=6 padding=4",
    )
    .unwrap();
    acl(
        &mut s,
        "ui nameplate fade=12 hide=30 class=tag style=tag_box",
    )
    .unwrap();
    assert_eq!(
        doc(&s)["theme"]["nameplate"],
        json!({"fade_from":12.0,"hide_beyond":30.0,"class":"tag","style":"tag_box"})
    );
    let n: axiom_core::ui_theme::NameplateTheme =
        serde_json::from_value(doc(&s)["theme"]["nameplate"].clone()).unwrap();
    assert_eq!(n.opacity(10.0), 1.0);
    assert!((n.opacity(21.0) - 0.5).abs() < 1e-6);
    assert_eq!(n.opacity(30.5), 0.0);
    // Without a theme: hidden beyond 40 m, no fade (the old behaviour).
    let plain = axiom_core::ui_theme::NameplateTheme::default();
    assert_eq!((plain.opacity(39.9), plain.opacity(40.1)), (1.0, 0.0));
    let at = rev(&s);
    for (text, needle) in [
        ("ui nameplate fade=30 hide=20", "/theme/nameplate/fade_from"),
        ("ui nameplate hide=0", "/theme/nameplate/hide_beyond"),
        ("ui nameplate class=nope", "/theme/nameplate/class"),
        ("ui nameplate style=nope", "/theme/nameplate/style"),
        ("ui nameplate size=3", "unknown option 'size'"),
        ("ui nameplate fade=far", "not a distance"),
    ] {
        let e = acl(&mut s, text).expect_err(text).to_string();
        assert!(e.contains(needle), "{text}: {e}");
    }
    assert_eq!(rev(&s), at, "refused edits commit nothing");
    acl(&mut s, "ui nameplate none").unwrap();
    assert!(doc(&s)["theme"].get("nameplate").is_none());
}

#[test]
fn progress_bars_take_classes_and_the_prompt_takes_a_look() {
    let mut w = hud();
    let ui_doc = w.resources.get_mut("ui.document").unwrap();
    let widgets = ui_doc["widgets"].as_array_mut().unwrap();
    widgets.push(json!({"kind":"progress","id":"bar","variable":"coins","max":10}));
    let mut s = Store::memory_with(Registry::core(), w).unwrap();
    acl(&mut s, "ui frame scroll parchment 24 24 24 24").unwrap();
    acl(
        &mut s,
        "ui progressclass coin_bar fill=66F5CD track=101820CC frame=scroll height=12 corner=3 value=off",
    )
    .unwrap();
    acl(&mut s, "ui class bar coin_bar").unwrap();
    acl(&mut s, "ui textclass hint size=20 color=FFFFFF").unwrap();
    acl(&mut s, "ui panelstyle hint_box fill=000000AA padding=12").unwrap();
    acl(&mut s, "ui prompt class=hint style=hint_box").unwrap();
    let d = doc(&s);
    assert_eq!(
        d["theme"]["progress"]["coin_bar"],
        json!({"fill":[102,245,205,255],"track":[16,24,32,204],"frame":"scroll","height":12.0,"corner":3.0,"value":false})
    );
    assert_eq!(d["classes"]["bar"], "coin_bar");
    assert_eq!(
        d["theme"]["prompt"],
        json!({"class":"hint","style":"hint_box"})
    );
    let at = rev(&s);
    for (text, needle) in [
        (
            "ui progressclass big height=200",
            "/theme/progress/big/height",
        ),
        ("ui progressclass x frame=nope", "/theme/progress/x/frame"),
        ("ui progressclass x value=maybe", "on or off"),
        ("ui progressclass x glow=1", "unknown progress class option"),
        // A progress widget takes a progress class, not a text class.
        ("ui class bar hint", "/classes/bar"),
        ("ui prompt class=nope", "/theme/prompt/class"),
        ("ui prompt style=nope", "/theme/prompt/style"),
        // A class still in use cannot be removed.
        ("ui progressclass coin_bar none", "/classes/bar"),
    ] {
        let e = acl(&mut s, text).expect_err(text).to_string();
        assert!(e.contains(needle), "{text}: {e}");
    }
    assert_eq!(rev(&s), at, "refused edits commit nothing");
    acl(&mut s, "ui prompt none").unwrap();
    assert!(doc(&s)["theme"].get("prompt").is_none());
}
