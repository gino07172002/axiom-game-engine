//! Typed Action authoring and deterministic planar melee simulation.
pub mod simulation;
use anyhow::{ensure, Context, Result};
use axiom_core::{
    local::{at, escape},
    *,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const SESSION: &str = "action.session";
pub const ABILITIES: &str = "action.abilities";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub start: u32,
    pub end: u32,
}
impl Window {
    pub fn contains(self, tick: u32) -> bool {
        (self.start..self.end).contains(&tick)
    }
    fn validate(self, duration: u32) -> Result<()> {
        ensure!(
            self.start < self.end && self.end <= duration,
            "window must satisfy 0 <= start < end <= duration"
        );
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Attack,
    Dodge,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AudioEvent {
    pub tick: u32,
    pub sound: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Ability {
    pub kind: Kind,
    pub clip: String,
    pub duration: u32,
    pub cost: f32,
    pub cooldown: u32,
    pub active: Option<Window>,
    pub invulnerable: Option<Window>,
    pub chain: Option<String>,
    pub chain_window: Option<Window>,
    pub dodge_window: Option<Window>,
    pub damage: f32,
    pub range: f32,
    pub arc_degrees: f32,
    pub travel_speed: f32,
    #[serde(default)]
    pub events: Vec<AudioEvent>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Library {
    pub abilities: BTreeMap<String, Ability>,
}
impl Library {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=64).contains(&self.abilities.len()),
            "ability budget must be 1..64"
        );
        for (id, a) in &self.abilities {
            let base = format!("/resources/{ABILITIES}/abilities/{}", escape(id));
            at(
                &base,
                (|| {
                    valid_id(id)?;
                    valid_id(&a.clip)?;
                    ensure!(
                        (1..=2400).contains(&a.duration) && a.cooldown <= 7200,
                        "invalid duration or cooldown"
                    );
                    ensure!(
                        [a.cost, a.damage, a.range, a.arc_degrees, a.travel_speed]
                            .iter()
                            .all(|x| x.is_finite()),
                        "nonfinite ability parameter"
                    );
                    ensure!(
                        (0.0..=1000.0).contains(&a.cost)
                            && (0.0..=1000.0).contains(&a.damage)
                            && (0.0..=6.0).contains(&a.range)
                            && (0.0..=360.0).contains(&a.arc_degrees)
                            && (0.0..=18.0).contains(&a.travel_speed),
                        "ability parameter out of range"
                    );
                    Ok(())
                })(),
            )?;
            for (name, window) in [
                ("active", a.active),
                ("invulnerable", a.invulnerable),
                ("chain_window", a.chain_window),
                ("dodge_window", a.dodge_window),
            ] {
                if let Some(window) = window {
                    at(&format!("{base}/{name}"), window.validate(a.duration))?;
                }
            }
            at(
                &base,
                (|| {
                    ensure!(
                        (a.kind == Kind::Attack) == a.active.is_some(),
                        "attacks require an active window; dodges use invulnerability"
                    );
                    ensure!(
                        a.chain.is_some() == a.chain_window.is_some(),
                        "chain and chain_window must be provided together"
                    );
                    if let Some(next) = &a.chain {
                        ensure!(
                            self.abilities
                                .get(next)
                                .is_some_and(|a| a.kind == Kind::Attack),
                            "chain must reference an attack"
                        );
                    }
                    ensure!(
                        a.events.len() <= 16 && a.events.iter().all(|e| e.tick < a.duration),
                        "invalid timeline event"
                    );
                    for e in &a.events {
                        valid_id(&e.sound)?;
                    }
                    Ok(())
                })(),
            )?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ActorDef {
    pub faction: u8,
    pub speed: f32,
    pub radius: f32,
    pub stamina: f32,
    pub regen: f32,
    pub idle_clip: String,
    pub run_clip: String,
    pub death_clip: String,
    pub attack: String,
    pub dodge: String,
    pub ai: bool,
    #[serde(default)]
    pub model_yaw: f32,
}
impl ActorDef {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.faction <= 1
                && [
                    self.speed,
                    self.radius,
                    self.stamina,
                    self.regen,
                    self.model_yaw
                ]
                .iter()
                .all(|x| x.is_finite()),
            "invalid actor"
        );
        ensure!(
            (0.1..=10.0).contains(&self.speed)
                && (0.1..=0.7).contains(&self.radius)
                && (1.0..=1000.0).contains(&self.stamina)
                && (0.0..=100.0).contains(&self.regen),
            "actor parameter out of range"
        );
        for id in [
            &self.idle_clip,
            &self.run_clip,
            &self.death_clip,
            &self.attack,
            &self.dodge,
        ] {
            valid_id(id)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Obstacle {
    pub min: [f32; 2],
    pub max: [f32; 2],
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub hero: String,
    pub camera: String,
    pub bounds: Obstacle,
    #[serde(default)]
    pub obstacles: Vec<Obstacle>,
    pub input_buffer_ticks: u32,
    pub lock_range: f32,
    pub camera_distance: f32,
    pub camera_height: f32,
    pub camera_yaw: f32,
    pub enemy_notice_range: f32,
    pub hit_sound: String,
    /// Draw the built-in combat HUD (health, stamina, lock-on, clear banner). Social or
    /// exploration content turns it off and relies on its own `ui.document`.
    #[serde(default = "combat_hud")]
    pub hud: bool,
    /// Zooming in with the optional `zoom` axis (mouse wheel) past the closest third-person
    /// distance ends in this first-person view. Without it, zoom stops at the closest
    /// third-person distance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_person: Option<FirstPerson>,
    /// Follow-camera feel (plan C1): zoom step and easing, first-person thresholds, orbit
    /// speed. Unset fields keep the engine defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_tuning: Option<CameraTuning>,
}
fn combat_hud() -> bool {
    true
}

/// `action.session.camera_tuning` (plan C1). Every field is optional; the defaults are the
/// values the follow camera always used.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraTuning {
    /// Each wheel notch toward the scene multiplies the distance by this (0.5..0.95, default 0.8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom_step: Option<f32>,
    /// Closest third-person distance; zooming in past it goes first person (meters, default 1.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_person_from: Option<f32>,
    /// At this distance and closer the view is fully first person (meters, default 0.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_person_at: Option<f32>,
    /// Fraction of the remaining distance covered per fixed step (0.01..1, default 0.08).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom_ease: Option<f32>,
    /// The hero is hidden while the camera is this close to its eye point (meters, default 0.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide_within: Option<f32>,
    /// Orbit speed at full axis (radians per second, default 1.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orbit_speed: Option<f32>,
}
impl CameraTuning {
    pub fn zoom_step(&self) -> f32 {
        self.zoom_step.unwrap_or(0.8)
    }
    pub fn first_person_from(&self) -> f32 {
        self.first_person_from.unwrap_or(1.6)
    }
    pub fn first_person_at(&self) -> f32 {
        self.first_person_at.unwrap_or(0.5)
    }
    pub fn zoom_ease(&self) -> f32 {
        self.zoom_ease.unwrap_or(0.08)
    }
    pub fn hide_within(&self) -> f32 {
        self.hide_within.unwrap_or(0.6)
    }
    pub fn orbit_speed(&self) -> f32 {
        self.orbit_speed.unwrap_or(1.5)
    }
    fn validate(&self, camera_distance: f32) -> Result<()> {
        let all = [
            self.zoom_step(),
            self.first_person_from(),
            self.first_person_at(),
            self.zoom_ease(),
            self.hide_within(),
            self.orbit_speed(),
        ];
        ensure!(
            all.iter().all(|v| v.is_finite()),
            "camera_tuning: numbers must be finite"
        );
        ensure!(
            (0.5..=0.95).contains(&self.zoom_step()),
            "camera_tuning: zoom_step must be 0.5..0.95"
        );
        ensure!(
            (0.01..=1.0).contains(&self.zoom_ease()),
            "camera_tuning: zoom_ease must be 0.01..1"
        );
        ensure!(
            self.first_person_at() >= 0.0
                && self.first_person_at() < self.first_person_from()
                && self.first_person_from() < camera_distance,
            "camera_tuning: 0 <= first_person_at < first_person_from < camera_distance"
        );
        ensure!(
            (0.0..=3.0).contains(&self.hide_within()),
            "camera_tuning: hide_within must be 0..3 m"
        );
        ensure!(
            (0.1..=10.0).contains(&self.orbit_speed()),
            "camera_tuning: orbit_speed must be 0.1..10 rad/s"
        );
        Ok(())
    }
}
/// The hero's eye point for the first-person end of the zoom range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirstPerson {
    /// Eye height above the hero's feet (meters).
    pub eye_height: f32,
    /// How far the eye sits in front of the hero's vertical axis, toward the view (meters):
    /// past the face, so the camera never looks out from inside the head.
    #[serde(default = "eye_forward")]
    pub eye_forward: f32,
}
fn eye_forward() -> f32 {
    0.25
}
impl Session {
    pub fn validate(&self) -> Result<()> {
        valid_id(&self.hero)?;
        valid_id(&self.camera)?;
        valid_id(&self.hit_sound)?;
        ensure!(
            self.obstacles.len() <= 128 && self.input_buffer_ticks <= 60,
            "session budget exceeded"
        );
        for rect in std::iter::once(&self.bounds).chain(&self.obstacles) {
            ensure!(
                rect.min
                    .iter()
                    .chain(&rect.max)
                    .all(|v| v.is_finite() && v.abs() <= 1000.0)
                    && rect.min[0] < rect.max[0]
                    && rect.min[1] < rect.max[1],
                "invalid planar rectangle"
            );
        }
        ensure!(
            [
                self.lock_range,
                self.camera_distance,
                self.camera_height,
                self.camera_yaw,
                self.enemy_notice_range
            ]
            .iter()
            .all(|x| x.is_finite()),
            "invalid camera or lock"
        );
        ensure!(
            (1.0..=50.0).contains(&self.lock_range)
                && (2.0..=12.0).contains(&self.camera_distance)
                && (1.0..=8.0).contains(&self.camera_height)
                && (1.0..=50.0).contains(&self.enemy_notice_range),
            "camera or target range out of bounds"
        );
        if let Some(c) = &self.camera_tuning {
            c.validate(self.camera_distance)?;
        }
        if let Some(f) = &self.first_person {
            ensure!(
                f.eye_height.is_finite()
                    && f.eye_forward.is_finite()
                    && (0.2..=3.0).contains(&f.eye_height)
                    && (0.0..=1.0).contains(&f.eye_forward),
                "first_person: eye_height must be 0.2..3 m and eye_forward 0..1 m"
            );
        }
        Ok(())
    }
}
pub fn resolved(world: &World, e: &Entity) -> Components {
    let mut values = e
        .prefab
        .as_ref()
        .and_then(|p| world.prefabs.get(p))
        .map(|p| p.components.clone())
        .unwrap_or_default();
    for (key, value) in &e.components {
        if let Some(old) = values.get_mut(key) {
            merge_json(old, value);
        } else {
            values.insert(key.clone(), value.clone());
        }
    }
    values
}
pub fn validate_world(w: &World) -> Result<()> {
    if !w.capabilities.contains_key("action") {
        return Ok(());
    }
    let s: Session = w.resource(SESSION)?;
    let lib: Library = w.resource(ABILITIES)?;
    let mut count = 0;
    let mut hero = false;
    let mut camera = false;
    for e in &w.entities {
        let c = resolved(w, e);
        if e.id == s.camera {
            ensure!(
                c.contains_key("render.camera") && c.contains_key("core.transform"),
                "action camera requires a camera and transform"
            );
            camera = true;
        }
        if let Some(value) = c.get("action.actor") {
            count += 1;
            let path = format!("/entities/@{}/components/action.actor", escape(&e.id));
            at(
                &path,
                (|| {
                    let a: ActorDef = serde_json::from_value(value.clone())?;
                    let t: Transform = serde_json::from_value(
                        c.get("core.transform")
                            .context("actor requires transform")?
                            .clone(),
                    )?;
                    let h: Health = serde_json::from_value(
                        c.get("core.health")
                            .context("actor requires health")?
                            .clone(),
                    )?;
                    ensure!(
                        t.position[1].abs() < 0.001 && h.value > 0.0,
                        "actors start alive on the xz plane"
                    );
                    ensure!(
                        !c.contains_key("fps.actor")
                            && !c.contains_key("rts.unit")
                            && !c.contains_key("core.motion"),
                        "actor has competing movement controllers"
                    );
                    ensure!(
                        !w.relations
                            .iter()
                            .any(|r| r.kind == "parent" && r.source == e.id),
                        "action actor requires a root transform"
                    );
                    ensure!(
                        simulation::walkable(
                            &s,
                            glam::Vec2::new(t.position[0], t.position[2]),
                            a.radius
                        ),
                        "actor starts outside the arena or inside an obstacle"
                    );
                    ensure!(
                        lib.abilities
                            .get(&a.attack)
                            .is_some_and(|a| a.kind == Kind::Attack)
                            && lib
                                .abilities
                                .get(&a.dodge)
                                .is_some_and(|a| a.kind == Kind::Dodge),
                        "actor attack/dodge reference is invalid"
                    );
                    ensure!(
                        c.contains_key("animation.player"),
                        "actor requires animation.player"
                    );
                    if e.id == s.hero {
                        ensure!(!a.ai, "hero cannot be AI controlled");
                        hero = true;
                    }
                    Ok(())
                })(),
            )?;
        }
    }
    ensure!(
        hero && camera && count <= 32,
        "action hero/camera missing or actor budget exceeded"
    );
    ensure!(
        !w.relations
            .iter()
            .any(|r| r.kind == "parent" && r.source == s.camera),
        "action camera requires a root transform"
    );
    ensure!(
        w.systems.iter().any(|s| s == "action.simulate"),
        "action.simulate is required"
    );
    Ok(())
}
pub fn register(r: &mut Registry) {
    use axiom_core::gameplay::ScalarType;
    r.resource_mergers.insert(ABILITIES.into(), |a, b| {
        axiom_core::package::merge_resource(a, b, &["abilities"], &[])
    });
    r.native_events.insert(
        "action.defeated".into(),
        NativeEvent {
            version: 1,
            publisher: "action.simulate".into(),
            fields: [
                ("entity".into(), ScalarType::Text),
                ("faction".into(), ScalarType::Int),
            ]
            .into(),
        },
    );
    r.native_events.insert(
        "action.victory".into(),
        NativeEvent {
            version: 1,
            publisher: "action.simulate".into(),
            fields: BTreeMap::new(),
        },
    );
    r.capabilities.insert("action".into(), 1);
    r.component("action.actor","action",json!({"faction":"0|1","speed":"meters/second","radius":"planar meters","stamina":"max","regen":"per second","idle_clip":"clip","run_clip":"clip","death_clip":"clip","attack":"ability ID","dodge":"ability ID","ai":"bool"}),|v|serde_json::from_value::<ActorDef>(v.clone())?.validate());
    r.movers
        .insert("action.actor".into(), "action.simulate".into());
    r.resource(ABILITIES,"action",json!({"abilities":"stable ID -> {kind:attack|dodge,clip,duration,cost,cooldown,active?,invulnerable?,chain?,chain_window?,dodge_window?,damage,range,arc_degrees,travel_speed,events:[{tick,sound}]}","windows":"half-open [start,end), fixed 120 Hz ticks","duration":"1..2400","cost":"0..1000","range":"0..6"}),|v|serde_json::from_value::<Library>(v.clone())?.validate());
    r.resource(SESSION,"action",json!({"hero":"entity ID","camera":"entity ID","bounds":"{min:[x,z],max:[x,z]}","obstacles":"bounded planar rectangles","input_buffer_ticks":"0..60","lock_range":"meters","camera_distance":"meters","camera_height":"meters","camera_yaw":"radians","enemy_notice_range":"meters","hit_sound":"audio cue ID","hud?":"bool, default true","first_person?":"{eye_height: meters 0.2..3, eye_forward?: meters 0..1 (default 0.25)}: the optional zoom axis (input.bindings, e.g. WheelUp/WheelDown) ends in a first-person view; without it zoom stops at the closest third-person distance","camera_tuning?":"{zoom_step? 0.5..0.95 (default 0.8, distance x per wheel notch), first_person_from? m (1.6), first_person_at? m (0.5), zoom_ease? 0.01..1 per step (0.08), hide_within? m (0.6), orbit_speed? rad/s (1.5)}"}),|v|serde_json::from_value::<Session>(v.clone())?.validate());
    r.systems.insert(
        "action.simulate".into(),
        SystemDescriptor {
            id: "action.simulate".into(),
            capability: "action".into(),
            phase: 20,
            after: vec![],
            reads: vec![
                SESSION.into(),
                ABILITIES.into(),
                "action.actor".into(),
                "input.bindings".into(),
            ],
            writes: vec![
                "core.transform".into(),
                "core.health".into(),
                "animation.cues".into(),
                "audio.events".into(),
            ],
        },
    );
    for name in ["attack", "dodge", "lock"] {
        r.action(&format!("action.{name}"), "action", json!({}), |v| {
            ensure!(
                v.as_object().is_some_and(|o| o.is_empty()),
                "expected empty arguments"
            );
            Ok(())
        });
    }
    r.action(
        "action.zoom",
        "action",
        json!({"distance":"meters from the hero, 0..camera_distance; 0 = first person when action.session.first_person is set (else the closest third-person distance, camera_tuning.first_person_from, default 1.6)"}),
        |v| {
            let d = v
                .as_object()
                .filter(|o| o.len() == 1)
                .and_then(|o| o.get("distance"))
                .and_then(Value::as_f64)
                .ok_or_else(|| anyhow::anyhow!("expected {{distance: meters}}"))?;
            ensure!(d.is_finite() && (0.0..=50.0).contains(&d), "distance must be 0..50 m");
            Ok(())
        },
    );
    r.world_validators.push(validate_world);
    r.topics.insert("action".into(),json!({
        "resources":[SESSION,ABILITIES],"actions":["attack","dodge","lock","zoom"],
        "inputs":{"axes":["move_x","move_z","orbit","zoom? (wheel: WheelUp / WheelDown)"],"buttons":["attack","dodge","lock"]},
        "inspect":{"op":"select","query":{"from":"resources","ids":[ABILITIES],"fields":["/abilities/slash2"]}},
        "timing":"120 Hz, windows [start,end), input buffer uses simulation ticks",
        "hit_policy":"one attempt per target per activation, invulnerability consumes the attempt, no damage through planar obstacles",
        "reload":"ability/session/actor changes restart the action simulation; unrelated edits preserve it",
        "observe":["/action/hero","/action/stats","/action/trace","/action/camera (when zoom is used)","/facing","/audio"],
        "checkpoint":"action private state is explicitly unsupported"
    }));
}
