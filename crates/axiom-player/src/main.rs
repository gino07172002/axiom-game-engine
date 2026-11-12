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
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const GAME_FILE: &str = axiom_runtime::packaging::GAME_FILE;
const NO_DIALOG: &str = "AXIOM_PLAYER_NO_DIALOG";
const MAX_GAME_FILE: u64 = 64 * 1024;

/// Where a failure is reported: `%APPDATA%\Axiom\<id>\crash.log` once the id is known.
static CRASH_LOG: Mutex<Option<PathBuf>> = Mutex::new(None);

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
    let data = data_dir(&game.id)?;
    *CRASH_LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(data.join("crash.log"));
    let db = match args.db {
        Some(db) => db,
        None => {
            std::fs::create_dir_all(&data)
                .with_context(|| format!("create save directory {}", data.display()))?;
            data.join("world.sqlite")
        }
    };
    if let Some(parent) = db.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create database directory {}", parent.display()))?;
    }
    let registry = axiom_runtime::installed_registry();
    let world = axiom_core::ContentPackage::load(&game.pack, &registry.authoring)
        .with_context(|| format!("load game content {}", game.pack.display()))?
        .world;
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
    let path = dir.join(GAME_FILE);
    let size = std::fs::metadata(&path)
        .with_context(|| format!("{} not found beside the player", path.display()))?
        .len();
    ensure!(size <= MAX_GAME_FILE, "{} exceeds 64 KiB", path.display());
    let raw = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_slice(&raw).with_context(|| format!("parse {}", path.display()))?;
    let text = |key: &str| {
        value[key]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
            .with_context(|| format!("{GAME_FILE}: \"{key}\" must be a non-empty string"))
    };
    let format = value["format"].as_u64();
    ensure!(
        format == Some(axiom_runtime::packaging::FORMAT as u64),
        "{GAME_FILE}: unsupported format {:?} (this player reads format {})",
        value["format"],
        axiom_runtime::packaging::FORMAT
    );
    let id = text("id")?;
    // The id names the save directory: one plain path component.
    ensure!(
        id.len() <= 128
            && id != "."
            && id != ".."
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
        "{GAME_FILE}: id {id:?} must use only letters, digits, '-', '_' or '.'"
    );
    let title = text("title")?;
    let pack = PathBuf::from(text("pack")?);
    ensure!(
        pack.components()
            .all(|c| matches!(c, std::path::Component::Normal(_))),
        "{GAME_FILE}: pack must be a relative path inside the game folder"
    );
    Ok(Game {
        id,
        title,
        pack: dir.join(pack),
    })
}

fn data_dir(id: &str) -> Result<PathBuf> {
    let appdata = std::env::var_os("APPDATA")
        .filter(|v| !v.is_empty())
        .context("APPDATA is not set; cannot choose a save directory (pass --db PATH)")?;
    Ok(PathBuf::from(appdata).join("Axiom").join(id))
}

/// Writes the crash log (best effort) and tells the person, since there is no console.
fn report(message: &str) {
    let log = CRASH_LOG
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| std::env::temp_dir().join("axiom-player-crash.log"));
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
        .and_then(|_| std::fs::write(&log, body))
        .is_ok();
    eprintln!("axiom-player: {message}");
    if std::env::var_os(NO_DIALOG).is_some_and(|v| v == "1") {
        return;
    }
    let detail = if written {
        format!("{message}\n\nDetails were written to:\n{}", log.display())
    } else {
        message.to_string()
    };
    dialog("The game could not start", &detail);
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

    #[test]
    fn reads_game_file_and_rejects_unsafe_fields() {
        let dir = std::env::temp_dir().join(format!("axiom-player-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = game(
            &dir,
            r#"{"format":1,"id":"adv-demo","title":"Last Signal","pack":"content","exe":"x.exe"}"#,
        )
        .unwrap();
        assert_eq!(ok.id, "adv-demo");
        assert_eq!(ok.title, "Last Signal");
        assert_eq!(ok.pack, dir.join("content"));
        for bad in [
            r#"{"format":2,"id":"a","title":"t","pack":"content"}"#,
            r#"{"format":1,"id":"../x","title":"t","pack":"content"}"#,
            r#"{"format":1,"id":"a","title":"","pack":"content"}"#,
            r#"{"format":1,"id":"a","title":"t","pack":"../content"}"#,
            r#"{"format":1,"id":"a","title":"t","pack":"C:\\content"}"#,
            "{not json",
        ] {
            assert!(game(&dir, bad).is_err(), "{bad}");
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
