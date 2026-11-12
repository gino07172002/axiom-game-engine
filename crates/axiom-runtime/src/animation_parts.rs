//! `animation.parts`: clickable capsules on an animated character's bones (authored, or
//! derived from the skeleton with a radius). A primary click without a drag casts the
//! generic pointer ray against every capsule at the current pose and reports the nearest
//! hit (entity + part ID) as a `behavior.program` event; `animation.pick` answers the same
//! query for a given ray. Poses are only read at a click (no per-step cost); capsule counts
//! are bounded.
use super::*;
use axiom_core::gameplay::ScalarType;
use axiom_core::Registry;

pub(super) const COMPONENT: &str = "animation.parts";
/// Capsules per entity and per world.
pub(super) const MAX_PARTS: usize = 128;
pub(super) const MAX_WORLD_PARTS: usize = 512;
/// A press and release farther apart than this (fraction of the view) is a drag, not a click.
const CLICK_SLOP: f32 = 0.01;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct PartsDef {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    parts: Vec<PartDef>,
    /// Also one capsule per bone toward each child joint (part ID = the bone), this radius.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_radius: Option<f32>,
    /// `behavior.program` event emitted on a click (text fields `entity` / `part` filled
    /// when declared).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    event: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct PartDef {
    id: String,
    bone: String,
    radius: f32,
    /// Capsule end at this bone's joint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to: Option<String>,
    /// Capsule end this far along the bone's +Y (meters, model space).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    length: Option<f32>,
}

impl PartsDef {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.parts.is_empty() || self.auto_radius.is_some(),
            "animation.parts needs parts and / or auto_radius"
        );
        ensure!(self.parts.len() <= MAX_PARTS, "at most {MAX_PARTS} parts");
        let radius = |r: f32| r.is_finite() && (0.005..=2.0).contains(&r);
        if let Some(r) = self.auto_radius {
            ensure!(radius(r), "auto_radius must be 0.005..2 meters");
        }
        if let Some(e) = &self.event {
            axiom_core::valid_id(e)?;
        }
        let mut ids = std::collections::BTreeSet::new();
        for p in &self.parts {
            axiom_core::valid_id(&p.id)?;
            axiom_core::valid_id(&p.bone)?;
            ensure!(ids.insert(&p.id), "duplicate part {}", p.id);
            ensure!(
                radius(p.radius),
                "part {}: radius must be 0.005..2 meters",
                p.id
            );
            ensure!(
                p.to.is_none() || p.length.is_none(),
                "part {}: give to or length, not both",
                p.id
            );
            if let Some(t) = &p.to {
                axiom_core::valid_id(t)?;
            }
            if let Some(l) = p.length {
                ensure!(
                    l.is_finite() && (0.0..=10.0).contains(&l),
                    "part {}: length must be 0..10 meters",
                    p.id
                );
            }
        }
        Ok(())
    }
}

/// One capsule: bone joint `a` to joint `b`, or `length` along `a`'s +Y.
struct Capsule {
    id: Arc<str>,
    a: usize,
    b: Option<usize>,
    length: f32,
    radius: f32,
}

struct Target {
    entity: String,
    capsules: Vec<Capsule>,
    /// Index into `Parts::events`.
    event: Option<usize>,
}

/// An event a click emits, with the text fields it fills.
struct PickEvent {
    name: String,
    entity: bool,
    part: bool,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Hit {
    entity: String,
    part: String,
    /// Meters along the ray.
    distance: f32,
}

/// Checkpoint form of the click counters (only for worlds with parts).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedPicks {
    hits: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last: Option<Hit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fault: Option<String>,
}

#[derive(Default)]
pub(super) struct Parts {
    targets: Vec<Target>,
    events: Vec<PickEvent>,
    /// Where the primary button went down (a release near it is a click).
    press: Option<[f32; 2]>,
    hits: u64,
    last: Option<Hit>,
    /// Last event that could not be delivered (recoverable).
    fault: Option<String>,
}

pub(super) fn register(r: &mut Registry) {
    r.component(
        COMPONENT,
        "animation",
        json!({"parts":"[{id, bone, radius 0.005..2 m, to?: bone (capsule to that joint) | length?: m along the bone's +Y (neither = sphere)}] up to 128","auto_radius":"optional meters: also one capsule per bone toward each child joint (part ID = bone name)","event":"optional behavior.program event emitted on a click (text fields entity / part filled when declared)","runtime":"a primary click (press and release without a drag) casts the pointer ray at the current pose; nearest capsule wins; observe animation.pick; action animation.pick {origin, direction} answers the same query"}),
        |v| serde_json::from_value::<PartsDef>(v.clone())?.validate(),
    );
    r.action(
        "animation.pick",
        "animation",
        json!({"origin":"[x,y,z] world meters","direction":"[x,y,z] (normalized here)","result":"nearest {entity, part, distance} or null; changes nothing"}),
        |v| {
            ray_arg(v)?;
            Ok(())
        },
    );
}

fn ray_arg(v: &Value) -> Result<(Vec3, Vec3)> {
    let vec = |k: &str| -> Result<Vec3> {
        let a: [f32; 3] = serde_json::from_value(
            v.get(k)
                .cloned()
                .with_context(|| format!("animation.pick needs {k}: [x,y,z]"))?,
        )?;
        let v = Vec3::from_array(a);
        ensure!(v.is_finite(), "{k} must be finite");
        Ok(v)
    };
    let (o, d) = (vec("origin")?, vec("direction")?);
    ensure!(d.length_squared() > 1e-12, "direction must not be zero");
    Ok((o, d.normalize()))
}

impl Parts {
    /// Compiles every `animation.parts` against its player's skeleton.
    pub(super) fn build(
        state: &RuntimeState,
        players: &BTreeMap<String, Controller>,
    ) -> Result<Self> {
        let mut out = Self::default();
        let mut total = 0usize;
        let program = state.compiled.source.resources.get("behavior.program");
        for e in &state.compiled.entities {
            let Some(v) = e.components.get(COMPONENT) else {
                continue;
            };
            let path = format!(
                "/entities/@{}/components/{COMPONENT}",
                axiom_core::local::escape(&e.id)
            );
            let fail = |m: String| axiom_core::Diagnostic::error("invalid_parts", &path, m);
            let def: PartsDef =
                serde_json::from_value(v.clone()).map_err(|e| fail(e.to_string()))?;
            def.validate().map_err(|e| fail(format!("{e:#}")))?;
            let p = players
                .get(&e.id)
                .ok_or_else(|| fail(format!("{} has no animation.player", e.id)))?;
            let skeleton = &p.asset.skeleton;
            let bone = |name: &str| {
                skeleton
                    .indices
                    .get(name)
                    .copied()
                    .ok_or_else(|| fail(format!("{} has no bone {name}", e.id)))
            };
            let mut capsules = vec![];
            for d in &def.parts {
                capsules.push(Capsule {
                    id: d.id.as_str().into(),
                    a: bone(&d.bone)?,
                    b: d.to.as_deref().map(bone).transpose()?,
                    length: d.length.unwrap_or(0.0),
                    radius: d.radius,
                });
            }
            if let Some(radius) = def.auto_radius {
                for (i, b) in skeleton.source.bones.iter().enumerate() {
                    if let Some(parent) = b.parent {
                        capsules.push(Capsule {
                            id: skeleton.source.bones[parent].id.as_str().into(),
                            a: parent,
                            b: Some(i),
                            length: 0.0,
                            radius,
                        });
                    }
                }
            }
            if capsules.len() > MAX_PARTS {
                return Err(fail(format!(
                    "{} capsules (at most {MAX_PARTS}; auto_radius adds one per bone)",
                    capsules.len()
                )));
            }
            total += capsules.len();
            if total > MAX_WORLD_PARTS {
                return Err(fail(format!(
                    "more than {MAX_WORLD_PARTS} part capsules in the world"
                )));
            }
            let event = match &def.event {
                Some(name) => Some(
                    out.event(program, name)
                        .map_err(|e| fail(format!("{e:#}")))?,
                ),
                None => None,
            };
            out.targets.push(Target {
                entity: e.id.clone(),
                capsules,
                event,
            });
        }
        if out.targets.iter().any(|t| t.event.is_some()) {
            ensure!(
                state
                    .compiled
                    .source
                    .systems
                    .iter()
                    .any(|s| s == "behavior.execute"),
                "animation.parts events need the behavior.execute system"
            );
        }
        Ok(out)
    }

    fn event(&mut self, program: Option<&Value>, name: &str) -> Result<usize> {
        if let Some(i) = self.events.iter().position(|e| e.name == name) {
            return Ok(i);
        }
        let fields = program
            .and_then(|p| {
                p.pointer(&format!(
                    "/events/{}/fields",
                    axiom_core::local::escape(name)
                ))
            })
            .and_then(Value::as_object)
            .with_context(|| format!("event {name} is not declared in behavior.program"))?;
        for (k, t) in fields {
            ensure!(
                (k == "entity" || k == "part")
                    && serde_json::from_value::<ScalarType>(t.clone()).ok()
                        == Some(ScalarType::Text),
                "event {name}: a click fills only text fields entity / part (field {k})"
            );
        }
        self.events.push(PickEvent {
            name: name.to_owned(),
            entity: fields.contains_key("entity"),
            part: fields.contains_key("part"),
        });
        Ok(self.events.len() - 1)
    }

    pub(super) fn any(&self) -> bool {
        !self.targets.is_empty()
    }

    pub(super) fn metrics(&self) -> Option<Value> {
        self.any().then(|| {
            json!({
                "targets": self.targets.len(),
                "capsules": self.targets.iter().map(|t| t.capsules.len()).sum::<usize>(),
                "hits": self.hits,
                "last": self.last,
                "fault": self.fault,
            })
        })
    }

    pub(super) fn save(&self) -> Option<SavedPicks> {
        self.any().then(|| SavedPicks {
            hits: self.hits,
            last: self.last.clone(),
            fault: self.fault.clone(),
        })
    }

    pub(super) fn load(&mut self, saved: Option<&SavedPicks>) -> Result<()> {
        match saved {
            Some(s) => {
                ensure!(
                    self.any(),
                    "checkpoint holds clicks on parts this world does not have"
                );
                self.hits = s.hits;
                self.last.clone_from(&s.last);
                self.fault.clone_from(&s.fault);
            }
            None => ensure!(
                !self.any(),
                "checkpoint lacks the animation.parts state (saved before parts existed)"
            ),
        }
        self.press = None;
        Ok(())
    }

    pub(super) fn transfer(&mut self, old: &Self) {
        self.hits = old.hits;
        self.last.clone_from(&old.last);
        self.fault.clone_from(&old.fault);
    }
}

/// Distance along the ray to a capsule (segment `a`-`b`, radius `r`), if it hits.
fn ray_capsule(o: Vec3, d: Vec3, a: Vec3, b: Vec3, r: f32) -> Option<f32> {
    // Closest points between the ray o + t d (t >= 0) and the segment a + s (b - a).
    let u = b - a;
    let w = o - a;
    let (uu, ud, uw, dw) = (u.dot(u), u.dot(d), u.dot(w), d.dot(w));
    let denom = uu - ud * ud;
    let (t, mut s) = if uu < 1e-12 {
        (-dw, 0.0)
    } else if denom.abs() < 1e-9 {
        (0.0, (uw / uu).clamp(0.0, 1.0))
    } else {
        let s = ((uw - ud * dw) / denom).clamp(0.0, 1.0);
        (ud * s - dw, s)
    };
    if t < 0.0 {
        // The ray starts past the closest approach: the segment point nearest the origin.
        s = if uu < 1e-12 {
            0.0
        } else {
            (uw / uu).clamp(0.0, 1.0)
        };
    }
    // With the segment point fixed, the best t again.
    let p = a + u * s;
    let t = (p - o).dot(d).max(0.0);
    let gap = (o + d * t - p).length();
    if gap > r {
        return None;
    }
    Some((t - (r * r - gap * gap).sqrt()).max(0.0))
}

impl AnimationSystem {
    /// The nearest part the ray (`direction` normalized) hits at the current poses.
    pub(super) fn pick(&self, state: &RuntimeState, origin: Vec3, direction: Vec3) -> Option<Hit> {
        if !self.parts.any() {
            return None;
        }
        let models = state.matrices();
        let mut best: Option<Hit> = None;
        for t in &self.parts.targets {
            let (Some(&slot), Some(p)) = (
                state.compiled.indices.get(&t.entity),
                self.players.get(&t.entity),
            ) else {
                continue;
            };
            if !state.compiled.enabled[slot] {
                continue;
            }
            let Ok(globals) = p.asset.skeleton.globals(&p.pose) else {
                continue;
            };
            let model = models[slot];
            for c in &t.capsules {
                let ga = model * globals[c.a];
                let a = ga.w_axis.truncate();
                let b = match c.b {
                    Some(b) => (model * globals[b]).w_axis.truncate(),
                    None => ga.transform_point3(Vec3::Y * c.length),
                };
                if let Some(d) = ray_capsule(origin, direction, a, b, c.radius) {
                    if best.as_ref().is_none_or(|h| d < h.distance) {
                        best = Some(Hit {
                            entity: t.entity.clone(),
                            part: c.id.to_string(),
                            distance: d,
                        });
                    }
                }
            }
        }
        best
    }

    pub(super) fn pick_command(&self, state: &RuntimeState, args: &Value) -> Result<Value> {
        let (o, d) = ray_arg(args)?;
        Ok(json!(self.pick(state, o, d)))
    }

    /// Primary press / release: a release near its press is a click on a part.
    pub(super) fn parts_pointer(&mut self, state: &mut RuntimeState, pointer: &ScreenPointer) {
        match pointer.kind {
            PointerKind::PrimaryDown => self.parts.press = Some(pointer.screen),
            PointerKind::PrimaryUp => {
                let Some(press) = self.parts.press.take() else {
                    return;
                };
                let moved = (pointer.screen[0] - press[0]).hypot(pointer.screen[1] - press[1]);
                if moved > CLICK_SLOP {
                    return;
                }
                let o = Vec3::from_array(pointer.origin);
                let d = Vec3::from_array(pointer.direction);
                let Some(hit) = self.pick(state, o, d) else {
                    return;
                };
                self.parts.hits += 1;
                let event = self
                    .parts
                    .targets
                    .iter()
                    .find(|t| t.entity == hit.entity)
                    .and_then(|t| t.event);
                if let Some(e) = event {
                    let info = &self.parts.events[e];
                    let mut fields = BTreeMap::new();
                    if info.entity {
                        fields.insert("entity".to_owned(), Scalar::Text(hit.entity.clone()));
                    }
                    if info.part {
                        fields.insert("part".to_owned(), Scalar::Text(hit.part.clone()));
                    }
                    let command = RuntimeCommand::Emit {
                        event: info.name.clone(),
                        fields,
                    };
                    let result = match state.gameplay.as_mut() {
                        Some(g) => g.command(&command, state.tick),
                        None => Err(anyhow::anyhow!("behavior.execute is not running")),
                    };
                    if let Err(err) = result {
                        self.parts.fault = Some(format!("emit {}: {err:#}", info.name));
                    }
                }
                self.parts.last = Some(hit);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ray_capsule;
    use glam::Vec3;

    #[test]
    fn ray_capsule_hits_misses_and_spheres() {
        let o = Vec3::new(0.0, 1.0, 5.0);
        let d = Vec3::NEG_Z;
        // Vertical capsule at the origin, radius 0.2: hit at z = 0.2 -> 4.8 m away.
        let t = ray_capsule(o, d, Vec3::ZERO, Vec3::Y * 2.0, 0.2).unwrap();
        assert!((t - 4.8).abs() < 1e-4, "{t}");
        // Off to the side.
        assert!(ray_capsule(o + Vec3::X, d, Vec3::ZERO, Vec3::Y * 2.0, 0.2).is_none());
        // Behind the origin.
        assert!(ray_capsule(o, Vec3::Z, Vec3::ZERO, Vec3::Y * 2.0, 0.2).is_none());
        // Degenerate segment = sphere.
        let t = ray_capsule(o, d, Vec3::Y, Vec3::Y, 0.5).unwrap();
        assert!((t - 4.5).abs() < 1e-4, "{t}");
        // Origin inside: distance 0.
        assert_eq!(
            ray_capsule(Vec3::Y, d, Vec3::ZERO, Vec3::Y * 2.0, 0.2),
            Some(0.0)
        );
        // Parallel ray along the capsule axis, inside the radius.
        let t = ray_capsule(
            Vec3::new(0.1, 5.0, 0.0),
            Vec3::NEG_Y,
            Vec3::ZERO,
            Vec3::Y * 2.0,
            0.2,
        )
        .unwrap();
        assert!(t > 2.8 && t < 3.0, "{t}");
    }
}
