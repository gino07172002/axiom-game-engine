use crate::{
    gameplay::{GameAction, Narrative, Program, ScalarType},
    local::{at, escape, fingerprint},
    NativeEvent, Registry, World,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
fn one() -> u32 {
    1
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub path: String,
    pub content_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResourceExport {
    pub version: u32,
    pub owner: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EventImport {
    pub package: String,
    pub version: u32,
    pub consumer: String,
    pub fields: BTreeMap<String, ScalarType>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResourceImport {
    pub package: String,
    pub version: u32,
    pub consumer: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Exports {
    pub events: BTreeMap<String, NativeEvent>,
    pub resources: BTreeMap<String, ResourceExport>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Imports {
    pub events: BTreeMap<String, EventImport>,
    pub resources: BTreeMap<String, ResourceImport>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageManifest {
    pub id: String,
    pub version: u32,
    pub world: String,
    pub capabilities: BTreeMap<String, u32>,
    #[serde(default = "one")]
    pub content_version: u32,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
    #[serde(default)]
    pub exports: Exports,
    #[serde(default)]
    pub imports: Imports,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub assets: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LockedPackage {
    pub content_version: u32,
    pub fingerprint: String,
    pub dependencies: BTreeMap<String, u32>,
    pub exports: Exports,
    pub imports: Imports,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub assets: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackageLock {
    pub root: String,
    pub packages: BTreeMap<String, LockedPackage>,
}

impl PackageLock {
    pub fn validate(&self, w: &World, r: &Registry) -> Result<()> {
        ensure!(
            (1..=32).contains(&self.packages.len()) && self.packages.contains_key(&self.root),
            "invalid package lock"
        );
        let scheduled: BTreeSet<_> = w.systems.iter().map(String::as_str).collect();
        let mut providers = BTreeMap::new();
        let mut resources = BTreeMap::new();
        for (id, p) in &self.packages {
            crate::valid_id(id)?;
            ensure!(
                p.assets.len() <= 256
                    && p.assets.iter().all(|(path, hash)| path.len() <= 256
                        && hash.len() == 64
                        && hash.bytes().all(|b| b.is_ascii_hexdigit())),
                "invalid locked asset inventory"
            );
            ensure!(
                p.content_version > 0
                    && p.fingerprint.len() == 16
                    && p.fingerprint.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid locked version or fingerprint"
            );
            ensure!(
                p.dependencies.len() <= 16
                    && p.exports.events.len() + p.exports.resources.len() <= 64
                    && p.imports.events.len() + p.imports.resources.len() <= 64,
                "package contract budget exceeded"
            );
            for (dep, version) in &p.dependencies {
                ensure!(
                    self.packages
                        .get(dep)
                        .is_some_and(|p| p.content_version == *version),
                    "dependency version mismatch for {dep}"
                );
            }
            for (event, contract) in &p.exports.events {
                at(
                    &format!("/packages/@{}/exports/events/{}", escape(id), escape(event)),
                    (|| {
                        ensure!(
                            providers.insert(event, id).is_none(),
                            "multiple owners for event {event}"
                        );
                        ensure!(
                            r.native_events.get(event) == Some(contract),
                            "native event version, publisher or payload mismatch"
                        );
                        ensure!(
                            scheduled.contains(contract.publisher.as_str()),
                            "event publisher is not scheduled"
                        );
                        Ok(())
                    })(),
                )?;
            }
            for (resource, export) in &p.exports.resources {
                at(
                    &format!(
                        "/packages/@{}/exports/resources/{}",
                        escape(id),
                        escape(resource)
                    ),
                    (|| {
                        ensure!(
                            resources.insert(resource, id).is_none(),
                            "multiple owners for resource {resource}"
                        );
                        let version = if let Some(descriptor) = r.resources.get(resource) {
                            *r.capabilities
                                .get(&descriptor.capability)
                                .context("resource capability missing")?
                        } else {
                            ensure!(
                                w.resource_schemas.contains_key(resource),
                                "exported resource type missing"
                            );
                            1
                        };
                        ensure!(
                            w.resources.contains_key(resource) && export.version == version,
                            "resource interface version mismatch"
                        );
                        ensure!(
                            scheduled.contains(export.owner.as_str()),
                            "resource owner is not scheduled"
                        );
                        Ok(())
                    })(),
                )?;
            }
        }
        let program = w
            .resources
            .get("behavior.program")
            .map(|v| serde_json::from_value::<Program>(v.clone()))
            .transpose()?;
        for (id, p) in &self.packages {
            for (event, import) in &p.imports.events {
                at(
                    &format!("/packages/@{}/imports/events/{}", escape(id), escape(event)),
                    (|| {
                        ensure!(
                            p.dependencies.contains_key(&import.package),
                            "event import must name a direct dependency"
                        );
                        let provider = self
                            .packages
                            .get(&import.package)
                            .context("event provider missing")?;
                        let export = provider
                            .exports
                            .events
                            .get(event)
                            .context("event is not exported by dependency")?;
                        ensure!(
                            export.version == import.version && export.fields == import.fields,
                            "event contract version or payload mismatch"
                        );
                        ensure!(
                            scheduled.contains(import.consumer.as_str())
                                && r.event_consumers.contains(&import.consumer),
                            "event consumer is unavailable"
                        );
                        if import.consumer == "behavior.execute" {
                            ensure!(
                                program
                                    .as_ref()
                                    .and_then(|p| p.events.get(event))
                                    .is_some_and(|s| s.fields == import.fields),
                                "behavior event schema differs from its imported contract"
                            );
                        }
                        Ok(())
                    })(),
                )?;
            }
            for (resource, import) in &p.imports.resources {
                at(
                    &format!(
                        "/packages/@{}/imports/resources/{}",
                        escape(id),
                        escape(resource)
                    ),
                    (|| {
                        ensure!(
                            p.dependencies.contains_key(&import.package),
                            "resource import must name a direct dependency"
                        );
                        ensure!(
                            self.packages
                                .get(&import.package)
                                .and_then(|p| p.exports.resources.get(resource))
                                .is_some_and(|e| e.version == import.version),
                            "resource import version mismatch"
                        );
                        ensure!(
                            scheduled.contains(import.consumer.as_str()),
                            "resource reader is not scheduled"
                        );
                        Ok(())
                    })(),
                )?;
            }
        }
        // A contract publisher owns its native event. Authored rules cannot impersonate it.
        let verify_actions = |actions: &[GameAction]| -> Result<()> {
            for action in actions {
                if let GameAction::Emit { event, .. } | GameAction::Delay { event, .. } = action {
                    ensure!(
                        !providers.contains_key(event),
                        "authored behavior cannot publish owned native event {event}"
                    );
                }
            }
            Ok(())
        };
        if let Some(program) = &program {
            ensure!(
                program
                    .inputs
                    .values()
                    .all(|event| !providers.contains_key(event)),
                "authored input cannot publish an owned native event"
            );
            for rule in &program.rules {
                verify_actions(&rule.actions)?;
            }
        }
        if let Some(value) = w.resources.get("narrative.graph") {
            let story: Narrative = serde_json::from_value(value.clone())?;
            for node in story.nodes {
                verify_actions(&node.on_enter)?;
                for choice in node.choices {
                    verify_actions(&choice.actions)?;
                }
            }
        }
        // Validate even deserialized locks, independently of the filesystem loader.
        fn visit<'a>(
            id: &'a str,
            lock: &'a PackageLock,
            visiting: &mut BTreeSet<&'a str>,
            done: &mut BTreeSet<&'a str>,
        ) -> Result<()> {
            if done.contains(id) {
                return Ok(());
            }
            ensure!(
                visiting.len() < 16 && visiting.insert(id),
                "dependency cycle or depth limit"
            );
            for dep in lock.packages[id].dependencies.keys() {
                visit(dep, lock, visiting, done)?;
            }
            visiting.remove(id);
            done.insert(id);
            Ok(())
        }
        let mut done = BTreeSet::new();
        visit(&self.root, self, &mut BTreeSet::new(), &mut done)?;
        ensure!(
            done.len() == self.packages.len(),
            "package lock contains unreachable packages"
        );
        Ok(())
    }
    pub fn summary(&self) -> Value {
        json!({"root":self.root,"packages":self.packages.iter().map(|(id,p)|json!({
            "id":id,"content_version":p.content_version,"fingerprint":p.fingerprint,"dependencies":p.dependencies,"assets":p.assets.len(),
            "exports":{"events":p.exports.events.keys().collect::<Vec<_>>(),"resources":p.exports.resources.keys().collect::<Vec<_>>()},
            "imports":{"events":p.imports.events.keys().collect::<Vec<_>>(),"resources":p.imports.resources.keys().collect::<Vec<_>>()}
        })).collect::<Vec<_>>()})
    }
}
#[derive(Debug)]
pub struct ContentPackage {
    pub manifest: PackageManifest,
    pub world: World,
}
impl ContentPackage {
    pub fn load(path: impl AsRef<Path>, registry: &Registry) -> Result<Self> {
        let root = path.as_ref().canonicalize()?;
        let mut loader = Loader {
            boundary: root.clone(),
            registry,
            visiting: BTreeSet::new(),
            nodes: BTreeMap::new(),
            order: vec![],
            bytes: 0,
            asset_bytes: 0,
        };
        let id = loader.visit(&root, None)?;
        let manifest = loader.nodes[&id].0.clone();
        let mut world = World::empty(&id);
        for id in &loader.order {
            merge_world(&mut world, &loader.nodes[id].1, registry)?;
        }
        world.id = loader.nodes[&id].1.id.clone();
        world.title = loader.nodes[&id].1.title.clone();
        // Legacy single-package worlds retain their original serialization.
        if loader.nodes.len() > 1
            || !manifest.exports.events.is_empty()
            || !manifest.exports.resources.is_empty()
            || !manifest.imports.events.is_empty()
            || !manifest.imports.resources.is_empty()
        {
            world.packages = Some(PackageLock {
                root: id,
                packages: loader
                    .nodes
                    .iter()
                    .map(|(id, (m, _, _, hash))| {
                        (
                            id.clone(),
                            LockedPackage {
                                content_version: m.content_version,
                                fingerprint: hash.clone(),
                                dependencies: m
                                    .dependencies
                                    .iter()
                                    .map(|(id, d)| (id.clone(), d.content_version))
                                    .collect(),
                                exports: m.exports.clone(),
                                imports: m.imports.clone(),
                                assets: m.assets.clone(),
                            },
                        )
                    })
                    .collect(),
            });
        }
        world
            .compile(registry)
            .context("content package validation")?;
        Ok(Self { manifest, world })
    }
}
struct Loader<'a> {
    boundary: PathBuf,
    registry: &'a Registry,
    visiting: BTreeSet<PathBuf>,
    nodes: BTreeMap<String, (PackageManifest, World, PathBuf, String)>,
    order: Vec<String>,
    bytes: usize,
    asset_bytes: u64,
}
impl Loader<'_> {
    fn visit(&mut self, root: &Path, expected: Option<(&str, &Dependency)>) -> Result<String> {
        ensure!(
            root.starts_with(&self.boundary),
            "dependency path escapes the root package"
        );
        ensure!(
            self.visiting.len() < 16 && !self.visiting.contains(root),
            "package dependency cycle or depth limit"
        );
        let registry = self.registry;
        let manifest_raw = std::fs::read(root.join("manifest.json"))?;
        ensure!(manifest_raw.len() <= 65536, "manifest exceeds 64 KiB");
        let manifest: PackageManifest = serde_json::from_slice(&manifest_raw)?;
        crate::valid_id(&manifest.id)?;
        ensure!(
            manifest.version == 1
                && manifest.content_version > 0
                && manifest.dependencies.len() <= 16,
            "unsupported package format/version or dependency budget"
        );
        let world_path = root.join(&manifest.world).canonicalize()?;
        ensure!(
            world_path.starts_with(root),
            "package world path escapes package directory"
        );
        let raw = std::fs::read_to_string(world_path)?;
        ensure!(raw.len() <= 4_194_304, "package world exceeds 4 MiB");
        let hash = fingerprint(&(
            serde_json::from_slice::<Value>(&manifest_raw)?,
            serde_json::from_str::<Value>(&raw)?,
        ))?;
        if let Some((id, dep)) = expected {
            at(
                &format!("/dependencies/{}", escape(id)),
                (|| {
                    ensure!(
                        manifest.id == id && manifest.content_version == dep.content_version,
                        "package ID or pinned content version mismatch"
                    );
                    ensure!(
                        dep.fingerprint.as_ref().is_none_or(|h| h == &hash),
                        "pinned source fingerprint mismatch"
                    );
                    Ok(())
                })(),
            )?;
        }
        if let Some((_, _, path, old_hash)) = self.nodes.get(&manifest.id) {
            ensure!(
                path == root && old_hash == &hash,
                "conflicting sources for package ID {}",
                manifest.id
            );
            return Ok(manifest.id);
        }
        ensure!(
            self.nodes.len() + self.visiting.len() < 32,
            "package count exceeds 32"
        );
        self.bytes += raw.len() + manifest_raw.len();
        ensure!(
            self.bytes <= 8_388_608,
            "composed package sources exceed 8 MiB"
        );
        self.visiting.insert(root.into());
        for (id, dep) in &manifest.dependencies {
            let path = root
                .join(&dep.path)
                .canonicalize()
                .with_context(|| format!("dependency {id} missing"))?;
            self.visit(&path, Some((id, dep)))?;
        }
        ensure!(
            manifest.assets.len() <= 256,
            "declared asset count exceeds 256"
        );
        for (relative, expected) in &manifest.assets {
            at(
                &format!("/assets/{}", escape(relative)),
                (|| {
                    ensure!(
                        relative.len() <= 256
                            && expected.len() == 64
                            && expected.bytes().all(|b| b.is_ascii_hexdigit()),
                        "asset requires a relative path and SHA-256"
                    );
                    let file = root.join(relative).canonicalize()?;
                    ensure!(file.starts_with(root), "asset path escapes its package");
                    let length = std::fs::metadata(&file)?.len();
                    self.asset_bytes += length;
                    ensure!(
                        length <= 33_554_432 && self.asset_bytes <= 134_217_728,
                        "declared asset byte budget exceeded"
                    );
                    let bytes = std::fs::read(file)?;
                    ensure!(
                        format!("{:x}", Sha256::digest(&bytes)).eq_ignore_ascii_case(expected),
                        "declared asset SHA-256 mismatch"
                    );
                    Ok(())
                })(),
            )?;
        }
        let mut world = registry.decode(serde_json::from_str(&raw)?)?;
        ensure!(
            world.packages.is_none(),
            "package source world cannot contain a pre-resolved lock"
        );
        ensure!(
            manifest.capabilities == world.capabilities,
            "manifest/world capability mismatch"
        );
        if let Some(doc) = world.resources.get_mut("ui.document") {
            let mut ui: crate::gameplay::UiDocument = serde_json::from_value(doc.clone())?;
            for source in ui.images.values_mut() {
                let asset = root
                    .join(&*source)
                    .canonicalize()
                    .context("missing UI image asset")?;
                ensure!(
                    asset.starts_with(root),
                    "UI image path escapes package directory"
                );
                ensure!(
                    std::fs::metadata(&asset)?.len() <= 4_194_304,
                    "UI image exceeds 4 MiB"
                );
                *source = asset.to_string_lossy().into_owned();
            }
            *doc = serde_json::to_value(ui)?;
        }
        for resolve in &registry.package_resolvers {
            resolve(root, &mut world)?;
        }
        for event in manifest.exports.events.keys() {
            ensure!(
                registry.native_events.contains_key(event),
                "unregistered native event {event}"
            );
            let publisher = &registry.native_events[event].publisher;
            ensure!(
                world.systems.contains(publisher),
                "package must schedule its exported event publisher"
            );
        }
        for resource in manifest.exports.resources.keys() {
            ensure!(
                world.resources.contains_key(resource),
                "package must define its exported resource {resource}"
            );
            ensure!(
                world
                    .systems
                    .contains(&manifest.exports.resources[resource].owner),
                "package must schedule its exported resource owner"
            );
        }
        self.visiting.remove(root);
        let id = manifest.id.clone();
        ensure!(
            !self.nodes.contains_key(&id),
            "duplicate package ID in dependency chain {id}"
        );
        self.nodes
            .insert(id.clone(), (manifest, world, root.into(), hash));
        self.order.push(id.clone());
        Ok(id)
    }
}

fn insert_unique<K: Ord + std::fmt::Debug + Clone, V: Clone>(
    into: &mut BTreeMap<K, V>,
    other: &BTreeMap<K, V>,
) -> Result<()> {
    for (id, value) in other {
        ensure!(!into.contains_key(id), "duplicate package object {id:?}");
        into.insert(id.clone(), value.clone());
    }
    Ok(())
}
fn merge_world(into: &mut World, other: &World, r: &Registry) -> Result<()> {
    for (id, version) in &other.capabilities {
        ensure!(
            into.capabilities.get(id).is_none_or(|v| v == version),
            "capability version conflict {id}"
        );
        into.capabilities.insert(id.clone(), *version);
    }
    insert_unique(&mut into.prefabs, &other.prefabs)?;
    insert_unique(&mut into.component_schemas, &other.component_schemas)?;
    insert_unique(&mut into.resource_schemas, &other.resource_schemas)?;
    for entity in &other.entities {
        ensure!(
            into.entity(&entity.id).is_none(),
            "duplicate entity across packages {}",
            entity.id
        );
        into.entities.push(entity.clone());
    }
    for scene in &other.scenes {
        ensure!(
            !into.scenes.iter().any(|s| s.id == scene.id),
            "duplicate scene across packages {}",
            scene.id
        );
        into.scenes.push(scene.clone());
    }
    for relation in &other.relations {
        ensure!(
            !into.relations.contains(relation),
            "duplicate relation across packages"
        );
        into.relations.push(relation.clone());
    }
    for (id, value) in &other.resources {
        if let Some(previous) = into.resources.get(id) {
            let merge = r.resource_mergers.get(id).with_context(|| {
                format!("resource {id} has multiple definitions without a composition policy")
            })?;
            into.resources.insert(
                id.clone(),
                at(
                    &format!("/resources/{}", escape(id)),
                    merge(previous, value),
                )?,
            );
        } else {
            into.resources.insert(id.clone(), value.clone());
        }
    }
    for system in &other.systems {
        if !into.systems.contains(system) {
            into.systems.push(system.clone());
        }
    }
    Ok(())
}

/// Installed resource types opt into merging disjoint maps and stable-ID lists.
pub fn merge_resource(a: &Value, b: &Value, maps: &[&str], lists: &[&str]) -> Result<Value> {
    let mut result = a.as_object().context("resource must be a record")?.clone();
    for (key, value) in b.as_object().context("resource must be a record")? {
        let Some(previous) = result.get_mut(key) else {
            result.insert(key.clone(), value.clone());
            continue;
        };
        if maps.contains(&key.as_str()) {
            let into = previous.as_object_mut().context("expected resource map")?;
            for (id, v) in value.as_object().context("expected resource map")? {
                ensure!(
                    !into.contains_key(id),
                    "duplicate resource entry {key}/{id}"
                );
                into.insert(id.clone(), v.clone());
            }
        } else if lists.contains(&key.as_str()) {
            let into = previous.as_array_mut().context("expected resource list")?;
            for v in value.as_array().context("expected resource list")? {
                ensure!(
                    !into.iter().any(|old| old["id"] == v["id"]),
                    "duplicate resource list ID"
                );
                into.push(v.clone());
            }
        } else {
            ensure!(previous == value, "conflicting resource setting {key}");
        }
    }
    Ok(Value::Object(result))
}
