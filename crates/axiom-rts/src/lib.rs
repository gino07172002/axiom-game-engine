pub mod content;
pub mod frontier;
pub mod nav;
pub mod simulation;
use anyhow::{ensure, Context, Result};
use axiom_core::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Worker,
    Soldier,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UnitDef {
    pub faction: u8,
    pub role: Role,
    pub speed: f32,
    pub radius: f32,
    pub carry_capacity: u32,
    pub gather_ticks: u32,
    pub damage: f32,
    pub attack_range: f32,
    pub attack_ticks: u32,
}
impl UnitDef {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.faction < 2
                && self.speed.is_finite()
                && (0.1..=20.0).contains(&self.speed)
                && self.radius.is_finite()
                && (0.1..=0.4).contains(&self.radius)
                && self.carry_capacity <= 100
                && self.gather_ticks > 0
                && self.attack_ticks > 0
                && self.damage.is_finite()
                && (0.0..=1000.0).contains(&self.damage)
                && self.attack_range.is_finite()
                && (0.5..=10.0).contains(&self.attack_range),
            "invalid RTS unit"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BuildingDef {
    pub faction: u8,
    pub depot: bool,
    pub produces: Vec<String>,
    /// Defensive structures fire at the nearest enemy unit in range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attack: Option<BuildingAttack>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BuildingAttack {
    pub damage: f32,
    pub range: f32,
    pub interval_ticks: u32,
}
impl BuildingDef {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.faction < 2 && self.produces.len() <= 16,
            "invalid building"
        );
        if let Some(a) = &self.attack {
            ensure!(
                a.damage.is_finite()
                    && (0.0..=1000.0).contains(&a.damage)
                    && a.range.is_finite()
                    && (1.0..=20.0).contains(&a.range)
                    && (1..=6000).contains(&a.interval_ticks),
                "building attack: damage 0..1000, range 1..20, interval_ticks 1..6000"
            );
        }
        Ok(())
    }
}
/// Data-driven opponent director for a non-player faction: economy, production, defense and
/// escalating attack waves. Deterministic; it issues the same checked orders as the player.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Opponent {
    pub faction: u8,
    /// Decision interval in fixed ticks.
    #[serde(default = "think_ticks")]
    pub think_ticks: u64,
    /// Recipe used to keep the worker count at `workers`.
    pub worker_recipe: String,
    pub workers: u32,
    /// Army recipes, produced in rotation once the worker target is met.
    pub army: Vec<String>,
    /// No attack wave launches before this tick.
    pub first_wave_tick: u64,
    /// Idle combat units at home required to launch the first wave.
    pub wave_size: u32,
    /// Extra units required for each following wave.
    #[serde(default)]
    pub wave_growth: u32,
    /// Enemy units this close to an owned building trigger a defense.
    #[serde(default = "defend_radius")]
    pub defend_radius: f32,
}
fn think_ticks() -> u64 {
    60
}
fn defend_radius() -> f32 {
    14.0
}
impl Opponent {
    pub fn validate(&self, session: &Session) -> Result<()> {
        ensure!(
            self.faction < 2 && self.faction != session.player_faction,
            "opponent faction must be the non-player faction"
        );
        ensure!(
            (10..=1200).contains(&self.think_ticks)
                && self.workers <= 64
                && (1..=64).contains(&self.wave_size)
                && self.wave_growth <= 16
                && self.defend_radius.is_finite()
                && (2.0..=60.0).contains(&self.defend_radius)
                && (1..=8).contains(&self.army.len()),
            "opponent budgets: think_ticks 10..1200, workers <=64, wave_size 1..64, wave_growth <=16, defend_radius 2..60, army 1..8 recipes"
        );
        for r in self.army.iter().chain([&self.worker_recipe]) {
            ensure!(
                session.recipes.contains_key(r),
                "opponent recipe {r} missing"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResourceDef {
    pub amount: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub prefab: String,
    pub cost: u32,
    pub build_ticks: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Session {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ui_bindings: BTreeMap<String, String>,
    #[serde(default = "enabled")]
    pub auto_combat: bool,
    pub grid: nav::GridConfig,
    pub player_faction: u8,
    pub initial_balance: [u32; 2],
    pub max_units: usize,
    pub nav_stride: u64,
    pub path_budget: usize,
    pub search_budget: usize,
    pub camera: String,
    pub recipes: BTreeMap<String, Recipe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opponent: Option<Opponent>,
    /// Faction-1 recolor for produced units: mesh parts whose color equals `[0]` become `[1]`.
    /// Empty keeps the legacy rule (first part turns red).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub team_colors: Vec<[[f32; 3]; 2]>,
    /// Draw the interactive minimap (bottom-left). Off by default so composed worlds keep
    /// their own bottom panels clickable.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub minimap: bool,
}
fn enabled() -> bool {
    true
}
impl Session {
    pub fn validate(&self) -> Result<()> {
        nav::Grid::new(self.grid.clone())?;
        ensure!(
            self.player_faction < 2
                && (1..=1024).contains(&self.max_units)
                && (1..=30).contains(&self.nav_stride)
                && (1..=32).contains(&self.path_budget)
                && (1..=16384).contains(&self.search_budget)
                && self.recipes.len() <= 16,
            "invalid RTS session budgets"
        );
        for r in self.recipes.values() {
            ensure!(
                r.cost > 0 && r.build_ticks > 0 && r.build_ticks <= 120000,
                "invalid production recipe"
            );
        }
        if let Some(o) = &self.opponent {
            o.validate(self)?;
        }
        ensure!(
            self.team_colors.len() <= 16
                && self
                    .team_colors
                    .iter()
                    .flatten()
                    .flatten()
                    .all(|c| c.is_finite() && (0.0..=1.0).contains(c)),
            "team_colors: up to 16 pairs of 0..1 colors"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraArgs {
    pub center: [f32; 2],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectArgs {
    pub ids: Vec<String>,
    #[serde(default)]
    pub additive: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleArgs {
    pub role: Role,
    #[serde(default)]
    pub additive: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RectArgs {
    pub min: [f32; 2],
    pub max: [f32; 2],
    #[serde(default)]
    pub additive: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveArgs {
    #[serde(default)]
    pub units: Option<Vec<String>>,
    pub destination: [f32; 2],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetArgs {
    #[serde(default)]
    pub units: Option<Vec<String>>,
    pub target: String,
    /// Attack only: fight enemy units met on the way to a structure, then resume it
    /// (attack-move). Default false keeps the order literal.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub engage: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainArgs {
    pub building: String,
    pub recipe: String,
    pub count: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectArgs {
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub limit: Option<usize>,
}
pub fn resolved(w: &World, e: &Entity) -> Components {
    let mut values = e
        .prefab
        .as_ref()
        .and_then(|id| w.prefabs.get(id))
        .map(|p| p.components.clone())
        .unwrap_or_default();
    for (id, v) in &e.components {
        if let Some(old) = values.get_mut(id) {
            merge_json(old, v);
        } else {
            values.insert(id.clone(), v.clone());
        }
    }
    values
}
pub fn register(r: &mut Registry) {
    use axiom_core::gameplay::ScalarType;
    r.native_events.insert(
        "rts.delivered".into(),
        NativeEvent {
            version: 1,
            publisher: "rts.simulate".into(),
            fields: [
                ("amount".into(), ScalarType::Int),
                ("faction".into(), ScalarType::Int),
            ]
            .into(),
        },
    );
    r.native_events.insert(
        "rts.produced".into(),
        NativeEvent {
            version: 1,
            publisher: "rts.simulate".into(),
            fields: [("entity".into(), ScalarType::Text)].into(),
        },
    );
    r.capabilities.insert("rts".into(), 1);
    r.action("rts.select_role","rts",json!({"type":"object","properties":{"role":{"enum":["worker","soldier"]},"additive":{"type":"boolean"}},"required":["role"],"additionalProperties":false}),|v|{let _:RoleArgs=serde_json::from_value(v.clone())?;Ok(())});
    r.component("rts.unit","rts",json!({"faction":"0|1","role":"worker|soldier","speed":"units/second","radius":"0.1..0.4","carry_capacity":"0..100","gather_ticks":"positive integer","damage":"number","attack_range":"number","attack_ticks":"positive integer"}),|v|serde_json::from_value::<UnitDef>(v.clone())?.validate());
    r.component(
        "rts.building",
        "rts",
        json!({"faction":"0|1","depot":"bool","produces":"recipe IDs","attack":"optional {damage, range 1..20, interval_ticks}: defensive tower"}),
        |v| serde_json::from_value::<BuildingDef>(v.clone())?.validate(),
    );
    r.component(
        "rts.resource",
        "rts",
        json!({"amount":"remaining mineral units"}),
        |v| {
            let d: ResourceDef = serde_json::from_value(v.clone())?;
            ensure!(d.amount <= 1_000_000, "resource amount exceeds budget");
            Ok(())
        },
    );
    r.resource("rts.session","rts",json!({"grid":"bounded planar cells and static blocked cells","player_faction":"0|1","max_units":"1..1024 including produced units","nav_stride":"fixed-tick decision interval","path_budget":"requests solved per decision step","search_budget":"expanded nodes per request","initial_balance":"two factions","camera":"entity ID","recipes":"recipe ID -> prefab, cost, build_ticks","opponent":"optional AI director {faction, worker_recipe, workers, army:[recipes], first_wave_tick, wave_size, wave_growth?, defend_radius?, think_ticks?}","team_colors":"[[faction-0 color],[faction-1 color]] recolor pairs for produced units","minimap":"bool, draw the interactive minimap"}),|v|serde_json::from_value::<Session>(v.clone())?.validate());
    r.systems.insert(
        "rts.simulate".into(),
        SystemDescriptor {
            id: "rts.simulate".into(),
            capability: "rts".into(),
            phase: 15,
            reads: vec![
                "rts.session".into(),
                "rts.unit".into(),
                "rts.resource".into(),
                "rts.building".into(),
            ],
            writes: vec![
                "core.transform".into(),
                "core.health".into(),
                "runtime.entities".into(),
            ],
            after: vec![],
        },
    );
    r.action("rts.select","rts",json!({"type":"object","required":["ids"],"properties":{"ids":{"type":"array","maxItems":1024,"items":{"type":"string"}},"additive":{"type":"boolean"}},"additionalProperties":false}),|v|{let a:SelectArgs=serde_json::from_value(v.clone())?;ensure!(a.ids.len()<=1024,"selection budget exceeded");Ok(())});
    r.action(
        "rts.select_rect",
        "rts",
        json!({"min":"[world x,z]","max":"[world x,z]","additive":"optional bool"}),
        |v| {
            let a: RectArgs = serde_json::from_value(v.clone())?;
            ensure!(
                a.min.iter().chain(&a.max).all(|v| v.is_finite()),
                "non-finite rectangle"
            );
            Ok(())
        },
    );
    r.action(
        "rts.move",
        "rts",
        json!({"units":"optional entity IDs; defaults to selection","destination":"[world x,z]"}),
        |v| {
            let a: MoveArgs = serde_json::from_value(v.clone())?;
            ensure!(
                a.units.as_ref().is_none_or(|u| u.len() <= 1024)
                    && a.destination.iter().all(|v| v.is_finite()),
                "invalid move arguments"
            );
            Ok(())
        },
    );
    for id in ["rts.gather", "rts.attack"] {
        r.action(
            id,
            "rts",
            json!({"units":"optional owned unit IDs","target":"resource or enemy entity ID","engage":"attack only: optional bool, fight defenders met on the way to a structure, then resume (attack-move)"}),
            |v| {
                let a: TargetArgs = serde_json::from_value(v.clone())?;
                ensure!(
                    a.units.as_ref().is_none_or(|u| u.len() <= 1024),
                    "unit command budget exceeded"
                );
                valid_id(&a.target)
            },
        );
    }
    r.action(
        "rts.train",
        "rts",
        json!({"building":"owned building ID","recipe":"recipe ID","count":"1..8"}),
        |v| {
            let a: TrainArgs = serde_json::from_value(v.clone())?;
            ensure!((1..=8).contains(&a.count), "count must be 1..8");
            valid_id(&a.building)?;
            valid_id(&a.recipe)
        },
    );
    r.action("rts.stop", "rts", json!({"ids":"owned unit IDs"}), |v| {
        let a: SelectArgs = serde_json::from_value(v.clone())?;
        ensure!(a.ids.len() <= 1024, "unit budget exceeded");
        Ok(())
    });
    r.action(
        "rts.camera",
        "rts",
        json!({"center":"[world x,z] the RTS camera looks at"}),
        |v| {
            let a: CameraArgs = serde_json::from_value(v.clone())?;
            ensure!(a.center.iter().all(|v| v.is_finite()), "non-finite center");
            Ok(())
        },
    );
    r.action("rts.inspect","rts",json!({"ids":"up to 128 unit IDs","fields":"id/position/faction/role/health/order/carried; omitted selects metadata"}),|v|{let a:InspectArgs=serde_json::from_value(v.clone())?;ensure!(a.ids.len()<=128&&a.fields.len()<=16,"inspection budget exceeded");Ok(())});
    r.world_validators.push(validate_world);
    r.topics.insert("rts".into(),json!({"actions":["select","select_rect","move","gather","attack","train","stop","camera","inspect"],"opponent":"rts.session.opponent drives the other faction (gather, train, defend within defend_radius, attack waves of wave_size + n*wave_growth after first_wave_tick); summary.opponent reports wave state","ui_metrics":"minerals units workers soldiers enemy_units selected queued delivered produced wave base_health enemy_base_health (percent) victory defeated alert","victory":"all enemy depot buildings destroyed","call":{"kind":"call","capability":"rts","action":"move","args":{"destination":[8,0]}},"navigation":"4-neighbor A*, bounded queue/cache/search; command replacement invalidates stale jobs","economy":"cost reserved at queue time; delivery credits the owner; depleted resources complete when carried minerals are delivered","stress":"64-unit functional world; 128/256 independent measured runs; no predetermined frame-rate claim"}));
}
fn validate_world(w: &World) -> Result<()> {
    if !w.capabilities.contains_key("rts") {
        return Ok(());
    }
    let session: Session = w.resource("rts.session")?;
    session.validate()?;
    if !session.ui_bindings.is_empty() {
        let program: axiom_core::gameplay::Program = w.resource("behavior.program")?;
        for (metric, variable) in &session.ui_bindings {
            let expected = match metric.as_str() {
                "victory" | "defeated" | "alert" => axiom_core::gameplay::ScalarType::Bool,
                "minerals" | "units" | "selected" | "queued" | "delivered" | "produced"
                | "workers" | "soldiers" | "enemy_units" | "wave" | "base_health"
                | "enemy_base_health" => axiom_core::gameplay::ScalarType::Int,
                _ => anyhow::bail!("unknown RTS UI metric {metric}"),
            };
            ensure!(
                program
                    .variables
                    .get(variable)
                    .is_some_and(|v| v.ty() == expected),
                "RTS UI binding variable {variable} missing or wrong type"
            );
        }
    }
    ensure!(
        w.systems.iter().any(|s| s == "rts.simulate"),
        "rts capability requires rts.simulate"
    );
    let grid = nav::Grid::new(session.grid.clone())?;
    let mut count = 0;
    let mut positions = vec![];
    for e in &w.entities {
        ensure!(
            !e.id.starts_with("produced/"),
            "produced/ is reserved for runtime-generated RTS units"
        );
        let c = resolved(w, e);
        if c.contains_key("rts.unit")
            || c.contains_key("rts.building")
            || c.contains_key("rts.resource")
        {
            ensure!(
                !c.contains_key("core.motion") && !c.contains_key("fps.actor"),
                "RTS entities cannot share another movement controller"
            );
            let t: Transform = serde_json::from_value(
                c.get("core.transform")
                    .context("RTS entity requires transform")?
                    .clone(),
            )?;
            ensure!(
                t.position[1].abs() < 0.01,
                "RTS entities must be on the xz plane"
            );
            let cell = grid
                .cell([t.position[0], t.position[2]])
                .context("RTS entity outside grid")?;
            if let Some(v) = c.get("rts.unit") {
                let d: UnitDef = serde_json::from_value(v.clone())?;
                ensure!(
                    grid.walkable(cell) && d.radius < session.grid.cell_size * 0.45,
                    "unit starts in a blocked cell or has oversized footprint"
                );
                ensure!(
                    c.contains_key("core.health"),
                    "RTS unit requires core.health"
                );
                count += 1;
                for (p, r) in &positions {
                    let delta =
                        glam::Vec2::from_array(*p) - glam::Vec2::new(t.position[0], t.position[2]);
                    ensure!(
                        delta.length_squared() >= ((d.radius + *r) * (d.radius + *r)) - 0.0001,
                        "overlapping RTS spawns"
                    );
                }
                positions.push(([t.position[0], t.position[2]], d.radius));
            }
            if let Some(v) = c.get("rts.building") {
                let d: BuildingDef = serde_json::from_value(v.clone())?;
                ensure!(
                    c.contains_key("core.health"),
                    "building requires core.health"
                );
                for id in d.produces {
                    ensure!(
                        session.recipes.contains_key(&id),
                        "unknown production recipe {id}"
                    );
                }
            }
        }
    }
    ensure!(
        count <= session.max_units
            && w.entities.len() + session.max_units <= MAX_ENTITIES
            && w.entities.len() + session.max_units * 4 < MAX_INSTANCES,
        "RTS population/render budget exceeded"
    );
    let camera = w.entity(&session.camera).context("RTS camera missing")?;
    ensure!(
        resolved(w, camera).contains_key("render.camera"),
        "RTS camera requires render.camera"
    );
    for recipe in session.recipes.values() {
        let prefab = w
            .prefabs
            .get(&recipe.prefab)
            .context("production prefab missing")?;
        ensure!(
            prefab.components.contains_key("rts.unit")
                && prefab.components.contains_key("core.transform")
                && prefab.components.contains_key("core.health"),
            "production prefab requires unit/transform/health"
        );
        ensure!(
            !prefab.components.contains_key("render.camera")
                && !prefab.components.contains_key("core.motion"),
            "production prefabs cannot install another camera or motion controller"
        );
    }
    Ok(())
}
