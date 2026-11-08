//! Data-authored exhibition navigation; independent of effect implementations.
use crate::{
    ui::{Action, UiState},
    world::*,
};
use anyhow::{ensure, Context, Result};
use axiom_core::{gameplay::RuntimeCommand, *};
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Control {
    pub label: String,
    pub resource: String,
    pub field: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Station {
    pub title: String,
    pub source_title: String,
    pub category: String,
    pub origin: [f32; 3],
    pub implementation: String,
    pub description: String,
    #[serde(default)]
    pub controls: Vec<Control>,
    pub emitter: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tour {
    pub camera: String,
    pub stations: BTreeMap<String, Station>,
    #[serde(default)]
    pub initial: Option<String>,
}
impl Tour {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.stations.is_empty() && self.stations.len() <= 64,
            "showcase station budget exceeded"
        );
        ensure!(
            self.initial
                .as_ref()
                .is_none_or(|id| self.stations.contains_key(id)),
            "initial showcase station missing"
        );
        for (id, s) in &self.stations {
            valid_id(id)?;
            ensure!(
                s.title.len() <= 256
                    && s.description.len() <= 2048
                    && s.controls.len() <= 4
                    && s.origin.iter().all(|p| p.is_finite() && p.abs() <= 1000.0)
                    && ["materials", "particles"].contains(&s.category.as_str()),
                "invalid showcase station"
            );
            for c in &s.controls {
                ensure!(
                    c.min.is_finite()
                        && c.max.is_finite()
                        && c.default.is_finite()
                        && c.min <= c.default
                        && c.default <= c.max
                        && c.field.starts_with('/'),
                    "invalid showcase control"
                );
            }
        }
        Ok(())
    }
}
pub struct Showcase {
    tour: Tour,
    active: Option<String>,
    yaw: f32,
    pitch: f32,
    distance: f32,
    orbit: bool,
    held: BTreeSet<String>,
}
impl Showcase {
    fn sync(&self, state: &mut RuntimeState) {
        let slot = state.compiled.indices[&self.tour.camera];
        let (eye, target, projection) = if let Some(s) = self
            .active
            .as_ref()
            .and_then(|id| self.tour.stations.get(id))
        {
            let target = Vec3::from_array(s.origin) + Vec3::Y * 1.25;
            let eye = target
                + Vec3::new(
                    self.yaw.sin() * self.pitch.cos(),
                    self.pitch.sin(),
                    self.yaw.cos() * self.pitch.cos(),
                ) * self.distance;
            (eye, target, Projection::Perspective { fov_degrees: 48.0 })
        } else {
            (
                Vec3::new(0.0, 47.0, 35.0),
                Vec3::new(0.0, 0.0, -25.0),
                Projection::Orthographic { height: 59.0 },
            )
        };
        if let Some(t) = &mut state.transforms[slot] {
            t.position = eye.to_array();
            t.rotation = Quat::from_mat4(&Mat4::look_at_rh(eye, target, Vec3::Y).inverse())
                .normalize()
                .to_array();
        }
        if let Some(c) = &mut state.compiled.entities[slot].camera {
            c.projection = projection;
        }
    }
    fn focus(&mut self, id: Option<&str>) -> Result<()> {
        ensure!(
            id.is_none_or(|id| self.tour.stations.contains_key(id)),
            "showcase station missing"
        );
        self.active = id.map(str::to_owned);
        self.yaw = 0.0;
        self.pitch = 0.28;
        self.distance = 7.8;
        Ok(())
    }
    fn next(&mut self, direction: i64) {
        let mut ids = self.tour.stations.keys().collect::<Vec<_>>();
        ids.sort_by(|a, b| {
            self.tour.stations[*a]
                .category
                .cmp(&self.tour.stations[*b].category)
                .then(a.cmp(b))
        });
        let index = self
            .active
            .as_ref()
            .and_then(|id| ids.iter().position(|p| *p == id))
            .map_or(0, |i| {
                (i as i64 + direction.signum()).rem_euclid(ids.len() as i64) as usize
            });
        let id = ids[index].clone();
        let _ = self.focus(Some(&id));
    }
}
fn call(action: &str, args: Value) -> Action {
    Action::Gameplay(RuntimeCommand::Call {
        capability: "showcase".into(),
        action: action.into(),
        args,
    })
}
impl RuntimeSystem for Showcase {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn pointer_allowed(&self) -> bool {
        false
    }
    fn step(&mut self, state: &mut RuntimeState, input: &ActionMap) {
        let pressed = ["tour.next", "tour.previous", "tour.overview"]
            .into_iter()
            .filter(|key| input.pressed(key))
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        for key in pressed.difference(&self.held).cloned().collect::<Vec<_>>() {
            match key.as_str() {
                "tour.next" => self.next(1),
                "tour.previous" => self.next(-1),
                _ => {
                    let _ = self.focus(None);
                }
            }
        }
        self.held = pressed;
        if self.active.is_some() {
            self.yaw +=
                (input.axis("view.orbit") * 1.1 + if self.orbit { 0.2 } else { 0.0 }) * FIXED_DT;
            self.pitch = (self.pitch + input.axis("view.pitch") * FIXED_DT).clamp(-0.1, 1.35);
            self.distance =
                (self.distance + input.axis("view.zoom") * FIXED_DT * 4.0).clamp(3.5, 16.0);
        }
        self.sync(state);
    }
    fn command(
        &mut self,
        state: &mut RuntimeState,
        cap: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if cap != "showcase" {
            return None;
        }
        Some((|| {
            match action {
                "focus" => self.focus(Some(
                    args["station"].as_str().context("station ID required")?,
                ))?,
                "overview" => self.focus(None)?,
                "next" => self.next(args["direction"].as_i64().unwrap_or(1)),
                "orbit" => {
                    self.orbit = args["enabled"].as_bool().context("enabled bool required")?
                }
                "viewport" => {
                    let rect: [f32; 4] = serde_json::from_value(args["rect"].clone())?;
                    ensure!(
                        rect.iter()
                            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
                            && rect[2] > 0.0
                            && rect[3] > 0.0
                            && rect[0] + rect[2] <= 1.00001
                            && rect[1] + rect[3] <= 1.00001,
                        "invalid showcase viewport"
                    );
                    let slot = state.compiled.indices[&self.tour.camera];
                    state.compiled.entities[slot]
                        .camera
                        .as_mut()
                        .unwrap()
                        .viewport = rect;
                }
                _ => anyhow::bail!("unknown showcase action"),
            };
            self.sync(state);
            Ok(json!({"active":self.active,"orbit":self.orbit}))
        })())
    }
    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        previous: &RuntimeState,
        next: &mut RuntimeState,
    ) -> Result<()> {
        let old = old
            .as_any()
            .downcast_ref::<Self>()
            .context("showcase transfer")?;
        self.active = old
            .active
            .clone()
            .filter(|id| self.tour.stations.contains_key(id));
        self.yaw = old.yaw;
        self.pitch = old.pitch;
        self.distance = old.distance;
        self.orbit = old.orbit;
        self.held = old.held.clone();
        next.compiled.entities[next.compiled.indices[&self.tour.camera]]
            .camera
            .as_mut()
            .unwrap()
            .viewport = previous.compiled.entities[previous.compiled.indices[&old.tour.camera]]
            .camera
            .as_ref()
            .unwrap()
            .viewport;
        self.sync(next);
        Ok(())
    }
    fn screen_pointer(&mut self, state: &mut RuntimeState, p: &ScreenPointer) -> Result<()> {
        if self.active.is_none()
            && matches!(p.kind, PointerKind::PrimaryUp)
            && p.direction[1].abs() > 1e-6
        {
            let t = -p.origin[1] / p.direction[1];
            if t > 0.0 {
                let hit = Vec3::from_array(p.origin) + Vec3::from_array(p.direction) * t;
                let id = self
                    .tour
                    .stations
                    .iter()
                    .find(|(_, s)| (Vec3::from_array(s.origin) - hit).length() < 3.8)
                    .map(|(id, _)| id.clone());
                if let Some(id) = id {
                    self.focus(Some(&id))?;
                    self.sync(state);
                }
            }
        }
        Ok(())
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([(
            "showcase".into(),
            json!({"active":self.active,"stations":self.tour.stations.len(),"materials":self.tour.stations.values().filter(|s|s.category=="materials").count(),"particles":self.tour.stations.values().filter(|s|s.category=="particles").count(),"orbit":self.orbit}),
        )])
    }
    fn draw_ui(
        &self,
        ctx: &egui::Context,
        state: &RuntimeState,
        ui: &mut UiState,
    ) -> Option<Vec<Action>> {
        let mut actions = vec![];
        let mint = egui::Color32::from_rgb(102, 245, 205);
        let gold = egui::Color32::from_rgb(242, 184, 108);
        egui::TopBottomPanel::top("effects_header")
            .exact_height(52.0)
            .show(ctx, |p| {
                p.horizontal_centered(|p| {
                    p.label(
                        egui::RichText::new("AXIOM / EFFECTS ATLAS")
                            .size(18.0)
                            .color(mint),
                    );
                    p.separator();
                    p.label("24 MATERIALS · 12 VFX");
                    p.with_layout(egui::Layout::right_to_left(egui::Align::Center), |p| {
                        if p.button("EXIT").clicked() {
                            actions.push(Action::Quit);
                        }
                        if p.button(if ui.playing { "PAUSE" } else { "PLAY" })
                            .clicked()
                        {
                            ui.playing = !ui.playing;
                        }
                        if p.button("OVERVIEW [O]").clicked() {
                            actions.push(call("overview", json!({})));
                        }
                    });
                });
            });
        egui::SidePanel::left("effects_catalog")
            .exact_width(240.0)
            .resizable(false)
            .show(ctx, |p| {
                p.add_space(8.0);
                p.label(egui::RichText::new("FEATURE TOUR").size(12.0).color(gold));
                p.label("選擇展台，或點選場中的展示座");
                p.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(p, |p| {
                        for category in ["materials", "particles"] {
                            p.label(
                                egui::RichText::new(if category == "materials" {
                                    "MATERIALS / 材質"
                                } else {
                                    "PARTICLES / 粒子"
                                })
                                .color(mint),
                            );
                            for (id, s) in self
                                .tour
                                .stations
                                .iter()
                                .filter(|(_, s)| s.category == category)
                            {
                                let response =
                                    p.selectable_label(self.active.as_ref() == Some(id), &s.title);
                                ui.targets.insert(format!("showcase/{id}"), response.rect);
                                if response.clicked() {
                                    actions.push(call("focus", json!({"station":id})));
                                }
                            }
                        }
                    });
            });
        egui::TopBottomPanel::bottom("effects_inspector").min_height(100.0).show(ctx, |p| {
            if let Some((id, station)) = self.active.as_ref().and_then(|id| self.tour.stations.get(id).map(|station| (id, station))) {
                p.horizontal_wrapped(|p| {
                    p.label(egui::RichText::new(&station.title).size(18.0).color(mint));
                    p.label(egui::RichText::new(&station.implementation).color(gold));
                    if p.button("◀").clicked() { actions.push(call("next", json!({"direction":-1}))); }
                    if p.button("▶").clicked() { actions.push(call("next", json!({"direction":1}))); }
                    if p.button(if self.orbit {"停止環繞"} else {"自動環繞"}).clicked() { actions.push(call("orbit", json!({"enabled":!self.orbit}))); }
                    if let Some(emitter) = &station.emitter {
                        if p.button("重播 VFX").clicked() { actions.push(Action::Gameplay(RuntimeCommand::Call { capability:"vfx".into(), action:"restart".into(), args:json!({"id":emitter}) })); }
                    }
                });
                p.label(&station.description);
                p.horizontal_wrapped(|p| {
                    for (index, control) in station.controls.iter().enumerate() {
                        let key = format!("{id}/{index}");
                        let stored = state.compiled.source.resources.get(&control.resource).and_then(|v|v.pointer(&control.field)).and_then(Value::as_f64).unwrap_or(control.default);
                        let draft = ui.numeric_drafts.entry(key.clone()).or_insert(stored);
                        let response = p.add(egui::Slider::new(draft,control.min..=control.max).text(&control.label).clamping(egui::SliderClamping::Always));
                        ui.targets.insert(format!("showcase/control/{id}/{index}"),response.rect);
                        if response.drag_stopped() || (response.changed() && !response.dragged()) {
                            let value = *draft;
                            ui.numeric_drafts.remove(&key);
                            actions.push(Action::Apply(json!({"op":"apply","expected_rev":state.revision,"edits":[{"kind":"set_field","target":{"kind":"resource","id":control.resource},"field":control.field,"value":value}]}).to_string()));
                        } else if !response.dragged() && !response.changed() { *draft = stored; }
                    }
                });
            } else {
                p.label(egui::RichText::new("A field guide to surfaces, light and motion").size(20.0).color(mint));
                p.label("36 個獨立展台。左右方向鍵切換；W/S 調整俯仰、A/D 環繞、Q/E 縮放。材質與效果參數可儲存為內容修訂。");
            }
        });
        let rect = ctx.available_rect();
        let screen = ctx.screen_rect();
        if rect.width() > 1.0 && rect.height() > 1.0 {
            let desired = [
                rect.left() / screen.width(),
                rect.top() / screen.height(),
                rect.width() / screen.width(),
                rect.height() / screen.height(),
            ];
            let slot = state.compiled.indices[&self.tour.camera];
            let camera = state.compiled.entities[slot].camera.as_ref().unwrap();
            if desired
                .iter()
                .zip(camera.viewport)
                .any(|(a, b)| (a - b).abs() > 0.002)
            {
                actions.push(call("viewport", json!({"rect":desired})));
            }
            if self.active.is_none() {
                let t = state.transforms[slot].as_ref().unwrap();
                let eye = Vec3::from_array(t.position);
                let rotation = Quat::from_array(t.rotation);
                let vp = camera.viewport;
                let aspect = screen.width() * vp[2] / (screen.height() * vp[3]);
                let projection = match camera.projection {
                    Projection::Orthographic { height } => Mat4::orthographic_rh(
                        -height * aspect * 0.5,
                        height * aspect * 0.5,
                        -height * 0.5,
                        height * 0.5,
                        camera.near,
                        camera.far,
                    ),
                    Projection::Perspective { fov_degrees } => Mat4::perspective_rh(
                        fov_degrees.to_radians(),
                        aspect,
                        camera.near,
                        camera.far,
                    ),
                };
                let matrix = projection
                    * Mat4::look_at_rh(eye, eye + rotation * -Vec3::Z, rotation * Vec3::Y);
                let painter = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Background,
                    "exhibit_labels".into(),
                ));
                for s in self.tour.stations.values() {
                    let p = matrix
                        * (Vec3::from_array(s.origin) + Vec3::new(0.0, 0.25, 3.0)).extend(1.0);
                    let p = p.truncate() / p.w;
                    if p.x.abs() > 1.0 || p.y.abs() > 1.0 {
                        continue;
                    }
                    let position = egui::pos2(
                        (vp[0] + (p.x * 0.5 + 0.5) * vp[2]) * screen.width(),
                        (vp[1] + (0.5 - p.y * 0.5) * vp[3]) * screen.height(),
                    );
                    painter.text(
                        position,
                        egui::Align2::CENTER_CENTER,
                        s.title.split(' ').next().unwrap_or(""),
                        egui::FontId::monospace(10.5),
                        mint,
                    );
                }
            }
        }
        Some(actions)
    }
}
pub fn register(r: &mut RuntimeRegistry) {
    r.authoring.capabilities.insert("showcase".into(), 1);
    r.authoring.resource("showcase.tour","showcase",json!({"camera":"entity ID","stations":"stable ID -> {title,source_title,category,origin,implementation,description,controls,emitter}"}),|v|serde_json::from_value::<Tour>(v.clone())?.validate());
    r.authoring.systems.insert(
        "showcase.navigate".into(),
        SystemDescriptor {
            id: "showcase.navigate".into(),
            capability: "showcase".into(),
            phase: 98,
            reads: vec!["showcase.tour".into()],
            writes: vec!["core.transform".into()],
            after: vec![],
        },
    );
    for action in ["focus", "overview", "next", "orbit", "viewport"] {
        r.authoring.action(&format!("showcase.{action}"),"showcase",json!({"station":"stable station ID for focus","direction":"-1|1 for next","enabled":"bool for orbit"}),|v|{ensure!(v.is_object(),"showcase arguments must be an object");Ok(())});
    }
    r.factories.insert("showcase.navigate".into(), |state| {
        let tour: Tour = state.compiled.source.resource("showcase.tour")?;
        tour.validate()?;
        for station in tour.stations.values() {
            for control in &station.controls {
                ensure!(
                    state
                        .compiled
                        .source
                        .resources
                        .get(&control.resource)
                        .and_then(|v| v.pointer(&control.field))
                        .and_then(Value::as_f64)
                        .is_some(),
                    "showcase control target must reference a numeric resource field: {}{}",
                    control.resource,
                    control.field
                );
            }
        }
        let slot = *state
            .compiled
            .indices
            .get(&tour.camera)
            .context("showcase camera missing")?;
        ensure!(
            state.compiled.entities[slot].camera.is_some(),
            "showcase camera component missing"
        );
        let system = Showcase {
            active: tour.initial.clone(),
            tour,
            yaw: 0.0,
            pitch: 0.28,
            distance: 7.8,
            orbit: false,
            held: BTreeSet::new(),
        };
        system.sync(state);
        Ok(Box::new(system))
    });
    r.authoring.topics.insert("showcase".into(),json!({"resource":"showcase.tour","navigation":["showcase.focus {station}","showcase.overview {}","showcase.next {direction:-1|1}","showcase.orbit {enabled}"],"observe":"/showcase","authoring":"select resources + projected JSON pointers; set_field with expected_rev; GUI sliders use the same apply path","catalog":"content/effects-tour/inventory.json records the pinned upstream scene and native/equivalent/approximate implementation"}));
}
