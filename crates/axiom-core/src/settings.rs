//! Player settings (docs/plan-player-settings.zh-TW.md): a fixed set of engine keys with
//! typed values and ranges. They are the player's preferences, not content and not a save:
//! the host keeps them in its own settings file, apart from authoring revisions and
//! checkpoints. Authors only choose defaults (`game.settings` resource).
use anyhow::{bail, ensure, Result};
use serde_json::{json, Value};

/// How the game window is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplayMode {
    #[default]
    Window,
    Borderless,
    Fullscreen,
}
impl DisplayMode {
    pub const ALL: [(&'static str, DisplayMode); 3] = [
        ("window", DisplayMode::Window),
        ("borderless", DisplayMode::Borderless),
        ("fullscreen", DisplayMode::Fullscreen),
    ];
    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, m)| *m == self)
            .map_or("window", |(n, _)| n)
    }
}

/// Sun shadow map size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShadowQuality {
    Low,
    #[default]
    Medium,
    High,
}
impl ShadowQuality {
    pub const ALL: [(&'static str, ShadowQuality); 3] = [
        ("low", ShadowQuality::Low),
        ("medium", ShadowQuality::Medium),
        ("high", ShadowQuality::High),
    ];
    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, q)| *q == self)
            .map_or("medium", |(n, _)| n)
    }
    /// Shadow map edge in texels.
    pub fn texels(self) -> u32 {
        match self {
            ShadowQuality::Low => 1024,
            ShadowQuality::Medium => 2048,
            ShadowQuality::High => 4096,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerSettings {
    pub master: f32,
    pub music: f32,
    pub sfx: f32,
    pub sensitivity: f32,
    pub invert_y: bool,
    pub display: DisplayMode,
    pub vsync: bool,
    pub shadows: ShadowQuality,
    pub fxaa: bool,
}
impl Default for PlayerSettings {
    fn default() -> Self {
        Self {
            master: 1.0,
            music: 1.0,
            sfx: 1.0,
            sensitivity: 1.0,
            invert_y: false,
            display: DisplayMode::Window,
            vsync: true,
            shadows: ShadowQuality::Medium,
            fxaa: true,
        }
    }
}

/// Every key, its kind and (for numbers) its range.
pub const KEYS: [(&str, &str); 9] = [
    ("audio.master", "number 0..1"),
    ("audio.music", "number 0..1"),
    ("audio.sfx", "number 0..1"),
    ("mouse.sensitivity", "number 0.1..5"),
    ("mouse.invert_y", "bool"),
    ("display.mode", "window|borderless|fullscreen"),
    ("display.vsync", "bool"),
    ("quality.shadows", "low|medium|high"),
    ("quality.fxaa", "bool"),
];

/// The range of a number key.
pub fn range(key: &str) -> Option<(f64, f64)> {
    match key {
        "audio.master" | "audio.music" | "audio.sfx" => Some((0.0, 1.0)),
        "mouse.sensitivity" => Some((0.1, 5.0)),
        _ => None,
    }
}

fn unknown(key: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "unknown setting '{key}' (settings: {})",
        KEYS.map(|(k, _)| k).join(", ")
    )
}

impl PlayerSettings {
    pub fn get(&self, key: &str) -> Result<Value> {
        // Numbers are f32 inside; reported to 4 decimals (0.35, not 0.3499999940395355).
        let n = |v: f32| json!((v as f64 * 10_000.0).round() / 10_000.0);
        Ok(match key {
            "audio.master" => n(self.master),
            "audio.music" => n(self.music),
            "audio.sfx" => n(self.sfx),
            "mouse.sensitivity" => n(self.sensitivity),
            "mouse.invert_y" => json!(self.invert_y),
            "display.mode" => json!(self.display.name()),
            "display.vsync" => json!(self.vsync),
            "quality.shadows" => json!(self.shadows.name()),
            "quality.fxaa" => json!(self.fxaa),
            _ => return Err(unknown(key)),
        })
    }

    /// A number key's value (sliders).
    pub fn number(&self, key: &str) -> Option<f64> {
        self.get(key).ok()?.as_f64()
    }

    /// Sets `key` after checking the value's type and range.
    pub fn set(&mut self, key: &str, value: &Value) -> Result<()> {
        let number = |v: &Value| -> Result<f32> {
            let (lo, hi) = range(key).expect("number key");
            let n = v
                .as_f64()
                .ok_or_else(|| anyhow::anyhow!("{key} is a number {lo}..{hi}"))?;
            ensure!(
                n.is_finite() && (lo..=hi).contains(&n),
                "{key} is a number {lo}..{hi} (got {n})"
            );
            Ok(n as f32)
        };
        let flag = |v: &Value| -> Result<bool> {
            v.as_bool()
                .ok_or_else(|| anyhow::anyhow!("{key} is true or false"))
        };
        fn pick<T: Copy>(key: &str, v: &Value, all: &[(&str, T)]) -> Result<T> {
            let name = v.as_str().unwrap_or_default();
            match all.iter().find(|(n, _)| *n == name) {
                Some((_, x)) => Ok(*x),
                None => bail!(
                    "{key} is one of {}",
                    all.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
                ),
            }
        }
        match key {
            "audio.master" => self.master = number(value)?,
            "audio.music" => self.music = number(value)?,
            "audio.sfx" => self.sfx = number(value)?,
            "mouse.sensitivity" => self.sensitivity = number(value)?,
            "mouse.invert_y" => self.invert_y = flag(value)?,
            "display.mode" => self.display = pick(key, value, &DisplayMode::ALL)?,
            "display.vsync" => self.vsync = flag(value)?,
            "quality.shadows" => self.shadows = pick(key, value, &ShadowQuality::ALL)?,
            "quality.fxaa" => self.fxaa = flag(value)?,
            _ => return Err(unknown(key)),
        }
        Ok(())
    }

    /// Every key and value (the settings file and `observe settings`).
    pub fn to_json(&self) -> Value {
        Value::Object(
            KEYS.iter()
                .map(|(k, _)| ((*k).to_owned(), self.get(k).expect("known key")))
                .collect(),
        )
    }

    /// `base` with the keys of `value` applied. Lenient (a player's file): an unknown key or
    /// a bad value keeps the base value and is reported, so a damaged file never stops the game.
    pub fn merged(base: PlayerSettings, value: &Value) -> (PlayerSettings, Vec<String>) {
        let mut out = base;
        let mut problems = vec![];
        match value.as_object() {
            Some(map) => {
                for (k, v) in map {
                    if let Err(e) = out.set(k, v) {
                        problems.push(format!("{k}: {e}"));
                    }
                }
            }
            None => problems.push("settings must be a JSON object".into()),
        }
        (out, problems)
    }
}

/// A default value token (ACL `settings default KEY VALUE` and the editor command): `none`
/// removes the default; JSON literals (`0.8`, `true`) and bare names (`fullscreen`) set it.
pub fn parse_token(token: &str) -> Option<Value> {
    match token {
        "none" => None,
        t => Some(serde_json::from_str(t).unwrap_or_else(|_| Value::String(t.to_owned()))),
    }
}

/// The `game.settings` resource: author defaults, `{"defaults": {KEY: VALUE}}`. Strict.
pub fn defaults(resource: Option<&Value>) -> Result<PlayerSettings> {
    let mut s = PlayerSettings::default();
    let Some(resource) = resource else {
        return Ok(s);
    };
    let object = resource
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("game.settings must be an object"))?;
    for k in object.keys() {
        ensure!(
            k == "defaults",
            "unknown game.settings field '{k}' (defaults)"
        );
    }
    if let Some(d) = object.get("defaults") {
        let map = d
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("game.settings.defaults must be an object"))?;
        for (k, v) in map {
            crate::local::at(
                &format!("/resources/game.settings/defaults/{k}"),
                s.set(k, v),
            )?;
        }
    }
    Ok(s)
}
