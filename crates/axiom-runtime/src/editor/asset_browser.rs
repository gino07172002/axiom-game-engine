//! Assets browser (the Assets dock tab): a kind / folder tree, search, a virtualized grid or
//! list with thumbnails, drag to the Scene (new object) or onto a typed reference field, New
//! asset dialogs, Find Usages and rename with reference fix-up. Every content change is an
//! `axiom_core::ops` call that ACL `asset place|create|rename` makes too; the list, usages and
//! per-cell thumbnail requests are cached per world version, never rebuilt per frame.
use super::thumbs::{self, Request, Shown, Thumbnails};
use super::{Action, Dock, Editor};
use axiom_core::assets::{self, AssetEntry, AssetKind, Owner, Usage};
use axiom_core::ops::{self, NewAsset};
use egui::{pos2, vec2, Color32, Rect, RichText, Sense, Ui};

const MUTED: Color32 = Color32::from_rgb(138, 154, 172);
const ACCENT: Color32 = Color32::from_rgb(105, 222, 195);
const WARNING: Color32 = Color32::from_rgb(242, 176, 92);
/// Grid cell: thumbnail square plus a label line.
const CELL: egui::Vec2 = egui::vec2(84.0, 98.0);
const LIST_ROW: f32 = 22.0;

/// Drag-and-drop payload of an asset cell (the Hierarchy drags entity IDs as `String`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AssetDrag {
    pub kind: AssetKind,
    pub id: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Filter {
    #[default]
    All,
    Kind(AssetKind),
    /// Source folder of model / image files.
    Folder(String),
}

/// The world's assets and everything drawn per cell, built once per world version.
struct Rows {
    version: u64,
    entries: Vec<AssetEntry>,
    /// Lower-case `kind id file` for search.
    haystack: Vec<String>,
    /// Source folder (files only).
    folder: Vec<Option<String>>,
    /// Grid label (ID, shortened) and list detail (`file · N refs`).
    label: Vec<String>,
    detail: Vec<String>,
    refs: Vec<usize>,
    requests: Vec<Option<Request>>,
    /// Tree rows: `All (n)`, one per kind, then folders `(folder, title)`.
    all_title: String,
    kind_titles: Vec<(AssetKind, String, usize)>,
    folders: Vec<(String, String)>,
}

impl Rows {
    fn build(world: &axiom_core::World, version: u64) -> Self {
        let entries = assets::list(world);
        let counts = assets::reference_counts(world);
        let mut rows = Rows {
            version,
            haystack: Vec::with_capacity(entries.len()),
            folder: Vec::with_capacity(entries.len()),
            label: Vec::with_capacity(entries.len()),
            detail: Vec::with_capacity(entries.len()),
            refs: Vec::with_capacity(entries.len()),
            requests: Vec::with_capacity(entries.len()),
            all_title: format!("All  {}", entries.len()),
            kind_titles: vec![],
            folders: vec![],
            entries: vec![],
        };
        let mut folders: std::collections::BTreeMap<String, usize> = Default::default();
        for e in &entries {
            let file = e.path.as_deref().map(assets::file_name);
            let refs = counts.get(&(e.kind, e.id.clone())).copied().unwrap_or(0);
            let folder = e.path.as_deref().map(|p| {
                let cut = p.len() - assets::file_name(p).len();
                p[..cut].trim_end_matches(['/', '\\']).to_owned()
            });
            if let Some(f) = &folder {
                *folders.entry(f.clone()).or_default() += 1;
            }
            rows.haystack
                .push(format!("{} {} {}", e.kind.name(), e.id, file.unwrap_or("")).to_lowercase());
            rows.label.push(if e.id.chars().count() > 14 {
                format!("{}…", e.id.chars().take(13).collect::<String>())
            } else {
                e.id.clone()
            });
            rows.detail.push(match file {
                Some(f) => format!("{f} · {refs} ref(s)"),
                None => format!("{refs} ref(s)"),
            });
            rows.refs.push(refs);
            rows.folder.push(folder);
            rows.requests.push(thumbs::request(world, e));
        }
        for kind in AssetKind::ALL {
            let n = entries.iter().filter(|e| e.kind == kind).count();
            rows.kind_titles
                .push((kind, format!("{}  {n}", kind.title()), n));
        }
        rows.folders = folders
            .into_iter()
            .map(|(f, n)| {
                // The last two folders are enough to tell packs apart.
                let parts: Vec<&str> = f.rsplit(['/', '\\']).take(2).collect();
                let short: Vec<&str> = parts.into_iter().rev().collect();
                let title = format!("{}  {n}", short.join("/"));
                (f, title)
            })
            .collect();
        rows.entries = entries;
        rows
    }

    fn matches(&self, i: usize, filter: &Filter, tokens: &[&str]) -> bool {
        let e = &self.entries[i];
        let keep = match filter {
            Filter::All => true,
            Filter::Kind(k) => e.kind == *k,
            Filter::Folder(f) => self.folder[i].as_deref() == Some(f.as_str()),
        };
        keep && tokens.iter().all(|t| self.haystack[i].contains(t))
    }

    fn find(&self, kind: AssetKind, id: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.kind == kind && e.id == id)
    }
}

/// Find Usages result of one asset (recomputed when the world changes while shown).
pub(crate) struct UsageView {
    pub kind: AssetKind,
    pub id: String,
    version: u64,
    pub rows: Vec<Usage>,
    lines: Vec<String>,
}

#[derive(Default)]
pub(crate) struct AssetBrowser {
    pub filter: Filter,
    pub query: String,
    /// List instead of grid.
    pub list: bool,
    pub selected: Option<(AssetKind, String)>,
    /// Rename being typed for the selected asset.
    pub rename: Option<String>,
    pub usages: Option<UsageView>,
    rows: Option<Rows>,
    /// Indices of the entries shown for (world version, filter, query).
    shown: Option<((u64, Filter, String), Vec<u32>)>,
    pub thumbs: Thumbnails,
    /// Times the row cache / the filtered list were rebuilt (tests: not per frame).
    pub builds: usize,
    pub filters: usize,
}

/// What a click in the browser asks for (applied after drawing).
enum Intent {
    Open(AssetKind, String),
    Place(AssetKind, String),
    Usages(AssetKind, String),
    Relist(AssetKind, String),
    Rename(AssetKind, String, String),
    Jump(Owner),
    CancelRename,
    CloseUsages,
    Command(&'static str),
}

fn badge(kind: AssetKind) -> (&'static str, Color32) {
    match kind {
        AssetKind::Model => ("GLB", Color32::from_rgb(120, 170, 255)),
        AssetKind::Image => ("PNG", Color32::from_rgb(232, 160, 220)),
        AssetKind::Material => ("MAT", Color32::from_rgb(105, 222, 195)),
        AssetKind::Shader => ("WGSL", Color32::from_rgb(160, 150, 255)),
        AssetKind::Prefab => ("PFB", Color32::from_rgb(255, 200, 110)),
        AssetKind::Machine => ("FSM", Color32::from_rgb(200, 140, 255)),
        AssetKind::Audio => ("SND", Color32::from_rgb(140, 220, 120)),
        AssetKind::Ui => ("UI", Color32::from_rgb(250, 140, 120)),
        AssetKind::Input => ("INPUT", Color32::from_rgb(200, 200, 200)),
    }
}

/// Where an asset dropped on the scene goes: the ground plane (y = 0) under the pointer, or
/// 6 m along the ray when it misses the ground (looking up / at the horizon). Millimetres.
pub(crate) fn drop_point(origin: glam::Vec3, direction: glam::Vec3) -> [f32; 3] {
    let t = -origin.y / direction.y;
    let hit = if direction.y < -1e-4 && t.is_finite() && t > 0.0 && t < 500.0 {
        origin + direction * t
    } else {
        origin + direction * 6.0
    };
    hit.to_array().map(|v| (v * 1000.0).round() / 1000.0)
}

/// `1.25,0,3` for an ACL `at=` (f32 `Display` round-trips through the ACL parser).
fn at_text(at: [f32; 3]) -> String {
    format!("{},{},{}", at[0], at[1], at[2])
}

impl Editor {
    /// Model / material thumbnails render on this device (the editor window's), from the
    /// thumbnail worker thread.
    pub(crate) fn set_gpu(&mut self, device: wgpu::Device, queue: wgpu::Queue) {
        self.assets.thumbs.gpu = Some((device, queue));
    }

    /// `<db stem>.thumbs/` next to the database (beside `<db stem>.editor-prefs.json`); none when
    /// preferences are not persisted.
    fn thumbs_dir(&self) -> Option<std::path::PathBuf> {
        let prefs = self.prefs_path.as_ref()?;
        let name = prefs.file_name()?.to_str()?;
        let stem = name.strip_suffix(".editor-prefs.json").unwrap_or(name);
        Some(prefs.with_file_name(format!("{stem}.thumbs")))
    }

    fn asset_rows(&mut self) {
        let version = self.world_version;
        if self
            .assets
            .rows
            .as_ref()
            .is_none_or(|r| r.version != version)
        {
            self.assets.rows = Some(Rows::build(&self.world, version));
            self.assets.builds += 1;
            if let Some((kind, id)) = &self.assets.selected {
                if !assets::exists(&self.world, *kind, id) {
                    self.assets.selected = None;
                    self.assets.rename = None;
                }
            }
            // A cancelled lazy job stays off until the world changes.
            self.assets.thumbs.resume_lazy();
        }
        if self.assets.thumbs.dir.is_none() {
            self.assets.thumbs.dir = self.thumbs_dir();
        }
    }

    /// Every frame (host): thumbnail results and the job outcome, drawn or not.
    pub(crate) fn poll_assets(&mut self) {
        self.assets.thumbs.poll();
    }

    /// Selects `kind:id` in the browser and lists what references it.
    pub(crate) fn find_usages(&mut self, kind: AssetKind, id: &str) -> anyhow::Result<()> {
        self.list_usages(kind, id, true)
    }

    /// `announce`: show the tab and the count (not when refreshing after a world change).
    fn list_usages(&mut self, kind: AssetKind, id: &str, announce: bool) -> anyhow::Result<()> {
        anyhow::ensure!(
            assets::exists(&self.world, kind, id),
            "no {} asset '{id}'",
            kind.name()
        );
        let rows = assets::usages(&self.world, kind, id);
        let lines = rows.iter().map(Usage::line).collect();
        self.assets.selected = Some((kind, id.to_owned()));
        self.assets.usages = Some(UsageView {
            kind,
            id: id.to_owned(),
            version: self.world_version,
            rows,
            lines,
        });
        if !announce {
            return Ok(());
        }
        self.show_dock(Dock::Assets);
        self.notice(format!(
            "{} usage(s) of {}:{id} (ACL: usages {}:{id})",
            self.assets.usages.as_ref().map_or(0, |u| u.rows.len()),
            kind.name(),
            kind.name()
        ));
        Ok(())
    }

    /// Assets ▸ Place in Scene / drop (`ACL: asset place KIND:ID [at=x,y,z] [scene=..]`; a
    /// prefab is the same instance as `instantiate PREFAB`). New objects land in the scene the
    /// Hierarchy is filtered to.
    pub(crate) fn place_asset(
        &mut self,
        kind: AssetKind,
        id: &str,
        at: Option<[f32; 3]>,
    ) -> Option<Action> {
        let result = self.placed(kind, id, at);
        self.created(result)
    }

    fn placed(
        &self,
        kind: AssetKind,
        id: &str,
        at: Option<[f32; 3]>,
    ) -> anyhow::Result<ops::Created> {
        let mut place = self.placement();
        place.position = at;
        ops::place_asset(&self.world, &self.registry, kind, id, &place)
    }

    /// An asset dropped on the Scene view at `at` (the ground point under the pointer).
    pub(crate) fn drop_asset(&mut self, drag: &AssetDrag, at: [f32; 3]) -> Option<Action> {
        if self.dirty() || self.previewing {
            self.set_error(
                "Stop Play and apply or reload the inspector draft before placing an asset",
            );
            return None;
        }
        let action = self.place_asset(drag.kind, &drag.id, Some(at))?;
        let scene = self
            .scene
            .as_deref()
            .map(|s| format!(" scene={s}"))
            .unwrap_or_default();
        self.notice(format!(
            "Placed {}:{} · ACL: asset place {}:{} at={}{scene}",
            drag.kind.name(),
            drag.id,
            drag.kind.name(),
            drag.id,
            at_text(at)
        ));
        Some(action)
    }

    /// Assets ▸ New (`ACL: asset create ...`); the new asset becomes the selection.
    pub(crate) fn create_asset(&mut self, new: &NewAsset) -> Option<Action> {
        let result = ops::create_asset(&self.world, &self.registry, new);
        let action = self.op_write(result, None)?;
        self.assets.selected = Some(new.target());
        self.assets.usages = None;
        self.show_dock(Dock::Assets);
        Some(action)
    }

    /// Assets ▸ Rename (`ACL: asset rename KIND:ID NEW`): one revision with every reference.
    /// Refusals become Problems.
    pub(crate) fn rename_asset(&mut self, kind: AssetKind, id: &str, new: &str) -> Option<Action> {
        match self.renamed(kind, id, new) {
            Ok(action) => action,
            Err(e) => {
                self.problem(&e);
                None
            }
        }
    }

    /// The rename write (`None` when the name is unchanged).
    fn renamed(&mut self, kind: AssetKind, id: &str, new: &str) -> anyhow::Result<Option<Action>> {
        let new = new.trim();
        let edits = ops::rename_asset(&self.world, kind, id, new)?;
        self.assets.rename = None;
        if edits.is_empty() {
            return Ok(None);
        }
        self.assets.selected = Some((kind, new.to_owned()));
        if self
            .assets
            .usages
            .as_ref()
            .is_some_and(|u| u.kind == kind && u.id == id)
        {
            self.assets.usages = None;
        }
        Ok(Some(self.write(edits, None)))
    }

    /// Opens the asset in the Inspector: prefabs and materials in their editors, the rest in
    /// the resource that holds them.
    fn open_asset(&mut self, kind: AssetKind, id: &str) {
        if self.dirty() {
            self.set_error("Apply or reload the inspector draft first");
            return;
        }
        match kind {
            AssetKind::Prefab => self.open_prefab(id),
            AssetKind::Material => self.open_material(id),
            _ => {
                let resource = kind
                    .singleton()
                    .or_else(|| kind.location().map(|(r, _)| r))
                    .expect("every other kind lives in a resource");
                self.open_resource(resource);
            }
        }
        self.show_dock(Dock::Inspector);
    }

    fn jump(&mut self, owner: Owner) {
        match owner {
            Owner::Entity(id) | Owner::Instance { entity: id, .. } => {
                self.try_select(Some(id));
                self.frame_selection = true;
            }
            Owner::Prefab(id) => self.open_asset(AssetKind::Prefab, &id),
            Owner::Resource(id) => {
                if self.dirty() {
                    self.set_error("Apply or reload the inspector draft first");
                } else {
                    self.open_resource(&id);
                    self.show_dock(Dock::Inspector);
                }
            }
        }
    }

    /// Why an `asset.*` command cannot run now.
    pub(crate) fn asset_command_blocked(&self, id: &str) -> Option<&'static str> {
        let busy = self.dirty() || self.drag.is_some();
        match id {
            "asset.cancel_thumbnails" if self.assets.thumbs.running().is_none() => {
                Some("no thumbnail job is running")
            }
            "asset.thumbnails" if self.assets.thumbs.explicit_running() => {
                Some("thumbnails are being generated")
            }
            "asset.new_ui_document" if self.world.resources.contains_key("ui.document") => {
                Some("the world already has its ui.document")
            }
            "asset.new_input_map" if self.world.resources.contains_key("input.bindings") => {
                Some("the world already has its input.bindings")
            }
            "asset.place"
            | "asset.rename"
            | "asset.new_material"
            | "asset.new_prefab"
            | "asset.new_machine"
            | "asset.new_ui_document"
            | "asset.new_input_map"
                if busy =>
            {
                Some("apply or reload the inspector draft first")
            }
            _ => None,
        }
    }

    /// Defaults of a New asset command run without its dialog (bridge, shortcut).
    pub(crate) fn default_new_asset(&self, id: &str) -> Option<NewAsset> {
        Some(match id {
            "asset.new_material" => NewAsset::Material {
                id: ops::free_asset_id(&self.world, AssetKind::Material, "material"),
                shader: None,
                tint: None,
            },
            "asset.new_prefab" => NewAsset::Prefab {
                id: ops::free_asset_id(&self.world, AssetKind::Prefab, "prefab"),
                from: None,
            },
            "asset.new_machine" => NewAsset::Machine {
                id: ops::free_asset_id(&self.world, AssetKind::Machine, "machine"),
                initial: None,
            },
            "asset.new_ui_document" => NewAsset::Ui { locale: None },
            "asset.new_input_map" => NewAsset::Input,
            _ => return None,
        })
    }

    /// `asset.*` commands (menu, palette, `editor_view {command, arg}`).
    pub(crate) fn asset_command(
        &mut self,
        id: &str,
        arg: Option<&str>,
    ) -> anyhow::Result<Vec<Action>> {
        let arg = arg.map(str::trim).filter(|a| !a.is_empty());
        let mut actions = vec![];
        if let Some(new) = self.default_new_asset(id) {
            anyhow::ensure!(arg.is_none(), "{id} takes no argument");
            actions.extend(self.create_asset(&new));
            return Ok(actions);
        }
        match (id, arg) {
            ("asset.thumbnails", None) => {
                self.asset_rows();
                let rows = self.assets.rows.as_ref().expect("built above");
                let requests: Vec<Request> = rows
                    .requests
                    .iter()
                    .flatten()
                    .take(thumbs::EXPLICIT_LIMIT)
                    .cloned()
                    .collect();
                let n = requests.len();
                self.assets.thumbs.start(requests, true)?;
                self.show_dock(Dock::Assets);
                self.notice(format!("Generating {n} thumbnail(s) in the background"));
            }
            ("asset.cancel_thumbnails", None) => {
                self.assets.thumbs.cancel();
                self.notice("Cancelling the thumbnail job");
            }
            ("asset.place" | "asset.find_usages" | "asset.rename", None) => {
                self.show_dock(Dock::Assets);
                self.notice(
                    "Pick the asset in the Assets tab (drag it onto the Scene to place it)",
                );
            }
            ("asset.place", Some(arg)) => {
                let (reference, at) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
                let (kind, asset) = assets::parse_reference(reference)?;
                let at = if at.trim().is_empty() {
                    None
                } else {
                    let v: Vec<f32> = at
                        .trim()
                        .trim_matches(['[', ']'])
                        .split(',')
                        .map(|p| p.trim().parse::<f32>())
                        .collect::<Result<_, _>>()
                        .map_err(|_| anyhow::anyhow!("position must be x,y,z"))?;
                    anyhow::ensure!(v.len() == 3, "position must be x,y,z");
                    Some([v[0], v[1], v[2]])
                };
                match self.placed(kind, &asset, at) {
                    Ok(created) => {
                        let first = created.ids.first().cloned();
                        actions.push(self.write(created.edits, first));
                    }
                    Err(e) => {
                        self.problem(&e);
                        return Err(e);
                    }
                }
            }
            ("asset.find_usages", Some(arg)) => {
                let (kind, asset) = assets::parse_reference(arg)?;
                self.find_usages(kind, &asset)?;
            }
            ("asset.rename", Some(arg)) => {
                let (reference, new) = arg
                    .split_once(char::is_whitespace)
                    .ok_or_else(|| anyhow::anyhow!("use: KIND:ID NEW"))?;
                let (kind, asset) = assets::parse_reference(reference)?;
                match self.renamed(kind, &asset, new) {
                    Ok(action) => actions.extend(action),
                    Err(e) => {
                        self.problem(&e);
                        return Err(e);
                    }
                }
            }
            (other, Some(_)) => anyhow::bail!("{other} takes no argument"),
            (other, None) => anyhow::bail!("unknown asset command {other}"),
        }
        Ok(actions)
    }

    /// Running thumbnail job for `observe` (`editor.jobs`) and the status bar.
    pub(crate) fn asset_job_observation(&self) -> Option<serde_json::Value> {
        self.assets.thumbs.running().map(|j| j.observation())
    }

    /// The Assets tab body.
    pub(super) fn asset_browser(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        self.asset_rows();
        let mut b = std::mem::take(&mut self.assets);
        b.thumbs.begin();
        let busy = self.dirty() || self.previewing;
        let mut intents: Vec<Intent> = vec![];
        #[cfg(test)]
        let mut rects: Vec<(String, Rect)> = vec![];
        // Widget rects for the egui tests; nothing (not even the key) is built otherwise.
        macro_rules! rect {
            ($key:expr, $r:expr) => {
                #[cfg(test)]
                rects.push(($key, $r));
                #[cfg(not(test))]
                let _ = &$r;
            };
        }
        // Toolbar: search, grid / list, New, thumbnails.
        ui.horizontal(|ui| {
            let search = ui.add(
                egui::TextEdit::singleline(&mut b.query)
                    .hint_text("Search assets")
                    .desired_width(180.0),
            );
            rect!("assets:search".into(), search.rect);
            if ui.selectable_label(!b.list, "Grid").clicked() {
                b.list = false;
            }
            if ui.selectable_label(b.list, "List").clicked() {
                b.list = true;
            }
            ui.add_enabled_ui(!busy, |ui| {
                ui.menu_button("New…", |ui| {
                    for (command, title) in [
                        ("asset.new_material", "Material…"),
                        ("asset.new_prefab", "Prefab…"),
                        ("asset.new_machine", "FSM Machine…"),
                        ("asset.new_ui_document", "UI Document…"),
                        ("asset.new_input_map", "Input Map…"),
                    ] {
                        let blocked = self.asset_command_blocked(command);
                        if ui
                            .add_enabled(blocked.is_none(), egui::Button::new(title))
                            .on_disabled_hover_text(blocked.unwrap_or_default())
                            .clicked()
                        {
                            intents.push(Intent::Command(command));
                            ui.close_menu();
                        }
                    }
                });
            });
            let generate = ui
                .add_enabled(
                    !b.thumbs.explicit_running(),
                    egui::Button::new("Thumbnails"),
                )
                .on_hover_text("Generate every model / image / material thumbnail in the background (cached by content hash)");
            rect!("assets:thumbnails".into(), generate.rect);
            if generate.clicked() {
                intents.push(Intent::Command("asset.thumbnails"));
            }
            if let Some(job) = b.thumbs.running() {
                let (done, total) = job.progress();
                ui.add(
                    egui::ProgressBar::new(if total == 0 {
                        0.0
                    } else {
                        done as f32 / total as f32
                    })
                    .desired_width(120.0)
                    .text(format!("{done}/{total}")),
                );
                let cancel = ui.small_button("Cancel");
                rect!("assets:cancel".into(), cancel.rect);
                if cancel.clicked() {
                    intents.push(Intent::Command("asset.cancel_thumbnails"));
                }
            }
        });
        let rows = b.rows.take().expect("asset_rows built it");
        rect!("assets:clip".into(), ui.clip_rect());
        // Tree: all, kinds, folders.
        egui::SidePanel::left("asset_tree")
            .resizable(true)
            .default_width(150.0)
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("asset_tree_scroll")
                    .show(ui, |ui| {
                        if ui
                            .selectable_label(b.filter == Filter::All, &rows.all_title)
                            .clicked()
                        {
                            b.filter = Filter::All;
                        }
                        for (kind, title, n) in &rows.kind_titles {
                            let text = RichText::new(title).color(if *n == 0 {
                                MUTED
                            } else {
                                Color32::LIGHT_GRAY
                            });
                            let r = ui.selectable_label(b.filter == Filter::Kind(*kind), text);
                            rect!(format!("assets:kind:{}", kind.name()), r.rect);
                            if r.clicked() {
                                b.filter = Filter::Kind(*kind);
                            }
                        }
                        if !rows.folders.is_empty() {
                            ui.label(RichText::new("FOLDERS").small().color(MUTED));
                            for (folder, title) in &rows.folders {
                                let on = matches!(&b.filter, Filter::Folder(f) if f == folder);
                                if ui
                                    .selectable_label(on, RichText::new(title).small())
                                    .on_hover_text(folder)
                                    .clicked()
                                {
                                    b.filter = Filter::Folder(folder.clone());
                                }
                            }
                        }
                    });
            });
        // Selection details, rename and Find Usages.
        let selected = b.selected.as_ref().and_then(|(k, id)| rows.find(*k, id));
        if let Some(i) = selected {
            let entry = &rows.entries[i];
            if b.usages.as_ref().is_some_and(|u| {
                u.version != rows.version && u.kind == entry.kind && u.id == entry.id
            }) {
                // The world changed while the list was shown: list it again.
                intents.push(Intent::Relist(entry.kind, entry.id.clone()));
            }
            // Beside the cells: the Assets tab is usually a wide, short bottom dock.
            egui::SidePanel::right("asset_details")
                .resizable(true)
                .default_width(380.0)
                .show_inside(ui, |ui| {
                    // A short dock scrolls the rename row and its ACL line.
                    egui::ScrollArea::vertical()
                        .id_salt("asset_details_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                let (text, color) = badge(entry.kind);
                                ui.label(RichText::new(text).monospace().strong().color(color));
                                ui.label(RichText::new(&entry.id).monospace().color(ACCENT));
                                let detail =
                                    ui.label(RichText::new(&rows.detail[i]).small().color(MUTED));
                                if let Some(path) = &entry.path {
                                    detail.on_hover_text(path);
                                }
                            });
                            ui.horizontal_wrapped(|ui| {
                                if ui.add_enabled(!busy, egui::Button::new("Open")).clicked() {
                                    intents.push(Intent::Open(entry.kind, entry.id.clone()));
                                }
                                let place = ui
                                    .add_enabled(
                                        !busy && entry.kind.placeable(),
                                        egui::Button::new("Place in Scene"),
                                    )
                                    .on_hover_text("Or drag the asset onto the Scene view");
                                rect!("assets:place".into(), place.rect);
                                if place.clicked() {
                                    intents.push(Intent::Place(entry.kind, entry.id.clone()));
                                }
                                let usages = ui.button("Find Usages");
                                rect!("assets:usages".into(), usages.rect);
                                if usages.clicked() {
                                    intents.push(Intent::Usages(entry.kind, entry.id.clone()));
                                }
                                let renameable = entry.kind.singleton().is_none();
                                let rename = ui.add_enabled(
                                    !busy && renameable && b.rename.is_none(),
                                    egui::Button::new("Rename…"),
                                );
                                rect!("assets:rename".into(), rename.rect);
                                if rename.clicked() {
                                    b.rename = Some(entry.id.clone());
                                }
                            });
                            if let Some(text) = &mut b.rename {
                                ui.horizontal(|ui| {
                                    let edit = ui
                                        .add(egui::TextEdit::singleline(text).desired_width(200.0));
                                    rect!("assets:rename_text".into(), edit.rect);
                                    let enter = edit.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                    let apply = ui.add_enabled(!busy, egui::Button::new("Apply"));
                                    rect!("assets:rename_apply".into(), apply.rect);
                                    if apply.clicked() || enter {
                                        intents.push(Intent::Rename(
                                            entry.kind,
                                            entry.id.clone(),
                                            text.clone(),
                                        ));
                                    }
                                    if ui.button("Cancel").clicked() {
                                        intents.push(Intent::CancelRename);
                                    }
                                });
                                ui.label(
                                    RichText::new(format!(
                                "ACL: asset rename {}:{} {}  (fixes {} reference(s), one revision)",
                                entry.kind.name(),
                                entry.id,
                                text.trim(),
                                rows.refs[i]
                            ))
                                    .monospace()
                                    .small()
                                    .weak(),
                                );
                            }
                        });
                });
            // Find Usages: its own column left of the details (virtualized rows).
            if let Some(view) = b
                .usages
                .as_ref()
                .filter(|u| u.kind == entry.kind && u.id == entry.id)
            {
                egui::SidePanel::right("asset_usages")
                    .resizable(true)
                    .default_width(380.0)
                    .show_inside(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{} usage(s) · ACL: usages {}:{}",
                                    view.rows.len(),
                                    view.kind.name(),
                                    view.id
                                ))
                                .small()
                                .color(MUTED),
                            );
                            if ui.small_button("Close").clicked() {
                                intents.push(Intent::CloseUsages);
                            }
                        });
                        let height = ui.text_style_height(&egui::TextStyle::Small) + 6.0;
                        egui::ScrollArea::vertical()
                            .id_salt("asset_usages_scroll")
                            .auto_shrink([false, false])
                            .show_rows(ui, height, view.rows.len(), |ui, range| {
                                for n in range {
                                    let r = ui.add(
                                        egui::Label::new(
                                            RichText::new(&view.lines[n]).monospace().small(),
                                        )
                                        .sense(Sense::click()),
                                    );
                                    rect!(format!("usage:{n}"), r.rect);
                                    if r.clicked() {
                                        intents.push(Intent::Jump(view.rows[n].owner.clone()));
                                    }
                                }
                            });
                    });
            }
        }
        // Cells: the filtered entries, virtualized (only visible rows are laid out).
        // Compared in place: nothing is built unless the version, filter or query changed.
        let stale = b
            .shown
            .as_ref()
            .is_none_or(|((version, filter, query), _)| {
                *version != rows.version || *filter != b.filter || *query != b.query
            });
        if stale {
            let lower = b.query.to_lowercase();
            let tokens: Vec<&str> = lower.split_whitespace().collect();
            let shown = (0..rows.entries.len())
                .filter(|&i| rows.matches(i, &b.filter, &tokens))
                .map(|i| i as u32)
                .collect();
            b.shown = Some(((rows.version, b.filter.clone(), b.query.clone()), shown));
            b.filters += 1;
        }
        let shown = &b.shown.as_ref().expect("filled above").1;
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().show_inside(ui, |ui| {
            if rows.entries.is_empty() {
                ui.label(RichText::new("This world names no assets yet: New… creates a material, prefab, FSM machine, UI document or input map.").small().color(MUTED));
            } else if shown.is_empty() {
                ui.label(RichText::new("No asset matches the search / filter.").small().color(MUTED));
            }
            let pointer = ui.input(|i| i.pointer.latest_pos());
            let mut cell = |ui: &mut Ui, i: usize, size: egui::Vec2, list: bool| {
                let entry = &rows.entries[i];
                let (r, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
                rect!(format!("asset:{}:{}", entry.kind.name(), entry.id), r);
                let is_selected = selected == Some(i);
                if ui.is_rect_visible(r) {
                    let painter = ui.painter();
                    if is_selected {
                        painter.rect_filled(r, 4.0, Color32::from_rgba_unmultiplied(105, 222, 195, 40));
                    } else if response.hovered() {
                        painter.rect_filled(r, 4.0, Color32::from_white_alpha(10));
                    }
                    let (text, color) = badge(entry.kind);
                    let square = if list {
                        Rect::from_min_size(r.min + vec2(2.0, 2.0), vec2(size.y - 4.0, size.y - 4.0))
                    } else {
                        Rect::from_min_size(r.min + vec2(6.0, 4.0), vec2(size.x - 12.0, size.x - 12.0))
                    };
                    let thumb = b.thumbs.shown(&ctx, rows.requests[i].as_ref());
                    match thumb {
                        Shown::Texture(t) => {
                            painter.image(
                                t.id(),
                                square,
                                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        other => {
                            painter.rect_filled(square, 6.0, Color32::from_rgb(28, 34, 42));
                            painter.text(
                                square.center(),
                                egui::Align2::CENTER_CENTER,
                                text,
                                egui::FontId::monospace(if list { 8.0 } else { 15.0 }),
                                color,
                            );
                            let mark = match other {
                                Shown::Pending => Some(MUTED),
                                Shown::Failed => Some(WARNING),
                                _ => None,
                            };
                            if let (Some(mark), false) = (mark, list) {
                                painter.circle_filled(square.right_top() + vec2(-8.0, 8.0), 3.5, mark);
                            }
                        }
                    }
                    if list {
                        painter.text(
                            pos2(square.right() + 8.0, r.center().y),
                            egui::Align2::LEFT_CENTER,
                            &entry.id,
                            egui::FontId::monospace(12.0),
                            Color32::LIGHT_GRAY,
                        );
                        painter.text(
                            pos2(r.right() - 6.0, r.center().y),
                            egui::Align2::RIGHT_CENTER,
                            &rows.detail[i],
                            egui::FontId::proportional(11.0),
                            MUTED,
                        );
                    } else {
                        painter.text(
                            pos2(r.center().x, square.bottom() + 9.0),
                            egui::Align2::CENTER_CENTER,
                            &rows.label[i],
                            egui::FontId::proportional(11.5),
                            Color32::LIGHT_GRAY,
                        );
                    }
                }
                if response.drag_started_by(egui::PointerButton::Primary) {
                    egui::DragAndDrop::set_payload(
                        ui.ctx(),
                        AssetDrag {
                            kind: entry.kind,
                            id: entry.id.clone(),
                        },
                    );
                }
                if response.dragged_by(egui::PointerButton::Primary) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    if let Some(p) = pointer {
                        ui.ctx()
                            .layer_painter(egui::LayerId::new(
                                egui::Order::Tooltip,
                                egui::Id::new("asset_drag_ghost"),
                            ))
                            .text(
                                p + vec2(14.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                &entry.id,
                                egui::FontId::proportional(13.0),
                                ACCENT,
                            );
                    }
                }
                if (response.clicked() || response.drag_started())
                    && b.selected.as_ref().is_none_or(|(k, id)| *k != entry.kind || *id != entry.id)
                {
                    b.selected = Some((entry.kind, entry.id.clone()));
                    b.rename = None;
                }
                if response.double_clicked() {
                    intents.push(Intent::Open(entry.kind, entry.id.clone()));
                }
                let response = if response.hovered() && !response.dragged() {
                    let failure = b
                        .thumbs
                        .failure(rows.requests[i].as_ref())
                        .map(|e| format!("\nThumbnail failed: {e}"))
                        .unwrap_or_default();
                    response.on_hover_text(match &entry.path {
                        Some(p) => format!("{} · {}\n{p}\nDrag onto the Scene or a reference field{failure}", entry.reference(), rows.detail[i]),
                        None => format!("{} · {}\nDrag onto the Scene or a reference field{failure}", entry.reference(), rows.detail[i]),
                    })
                } else {
                    response
                };
                response.context_menu(|ui| {
                    if ui.add_enabled(!busy, egui::Button::new("Open")).clicked() {
                        intents.push(Intent::Open(entry.kind, entry.id.clone()));
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(!busy && entry.kind.placeable(), egui::Button::new("Place in Scene"))
                        .clicked()
                    {
                        intents.push(Intent::Place(entry.kind, entry.id.clone()));
                        ui.close_menu();
                    }
                    if ui.button("Find Usages").clicked() {
                        intents.push(Intent::Usages(entry.kind, entry.id.clone()));
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(!busy && entry.kind.singleton().is_none(), egui::Button::new("Rename…"))
                        .clicked()
                    {
                        b.selected = Some((entry.kind, entry.id.clone()));
                        b.rename = Some(entry.id.clone());
                        ui.close_menu();
                    }
                    if ui.button("Copy reference").clicked() {
                        ui.ctx().copy_text(entry.reference());
                        ui.close_menu();
                    }
                });
            };
            if b.list {
                egui::ScrollArea::vertical()
                    .id_salt("asset_list")
                    .auto_shrink([false, false])
                    .show_rows(ui, LIST_ROW, shown.len(), |ui, range| {
                        let width = ui.available_width();
                        for n in range {
                            cell(ui, shown[n] as usize, vec2(width, LIST_ROW), true);
                        }
                    });
            } else {
                let spacing = ui.spacing().item_spacing.x;
                let columns = (((ui.available_width() + spacing) / (CELL.x + spacing)).floor() as usize).max(1);
                let lines = shown.len().div_ceil(columns);
                egui::ScrollArea::vertical()
                    .id_salt("asset_grid")
                    .auto_shrink([false, false])
                    .show_rows(ui, CELL.y, lines, |ui, range| {
                        for line in range {
                            ui.horizontal(|ui| {
                                for c in 0..columns {
                                    if let Some(&i) = shown.get(line * columns + c) {
                                        cell(ui, i as usize, CELL, false);
                                    }
                                }
                            });
                        }
                    });
            }
        });
        b.thumbs.end();
        b.rows = Some(rows);
        self.assets = b;
        #[cfg(test)]
        self.rects.extend(rects);
        for intent in intents {
            match intent {
                Intent::Open(kind, id) => self.open_asset(kind, &id),
                Intent::Place(kind, id) => actions.extend(self.place_asset(kind, &id, None)),
                Intent::Relist(kind, id) => {
                    if let Err(e) = self.list_usages(kind, &id, false) {
                        self.set_error(format!("{e:#}"));
                    }
                }
                Intent::Usages(kind, id) => {
                    if let Err(e) = self.find_usages(kind, &id) {
                        self.set_error(format!("{e:#}"));
                    }
                }
                Intent::Rename(kind, id, new) => actions.extend(self.rename_asset(kind, &id, &new)),
                Intent::Jump(owner) => self.jump(owner),
                Intent::CancelRename => self.assets.rename = None,
                Intent::CloseUsages => self.assets.usages = None,
                Intent::Command(id) => self.invoke(id, actions),
            }
        }
    }
}
