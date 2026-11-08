use super::{Action, Editor};
use crate::{presentation::RenderView, world::WorldRuntime};
use axiom_core::{Camera, Projection, Transform};
use egui::{pos2, vec2, Color32, PointerButton, Pos2, Rect, Stroke, Vec2};
use glam::{EulerRot, Mat4, Quat, Vec3, Vec4};

const FOV: f32 = 52.0;
const NEAR: f32 = 0.04;
const FAR: f32 = 8000.0;
const AXES: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
const COLORS: [Color32; 3] = [
    Color32::from_rgb(255, 106, 103),
    Color32::from_rgb(130, 224, 134),
    Color32::from_rgb(117, 173, 255),
];

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Tool {
    Select,
    Move,
    Rotate,
    Scale,
}
impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Move => "Move",
            Self::Rotate => "Rotate",
            Self::Scale => "Scale",
        }
    }
}

pub(super) struct Orbit {
    target: Vec3,
    distance: f32,
    yaw: f32,
    pitch: f32,
}
impl Default for Orbit {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 0.8, 0.0),
            distance: 12.5,
            yaw: 0.70,
            pitch: 0.45,
        }
    }
}
impl Orbit {
    fn eye(&self) -> Vec3 {
        self.target
            + self.distance
                * Vec3::new(
                    self.pitch.cos() * self.yaw.sin(),
                    self.pitch.sin(),
                    self.pitch.cos() * self.yaw.cos(),
                )
    }
    fn matrix(&self, rect: Rect) -> Mat4 {
        Mat4::perspective_rh(
            FOV.to_radians(),
            rect.width() / rect.height().max(1.0),
            NEAR,
            FAR,
        ) * Mat4::look_at_rh(self.eye(), self.target, Vec3::Y)
    }
    pub fn view(&self, rect: Rect, screen: Rect) -> RenderView {
        let rect = rect.intersect(screen);
        let viewport = if rect.is_positive() {
            [
                (rect.min.x - screen.min.x) / screen.width(),
                (rect.min.y - screen.min.y) / screen.height(),
                rect.width() / screen.width(),
                rect.height() / screen.height(),
            ]
        } else {
            [0.0, 0.0, 1.0, 1.0]
        };
        RenderView {
            id: "editor.orbit".into(),
            eye: self.eye(),
            forward: (self.target - self.eye()).normalize(),
            up: Vec3::Y,
            instances: vec![],
            camera: Camera {
                projection: Projection::Perspective { fov_degrees: FOV },
                near: NEAR,
                far: FAR,
                viewport,
                active: true,
            },
        }
    }
    fn pan(&mut self, delta: Vec2, height: f32) {
        let forward = (self.target - self.eye()).normalize();
        let right = forward.cross(Vec3::Y).normalize();
        let up = right.cross(forward);
        self.target += (-right * delta.x + up * delta.y) * self.distance / height.max(1.0);
    }
}

pub(super) struct Drag {
    initial: Transform,
    origin: Pos2,
    screen_axis: Vec2,
    parent_inverse: Mat4,
    axis: usize,
    world_units: f32,
    tool: Tool,
}
impl Drag {
    fn transform(&self, cursor: Pos2) -> Transform {
        let mut next = self.initial.clone();
        let pixels = (cursor - self.origin).dot(self.screen_axis);
        match self.tool {
            Tool::Move => {
                let offset = self
                    .parent_inverse
                    .transform_vector3(AXES[self.axis] * pixels * self.world_units);
                next.position = (Vec3::from_array(next.position) + offset)
                    .clamp(Vec3::splat(-100000.0), Vec3::splat(100000.0))
                    .to_array();
            }
            Tool::Rotate => {
                next.rotation = (Quat::from_array(next.rotation)
                    * Quat::from_axis_angle(AXES[self.axis], pixels * 0.012))
                .normalize()
                .to_array();
            }
            Tool::Scale => {
                next.scale[self.axis] =
                    (next.scale[self.axis] * (pixels * 0.014).exp()).clamp(0.0001, 10000.0)
            }
            Tool::Select => {}
        }
        next
    }
}

pub(super) fn degrees(transform: &Transform) -> [f32; 3] {
    let (x, y, z) = Quat::from_array(transform.rotation).to_euler(EulerRot::XYZ);
    [x.to_degrees(), y.to_degrees(), z.to_degrees()]
}
pub(super) fn rotation([x, y, z]: [f32; 3]) -> [f32; 4] {
    Quat::from_euler(
        EulerRot::XYZ,
        x.to_radians(),
        y.to_radians(),
        z.to_radians(),
    )
    .normalize()
    .to_array()
}

struct ProjectionView {
    matrix: Mat4,
    rect: Rect,
}
impl ProjectionView {
    fn project(&self, point: Vec3) -> Option<Pos2> {
        let clip = self.matrix * point.extend(1.0);
        if clip.w < NEAR || !clip.is_finite() {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(pos2(
            self.rect.left() + (ndc.x + 1.0) * self.rect.width() / 2.0,
            self.rect.top() + (1.0 - ndc.y) * self.rect.height() / 2.0,
        ))
    }
    fn ray(&self, p: Pos2) -> (Vec3, Vec3) {
        let x = 2.0 * (p.x - self.rect.left()) / self.rect.width() - 1.0;
        let y = 1.0 - 2.0 * (p.y - self.rect.top()) / self.rect.height();
        let inverse = self.matrix.inverse();
        let unproject = |z| {
            let v = inverse * Vec4::new(x, y, z, 1.0);
            v.truncate() / v.w
        };
        let start = unproject(0.0);
        (start, (unproject(0.75) - start).normalize())
    }
}

fn box_hit(origin: Vec3, direction: Vec3, matrix: Mat4) -> Option<f32> {
    let inv = matrix.inverse();
    let origin = inv.transform_point3(origin);
    let direction = inv.transform_vector3(direction);
    if !origin.is_finite() || !direction.is_finite() {
        return None;
    }
    let mut near: f32 = 0.0;
    let mut far = FAR;
    for i in 0..3 {
        if direction[i].abs() < 1e-10 {
            if origin[i].abs() > 0.5 {
                return None;
            }
            continue;
        }
        let a = (-0.5 - origin[i]) / direction[i];
        let b = (0.5 - origin[i]) / direction[i];
        near = near.max(a.min(b));
        far = far.min(a.max(b));
        if near > far {
            return None;
        }
    }
    Some(near)
}

fn pick(
    runtime: &WorldRuntime,
    matrices: &[Mat4],
    view: &ProjectionView,
    p: Pos2,
) -> Option<String> {
    let (origin, direction) = view.ray(p);
    let state = &runtime.state.compiled;
    let mut best = None;
    let mut depth = FAR;
    for (i, (entity, visible)) in state
        .entities
        .iter()
        .zip(&runtime.authored_visibility)
        .enumerate()
    {
        if !visible {
            continue;
        }
        let matrix = matrices[i];
        let distance = if let Some(mesh) = &entity.mesh {
            mesh.parts
                .iter()
                .filter_map(|part| box_hit(origin, direction, matrix * part.transform.matrix()))
                .min_by(f32::total_cmp)
        } else if entity.transform.is_some() {
            let point = matrix.transform_point3(Vec3::ZERO);
            view.project(point)
                .filter(|pos| pos.distance(p) <= 9.0)
                .map(|_| (point - origin).dot(direction))
        } else {
            None
        };
        if let Some(distance) = distance.filter(|&d| d >= 0.0 && d < depth) {
            best = Some(entity.id.clone());
            depth = distance;
        }
    }
    best
}

fn draw_box(painter: &egui::Painter, view: &ProjectionView, matrix: Mat4) {
    for i in 0..8 {
        let corner = |j: usize| {
            Vec3::new(
                (j & 1) as f32 - 0.5,
                ((j >> 1) & 1) as f32 - 0.5,
                ((j >> 2) & 1) as f32 - 0.5,
            )
        };
        for axis in [1, 2, 4] {
            if i & axis == 0 {
                draw_line(
                    painter,
                    view,
                    matrix.transform_point3(corner(i)),
                    matrix.transform_point3(corner(i | axis)),
                    Stroke::new(1.2_f32, Color32::from_rgb(252, 188, 111)),
                );
            }
        }
    }
}
fn draw_line(painter: &egui::Painter, view: &ProjectionView, a: Vec3, b: Vec3, stroke: Stroke) {
    if let (Some(a), Some(b)) = (view.project(a), view.project(b)) {
        painter.line_segment([a, b], stroke);
    }
}

struct Handles {
    pivot: Pos2,
    tips: [Option<Pos2>; 3],
    units: f32,
}
impl Handles {
    fn new(view: &ProjectionView, orbit: &Orbit, model: Mat4, tool: Tool) -> Option<Self> {
        let world = model.transform_point3(Vec3::ZERO);
        let pivot = view.project(world)?;
        let units = (world - orbit.eye()).length() * 92.0 / view.rect.height().max(1.0);
        let tips = AXES.map(|axis| {
            let axis = if tool == Tool::Move {
                axis
            } else {
                model.transform_vector3(axis).normalize_or_zero()
            };
            view.project(world + axis * units)
        });
        Some(Self { pivot, tips, units })
    }
    fn hit(&self, cursor: Pos2) -> Option<usize> {
        self.tips
            .iter()
            .enumerate()
            .filter_map(|(i, tip)| {
                let segment = (*tip)? - self.pivot;
                if segment.length() < 12.0 {
                    return None;
                }
                let t = (cursor - self.pivot).dot(segment) / segment.length_sq();
                let d = cursor.distance(self.pivot + segment * t.clamp(0.20, 1.0));
                (t > 0.14 && d < 8.0).then_some((i, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
    fn draw(&self, painter: &egui::Painter, tool: Tool, highlighted: Option<usize>) {
        painter.circle_filled(self.pivot, 3.5, Color32::WHITE);
        for (i, tip) in self.tips.iter().enumerate() {
            let Some(tip) = tip else { continue };
            let stroke = Stroke::new(
                2.5_f32,
                if highlighted == Some(i) {
                    Color32::WHITE
                } else {
                    COLORS[i]
                },
            );
            let vector = *tip - self.pivot;
            if vector.length() < 5.0 {
                continue;
            }
            if tool == Tool::Move {
                painter.arrow(self.pivot, vector, stroke);
            } else {
                painter.line_segment([self.pivot, *tip], stroke);
                if tool == Tool::Scale {
                    painter.rect_filled(
                        Rect::from_center_size(*tip, Vec2::splat(8.0)),
                        1.5,
                        stroke.color,
                    );
                } else {
                    painter.circle_stroke(*tip, 5.0, stroke);
                }
            }
            painter.text(
                *tip + vec2(7.0, -7.0),
                egui::Align2::LEFT_BOTTOM,
                ["X", "Y", "Z"][i],
                egui::FontId::monospace(11.0),
                stroke.color,
            );
        }
    }
}

impl Editor {
    pub(super) fn scene_view(
        &mut self,
        ui: &mut egui::Ui,
        runtime: &WorldRuntime,
        actions: &mut Vec<Action>,
    ) {
        let rect = ui.available_rect_before_wrap();
        self.viewport = rect;
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let ready = self.rev == runtime.rev;
        let matrices = runtime.state.matrices();
        let state = &runtime.state.compiled;
        let visible = |i| runtime.authored_visibility.get(i).copied().unwrap_or(false);
        let selected = self
            .selected
            .as_ref()
            .and_then(|id| state.indices.get(id))
            .copied();
        let delta = ui.input(|i| i.pointer.delta());
        if self.drag.is_none() {
            if response.dragged_by(PointerButton::Secondary) {
                self.orbit.yaw -= delta.x * 0.008;
                self.orbit.pitch = (self.orbit.pitch + delta.y * 0.008).clamp(-1.52, 1.52);
            }
            if response.dragged_by(PointerButton::Middle) {
                self.orbit.pan(delta, rect.height());
            }
            if response.hovered() {
                let wheel = ui.input(|i| i.smooth_scroll_delta.y);
                self.orbit.distance =
                    (self.orbit.distance * (-wheel * 0.003).exp()).clamp(0.25, 4000.0);
            }
        }
        if std::mem::take(&mut self.frame_selection) {
            if let Some(i) = selected {
                let matrix = matrices[i];
                self.orbit.target = matrix.transform_point3(Vec3::ZERO);
                let extent = matrix
                    .x_axis
                    .truncate()
                    .length()
                    .max(matrix.y_axis.truncate().length())
                    .max(matrix.z_axis.truncate().length());
                self.orbit.distance = (extent * 4.4).clamp(1.3, 4000.0);
            } else {
                self.orbit = Orbit::default();
            }
        }
        let view = ProjectionView {
            matrix: self.orbit.matrix(rect),
            rect,
        };
        if !state
            .entities
            .iter()
            .enumerate()
            .any(|(i, e)| visible(i) && e.mesh.is_some())
        {
            for i in -8..=8 {
                let p = i as f32;
                let stroke = Stroke::new(
                    1.0_f32,
                    Color32::from_white_alpha(if i == 0 { 78 } else { 30 }),
                );
                draw_line(
                    &painter,
                    &view,
                    Vec3::new(-8.0, 0.0, p),
                    Vec3::new(8.0, 0.0, p),
                    stroke,
                );
                draw_line(
                    &painter,
                    &view,
                    Vec3::new(p, 0.0, -8.0),
                    Vec3::new(p, 0.0, 8.0),
                    stroke,
                );
            }
        }
        // Spatial objects without a primitive mesh use a pickable pivot marker.
        for (i, entity) in state
            .entities
            .iter()
            .take(runtime.authored_visibility.len())
            .enumerate()
        {
            if visible(i) && entity.mesh.is_none() && entity.transform.is_some() {
                let position = matrices[i].transform_point3(Vec3::ZERO);
                if let Some(point) = view.project(position).filter(|p| rect.contains(*p)) {
                    let color = if selected == Some(i) {
                        Color32::from_rgb(252, 188, 111)
                    } else {
                        Color32::from_rgb(180, 203, 225)
                    };
                    painter.circle_stroke(point, 4.0, Stroke::new(1.2_f32, color));
                    if entity.camera.is_some() {
                        draw_line(
                            &painter,
                            &view,
                            position,
                            position
                                + matrices[i].transform_vector3(-Vec3::Z).normalize_or_zero() * 0.8,
                            Stroke::new(1.2_f32, color),
                        );
                    }
                }
            }
        }
        let parent = selected
            .and_then(|i| state.parents[i])
            .map_or(Mat4::IDENTITY, |i| matrices[i]);
        let display = selected.map(|i| {
            self.draft
                .as_ref()
                .filter(|d| d.transform_dirty && d.rev == runtime.rev)
                .and_then(|d| d.transform.as_ref())
                .filter(|t| t.validate().is_ok())
                .map_or(matrices[i], |t| parent * t.matrix())
        });
        let handles = display
            .filter(|_| {
                self.tool != Tool::Select
                    && ready
                    && selected.is_some_and(visible)
                    && self.draft.as_ref().is_some_and(|d| d.transform.is_some())
            })
            .and_then(|m| Handles::new(&view, &self.orbit, m, self.tool));
        let hovered_axis = response
            .hover_pos()
            .and_then(|pos| handles.as_ref()?.hit(pos));
        if response.drag_started_by(PointerButton::Primary) && !self.dirty() {
            let origin = ui.input(|i| i.pointer.press_origin());
            if let (Some(origin), Some(handles), Some(initial)) = (
                origin,
                &handles,
                self.draft.as_ref().and_then(|d| d.transform.clone()),
            ) {
                if let Some(axis) = handles.hit(origin) {
                    let vector = handles.tips[axis].unwrap() - handles.pivot;
                    self.drag = Some(Drag {
                        initial,
                        origin,
                        screen_axis: vector.normalized(),
                        parent_inverse: parent.inverse(),
                        axis,
                        world_units: handles.units / vector.length(),
                        tool: self.tool,
                    });
                }
            }
        }
        if let Some(drag) = &self.drag {
            let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused);
            if cancel {
                self.select(self.selected.clone());
            } else if let Some(draft) = &mut self.draft {
                if let Some(cursor) = ui.input(|i| i.pointer.latest_pos()) {
                    let t = drag.transform(cursor);
                    draft.transform_dirty = t != drag.initial;
                    draft.angles = degrees(&t);
                    draft.transform = Some(t);
                }
                if response.drag_stopped_by(PointerButton::Primary) {
                    if draft.transform_dirty {
                        actions.push(Action::Write {
                            request: axiom_core::Request::Apply {
                                expected_rev: Some(draft.rev),
                                edits: draft.transform_edits(),
                            },
                            select: None,
                        });
                    }
                    self.drag = None;
                } else if !ui.input(|i| i.pointer.primary_down()) {
                    self.select(self.selected.clone());
                }
            }
        } else if ready && response.clicked_by(PointerButton::Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                if hovered_axis.is_none() {
                    self.try_select(pick(runtime, &matrices, &view, pos));
                }
            }
        }
        if let (Some(i), Some(model)) = (selected, display) {
            if visible(i) {
                if let Some(mesh) = &state.entities[i].mesh {
                    for part in &mesh.parts {
                        draw_box(&painter, &view, model * part.transform.matrix());
                    }
                }
                if let Some(handles) = handles {
                    handles.draw(
                        &painter,
                        self.tool,
                        self.drag.as_ref().map(|d| d.axis).or(hovered_axis),
                    );
                }
            }
        }
        let anchor = rect.left_bottom() + vec2(12.0, -12.0);
        painter.text(
            anchor,
            egui::Align2::LEFT_BOTTOM,
            "RMB orbit    MMB pan    Wheel zoom    F frame",
            egui::FontId::monospace(11.0),
            Color32::from_rgb(216, 225, 238),
        );
        let axes = rect.right_bottom() + vec2(-37.0, -32.0);
        let direction = (self.orbit.target - self.orbit.eye()).normalize();
        let right = direction.cross(Vec3::Y).normalize();
        let up = right.cross(direction);
        for (i, axis) in AXES.iter().enumerate() {
            let tip = axes + vec2(axis.dot(right), -axis.dot(up)) * 19.0;
            painter.line_segment([axes, tip], Stroke::new(2.0_f32, COLORS[i]));
            painter.text(
                tip,
                egui::Align2::CENTER_BOTTOM,
                ["X", "Y", "Z"][i],
                egui::FontId::monospace(10.0),
                COLORS[i],
            );
        }
        if !ready {
            painter.text(
                rect.center_top() + vec2(0.0, 16.0),
                egui::Align2::CENTER_TOP,
                format!("Showing last renderable revision {}", runtime.rev),
                egui::FontId::proportional(13.0),
                Color32::from_rgb(255, 188, 106),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_core::{CoreEdit, Edit, Request, Store, World};

    fn fixture() -> (World, WorldRuntime) {
        let world: World =
            serde_json::from_str(include_str!("../../../../content/editor-lab/world.json"))
                .unwrap();
        let runtime = WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap();
        (world, runtime)
    }

    fn frame(
        ctx: &egui::Context,
        editor: &mut Editor,
        runtime: &WorldRuntime,
        events: Vec<egui::Event>,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 760.0))),
                events,
                ..Default::default()
            },
            |ctx| actions = editor.draw(ctx, runtime),
        );
        actions
    }

    fn primary(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn viewport_select_and_axis_drag_commit_once_against_the_original_revision() {
        let (world, mut runtime) = fixture();
        let mut editor = Editor::new(world.clone(), 1, None);
        let ctx = egui::Context::default();
        frame(&ctx, &mut editor, &runtime, vec![]);
        let slot = runtime.state.compiled.indices["stage/crate"];
        let model = runtime.state.matrices()[slot];
        let projection = ProjectionView {
            matrix: editor.orbit.matrix(editor.viewport),
            rect: editor.viewport,
        };
        let point = projection
            .project(model.transform_point3(Vec3::ZERO))
            .unwrap();
        assert_eq!(
            pick(&runtime, &runtime.state.matrices(), &projection, point).as_deref(),
            Some("stage/crate")
        );
        frame(
            &ctx,
            &mut editor,
            &runtime,
            vec![egui::Event::PointerMoved(point), primary(point, true)],
        );
        frame(&ctx, &mut editor, &runtime, vec![primary(point, false)]);
        assert_eq!(editor.observation()["selected"], "stage/crate");
        frame(&ctx, &mut editor, &runtime, vec![]);
        let projection = ProjectionView {
            matrix: editor.orbit.matrix(editor.viewport),
            rect: editor.viewport,
        };
        let handles = Handles::new(&projection, &editor.orbit, model, Tool::Move).unwrap();
        let start = handles.tips[0].unwrap();
        let end = start + (start - handles.pivot).normalized() * 27.0;
        assert!(editor.viewport.contains(end));
        assert_eq!(handles.hit(start), Some(0));
        assert!(frame(
            &ctx,
            &mut editor,
            &runtime,
            vec![egui::Event::PointerMoved(start), primary(start, true)]
        )
        .is_empty());
        assert!(frame(
            &ctx,
            &mut editor,
            &runtime,
            vec![egui::Event::PointerMoved(end)]
        )
        .is_empty());
        assert!(editor.dirty());
        let committed_before = runtime.state.transforms[slot].clone();
        runtime.state.compiled.enabled[slot] = false;
        let preview = editor.render_frame(&mut runtime, ctx.screen_rect());
        assert_eq!(preview.views[0].id, "editor.orbit");
        assert_eq!(runtime.state.transforms[slot], committed_before);
        assert!(!runtime.state.compiled.enabled[slot]);
        assert!(preview.instances.len() > runtime.render_frame().instances.len());
        let actions = frame(&ctx, &mut editor, &runtime, vec![primary(end, false)]);
        let [Action::Write { request, .. }] = actions.as_slice() else {
            panic!("expected one write on release");
        };
        let Request::Apply {
            expected_rev: Some(1),
            edits,
        } = request
        else {
            panic!("missing original revision");
        };
        let [Edit::Core(CoreEdit::SetField { field, value, .. })] = edits.as_slice() else {
            panic!("expected one position edit");
        };
        assert_eq!(field, "/components/core.transform/position");
        assert!(value[0].as_f64().unwrap() > 1.42);
        let mut store = Store::memory_with(crate::installed_registry().authoring, world).unwrap();
        assert_eq!(store.execute(request.clone()).unwrap()["rev"], 2);
        assert!(frame(&ctx, &mut editor, &runtime, vec![]).is_empty());
    }

    #[test]
    fn external_edit_keeps_stale_draft_until_explicit_reload() {
        let (mut world, runtime) = fixture();
        let mut editor = Editor::new(world.clone(), 1, Some("stage/crate".into()));
        let draft = editor.draft.as_mut().unwrap();
        draft.label = "Local edit".into();
        draft.label_dirty = true;
        world.entity_mut("stage/crate").unwrap().label = Some("Agent edit".into());
        editor.update(2, world, false);
        assert_eq!(editor.observation()["draft_rev"], 1);
        assert_eq!(editor.observation()["authoring_rev"], 2);
        assert_eq!(editor.draft.as_ref().unwrap().label, "Local edit");
        assert_eq!(editor.changes, ["changed  /entities/@stage~1crate/label"]);
        assert!(editor.dirty());
        assert!(frame(&egui::Context::default(), &mut editor, &runtime, vec![]).is_empty());
        editor.reset_draft();
        assert_eq!(editor.draft.as_ref().unwrap().label, "Agent edit");
        assert!(!editor.dirty());
    }
}
