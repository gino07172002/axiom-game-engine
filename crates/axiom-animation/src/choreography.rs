//! `animation.choreography`: named stages that start several actors on the same fixed step
//! (`animation.stage {stage}`), each actor with its own clip / playback rate / controller
//! parameters, optionally followed by another stage once one actor completed a number of
//! clip cycles (`after`). Pure data and validation; the runtime applies it atomically.
use crate::controller::ParamValue;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_STAGES: usize = 64;
pub const MAX_ACTORS: usize = 16;
pub const MAX_ACTOR_PARAMS: usize = 16;
pub const MAX_CYCLES: u32 = 1000;

/// The `animation.choreography` resource.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Choreography {
    #[serde(default)]
    pub stages: BTreeMap<String, Stage>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    pub actors: Vec<Actor>,
    /// Starts `next` once `entity` completed `cycles` cycles of its clip (one-shot: reached
    /// its end) after this stage started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<After>,
}

/// What one entity does when the stage starts: play `clip` (from 0, with `fade_seconds`,
/// `looping`, `speed`), or without a clip change its rate (`speed`, `ramp_seconds`; time is
/// kept); and set controller `params`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub entity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fade_seconds: f32,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub looping: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ramp_seconds: f32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ParamValue>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct After {
    pub entity: String,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub cycles: u32,
    pub next: String,
}

fn yes() -> bool {
    true
}
fn is_true(v: &bool) -> bool {
    *v
}
fn is_zero(v: &f32) -> bool {
    *v == 0.0
}
fn one() -> u32 {
    1
}
fn is_one(v: &u32) -> bool {
    *v == 1
}

impl Actor {
    pub fn validate(&self) -> Result<()> {
        axiom_core::valid_id(&self.entity)?;
        ensure!(
            self.clip.is_some() || self.speed.is_some() || !self.params.is_empty(),
            "actor {} does nothing: give it a clip, a speed and / or params",
            self.entity
        );
        if let Some(c) = &self.clip {
            axiom_core::valid_id(c)?;
        } else {
            ensure!(
                self.fade_seconds == 0.0 && self.looping,
                "actor {}: fade_seconds / looping go with a clip",
                self.entity
            );
        }
        ensure!(
            self.fade_seconds.is_finite() && (0.0..=10.0).contains(&self.fade_seconds),
            "actor {}: fade_seconds must be 0..10",
            self.entity
        );
        if let Some(s) = self.speed {
            ensure!(
                s.is_finite() && s.abs() <= 8.0,
                "actor {}: speed must be -8..8",
                self.entity
            );
        }
        ensure!(
            self.ramp_seconds.is_finite() && (0.0..=60.0).contains(&self.ramp_seconds),
            "actor {}: ramp_seconds must be 0..60",
            self.entity
        );
        ensure!(
            self.ramp_seconds == 0.0 || (self.speed.is_some() && self.clip.is_none()),
            "actor {}: ramp_seconds ramps a speed change without a clip",
            self.entity
        );
        ensure!(
            self.params.len() <= MAX_ACTOR_PARAMS,
            "actor {}: at most {MAX_ACTOR_PARAMS} params",
            self.entity
        );
        for (name, v) in &self.params {
            axiom_core::valid_id(name)?;
            if let ParamValue::Number(n) = v {
                ensure!(
                    n.is_finite(),
                    "actor {}: param {name} must be finite",
                    self.entity
                );
            }
        }
        Ok(())
    }
}

impl Choreography {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.stages.len() <= MAX_STAGES,
            "at most {MAX_STAGES} stages"
        );
        for (id, stage) in &self.stages {
            (|| -> Result<()> {
                axiom_core::valid_id(id)?;
                ensure!(
                    (1..=MAX_ACTORS).contains(&stage.actors.len()),
                    "a stage has 1..{MAX_ACTORS} actors"
                );
                let mut seen = BTreeSet::new();
                for a in &stage.actors {
                    ensure!(
                        seen.insert(&a.entity),
                        "entity {} appears twice (one entry per actor)",
                        a.entity
                    );
                    a.validate()?;
                }
                if let Some(after) = &stage.after {
                    axiom_core::valid_id(&after.entity)?;
                    ensure!(
                        (1..=MAX_CYCLES).contains(&after.cycles),
                        "after.cycles must be 1..{MAX_CYCLES}"
                    );
                    ensure!(
                        self.stages.contains_key(&after.next),
                        "after.next names unknown stage {}",
                        after.next
                    );
                }
                Ok(())
            })()
            .with_context(|| format!("stage {id}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stages_validate_and_round_trip() {
        let c: Choreography = serde_json::from_value(json!({"stages":{
            "intro":{"actors":[{"entity":"a","clip":"wave","looping":false},{"entity":"b","speed":2.0,"ramp_seconds":1.0}],"after":{"entity":"a","next":"idle"}},
            "idle":{"actors":[{"entity":"a","clip":"idle"},{"entity":"b","params":{"speed":0.0,"wave":true}}]}
        }}))
        .unwrap();
        c.validate().unwrap();
        let back: Choreography = serde_json::from_value(serde_json::to_value(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        // Defaults are not written out.
        assert!(
            serde_json::to_value(&c).unwrap()["stages"]["idle"]["actors"][0]
                .get("looping")
                .is_none()
        );
    }

    #[test]
    fn invalid_stages_are_refused_with_the_stage_named() {
        for (bad, says) in [
            (json!({"stages":{"s":{"actors":[]}}}), "1..16 actors"),
            (
                json!({"stages":{"s":{"actors":[{"entity":"a"}]}}}),
                "does nothing",
            ),
            (
                json!({"stages":{"s":{"actors":[{"entity":"a","clip":"x"},{"entity":"a","clip":"y"}]}}}),
                "appears twice",
            ),
            (
                json!({"stages":{"s":{"actors":[{"entity":"a","clip":"x"}],"after":{"entity":"a","next":"nope"}}}}),
                "unknown stage",
            ),
            (
                json!({"stages":{"s":{"actors":[{"entity":"a","clip":"x","ramp_seconds":1}]}}}),
                "ramp_seconds",
            ),
            (
                json!({"stages":{"s":{"actors":[{"entity":"a","speed":9}]}}}),
                "-8..8",
            ),
        ] {
            let c: Choreography = serde_json::from_value(bad).unwrap();
            let e = format!("{:#}", c.validate().unwrap_err());
            assert!(e.contains("stage s") && e.contains(says), "{e}");
        }
    }
}
