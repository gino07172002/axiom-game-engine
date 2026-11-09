//! Inspector forms built from the typed schema of a component or resource
//! (`axiom_core::typed_schema`): ranged drags with units, enum dropdowns, colour pickers,
//! quaternions as Euler degrees, entity pickers (search, drag from the Hierarchy), clip /
//! event / machine dropdowns, collapsible objects and arrays with add / insert / remove /
//! reorder. Shapes the schema does not cover follow the value's JSON shape; values no widget
//! fits fall back to an inline JSON field. Only changed leaves are written back (`changes`),
//! so prefab-inherited fields stay inherited. Array structure changes are returned as
//! `ArrayEdit`s for `axiom_core::ops::edit_array`.
use axiom_core::{ops::ArrayEdit, World};
use egui::{Color32, RichText, Ui};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

const MUTED: Color32 = Color32::from_rgb(138, 154, 172);
const WARNING: Color32 = Color32::from_rgb(255, 178, 111);
const ERROR: Color32 = Color32::from_rgb(240, 96, 96);
/// "—": the selected objects have different values here.
pub(super) const MIXED: &str = "—";
/// Array items drawn per page; longer arrays page so a frame never lays out thousands.
pub(super) const PAGE: usize = 32;

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Widget for one value, from the typed schema.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Kind {
    /// No schema type: the widget follows the value's JSON shape.
    Auto,
    Bool,
    Int,
    Number,
    Text,
    Enum(Vec<String>),
    Color,
    /// Unit quaternion `[x,y,z,w]` edited as Euler degrees (XYZ, like the Inspector).
    Euler,
    Vec3,
    Rect,
    /// 1–4 numbers without a schema widget, edited in one row.
    Numbers,
    Entity {
        has: Option<String>,
    },
    /// A `behavior.program` event name.
    Event,
    /// A key of the object at `pointer` in resource `resource` (audio clips, machines, ...).
    Key {
        resource: String,
        pointer: String,
    },
    Object,
    Array,
}

/// Compiled typed schema of one value (cached per type; never rebuilt per frame).
#[derive(Clone, Debug)]
pub(super) struct Spec {
    pub kind: Kind,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub unit: Option<String>,
    pub desc: Option<String>,
    pub default: Option<Value>,
    pub fields: BTreeMap<String, Spec>,
    /// Schema of every value of a map-like object (`additionalProperties`).
    pub any: Option<Box<Spec>>,
    pub items: Option<Box<Spec>>,
    pub max_items: Option<usize>,
    /// The type has a valid default value (component Reset is offered).
    pub resettable: bool,
}

impl Spec {
    fn auto() -> Self {
        Spec {
            kind: Kind::Auto,
            min: None,
            max: None,
            unit: None,
            desc: None,
            default: None,
            fields: BTreeMap::new(),
            any: None,
            items: None,
            max_items: None,
            resettable: false,
        }
    }

    /// Compiles a typed schema node (see `axiom_core::typed_schema`).
    pub fn compile(node: &Value) -> Self {
        let mut spec = Spec::auto();
        let text = |k: &str| node.get(k).and_then(Value::as_str);
        spec.desc = text("description").map(str::to_owned);
        spec.unit = text("x-unit").map(str::to_owned);
        spec.min = node.get("minimum").and_then(Value::as_f64);
        spec.max = node.get("maximum").and_then(Value::as_f64);
        spec.default = node.get("default").cloned();
        spec.max_items = node
            .get("maxItems")
            .and_then(Value::as_f64)
            .map(|v| v as usize);
        if let Some(props) = node.get("properties").and_then(Value::as_object) {
            spec.fields = props
                .iter()
                .map(|(k, v)| (k.clone(), Spec::compile(v)))
                .collect();
        }
        spec.any = node
            .get("additionalProperties")
            .filter(|v| v.is_object())
            .map(|v| Box::new(Spec::compile(v)));
        spec.items = node
            .get("items")
            .filter(|v| v.is_object())
            .map(|v| Box::new(Spec::compile(v)));
        let enums: Option<Vec<String>> = node.get("enum").and_then(Value::as_array).map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        });
        spec.kind = match (text("x-ref"), text("x-widget"), text("type")) {
            (Some("entity"), _, _) => Kind::Entity {
                has: text("x-has").map(str::to_owned),
            },
            (Some("event"), _, _) => Kind::Event,
            (Some(r), _, _) if r.starts_with("key:") => {
                let rest = &r[4..];
                let (resource, pointer) = rest.split_once('/').unwrap_or((rest, ""));
                Kind::Key {
                    resource: resource.to_owned(),
                    pointer: format!("/{pointer}"),
                }
            }
            (_, Some("color"), _) => Kind::Color,
            (_, Some("euler"), _) => Kind::Euler,
            (_, Some("vec3"), _) => Kind::Vec3,
            (_, Some("rect"), _) => Kind::Rect,
            (_, _, Some("string")) if enums.is_some() => Kind::Enum(enums.unwrap()),
            (_, _, Some("boolean")) => Kind::Bool,
            (_, _, Some("integer")) => Kind::Int,
            (_, _, Some("number")) => Kind::Number,
            (_, _, Some("string")) => Kind::Text,
            (_, _, Some("object")) => Kind::Object,
            (_, _, Some("array")) => Kind::Array,
            _ => Kind::Auto,
        };
        spec
    }

    fn child(&self, key: &str) -> Option<&Spec> {
        self.fields.get(key).or(self.any.as_deref())
    }

    /// The spec of the value at JSON `pointer` under this one (a resource projection).
    pub fn at(&self, pointer: &str) -> Option<&Spec> {
        let mut spec = self;
        for segment in pointer.split('/').skip(1) {
            let key = segment.replace("~1", "/").replace("~0", "~");
            spec = match (spec.child(&key), spec.items.as_deref()) {
                (Some(child), _) => child,
                (None, Some(items)) if key.parse::<usize>().is_ok() => items,
                _ => return None,
            };
        }
        Some(spec)
    }

    /// The kind to draw `v` with: the schema's when it fits the value, else from the shape.
    fn kind_for(&self, key: &str, v: &Value) -> Kind {
        let numbers = |n: usize| {
            v.as_array()
                .is_some_and(|a| a.len() == n && a.iter().all(Value::is_number))
        };
        let fits = match &self.kind {
            Kind::Bool => v.is_boolean(),
            Kind::Int => v.is_i64() || v.is_u64(),
            Kind::Number => v.is_number(),
            Kind::Text | Kind::Enum(_) | Kind::Entity { .. } | Kind::Event | Kind::Key { .. } => {
                v.is_string()
            }
            Kind::Color => is_color(key, v) || numbers(3) || numbers(4),
            Kind::Euler => numbers(4),
            Kind::Vec3 => numbers(3),
            Kind::Rect => numbers(4),
            Kind::Numbers => small_numbers(v),
            Kind::Object => v.is_object(),
            Kind::Array => v.is_array(),
            Kind::Auto => false,
        };
        if fits {
            return self.kind.clone();
        }
        match v {
            Value::Bool(_) => Kind::Bool,
            Value::Number(n) if n.is_f64() => Kind::Number,
            Value::Number(_) => Kind::Int,
            Value::String(_) => Kind::Text,
            Value::Array(_) if is_color(key, v) => Kind::Color,
            Value::Array(_) if small_numbers(v) => Kind::Numbers,
            Value::Array(_) => Kind::Array,
            Value::Object(_) => Kind::Object,
            Value::Null => Kind::Auto,
        }
    }
}

/// Typed form specs per component / resource type. Registry types are keyed by ID and
/// dropped when the host installs another registry; world-defined record types are kept
/// with the schema they were built from and rebuilt only when it changes.
#[derive(Default)]
pub(super) struct SpecCache {
    map: HashMap<String, (Option<axiom_core::RecordSchema>, Arc<Spec>)>,
    /// Specs built so far (tests check this does not grow per frame).
    pub builds: usize,
}

impl SpecCache {
    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn get(&mut self, registry: &axiom_core::Registry, world: &World, id: &str) -> Arc<Spec> {
        let record = if registry.components.contains_key(id) || registry.resources.contains_key(id)
        {
            None
        } else {
            world
                .component_schemas
                .get(id)
                .or_else(|| world.resource_schemas.get(id))
        };
        if let Some((built_from, spec)) = self.map.get(id) {
            if built_from.as_ref() == record {
                return spec.clone();
            }
        }
        self.builds += 1;
        let mut spec = axiom_core::typed_schema::typed_schema(registry, world, id)
            .map_or_else(Spec::auto, |v| Spec::compile(&v));
        spec.resettable = axiom_core::ops::default_value(world, registry, id).is_some();
        let spec = Arc::new(spec);
        self.map
            .insert(id.to_owned(), (record.cloned(), spec.clone()));
        spec
    }
}

/// Inline entity picker state (one open at a time): the field, the query and the matches
/// for (query, world version), so a 4096-object world is filtered once per keystroke.
#[derive(Default)]
pub(super) struct Picker {
    /// The form (component / resource ID) the picker belongs to.
    owner: String,
    pub pointer: String,
    pub query: String,
    focused: bool,
    matched: Option<(String, u64, Vec<usize>)>,
}

/// What one form pass needs besides the value.
pub(super) struct Ctx<'a> {
    pub world: &'a World,
    /// Bumped when `world` changes (picker match cache key).
    pub world_version: u64,
    /// Committed value: a mixed field shows "—" until it is edited.
    pub base: &'a Value,
    /// JSON pointers (relative to the value) whose values differ across the selection.
    pub mixed: &'a HashSet<String>,
    /// Lower-case field filter; empty shows everything.
    pub filter: &'a str,
    /// Array add / insert / remove / reorder buttons are enabled.
    pub structural: bool,
    pub picker: &'a mut Option<Picker>,
    /// Screen rects of the widgets drawn (tests, `editor_view` capture), keyed `form:POINTER…`.
    pub rects: Option<&'a mut HashMap<String, egui::Rect>>,
}

/// Result of one form pass.
#[derive(Default)]
pub(super) struct Out {
    pub changed: bool,
    /// An array structure edit (pointer relative to the value).
    pub array: Option<(String, ArrayEdit)>,
}

impl Ctx<'_> {
    fn rect(&mut self, key: String, rect: egui::Rect) {
        if let Some(rects) = self.rects.as_deref_mut() {
            rects.insert(key, rect);
        }
    }
    fn is_mixed(&self, pointer: &str, v: &Value) -> bool {
        self.mixed.contains(pointer) && self.base.pointer(pointer) == Some(v)
    }
}

/// Draws `value` with `spec`; returns what changed.
pub(super) fn show(ui: &mut Ui, ctx: &mut Ctx, spec: &Spec, value: &mut Value, id: &str) -> Out {
    let mut out = Out::default();
    // A picker opened on another component / resource never writes into this one.
    if ctx
        .picker
        .as_ref()
        .is_some_and(|p| !p.owner.is_empty() && p.owner != id)
    {
        *ctx.picker = None;
    }
    ui.push_id(id, |ui| {
        object_fields(ui, ctx, spec, value, "", 0, &mut out);
    });
    if let Some(p) = ctx.picker.as_mut().filter(|p| p.owner.is_empty()) {
        p.owner = id.to_owned();
    }
    out
}

/// Whether the field at `pointer` (or something under it) matches the filter.
fn matches_filter(filter: &str, pointer: &str, v: &Value) -> bool {
    if filter.is_empty() || pointer.to_lowercase().contains(filter) {
        return true;
    }
    match v {
        Value::Object(map) => map
            .iter()
            .any(|(k, c)| matches_filter(filter, &format!("{pointer}/{}", escape(k)), c)),
        Value::Array(items) => items
            .iter()
            .take(4096)
            .enumerate()
            .any(|(i, c)| matches_filter(filter, &format!("{pointer}/{i}"), c)),
        _ => false,
    }
}

fn object_fields(
    ui: &mut Ui,
    ctx: &mut Ctx,
    spec: &Spec,
    value: &mut Value,
    prefix: &str,
    depth: usize,
    out: &mut Out,
) {
    let Value::Object(map) = value else {
        return;
    };
    let auto = Spec::auto();
    // Schema order is not meaningful (alphabetical); the value's own order is kept.
    let keys: Vec<String> = map.keys().cloned().collect();
    for key in keys {
        let pointer = format!("{prefix}/{}", escape(&key));
        let child = map.get_mut(&key).expect("key listed above");
        if !matches_filter(ctx.filter, &pointer, child) {
            continue;
        }
        let field_spec = spec.child(&key).unwrap_or(&auto);
        field(ui, ctx, field_spec, &key, child, &pointer, depth, out);
    }
}

fn label(ui: &mut Ui, key: &str, spec: &Spec) {
    let mut text = RichText::new(key).small().color(MUTED);
    if spec.desc.is_some() {
        text = text.underline();
    }
    let response = ui.label(text);
    if let Some(desc) = &spec.desc {
        response.on_hover_text(desc);
    }
}

#[allow(clippy::too_many_arguments)]
fn field(
    ui: &mut Ui,
    ctx: &mut Ctx,
    spec: &Spec,
    key: &str,
    v: &mut Value,
    pointer: &str,
    depth: usize,
    out: &mut Out,
) {
    let kind = spec.kind_for(key, v);
    match kind {
        Kind::Object if depth < 8 => {
            let open = (!ctx.filter.is_empty()).then_some(true);
            let response = egui::CollapsingHeader::new(RichText::new(key).color(MUTED))
                .id_salt(pointer)
                .default_open(depth == 0)
                .open(open)
                .show(ui, |ui| {
                    object_fields(ui, ctx, spec, v, pointer, depth + 1, out)
                });
            if let Some(desc) = &spec.desc {
                response.header_response.on_hover_text(desc);
            }
        }
        Kind::Array if depth < 8 && !small_numbers(v) => {
            array(ui, ctx, spec, key, v, pointer, depth, out);
        }
        _ => {
            ui.horizontal(|ui| {
                label(ui, key, spec);
                out.changed |= leaf(ui, ctx, spec, &kind, key, v, pointer);
            });
            out.changed |= picker_below(ui, ctx, &kind, v, pointer);
        }
    }
}

/// The open entity picker of the field at `pointer`, under its row.
fn picker_below(ui: &mut Ui, ctx: &mut Ctx, kind: &Kind, v: &mut Value, pointer: &str) -> bool {
    let Kind::Entity { has } = kind else {
        return false;
    };
    if !ctx.picker.as_ref().is_some_and(|p| p.pointer == pointer) {
        return false;
    }
    let Some(id) = picker(ui, ctx, has.as_deref(), pointer) else {
        return false;
    };
    *ctx.picker = None;
    if v.as_str() == Some(id.as_str()) {
        return false;
    }
    *v = json!(id);
    true
}

fn small_numbers(v: &Value) -> bool {
    v.as_array()
        .is_some_and(|a| (1..=4).contains(&a.len()) && a.iter().all(Value::is_number))
}

/// The value a new array item starts with: a copy of a neighbour, else the schema default,
/// else an empty value of the item kind; `None` when nothing says what an item looks like
/// (an empty array without an item schema: + Add item is disabled, use JSON).
fn new_item(spec: &Spec, items: &[Value], at: usize) -> Option<Value> {
    if let Some(v) = items.get(at.min(items.len().saturating_sub(1))) {
        return Some(v.clone());
    }
    let item = spec.items.as_deref()?;
    if let Some(d) = item.default.clone() {
        return Some(d);
    }
    Some(match item.kind {
        Kind::Bool => json!(false),
        Kind::Int => json!(0),
        Kind::Number => json!(0.0),
        Kind::Vec3 => json!([0.0, 0.0, 0.0]),
        Kind::Text | Kind::Enum(_) | Kind::Entity { .. } | Kind::Event | Kind::Key { .. } => {
            json!("")
        }
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments)]
fn array(
    ui: &mut Ui,
    ctx: &mut Ctx,
    spec: &Spec,
    key: &str,
    v: &mut Value,
    pointer: &str,
    depth: usize,
    out: &mut Out,
) {
    let Value::Array(items) = v else { return };
    let len = items.len();
    let full = spec.max_items.is_some_and(|m| len >= m);
    let page_id = ui.make_persistent_id(("page", pointer));
    let pages = len.div_ceil(PAGE).max(1);
    let mut page = ui
        .data(|d| d.get_temp::<usize>(page_id))
        .unwrap_or(0)
        .min(pages - 1);
    let open = (!ctx.filter.is_empty()).then_some(true);
    let response =
        egui::CollapsingHeader::new(RichText::new(format!("{key}  [{len}]")).color(MUTED))
            .id_salt(pointer)
            .default_open(depth == 0)
            .open(open)
            .show(ui, |ui| {
                let auto = Spec::auto();
                let item_spec = spec.items.as_deref().unwrap_or(&auto);
                if pages > 1 {
                    ui.horizontal(|ui| {
                        if ui.small_button("<").clicked() {
                            page = page.saturating_sub(1);
                        }
                        ui.label(
                            RichText::new(format!(
                                "{}–{} of {len}",
                                page * PAGE,
                                ((page + 1) * PAGE).min(len) - 1
                            ))
                            .small(),
                        );
                        if ui.small_button(">").clicked() {
                            page = (page + 1).min(pages - 1);
                        }
                    });
                }
                let range = page * PAGE..((page + 1) * PAGE).min(len);
                for i in range {
                    let item_pointer = format!("{pointer}/{i}");
                    ui.push_id(i, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{i}"))
                                    .small()
                                    .monospace()
                                    .color(MUTED),
                            );
                            ui.add_enabled_ui(ctx.structural, |ui| {
                                let buttons: [(&str, &str, bool, ArrayEdit); 4] = [
                                    ("▲", "up", i > 0, ArrayEdit::Move(i, i.saturating_sub(1))),
                                    ("▼", "down", i + 1 < len, ArrayEdit::Move(i, i + 1)),
                                    ("+", "ins", !full, ArrayEdit::Insert(i, Value::Null)),
                                    ("x", "del", true, ArrayEdit::Remove(i)),
                                ];
                                for (text, name, enabled, edit) in buttons {
                                    let r = ui
                                        .add_enabled(enabled, egui::Button::new(text).small())
                                        .on_hover_text(match name {
                                            "up" => "Move up",
                                            "down" => "Move down",
                                            "ins" => "Insert a copy before this item",
                                            _ => "Remove this item",
                                        });
                                    ctx.rect(format!("form:{pointer}:{name}:{i}"), r.rect);
                                    if r.clicked() {
                                        let edit = match edit {
                                            ArrayEdit::Insert(at, _) => ArrayEdit::Insert(
                                                at,
                                                new_item(spec, items, at).expect("item exists"),
                                            ),
                                            other => other,
                                        };
                                        out.array = Some((pointer.to_owned(), edit));
                                    }
                                }
                            });
                        });
                        let item = &mut items[i];
                        let kind = item_spec.kind_for("", item);
                        match kind {
                            Kind::Object | Kind::Array => {
                                ui.indent("item", |ui| {
                                    if item.is_object() {
                                        object_fields(
                                            ui,
                                            ctx,
                                            item_spec,
                                            item,
                                            &item_pointer,
                                            depth + 1,
                                            out,
                                        );
                                    } else {
                                        field(
                                            ui,
                                            ctx,
                                            item_spec,
                                            "items",
                                            item,
                                            &item_pointer,
                                            depth + 1,
                                            out,
                                        );
                                    }
                                });
                            }
                            kind => {
                                ui.indent("item", |ui| {
                                    ui.horizontal(|ui| {
                                        out.changed |= leaf(
                                            ui,
                                            ctx,
                                            item_spec,
                                            &kind,
                                            key,
                                            item,
                                            &item_pointer,
                                        );
                                    });
                                    out.changed |=
                                        picker_below(ui, ctx, &kind, item, &item_pointer);
                                });
                            }
                        }
                    });
                }
                let appended = new_item(spec, items, len);
                ui.add_enabled_ui(ctx.structural && !full && appended.is_some(), |ui| {
                    let r = ui.small_button("+ Add item");
                    ctx.rect(format!("form:{pointer}:add"), r.rect);
                    if let (true, Some(item)) = (r.clicked(), appended) {
                        out.array = Some((pointer.to_owned(), ArrayEdit::Append(item)));
                    }
                });
            });
    ui.data_mut(|d| d.insert_temp(page_id, page));
    if let Some(desc) = &spec.desc {
        response.header_response.on_hover_text(desc);
    }
}

fn drag_f64(ui: &mut Ui, x: &mut f64, spec: &Spec, mixed: bool, prefix: &str) -> egui::Response {
    let (lo, hi) = (
        spec.min.unwrap_or(f64::NEG_INFINITY),
        spec.max.unwrap_or(f64::INFINITY),
    );
    let speed = if lo.is_finite() && hi.is_finite() && hi > lo {
        ((hi - lo) / 400.0).clamp(0.001, 10.0)
    } else {
        (x.abs() * 0.01).max(0.01)
    };
    let mut drag = egui::DragValue::new(x)
        .speed(speed)
        .range(lo..=hi)
        .clamp_existing_to_range(false)
        .max_decimals(4)
        .prefix(prefix);
    if let Some(unit) = &spec.unit {
        drag = drag.suffix(format!(" {unit}"));
    }
    if mixed {
        drag = drag.custom_formatter(|_, _| MIXED.to_owned());
    }
    ui.add(drag)
}

/// Writes `x`; a value that was an integer stays an integer while `x` is whole (integer
/// fields typed only as numbers would refuse `33.0`).
fn set_f64(v: &mut Value, x: f64) {
    if !x.is_finite() {
        return;
    }
    let was_integer = v.is_i64() || v.is_u64();
    *v = if was_integer && x.fract() == 0.0 && x.abs() < 9.0e15 {
        json!(x as i64)
    } else {
        serde_json::Number::from_f64(x).map_or(Value::Null, Value::Number)
    };
}

/// One editable leaf. Returns true when the value changed.
fn leaf(
    ui: &mut Ui,
    ctx: &mut Ctx,
    spec: &Spec,
    kind: &Kind,
    key: &str,
    v: &mut Value,
    pointer: &str,
) -> bool {
    let mixed = ctx.is_mixed(pointer, v);
    let rect_key = format!("form:{pointer}");
    let (changed, rect) = match kind {
        Kind::Bool => {
            let mut b = v.as_bool().unwrap_or(false);
            let r = ui.add(egui::Checkbox::without_text(&mut b).indeterminate(mixed));
            if r.changed() {
                // Mixed: the first click turns every selected object on.
                *v = json!(b || mixed);
            }
            (r.changed(), r.rect)
        }
        Kind::Int => {
            let mut x = v.as_i64().unwrap_or(0);
            let mut drag = egui::DragValue::new(&mut x)
                .speed(1.0)
                .clamp_existing_to_range(false);
            if let (Some(lo), Some(hi)) = (spec.min, spec.max) {
                drag = drag.range(lo as i64..=hi as i64);
            }
            if let Some(unit) = &spec.unit {
                drag = drag.suffix(format!(" {unit}"));
            }
            if mixed {
                drag = drag.custom_formatter(|_, _| MIXED.to_owned());
            }
            let r = ui.add(drag);
            if r.changed() {
                *v = json!(x);
            }
            (r.changed(), r.rect)
        }
        Kind::Number => {
            let mut x = v.as_f64().unwrap_or(0.0);
            let r = drag_f64(ui, &mut x, spec, mixed, "");
            if r.changed() {
                set_f64(v, x);
            }
            (r.changed(), r.rect)
        }
        Kind::Text => {
            let Value::String(s) = v else { return false };
            let mut edit = egui::TextEdit::singleline(s).desired_width(170.0);
            if mixed {
                edit = edit.text_color(WARNING);
            }
            let r = ui.add(edit);
            if mixed {
                ui.label(RichText::new(MIXED).color(WARNING))
                    .on_hover_text("The selected objects differ; typing sets all of them");
            }
            (r.changed(), r.rect)
        }
        Kind::Enum(options) => {
            let current = v.as_str().unwrap_or_default().to_owned();
            let mut picked = None;
            let r = egui::ComboBox::from_id_salt(("enum", pointer))
                .selected_text(if mixed { MIXED } else { current.as_str() })
                .show_ui(ui, |ui| {
                    for option in options {
                        if ui
                            .selectable_label(*option == current, option.as_str())
                            .clicked()
                        {
                            picked = Some(option.clone());
                        }
                    }
                })
                .response;
            let changed = picked.as_ref().is_some_and(|p| *p != current || mixed);
            if let Some(p) = picked.filter(|_| changed) {
                *v = json!(p);
            }
            (changed, r.rect)
        }
        Kind::Color => {
            let r = color(ui, v);
            if mixed {
                ui.label(RichText::new(MIXED).color(WARNING));
            }
            (r.0, r.1)
        }
        Kind::Euler => euler(ui, spec, v, pointer, ctx),
        Kind::Vec3 | Kind::Rect | Kind::Numbers => {
            let labels: &[&str] = match kind {
                Kind::Rect => &["x ", "y ", "w ", "h "],
                Kind::Vec3 => &["X ", "Y ", "Z "],
                _ => &["", "", "", ""],
            };
            let mut changed = false;
            let mut rect = egui::Rect::NOTHING;
            if let Value::Array(items) = v {
                for (i, (item, axis)) in items.iter_mut().zip(labels).enumerate() {
                    let axis_pointer = format!("{pointer}/{i}");
                    let mixed = ctx.mixed.contains(&axis_pointer)
                        && ctx.base.pointer(&axis_pointer) == Some(item);
                    let mut x = item.as_f64().unwrap_or(0.0);
                    let r = drag_f64(ui, &mut x, spec, mixed, axis);
                    ctx.rect(format!("form:{axis_pointer}"), r.rect);
                    rect = rect.union(r.rect);
                    if r.changed() {
                        set_f64(item, x);
                        changed = true;
                    }
                }
            }
            (changed, rect)
        }
        Kind::Entity { has } => entity(ui, ctx, has.as_deref(), v, pointer, mixed),
        Kind::Event | Kind::Key { .. } => reference(ui, ctx, kind, v, pointer, mixed),
        Kind::Object | Kind::Array | Kind::Auto => json_leaf(ui, key, v, pointer),
    };
    ctx.rect(rect_key, rect);
    changed
}

/// Quaternion as Euler degrees. The angles typed last are kept per field while the value
/// still matches them, so dragging one axis does not make the others jump.
fn euler(
    ui: &mut Ui,
    spec: &Spec,
    v: &mut Value,
    pointer: &str,
    ctx: &mut Ctx,
) -> (bool, egui::Rect) {
    let Some(q) = v.as_array().and_then(|a| {
        let q: Vec<f32> = a
            .iter()
            .filter_map(|x| x.as_f64().map(|x| x as f32))
            .collect();
        <[f32; 4]>::try_from(q).ok()
    }) else {
        return (false, egui::Rect::NOTHING);
    };
    let id = ui.make_persistent_id(("euler", pointer));
    let cached = ui.data(|d| d.get_temp::<([f32; 3], [f32; 4])>(id));
    let mut angles = match cached {
        Some((angles, quat)) if quat == q => angles,
        _ => {
            let t = axiom_core::Transform {
                rotation: q,
                ..Default::default()
            };
            super::viewport::degrees(&t)
        }
    };
    let mut changed = false;
    let mut rect = egui::Rect::NOTHING;
    let spec = Spec {
        unit: Some("°".into()),
        min: Some(-36000.0),
        max: Some(36000.0),
        ..spec.clone()
    };
    for (i, axis) in ["X ", "Y ", "Z "].into_iter().enumerate() {
        let mut x = angles[i] as f64;
        let mixed = ctx.mixed.contains(&format!("{pointer}/euler{i}"))
            && ctx.base.pointer(pointer) == Some(&*v);
        let r = drag_f64(ui, &mut x, &spec, mixed, axis);
        ctx.rect(format!("form:{pointer}/euler{i}"), r.rect);
        rect = rect.union(r.rect);
        if r.changed() {
            angles[i] = x as f32;
            changed = true;
        }
    }
    if changed {
        let q = super::viewport::rotation(angles);
        *v = json!(q);
        ui.data_mut(|d| d.insert_temp(id, (angles, q)));
    }
    (changed, rect)
}

/// Entity reference: shows the ID (red when missing, orange when it lacks the component the
/// field needs), a picker with search, and accepts a Hierarchy row dropped on it.
fn entity(
    ui: &mut Ui,
    ctx: &mut Ctx,
    has: Option<&str>,
    v: &mut Value,
    pointer: &str,
    mixed: bool,
) -> (bool, egui::Rect) {
    let current = v.as_str().unwrap_or_default().to_owned();
    let found = ctx.world.entity(&current);
    let problem = if current.is_empty() {
        None
    } else if found.is_none() {
        Some("missing")
    } else if has.is_some_and(|c| !has_component(ctx.world, found.unwrap(), c)) {
        Some("lacks component")
    } else {
        None
    };
    let text = if mixed {
        MIXED.to_owned()
    } else if current.is_empty() {
        "(none)".to_owned()
    } else {
        current.clone()
    };
    let color = match problem {
        Some("missing") => ERROR,
        Some(_) => WARNING,
        None => Color32::from_rgb(105, 222, 195),
    };
    let r = ui
        .add(
            egui::Button::new(RichText::new(text).monospace().color(color))
                .min_size(egui::vec2(150.0, 0.0)),
        )
        .on_hover_text(match (problem, has) {
            (Some("missing"), _) => {
                format!("No object '{current}'. Pick one or drag a Hierarchy row here.")
            }
            (Some(_), Some(c)) => format!("'{current}' has no {c}"),
            (_, Some(c)) => {
                format!("Object with {c}. Click to pick, or drag a Hierarchy row here.")
            }
            _ => "Click to pick an object, or drag a Hierarchy row here.".to_owned(),
        });
    let mut changed = false;
    if let Some(dropped) = r.dnd_release_payload::<String>() {
        if ctx.world.entity(&dropped).is_some() && *dropped != current {
            *v = json!(dropped.as_str());
            changed = true;
        }
    }
    if r.dnd_hover_payload::<String>().is_some() {
        ui.painter().rect_stroke(
            r.rect.expand(2.0),
            3.0,
            egui::Stroke::new(1.5f32, Color32::from_rgb(105, 222, 195)),
            egui::StrokeKind::Outside,
        );
    }
    let open = ctx.picker.as_ref().is_some_and(|p| p.pointer == pointer);
    if r.clicked() {
        *ctx.picker = (!open).then(|| Picker {
            pointer: pointer.to_owned(),
            ..Default::default()
        });
    }
    if let Some(problem) = problem {
        ui.label(RichText::new(problem).small().color(color));
    }
    (changed, r.rect)
}

fn has_component(world: &World, e: &axiom_core::Entity, component: &str) -> bool {
    e.components.contains_key(component)
        || e.prefab
            .as_ref()
            .and_then(|p| world.prefabs.get(p))
            .is_some_and(|p| p.components.contains_key(component))
}

/// The picker list under an entity field. Returns the picked ID ("" = none).
fn picker(ui: &mut Ui, ctx: &mut Ctx, has: Option<&str>, pointer: &str) -> Option<String> {
    let world = ctx.world;
    let version = ctx.world_version;
    let state = ctx.picker.as_mut()?;
    let mut picked = None;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(220.0);
        let search = ui.add(
            egui::TextEdit::singleline(&mut state.query)
                .hint_text("Search objects")
                .desired_width(200.0),
        );
        if !state.focused {
            search.request_focus();
            state.focused = true;
        }
        let query = state.query.to_lowercase();
        let stale = !matches!(&state.matched, Some((q, v, _)) if *q == query && *v == version);
        if stale {
            let rows = world
                .entities
                .iter()
                .enumerate()
                .filter(|(_, e)| has.is_none_or(|c| has_component(world, e, c)))
                .filter(|(_, e)| {
                    query.is_empty()
                        || e.id.to_lowercase().contains(&query)
                        || e.label
                            .as_ref()
                            .is_some_and(|l| l.to_lowercase().contains(&query))
                })
                .map(|(i, _)| i)
                .collect();
            state.matched = Some((query, version, rows));
        }
        let rows = &state.matched.as_ref().expect("filled above").2;
        if let Some(c) = has {
            ui.label(
                RichText::new(format!("{} with {c}", rows.len()))
                    .small()
                    .color(MUTED),
            );
        }
        if ui.small_button("(none)").clicked() {
            picked = Some(String::new());
        }
        let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
        egui::ScrollArea::vertical()
            .id_salt(("picker", pointer))
            .max_height(180.0)
            .show_rows(ui, row_height, rows.len(), |ui, range| {
                for &i in &rows[range] {
                    let e = &world.entities[i];
                    let text = match &e.label {
                        Some(l) => format!("{}  ·  {l}", e.id),
                        None => e.id.clone(),
                    };
                    let r = ui.selectable_label(false, RichText::new(text).monospace());
                    if let Some(rects) = ctx.rects.as_deref_mut() {
                        rects.insert(format!("form:pick:{pointer}:{}", e.id), r.rect);
                    }
                    if r.clicked() {
                        picked = Some(e.id.clone());
                    }
                }
            });
    });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        *ctx.picker = None;
    }
    picked
}

/// Event names (with the declared ones offered) and resource keys (clips, machines, ...).
fn reference(
    ui: &mut Ui,
    ctx: &mut Ctx,
    kind: &Kind,
    v: &mut Value,
    pointer: &str,
    mixed: bool,
) -> (bool, egui::Rect) {
    let current = v.as_str().unwrap_or_default().to_owned();
    let (resource, at) = match kind {
        Kind::Event => ("behavior.program", "/events"),
        Kind::Key { resource, pointer } => (resource.as_str(), pointer.as_str()),
        _ => return (false, egui::Rect::NOTHING),
    };
    let options: Vec<&String> = ctx
        .world
        .resources
        .get(resource)
        .and_then(|r| r.pointer(at))
        .and_then(Value::as_object)
        .map(|o| o.keys().collect())
        .unwrap_or_default();
    let known = current.is_empty() || options.iter().any(|o| **o == current);
    let mut picked = None;
    let text = if mixed {
        MIXED.to_owned()
    } else if current.is_empty() {
        "(none)".to_owned()
    } else {
        current.clone()
    };
    let r = egui::ComboBox::from_id_salt(("ref", pointer))
        .selected_text(RichText::new(text).color(if known { Color32::LIGHT_GRAY } else { WARNING }))
        .show_ui(ui, |ui| {
            if *kind == Kind::Event && ui.selectable_label(current.is_empty(), "(none)").clicked() {
                picked = Some(String::new());
            }
            for option in &options {
                if ui
                    .selectable_label(**option == current, option.as_str())
                    .clicked()
                {
                    picked = Some((*option).clone());
                }
            }
        })
        .response;
    // An asset of the kind this field names, dropped from the Assets browser.
    let accepts = |d: &super::asset_browser::AssetDrag| d.kind.location() == Some((resource, at));
    if let Some(drag) = r
        .dnd_release_payload::<super::asset_browser::AssetDrag>()
        .filter(|d| accepts(d))
    {
        picked = Some(drag.id.clone());
    }
    if r.dnd_hover_payload::<super::asset_browser::AssetDrag>()
        .is_some_and(|d| accepts(&d))
    {
        ui.painter().rect_stroke(
            r.rect.expand(2.0),
            3.0,
            egui::Stroke::new(1.5f32, Color32::from_rgb(105, 222, 195)),
            egui::StrokeKind::Outside,
        );
    }
    let mut changed = picked.as_ref().is_some_and(|p| *p != current || mixed);
    if let Some(p) = picked.filter(|_| changed) {
        *v = json!(p);
    }
    if *kind == Kind::Event {
        // A new event name: Problems offers "Declare event" when it is not declared yet.
        if let Value::String(s) = v {
            changed |= ui
                .add(
                    egui::TextEdit::singleline(s)
                        .desired_width(90.0)
                        .hint_text("new event"),
                )
                .changed();
        }
    } else if !known {
        ui.label(
            RichText::new(format!("no {current} in {resource}"))
                .small()
                .color(WARNING),
        );
    }
    (changed, r.rect)
}

/// A value no widget fits (null, deep nesting, mismatched schema): edited as JSON text,
/// applied when it parses.
fn json_leaf(ui: &mut Ui, key: &str, v: &mut Value, pointer: &str) -> (bool, egui::Rect) {
    let id = ui.make_persistent_id(("json", pointer));
    let mut text = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| v.to_string());
    let parsed = serde_json::from_str::<Value>(&text);
    let r = ui
        .add(
            egui::TextEdit::singleline(&mut text)
                .code_editor()
                .desired_width(170.0)
                .text_color(if parsed.is_ok() {
                    Color32::LIGHT_GRAY
                } else {
                    ERROR
                }),
        )
        .on_hover_text(format!(
            "{key}: no form widget for this value; edit it as JSON"
        ));
    ui.label(RichText::new("JSON").small().color(WARNING));
    let mut changed = false;
    if r.changed() {
        if let Ok(next) = serde_json::from_str::<Value>(&text) {
            if next != *v {
                *v = next;
                changed = true;
            }
        }
    }
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    (changed, r.rect)
}

/// `color`/`tint`-named fields holding 3–4 numbers in 0..=1 get a color picker.
pub(super) fn is_color(key: &str, v: &Value) -> bool {
    let key = key.to_ascii_lowercase();
    (key.contains("color") || key.contains("tint") || key == "accent")
        && v.as_array().is_some_and(|a| {
            (3..=4).contains(&a.len())
                && a.iter()
                    .all(|x| x.as_f64().is_some_and(|x| (0.0..=1.0).contains(&x)))
        })
}

/// Color picker that writes back numbers rounded to 4 decimals (stable diffs).
pub(super) fn color(ui: &mut Ui, v: &mut Value) -> (bool, egui::Rect) {
    let Some(a) = v.as_array() else {
        return (false, egui::Rect::NOTHING);
    };
    let c: Vec<f32> = a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
    let round = |x: f32| ((x as f64) * 10000.0).round() / 10000.0;
    let (changed, rect) = if c.len() == 4 {
        let mut rgba = [c[0], c[1], c[2], c[3]];
        let r = ui.color_edit_button_rgba_unmultiplied(&mut rgba);
        if r.changed() {
            *v = json!(rgba.map(round));
        }
        (r.changed(), r.rect)
    } else if c.len() == 3 {
        let mut rgb = [c[0], c[1], c[2]];
        let r = ui.color_edit_button_rgb(&mut rgb);
        if r.changed() {
            *v = json!(rgb.map(round));
        }
        (r.changed(), r.rect)
    } else {
        return (false, egui::Rect::NOTHING);
    };
    ui.label(RichText::new(v.to_string()).small().weak());
    (changed, rect)
}

/// Changed leaves between two versions of the same shape, as (JSON pointer, new value).
/// Shape changes (different types or array lengths) replace the whole subtree.
pub(super) fn changes(base: &Value, next: &Value, prefix: &str, out: &mut Vec<(String, Value)>) {
    match (base, next) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                let path = format!("{prefix}/{}", escape(k));
                match a.get(k) {
                    Some(old) => changes(old, v, &path, out),
                    None => out.push((path, v.clone())),
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (old, new)) in a.iter().zip(b).enumerate() {
                changes(old, new, &format!("{prefix}/{i}"), out);
            }
        }
        _ if base != next => out.push((prefix.to_owned(), next.clone())),
        _ => {}
    }
}

/// Pointers (relative to `primary`) of the leaves whose value differs in any of `others`
/// that has the same field. Quaternions also mark their Euler axes (`/rotation/euler0..2`).
pub(super) fn mixed_leaves(primary: &Value, others: &[Value], spec: &Spec) -> HashSet<String> {
    let mut out = HashSet::new();
    walk_mixed(primary, others, spec, "", &mut out);
    out
}

fn walk_mixed(v: &Value, others: &[Value], spec: &Spec, pointer: &str, out: &mut HashSet<String>) {
    match v {
        Value::Object(map) => {
            for (k, c) in map {
                let auto = Spec::auto();
                let child = spec.child(k).unwrap_or(&auto);
                walk_mixed(c, others, child, &format!("{pointer}/{}", escape(k)), out);
            }
        }
        Value::Array(items) if spec.kind == Kind::Euler && items.len() == 4 => {
            let angles = |v: &Value| -> Option<[f32; 3]> {
                let q: Vec<f32> = v
                    .as_array()?
                    .iter()
                    .filter_map(|x| Some(x.as_f64()? as f32))
                    .collect();
                let rotation = <[f32; 4]>::try_from(q).ok()?;
                Some(super::viewport::degrees(&axiom_core::Transform {
                    rotation,
                    ..Default::default()
                }))
            };
            let mine = angles(v);
            for o in others.iter().filter_map(|o| o.pointer(pointer)) {
                if o != v {
                    out.insert(pointer.to_owned());
                }
                if let (Some(a), Some(b)) = (mine, angles(o)) {
                    for k in 0..3 {
                        if (a[k] - b[k]).abs() > 1e-3 {
                            out.insert(format!("{pointer}/euler{k}"));
                        }
                    }
                }
            }
        }
        Value::Array(items) => {
            // A colour / vector edited as one widget is mixed as a whole too.
            if small_numbers(v)
                && others
                    .iter()
                    .filter_map(|o| o.pointer(pointer))
                    .any(|o| o != v)
            {
                out.insert(pointer.to_owned());
            }
            let auto = Spec::auto();
            let item = spec.items.as_deref().unwrap_or(&auto);
            for (i, c) in items.iter().enumerate().take(4096) {
                walk_mixed(c, others, item, &format!("{pointer}/{i}"), out);
            }
        }
        leaf => {
            if others
                .iter()
                .filter_map(|o| o.pointer(pointer))
                .any(|o| o != leaf)
            {
                out.insert(pointer.to_owned());
            }
        }
    }
}

/// Registry descriptors keyed by component ID, merged with the world's typed schemas (shown
/// as the schema text of a component typed as JSON).
pub(super) fn hints_for(
    registry: &BTreeMap<String, Value>,
    world: &World,
    component: &str,
) -> Option<Value> {
    if let Some(schema) = world.component_schemas.get(component) {
        return serde_json::to_value(schema).ok();
    }
    registry.get(component).cloned()
}

/// A form over `value` without selection context (prefab defaults, resource projections).
pub(super) fn edit_plain(
    ui: &mut Ui,
    world: &World,
    spec: &Spec,
    id: &str,
    value: &mut Value,
    picker: &mut Option<Picker>,
) -> Out {
    let base = value.clone();
    let mixed = HashSet::new();
    let mut ctx = Ctx {
        world,
        world_version: 0,
        base: &base,
        mixed: &mixed,
        filter: "",
        structural: false,
        picker,
        rects: None,
    };
    show(ui, &mut ctx, spec, value, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn only_changed_leaves_are_reported() {
        let base =
            json!({"speed":1.8,"damage":8,"shape":{"radius":0.4,"kind":"capsule"},"p":[0,1,2]});
        let mut next = base.clone();
        next["speed"] = json!(2.5);
        next["shape"]["radius"] = json!(0.5);
        next["p"][1] = json!(3);
        let mut out = vec![];
        changes(&base, &next, "", &mut out);
        assert_eq!(
            out,
            vec![
                ("/p/1".to_owned(), json!(3)),
                ("/shape/radius".to_owned(), json!(0.5)),
                ("/speed".to_owned(), json!(2.5)),
            ]
        );
        let mut none = vec![];
        changes(&base, &base, "", &mut none);
        assert!(none.is_empty());
    }

    #[test]
    fn specs_pick_widgets_from_the_typed_schema_and_fall_back_to_the_shape() {
        let registry = crate::installed_registry().authoring;
        let world = World::empty("t");
        let mut cache = SpecCache::default();
        let light = cache.get(&registry, &world, "render.light");
        assert_eq!(
            light.fields["kind"].kind,
            Kind::Enum(vec![
                "directional".into(),
                "point".into(),
                "spot".into(),
                "area".into()
            ])
        );
        assert_eq!(
            light.fields["color"].kind_for("color", &json!([1, 0.5, 0])),
            Kind::Color
        );
        // A value that does not fit the schema type is drawn by its shape.
        assert_eq!(
            light.fields["intensity"].kind_for("intensity", &json!("x")),
            Kind::Text
        );
        let audio = cache.get(&registry, &world, "audio.source");
        assert!(
            matches!(&audio.fields["clip"].kind, Kind::Key { resource, pointer } if resource == "audio.library" && pointer == "/clips")
        );
        let fsm = cache.get(&registry, &world, "fsm.agent");
        assert!(matches!(&fsm.fields["machine"].kind, Kind::Key { .. }));
        // Cached: the same type again is not rebuilt.
        let builds = cache.builds;
        cache.get(&registry, &world, "render.light");
        assert_eq!(cache.builds, builds);
        // Unknown type: an empty spec, every field by shape.
        assert_eq!(cache.get(&registry, &world, "no.such").kind, Kind::Auto);
    }

    #[test]
    fn whole_drags_keep_integer_fields_integers() {
        let mut v = json!(32);
        set_f64(&mut v, 33.0);
        assert!(v.is_i64(), "{v}");
        set_f64(&mut v, 33.5);
        assert_eq!(v, json!(33.5));
        let mut f = json!(1.5);
        set_f64(&mut f, 2.0);
        assert!(f.is_f64());
    }

    #[test]
    fn mixed_leaves_mark_differing_fields_and_euler_axes() {
        let spec = Spec::compile(
            &json!({"type":"object","properties":{"rotation":{"type":"array","x-widget":"euler"}}}),
        );
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let a = json!({"speed":1.0,"name":"a","rotation":[0.0,0.0,0.0,1.0],"same":true,"color":[1.0,0.0,0.0]});
        let b = json!({"speed":2.0,"name":"a","rotation":[0.0,h,0.0,h],"same":true,"color":[1.0,1.0,0.0]});
        let mixed = mixed_leaves(&a, &[b], &spec);
        assert!(mixed.contains("/speed"));
        assert!(!mixed.contains("/name") && !mixed.contains("/same"));
        assert!(mixed.contains("/rotation/euler1"));
        // A colour differs as a whole (one picker) and per channel.
        assert!(mixed.contains("/color") && mixed.contains("/color/1"));
        assert!(!mixed.contains("/color/0"));
        assert!(!mixed.contains("/rotation/euler0"));
    }

    /// Every component and resource in the shipped content fits its typed schema: ranges,
    /// enums and item counts derived from the description text never reject real data.
    #[test]
    fn shipped_content_fits_the_typed_schemas() {
        let registry = crate::installed_registry().authoring;
        let mut cache = SpecCache::default();
        let mut problems = vec![];
        let mut checked = 0;
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content");
        for pack in std::fs::read_dir(root).unwrap().flatten() {
            if !pack.path().join("manifest.json").exists() {
                continue;
            }
            let Ok(package) = axiom_core::ContentPackage::load(pack.path(), &registry) else {
                continue;
            };
            let world = package.world;
            let mut check = |id: &str, value: &Value| {
                let spec = cache.get(&registry, &world, id);
                checked += 1;
                fits(
                    &spec,
                    id,
                    value,
                    &format!("{}:{id}", pack.file_name().to_string_lossy()),
                    &mut problems,
                );
            };
            for e in &world.entities {
                for (c, v) in world.resolved_components(e) {
                    check(&c, &v);
                }
            }
            for (r, v) in &world.resources {
                check(r, v);
            }
        }
        assert!(checked > 100, "only {checked} values checked");
        assert!(problems.is_empty(), "{problems:#?}");
    }

    fn fits(spec: &Spec, key: &str, v: &Value, at: &str, problems: &mut Vec<String>) {
        match spec.kind_for(key, v) {
            Kind::Number | Kind::Int => {
                let x = v.as_f64().unwrap();
                if spec.min.is_some_and(|m| x < m) || spec.max.is_some_and(|m| x > m) {
                    problems.push(format!("{at}: {x} outside {:?}..{:?}", spec.min, spec.max));
                }
            }
            Kind::Enum(options) => {
                if !options.iter().any(|o| v.as_str() == Some(o)) {
                    problems.push(format!("{at}: {v} not in {options:?}"));
                }
            }
            Kind::Object => {
                for (k, c) in v.as_object().unwrap() {
                    if let Some(child) = spec.child(k) {
                        fits(child, k, c, &format!("{at}/{k}"), problems);
                    }
                }
            }
            Kind::Array => {
                let items = v.as_array().unwrap();
                if spec.max_items.is_some_and(|m| items.len() > m) {
                    problems.push(format!(
                        "{at}: {} items > {:?}",
                        items.len(),
                        spec.max_items
                    ));
                }
                if let Some(item) = spec.items.as_deref() {
                    for (i, c) in items.iter().enumerate() {
                        fits(item, key, c, &format!("{at}/{i}"), problems);
                    }
                }
            }
            Kind::Color | Kind::Vec3 | Kind::Rect | Kind::Numbers => {
                for x in v.as_array().unwrap().iter().filter_map(Value::as_f64) {
                    if spec.min.is_some_and(|m| x < m) || spec.max.is_some_and(|m| x > m) {
                        problems.push(format!("{at}: {x} outside {:?}..{:?}", spec.min, spec.max));
                    }
                }
            }
            _ => {}
        }
    }
}
