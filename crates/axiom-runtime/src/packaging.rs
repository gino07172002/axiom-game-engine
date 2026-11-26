//! Turns a content package into a portable game folder (docs/plan-packaging.zh-TW.md).
//!
//! The output ships the pack *directory*, not an authoring database: package import rewrites
//! asset paths to absolute ones, so the player re-imports `content/` on every launch and the
//! folder can move. Output is staged in a sibling `<out>.partial-<unique>` directory that this
//! build creates atomically (and is the only directory a failed build removes), then published
//! only after the copied pack loads cleanly with no file outside the output. Publishing never
//! overwrites a destination that appeared during the build; `--force` moves the previous build
//! aside and restores it if the new one cannot be put in place. One build per output folder at
//! a time (an OS file lock that the process releases even when it dies).
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// Written into every output; the player reads it.
pub const GAME_FILE: &str = "game.json";
/// Written by the packager beside `game.json` (it records that file's SHA-256 and a digest of
/// every file the build wrote); `--force` replaces only folders whose files still match it.
pub const BUILD_MARKER: &str = ".axiom-build";
const MAX_MARKER: usize = 1024;
/// Format 2 adds `files_sha256`; a format-1 marker cannot prove the content is unchanged.
const MARKER_FORMAT: u32 = 2;
pub const CONTENT_DIR: &str = "content";
pub const FORMAT: u32 = 1;
/// Largest `game.json` the packager writes and the player reads.
pub const MAX_GAME_FILE: usize = 64 * 1024;
/// Build sources that a pack keeps beside its world but never reads at run time.
pub const DEFAULT_EXCLUDES: &[&str] = &[
    "build/", ".git/", "*.acl", "*.blend", "*.blend1", "*.py", "*.ps1",
];
const MAX_FILES: usize = 4096;
/// Every directory entry the walk reads, kept or excluded.
const MAX_ENTRIES: usize = 65_536;
const MAX_DEPTH: usize = 32;
/// Whole output: content, player executable and `game.json`.
const MAX_BYTES: u64 = 4 << 30;
const COPY_CHUNK: usize = 1 << 16;
/// Excluded paths named in the report (the total is always given).
const SKIPPED_LISTED: usize = 32;
/// Largest glTF JSON / `.axrig` bundle read to list its dependencies (the importer's limit).
const MAX_ASSET_FILE: usize = 134_217_728;

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
    /// Refuse to publish unless the shipped pack files hash to this `pack_digest`.
    pub expected_pack_digest: Option<String>,
}

/// Shared with a background job: countable progress and cooperative cancellation.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub cancel: AtomicBool,
}

impl Progress {
    fn check(&self) -> Result<()> {
        ensure!(!self.cancel.load(Ordering::Relaxed), "packaging cancelled");
        Ok(())
    }
}

/// `game.json`: the one contract between the packager (writer) and the player (reader).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameManifest {
    pub format: u32,
    /// Names the save directory: one plain ASCII path component.
    pub id: String,
    pub title: String,
    /// Relative path of the shipped pack inside the game folder.
    pub pack: String,
    #[serde(default)]
    pub exe: String,
    #[serde(default)]
    pub engine: String,
}

impl GameManifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.format == FORMAT,
            "{GAME_FILE}: unsupported format {} (this player reads format {FORMAT})",
            self.format
        );
        let id = &self.id;
        ensure!(
            !id.is_empty()
                && id.len() <= 128
                && id != "."
                && id != ".."
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
            "{GAME_FILE}: id {id:?} must be 1-128 ASCII letters, digits, '-', '_' or '.' (it names the save folder)"
        );
        ensure!(
            !self.title.trim().is_empty(),
            "{GAME_FILE}: \"title\" must be a non-empty string"
        );
        let pack = Path::new(&self.pack);
        ensure!(
            !self.pack.trim().is_empty()
                && pack
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_))),
            "{GAME_FILE}: pack must be a relative path inside the game folder"
        );
        ensure!(
            self.exe.is_empty()
                || (Path::new(&self.exe).components().count() == 1
                    && Path::new(&self.exe)
                        .components()
                        .all(|c| matches!(c, std::path::Component::Normal(_)))),
            "{GAME_FILE}: exe must be a file name"
        );
        Ok(())
    }

    /// Validated, pretty JSON within [`MAX_GAME_FILE`].
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self)?;
        ensure!(
            bytes.len() <= MAX_GAME_FILE,
            "{GAME_FILE} exceeds {MAX_GAME_FILE} bytes (title too long?)"
        );
        Ok(bytes)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_GAME_FILE,
            "{GAME_FILE} exceeds {MAX_GAME_FILE} bytes"
        );
        let game: Self =
            serde_json::from_slice(bytes).with_context(|| format!("parse {GAME_FILE}"))?;
        game.validate()?;
        Ok(game)
    }

    /// Reads `dir/game.json` (never more than [`MAX_GAME_FILE`] + 1 bytes).
    pub fn read(dir: &Path) -> Result<Self> {
        let path = dir.join(GAME_FILE);
        let bytes =
            axiom_core::read_bounded(&path, MAX_GAME_FILE, GAME_FILE).with_context(|| {
                format!(
                    "{} not found or unreadable beside the player",
                    path.display()
                )
            })?;
        Self::parse(&bytes).with_context(|| format!("{}", path.display()))
    }
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
    // The same contract the player enforces, checked before anything is written.
    let game = GameManifest {
        format: FORMAT,
        id: loaded.manifest.id.clone(),
        title,
        pack: CONTENT_DIR.into(),
        exe: format!("{exe_name}.exe"),
        engine: env!("CARGO_PKG_VERSION").into(),
    };
    let game_bytes = game.to_bytes()?;
    if let Some(expected) = &options.expected_pack_digest {
        ensure!(
            expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit()),
            "expected_pack_digest must be 64 hex digits (a report's pack_digest)"
        );
    }

    let out = output_path(&options.out)?;
    // `out` keeps its last component as written (so a link there is inspected as itself); the
    // overlap check also uses the folder it really opens, which on Windows may be spelled
    // differently (a case alias of an existing build) or be a link's target.
    let opened = normalized(&out)?;
    ensure!(
        [&out, &opened]
            .iter()
            .all(|o| !o.starts_with(&pack) && !pack.starts_with(o)),
        "output must not overlap the pack directory"
    );
    if let Some(meta) = existing(&out)? {
        ensure!(
            options.force,
            "output {} exists; pass --force to replace it",
            shown(&out)
        );
        game_folder(&out, &meta, progress)?;
    }
    progress.check()?;
    let parent = out
        .parent()
        .context("output path needs a parent directory")?;
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", shown(parent)))?;
    let _lock = OutputLock::acquire(&out)?;
    let staging = Staging::create(&out)?;
    let mut report = build(
        registry,
        options,
        progress,
        &pack,
        &staging.0,
        &game,
        &game_bytes,
    )?;
    annotate(&mut report, &referenced);
    if let Some(warning) = publish(&staging.0, &out, options.force, progress)? {
        if let Some(warnings) = report["warnings"].as_array_mut() {
            warnings.push(json!(warning));
        }
    }
    staging.published();
    report["out"] = json!(shown(&out));
    report["ms"] = json!(started.elapsed().as_millis() as u64);
    Ok(report)
}

/// The staging directory this build created; removed on drop unless published.
struct Staging(PathBuf, bool);

impl Staging {
    /// `create_dir` (never `create_dir_all`) on a fresh unique name: the directory exists only
    /// because this call made it, so removing it can never touch a pack or another build.
    fn create(out: &Path) -> Result<Self> {
        let path = unique_sibling(out, "partial", |p| std::fs::create_dir(p))?;
        Ok(Self(path, false))
    }

    fn published(mut self) {
        self.1 = true;
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.1 {
            let _ = remove_own_dir(&self.0);
        }
    }
}

static UNIQUE: AtomicU64 = AtomicU64::new(0);

/// Tries `<out>.<suffix>-<pid>-<n>` names until `make` succeeds on one that did not exist.
fn unique_sibling(
    out: &Path,
    suffix: &str,
    make: impl Fn(&Path) -> std::io::Result<()>,
) -> Result<PathBuf> {
    let name = out
        .file_name()
        .context("output path needs a final directory name")?;
    for _ in 0..64 {
        let n = UNIQUE.fetch_add(1, Ordering::Relaxed);
        let mut candidate = name.to_os_string();
        candidate.push(format!(".{suffix}-{}-{n}", std::process::id()));
        let path = out.with_file_name(candidate);
        match make(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).with_context(|| format!("create {}", shown(&path))),
        }
    }
    bail!("no free {suffix} name beside {}", shown(out))
}

/// Removes a directory this build created or moved aside; never a link or junction.
fn remove_own_dir(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() && !is_link(&meta),
        "refusing to remove {}: not a plain directory",
        shown(path)
    );
    std::fs::remove_dir_all(path).with_context(|| format!("remove {}", shown(path)))
}

fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.file_type().is_symlink() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

fn existing(path: &Path) -> Result<Option<std::fs::Metadata>> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("inspect {}", shown(path))),
    }
}

/// `--force` replaces only a folder proven to be this packager's output; anything else is
/// refused and left untouched.
fn game_folder(out: &Path, meta: &std::fs::Metadata, progress: &Progress) -> Result<()> {
    proven_build(out, meta, progress).with_context(|| {
        format!(
            "refusing to replace {}: it is not a packaged game folder written by `axiom package` \
             (remove it yourself or choose another output)",
            shown(out)
        )
    })
}

/// The bytes of [`BUILD_MARKER`] for a build whose `game.json` is `game_bytes` and whose
/// files (everything but the marker) hash to `files_sha256` (see [`files_digest`]).
fn marker_bytes(game_bytes: &[u8], files_sha256: &str) -> Vec<u8> {
    format!("{}{files_sha256}\"}}\n", marker_head(game_bytes)).into_bytes()
}

/// The part of the marker that depends only on `game.json` (checked before any hashing).
fn marker_head(game_bytes: &[u8]) -> String {
    format!(
        "{{\"format\":{MARKER_FORMAT},\"game_sha256\":\"{:x}\",\"files_sha256\":\"",
        Sha256::digest(game_bytes)
    )
}

/// SHA-256 over the sorted `path\tsha256\n` lines of a build's files (slash paths relative to
/// the output folder, marker excluded).
fn files_digest(files: &BTreeMap<String, String>) -> String {
    let mut digest = Sha256::new();
    for (path, hash) in files {
        digest.update(format!("{path}\t{hash}\n"));
    }
    format!("{:x}", digest.finalize())
}

/// SHA-256 of a plain file, streamed; cancellation is checked between chunks.
fn hash_file(path: &Path, buffer: &mut [u8], progress: &Progress) -> Result<String> {
    let mut input = std::fs::File::open(path).with_context(|| format!("read {}", shown(path)))?;
    let mut hasher = Sha256::new();
    loop {
        progress.check()?;
        let n = input
            .read(buffer)
            .with_context(|| format!("read {}", shown(path)))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// A plain directory (no link or junction) whose valid `game.json` matches the build marker,
/// whose executable and pack directory exist, which holds nothing else at its root, and whose
/// files (pack included, walked with the packager's bounds) are exactly the ones the build
/// wrote: a file added or changed anywhere (`content\my notes.txt`) refuses the replacement.
fn proven_build(out: &Path, meta: &std::fs::Metadata, progress: &Progress) -> Result<()> {
    ensure!(meta.is_dir() && !is_link(meta), "not a plain directory");
    let game_bytes = read_plain_file(&out.join(GAME_FILE), MAX_GAME_FILE)?;
    let game = GameManifest::parse(&game_bytes)?;
    let marker = read_plain_file(&out.join(BUILD_MARKER), MAX_MARKER)?;
    ensure!(
        marker.starts_with(marker_head(&game_bytes).as_bytes()),
        "{BUILD_MARKER} does not match {GAME_FILE}"
    );
    ensure!(!game.exe.is_empty(), "{GAME_FILE} names no executable");
    let exe = std::fs::symlink_metadata(out.join(&game.exe))
        .with_context(|| format!("executable {}", game.exe))?;
    ensure!(
        exe.is_file() && !is_link(&exe),
        "{} is not a plain file",
        game.exe
    );
    let pack_root = Path::new(&game.pack)
        .components()
        .next()
        .context("pack is empty")?
        .as_os_str()
        .to_os_string();
    let pack = std::fs::symlink_metadata(out.join(&pack_root))
        .with_context(|| format!("pack {}", game.pack))?;
    ensure!(
        pack.is_dir() && !is_link(&pack),
        "pack {} is not a plain directory",
        game.pack
    );
    let allowed = [
        std::ffi::OsString::from(GAME_FILE),
        BUILD_MARKER.into(),
        game.exe.clone().into(),
        pack_root.clone(),
    ];
    for entry in std::fs::read_dir(out)?.take(allowed.len() + 1) {
        let name = entry?.file_name();
        ensure!(
            allowed.contains(&name),
            "{} is not part of a build",
            name.to_string_lossy()
        );
    }
    let digest = build_files_digest(out, &pack_root, &game.exe, &game_bytes, progress)?;
    ensure!(
        marker == marker_bytes(&game_bytes, &digest),
        "its files were added to or changed since the build (for example inside {})",
        pack_root.to_string_lossy()
    );
    Ok(())
}

/// [`files_digest`] of an existing build folder: the pack walked with the packager's bounds
/// (links refused), plus the executable and `game.json`.
fn build_files_digest(
    out: &Path,
    pack_root: &std::ffi::OsStr,
    exe: &str,
    game_bytes: &[u8],
    progress: &Progress,
) -> Result<String> {
    let pack = out.join(pack_root);
    let walk = collect(&pack, &[], progress)?;
    let mut buffer = vec![0u8; COPY_CHUNK];
    let prefix = pack_root.to_string_lossy();
    let mut files = BTreeMap::new();
    for relative in &walk.files {
        let hash = hash_file(&pack.join(relative), &mut buffer, progress)?;
        files.insert(format!("{prefix}/{}", slash(relative)), hash);
    }
    files.insert(
        exe.to_string(),
        hash_file(&out.join(exe), &mut buffer, progress)?,
    );
    files.insert(
        GAME_FILE.to_string(),
        format!("{:x}", Sha256::digest(game_bytes)),
    );
    Ok(files_digest(&files))
}

/// Reads a regular file (never a link) of at most `limit` bytes.
fn read_plain_file(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let meta = std::fs::symlink_metadata(path).with_context(|| format!("{name} missing"))?;
    ensure!(
        meta.is_file() && !is_link(&meta),
        "{name} is not a plain file"
    );
    axiom_core::read_bounded(path, limit, &name)
}

/// Serializes builds of one output folder across processes (CLI and editor). The lock file
/// lives in the temp directory, keyed by the output path; the OS drops the lock when the
/// holder exits, so a crashed build never blocks the next one.
struct OutputLock(#[allow(dead_code)] std::fs::File);

impl OutputLock {
    fn acquire(out: &Path) -> Result<Self> {
        let key = format!(
            "{:x}",
            Sha256::digest(out.to_string_lossy().to_lowercase().as_bytes())
        );
        let path = std::env::temp_dir().join(format!("axiom-build-{}.lock", &key[..16]));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open build lock {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Self(file)),
            Err(std::fs::TryLockError::WouldBlock) => bail!(
                "another build is writing {}; wait for it or cancel it",
                shown(out)
            ),
            Err(std::fs::TryLockError::Error(e)) => {
                Err(e).with_context(|| format!("lock {}", path.display()))
            }
        }
    }
}

/// Renames `from` to `to`, failing (never replacing) when `to` exists.
fn rename_noreplace(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS};
        let wide = |p: &Path| p.as_os_str().encode_wide().chain([0]).collect::<Vec<u16>>();
        let (from, to) = (wide(from), wide(to));
        // SAFETY: both buffers are NUL-terminated UTF-16 that outlive the call. No flags:
        // MoveFileExW fails with ERROR_ALREADY_EXISTS instead of replacing `to`.
        unsafe {
            MoveFileExW(
                PCWSTR::from_raw(from.as_ptr()),
                PCWSTR::from_raw(to.as_ptr()),
                MOVE_FILE_FLAGS(0),
            )
        }
        .map_err(|e| {
            let code = e.code().0 as u32;
            if code & 0xFFFF_0000 == 0x8007_0000 {
                std::io::Error::from_raw_os_error((code & 0xFFFF) as i32)
            } else {
                std::io::Error::other(e)
            }
        })
    }
    #[cfg(not(windows))]
    {
        // Best effort off Windows: check, then rename (std::fs::rename replaces).
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(std::io::ErrorKind::AlreadyExists.into());
        }
        std::fs::rename(from, to)
    }
}

/// The cancellation cut-off: a cancel seen here still wins; after it the build is published.
/// Returns a warning when the previous build could not be removed after a successful swap.
fn publish(staging: &Path, out: &Path, force: bool, progress: &Progress) -> Result<Option<String>> {
    progress.check()?;
    let Some(meta) = existing(out)? else {
        return rename_noreplace(staging, out)
            .map(|()| None)
            .with_context(|| format!("finalize {}", shown(out)));
    };
    // Re-validated here: the folder may have appeared or changed during the build.
    ensure!(
        force,
        "output {} appeared during the build; pass --force to replace it",
        shown(out)
    );
    // Past the cancellation cut-off: this re-validation runs to completion.
    game_folder(out, &meta, &Progress::default())?;
    let backup = unique_sibling(out, "old", |p| rename_noreplace(out, p))
        .with_context(|| format!("move the previous build {} aside", shown(out)))?;
    if let Err(error) = rename_noreplace(staging, out) {
        let restored = rename_noreplace(&backup, out);
        return Err(error).with_context(|| match restored {
            Ok(()) => format!("finalize {} (the previous build was restored)", shown(out)),
            Err(_) => format!(
                "finalize {} (the previous build is kept at {})",
                shown(out),
                shown(&backup)
            ),
        });
    }
    Ok(remove_own_dir(&backup)
        .err()
        .map(|e| format!("previous build kept at {}: {e:#}", shown(&backup))))
}

#[allow(clippy::too_many_arguments)]
fn build(
    registry: &axiom_core::Registry,
    options: &PackageOptions,
    progress: &Progress,
    pack: &Path,
    staging: &Path,
    game: &GameManifest,
    game_bytes: &[u8],
) -> Result<Value> {
    let excludes: Vec<&str> = DEFAULT_EXCLUDES
        .iter()
        .copied()
        .chain(options.excludes.iter().map(String::as_str))
        .collect();
    let walk = collect(pack, &excludes, progress)?;
    // Content files, the player, game.json and the build marker.
    progress
        .total
        .store(walk.files.len() + 3, Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);

    let content = staging.join(CONTENT_DIR);
    std::fs::create_dir(&content)?;
    let mut files = vec![];
    let mut remaining = MAX_BYTES;
    let mut buffer = vec![0u8; COPY_CHUNK];
    let mut digest = Sha256::new();
    for relative in &walk.files {
        progress.check()?;
        let target = content.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let (size, hash) = copy_hashed(
            &pack.join(relative),
            &target,
            &mut remaining,
            &mut buffer,
            progress,
        )?;
        let path = slash(relative);
        digest.update(format!("{path}\t{hash}\n"));
        files.push(json!({"path":format!("{CONTENT_DIR}/{path}"),"bytes":size,"sha256":hash}));
        progress.done.fetch_add(1, Ordering::Relaxed);
    }
    let pack_digest = format!("{:x}", digest.finalize());
    let (size, hash) = copy_hashed(
        &options.player_exe,
        &staging.join(&game.exe),
        &mut remaining,
        &mut buffer,
        progress,
    )
    .with_context(|| format!("copy player {}", options.player_exe.display()))?;
    files.push(json!({"path":game.exe,"bytes":size,"sha256":hash}));
    progress.done.fetch_add(1, Ordering::Relaxed);

    let size = game_bytes.len() as u64;
    ensure!(size <= remaining, "output exceeds the 4 GiB budget");
    remaining -= size;
    write_new(&staging.join(GAME_FILE), game_bytes)?;
    files.push(
        json!({"path":GAME_FILE,"bytes":size,"sha256":format!("{:x}", Sha256::digest(game_bytes))}),
    );
    progress.done.fetch_add(1, Ordering::Relaxed);
    let written: BTreeMap<String, String> = files
        .iter()
        .map(|f| {
            (
                f["path"].as_str().unwrap_or_default().to_string(),
                f["sha256"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let marker = marker_bytes(game_bytes, &files_digest(&written));
    let size = marker.len() as u64;
    ensure!(size <= remaining, "output exceeds the 4 GiB budget");
    remaining -= size;
    write_new(&staging.join(BUILD_MARKER), &marker)?;
    files.push(
        json!({"path":BUILD_MARKER,"bytes":size,"sha256":format!("{:x}", Sha256::digest(&marker))}),
    );
    progress.done.fetch_add(1, Ordering::Relaxed);
    progress.check()?;

    // The copy must load on its own: every file the resolved world reads stays inside the
    // output folder, and every animation asset finds the files its importer will read.
    let world = axiom_core::ContentPackage::load(&content, registry)
        .context("packaged content does not load (an excluded file is still referenced?)")?
        .world;
    progress.check()?;
    let world = serde_json::to_value(&world)?;
    let root = staging.canonicalize()?;
    let escaped = outside_files(&world, &root);
    ensure!(
        escaped.is_empty(),
        "packaged world references files outside the output: {}",
        listing(&escaped)
    );
    check_asset_dependencies(&world, progress)?;
    if let Some(expected) = &options.expected_pack_digest {
        ensure!(
            expected.eq_ignore_ascii_case(&pack_digest),
            "pack_digest {pack_digest} does not match expected_pack_digest {expected}: the pack changed since it was reviewed"
        );
    }
    Ok(json!({
        "ok":true,"id":game.id,"title":game.title,"exe":game.exe,"files":files,
        "bytes":MAX_BYTES - remaining,"pack_digest":pack_digest,
        "skipped":walk.skipped,"skipped_total":walk.skipped_total,
    }))
}

struct Walk {
    files: Vec<PathBuf>,
    /// The first [`SKIPPED_LISTED`] excluded paths (directories end in `/`).
    skipped: Vec<String>,
    skipped_total: usize,
}

/// Iterative, bounded walk: at most [`MAX_ENTRIES`] entries read, [`MAX_DEPTH`] levels and
/// [`MAX_FILES`] kept files. Links and junctions are refused, never followed.
fn collect(root: &Path, excludes: &[&str], progress: &Progress) -> Result<Walk> {
    let mut walk = Walk {
        files: vec![],
        skipped: vec![],
        skipped_total: 0,
    };
    let mut entries_read = 0usize;
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        progress.check()?;
        let mut entries = vec![];
        for entry in std::fs::read_dir(&dir)? {
            entries_read += 1;
            ensure!(
                entries_read <= MAX_ENTRIES,
                "pack walk exceeds {MAX_ENTRIES} directory entries"
            );
            entries.push(entry?);
        }
        entries.sort_by_key(|e| e.file_name());
        let mut subdirs = vec![];
        for entry in entries {
            // `DirEntry::file_type` does not follow links; junctions count as links.
            let kind = entry.file_type()?;
            let relative = entry.path().strip_prefix(root)?.to_path_buf();
            ensure!(
                !kind.is_symlink(),
                "symbolic link or junction {} is not supported",
                slash(&relative)
            );
            let is_dir = kind.is_dir();
            if excluded(&relative, is_dir, excludes) {
                walk.skipped_total += 1;
                if walk.skipped.len() < SKIPPED_LISTED {
                    walk.skipped
                        .push(slash(&relative) + if is_dir { "/" } else { "" });
                }
                continue;
            }
            if is_dir {
                ensure!(
                    depth < MAX_DEPTH,
                    "pack is nested deeper than {MAX_DEPTH} levels at {}",
                    slash(&relative)
                );
                subdirs.push((entry.path(), depth + 1));
            } else {
                walk.files.push(relative);
                ensure!(
                    walk.files.len() <= MAX_FILES,
                    "pack exceeds {MAX_FILES} files"
                );
            }
        }
        stack.extend(subdirs.into_iter().rev());
    }
    walk.files.sort();
    Ok(walk)
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

/// Streams `from` into the new file `to` through `buffer`, hashing as it goes; the bytes come
/// out of `remaining` before they are written, and cancellation is checked between chunks.
fn copy_hashed(
    from: &Path,
    to: &Path,
    remaining: &mut u64,
    buffer: &mut [u8],
    progress: &Progress,
) -> Result<(u64, String)> {
    let mut input =
        std::fs::File::open(from).with_context(|| format!("read {}", from.display()))?;
    let length = input.metadata()?.len();
    ensure!(
        length <= *remaining,
        "output exceeds the 4 GiB budget at {} ({length} bytes)",
        from.display()
    );
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)
        .with_context(|| format!("write {}", to.display()))?;
    let mut hasher = Sha256::new();
    let mut copied = 0u64;
    loop {
        progress.check()?;
        let n = input
            .read(buffer)
            .with_context(|| format!("read {}", from.display()))?;
        if n == 0 {
            break;
        }
        copied += n as u64;
        ensure!(
            copied <= *remaining,
            "output exceeds the 4 GiB budget at {}",
            from.display()
        );
        hasher.update(&buffer[..n]);
        output
            .write_all(&buffer[..n])
            .with_context(|| format!("write {}", to.display()))?;
    }
    *remaining -= copied;
    Ok((copied, format!("{:x}", hasher.finalize())))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut f| f.write_all(bytes))
        .with_context(|| format!("write {}", path.display()))
}

const LISTED: usize = 8;

/// Every file the resolved world reads, by the field that names it: `(JSON pointer, path)`.
/// These are the fields package import rewrites to absolute paths (UI images, animation
/// assets, audio clip files; shaders are inlined). Other strings are content, never files.
pub fn resolved_files(world: &Value) -> Vec<(String, String)> {
    let mut found = vec![];
    let mut add = |pointer: String, value: &Value| {
        if let Some(path) = value.as_str() {
            found.push((pointer, path.to_string()));
        }
    };
    let esc = axiom_core::local::escape;
    if let Some(images) = world
        .pointer("/resources/ui.document/images")
        .and_then(Value::as_object)
    {
        for (id, path) in images {
            add(format!("/resources/ui.document/images/{}", esc(id)), path);
        }
    }
    // Content fonts (plan M4), resolved like images.
    if let Some(fonts) = world
        .pointer("/resources/ui.document/theme/fonts")
        .and_then(Value::as_object)
    {
        for (id, font) in fonts {
            add(
                format!("/resources/ui.document/theme/fonts/{}/path", esc(id)),
                &font["path"],
            );
        }
    }
    if let Some(assets) = world
        .pointer("/resources/animation.library/assets")
        .and_then(Value::as_object)
    {
        for (id, asset) in assets {
            add(
                format!("/resources/animation.library/assets/{}/path", esc(id)),
                &asset["path"],
            );
        }
    }
    if let Some(clips) = world
        .pointer("/resources/audio.library/clips")
        .and_then(Value::as_object)
    {
        for (id, clip) in clips {
            if let Some(path) = clip.pointer("/file/path") {
                add(
                    format!("/resources/audio.library/clips/{}/file/path", esc(id)),
                    path,
                );
            }
        }
    }
    found
}

/// Files the resolved world reads outside `root` (path -> why): every `external: true`
/// animation asset (resolved against the working directory, so never relocatable, even when
/// it happens to sit inside `root`) and every resolved file that does not canonicalize inside
/// `root` (missing files count as outside).
fn outside_files(world: &Value, root: &Path) -> BTreeMap<String, String> {
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
    for (pointer, path) in resolved_files(world) {
        let inside = Path::new(&path)
            .canonicalize()
            .is_ok_and(|p| p.starts_with(root));
        if !inside {
            found
                .entry(plain(&path))
                .or_insert_with(|| format!("{pointer} outside the pack"));
        }
    }
    found
}

fn listing(found: &BTreeMap<String, String>) -> String {
    let listed: Vec<String> = found
        .iter()
        .take(LISTED)
        .map(|(path, why)| format!("{path} ({why})"))
        .collect();
    let more = found.len().saturating_sub(LISTED);
    format!(
        "{}{}",
        listed.join(", "),
        if more > 0 {
            format!(", and {more} more")
        } else {
            String::new()
        }
    )
}

/// Fails, listing the first [`LISTED`] and the total, when the resolved source world reads a
/// file outside `pack`.
fn reject_outside(world: &Value, pack: &Path) -> Result<()> {
    let found = outside_files(world, pack);
    ensure!(
        found.is_empty(),
        "pack references {} file(s) outside {}; move them into the pack with relative paths (external: false): {}",
        found.len(),
        plain(&pack.to_string_lossy()),
        listing(&found)
    );
    Ok(())
}

/// The player's launch check, shared with the packager: the loaded world reads no file
/// outside `root` (the canonical shipped pack directory) and no `external` animation asset.
pub fn ensure_files_inside(world: &axiom_core::World, root: &Path) -> Result<()> {
    let found = outside_files(&serde_json::to_value(world)?, root);
    ensure!(
        found.is_empty(),
        "game content reads {} file(s) outside {}: {}",
        found.len(),
        plain(&root.to_string_lossy()),
        listing(&found)
    );
    Ok(())
}

/// Every animation asset of the resolved world finds the files its importer reads besides the
/// asset itself (glTF buffer/image URIs, `.axrig` textures; a PMX texture table sits behind
/// its geometry, so a PMX is imported whole).
fn check_asset_dependencies(world: &Value, progress: &Progress) -> Result<()> {
    let Some(library) = world.pointer("/resources/animation.library") else {
        return Ok(());
    };
    let library: axiom_animation::authoring::Library = serde_json::from_value(library.clone())?;
    for (id, source) in &library.assets {
        progress.check()?;
        asset_dependencies(&source.path, &source.options).with_context(|| {
            format!(
                "packaged animation asset {id} ({}) is incomplete (an excluded file is still needed?)",
                plain(&source.path.to_string_lossy())
            )
        })?;
    }
    Ok(())
}

/// Files `path` reads besides itself, resolved with the importer's `confined` rule (an error
/// names a missing one). Unknown formats have none here (the importer refuses them).
fn asset_dependencies(
    path: &Path,
    options: &axiom_animation::import::ImportOptions,
) -> Result<Vec<PathBuf>> {
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let uris: Vec<String> = match extension.as_str() {
        "gltf" => gltf_uris(&axiom_core::read_bounded(path, MAX_ASSET_FILE, "glTF")?)?,
        "glb" => gltf_uris(&glb_json(path)?)?,
        "axrig" => {
            #[derive(Deserialize)]
            struct Textures {
                #[serde(default)]
                textures: Vec<String>,
            }
            let bundle: Textures = serde_json::from_slice(&axiom_core::read_bounded(
                path,
                MAX_ASSET_FILE,
                ".axrig bundle",
            )?)?;
            bundle.textures
        }
        "pmx" => {
            axiom_animation::import::load(path, options)?;
            vec![]
        }
        _ => vec![],
    };
    let root = path.parent().unwrap_or(Path::new("."));
    uris.iter()
        .map(|uri| axiom_animation::import::confined(root, uri))
        .collect()
}

/// External (non-`data:`) buffer and image URIs of a glTF JSON document, percent-decoded the
/// way the importer decodes them.
fn gltf_uris(json: &[u8]) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Uri {
        #[serde(default)]
        uri: Option<String>,
    }
    #[derive(Deserialize)]
    struct Doc {
        #[serde(default)]
        buffers: Vec<Uri>,
        #[serde(default)]
        images: Vec<Uri>,
    }
    let doc: Doc = serde_json::from_slice(json).context("glTF JSON")?;
    let mut uris = vec![];
    for uri in doc
        .buffers
        .into_iter()
        .chain(doc.images)
        .filter_map(|u| u.uri)
    {
        if uri.starts_with("data:") {
            continue;
        }
        let mut bytes = vec![];
        let mut input = uri.bytes();
        while let Some(b) = input.next() {
            if b == b'%' {
                let digits = [
                    input.next().context("invalid URI escape")?,
                    input.next().context("invalid URI escape")?,
                ];
                bytes.push(u8::from_str_radix(std::str::from_utf8(&digits)?, 16)?);
            } else {
                bytes.push(b);
            }
        }
        uris.push(String::from_utf8(bytes)?);
    }
    Ok(uris)
}

/// The JSON chunk of a GLB, read without loading its binary chunk.
fn glb_json(path: &Path) -> Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();
    let mut header = [0u8; 20];
    file.read_exact(&mut header)?;
    ensure!(
        &header[0..4] == b"glTF" && &header[16..20] == b"JSON",
        "invalid GLB header"
    );
    let size = u32::from_le_bytes(header[12..16].try_into()?) as u64;
    ensure!(
        size <= length && size <= MAX_ASSET_FILE as u64,
        "truncated GLB JSON chunk"
    );
    let mut json = vec![];
    file.take(size).read_to_end(&mut json)?;
    ensure!(json.len() as u64 == size, "truncated GLB JSON chunk");
    Ok(json)
}

/// Pack-relative files the runtime reads: for the root package and every dependency
/// (recursively, through manifest `dependencies[*].path`) its manifest, world file, declared
/// `assets` and `render.shaders` `.wgsl` files (inlined at import, so only the raw world names
/// them); plus every resolved file field of the world (animation assets with their glTF
/// buffers and textures, UI images, audio clips). Anything read by other means is not covered.
fn referenced_files(pack: &Path, world: &Value) -> Result<BTreeSet<String>> {
    let mut referenced = BTreeSet::new();
    let mut visited = BTreeSet::new();
    manifest_closure(pack, pack, &mut visited, &mut referenced)?;
    for (_, path) in resolved_files(world) {
        add_file(pack, Path::new(&path), &mut referenced);
    }
    if let Some(assets) = world
        .pointer("/resources/animation.library/assets")
        .and_then(Value::as_object)
    {
        for asset in assets.values() {
            let Some(path) = asset["path"].as_str().map(Path::new) else {
                continue;
            };
            // Report only: a PMX is not imported here, and a broken dependency fails later,
            // in the staged-output check.
            if path
                .extension()
                .is_some_and(|e| !e.eq_ignore_ascii_case("pmx"))
            {
                for dependency in asset_dependencies(path, &Default::default()).unwrap_or_default()
                {
                    add_file(pack, &dependency, &mut referenced);
                }
            }
        }
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
    let manifest: Value = serde_json::from_slice(&axiom_core::read_bounded(
        &manifest_path,
        65536,
        "manifest",
    )?)?;
    add_file(pack, &manifest_path, referenced);
    if let Some(world) = manifest["world"].as_str() {
        let world_path = dir.join(world);
        add_file(pack, &world_path, referenced);
        let raw: Value = serde_json::from_slice(&axiom_core::read_bounded(
            &world_path,
            4_194_304,
            "package world",
        )?)?;
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

/// The output folder: its parent in canonical form, its last component kept as written. The
/// last component is never resolved, so an existing link or junction there is inspected (and
/// refused) as itself instead of as the directory it points to.
fn output_path(path: &Path) -> Result<PathBuf> {
    let path = absolute(path)?;
    let name = match path.components().next_back() {
        Some(std::path::Component::Normal(name)) => name.to_os_string(),
        _ => bail!("output {} must end in a folder name", path.display()),
    };
    let parent = path
        .parent()
        .context("output path needs a parent directory")?;
    Ok(normalized(parent)?.join(name))
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

/// A path as people read it: canonical Windows paths lose their `\\?\` verbatim prefix.
fn shown(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string()
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
            expected_pack_digest: None,
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
        assert!(!base.join("out/cancelled").exists() && leftovers(&base.join("out")).is_empty());

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
            expected_pack_digest: None,
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
        assert!(!out.exists() && leftovers(&base.join("out")).is_empty());
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
    fn audio_file_clips_are_referenced_and_packaged() {
        let registry = crate::installed_registry().authoring;
        let (base, player) = scratch("audio");
        let pack = base.join("pack");
        copy_dir(&content("audio-lab"), &pack);
        std::fs::write(pack.join("sounds/unused.wav"), b"not read").unwrap();
        let out = base.join("out/Game");
        let report = package(
            &registry,
            &options(&pack, &out, &player),
            &Progress::default(),
        )
        .unwrap();
        assert_eq!(
            strings(&report, "unreferenced"),
            ["sounds/unused.wav"],
            "{report}"
        );
        for file in ["sounds/hum.wav", "sounds/chime.ogg"] {
            assert!(out.join("content").join(file).is_file(), "{file}");
        }
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

    /// Staging and backup directories a build may leave beside its output.
    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.contains(".partial-") || n.contains(".old-"))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn two_cameras(base: &Path) -> PathBuf {
        let pack = base.join("pack");
        copy_dir(&content("two-cameras"), &pack);
        pack
    }

    // R9 F1: a pack named like the old fixed staging folder (`<out>.partial`) is never removed,
    // and neither is another build's staging folder.
    #[test]
    fn staging_never_removes_a_pack_or_foreign_partial_folder() {
        let registry = crate::installed_registry().authoring;
        let (base, player) = scratch("f1");
        let pack = base.join("Game.partial");
        copy_dir(&content("two-cameras"), &pack);
        let foreign = base.join("Other.partial");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("keep.txt"), "x").unwrap();
        let out = base.join("Game");
        package(
            &registry,
            &options(&pack, &out, &player),
            &Progress::default(),
        )
        .unwrap();
        assert!(pack.join("manifest.json").is_file(), "the pack was deleted");
        assert!(out.join(GAME_FILE).is_file());
        // A failed build (missing player) removes only its own staging folder.
        let mut failing = options(&pack, &base.join("Other"), &base.join("missing.exe"));
        failing.force = true;
        assert!(package(&registry, &failing, &Progress::default()).is_err());
        assert!(foreign.join("keep.txt").is_file() && pack.join("manifest.json").is_file());
        assert!(leftovers(&base).is_empty(), "{:?}", leftovers(&base));
        std::fs::remove_dir_all(&base).unwrap();
    }

    // R9 F2: publishing never replaces a destination that appeared during the build, and a
    // forced replacement that cannot be put in place restores the previous build.
    #[test]
    fn publish_refuses_new_destinations_and_restores_on_failure() {
        let (base, _) = scratch("f2");
        let out = base.join("Game");
        let staging = base.join("Game.partial-test");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join(GAME_FILE), "new").unwrap();
        // Appeared during the build (not a game folder, no --force): untouched.
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("mine.txt"), "x").unwrap();
        let error = publish(&staging, &out, false, &Progress::default()).unwrap_err();
        assert!(
            error.to_string().contains("appeared during the build"),
            "{error}"
        );
        assert!(out.join("mine.txt").is_file() && staging.join(GAME_FILE).is_file());
        // --force re-validates: still not a packaged game folder.
        let error = publish(&staging, &out, true, &Progress::default()).unwrap_err();
        assert!(error.to_string().contains("not a packaged game"), "{error}");
        assert!(out.join("mine.txt").is_file());
        // A previous build whose replacement fails (staging vanished) is restored in place.
        std::fs::remove_dir_all(&out).unwrap();
        let old = fake_build(&out, "old");
        let gone = base.join("Game.partial-gone");
        let error = publish(&gone, &out, true, &Progress::default()).unwrap_err();
        assert!(format!("{error:#}").contains("restored"), "{error:#}");
        assert_eq!(std::fs::read(out.join(GAME_FILE)).unwrap(), old);
        // A successful forced swap leaves no backup behind.
        publish(&staging, &out, true, &Progress::default()).unwrap();
        assert_eq!(std::fs::read_to_string(out.join(GAME_FILE)).unwrap(), "new");
        assert!(
            !staging.exists() && leftovers(&base).is_empty(),
            "{:?}",
            leftovers(&base)
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// A folder laid out exactly like a build (valid game.json titled `title`, matching marker,
    /// executable, content directory); returns its game.json bytes.
    fn fake_build(out: &Path, title: &str) -> Vec<u8> {
        std::fs::create_dir_all(out.join(CONTENT_DIR)).unwrap();
        let game = GameManifest {
            format: FORMAT,
            id: "axiom.test".into(),
            title: title.into(),
            pack: CONTENT_DIR.into(),
            exe: "Game.exe".into(),
            engine: String::new(),
        }
        .to_bytes()
        .unwrap();
        std::fs::write(out.join(GAME_FILE), &game).unwrap();
        std::fs::write(out.join("Game.exe"), b"stub").unwrap();
        let digest = build_files_digest(
            out,
            CONTENT_DIR.as_ref(),
            "Game.exe",
            &game,
            &Progress::default(),
        )
        .unwrap();
        std::fs::write(out.join(BUILD_MARKER), marker_bytes(&game, &digest)).unwrap();
        game
    }

    /// Relative path and contents of every file under `dir`.
    fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut found = vec![];
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let e = e.unwrap();
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else {
                    found.push((
                        e.path().strip_prefix(dir).unwrap().to_path_buf(),
                        std::fs::read(e.path()).unwrap(),
                    ));
                }
            }
        }
        found.sort();
        found
    }

    // R9 F2 (round 3): `package --force` given a junction to a real build refuses the junction
    // itself; the build it points to keeps every file. Goes through the public entry, which
    // used to resolve the junction to its target before checking it.
    #[cfg(windows)]
    #[test]
    fn force_refuses_a_junction_output() {
        let registry = crate::installed_registry().authoring;
        let (base, player) = scratch("junction");
        let pack = two_cameras(&base);
        let real = base.join("real");
        package(
            &registry,
            &options(&pack, &real, &player),
            &Progress::default(),
        )
        .unwrap();
        let before = snapshot(&real);
        let link = base.join("link");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&real)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        let mut opts = options(&pack, &link, &player);
        opts.force = true;
        opts.title = Some("Replacement".into());
        let error = package(&registry, &opts, &Progress::default()).unwrap_err();
        assert!(
            error.to_string().contains("not a packaged game"),
            "{error:#}"
        );
        assert_eq!(snapshot(&real), before, "the junction target changed");
        let meta = std::fs::symlink_metadata(&link).unwrap();
        assert!(is_link(&meta), "the junction itself was replaced");
        assert!(leftovers(&base).is_empty(), "{:?}", leftovers(&base));
        std::fs::remove_dir(&link).unwrap();
        std::fs::remove_dir_all(&base).unwrap();
    }

    // R9 round 4 N1: on Windows `GAME` opens the existing build `Game`; packaging that build's
    // own `content` into the case alias with --force is an overlap and changes nothing.
    #[cfg(windows)]
    #[test]
    fn force_refuses_a_case_alias_of_the_source_build() {
        let registry = crate::installed_registry().authoring;
        let (base, player) = scratch("case");
        let pack = two_cameras(&base);
        let game = base.join("Game");
        package(
            &registry,
            &options(&pack, &game, &player),
            &Progress::default(),
        )
        .unwrap();
        let before = snapshot(&game);
        let mut opts = options(&game.join(CONTENT_DIR), &base.join("GAME"), &player);
        opts.force = true;
        let error = package(&registry, &opts, &Progress::default()).unwrap_err();
        assert!(error.to_string().contains("overlap"), "{error:#}");
        assert_eq!(snapshot(&game), before, "the source build changed");
        let names: Vec<_> = std::fs::read_dir(&base)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(names.contains(&"Game".into()), "{names:?}");
        assert!(leftovers(&base).is_empty(), "{:?}", leftovers(&base));
        std::fs::remove_dir_all(&base).unwrap();
    }

    // R9 N2: --force replaces only folders proven to be a build (valid game.json, matching
    // marker, executable, content, nothing else). Every other folder keeps every file.
    #[test]
    fn force_refuses_folders_that_are_not_proven_builds() {
        let registry = crate::installed_registry().authoring;
        let (base, player) = scratch("n2");
        let pack = two_cameras(&base);
        let mut opts = options(&pack, &base.join("built"), &player);
        package(&registry, &opts, &Progress::default()).unwrap();
        opts.force = true;
        type Case = (&'static str, fn(&Path));
        let cases: [Case; 8] = [
            ("empty-game-json", |d| {
                std::fs::write(d.join(GAME_FILE), "").unwrap();
                std::fs::write(d.join("keep.txt"), "work").unwrap();
            }),
            ("invalid-game-json", |d| {
                std::fs::write(d.join(GAME_FILE), "{}").unwrap();
                std::fs::write(d.join("keep.txt"), "work").unwrap();
            }),
            ("no-marker", |d| {
                std::fs::remove_file(d.join(BUILD_MARKER)).unwrap();
            }),
            ("stale-marker", |d| {
                let mut game: Value =
                    serde_json::from_slice(&std::fs::read(d.join(GAME_FILE)).unwrap()).unwrap();
                game["title"] = json!("Edited by hand");
                std::fs::write(d.join(GAME_FILE), serde_json::to_vec(&game).unwrap()).unwrap();
            }),
            ("extra-file", |d| {
                std::fs::write(d.join("my notes.txt"), "work").unwrap();
            }),
            // R9 round 4 N2: files added or changed inside `content` are user data too.
            ("nested-extra-file", |d| {
                std::fs::write(d.join(CONTENT_DIR).join("my notes.txt"), "work").unwrap();
            }),
            ("deep-extra-file", |d| {
                let dir = d.join(CONTENT_DIR).join("notes").join("today");
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join("draft.md"), "work").unwrap();
            }),
            ("changed-content-file", |d| {
                let world = d.join(CONTENT_DIR).join("world.json");
                let mut bytes = std::fs::read(&world).unwrap();
                bytes.extend_from_slice(b"\n");
                std::fs::write(world, bytes).unwrap();
            }),
        ];
        for (name, prepare) in cases {
            let dir = base.join(name);
            if name == "empty-game-json" || name == "invalid-game-json" {
                std::fs::create_dir_all(&dir).unwrap();
            } else {
                copy_dir(&base.join("built"), &dir);
            }
            prepare(&dir);
            let before = snapshot(&dir);
            opts.out = dir.clone();
            let error = package(&registry, &opts, &Progress::default()).unwrap_err();
            assert!(
                error.to_string().contains("not a packaged game folder"),
                "{name}: {error:#}"
            );
            assert_eq!(snapshot(&dir), before, "{name}: files changed");
        }
        // The real build is replaced.
        opts.out = base.join("built");
        opts.title = Some("Rebuilt".into());
        package(&registry, &opts, &Progress::default()).unwrap();
        assert_eq!(
            GameManifest::read(&base.join("built")).unwrap().title,
            "Rebuilt"
        );
        assert!(leftovers(&base).is_empty(), "{:?}", leftovers(&base));
        std::fs::remove_dir_all(&base).unwrap();
    }

    // R9 F3: the byte budget is checked before a file is written, and it covers the player
    // and game.json, not only content.
    #[test]
    fn byte_budget_is_checked_before_writing() {
        let (base, player) = scratch("f3");
        let target = base.join("copy.exe");
        let mut remaining = 3u64;
        let mut buffer = vec![0u8; 2];
        let error = copy_hashed(
            &player,
            &target,
            &mut remaining,
            &mut buffer,
            &Progress::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("4 GiB budget"), "{error}");
        assert!(!target.exists(), "nothing may be written over budget");
        assert_eq!(remaining, 3);
        // Streamed through a 2-byte buffer: same bytes and hash as a whole-file read.
        let mut remaining = 100u64;
        let (size, hash) = copy_hashed(
            &player,
            &target,
            &mut remaining,
            &mut buffer,
            &Progress::default(),
        )
        .unwrap();
        assert_eq!((size, remaining), (4, 96));
        assert_eq!(hash, format!("{:x}", Sha256::digest(b"stub")));
        assert_eq!(std::fs::read(&target).unwrap(), b"stub");
        std::fs::remove_dir_all(&base).unwrap();
    }

    // R9 F5: a cancel that arrives after the last copy still wins at the publish cut-off.
    #[test]
    fn cancel_before_publish_keeps_the_previous_output() {
        let (base, _) = scratch("f5");
        let out = base.join("Game");
        let staging = base.join("Game.partial-test");
        std::fs::create_dir_all(&staging).unwrap();
        let progress = Progress::default();
        progress.cancel.store(true, Ordering::Relaxed);
        let error = publish(&staging, &out, true, &progress).unwrap_err();
        assert!(error.to_string().contains("cancelled"), "{error}");
        assert!(!out.exists() && staging.exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    // R9 F6: a kept .gltf whose excluded .bin it still needs fails the build; kept, the .bin
    // counts as referenced.
    #[test]
    fn gltf_buffers_must_ship_with_the_gltf() {
        let registry = crate::installed_registry().authoring;
        let (base, player) = scratch("f6");
        let pack = base.join("pack");
        copy_dir(&content("animation"), &pack);
        std::fs::write(
            pack.join("model.gltf"),
            r#"{"asset":{"version":"2.0"},"buffers":[{"uri":"mesh%20data.bin","byteLength":4}],"images":[{"uri":"data:image/png;base64,AA=="}]}"#,
        )
        .unwrap();
        std::fs::write(pack.join("mesh data.bin"), b"abcd").unwrap();
        let world_path = pack.join("world.json");
        let mut world: Value =
            serde_json::from_slice(&std::fs::read(&world_path).unwrap()).unwrap();
        world["resources"]["animation.library"]["assets"]["extra"] = json!({"path":"model.gltf"});
        std::fs::write(&world_path, serde_json::to_vec(&world).unwrap()).unwrap();
        let mut opts = options(&pack, &base.join("out/Game"), &player);
        opts.excludes = vec!["mesh data.bin".into()];
        let error = format!(
            "{:#}",
            package(&registry, &opts, &Progress::default()).unwrap_err()
        );
        assert!(
            error.contains("animation asset extra") && error.contains("mesh data.bin"),
            "{error}"
        );
        assert!(!base.join("out/Game").exists() && leftovers(&base.join("out")).is_empty());
        opts.excludes.clear();
        let report = package(&registry, &opts, &Progress::default()).unwrap();
        assert!(
            !strings(&report, "unreferenced").contains(&"mesh data.bin".to_string()),
            "{report}"
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    // R9 F7 + F13: the packager writes the game.json the player reads, refuses an id the
    // player would refuse before writing anything, and lists game.json with its hash.
    #[test]
    fn game_json_contract_and_hash() {
        let registry = crate::installed_registry().authoring;
        let (base, player) = scratch("f7");
        let pack = two_cameras(&base);
        let out = base.join("out/Game");
        let mut opts = options(&pack, &out, &player);
        opts.title = Some("First".into());
        let first = package(&registry, &opts, &Progress::default()).unwrap();
        let game = GameManifest::read(&out).unwrap();
        assert_eq!(
            (
                game.id.as_str(),
                game.title.as_str(),
                game.pack.as_str(),
                game.exe.as_str()
            ),
            ("axiom.two-cameras", "First", CONTENT_DIR, "Game.exe")
        );
        let entry = first["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == GAME_FILE)
            .expect("game.json is listed")
            .clone();
        let bytes = std::fs::read(out.join(GAME_FILE)).unwrap();
        assert_eq!(entry["sha256"], format!("{:x}", Sha256::digest(&bytes)));
        let total: u64 = first["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["bytes"].as_u64().unwrap())
            .sum();
        assert_eq!(first["bytes"], total);
        opts.title = Some("Second".into());
        opts.force = true;
        let second = package(&registry, &opts, &Progress::default()).unwrap();
        assert_ne!(hashes(&first), hashes(&second), "a title change must show");

        // A Unicode id passes content validation but not the player's rule: refused early.
        let manifest_path = pack.join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest["id"] = json!("相機");
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let unicode = base.join("out/Unicode");
        let error = package(
            &registry,
            &options(&pack, &unicode, &player),
            &Progress::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("save folder"), "{error:#}");
        assert!(!unicode.exists() && leftovers(&base.join("out")).is_empty());
        for bad in [
            r#"{"format":2,"id":"a","title":"t","pack":"content"}"#,
            r#"{"format":1,"id":"../x","title":"t","pack":"content"}"#,
            r#"{"format":1,"id":"a","title":" ","pack":"content"}"#,
            r#"{"format":1,"id":"a","title":"t","pack":"../content"}"#,
            r#"{"format":1,"id":"a","title":"t","pack":"content","exe":"..\\x.exe"}"#,
        ] {
            assert!(GameManifest::parse(bad.as_bytes()).is_err(), "{bad}");
        }
        assert!(GameManifest::parse(&vec![b' '; MAX_GAME_FILE + 1]).is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }

    // R9 F8: the report carries the digest of the shipped pack files, and a build pinned to
    // a reviewed digest refuses a pack that changed since.
    #[test]
    fn pack_digest_is_reported_and_can_be_pinned() {
        let registry = crate::installed_registry().authoring;
        let (base, player) = scratch("f8");
        let pack = two_cameras(&base);
        let first = package(
            &registry,
            &options(&pack, &base.join("out/a"), &player),
            &Progress::default(),
        )
        .unwrap();
        let digest = first["pack_digest"].as_str().unwrap().to_string();
        assert_eq!(digest.len(), 64);
        let mut pinned = options(&pack, &base.join("out/b"), &player);
        pinned.expected_pack_digest = Some(digest.clone());
        let second = package(&registry, &pinned, &Progress::default()).unwrap();
        assert_eq!(second["pack_digest"], digest.as_str());
        std::fs::write(pack.join("notes.txt"), "changed after review").unwrap();
        pinned.out = base.join("out/c");
        let error = package(&registry, &pinned, &Progress::default()).unwrap_err();
        assert!(
            error.to_string().contains("expected_pack_digest"),
            "{error}"
        );
        assert!(!base.join("out/c").exists() && leftovers(&base.join("out")).is_empty());
        std::fs::remove_dir_all(&base).unwrap();
    }

    // R9 F10: the walk is bounded in depth and reports a bounded sample of exclusions.
    #[test]
    fn walk_bounds_depth_and_excluded_listing() {
        let (base, _) = scratch("f10");
        let pack = base.join("pack");
        std::fs::create_dir_all(&pack).unwrap();
        for i in 0..40 {
            std::fs::write(pack.join(format!("s{i:02}.acl")), "x").unwrap();
        }
        std::fs::write(pack.join("keep.txt"), "x").unwrap();
        let walk = collect(&pack, DEFAULT_EXCLUDES, &Progress::default()).unwrap();
        assert_eq!(walk.files, [PathBuf::from("keep.txt")]);
        assert_eq!(walk.skipped_total, 40);
        assert_eq!(walk.skipped.len(), SKIPPED_LISTED);
        let mut deep = pack.clone();
        for _ in 0..=MAX_DEPTH {
            deep = deep.join("d");
        }
        std::fs::create_dir_all(&deep).unwrap();
        let error = collect(&pack, DEFAULT_EXCLUDES, &Progress::default())
            .err()
            .expect("too deep");
        assert!(error.to_string().contains("nested deeper"), "{error}");
        std::fs::remove_dir_all(&base).unwrap();
    }

    // R9 F12: an absolute path that is only content text (here the world title) is not a
    // file reference; resolved file fields are named in errors.
    #[test]
    fn absolute_path_text_is_not_a_file_reference() {
        let registry = crate::installed_registry().authoring;
        let (base, player) = scratch("f12");
        let pack = two_cameras(&base);
        let world_path = pack.join("world.json");
        let mut world: Value =
            serde_json::from_slice(&std::fs::read(&world_path).unwrap()).unwrap();
        world["title"] = json!(r"Saves go to C:\Users\Public\Documents\never-read.txt");
        std::fs::write(&world_path, serde_json::to_vec(&world).unwrap()).unwrap();
        let mut opts = options(&pack, &base.join("out/Game"), &player);
        opts.title = Some("Game".into());
        package(&registry, &opts, &Progress::default()).unwrap();
        let resolved = json!({"resources":{"audio.library":{"clips":{"hum":{"file":{"path":r"C:\nowhere\hum.wav"}}}}}});
        let found = outside_files(&resolved, &base.canonicalize().unwrap());
        assert_eq!(
            found.values().collect::<Vec<_>>(),
            ["/resources/audio.library/clips/hum/file/path outside the pack"]
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    // R9 F4: the player's launch check refuses external animation assets.
    #[test]
    fn launch_check_refuses_external_animation_assets() {
        let registry = crate::installed_registry().authoring;
        let (base, _) = scratch("f4");
        let pack = base.join("pack");
        copy_dir(&content("animation"), &pack);
        let root = pack.canonicalize().unwrap();
        let world = axiom_core::ContentPackage::load(&root, &registry)
            .unwrap()
            .world;
        ensure_files_inside(&world, &root).unwrap();
        let world_path = pack.join("world.json");
        let mut raw: Value = serde_json::from_slice(&std::fs::read(&world_path).unwrap()).unwrap();
        let asset = &mut raw["resources"]["animation.library"]["assets"]["character"];
        asset["external"] = json!(true);
        asset["path"] = json!(root.join("rigify.glb").to_string_lossy());
        std::fs::write(&world_path, serde_json::to_vec(&raw).unwrap()).unwrap();
        let world = axiom_core::ContentPackage::load(&root, &registry)
            .unwrap()
            .world;
        let error = ensure_files_inside(&world, &root).unwrap_err();
        assert!(
            error.to_string().contains("external animation asset"),
            "{error}"
        );
        std::fs::remove_dir_all(&base).unwrap();
    }
}
