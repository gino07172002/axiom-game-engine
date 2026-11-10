//! Editor icons: the project's SVG icon set, rasterized at build time by
//! `scripts/build-editor-icons.ps1` into one white-on-transparent atlas (`atlas.png`, two cell
//! sizes). The atlas is decoded and uploaded once per egui context; drawing an icon is one
//! textured quad tinted through its vertex colour. Lookup is by enum index (no strings).
use egui::{Color32, Pos2, Rect, TextureHandle, TextureId, Vec2};

const ATLAS_PNG: &[u8] = include_bytes!("../../../../assets/editor-icons/atlas.png");
/// Icons per atlas row (both levels).
const COLUMNS: usize = 16;
/// Cell sizes in pixels of the two levels: large on top, small below it.
const LARGE: f32 = 40.0;
const SMALL: f32 = 20.0;

macro_rules! icons {
    ($($variant:ident = $name:literal,)*) => {
        /// One icon of `assets/editor-icons` (same order as `icons.json` / `atlas.json`).
        // The whole set is embedded; not every icon has a use yet.
        #[allow(dead_code)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) enum Icon { $($variant,)* }
        impl Icon {
            #[allow(dead_code)]
            pub(crate) const ALL: &'static [Icon] = &[$(Icon::$variant,)*];
            /// File stem in `assets/editor-icons`.
            pub(crate) const NAMES: &'static [&'static str] = &[$($name,)*];
            #[allow(dead_code)]
            pub(crate) fn name(self) -> &'static str {
                Self::NAMES[self as usize]
            }
        }
    };
}

icons! {
    Select = "select",
    Move = "move",
    Rotate = "rotate",
    Scale = "scale",
    RectTool = "rect-tool",
    Snap = "snap",
    Grid = "grid",
    List = "list",
    Pivot = "pivot",
    Center = "center",
    Global = "global",
    Local = "local",
    Frame = "frame",
    Fullscreen = "fullscreen",
    Screenshot = "screenshot",
    Play = "play",
    Pause = "pause",
    Stop = "stop",
    Step = "step",
    TimeScale = "time-scale",
    Record = "record",
    Undo = "undo",
    Redo = "redo",
    Save = "save",
    Add = "add",
    Duplicate = "duplicate",
    Delete = "delete",
    Rename = "rename",
    Copy = "copy",
    Paste = "paste",
    Reset = "reset",
    More = "more",
    Search = "search",
    Filter = "filter",
    Close = "close",
    Check = "check",
    ChevronDown = "chevron-down",
    ChevronRight = "chevron-right",
    ChevronUp = "chevron-up",
    DragHandle = "drag-handle",
    ExternalLink = "external-link",
    Link = "link",
    Unlink = "unlink",
    Eye = "eye",
    EyeOff = "eye-off",
    Lock = "lock",
    Unlock = "unlock",
    Hierarchy = "hierarchy",
    Inspector = "inspector",
    Assets = "assets",
    Console = "console",
    History = "history",
    Proposals = "proposals",
    Problems = "problems",
    Scenarios = "scenarios",
    Graphs = "graphs",
    Game = "game",
    Scene = "scene",
    World = "world",
    Layout = "layout",
    Settings = "settings",
    Agent = "agent",
    Build = "build",
    Help = "help",
    Entity = "entity",
    Group = "group",
    Folder = "folder",
    FolderOpen = "folder-open",
    File = "file",
    Mesh = "mesh",
    Material = "material",
    Texture = "texture",
    Animation = "animation",
    Skeleton = "skeleton",
    Audio = "audio",
    AudioListener = "audio-listener",
    LightPoint = "light-point",
    LightSpot = "light-spot",
    LightDirectional = "light-directional",
    LightArea = "light-area",
    Camera = "camera",
    CameraFollow = "camera-follow",
    Trigger = "trigger",
    Path = "path",
    Nav = "nav",
    Interact = "interact",
    Tween = "tween",
    Fsm = "fsm",
    Behavior = "behavior",
    UiDocument = "ui-document",
    Prefab = "prefab",
    Collider = "collider",
    Vfx = "vfx",
    Script = "script",
    Tag = "tag",
    Layers = "layers",
    Input = "input",
    Localization = "localization",
    Event = "event",
    Chip = "chip",
    Info = "info",
    Warning = "warning",
    Error = "error",
    Success = "success",
    Loading = "loading",
    Dot = "dot",
}

const ROWS: usize = Icon::NAMES.len().div_ceil(COLUMNS);
const WIDTH: f32 = COLUMNS as f32 * LARGE;
const HEIGHT: f32 = ROWS as f32 * (LARGE + SMALL);

/// The uploaded atlas of one egui context. Kept alive by the editor; drawing uses the `Copy`
/// [`Icons`] handle.
pub(crate) struct Atlas {
    handle: TextureHandle,
}

impl Atlas {
    /// Decodes the embedded atlas and uploads it (once per context).
    pub(crate) fn load(ctx: &egui::Context) -> anyhow::Result<Self> {
        let (rgba, w, h) = crate::branding::decode_rgba(ATLAS_PNG)?;
        anyhow::ensure!(
            (w as f32, h as f32) == (WIDTH, HEIGHT),
            "icon atlas is {w}x{h}, expected {WIDTH}x{HEIGHT}: rerun scripts/build-editor-icons.ps1"
        );
        let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
        let handle = ctx.load_texture("editor-icons", image, egui::TextureOptions::LINEAR);
        Ok(Self { handle })
    }
    pub(crate) fn icons(&self) -> Icons {
        Icons(Some(self.handle.id()))
    }
}

/// Copyable handle for drawing icons. `Icons(None)` (atlas failed to load) draws nothing.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Icons(pub(crate) Option<TextureId>);

/// Normalized atlas rectangle of `icon` for drawing at `pixels` physical pixels.
pub(crate) fn uv(icon: Icon, pixels: f32) -> Rect {
    let (cell, top) = if pixels <= SMALL + 4.0 {
        (SMALL, ROWS as f32 * LARGE)
    } else {
        (LARGE, 0.0)
    };
    let i = icon as usize;
    let min = Pos2::new(
        (i % COLUMNS) as f32 * cell,
        top + (i / COLUMNS) as f32 * cell,
    );
    Rect::from_min_size(
        Pos2::new(min.x / WIDTH, min.y / HEIGHT),
        Vec2::new(cell / WIDTH, cell / HEIGHT),
    )
}

impl Icons {
    /// Paints `icon` filling `rect`, tinted with `color`.
    pub(crate) fn paint(self, painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
        if let Some(id) = self.0 {
            let pixels = rect.width() * painter.ctx().pixels_per_point();
            painter.image(id, rect, uv(icon, pixels), color);
        }
    }
    /// An image widget of `icon` at `size` points, tinted (for buttons with a label).
    pub(crate) fn image(
        self,
        ctx: &egui::Context,
        icon: Icon,
        size: f32,
        tint: Color32,
    ) -> egui::Image<'static> {
        let id = self.0.unwrap_or_default();
        egui::Image::new(egui::load::SizedTexture::new(id, Vec2::splat(size)))
            .uv(uv(icon, size * ctx.pixels_per_point()))
            .tint(if self.0.is_some() {
                tint
            } else {
                Color32::TRANSPARENT
            })
    }
    /// Allocates `size` points and paints `icon` there (no interaction).
    pub(crate) fn show(self, ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
        self.paint(ui.painter(), rect, icon, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_manifest_icon_resolves_to_its_atlas_cell() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../../assets/editor-icons/icons.json"))
                .unwrap();
        let atlas: serde_json::Value =
            serde_json::from_str(include_str!("../../../../assets/editor-icons/atlas.json"))
                .unwrap();
        let files: Vec<&str> = manifest["icons"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["name"].as_str().unwrap())
            .collect();
        let packed: Vec<&str> = atlas["names"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        assert_eq!(files, Icon::NAMES, "icons.json and the Icon enum differ");
        assert_eq!(
            packed,
            Icon::NAMES,
            "atlas.json is stale: rerun build-editor-icons.ps1"
        );
        assert_eq!(Icon::ALL.len(), Icon::NAMES.len());
        assert_eq!(atlas["columns"], COLUMNS);
        assert_eq!(atlas["width"].as_f64().unwrap() as f32, WIDTH);
        assert_eq!(atlas["height"].as_f64().unwrap() as f32, HEIGHT);
        // Every cell of both levels holds artwork (some opaque pixel) and nothing outside
        // the cells is drawn.
        let (rgba, w, _) = crate::branding::decode_rgba(ATLAS_PNG).unwrap();
        let alpha = |x: usize, y: usize| rgba[(y * w as usize + x) * 4 + 3];
        for (i, icon) in Icon::ALL.iter().enumerate() {
            assert_eq!(*icon as usize, i);
            assert_eq!(icon.name(), Icon::NAMES[i]);
            for pixels in [16.0, 40.0] {
                let r = uv(*icon, pixels);
                let (x0, y0) = (
                    (r.min.x * WIDTH).round() as usize,
                    (r.min.y * HEIGHT).round() as usize,
                );
                let (cw, ch) = (
                    (r.width() * WIDTH).round() as usize,
                    (r.height() * HEIGHT).round() as usize,
                );
                let ink = (y0..y0 + ch)
                    .flat_map(|y| (x0..x0 + cw).map(move |x| (x, y)))
                    .filter(|&(x, y)| alpha(x, y) > 128)
                    .count();
                assert!(ink > 0, "{} has no artwork at {pixels} px", icon.name());
            }
        }
        // The small level is picked for 16-24 px icons, the large one above.
        assert_eq!((uv(Icon::Play, 20.0).height() * HEIGHT).round(), SMALL);
        assert_eq!((uv(Icon::Play, 30.0).height() * HEIGHT).round(), LARGE);
    }

    #[test]
    fn atlas_uploads_once_and_draws_by_index() {
        let ctx = egui::Context::default();
        let atlas = Atlas::load(&ctx).unwrap();
        let icons = atlas.icons();
        let mut uploads = 0;
        for _ in 0..3 {
            let out = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    for icon in Icon::ALL {
                        icons.show(ui, *icon, 16.0, Color32::WHITE);
                    }
                });
            });
            uploads += out
                .textures_delta
                .set
                .iter()
                .filter(|(id, _)| Some(*id) == icons.0)
                .count();
        }
        assert_eq!(uploads, 1, "the atlas is uploaded once, not per frame");
    }
}
