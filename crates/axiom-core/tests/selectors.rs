use axiom_core::{parse_request, Entity, Prefab, Registry, Store, World};
use serde_json::{json, Value};

/// 120 enemies (every third one inherits health from a prefab) plus unrelated props.
fn fixture() -> Store {
    let mut world = World::empty("selectors");
    world.prefabs.insert(
        "enemy.basic".into(),
        Prefab {
            components: [("core.health".into(), json!({"value":100,"max":100}))].into(),
        },
    );
    for i in 0..120 {
        let mut e = Entity::new(format!("enemy/{i:03}"));
        e.tags = vec!["enemy".into()];
        if i % 3 == 0 {
            e.prefab = Some("enemy.basic".into());
        } else {
            e.set("core.health", json!({"value": i, "max": 200}))
                .unwrap();
        }
        world.entities.push(e);
    }
    for i in 0..5 {
        let mut e = Entity::new(format!("prop/{i}"));
        e.set("core.health", json!({"value": 1, "max": 1})).unwrap();
        world.entities.push(e);
    }
    Store::memory_with(Registry::core(), world).unwrap()
}

fn run(store: &mut Store, request: Value) -> Value {
    store
        .execute(parse_request(&request.to_string()).unwrap())
        .unwrap()
}

#[test]
fn glob_where_and_lines_return_only_what_was_asked() {
    let mut store = fixture();
    let out = run(
        &mut store,
        json!({"op":"select","query":{"from":"entities","id_glob":"enemy/*",
            "where":[{"field":"/components/core.health/value","op":"lt","value":5}],
            "fields":["/components/core.health/value"],"format":"lines","limit":10}}),
    );
    assert_eq!(out["format"], "lines");
    let text = out["text"].as_str().unwrap();
    // enemy/000 and enemy/003 inherit 100 from the prefab, so only 1, 2, 4 match.
    assert_eq!(
        text,
        "enemy/001 /components/core.health/value=1\n\
         enemy/002 /components/core.health/value=2\n\
         enemy/004 /components/core.health/value=4\n\
         matched=3 rev=1"
    );
}

#[test]
fn where_sees_prefab_inherited_values_and_paginates_in_lines_mode() {
    let mut store = fixture();
    let out = run(
        &mut store,
        json!({"op":"select","query":{"from":"entities","tags":["enemy"],
            "where":[{"field":"/components/core.health/value","op":"eq","value":100}],
            "format":"lines","limit":2}}),
    );
    let text = out["text"].as_str().unwrap();
    assert!(text.starts_with("enemy/000\nenemy/003\nmatched="), "{text}");
    // 40 prefab instances plus enemy/100 itself (value 100).
    assert!(text.contains("matched=41 rev=1 next="), "{text}");
    assert!(out["next_cursor"].is_string());
}

#[test]
fn set_where_updates_every_match_in_one_revision_with_per_object_diff() {
    let mut store = fixture();
    let out = run(
        &mut store,
        json!({"op":"apply","expected_rev":1,"edits":[{"kind":"set_where",
            "select":{"from":"entities","id_glob":"enemy/*"},
            "field":"/components/core.health/value","value":90}]}),
    );
    assert_eq!(out["rev"], 2, "{out}");
    let changes = out["diff"]["total_changes"].as_u64().unwrap();
    assert!(
        changes >= 120,
        "every enemy must appear in the diff: {changes}"
    );
    let check = run(
        &mut store,
        json!({"op":"select","query":{"from":"entities","tags":["enemy"],
            "where":[{"field":"/components/core.health/value","op":"ne","value":90}],
            "format":"lines"}}),
    );
    assert_eq!(check["text"], "matched=0 rev=2");
    // Prefab instances got a local override; the prefab itself is untouched.
    let prefab = run(
        &mut store,
        json!({"op":"select","query":{"from":"prefabs","ids":["enemy.basic"],
            "fields":["/components/core.health/value"]}}),
    );
    assert_eq!(
        prefab["items"][0]["data"]["/components/core.health/value"],
        100
    );
    // Props were not matched.
    let props = run(
        &mut store,
        json!({"op":"select","query":{"from":"entities","id_glob":"prop/*",
            "where":[{"field":"/components/core.health/value","op":"eq","value":1}],"format":"lines"}}),
    );
    assert!(props["text"].as_str().unwrap().ends_with("matched=5 rev=2"));
}

#[test]
fn set_where_refuses_an_expansion_over_its_byte_budget_before_copying() {
    // 150 KB request, 120 matches: 18 MB would be written, over the 16 MiB budget.
    let mut store = fixture();
    let request = parse_request(
        &json!({"op":"apply","expected_rev":1,"edits":[
            {"kind":"set_where","select":{"from":"entities","id_glob":"enemy/*"},"field":"/label","value":"x".repeat(150_000)}]})
        .to_string(),
    )
    .unwrap();
    let response = axiom_core::error_response(&store.execute(request).unwrap_err());
    assert_eq!(
        response["diagnostics"][0]["code"], "budget_exceeded",
        "{response}"
    );
    assert!(response.to_string().contains("120 matches"), "{response}");
    assert_eq!(store.current().unwrap().0, 1, "nothing was committed");
}

#[test]
fn set_where_edits_of_one_batch_share_one_expansion_budget() {
    // Each edit writes 120 matches x ~4 KB = ~481 KB, far under the 16 MiB budget on its own;
    // 34 of them fit the batch's shared budget, the 35th is refused before it copies.
    let mut world = fixture().current().unwrap().1;
    world.component_schemas.insert(
        "game.note".into(),
        serde_json::from_value(json!({"fields":{"text":{"ty":"string"}}})).unwrap(),
    );
    let mut store = Store::memory_with(Registry::core(), world).unwrap();
    let edit = json!({"kind":"set_where","select":{"from":"entities","id_glob":"enemy/*"},
        "field":"/components/game.note","value":{"text":"x".repeat(4000)}});
    let batch = |op: &str, n: usize| {
        parse_request(&json!({"op":op,"expected_rev":1,"edits":vec![edit.clone(); n]}).to_string())
            .unwrap()
    };
    let fits = store.execute(batch("preview", 34)).unwrap();
    assert_eq!(fits["changed"], true, "{fits}");
    let response = axiom_core::error_response(&store.execute(batch("apply", 36)).unwrap_err());
    assert_eq!(
        response["diagnostics"][0]["code"], "budget_exceeded",
        "{response}"
    );
    assert!(
        response.to_string().contains("split the batch"),
        "{response}"
    );
    assert_eq!(store.current().unwrap().0, 1, "nothing was committed");
}

#[test]
fn integer_predicates_compare_exactly_beyond_two_to_the_53() {
    let mut world = World::empty("ints");
    world.component_schemas.insert(
        "game.counter".into(),
        serde_json::from_value(json!({"fields":{"n":{"ty":"int"}}})).unwrap(),
    );
    // 2^53 and 2^53 + 1 are the same f64.
    for (id, n) in [
        ("a", 9_007_199_254_740_992_i64),
        ("b", 9_007_199_254_740_993),
    ] {
        let mut e = Entity::new(id);
        e.set("game.counter", json!({ "n": n })).unwrap();
        world.entities.push(e);
    }
    let mut store = Store::memory_with(Registry::core(), world).unwrap();
    let find = |store: &mut Store, op: &str| {
        run(
            store,
            json!({"op":"select","query":{"from":"entities","where":[{"field":"/components/game.counter/n","op":op,"value":9_007_199_254_740_993_i64}],"format":"lines"}}),
        )["text"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let eq = find(&mut store, "eq");
    assert!(eq.starts_with('b') && eq.contains("matched=1"), "{eq}");
    let lt = find(&mut store, "lt");
    assert!(lt.starts_with('a') && lt.contains("matched=1"), "{lt}");
    // set_where changes exactly the matching object.
    run(
        &mut store,
        json!({"op":"apply","expected_rev":1,"edits":[{"kind":"set_where","select":{"from":"entities","where":[{"field":"/components/game.counter/n","op":"eq","value":9_007_199_254_740_993_i64}]},"field":"/label","value":"hit"}]}),
    );
    let world = store.current().unwrap().1;
    assert_eq!(world.entity("b").unwrap().label.as_deref(), Some("hit"));
    assert_eq!(world.entity("a").unwrap().label, None);
}

#[test]
fn selecting_from_a_malformed_intermediate_resource_is_an_error_not_a_panic() {
    // The batch first writes widgets without ids, then selects from them: set_where runs on
    // the not-yet-validated intermediate world and must answer with a diagnostic.
    let mut store = fixture();
    let request = parse_request(
        &json!({"op":"apply","expected_rev":1,"edits":[
            {"kind":"set_resource","id":"ui.document","value":{"widgets":[{}]}},
            {"kind":"set_where","select":{"from":"widgets"},"field":"/text","value":"x"}]})
        .to_string(),
    )
    .unwrap();
    let error = store.execute(request).unwrap_err();
    let response = axiom_core::error_response(&error);
    assert_eq!(response["ok"], false, "{response}");
    assert!(response.to_string().contains("no string id"), "{response}");
    assert_eq!(store.current().unwrap().0, 1, "nothing was committed");
}

#[test]
fn set_where_rejects_empty_and_oversized_matches_without_writing() {
    let mut store = fixture();
    for (select, code) in [
        (json!({"from":"entities","id_glob":"ghost/*"}), "no_match"),
        (
            json!({"from":"entities","id_glob":"enemy/*","limit":10}),
            "too_many_matches",
        ),
    ] {
        let request = parse_request(
            &json!({"op":"apply","expected_rev":1,"edits":[{"kind":"set_where","select":select,
                "field":"/components/core.health/value","value":1}]})
            .to_string(),
        )
        .unwrap();
        let error = store.execute(request).unwrap_err();
        let response = axiom_core::error_response(&error);
        assert_eq!(response["diagnostics"][0]["code"], code, "{response}");
    }
    let (rev, _) = store.current().unwrap();
    assert_eq!(rev, 1, "failed selector edits must not write a revision");
}

#[test]
fn compact_text_rounds_f32_noise_for_display_only() {
    use axiom_core::local::display_value;
    let noisy = json!({"p":[0.8999999761581421_f64, 1.7999999523162842_f64, 100.0, 3]});
    assert_eq!(display_value(&noisy), json!({"p":[0.9, 1.8, 100.0, 3]}));
}

#[test]
fn glob_supports_star_and_question_mark() {
    use axiom_core::local::glob_match;
    assert!(glob_match("enemy/*", "enemy/007"));
    assert!(glob_match("enemy/00?", "enemy/007"));
    assert!(!glob_match("enemy/00?", "enemy/0071"));
    assert!(glob_match("*/boss", "level2/boss"));
    assert!(glob_match("*", ""));
    assert!(!glob_match("prop/*", "enemy/1"));
}
