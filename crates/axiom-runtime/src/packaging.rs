//! Turns a content package into a portable game folder (docs/plan-packaging.zh-TW.md).
//!
//! The output ships the pack *directory*, not an authoring database: package import rewrites
//! asset paths to absolute ones, so the player re-imports `content/` on every launch and the
//! folder can move. Output is staged in a sibling `*.partial` directory and renamed only after
//! the copied pack loads cleanly with no path outside the output, so failure or cancellation
//! never leaves a half-written game.
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Written into every output; `--force` only replaces directories that carry it.
pub const GAME_FILE: &str = "game.json";
pub const CONTENT_DIR: &str = "content";
pub const FORMAT: u32 = 1;
/// Build sources that a pack keeps beside its world but never reads at run time.
pub const DEFAULT_EXCLUDES: &[&str] = &[
    "build/", ".git/", "*.acl", "*.blend", "*.blend1", "*.py", "*.ps1",
];
const MAX_FILES: usize = 4096;
const MAX_BYTES: u64 = 4 << 30;

pub struct PackageOptions {
    pub pack: PathBuf,
    pub out: PathBuf,
    pub title: Option<String>,
    pub exe_name: Option<String>,
    /// Added to [`DEFAULT_EXCLUDES`]: `dir/` (a path component), `*.ext`, or a relative path.
    pub excludes: Vec<String>,
    pub force: bool,
    /// Executable copied into the output as `<exe_name>.exe`.
    pub player_exe: PathBuf,
}

/// Shared with a background job: countable progress and cooperative cancellation.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub cancel: AtomicBool,
}

/// The windowed player (`crates/axiom-player`) built beside the running executable; it reads
/// `game.json` next to itself and keeps its database and saves in `%APPDATA%\Axiom\<id>`.
pub fn default_player_exe() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let player = exe
        .with_file_name("axiom-player")
        .with_extension(std::env::consts::EXE_EXTENSION);
    ensure!(
        player.is_file(),
        "player executable {} not found; build it first with `cargo build -p axiom-player` \
         (same profile as {})",
        player.display(),
        exe.display()
    );
    Ok(player)
}

/// Builds the game folder and returns the compact report (file list with SHA-256).
pub fn package(
    registry: &axiom_core::Registry,
    options: &PackageOptions,
    progress: &Progress,
) -> Result<Value> {
    let started = std::time::Instant::now();
    let pack = options
        .pack
        .canonicalize()
        .with_context(|| format!("pack {} not found", options.pack.display()))?;
    let loaded = axiom_core::ContentPackage::load(&pack, registry).context("pack validation")?;
    // Refuse files outside the pack before anything is written, then compute the closure the
    // runtime reads so the report can flag copied-but-unreferenced files (report only).
    let source_world = serde_json::to_value(&loaded.world)?;
    reject_outside(&source_world, &pack)?;
    let referenced = referenced_files(&pack, &source_world)?;
    let id = loaded.manifest.id.clone();
    let title = options
        .title
        .clone()
        .unwrap_or_else(|| loaded.world.title.clone());
    ensure!(!title.trim().is_empty(), "title must not be empty");
    let exe_name = sanitize(options.exe_name.as_deref().unwrap_or(&title));
    ensure!(
        !exe_name.is_empty(),
        "executable name has no usable characters"
    );

    let out = normalized(&options.out)?;
    ensure!(
        !out.starts_with(&pack) && !pack.starts_with(&out),
        "output must not overlap the pack directory"
    );
    if out.exists() {
        ensure!(
            options.force,
            "output {} exists; pass --force to replace it",
            out.display()
        );
        ensure!(
            out.join(GAME_FILE).is_file(),
            "refusing to replace {}: it is not a packaged game folder",
            out.display()
        );
    }
    let staging = sibling(&out, "partial")?;
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .with_context(|| format!("remove stale {}", staging.display()))?;
    }
    let result = build(
        registry, options, progress, &pack, &staging, &id, &title, &exe_name,
    );
    let mut report = match result {
        Ok(mut report) => {
            annotate(&mut report, &referenced);
            report
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    if out.exists() {
        std::fs::remove_dir_all(&out).with_context(|| format!("replace {}", out.display()))?;
    }
    std::fs::rename(&staging, &out).with_context(|| format!("finalize {}", out.display()))?;
    report["out"] = json!(out.to_string_lossy().trim_start_matches(r"\\?\"));
    report["ms"] = json!(started.elapsed().as_millis() as u64);
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn build(
    registry: &axiom_core::Registry,
    options: &PackageOptions,
    progress: &Progress,
    pack: &Path,
    staging: &Path,
    id: &str,
    title: &str,
    exe_name: &str,
) -> Result<Value> {
    let excludes: Vec<&str> = DEFAULT_EXCLUDES
        .iter()
        .copied()
        .chain(options.excludes.iter().map(String::as_str))
        .collect();
    let mut sources = vec![];
    let mut skipped = vec![];
    collect(pack, pack, &excludes, &mut sources, &mut skipped)?;
    sources.sort();
    let exe = format!("{exe_name}.exe");
    progress.total.store(sources.len() + 1, Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);

    let content = staging.join(CONTENT_DIR);
    std::fs::create_dir_all(&content)?;
    let mut files = vec![];
    let mut bytes = 0u64;
    for relative in &sources {
        ensure!(
            !progress.cancel.load(Ordering::Relaxed),
            "packaging cancelled"
        );
        let target = content.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let (size, hash) = copy_hashed(&pack.join(relative), &target)?;
        bytes += size;
        ensure!(bytes <= MAX_BYTES, "output exceeds the 4 GiB budget");
        files.push(
            json!({"path":format!("{CONTENT_DIR}/{}",slash(relative)),"bytes":size,"sha256":hash}),
        );
        progress.done.fetch_add(1, Ordering::Relaxed);
    }
    ensure!(
        !progress.cancel.load(Ordering::Relaxed),
        "packaging cancelled"
    );
    let (size, hash) = copy_hashed(&options.player_exe, &staging.join(&exe))
        .with_context(|| format!("copy player {}", options.player_exe.display()))?;
    bytes += size;
    files.push(json!({"path":exe,"bytes":size,"sha256":hash}));
    progress.done.fetch_add(1, Ordering::Relaxed);

    let game = json!({"format":FORMAT,"id":id,"title":title,"pack":CONTENT_DIR,"exe":exe,"engine":env!("CARGO_PKG_VERSION")});
    std::fs::write(staging.join(GAME_FILE), serde_json::to_vec_pretty(&game)?)?;

    // The copy must load on its own: every resolved path stays inside the output folder.
    let world = axiom_core::ContentPackage::load(&content, registry)
        .context("packaged content does not load (an excluded file is still referenced?)")?
        .world;
    let root = staging.canonicalize()?;
    let mut escaped = vec![];
    escapes(&serde_json::to_value(&world)?, &root, &mut escaped);
    ensure!(
        escaped.is_empty(),
        "packaged world references files outside the output: {}",
        escaped.join(", ")
    );
    Ok(
        json!({"ok":true,"id":id,"title":title,"exe":exe,"files":files,"bytes":bytes,"skipped":skipped}),
    )
}

fn collect(
    root: &Path,
    dir: &Path,
    excludes: &[&str],
    out: &mut Vec<PathBuf>,
    skipped: &mut Vec<String>,
) -> Result<()> {
    let mut entries = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let kind = entry.file_type()?;
        let relative = entry.path().strip_prefix(root)?.to_path_buf();
        ensure!(
            !kind.is_symlink(),
            "symbolic link {} is not supported",
            slash(&relative)
        );
        if excluded(&relative, kind.is_dir(), excludes) {
            skipped.push(slash(&relative) + if kind.is_dir() { "/" } else { "" });
            continue;
        }
        if kind.is_dir() {
            collect(root, &entry.path(), excludes, out, skipped)?;
        } else {
            out.push(relative);
            ensure!(out.len() <= MAX_FILES, "pack exceeds {MAX_FILES} files");
        }
    }
    Ok(())
}

fn excluded(relative: &Path, dir: bool, patterns: &[&str]) -> bool {
    let text = slash(relative);
    let name = relative
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    patterns.iter().any(|p| {
        if let Some(d) = p.strip_suffix('/') {
            dir && name.eq_ignore_ascii_case(d)
        } else if let Some(ext) = p.strip_prefix("*.") {
            !dir && relative
                .extension()
                .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(ext))
        } else {
            text.eq_ignore_ascii_case(p.trim_start_matches("./"))
        }
    })
}

fn copy_hashed(from: &Path, to: &Path) -> Result<(u64, String)> {
    let bytes = std::fs::read(from).with_context(|| format!("read {}", from.display()))?;
    std::fs::write(to, &bytes).with_context(|| format!("write {}", to.display()))?;
    Ok((bytes.len() as u64, format!("{:x}", Sha256::digest(&bytes))))
}

/// Absolute filesystem paths in the resolved world that point outside `root`.
fn escapes(value: &Value, root: &Path, out: &mut Vec<String>) {
    match value {
        Value::String(s) => {
            let path = Path::new(s.trim_start_matches(r"\\?\"));
            if path.is_absolute() && path.components().count() > 1 {
                let inside = Path::new(s)
                    .canonicalize()
                    .map(|p| p.starts_with(root))
                    .unwrap_or(false);
                if !inside && out.len() < 8 && !out.contains(s) {
                    out.push(s.clone());
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| escapes(v, root, out)),
        Value::Object(map) => map.values().for_each(|v| escapes(v, root, out)),
        _ => {}
    }
}

const LISTED: usize = 8;

/// Fails, listing the first [`LISTED`] and the total, when the resolved source world reads a
/// file outside `pack`: every `external: true` animation asset (resolved against the working
/// directory, so never relocatable, even when it happens to sit inside the pack) and every
/// absolute path that does not resolve inside the pack (missing files count as outside).
fn reject_outside(world: &Value, pack: &Path) -> Result<()> {
    let mut found = BTreeMap::<String, String>::new();
    if let Some(assets) = world
        .pointer("/resources/animation.library/assets")
        .and_then(Value::as_object)
    {
        for (id, asset) in assets {
            if asset["external"].as_bool() == Some(true) {
                let path = asset["path"].as_str().unwrap_or_default();
                found.insert(plain(path), format!("external animation asset {id}"));
            }
        }
    }
    let mut paths = BTreeSet::new();
    absolute_strings(world, &mut paths);
    for path in paths {
        let inside = Path::new(&path)
            .canonicalize()
            .is_ok_and(|p| p.starts_with(pack));
        if !inside {
            found
                .entry(plain(&path))
                .or_insert_with(|| "absolute path outside the pack".into());
        }
    }
    if found.is_empty() {
        return Ok(());
    }
    let listed: Vec<String> = found
        .iter()
        .take(LISTED)
        .map(|(path, why)| format!("{path} ({why})"))
        .collect();
    let more = found.len().saturating_sub(LISTED);
    bail!(
        "pack references {} file(s) outside {}; move them into the pack with relative paths (external: false): {}{}",
        found.len(),
        plain(&pack.to_string_lossy()),
        listed.join(", "),
        if more > 0 { format!(", and {more} more") } else { String::new() }
    )
}

/// Pack-relative files the runtime reads: for the root package and every dependency
/// (recursively, through manifest `dependencies[*].path`) its manifest, world file, declared
/// `assets` and `render.shaders` `.wgsl` files (inlined at import, so only the raw world names
/// them); plus every absolute path in the resolved world that lands on a file inside the pack
/// (animation GLBs, UI images, ...). Anything read by other means is not covered.
fn referenced_files(pack: &Path, world: &Value) -> Result<BTreeSet<String>> {
    let mut referenced = BTreeSet::new();
    let mut visited = BTreeSet::new();
    manifest_closure(pack, pack, &mut visited, &mut referenced)?;
    let mut paths = BTreeSet::new();
    absolute_strings(world, &mut paths);
    for path in paths {
        add_file(pack, Path::new(&path), &mut referenced);
    }
    Ok(referenced)
}

fn manifest_closure(
    pack: &Path,
    dir: &Path,
    visited: &mut BTreeSet<PathBuf>,
    referenced: &mut BTreeSet<String>,
) -> Result<()> {
    let dir = dir
        .canonicalize()
        .with_context(|| format!("package {}", dir.display()))?;
    if !dir.starts_with(pack) || !visited.insert(dir.clone()) {
        return Ok(());
    }
    ensure!(visited.len() <= 32, "package count exceeds 32");
    let manifest_path = dir.join("manifest.json");
    let manifest: Value = serde_json::from_slice(&std::fs::read(&manifest_path)?)?;
    add_file(pack, &manifest_path, referenced);
    if let Some(world) = manifest["world"].as_str() {
        let world_path = dir.join(world);
        add_file(pack, &world_path, referenced);
        let raw: Value = serde_json::from_slice(&std::fs::read(&world_path)?)?;
        if let Some(shaders) = raw
            .pointer("/resources/render.shaders/shaders")
            .and_then(Value::as_object)
        {
            for file in shaders.values().filter_map(|s| s["file"].as_str()) {
                add_file(pack, &dir.join(file), referenced);
            }
        }
    }
    if let Some(assets) = manifest["assets"].as_object() {
        for relative in assets.keys() {
            add_file(pack, &dir.join(relative), referenced);
        }
    }
    if let Some(dependencies) = manifest["dependencies"].as_object() {
        for path in dependencies.values().filter_map(|d| d["path"].as_str()) {
            manifest_closure(pack, &dir.join(path), visited, referenced)?;
        }
    }
    Ok(())
}

fn add_file(pack: &Path, path: &Path, referenced: &mut BTreeSet<String>) {
    if let Ok(path) = path.canonicalize() {
        if path.is_file() {
            if let Ok(relative) = path.strip_prefix(pack) {
                referenced.insert(slash(relative));
            }
        }
    }
}

/// Every string in `value` that is an absolute filesystem path (`C:\...`, `\\?\C:\...`).
fn absolute_strings(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::String(s) => {
            let path = Path::new(s.trim_start_matches(r"\\?\"));
            if path.is_absolute() && path.components().count() > 1 {
                out.insert(s.clone());
            }
        }
        Value::Array(items) => items.iter().for_each(|v| absolute_strings(v, out)),
        Value::Object(map) => map.values().for_each(|v| absolute_strings(v, out)),
        _ => {}
    }
}

/// Adds `unreferenced` (copied pack files outside [`referenced_files`]; a warning, never a
/// deletion) and `warnings`, and sorts the report's name lists so equal inputs give equal
/// reports apart from `out` and `ms`.
fn annotate(report: &mut Value, referenced: &BTreeSet<String>) {
    let prefix = format!("{CONTENT_DIR}/");
    let unreferenced: Vec<String> = report["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| f["path"].as_str()?.strip_prefix(&prefix))
        .filter(|path| !referenced.contains(*path))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut warnings: Vec<String> = report["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| w.as_str().map(str::to_string))
        .collect();
    if !unreferenced.is_empty() {
        let listed: Vec<&str> = unreferenced
            .iter()
            .take(LISTED)
            .map(String::as_str)
            .collect();
        warnings.push(format!(
            "{} copied file(s) look unreferenced (kept; add --exclude to drop them): {}{}",
            unreferenced.len(),
            listed.join(", "),
            if unreferenced.len() > LISTED {
                ", ..."
            } else {
                ""
            }
        ));
    }
    warnings.sort();
    if let Some(skipped) = report["skipped"].as_array_mut() {
        skipped.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    }
    report["referenced"] = json!(referenced.len());
    report["unreferenced"] = json!(unreferenced);
    report["warnings"] = json!(warnings);
}

/// Display form without the Windows verbatim `\\?\` prefix.
fn plain(path: &str) -> String {
    path.trim_start_matches(r"\\?\").to_string()
}

fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    })
}

/// Absolute, in the same canonical (verbatim) form as `canonicalize`, even when the path
/// does not exist yet: the deepest existing ancestor is canonicalized and the rest appended.
fn normalized(path: &Path) -> Result<PathBuf> {
    let path = absolute(path)?;
    let mut rest = vec![];
    let mut base = path.as_path();
    loop {
        if let Ok(found) = base.canonicalize() {
            return Ok(rest.iter().rev().fold(found, |p: PathBuf, c| p.join(c)));
        }
        rest.push(
            base.file_name()
                .context("output path has no existing ancestor")?,
        );
        base = base
            .parent()
            .context("output path has no existing ancestor")?;
    }
}

fn sibling(out: &Path, suffix: &str) -> Result<PathBuf> {
    let name = out
        .file_name()
        .context("output path needs a final directory name")?;
    let mut name = name.to_os_string();
    name.push(format!(".{suffix}"));
    Ok(out.with_file_name(name))
}

/// Keeps letters, digits, space, `-`, `_` and `.`; Windows-reserved characters are dropped.
pub fn sanitize(name: &str) -> String {
    let kept: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
        .collect();
    let words = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    let short: String = words.chars().take(64).collect();
    short.trim().trim_end_matches('.').trim().to_string()
}

fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Refuses an output inside a git work tree's tracked area only by warning; callers print it.
pub fn inside_repo(out: &Path) -> bool {
    let Ok(out) = absolute(out) else { return false };
    out.ancestors().any(|a| a.join(".git").exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclude_patterns_match_directories_extensions_and_paths() {
        let p = ["build/", "*.acl", "notes/todo.txt"];
        assert!(excluded(Path::new("build"), true, &p));
        assert!(!excluded(Path::new("build"), false, &p));
        assert!(excluded(Path::new("a/10-env.ACL"), false, &p));
        assert!(excluded(Path::new("notes/todo.txt"), false, &p));
        assert!(!excluded(Path::new("world.json"), false, &p));
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                copy_dir(&e.path(), &to.join(e.file_name()));
            } else {
                std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
            }
        }
    }

    fn hashes(report: &Value) -> Vec<(String, String)> {
        report["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["path"].as_str().unwrap().to_string(),
                    f["sha256"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn packages_relocatable_folder_deterministically_and_safely() {
        let registry = crate::installed_registry().authoring;
        let base = std::env::temp_dir().join(format!("axiom-pkg-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pack = base.join("pack");
        copy_dir(
            Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../content/two-cameras"
            )),
            &pack,
        );
        std::fs::create_dir_all(pack.join("build")).unwrap();
        std::fs::write(pack.join("build/00.acl"), "seed").unwrap();
        std::fs::write(pack.join("notes.txt"), "keep").unwrap();
        let player = base.join("player.exe");
        std::fs::write(&player, b"not really an exe").unwrap();
        let out = base.join("out/Two Cameras");
        let mut options = PackageOptions {
            pack: pack.clone(),
            out: out.clone(),
            title: Some("Two: Cameras".into()),
            exe_name: None,
            excludes: vec!["notes.txt".into()],
            force: false,
            player_exe: player,
        };
        let first = package(&registry, &options, &Progress::default()).unwrap();
        assert_eq!(first["exe"], "Two Cameras.exe");
        assert!(out.join("Two Cameras.exe").is_file() && out.join(GAME_FILE).is_file());
        // The windowed player reads game.json itself; no console launcher ships.
        assert!(!out.join("Play.cmd").exists());
        let game: Value =
            serde_json::from_slice(&std::fs::read(out.join(GAME_FILE)).unwrap()).unwrap();
        assert_eq!(game["format"], FORMAT);
        assert_eq!(game["pack"], CONTENT_DIR);
        assert_eq!(game["exe"], "Two Cameras.exe");
        assert_eq!(game["title"], "Two: Cameras");
        assert!(!out.join("content/build").exists() && !out.join("content/notes.txt").exists());
        let skipped = first["skipped"].to_string();
        assert!(
            skipped.contains("build/") && skipped.contains("notes.txt"),
            "{skipped}"
        );

        // Existing output: refused without --force, identical file list with it.
        let refused = package(&registry, &options, &Progress::default()).unwrap_err();
        assert!(refused.to_string().contains("--force"), "{refused}");
        options.force = true;
        let second = package(&registry, &options, &Progress::default()).unwrap();
        assert_eq!(hashes(&first), hashes(&second));

        // --force never deletes a directory that is not a packaged game.
        let foreign = base.join("out/foreign");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("keep.txt"), "x").unwrap();
        options.out = foreign.clone();
        let error = package(&registry, &options, &Progress::default()).unwrap_err();
        assert!(
            error.to_string().contains("not a packaged game folder"),
            "{error}"
        );
        assert!(foreign.join("keep.txt").is_file());

        // Cancellation leaves neither the output nor its staging directory.
        options.out = base.join("out/cancelled");
        let progress = Progress::default();
        progress.cancel.store(true, Ordering::Relaxed);
        let error = package(&registry, &options, &progress).unwrap_err();
        assert!(error.to_string().contains("cancelled"), "{error}");
        assert!(
            !base.join("out/cancelled").exists() && !base.join("out/cancelled.partial").exists()
        );

        // Output inside the pack is refused before anything is written.
        options.out = pack.join("dist");
        assert!(package(&registry, &options, &Progress::default()).is_err());
        assert!(!pack.join("dist").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    fn content(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content")
            .join(name)
    }

    fn options(pack: &Path, out: &Path, player: &Path) -> PackageOptions {
        PackageOptions {
            pack: pack.into(),
            out: out.into(),
            title: None,
            exe_name: Some("Game".into()),
            excludes: vec![],
            force: false,
            player_exe: player.into(),
        }
    }

    fn scratch(tag: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("axiom-pkg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let player = base.join("player.exe");
        std::fs::write(&player, b"stub").unwrap();
        (base, player)
    }

    fn strings(report: &Value, key: &str) -> Vec<String> {
        report[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn external_animation_asset_is_refused_before_anything_is_written() {
        let registry = crate::installed_registry().authoring;
        let (base, player) = scratch("external");
        let pack = base.join("pack");
        copy_dir(&content("animation"), &pack);
        let outside = base.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::rename(pack.join("rigify.glb"), outside.join("rigify.glb")).unwrap();
        let glb = outside.join("rigify.glb").canonicalize().unwrap();
        let world_path = pack.join("world.json");
        let mut world: Value =
            serde_json::from_slice(&std::fs::read(&world_path).unwrap()).unwrap();
        let asset = &mut world["resources"]["animation.library"]["assets"]["character"];
        asset["external"] = json!(true);
        asset["path"] = json!(glb.to_string_lossy());
        std::fs::write(&world_path, serde_json::to_vec(&world).unwrap()).unwrap();
        // The source pack itself is valid: the engine accepts external assets in authoring.
        axiom_core::ContentPackage::load(&pack, &registry).unwrap();

        let out = base.join("out/Game");
        let error = package(
            &registry,
            &options(&pack, &out, &player),
            &Progress::default(),
        )
        .unwrap_err()
        .to_string();
        let shown = glb
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_string();
        assert!(
            error.contains("outside")
                && error.contains(&shown)
                && error.contains("external animation asset character"),
            "{error}"
        );
        assert!(!out.exists() && !base.join("out/Game.partial").exists());
        assert!(!base.join("out").exists(), "nothing may be written");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn report_lists_unreferenced_files_and_follows_dependencies() {
        let registry = crate::installed_registry().authoring;
        let (base, player) = scratch("closure");
        let pack = base.join("pack");
        copy_dir(&content("m4"), &pack);
        std::fs::write(pack.join("extra.txt"), "not read by the runtime").unwrap();
        let out = base.join("out/Game");
        let report = package(
            &registry,
            &options(&pack, &out, &player),
            &Progress::default(),
        )
        .unwrap();
        let unreferenced = strings(&report, "unreferenced");
        assert!(unreferenced.contains(&"extra.txt".to_string()), "{report}");
        // Copied anyway (plan decision #3): unreferenced is a warning, not an exclusion.
        assert!(out.join("content/extra.txt").is_file());
        for needed in [
            "manifest.json",
            "world.json",
            "packs/quest/manifest.json",
            "packs/quest/world.json",
            // Second level: quest depends on ../hero and ../squad.
            "packs/hero/manifest.json",
            "packs/hero/world.json",
            "packs/squad/world.json",
            "packs/hero/fighter.glb",
        ] {
            assert!(out.join("content").join(needed).is_file(), "{needed}");
            assert!(
                !unreferenced.contains(&needed.to_string()),
                "{needed}: {report}"
            );
        }
        assert_eq!(unreferenced, ["extra.txt"], "{report}");
        assert!(
            strings(&report, "warnings")
                .iter()
                .any(|w| w.contains("extra.txt")),
            "{report}"
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn sanitize_drops_reserved_characters() {
        assert_eq!(sanitize("Sakura: Plaza?"), "Sakura Plaza");
        assert_eq!(
            sanitize("Last Signal / 最後的訊號"),
            "Last Signal 最後的訊號"
        );
        assert_eq!(sanitize("..."), "");
        assert_eq!(sanitize(&"x".repeat(80)).len(), 64);
    }
}
