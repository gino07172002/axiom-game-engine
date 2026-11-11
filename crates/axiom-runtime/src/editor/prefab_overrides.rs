//! Inspector prefab override markers. Fields of a prefab instance that its own component
//! patch provides (`ops::prefab_overrides`) get a blue bar at the left of their row; right
//! click offers Revert to Prefab (`ACL: prefab revert SEL C/F`, every selected instance) and
//! Apply to Prefab (`ACL: prefab apply ID C/F`, one object). Inherited fields look normal.
//! Arrays override as a whole (they never merge); inside one, the items / fields that read
//! differently from the prefab's array (`ops::prefab_array_differences`) are marked too and
//! revert / apply one value at a time, or the whole array. The component's JSON mode shows
//! the same marks in a gutter beside the overridden keys' lines.
//! The sets are computed once per revision and selected object, never per frame.
use super::{theme, Action, Editor};
use axiom_core::ops;
use egui::{pos2, Rect, Response, Ui};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

/// What a marker's context menu asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrefabAction {
    Revert,
    Apply,
}

/// How a field relates to the prefab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    /// The instance's patch provides this field.
    Own,
    /// An object / array holding overridden fields.
    Holds,
    /// Inside an array overridden as a whole: reads differently from the prefab's value at
    /// the same place.
    Differs,
    /// Inside such an array: an item / object holding values that differ.
    HoldsDiffering,
}

impl Mark {
    fn strong(self) -> bool {
        matches!(self, Mark::Own | Mark::Differs)
    }
}

/// One component's overridden fields (JSON pointers relative to the component, escaped), the
/// objects / arrays above them (their headers get a lighter bar), and inside the arrays it
/// overrides, the values that differ from the prefab's.
#[derive(Default, Debug)]
pub(crate) struct OverrideSet {
    pub fields: HashSet<String>,
    pub within: HashSet<String>,
    /// Overridden arrays the prefab also has (their items can revert / apply one by one).
    arrays: Vec<String>,
    pub differs: HashSet<String>,
    differs_within: HashSet<String>,
    /// Differing values only the whole array can revert / apply: items the prefab's array
    /// lacks (a key the prefab's item lacks can be removed / added one by one).
    whole_only: HashSet<String>,
    /// Overridden fields whose value is an array (overrides as a whole).
    own_arrays: HashSet<String>,
    /// JSON mode: (line of the authored override's pretty JSON, pointer) of every marked
    /// key / item, in line order, and the length of that text (the box shows it only while
    /// the draft's text is the same).
    pub json_lines: Vec<(usize, String)>,
    pub json_len: usize,
}

/// Inserts the objects / arrays above `field` into `within`, up to (not including) `stop`.
fn ancestors(field: &str, stop: &str, within: &mut HashSet<String>) {
    let mut at = field;
    while let Some(i) = at.rfind('/') {
        at = &at[..i];
        if at.len() <= stop.len() || !within.insert(at.to_owned()) {
            break;
        }
    }
}

impl OverrideSet {
    /// `own`: the instance's override of the component, `base`: the prefab's value.
    pub(crate) fn new(
        fields: Vec<String>,
        differs: Vec<String>,
        own: Option<&Value>,
        base: Option<&Value>,
    ) -> Self {
        let mut within = HashSet::new();
        for field in &fields {
            ancestors(field, "", &mut within);
        }
        let fields: HashSet<String> = fields.into_iter().collect();
        let mut set = Self {
            fields,
            within,
            ..Default::default()
        };
        for field in differs {
            let Some(array) = set.fields.iter().find(|a| {
                field.len() > a.len()
                    && field.starts_with(a.as_str())
                    && field[a.len()..].starts_with('/')
            }) else {
                continue;
            };
            let array = array.clone();
            ancestors(&field, &array, &mut set.differs_within);
            if !set.arrays.contains(&array) {
                set.arrays.push(array);
            }
            // Same rule as `ops::revert_prefab_field`: a value the prefab lacks reverts /
            // applies alone only as a key of an object the prefab's item has.
            let parent = &field[..field.rfind('/').unwrap_or(0)];
            let single = base.is_some_and(|b| {
                b.pointer(&field).is_some() || b.pointer(parent).is_some_and(Value::is_object)
            });
            if !single {
                set.whole_only.insert(field.clone());
            }
            set.differs.insert(field);
        }
        if let Some(own) = own {
            set.own_arrays = set
                .fields
                .iter()
                .filter(|f| own.pointer(f).is_some_and(Value::is_array))
                .cloned()
                .collect();
            set.json_len = serde_json::to_string_pretty(own).map_or(0, |t| t.len());
            json_layout(own, "", 0, &mut |line, pointer| {
                if set.marked(pointer).is_some() {
                    set.json_lines.push((line, pointer.to_owned()));
                }
            });
        }
        set
    }

    /// How the field at `pointer` is marked (`None`: it shows the prefab's value).
    pub(crate) fn marked(&self, pointer: &str) -> Option<Mark> {
        if self.fields.contains(pointer) {
            Some(Mark::Own)
        } else if self.within.contains(pointer) {
            Some(Mark::Holds)
        } else if self.differs.contains(pointer) {
            Some(Mark::Differs)
        } else if self.differs_within.contains(pointer) {
            Some(Mark::HoldsDiffering)
        } else {
            None
        }
    }

    /// The overridden array `pointer` is inside of, if any.
    fn array_of(&self, pointer: &str) -> Option<&str> {
        self.arrays
            .iter()
            .filter(|a| pointer.len() > a.len() && pointer.starts_with(a.as_str()))
            .filter(|a| pointer[a.len()..].starts_with('/'))
            .max_by_key(|a| a.len())
            .map(String::as_str)
    }
}

/// Lines of `serde_json::to_string_pretty(value)` (the JSON mode text): calls `visit` with
/// the line each key / item starts on and its pointer. Returns how many lines `value` spans
/// from `line` (where it starts). Containers open on their key's line, close on their own.
fn json_layout(
    value: &Value,
    pointer: &str,
    line: usize,
    visit: &mut dyn FnMut(usize, &str),
) -> usize {
    if !pointer.is_empty() {
        visit(line, pointer);
    }
    let mut next = line + 1;
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                let at = format!("{pointer}/{}", axiom_core::local::escape(key));
                next += json_layout(child, &at, next, visit);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (i, child) in items.iter().enumerate() {
                next += json_layout(child, &format!("{pointer}/{i}"), next, visit);
            }
        }
        _ => return 1,
    }
    next + 1 - line
}

/// The selected object's override sets, keyed by the revision / world version and object.
#[derive(Default)]
pub(crate) struct OverrideCache {
    key: Option<(i64, u64, String)>,
    sets: BTreeMap<String, OverrideSet>,
    /// Rebuilds so far (tests: not per frame).
    pub builds: usize,
}

impl OverrideCache {
    pub fn get(&self, component: &str) -> Option<&OverrideSet> {
        self.sets.get(component)
    }
}

/// Bar + context menu on `response` (a field label or an object / array header) when
/// `pointer` is overridden or holds overrides. `apply` enables Apply to Prefab. Returns the
/// pointer to act on (this field, or for a value inside an overridden array, possibly the
/// whole array) and the action.
pub(super) fn marker(
    ui: &Ui,
    response: &Response,
    set: Option<&OverrideSet>,
    pointer: &str,
    apply: bool,
    rects: Option<&mut HashMap<String, Rect>>,
) -> Option<(String, PrefabAction)> {
    let r = response.rect;
    let bar = Rect::from_min_max(
        pos2(r.left() - 6.0, r.top() + 2.0),
        pos2(r.left() - 3.0, r.bottom() - 2.0),
    );
    marker_with_bar(ui, response, bar, set, pointer, apply, rects)
}

fn marker_with_bar(
    ui: &Ui,
    response: &Response,
    bar: Rect,
    set: Option<&OverrideSet>,
    pointer: &str,
    apply: bool,
    rects: Option<&mut HashMap<String, Rect>>,
) -> Option<(String, PrefabAction)> {
    let set = set?;
    let mark = set.marked(pointer)?;
    let color = if mark.strong() {
        theme::PREFAB
    } else {
        theme::PREFAB.gamma_multiply(0.45)
    };
    ui.painter().rect_filled(bar, 1.0, color);
    // Values inside an overridden array also offer the whole array.
    let array = matches!(mark, Mark::Differs | Mark::HoldsDiffering)
        .then(|| set.array_of(pointer))
        .flatten();
    // An item the prefab's array lacks has no single-value Revert / Apply.
    let single = !set.whole_only.contains(pointer);
    let response = if response.hovered() {
        let hover = match mark {
            Mark::Own if set.own_arrays.contains(pointer) => "Overrides the prefab's array as a whole (arrays never merge: every item is this object's own) · right-click: Revert to Prefab / Apply to Prefab".to_owned(),
            Mark::Own => "Overrides the prefab's value · right-click: Revert to Prefab / Apply to Prefab".to_owned(),
            Mark::Holds => "Holds fields that override the prefab · right-click: Revert to Prefab / Apply to Prefab".to_owned(),
            Mark::Differs if !single => format!(
                "The prefab's {} has no item here (the array overrides as a whole) · right-click: Revert / Apply the whole array",
                array.unwrap_or("array")
            ),
            Mark::Differs => format!(
                "Differs from the prefab's value at the same place in {} (the array overrides as a whole) · right-click: Revert / Apply this value or the whole array",
                array.unwrap_or("the array")
            ),
            Mark::HoldsDiffering => format!(
                "Holds values that differ from the prefab's in {} (the array overrides as a whole) · right-click: Revert / Apply this item or the whole array",
                array.unwrap_or("the array")
            ),
        };
        response.clone().on_hover_text(hover)
    } else {
        response.clone()
    };
    let mut picked = None;
    let mut menu_rects = vec![];
    response.context_menu(|ui| {
        let one = "Select one object to apply its value to the prefab";
        let whole = "The prefab's array has no item here: use the whole array";
        let mut entry = |ui: &mut Ui, key: &str, title: &str, hint: &str, why_not: Option<&str>, target: &str, action| {
            let r = ui
                .add_enabled(why_not.is_none(), egui::Button::new(title))
                .on_hover_text(hint)
                .on_disabled_hover_text(why_not.unwrap_or_default());
            menu_rects.push((format!("override:{key}"), r.rect));
            if r.clicked() {
                picked = Some((target.to_owned(), action));
                ui.close_menu();
            }
        };
        entry(
            ui,
            "revert",
            "Revert to Prefab",
            "Stop overriding: the prefab's value shows again (ACL: prefab revert SEL COMPONENT/FIELD)",
            (!single).then_some(whole),
            pointer,
            PrefabAction::Revert,
        );
        entry(
            ui,
            "apply",
            "Apply to Prefab",
            "This value becomes the prefab's, for every instance (ACL: prefab apply ID COMPONENT/FIELD)",
            if !single { Some(whole) } else { (!apply).then_some(one) },
            pointer,
            PrefabAction::Apply,
        );
        if let Some(array) = array {
            ui.separator();
            entry(
                ui,
                "revert_array",
                "Revert Whole Array",
                "The whole array shows the prefab's again (ACL: prefab revert SEL COMPONENT/ARRAY)",
                None,
                array,
                PrefabAction::Revert,
            );
            entry(
                ui,
                "apply_array",
                "Apply Whole Array",
                "This object's whole array becomes the prefab's (ACL: prefab apply ID COMPONENT/ARRAY)",
                (!apply).then_some(one),
                array,
                PrefabAction::Apply,
            );
        }
    });
    if let Some(rects) = rects {
        rects.insert(format!("override:{pointer}"), response.rect);
        rects.extend(menu_rects);
    }
    picked
}

/// Marker for a row drawn without a clickable label (the Transform vectors): the label
/// (`width` wide, one line from `top`) takes the right click. One line only: a narrow
/// Inspector stacks the label above the X / Y / Z fields, which must keep their clicks.
pub(super) fn row_marker(
    ui: &Ui,
    top: f32,
    width: f32,
    set: Option<&OverrideSet>,
    pointer: &str,
    apply: bool,
    rects: &mut HashMap<String, Rect>,
) -> Option<PrefabAction> {
    set?.marked(pointer)?;
    let left = ui.min_rect().left();
    let bottom = (ui.cursor().top() - ui.spacing().item_spacing.y)
        .min(top + ui.spacing().interact_size.y)
        .max(top + 1.0);
    let area = Rect::from_min_max(pos2(left, top), pos2(left + width, bottom));
    let response = ui.interact(
        area,
        ui.id().with(("prefab_override", pointer)),
        egui::Sense::click(),
    );
    marker(ui, &response, set, pointer, apply, Some(rects)).map(|(_, action)| action)
}

/// Width of the JSON mode's marker gutter (left of the text box).
pub(super) const GUTTER: f32 = 10.0;

/// JSON mode: a bar in the gutter left of every marked key's line of the text box `text`
/// (which shows the authored override exactly as `json_lines` was laid out), with the same
/// hover and context menu as the form's markers.
pub(super) fn json_gutter(
    ui: &Ui,
    text: &egui::text_edit::TextEditOutput,
    set: &OverrideSet,
    apply: bool,
    rects: &mut HashMap<String, Rect>,
) -> Option<(String, PrefabAction)> {
    let mut picked = None;
    let left = text.response.rect.left();
    let clip = text.text_clip_rect.intersect(ui.clip_rect());
    let mut marks = set.json_lines.iter().peekable();
    let mut line = 0;
    let mut starts = true;
    for row in &text.galley.rows {
        let Some((at, pointer)) = marks.peek() else {
            break;
        };
        if starts && *at == line {
            let r = row.rect.translate(text.galley_pos.to_vec2());
            if r.bottom() > clip.top() && r.top() < clip.bottom() {
                let area = Rect::from_min_max(
                    pos2(left - GUTTER, r.top().max(clip.top())),
                    pos2(left - 1.0, r.bottom().min(clip.bottom())),
                );
                let response = ui.interact(
                    area,
                    ui.id().with(("prefab_json", pointer.as_str())),
                    egui::Sense::click(),
                );
                let bar = Rect::from_min_max(
                    pos2(left - GUTTER + 3.0, area.top() + 1.0),
                    pos2(left - GUTTER + 6.0, area.bottom() - 1.0),
                );
                if let Some(p) =
                    marker_with_bar(ui, &response, bar, Some(set), pointer, apply, Some(rects))
                {
                    picked = Some(p);
                }
            }
            marks.next();
        }
        starts = row.ends_with_newline;
        if row.ends_with_newline {
            line += 1;
        }
    }
    picked
}

impl Editor {
    /// Brings the override sets up to date with the revision and the selected object.
    pub(super) fn refresh_overrides(&mut self) {
        let Some(id) = self.selected.as_deref() else {
            self.overrides.key = None;
            self.overrides.sets.clear();
            return;
        };
        let fresh = self
            .overrides
            .key
            .as_ref()
            .is_some_and(|(rev, version, key)| {
                *rev == self.rev && *version == self.world_version && key == id
            });
        if fresh {
            return;
        }
        let mut differs = ops::prefab_array_differences(&self.world, id);
        let entity = self.world.entity(id);
        let prefab = entity
            .and_then(|e| e.prefab.as_deref())
            .and_then(|p| self.world.prefabs.get(p));
        self.overrides.sets = ops::prefab_overrides(&self.world, id)
            .into_iter()
            .map(|(component, fields)| {
                let own = entity.and_then(|e| e.components.get(&component));
                let base = prefab.and_then(|p| p.components.get(&component));
                let differs = differs.remove(&component).unwrap_or_default();
                let set = OverrideSet::new(fields, differs, own, base);
                (component, set)
            })
            .collect();
        self.overrides.key = Some((self.rev, self.world_version, id.to_owned()));
        self.overrides.builds += 1;
    }

    /// Apply to Prefab needs exactly one selected object.
    pub(super) fn prefab_apply_allowed(&self) -> bool {
        self.extra
            .iter()
            .all(|id| Some(id) == self.selected.as_ref())
    }

    /// The edits of Revert / Apply to Prefab for `component` / `pointer` on the selection.
    fn prefab_edits(
        &self,
        action: PrefabAction,
        component: &str,
        pointer: &str,
    ) -> anyhow::Result<Vec<axiom_core::Edit>> {
        match action {
            // The op leaves selected objects that do not inherit the component alone.
            PrefabAction::Revert => {
                ops::revert_prefab_field(&self.world, &self.selection(), component, pointer)
            }
            PrefabAction::Apply => {
                anyhow::ensure!(
                    self.prefab_apply_allowed(),
                    "Apply to Prefab takes one selected object"
                );
                let id = self
                    .selected
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("nothing selected"))?;
                ops::apply_prefab_field(&self.world, id, component, pointer)
            }
        }
    }

    /// Inspector marker menu: one write, or the error in Problems.
    pub(super) fn prefab_field(
        &mut self,
        action: PrefabAction,
        component: &str,
        pointer: &str,
    ) -> Option<Action> {
        if self.dirty() {
            self.set_error("Apply or reload the Inspector draft first");
            return None;
        }
        let result = self.prefab_edits(action, component, pointer);
        self.op_write(result, None)
    }

    /// `prefab.revert` / `prefab.apply` with `COMPONENT/FIELD` (the ACL field syntax).
    pub(super) fn prefab_command(
        &mut self,
        id: &str,
        arg: Option<&str>,
    ) -> anyhow::Result<Vec<Action>> {
        let Some(arg) = arg.map(str::trim).filter(|a| !a.is_empty()) else {
            self.show_dock(super::Dock::Inspector);
            self.notice("Right-click a field with a blue override bar in the Inspector");
            return Ok(vec![]);
        };
        let action = if id == "prefab.apply" {
            PrefabAction::Apply
        } else {
            PrefabAction::Revert
        };
        let (component, pointer) = axiom_core::acl::component_field(arg);
        let edits = self.prefab_edits(action, &component, &pointer)?;
        Ok(vec![self.write(edits, None)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn set() -> (OverrideSet, Value) {
        // `parts` (an array) and `name` are overridden; inside `parts`, item 1's color and the
        // new item 2 differ from the prefab's; item 0 has a key the prefab's item lacks.
        let own = json!({"name":"m","parts":[{"color":[0.1,0.1,0.1],"glow":2},{"color":[0,0,1],"size":2},{"color":[1,1,1]}],"light":{"range":{"far":9}}});
        let base = json!({"name":"lamp","parts":[{"color":[0.1,0.1,0.1]},{"color":[1,0.8,0.6],"size":2}],"light":{"range":{"far":1,"near":0}}});
        let set = OverrideSet::new(
            vec!["/light/range/far".into(), "/name".into(), "/parts".into()],
            vec![
                "/parts/0/glow".into(),
                "/parts/1/color".into(),
                "/parts/2".into(),
            ],
            Some(&own),
            Some(&base),
        );
        (set, own)
    }

    #[test]
    fn nested_paths_are_marked_by_how_they_relate_to_the_prefab() {
        let (set, _) = set();
        assert_eq!(set.marked("/name"), Some(Mark::Own));
        assert_eq!(set.marked("/parts"), Some(Mark::Own), "the array itself");
        assert_eq!(set.marked("/light"), Some(Mark::Holds));
        assert_eq!(set.marked("/light/range"), Some(Mark::Holds));
        assert_eq!(set.marked("/light/range/far"), Some(Mark::Own));
        assert_eq!(set.marked("/parts/1/color"), Some(Mark::Differs));
        assert_eq!(set.marked("/parts/2"), Some(Mark::Differs));
        assert_eq!(set.marked("/parts/1"), Some(Mark::HoldsDiffering));
        // Item 0 holds a key the prefab's item lacks; values equal to the prefab's look normal.
        assert_eq!(set.marked("/parts/0"), Some(Mark::HoldsDiffering));
        assert_eq!(set.marked("/parts/0/glow"), Some(Mark::Differs));
        assert_eq!(set.marked("/parts/0/color"), None);
        assert_eq!(set.marked("/parts/1/size"), None);
        assert_eq!(set.marked(""), None);
        assert_eq!(set.array_of("/parts/1/color"), Some("/parts"));
        assert_eq!(set.array_of("/parts"), None);
        assert_eq!(set.array_of("/partsx/1"), None);
        // Only the whole array reverts / applies an item the prefab lacks; a key it lacks can.
        assert!(set.whole_only.contains("/parts/2"));
        assert!(!set.whole_only.contains("/parts/0/glow"));
        assert!(!set.whole_only.contains("/parts/1/color"));
        assert!(set.own_arrays.contains("/parts") && !set.own_arrays.contains("/name"));
    }

    #[test]
    fn json_lines_match_the_pretty_printed_override() {
        let (set, own) = set();
        let text = serde_json::to_string_pretty(&own).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let marked: Vec<(usize, &str)> = set
            .json_lines
            .iter()
            .map(|(l, p)| (*l, p.as_str()))
            .collect();
        // Each marked key's line names that key; items start their own line.
        let key_line = |p: &str| lines[marked.iter().find(|(_, q)| *q == p).unwrap().0];
        assert!(key_line("/light").contains("\"light\""));
        assert!(key_line("/light/range/far").contains("\"far\": 9"));
        assert!(key_line("/name").contains("\"name\": \"m\""));
        assert!(key_line("/parts").contains("\"parts\": ["));
        assert!(key_line("/parts/1/color").contains("\"color\": ["));
        assert_eq!(key_line("/parts/1").trim(), "{");
        assert_eq!(key_line("/parts/2").trim(), "{");
        // In line order, unmarked keys left out; every line exists.
        assert!(marked.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(marked.iter().all(|(l, _)| *l < lines.len()));
        assert!(!marked.iter().any(|(_, p)| p.starts_with("/parts/0/color")));
        assert_eq!(set.json_len, text.len());
        // Layout of empty containers and the line count of the whole value.
        let mut seen = vec![];
        let value = json!({"a":[],"b":{},"c":[1,{"d":2}]});
        let span = json_layout(&value, "", 0, &mut |l, p| seen.push((l, p.to_owned())));
        let pretty = serde_json::to_string_pretty(&value).unwrap();
        assert_eq!(span, pretty.lines().count());
        for (l, p) in &seen {
            let line = pretty.lines().nth(*l).unwrap();
            let last = p.rsplit('/').next().unwrap();
            assert!(
                line.contains(&format!("\"{last}\"")) || last.parse::<usize>().is_ok(),
                "{p} on line {l}: {line}"
            );
        }
        assert_eq!(seen.iter().find(|(_, p)| p == "/c/1/d").unwrap().0, 6);
    }
}
