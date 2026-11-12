//! Model (GLB / glTF / PMX) hot reload by content hash, off the main thread.
//!
//! A reload is explicit (protocol `reload_assets`, editor `asset.reload`) or follows file
//! changes (`watch`: model file size / modification time polled once per second, a few
//! `stat` calls). The job hashes every model file of the running world (cancellable between
//! files, countable progress) and, when a hash differs from the last accepted one, builds a
//! replacement runtime on the worker (the importer cache misses only for changed files).
//! The main thread then compares each animated entity's asset by its content key: unchanged
//! -> nothing happens; changed with the same skeleton (bone IDs and parents in order) -> the
//! replacement is adopted like any revision (clip time, controller state, physics carry
//! over); a changed skeleton -> refused, the running runtime is untouched.
use crate::jobs::{Job, Outcome};
use crate::world::WorldRuntime;
use anyhow::Result;
use axiom_animation::authoring::Source;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};

/// Watchdog of one reload (hash + runtime build).
const RELOAD_TIMEOUT: Duration = Duration::from_secs(60);
/// How often `watch` stats the model files.
const WATCH_INTERVAL: Duration = Duration::from_secs(1);
/// Model files watched / hashed per reload.
const MAX_FILES: usize = 64;

/// What a finished job hands to the main thread.
pub(crate) struct Built {
    rev: i64,
    session: String,
    /// Content hash of each model (asset ID -> `content_hash`).
    hashes: BTreeMap<String, u64>,
    /// Files whose hash differs from the last accepted reload (all on the first one).
    files_changed: Vec<String>,
    /// No accepted reload before this one (every file counted as changed).
    first: bool,
    runtime: Option<WorldRuntime>,
    hash_ms: f64,
    build_ms: f64,
}

/// The main thread's decision about a finished job.
pub(crate) enum Decision {
    /// Adopt this runtime (the caller prepares GPU state and swaps it in), then call
    /// [`Reload::adopted`].
    Adopt(Box<WorldRuntime>),
    /// Nothing to adopt; the built runtime (if any) is to be freed off the main thread.
    Discard(Option<Box<WorldRuntime>>),
}

#[derive(Default)]
pub(crate) struct Reload {
    job: Option<(&'static str, Job<Result<Built>>)>,
    pub watch: bool,
    /// Per model file: its own and its folder's files (size, modification time) at the last
    /// poll. Kept across revisions so a change made while a revision activates still counts.
    stamps: BTreeMap<PathBuf, Vec<(PathBuf, u64, Option<SystemTime>)>>,
    last_poll: Option<Instant>,
    /// Slowest `watch` poll on the main thread (milliseconds).
    watch_poll_ms: f64,
    /// Content hashes of the last accepted reload.
    hashes: BTreeMap<String, u64>,
    /// The pending adoption's report (filled by `decide`, completed by `adopted`).
    pending: Option<Value>,
    last: Option<Value>,
    pub reloads: u64,
}

/// Model sources of the running world (asset ID -> source), at most [`MAX_FILES`].
fn sources(runtime: &WorldRuntime) -> Result<BTreeMap<String, Source>> {
    let Some(assets) = runtime
        .state
        .compiled
        .source
        .resources
        .get("animation.library")
        .and_then(|l| l.get("assets"))
    else {
        return Ok(BTreeMap::new());
    };
    let sources: BTreeMap<String, Source> = serde_json::from_value(assets.clone())?;
    anyhow::ensure!(
        sources.len() <= MAX_FILES,
        "reload watches at most {MAX_FILES} model files"
    );
    Ok(sources)
}

/// Files under a model's folder an importer may read besides the model file itself (glTF
/// `.bin`, textures): path, size and modification time, sorted; the same inputs as the
/// importer cache key (`animation::asset_signature`). Bounded like it.
const FOLDER_FILES: usize = 512;
fn companions(source: &Source) -> Result<Vec<(PathBuf, u64, Option<SystemTime>)>> {
    let root = source
        .path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    let mut pending = vec![root];
    let mut files = vec![];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let meta = entry.metadata()?;
            if meta.is_dir() {
                pending.push(entry.path());
            } else if entry.path() != source.path {
                files.push((entry.path(), meta.len(), meta.modified().ok()));
            }
            anyhow::ensure!(
                files.len() + pending.len() <= FOLDER_FILES,
                "model folder has more than {FOLDER_FILES} files"
            );
        }
    }
    files.sort();
    Ok(files)
}

/// FNV-1a over the model file bytes (streamed), the import options and the size and
/// modification time of every other file in the model's folder.
fn content_hash(source: &Source) -> Result<u64> {
    use std::io::Read;
    let meta = std::fs::metadata(&source.path)?;
    anyhow::ensure!(meta.len() <= 134_217_728, "asset file exceeds 128 MiB");
    let mut hash = 0xcbf29ce484222325u64;
    let mut mix = |bytes: &[u8]| {
        for b in bytes {
            hash = (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3);
        }
    };
    let mut file = std::fs::File::open(&source.path)?;
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        mix(&buffer[..n]);
    }
    mix(&serde_json::to_vec(&source.options)?);
    for (path, len, modified) in companions(source)? {
        mix(path.to_string_lossy().as_bytes());
        mix(&len.to_le_bytes());
        let nanos = modified
            .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        mix(&nanos.to_le_bytes());
    }
    Ok(hash)
}

/// What `watch` compares between polls: the model file's and its folder's sizes and times.
fn stamp(source: &Source) -> Option<Vec<(PathBuf, u64, Option<SystemTime>)>> {
    let meta = std::fs::metadata(&source.path).ok()?;
    let mut files = companions(source).ok()?;
    files.push((source.path.clone(), meta.len(), meta.modified().ok()));
    Some(files)
}

/// Why `next` may not replace `current`: an animated entity's skeleton changed. `Ok(changed)`
/// lists `entity: asset` whose model content key changed.
pub(crate) fn compare(current: &WorldRuntime, next: &WorldRuntime) -> Result<Vec<String>, String> {
    let (Some(old), Some(new)) = (
        current.system::<crate::animation::AnimationSystem>(),
        next.system::<crate::animation::AnimationSystem>(),
    ) else {
        return Ok(vec![]);
    };
    let mut changed = vec![];
    for (entity, asset_id, asset, source) in new.assets() {
        let Some((_, previous_id, previous, previous_source)) =
            old.assets().find(|(e, ..)| *e == entity)
        else {
            continue;
        };
        if previous_id != asset_id {
            continue;
        }
        for (a, b) in [(previous, asset), (previous_source, source)] {
            if a.key == b.key {
                continue;
            }
            if let Some(reason) = skeleton_change(&a.skeleton, &b.skeleton) {
                return Err(format!("{entity} ({asset_id}): {reason}"));
            }
            let line = format!("{entity}: {asset_id}");
            if !changed.contains(&line) {
                changed.push(line);
            }
        }
    }
    Ok(changed)
}

/// `None` when both skeletons have the same bone IDs and parents in the same order (poses,
/// clips and bone overrides index bones by position).
fn skeleton_change(
    a: &axiom_animation::CompiledSkeleton,
    b: &axiom_animation::CompiledSkeleton,
) -> Option<String> {
    let (a, b) = (&a.source.bones, &b.source.bones);
    if a.len() != b.len() {
        return Some(format!("bone count {} -> {}", a.len(), b.len()));
    }
    a.iter().zip(b).enumerate().find_map(|(i, (x, y))| {
        (x.id != y.id || x.parent != y.parent).then(|| {
            format!(
                "bone {i} {} (parent {:?}) -> {} (parent {:?})",
                x.id, x.parent, y.id, y.parent
            )
        })
    })
}

impl Reload {
    /// Starts a reload of `runtime`'s model files. `trigger`: `command` or `watch`.
    pub(crate) fn start(&mut self, runtime: &WorldRuntime, trigger: &'static str) -> Result<Value> {
        anyhow::ensure!(self.job.is_none(), "an asset reload is already running");
        let sources = sources(runtime)?;
        anyhow::ensure!(!sources.is_empty(), "the world has no model assets");
        let started = Instant::now();
        let world = runtime.state.compiled.source.clone();
        let registry = runtime.registry_clone();
        let (rev, session) = (runtime.rev, runtime.runtime_session.clone());
        let baseline = self.hashes.clone();
        let first = baseline.is_empty();
        let clone_ms = started.elapsed().as_secs_f64() * 1000.0;
        let total = sources.len() as u64 + 1;
        let job = Job::spawn("asset_reload", trigger, RELOAD_TIMEOUT, move |p| {
            p.set_total(total);
            let phase = Instant::now();
            let mut hashes = BTreeMap::new();
            let mut files_changed = vec![];
            for (i, (id, source)) in sources.iter().enumerate() {
                anyhow::ensure!(!p.cancelled(), "asset reload cancelled");
                let hash = axiom_core::local::at(
                    &format!(
                        "/resources/animation.library/assets/{}/path",
                        axiom_core::local::escape(id)
                    ),
                    content_hash(source),
                )?;
                if baseline.get(id) != Some(&hash) {
                    files_changed.push(id.clone());
                }
                hashes.insert(id.clone(), hash);
                p.set_done(i as u64 + 1);
            }
            let hash_ms = phase.elapsed().as_secs_f64() * 1000.0;
            let phase = Instant::now();
            let runtime = if files_changed.is_empty() {
                None
            } else {
                anyhow::ensure!(!p.cancelled(), "asset reload cancelled");
                Some(WorldRuntime::new(&world, rev, registry)?)
            };
            p.set_done(total);
            Ok(Built {
                rev,
                session,
                hashes,
                files_changed,
                first,
                runtime,
                hash_ms,
                build_ms: phase.elapsed().as_secs_f64() * 1000.0,
            })
        })?;
        let observation = job.observation();
        self.job = Some((trigger, job));
        Ok(json!({"ok":true,"started":true,"job":observation,"main_thread_ms":clone_ms}))
    }

    pub(crate) fn cancel(&mut self) -> bool {
        match &mut self.job {
            Some((_, job)) => {
                job.cancel();
                true
            }
            None => false,
        }
    }

    pub(crate) fn running(&self) -> bool {
        self.job.is_some()
    }

    /// `watch`: whether a model file (or a file in its folder) changed since the last poll, at
    /// most once per second. A file seen for the first time does not count as a change.
    pub(crate) fn watch_due(&mut self, runtime: &WorldRuntime) -> bool {
        if !self.watch || self.job.is_some() {
            return false;
        }
        if self.last_poll.is_some_and(|t| t.elapsed() < WATCH_INTERVAL) {
            return false;
        }
        let started = Instant::now();
        self.last_poll = Some(started);
        let Ok(sources) = sources(runtime) else {
            return false;
        };
        let mut changed = false;
        for source in sources.values() {
            let Some(stamp) = stamp(source) else {
                continue;
            };
            match self.stamps.insert(source.path.clone(), stamp) {
                Some(old) if Some(&old) != self.stamps.get(&source.path) => changed = true,
                _ => {}
            }
        }
        self.watch_poll_ms = self
            .watch_poll_ms
            .max(started.elapsed().as_secs_f64() * 1000.0);
        changed
    }

    /// Turns watching on (checking that the world's models can be watched) or off.
    pub(crate) fn set_watch(&mut self, runtime: &WorldRuntime, on: bool) -> Result<()> {
        if on {
            let sources = sources(runtime)?;
            anyhow::ensure!(!sources.is_empty(), "the world has no model assets");
        }
        self.watch = on;
        Ok(())
    }
    /// Non-blocking; once per frame. A finished job's result, if any.
    pub(crate) fn poll(&mut self) -> Option<(Result<Built>, f64)> {
        let (_, job) = self.job.as_mut()?;
        let outcome = job.poll()?;
        let elapsed = job.elapsed().as_secs_f64() * 1000.0;
        self.job = None;
        Some((
            match outcome {
                Outcome::Done { value, .. } => value,
                Outcome::Abandoned { stop } => Err(anyhow::anyhow!(
                    "asset reload worker did not stop ({}); detached",
                    stop.name()
                )),
            },
            elapsed,
        ))
    }

    /// Decides what to do with a finished job given the running runtime. `busy`: a revision
    /// activation is pending (its build loads the new files anyway).
    pub(crate) fn decide(
        &mut self,
        result: Result<Built>,
        elapsed_ms: f64,
        current: &WorldRuntime,
        busy: bool,
    ) -> Decision {
        let mut report = json!({"elapsed_ms":elapsed_ms});
        let built = match result {
            Ok(built) => built,
            Err(error) => {
                report["state"] = json!("failed");
                report["error"] = json!(format!("{error:#}"));
                self.finish(report);
                return Decision::Discard(None);
            }
        };
        report["hash_ms"] = json!(built.hash_ms);
        report["build_ms"] = json!(built.build_ms);
        report["files_changed"] = json!(built.files_changed);
        let Some(next) = built.runtime else {
            report["state"] = json!("unchanged");
            self.hashes = built.hashes;
            self.finish(report);
            return Decision::Discard(None);
        };
        if busy || built.rev != current.rev || built.session != current.runtime_session {
            report["state"] = json!("superseded");
            self.finish(report);
            return Decision::Discard(Some(Box::new(next)));
        }
        match compare(current, &next) {
            Err(reason) => {
                report["state"] = json!("refused");
                report["error"] = json!(format!(
                    "skeleton changed, reload refused (re-open the pack or keep bone names and order): {reason}"
                ));
                self.finish(report);
                Decision::Discard(Some(Box::new(next)))
            }
            Ok(changed) if changed.is_empty() && built.first => {
                // No baseline yet and the same content keys: nothing to swap.
                report["state"] = json!("unchanged");
                self.hashes = built.hashes;
                self.finish(report);
                Decision::Discard(Some(Box::new(next)))
            }
            Ok(changed) => {
                // Files changed since the last accepted reload. With no live player using them
                // (models only spawned later, a texture the keys did not change) the rebuilt
                // runtime is still adopted, so later spawns and rebuilds see the new files.
                report["state"] = json!("reloaded");
                report["changed"] = json!(changed);
                self.hashes = built.hashes;
                self.pending = Some(report);
                Decision::Adopt(Box::new(next))
            }
        }
    }

    /// The adoption of a `Decision::Adopt` finished (`adopt_ms` on the main thread).
    pub(crate) fn adopted(&mut self, result: Result<()>, adopt_ms: f64) {
        let mut report = self.pending.take().unwrap_or_else(|| json!({}));
        report["adopt_ms"] = json!(adopt_ms);
        if let Err(error) = result {
            report["state"] = json!("failed");
            report["error"] = json!(format!("{error:#}"));
            // The new files were not accepted; the next reload compares against the old ones.
            self.hashes.clear();
        }
        self.finish(report);
    }

    fn finish(&mut self, report: Value) {
        self.reloads += 1;
        self.last = Some(report);
    }

    /// One line for the console / editor status.
    pub(crate) fn summary(&self) -> Option<String> {
        let last = self.last.as_ref()?;
        let state = last["state"].as_str().unwrap_or("?");
        let ms = last["elapsed_ms"].as_f64().unwrap_or(0.0);
        Some(match state {
            "reloaded" => format!(
                "Assets reloaded ({}) in {ms:.0} ms",
                last["changed"]
                    .as_array()
                    .map(|c| c
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", "))
                    .unwrap_or_default()
            ),
            "unchanged" => format!("Assets unchanged (content hash) · {ms:.0} ms"),
            _ => format!(
                "Asset reload {state}: {}",
                last["error"].as_str().unwrap_or("")
            ),
        })
    }

    /// `observe` metrics (`assets_reload`).
    pub(crate) fn observation(&self) -> Value {
        json!({
            "watch":self.watch,
            "watch_poll_ms_max":self.watch_poll_ms,
            "running":self.job.as_ref().map(|(_, j)| j.observation()),
            "last":self.last,
            "reloads":self.reloads,
            "timeout_ms":RELOAD_TIMEOUT.as_millis() as u64,
        })
    }

    /// The running job for the editor's job list.
    pub(crate) fn job_observation(&self) -> Option<Value> {
        self.job.as_ref().map(|(_, j)| j.observation())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_animation::{Bone, Skeleton, Trs};

    fn skeleton(bones: &[(&str, Option<usize>)]) -> axiom_animation::CompiledSkeleton {
        Skeleton {
            bones: bones
                .iter()
                .map(|(id, parent)| Bone {
                    id: (*id).into(),
                    aliases: vec![],
                    parent: *parent,
                    rest: Trs::default(),
                    inverse_bind: glam::Mat4::IDENTITY,
                })
                .collect(),
        }
        .compile()
        .unwrap()
    }

    #[test]
    fn skeletons_match_by_bone_ids_and_parents_in_order() {
        let a = skeleton(&[("root", None), ("head", Some(0))]);
        assert_eq!(skeleton_change(&a, &a.clone()), None);
        let renamed = skeleton(&[("root", None), ("face", Some(0))]);
        assert!(skeleton_change(&a, &renamed).unwrap().contains("head"));
        let longer = skeleton(&[("root", None), ("head", Some(0)), ("hat", Some(1))]);
        assert_eq!(
            skeleton_change(&a, &longer).as_deref(),
            Some("bone count 2 -> 3")
        );
        let reparented = skeleton(&[("root", None), ("head", None)]);
        assert!(skeleton_change(&a, &reparented).is_some());
    }

    /// Waits for the job (test only; the app polls once per frame).
    fn finish(reload: &mut Reload, runtime: &WorldRuntime) -> Decision {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some((result, ms)) = reload.poll() {
                return reload.decide(result, ms, runtime, false);
            }
            assert!(Instant::now() < deadline, "reload never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn reload_keeps_state_for_the_same_skeleton_and_refuses_another() {
        let content = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content");
        let dir = std::env::temp_dir().join(format!("axiom-reload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let model = dir.join("model.glb");
        std::fs::copy(format!("{content}/mascot/axiom-short.glb"), &model).unwrap();
        std::fs::write(dir.join("manifest.json"), r#"{"id":"reload.test","version":1,"world":"world.json","capabilities":{"animation":1,"core":1,"render":1}}"#).unwrap();
        std::fs::write(
            dir.join("world.json"),
            json!({"schema_version":2,"id":"reload-test","title":"Reload test","capabilities":{"animation":1,"core":1,"render":1},
                "systems":["animation.evaluate"],
                "entities":[{"id":"hero","components":{"core.transform":{},"animation.player":{"asset":"m","clip":"idle"}}}],
                "scenes":[{"id":"s","active":true,"entities":["hero"]}],
                "resources":{"animation.library":{"assets":{"m":{"path":"model.glb"}}}}})
            .to_string(),
        )
        .unwrap();
        let registry = crate::installed_registry();
        let world = axiom_core::ContentPackage::load(&dir, &registry.authoring)
            .unwrap()
            .world;
        let mut runtime = WorldRuntime::new(world, 1, registry).unwrap();
        let input = crate::ActionMap::default();
        for _ in 0..90 {
            runtime.step(&input);
        }
        let player =
            |r: &WorldRuntime| r.observation().metrics["animation"]["players"]["hero"].clone();
        let before = player(&runtime);
        let mut reload = Reload::default();
        // Same bytes: the first reload builds (no baseline yet) and finds the same content key.
        reload.start(&runtime, "command").unwrap();
        assert!(reload.start(&runtime, "command").is_err(), "one at a time");
        assert!(matches!(
            finish(&mut reload, &runtime),
            Decision::Discard(Some(_))
        ));
        assert_eq!(reload.last.as_ref().unwrap()["state"], "unchanged");
        // Same bytes again: the hash baseline skips the build entirely.
        reload.start(&runtime, "command").unwrap();
        assert!(matches!(
            finish(&mut reload, &runtime),
            Decision::Discard(None)
        ));
        assert_eq!(reload.last.as_ref().unwrap()["files_changed"], json!([]));
        // The long-hair mascot: same rig, different mesh -> adopted, clip time kept.
        std::fs::copy(format!("{content}/mascot/axiom-long.glb"), &model).unwrap();
        reload.start(&runtime, "command").unwrap();
        let Decision::Adopt(next) = finish(&mut reload, &runtime) else {
            panic!("expected an adoption: {:?}", reload.last);
        };
        let old = runtime
            .adopt(Ok(*next), |_| Ok(()), Instant::now())
            .unwrap();
        drop(old);
        reload.adopted(Ok(()), 0.0);
        let after = player(&runtime);
        assert_eq!(reload.last.as_ref().unwrap()["state"], "reloaded");
        assert_eq!(reload.last.as_ref().unwrap()["changed"], json!(["hero: m"]));
        assert_ne!(after["vertices"], before["vertices"]);
        assert_eq!(after["time"], before["time"]);
        assert_eq!(after["clip"], "idle");
        // Watch: the first poll only records; a file added next to the model is a change.
        reload.set_watch(&runtime, true).unwrap();
        assert!(!reload.watch_due(&runtime), "first sight is not a change");
        reload.last_poll = None;
        assert!(!reload.watch_due(&runtime));
        std::fs::write(dir.join("model.bin"), b"buffer").unwrap();
        assert!(!reload.watch_due(&runtime), "at most once per second");
        reload.last_poll = None;
        assert!(reload.watch_due(&runtime));
        reload.set_watch(&runtime, false).unwrap();
        assert!(!reload.watch_due(&runtime));
        // A different skeleton (the render-lab fighter): refused, the runtime keeps running.
        std::fs::copy(format!("{content}/render-lab/fighter.glb"), &model).unwrap();
        reload.start(&runtime, "command").unwrap();
        assert!(matches!(
            finish(&mut reload, &runtime),
            Decision::Discard(Some(_))
        ));
        let last = reload.last.clone().unwrap();
        assert_eq!(last["state"], "refused", "{last}");
        assert!(last["error"].as_str().unwrap().contains("skeleton changed"));
        assert_eq!(player(&runtime)["vertices"], after["vertices"]);
        // A missing file fails the job with the asset's path; nothing changes.
        std::fs::remove_file(&model).unwrap();
        reload.start(&runtime, "command").unwrap();
        assert!(matches!(
            finish(&mut reload, &runtime),
            Decision::Discard(None)
        ));
        let last = reload.last.clone().unwrap();
        assert_eq!(last["state"], "failed");
        assert!(
            last["error"].as_str().unwrap().contains("assets/m/path"),
            "{last}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_running_reload_is_cancelled_between_files() {
        let mut reload = Reload::default();
        assert!(!reload.cancel());
        // A job that only waits for cancellation stands in for a slow hash.
        reload.job = Some((
            "command",
            Job::spawn("asset_reload", "test", RELOAD_TIMEOUT, |p| {
                while !p.cancelled() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(anyhow::anyhow!("asset reload cancelled"))
            })
            .unwrap(),
        ));
        assert!(reload.running());
        assert!(reload.observation()["running"]["kind"] == "asset_reload");
        assert!(reload.cancel());
        let deadline = Instant::now() + Duration::from_secs(20);
        let (result, _) = loop {
            if let Some(done) = reload.poll() {
                break done;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        };
        assert!(result.is_err());
        assert!(!reload.running());
    }

    #[test]
    fn content_hash_covers_bytes_and_options() {
        let dir = std::env::temp_dir().join(format!("axiom-reload-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.glb");
        std::fs::write(&path, b"one").unwrap();
        let mut source: Source = serde_json::from_value(json!({"path":path})).expect("source");
        let first = content_hash(&source).unwrap();
        assert_eq!(first, content_hash(&source).unwrap());
        std::fs::write(&path, b"two").unwrap();
        let second = content_hash(&source).unwrap();
        assert_ne!(first, second);
        source.options.strict = true;
        assert_ne!(second, content_hash(&source).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
