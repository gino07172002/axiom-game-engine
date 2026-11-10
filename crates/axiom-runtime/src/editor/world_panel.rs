//! Window ▸ World (world id/title, capabilities, systems by phase, resources) and the
//! Inspector's tag chips. Every write goes through `axiom_core::ops`, like the ACL verbs
//! `world`, `capability`, `system`, `resource`, `tag`, `add` and `event declare`. The rows
//! both draw are rebuilt only when the revision (or the staged world) changes, never per frame.
use super::{Action, Editor};
use axiom_core::{ops, World};
use egui::{RichText, Ui};
use serde_json::Value;

const MUTED: egui::Color32 = egui::Color32::from_rgb(155, 174, 194);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(105, 222, 195);
const WARNING: egui::Color32 = egui::Color32::from_rgb(255, 178, 111);

/// Identity of the world a cache was built from: the revision and the editor's world version
/// (a staged write shows a new world at the same revision).
type WorldKey = (i64, u64);

struct Row {
    id: String,
    /// Display text built with the cache (no formatting while drawing).
    text: String,
    /// Why the row cannot be removed, if it cannot.
    locked: Option<String>,
}

/// Rows of the World window for one world.
struct WorldRows {
    key: WorldKey,
    capabilities: Vec<Row>,
    missing_capabilities: Vec<String>,
    /// Scheduled systems sorted by (phase, id).
    systems: Vec<Row>,
    /// Registered systems the world does not schedule, as (id, "phase · capability").
    missing_systems: Vec<(String, String)>,
    resources: Vec<Row>,
    missing_resources: Vec<String>,
    heading: [String; 3],
}

impl WorldRows {
    fn build(key: WorldKey, world: &World, registry: &axiom_core::Registry) -> Self {
        let capabilities: Vec<Row> = world
            .capabilities
            .iter()
            .map(|(id, version)| {
                let users = ops::capability_users(world, registry, id).len();
                Row {
                    id: id.clone(),
                    text: format!("{id}  v{version} · {users} use(s)"),
                    locked: if id == "core" {
                        Some("the core capability is required".into())
                    } else if users > 0 {
                        Some(format!(
                            "used by {users} component(s)/resource(s)/system(s); remove those first"
                        ))
                    } else {
                        None
                    },
                }
            })
            .collect();
        let missing_capabilities = registry
            .capabilities
            .keys()
            .filter(|c| !world.capabilities.contains_key(*c))
            .cloned()
            .collect();
        let mut scheduled: Vec<(u32, &String)> = world
            .systems
            .iter()
            .map(|s| (registry.systems.get(s).map_or(u32::MAX, |d| d.phase), s))
            .collect();
        scheduled.sort();
        let systems = scheduled
            .into_iter()
            .map(|(phase, id)| {
                let d = registry.systems.get(id);
                let dependents: Vec<&str> = world
                    .systems
                    .iter()
                    .filter(|s| {
                        registry
                            .systems
                            .get(s.as_str())
                            .is_some_and(|d| d.after.iter().any(|a| a == id))
                    })
                    .map(String::as_str)
                    .collect();
                Row {
                    id: id.clone(),
                    text: match d {
                        Some(d) => format!("{phase:>3}  {id}  ({})", d.capability),
                        None => format!("  ?  {id}  (not registered)"),
                    },
                    locked: (!dependents.is_empty())
                        .then(|| format!("{} must run after it", dependents.join(", "))),
                }
            })
            .collect();
        let mut missing: Vec<&axiom_core::SystemDescriptor> = registry
            .systems
            .values()
            .filter(|d| !world.systems.contains(&d.id))
            .collect();
        missing.sort_by(|a, b| (a.phase, &a.id).cmp(&(b.phase, &b.id)));
        let missing_systems = missing
            .into_iter()
            .map(|d| (d.id.clone(), format!("{} · {}", d.phase, d.capability)))
            .collect();
        let resources = world
            .resources
            .iter()
            .map(|(id, value)| {
                let capability = registry
                    .resources
                    .get(id)
                    .map_or("world schema", |d| d.capability.as_str());
                let bytes = serde_json::to_vec(value).map_or(0, |v| v.len());
                Row {
                    id: id.clone(),
                    text: format!("{capability} · {bytes} B"),
                    locked: None,
                }
            })
            .collect();
        let missing_resources = registry
            .resources
            .keys()
            .chain(world.resource_schemas.keys())
            .filter(|r| !world.resources.contains_key(*r))
            .cloned()
            .collect();
        Self {
            key,
            heading: [
                format!("CAPABILITIES · {}", world.capabilities.len()),
                format!("SYSTEMS · {} · by phase", world.systems.len()),
                format!("RESOURCES · {}", world.resources.len()),
            ],
            capabilities,
            missing_capabilities,
            systems,
            missing_systems,
            resources,
            missing_resources,
        }
    }
}

/// Window ▸ World state (the World tab; whether it is open lives in the dock layout).
#[derive(Default)]
pub(crate) struct WorldPanel {
    rows: Option<WorldRows>,
    /// Chosen in the "add" combo boxes.
    pub(super) capability: String,
    pub(super) system: String,
    pub(super) resource: String,
    /// JSON for the resource being created (prefilled with its default).
    resource_json: String,
    id: String,
    title: String,
    /// The world the id/title fields were loaded from (reloaded when it changes).
    meta_key: Option<WorldKey>,
    /// The world id/title last loaded into the fields.
    loaded: (String, String),
    /// Row rebuilds so far (tests: the rows are not rebuilt per frame).
    #[cfg(test)]
    pub builds: usize,
}

impl WorldPanel {
    /// Drops the cached rows (e.g. after the registry changed).
    pub fn invalidate(&mut self) {
        self.rows = None;
    }
}

enum WorldRequest {
    SetId(String),
    SetTitle(String),
    CapabilityAdd(String),
    CapabilityRemove(String),
    SystemAdd(String),
    SystemRemove(String),
    ResourceCreate(String, String),
    ResourceRemove(String),
    ResourceOpen(String),
}

/// Inspector tag chips: the tag being typed and the chips of the current selection.
#[derive(Default)]
pub(crate) struct TagUi {
    pub input: String,
    cache: Option<TagCache>,
    /// Cache rebuilds so far (tests).
    #[cfg(test)]
    pub builds: usize,
}

struct TagCache {
    key: WorldKey,
    primary: String,
    extra: Vec<String>,
    /// Union of the selection's tags (primary's first): (tag, chip text, on every object).
    chips: Vec<(String, String, bool)>,
    objects: usize,
    caption: String,
    /// Every tag the world uses, for suggestions.
    known: Vec<String>,
}

/// What the tag chips asked for this frame.
pub(super) struct TagRequest {
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

impl Editor {
    /// Brings the tag-chip cache up to date with the revision and selection (cheap check).
    pub(super) fn refresh_tag_cache(&mut self) {
        let Some(primary) = self.selected.as_deref() else {
            self.tag_ui.cache = None;
            return;
        };
        let key: WorldKey = (self.rev, self.world_version);
        let fresh = self.tag_ui.cache.as_ref().is_some_and(|c| {
            c.key == key
                && c.primary == primary
                && c.extra.len() == self.extra.len()
                && c.extra.iter().zip(&self.extra).all(|(a, b)| a == b)
        });
        if fresh {
            return;
        }
        let ids = self.selection();
        let mut chips: Vec<(String, String, bool)> = vec![];
        let mut counts: Vec<usize> = vec![];
        for id in &ids {
            for tag in self.world.entity(id).iter().flat_map(|e| &e.tags) {
                match chips.iter().position(|(t, _, _)| t == tag) {
                    Some(i) => counts[i] += 1,
                    None => {
                        chips.push((tag.clone(), String::new(), false));
                        counts.push(1);
                    }
                }
            }
        }
        for ((tag, text, all), n) in chips.iter_mut().zip(counts) {
            *all = n == ids.len();
            *text = if *all {
                tag.clone()
            } else {
                format!("{tag} {n}/{}", ids.len())
            };
        }
        #[cfg(test)]
        {
            self.tag_ui.builds += 1;
        }
        self.tag_ui.cache = Some(TagCache {
            key,
            primary: primary.to_owned(),
            extra: self.extra.iter().cloned().collect(),
            chips,
            objects: ids.len(),
            caption: if ids.len() > 1 {
                format!("TAGS · {} OBJECTS", ids.len())
            } else {
                "TAGS".into()
            },
            known: ops::known_tags(&self.world).into_iter().collect(),
        });
    }

    /// Inspector tag chips / `object.tag "+a -b"` (`ACL: tag SEL +a -b`) on the selection.
    pub(super) fn edit_tags(&mut self, add: &[String], remove: &[String]) -> Option<Action> {
        let result = ops::edit_tags(&self.world, &self.selection(), add, remove);
        match result {
            Ok(edits) if edits.is_empty() => {
                self.notice("Tags already as requested");
                None
            }
            result => self.op_write(result, None),
        }
    }

    /// Inspector ▸ Add component / `object.add_component` (`ACL: add SEL COMPONENT`): the
    /// component's default plus the capability and systems it needs, then opened in the
    /// Inspector. A component without a default opens as JSON to type (Stage component).
    pub(super) fn add_component(&mut self, component: &str) -> Option<Action> {
        let ids = self.selection();
        match ops::add_component(&self.world, &self.registry, &ids, component, None) {
            Ok(edits) => {
                self.open_component_after = Some(component.to_owned());
                Some(self.write(edits, None))
            }
            Err(error)
                if error
                    .downcast_ref::<axiom_core::Diagnostic>()
                    .is_some_and(|d| d.code == "no_default") =>
            {
                let world = self.world.clone();
                let entity = self.selected.as_deref().and_then(|id| world.entity(id));
                if let (Some(draft), Some(entity)) = (self.draft.as_mut(), entity) {
                    draft.open_component(&world, entity, component);
                    draft.json_mode = true;
                }
                self.notice(format!(
                    "{component} has no default: type its JSON, press Stage component, then Apply (its capability and systems are added with it)"
                ));
                None
            }
            Err(error) => {
                self.problem(&error);
                None
            }
        }
    }

    /// Problems ▸ Declare event / `event.declare` (`ACL: event declare NAME`). An Inspector
    /// draft waiting for the event (its Apply was refused) is written in the same revision,
    /// after the declaration, so it is not lost when the write is shown.
    pub(super) fn declare_event(&mut self, name: &str) -> Option<Action> {
        let mut edits = match ops::declare_event(&self.world, name, &[]) {
            Ok(edits) if edits.is_empty() => {
                self.notice(format!("Event {name} is already declared"));
                return None;
            }
            Ok(edits) => edits,
            Err(error) => {
                self.problem(&error);
                return None;
            }
        };
        let waiting = self
            .draft
            .as_ref()
            .filter(|d| d.dirty() && d.rev == self.rev);
        let applied = waiting.is_some();
        if let Some(draft) = waiting {
            let mut declared = (*self.world).clone();
            let more = edits
                .iter()
                .try_for_each(|e| e.clone().apply(&mut declared, &self.registry))
                .and_then(|_| {
                    super::inspector_edits(&declared, &self.registry, draft, &self.extra)
                });
            match more {
                Ok(more) => edits.extend(more),
                Err(error) => {
                    self.problem(&error);
                    return None;
                }
            }
        }
        self.notice(if applied {
            format!("Declared event {name} and applied the Inspector draft")
        } else {
            format!("Declared event {name}")
        });
        Some(self.write(edits, None))
    }
    fn world_write(&mut self, result: anyhow::Result<Vec<axiom_core::Edit>>) -> Option<Action> {
        match result {
            Ok(edits) if edits.is_empty() => {
                self.notice("World already as requested");
                None
            }
            result => self.op_write(result, None),
        }
    }

    fn world_request(&mut self, request: WorldRequest) -> Option<Action> {
        let registry = self.registry.clone();
        let world = self.world.clone();
        let result = match request {
            WorldRequest::SetId(id) => ops::set_world_id(&world, id.trim()),
            WorldRequest::SetTitle(title) => ops::set_world_title(&world, &title),
            WorldRequest::CapabilityAdd(id) => ops::add_capability(&world, &registry, &id),
            WorldRequest::CapabilityRemove(id) => ops::remove_capability(&world, &registry, &id),
            WorldRequest::SystemAdd(id) => ops::add_system(&world, &registry, &id),
            WorldRequest::SystemRemove(id) => ops::remove_system(&world, &registry, &id),
            WorldRequest::ResourceCreate(id, json) => {
                let value = if json.trim().is_empty() {
                    None
                } else {
                    match serde_json::from_str::<Value>(&json) {
                        Ok(v) => Some(v),
                        Err(e) => {
                            self.set_error(format!("Resource JSON is invalid: {e}"));
                            return None;
                        }
                    }
                };
                ops::create_resource(&world, &registry, &id, value)
            }
            WorldRequest::ResourceRemove(id) => ops::remove_resource(&world, &id),
            WorldRequest::ResourceOpen(id) => {
                if self.dirty() {
                    self.set_error("Apply or reload the inspector draft first");
                } else {
                    self.open_resource(&id);
                }
                return None;
            }
        };
        self.world_write(result)
    }

    /// A command that takes an argument: with it, the operation; without it (menu, palette),
    /// the UI where a person picks it.
    pub(super) fn run_with_arg(&mut self, id: &str, arg: Option<&str>) -> Vec<Action> {
        let Some(arg) = arg.map(str::trim) else {
            match id {
                "object.tag" => self.notice("Type a tag under Inspector › Tags (Enter adds it)"),
                "object.add_component" => {
                    self.notice("Pick it under Inspector › Components › Add component")
                }
                "object.camera_blocker" => self.notice(
                    "Use the Inspector render.mesh checkbox 'Blocks follow cameras', or on|off",
                ),
                "event.declare" => {
                    self.show_dock(super::Dock::Problems);
                    self.notice("Undeclared events show a Declare button in Problems");
                }
                _ => {
                    self.show_dock(super::Dock::World);
                    self.notice("World settings: pick the item in the World window");
                }
            }
            return vec![];
        };
        let split = |arg: &str| -> (String, String) {
            match arg.split_once(char::is_whitespace) {
                Some((a, b)) => (a.to_owned(), b.trim().to_owned()),
                None => (arg.to_owned(), String::new()),
            }
        };
        let action = match id {
            "object.tag" => {
                let (mut add, mut remove) = (vec![], vec![]);
                for token in arg.split_whitespace() {
                    match (token.strip_prefix('+'), token.strip_prefix('-')) {
                        (Some(t), _) => add.push(t.to_owned()),
                        (_, Some(t)) => remove.push(t.to_owned()),
                        _ => add.push(token.to_owned()),
                    }
                }
                self.edit_tags(&add, &remove)
            }
            "object.add_component" => self.add_component(arg),
            "object.camera_blocker" => match arg {
                "on" => self.camera_blocker_selection(true),
                "off" => self.camera_blocker_selection(false),
                other => {
                    self.set_error(format!("object.camera_blocker takes on|off, not '{other}'"));
                    None
                }
            },
            "event.declare" => self.declare_event(arg),
            "world.set_id" => self.world_request(WorldRequest::SetId(arg.into())),
            "world.set_title" => self.world_request(WorldRequest::SetTitle(arg.into())),
            "world.capability_add" => self.world_request(WorldRequest::CapabilityAdd(arg.into())),
            "world.capability_remove" => {
                self.world_request(WorldRequest::CapabilityRemove(arg.into()))
            }
            "world.system_add" => self.world_request(WorldRequest::SystemAdd(arg.into())),
            "world.system_remove" => self.world_request(WorldRequest::SystemRemove(arg.into())),
            "world.resource_create" => {
                let (id, json) = split(arg);
                self.world_request(WorldRequest::ResourceCreate(id, json))
            }
            "world.resource_remove" => self.world_request(WorldRequest::ResourceRemove(arg.into())),
            other => {
                self.set_error(format!("{other} takes no argument"));
                None
            }
        };
        action.into_iter().collect()
    }

    /// Window ▸ World (a dock tab, floating by default). Drawn only while shown; its rows come
    /// from the per-revision cache.
    pub(super) fn world_tab(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let key: WorldKey = (self.rev, self.world_version);
        let mut panel = std::mem::take(&mut self.world_panel);
        if panel.rows.as_ref().is_none_or(|r| r.key != key) {
            panel.rows = Some(WorldRows::build(key, &self.world, &self.registry));
            #[cfg(test)]
            {
                panel.builds += 1;
            }
        }
        // A new world refreshes the fields, except text being typed (it differs from what
        // was loaded) which is kept until applied or reverted by hand.
        if panel.meta_key != Some(key) {
            if panel.meta_key.is_none() || panel.id == panel.loaded.0 {
                panel.id.clone_from(&self.world.id);
            }
            if panel.meta_key.is_none() || panel.title == panel.loaded.1 {
                panel.title.clone_from(&self.world.title);
            }
            panel.loaded = (self.world.id.clone(), self.world.title.clone());
            panel.meta_key = Some(key);
        }
        let mut requests: Vec<WorldRequest> = vec![];
        let enabled = !self.dirty() && !self.previewing;
        let mut chose_resource = None;
        #[cfg(test)]
        let mut rects: Vec<(String, egui::Rect)> = vec![];
        ui.vertical(|ui| {
                let rows = panel.rows.as_ref().unwrap();
                ui.spacing_mut().item_spacing.y = 3.0;
                // Room for the warning line below the list.
                let height = (ui.available_height() - 24.0).max(80.0);
                ui.add_enabled_ui(enabled, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("editor_world_scroll")
                        .auto_shrink([false, true])
                        .max_height(height)
                        .show(ui, |ui| {
                            ui.label(RichText::new("WORLD").small().color(MUTED));
                            ui.horizontal(|ui| {
                                ui.add_sized([40.0, 20.0], egui::Label::new("ID"));
                                ui.add(egui::TextEdit::singleline(&mut panel.id).desired_width(240.0));
                                if ui
                                    .add_enabled(panel.id != self.world.id, egui::Button::new("Apply"))
                                    .clicked()
                                {
                                    requests.push(WorldRequest::SetId(panel.id.clone()));
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.add_sized([40.0, 20.0], egui::Label::new("Title"));
                                ui.add(egui::TextEdit::singleline(&mut panel.title).desired_width(240.0));
                                if ui
                                    .add_enabled(panel.title != self.world.title, egui::Button::new("Apply"))
                                    .clicked()
                                {
                                    requests.push(WorldRequest::SetTitle(panel.title.clone()));
                                }
                            });
                            ui.separator();
                            ui.label(RichText::new(&rows.heading[0]).small().color(MUTED));
                            for row in &rows.capabilities {
                                ui.horizontal(|ui| {
                                    let remove = ui
                                        .add_enabled(row.locked.is_none(), egui::Button::new("Remove").small())
                                        .on_disabled_hover_text(row.locked.as_deref().unwrap_or(""));
                                    if remove.clicked() {
                                        requests.push(WorldRequest::CapabilityRemove(row.id.clone()));
                                    }
                                    ui.label(RichText::new(&row.text).monospace().small());
                                });
                            }
                            ui.horizontal(|ui| {
                                let _combo = egui::ComboBox::from_id_salt("world_add_capability")
                                    .selected_text(if panel.capability.is_empty() { "capability…" } else { panel.capability.as_str() })
                                    .width(180.0)
                                    .show_ui(ui, |ui| {
                                        for c in &rows.missing_capabilities {
                                            let item = ui.selectable_label(panel.capability == *c, c.as_str());
                                            #[cfg(test)]
                                            rects.push((format!("world:capability:{c}"), item.rect));
                                            if item.clicked() {
                                                panel.capability.clone_from(c);
                                            }
                                        }
                                    });
                                #[cfg(test)]
                                rects.push(("world:capability_combo".into(), _combo.response.rect));
                                let add = ui.add_enabled(!panel.capability.is_empty(), egui::Button::new("Add capability"));
                                #[cfg(test)]
                                rects.push(("world:capability_add".into(), add.rect));
                                if add.clicked() {
                                    requests.push(WorldRequest::CapabilityAdd(panel.capability.clone()));
                                }
                            });
                            ui.separator();
                            ui.label(RichText::new(&rows.heading[1]).small().color(MUTED));
                            for row in &rows.systems {
                                ui.horizontal(|ui| {
                                    let remove = ui
                                        .add_enabled(row.locked.is_none(), egui::Button::new("Remove").small())
                                        .on_disabled_hover_text(row.locked.as_deref().unwrap_or(""));
                                    if remove.clicked() {
                                        requests.push(WorldRequest::SystemRemove(row.id.clone()));
                                    }
                                    ui.label(RichText::new(&row.text).monospace().small());
                                });
                            }
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt("world_add_system")
                                    .selected_text(if panel.system.is_empty() { "system…" } else { panel.system.as_str() })
                                    .width(180.0)
                                    .show_ui(ui, |ui| {
                                        for (s, info) in &rows.missing_systems {
                                            if ui
                                                .selectable_label(panel.system == *s, s.as_str())
                                                .on_hover_text(info.as_str())
                                                .clicked()
                                            {
                                                panel.system.clone_from(s);
                                            }
                                        }
                                    });
                                let add = ui
                                    .add_enabled(!panel.system.is_empty(), egui::Button::new("Add system"))
                                    .on_hover_text("Also adds its capability and the systems it runs after");
                                #[cfg(test)]
                                rects.push(("world:system_add".into(), add.rect));
                                if add.clicked() {
                                    requests.push(WorldRequest::SystemAdd(panel.system.clone()));
                                }
                            });
                            ui.separator();
                            ui.label(RichText::new(&rows.heading[2]).small().color(MUTED));
                            for row in &rows.resources {
                                ui.horizontal(|ui| {
                                    if ui.small_button("Remove").clicked() {
                                        requests.push(WorldRequest::ResourceRemove(row.id.clone()));
                                    }
                                    if ui
                                        .link(RichText::new(&row.id).monospace().color(ACCENT))
                                        .on_hover_text("Open in the Inspector")
                                        .clicked()
                                    {
                                        requests.push(WorldRequest::ResourceOpen(row.id.clone()));
                                    }
                                    ui.label(RichText::new(&row.text).small().color(MUTED));
                                });
                            }
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt("world_add_resource")
                                    .selected_text(if panel.resource.is_empty() { "resource…" } else { panel.resource.as_str() })
                                    .width(180.0)
                                    .show_ui(ui, |ui| {
                                        for r in &rows.missing_resources {
                                            if ui.selectable_label(panel.resource == *r, r.as_str()).clicked() {
                                                chose_resource = Some(r.clone());
                                            }
                                        }
                                    });
                                let create = ui
                                    .add_enabled(!panel.resource.is_empty(), egui::Button::new("Create resource"))
                                    .on_hover_text("With the JSON below (its default when one exists); adds its capability");
                                #[cfg(test)]
                                rects.push(("world:resource_create".into(), create.rect));
                                if create.clicked() {
                                    requests.push(WorldRequest::ResourceCreate(
                                        panel.resource.clone(),
                                        panel.resource_json.clone(),
                                    ));
                                }
                            });
                            if !panel.resource.is_empty() {
                                ui.add(
                                    egui::TextEdit::multiline(&mut panel.resource_json)
                                        .code_editor()
                                        .desired_rows(4)
                                        .desired_width(f32::INFINITY),
                                );
                            }
                        });
                });
                if !enabled {
                    ui.label(
                        RichText::new("Apply or reload the Inspector draft (and stop Play) to change the world.")
                            .small()
                            .color(WARNING),
                    );
                }
            });
        if let Some(id) = chose_resource {
            panel.resource_json = ops::default_value(&self.world, &self.registry, &id)
                .and_then(|v| serde_json::to_string_pretty(&v).ok())
                .unwrap_or_default();
            panel.resource = id;
        }
        self.world_panel = panel;
        #[cfg(test)]
        for (name, rect) in rects {
            self.rects.insert(name, rect);
        }
        for request in requests {
            actions.extend(self.world_request(request));
        }
    }
}

/// Draws the tag chips of the selection (Inspector). Removing a chip or adding a typed or
/// suggested tag returns a request; it applies to every selected object.
pub(super) fn tag_chips(
    ui: &mut Ui,
    tags: &mut TagUi,
    enabled: bool,
    #[allow(unused_variables)] rects: &mut std::collections::HashMap<String, egui::Rect>,
) -> Option<TagRequest> {
    let cache = tags.cache.as_ref()?;
    let mut request = None;
    ui.label(RichText::new(&cache.caption).size(11.0).color(MUTED));
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (tag, text, all) in &cache.chips {
                let chip = egui::Button::new(RichText::new(text).small().color(if *all {
                    ACCENT
                } else {
                    MUTED
                }))
                .small()
                .selected(*all);
                ui.add(chip).on_hover_text(if *all {
                    "On every selected object"
                } else {
                    "On some of the selected objects; x removes it from all"
                });
                let remove = ui.small_button("x").on_hover_text("Remove this tag");
                #[cfg(test)]
                rects.insert(format!("tag:remove:{tag}"), remove.rect);
                if remove.clicked() {
                    request = Some(TagRequest {
                        add: vec![],
                        remove: vec![tag.clone()],
                    });
                }
            }
            if cache.chips.is_empty() {
                ui.label(RichText::new("no tags").small().weak());
            }
        });
        ui.horizontal(|ui| {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut tags.input)
                    .hint_text("add tag…")
                    .desired_width(120.0),
            );
            #[cfg(test)]
            rects.insert("tag:input".into(), edit.rect);
            let entered = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let add = ui.add_enabled(
                !tags.input.trim().is_empty(),
                egui::Button::new("Add").small(),
            );
            #[cfg(test)]
            rects.insert("tag:add".into(), add.rect);
            if (entered || add.clicked()) && !tags.input.trim().is_empty() {
                request = Some(TagRequest {
                    add: vec![tags.input.trim().to_owned()],
                    remove: vec![],
                });
            }
        });
        // Suggestions: tags the world already uses (trigger/interact filters included) that
        // not every selected object has, filtered by what is typed.
        let typed = tags.input.trim();
        let mut shown = 0;
        ui.horizontal_wrapped(|ui| {
            for tag in &cache.known {
                if shown == 6 {
                    break;
                }
                let prefix = tag
                    .get(..typed.len())
                    .is_some_and(|p| p.eq_ignore_ascii_case(typed));
                let on_all = cache.chips.iter().any(|(t, _, all)| t == tag && *all);
                if !prefix || on_all {
                    continue;
                }
                shown += 1;
                let suggest = ui
                    .small_button(RichText::new(tag).small())
                    .on_hover_text("Add this tag (used elsewhere in the world)");
                #[cfg(test)]
                rects.insert(format!("tag:suggest:{tag}"), suggest.rect);
                if suggest.clicked() {
                    request = Some(TagRequest {
                        add: vec![tag.clone()],
                        remove: vec![],
                    });
                }
            }
        });
        if cache.objects > 1 {
            ui.label(
                RichText::new("Tag edits apply to every selected object")
                    .small()
                    .weak(),
            );
        }
    });
    request
}
