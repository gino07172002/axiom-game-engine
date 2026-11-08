use axiom_core::{gameplay::ScalarType, ContentPackage, NativeEvent, Registry, World};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture {
    root: PathBuf,
    registry: Registry,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../.tools/tmp/m4-pack-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut registry = Registry::core();
        registry.native_events.insert(
            "counter.done".into(),
            NativeEvent {
                version: 1,
                publisher: "motion.integrate".into(),
                fields: [("amount".into(), ScalarType::Int)].into(),
            },
        );
        let f = Self { root, registry };
        f.write("provider/manifest.json",json!({
            "id":"provider","version":1,"content_version":3,"world":"world.json","capabilities":{"core":1,"motion":1},
            "exports":{"events":{"counter.done":{"version":1,"publisher":"motion.integrate","fields":{"amount":"int"}}},
                "resources":{"input.bindings":{"version":1,"owner":"motion.integrate"}}}
        }));
        f.write("provider/world.json",json!({
            "schema_version":2,"id":"provider","title":"Provider","capabilities":{"core":1,"motion":1},
            "systems":["motion.integrate"],"resources":{"input.bindings":{"buttons":{}}}
        }));
        f.write("quest/manifest.json",json!({
            "id":"quest","version":1,"world":"world.json","capabilities":{"core":1,"behavior":1},
            "dependencies":{"provider":{"path":"../provider","content_version":3}},
            "imports":{"events":{"counter.done":{"package":"provider","version":1,"consumer":"behavior.execute","fields":{"amount":"int"}}},
                "resources":{"input.bindings":{"package":"provider","version":1,"consumer":"behavior.execute"}}}
        }));
        f.write("quest/world.json",json!({
            "schema_version":2,"id":"quest","title":"Quest","capabilities":{"core":1,"behavior":1},"systems":["behavior.execute"],
            "resources":{"behavior.program":{"variables":{"count":0},"events":{"counter.done":{"fields":{"amount":"int"}}},
                "rules":[{"id":"count","event":"counter.done","actions":[{"kind":"add","variable":"count","value":{"kind":"event","field":"amount"}}]}]}}
        }));
        f.write("manifest.json",json!({
            "id":"root","version":1,"world":"world.json","capabilities":{"core":1},
            "dependencies":{"quest":{"path":"quest","content_version":1},"provider":{"path":"provider","content_version":3}}
        }));
        f.write("world.json",json!({"schema_version":2,"id":"combined","title":"Combined","capabilities":{"core":1}}));
        f
    }
    fn write(&self, file: &str, value: Value) {
        let path = self.root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
    fn edit(&self, file: &str, edit: impl FnOnce(&mut Value)) {
        let mut v: Value =
            serde_json::from_slice(&std::fs::read(self.root.join(file)).unwrap()).unwrap();
        edit(&mut v);
        self.write(file, v);
    }
    fn load(&self) -> anyhow::Result<ContentPackage> {
        ContentPackage::load(&self.root, &self.registry)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn dependencies_are_deduplicated_locked_and_survive_world_roundtrip() {
    let f = Fixture::new();
    let package = f.load().unwrap();
    let lock = package.world.packages.as_ref().unwrap();
    assert_eq!(lock.packages.len(), 3);
    assert_eq!(lock.packages["provider"].content_version, 3);
    assert_eq!(
        package
            .world
            .systems
            .iter()
            .filter(|s| *s == "motion.integrate")
            .count(),
        1
    );
    let roundtrip: World = serde_json::from_value(json!(package.world)).unwrap();
    roundtrip.compile(&f.registry).unwrap();
    assert_eq!(
        roundtrip.packages.as_ref().unwrap().summary()["root"],
        "root"
    );
}
#[test]
fn exact_versions_and_optional_source_fingerprints_reject_drift() {
    let f = Fixture::new();
    f.edit("quest/manifest.json", |v| {
        v["dependencies"]["provider"]["content_version"] = json!(4)
    });
    assert!(f
        .load()
        .unwrap_err()
        .to_string()
        .contains("pinned content version mismatch"));
}
#[test]
fn fingerprint_pin_rejects_changed_source_without_a_release_bump() {
    let f = Fixture::new();
    let hash = f.load().unwrap().world.packages.unwrap().packages["provider"]
        .fingerprint
        .clone();
    f.edit("quest/manifest.json", |v| {
        v["dependencies"]["provider"]["fingerprint"] = json!(hash)
    });
    f.edit("provider/world.json", |v| v["title"] = json!("Changed"));
    assert!(f
        .load()
        .unwrap_err()
        .to_string()
        .contains("fingerprint mismatch"));
}
#[test]
fn declared_assets_are_checked_against_their_sha256_inventory() {
    use sha2::{Digest, Sha256};
    let f = Fixture::new();
    let bytes = b"fixture";
    std::fs::write(f.root.join("provider/data.bin"), bytes).unwrap();
    f.edit("provider/manifest.json", |v| {
        v["assets"] = json!({"data.bin":format!("{:x}",Sha256::digest(bytes))})
    });
    let package = f.load().unwrap();
    assert_eq!(
        package.world.packages.unwrap().packages["provider"]
            .assets
            .len(),
        1
    );
    std::fs::write(f.root.join("provider/data.bin"), b"changed").unwrap();
    assert!(f
        .load()
        .unwrap_err()
        .to_string()
        .contains("SHA-256 mismatch"));
}
#[test]
fn payload_versions_and_resource_contracts_fail_before_activation() {
    for (category, field, value) in [
        ("events", "version", json!(2)),
        ("events", "fields", json!({"amount":"text"})),
        ("resources", "version", json!(2)),
    ] {
        let f = Fixture::new();
        let id = if category == "events" {
            "counter.done"
        } else {
            "input.bindings"
        };
        f.edit("quest/manifest.json", |v| {
            v["imports"][category][id][field] = value
        });
        assert!(f.load().is_err());
    }
}
#[test]
fn cycles_escapes_and_duplicate_object_ids_are_rejected() {
    let f = Fixture::new();
    f.edit("provider/manifest.json", |v| {
        v["dependencies"] = json!({"root":{"path":"..","content_version":1}})
    });
    assert!(f.load().unwrap_err().to_string().contains("cycle"));
    let f = Fixture::new();
    f.edit("quest/manifest.json", |v| {
        v["dependencies"]["provider"]["path"] = json!("../../..")
    });
    assert!(f.load().unwrap_err().to_string().contains("escapes"));
    let f = Fixture::new();
    for file in ["provider/world.json", "quest/world.json"] {
        f.edit(file, |v| v["entities"] = json!([{"id":"collision"}]));
    }
    assert!(f
        .load()
        .unwrap_err()
        .to_string()
        .contains("duplicate entity"));
}
#[test]
fn resource_merges_are_opt_in_and_reject_ambiguous_map_entries() {
    let f = Fixture::new();
    f.edit("world.json", |v| {
        v["resources"] = json!({"input.bindings":{"buttons":{"jump":"Space"}}})
    });
    let package = f.load().unwrap();
    assert_eq!(
        package.world.resources["input.bindings"]["buttons"]["jump"],
        "Space"
    );
    f.edit("provider/world.json", |v| {
        v["resources"]["input.bindings"]["buttons"]["jump"] = json!("Space")
    });
    assert!(f
        .load()
        .unwrap_err()
        .to_string()
        .contains("duplicate resource entry"));
}
