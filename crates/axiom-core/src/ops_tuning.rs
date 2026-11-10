//! Operations of the tuning editors (re-exported from [`crate::ops`]): the input map
//! (`input.bindings`), the localization table (`ui.document` catalogs), ability timeline
//! events (`action.abilities`), VFX size curves and colour gradients (`vfx.library`) and custom
//! shader source (`render.shaders`). Pure like the rest of `ops`: `fn(&World, ..) ->
//! Result<Vec<Edit>>`, shared by the editor tabs and the ACL `input` / `loc` / `ability` /
//! `vfx` / `shader` verbs. An edit that changes nothing returns no edits.
use crate::local::{escape, Target};
use crate::*;
use anyhow::Result;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const INPUT_BINDINGS: &str = "input.bindings";
/// Ability library of the action capability (`axiom-action`; edited here as JSON).
pub const ABILITIES: &str = "action.abilities";
pub const VFX_LIBRARY: &str = "vfx.library";
pub const SHADER_LIBRARY: &str = "render.shaders";
/// Budgets of the registered validators, checked here for clearer messages.
pub const MAX_INPUT_ACTIONS: usize = 64;
pub const MAX_INPUT_NAME: usize = 64;
pub const MAX_LOCALES: usize = 16;
pub const MAX_LOCALIZATION_KEYS: usize = 2048;
pub const MAX_LOCALIZED_TEXT: usize = 8192;
pub const MAX_ABILITY_EVENTS: usize = 16;
pub const MAX_SHADER_SOURCE: usize = 16 * 1024;
/// The pointer button the runtime adds to the pressed keys (`app.rs`).
pub const MOUSE_LEFT: &str = "MouseLeft";

fn invalid(path: impl Into<String>, message: impl Into<String>) -> anyhow::Error {
    Diagnostic::error("invalid_value", path, message)
}

fn resource<'a>(world: &'a World, id: &str, hint: &str) -> Result<&'a Value> {
    world.resources.get(id).ok_or_else(|| {
        Diagnostic::error(
            "not_found",
            format!("/resources/{id}"),
            format!("the world has no {id} ({hint})"),
        )
    })
}

fn put(id: &str, value: Value) -> Vec<Edit> {
    vec![Edit::Core(CoreEdit::SetResource {
        id: id.into(),
        value,
    })]
}

fn set_field(id: &str, field: String, value: Value) -> Edit {
    Edit::Core(CoreEdit::SetField {
        target: Target::Resource { id: id.into() },
        field,
        value,
    })
}

/// 4 decimals: stable diffs for values that come from drags.
fn round(x: f64) -> f64 {
    (x * 10000.0).round() / 10000.0
}

// ---------------------------------------------------------------------------------------
// Input map

/// One change to `input.bindings`.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEdit {
    /// Binds (or rebinds) a button action to one key.
    Button { action: String, key: String },
    /// Binds (or rebinds) an axis action to its positive and negative keys.
    Axis {
        action: String,
        positive: String,
        negative: String,
    },
    /// Removes an action (references elsewhere then fail validation with their path).
    Remove { action: String },
}

/// A key name as the runtime matches it: winit `KeyCode` names (`KeyW`, `Space`, `Digit1`,
/// `ShiftLeft`, `ArrowUp`, ..) and `MouseLeft`. Only the shape is checked here.
pub fn valid_key_name(key: &str) -> bool {
    key.len() > 1 && key.len() <= MAX_INPUT_NAME && key.chars().all(|c| c.is_ascii_alphanumeric())
}

/// What a key name probably meant (`W` → `KeyW`, `1` → `Digit1`).
fn key_hint(key: &str) -> String {
    match key.chars().next() {
        Some(c) if key.len() == 1 && c.is_ascii_alphabetic() => {
            format!(" (did you mean Key{}?)", c.to_ascii_uppercase())
        }
        Some(c) if key.len() == 1 && c.is_ascii_digit() => format!(" (did you mean Digit{c}?)"),
        _ => String::new(),
    }
}

pub fn edit_input(world: &World, edit: &InputEdit) -> Result<Vec<Edit>> {
    let raw = resource(
        world,
        INPUT_BINDINGS,
        "asset create input, or resource create input.bindings",
    )?;
    let mut bindings: InputBindings = serde_json::from_value(raw.clone()).map_err(|e| {
        invalid(
            "/resources/input.bindings",
            format!("input.bindings does not parse: {e}"),
        )
    })?;
    let action = match edit {
        InputEdit::Button { action, .. }
        | InputEdit::Axis { action, .. }
        | InputEdit::Remove { action } => action,
    };
    let path = |map: &str| format!("/resources/input.bindings/{map}/{}", escape(action));
    let key_ok = |key: &str, map: &str| -> Result<()> {
        if valid_key_name(key) {
            Ok(())
        } else {
            Err(invalid(
                path(map),
                format!(
                    "'{key}' is not a key name{} (use KeyW, Space, Digit1, ShiftLeft, ArrowUp, MouseLeft ..)",
                    key_hint(key)
                ),
            ))
        }
    };
    let name_ok = || -> Result<()> {
        if action.len() <= MAX_INPUT_NAME && valid_id(action).is_ok() {
            Ok(())
        } else {
            Err(invalid(
                path("buttons"),
                format!("'{action}' is not an action name (letters, digits, . _ / -, at most {MAX_INPUT_NAME})"),
            ))
        }
    };
    let added = !bindings.buttons.contains_key(action) && !bindings.axes.contains_key(action);
    match edit {
        InputEdit::Button { key, .. } => {
            name_ok()?;
            key_ok(key, "buttons")?;
            if bindings.axes.contains_key(action) {
                return Err(Diagnostic::error(
                    "action_kind",
                    path("axes"),
                    format!("{action} is an axis; remove it first (input remove {action}) to make it a button"),
                ));
            }
            bindings.buttons.insert(action.clone(), key.clone());
        }
        InputEdit::Axis {
            positive, negative, ..
        } => {
            name_ok()?;
            key_ok(positive, "axes")?;
            key_ok(negative, "axes")?;
            if bindings.buttons.contains_key(action) {
                return Err(Diagnostic::error(
                    "action_kind",
                    path("buttons"),
                    format!("{action} is a button; remove it first (input remove {action}) to make it an axis"),
                ));
            }
            bindings.axes.insert(
                action.clone(),
                AxisBinding {
                    positive: positive.clone(),
                    negative: negative.clone(),
                },
            );
        }
        InputEdit::Remove { .. } => {
            if bindings.buttons.remove(action).is_none() && bindings.axes.remove(action).is_none() {
                return Err(Diagnostic::error(
                    "not_found",
                    path("buttons"),
                    format!("input.bindings has no action {action}"),
                ));
            }
        }
    }
    if added && bindings.buttons.len() + bindings.axes.len() > MAX_INPUT_ACTIONS {
        return Err(Diagnostic::error(
            "budget_exceeded",
            "/resources/input.bindings",
            format!("input.bindings holds at most {MAX_INPUT_ACTIONS} actions"),
        ));
    }
    let before: InputBindings = serde_json::from_value(raw.clone()).expect("parsed above");
    if before == bindings {
        return Ok(vec![]);
    }
    let mut out = raw.clone();
    out["buttons"] = json!(bindings.buttons);
    out["axes"] = json!(bindings.axes);
    if out == *raw {
        return Ok(vec![]);
    }
    Ok(put(INPUT_BINDINGS, out))
}

/// Keys bound to more than one action (an axis half counts as `ACTION+` / `ACTION-`), in key
/// order. The runtime allows it (every action sees the key), so this is a warning.
pub fn input_conflicts(bindings: &InputBindings) -> Vec<(String, Vec<String>)> {
    let mut by_key: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (action, key) in &bindings.buttons {
        by_key.entry(key).or_default().push(action.clone());
    }
    for (action, axis) in &bindings.axes {
        by_key
            .entry(&axis.positive)
            .or_default()
            .push(format!("{action}+"));
        by_key
            .entry(&axis.negative)
            .or_default()
            .push(format!("{action}-"));
    }
    by_key
        .into_iter()
        .filter(|(_, actions)| actions.len() > 1)
        .map(|(key, actions)| (key.to_owned(), actions))
        .collect()
}

// ---------------------------------------------------------------------------------------
// Localization

/// One change to the `ui.document` catalogs (every locale keeps the same key set).
#[derive(Clone, Debug, PartialEq)]
pub enum LocEdit {
    Set {
        locale: String,
        key: String,
        text: String,
    },
    /// New key: `text` in the default locale, empty (= missing) in the others.
    AddKey {
        key: String,
        text: String,
    },
    RemoveKey {
        key: String,
    },
    /// New locale with every key: copied from `from`, else empty (missing).
    AddLocale {
        locale: String,
        from: Option<String>,
    },
    /// Removes a locale other than the default one.
    RemoveLocale {
        locale: String,
    },
}

/// Why a localized text needs attention, if it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocIssue {
    /// Empty, or still the key itself.
    Missing,
    /// Same text as the default locale (in another locale).
    Untranslated,
}

/// The editor's and the UI Editor's warning rule for one cell.
pub fn loc_issue(
    key: &str,
    text: &str,
    default_text: Option<&str>,
    is_default: bool,
) -> Option<LocIssue> {
    if text.trim().is_empty() || text == key {
        Some(LocIssue::Missing)
    } else if !is_default && default_text == Some(text) {
        Some(LocIssue::Untranslated)
    } else {
        None
    }
}

fn valid_loc_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || "._-".contains(c))
}

pub fn edit_localization(world: &World, edit: &LocEdit) -> Result<Vec<Edit>> {
    const DOC: &str = "/resources/ui.document";
    let raw = resource(
        world,
        crate::ops::UI_DOCUMENT,
        "asset create ui, or resource create ui.document",
    )?;
    let default = raw["locale"].as_str().unwrap_or_default().to_owned();
    let catalogs = raw
        .get("catalogs")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid(format!("{DOC}/catalogs"), "ui.document has no catalogs"))?;
    let locale_path = |l: &str| format!("{DOC}/catalogs/{}", escape(l));
    let no_locale = |l: &str| {
        Diagnostic::error(
            "not_found",
            locale_path(l),
            format!(
                "ui.document has no locale {l} (locales: {})",
                catalogs.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
        )
    };
    let keys: Vec<String> = catalogs
        .get(&default)
        .and_then(Value::as_object)
        .map(|c| c.keys().cloned().collect())
        .unwrap_or_default();
    let text_ok = |text: &str, path: String| -> Result<()> {
        if text.len() <= MAX_LOCALIZED_TEXT {
            Ok(())
        } else {
            Err(invalid(
                path,
                format!("localized text is at most {MAX_LOCALIZED_TEXT} bytes"),
            ))
        }
    };
    let mut out = raw.clone();
    let cats = out["catalogs"]
        .as_object_mut()
        .expect("catalogs checked above");
    match edit {
        LocEdit::Set { locale, key, text } => {
            let path = format!("{}/{}", locale_path(locale), escape(key));
            let catalog = catalogs.get(locale).ok_or_else(|| no_locale(locale))?;
            let Some(before) = catalog.get(key) else {
                return Err(Diagnostic::error(
                    "not_found",
                    path,
                    format!("no localization key {key} (add it: loc key add {key})"),
                ));
            };
            text_ok(text, path)?;
            if before.as_str() == Some(text.as_str()) {
                return Ok(vec![]);
            }
            return Ok(vec![set_field(
                crate::ops::UI_DOCUMENT,
                format!("/catalogs/{}/{}", escape(locale), escape(key)),
                json!(text),
            )]);
        }
        LocEdit::AddKey { key, text } => {
            let path = format!("{}/{}", locale_path(&default), escape(key));
            if !valid_loc_key(key) {
                return Err(invalid(
                    path,
                    format!("'{key}' is not a localization key (letters, digits, . _ -)"),
                ));
            }
            if keys.iter().any(|k| k == key) {
                return Err(Diagnostic::error(
                    "id_taken",
                    path,
                    format!("localization key {key} already exists"),
                ));
            }
            if keys.len() >= MAX_LOCALIZATION_KEYS {
                return Err(Diagnostic::error(
                    "budget_exceeded",
                    path,
                    format!("ui.document holds at most {MAX_LOCALIZATION_KEYS} keys"),
                ));
            }
            text_ok(text, path)?;
            for (locale, catalog) in cats.iter_mut() {
                let value = if *locale == default {
                    text.as_str()
                } else {
                    ""
                };
                if let Some(c) = catalog.as_object_mut() {
                    c.insert(key.clone(), json!(value));
                }
            }
        }
        LocEdit::RemoveKey { key } => {
            if !keys.iter().any(|k| k == key) {
                return Err(Diagnostic::error(
                    "not_found",
                    format!("{}/{}", locale_path(&default), escape(key)),
                    format!("no localization key {key}"),
                ));
            }
            for catalog in cats.values_mut() {
                if let Some(c) = catalog.as_object_mut() {
                    c.remove(key);
                }
            }
        }
        LocEdit::AddLocale { locale, from } => {
            let path = locale_path(locale);
            if valid_id(locale).is_err() || locale.contains('/') {
                return Err(invalid(
                    path,
                    format!("'{locale}' is not a locale ID (e.g. en, zh-TW, ja)"),
                ));
            }
            if catalogs.contains_key(locale) {
                return Err(Diagnostic::error(
                    "id_taken",
                    path,
                    format!("locale {locale} already exists"),
                ));
            }
            if catalogs.len() >= MAX_LOCALES {
                return Err(Diagnostic::error(
                    "budget_exceeded",
                    path,
                    format!("ui.document holds at most {MAX_LOCALES} locales"),
                ));
            }
            let source = match from {
                Some(f) => Some(catalogs.get(f).ok_or_else(|| no_locale(f))?),
                None => None,
            };
            let catalog: Map<String, Value> = keys
                .iter()
                .map(|k| {
                    let text = source.and_then(|s| s.get(k)).cloned().unwrap_or(json!(""));
                    (k.clone(), text)
                })
                .collect();
            cats.insert(locale.clone(), Value::Object(catalog));
        }
        LocEdit::RemoveLocale { locale } => {
            if !catalogs.contains_key(locale) {
                return Err(no_locale(locale));
            }
            if *locale == default {
                return Err(Diagnostic::error(
                    "default_locale",
                    locale_path(locale),
                    format!("{locale} is the default locale (set /locale to another one first)"),
                ));
            }
            cats.remove(locale);
        }
    }
    Ok(put(crate::ops::UI_DOCUMENT, out))
}

// ---------------------------------------------------------------------------------------
// Ability timeline events

/// One change to an ability's timeline `events` (`[{tick, sound}]`, ticks of 1/120 s).
#[derive(Clone, Debug, PartialEq)]
pub enum AbilityEventEdit {
    Add { tick: u32, sound: String },
    Move { index: usize, tick: u32 },
    Sound { index: usize, sound: String },
    Remove { index: usize },
}

pub fn edit_ability_event(
    world: &World,
    ability: &str,
    edit: &AbilityEventEdit,
) -> Result<Vec<Edit>> {
    let raw = resource(world, ABILITIES, "an action pack declares abilities")?;
    let path = format!("/resources/{ABILITIES}/abilities/{}", escape(ability));
    let a = raw
        .get("abilities")
        .and_then(|m| m.get(ability))
        .ok_or_else(|| {
            Diagnostic::error(
                "not_found",
                &path,
                format!(
                    "no ability {ability} (abilities: {})",
                    raw["abilities"]
                        .as_object()
                        .map(|m| m.keys().cloned().collect::<Vec<_>>().join(", "))
                        .unwrap_or_default()
                ),
            )
        })?;
    let duration = a["duration"].as_u64().unwrap_or(0);
    let before = a.get("events").cloned().unwrap_or(json!([]));
    let mut events = before.as_array().cloned().unwrap_or_default();
    let events_path = format!("{path}/events");
    let tick_ok = |tick: u32| -> Result<()> {
        if u64::from(tick) < duration {
            Ok(())
        } else {
            Err(invalid(
                &events_path,
                format!("tick {tick} is outside the ability (0..{duration})"),
            ))
        }
    };
    let sound_ok = |sound: &str| -> Result<()> {
        let known = world
            .resources
            .get("audio.library")
            .and_then(|l| l.get("cues"))
            .and_then(Value::as_object)
            .is_some_and(|c| c.contains_key(sound));
        if known {
            Ok(())
        } else {
            Err(Diagnostic::error(
                "missing_cue",
                &events_path,
                format!("audio.library has no cue {sound}"),
            ))
        }
    };
    let index_ok = |index: usize, len: usize| -> Result<()> {
        if index < len {
            Ok(())
        } else {
            Err(Diagnostic::error(
                "not_found",
                format!("{events_path}/{index}"),
                format!("ability {ability} has {len} event(s)"),
            ))
        }
    };
    match edit {
        AbilityEventEdit::Add { tick, sound } => {
            tick_ok(*tick)?;
            sound_ok(sound)?;
            if events.len() >= MAX_ABILITY_EVENTS {
                return Err(Diagnostic::error(
                    "budget_exceeded",
                    &events_path,
                    format!("an ability holds at most {MAX_ABILITY_EVENTS} events"),
                ));
            }
            events.push(json!({"tick": tick, "sound": sound}));
        }
        AbilityEventEdit::Move { index, tick } => {
            index_ok(*index, events.len())?;
            tick_ok(*tick)?;
            events[*index]["tick"] = json!(tick);
        }
        AbilityEventEdit::Sound { index, sound } => {
            index_ok(*index, events.len())?;
            sound_ok(sound)?;
            events[*index]["sound"] = json!(sound);
        }
        AbilityEventEdit::Remove { index } => {
            index_ok(*index, events.len())?;
            events.remove(*index);
        }
    }
    let after = Value::Array(events);
    if after == before {
        return Ok(vec![]);
    }
    Ok(vec![set_field(
        ABILITIES,
        format!("/abilities/{}/events", escape(ability)),
        after,
    )])
}

// ---------------------------------------------------------------------------------------
// VFX curves and gradients

/// Keys of an emitter's `size_curve` / stops of its `color_gradient` (the runtime's
/// `vfx::MAX_CURVE_KEYS`, sized for the GPU uniform).
pub const MAX_VFX_CURVE_KEYS: usize = 8;

/// The emitter parameters the VFX tab edits: size over life (linear from `size` to
/// `end_size`, or the optional N-key `size_curve`) and colour over life (a two-stop gradient
/// from `color` to `end_color`, or the optional N-stop `color_gradient`). Curve edits replace
/// the whole list (the tab's add / remove / drag of one key is one such edit); an empty list
/// removes the field, back to the two-point values.
#[derive(Clone, Debug, PartialEq)]
pub enum VfxEdit {
    Size {
        start: f64,
        end: f64,
    },
    Color {
        start: [f64; 4],
        end: [f64; 4],
    },
    /// `[t, size]` keys.
    SizeCurve {
        keys: Vec<[f64; 2]>,
    },
    /// `(t, rgba)` stops.
    ColorGradient {
        stops: Vec<(f64, [f64; 4])>,
    },
}

/// Curve times: 2..=MAX keys, finite, 0..1, strictly increasing.
fn curve_times_ok(times: &[f64]) -> bool {
    (2..=MAX_VFX_CURVE_KEYS).contains(&times.len())
        && times
            .iter()
            .all(|t| t.is_finite() && (0.0..=1.0).contains(t))
        && times.windows(2).all(|w| w[0] < w[1])
}

pub fn edit_vfx(world: &World, emitter: &str, edit: &VfxEdit) -> Result<Vec<Edit>> {
    let raw = resource(world, VFX_LIBRARY, "resource create vfx.library")?;
    let path = format!("/resources/{VFX_LIBRARY}/emitters/{}", escape(emitter));
    let e = raw
        .get("emitters")
        .and_then(|m| m.get(emitter))
        .ok_or_else(|| Diagnostic::error("not_found", &path, format!("no emitter {emitter}")))?;
    let base = format!("/emitters/{}", escape(emitter));
    let mut edits = vec![];
    // Numeric comparison: an authored `1` equals `1.0` (JSON values would differ).
    let same = |a: Option<&Value>, b: &Value| match (a, b) {
        (Some(Value::Array(x)), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| x.as_f64() == y.as_f64())
        }
        (Some(x), y) => x.as_f64().is_some() && x.as_f64() == y.as_f64(),
        (None, _) => false,
    };
    let mut put_field = |field: &str, value: Value| {
        if !same(e.get(field), &value) {
            edits.push(set_field(VFX_LIBRARY, format!("{base}/{field}"), value));
        }
    };
    match edit {
        VfxEdit::Size { start, end } => {
            if !(start.is_finite() && (0.005..=4.0).contains(start))
                || !(end.is_finite() && (0.0..=4.0).contains(end))
            {
                return Err(invalid(
                    format!("{path}/size"),
                    "size is 0.005..4 and end_size 0..4 (metres)",
                ));
            }
            put_field("size", json!(round(*start)));
            put_field("end_size", json!(round(*end)));
        }
        VfxEdit::Color { start, end } => {
            if !start
                .iter()
                .chain(end)
                .all(|c| c.is_finite() && (0.0..=1.0).contains(c))
            {
                return Err(invalid(
                    format!("{path}/color"),
                    "colour channels are 0..1 (r,g,b,a)",
                ));
            }
            put_field("color", json!(start.map(round)));
            put_field("end_color", json!(end.map(round)));
        }
        VfxEdit::SizeCurve { keys } => {
            let field = "size_curve";
            // Checked as written (rounded), so two keys cannot round onto one time.
            let keys: Vec<[f64; 2]> = keys.iter().map(|k| k.map(round)).collect();
            if !keys.is_empty()
                && !(curve_times_ok(&keys.iter().map(|k| k[0]).collect::<Vec<_>>())
                    && keys
                        .iter()
                        .all(|k| k[1].is_finite() && (0.0..=4.0).contains(&k[1])))
            {
                return Err(invalid(
                    format!("{path}/{field}"),
                    format!("size_curve: 2..{MAX_VFX_CURVE_KEYS} keys, t 0..1 strictly increasing, size 0..4 (metres)"),
                ));
            }
            let value = json!(keys
                .iter()
                .map(|k| json!({"t": k[0], "size": k[1]}))
                .collect::<Vec<_>>());
            edits.extend(curve_field(&base, e, field, value));
        }
        VfxEdit::ColorGradient { stops } => {
            let field = "color_gradient";
            let stops: Vec<(f64, [f64; 4])> = stops
                .iter()
                .map(|(t, c)| (round(*t), c.map(round)))
                .collect();
            if !stops.is_empty()
                && !(curve_times_ok(&stops.iter().map(|s| s.0).collect::<Vec<_>>())
                    && stops
                        .iter()
                        .flat_map(|s| s.1)
                        .all(|c| c.is_finite() && (0.0..=1.0).contains(&c)))
            {
                return Err(invalid(
                    format!("{path}/{field}"),
                    format!("color_gradient: 2..{MAX_VFX_CURVE_KEYS} stops, t 0..1 strictly increasing, channels 0..1"),
                ));
            }
            let value = json!(stops
                .iter()
                .map(|(t, c)| json!({"t": t, "color": c}))
                .collect::<Vec<_>>());
            edits.extend(curve_field(&base, e, field, value));
        }
    }
    Ok(edits)
}

/// Sets curve `field` of emitter `e` (at `base` in vfx.library) to `value`, or removes it for
/// an empty list (the whole emitter is rewritten without the key). No edit when unchanged.
fn curve_field(base: &str, e: &Value, field: &str, value: Value) -> Option<Edit> {
    // Numeric comparison: authored `1` equals `1.0`.
    let numbers = |v: &Value| -> Vec<Option<f64>> {
        fn walk(v: &Value, out: &mut Vec<Option<f64>>) {
            match v {
                Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
                Value::Object(o) => o.values().for_each(|x| walk(x, out)),
                x => out.push(x.as_f64()),
            }
        }
        let mut out = vec![];
        walk(v, &mut out);
        out
    };
    let empty = value.as_array().is_some_and(Vec::is_empty);
    match e.get(field) {
        None if empty => None,
        Some(_) if empty => {
            let mut without = e.clone();
            without.as_object_mut()?.remove(field);
            Some(set_field(VFX_LIBRARY, base.to_owned(), without))
        }
        Some(old)
            if old.as_array().map(Vec::len) == value.as_array().map(Vec::len)
                && numbers(old) == numbers(&value) =>
        {
            None
        }
        _ => Some(set_field(VFX_LIBRARY, format!("{base}/{field}"), value)),
    }
}

// ---------------------------------------------------------------------------------------
// Shader source

/// Replaces a custom shader's WGSL surface code. The world validator compiles it on commit
/// (an invalid hook is refused with `line N, column M`).
pub fn set_shader_source(world: &World, shader: &str, wgsl: &str) -> Result<Vec<Edit>> {
    let raw = resource(world, SHADER_LIBRARY, "asset create material ID shader=..")?;
    let path = format!(
        "/resources/{SHADER_LIBRARY}/shaders/{}/wgsl",
        escape(shader)
    );
    if crate::assets::BUILTIN_SHADERS.contains(&shader) {
        return Err(Diagnostic::error(
            "builtin_shader",
            path,
            format!("{shader} is built in; its source cannot be edited"),
        ));
    }
    let Some(program) = raw.get("shaders").and_then(|s| s.get(shader)) else {
        return Err(Diagnostic::error(
            "not_found",
            path,
            format!("render.shaders has no custom shader {shader}"),
        ));
    };
    if wgsl.is_empty() || wgsl.len() > MAX_SHADER_SOURCE {
        return Err(invalid(
            path,
            format!("WGSL source must be 1..{MAX_SHADER_SOURCE} bytes"),
        ));
    }
    if program.get("wgsl").and_then(Value::as_str) == Some(wgsl) {
        return Ok(vec![]);
    }
    Ok(vec![set_field(
        SHADER_LIBRARY,
        format!("/shaders/{}/wgsl", escape(shader)),
        json!(wgsl),
    )])
}
