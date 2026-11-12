//! Turns a content package into a portable game folder (docs/plan-packaging.zh-TW.md).
//!
//! The output ships the pack *directory*, not an authoring database: package import rewrites
//! asset paths to absolute ones, so the player re-imports `content/` on every launch and the
//! folder can move. Output is staged in a sibling `*.partial` directory and renamed only after
//! the copied pack loads cleanly with no path outside the output, so failure or cancellation
//! never leaves a half-written game.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
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
        Ok(report) => report,
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
