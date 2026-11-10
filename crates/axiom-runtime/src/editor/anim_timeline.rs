//! Animation tab: a skeletal clip on a timeline with the ability events that play over it.
//! Scrubbing (ruler or clip row) poses the model at the playhead in the scene view for that
//! frame only (edit mode; nothing is written and the simulation's controller is restored after
//! extraction). Event markers are the clip-synchronised events the engine has: an
//! `action.abilities` ability plays its clip stretched over `duration` ticks and fires
//! `events [{tick, sound}]`; add at the playhead / drag (one revision on release) / delete =
//! `ACL: ability ID event add|move|sound|remove`. Ability windows are drawn read-only.
use super::icons::Icon;
use super::tuning::mark;
use super::{theme, Action, Dock, Editor};
use crate::world::WorldRuntime;
use axiom_core::ops::{self, AbilityEventEdit};
use egui::{pos2, vec2, Color32, Rect, RichText, Sense, Stroke, Ui};
use serde_json::{json, Value};

const RULER: f32 = 18.0;
const ROW: f32 = 26.0;
const LABELS: f32 = 112.0;
const MARKER: f32 = 6.0;

#[derive(Default)]
pub(crate) struct AnimUi {
    /// Model (entity with an `animation.player`) and clip on the timeline.
    pub entity: Option<String>,
    pub clip: Option<String>,
    /// Playhead in clip seconds.
    pub time: f32,
    /// The scene shows the model at the playhead while the tab is drawn.
    pub preview: bool,
    /// The tab was drawn this frame (`Tuning::begin_frame` clears it).
    pub visible: bool,
    /// Selected event: (ability, index).
    pub selected: Option<(String, usize)>,
    /// The (tick, sound) the selection should still find at its index after the next
    /// revision; another value there (an external edit shifted the events) clears it.
    selected_value: Option<(u32, String)>,
    /// Marker being dragged: (ability, index, tick).
    pub drag: Option<(String, usize, u32)>,
    /// Ability that "+ Event" adds to, and the cue it uses.
    pub ability: Option<String>,
    pub sound: String,
    /// The selection the model last followed (a new selection with a player switches to it).
    followed: Option<String>,
    cache: Option<Model>,
    /// Times the ability model was rebuilt (tests: not per frame).
    pub builds: usize,
}

#[derive(Clone)]
struct Ability {
    id: String,
    clip: String,
    duration: u32,
    events: Vec<(u32, String)>,
    /// (name, start, end) ticks, half-open.
    windows: Vec<(&'static str, u32, u32)>,
}

struct Model {
    key: (i64, u64),
    abilities: Vec<Ability>,
    cues: Vec<String>,
}

impl Model {
    fn build(key: (i64, u64), world: &axiom_core::World) -> Self {
        let mut abilities = vec![];
        if let Some(map) = world
            .resources
            .get(ops::ABILITIES)
            .and_then(|r| r.get("abilities"))
            .and_then(Value::as_object)
        {
            for (id, a) in map {
                let events = a["events"]
                    .as_array()
                    .map(|e| {
                        e.iter()
                            .map(|e| {
                                (
                                    e["tick"].as_u64().unwrap_or(0) as u32,
                                    e["sound"].as_str().unwrap_or_default().to_owned(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let windows = ["active", "invulnerable", "chain_window", "dodge_window"]
                    .into_iter()
                    .filter_map(|w| {
                        let v = a.get(w)?;
                        Some((w, v["start"].as_u64()? as u32, v["end"].as_u64()? as u32))
                    })
                    .collect();
                abilities.push(Ability {
                    id: id.clone(),
                    clip: a["clip"].as_str().unwrap_or_default().to_owned(),
                    duration: a["duration"].as_u64().unwrap_or(0) as u32,
                    events,
                    windows,
                });
            }
        }
        let cues = world
            .resources
            .get("audio.library")
            .and_then(|l| l.get("cues"))
            .and_then(Value::as_object)
            .map(|c| c.keys().cloned().collect())
            .unwrap_or_default();
        Self {
            key,
            abilities,
            cues,
        }
    }
}

impl AnimUi {
    pub fn observation(&self) -> Value {
        json!({
            "entity":self.entity,
            "clip":self.clip,
            "time":self.time,
            "preview":self.preview && self.visible,
            "selected":self.selected.as_ref().map(|(a, i)| format!("{a}#{i}")),
        })
    }
}

/// Tick of an ability at clip fraction `f` (the clip is stretched over the ability).
fn tick_at(f: f32, duration: u32) -> u32 {
    ((f.clamp(0.0, 1.0) * duration as f32).round() as u32).min(duration.saturating_sub(1))
}

enum Intent {
    Edit(String, AbilityEventEdit),
    Select(Option<(String, usize)>),
}

impl Editor {
    /// One ability timeline edit (`ACL: ability ID event ..`).
    pub(super) fn ability_event_edit(
        &mut self,
        ability: &str,
        edit: &AbilityEventEdit,
    ) -> Option<Action> {
        let result = ops::edit_ability_event(&self.world, ability, edit);
        self.tuning_write(result)
    }

    /// `anim.target ENTITY [CLIP]` (view only).
    pub(super) fn anim_target_command(&mut self, arg: Option<&str>) -> anyhow::Result<()> {
        self.show_dock(Dock::Animation);
        let Some(arg) = arg else {
            return Ok(());
        };
        let mut words = arg.split_whitespace();
        let entity = words.next().unwrap_or_default();
        anyhow::ensure!(
            self.world
                .entity(entity)
                .is_some_and(
                    |e| super::ui::component_value(&self.world, e, "animation.player").is_some()
                ),
            "anim.target: {entity} has no animation.player"
        );
        self.tuning.anim.entity = Some(entity.to_owned());
        self.tuning.anim.clip = words.next().map(str::to_owned);
        self.tuning.anim.time = 0.0;
        self.tuning.anim.selected = None;
        Ok(())
    }

    /// `anim.scrub SECONDS` (view only): moves the playhead and previews the pose.
    pub(super) fn anim_scrub_command(&mut self, arg: Option<&str>) -> anyhow::Result<()> {
        self.show_dock(Dock::Animation);
        let Some(arg) = arg else {
            return Ok(());
        };
        let t: f32 = arg
            .parse()
            .map_err(|_| anyhow::anyhow!("anim.scrub takes seconds, e.g. 0.5"))?;
        anyhow::ensure!(t.is_finite() && t >= 0.0, "anim.scrub takes seconds >= 0");
        self.tuning.anim.time = t;
        self.tuning.anim.preview = true;
        Ok(())
    }

    /// The model, clip and playhead the scene view poses this frame (edit mode only).
    pub(super) fn anim_scrub_target(&self) -> Option<(&str, &str, f32)> {
        let a = &self.tuning.anim;
        if !a.preview || !a.visible || self.previewing {
            return None;
        }
        Some((a.entity.as_deref()?, a.clip.as_deref()?, a.time))
    }

    /// The Animation tab body.
    pub(super) fn animation_tab(
        &mut self,
        ui: &mut Ui,
        runtime: &WorldRuntime,
        actions: &mut Vec<Action>,
    ) {
        self.tuning.anim.visible = true;
        let key = (self.rev, self.world_version);
        if self.tuning.anim.cache.as_ref().is_none_or(|m| m.key != key) {
            self.tuning.anim.cache = Some(Model::build(key, &self.world));
            self.tuning.anim.builds += 1;
            self.tuning.anim.drag = None;
            // Event indices may have shifted in the new revision: keep the selection only
            // where the expected event still is.
            let a = &mut self.tuning.anim;
            let still = a
                .selected
                .as_ref()
                .zip(a.selected_value.as_ref())
                .is_some_and(|((id, i), (tick, sound))| {
                    a.cache
                        .as_ref()
                        .and_then(|m| m.abilities.iter().find(|x| &x.id == id))
                        .and_then(|x| x.events.get(*i))
                        .is_some_and(|(t, s)| t == tick && s == sound)
                });
            if !still {
                a.selected = None;
                a.selected_value = None;
            }
        }
        let model = self.tuning.anim.cache.take().expect("built above");
        let catalog = &runtime.state.animation_cues.catalog;
        // Model: the selection when it has a player, else the chosen / first one.
        if let Some(sel) = self
            .selected
            .as_deref()
            .filter(|s| catalog.contains_key(*s))
        {
            if self.tuning.anim.followed.as_deref() != Some(sel) {
                self.tuning.anim.followed = Some(sel.to_owned());
                if self.tuning.anim.entity.as_deref() != Some(sel) {
                    self.tuning.anim.entity = Some(sel.to_owned());
                    self.tuning.anim.clip = None;
                }
            }
        }
        if self
            .tuning
            .anim
            .entity
            .as_ref()
            .is_none_or(|e| !catalog.contains_key(e))
        {
            self.tuning.anim.entity = catalog.keys().next().cloned();
            self.tuning.anim.clip = None;
        }
        let mut intents = vec![];
        match self.tuning.anim.entity.clone() {
            None => {
                ui.label(
                    RichText::new(
                        "No object with an animation.player (and a loaded model) in this world.",
                    )
                    .color(theme::MUTED),
                );
            }
            Some(entity) => {
                let clips = &catalog[&entity];
                if self
                    .tuning
                    .anim
                    .clip
                    .as_ref()
                    .is_none_or(|c| !clips.contains_key(c))
                {
                    // Prefer a clip an ability plays (its events are what the tab edits).
                    self.tuning.anim.clip = model
                        .abilities
                        .iter()
                        .map(|a| &a.clip)
                        .find(|c| clips.contains_key(*c))
                        .or_else(|| clips.keys().next())
                        .cloned();
                }
                self.animation_body(ui, &entity, catalog, &model, &mut intents);
            }
        }
        self.tuning.anim.cache = Some(model);
        for intent in intents {
            match intent {
                Intent::Select(s) => {
                    let a = &mut self.tuning.anim;
                    a.selected_value = s.as_ref().and_then(|(id, i)| {
                        a.cache
                            .as_ref()?
                            .abilities
                            .iter()
                            .find(|x| &x.id == id)?
                            .events
                            .get(*i)
                            .cloned()
                    });
                    a.selected = s;
                }
                Intent::Edit(ability, edit) => {
                    let events = self
                        .tuning
                        .anim
                        .cache
                        .as_ref()
                        .and_then(|m| m.abilities.iter().find(|a| a.id == ability))
                        .map(|a| a.events.clone())
                        .unwrap_or_default();
                    // What the edited event looks like once the write lands.
                    let (selected, value) = match &edit {
                        AbilityEventEdit::Add { tick, sound } => (
                            Some((ability.clone(), events.len())),
                            Some((*tick, sound.clone())),
                        ),
                        AbilityEventEdit::Move { index, tick } => (
                            Some((ability.clone(), *index)),
                            events.get(*index).map(|(_, s)| (*tick, s.clone())),
                        ),
                        AbilityEventEdit::Sound { index, sound } => (
                            Some((ability.clone(), *index)),
                            events.get(*index).map(|(t, _)| (*t, sound.clone())),
                        ),
                        AbilityEventEdit::Remove { .. } => (None, None),
                    };
                    self.tuning.anim.selected = selected;
                    self.tuning.anim.selected_value = value;
                    actions.extend(self.ability_event_edit(&ability, &edit));
                }
            }
        }
    }

    fn animation_body(
        &mut self,
        ui: &mut Ui,
        entity: &str,
        catalog: &std::collections::BTreeMap<String, std::collections::BTreeMap<String, f32>>,
        model: &Model,
        intents: &mut Vec<Intent>,
    ) {
        let icons = self.icons;
        let editable = !self.previewing;
        let clips = &catalog[entity];
        let clip = self.tuning.anim.clip.clone().unwrap_or_default();
        let length = clips.get(&clip).copied().unwrap_or(0.0).max(1e-3);
        // Toolbar: model, clip, preview, playhead readout.
        ui.horizontal(|ui| {
            icons.show(ui, Icon::Animation, 16.0, theme::ACCENT);
            egui::ComboBox::from_id_salt("anim_entity")
                .selected_text(entity)
                .width(150.0)
                .show_ui(ui, |ui| {
                    for e in catalog.keys() {
                        let r = ui.selectable_label(e == entity, e);
                        mark!(self.rects, format!("anim:entity:{e}"), r.rect);
                        if r.clicked() {
                            self.tuning.anim.entity = Some(e.clone());
                            self.tuning.anim.clip = None;
                            self.tuning.anim.selected = None;
                        }
                    }
                });
            let combo = egui::ComboBox::from_id_salt("anim_clip")
                .selected_text(format!(
                    "{clip}  {:.2} s",
                    clips.get(&clip).copied().unwrap_or(0.0)
                ))
                .width(170.0)
                .show_ui(ui, |ui| {
                    for (c, d) in clips {
                        let used = model.abilities.iter().filter(|a| &a.clip == c).count();
                        let text = if used > 0 {
                            format!("{c}  {d:.2} s  · {used} ability")
                        } else {
                            format!("{c}  {d:.2} s")
                        };
                        let r = ui.selectable_label(*c == clip, text);
                        mark!(self.rects, format!("anim:clip:{c}"), r.rect);
                        if r.clicked() {
                            self.tuning.anim.clip = Some(c.clone());
                            self.tuning.anim.time = 0.0;
                            self.tuning.anim.selected = None;
                        }
                    }
                });
            mark!(self.rects, "anim:clip".into(), combo.response.rect);
            let eye = theme::icon_button(
                ui,
                icons,
                if self.tuning.anim.preview {
                    Icon::Eye
                } else {
                    Icon::EyeOff
                },
                "Pose the model at the playhead in the Scene view (edit mode; nothing is written)",
                self.tuning.anim.preview,
            );
            mark!(self.rects, "anim:preview".into(), eye.rect);
            if eye.clicked() {
                self.tuning.anim.preview = !self.tuning.anim.preview;
            }
            let t = self.tuning.anim.time.min(length);
            ui.label(
                RichText::new(format!(
                    "{t:.3} / {length:.2} s · {:.0}%",
                    t / length * 100.0
                ))
                .monospace()
                .color(theme::TEXT),
            );
        });
        let rows: Vec<&Ability> = model.abilities.iter().filter(|a| a.clip == clip).collect();
        if rows.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("No ability plays this clip, so it has no events. Abilities:")
                        .size(12.0)
                        .color(theme::MUTED),
                );
                for a in &model.abilities {
                    if clips.contains_key(&a.clip)
                        && ui.small_button(format!("{} ({})", a.id, a.clip)).clicked()
                    {
                        self.tuning.anim.clip = Some(a.clip.clone());
                        self.tuning.anim.time = 0.0;
                    }
                }
            });
        }
        // Timeline: ruler, clip row (scrub), one row per ability (windows + event markers).
        let width = ui.available_width().max(240.0);
        let (rect, response) = ui.allocate_exact_size(
            vec2(width, RULER + ROW * (1 + rows.len()) as f32 + 4.0),
            Sense::click_and_drag(),
        );
        mark!(self.rects, "anim:strip".into(), rect);
        let lane = Rect::from_min_max(pos2(rect.left() + LABELS, rect.top()), rect.right_bottom());
        let span = (lane.width() - 10.0).max(1.0);
        let x_of = |f: f32| lane.left() + 5.0 + f.clamp(0.0, 1.0) * span;
        let f_of = |x: f32| ((x - lane.left() - 5.0) / span).clamp(0.0, 1.0);
        let row_top = |r: usize| rect.top() + RULER + ROW * r as f32;
        let pointer = response.interact_pointer_pos();
        let hit_marker = |p: egui::Pos2| -> Option<(usize, usize)> {
            let r = ((p.y - rect.top() - RULER) / ROW).floor();
            if r < 1.0 || r as usize > rows.len() {
                return None;
            }
            let a = rows[r as usize - 1];
            a.events
                .iter()
                .position(|(tick, _)| {
                    (x_of(*tick as f32 / a.duration.max(1) as f32) - p.x).abs() <= MARKER + 1.0
                })
                .map(|i| (r as usize - 1, i))
        };
        // A drag whose release was never seen here (tab switched mid-drag) must not turn the
        // next press into a move of that old marker.
        if response.drag_started()
            || response.clicked()
            || !(response.dragged() || response.drag_stopped())
        {
            self.tuning.anim.drag = None;
        }
        if response.drag_started() || response.clicked() {
            if let Some(p) = ui.input(|i| i.pointer.press_origin()).or(pointer) {
                match hit_marker(p) {
                    Some((r, i)) => {
                        let a = rows[r];
                        intents.push(Intent::Select(Some((a.id.clone(), i))));
                        self.tuning.anim.ability = Some(a.id.clone());
                        if editable && response.drag_started() {
                            self.tuning.anim.drag = Some((a.id.clone(), i, a.events[i].0));
                        }
                    }
                    None if p.x >= lane.left() => {
                        let r = ((p.y - rect.top() - RULER) / ROW).floor();
                        if r >= 1.0 && (r as usize) <= rows.len() {
                            self.tuning.anim.ability = Some(rows[r as usize - 1].id.clone());
                        }
                        intents.push(Intent::Select(None));
                        self.tuning.anim.time = f_of(p.x) * length;
                        self.tuning.anim.preview = true;
                    }
                    None => {}
                }
            }
        }
        if response.dragged() {
            if let Some(p) = pointer {
                match &mut self.tuning.anim.drag {
                    Some((id, _, tick)) => {
                        if let Some(a) = rows.iter().find(|a| &a.id == id) {
                            *tick = tick_at(f_of(p.x), a.duration);
                        }
                    }
                    None if p.x >= lane.left() => {
                        self.tuning.anim.time = f_of(p.x) * length;
                        self.tuning.anim.preview = true;
                    }
                    None => {}
                }
            }
        }
        if response.drag_stopped() {
            if let Some((id, index, tick)) = self.tuning.anim.drag.take() {
                let moved = rows
                    .iter()
                    .find(|a| a.id == id)
                    .and_then(|a| a.events.get(index))
                    .is_some_and(|(t, _)| *t != tick);
                if moved {
                    intents.push(Intent::Edit(id, AbilityEventEdit::Move { index, tick }));
                }
            }
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, theme::RADIUS, theme::FIELD);
        // Ruler: a tick every tenth of the clip, labelled every fifth (seconds).
        for i in 0..=10 {
            let f = i as f32 / 10.0;
            let x = x_of(f);
            painter.line_segment(
                [
                    pos2(x, rect.top() + if i % 5 == 0 { 3.0 } else { 10.0 }),
                    pos2(x, rect.bottom()),
                ],
                Stroke::new(1.0_f32, theme::BORDER),
            );
            if i % 5 == 0 {
                let align = if i == 10 {
                    egui::Align2::RIGHT_TOP
                } else {
                    egui::Align2::LEFT_TOP
                };
                painter.text(
                    pos2(x + if i == 10 { -3.0 } else { 3.0 }, rect.top() + 2.0),
                    align,
                    format!("{:.2}s", f * length),
                    egui::FontId::monospace(10.0),
                    theme::MUTED,
                );
            }
        }
        // Clip row.
        let top = row_top(0);
        painter.text(
            pos2(rect.left() + 8.0, top + ROW * 0.5),
            egui::Align2::LEFT_CENTER,
            &clip,
            egui::FontId::proportional(12.0),
            theme::TEXT,
        );
        painter.rect_filled(
            Rect::from_min_max(pos2(x_of(0.0), top + 6.0), pos2(x_of(1.0), top + ROW - 6.0)),
            4.0,
            theme::CONTROL,
        );
        // Ability rows.
        for (r, a) in rows.iter().enumerate() {
            let top = row_top(r + 1);
            let current = self.tuning.anim.ability.as_deref() == Some(a.id.as_str());
            if current {
                painter.rect_filled(
                    Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), top + ROW)),
                    0.0,
                    theme::ACCENT_SOFT.gamma_multiply(0.5),
                );
            }
            painter.text(
                pos2(rect.left() + 8.0, top + ROW * 0.5),
                egui::Align2::LEFT_CENTER,
                &a.id,
                egui::FontId::proportional(12.0),
                if current { theme::ACCENT } else { theme::TEXT },
            );
            painter.text(
                pos2(lane.left() - 6.0, top + ROW * 0.5),
                egui::Align2::RIGHT_CENTER,
                format!("{}t", a.duration),
                egui::FontId::monospace(10.0),
                theme::MUTED,
            );
            let d = a.duration.max(1) as f32;
            for (name, start, end) in &a.windows {
                let color = match *name {
                    "active" => theme::ACCENT,
                    "invulnerable" => theme::PREFAB,
                    "dodge_window" => theme::SUCCESS,
                    _ => theme::WARNING,
                };
                painter.rect_filled(
                    Rect::from_min_max(
                        pos2(x_of(*start as f32 / d), top + ROW - 7.0),
                        pos2(
                            x_of(*end as f32 / d).max(x_of(*start as f32 / d) + 2.0),
                            top + ROW - 3.0,
                        ),
                    ),
                    1.0,
                    color.gamma_multiply(0.7),
                );
            }
            for (i, (tick, sound)) in a.events.iter().enumerate() {
                let dragged = self
                    .tuning
                    .anim
                    .drag
                    .as_ref()
                    .filter(|(id, j, _)| *id == a.id && *j == i)
                    .map(|(.., t)| *t);
                let tick = dragged.unwrap_or(*tick);
                let x = x_of(tick as f32 / d);
                let y = top + ROW * 0.42;
                let selected = self
                    .tuning
                    .anim
                    .selected
                    .as_ref()
                    .is_some_and(|(id, j)| *id == a.id && *j == i);
                let color = if selected || dragged.is_some() {
                    theme::WARNING
                } else {
                    theme::SUCCESS
                };
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        pos2(x, y - MARKER),
                        pos2(x + MARKER, y),
                        pos2(x, y + MARKER),
                        pos2(x - MARKER, y),
                    ],
                    color,
                    Stroke::new(1.0_f32, Color32::BLACK),
                ));
                painter.text(
                    pos2(x + MARKER + 2.0, y),
                    egui::Align2::LEFT_CENTER,
                    sound,
                    egui::FontId::proportional(10.0),
                    theme::TEXT,
                );
                mark!(
                    self.rects,
                    format!("anim:event:{}:{i}", a.id),
                    Rect::from_center_size(pos2(x, y), vec2(MARKER * 2.0, MARKER * 2.0))
                );
            }
        }
        let px = x_of(self.tuning.anim.time / length);
        painter.line_segment(
            [pos2(px, rect.top()), pos2(px, rect.bottom())],
            Stroke::new(1.5_f32, theme::ERROR),
        );
        // Event controls: add at the playhead; the selected event's cue / delete.
        if rows.is_empty() {
            return;
        }
        if self
            .tuning
            .anim
            .ability
            .as_ref()
            .is_none_or(|id| !rows.iter().any(|a| &a.id == id))
        {
            self.tuning.anim.ability = Some(rows[0].id.clone());
        }
        if self.tuning.anim.sound.is_empty() || !model.cues.contains(&self.tuning.anim.sound) {
            self.tuning.anim.sound = model.cues.first().cloned().unwrap_or_default();
        }
        let ability = rows
            .iter()
            .find(|a| Some(&a.id) == self.tuning.anim.ability.as_ref())
            .copied()
            .expect("chosen above");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(editable, |ui| {
                let tick = tick_at(self.tuning.anim.time / length, ability.duration);
                let add = ui
                    .add_enabled(
                        !model.cues.is_empty(),
                        egui::Button::new(format!("+ Event at tick {tick}")),
                    )
                    .on_hover_text(format!(
                        "Adds a {} cue to {} at the playhead",
                        self.tuning.anim.sound, ability.id
                    ));
                mark!(self.rects, "anim:add_event".into(), add.rect);
                if add.clicked() {
                    intents.push(Intent::Edit(
                        ability.id.clone(),
                        AbilityEventEdit::Add {
                            tick,
                            sound: self.tuning.anim.sound.clone(),
                        },
                    ));
                }
                egui::ComboBox::from_id_salt("anim_sound")
                    .selected_text(self.tuning.anim.sound.as_str())
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for c in &model.cues {
                            if ui
                                .selectable_label(*c == self.tuning.anim.sound, c)
                                .clicked()
                            {
                                self.tuning.anim.sound = c.clone();
                            }
                        }
                    });
                ui.label(RichText::new(format!("to {}", ability.id)).color(theme::MUTED));
                let Some((id, index)) = self.tuning.anim.selected.clone() else {
                    return;
                };
                let Some((tick, sound)) = rows
                    .iter()
                    .find(|a| a.id == id)
                    .and_then(|a| a.events.get(index))
                else {
                    return;
                };
                theme::divider(ui);
                ui.label(
                    RichText::new(format!(
                        "{id} #{index} · tick {tick} ({:.3} s)",
                        *tick as f32 * crate::world::FIXED_DT
                    ))
                    .color(theme::TEXT),
                );
                let combo = egui::ComboBox::from_id_salt("anim_event_sound")
                    .selected_text(sound.as_str())
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for c in &model.cues {
                            let r = ui.selectable_label(c == sound, c);
                            mark!(self.rects, format!("anim:event_sound:{c}"), r.rect);
                            if r.clicked() && c != sound {
                                intents.push(Intent::Edit(
                                    id.clone(),
                                    AbilityEventEdit::Sound {
                                        index,
                                        sound: c.clone(),
                                    },
                                ));
                            }
                        }
                    });
                mark!(self.rects, "anim:event_sound".into(), combo.response.rect);
                let delete = theme::icon_text_button(
                    ui,
                    icons,
                    Icon::Delete,
                    "Delete event",
                    None,
                    theme::TEXT,
                );
                mark!(self.rects, "anim:delete_event".into(), delete.rect);
                if delete.clicked() {
                    intents.push(Intent::Edit(id, AbilityEventEdit::Remove { index }));
                }
            });
        });
    }
}
