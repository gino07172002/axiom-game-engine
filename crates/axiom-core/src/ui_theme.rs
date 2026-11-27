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
/// Deepest panel nesting (a root panel is depth 1).
pub const MAX_PANEL_DEPTH: usize = 4;
pub const MAX_GRID_COLUMNS: u8 = 16;
/// Content fonts per document (plan M4).
pub const MAX_FONTS: usize = 4;
/// Byte budget of one font file.
pub const FONT_BYTES: u64 = 25_165_824;
/// Pages per document (plan M6) and open pages at once.
pub const MAX_PAGES: usize = 8;

/// A page (plan M6): root panels shown only while the page is open. Pages open and close
/// with the typed gameplay commands `open` / `close`; the open stack is gameplay state
/// (saved in checkpoints). A modal page dims the HUD under it, makes its buttons unusable
/// and takes keyboard / gamepad focus; a pause page also stops the fixed-step simulation;
/// the escape page is what Esc (gamepad Start) opens and closes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UiPage {
    /// Root panel IDs, drawn in this order (z still orders them).
    pub panels: Vec<String>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub modal: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pause: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub escape: bool,
}
fn yes() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}
fn is_false(b: &bool) -> bool {
    !*b
}

/// Licences a content font may declare (SPDX IDs): open licences that allow redistribution.
pub const FONT_LICENSES: [&str; 4] = ["OFL-1.1", "Apache-2.0", "MIT", "CC0-1.0"];

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
    /// Font ID -> TrueType / OpenType file shipped with the content (plan M4).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fonts: BTreeMap<String, UiFont>,
    /// Reference screen [width, height] the panels are authored for (plan M3); none = panels
    /// are laid out in screen pixels, unscaled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<[f32; 2]>,
    /// How the reference screen maps to the window (default fit_height).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<UiScale>,
    /// In-world name tags (`ui.nameplate`, plan N1): default look and distances.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nameplate: Option<NameplateTheme>,
}

/// Default nameplate distance without a theme: farther plates are not drawn.
pub const NAMEPLATE_HIDE_BEYOND: f32 = 40.0;

/// `theme.nameplate` (plan N1): every field optional; without it plates look as before
/// (14 px label on a dark box, hidden beyond 40 m, no fade).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NameplateTheme {
    /// Meters from the camera where plates start to fade out (reach 0 at `hide_beyond`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_from: Option<f32>,
    /// Meters from the camera beyond which plates are not drawn (default 40).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide_beyond: Option<f32>,
    /// Default text class (`theme.text`) of plates without their own `class`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// Default panel style (`theme.panel`) of plates without their own `style`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}
impl NameplateTheme {
    /// The distance beyond which plates are hidden.
    pub fn hide(&self) -> f32 {
        self.hide_beyond.unwrap_or(NAMEPLATE_HIDE_BEYOND)
    }
    /// Opacity 0..1 of a plate `distance` meters from the camera (0 = not drawn).
    pub fn opacity(&self, distance: f32) -> f32 {
        let hide = self.hide();
        // NaN (no distance) counts as out of range.
        if distance.is_nan() || distance > hide {
            return 0.0;
        }
        match self.fade_from {
            Some(from) if distance > from => ((hide - distance) / (hide - from)).clamp(0.0, 1.0),
            _ => 1.0,
        }
    }
}

/// Panel scaling against `theme.reference`: the whole panel layer is scaled by one factor.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UiScale {
    /// window height / reference height
    #[default]
    FitHeight,
    /// window width / reference width
    FitWidth,
    /// the smaller of the two (the whole reference screen stays visible)
    Fit,
    /// 1: reference pixels are screen pixels
    None,
}
impl UiScale {
    /// The scale factor for a window of `screen` size.
    pub fn factor(self, reference: [f32; 2], screen: [f32; 2]) -> f32 {
        let [rw, rh] = [reference[0].max(1.0), reference[1].max(1.0)];
        let f = match self {
            UiScale::FitHeight => screen[1] / rh,
            UiScale::FitWidth => screen[0] / rw,
            UiScale::Fit => (screen[0] / rw).min(screen[1] / rh),
            UiScale::None => 1.0,
        };
        if f.is_finite() {
            f.clamp(0.1, 8.0)
        } else {
            1.0
        }
    }
}
impl UiTheme {
    /// The panel scale factor for a window of `screen` size (1 without a reference).
    pub fn scale_factor(&self, screen: [f32; 2]) -> f32 {
        match self.reference {
            Some(r) => self.scale.unwrap_or_default().factor(r, screen),
            None => 1.0,
        }
    }
}

/// A panel length: reference pixels (`900`) or a percentage of the reference screen
/// (`"50%"`; of the window when the theme has no reference).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    Px(f32),
    Percent(f32),
}
impl Length {
    /// Pixels against a screen dimension of `of` reference pixels.
    pub fn resolve(self, of: f32) -> f32 {
        match self {
            Length::Px(v) => v,
            Length::Percent(p) => of * p / 100.0,
        }
    }
    /// `900` or `50%` (ACL and editor text).
    pub fn parse(text: &str) -> Result<Length> {
        let bad = || anyhow::anyhow!("'{text}' must be pixels (900) or a percentage (50%)");
        match text.strip_suffix('%') {
            Some(p) => Ok(Length::Percent(p.trim().parse().map_err(|_| bad())?)),
            None => Ok(Length::Px(text.trim().parse().map_err(|_| bad())?)),
        }
    }
}
impl Serialize for Length {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Length::Px(v) => s.serialize_f32(*v),
            Length::Percent(p) => s.serialize_str(&format!("{p}%")),
        }
    }
}
impl<'de> Deserialize<'de> for Length {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Number(f32),
            Text(String),
        }
        match Raw::deserialize(d)? {
            Raw::Number(v) => Ok(Length::Px(v)),
            Raw::Text(t) if t.ends_with('%') => Length::parse(&t).map_err(serde::de::Error::custom),
            Raw::Text(t) => Err(serde::de::Error::custom(format!(
                "length '{t}' must be a number of pixels or a percentage like \"50%\""
            ))),
        }
    }
}

/// How a panel arranges its children.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PanelLayout {
    /// top to bottom
    #[default]
    Column,
    /// left to right
    Row,
    /// `columns` per row, left to right then down
    Grid,
    /// all children at the same origin, later ones on top
    Stack,
}
fn is_column(l: &PanelLayout) -> bool {
    *l == PanelLayout::Column
}
impl UiTheme {
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
            && self.panel.is_empty()
            && self.text.is_empty()
            && self.button.is_empty()
            && self.fonts.is_empty()
            && self.reference.is_none()
            && self.scale.is_none()
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

/// A content font: a .ttf / .otf path (package-relative in content, absolute once imported)
/// and its SPDX licence (one of `FONT_LICENSES`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UiFont {
    pub path: String,
    pub license: String,
}

/// A text class: size (px), colour and an optional drop shadow colour (2 px down-right).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextClass {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    /// Theme font ID; none = the default UI font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
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
    /// Theme font ID of the label; none = the default UI font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
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

/// An anchored panel holding widgets and child panels, placed at `anchor` + `offset`. A
/// panel named as another panel's child is drawn inside it (its anchor, offset and z are
/// then unused).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UiPanel {
    pub id: String,
    pub anchor: Region,
    /// Offset from the anchor in reference pixels (+x right, +y down).
    #[serde(default, skip_serializing_if = "is_origin")]
    pub offset: [f32; 2],
    /// Fixed width (reference pixels or a % of the reference width); none = fit the content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<Length>,
    /// Fixed height (pixels or %); none = fit the content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<Length>,
    #[serde(default, skip_serializing_if = "is_column")]
    pub layout: PanelLayout,
    /// Cells per row of a grid layout (default 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u8>,
    /// Space between children, pixels 0..=128 (default: the UI's item spacing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<f32>,
    /// Drawing order of root panels: higher on top; ties keep list order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z: Option<i16>,
    /// Theme panel style class; none = no fill and no frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Widget and panel IDs, in layout order.
    #[serde(default)]
    pub children: Vec<String>,
}
impl UiPanel {
    pub fn new(id: &str, anchor: Region) -> UiPanel {
        UiPanel {
            id: id.to_owned(),
            anchor,
            offset: [0.0, 0.0],
            width: None,
            height: None,
            layout: PanelLayout::Column,
            columns: None,
            gap: None,
            z: None,
            style: None,
            children: vec![],
        }
    }
}
fn is_origin(o: &[f32; 2]) -> bool {
    *o == [0.0, 0.0]
}

fn invalid(path: String, message: String) -> anyhow::Error {
    Diagnostic::error("invalid_value", path, message)
}
const DOC: &str = "/resources/ui.document";

/// `ui.nameplate` `class` / `style` (plan N1) must name a text class / panel style of the
/// document theme; the error names the entity.
pub fn validate_nameplates(world: &crate::World, doc: Option<&UiDocument>) -> Result<()> {
    let theme = doc.and_then(|d| d.theme.as_ref());
    for e in &world.entities {
        let Some(v) = e.components.get("ui.nameplate") else {
            continue;
        };
        let path = format!("/entities/{}/components/ui.nameplate", e.id);
        for (field, kind) in [("class", "theme.text"), ("style", "theme.panel")] {
            let Some(name) = v.get(field).and_then(|n| n.as_str()) else {
                continue;
            };
            let known = theme.is_some_and(|t| match field {
                "class" => t.text.contains_key(name),
                _ => t.panel.contains_key(name),
            });
            if !known {
                return Err(invalid(
                    format!("{path}/{field}"),
                    format!("'{name}' is not in ui.document {kind}"),
                ));
            }
        }
    }
    Ok(())
}

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
        if let Some([w, h]) = theme.reference {
            if !(w.is_finite()
                && h.is_finite()
                && (320.0..=7680.0).contains(&w)
                && (240.0..=4320.0).contains(&h))
            {
                return Err(invalid(
                    format!("{DOC}/theme/reference"),
                    "reference is [width 320..=7680, height 240..=4320] px".into(),
                ));
            }
        } else if theme.scale.is_some() {
            return Err(invalid(
                format!("{DOC}/theme/scale"),
                "scale needs a theme.reference screen size".into(),
            ));
        }
        if theme.fonts.len() > MAX_FONTS {
            return Err(invalid(
                format!("{DOC}/theme/fonts"),
                format!("at most {MAX_FONTS} fonts"),
            ));
        }
        for (id, f) in &theme.fonts {
            let path = format!("{DOC}/theme/fonts/{id}");
            valid_id(id).map_err(|e| invalid(path.clone(), e.to_string()))?;
            let font_file = std::path::Path::new(&f.path)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("ttf") || e.eq_ignore_ascii_case("otf"));
            if f.path.len() > 1024 || !font_file {
                return Err(invalid(
                    format!("{path}/path"),
                    "a font is a .ttf or .otf file (path <= 1024 bytes)".into(),
                ));
            }
            if !FONT_LICENSES.contains(&f.license.as_str()) {
                return Err(invalid(
                    format!("{path}/license"),
                    format!(
                        "license '{}' is not accepted; use one of {} (fonts ship with the game)",
                        f.license,
                        FONT_LICENSES.join(", ")
                    ),
                ));
            }
        }
        let font_ref = |path: String, font: &Option<String>| -> Result<()> {
            match font {
                Some(f) if !theme.fonts.contains_key(f) => {
                    Err(invalid(path, format!("font '{f}' is not in theme.fonts")))
                }
                _ => Ok(()),
            }
        };
        if let Some(n) = &theme.nameplate {
            let path = format!("{DOC}/theme/nameplate");
            let hide = n.hide();
            if !(hide.is_finite() && hide > 0.0 && hide <= 1000.0) {
                return Err(invalid(
                    format!("{path}/hide_beyond"),
                    "hide_beyond is meters 0 < N <= 1000".into(),
                ));
            }
            if let Some(from) = n.fade_from {
                if !(from.is_finite() && from >= 0.0 && from < hide) {
                    return Err(invalid(
                        format!("{path}/fade_from"),
                        format!("fade_from is meters 0..{hide} (below hide_beyond)"),
                    ));
                }
            }
            if let Some(c) = n.class.as_ref().filter(|c| !theme.text.contains_key(*c)) {
                return Err(invalid(
                    format!("{path}/class"),
                    format!("text class '{c}' is not in theme.text"),
                ));
            }
            if let Some(s) = n.style.as_ref().filter(|s| !theme.panel.contains_key(*s)) {
                return Err(invalid(
                    format!("{path}/style"),
                    format!("panel style '{s}' is not in theme.panel"),
                ));
            }
        }
        for (class, t) in &theme.text {
            font_ref(format!("{DOC}/theme/text/{class}/font"), &t.font)?;
        }
        for (class, b) in &theme.button {
            font_ref(format!("{DOC}/theme/button/{class}/font"), &b.font)?;
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
        for (name, length) in [("width", p.width), ("height", p.height)] {
            let ok = match length {
                None => true,
                Some(Length::Px(v)) => v.is_finite() && (16.0..=4096.0).contains(&v),
                Some(Length::Percent(v)) => v.is_finite() && (1.0..=100.0).contains(&v),
            };
            if !ok {
                return Err(invalid(
                    format!("{path}/{name}"),
                    format!("{name} is 16..=4096 px or 1..=100%"),
                ));
            }
        }
        match (p.layout, p.columns) {
            (PanelLayout::Grid, Some(c)) if !(1..=MAX_GRID_COLUMNS).contains(&c) => {
                return Err(invalid(
                    format!("{path}/columns"),
                    format!("columns is 1..={MAX_GRID_COLUMNS}"),
                ))
            }
            (PanelLayout::Grid, _) | (_, None) => {}
            (_, Some(_)) => {
                return Err(invalid(
                    format!("{path}/columns"),
                    "columns applies to layout grid".into(),
                ))
            }
        }
        if let Some(gap) = p.gap {
            if !(gap.is_finite() && (0.0..=128.0).contains(&gap)) {
                return Err(invalid(format!("{path}/gap"), "gap is 0..=128 px".into()));
            }
        }
        if let Some(z) = p.z {
            if !(-100..=100).contains(&z) {
                return Err(invalid(format!("{path}/z"), "z is -100..=100".into()));
            }
        }
        if widgets.contains(p.id.as_str()) {
            return Err(invalid(
                format!("{path}/id"),
                format!(
                    "panel id '{}' is also a widget id; panel children name either",
                    p.id
                ),
            ));
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
            if doc.panels.iter().any(|q| &q.id == child) {
                if child == &p.id {
                    return Err(invalid(at, format!("panel '{child}' cannot hold itself")));
                }
                if !placed.insert(child.as_str()) {
                    return Err(invalid(at, format!("panel '{child}' is in two panels")));
                }
                continue;
            }
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
    // Pages (plan M6): root panels, each in at most one page; at most one escape page.
    if doc.pages.len() > MAX_PAGES {
        return Err(invalid(
            format!("{DOC}/pages"),
            format!("at most {MAX_PAGES} pages"),
        ));
    }
    let mut paged = BTreeSet::new();
    for (id, page) in &doc.pages {
        let path = format!("{DOC}/pages/{id}");
        valid_id(id).map_err(|e| invalid(path.clone(), e.to_string()))?;
        if page.panels.is_empty() || page.panels.len() > MAX_PANELS {
            return Err(invalid(
                format!("{path}/panels"),
                format!("a page shows 1..={MAX_PANELS} panels"),
            ));
        }
        for (j, panel) in page.panels.iter().enumerate() {
            let at = format!("{path}/panels/{j}");
            if doc.panels.iter().all(|p| &p.id != panel) {
                return Err(invalid(at, format!("unknown panel '{panel}'")));
            }
            if placed.contains(panel.as_str()) {
                return Err(invalid(
                    at,
                    format!("panel '{panel}' is inside another panel; pages show root panels"),
                ));
            }
            if !paged.insert(panel.as_str()) {
                return Err(invalid(at, format!("panel '{panel}' is in two pages")));
            }
        }
    }
    if doc.pages.values().filter(|p| p.escape).count() > 1 {
        return Err(invalid(
            format!("{DOC}/pages"),
            "at most one page has escape=on".into(),
        ));
    }
    // Nesting: every panel is reached from a root (no cycles), at most MAX_PANEL_DEPTH deep.
    let depths = panel_depths(&doc.panels);
    for (i, p) in doc.panels.iter().enumerate() {
        match depths[i] {
            0 => {
                return Err(invalid(
                    format!("{DOC}/panels/{i}"),
                    format!("panel '{}' is inside a cycle of child panels", p.id),
                ))
            }
            d if d > MAX_PANEL_DEPTH => {
                return Err(invalid(
                    format!("{DOC}/panels/{i}"),
                    format!(
                        "panel '{}' is nested {d} deep (at most {MAX_PANEL_DEPTH})",
                        p.id
                    ),
                ))
            }
            _ => {}
        }
    }
    Ok(())
}

/// Depth of each panel (1 = root: no panel names it as a child; 0 = not reachable from a
/// root, i.e. inside a cycle).
pub fn panel_depths(panels: &[UiPanel]) -> Vec<usize> {
    let index = |id: &str| panels.iter().position(|p| p.id == id);
    let mut parent = vec![None; panels.len()];
    for (i, p) in panels.iter().enumerate() {
        for c in &p.children {
            if let Some(j) = index(c) {
                if j != i && parent[j].is_none() {
                    parent[j] = Some(i);
                }
            }
        }
    }
    (0..panels.len())
        .map(|mut i| {
            let mut depth = 1;
            while let Some(up) = parent[i] {
                depth += 1;
                i = up;
                if depth > panels.len() {
                    return 0;
                }
            }
            depth
        })
        .collect()
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
    let mut p = UiPanel::new(id, anchor);
    let number = |k: &str, v: &str| -> Result<f32> {
        v.parse()
            .map_err(|_| anyhow::anyhow!("{k}='{v}' is not a number"))
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
            "width" => p.width = Some(Length::parse(v)?),
            "height" => p.height = Some(Length::parse(v)?),
            "layout" => {
                p.layout = serde_json::from_value(serde_json::json!(v)).map_err(|_| {
                    anyhow::anyhow!("layout '{v}' must be column, row, grid or stack")
                })?
            }
            "columns" => {
                p.columns = Some(
                    v.parse()
                        .map_err(|_| anyhow::anyhow!("columns='{v}' is not a count"))?,
                )
            }
            "gap" => p.gap = Some(number(k, v)?),
            "z" => {
                p.z = Some(
                    v.parse()
                        .map_err(|_| anyhow::anyhow!("z='{v}' is not a whole number"))?,
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
            _ => anyhow::bail!(
                "unknown panel option '{k}' (at, width, height, layout, columns, gap, z, style, widgets)"
            ),
        }
    }
    Ok(p)
}

/// `ui font ID PATH license=SPDX`: a content font.
pub fn font_from(path: &str, options: &[(String, String)]) -> Result<UiFont> {
    let mut license = None;
    for (k, v) in options {
        match k.as_str() {
            "license" => license = Some(v.clone()),
            _ => anyhow::bail!("unknown font option '{k}' (license)"),
        }
    }
    Ok(UiFont {
        path: path.to_owned(),
        license: license.ok_or_else(|| {
            anyhow::anyhow!(
                "a font needs license=SPDX (one of {})",
                FONT_LICENSES.join(", ")
            )
        })?,
    })
}

/// `ui textclass CLASS size=N font=ID color=RRGGBB[AA] shadow=RRGGBB[AA]` options.
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
            "font" => t.font = Some(v.clone()),
            _ => anyhow::bail!("unknown text class option '{k}' (size, font, color, shadow)"),
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
            "font" => b.font = Some(v.clone()),
            _ => {
                let (state, field) = k.split_once('.').ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown button class option '{k}' (size, font, padding, STATE.frame|fill|text; STATE: {})",
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

/// `ui page ID panels=A,B [modal=on|off] [pause=on|off] [escape=on|off]` options.
pub fn page_from(options: &[(String, String)]) -> Result<UiPage> {
    let mut page = UiPage {
        panels: vec![],
        modal: true,
        pause: false,
        escape: false,
    };
    let flag = |k: &str, v: &str| -> Result<bool> {
        match v {
            "on" | "true" => Ok(true),
            "off" | "false" => Ok(false),
            _ => anyhow::bail!("{k}='{v}' must be on or off"),
        }
    };
    for (k, v) in options {
        match k.as_str() {
            "panels" => {
                page.panels = v
                    .split(',')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_owned)
                    .collect()
            }
            "modal" => page.modal = flag(k, v)?,
            "pause" => page.pause = flag(k, v)?,
            "escape" => page.escape = flag(k, v)?,
            _ => anyhow::bail!("unknown page option '{k}' (panels, modal, pause, escape)"),
        }
    }
    Ok(page)
}

/// `ui screen W H scale=fit_height|fit_width|fit|none`: the panels' scale mode.
pub fn parse_scale(text: &str) -> Result<UiScale> {
    serde_json::from_value(serde_json::json!(text))
        .map_err(|_| anyhow::anyhow!("scale '{text}' must be fit_height, fit_width, fit or none"))
}

/// `center=stretch|tile|hidden` of `ui frame`.
pub fn parse_center(text: &str) -> Result<FrameCenter> {
    serde_json::from_value(serde_json::json!(text))
        .map_err(|_| anyhow::anyhow!("center '{text}' must be stretch, tile or hidden"))
}
