//! The store's decoded-revision cache must be indistinguishable from a fresh read.
use axiom_core::{CoreEdit, Edit, Store};
use serde_json::json;

fn edit(id: &str, x: f64) -> Vec<Edit> {
    moved(id, [x, 0.25, -1.5])
}
fn moved(id: &str, p: [f64; 3]) -> Vec<Edit> {
    vec![Edit::Core(CoreEdit::SetField {
        target: axiom_core::local::Target::Entity { id: id.into() },
        field: "/components/core.transform/position".into(),
        value: json!(p),
    })]
}

#[test]
fn cached_revision_equals_a_fresh_decode_for_every_content_pack() {
    let registry = axiom_runtime::installed_registry();
    for pack in [
        "fps",
        "rts-bench",
        "rts",
        "axiom-town",
        "effects-tour",
        "light-lab",
    ] {
        let world = axiom_core::ContentPackage::load(
            format!("{}/../../content/{pack}", env!("CARGO_MANIFEST_DIR")),
            &registry.authoring,
        )
        .unwrap()
        .world;
        let (target, base) = world
            .entities
            .iter()
            .find(|e| {
                !e.components.contains_key("render.camera")
                    && e.components
                        .get("core.transform")
                        .and_then(|t| t.get("position"))
                        .is_some()
            })
            .map(|e| {
                let p: Vec<f64> =
                    serde_json::from_value(e.components["core.transform"]["position"].clone())
                        .unwrap();
                (e.id.clone(), p)
            })
            .unwrap();
        let mut store = Store::memory_with(registry.authoring.clone(), world).unwrap();
        for i in 0..3 {
            let rev = store.revision().unwrap();
            let result = store
                .apply(
                    Some(rev),
                    moved(
                        &target,
                        [base[0] + 0.001 * (i + 1) as f64, base[1], base[2]],
                    ),
                )
                .unwrap();
            assert_eq!(result["changed"], true, "{pack}");
            let (cached_rev, cached) = store.current_shared().unwrap();
            let (fresh_rev, fresh) = store.current().unwrap();
            assert_eq!(cached_rev, fresh_rev, "{pack}");
            assert_eq!(
                *cached, fresh,
                "{pack}: cached world differs from stored text"
            );
            assert_eq!(
                serde_json::to_string(&*cached).unwrap(),
                serde_json::to_string(&fresh).unwrap(),
                "{pack}"
            );
        }
    }
}

#[test]
fn cache_sees_revisions_committed_by_another_connection() {
    let registry = axiom_runtime::installed_registry();
    let world: axiom_core::World =
        serde_json::from_str(include_str!("../../../content/two-cameras/world.json")).unwrap();
    let dir = std::env::temp_dir().join(format!("axiom-store-cache-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("shared.sqlite");
    let _ = std::fs::remove_file(&db);
    let mut a = Store::open_with(&db, registry.authoring.clone(), world.clone()).unwrap();
    let mut b = Store::open_with(&db, registry.authoring.clone(), world).unwrap();
    a.apply(Some(1), edit("marker", 1.0)).unwrap();
    assert_eq!(a.current_shared().unwrap().0, 2);
    b.apply(Some(2), edit("marker", 2.0)).unwrap();
    let (rev, seen) = a.current_shared().unwrap();
    assert_eq!(rev, 3);
    assert_eq!(*seen, a.current().unwrap().1);
    assert_eq!(
        seen.entity("marker").unwrap().components["core.transform"]["position"][0],
        2.0
    );
    // Undo appends through a path that does not fill the cache; reads still follow.
    a.execute(axiom_core::Request::Undo {
        rev: 2,
        expected_rev: Some(3),
    })
    .unwrap();
    let (rev, seen) = a.current_shared().unwrap();
    assert_eq!(rev, 4);
    assert_eq!(*seen, a.current().unwrap().1);
}

#[test]
fn connections_sharing_the_cache_reuse_the_committed_world() {
    let registry = axiom_runtime::installed_registry();
    let world: axiom_core::World =
        serde_json::from_str(include_str!("../../../content/two-cameras/world.json")).unwrap();
    let dir = std::env::temp_dir().join(format!("axiom-store-share-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("shared.sqlite");
    let _ = std::fs::remove_file(&db);
    let app = Store::open_with(&db, registry.authoring.clone(), world.clone()).unwrap();
    let mut bridge = Store::open_with(&db, registry.authoring.clone(), world).unwrap();
    bridge.share_latest_cache(app.latest_cache());
    bridge.apply(Some(1), edit("marker", 3.0)).unwrap();
    let (rev, seen) = app.current_shared().unwrap();
    let (_, committed) = bridge.current_shared().unwrap();
    assert_eq!(rev, 2);
    assert!(
        std::sync::Arc::ptr_eq(&seen, &committed),
        "no second decode"
    );
    assert_eq!(*seen, app.current().unwrap().1);
}
