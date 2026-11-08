//! `trigger.detect`: runtime side of `trigger.zone` (see `axiom_core::trigger`).
//!
//! Zones, filters and candidate lists are compiled once into dense typed arrays. Each fixed
//! step computes the world pivot of every enabled candidate once and bins it into a hashed
//! XZ grid (counting sort into preallocated arrays, `CELL` meters per cell). Each enabled
//! zone then tests only the candidates in the cells its bounding sphere overlaps: per step
//! O(candidates + buckets + Σ zones × nearby candidates). A zone whose bounds span more than
//! `MAX_QUERY_CELLS` cells falls back to testing all of its filter's candidates; the worst
//! case is bounded by `MAX_TRIGGER_PAIRS` (sum over zones of filter candidates). Occupants are kept as sorted slot vectors; the next set is built in
//! a reused scratch vector and swapped in, so a steady step does not allocate. Events are
//! collected first and emitted after the overlap pass, through the same gameplay command
//! path as `fsm.agent` state events, so `behavior.program` rules and `runtime.bindings` run.
use crate::world::*;
use anyhow::{ensure, Context, Result};
use axiom_core::{
    gameplay::{RuntimeCommand, Scalar},
    trigger::*,
    Transform,
};
use glam::{Mat4, Vec3};
use serde_json::{json, Value};
use std::{
    any::Any,
    collections::{BTreeMap, VecDeque},
};

/// Recent occupancy changes kept for `observe`.
const RECENT: usize = 32;
/// Zones and occupants listed per zone in `observe` (counts are always complete).
const OBSERVED_ZONES: usize = 32;
const OBSERVED_OCCUPANTS: usize = 8;
/// Grid cell edge (meters) and the largest cell range a zone queries before it falls back
/// to its whole candidate list.
const CELL: f32 = 4.0;
const MAX_QUERY_CELLS: i64 = 64;

fn cell(v: f32) -> i32 {
    // `as` saturates; pivots are finite (transforms are validated).
    (v / CELL).floor() as i32
}

fn bucket(ix: i32, iz: i32, mask: usize) -> usize {
    ((ix as u32).wrapping_mul(0x9E37_79B1) ^ (iz as u32).wrapping_mul(0x85EB_CA77)) as usize & mask
}

/// Point-in-shape test in the zone frame, the shape grown by `grow` meters.
fn contains(shape: Shape, grow: f32, inverse: glam::Quat, d: Vec3) -> bool {
    match shape {
        Shape::Sphere(r) => d.length_squared() <= (r + grow) * (r + grow),
        Shape::Box(half) => (inverse * d).abs().cmple(half + grow).all(),
    }
}

#[derive(Clone, Copy)]
enum Shape {
    Box(Vec3),
    Sphere(f32),
}

impl Shape {
    /// Bounding-sphere radius around the zone center.
    fn reach(self) -> f32 {
        match self {
            Shape::Sphere(r) => r,
            Shape::Box(half) => half.length(),
        }
    }
}

#[derive(Clone, Copy)]
struct Change {
    tick: u64,
    zone: u16,
    slot: u32,
    enter: bool,
}

struct Zone {
    slot: usize,
    shape: Shape,
    margin: f32,
    filter: usize,
    on_enter: Option<usize>,
    on_exit: Option<usize>,
    once: bool,
    enabled: bool,
    enters: u32,
    exits: u32,
    /// Occupant slots, ascending.
    inside: Vec<u32>,
}

struct Filter {
    def: TriggerFilter,
    /// Matching entity slots with a transform, ascending.
    slots: Vec<u32>,
    /// Bitset over entity slots: membership of `slots`.
    member: Vec<u64>,
}

struct EventInfo {
    name: String,
    trigger_field: bool,
    entity_field: bool,
}

pub struct TriggerSystem {
    zones: Vec<Zone>,
    /// Zone entity IDs (parallel to `zones`), for observation and hot reload.
    zone_ids: Vec<String>,
    filters: Vec<Filter>,
    events: Vec<EventInfo>,
    /// Union of every filter's slots, ascending, and their entity IDs.
    union: Vec<u32>,
    names: Vec<String>,
    /// World pivot per entity slot (only candidate slots are kept current).
    pivots: Vec<Vec3>,
    /// Entity slots already matched against the filters (runtime spawns append).
    scanned: usize,
    pairs: usize,
    next: Vec<u32>,
    /// Hashed XZ grid of enabled candidates: bucket b holds `grid_items[start[b]..start[b+1]]`.
    grid_start: Vec<u32>,
    grid_cursor: Vec<u32>,
    grid_items: Vec<u32>,
    changes: Vec<Change>,
    recent: VecDeque<Change>,
    emitted: u64,
    /// Set when a budget is exceeded; detection stops and observe reports why.
    halted: Option<String>,
    fault: Option<String>,
}

fn local(state: &RuntimeState, i: usize) -> Mat4 {
    state.transforms[i]
        .as_ref()
        .map_or(Mat4::IDENTITY, Transform::matrix)
}

/// World matrix by walking the parent chain (no allocation; depth is acyclic and small).
fn world_matrix(state: &RuntimeState, slot: usize) -> Mat4 {
    let mut m = local(state, slot);
    let mut parent = state.compiled.parents[slot];
    while let Some(p) = parent {
        m = local(state, p) * m;
        parent = state.compiled.parents[p];
    }
    m
}

fn pivot(state: &RuntimeState, slot: usize) -> Vec3 {
    let position = state.transforms[slot]
        .as_ref()
        .map_or(Vec3::ZERO, |t| Vec3::from_array(t.position));
    match state.compiled.parents[slot] {
        None => position,
        Some(p) => world_matrix(state, p).transform_point3(position),
    }
}

impl TriggerSystem {
    pub fn new(state: &RuntimeState) -> Result<Self> {
        let program = state.compiled.source.resources.get("behavior.program");
        let mut system = Self {
            zones: vec![],
            zone_ids: vec![],
            filters: vec![],
            events: vec![],
            union: vec![],
            names: vec![],
            pivots: vec![],
            scanned: 0,
            pairs: 0,
            grid_start: vec![],
            grid_cursor: vec![],
            grid_items: vec![],
            next: vec![],
            changes: Vec::with_capacity(MAX_TRIGGER_EVENTS_PER_STEP),
            recent: VecDeque::with_capacity(RECENT),
            emitted: 0,
            halted: None,
            fault: None,
        };
        for (slot, e) in state.compiled.entities.iter().enumerate() {
            let Some(zone) = &e.trigger else {
                continue;
            };
            ensure!(
                state.transforms[slot].is_some(),
                "trigger zone {} needs core.transform",
                e.id
            );
            let filter = match system.filters.iter().position(|f| f.def == zone.filter) {
                Some(i) => i,
                None => {
                    system.filters.push(Filter {
                        def: zone.filter.clone(),
                        slots: vec![],
                        member: vec![],
                    });
                    system.filters.len() - 1
                }
            };
            let mut event = |name: &str| -> Result<Option<usize>> {
                if name.is_empty() {
                    return Ok(None);
                }
                if let Some(i) = system.events.iter().position(|e| e.name == name) {
                    return Ok(Some(i));
                }
                let fields = program
                    .and_then(|p| {
                        p.pointer(&format!(
                            "/events/{}/fields",
                            axiom_core::local::escape(name)
                        ))
                    })
                    .and_then(Value::as_object)
                    .with_context(|| {
                        format!("trigger emits {name}, which behavior.program does not declare")
                    })?;
                system.events.push(EventInfo {
                    name: name.to_owned(),
                    trigger_field: fields.contains_key("trigger"),
                    entity_field: fields.contains_key("entity"),
                });
                Ok(Some(system.events.len() - 1))
            };
            let on_enter = event(&zone.on_enter)?;
            let on_exit = event(&zone.on_exit)?;
            system.zones.push(Zone {
                slot,
                shape: match zone.shape {
                    TriggerShape::Box => Shape::Box(Vec3::from_array(
                        zone.half_extents.context("box needs half_extents")?,
                    )),
                    TriggerShape::Sphere => {
                        Shape::Sphere(zone.radius.context("sphere needs radius")?)
                    }
                },
                margin: zone.exit_margin,
                filter,
                on_enter,
                on_exit,
                once: zone.once,
                enabled: zone.enabled,
                enters: 0,
                exits: 0,
                inside: vec![],
            });
            system.zone_ids.push(e.id.clone());
        }
        ensure!(
            system.zones.len() <= MAX_TRIGGERS,
            "trigger zone budget exceeded ({} > {MAX_TRIGGERS})",
            system.zones.len()
        );
        if !system.events.is_empty() {
            ensure!(
                state
                    .compiled
                    .source
                    .systems
                    .iter()
                    .any(|s| s == "behavior.execute"),
                "trigger events need the behavior.execute system"
            );
        }
        system.scan(state, true);
        if let Some(reason) = system.halted.take() {
            anyhow::bail!(reason);
        }
        Ok(system)
    }

    /// Matches entity slots added since the last scan (all of them on construction).
    fn scan(&mut self, state: &RuntimeState, initial: bool) {
        let len = state.compiled.len();
        for f in &mut self.filters {
            f.member.resize(len.div_ceil(64), 0);
        }
        for slot in self.scanned..len {
            let e = &state.compiled.entities[slot];
            if !initial && e.trigger.is_some() {
                self.halted = Some(format!(
                    "runtime-spawned entity {} has trigger.zone; spawned trigger zones are not supported (author the zone instead)",
                    e.id
                ));
                return;
            }
            if state.transforms[slot].is_none() {
                continue;
            }
            let mut any = false;
            for f in &mut self.filters {
                if f.def.matches(&e.tags, |c| e.components.contains_key(c)) {
                    f.slots.push(slot as u32);
                    f.member[slot / 64] |= 1 << (slot % 64);
                    any = true;
                }
            }
            if any {
                self.union.push(slot as u32);
                self.names.push(e.id.clone());
            }
        }
        self.scanned = len;
        self.pivots.resize(len, Vec3::ZERO);
        let buckets = (self.union.len() * 2).next_power_of_two().clamp(64, 8192);
        self.grid_start.resize(buckets + 1, 0);
        self.grid_cursor.resize(buckets, 0);
        self.grid_items.resize(self.union.len(), 0);
        self.pairs = self
            .zones
            .iter()
            .map(|z| self.filters[z.filter].slots.len())
            .sum();
        if self.pairs > MAX_TRIGGER_PAIRS {
            self.halted = Some(format!(
                "trigger candidate budget exceeded ({} zone x entity tests > {MAX_TRIGGER_PAIRS} per step)",
                self.pairs
            ));
        }
    }

    fn name(&self, slot: u32) -> &str {
        self.union
            .binary_search(&slot)
            .map_or("?", |i| self.names[i].as_str())
    }

    /// Play-preview overlay: world-space wireframe segments of every zone that is currently
    /// evaluated (enabled, entity enabled), with whether anything is inside it. Sphere = three
    /// 24-segment circles, box = 12 edges along the zone's world axes.
    pub fn wireframes(&self, state: &RuntimeState) -> Vec<(Vec3, Vec3, bool)> {
        let mut out = vec![];
        for zone in &self.zones {
            if !zone.enabled || !state.compiled.enabled[zone.slot] {
                continue;
            }
            let occupied = !zone.inside.is_empty();
            let (_, rotation, center) =
                world_matrix(state, zone.slot).to_scale_rotation_translation();
            match zone.shape {
                Shape::Sphere(r) => {
                    for (a, b) in [(Vec3::X, Vec3::Z), (Vec3::X, Vec3::Y), (Vec3::Y, Vec3::Z)] {
                        let at = |k: usize| {
                            let t = k as f32 / 24.0 * std::f32::consts::TAU;
                            center + (a * t.cos() + b * t.sin()) * r
                        };
                        for k in 0..24 {
                            out.push((at(k), at(k + 1), occupied));
                        }
                    }
                }
                Shape::Box(half) => {
                    let corner = |j: usize| {
                        let s = |bit: usize| if j & bit == 0 { -1.0 } else { 1.0 };
                        center + rotation * Vec3::new(s(1) * half.x, s(2) * half.y, s(4) * half.z)
                    };
                    for j in 0..8 {
                        for bit in [1, 2, 4] {
                            if j & bit == 0 {
                                out.push((corner(j), corner(j | bit), occupied));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Occupant entity IDs of the zone on `entity`, in authored order.
    pub fn occupants(&self, entity: &str) -> Option<Vec<&str>> {
        let z = self.zone_ids.iter().position(|id| id == entity)?;
        Some(self.zones[z].inside.iter().map(|s| self.name(*s)).collect())
    }

    fn emit(&mut self, state: &mut RuntimeState, event: usize, zone: usize, slot: u32) {
        let info = &self.events[event];
        let mut fields = BTreeMap::new();
        if info.trigger_field {
            fields.insert(
                "trigger".to_owned(),
                Scalar::Text(self.zone_ids[zone].clone()),
            );
        }
        if info.entity_field {
            fields.insert(
                "entity".to_owned(),
                Scalar::Text(self.name(slot).to_owned()),
            );
        }
        let tick = state.tick;
        let result = match state.gameplay.as_mut() {
            Some(g) => g.command(
                &RuntimeCommand::Emit {
                    event: info.name.clone(),
                    fields,
                },
                tick,
            ),
            None => Err(anyhow::anyhow!("behavior.execute is not running")),
        };
        match result {
            Ok(()) => self.emitted += 1,
            // Keep detecting; surface the first failure through observe.
            Err(e) => {
                let name = &self.events[event].name;
                self.fault
                    .get_or_insert_with(|| format!("emit {name}: {e:#}"));
            }
        }
    }

    fn summary(&self) -> Value {
        let inside: serde_json::Map<String, Value> = self
            .zones
            .iter()
            .zip(&self.zone_ids)
            .filter(|(z, _)| !z.inside.is_empty())
            .take(OBSERVED_ZONES)
            .map(|(z, id)| {
                let mut names: Vec<Value> = z
                    .inside
                    .iter()
                    .take(OBSERVED_OCCUPANTS)
                    .map(|s| json!(self.name(*s)))
                    .collect();
                if z.inside.len() > OBSERVED_OCCUPANTS {
                    names.push(json!(format!("+{}", z.inside.len() - OBSERVED_OCCUPANTS)));
                }
                (id.clone(), Value::Array(names))
            })
            .collect();
        let recent: Vec<Value> = self
            .recent
            .iter()
            .map(|c| {
                json!([
                    c.tick,
                    if c.enter { "enter" } else { "exit" },
                    self.zone_ids[c.zone as usize],
                    self.name(c.slot)
                ])
            })
            .collect();
        json!({
            "zones":self.zones.len(),
            "occupied":self.zones.iter().filter(|z| !z.inside.is_empty()).count(),
            "inside":inside,
            "recent":recent,
            "emitted":self.emitted,
            "pairs":self.pairs,
            "halted":self.halted,
            "fault":self.fault,
        })
    }
}

impl RuntimeSystem for TriggerSystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn step(&mut self, state: &mut RuntimeState, _input: &ActionMap) {
        if self.halted.is_some() {
            return;
        }
        if state.compiled.len() > self.scanned {
            self.scan(state, false);
            if self.halted.is_some() {
                return;
            }
        }
        // Pivots of enabled candidates, binned into the hashed grid (counting sort).
        let mask = self.grid_start.len() - 2;
        self.grid_start.fill(0);
        for &s in &self.union {
            let slot = s as usize;
            if state.compiled.enabled[slot] {
                let p = pivot(state, slot);
                self.pivots[slot] = p;
                self.grid_start[bucket(cell(p.x), cell(p.z), mask) + 1] += 1;
            }
        }
        for b in 0..=mask {
            self.grid_start[b + 1] += self.grid_start[b];
            self.grid_cursor[b] = self.grid_start[b];
        }
        for &s in &self.union {
            let slot = s as usize;
            if state.compiled.enabled[slot] {
                let p = self.pivots[slot];
                let b = bucket(cell(p.x), cell(p.z), mask);
                self.grid_items[self.grid_cursor[b] as usize] = s;
                self.grid_cursor[b] += 1;
            }
        }
        self.changes.clear();
        let tick = state.tick;
        for (zi, zone) in self.zones.iter_mut().enumerate() {
            if !zone.enabled || !state.compiled.enabled[zone.slot] {
                continue;
            }
            let (_, rotation, center) =
                world_matrix(state, zone.slot).to_scale_rotation_translation();
            let inverse = rotation.inverse();
            let filter = &self.filters[zone.filter];
            let pivots = &self.pivots;
            let next = &mut self.next;
            next.clear();
            let mut test = |s: u32| {
                let grow = if zone.margin > 0.0 && zone.inside.binary_search(&s).is_ok() {
                    zone.margin
                } else {
                    0.0
                };
                if s as usize != zone.slot
                    && contains(zone.shape, grow, inverse, pivots[s as usize] - center)
                {
                    next.push(s);
                }
            };
            let reach = zone.shape.reach() + zone.margin;
            let (x0, x1) = (cell(center.x - reach), cell(center.x + reach));
            let (z0, z1) = (cell(center.z - reach), cell(center.z + reach));
            let cells = (i64::from(x1) - i64::from(x0) + 1) * (i64::from(z1) - i64::from(z0) + 1);
            if cells <= MAX_QUERY_CELLS {
                for ix in x0..=x1 {
                    for iz in z0..=z1 {
                        let b = bucket(ix, iz, mask);
                        let range = self.grid_start[b] as usize..self.grid_start[b + 1] as usize;
                        for &s in &self.grid_items[range] {
                            if filter.member[s as usize / 64] & (1 << (s % 64)) != 0 {
                                test(s);
                            }
                        }
                    }
                }
            } else {
                for &s in &filter.slots {
                    if state.compiled.enabled[s as usize] {
                        test(s);
                    }
                }
            }
            // Grid buckets are hashed: restore slot order and drop repeats from collisions.
            next.sort_unstable();
            next.dedup();
            // Both lists are ascending: exits (old not in new), then enters (new not in old).
            for enter in [false, true] {
                let (from, other) = if enter {
                    (&self.next, &zone.inside)
                } else {
                    (&zone.inside, &self.next)
                };
                for &s in from {
                    if other.binary_search(&s).is_err() {
                        self.changes.push(Change {
                            tick,
                            zone: zi as u16,
                            slot: s,
                            enter,
                        });
                    }
                }
            }
            std::mem::swap(&mut zone.inside, &mut self.next);
        }
        if self.changes.is_empty() {
            return;
        }
        // Which changes emit an event (respecting `once`), counted before anything is emitted.
        let mut events = 0;
        for c in &self.changes {
            let z = &self.zones[c.zone as usize];
            let (event, fired) = if c.enter {
                (z.on_enter, z.enters)
            } else {
                (z.on_exit, z.exits)
            };
            if event.is_some() && !(z.once && fired > 0) {
                events += 1;
            }
        }
        if events > MAX_TRIGGER_EVENTS_PER_STEP {
            self.halted = Some(format!(
                "trigger event budget exceeded ({events} > {MAX_TRIGGER_EVENTS_PER_STEP} events in one step at tick {tick})"
            ));
            return;
        }
        let mut emitted_any = false;
        for i in 0..self.changes.len() {
            let c = self.changes[i];
            if self.recent.len() == RECENT {
                self.recent.pop_front();
            }
            self.recent.push_back(c);
            let z = &mut self.zones[c.zone as usize];
            let (event, fired) = if c.enter {
                (z.on_enter, &mut z.enters)
            } else {
                (z.on_exit, &mut z.exits)
            };
            let Some(event) = event else { continue };
            if z.once && *fired > 0 {
                continue;
            }
            *fired += 1;
            self.emit(state, event, c.zone as usize, c.slot);
            emitted_any = true;
        }
        if emitted_any {
            state.sync_scenes();
        }
    }
    fn transfer(
        &mut self,
        old: &dyn RuntimeSystem,
        previous: &RuntimeState,
        next: &mut RuntimeState,
    ) -> Result<()> {
        // Hot reload keeps occupancy and once-counters per zone entity ID; occupants are
        // re-resolved by entity ID (deleted occupants are dropped without an exit event).
        let Some(old) = old.as_any().downcast_ref::<Self>() else {
            return Ok(());
        };
        let remap = |slot: u32| -> Option<u32> {
            let id = &previous.compiled.entities.get(slot as usize)?.id;
            next.compiled.indices.get(id).map(|&s| s as u32)
        };
        for (z, id) in self.zones.iter_mut().zip(&self.zone_ids) {
            let Some(o) = old.zone_ids.iter().position(|x| x == id) else {
                continue;
            };
            let oz = &old.zones[o];
            z.enters = oz.enters;
            z.exits = oz.exits;
            z.inside = oz.inside.iter().filter_map(|s| remap(*s)).collect();
            z.inside.sort_unstable();
        }
        for c in &old.recent {
            let zone = self
                .zone_ids
                .iter()
                .position(|x| *x == old.zone_ids[c.zone as usize]);
            if let (Some(zone), Some(slot)) = (zone, remap(c.slot)) {
                self.recent.push_back(Change {
                    zone: zone as u16,
                    slot,
                    ..*c
                });
            }
        }
        // Carried occupants that no longer match any filter still need their ID for the exit
        // event and observe: keep them in the (sorted) known list. The grid only offers a
        // slot to zones whose filter bitset contains it, so this never adds candidates.
        let carried: Vec<u32> = self
            .zones
            .iter()
            .flat_map(|z| z.inside.iter().copied())
            .chain(self.recent.iter().map(|c| c.slot))
            .collect();
        for slot in carried {
            if let Err(i) = self.union.binary_search(&slot) {
                self.union.insert(i, slot);
                self.names
                    .insert(i, next.compiled.entities[slot as usize].id.clone());
            }
        }
        self.grid_items.resize(self.union.len(), 0);
        self.emitted = old.emitted;
        self.fault = old.fault.clone();
        Ok(())
    }
    fn command(
        &mut self,
        _state: &mut RuntimeState,
        capability: &str,
        action: &str,
        args: &Value,
    ) -> Option<Result<Value>> {
        if capability != "trigger" {
            return None;
        }
        Some(match action {
            "summary" => Ok(self.summary()),
            "inspect" => args
                .get("id")
                .and_then(Value::as_str)
                .context("trigger.inspect needs {\"id\":\"zone entity\"}")
                .and_then(|id| {
                    let z = self.zone_ids.iter().position(|x| x == id).ok_or_else(|| {
                        axiom_core::Diagnostic::error(
                            "not_found",
                            "/args/id",
                            format!("{id} is not a trigger zone"),
                        )
                    })?;
                    let zone = &self.zones[z];
                    Ok(json!({
                        "id":id,
                        "enabled":zone.enabled,
                        "inside":zone.inside.iter().map(|s| self.name(*s)).collect::<Vec<_>>(),
                        "candidates":self.filters[zone.filter].slots.len(),
                        "enters":zone.enters,"exits":zone.exits,"once":zone.once
                    }))
                }),
            other => Err(anyhow::anyhow!("unknown trigger action {other}")),
        })
    }
    fn metrics(&self) -> BTreeMap<String, Value> {
        BTreeMap::from([("trigger".into(), self.summary())])
    }
}

pub fn register(r: &mut RuntimeRegistry) {
    r.authoring.action(
        "trigger.inspect",
        "trigger",
        json!({"id":"zone entity ID; returns occupants, candidate count and enter/exit counters"}),
        |v| {
            ensure!(
                v.get("id").and_then(Value::as_str).is_some(),
                "trigger.inspect needs id"
            );
            Ok(())
        },
    );
    r.authoring
        .action("trigger.summary", "trigger", json!({}), |_| Ok(()));
    r.factories.insert("trigger.detect".into(), |state| {
        Ok(Box::new(TriggerSystem::new(state)?))
    });
}
