//! Game tab, editor Play controls and the Inspector camera preview.
//!
//! Every view is composed by the renderer straight into a rectangle of the main surface
//! (`camera.viewport`, decision 5 of the editor plan): the tab body (or the Inspector preview
//! hole) stays transparent and the world drawn underneath shows through. Nothing here renders
//! offscreen or allocates per-view targets.
use super::ui::{caption, ACCENT, MUTED, WARNING};
use super::{Action, Editor};
use crate::presentation::{RenderFrame, RenderView};
use crate::world::WorldRuntime;
use egui::{Color32, Rect, RichText, Ui};
use serde_json::{json, Value};

/// Game tab aspect presets (letterboxed inside the tab body).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aspect {
    Free,
    Wide,
    Standard,
    Square,
}

impl Aspect {
    pub(crate) const ALL: [Aspect; 4] =
        [Aspect::Free, Aspect::Wide, Aspect::Standard, Aspect::Square];
    /// Name in the preferences file and `game.aspect` argument.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Aspect::Free => "free",
            Aspect::Wide => "16:9",
            Aspect::Standard => "4:3",
            Aspect::Square => "1:1",
        }
    }
    pub(crate) fn from_name(name: &str) -> Option<Aspect> {
        Aspect::ALL.into_iter().find(|a| a.name() == name.trim())
    }
    fn label(self) -> &'static str {
        match self {
            Aspect::Free => "Free Aspect",
            Aspect::Wide => "16:9",
            Aspect::Standard => "4:3",
            Aspect::Square => "1:1",
        }
    }
    /// Width / height, `None` = whatever the tab is.
    fn ratio(self) -> Option<f32> {
        match self {
            Aspect::Free => None,
            Aspect::Wide => Some(16.0 / 9.0),
            Aspect::Standard => Some(4.0 / 3.0),
            Aspect::Square => Some(1.0),
        }
    }
}

/// Simulation speeds offered by `play.time_scale` (steps per real second × 120).
pub(crate) const TIME_SCALES: [f32; 4] = [0.25, 0.5, 1.0, 2.0];

/// Smallest game / preview rectangle (points) that is still rendered.
const MIN_VIEW: f32 = 2.0;

/// The largest rectangle of `aspect` centred in `body` (the rest is letterbox).
pub(crate) fn fit(body: Rect, aspect: Aspect) -> Rect {
    let Some(ratio) = aspect.ratio() else {
        return body;
    };
    if !body.is_positive() {
        return body;
    }
    let size = if body.width() / body.height() > ratio {
        egui::vec2(body.height() * ratio, body.height())
    } else {
        egui::vec2(body.width(), body.width() / ratio)
    };
    Rect::from_center_size(body.center(), size)
}

/// A camera viewport (fractions of the game screen) placed inside `game`, as fractions of
/// the window `screen` the renderer draws into.
pub(crate) fn compose(viewport: [f32; 4], game: Rect, screen: Rect) -> [f32; 4] {
    let [x, y, w, h] = viewport;
    [
        (game.min.x + x * game.width() - screen.min.x) / screen.width(),
        (game.min.y + y * game.height() - screen.min.y) / screen.height(),
        w * game.width() / screen.width(),
        h * game.height() / screen.height(),
    ]
}

/// Smallest scale of the game's interface in the Game tab (below it text is unreadable).
pub(crate) const HUD_SCALE_MIN: f32 = 0.25;

/// Scale of the game's interface (HUD, dialogue, prompts) inside a Game tab `area`: it lays
/// out in the screen Play Maximized gives it (the editor `window`, in points) and is shrunk
/// to fit, so the Game tab shows the maximized layout at a smaller size. When the aspects
/// differ the shorter side matches and the longer one gets the extra room.
pub(crate) fn hud_scale(area: egui::Vec2, window: egui::Vec2) -> f32 {
    if !(area.x > 0.0 && area.y > 0.0 && window.x > 0.0 && window.y > 0.0) {
        return 1.0;
    }
    // Steps of 1/32 (rounded down, so it still fits): egui rebuilds the game context's font
    // atlas for every new pixels-per-point, which must not happen on every frame of a
    // splitter drag.
    let fit = (area.x / window.x).min(area.y / window.y);
    ((fit * 32.0).floor() / 32.0).clamp(HUD_SCALE_MIN, 1.0)
}

/// `rect` (editor points) in the game interface's points at `scale`.
pub(crate) fn hud_rect(rect: Rect, scale: f32) -> Rect {
    Rect::from_min_max(
        (rect.min.to_vec2() / scale).to_pos2(),
        (rect.max.to_vec2() / scale).to_pos2(),
    )
}

/// Turns the game area's input (editor points, from `app::game_raw_input`) into the game
/// interface's points at `scale`: screen and pointer positions (and wheel deltas given in
/// points) are divided by it and the
/// pixels per point multiplied (`editor_ppp` = the editor's), so the game draws on the same
/// physical pixels at `scale` times the size.
pub(crate) fn scale_game_input(raw: &mut egui::RawInput, scale: f32, editor_ppp: f32) {
    use egui::Event;
    let to_game = |p: egui::Pos2| (p.to_vec2() / scale).to_pos2();
    raw.screen_rect = raw.screen_rect.map(|r| hud_rect(r, scale));
    for event in &mut raw.events {
        match event {
            Event::PointerMoved(p) => *p = to_game(*p),
            Event::PointerButton { pos, .. } | Event::Touch { pos, .. } => *pos = to_game(*pos),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta,
                ..
            } => *delta /= scale,
            _ => {}
        }
    }
    raw.viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .native_pixels_per_point = Some(editor_ppp * scale);
}

/// What the Game tab shows and where (view state, never in revisions).
pub(crate) struct GameState {
    /// The tab body was drawn this frame.
    pub visible: bool,
    /// Letterboxed area the camera views are composed into (`NOTHING` when not drawn).
    pub rect: Rect,
    /// View-only camera choice: `None` = the runtime's active camera(s).
    pub camera: Option<String>,
    /// The pointer is over the game area and no editor window or popup covers it.
    pub hovered: bool,
    /// Clicked during Play: the host gives keyboard and mouse to the game.
    pub capture_request: bool,
    /// Esc (or the tab going away) while the game had input: the host takes it back.
    pub release_request: bool,
    /// Set by the host: the game currently receives keyboard input (editor shortcuts other
    /// than `play.*` are ignored meanwhile).
    pub input: bool,
    /// Play brought the Game tab in front of a visible Scene tab: Stop brings the Scene back.
    pub restore_scene: bool,
    /// Views composed / dropped over the camera budget in the last rendered frame.
    pub views: std::cell::Cell<u32>,
    pub dropped: std::cell::Cell<u32>,
}

impl Default for GameState {
    fn default() -> Self {
        Self {
            visible: false,
            rect: Rect::NOTHING,
            camera: None,
            hovered: false,
            capture_request: false,
            release_request: false,
            input: false,
            restore_scene: false,
            views: Default::default(),
            dropped: Default::default(),
        }
    }
}

/// Editor Play state shown in the toolbar and `observe` (the host keeps it current).
pub(crate) struct PlayView {
    pub paused: bool,
    /// Real time × scale feeds the fixed-step accumulator; every step stays `FIXED_DT`.
    pub time_scale: f32,
    pub tick: u64,
    /// `play.step` single steps since Play started.
    pub manual_steps: u64,
    /// This Play session uses the whole window (`prefs.play_maximized` when it started).
    pub maximized: bool,
}

impl Default for PlayView {
    fn default() -> Self {
        Self {
            paused: false,
            time_scale: 1.0,
            tick: 0,
            manual_steps: 0,
            maximized: false,
        }
    }
}

/// Inspector camera preview of the selected `render.camera`.
pub(crate) struct CameraPreview {
    /// Visible part of the preview hole this frame (`NOTHING` when not drawn).
    pub rect: Rect,
    pub camera: Option<String>,
}

impl Default for CameraPreview {
    fn default() -> Self {
        Self {
            rect: Rect::NOTHING,
            camera: None,
        }
    }
}

/// Read-only runtime state of the object selected during Play: results of the installed
/// `*.inspect` actions for it, refreshed by the host a few times per second.
#[derive(Default)]
pub(crate) struct LiveInspect {
    /// The Inspector drew the live panel this frame (nothing is read otherwise).
    pub shown: bool,
    pub id: String,
    pub tick: u64,
    pub at: Option<std::time::Instant>,
    /// (action, result) for every `CAPABILITY.inspect {"id"}` that answered.
    /// Compact text per action, formatted once per refresh (not per frame).
    pub results: Vec<(String, String)>,
}

/// How often the live Inspector asks the runtime again during Play.
pub(crate) const LIVE_REFRESH: std::time::Duration = std::time::Duration::from_millis(250);
/// Longest text shown per inspect result (bytes).
const LIVE_TEXT: usize = 600;

/// The view of `camera` (runtime slot) drawn into `viewport`, from the runtime's current pose.
fn camera_view(
    runtime: &WorldRuntime,
    slot: usize,
    id: &str,
    viewport: [f32; 4],
) -> Option<RenderView> {
    let camera = runtime.state.compiled.entities.get(slot)?.camera.as_ref()?;
    let m = crate::trigger::world_matrix(&runtime.state, slot);
    let forward = m.transform_vector3(-glam::Vec3::Z).normalize_or_zero();
    let up = m.transform_vector3(glam::Vec3::Y).normalize_or_zero();
    if forward == glam::Vec3::ZERO || up == glam::Vec3::ZERO {
        return None;
    }
    let mut camera = camera.clone();
    camera.viewport = viewport;
    Some(RenderView {
        id: id.to_owned(),
        eye: m.transform_point3(glam::Vec3::ZERO),
        forward,
        up,
        camera,
        instances: vec![],
    })
}

/// Fraction of `screen` covered by `rect`.
fn fraction(rect: Rect, screen: Rect) -> [f32; 4] {
    [
        (rect.min.x - screen.min.x) / screen.width(),
        (rect.min.y - screen.min.y) / screen.height(),
        rect.width() / screen.width(),
        rect.height() / screen.height(),
    ]
}

fn usable(rect: Rect, screen: Rect) -> Option<Rect> {
    let rect = rect.intersect(screen);
    (rect.is_finite() && rect.width() >= MIN_VIEW && rect.height() >= MIN_VIEW).then_some(rect)
}

impl Editor {
    /// Enters or leaves Play (the host has swapped the runtimes). Play in the Game tab opens
    /// and focuses it; the time scale is kept for the session.
    pub(crate) fn set_playing(&mut self, play: bool) {
        self.previewing = play;
        self.play.paused = false;
        self.play.manual_steps = 0;
        self.play.maximized = play && self.prefs.play_maximized;
        self.game.input = false;
        self.game.capture_request = false;
        self.game.release_request = false;
        self.live = LiveInspect::default();
        // A gizmo drag or path point drag cannot continue across the switch.
        self.drag = None;
        self.scene_input.point_drag = None;
        if play && !self.play.maximized {
            self.game.restore_scene = self.scene_visible && !self.game.visible;
            self.show_dock(super::Dock::Game);
        } else if !play && std::mem::take(&mut self.game.restore_scene) {
            self.show_dock(super::Dock::Scene);
        }
    }

    /// The object whose live state the host should read now (during Play, a few times per
    /// second, at once when the selection changed).
    pub(crate) fn live_target(&self, tick: u64) -> Option<String> {
        let id = self
            .selected
            .as_deref()
            .filter(|_| self.previewing && self.live.shown)?;
        let due = self.live.id != id
            || (self.live.tick != tick
                && self.live.at.is_none_or(|at| at.elapsed() >= LIVE_REFRESH));
        due.then(|| id.to_owned())
    }

    pub(crate) fn set_live(&mut self, id: String, tick: u64, results: Vec<(String, Value)>) {
        self.live = LiveInspect {
            shown: self.live.shown,
            id,
            tick,
            at: Some(std::time::Instant::now()),
            results: results
                .into_iter()
                .map(|(action, value)| {
                    let mut text = axiom_core::local::display_value(&value).to_string();
                    if text.len() > LIVE_TEXT {
                        let cut = (0..=LIVE_TEXT)
                            .rev()
                            .find(|&i| text.is_char_boundary(i))
                            .unwrap_or(0);
                        text.truncate(cut);
                        text.push('…');
                    }
                    (action, text)
                })
                .collect(),
        };
    }

    pub(crate) fn aspect(&self) -> Aspect {
        Aspect::from_name(&self.prefs.game_aspect).unwrap_or(Aspect::Free)
    }

    /// Whether any world view (Scene, Game, camera preview) is on screen this frame.
    pub(crate) fn world_visible(&self) -> bool {
        self.scene_visible || self.game.visible || self.camera_preview.rect.is_positive()
    }

    /// The Game tab's and the camera preview's views, appended to `frame.views` after the
    /// Scene view. `game` holds the runtime's own camera views (fractions of the game
    /// screen). The renderer draws at most `MAX_CAMERAS` views; Game views come first, then
    /// the preview; what does not fit is counted in `observe` (`game.dropped_views`).
    pub(crate) fn compose_views(
        &self,
        frame: &mut RenderFrame,
        mut game: Vec<RenderView>,
        runtime: &WorldRuntime,
        screen: Rect,
    ) {
        let budget = axiom_core::MAX_CAMERAS;
        let mut dropped = 0;
        let mut composed = 0;
        if let Some(rect) = self
            .game
            .visible
            .then(|| usable(self.game.rect, screen))
            .flatten()
        {
            if let Some(id) = &self.game.camera {
                // One chosen camera, full game area (its own viewport is for split screens).
                let full = fraction(rect, screen);
                let chosen = match game.iter().position(|v| &v.id == id) {
                    Some(i) => {
                        let mut view = game.swap_remove(i);
                        view.camera.viewport = full;
                        Some(view)
                    }
                    None => runtime
                        .state
                        .compiled
                        .indices
                        .get(id)
                        .and_then(|&slot| camera_view(runtime, slot, id, full)),
                };
                game.clear();
                game.extend(chosen);
            } else {
                for view in &mut game {
                    view.camera.viewport = compose(view.camera.viewport, rect, screen);
                }
            }
            for view in game {
                if frame.views.len() < budget {
                    frame.views.push(view);
                    composed += 1;
                } else {
                    dropped += 1;
                }
            }
        }
        if let Some(rect) = usable(self.camera_preview.rect, screen) {
            let view = self.camera_preview.camera.as_ref().and_then(|id| {
                let &slot = runtime.state.compiled.indices.get(id)?;
                camera_view(
                    runtime,
                    slot,
                    "editor.camera_preview",
                    fraction(rect, screen),
                )
            });
            if let Some(view) = view {
                if frame.views.len() < budget {
                    frame.views.push(view);
                } else {
                    dropped += 1;
                }
            }
        }
        self.game.views.set(composed);
        self.game.dropped.set(dropped);
    }

    /// `play.*` / `game.*` commands that take an argument or report errors to the bridge.
    pub(crate) fn play_command(
        &mut self,
        id: &str,
        arg: Option<&str>,
    ) -> anyhow::Result<Vec<Action>> {
        let arg = arg.map(str::trim).filter(|a| !a.is_empty());
        match (id, arg) {
            ("play.time_scale", None) => {
                self.notice("Pick a time scale in the toolbar or under Play › Time Scale");
            }
            ("play.time_scale", Some(text)) => {
                let scale = text
                    .trim_end_matches(['x', '×'])
                    .parse::<f32>()
                    .ok()
                    .filter(|s| TIME_SCALES.contains(s))
                    .ok_or_else(|| {
                        anyhow::anyhow!("time scale {text} is not one of 0.25|0.5|1|2")
                    })?;
                self.play.time_scale = scale;
                self.notice(format!("Time scale {scale}× (each step stays 1/120 s)"));
            }
            ("game.aspect", None) => self.notice("Pick the aspect in the Game tab toolbar"),
            ("game.aspect", Some(name)) => {
                let aspect = Aspect::from_name(name).ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown aspect {name}; use {}",
                        Aspect::ALL.map(Aspect::name).join("|")
                    )
                })?;
                self.prefs.game_aspect = aspect.name().into();
                self.notice(format!("Game view {}", aspect.label()));
            }
            ("game.camera", None) => self.notice("Pick the camera in the Game tab toolbar"),
            ("game.camera", Some("active")) => {
                self.game.camera = None;
                self.notice("Game view shows the active camera(s)");
            }
            ("game.camera", Some(camera)) => {
                anyhow::ensure!(
                    self.is_camera(camera),
                    "{camera} is not an object with render.camera"
                );
                self.game.camera = Some(camera.to_owned());
                self.show_dock(super::Dock::Game);
                self.notice(format!(
                    "Game view previews {camera} (view only; Set as Main Camera writes it)"
                ));
            }
            ("game.main_camera", arg) => {
                let Some(camera) = arg.map(str::to_owned).or_else(|| self.game.camera.clone())
                else {
                    self.show_dock(super::Dock::Game);
                    self.notice("Pick a camera in the Game tab first (or pass its ID)");
                    return Ok(vec![]);
                };
                let result = axiom_core::ops::make_main_camera(&self.world, &camera);
                let action = self.op_write(result, None);
                if action.is_some() {
                    // The Game view follows the new main camera once it is committed.
                    self.game.camera = None;
                }
                return Ok(action.into_iter().collect());
            }
            _ => anyhow::bail!("unknown play command {id}"),
        }
        Ok(vec![])
    }

    /// Game tab: toolbar (aspect, camera, Set as Main Camera) over the composed views.
    pub(super) fn game_tab(
        &mut self,
        ui: &mut Ui,
        runtime: &WorldRuntime,
        actions: &mut Vec<Action>,
    ) {
        ui.spacing_mut().item_spacing.y = 0.0;
        let aspect = self.aspect();
        let mut run: Option<(&'static str, Option<String>)> = None;
        egui::Frame::new()
            .fill(super::theme::HEADER)
            .inner_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt("game_aspect")
                        .selected_text(aspect.label())
                        .show_ui(ui, |ui| {
                            for a in Aspect::ALL {
                                if ui.selectable_label(a == aspect, a.label()).clicked() {
                                    run = Some(("game.aspect", Some(a.name().into())));
                                }
                            }
                        });
                    let compiled = &runtime.state.compiled;
                    let shown = self.game.camera.as_deref().unwrap_or("Active camera");
                    egui::ComboBox::from_id_salt("game_camera")
                        .selected_text(super::ui::truncate(shown, 22))
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(self.game.camera.is_none(), "Active camera")
                                .clicked()
                            {
                                run = Some(("game.camera", Some("active".into())));
                            }
                            for e in compiled.entities.iter().filter(|e| e.camera.is_some()) {
                                let active = e.camera.as_ref().is_some_and(|c| c.active);
                                let text = if active {
                                    format!("{} · main", e.id)
                                } else {
                                    e.id.clone()
                                };
                                let chosen = self.game.camera.as_deref() == Some(e.id.as_str());
                                if ui.selectable_label(chosen, text).clicked() {
                                    run = Some(("game.camera", Some(e.id.clone())));
                                }
                            }
                        })
                        .response
                        .on_hover_text(
                            "View only: what this tab shows. Set as Main Camera writes the choice.",
                        );
                    let blocked = self.command_blocked("game.main_camera");
                    let chosen_is_main = self
                        .game
                        .camera
                        .as_deref()
                        .and_then(|id| compiled.indices.get(id))
                        .and_then(|&i| compiled.entities.get(i)?.camera.as_ref())
                        .is_some_and(|c| c.active);
                    let can_set =
                        self.game.camera.is_some() && !chosen_is_main && blocked.is_none();
                    if ui
                        .add_enabled(can_set, egui::Button::new("Set as Main Camera"))
                        .on_hover_text("camera main ID (one revision)")
                        .on_disabled_hover_text(
                            blocked.unwrap_or("pick a camera that is not the main one"),
                        )
                        .clicked()
                    {
                        run = Some(("game.main_camera", None));
                    }
                    if self.previewing {
                        ui.separator();
                        ui.checkbox(&mut self.overlay, "Behavior overlay");
                        ui.separator();
                        let (hint, color) = if self.game.input {
                            ("input: game · Esc releases", ACCENT)
                        } else {
                            ("click game for input", MUTED)
                        };
                        ui.label(RichText::new(hint).size(11.0).color(color));
                    }
                    ui.separator();
                    let mut active = compiled
                        .entities
                        .iter()
                        .enumerate()
                        .filter(|(i, e)| {
                            compiled.enabled.get(*i).copied().unwrap_or(false)
                                && e.camera.as_ref().is_some_and(|c| c.active)
                        })
                        .map(|(_, e)| e.id.as_str());
                    let label = match (active.next(), active.count()) {
                        (None, _) => "no active camera".to_owned(),
                        (Some(id), 0) => format!("main: {id}"),
                        (Some(id), more) => format!("main: {id} +{more}"),
                    };
                    caption(ui, label);
                });
            });
        if let Some((id, arg)) = run {
            match self.run_command_with(id, arg.as_deref()) {
                Ok(more) => actions.extend(more),
                Err(e) => self.set_error(format!("{e:#}")),
            }
        }
        let body = ui.available_rect_before_wrap();
        let rect = fit(body, self.aspect());
        let response = ui.allocate_rect(body, egui::Sense::click());
        let painter = ui.painter_at(body);
        // Letterbox: everything outside the game area is painted; the area itself stays
        // transparent so the views drawn underneath show.
        for bar in [
            Rect::from_min_max(body.min, egui::pos2(body.max.x, rect.min.y)),
            Rect::from_min_max(egui::pos2(body.min.x, rect.max.y), body.max),
            Rect::from_min_max(
                egui::pos2(body.min.x, rect.min.y),
                egui::pos2(rect.min.x, rect.max.y),
            ),
            Rect::from_min_max(
                egui::pos2(rect.max.x, rect.min.y),
                egui::pos2(body.max.x, rect.max.y),
            ),
        ] {
            if bar.is_positive() {
                painter.rect_filled(bar, 0.0, super::theme::BACKDROP);
            }
        }
        self.game.rect = if rect.width() >= MIN_VIEW && rect.height() >= MIN_VIEW {
            rect
        } else {
            Rect::NOTHING
        };
        self.game.hovered = response.hovered()
            && ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|p| rect.contains(p));
        self.rects.insert("game.view".into(), rect);
        let has_view = self.game.camera.is_some()
            || runtime
                .state
                .compiled
                .entities
                .iter()
                .any(|e| e.camera.as_ref().is_some_and(|c| c.active));
        if !has_view {
            painter.rect_filled(rect, 0.0, super::theme::FIELD);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No active camera · pick one above",
                egui::FontId::proportional(13.0),
                MUTED,
            );
        }
        if self.previewing {
            let on_game = response
                .interact_pointer_pos()
                .is_some_and(|p| rect.contains(p));
            if response.clicked() && on_game && !self.game.input {
                self.game.capture_request = true;
            }
            if self.game.input {
                painter.rect_stroke(
                    rect.shrink(1.0),
                    0.0,
                    egui::Stroke::new(2.0_f32, ACCENT),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }

    /// Play group of the top bar (left to right): Run (with the Game tab / Maximized choice),
    /// Pause / Resume, Step, Stop, time scale and the tick. Every button is a registry command.
    pub(super) fn play_toolbar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        use super::icons::Icon;
        use super::theme::{icon_button_sized, ACCENT_SOFT, PILL_H, TEXT};
        let icons = self.icons;
        let mut run: Option<(&'static str, Option<String>)> = None;
        let blocked = self.command_blocked("play.start");
        let (fill, tint) = if self.previewing {
            (ACCENT_SOFT, TEXT)
        } else {
            (ACCENT, Color32::WHITE)
        };
        let title = if self.previewing { "Running" } else { "Run" };
        let r = ui
            .add_enabled_ui(blocked.is_none(), |ui| {
                let image = icons.image(ui.ctx(), Icon::Play, 18.0, tint);
                ui.add(
                    egui::Button::image_and_text(
                        image,
                        RichText::new(title).strong().size(15.0).color(tint),
                    )
                    .fill(fill)
                    .stroke(egui::Stroke::new(1.0_f32, super::theme::ACCENT_EDGE))
                    .corner_radius(8)
                    .min_size(egui::vec2(104.0, PILL_H)),
                )
            })
            .inner
            .on_hover_text(format!(
                "Play in the {} ({})",
                if self.prefs.play_maximized {
                    "whole window"
                } else {
                    "Game tab"
                },
                self.shortcut_text("play.start")
            ))
            .on_disabled_hover_text(blocked.unwrap_or_default());
        self.rects.insert("play:start".into(), r.rect);
        if r.clicked() {
            run = Some(("play.start", None));
        }
        // Where Play runs (the Play Maximized preference), chosen before starting.
        let target = ui
            .add_enabled_ui(!self.previewing, |ui| {
                icon_button_sized(
                    ui,
                    icons,
                    Icon::ChevronDown,
                    "Play in the Game tab or the whole window",
                    false,
                    PILL_H - 6.0,
                )
            })
            .inner;
        self.rects.insert("play:target".into(), target.rect);
        let id = ui.make_persistent_id("play_target");
        let maximized = self.prefs.play_maximized;
        super::theme::dropdown(ui, id, &target, |ui| {
            for (on, title) in [
                (false, "Play in Game tab"),
                (true, "Play Maximized (whole window)"),
            ] {
                let r = ui.selectable_label(maximized == on, title);
                self.rects.insert(format!("play:maximized:{on}"), r.rect);
                if r.clicked() {
                    if maximized != on {
                        run = Some(("play.maximize", None));
                    }
                    super::theme::close_dropdown(ui, id);
                }
            }
        });
        let paused = self.play.paused;
        let recording = self.debug.recorder.is_some() && self.previewing;
        super::theme::pill_group(ui, |ui| {
            for (icon, cmd, tip, selected) in [
                (
                    if paused && self.previewing {
                        Icon::Play
                    } else {
                        Icon::Pause
                    },
                    "play.pause",
                    if paused { "Resume" } else { "Pause" },
                    self.previewing && paused,
                ),
                (
                    Icon::Step,
                    "play.step",
                    "Step one fixed tick (1/120 s)",
                    false,
                ),
                (Icon::Stop, "play.stop", "Stop (back to editing)", false),
                (
                    Icon::Record,
                    "play.record",
                    if recording {
                        "Stop recording and save the scenario (Play continues)"
                    } else {
                        "Play and record input + commands to examples/NAME.scenario.json"
                    },
                    recording,
                ),
            ] {
                let blocked = self.command_blocked(cmd);
                let tip = format!("{tip}  {}", self.shortcut_text(cmd));
                let r = ui
                    .add_enabled_ui(blocked.is_none(), |ui| {
                        icon_button_sized(ui, icons, icon, &tip, selected, PILL_H - 4.0)
                    })
                    .inner;
                self.rects.insert(format!("play:{cmd}"), r.rect);
                if r.clicked() {
                    run = Some((cmd, None));
                }
            }
        });
        if self.previewing {
            let scale = self.play.time_scale;
            egui::ComboBox::from_id_salt("play_time_scale")
                .width(56.0)
                .selected_text(format!("{scale}×"))
                .show_ui(ui, |ui| {
                    for s in TIME_SCALES {
                        if ui.selectable_label(s == scale, format!("{s}×")).clicked() {
                            run = Some(("play.time_scale", Some(s.to_string())));
                        }
                    }
                })
                .response
                .on_hover_text("Time scale: steps per real second (each step stays 1/120 s)");
            if let Some(r) = self.debug.recorder.as_ref() {
                super::theme::chip(ui, format!("● REC {} ticks", r.ticks), super::theme::ERROR)
                    .on_hover_ui(|ui| {
                        ui.label(format!(
                            "Recording to {} ({} input changes, {} commands)",
                            r.path.display(),
                            r.inputs_len(),
                            r.commands_len(),
                        ));
                        if let Some(why) = &r.inexact {
                            ui.label(
                                RichText::new(format!("Will not replay exactly: {why}"))
                                    .color(WARNING),
                            );
                        }
                    });
            }
            super::theme::chip(
                ui,
                format!(
                    "{} · tick {}",
                    if paused { "PAUSED" } else { "PLAYING" },
                    self.play.tick
                ),
                if paused { WARNING } else { ACCENT },
            );
        } else if self.prefs.play_maximized {
            super::theme::chip(ui, "Maximized", MUTED)
                .on_hover_text("Play uses the whole window (the editor panels hide; F8 stops)");
        }
        if let Some((id, arg)) = run {
            match self.run_command_with(id, arg.as_deref()) {
                Ok(more) => actions.extend(more),
                Err(e) => self.set_error(format!("{e:#}")),
            }
        }
    }

    /// `observe` → `editor.play` / `editor.game`.
    pub(crate) fn play_observation(&self) -> (Value, Value) {
        let play = json!({
            "state": if !self.previewing {"stopped"} else if self.play.paused {"paused"} else {"playing"},
            "time_scale": self.play.time_scale,
            "tick": self.play.tick,
            "manual_steps": self.play.manual_steps,
            "maximized": if self.previewing { self.play.maximized } else { self.prefs.play_maximized },
        });
        let r = self.game.rect;
        let game = json!({
            "visible": self.game.visible,
            "aspect": self.aspect().name(),
            "camera": self.game.camera.as_deref().unwrap_or("active"),
            "rect": self.game.visible.then(|| [r.min.x, r.min.y, r.width(), r.height()].map(|v| v.round())),
            "views": self.game.views.get(),
            "dropped_views": self.game.dropped.get(),
            "input": self.game.input,
            "camera_preview": self.camera_preview.rect.is_positive().then_some(self.camera_preview.camera.as_deref()).flatten(),
        });
        (play, game)
    }
}

/// Live (read-only) runtime state of the selected object during Play.
pub(super) fn live_panel(ui: &mut Ui, live: &LiveInspect, runtime: &WorldRuntime, id: &str) {
    {
        egui::CollapsingHeader::new(RichText::new("Live (read-only)").color(ACCENT))
            .id_salt("live_inspector")
            .default_open(true)
            .show(ui, |ui| {
                let Some(&slot) = runtime.state.compiled.indices.get(id) else {
                    caption(ui, "Not in the running game (removed or not spawned).");
                    return;
                };
                caption(
                    ui,
                    format!(
                        "tick {} · {}",
                        runtime.state.tick,
                        if runtime
                            .state
                            .compiled
                            .enabled
                            .get(slot)
                            .copied()
                            .unwrap_or(false)
                        {
                            "visible"
                        } else {
                            "hidden / inactive scene"
                        }
                    ),
                );
                if let Some(t) = runtime.state.transforms.get(slot).and_then(Option::as_ref) {
                    let angles = super::viewport::degrees(t);
                    egui::Grid::new("live_transform")
                        .num_columns(2)
                        .show(ui, |ui| {
                            for (name, v) in [
                                ("Position", t.position),
                                ("Rotation°", angles),
                                ("Scale", t.scale),
                            ] {
                                ui.label(RichText::new(name).small());
                                ui.label(
                                    RichText::new(format!("{:.3}  {:.3}  {:.3}", v[0], v[1], v[2]))
                                        .monospace()
                                        .small(),
                                );
                                ui.end_row();
                            }
                        });
                }
                if let Some(h) = runtime.state.health.get(slot).and_then(Option::as_ref) {
                    ui.label(
                        RichText::new(format!("Health  {:.1}", h.value))
                            .monospace()
                            .small(),
                    );
                }
                if live.id == id {
                    for (action, value) in &live.results {
                        ui.label(RichText::new(action.as_str()).small().color(MUTED));
                        ui.label(RichText::new(value.as_str()).monospace().small());
                    }
                    if live.results.is_empty() {
                        caption(
                            ui,
                            "No runtime system reports this object (inspect actions).",
                        );
                    }
                }
                caption(
                    ui,
                    "Authored values below are read-only while playing; stop Play to edit.",
                );
            });
    }
}

/// Inspector camera preview of `id` (an object with `render.camera`): reserves the hole the
/// renderer composes the camera into. Only in a docked (not floating) Inspector. Returns
/// whether the preview toggle was clicked (the caller runs `view.camera_preview`).
pub(super) fn camera_preview_hole(
    ui: &mut Ui,
    enabled: bool,
    preview: &mut CameraPreview,
    rects: &mut std::collections::HashMap<String, Rect>,
    id: &str,
    floating: bool,
) -> bool {
    let mut toggle = false;
    ui.horizontal(|ui| {
        caption(ui, "CAMERA PREVIEW");
        let mut on = enabled;
        let response = ui
            .checkbox(&mut on, "")
            .on_hover_text("view.camera_preview: one extra view, only while visible");
        rects.insert("camera.preview.toggle".into(), response.rect);
        toggle = response.changed();
    });
    if !enabled {
        return toggle;
    }
    if floating {
        caption(ui, "Dock the Inspector to see the preview.");
        return toggle;
    }
    // The Inspector's content can be wider than the tab (long rows): fit the visible width.
    let width = ui
        .available_width()
        .min(ui.clip_rect().max.x - ui.cursor().min.x - 1.0)
        .max(1.0);
    let height = (width * 9.0 / 16.0).clamp(60.0, 200.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    // Only while wholly visible: the renderer takes the projection aspect from the composed
    // rectangle, so a part scrolled out of the Inspector would squash the image, not crop it.
    preview.rect = if ui.clip_rect().contains_rect(rect) {
        rect
    } else {
        Rect::NOTHING
    };
    preview.camera = Some(id.to_owned());
    rects.insert("camera.preview".into(), rect);
    ui.painter().rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0_f32, Color32::from_rgb(70, 86, 104)),
        egui::StrokeKind::Outside,
    );
    toggle
}
