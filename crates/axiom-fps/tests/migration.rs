use axiom_core::*;
use serde_json::json;

#[test]
fn inherited_fps_components_work_with_legacy_group_edits() {
    let mut w = axiom_fps::demo_world().unwrap();
    let e = w.entity_mut("enemy/0").unwrap();
    let base = Prefab {
        components: e.components.clone(),
    };
    e.components.clear();
    e.prefab = Some("inherited".into());
    w.prefabs.insert("inherited".into(), base);
    w.compile(&axiom_fps::registry()).unwrap();
    let mut s = Store::memory_with(axiom_fps::registry(), w).unwrap();
    s.apply(
        None,
        vec![
            Edit::set("enemies.speed", json!(0.5)),
            Edit::set("enemies.count", json!(10)),
        ],
    )
    .unwrap();
    let c = s
        .current()
        .unwrap()
        .1
        .compile(&axiom_fps::registry())
        .unwrap();
    let e = &c.entities[c.indices["enemy/0"]];
    assert_eq!(e.components["fps.sentinel"]["speed"], 0.5);
    assert_eq!(axiom_fps::from_world(&c.source).unwrap().enemies.count, 10);
}
#[test]
fn legacy_sqlite_is_migrated_once_and_undo_keeps_historical_data() {
    let path = std::env::temp_dir().join(format!(
        "axiom-v1-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let old = axiom_fps::LegacyWorld::demo();
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE revisions(rev INTEGER PRIMARY KEY AUTOINCREMENT,world TEXT NOT NULL,reason TEXT NOT NULL,created_at TEXT NOT NULL DEFAULT 'fixture');").unwrap();
        c.execute(
            "INSERT INTO revisions(world,reason) VALUES (?1,'legacy fixture')",
            [serde_json::to_string(&old).unwrap()],
        )
        .unwrap();
    }
    {
        let mut s = axiom_fps::open_store(&path).unwrap();
        assert_eq!(s.revision().unwrap(), 2);
        assert_eq!(s.current().unwrap().1.schema_version, 2);
        assert_eq!(axiom_fps::from_world(&s.current().unwrap().1).unwrap(), old);
        s.execute(Request::Undo {
            rev: 1,
            expected_rev: Some(2),
        })
        .unwrap();
        assert_eq!(s.current().unwrap().1.schema_version, 2);
        assert_eq!(s.revision().unwrap(), 3);
    }
    {
        let s = axiom_fps::open_store(&path).unwrap();
        assert_eq!(s.revision().unwrap(), 3);
        let c = rusqlite::Connection::open(&path).unwrap();
        let raw: String = c
            .query_row("SELECT world FROM revisions WHERE rev=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&raw).unwrap()["schema_version"],
            1
        );
    }
    std::fs::remove_file(path).unwrap();
}
#[test]
fn generic_edits_and_fps_adapter_preserve_unrelated_entities() {
    let mut world = axiom_fps::demo_world().unwrap();
    let mut other = Entity::new("story/book");
    other
        .set(
            "core.transform",
            Transform {
                position: [1.0, 4.0, 1.0],
                ..Default::default()
            },
        )
        .unwrap();
    world.entities.push(other.clone());
    let mut s = Store::memory_with(axiom_fps::registry(), world).unwrap();
    s.apply(
        None,
        vec![
            Edit::set("weapon.damage", json!(100)),
            Edit::set("enemies.count", json!(12)),
        ],
    )
    .unwrap();
    let w = s.current().unwrap().1;
    assert_eq!(w.entity("story/book"), Some(&other));
    assert_eq!(axiom_fps::from_world(&w).unwrap().weapon.damage, 100.0);
}
#[test]
fn v1_import_exports_v2_and_preview_is_read_only() {
    let mut s = Store::memory_with(axiom_fps::registry(), World::empty("empty")).unwrap();
    let edit = Edit::Core(CoreEdit::ReplaceWorld {
        world: json!(axiom_fps::LegacyWorld::demo()),
    });
    let p = s
        .execute(Request::Preview {
            expected_rev: Some(1),
            edits: vec![edit.clone()],
        })
        .unwrap();
    assert_eq!(p["entities"], 16);
    assert!(s.current().unwrap().1.entities.is_empty());
    s.apply(Some(1), vec![edit]).unwrap();
    assert_eq!(json!(s.current().unwrap().1)["schema_version"], 2);
}
#[test]
fn mixed_generic_and_group_edits_fail_recoverably_instead_of_panicking() {
    let mut s =
        Store::memory_with(axiom_fps::registry(), axiom_fps::demo_world().unwrap()).unwrap();
    let before = s.current().unwrap();
    for path in ["enemies.speed", "enemies.damage"] {
        let edits = vec![
            Edit::Core(CoreEdit::SetComponent {
                entity: "enemy/0".into(),
                component: "fps.sentinel".into(),
                value: json!(5),
            }),
            Edit::set(path, json!(0.5)),
        ];
        let e = s.apply(Some(before.0), edits.clone()).unwrap_err();
        assert!(format!("{e:#}").contains("must be an object"), "{e:#}");
        assert!(s
            .execute(Request::Preview {
                expected_rev: Some(before.0),
                edits,
            })
            .is_err());
    }
    let edits = vec![
        Edit::Core(CoreEdit::SetComponent {
            entity: "player".into(),
            component: "fps.weapon".into(),
            value: json!([1]),
        }),
        Edit::set("weapon.damage", json!(30)),
    ];
    assert!(s.apply(Some(before.0), edits).is_err());
    assert_eq!(s.current().unwrap(), before);
}
#[test]
fn compatibility_deletes_share_the_v2_reference_cleanup_and_prefab_resolution() {
    let mut w = axiom_fps::demo_world().unwrap();
    let cover = axiom_fps::from_world(&w).unwrap().obstacles[0].id.clone();
    let cover_entity = format!("obstacle/{cover}");
    // The cover component comes from a prefab; the sentinel and cover are scene members.
    let e = w.entity_mut(&cover_entity).unwrap();
    let base = Prefab {
        components: std::mem::take(&mut e.components),
    };
    e.prefab = Some("cover".into());
    w.prefabs.insert("cover".into(), base);
    w.scenes.push(Scene {
        id: "arena".into(),
        entities: vec!["enemy/0".into(), "enemy/7".into(), cover_entity.clone()],
        active: true,
    });
    w.compile(&axiom_fps::registry()).unwrap();
    let mut s = Store::memory_with(axiom_fps::registry(), w).unwrap();
    s.apply(
        None,
        vec![
            Edit::set("enemies.count", json!(4)),
            Edit::Extension(json!({"kind":"remove_obstacle","id":cover})),
        ],
    )
    .unwrap();
    let (_, w) = s.current().unwrap();
    assert!(w.entity("enemy/7").is_none() && w.entity(&cover_entity).is_none());
    assert_eq!(w.scenes[0].entities, vec!["enemy/0".to_string()]);
    assert_eq!(axiom_fps::from_world(&w).unwrap().enemies.count, 4);
}
