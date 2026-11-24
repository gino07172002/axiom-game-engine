//! Windowed player for a packaged game (docs/plan-packaging.zh-TW.md, P2).
//!
//! Reads `game.json` beside its own executable, keeps the database and saves in
//! `%APPDATA%\Axiom\<id>\` (the game folder stays read-only), re-imports the shipped pack on
//! every launch and runs it with player chrome: no developer bar, console or loopback port.
//! There is no console window, so startup errors go to `crash.log` and a message box
//! (`AXIOM_PLAYER_NO_DIALOG=1` skips the box for automation).
//!
//! Automation flags: `--smoke-frames N --screenshot PATH --db PATH --port P`.
#![windows_subsystem = "windows"]

use anyhow::{bail, ensure, Context, Result};
use axiom_runtime::packaging::{GameManifest, GAME_FILE};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const NO_DIALOG: &str = "AXIOM_PLAYER_NO_DIALOG";

/// Where a failure is reported: `%APPDATA%\Axiom\<id>\crash.log` (or beside `--db`) once the
/// id is known, and the database path no crash log may ever take (it may not exist yet).
static CRASH_LOG: Mutex<Option<(PathBuf, PathBuf)>> = Mutex::new(None);

#[derive(Default)]
struct Args {
    smoke_frames: Option<u32>,
    screenshot: Option<PathBuf>,
    db: Option<PathBuf>,
    port: u16,
}

struct Game {
    id: String,
    title: String,
    /// Canonical shipped pack directory, inside the canonical game folder.
    pack: PathBuf,
}

fn main() {
    std::panic::set_hook(Box::new(|info| report(&format!("panic: {info}"))));
    if let Err(error) = run() {
        report(&format!("{error:#}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = parse_args(std::env::args_os().skip(1))?;
    let exe = std::env::current_exe().context("locate the player executable")?;
    let dir = exe.parent().context("player executable has no directory")?;
    let game = read_game(dir)?;
    let (db, crash_log) = locations(args.db, &game.id, std::env::var_os("APPDATA"))?;
    *CRASH_LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some((crash_log, db.clone()));
    if let Some(parent) = db.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create database directory {}", parent.display()))?;
    }
    let registry = axiom_runtime::installed_registry();
    let world = axiom_core::ContentPackage::load(&game.pack, &registry.authoring)
        .with_context(|| format!("load game content {}", game.pack.display()))?
        .world;
    // The packager's rule, enforced again at launch: nothing outside the shipped pack.
    axiom_runtime::packaging::ensure_files_inside(&world, &game.pack)?;
    axiom_runtime::run(
        axiom_runtime::RunOptions {
            mode: axiom_runtime::RunMode::Play {
                player: Some(game.title),
            },
            load_slot: None,
            // The shipped pack is the truth: re-import it on every launch (paths are absolute
            // in the database, so the folder may have moved since the last run).
            replace_world: true,
            db,
            port: args.port,
            smoke_frames: args.smoke_frames,
            screenshot: args.screenshot,
            start_play: true,
        },
        registry,
        world,
    )
}

fn parse_args(mut raw: impl Iterator<Item = std::ffi::OsString>) -> Result<Args> {
    let mut args = Args::default();
    while let Some(flag) = raw.next() {
        let flag = flag.to_string_lossy().into_owned();
        let mut value = || {
            raw.next()
                .with_context(|| format!("{flag} requires a value"))
        };
        match flag.as_str() {
            "--smoke-frames" => {
                let n: u32 = value()?.to_string_lossy().parse()?;
                ensure!(n > 0, "--smoke-frames must be positive");
                args.smoke_frames = Some(n);
            }
            "--screenshot" => args.screenshot = Some(value()?.into()),
            "--db" => args.db = Some(value()?.into()),
            "--port" => args.port = value()?.to_string_lossy().parse()?,
            other => bail!("unknown argument {other}"),
        }
    }
    ensure!(
        args.screenshot.is_none() || args.smoke_frames.is_some(),
        "--screenshot requires --smoke-frames"
    );
    Ok(args)
}

fn read_game(dir: &Path) -> Result<Game> {
    let game = GameManifest::read(dir)?;
    // Resolve links and junctions: the pack must really sit inside the game folder.
    let root = dir
        .canonicalize()
        .with_context(|| format!("game folder {}", dir.display()))?;
    let pack = root
        .join(&game.pack)
        .canonicalize()
        .with_context(|| format!("game content {} not found", dir.join(&game.pack).display()))?;
    ensure!(
        pack.starts_with(&root) && pack != root,
        "{GAME_FILE}: pack {:?} resolves outside the game folder (to {})",
        game.pack,
        pack.display()
    );
    Ok(Game {
        id: game.id,
        title: game.title,
        pack,
    })
}

/// The database and crash log. `%APPDATA%` is needed only for the default save directory:
/// with `--db` the crash log goes beside the database, under a name distinct from it.
fn locations(
    db: Option<PathBuf>,
    id: &str,
    appdata: Option<std::ffi::OsString>,
) -> Result<(PathBuf, PathBuf)> {
    if let Some(db) = db {
        let dir = db
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let taken = db
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("crash.log"));
        let log = if taken {
            "player-crash.log"
        } else {
            "crash.log"
        };
        return Ok((db, dir.join(log)));
    }
    let appdata = appdata
        .filter(|v| !v.is_empty())
        .context("APPDATA is not set; cannot choose a save directory (pass --db PATH)")?;
    let data = PathBuf::from(appdata).join("Axiom").join(id);
    std::fs::create_dir_all(&data)
        .with_context(|| format!("create save directory {}", data.display()))?;
    Ok((data.join("world.sqlite"), data.join("crash.log")))
}

/// Writes the crash log (best effort) and tells the person, since there is no console.
fn report(message: &str) {
    let (log, db) = CRASH_LOG.lock().ok().and_then(|g| g.clone()).map_or_else(
        || (std::env::temp_dir().join("axiom-player-crash.log"), None),
        |(log, db)| (log, Some(db)),
    );
    let when = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let exe = std::env::current_exe().unwrap_or_default();
    let body = format!(
        "axiom-player {}\r\nunix_time: {when}\r\nexe: {}\r\nerror: {message}\r\n",
        env!("CARGO_PKG_VERSION"),
        exe.display()
    );
    let written = log
        .parent()
        .map(std::fs::create_dir_all)
        .transpose()
        .ok()
        .and_then(|_| write_log(&log, db.as_deref(), body.as_bytes()));
    eprintln!("axiom-player: {message}");
    if std::env::var_os(NO_DIALOG).is_some_and(|v| v == "1") {
        return;
    }
    let detail = match written {
        Some(path) => format!("{message}\n\nDetails were written to:\n{}", path.display()),
        None => message.to_string(),
    };
    dialog("The game could not start", &detail);
}

/// Every crash log starts with this; an existing file without it is never overwritten.
const LOG_HEADER: &[u8] = b"axiom-player ";

/// Writes `body` (which starts with [`LOG_HEADER`]) to `path`, or to `<stem>-<n>.<ext>`
/// beside it when `path` holds something other than an earlier crash log (a database, for
/// example). No candidate ever has the file name of `db` (compared ignoring ASCII case), even
/// when that database does not exist yet: a later launch with the same `--db` must be able to
/// create it. Returns where it was written.
fn write_log(path: &Path, db: Option<&Path>, body: &[u8]) -> Option<PathBuf> {
    let avoid = db.and_then(Path::file_name);
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    let ext = path
        .extension()
        .map_or_else(String::new, |e| format!(".{}", e.to_string_lossy()));
    std::iter::once(path.to_path_buf())
        .chain((1..=16).map(|n| path.with_file_name(format!("{stem}-{n}{ext}"))))
        .filter(|candidate| {
            !avoid.is_some_and(|a| {
                candidate
                    .file_name()
                    .is_some_and(|c| c.eq_ignore_ascii_case(a))
            })
        })
        .find(|candidate| replace_log(candidate, body).is_ok_and(|w| w))
}

/// Creates `path`, or overwrites it only when it is an earlier crash log (checked on the same
/// handle that is then truncated). `Ok(false)` means the file belongs to someone else.
fn replace_log(path: &Path, body: &[u8]) -> std::io::Result<bool> {
    use std::io::{Read, Seek, Write};
    let mut file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if !std::fs::symlink_metadata(path)?.is_file() {
                return Ok(false);
            }
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)?;
            let mut head = [0u8; LOG_HEADER.len()];
            let mut read = 0;
            while read < head.len() {
                match file.read(&mut head[read..])? {
                    0 => break,
                    n => read += n,
                }
            }
            if head[..read] != *LOG_HEADER {
                return Ok(false);
            }
            file.set_len(0)?;
            file.seek(std::io::SeekFrom::Start(0))?;
            file
        }
        Err(e) => return Err(e),
    };
    file.write_all(body)?;
    Ok(true)
}

#[cfg(windows)]
fn dialog(caption: &str, text: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, caption) = (wide(text), wide(caption));
    // SAFETY: both buffers are NUL-terminated UTF-16 that outlive the call.
    unsafe {
        MessageBoxW(
            HWND::default(),
            PCWSTR::from_raw(text.as_ptr()),
            PCWSTR::from_raw(caption.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn dialog(_caption: &str, _text: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(dir: &Path, json: &str) -> Result<Game> {
        std::fs::write(dir.join(GAME_FILE), json).unwrap();
        read_game(dir)
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("axiom-player-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("content")).unwrap();
        dir
    }

    #[test]
    fn reads_game_file_and_rejects_unsafe_fields() {
        let dir = scratch("game");
        let ok = game(
            &dir,
            r#"{"format":1,"id":"adv-demo","title":"Last Signal","pack":"content","exe":"x.exe"}"#,
        )
        .unwrap();
        assert_eq!(ok.id, "adv-demo");
        assert_eq!(ok.title, "Last Signal");
        assert_eq!(ok.pack, dir.join("content").canonicalize().unwrap());
        for bad in [
            r#"{"format":2,"id":"a","title":"t","pack":"content"}"#,
            r#"{"format":1,"id":"../x","title":"t","pack":"content"}"#,
            r#"{"format":1,"id":"a","title":"","pack":"content"}"#,
            r#"{"format":1,"id":"a","title":"t","pack":"../content"}"#,
            r#"{"format":1,"id":"a","title":"t","pack":"C:\content"}"#,
            r#"{"format":1,"id":"a","title":"t","pack":"missing"}"#,
            "{not json",
        ] {
            assert!(game(&dir, bad).is_err(), "{bad}");
        }
        std::fs::write(dir.join(GAME_FILE), vec![b' '; 70_000]).unwrap();
        assert!(read_game(&dir).is_err(), "over 64 KiB");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // R9 F4: a `content` junction that leads outside the game folder is refused.
    #[cfg(windows)]
    #[test]
    fn pack_junction_outside_the_game_folder_is_refused() {
        let dir = scratch("junction");
        let outside =
            std::env::temp_dir().join(format!("axiom-player-outside-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        let link = dir.join("linked");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        let error = game(&dir, r#"{"format":1,"id":"a","title":"t","pack":"linked"}"#)
            .err()
            .expect("junction leaving the folder must be refused");
        assert!(
            error.to_string().contains("outside the game folder"),
            "{error:#}"
        );
        std::fs::remove_dir(&link).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    // R9 F15: `--db` works without APPDATA, as the error message promises.
    #[test]
    fn explicit_db_does_not_need_appdata() {
        let dir = scratch("db");
        let db = dir.join("saves/world.sqlite");
        let (chosen, log) = locations(Some(db.clone()), "adv-demo", None).unwrap();
        assert_eq!(chosen, db);
        assert_eq!(log, dir.join("saves/crash.log"));
        let error = locations(None, "adv-demo", None).unwrap_err();
        assert!(error.to_string().contains("--db"), "{error}");
        let (chosen, log) =
            locations(None, "adv-demo", Some(dir.clone().into_os_string())).unwrap();
        assert_eq!(chosen, dir.join("Axiom/adv-demo/world.sqlite"));
        assert_eq!(log, dir.join("Axiom/adv-demo/crash.log"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // R9 N1: a `--db` named crash.log is never overwritten by the crash log, and the log never
    // replaces any existing file that is not an earlier crash log.
    #[test]
    fn crash_log_never_overwrites_the_database() {
        let dir = scratch("n1");
        let sqlite = b"SQLite format 3\0 pretend pages".to_vec();
        let db = dir.join("crash.log");
        std::fs::write(&db, &sqlite).unwrap();
        let (chosen, log) = locations(Some(db.clone()), "adv-demo", None).unwrap();
        assert_eq!(chosen, db);
        assert_ne!(log, db, "the crash log must not share the database's name");
        let upper = dir.join("CRASH.LOG");
        let (_, log_upper) = locations(Some(upper.clone()), "adv-demo", None).unwrap();
        assert_ne!(
            log_upper.file_name().unwrap().to_ascii_lowercase(),
            upper.file_name().unwrap().to_ascii_lowercase()
        );
        let body = b"axiom-player 0.0.0\r\nerror: boom\r\n";
        // Even pointed straight at the database, the log goes elsewhere.
        let written = write_log(&db, None, body).unwrap();
        assert_ne!(written, db);
        assert_eq!(
            std::fs::read(&db).unwrap(),
            sqlite,
            "database bytes changed"
        );
        assert_eq!(std::fs::read(&written).unwrap(), body);
        // An empty foreign file is not a crash log either.
        let empty = dir.join("empty.log");
        std::fs::write(&empty, b"").unwrap();
        assert_ne!(write_log(&empty, None, body).unwrap(), empty);
        assert!(std::fs::read(&empty).unwrap().is_empty());
        // An earlier crash log is replaced in place.
        let old = dir.join("player-crash.log");
        std::fs::write(
            &old,
            b"axiom-player 0.0.0\r\nerror: an older and much longer message\r\n",
        )
        .unwrap();
        assert_eq!(write_log(&old, None, body).unwrap(), old);
        assert_eq!(std::fs::read(&old).unwrap(), body);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // R9 round 4 N3: a `--db` that does not exist yet is never taken by a fallback crash-log
    // name, so the same command works once the failure is fixed.
    #[test]
    fn fallback_crash_log_never_takes_a_database_that_does_not_exist_yet() {
        let dir = scratch("n3");
        std::fs::write(dir.join("crash.log"), b"my own notes").unwrap();
        for name in ["crash-1.log", "CRASH-1.LOG"] {
            let db = dir.join(name);
            let (_, log) = locations(Some(db.clone()), "adv-demo", None).unwrap();
            assert_eq!(log, dir.join("crash.log"));
            let body = b"axiom-player 0.0.0\r\nerror: pack failed to load\r\n";
            let written = write_log(&log, Some(&db), body).unwrap();
            assert!(
                !written
                    .file_name()
                    .unwrap()
                    .eq_ignore_ascii_case(db.file_name().unwrap()),
                "{written:?} took the database name"
            );
            assert!(
                !db.exists(),
                "the database path was created by the crash log"
            );
            assert_eq!(std::fs::read(&written).unwrap(), body);
            assert_eq!(
                std::fs::read(dir.join("crash.log")).unwrap(),
                b"my own notes"
            );
            // The retry with the same --db can create its database.
            drop(axiom_core::Store::open(&db).expect("retry opens the database"));
            for sidecar in ["", "-wal", "-shm"] {
                let mut p = db.clone().into_os_string();
                p.push(sidecar);
                let _ = std::fs::remove_file(p);
            }
            std::fs::remove_file(&written).unwrap();
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_automation_flags() {
        let args = parse_args(
            [
                "--smoke-frames",
                "120",
                "--screenshot",
                "a.png",
                "--port",
                "0",
            ]
            .into_iter()
            .map(Into::into),
        )
        .unwrap();
        assert_eq!(args.smoke_frames, Some(120));
        assert!(parse_args(["--screenshot", "a.png"].into_iter().map(Into::into)).is_err());
        assert!(parse_args(["--bogus"].into_iter().map(Into::into)).is_err());
        assert!(parse_args(["--smoke-frames", "0"].into_iter().map(Into::into)).is_err());
    }
}
