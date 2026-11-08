use crate::*;
use glam::Vec2;
use serde::Serialize;
use std::collections::VecDeque;

const DT: f32 = 1.0 / 120.0;
#[derive(Clone, Debug)]
pub struct Active {
    pub ability: String,
    pub started: u64,
    attempted: BTreeSet<String>,
}
#[derive(Clone, Debug)]
struct Buffered {
    kind: Kind,
    direction: Vec2,
    expires: u64,
}
#[derive(Clone, Debug)]
pub struct Actor {
    pub id: String,
    pub def: ActorDef,
    pub position: Vec2,
    pub yaw: f32,
    pub health: f32,
    pub max_health: f32,
    pub stamina: f32,
    pub active: Option<Active>,
    pub cooldowns: BTreeMap<String, u64>,
    buffered: Option<Buffered>,
    pub moving: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub seq: u64,
    pub tick: u64,
    pub actor: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Stats {
    pub hits: u64,
    pub dodged: u64,
    pub attacks: u64,
    pub rolls: u64,
    pub chains: u64,
    pub denied: u64,
}
#[derive(Clone, Debug)]
pub struct Simulation {
    pub session: Session,
    pub library: Library,
    pub actors: Vec<Actor>,
    pub hero: usize,
    pub target: Option<usize>,
    pub tick: u64,
    pub stats: Stats,
    pub trace: VecDeque<Event>,
    pub events: Vec<Event>,
    sequence: u64,
    pub combat: Option<axiom_combat::Scene>,
    pub hits: Vec<axiom_combat::Hit>,
    pub external_target: Option<String>,
}
pub fn walkable(s: &Session, p: Vec2, radius: f32) -> bool {
    p.x >= s.bounds.min[0] + radius
        && p.x <= s.bounds.max[0] - radius
        && p.y >= s.bounds.min[1] + radius
        && p.y <= s.bounds.max[1] - radius
        && s.obstacles.iter().all(|o| {
            let q = p.clamp(Vec2::from_array(o.min), Vec2::from_array(o.max));
            p.distance_squared(q) >= radius * radius
        })
}
fn clear_segment(s: &Session, a: Vec2, b: Vec2) -> bool {
    s.obstacles.iter().all(|o| {
        let d = b - a;
        let mut low = 0.0_f32;
        let mut high = 1.0_f32;
        for axis in 0..2 {
            if d[axis].abs() < 1e-8 {
                if a[axis] < o.min[axis] || a[axis] > o.max[axis] {
                    return true;
                }
            } else {
                let p = (o.min[axis] - a[axis]) / d[axis];
                let q = (o.max[axis] - a[axis]) / d[axis];
                low = low.max(p.min(q));
                high = high.min(p.max(q));
            }
        }
        low > high
    })
}
pub fn forward(yaw: f32) -> Vec2 {
    Vec2::new(-yaw.sin(), -yaw.cos())
}
pub fn facing(d: Vec2) -> f32 {
    (-d.x).atan2(-d.y)
}

impl Simulation {
    pub fn new(compiled: &CompiledWorld) -> Result<Self> {
        let session: Session = compiled.source.resource(SESSION)?;
        let library: Library = compiled.source.resource(ABILITIES)?;
        let mut actors = vec![];
        for e in &compiled.entities {
            if let Some(value) = e.components.get("action.actor") {
                let def: ActorDef = serde_json::from_value(value.clone())?;
                let t = e.transform.as_ref().context("action transform missing")?;
                let h = e.health.as_ref().context("action health missing")?;
                let position = Vec2::new(t.position[0], t.position[2]);
                ensure!(
                    compiled.enabled[compiled.indices[&e.id]],
                    "action actor must belong to an active scene"
                );
                actors.push(Actor {
                    id: e.id.clone(),
                    stamina: def.stamina,
                    def,
                    position,
                    yaw: facing((glam::Quat::from_array(t.rotation) * -glam::Vec3::Z).xz()),
                    health: h.value,
                    max_health: h.max,
                    active: None,
                    cooldowns: BTreeMap::new(),
                    buffered: None,
                    moving: false,
                });
            }
        }
        let hero = actors
            .iter()
            .position(|a| a.id == session.hero)
            .context("hero missing")?;
        ensure!(
            actors
                .iter()
                .any(|a| a.def.faction != actors[hero].def.faction)
                || compiled.source.resources.contains_key("combat.session"),
            "action scenario requires an opponent"
        );
        for i in 0..actors.len() {
            for b in &actors[i + 1..] {
                ensure!(
                    actors[i].position.distance(b.position) + 0.001
                        >= actors[i].def.radius + b.def.radius,
                    "actors overlap at spawn"
                );
            }
        }
        Ok(Self {
            session,
            library,
            actors,
            hero,
            target: None,
            tick: 0,
            stats: Stats::default(),
            trace: VecDeque::new(),
            events: vec![],
            sequence: 0,
            combat: None,
            hits: vec![],
            external_target: None,
        })
    }
    pub fn victory(&self) -> bool {
        if let Some(scene) = &self.combat {
            return scene
                .bodies
                .values()
                .filter(|b| b.faction != self.actors[self.hero].def.faction)
                .all(|b| !b.alive());
        }
        self.actors
            .iter()
            .filter(|a| a.def.faction != self.actors[self.hero].def.faction)
            .all(|a| a.health <= 0.0)
    }
    pub fn defeated(&self) -> bool {
        self.actors[self.hero].health <= 0.0
    }
    fn emit(&mut self, i: usize, kind: &str, value: Option<String>) {
        self.sequence += 1;
        let event = Event {
            seq: self.sequence,
            tick: self.tick,
            actor: self.actors[i].id.clone(),
            kind: kind.into(),
            value,
        };
        if self.events.len() == 512 {
            self.events.remove(0);
        }
        self.events.push(event.clone());
        self.trace.push_back(event);
        if self.trace.len() > 64 {
            self.trace.pop_front();
        }
    }
    pub fn lock(&mut self) {
        if self.target.take().is_some() || self.external_target.take().is_some() {
            return;
        }
        let hero = &self.actors[self.hero];
        if let Some(scene) = &self.combat {
            let candidate = scene
                .bodies
                .values()
                .filter(|b| {
                    b.alive()
                        && b.faction != hero.def.faction
                        && b.position.distance(hero.position) <= self.session.lock_range
                        && scene.line_clear(hero.position, b.position)
                })
                .min_by(|a, b| {
                    a.position
                        .distance_squared(hero.position)
                        .total_cmp(&b.position.distance_squared(hero.position))
                });
            self.external_target = candidate.map(|b| b.handle.id.clone());
            return;
        }
        self.target = self
            .actors
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                a.health > 0.0
                    && a.def.faction != hero.def.faction
                    && a.position.distance(hero.position) <= self.session.lock_range
                    && clear_segment(&self.session, hero.position, a.position)
            })
            .min_by(|(_, a), (_, b)| {
                a.position
                    .distance_squared(hero.position)
                    .total_cmp(&b.position.distance_squared(hero.position))
            })
            .map(|(i, _)| i);
    }
    fn choice(&self, i: usize, kind: Kind) -> String {
        let a = &self.actors[i];
        if kind == Kind::Dodge {
            return a.def.dodge.clone();
        }
        a.active
            .as_ref()
            .and_then(|active| self.library.abilities[&active.ability].chain.clone())
            .unwrap_or_else(|| a.def.attack.clone())
    }
    fn startable(&self, i: usize, id: &str) -> bool {
        let a = &self.actors[i];
        if let Some(active) = &a.active {
            let current = &self.library.abilities[&active.ability];
            let elapsed = self
                .tick
                .saturating_sub(active.started)
                .min(u64::from(u32::MAX)) as u32;
            match self.library.abilities[id].kind {
                Kind::Attack => {
                    current.chain.as_deref() == Some(id)
                        && current.chain_window.is_some_and(|w| w.contains(elapsed))
                }
                Kind::Dodge => current.dodge_window.is_some_and(|w| w.contains(elapsed)),
            }
        } else {
            true
        }
    }
    fn funded(&self, i: usize, id: &str) -> bool {
        self.actors[i].stamina + 1e-6 >= self.library.abilities[id].cost
            && self.actors[i]
                .cooldowns
                .get(id)
                .is_none_or(|&ready| ready <= self.tick)
    }
    fn start(&mut self, i: usize, id: String) {
        let ability = &self.library.abilities[&id];
        let actor = &mut self.actors[i];
        if actor.active.is_some() && ability.kind == Kind::Attack {
            self.stats.chains += 1;
        }
        actor.stamina = (actor.stamina - ability.cost).max(0.0);
        actor
            .cooldowns
            .insert(id.clone(), self.tick + u64::from(ability.cooldown));
        actor.active = Some(Active {
            ability: id.clone(),
            started: self.tick,
            attempted: BTreeSet::new(),
        });
        actor.buffered = None;
        if ability.kind == Kind::Attack {
            self.stats.attacks += 1;
        } else {
            self.stats.rolls += 1;
        }
        let sounds = ability
            .events
            .iter()
            .filter(|e| e.tick == 0)
            .map(|e| e.sound.clone())
            .collect::<Vec<_>>();
        self.emit(i, "ability", Some(id));
        for sound in sounds {
            self.emit(i, "sound", Some(sound));
        }
    }
    pub fn request(&mut self, kind: Kind, direction: Vec2) -> Result<&'static str> {
        ensure!(!self.defeated() && !self.victory(), "round has ended");
        let i = self.hero;
        let id = self.choice(i, kind);
        if !self.funded(i, &id) {
            self.stats.denied += 1;
            anyhow::bail!("ability unavailable: stamina or cooldown");
        }
        if self.startable(i, &id) {
            if kind == Kind::Dodge && direction.length_squared() > 1e-6 {
                self.actors[i].yaw = facing(direction);
            }
            self.start(i, id);
            Ok("started")
        } else {
            self.actors[i].buffered = Some(Buffered {
                kind,
                direction,
                expires: self.tick + u64::from(self.session.input_buffer_ticks),
            });
            Ok("buffered")
        }
    }
    fn try_buffer(&mut self, i: usize) {
        let Some(buffer) = self.actors[i].buffered.clone() else {
            return;
        };
        if self.tick > buffer.expires {
            self.actors[i].buffered = None;
            return;
        }
        let id = self.choice(i, buffer.kind);
        if self.startable(i, &id) && self.funded(i, &id) {
            if buffer.kind == Kind::Dodge && buffer.direction.length_squared() > 1e-6 {
                self.actors[i].yaw = facing(buffer.direction);
            }
            self.start(i, id);
        }
    }
    fn move_actor(&mut self, i: usize, delta: Vec2) {
        // Substeps prevent a fast authored roll from tunneling across a thin blocker.
        let steps = (delta.length() / 0.08).ceil().max(1.0) as usize;
        let step = delta / steps as f32;
        for _ in 0..steps {
            for offset in [Vec2::new(step.x, 0.0), Vec2::new(0.0, step.y)] {
                let proposed = self.actors[i].position + offset;
                let radius = self.actors[i].def.radius;
                if self
                    .combat
                    .as_ref()
                    .is_none_or(|scene| scene.clear(&self.actors[i].id, proposed, radius))
                    && walkable(&self.session, proposed, radius)
                    && self.actors.iter().enumerate().all(|(j, a)| {
                        j == i
                            || a.health <= 0.0
                            || proposed.distance_squared(a.position) + 1e-6
                                >= (radius + a.def.radius).powi(2)
                    })
                {
                    self.actors[i].position = proposed;
                    if let Some(scene) = &mut self.combat {
                        scene.set_position(&self.actors[i].id, proposed);
                    }
                }
            }
        }
    }
    pub fn step(&mut self, movement: Vec2) {
        self.tick += 1;
        if self.external_target.as_ref().is_some_and(|id| {
            self.combat
                .as_ref()
                .and_then(|s| s.bodies.get(id))
                .is_none_or(|b| {
                    !b.alive()
                        || b.position.distance(self.actors[self.hero].position)
                            > self.session.lock_range
                })
        }) {
            self.external_target = None;
        }
        if self.target.is_some_and(|i| {
            self.actors[i].health <= 0.0
                || self.actors[i]
                    .position
                    .distance(self.actors[self.hero].position)
                    > self.session.lock_range
        }) {
            self.target = None;
        }
        for i in 0..self.actors.len() {
            self.actors[i].moving = false;
            if self.actors[i].health <= 0.0 {
                continue;
            }
            let finished = self.actors[i].active.as_ref().is_some_and(|a| {
                self.tick - a.started >= u64::from(self.library.abilities[&a.ability].duration)
            });
            if finished {
                self.actors[i].active = None;
            }
            self.try_buffer(i);
            if self.actors[i].active.is_none() {
                self.actors[i].stamina = (self.actors[i].stamina + self.actors[i].def.regen * DT)
                    .min(self.actors[i].def.stamina);
            }
            let target_position = if let Some(scene) = &self.combat {
                if i == self.hero {
                    self.external_target
                        .as_ref()
                        .and_then(|id| scene.bodies.get(id))
                        .map(|b| b.position)
                } else {
                    scene
                        .bodies
                        .values()
                        .filter(|b| b.alive() && b.faction != self.actors[i].def.faction)
                        .min_by(|a, b| {
                            a.position
                                .distance_squared(self.actors[i].position)
                                .total_cmp(&b.position.distance_squared(self.actors[i].position))
                        })
                        .map(|b| b.position)
                }
            } else {
                let target = if i == self.hero {
                    self.target
                } else {
                    Some(self.hero)
                };
                target.map(|j| self.actors[j].position)
            };
            let direction = target_position.map_or(Vec2::ZERO, |p| p - self.actors[i].position);
            if self.actors[i].active.is_none()
                && direction.length_squared() > 1e-6
                && (i == self.hero || direction.length() <= self.session.enemy_notice_range)
            {
                self.actors[i].yaw = facing(direction);
            }
            if self.actors[i].def.ai && !self.defeated() && !self.victory() {
                let id = self.actors[i].def.attack.clone();
                let distance = direction.length();
                if self.actors[i].active.is_none()
                    && distance <= self.library.abilities[&id].range * 0.90
                    && self.combat.as_ref().is_none_or(|s| {
                        s.line_clear(
                            self.actors[i].position,
                            target_position.unwrap_or(self.actors[self.hero].position),
                        )
                    })
                    && clear_segment(
                        &self.session,
                        self.actors[i].position,
                        target_position.unwrap_or(self.actors[self.hero].position),
                    )
                    && self.funded(i, &id)
                {
                    self.start(i, id);
                } else if self.actors[i].active.is_none()
                    && distance <= self.session.enemy_notice_range
                    && distance > self.library.abilities[&id].range * 0.80
                {
                    let delta = direction.normalize_or_zero() * self.actors[i].def.speed * DT;
                    self.move_actor(i, delta);
                    self.actors[i].moving = true;
                }
            }
            if let Some(active) = self.actors[i].active.clone() {
                let ability = &self.library.abilities[&active.ability];
                let elapsed = (self.tick - active.started) as u32;
                let sounds = ability
                    .events
                    .iter()
                    .filter(|e| elapsed > 0 && e.tick == elapsed)
                    .map(|e| e.sound.clone())
                    .collect::<Vec<_>>();
                let travel = forward(self.actors[i].yaw) * ability.travel_speed * DT;
                self.move_actor(i, travel);
                for sound in sounds {
                    self.emit(i, "sound", Some(sound));
                }
            } else if i == self.hero && !self.defeated() {
                let movement = movement.clamp_length_max(1.0);
                self.move_actor(i, movement * self.actors[i].def.speed * DT);
                self.actors[i].moving = movement.length_squared() > 1e-6;
                if self.target.is_none() && self.external_target.is_none() && self.actors[i].moving
                {
                    self.actors[i].yaw = facing(movement);
                }
            }
        }
        if let Some(scene) = &self.combat {
            for i in 0..self.actors.len() {
                let a = &self.actors[i];
                let Some(active) = &a.active else { continue };
                let ability = &self.library.abilities[&active.ability];
                if !ability
                    .active
                    .is_some_and(|w| w.contains((self.tick - active.started) as u32))
                {
                    continue;
                }
                let hits = scene
                    .bodies
                    .values()
                    .filter(|b| !active.attempted.contains(&b.handle.id))
                    .filter_map(|b| {
                        scene.attempt(
                            &a.id,
                            &b.handle.id,
                            ability.damage,
                            ability.range,
                            Some((
                                forward(a.yaw),
                                (ability.arc_degrees.to_radians() * 0.5).cos(),
                            )),
                        )
                    })
                    .filter(|h| scene.in_range(h))
                    .collect::<Vec<_>>();
                for hit in hits {
                    self.actors[i]
                        .active
                        .as_mut()
                        .unwrap()
                        .attempted
                        .insert(hit.target.id.clone());
                    self.hits.push(hit);
                }
            }
            return;
        }
        // Snapshot attempts before damage, so actors attacking on the same tick can trade hits.
        let mut attempts = vec![];
        for (i, a) in self.actors.iter().enumerate() {
            let Some(active) = &a.active else {
                continue;
            };
            let ability = &self.library.abilities[&active.ability];
            if !ability
                .active
                .is_some_and(|w| w.contains((self.tick - active.started) as u32))
            {
                continue;
            }
            for (j, b) in self.actors.iter().enumerate() {
                let d = b.position - a.position;
                if b.health > 0.0
                    && a.health > 0.0
                    && a.def.faction != b.def.faction
                    && !active.attempted.contains(&b.id)
                    && d.length() <= ability.range + b.def.radius
                    && (d.length_squared() < 1e-6
                        || forward(a.yaw).dot(d.normalize()) + 1e-6
                            >= (ability.arc_degrees.to_radians() * 0.5).cos())
                    && clear_segment(&self.session, a.position, b.position)
                {
                    let invulnerable = b.active.as_ref().is_some_and(|active| {
                        self.library.abilities[&active.ability]
                            .invulnerable
                            .is_some_and(|w| w.contains((self.tick - active.started) as u32))
                    });
                    attempts.push((i, j, ability.damage, invulnerable));
                }
            }
        }
        for (i, j, damage, invulnerable) in attempts {
            let target_id = self.actors[j].id.clone();
            if let Some(active) = &mut self.actors[i].active {
                active.attempted.insert(target_id);
            }
            if invulnerable {
                self.stats.dodged += 1;
                self.emit(j, "evade", Some(self.actors[i].id.clone()));
            } else {
                let alive = self.actors[j].health > 0.0;
                self.actors[j].health = (self.actors[j].health - damage).max(0.0);
                self.stats.hits += 1;
                self.emit(i, "hit", Some(self.actors[j].id.clone()));
                self.emit(i, "sound", Some(self.session.hit_sound.clone()));
                if alive && self.actors[j].health <= 0.0 {
                    self.actors[j].active = None;
                    self.actors[j].buffered = None;
                    self.emit(j, "death", Some(self.actors[j].def.death_clip.clone()));
                }
            }
        }
    }
    pub fn combat_bodies(&self) -> Vec<axiom_combat::Body> {
        self.actors
            .iter()
            .map(|a| axiom_combat::Body {
                handle: axiom_combat::Handle {
                    id: a.id.clone(),
                    generation: 0,
                },
                owner: "action.simulate".into(),
                position: a.position,
                radius: a.def.radius,
                faction: a.def.faction,
                health: a.health,
                max_health: a.max_health,
                invulnerable: a.active.as_ref().is_some_and(|active| {
                    self.library.abilities[&active.ability]
                        .invulnerable
                        .is_some_and(|w| w.contains((self.tick - active.started) as u32))
                }),
            })
            .collect()
    }
    pub fn commit_combat(
        &mut self,
        scene: &axiom_combat::Scene,
        receipts: &[axiom_combat::Receipt],
    ) {
        for i in 0..self.actors.len() {
            if let Some(b) = scene.bodies.get(&self.actors[i].id) {
                let died = self.actors[i].health > 0.0 && b.health <= 0.0;
                self.actors[i].health = b.health;
                if died {
                    self.actors[i].active = None;
                    self.actors[i].buffered = None;
                    self.emit(i, "death", Some(self.actors[i].def.death_clip.clone()));
                }
            }
        }
        for r in receipts {
            if r.owner != "action.simulate" {
                continue;
            }
            let Some(i) = self.actors.iter().position(|a| a.id == r.source) else {
                continue;
            };
            match r.outcome {
                axiom_combat::Outcome::Hit => {
                    self.stats.hits += 1;
                    self.emit(i, "hit", Some(r.target.clone()));
                    self.emit(i, "sound", Some(self.session.hit_sound.clone()));
                }
                axiom_combat::Outcome::Evaded => {
                    self.stats.dodged += 1;
                    self.emit(i, "evade", Some(r.target.clone()));
                }
                _ => {}
            }
        }
        self.combat = Some(scene.clone());
    }
    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
    pub fn observation(&self) -> Value {
        let describe = |a: &Actor| {
            json!({
                "id":a.id,"position":a.position.to_array(),"health":a.health,"max_health":a.max_health,
                "stamina":a.stamina,"max_stamina":a.def.stamina,
                "ability":a.active.as_ref().map(|a|&a.ability),
                "elapsed":a.active.as_ref().map(|a|self.tick-a.started),
                "buffered":a.buffered.as_ref().map(|b|b.kind)
            })
        };
        json!({"tick":self.tick,"hero":describe(&self.actors[self.hero]),
            "target":self.external_target.as_ref().and_then(|id|self.combat.as_ref()?.bodies.get(id)).map(|b|json!({"id":b.handle.id,"position":b.position.to_array(),"health":b.health,"max_health":b.max_health})).or_else(||self.target.map(|i|describe(&self.actors[i]))),
            "opponents_alive":self.combat.as_ref().map_or_else(||self.actors.iter().filter(|a|a.def.faction!=self.actors[self.hero].def.faction && a.health>0.0).count(),|scene|scene.bodies.values().filter(|b|b.alive()&&b.faction!=self.actors[self.hero].def.faction).count()),
            "victory":self.victory(),"defeated":self.defeated(),"stats":self.stats,
            "trace":self.trace.iter().rev().take(12).collect::<Vec<_>>()})
    }
}

use glam::Vec3Swizzles;
