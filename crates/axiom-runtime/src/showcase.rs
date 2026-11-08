//! Data-authored exhibition navigation; independent of effect implementations.
use crate::{
    presentation::RenderFrame,
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
    #[serde(default)]
    pub postfx: Option<BTreeSet<String>>,
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
            if let Some(volumes) = &s.postfx {
                ensure!(volumes.len() <= 8, "showcase post effect budget exceeded");
                for id in volumes {
                    valid_id(id)?;
                }
            }
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
    /// Station view: target offset from the pedestal (right-drag pan).
    pan: Vec3,
    /// Overview: ground point the orthographic camera looks at, and its view height.
    overview_target: Vec3,
    overview_height: f32,
    orbit: bool,
    held: BTreeSet<String>,
    /// Last station the catalog list scrolled to (UI convenience only).
    scrolled: std::sync::Mutex<Option<String>>,
}
const FOV_DEGREES: f32 = 48.0;
const OVERVIEW_TARGET: Vec3 = Vec3::new(0.0, 0.0, -25.0);
const OVERVIEW_OFFSET: Vec3 = Vec3::new(0.0, 47.0, 60.0);
const OVERVIEW_HEIGHT: f32 = 44.0;
impl Showcase {
    /// Tour order: materials then particles, each by stable ID (M01.., P01..).
    fn order(&self) -> Vec<&String> {
        let mut ids = self.tour.stations.keys().collect::<Vec<_>>();
        ids.sort_by(|a, b| {
            self.tour.stations[*a]
                .category
                .cmp(&self.tour.stations[*b].category)
                .then(a.cmp(b))
        });
        ids
    }
    fn offset(&self) -> Vec3 {
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.cos() * self.pitch.cos(),
        )
    }
    fn sync(&self, state: &mut RuntimeState) {
        let slot = state.compiled.indices[&self.tour.camera];
        let (eye, target, projection) = if let Some(s) = self
            .active
            .as_ref()
            .and_then(|id| self.tour.stations.get(id))
        {
            let target = Vec3::from_array(s.origin) + Vec3::Y * 1.25 + self.pan;
            let eye = target + self.offset() * self.distance;
            (
                eye,
                target,
                Projection::Perspective {
                    fov_degrees: FOV_DEGREES,
                },
            )
        } else {
            (
                self.overview_target + OVERVIEW_OFFSET,
                self.overview_target,
                Projection::Orthographic {
                    height: self.overview_height,
                },
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
        self.reset_view();
        Ok(())
    }
    fn reset_view(&mut self) {
        self.yaw = 0.0;
        self.pitch = 0.28;
        self.distance = 7.8;
        self.pan = Vec3::ZERO;
        self.overview_target = OVERVIEW_TARGET;
        self.overview_height = OVERVIEW_HEIGHT;
    }
    /// Free camera: orbit/pitch in radians, zoom as a distance (or view height) factor,
    /// pan as a fraction of the viewport (screen right/down), aspect of the viewport.
    fn view(&mut self, args: &Value) -> Result<()> {
        let num = |key: &str, default: f32| -> Result<f32> {
            let v = args.get(key).map_or(Ok(default), |v| {
                v.as_f64()
                    .map(|v| v as f32)
                    .context("view arguments must be numbers")
            })?;
            ensure!(v.is_finite(), "view arguments must be finite");
            Ok(v)
        };
        let (orbit, pitch, zoom, aspect) = (
            num("orbit", 0.0)?,
            num("pitch", 0.0)?,
            num("zoom", 1.0)?,
            num("aspect", 16.0 / 9.0)?,
        );
        let pan: [f32; 2] = match args.get("pan") {
            Some(v) => serde_json::from_value(v.clone())?,
            None => [0.0, 0.0],
        };
        ensure!(
            zoom > 0.0 && aspect > 0.0 && pan.iter().all(|v| v.is_finite()),
            "invalid view arguments"
        );
        if self.active.is_some() {
            self.yaw += orbit;
            self.pitch = (self.pitch + pitch).clamp(-0.1, 1.45);
            self.distance = (self.distance * zoom).clamp(2.5, 24.0);
            if pan != [0.0, 0.0] {
                let back = self.offset();
                let right = Vec3::Y.cross(back).normalize_or_zero();
                let up = back.cross(right).normalize_or_zero();
                let height = 2.0 * self.distance * (FOV_DEGREES.to_radians() * 0.5).tan();
                self.pan += -right * pan[0] * height * aspect + up * pan[1] * height;
                self.pan = self.pan.clamp(Vec3::splat(-6.0), Vec3::splat(6.0));
            }
        } else {
            self.overview_height = (self.overview_height * zoom).clamp(12.0, 90.0);
            // The overview looks down the -Z axis at a fixed elevation: screen-up is ground -Z.
            let elevation = OVERVIEW_OFFSET.y / OVERVIEW_OFFSET.length();
            self.overview_target.x -= pan[0] * self.overview_height * aspect;
            self.overview_target.z -= pan[1] * self.overview_height / elevation;
            self.overview_target = self
                .overview_target
                .clamp(Vec3::new(-70.0, 0.0, -90.0), Vec3::new(70.0, 0.0, 40.0));
        }
        Ok(())
    }
    fn next(&mut self, direction: i64) {
        let ids = self.order();
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
    fn extract(&self, _state: &RuntimeState, frame: &mut RenderFrame) {
        if let Some(volumes) = self
            .active
            .as_ref()
            .and_then(|id| self.tour.stations.get(id))
            .and_then(|s| s.postfx.as_ref())
        {
            frame.postfx.retain(|v| volumes.contains(&v.id));
        }
    }
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
        // Auto-orbit and mouse input are applied per rendered frame (draw_ui -> `view`), so the
        // camera moves smoothly regardless of how many fixed steps a frame runs.
        if self.active.is_some() {
            self.yaw += input.axis("view.orbit") * 1.1 * FIXED_DT;
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
                "view" => self.view(args)?,
                "reset_view" => {
                    let (center, height) = (self.overview_target, self.overview_height);
                    self.reset_view();
                    if self.active.is_some() {
                        // Keep the overview where the visitor left it.
                        self.overview_target = center;
                        self.overview_height = height;
                    }
                }
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
        self.pan = old.pan;
        self.overview_target = old.overview_target;
        self.overview_height = old.overview_height;
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
        let muted = egui::Color32::from_rgb(140, 156, 160);
        let order = self.order();
        let index = self
            .active
            .as_ref()
            .and_then(|id| order.iter().position(|p| *p == id));
        let short = |id: &str| -> String {
            self.tour.stations[id]
                .title
                .split(" / ")
                .next()
                .unwrap_or_default()
                .to_string()
        };
        let neighbor = |step: i64| -> &String {
            let i = index.map_or(0, |i| {
                (i as i64 + step).rem_euclid(order.len() as i64) as usize
            });
            order[i]
        };
        // Header: where am I, and one click to the previous / next exhibit or the map.
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
                    let previous = neighbor(-1);
                    let next = if index.is_some() {
                        neighbor(1)
                    } else {
                        order[0]
                    };
                    if index.is_some()
                        && p.button(format!("◀ {}", short(previous)))
                            .on_hover_text("上一站 (←)")
                            .clicked()
                    {
                        actions.push(call("next", json!({"direction":-1})));
                    }
                    match index {
                        Some(i) => p.label(
                            egui::RichText::new(format!("{} / {}", i + 1, order.len()))
                                .color(gold)
                                .strong(),
                        ),
                        None => p.label(egui::RichText::new("總覽地圖").color(gold).strong()),
                    };
                    if p.button(format!("{} ▶", short(next)))
                        .on_hover_text("下一站 (→)")
                        .clicked()
                    {
                        actions.push(call("next", json!({"direction":1})));
                    }
                    p.with_layout(egui::Layout::right_to_left(egui::Align::Center), |p| {
                        if p.button("EXIT").clicked() {
                            actions.push(Action::Quit);
                        }
                        if p.button(if ui.playing { "PAUSE" } else { "PLAY" })
                            .clicked()
                        {
                            ui.playing = !ui.playing;
                        }
                        if p.add_enabled(index.is_some(), egui::Button::new("總覽地圖 [O]"))
                            .clicked()
                        {
                            actions.push(call("overview", json!({})));
                        }
                    });
                });
            });
        // Catalog in tour order; the current exhibit stays scrolled into view.
        egui::SidePanel::left("effects_catalog")
            .exact_width(250.0)
            .resizable(false)
            .show(ctx, |p| {
                p.add_space(8.0);
                p.label(egui::RichText::new("FEATURE TOUR").size(12.0).color(gold));
                p.label(egui::RichText::new("點選展台，或在地圖上點展示座").color(muted));
                p.separator();
                let mut scrolled = self.scrolled.lock().unwrap();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(p, |p| {
                        let mut category = "";
                        for id in &order {
                            let s = &self.tour.stations[*id];
                            if s.category != category {
                                category = &s.category;
                                p.add_space(4.0);
                                p.label(
                                    egui::RichText::new(if category == "materials" {
                                        "MATERIALS / 材質"
                                    } else {
                                        "PARTICLES / 粒子"
                                    })
                                    .color(mint),
                                );
                            }
                            let selected = self.active.as_ref() == Some(*id);
                            let response = p
                                .selectable_label(selected, &s.title)
                                .on_hover_text(&s.description);
                            ui.targets.insert(format!("showcase/{id}"), response.rect);
                            if selected && scrolled.as_deref() != Some(id.as_str()) {
                                response.scroll_to_me(Some(egui::Align::Center));
                                *scrolled = Some((*id).clone());
                            }
                            if response.clicked() {
                                actions.push(call("focus", json!({"station":id})));
                            }
                        }
                    });
                if self.active.is_none() {
                    *scrolled = None;
                }
            });
        egui::TopBottomPanel::bottom("effects_inspector").min_height(100.0).show(ctx, |p| {
            if let Some((id, station)) = self.active.as_ref().and_then(|id| self.tour.stations.get(id).map(|station| (id, station))) {
                p.horizontal_wrapped(|p| {
                    p.label(egui::RichText::new(&station.title).size(18.0).color(mint));
                    p.label(egui::RichText::new(&station.implementation).color(gold));
                    if p.button(if self.orbit {"停止環繞"} else {"自動環繞"}).clicked() { actions.push(call("orbit", json!({"enabled":!self.orbit}))); }
                    if p.button("重置視角").clicked() { actions.push(call("reset_view", json!({}))); }
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
                p.label(egui::RichText::new("左鍵拖曳旋轉 · 右鍵拖曳平移 · 滾輪縮放 · 雙擊重置視角 · ←/→ 上一站/下一站 · O 總覽地圖").small().color(muted));
            } else {
                p.label(egui::RichText::new("A field guide to surfaces, light and motion").size(20.0).color(mint));
                p.label("36 個獨立展台。點地圖上的展示座或左側清單進入；也可按 → 從第一站依序參觀。材質與效果參數可儲存為內容修訂。");
                p.label(egui::RichText::new("左鍵拖曳平移地圖 · 滾輪縮放 · 點選展台進入 · → 開始導覽").small().color(muted));
            }
        });
        let rect = ctx.available_rect();
        let screen = ctx.screen_rect();
        if rect.width() <= 1.0 || rect.height() <= 1.0 {
            return Some(actions);
        }
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
        // The 3D view itself takes the mouse: drag to orbit/pan, wheel to zoom.
        let response = egui::Area::new(egui::Id::new("showcase_view"))
            .fixed_pos(rect.min)
            .order(egui::Order::Background)
            .show(ctx, |p| {
                p.allocate_rect(rect, egui::Sense::click_and_drag())
            })
            .inner;
        let aspect = rect.width() / rect.height();
        let mut orbit = 0.0;
        let mut pitch = 0.0;
        let mut zoom = 1.0;
        let mut pan = [0.0f32, 0.0];
        let delta = response.drag_delta();
        let station_view = self.active.is_some();
        if station_view && response.dragged_by(egui::PointerButton::Primary) {
            orbit -= delta.x * 0.008;
            pitch += delta.y * 0.006;
        } else if response.dragged_by(egui::PointerButton::Primary)
            || response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
        {
            pan = [delta.x / rect.width(), delta.y / rect.height()];
        }
        if response.hovered() {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                zoom = (-scroll * 0.0018).exp();
            }
        }
        if station_view && self.orbit && ui.playing {
            orbit += 0.25 * ctx.input(|i| i.stable_dt).min(0.1);
        }
        if response.double_clicked() {
            actions.push(call("reset_view", json!({})));
        } else if orbit != 0.0 || pitch != 0.0 || zoom != 1.0 || pan != [0.0, 0.0] {
            actions.push(call(
                "view",
                json!({"orbit":orbit,"pitch":pitch,"zoom":zoom,"pan":pan,"aspect":aspect}),
            ));
        }
        if response.dragged() {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if station_view {
            return Some(actions);
        }
        // Overview map: labelled pedestals; hover highlights, click enters the exhibit.
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
            Projection::Perspective { fov_degrees } => {
                Mat4::perspective_rh(fov_degrees.to_radians(), aspect, camera.near, camera.far)
            }
        };
        let matrix =
            projection * Mat4::look_at_rh(eye, eye + rotation * -Vec3::Z, rotation * Vec3::Y);
        let to_screen = |p: Vec3| -> Option<egui::Pos2> {
            let p = matrix * p.extend(1.0);
            let p = p.truncate() / p.w;
            (p.x.abs() <= 1.0 && p.y.abs() <= 1.0).then(|| {
                egui::pos2(
                    (vp[0] + (p.x * 0.5 + 0.5) * vp[2]) * screen.width(),
                    (vp[1] + (0.5 - p.y * 0.5) * vp[3]) * screen.height(),
                )
            })
        };
        let pointer = response.hover_pos();
        let labels: Vec<(&String, egui::Pos2, egui::Pos2)> = order
            .iter()
            .filter_map(|id| {
                let origin = Vec3::from_array(self.tour.stations[*id].origin);
                Some((
                    *id,
                    to_screen(origin + Vec3::new(0.0, 0.25, 3.0))?,
                    to_screen(origin + Vec3::Y * 0.8)?,
                ))
            })
            .collect();
        let hovered = pointer.and_then(|p| {
            labels
                .iter()
                .map(|(id, label, pedestal)| (*id, p.distance(*label).min(p.distance(*pedestal))))
                .filter(|(_, d)| *d < 48.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(id, _)| id)
        });
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            "exhibit_labels".into(),
        ));
        // The hovered label paints last so its full name sits on top of its neighbours.
        for (id, label, _) in labels
            .iter()
            .filter(|l| hovered != Some(l.0))
            .chain(labels.iter().filter(|l| hovered == Some(l.0)))
        {
            let hot = hovered == Some(*id);
            // Codes stay readable when zoomed out; the hovered pedestal shows its full name.
            let text = if hot {
                short(id)
            } else {
                self.tour.stations[*id]
                    .title
                    .split(' ')
                    .next()
                    .unwrap_or_default()
                    .to_string()
            };
            let galley = painter.layout_no_wrap(
                text,
                egui::FontId::proportional(if hot { 14.0 } else { 12.0 }),
                if hot { egui::Color32::BLACK } else { mint },
            );
            let back = egui::Rect::from_center_size(*label, galley.size() + egui::vec2(10.0, 4.0));
            painter.rect_filled(
                back,
                4.0,
                if hot {
                    gold
                } else {
                    egui::Color32::from_rgba_unmultiplied(8, 14, 18, 190)
                },
            );
            painter.galley(
                back.min + egui::vec2(5.0, 2.0),
                galley,
                egui::Color32::WHITE,
            );
        }
        if let Some(id) = hovered {
            if !response.dragged() {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() {
                actions.push(call("focus", json!({"station":id})));
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
    for action in [
        "focus",
        "overview",
        "next",
        "orbit",
        "viewport",
        "view",
        "reset_view",
    ] {
        r.authoring.action(&format!("showcase.{action}"),"showcase",json!({"station":"stable station ID for focus","direction":"-1|1 for next","enabled":"bool for orbit","view":"{orbit?,pitch? radians, zoom? factor, pan?:[x,y] viewport fraction, aspect?}"}),|v|{ensure!(v.is_object(),"showcase arguments must be an object");Ok(())});
    }
    r.factories.insert("showcase.navigate".into(), |state| {
        let tour: Tour = state.compiled.source.resource("showcase.tour")?;
        tour.validate()?;
        for station in tour.stations.values() {
            if let Some(volumes) = &station.postfx {
                for id in volumes {
                    ensure!(
                        state
                            .compiled
                            .source
                            .systems
                            .iter()
                            .any(|s| s == "postfx.prepare")
                            && state
                                .compiled
                                .source
                                .resources
                                .get("postfx.settings")
                                .and_then(|v| v["volumes"].as_array())
                                .is_some_and(|v| v
                                    .iter()
                                    .any(|v| v["id"].as_str() == Some(id.as_str()))),
                        "showcase post effect volume missing: {id}"
                    );
                }
            }
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
            pan: Vec3::ZERO,
            overview_target: OVERVIEW_TARGET,
            overview_height: OVERVIEW_HEIGHT,
            orbit: false,
            held: BTreeSet::new(),
            scrolled: std::sync::Mutex::new(None),
        };
        system.sync(state);
        Ok(Box::new(system))
    });
    r.authoring.topics.insert("showcase".into(),json!({"resource":"showcase.tour","navigation":["showcase.focus {station}","showcase.overview {}","showcase.next {direction:-1|1}","showcase.orbit {enabled}"],"observe":"/showcase","authoring":"select resources + projected JSON pointers; set_field with expected_rev; GUI sliders use the same apply path","catalog":"content/effects-tour/inventory.json records the pinned upstream scene and native/equivalent/approximate implementation"}));
}
