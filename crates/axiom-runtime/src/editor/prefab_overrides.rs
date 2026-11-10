//! Inspector prefab override markers. Fields of a prefab instance that its own component
//! patch provides (`ops::prefab_overrides`) get a blue bar at the left of their row; right
//! click offers Revert to Prefab (`ACL: prefab revert SEL C/F`, every selected instance) and
//! Apply to Prefab (`ACL: prefab apply ID C/F`, one object). Inherited fields look normal.
//! The sets are computed once per revision and selected object, never per frame.
use super::{theme, Action, Editor};
use axiom_core::ops;
use egui::{pos2, Rect, Response, Ui};
use std::collections::{BTreeMap, HashSet};

/// What a marker's context menu asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrefabAction {
    Revert,
    Apply,
}

/// One component's overridden fields (JSON pointers relative to the component, escaped) and
/// the objects / arrays above them (their headers get a lighter bar).
#[derive(Default, Debug)]
pub(crate) struct OverrideSet {
    pub fields: HashSet<String>,
    pub within: HashSet<String>,
}

impl OverrideSet {
    fn new(fields: Vec<String>) -> Self {
        let mut within = HashSet::new();
        for field in &fields {
            let mut at = field.as_str();
            while let Some(i) = at.rfind('/') {
                at = &at[..i];
                if !within.insert(at.to_owned()) {
                    break;
                }
            }
        }
        Self {
            fields: fields.into_iter().collect(),
            within,
        }
    }

    /// The field is overridden itself, or something under it is.
    fn marked(&self, pointer: &str) -> Option<bool> {
        if self.fields.contains(pointer) {
            Some(true)
        } else if self.within.contains(pointer) {
            Some(false)
        } else {
            None
        }
    }
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
/// `pointer` is overridden or holds overrides. `apply` enables Apply to Prefab.
pub(super) fn marker(
    ui: &Ui,
    response: &Response,
    set: Option<&OverrideSet>,
    pointer: &str,
    apply: bool,
    rects: Option<&mut std::collections::HashMap<String, Rect>>,
) -> Option<PrefabAction> {
    let own = set?.marked(pointer)?;
    let r = response.rect;
    let color = if own {
        theme::PREFAB
    } else {
        theme::PREFAB.gamma_multiply(0.45)
    };
    ui.painter().rect_filled(
        Rect::from_min_max(
            pos2(r.left() - 6.0, r.top() + 2.0),
            pos2(r.left() - 3.0, r.bottom() - 2.0),
        ),
        1.0,
        color,
    );
    let mut picked = None;
    let mut menu_rects = vec![];
    response
        .clone()
        .on_hover_text(if own {
            "Overrides the prefab's value · right-click: Revert to Prefab / Apply to Prefab"
        } else {
            "Holds fields that override the prefab · right-click: Revert to Prefab / Apply to Prefab"
        })
        .context_menu(|ui| {
            let revert = ui.button("Revert to Prefab").on_hover_text(
                "Stop overriding: the prefab's value shows again (ACL: prefab revert SEL COMPONENT/FIELD)",
            );
            menu_rects.push(("override:revert".to_owned(), revert.rect));
            if revert.clicked() {
                picked = Some(PrefabAction::Revert);
                ui.close_menu();
            }
            let applied = ui
                .add_enabled(apply, egui::Button::new("Apply to Prefab"))
                .on_hover_text("This value becomes the prefab's, for every instance (ACL: prefab apply ID COMPONENT/FIELD)")
                .on_disabled_hover_text("Select one object to apply its value to the prefab");
            menu_rects.push(("override:apply".to_owned(), applied.rect));
            if applied.clicked() {
                picked = Some(PrefabAction::Apply);
                ui.close_menu();
            }
        });
    if let Some(rects) = rects {
        rects.insert(format!("override:{pointer}"), r);
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
    rects: &mut std::collections::HashMap<String, Rect>,
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
    marker(ui, &response, set, pointer, apply, Some(rects))
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
        self.overrides.sets = ops::prefab_overrides(&self.world, id)
            .into_iter()
            .map(|(component, fields)| (component, OverrideSet::new(fields)))
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
