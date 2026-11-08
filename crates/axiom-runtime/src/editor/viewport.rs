use super::{Action, Editor};
use crate::{presentation::RenderView, world::WorldRuntime};
use axiom_core::{Camera, Projection, Transform};
use egui::{pos2, vec2, Color32, PointerButton, Pos2, Rect, Stroke, Vec2};
use glam::{EulerRot, Mat4, Quat, Vec3, Vec4};
use serde::{Deserialize, Serialize};

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

/// Scene-view projection: free perspective orbit, or an axis-aligned orthographic view.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ViewMode {
    Perspective,
    Top,
    Bottom,
    Front,
    Back,
    Right,
    Left,
}
impl ViewMode {
    pub const ALL: [ViewMode; 7] = [
        Self::Perspective,
        Self::Top,
        Self::Bottom,
        Self::Front,
        Self::Back,
        Self::Right,
        Self::Left,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Perspective => "Perspective",
            Self::Top => "Top",
            Self::Bottom => "Bottom",
            Self::Front => "Front",
            Self::Back => "Back",
            Self::Right => "Right",
            Self::Left => "Left",
        }
    }
    pub fn command(self) -> &'static str {
        match self {
            Self::Perspective => "view.perspective",
            Self::Top => "view.top",
            Self::Bottom => "view.bottom",
            Self::Front => "view.front",
            Self::Back => "view.back",
            Self::Right => "view.right",
            Self::Left => "view.left",
        }
    }
    pub fn from_command(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.command() == id)
    }
    /// Orbit yaw/pitch looking along the view axis; `None` for the free perspective view.
    /// Front looks along -Z (eye on +Z), Right along -X (eye on +X), Top straight down with
    /// -Z (north) up on screen.
    fn angles(self) -> Option<(f32, f32)> {
        use std::f32::consts::{FRAC_PI_2, PI};
        match self {
            Self::Perspective => None,
            Self::Top => Some((0.0, FRAC_PI_2)),
            Self::Bottom => Some((0.0, -FRAC_PI_2)),
            Self::Front => Some((0.0, 0.0)),
            Self::Back => Some((PI, 0.0)),
            Self::Right => Some((FRAC_PI_2, 0.0)),
            Self::Left => Some((-FRAC_PI_2, 0.0)),
        }
    }
}

/// A saved scene-view camera (per world, in the editor prefs).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Bookmark {
    pub target: [f32; 3],
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub mode: ViewMode,
}
impl Bookmark {
    pub fn valid(&self) -> bool {
        self.target
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1.0e6)
            && self.distance.is_finite()
            && (0.25..=4000.0).contains(&self.distance)
            && self.yaw.is_finite()
            && self.pitch.is_finite()
            && self.pitch.abs() <= std::f32::consts::FRAC_PI_2 + 1e-4
    }
}

/// Orthographic views keep the eye at least this far back so objects between the target and
/// the eye are not clipped. Not further: the scene shader fogs by eye distance
/// (1 - exp(-d^2 * 0.00012), about 10% at 30 m, fully fogged by ~200 m).
const ORTHO_MIN_BACK: f32 = 30.0;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Orbit {
    pub(super) target: Vec3,
    pub(super) distance: f32,
    yaw: f32,
    pitch: f32,
    pub(super) mode: ViewMode,
}
impl Default for Orbit {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 0.8, 0.0),
            distance: 12.5,
            yaw: 0.70,
            pitch: 0.45,
            mode: ViewMode::Perspective,
        }
    }
}
impl Orbit {
    /// Unit vector from the target towards the eye.
    fn dir(&self) -> Vec3 {
        Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            self.pitch.cos() * self.yaw.cos(),
        )
    }
    /// Screen-up vector (d dir / d pitch): equals the Y-up `look_at` basis below the poles and
    /// stays defined straight above/below, where Top/Bottom views need it.
    fn up(&self) -> Vec3 {
        Vec3::new(
            -self.pitch.sin() * self.yaw.sin(),
            self.pitch.cos(),
            -self.pitch.sin() * self.yaw.cos(),
        )
    }
    /// (forward, right, up) of the view.
    pub(super) fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let forward = -self.dir();
        let up = self.up();
        (forward, forward.cross(up).normalize_or_zero(), up)
    }
    pub(super) fn ortho(&self) -> bool {
        self.mode != ViewMode::Perspective
    }
    fn eye(&self) -> Vec3 {
        let back = if self.ortho() {
            self.distance.max(ORTHO_MIN_BACK)
        } else {
            self.distance
        };
        self.target + back * self.dir()
    }
    /// Half the visible height at the target (orthographic views match the perspective
    /// framing at `distance`, so switching views keeps the scale).
    fn half_height(&self) -> f32 {
        self.distance * (FOV.to_radians() * 0.5).tan()
    }
    /// Distance-equivalent for sizing screen-constant handles at `point`.
    fn scale_at(&self, point: Vec3) -> f32 {
        if self.ortho() {
            self.distance
        } else {
            (point - self.eye()).length()
        }
    }
    fn projection(&self, aspect: f32) -> Mat4 {
        if self.ortho() {
            let h = self.half_height();
            Mat4::orthographic_rh(-h * aspect, h * aspect, -h, h, NEAR, FAR)
        } else {
            Mat4::perspective_rh(FOV.to_radians(), aspect, NEAR, FAR)
        }
    }
    /// View-projection used for picking, handles and overlays; identical to what the
    /// renderer builds from [`Orbit::view`] (projection · look_at(eye, eye+forward, up)).
    pub(super) fn matrix(&self, rect: Rect) -> Mat4 {
        let eye = self.eye();
        self.projection(rect.width() / rect.height().max(1.0))
            * Mat4::look_at_rh(eye, eye - self.dir(), self.up())
    }
    pub(super) fn set_mode(&mut self, mode: ViewMode) {
        self.mode = mode;
        match mode.angles() {
            Some((yaw, pitch)) => {
                self.yaw = yaw;
                self.pitch = pitch;
            }
            None => self.pitch = self.pitch.clamp(-1.52, 1.52),
        }
    }
    /// RMB drag: orbit around the target. Leaving an axis view returns to perspective.
    fn orbit_by(&mut self, delta: Vec2) {
        if self.ortho() {
            self.set_mode(ViewMode::Perspective);
        }
        self.yaw -= delta.x * 0.008;
        self.pitch = (self.pitch + delta.y * 0.008).clamp(-1.52, 1.52);
    }
    /// RMB drag while flying: turn around the eye (first-person look).
    fn look_by(&mut self, delta: Vec2) {
        if self.ortho() {
            self.set_mode(ViewMode::Perspective);
        }
        let eye = self.eye();
        self.yaw -= delta.x * 0.005;
        self.pitch = (self.pitch + delta.y * 0.005).clamp(-1.52, 1.52);
        self.target = eye - self.distance * self.dir();
    }
    /// Fly by `amount` metres along (right, up, forward) weights in `m`. Perspective moves the
    /// eye (up/down = world Y); orthographic views pan in the view plane and forward zooms.
    fn fly(&mut self, m: Vec3, amount: f32) {
        let (forward, right, up) = self.basis();
        if self.ortho() {
            self.target += (right * m.x + up * m.y) * amount;
            self.distance = (self.distance - m.z * amount).clamp(0.25, 4000.0);
        } else {
            self.target += (right * m.x + Vec3::Y * m.y + forward * m.z) * amount;
        }
        self.target = self.target.clamp(Vec3::splat(-1.0e6), Vec3::splat(1.0e6));
    }
    pub(super) fn bookmark(&self) -> Bookmark {
        Bookmark {
            target: self.target.to_array(),
            distance: self.distance,
            yaw: self.yaw,
            pitch: self.pitch,
            mode: self.mode,
        }
    }
    pub(super) fn from_bookmark(b: &Bookmark) -> Self {
        Self {
            target: Vec3::from_array(b.target),
            distance: b.distance,
            yaw: b.yaw,
            pitch: b.pitch,
            mode: b.mode,
        }
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
            forward: -self.dir(),
            up: self.up(),
            instances: vec![],
            camera: Camera {
                projection: if self.ortho() {
                    Projection::Orthographic {
                        height: 2.0 * self.half_height(),
                    }
                } else {
                    Projection::Perspective { fov_degrees: FOV }
                },
                near: NEAR,
                far: FAR,
                viewport,
                active: true,
            },
        }
    }
    fn pan(&mut self, delta: Vec2, height: f32) {
        let (_, right, up) = self.basis();
        self.target += (-right * delta.x + up * delta.y) * self.distance / height.max(1.0);
    }
}

/// Transient scene-view input state (never persisted).
#[derive(Default)]
pub(super) struct SceneInput {
    /// Box-selection start while the left button is held on empty space.
    pub marquee: Option<Pos2>,
    /// RMB is held and a movement key was used during this press (look around the eye).
    pub flying: bool,
    /// Input time until which the fly speed readout stays visible.
    pub speed_hint_until: f64,
    /// Component gizmo primitives drawn last frame (tests, diagnostics).
    pub gizmo_shapes: usize,
}

/// Movement weights (right, up, forward) from WASD/QE.
fn fly_keys(i: &egui::InputState) -> Vec3 {
    let axis = |pos: egui::Key, neg: egui::Key| {
        f32::from(u8::from(i.key_down(pos))) - f32::from(u8::from(i.key_down(neg)))
    };
    Vec3::new(
        axis(egui::Key::D, egui::Key::A),
        axis(egui::Key::E, egui::Key::Q),
        axis(egui::Key::W, egui::Key::S),
    )
}

/// RMB pressed inside `rect` and still held: the scene view owns WASD/QE and the wheel.
pub(super) fn rmb_held_in(i: &egui::InputState, rect: Rect) -> bool {
    i.pointer.secondary_down() && i.pointer.press_origin().is_some_and(|p| rect.contains(p))
}

pub(super) struct Drag {
    initial: Transform,
    origin: Pos2,
    screen_axis: Vec2,
    parent_inverse: Mat4,
    axis: usize,
    /// Handle direction in world space (object axis when `local`, world axis otherwise).
    world_axis: Vec3,
    local: bool,
    world_units: f32,
    tool: Tool,
}
impl Drag {
    /// Gizmo result; with `snap` = [move m, rotate radians, scale], values are quantized.
    fn transform(&self, cursor: Pos2, snap: Option<[f32; 3]>) -> Transform {
        let mut next = self.initial.clone();
        let pixels = (cursor - self.origin).dot(self.screen_axis);
        let [snap_move, snap_rotate, snap_scale] = snap.unwrap_or([0.0; 3]);
        let step = |x: f32, s: f32| {
            if snap.is_some() {
                (x / s).round() * s
            } else {
                x
            }
        };
        match self.tool {
            Tool::Move => {
                let offset = self.parent_inverse.transform_vector3(
                    self.world_axis * step(pixels * self.world_units, snap_move),
                );
                next.position = (Vec3::from_array(next.position) + offset)
                    .clamp(Vec3::splat(-100000.0), Vec3::splat(100000.0))
                    .to_array();
            }
            Tool::Rotate => {
                let angle = step(pixels * 0.012, snap_rotate);
                let current = Quat::from_array(next.rotation);
                next.rotation = if self.local {
                    current * Quat::from_axis_angle(AXES[self.axis], angle)
                } else {
                    // World-axis spin expressed in the parent's frame: P⁻¹·Q·P·R.
                    let (_, parent_inv, _) = self.parent_inverse.to_scale_rotation_translation();
                    parent_inv
                        * Quat::from_axis_angle(self.world_axis, angle)
                        * parent_inv.inverse()
                        * current
                }
                .normalize()
                .to_array();
            }
            Tool::Scale => {
                next.scale[self.axis] =
                    step(next.scale[self.axis] * (pixels * 0.014).exp(), snap_scale)
                        .clamp(if snap.is_some() { snap_scale } else { 0.0001 }, 10000.0)
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
pub(super) fn rotation(degrees: [f32; 3]) -> [f32; 4] {
    axiom_core::ops::rotation_from_degrees(degrees)
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

/// Nearest object under `p`. `skip` excludes editor-view hidden or locked objects.
fn pick(
    runtime: &WorldRuntime,
    matrices: &[Mat4],
    view: &ProjectionView,
    p: Pos2,
    skip: &dyn Fn(&str) -> bool,
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
        if !visible || skip(&entity.id) {
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
    // Imported/skinned meshes (glTF, procedural geometry) have no render.mesh parts; test the
    // ray against the world-space bounds of their current pose. Only runs on click.
    for mesh in runtime.render_frame().meshes {
        let Some(&slot) = state.indices.get(&mesh.id).filter(|_| !skip(&mesh.id)) else {
            continue;
        };
        if !runtime
            .authored_visibility
            .get(slot)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        if let Some(bounds) = mesh_bounds(&mesh) {
            if let Some(distance) =
                box_hit(origin, direction, bounds).filter(|&d| d >= 0.0 && d < depth)
            {
                best = Some(mesh.id.clone());
                depth = distance;
            }
        }
    }
    best
}

/// Unit-box matrix covering a mesh frame's world-space vertex bounds.
pub(super) fn mesh_bounds(mesh: &crate::presentation::MeshFrame) -> Option<Mat4> {
    let (min, max) = mesh.vertices.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(lo, hi), v| {
            let p = mesh.model.transform_point3(v.position);
            (lo.min(p), hi.max(p))
        },
    );
    (min.is_finite() && max.is_finite()).then(|| {
        Mat4::from_scale_rotation_translation(
            (max - min).max(Vec3::splat(0.02)),
            Quat::IDENTITY,
            (min + max) * 0.5,
        )
    })
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

/// Gizmo axis in world space. Scale always uses the object's axes; Move and Rotate follow
/// the Global/Local toggle.
fn handle_axis(model: Mat4, axis: Vec3, tool: Tool, local: bool) -> Vec3 {
    if tool == Tool::Scale || local {
        model.transform_vector3(axis).normalize_or_zero()
    } else {
        axis
    }
}

struct Handles {
    pivot: Pos2,
    tips: [Option<Pos2>; 3],
    units: f32,
}
impl Handles {
    fn new(
        view: &ProjectionView,
        orbit: &Orbit,
        model: Mat4,
        tool: Tool,
        local: bool,
    ) -> Option<Self> {
        let world = model.transform_point3(Vec3::ZERO);
        let pivot = view.project(world)?;
        let units = orbit.scale_at(world) * 92.0 / view.rect.height().max(1.0);
        let tips =
            AXES.map(|axis| view.project(world + handle_axis(model, axis, tool, local) * units));
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

pub(super) const FLY_MIN: f32 = 0.1;
pub(super) const FLY_MAX: f32 = 500.0;

/// One clickable label of the corner axis widget.
struct CubeLabel {
    key: &'static str,
    text: &'static str,
    at: Pos2,
    mode: ViewMode,
    color: Color32,
}
impl CubeLabel {
    fn hit_rect(&self) -> Rect {
        let size = if self.key == "mode" {
            vec2(120.0, 14.0)
        } else {
            vec2(18.0, 18.0)
        };
        Rect::from_center_size(self.at, size)
    }
}

fn view_cube(orbit: &Orbit, rect: Rect) -> Vec<CubeLabel> {
    let origin = rect.right_bottom() + vec2(-37.0, -32.0);
    let (_, right, up) = orbit.basis();
    let pairs = [
        (ViewMode::Right, ViewMode::Left),
        (ViewMode::Top, ViewMode::Bottom),
        (ViewMode::Front, ViewMode::Back),
    ];
    let mut out: Vec<CubeLabel> = AXES
        .iter()
        .enumerate()
        .map(|(i, axis)| {
            let (pos, neg) = pairs[i];
            CubeLabel {
                key: ["x", "y", "z"][i],
                text: ["X", "Y", "Z"][i],
                at: origin + vec2(axis.dot(right), -axis.dot(up)) * 19.0,
                mode: if orbit.mode == pos { neg } else { pos },
                color: COLORS[i],
            }
        })
        .collect();
    out.push(CubeLabel {
        key: "mode",
        text: "",
        at: origin + vec2(-24.0, -38.0),
        mode: ViewMode::Perspective,
        color: Color32::from_rgb(216, 225, 238),
    });
    out
}

/// Pickable objects whose projected center (mesh/skinned bounds, else pivot) lies in `area`,
/// in authored order.
fn marquee_hits(
    runtime: &WorldRuntime,
    matrices: &[Mat4],
    view: &ProjectionView,
    area: Rect,
    keep: &dyn Fn(usize, &str) -> bool,
) -> Vec<String> {
    let state = &runtime.state.compiled;
    let skinned: std::collections::HashMap<String, Vec3> = runtime
        .render_frame()
        .meshes
        .iter()
        .filter_map(|m| Some((m.id.clone(), mesh_bounds(m)?.transform_point3(Vec3::ZERO))))
        .collect();
    state
        .entities
        .iter()
        .enumerate()
        .take(runtime.authored_visibility.len())
        .filter(|(i, e)| keep(*i, &e.id))
        .filter_map(|(i, e)| {
            let center = if let Some(c) = skinned.get(&e.id) {
                *c
            } else if e.mesh.is_some() {
                focus_bounds(matrices[i], e.mesh.as_ref()).0
            } else if e.transform.is_some() {
                matrices[i].transform_point3(Vec3::ZERO)
            } else {
                return None;
            };
            view.project(center)
                .filter(|p| area.contains(*p))
                .map(|_| e.id.clone())
        })
        .collect()
}

/// Counts what it draws so tests can check the gizmo toggle.
struct Pen<'a> {
    painter: &'a egui::Painter,
    view: &'a ProjectionView,
    shapes: usize,
}
impl Pen<'_> {
    fn line(&mut self, a: Vec3, b: Vec3, color: Color32) {
        if let (Some(a), Some(b)) = (self.view.project(a), self.view.project(b)) {
            self.painter
                .line_segment([a, b], Stroke::new(1.3_f32, color));
            self.shapes += 1;
        }
    }
    fn loop_through(&mut self, points: &[Vec3], color: Color32) {
        for (k, p) in points.iter().enumerate() {
            self.line(*p, points[(k + 1) % points.len()], color);
        }
    }
    fn open(&mut self, points: &[Vec3], color: Color32) {
        for pair in points.windows(2) {
            self.line(pair[0], pair[1], color);
        }
    }
    fn label(&mut self, at: Vec3, text: &str, color: Color32) {
        if let Some(p) = self.view.project(at) {
            self.painter.text(
                p + vec2(7.0, -6.0),
                egui::Align2::LEFT_BOTTOM,
                text,
                egui::FontId::monospace(10.0),
                color,
            );
            self.shapes += 1;
        }
    }
}

/// `n` points of a circle of radius `r` around `center` in the plane spanned by `a`, `b`
/// (from angle `from` to `to`).
fn arc(center: Vec3, a: Vec3, b: Vec3, r: f32, from: f32, to: f32, n: usize) -> Vec<Vec3> {
    (0..n)
        .map(|k| {
            let t = from + (to - from) * k as f32 / (n - 1).max(1) as f32;
            center + (a * t.cos() + b * t.sin()) * r
        })
        .collect()
}

const GIZMO_CAMERA: Color32 = Color32::from_rgb(170, 186, 206);
const GIZMO_CAMERA_ACTIVE: Color32 = Color32::from_rgb(255, 214, 92);
const GIZMO_BLOCKER: Color32 = Color32::from_rgb(255, 146, 72);
const GIZMO_COLLIDER: Color32 = Color32::from_rgb(110, 232, 150);
const GIZMO_AGENT: Color32 = Color32::from_rgb(232, 140, 255);
const GIZMO_LIGHT: Color32 = Color32::from_rgb(255, 222, 120);
const GIZMO_LIGHT_OFF: Color32 = Color32::from_rgb(130, 120, 90);
const GIZMO_TRIGGER: Color32 = Color32::from_rgb(120, 230, 210);
const GIZMO_TRIGGER_OFF: Color32 = Color32::from_rgb(80, 120, 115);

/// `trigger.zone` wireframe: a box along the entity's world axes (scale ignored, like the
/// runtime) or three sphere circles; `exit_margin` adds a dim ring / rectangle at mid height.
fn draw_trigger_gizmo(
    pen: &mut Pen,
    zone: &axiom_core::trigger::TriggerZone,
    position: Vec3,
    [x, y, z]: [Vec3; 3],
    color: Color32,
) {
    use std::f32::consts::TAU;
    let margin = zone.exit_margin;
    if let Some(r) = zone.radius {
        for (a, b) in [(Vec3::X, Vec3::Z), (Vec3::X, Vec3::Y), (Vec3::Y, Vec3::Z)] {
            pen.loop_through(&arc(position, a, b, r, 0.0, TAU, 33)[..32], color);
        }
        if margin > 0.0 {
            pen.loop_through(
                &arc(position, Vec3::X, Vec3::Z, r + margin, 0.0, TAU, 33)[..32],
                color.gamma_multiply(0.45),
            );
        }
    }
    if let Some([hx, hy, hz]) = zone.half_extents {
        let corner = |j: usize| {
            let s = |bit: usize| if j & bit == 0 { -1.0 } else { 1.0 };
            position + x * (s(1) * hx) + y * (s(2) * hy) + z * (s(4) * hz)
        };
        for j in 0..8 {
            for bit in [1, 2, 4] {
                if j & bit == 0 {
                    pen.line(corner(j), corner(j | bit), color);
                }
            }
        }
        if margin > 0.0 {
            let (mx, mz) = (hx + margin, hz + margin);
            pen.loop_through(
                &[(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                    .map(|(sx, sz)| position + x * (sx * mx) + z * (sz * mz)),
                color.gamma_multiply(0.45),
            );
        }
    }
    pen.label(position, "TRIG", color);
}

/// `render.light` gizmo: directional = arrow along the light, point = range circles,
/// spot = outer/inner cone, area = rect/disk outline + normal. Returns nothing; counts via `pen`.
fn draw_light_gizmo(
    pen: &mut Pen,
    light: &axiom_core::RenderLight,
    position: Vec3,
    [x, y, z]: [Vec3; 3],
    color: Color32,
) {
    use axiom_core::{AreaShape, LightKind};
    use std::f32::consts::TAU;
    let forward = -z;
    let range = light.range.unwrap_or(1.0);
    match light.kind {
        LightKind::Directional => {
            pen.loop_through(&arc(position, x, y, 0.25, 0.0, TAU, 13)[..12], color);
            let tip = position + forward * 1.6;
            pen.line(position, tip, color);
            for side in [x, -x, y, -y] {
                pen.line(tip, tip - forward * 0.3 + side * 0.15, color);
            }
            pen.label(position, "SUN", color);
        }
        LightKind::Point => {
            for (a, b) in [(Vec3::X, Vec3::Y), (Vec3::X, Vec3::Z), (Vec3::Y, Vec3::Z)] {
                pen.loop_through(&arc(position, a, b, range, 0.0, TAU, 33)[..32], color);
            }
            if let Some(r) = light.source_radius.filter(|r| *r > 0.0) {
                pen.loop_through(&arc(position, x, y, r, 0.0, TAU, 13)[..12], color);
            }
            pen.label(position, "POINT", color);
        }
        LightKind::Spot => {
            let cone = light.spot.unwrap_or(axiom_core::SpotCone {
                inner_degrees: 0.0,
                outer_degrees: 30.0,
            });
            // Rim on the range sphere, so wide cones stay bounded.
            for (degrees, dim) in [(cone.outer_degrees, false), (cone.inner_degrees, true)] {
                let half = (degrees * 0.5).to_radians();
                let (radius, depth) = (range * half.sin(), range * half.cos());
                let rim = arc(position + forward * depth, x, y, radius, 0.0, TAU, 25);
                let c = if dim {
                    color.gamma_multiply(0.5)
                } else {
                    color
                };
                pen.loop_through(&rim[..24], c);
                if !dim {
                    for k in [0, 6, 12, 18] {
                        pen.line(position, rim[k], c);
                    }
                }
            }
            pen.line(
                position,
                position + forward * range,
                color.gamma_multiply(0.5),
            );
            pen.label(position, "SPOT", color);
        }
        LightKind::Area => {
            let area = light.area.unwrap_or(axiom_core::AreaLight {
                shape: AreaShape::Rect,
                size: [1.0, 1.0],
            });
            let (hw, hh) = (area.size[0] * 0.5, area.size[1] * 0.5);
            match area.shape {
                AreaShape::Rect => pen.loop_through(
                    &[(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                        .map(|(sx, sy)| position + x * (sx * hw) + y * (sy * hh)),
                    color,
                ),
                AreaShape::Disk => {
                    pen.loop_through(&arc(position, x, y, hw, 0.0, TAU, 25)[..24], color)
                }
            }
            let tip = position + forward * hw.max(hh).max(0.5);
            pen.line(position, tip, color);
            pen.label(position, "AREA", color);
        }
    }
}

/// Editor-only component gizmos: camera frusta (active camera highlighted), `render.light`
/// shapes (see `draw_light_gizmo`), `trigger.zone` wireframes (occupancy is drawn by the Play
/// preview overlay), `nav.blocker`
/// rectangles (XZ at the authored position + offset, like the runtime), `physics.collider`
/// boxes/capsules and `fsm.agent` body radii. Returns the number of shapes drawn.
fn draw_component_gizmos(
    painter: &egui::Painter,
    view: &ProjectionView,
    runtime: &WorldRuntime,
    matrices: &[Mat4],
    visible: &dyn Fn(usize) -> bool,
    selected: Option<usize>,
) -> usize {
    use std::f32::consts::{PI, TAU};
    let mut pen = Pen {
        painter,
        view,
        shapes: 0,
    };
    let state = &runtime.state.compiled;
    for (i, e) in state
        .entities
        .iter()
        .enumerate()
        .take(runtime.authored_visibility.len())
    {
        if !visible(i) || e.transform.is_none() {
            continue;
        }
        let (_, rotation, position) = matrices[i].to_scale_rotation_translation();
        let (x, y, z) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z);
        if let Some(camera) = &e.camera {
            let color = if camera.active {
                GIZMO_CAMERA_ACTIVE
            } else {
                GIZMO_CAMERA
            };
            let depth = 1.6_f32.min(camera.far).max(camera.near * 2.0);
            let aspect = 16.0 / 9.0 * camera.viewport[2] / camera.viewport[3].max(0.001);
            let forward = -z;
            let rectangle = |d: f32, half: f32| {
                let c = position + forward * d;
                [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)]
                    .map(|(sx, sy)| c + x * (sx * half * aspect) + y * (sy * half))
            };
            let (near, far, half) = match camera.projection {
                Projection::Perspective { fov_degrees } => {
                    let half = depth * (fov_degrees.to_radians() * 0.5).tan();
                    (None, rectangle(depth, half), half)
                }
                Projection::Orthographic { height } => {
                    let half = (height * 0.5).min(50.0);
                    (
                        Some(rectangle(camera.near, half)),
                        rectangle(depth, half),
                        half,
                    )
                }
            };
            pen.loop_through(&far, color);
            for (k, corner) in far.iter().enumerate() {
                let from = near.map_or(position, |n| n[k]);
                pen.line(from, *corner, color);
            }
            if let Some(near) = near {
                pen.loop_through(&near, color);
            }
            // "Up" notch on the top edge.
            let top = position + forward * depth + y * half;
            pen.open(
                &[
                    top - x * half * 0.3,
                    top + y * half * 0.35,
                    top + x * half * 0.3,
                ],
                color,
            );
            let name = if camera.active { "CAM (active)" } else { "CAM" };
            pen.label(position, name, color);
        }
        if let Some(light) = &e.light {
            let color = if selected == Some(i) {
                Color32::WHITE
            } else if light.enabled {
                GIZMO_LIGHT
            } else {
                GIZMO_LIGHT_OFF
            };
            draw_light_gizmo(&mut pen, light, position, [x, y, z], color);
        }
        if let Some(zone) = &e.trigger {
            let color = if selected == Some(i) {
                Color32::WHITE
            } else if zone.enabled {
                GIZMO_TRIGGER
            } else {
                GIZMO_TRIGGER_OFF
            };
            draw_trigger_gizmo(&mut pen, zone, position, [x, y, z], color);
        }
        if let Some(blocker) = e
            .components
            .get("nav.blocker")
            .and_then(|v| serde_json::from_value::<axiom_core::blocker::NavBlocker>(v.clone()).ok())
        {
            // The runtime uses the authored position (see axiom_core::blocker::blockers).
            let authored = e.transform.as_ref().map_or([0.0; 3], |t| t.position);
            let (lo, hi) = blocker.rect(authored);
            let h = position.y + 0.03;
            pen.loop_through(
                &[
                    Vec3::new(lo[0], h, lo[1]),
                    Vec3::new(hi[0], h, lo[1]),
                    Vec3::new(hi[0], h, hi[1]),
                    Vec3::new(lo[0], h, hi[1]),
                ],
                GIZMO_BLOCKER,
            );
        }
        if let Some(collider) = &e.collider {
            match collider.shape {
                axiom_core::ColliderShape::Box { half_extents } => {
                    let m = Mat4::from_rotation_translation(rotation, position)
                        * Mat4::from_scale(Vec3::from_array(half_extents) * 2.0);
                    let corner = |j: usize| {
                        m.transform_point3(Vec3::new(
                            (j & 1) as f32 - 0.5,
                            ((j >> 1) & 1) as f32 - 0.5,
                            ((j >> 2) & 1) as f32 - 0.5,
                        ))
                    };
                    for j in 0..8 {
                        for bit in [1, 2, 4] {
                            if j & bit == 0 {
                                pen.line(corner(j), corner(j | bit), GIZMO_COLLIDER);
                            }
                        }
                    }
                }
                axiom_core::ColliderShape::Capsule {
                    half_height,
                    radius,
                } => {
                    let top = position + y * half_height;
                    let bottom = position - y * half_height;
                    for c in [top, bottom] {
                        pen.loop_through(&arc(c, x, z, radius, 0.0, TAU, 17)[..16], GIZMO_COLLIDER);
                    }
                    for side in [x, -x, z, -z] {
                        pen.line(top + side * radius, bottom + side * radius, GIZMO_COLLIDER);
                    }
                    for side in [x, z] {
                        pen.open(&arc(top, side, y, radius, 0.0, PI, 9), GIZMO_COLLIDER);
                        pen.open(&arc(bottom, side, -y, radius, 0.0, PI, 9), GIZMO_COLLIDER);
                    }
                }
            }
        }
        if let Some(agent) = e.components.get("fsm.agent") {
            let radius = agent["radius"].as_f64().map_or(0.3, |r| r as f32);
            let color = if selected == Some(i) {
                Color32::WHITE
            } else {
                GIZMO_AGENT
            };
            pen.loop_through(
                &arc(
                    position + Vec3::Y * 0.02,
                    Vec3::X,
                    Vec3::Z,
                    radius,
                    0.0,
                    TAU,
                    21,
                )[..20],
                color,
            );
            pen.label(position + Vec3::Y * 0.02, "FSM", color);
        }
    }
    pen.shapes
}

impl Editor {
    /// Box selection result: Shift adds, Ctrl toggles (the primary selection stays), a plain
    /// drag replaces the selection (an empty box clears it).
    pub(super) fn box_select(&mut self, ids: Vec<String>, modifiers: egui::Modifiers) {
        if self.dirty() {
            self.set_error("Apply or reload the inspector draft before changing the selection.");
            return;
        }
        let count = ids.len();
        if modifiers.command {
            for id in ids {
                if self.selected.as_ref() == Some(&id) {
                    continue;
                }
                if !self.extra.remove(&id) {
                    if self.selected.is_none() {
                        self.select(Some(id));
                    } else {
                        self.extra.insert(id);
                    }
                }
            }
        } else if modifiers.shift {
            for id in ids {
                if self.selected.is_none() {
                    self.select(Some(id));
                } else if self.selected.as_ref() != Some(&id) {
                    self.extra.insert(id);
                }
            }
        } else {
            self.extra.clear();
            let mut ids = ids.into_iter();
            self.select(ids.next());
            self.extra.extend(ids);
        }
        self.notice(format!("Box selected {count} object(s)"));
    }

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
        // Editor-view flags: hidden objects are neither drawn nor picked; locked ones are drawn
        // but cannot be picked or dragged here (the hierarchy still selects them).
        let hidden = self.view_flagged(false);
        let locked = self.view_flagged(true);
        let hidden_slots: std::collections::HashSet<usize> = hidden
            .iter()
            .filter_map(|id| state.indices.get(id).copied())
            .collect();
        let visible = |i| {
            runtime.authored_visibility.get(i).copied().unwrap_or(false)
                && !hidden_slots.contains(&i)
        };
        let skip = |id: &str| hidden.contains(id) || locked.contains(id);
        let selected_locked = self.selected.as_ref().is_some_and(|id| locked.contains(id));
        let selected = self
            .selected
            .as_ref()
            .and_then(|id| state.indices.get(id))
            .copied();
        let delta = ui.input(|i| i.pointer.delta());
        // RMB held: plain drag orbits; with WASD/QE it flies (and the mouse looks around the
        // eye for the rest of that press). The wheel then sets the fly speed instead of zoom.
        let rmb = ui.input(|i| rmb_held_in(i, rect));
        if !rmb {
            self.scene_input.flying = false;
        }
        if self.drag.is_none() && self.scene_input.marquee.is_none() {
            if rmb {
                let (keys, shift, dt, wheel, now) = ui.input(|i| {
                    (
                        fly_keys(i),
                        i.modifiers.shift,
                        i.stable_dt.clamp(0.0, 0.1),
                        i.raw_scroll_delta.y,
                        i.time,
                    )
                });
                if keys != Vec3::ZERO {
                    self.scene_input.flying = true;
                }
                if wheel != 0.0 {
                    self.prefs.fly_speed =
                        (self.prefs.fly_speed * (wheel * 0.004).exp()).clamp(FLY_MIN, FLY_MAX);
                    self.scene_input.speed_hint_until = now + 1.5;
                }
                if self.scene_input.flying {
                    self.orbit.look_by(delta);
                    let speed = self.prefs.fly_speed * if shift { 3.0 } else { 1.0 };
                    self.orbit.fly(keys, speed * dt);
                    ui.ctx().request_repaint();
                } else if response.dragged_by(PointerButton::Secondary) {
                    self.orbit.orbit_by(delta);
                }
            }
            if response.dragged_by(PointerButton::Middle) {
                self.orbit.pan(delta, rect.height());
            }
            if response.hovered() && !rmb {
                let wheel = ui.input(|i| i.smooth_scroll_delta.y);
                self.orbit.distance =
                    (self.orbit.distance * (-wheel * 0.003).exp()).clamp(0.25, 4000.0);
            }
        }
        if std::mem::take(&mut self.frame_selection) {
            if let Some(i) = selected {
                let skinned = state.entities[i]
                    .mesh
                    .is_none()
                    .then(|| {
                        runtime
                            .render_frame()
                            .meshes
                            .iter()
                            .find(|m| m.id == state.entities[i].id)
                            .and_then(mesh_bounds)
                    })
                    .flatten();
                let (center, extent) = match skinned {
                    Some(b) => focus_bounds(b, None),
                    None => focus_bounds(matrices[i], state.entities[i].mesh.as_ref()),
                };
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
        let cube = view_cube(&self.orbit, rect);
        for label in &cube {
            self.rects
                .insert(format!("view.cube.{}", label.key), label.hit_rect());
        }
        let cube_hit = |p: Pos2| {
            cube.iter()
                .find(|l| l.hit_rect().contains(p))
                .map(|l| l.mode)
        };
        if self.prefs.grid
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
                    && !selected_locked
                    && self.draft.as_ref().is_some_and(|d| d.transform.is_some())
            })
            .and_then(|m| Handles::new(&view, &self.orbit, m, self.tool, self.prefs.local_space));
        let hovered_axis = response
            .hover_pos()
            .and_then(|pos| handles.as_ref()?.hit(pos));
        if response.drag_started_by(PointerButton::Primary) {
            let origin = ui.input(|i| i.pointer.press_origin());
            let hit = origin.and_then(|o| handles.as_ref()?.hit(o));
            if hit.is_none() && ready && origin.is_some_and(|o| cube_hit(o).is_none()) {
                // Left-drag on anything but a gizmo handle draws a selection box.
                self.scene_input.marquee = origin;
            }
            if let (Some(origin), Some(handles), Some(initial), false) = (
                origin,
                &handles,
                self.draft.as_ref().and_then(|d| d.transform.clone()),
                self.dirty(),
            ) {
                if let Some(axis) = hit {
                    let vector = handles.tips[axis].unwrap() - handles.pivot;
                    let model = parent * initial.matrix();
                    self.drag = Some(Drag {
                        world_axis: handle_axis(
                            model,
                            AXES[axis],
                            self.tool,
                            self.prefs.local_space,
                        ),
                        local: self.prefs.local_space,
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
        let snap_steps = self.snap_steps();
        if let Some(drag) = &self.drag {
            let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused);
            if cancel {
                self.select(self.selected.clone());
            } else if let Some(draft) = &mut self.draft {
                if let Some(cursor) = ui.input(|i| i.pointer.latest_pos()) {
                    let snap = self.prefs.snap != ui.input(|i| i.modifiers.command);
                    let t = drag.transform(cursor, snap.then_some(snap_steps));
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
        } else if response.clicked_by(PointerButton::Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(mode) = cube_hit(pos) {
                    self.orbit.set_mode(mode);
                    self.notice(format!("{} view", mode.name()));
                } else if ready && hovered_axis.is_none() {
                    let modifiers = ui.input(|i| i.modifiers);
                    self.click_select(pick(runtime, &matrices, &view, pos, &skip), modifiers);
                }
            }
        }
        if let Some(start) = self.scene_input.marquee {
            let (latest, down, escape, modifiers) = ui.input(|i| {
                (
                    i.pointer.latest_pos(),
                    i.pointer.primary_down(),
                    i.key_pressed(egui::Key::Escape),
                    i.modifiers,
                )
            });
            let area = Rect::from_two_pos(start, latest.unwrap_or(start));
            if escape {
                self.scene_input.marquee = None;
            } else if down {
                painter.rect_filled(area, 0.0, Color32::from_rgba_unmultiplied(90, 150, 255, 26));
                painter.rect_stroke(
                    area,
                    0.0,
                    Stroke::new(1.0_f32, Color32::from_rgb(140, 190, 255)),
                    egui::StrokeKind::Inside,
                );
            } else {
                self.scene_input.marquee = None;
                let keep = |i: usize, id: &str| visible(i) && !skip(id);
                let ids = marquee_hits(runtime, &matrices, &view, area, &keep);
                self.box_select(ids, modifiers);
            }
        }
        self.scene_input.gizmo_shapes = if self.prefs.gizmos {
            draw_component_gizmos(&painter, &view, runtime, &matrices, &visible, selected)
        } else {
            0
        };
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
            "RMB orbit  RMB+WASD/QE fly  MMB pan  Wheel zoom  F frame  Drag box-select",
            egui::FontId::monospace(11.0),
            Color32::from_rgb(216, 225, 238),
        );
        // Axis view cube: click X/Y/Z for Right/Top/Front (again for the opposite side), the
        // mode caption for Perspective.
        let axes = rect.right_bottom() + vec2(-37.0, -32.0);
        for label in &cube {
            if label.key == "mode" {
                painter.text(
                    label.at,
                    egui::Align2::CENTER_CENTER,
                    format!(
                        "{}{}",
                        self.orbit.mode.name(),
                        if self.orbit.ortho() { " (ortho)" } else { "" }
                    ),
                    egui::FontId::monospace(10.0),
                    label.color,
                );
                continue;
            }
            painter.line_segment([axes, label.at], Stroke::new(2.0_f32, label.color));
            painter.text(
                label.at,
                egui::Align2::CENTER_BOTTOM,
                label.text,
                egui::FontId::monospace(10.0),
                label.color,
            );
        }
        let now = ui.input(|i| i.time);
        if now < self.scene_input.speed_hint_until {
            painter.text(
                rect.center_bottom() + vec2(0.0, -40.0),
                egui::Align2::CENTER_BOTTOM,
                format!("Fly speed {:.1} m/s", self.prefs.fly_speed),
                egui::FontId::proportional(14.0),
                Color32::from_rgb(255, 222, 140),
            );
            ui.ctx().request_repaint();
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
            pick(
                &runtime,
                &runtime.state.matrices(),
                &projection,
                point,
                &|_| false
            )
            .as_deref(),
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
        let handles = Handles::new(&projection, &editor.orbit, model, Tool::Move, false).unwrap();
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
        let handles = Handles::new(&projection, &editor.orbit, model, Tool::Move, false).unwrap();
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
        // Copies are moved right after their sources (MoveEntity edits follow).
        let ids: Vec<_> = edits
            .iter()
            .filter_map(|e| match e {
                Edit::Core(CoreEdit::DuplicateEntity { id, .. }) => Some(id.clone()),
                Edit::Core(CoreEdit::MoveEntity { .. }) => None,
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
    fn skinned_gltf_characters_are_pickable_by_their_pose_bounds() {
        let world = pack("mascot");
        let runtime = WorldRuntime::new(world, 1, crate::installed_registry()).unwrap();
        let frame = runtime.render_frame();
        assert!(
            !frame.meshes.is_empty(),
            "mascot pack renders skinned meshes"
        );
        let target = &frame.meshes[0];
        let bounds = mesh_bounds(target).unwrap();
        let center = bounds.transform_point3(Vec3::ZERO);
        let size = bounds.transform_vector3(Vec3::ONE);
        assert!(
            size.y > 0.5,
            "a character is taller than half a meter: {size:?}"
        );
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0));
        let eye = center + Vec3::new(0.0, 0.3, 4.0);
        let view = ProjectionView {
            matrix: Mat4::perspective_rh(FOV.to_radians(), 1280.0 / 720.0, NEAR, FAR)
                * Mat4::look_at_rh(eye, center, Vec3::Y),
            rect,
        };
        let at = view.project(center).unwrap();
        let picked = pick(&runtime, &runtime.state.matrices(), &view, at, &|_| false);
        assert_eq!(picked.as_deref(), Some(target.id.as_str()));
        // A click in the screen corner, far outside the character's bounds, must not pick it.
        assert_ne!(
            pick(
                &runtime,
                &runtime.state.matrices(),
                &view,
                pos2(5.0, 5.0),
                &|_| false
            )
            .as_deref(),
            Some(target.id.as_str())
        );
    }

    #[test]
    fn global_and_local_gizmo_axes_move_and_rotate_correctly() {
        let yaw90 = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let initial = Transform {
            rotation: yaw90.to_array(),
            ..Default::default()
        };
        let model = initial.matrix();
        let drag = |tool, local, parent: Mat4| Drag {
            world_axis: handle_axis(parent * model, AXES[0], tool, local),
            local,
            initial: initial.clone(),
            origin: Pos2::ZERO,
            screen_axis: vec2(1.0, 0.0),
            parent_inverse: parent.inverse(),
            axis: 0,
            world_units: 0.01,
            tool,
        };
        let cursor = pos2(100.0, 0.0);
        // Global X moves along world X; local X of a yawed object points along world -Z.
        let p = drag(Tool::Move, false, Mat4::IDENTITY)
            .transform(cursor, None)
            .position;
        assert!((Vec3::from_array(p) - Vec3::X).length() < 1e-4, "{p:?}");
        let p = drag(Tool::Move, true, Mat4::IDENTITY)
            .transform(cursor, None)
            .position;
        assert!((Vec3::from_array(p) - Vec3::NEG_Z).length() < 1e-4, "{p:?}");
        // World-axis rotation under a rotated parent: world result = Q(X, a)·P·R.
        let parent = Mat4::from_quat(Quat::from_rotation_z(0.7));
        let angle = 100.0 * 0.012;
        let r = drag(Tool::Rotate, false, parent)
            .transform(cursor, None)
            .rotation;
        let world = Quat::from_rotation_z(0.7) * Quat::from_array(r);
        let expected = Quat::from_axis_angle(Vec3::X, angle) * Quat::from_rotation_z(0.7) * yaw90;
        assert!(
            world.dot(expected).abs() > 0.9999,
            "{world:?} vs {expected:?}"
        );
        // Local rotation spins about the object's own X.
        let r = drag(Tool::Rotate, true, Mat4::IDENTITY)
            .transform(cursor, None)
            .rotation;
        let expected = yaw90 * Quat::from_axis_angle(Vec3::X, angle);
        assert!(Quat::from_array(r).dot(expected).abs() > 0.9999);
    }

    #[test]
    fn snapping_quantizes_move_rotate_and_scale() {
        let drag = |tool| Drag {
            world_axis: Vec3::X,
            local: false,
            initial: Transform::default(),
            origin: Pos2::ZERO,
            screen_axis: vec2(1.0, 0.0),
            parent_inverse: Mat4::IDENTITY,
            axis: 0,
            world_units: 0.013,
            tool,
        };
        let cursor = pos2(31.0, 0.0);
        let steps = Some([0.25, 15f32.to_radians(), 0.1]);
        let moved = drag(Tool::Move).transform(cursor, steps).position[0];
        assert!(
            (moved / 0.25 - (moved / 0.25).round()).abs() < 1e-5,
            "{moved}"
        );
        assert!((drag(Tool::Move).transform(cursor, None).position[0] - 0.403).abs() < 1e-4);
        let r = Quat::from_array(drag(Tool::Rotate).transform(cursor, steps).rotation);
        let deg = r.to_axis_angle().1.to_degrees();
        assert!((deg / 15.0 - (deg / 15.0).round()).abs() < 1e-3, "{deg}");
        let s = drag(Tool::Scale).transform(cursor, steps).scale[0];
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
    fn scenarios_tab_shows_real_pass_and_fail_results() {
        let (world, runtime) = fixture();
        let mut editor = Editor::new(world.clone(), 1, None);
        editor.set_scenarios(vec!["a.scenario.json".into(), "b.scenario.json".into()]);
        let pass: crate::scenario::Scenario = serde_json::from_value(serde_json::json!({
            "ticks": 10, "assertions": [{"kind":"finite"}]
        }))
        .unwrap();
        let ok = crate::scenario::run(world.clone(), 1, crate::installed_registry(), pass).unwrap();
        editor.set_scenario_result("a.scenario.json", ok);
        assert!(!editor.error && editor.status.contains("passed"));
        let fail: crate::scenario::Scenario = serde_json::from_value(serde_json::json!({
            "ticks": 10, "assertions": [{"kind":"field","entity":"stage/crate","field":"health","op":"gt","value":1}]
        }))
        .unwrap();
        let err = crate::scenario::run(world, 1, crate::installed_registry(), fail).unwrap_err();
        editor.set_scenario_result("b.scenario.json", axiom_core::error_response(&err));
        assert!(editor.error && editor.status.contains("failed"));
        assert!(matches!(editor.dock, Dock::Scenarios));
        assert!(frame(&egui::Context::default(), &mut editor, &runtime, vec![]).is_empty());
    }

    fn pack(name: &str) -> World {
        axiom_core::ContentPackage::load(
            format!("{}/../../content/{name}", env!("CARGO_MANIFEST_DIR")),
            &crate::installed_registry().authoring,
        )
        .unwrap()
        .world
    }

    #[test]
    fn material_and_prefab_drafts_write_through_the_transaction_protocol() {
        let world = pack("shader-lab");
        let material = world.resources["render.shaders"]["materials"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        let mut editor = Editor::new(world.clone(), 1, None);
        editor.open_material(&material);
        let draft = editor.material.as_mut().unwrap();
        assert!(!draft.dirty(), "opening a material must not create edits");
        draft.value["tint"] = serde_json::json!([0.5, 0.25, 1.0, 1.0]);
        assert!(editor.dirty());
        let edits = editor.material.as_ref().unwrap().edits();
        assert_eq!(edits.len(), 1);
        let mut store = Store::memory_with(crate::installed_registry().authoring, world).unwrap();
        store
            .execute(Request::Apply {
                expected_rev: Some(1),
                edits,
            })
            .unwrap();
        let (_, after) = store.current().unwrap();
        assert_eq!(
            after.resources["render.shaders"]["materials"][&material]["tint"],
            serde_json::json!([0.5, 0.25, 1.0, 1.0])
        );

        let lab = pack("behavior-lab");
        let mut editor = Editor::new(lab.clone(), 1, None);
        editor.open_prefab("enemy.basic");
        let draft = editor.prefab.as_mut().unwrap();
        draft.component = "core.health".into();
        draft.base = lab.prefabs["enemy.basic"].components["core.health"].clone();
        draft.value = Some(draft.base.clone());
        draft.value.as_mut().unwrap()["max"] = serde_json::json!(150);
        let edits = editor.prefab.as_ref().unwrap().edits();
        let mut store = Store::memory_with(crate::installed_registry().authoring, lab).unwrap();
        store
            .execute(Request::Apply {
                expected_rev: Some(1),
                edits,
            })
            .unwrap();
        let (_, after) = store.current().unwrap();
        assert_eq!(
            after.prefabs["enemy.basic"].components["core.health"]["max"],
            150
        );
    }

    #[test]
    fn hierarchy_search_supports_tag_component_and_prefab_filters() {
        let world = pack("behavior-lab");
        let hits = |term: &str| {
            let mut ids: Vec<&str> = world
                .entities
                .iter()
                .filter(|e| super::super::ui::search_matches(&world, e, &term.to_lowercase()))
                .map(|e| e.id.as_str())
                .collect();
            ids.sort();
            ids
        };
        assert_eq!(hits("tag:player"), ["player"]);
        assert_eq!(
            hits("has:fsm.agent").len(),
            6,
            "prefab-inherited components count"
        );
        assert_eq!(hits("prefab:enemy.basic enemy/3"), ["enemy/3"]);
        assert_eq!(hits("has:core.motion tag:player"), ["player"]);
        assert!(hits("tag:enemy has:core.motion").is_empty());
        assert_eq!(hits("FLOOR"), ["floor"]);
    }

    #[test]
    fn graphs_tab_renders_fsm_and_narrative_content() {
        let (_, runtime) = fixture();
        for name in ["behavior-lab", "adv"] {
            let mut editor = Editor::new(pack(name), 1, None);
            editor.dock = Dock::Graphs;
            assert!(frame(&egui::Context::default(), &mut editor, &runtime, vec![]).is_empty());
        }
        assert!(super::super::form::is_color(
            "color",
            &serde_json::json!([0.2, 0.4, 0.6])
        ));
        assert!(!super::super::form::is_color(
            "position",
            &serde_json::json!([0.2, 0.4, 0.6])
        ));
        assert!(!super::super::form::is_color(
            "color",
            &serde_json::json!([2.0, 0.4, 0.6])
        ));
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

    /// Screen point of the crate's pivot in the editor's current view.
    fn crate_point(editor: &Editor, runtime: &WorldRuntime) -> Pos2 {
        let model = runtime.state.matrices()[runtime.state.compiled.indices["stage/crate"]];
        ProjectionView {
            matrix: editor.orbit.matrix(editor.viewport),
            rect: editor.viewport,
        }
        .project(model.transform_point3(Vec3::ZERO))
        .unwrap()
    }

    fn click_at(ctx: &egui::Context, editor: &mut Editor, runtime: &WorldRuntime, at: Pos2) {
        frame(
            ctx,
            editor,
            runtime,
            vec![egui::Event::PointerMoved(at), primary(at, true)],
        );
        frame(ctx, editor, runtime, vec![primary(at, false)]);
    }

    #[test]
    fn hidden_objects_are_not_drawn_or_pickable_in_the_editor_view_only() {
        let (world, mut runtime) = fixture();
        let ctx = egui::Context::default();
        // Baseline: a click on the crate picks it.
        let mut editor = Editor::new(world.clone(), 1, None);
        frame(&ctx, &mut editor, &runtime, vec![]);
        let at = crate_point(&editor, &runtime);
        click_at(&ctx, &mut editor, &runtime, at);
        assert_eq!(editor.selected.as_deref(), Some("stage/crate"));
        let drawn = editor
            .render_frame(&mut runtime, ctx.screen_rect())
            .instances
            .len();
        // Hide it: the same click no longer picks it, and the editor view draws fewer instances.
        editor.select(None);
        editor.set_view_flag(&["stage/crate".into()], false);
        click_at(&ctx, &mut editor, &runtime, at);
        assert_ne!(editor.selected.as_deref(), Some("stage/crate"));
        let hidden = editor
            .render_frame(&mut runtime, ctx.screen_rect())
            .instances
            .len();
        assert!(hidden < drawn, "{hidden} < {drawn}");
        // Play renders from the runtime directly and still shows it; content is untouched.
        assert!(runtime.render_frame().instances.len() >= drawn);
        assert_eq!(*editor.world, world);
        // Hiding a parent hides its children too.
        editor.set_view_flag(&["stage/rack".into()], false);
        assert!(editor.view_flagged(false).contains("stage/rack/cargo"));
    }

    fn secondary(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn key(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// Like `frame`, with modifiers held on the raw input; returns the painted shape count.
    fn frame_with(
        ctx: &egui::Context,
        editor: &mut Editor,
        runtime: &WorldRuntime,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> usize {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 760.0))),
                modifiers,
                events,
                ..Default::default()
            },
            |ctx| {
                editor.draw(ctx, runtime);
            },
        )
        .shapes
        .len()
    }

    fn screen_of(editor: &Editor, point: Vec3) -> Pos2 {
        ProjectionView {
            matrix: editor.orbit.matrix(editor.viewport),
            rect: editor.viewport,
        }
        .project(point)
        .unwrap()
    }

    fn pivot(runtime: &WorldRuntime, id: &str) -> Vec3 {
        runtime.state.matrices()[runtime.state.compiled.indices[id]].transform_point3(Vec3::ZERO)
    }

    #[test]
    fn rmb_plus_wasd_flies_and_the_wheel_sets_the_speed() {
        let (world, runtime) = fixture();
        let mut editor = Editor::new(world, 1, None);
        editor.tool = Tool::Select;
        let ctx = egui::Context::default();
        frame(&ctx, &mut editor, &runtime, vec![]);
        let at = editor.viewport.center();
        let (forward, _, _) = editor.orbit.basis();
        let eye = editor.orbit.eye();
        frame(
            &ctx,
            &mut editor,
            &runtime,
            vec![egui::Event::PointerMoved(at), secondary(at, true)],
        );
        frame(&ctx, &mut editor, &runtime, vec![key(egui::Key::W, true)]);
        for _ in 0..11 {
            frame(&ctx, &mut editor, &runtime, vec![]);
        }
        let moved = editor.orbit.eye() - eye;
        // 6 m/s for 12 frames of 1/60 s = 1.2 m forward.
        assert!(
            (moved.dot(forward) - 1.2).abs() < 0.05,
            "{moved:?} along {forward:?}"
        );
        assert!(moved.reject_from(forward).length() < 1e-3);
        assert!(
            editor.tool == Tool::Select,
            "W flies instead of picking Move"
        );
        // Wheel while RMB is held changes the fly speed (and shows it), not the zoom.
        let distance = editor.orbit.distance;
        frame(
            &ctx,
            &mut editor,
            &runtime,
            vec![
                key(egui::Key::W, false),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, 100.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert!(editor.prefs.fly_speed > 8.0, "{}", editor.prefs.fly_speed);
        assert_eq!(editor.orbit.distance, distance);
        assert!(editor.scene_input.speed_hint_until > 0.0);
        frame(&ctx, &mut editor, &runtime, vec![secondary(at, false)]);
        // Released: W is the Move tool shortcut again.
        frame(&ctx, &mut editor, &runtime, vec![key(egui::Key::W, true)]);
        assert!(editor.tool == Tool::Move);
        frame(&ctx, &mut editor, &runtime, vec![key(egui::Key::W, false)]);
        // A plain RMB drag still orbits around the target.
        let target = editor.orbit.target;
        let yaw = editor.orbit.yaw;
        frame(
            &ctx,
            &mut editor,
            &runtime,
            vec![egui::Event::PointerMoved(at), secondary(at, true)],
        );
        for step in 1..=4 {
            frame(
                &ctx,
                &mut editor,
                &runtime,
                vec![egui::Event::PointerMoved(
                    at + vec2(15.0 * step as f32, 0.0),
                )],
            );
        }
        frame(&ctx, &mut editor, &runtime, vec![secondary(at, false)]);
        assert_eq!(editor.orbit.target, target);
        assert!(
            editor.orbit.yaw < yaw - 0.2,
            "{} vs {yaw}",
            editor.orbit.yaw
        );
    }

    #[test]
    fn orthographic_views_pick_and_drag_like_perspective() {
        let (world, mut runtime) = fixture();
        let mut editor = Editor::new(world, 1, None);
        let ctx = egui::Context::default();
        editor.run_command("view.top").unwrap();
        frame(&ctx, &mut editor, &runtime, vec![]);
        let view = editor.render_frame(&mut runtime, ctx.screen_rect()).views[0].clone();
        assert!(matches!(
            view.camera.projection,
            Projection::Orthographic { .. }
        ));
        // The scene shader fogs by eye distance: an ortho eye 1 km away rendered only fog.
        assert!((view.eye - editor.orbit.target).length() <= 30.0 + 1e-3);
        // Top view: +X to the right, -Z up on screen.
        let crate_at = screen_of(&editor, pivot(&runtime, "stage/crate"));
        let beacon_at = screen_of(&editor, pivot(&runtime, "stage/beacon"));
        assert!(beacon_at.x > crate_at.x && beacon_at.y < crate_at.y);
        click_at(&ctx, &mut editor, &runtime, crate_at);
        assert_eq!(editor.selected.as_deref(), Some("stage/crate"));
        // Front view hits the same object from the side; the X handle drag still writes X.
        editor.run_command("view.front").unwrap();
        let end = start_drag(&ctx, &mut editor, &runtime);
        let actions = frame(&ctx, &mut editor, &runtime, vec![primary(end, false)]);
        let [Action::Write {
            request: Request::Apply { edits, .. },
            ..
        }] = actions.as_slice()
        else {
            panic!("expected one write");
        };
        let [Edit::Core(CoreEdit::SetField { field, value, .. })] = edits.as_slice() else {
            panic!("{edits:?}");
        };
        assert_eq!(field, "/components/core.transform/position");
        assert!(value[0].as_f64().unwrap() > 1.6, "{value}");
        assert!((value[1].as_f64().unwrap() - 0.63).abs() < 1e-4);
        // Orbiting out of an axis view returns to perspective.
        editor.orbit.orbit_by(vec2(10.0, 0.0));
        assert!(editor.orbit.mode == ViewMode::Perspective);
        // The view cube switches views on click (Y: Top, again: Bottom).
        frame(&ctx, &mut editor, &runtime, vec![]);
        let y = editor.rects["view.cube.y"].center();
        click_at(&ctx, &mut editor, &runtime, y);
        assert!(editor.orbit.mode == ViewMode::Top);
        // Labels move with the view (Y now points at the camera); use this frame's layout.
        frame(&ctx, &mut editor, &runtime, vec![]);
        let y = editor.rects["view.cube.y"].center();
        click_at(&ctx, &mut editor, &runtime, y);
        assert!(editor.orbit.mode == ViewMode::Bottom);
    }

    /// Left-drags a selection box from `a` to `b` with `modifiers` held.
    fn box_drag(
        ctx: &egui::Context,
        editor: &mut Editor,
        runtime: &WorldRuntime,
        a: Pos2,
        b: Pos2,
        modifiers: egui::Modifiers,
    ) {
        frame_with(
            ctx,
            editor,
            runtime,
            vec![egui::Event::PointerMoved(a), primary(a, true)],
            modifiers,
        );
        for t in [0.3, 0.7, 1.0] {
            frame_with(
                ctx,
                editor,
                runtime,
                vec![egui::Event::PointerMoved(a + (b - a) * t)],
                modifiers,
            );
        }
        frame_with(ctx, editor, runtime, vec![primary(b, false)], modifiers);
    }

    #[test]
    fn box_selection_selects_inside_skips_hidden_and_locked_and_honours_modifiers() {
        let (world, runtime) = fixture();
        let mut editor = Editor::new(world, 1, None);
        editor.tool = Tool::Select;
        let ctx = egui::Context::default();
        editor.run_command("view.top").unwrap();
        // Let the panel layout settle so projected points match the next frames.
        for _ in 0..3 {
            frame(&ctx, &mut editor, &runtime, vec![]);
        }
        let state = &runtime.state.compiled;
        let center = |id: &str| {
            let i = state.indices[id];
            screen_of(
                &editor,
                focus_bounds(runtime.state.matrices()[i], state.entities[i].mesh.as_ref()).0,
            )
        };
        let (c, b) = (center("stage/crate"), center("stage/beacon"));
        let (a, z) = (c.min(b) - vec2(8.0, 8.0), c.max(b) + vec2(8.0, 8.0));
        // Everything whose center falls in the box, computed independently.
        let expected: Vec<String> = state
            .entities
            .iter()
            .enumerate()
            .filter(|(i, e)| e.transform.is_some() && runtime.authored_visibility[*i])
            .map(|(_, e)| e)
            .filter(|e| Rect::from_two_pos(a, z).contains(center(&e.id)))
            .map(|e| e.id.clone())
            .collect();
        assert!(expected.contains(&"stage/crate".to_owned()));
        assert!(expected.contains(&"stage/beacon".to_owned()));
        assert!(!expected.contains(&"stage/rack".to_owned()));
        box_drag(&ctx, &mut editor, &runtime, a, z, egui::Modifiers::NONE);
        let mut got = editor.selection();
        got.sort();
        let mut want = expected.clone();
        want.sort();
        assert_eq!(got, want);
        assert!(editor.drag.is_none() && editor.scene_input.marquee.is_none());
        // Shift adds to an existing selection; Ctrl toggles the boxed ones back out.
        editor.click_select(Some("stage/rack".into()), egui::Modifiers::NONE);
        box_drag(&ctx, &mut editor, &runtime, a, z, egui::Modifiers::SHIFT);
        assert_eq!(editor.selection().len(), want.len() + 1);
        assert_eq!(editor.selected.as_deref(), Some("stage/rack"));
        let ctrl = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
        box_drag(&ctx, &mut editor, &runtime, a, z, ctrl);
        assert_eq!(editor.selection(), ["stage/rack"]);
        // Hidden and locked objects are never boxed.
        editor.set_view_flag(&["stage/crate".into()], false);
        editor.set_view_flag(&["stage/beacon".into()], true);
        box_drag(&ctx, &mut editor, &runtime, a, z, egui::Modifiers::NONE);
        let got = editor.selection();
        assert!(
            !got.contains(&"stage/crate".to_owned()) && !got.contains(&"stage/beacon".to_owned())
        );
        assert_eq!(got.len(), want.len() - 2);
        // An empty box clears the selection.
        let corner = editor.viewport.left_top() + vec2(30.0, 30.0);
        box_drag(
            &ctx,
            &mut editor,
            &runtime,
            corner,
            corner + vec2(12.0, 12.0),
            egui::Modifiers::NONE,
        );
        assert!(editor.selection().is_empty());
    }

    #[test]
    fn component_gizmos_draw_in_the_editor_view_and_toggle_off_completely() {
        let world = pack("axiom-town");
        let mut runtime = WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap();
        let mut editor = Editor::new(world, 1, None);
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&ctx, &mut editor, &runtime, vec![]);
        }
        let on = frame_with(&ctx, &mut editor, &runtime, vec![], egui::Modifiers::NONE);
        let gizmos = editor.scene_input.gizmo_shapes;
        // Camera frustum, 15 nav.blocker rectangles and 6 FSM agents (circle + label) exist;
        // how many project in front of the default orbit depends on the layout.
        assert!(gizmos > 50, "{gizmos}");
        let instances = editor
            .render_frame(&mut runtime, ctx.screen_rect())
            .instances
            .len();
        editor.run_command("view.gizmos").unwrap();
        frame(&ctx, &mut editor, &runtime, vec![]);
        let off = frame_with(&ctx, &mut editor, &runtime, vec![], egui::Modifiers::NONE);
        assert_eq!(editor.scene_input.gizmo_shapes, 0);
        // Only the gizmos disappear (the toolbar toggle may differ by its highlight).
        assert!(
            on - off >= gizmos && on - off <= gizmos + 1,
            "on {on} off {off} gizmos {gizmos}"
        );
        // Painter-only overlays: the rendered scene is identical, and play never sees them.
        assert_eq!(
            editor
                .render_frame(&mut runtime, ctx.screen_rect())
                .instances
                .len(),
            instances
        );
        // Hidden objects lose their gizmos too.
        editor.run_command("view.gizmos").unwrap();
        frame(&ctx, &mut editor, &runtime, vec![]);
        editor.set_view_flag(&["plaza/fountain".into()], false);
        frame(&ctx, &mut editor, &runtime, vec![]);
        assert!(editor.scene_input.gizmo_shapes < gizmos);
    }

    #[test]
    fn trigger_gizmos_follow_hidden_and_toggle() {
        let world = pack("axiom-town");
        let runtime = WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap();
        let mut editor = Editor::new(world, 1, None);
        let ctx = egui::Context::default();
        let shapes = |editor: &mut Editor, runtime: &WorldRuntime| {
            for _ in 0..3 {
                frame(&ctx, editor, runtime, vec![]);
            }
            editor.scene_input.gizmo_shapes
        };
        let all = shapes(&mut editor, &runtime);
        // Hiding each sphere zone removes its 3 x 32 circle segments + 32 margin segments.
        editor.set_view_flag(&["spot/bench-east".into()], false);
        let one = shapes(&mut editor, &runtime);
        assert!(all - one >= 128, "{all} -> {one}");
        editor.set_view_flag(&["spot/hairclip".into()], false);
        let none = shapes(&mut editor, &runtime);
        assert!(one - none >= 128, "{one} -> {none}");
        // Toggling both again shows them.
        editor.set_view_flag(&["spot/bench-east".into(), "spot/hairclip".into()], false);
        assert_eq!(shapes(&mut editor, &runtime), all);
        editor.run_command("view.gizmos").unwrap();
        assert_eq!(shapes(&mut editor, &runtime), 0);
    }

    #[test]
    fn light_gizmos_draw_every_kind_and_follow_hidden_and_toggle() {
        let world = pack("light-lab");
        let runtime = WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap();
        let mut editor = Editor::new(world, 1, None);
        let ctx = egui::Context::default();
        let shapes = |editor: &mut Editor| {
            for _ in 0..3 {
                frame(&ctx, editor, &runtime, vec![]);
            }
            editor.scene_input.gizmo_shapes
        };
        let all = shapes(&mut editor);
        // Three point lights alone are 3 x 3 range circles of 32 segments.
        assert!(all > 200, "{all}");
        let mut previous = all;
        for id in [
            "statue/spot",
            "sign/light",
            "kiosk/light",
            "moon",
            "lamp/west",
        ] {
            editor.set_view_flag(&[id.into()], false);
            let now = shapes(&mut editor);
            assert!(now < previous, "hiding {id}: {now} !< {previous}");
            previous = now;
        }
        editor.run_command("view.gizmos").unwrap();
        assert_eq!(shapes(&mut editor), 0);
    }

    #[test]
    fn hidden_geometry_skinned_meshes_and_attached_vfx_are_not_drawn_in_the_editor() {
        // Procedural geometry and attached VFX (Effects Atlas).
        let world = pack("effects-tour");
        let mut runtime = WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap();
        let mut editor = Editor::new(world.clone(), 1, None);
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 760.0));
        let geometry = world
            .entities
            .iter()
            .find(|e| world.resolved_components(e).contains_key("render.geometry"))
            .unwrap()
            .id
            .clone();
        let before = editor.render_frame(&mut runtime, screen);
        assert!(before.meshes.iter().any(|m| m.id == geometry));
        let attached = |f: &crate::presentation::RenderFrame, id: &str| {
            f.vfx
                .batches
                .iter()
                .filter(|b| b.emitter.attachment.as_deref() == Some(id))
                .count()
                + f.vfx
                    .ribbons
                    .iter()
                    .filter(|r| r.attachment.as_deref() == Some(id))
                    .count()
        };
        let (ribbon, duelist) = ("fx/05-ribbon/spinner", "fx/06-attached/duelist");
        assert!(attached(&before, ribbon) > 0 && attached(&before, duelist) > 0);
        editor.set_view_flag(&[geometry.clone(), ribbon.into(), duelist.into()], false);
        let after = editor.render_frame(&mut runtime, screen);
        assert!(!after.meshes.iter().any(|m| m.id == geometry));
        assert_eq!(attached(&after, ribbon) + attached(&after, duelist), 0);
        assert!(
            after.vfx.batches.len() + after.vfx.ribbons.len()
                < before.vfx.batches.len() + before.vfx.ribbons.len()
        );
        // Play (the runtime's own frame) still draws all of it.
        let play = runtime.render_frame();
        assert!(play.meshes.iter().any(|m| m.id == geometry));
        assert!(attached(&play, ribbon) > 0 && attached(&play, duelist) > 0);
        // Skinned glTF characters (mascot).
        let world = pack("mascot");
        let mut runtime = WorldRuntime::new(world.clone(), 1, crate::installed_registry()).unwrap();
        let mut editor = Editor::new(world, 1, None);
        let id = runtime.render_frame().meshes[0].id.clone();
        assert!(editor
            .render_frame(&mut runtime, screen)
            .meshes
            .iter()
            .any(|m| m.id == id));
        editor.set_view_flag(std::slice::from_ref(&id), false);
        assert!(!editor
            .render_frame(&mut runtime, screen)
            .meshes
            .iter()
            .any(|m| m.id == id));
        assert!(runtime.render_frame().meshes.iter().any(|m| m.id == id));
    }

    #[test]
    fn locked_objects_are_drawn_but_not_pickable_or_draggable() {
        let (world, runtime) = fixture();
        let ctx = egui::Context::default();
        let mut editor = Editor::new(world.clone(), 1, None);
        editor.set_view_flag(&["stage/crate".into()], true);
        frame(&ctx, &mut editor, &runtime, vec![]);
        let at = crate_point(&editor, &runtime);
        click_at(&ctx, &mut editor, &runtime, at);
        assert_ne!(editor.selected.as_deref(), Some("stage/crate"));
        // Selected from the hierarchy, its gizmo does not respond to a drag.
        editor.select(Some("stage/crate".into()));
        let end = start_drag(&ctx, &mut editor, &runtime);
        assert!(editor.drag.is_none() && !editor.dirty());
        assert!(frame(&ctx, &mut editor, &runtime, vec![primary(end, false)]).is_empty());
        // Unlocked again, the same drag writes one transform edit.
        editor.set_view_flag(&["stage/crate".into()], true);
        editor.select(Some("stage/crate".into()));
        let end = start_drag(&ctx, &mut editor, &runtime);
        assert!(editor.drag.is_some());
        assert_eq!(
            frame(&ctx, &mut editor, &runtime, vec![primary(end, false)]).len(),
            1
        );
    }
}
