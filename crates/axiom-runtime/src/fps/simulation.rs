use anyhow::Result;
use axiom_fps::{EntityDef, LegacyWorld as World, Sentinel, State};
use glam::Vec3;
use rapier3d::{control::KinematicCharacterController, prelude::*};
use serde::Serialize;

pub const FIXED_DT: f32 = 1.0 / 120.0;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct EntityId {
    pub index: u32,
    pub generation: u32,
}

/// Dense, parallel component arrays. Logical slots remain stable across content edits.
pub struct Actors {
    pub ids: Vec<EntityId>,
    pub positions: Vec<Vec3>,
    pub health: Vec<f32>,
    pub states: Vec<State>,
    pub cooldowns: Vec<f32>,
    pub colliders: Vec<ColliderHandle>,
    next_generation: u32,
}

#[derive(Default, Clone, Copy)]
pub struct Input {
    pub forward: f32,
    pub right: f32,
    pub sprint: bool,
    pub jump: bool,
    pub fire: bool,
    pub reload: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Observation {
    pub rev: i64,
    pub tick: u64,
    pub elapsed_seconds: f32,
    pub player_position: [f32; 3],
    pub player_health: f32,
    pub enemies_alive: usize,
    pub enemies_total: usize,
    pub kills: u32,
    pub shots: u32,
    pub hits: u32,
    pub ammo: u32,
    pub victory: bool,
    pub defeated: bool,
    pub frame_ms_cpu: f32,
    pub fps: f32,
    pub gpu: String,
}

pub struct Simulation {
    pub world: World,
    pub rev: i64,
    pub tick: u64,
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub velocity_y: f32,
    pub grounded: bool,
    pub health: f32,
    pub ammo: u32,
    pub reload_left: f32,
    pub fire_cooldown: f32,
    pub kills: u32,
    pub shots: u32,
    pub hits: u32,
    pub hit_flash: f32,
    pub damage_flash: f32,
    pub muzzle: f32,
    pub tracers: Vec<(Vec3, Vec3, f32)>,
    pub actors: Actors,
    pub compiled: Vec<EntityDef>,
    pub camera: axiom_core::Camera,
    initial_health: Vec<f32>,
    initial_camera_rotation: [f32; 4],
    settings: Vec<Sentinel>,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    query: QueryPipeline,
    player_collider: ColliderHandle,
    controller: KinematicCharacterController,
}

impl Simulation {
    pub fn new(world: World, rev: i64) -> Result<Self> {
        let v2 = axiom_fps::from_legacy(&world)?;
        Self::from_compiled(&v2.compile(&axiom_fps::registry())?, rev)
    }
    pub fn from_compiled(compiled: &axiom_core::CompiledWorld, rev: i64) -> Result<Self> {
        let world = axiom_fps::from_world(&compiled.source)?;
        let (definitions, settings) = axiom_fps::actor_definitions(compiled)?;
        let mut sim = Self::new_config(world, definitions, settings, rev)?;
        let session: axiom_fps::Session = compiled.source.resource("fps.session")?;
        let camera = &compiled.entities[compiled.indices[&session.camera]];
        sim.camera = camera.camera.clone().unwrap();
        sim.initial_camera_rotation = camera.transform.as_ref().unwrap().rotation;
        let (yaw, pitch, _) =
            glam::Quat::from_array(sim.initial_camera_rotation).to_euler(glam::EulerRot::YXZ);
        sim.yaw = -yaw;
        sim.pitch = pitch;
        let player = compiled.indices[&sim.compiled[0].id];
        sim.health = compiled.entities[player].health.as_ref().unwrap().value;
        for i in 0..sim.actors.health.len() {
            let slot = compiled.indices[&sim.compiled[i + 1].id];
            sim.actors.health[i] = compiled.entities[slot].health.as_ref().unwrap().value;
            if sim.actors.health[i] <= 0.0 {
                sim.actors.states[i] = State::Dead;
            }
        }
        sim.initial_health = std::iter::once(sim.health)
            .chain(sim.actors.health.iter().copied())
            .collect();
        sim.rebuild_collision();
        Ok(sim)
    }
    fn new_config(
        world: World,
        compiled: Vec<EntityDef>,
        settings: Vec<Sentinel>,
        rev: i64,
    ) -> Result<Self> {
        let initial_health = compiled.iter().map(|d| d.health.unwrap()).collect();
        let mut sim = Self {
            camera: axiom_core::Camera::perspective(),
            initial_health,
            initial_camera_rotation: [0.0, 0.0, 0.0, 1.0],
            position: Vec3::from_array(compiled[0].position),
            health: world.player.health,
            ammo: world.weapon.magazine,
            world,
            rev,
            tick: 0,
            yaw: 0.0,
            pitch: -0.035,
            velocity_y: 0.0,
            grounded: true,
            reload_left: 0.0,
            fire_cooldown: 0.0,
            kills: 0,
            shots: 0,
            hits: 0,
            hit_flash: 0.0,
            damage_flash: 0.0,
            muzzle: 0.0,
            tracers: Vec::with_capacity(32),
            actors: Actors {
                ids: vec![],
                positions: vec![],
                health: vec![],
                states: vec![],
                cooldowns: vec![],
                colliders: vec![],
                next_generation: 1,
            },
            compiled,
            settings,
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            query: QueryPipeline::new(),
            player_collider: ColliderHandle::invalid(),
            controller: KinematicCharacterController::default(),
        };
        sim.rebuild_collision();
        sim.spawn_to_count();
        Ok(sim)
    }

    fn rebuild_collision(&mut self) {
        self.colliders = ColliderSet::new();
        self.bodies = RigidBodySet::new();
        let h = self.world.arena.half_size;
        let height = self.world.arena.wall_height;
        let cuboids = [
            ([0.0, -0.5, 0.0], [h + 1.0, 0.5, h + 1.0]),
            ([-h - 0.5, height * 0.5, 0.0], [0.5, height * 0.5, h + 1.0]),
            ([h + 0.5, height * 0.5, 0.0], [0.5, height * 0.5, h + 1.0]),
            ([0.0, height * 0.5, -h - 0.5], [h, height * 0.5, 0.5]),
            ([0.0, height * 0.5, h + 0.5], [h, height * 0.5, 0.5]),
        ];
        for (p, s) in cuboids {
            self.colliders.insert(
                ColliderBuilder::cuboid(s[0], s[1], s[2]).translation(vector![p[0], p[1], p[2]]),
            );
        }
        for o in &self.world.obstacles {
            self.colliders.insert(
                ColliderBuilder::cuboid(o.size[0] * 0.5, o.size[1] * 0.5, o.size[2] * 0.5)
                    .translation(vector![o.position[0], o.position[1], o.position[2]]),
            );
        }
        self.player_collider =
            self.colliders
                .insert(ColliderBuilder::capsule_y(0.55, 0.35).translation(vector![
                    self.position.x,
                    self.position.y,
                    self.position.z
                ]));
        self.actors.colliders.clear();
        for i in 0..self.actors.positions.len() {
            let handle =
                self.insert_actor_collider(self.actors.positions[i], self.actors.health[i] > 0.0);
            self.actors.colliders.push(handle);
        }
        self.query.update(&self.colliders);
    }

    fn insert_actor_collider(&mut self, p: Vec3, alive: bool) -> ColliderHandle {
        self.colliders.insert(
            ColliderBuilder::capsule_y(0.5, 0.4)
                .translation(vector![p.x, p.y, p.z])
                .enabled(alive),
        )
    }
    fn spawn_to_count(&mut self) {
        while self.actors.positions.len() < self.world.enemies.count {
            let i = self.actors.positions.len();
            let p = Vec3::from_array(self.compiled[i + 1].position);
            let handle = self.insert_actor_collider(p, true);
            self.actors.ids.push(EntityId {
                index: i as u32,
                generation: self.actors.next_generation,
            });
            self.actors.next_generation = self.actors.next_generation.wrapping_add(1);
            self.actors.positions.push(p);
            self.actors
                .health
                .push(self.compiled[i + 1].health.unwrap());
            self.actors.states.push(self.world.behavior.initial);
            self.actors.cooldowns.push(0.0);
            self.actors.colliders.push(handle);
        }
        self.query.update(&self.colliders);
    }

    /// Apply only between fixed ticks. Preserve actors and damage when tuning values.
    pub fn reconcile(&mut self, world: World, rev: i64) -> Result<()> {
        let v2 = axiom_fps::from_legacy(&world)?;
        let mut next = Self::from_compiled(&v2.compile(&axiom_fps::registry())?, rev)?;
        next.restore_from(self);
        *self = next;
        Ok(())
    }
    pub fn restore_from(&mut self, old: &Self) {
        let layout = self.world.arena.half_size != old.world.arena.half_size
            || self.world.arena.wall_height != old.world.arena.wall_height
            || self.world.obstacles != old.world.obstacles
            || self.world.player.spawn != old.world.player.spawn
            || self.compiled[0].id != old.compiled[0].id;
        self.tick = old.tick;
        if self.initial_camera_rotation == old.initial_camera_rotation {
            self.yaw = old.yaw;
            self.pitch = old.pitch;
        }
        if !layout {
            self.position = old.position;
            self.velocity_y = old.velocity_y;
            self.grounded = old.grounded;
        }
        self.health = (old.health + self.initial_health[0] - old.initial_health[0])
            .clamp(0.0, self.world.player.health);
        self.ammo = old.ammo.min(self.world.weapon.magazine);
        self.reload_left = old.reload_left.min(self.world.weapon.reload_seconds);
        self.fire_cooldown = old.fire_cooldown;
        self.kills = old.kills;
        self.shots = old.shots;
        self.hits = old.hits;
        self.hit_flash = old.hit_flash;
        self.damage_flash = old.damage_flash;
        let mut generation = old.actors.next_generation;
        for i in 0..self.actors.positions.len() {
            if let Some(j) = old
                .compiled
                .iter()
                .skip(1)
                .position(|d| d.id == self.compiled[i + 1].id)
            {
                self.actors.ids[i] = old.actors.ids[j];
                self.actors.states[i] = old.actors.states[j];
                self.actors.cooldowns[i] = old.actors.cooldowns[j];
                self.actors.health[i] = if old.actors.health[j] <= 0.0 {
                    old.actors.health[j]
                } else if self.initial_health[i + 1] == 0.0 {
                    0.0
                } else {
                    (old.actors.health[j] + self.initial_health[i + 1] - old.initial_health[j + 1])
                        .max(1.0)
                        .min(self.max_health(i))
                };
                if self.actors.health[i] <= 0.0 {
                    self.actors.states[i] = State::Dead;
                }
                if !layout && self.compiled[i + 1].position == old.compiled[j + 1].position {
                    self.actors.positions[i] = old.actors.positions[j];
                }
            } else {
                self.actors.ids[i].generation = generation;
                generation = generation.wrapping_add(1);
            }
        }
        self.actors.next_generation = generation;
        self.rebuild_collision();
    }
    pub fn max_health(&self, index: usize) -> f32 {
        self.compiled[index + 1].health.unwrap()
    }
    pub fn camera_projection(&self, aspect: f32) -> glam::Mat4 {
        match self.camera.projection {
            axiom_core::Projection::Perspective { fov_degrees } => glam::Mat4::perspective_rh(
                fov_degrees.to_radians(),
                aspect,
                self.camera.near,
                self.camera.far,
            ),
            axiom_core::Projection::Orthographic { height } => glam::Mat4::orthographic_rh(
                -height * aspect * 0.5,
                height * aspect * 0.5,
                -height * 0.5,
                height * 0.5,
                self.camera.near,
                self.camera.far,
            ),
        }
    }
    pub fn eye(&self) -> Vec3 {
        self.position + Vec3::Y * 0.65
    }
    pub fn actor_visible(&self, index: usize) -> bool {
        let Some(&position) = self.actors.positions.get(index) else {
            return false;
        };
        if self.actors.health[index] <= 0.0 {
            return false;
        }
        let start = self.eye();
        let delta = position + Vec3::Y * 0.6 - start;
        let distance = delta.length();
        if distance < 0.001 {
            return false;
        }
        let d = delta / distance;
        self.query
            .cast_ray(
                &self.bodies,
                &self.colliders,
                &Ray::new(point![start.x, start.y, start.z], vector![d.x, d.y, d.z]),
                distance + 0.01,
                true,
                QueryFilter::default().exclude_collider(self.player_collider),
            )
            .is_some_and(|(handle, _)| handle == self.actors.colliders[index])
    }
    pub fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        )
    }
    pub fn alive(&self) -> usize {
        self.actors.health.iter().filter(|&&hp| hp > 0.0).count()
    }
    pub fn victory(&self) -> bool {
        self.alive() == 0 && !self.actors.health.is_empty()
    }
    pub fn look(&mut self, dx: f32, dy: f32) {
        self.yaw += dx * 0.002;
        self.pitch = (self.pitch - dy * 0.002).clamp(-1.5, 1.5);
    }

    fn move_capsule(&self, handle: ColliderHandle, delta: Vec3, dt: f32) -> (Vec3, bool) {
        let collider = &self.colliders[handle];
        let movement = self.controller.move_shape(
            dt,
            &self.bodies,
            &self.colliders,
            &self.query,
            collider.shape(),
            collider.position(),
            vector![delta.x, delta.y, delta.z],
            QueryFilter::default().exclude_collider(handle),
            |_| {},
        );
        (
            Vec3::new(
                movement.translation.x,
                movement.translation.y,
                movement.translation.z,
            ),
            movement.grounded,
        )
    }

    pub fn step(&mut self, input: Input) {
        let dt = FIXED_DT;
        self.hit_flash = (self.hit_flash - dt).max(0.0);
        self.damage_flash = (self.damage_flash - dt).max(0.0);
        self.muzzle = (self.muzzle - dt).max(0.0);
        self.tracers.iter_mut().for_each(|t| t.2 -= dt);
        self.tracers.retain(|t| t.2 > 0.0);
        if self.health <= 0.0 || self.victory() {
            return;
        }
        self.tick += 1;
        self.fire_cooldown = (self.fire_cooldown - dt).max(0.0);
        if self.reload_left > 0.0 {
            self.reload_left -= dt;
            if self.reload_left <= 0.0 {
                self.reload_left = 0.0;
                self.ammo = self.world.weapon.magazine;
            }
        }
        if input.reload && self.reload_left == 0.0 && self.ammo < self.world.weapon.magazine {
            self.reload_left = self.world.weapon.reload_seconds;
        }
        let f = Vec3::new(self.yaw.sin(), 0.0, -self.yaw.cos());
        let r = Vec3::new(self.yaw.cos(), 0.0, self.yaw.sin());
        let dir = (f * input.forward + r * input.right).normalize_or_zero();
        if input.jump && self.grounded {
            self.velocity_y = self.world.player.jump_speed;
        }
        self.velocity_y = (self.velocity_y - 18.0 * dt).max(-30.0);
        let speed = self.world.player.speed
            * if input.sprint {
                self.world.player.sprint_multiplier
            } else {
                1.0
            };
        let (delta, grounded) = self.move_capsule(
            self.player_collider,
            dir * speed * dt + Vec3::Y * self.velocity_y * dt,
            dt,
        );
        self.position += delta;
        self.grounded = grounded;
        if grounded && self.velocity_y < 0.0 {
            self.velocity_y = 0.0;
        }
        self.colliders[self.player_collider].set_translation(vector![
            self.position.x,
            self.position.y,
            self.position.z
        ]);
        self.query.update(&self.colliders);
        if input.fire && self.fire_cooldown == 0.0 && self.reload_left == 0.0 {
            if self.ammo > 0 {
                self.fire();
            } else {
                self.reload_left = self.world.weapon.reload_seconds;
            }
        }
        for i in 0..self.actors.positions.len() {
            if self.actors.health[i] <= 0.0 {
                continue;
            }
            let offset = self.position - self.actors.positions[i];
            let distance = offset.length();
            self.actors.states[i] =
                self.world
                    .behavior
                    .step(self.actors.states[i], distance, self.actors.health[i]);
            if self.actors.states[i] == State::Dead {
                self.actors.health[i] = 0.0;
                self.colliders[self.actors.colliders[i]].set_enabled(false);
                continue;
            }
            self.actors.cooldowns[i] = (self.actors.cooldowns[i] - dt).max(0.0);
            match self.actors.states[i] {
                State::Chase => {
                    let dir = Vec3::new(offset.x, 0.0, offset.z).normalize_or_zero();
                    let mut best = Vec3::ZERO;
                    let mut score = f32::MIN;
                    // Local obstacle avoidance. Deterministic candidates; no path allocation.
                    for angle in [0.0_f32, 0.75, -0.75, 1.5, -1.5] {
                        let d = glam::Quat::from_rotation_y(angle) * dir;
                        let (move_by, _) = self.move_capsule(
                            self.actors.colliders[i],
                            d * self.settings[i].speed * dt - Vec3::Y * 0.04,
                            dt,
                        );
                        let quality = move_by.xz().length() * (1.0 - 0.18 * angle.abs());
                        if quality > score {
                            score = quality;
                            best = move_by;
                        }
                    }
                    self.actors.positions[i] += best;
                    let p = self.actors.positions[i];
                    self.colliders[self.actors.colliders[i]]
                        .set_translation(vector![p.x, p.y, p.z]);
                }
                State::Attack if self.actors.cooldowns[i] == 0.0 && distance < 2.5 => {
                    let start = self.actors.positions[i] + Vec3::Y * 0.3;
                    let end = self.position + Vec3::Y * 0.3;
                    let d = (end - start).normalize_or_zero();
                    let hit = self.query.cast_ray(
                        &self.bodies,
                        &self.colliders,
                        &Ray::new(point![start.x, start.y, start.z], vector![d.x, d.y, d.z]),
                        distance + 0.5,
                        true,
                        QueryFilter::default().exclude_collider(self.actors.colliders[i]),
                    );
                    if hit.is_some_and(|(handle, _)| handle == self.player_collider) {
                        self.health = (self.health - self.settings[i].damage).max(0.0);
                        self.damage_flash = 0.35;
                    }
                    self.actors.cooldowns[i] = self.world.behavior.attack_interval;
                }
                _ => {}
            }
        }
        self.query.update(&self.colliders);
    }

    fn fire(&mut self) {
        self.ammo -= 1;
        self.shots += 1;
        self.muzzle = 0.065;
        self.fire_cooldown = self.world.weapon.fire_interval;
        let start = self.eye();
        let d = self.forward();
        let mut end = start + d * self.world.weapon.range;
        if let Some((handle, toi)) = self.query.cast_ray(
            &self.bodies,
            &self.colliders,
            &Ray::new(point![start.x, start.y, start.z], vector![d.x, d.y, d.z]),
            self.world.weapon.range,
            true,
            QueryFilter::default().exclude_collider(self.player_collider),
        ) {
            end = start + d * toi;
            if let Some(i) = self.actors.colliders.iter().position(|&h| h == handle) {
                let headshot = end.y > self.actors.positions[i].y + 0.4;
                self.actors.health[i] -=
                    self.world.weapon.damage * if headshot { 1.8 } else { 1.0 };
                self.hits += 1;
                self.hit_flash = 0.16;
                if self.actors.health[i] <= 0.0 {
                    self.actors.states[i] = State::Dead;
                    self.kills += 1;
                    self.colliders[handle].set_enabled(false);
                    self.query.update(&self.colliders);
                }
            }
        }
        self.tracers
            .push((start + d * 0.6 + Vec3::new(0.12, -0.13, 0.0), end, 0.06));
    }
    pub fn observation(&self) -> Observation {
        Observation {
            rev: self.rev,
            tick: self.tick,
            elapsed_seconds: self.tick as f32 * FIXED_DT,
            player_position: self.position.to_array(),
            player_health: self.health,
            enemies_alive: self.alive(),
            enemies_total: self.actors.health.len(),
            kills: self.kills,
            shots: self.shots,
            hits: self.hits,
            ammo: self.ammo,
            victory: self.victory(),
            defeated: self.health <= 0.0,
            frame_ms_cpu: 0.0,
            fps: 0.0,
            gpu: String::new(),
        }
    }
}
use glam::Vec3Swizzles;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deterministic_fixed_step() {
        let mut a = Simulation::new(World::demo(), 1).unwrap();
        let mut b = Simulation::new(World::demo(), 1).unwrap();
        for _ in 0..600 {
            let input = Input {
                forward: 1.0,
                fire: true,
                ..Default::default()
            };
            a.step(input);
            b.step(input);
        }
        assert_eq!(a.position, b.position);
        assert_eq!(a.actors.positions, b.actors.positions);
        assert_eq!(a.ammo, b.ammo);
        assert_eq!(a.health, b.health);
    }
    #[test]
    fn floor_and_walls_are_solid() {
        let mut world = World::demo();
        world.enemies.count = 0;
        let mut sim = Simulation::new(world, 1).unwrap();
        sim.yaw = std::f32::consts::FRAC_PI_2;
        for _ in 0..1800 {
            sim.step(Input {
                forward: 1.0,
                ..Default::default()
            });
        }
        assert!(sim.position.x < 21.7);
        assert!(sim.position.x > 21.0);
        assert!((sim.position.y - 0.9).abs() < 0.1);
    }
    #[test]
    fn hitscan_kills_and_cover_blocks() {
        let mut world = World::demo();
        world.enemies.count = 1;
        world.enemies.speed = 0.0;
        world.player.spawn = [0.0, 0.9, -12.0];
        let mut sim = Simulation::new(world.clone(), 1).unwrap();
        sim.pitch = -0.03;
        assert!(sim.actor_visible(0));
        for _ in 0..100 {
            sim.step(Input {
                fire: true,
                ..Default::default()
            });
        }
        assert_eq!(sim.kills, 1);
        assert!(sim.hits >= 2);
        world.player.spawn = [0.0, 0.9, 12.0];
        let mut blocked = Simulation::new(world, 1).unwrap();
        blocked.pitch = 0.0;
        assert!(!blocked.actor_visible(0));
        for _ in 0..100 {
            blocked.step(Input {
                fire: true,
                ..Default::default()
            });
        }
        assert_eq!(blocked.hits, 0);
    }
    #[test]
    fn reload_and_reconcile_preserve_runtime() {
        let mut world = World::demo();
        world.enemies.speed = 0.0;
        world.weapon.magazine = 2;
        let mut sim = Simulation::new(world.clone(), 1).unwrap();
        for _ in 0..40 {
            sim.step(Input {
                fire: true,
                ..Default::default()
            });
        }
        assert!(sim.reload_left > 0.0);
        for _ in 0..240 {
            sim.step(Input::default());
        }
        assert_eq!(sim.ammo, 2);
        sim.actors.health[0] = 20.0;
        let id = sim.actors.ids[0];
        world.enemies.count = 12;
        sim.reconcile(world.clone(), 2).unwrap();
        assert_eq!(sim.actors.health[0], 20.0);
        assert_eq!(sim.actors.ids[0], id);
        assert_eq!(sim.alive(), 12);
        let removed = sim.actors.ids[11];
        world.enemies.count = 8;
        sim.reconcile(world.clone(), 3).unwrap();
        world.enemies.count = 12;
        sim.reconcile(world, 4).unwrap();
        assert_ne!(removed, sim.actors.ids[11]);
    }
}
