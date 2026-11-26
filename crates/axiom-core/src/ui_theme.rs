//! `ui.document` v2, milestone M1a (docs/plan-ui-styling.zh-TW.md): a theme of nine-slice
//! frames and panel styles, and anchored panels that hold widgets. All fields are optional
//! and skipped when empty, so documents without them serialize exactly as before.
//!
//! `bound_ui` draws each panel as an anchored column (z order = list order) with its style
//! fill and nine-slice frame behind the widgets; panel widgets never fall back to the bottom
//! document panel.
use crate::gameplay::{Region, UiDocument};
use crate::{valid_id, Diagnostic};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_FRAMES: usize = 32;
pub const MAX_PANEL_STYLES: usize = 32;
pub const MAX_PANELS: usize = 32;
pub const MAX_PANEL_CHILDREN: usize = 64;
/// Largest slice margin, in source-image pixels.
pub const MAX_SLICE: u16 = 1024;
pub const MAX_CLASSES: usize = 32;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UiTheme {
    /// Frame ID -> nine-slice image.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub frames: BTreeMap<String, UiFrame>,
    /// Panel style class -> look of the panels that use it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub panel: BTreeMap<String, PanelStyle>,
    /// Text class -> look of the text widgets that use it (`ui.document.classes`, plan M2).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub text: BTreeMap<String, TextClass>,
    /// Button class -> look of the buttons that use it, per state (plan M2).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub button: BTreeMap<String, ButtonClass>,
}
impl UiTheme {
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
            && self.panel.is_empty()
            && self.text.is_empty()
            && self.button.is_empty()
    }
}

/// How a frame's centre fills the inside: stretched (default), tiled at the source size, or
/// left empty (a border only).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FrameCenter {
    #[default]
    Stretch,
    Tile,
    Hidden,
}
fn is_stretch(c: &FrameCenter) -> bool {
    *c == FrameCenter::Stretch
}

/// A text class: size (px), colour and an optional drop shadow colour (2 px down-right).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextClass {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<[u8; 4]>,
}

/// The look of a button in one state; unset fields fall back to the `normal` state.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ButtonLook {
    /// Theme frame ID drawn behind the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
    /// Background RGBA drawn under the frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<[u8; 4]>,
    /// Label colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<[u8; 4]>,
}
impl ButtonLook {
    pub fn is_empty(&self) -> bool {
        self.frame.is_none() && self.fill.is_none() && self.text.is_none()
    }
    /// `self` with unset fields taken from `base`.
    pub fn over(&self, base: &ButtonLook) -> ButtonLook {
        ButtonLook {
            frame: self.frame.clone().or_else(|| base.frame.clone()),
            fill: self.fill.or(base.fill),
            text: self.text.or(base.text),
        }
    }
}

/// A button class: label size and padding, and a look per state. Disabled = the game is not
/// running or the console is open (buttons cannot be used then).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ButtonClass {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<f32>,
    #[serde(default, skip_serializing_if = "ButtonLook::is_empty")]
    pub normal: ButtonLook,
    #[serde(default, skip_serializing_if = "ButtonLook::is_empty")]
    pub hover: ButtonLook,
    #[serde(default, skip_serializing_if = "ButtonLook::is_empty")]
    pub pressed: ButtonLook,
    #[serde(default, skip_serializing_if = "ButtonLook::is_empty")]
    pub disabled: ButtonLook,
}
/// Button states, in the order `ButtonClass` lists them.
pub const BUTTON_STATES: [&str; 4] = ["normal", "hover", "pressed", "disabled"];
impl ButtonClass {
    pub fn state(&self, name: &str) -> Option<&ButtonLook> {
        match name {
            "normal" => Some(&self.normal),
            "hover" => Some(&self.hover),
            "pressed" => Some(&self.pressed),
            "disabled" => Some(&self.disabled),
            _ => None,
        }
    }
    fn state_mut(&mut self, name: &str) -> Option<&mut ButtonLook> {
        match name {
            "normal" => Some(&mut self.normal),
            "hover" => Some(&mut self.hover),
            "pressed" => Some(&mut self.pressed),
            "disabled" => Some(&mut self.disabled),
            _ => None,
        }
    }
}

/// A nine-slice frame: `slice` = [left, top, right, bottom] margins in pixels of the
/// `ui.document.images` image. Corners keep their size, edges and centre stretch.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UiFrame {
    pub image: String,
    pub slice: [u16; 4],
    #[serde(default, skip_serializing_if = "is_stretch")]
    pub center: FrameCenter,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PanelStyle {
    /// Background RGBA (drawn under the frame); none = no fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<[u8; 4]>,
    /// Corner radius of the fill, pixels 0..=64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner: Option<f32>,
    /// Inner margin, pixels 0..=128.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<f32>,
    /// Nine-slice frame drawn behind the content (theme frame ID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
}

/// An anchored panel: widgets stacked in a column, placed at `anchor` + `offset`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UiPanel {
    pub id: String,
    pub anchor: Region,
    /// Offset from the anchor in reference pixels (+x right, +y down).
    #[serde(default, skip_serializing_if = "is_origin")]
    pub offset: [f32; 2],
    /// Fixed width in reference pixels; none = fit the content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// Theme panel style class; none = no fill and no frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Widget IDs, top to bottom.
    #[serde(default)]
    pub children: Vec<String>,
}
fn is_origin(o: &[f32; 2]) -> bool {
    *o == [0.0, 0.0]
}

fn invalid(path: String, message: String) -> anyhow::Error {
    Diagnostic::error("invalid_value", path, message)
}
const DOC: &str = "/resources/ui.document";

/// Checks the theme and panels of `doc` against its images and widgets.
pub fn validate(doc: &UiDocument, widgets: &BTreeSet<&str>) -> Result<()> {
    if let Some(theme) = &doc.theme {
        if theme.frames.len() > MAX_FRAMES || theme.panel.len() > MAX_PANEL_STYLES {
            return Err(invalid(
                format!("{DOC}/theme"),
                format!("at most {MAX_FRAMES} frames and {MAX_PANEL_STYLES} panel styles"),
            ));
        }
        for (id, f) in &theme.frames {
            let path = format!("{DOC}/theme/frames/{id}");
            valid_id(id).map_err(|e| invalid(path.clone(), e.to_string()))?;
            if !doc.images.contains_key(&f.image) {
                return Err(invalid(
                    format!("{path}/image"),
                    format!(
                        "image '{}' is not in ui.document.images (images: {})",
                        f.image,
                        doc.images.keys().cloned().collect::<Vec<_>>().join(", ")
                    ),
                ));
            }
            if f.slice.iter().any(|&s| s > MAX_SLICE) {
                return Err(invalid(
                    format!("{path}/slice"),
                    format!("slice margins are pixels 0..={MAX_SLICE}"),
                ));
            }
        }
        for (class, s) in &theme.panel {
            let path = format!("{DOC}/theme/panel/{class}");
            valid_id(class).map_err(|e| invalid(path.clone(), e.to_string()))?;
            if let Some(c) = s.corner {
                if !(c.is_finite() && (0.0..=64.0).contains(&c)) {
                    return Err(invalid(
                        format!("{path}/corner"),
                        "corner is 0..=64 px".into(),
                    ));
                }
            }
            if let Some(p) = s.padding {
                if !(p.is_finite() && (0.0..=128.0).contains(&p)) {
                    return Err(invalid(
                        format!("{path}/padding"),
                        "padding is 0..=128 px".into(),
                    ));
                }
            }
            if let Some(frame) = &s.frame {
                if !theme.frames.contains_key(frame) {
                    return Err(invalid(
                        format!("{path}/frame"),
                        format!("frame '{frame}' is not in theme.frames"),
                    ));
                }
            }
        }
        if theme.text.len() > MAX_CLASSES || theme.button.len() > MAX_CLASSES {
            return Err(invalid(
                format!("{DOC}/theme"),
                format!("at most {MAX_CLASSES} text and {MAX_CLASSES} button classes"),
            ));
        }
        let size = |path: String, v: Option<f32>| -> Result<()> {
            match v {
                Some(v) if !(v.is_finite() && (8.0..=96.0).contains(&v)) => {
                    Err(invalid(path, "size is 8..=96 px".into()))
                }
                _ => Ok(()),
            }
        };
        for (class, t) in &theme.text {
            let path = format!("{DOC}/theme/text/{class}");
            valid_id(class).map_err(|e| invalid(path.clone(), e.to_string()))?;
            size(format!("{path}/size"), t.size)?;
        }
        for (class, b) in &theme.button {
            let path = format!("{DOC}/theme/button/{class}");
            valid_id(class).map_err(|e| invalid(path.clone(), e.to_string()))?;
            size(format!("{path}/size"), b.size)?;
            if let Some(p) = b.padding {
                if !(p.is_finite() && (0.0..=64.0).contains(&p)) {
                    return Err(invalid(
                        format!("{path}/padding"),
                        "padding is 0..=64 px".into(),
                    ));
                }
            }
            for state in BUTTON_STATES {
                if let Some(frame) = b.state(state).and_then(|l| l.frame.as_ref()) {
                    if !theme.frames.contains_key(frame) {
                        return Err(invalid(
                            format!("{path}/{state}/frame"),
                            format!("frame '{frame}' is not in theme.frames"),
                        ));
                    }
                }
            }
        }
    }
    for (widget, class) in &doc.classes {
        let path = format!("{DOC}/classes/{widget}");
        let Some(w) = doc.widgets.iter().find(|w| w.id() == widget) else {
            return Err(invalid(path, format!("unknown widget '{widget}'")));
        };
        if doc.styles.contains_key(widget) {
            return Err(invalid(
                path,
                format!("widget '{widget}' also has a text style; use one of styles or classes"),
            ));
        }
        let theme = doc.theme.as_ref();
        let found = match w {
            crate::gameplay::Widget::Text { .. } => {
                theme.is_some_and(|t| t.text.contains_key(class))
            }
            crate::gameplay::Widget::Button { .. } => {
                theme.is_some_and(|t| t.button.contains_key(class))
            }
            _ => {
                return Err(invalid(
                    path,
                    format!("classes apply to text and button widgets; '{widget}' is neither"),
                ))
            }
        };
        if !found {
            return Err(invalid(
                path,
                format!("class '{class}' is not in theme.text / theme.button for this widget kind"),
            ));
        }
    }
    if doc.panels.len() > MAX_PANELS {
        return Err(invalid(
            format!("{DOC}/panels"),
            format!("at most {MAX_PANELS} panels"),
        ));
    }
    let mut ids = BTreeSet::new();
    let mut placed = BTreeSet::new();
    for (i, p) in doc.panels.iter().enumerate() {
        let path = format!("{DOC}/panels/{i}");
        valid_id(&p.id).map_err(|e| invalid(format!("{path}/id"), e.to_string()))?;
        if !ids.insert(p.id.as_str()) {
            return Err(invalid(
                format!("{path}/id"),
                format!("duplicate panel id '{}'", p.id),
            ));
        }
        if !p.offset.iter().all(|v| v.is_finite() && v.abs() <= 8192.0) {
            return Err(invalid(
                format!("{path}/offset"),
                "offset is finite, |x|,|y| <= 8192 px".into(),
            ));
        }
        if let Some(w) = p.width {
            if !(w.is_finite() && (16.0..=4096.0).contains(&w)) {
                return Err(invalid(
                    format!("{path}/width"),
                    "width is 16..=4096 px".into(),
                ));
            }
        }
        if let Some(style) = &p.style {
            if !doc
                .theme
                .as_ref()
                .is_some_and(|t| t.panel.contains_key(style))
            {
                return Err(invalid(
                    format!("{path}/style"),
                    format!("panel style '{style}' is not in theme.panel"),
                ));
            }
        }
        if p.children.len() > MAX_PANEL_CHILDREN {
            return Err(invalid(
                format!("{path}/children"),
                format!("at most {MAX_PANEL_CHILDREN} widgets per panel"),
            ));
        }
        for (j, child) in p.children.iter().enumerate() {
            let at = format!("{path}/children/{j}");
            if !widgets.contains(child.as_str()) {
                return Err(invalid(at, format!("unknown widget '{child}'")));
            }
            if doc.regions.contains_key(child) {
                return Err(invalid(
                    at,
                    format!("widget '{child}' is also placed in a region; use one placement"),
                ));
            }
            if !placed.insert(child.as_str()) {
                return Err(invalid(at, format!("widget '{child}' is in two panels")));
            }
        }
    }
    Ok(())
}

/// `RRGGBB` or `RRGGBBAA` (optional leading `#`).
pub fn parse_rgba(text: &str) -> Result<[u8; 4]> {
    let hex = text.trim_start_matches('#');
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16);
    let bad = || anyhow::anyhow!("colour '{text}' must be RRGGBB or RRGGBBAA hex");
    match hex.len() {
        6 if hex.is_ascii() => Ok([
            byte(0).map_err(|_| bad())?,
            byte(2).map_err(|_| bad())?,
            byte(4).map_err(|_| bad())?,
            255,
        ]),
        8 if hex.is_ascii() => Ok([
            byte(0).map_err(|_| bad())?,
            byte(2).map_err(|_| bad())?,
            byte(4).map_err(|_| bad())?,
            byte(6).map_err(|_| bad())?,
        ]),
        _ => Err(bad()),
    }
}

/// `ui panelstyle CLASS fill=RRGGBB[AA] corner=N padding=N frame=ID` options.
pub fn panel_style_from(options: &[(String, String)]) -> Result<PanelStyle> {
    let mut s = PanelStyle::default();
    let number = |k: &str, v: &str| -> Result<f32> {
        v.parse()
            .map_err(|_| anyhow::anyhow!("{k}='{v}' is not a number"))
    };
    for (k, v) in options {
        match k.as_str() {
            "fill" => s.fill = Some(parse_rgba(v)?),
            "corner" => s.corner = Some(number(k, v)?),
            "padding" => s.padding = Some(number(k, v)?),
            "frame" => s.frame = Some(v.clone()),
            _ => anyhow::bail!("unknown panel style option '{k}' (fill, corner, padding, frame)"),
        }
    }
    Ok(s)
}

/// `ui panel ID ANCHOR at=X,Y width=W style=CLASS widgets=A,B` options.
pub fn panel_from(id: &str, anchor: Region, options: &[(String, String)]) -> Result<UiPanel> {
    let mut p = UiPanel {
        id: id.to_owned(),
        anchor,
        offset: [0.0, 0.0],
        width: None,
        style: None,
        children: vec![],
    };
    for (k, v) in options {
        match k.as_str() {
            "at" => {
                let parts: Vec<f32> = v
                    .split(',')
                    .map(|n| n.trim().parse::<f32>())
                    .collect::<std::result::Result<_, _>>()
                    .map_err(|_| anyhow::anyhow!("at='{v}' must be X,Y"))?;
                anyhow::ensure!(parts.len() == 2, "at='{v}' must be X,Y");
                p.offset = [parts[0], parts[1]];
            }
            "width" => {
                p.width = Some(
                    v.parse()
                        .map_err(|_| anyhow::anyhow!("width='{v}' is not a number"))?,
                )
            }
            "style" => p.style = Some(v.clone()),
            "widgets" => {
                p.children = v
                    .split(',')
                    .map(str::trim)
                    .filter(|w| !w.is_empty())
                    .map(str::to_owned)
                    .collect()
            }
            _ => anyhow::bail!("unknown panel option '{k}' (at, width, style, widgets)"),
        }
    }
    Ok(p)
}

/// `ui textclass CLASS size=N color=RRGGBB[AA] shadow=RRGGBB[AA]` options.
pub fn text_class_from(options: &[(String, String)]) -> Result<TextClass> {
    let mut t = TextClass::default();
    for (k, v) in options {
        match k.as_str() {
            "size" => {
                t.size = Some(
                    v.parse()
                        .map_err(|_| anyhow::anyhow!("size='{v}' is not a number"))?,
                )
            }
            "color" => t.color = Some(parse_rgba(v)?),
            "shadow" => t.shadow = Some(parse_rgba(v)?),
            _ => anyhow::bail!("unknown text class option '{k}' (size, color, shadow)"),
        }
    }
    Ok(t)
}

/// `ui buttonclass CLASS size=N padding=N STATE.frame=ID STATE.fill=.. STATE.text=..`
/// (STATE = normal | hover | pressed | disabled) options.
pub fn button_class_from(options: &[(String, String)]) -> Result<ButtonClass> {
    let mut b = ButtonClass::default();
    for (k, v) in options {
        let number = || -> Result<f32> {
            v.parse()
                .map_err(|_| anyhow::anyhow!("{k}='{v}' is not a number"))
        };
        match k.as_str() {
            "size" => b.size = Some(number()?),
            "padding" => b.padding = Some(number()?),
            _ => {
                let (state, field) = k.split_once('.').ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown button class option '{k}' (size, padding, STATE.frame|fill|text; STATE: {})",
                        BUTTON_STATES.join(", ")
                    )
                })?;
                let look = b.state_mut(state).ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown button state '{state}' (use {})",
                        BUTTON_STATES.join(", ")
                    )
                })?;
                match field {
                    "frame" => look.frame = Some(v.clone()),
                    "fill" => look.fill = Some(parse_rgba(v)?),
                    "text" => look.text = Some(parse_rgba(v)?),
                    _ => anyhow::bail!("unknown button look field '{field}' (frame, fill, text)"),
                }
            }
        }
    }
    Ok(b)
}

/// `center=stretch|tile|hidden` of `ui frame`.
pub fn parse_center(text: &str) -> Result<FrameCenter> {
    serde_json::from_value(serde_json::json!(text))
        .map_err(|_| anyhow::anyhow!("center '{text}' must be stretch, tile or hidden"))
}
