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
    /// Gizmo result; with `snap`, moves step 0.25 m, rotations 15°, scales 0.1.
    fn transform(&self, cursor: Pos2, snap: bool) -> Transform {
        let mut next = self.initial.clone();
        let pixels = (cursor - self.origin).dot(self.screen_axis);
        let step = |x: f32, s: f32| if snap { (x / s).round() * s } else { x };
        match self.tool {
            Tool::Move => {
                let offset = self
                    .parent_inverse
                    .transform_vector3(AXES[self.axis] * step(pixels * self.world_units, 0.25));
                next.position = (Vec3::from_array(next.position) + offset)
                    .clamp(Vec3::splat(-100000.0), Vec3::splat(100000.0))
                    .to_array();
            }
            Tool::Rotate => {
                next.rotation = (Quat::from_array(next.rotation)
                    * Quat::from_axis_angle(
                        AXES[self.axis],
                        step(pixels * 0.012, 15f32.to_radians()),
                    ))
                .normalize()
                .to_array();
            }
            Tool::Scale => {
                next.scale[self.axis] = step(next.scale[self.axis] * (pixels * 0.014).exp(), 0.1)
                    .clamp(if snap { 0.1 } else { 0.0001 }, 10000.0)
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

/// World-space center and full extent (largest box side, matching the previous framing scale). Mesh parts are unit boxes placed by
/// their own transforms, so offset parts move the center instead of the entity pivot.
pub(super) fn focus_bounds(matrix: Mat4, mesh: Option<&axiom_core::RenderMesh>) -> (Vec3, f32) {
    let corners = |m: Mat4| {
        (0..8).map(move |j: usize| {
            m.transform_point3(Vec3::new(
                (j & 1) as f32 - 0.5,
                ((j >> 1) & 1) as f32 - 0.5,
                ((j >> 2) & 1) as f32 - 0.5,
            ))
        })
    };
    let points: Vec<Vec3> = match mesh.filter(|m| !m.parts.is_empty()) {
        Some(mesh) => mesh
            .parts
            .iter()
            .flat_map(|part| corners(matrix * part.transform.matrix()))
            .collect(),
        None => corners(matrix).collect(),
    };
    let (min, max) = points.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    );
    let center = (min + max) * 0.5;
    let extent = (max - min).max_element();
    if center.is_finite() && extent.is_finite() {
        (center, extent)
    } else {
        (matrix.transform_point3(Vec3::ZERO), 1.0)
    }
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

fn draw_box(painter: &egui::Painter, view: &ProjectionView, matrix: Mat4, color: Color32) {
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
                    Stroke::new(1.2_f32, color),
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
                let (center, extent) = focus_bounds(matrices[i], state.entities[i].mesh.as_ref());
                self.orbit.target = center;
                self.orbit.distance = (extent * 4.4).clamp(1.3, 4000.0);
            } else {
                self.orbit = Orbit::default();
            }
        }
        let view = ProjectionView {
            matrix: self.orbit.matrix(rect),
            rect,
        };
        if self.grid
            || !state
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
                    let snap = self.snap != ui.input(|i| i.modifiers.command);
                    let t = drag.transform(cursor, snap);
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
                    let modifiers = ui.input(|i| i.modifiers);
                    self.click_select(pick(runtime, &matrices, &view, pos), modifiers);
                }
            }
        }
        // Secondary selections: thinner, cooler outline.
        for id in &self.extra {
            if let Some(&i) = state.indices.get(id).filter(|&&i| visible(i)) {
                if let Some(mesh) = &state.entities[i].mesh {
                    for part in &mesh.parts {
                        draw_box(
                            &painter,
                            &view,
                            matrices[i] * part.transform.matrix(),
                            Color32::from_rgb(140, 200, 255),
                        );
                    }
                }
            }
        }
        if let (Some(i), Some(model)) = (selected, display) {
            if visible(i) {
                if let Some(mesh) = &state.entities[i].mesh {
                    for part in &mesh.parts {
                        draw_box(
                            &painter,
                            &view,
                            model * part.transform.matrix(),
                            Color32::from_rgb(252, 188, 111),
                        );
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
    use crate::editor::Dock;
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

    /// Starts a Move drag on the crate's X handle and returns the release point.
    fn start_drag(ctx: &egui::Context, editor: &mut Editor, runtime: &WorldRuntime) -> Pos2 {
        frame(ctx, editor, runtime, vec![]);
        let model = runtime.state.matrices()[runtime.state.compiled.indices["stage/crate"]];
        let projection = ProjectionView {
            matrix: editor.orbit.matrix(editor.viewport),
            rect: editor.viewport,
        };
        let handles = Handles::new(&projection, &editor.orbit, model, Tool::Move).unwrap();
        let start = handles.tips[0].unwrap();
        let end = start + (start - handles.pivot).normalized() * 27.0;
        frame(
            ctx,
            editor,
            runtime,
            vec![egui::Event::PointerMoved(start), primary(start, true)],
        );
        frame(ctx, editor, runtime, vec![egui::Event::PointerMoved(end)]);
        end
    }

    #[test]
    fn external_write_during_drag_cancels_the_drag_instead_of_stranding_a_stale_draft() {
        let (mut world, runtime) = fixture();
        let mut editor = Editor::new(world.clone(), 1, Some("stage/crate".into()));
        let ctx = egui::Context::default();
        let end = start_drag(&ctx, &mut editor, &runtime);
        assert!(editor.drag.is_some() && editor.dirty());
        world.entity_mut("stage/crate").unwrap().label = Some("Agent edit".into());
        editor.update(2, world, false);
        assert!(editor.drag.is_none());
        assert!(
            !editor.dirty(),
            "gizmo state must not survive as a stale draft"
        );
        assert_eq!(editor.observation()["draft_rev"], 2);
        assert!(editor.error && editor.status.contains("Drag cancelled"));
        // Releasing the mouse afterwards commits nothing.
        assert!(frame(&ctx, &mut editor, &runtime, vec![primary(end, false)]).is_empty());
    }

    #[test]
    fn modified_letter_keys_do_not_switch_tools_and_ctrl_y_redoes() {
        let (world, runtime) = fixture();
        let mut editor = Editor::new(world, 1, None);
        let ctx = egui::Context::default();
        // egui-winit reports modifiers both on the event and on RawInput; mirror that.
        let press = |editor: &mut Editor, key, modifiers| {
            let mut actions = Vec::new();
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 760.0))),
                    modifiers,
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                },
                |ctx| actions = editor.draw(ctx, &runtime),
            );
            actions
        };
        press(&mut editor, egui::Key::R, egui::Modifiers::CTRL);
        assert!(editor.tool == Tool::Move);
        press(&mut editor, egui::Key::R, egui::Modifiers::NONE);
        assert!(editor.tool == Tool::Scale);
        editor.redo.push(1);
        let actions = press(
            &mut editor,
            egui::Key::Y,
            egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
        );
        assert!(matches!(
            actions.as_slice(),
            [Action::Restore { redo: true, .. }]
        ));
    }

    #[test]
    fn same_frame_applies_merge_into_one_batch() {
        let apply = |rev, id: &str, select: Option<&str>| Action::Write {
            request: Request::Apply {
                expected_rev: Some(rev),
                edits: vec![Edit::Core(CoreEdit::DeleteEntity { id: id.into() })],
            },
            select: select.map(Into::into),
        };
        let merged = crate::editor::coalesce(vec![
            apply(3, "a", None),
            apply(3, "b", Some("b")),
            Action::Play,
            apply(3, "c", None),
        ]);
        assert_eq!(merged.len(), 3);
        let Action::Write {
            request: Request::Apply { edits, .. },
            select,
        } = &merged[0]
        else {
            panic!("first action must stay an apply");
        };
        assert_eq!(edits.len(), 2);
        assert_eq!(select.as_deref(), Some("b"));
        assert!(matches!(merged[1], Action::Play));
    }

    #[test]
    fn form_edits_write_only_changed_leaves_and_keep_prefab_inheritance() {
        let mut world = World::empty("forms");
        world.prefabs.insert(
            "grunt".into(),
            axiom_core::Prefab {
                components: [(
                    "core.health".into(),
                    serde_json::json!({"value":100,"max":100}),
                )]
                .into(),
            },
        );
        let mut e = axiom_core::Entity::new("grunt/1");
        e.prefab = Some("grunt".into());
        world.entities.push(e);
        let mut editor = Editor::new(world.clone(), 1, Some("grunt/1".into()));
        let entity = editor.world.entity("grunt/1").unwrap().clone();
        let draft = editor.draft.as_mut().unwrap();
        draft.open_component(&world, &entity, "core.health");
        assert!(!draft.json_mode, "objects open in form mode");
        assert_eq!(
            draft.form_base["max"], 100,
            "form shows the inherited value"
        );
        draft.form.as_mut().unwrap()["value"] = serde_json::json!(40);
        draft.form_dirty = true;
        let edits = draft.edits().unwrap();
        let [Edit::Core(CoreEdit::SetField { field, value, .. })] = edits.as_slice() else {
            panic!("expected exactly one leaf edit, got {edits:?}");
        };
        assert_eq!(field, "/components/core.health/value");
        assert_eq!(value, &serde_json::json!(40));
        let mut store = Store::memory_with(crate::installed_registry().authoring, world).unwrap();
        store
            .execute(Request::Apply {
                expected_rev: Some(1),
                edits,
            })
            .unwrap();
        let (_, after) = store.current().unwrap();
        assert_eq!(
            after.entity("grunt/1").unwrap().components["core.health"],
            serde_json::json!({"value":40}),
            "only the changed leaf becomes an override"
        );
        assert_eq!(
            after.prefabs["grunt"].components["core.health"]["value"],
            100
        );
    }

    #[test]
    fn multi_select_batch_delete_duplicate_and_clipboard() {
        let (world, runtime) = fixture();
        let mut editor = Editor::new(world, 1, Some("stage/crate".into()));
        let ctrl = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
        editor.click_select(Some("stage/rack".into()), ctrl);
        editor.click_select(Some("stage/beacon".into()), egui::Modifiers::SHIFT);
        assert_eq!(
            editor.selection(),
            ["stage/crate", "stage/beacon", "stage/rack"]
        );
        editor.click_select(Some("stage/rack".into()), ctrl);
        assert_eq!(editor.selection(), ["stage/crate", "stage/beacon"]);
        let Some(Action::Write {
            request: Request::Apply { edits, .. },
            ..
        }) = editor.delete_selection()
        else {
            panic!("delete must be one write");
        };
        assert_eq!(edits.len(), 2);
        let Some(Action::Write {
            request: Request::Apply { edits, .. },
            select,
        }) = editor.duplicate_ids(&editor.selection())
        else {
            panic!("duplicate must be one write");
        };
        let ids: Vec<_> = edits
            .iter()
            .map(|e| match e {
                Edit::Core(CoreEdit::DuplicateEntity { id, .. }) => id.clone(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(ids, ["stage/crate/copy", "stage/beacon/copy"]);
        assert_eq!(select.as_deref(), Some("stage/crate/copy"));
        // Ctrl+C / Ctrl+V arrive as Copy / Paste events from egui-winit.
        let ctx = egui::Context::default();
        frame(&ctx, &mut editor, &runtime, vec![egui::Event::Copy]);
        assert_eq!(editor.clipboard, ["stage/crate", "stage/beacon"]);
        let pasted = frame(
            &ctx,
            &mut editor,
            &runtime,
            vec![egui::Event::Paste("stage/rack\nnot-an-entity\n".into())],
        );
        assert!(matches!(
            pasted.as_slice(),
            [Action::Write { request: Request::Apply { edits, .. }, .. }]
                if matches!(&edits[0], Edit::Core(CoreEdit::DuplicateEntity { source, .. }) if source == "stage/rack")
        ));
        let key = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &mut editor, &runtime, vec![key]);
        assert!(editor.selection().is_empty());
    }

    #[test]
    fn snapping_quantizes_move_rotate_and_scale() {
        let drag = |tool| Drag {
            initial: Transform::default(),
            origin: Pos2::ZERO,
            screen_axis: vec2(1.0, 0.0),
            parent_inverse: Mat4::IDENTITY,
            axis: 0,
            world_units: 0.013,
            tool,
        };
        let cursor = pos2(31.0, 0.0);
        let moved = drag(Tool::Move).transform(cursor, true).position[0];
        assert!(
            (moved / 0.25 - (moved / 0.25).round()).abs() < 1e-5,
            "{moved}"
        );
        assert!((drag(Tool::Move).transform(cursor, false).position[0] - 0.403).abs() < 1e-4);
        let r = Quat::from_array(drag(Tool::Rotate).transform(cursor, true).rotation);
        let deg = r.to_axis_angle().1.to_degrees();
        assert!((deg / 15.0 - (deg / 15.0).round()).abs() < 1e-3, "{deg}");
        let s = drag(Tool::Scale).transform(cursor, true).scale[0];
        assert!((s * 10.0 - (s * 10.0).round()).abs() < 1e-4, "{s}");
    }

    #[test]
    fn proposals_panel_lists_pending_reviews_from_the_store() {
        let (world, runtime) = fixture();
        let mut store = Store::memory_with(crate::installed_registry().authoring, world).unwrap();
        store
            .execute(Request::Acl {
                text: "set stage/crate label Proposed".into(),
                preview: false,
                expected_rev: None,
                propose: true,
            })
            .unwrap();
        let (rev, current) = store.current().unwrap();
        let mut editor = Editor::new(current, rev, None);
        let rows = store
            .execute(Request::Proposals {
                status: Some("pending".into()),
                limit: Some(32),
            })
            .unwrap();
        editor.set_proposals(store.proposal_signature().unwrap(), &rows["proposals"]);
        assert_eq!(editor.observation()["pending_proposals"], 1);
        assert_eq!(editor.proposal_focus, Some(1));
        assert!(editor.status.contains("awaiting review"));
        editor.dock = Dock::Proposals;
        assert!(frame(&egui::Context::default(), &mut editor, &runtime, vec![]).is_empty());
    }

    #[test]
    fn remote_view_selects_and_refuses_to_clobber_drafts() {
        let (world, _) = fixture();
        let mut editor = Editor::new(world, 1, None);
        editor
            .remote_view(
                &["stage/crate".into(), "stage/beacon".into()],
                true,
                Some("history"),
            )
            .unwrap();
        assert_eq!(editor.selection(), ["stage/crate", "stage/beacon"]);
        assert!(editor.frame_selection);
        assert!(matches!(editor.dock, Dock::History));
        assert!(editor.remote_view(&["ghost".into()], false, None).is_err());
        assert!(editor.remote_view(&[], false, Some("bogus")).is_err());
        let text = editor.context_text();
        assert!(
            text.starts_with("# world editor-lab rev 1 · selection 2"),
            "{text}"
        );
        assert!(
            text.contains("stage/crate label=\"Signal Crate\" prefab=\"cargo-crate\""),
            "{text}"
        );
        assert!(text.contains("\n  render.mesh "), "{text}");
        editor.draft.as_mut().unwrap().label_dirty = true;
        let refused = editor
            .remote_view(&["stage/rack".into()], false, None)
            .unwrap_err();
        assert!(format!("{refused:#}").contains("unsaved draft"));
        assert_eq!(editor.selection()[0], "stage/crate");
    }

    #[test]
    fn diagnostic_paths_resolve_to_selectable_entities() {
        let (world, _) = fixture();
        use crate::editor::entity_in_path;
        assert_eq!(
            entity_in_path("/entities/@stage~1crate/label", &world).as_deref(),
            Some("stage/crate")
        );
        assert_eq!(
            entity_in_path("entities/stage/crate/components", &world).as_deref(),
            Some("stage/crate")
        );
        assert_eq!(entity_in_path("/resources/x", &world), None);
        assert_eq!(entity_in_path("/entities/@ghost/label", &world), None);
    }

    #[test]
    fn history_and_problem_panels_render_with_real_store_data() {
        let (world, runtime) = fixture();
        let mut store =
            Store::memory_with(crate::installed_registry().authoring, world.clone()).unwrap();
        let apply = Request::Apply {
            expected_rev: Some(1),
            edits: vec![Edit::Core(CoreEdit::SetField {
                target: axiom_core::local::Target::Entity {
                    id: "stage/crate".into(),
                },
                field: "/label".into(),
                value: serde_json::json!("Renamed"),
            })],
        };
        store.execute(apply.clone()).unwrap();
        let conflict = store.execute(apply).unwrap_err();
        let (rev, after) = store.current().unwrap();
        let mut editor = Editor::new(after, rev, None);
        editor
            .set_history(&store.execute(Request::History { limit: Some(8) }).unwrap()["revisions"]);
        assert_eq!(editor.history.len(), 2);
        assert!(!editor.needs_history);
        let diff = store
            .execute(Request::Diff {
                from_rev: 1,
                to_rev: 2,
                offset: 0,
                limit: Some(64),
            })
            .unwrap();
        editor.set_history_diff(2, diff);
        editor.problem(&conflict);
        assert_eq!(editor.problems[0]["code"], "revision_conflict");
        let ctx = egui::Context::default();
        for dock in [Dock::History, Dock::Problems] {
            editor.dock = dock;
            // Rendering must not panic and must not emit writes on its own.
            assert!(frame(&ctx, &mut editor, &runtime, vec![]).is_empty());
        }
    }

    #[test]
    fn focus_uses_mesh_part_bounds_not_the_pivot() {
        let part: axiom_core::MeshPart = serde_json::from_value(serde_json::json!({
            "transform": {"position": [10.0, 0.0, 0.0], "scale": [2.0, 2.0, 2.0]},
            "color": [1.0, 1.0, 1.0]
        }))
        .unwrap();
        let mesh = axiom_core::RenderMesh { parts: vec![part] };
        let (center, extent) = focus_bounds(Mat4::IDENTITY, Some(&mesh));
        assert!((center - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-4);
        assert!((extent - 2.0).abs() < 1e-4);
        let (pivot, unit) = focus_bounds(Mat4::IDENTITY, None);
        assert_eq!((pivot, unit), (Vec3::ZERO, 1.0));
    }
}
