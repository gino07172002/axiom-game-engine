use crate::*;
use axiom_core::{Registry, SystemDescriptor};
use serde_json::json;
use std::path::PathBuf;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub path: PathBuf,
    #[serde(default)]
    pub external: bool,
    #[serde(default)]
    pub options: import::ImportOptions,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Library {
    pub assets: BTreeMap<String, Source>,
    pub clips: BTreeMap<String, clip::Clip>,
    pub profiles: BTreeMap<String, humanoid::Profile>,
    /// Clip ID -> markers fired when playback crosses them (once per loop iteration).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub markers: BTreeMap<String, Vec<Marker>>,
}
/// A point in a clip that emits a `behavior.program` event and / or plays an audio cue.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    /// Seconds into the clip.
    pub at: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cue: Option<String>,
}
pub const MAX_MARKERS_PER_CLIP: usize = 64;
impl Library {
    pub fn validate_markers(&self) -> Result<()> {
        ensure!(
            self.markers.len() <= 128,
            "animation marker clip budget exceeded"
        );
        for (clip, list) in &self.markers {
            axiom_core::valid_id(clip)?;
            ensure!(
                list.len() <= MAX_MARKERS_PER_CLIP,
                "clip {clip}: at most {MAX_MARKERS_PER_CLIP} markers"
            );
            for m in list {
                ensure!(
                    m.at.is_finite() && (0.0..=86400.0).contains(&m.at),
                    "clip {clip}: marker time 0..86400 s"
                );
                ensure!(
                    m.event.is_some() || m.cue.is_some(),
                    "clip {clip}: a marker needs an event or a cue"
                );
                for id in m.event.iter().chain(&m.cue) {
                    axiom_core::valid_id(id)?;
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Retarget {
    pub source_asset: String,
    pub source_profile: String,
    pub target_profile: String,
    #[serde(default)]
    pub root_motion: bool,
    #[serde(default = "yes")]
    pub require_complete: bool,
}
fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Player {
    pub asset: String,
    pub clip: Option<String>,
    pub retarget: Option<Retarget>,
    pub speed: f32,
    pub looping: bool,
    pub playing: bool,
    pub skinning: skin::Skinning,
    pub twists: BTreeMap<String, twist::Twist>,
    pub ik: BTreeMap<String, ik::TwoBoneIk>,
    pub debug_skeleton: bool,
    /// `behavior.program` event emitted each time the playing clip completes (a one-shot at
    /// its end, a loop at every wrap; a controller: its current state's clip).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_end: Option<String>,
}
impl Default for Player {
    fn default() -> Self {
        Self {
            asset: String::new(),
            clip: None,
            retarget: None,
            speed: 1.,
            looping: true,
            playing: true,
            skinning: skin::Skinning::Auto,
            twists: BTreeMap::new(),
            ik: BTreeMap::new(),
            debug_skeleton: false,
            on_end: None,
        }
    }
}
impl Player {
    pub fn validate(&self) -> Result<()> {
        axiom_core::valid_id(&self.asset)?;
        ensure!(
            self.speed.is_finite()
                && self.speed.abs() <= 8.
                && self.twists.len() <= 64
                && self.ik.len() <= 32,
            "animation controller budget exceeded"
        );
        for id in self.twists.keys().chain(self.ik.keys()) {
            axiom_core::valid_id(id)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayArgs {
    pub entity: String,
    pub clip: String,
    #[serde(default)]
    pub fade_seconds: f32,
    #[serde(default = "yes")]
    pub looping: bool,
    #[serde(default = "one")]
    pub speed: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeekArgs {
    pub entity: String,
    pub seconds: f32,
    #[serde(default)]
    pub paused: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IkArgs {
    pub entity: String,
    pub id: String,
    /// A fixed model-space target (unbinds a socket).
    #[serde(default)]
    pub target: Option<Vec3>,
    /// Binds the target to another entity's bone (resolved every step).
    #[serde(default)]
    pub socket: Option<ik::Socket>,
    #[serde(default)]
    pub pole: Option<Vec3>,
    #[serde(default)]
    pub weight: Option<f32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TwistArgs {
    pub entity: String,
    pub id: String,
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoneArgs {
    pub entity: String,
    pub bone: String,
    pub rotation: Quat,
}
/// `animation.speed`: changes the playback rate without resetting time, at once or ramped
/// linearly over `ramp_seconds` (a controller scales its tick by it).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeedArgs {
    pub entity: String,
    pub speed: f32,
    #[serde(default)]
    pub ramp_seconds: f32,
}
/// `animation.param`: sets one controller parameter of `entity`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamArgs {
    pub entity: String,
    pub name: String,
    pub value: controller::ParamValue,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectArgs {
    pub entity: String,
    #[serde(default)]
    pub bones: Vec<String>,
}
pub fn register(r: &mut Registry) {
    r.resource_mergers
        .insert("animation.library".into(), |a, b| {
            axiom_core::package::merge_resource(
                a,
                b,
                &["assets", "clips", "profiles", "markers"],
                &[],
            )
        });
    r.capabilities.insert("animation".into(), 1);
    rig::register(r);
    r.resource("animation.library","animation",json!({"assets":"stable ID -> {path, external:false, options:{coordinates?, strict:false}}","clips":"stable ID -> {id,duration,tracks}","profiles":"stable ID -> {mapping:HumanBodyBones->bone ID,reference:bone ID->local TRS}","markers":"clip ID -> [{at:seconds, event?:behavior.program event, cue?:audio cue}] (fired once per loop iteration)"}),|v|{let lib:Library=serde_json::from_value(v.clone())?;ensure!(lib.assets.len()<=32&&lib.clips.len()<=128&&lib.profiles.len()<=64,"animation library budget exceeded");lib.validate_markers()?;for id in lib.assets.keys().chain(lib.clips.keys()).chain(lib.profiles.keys()){axiom_core::valid_id(id)?;}for source in lib.assets.values(){ensure!(!source.path.as_os_str().is_empty(),"asset path missing");if let Some(c)=source.options.coordinates{c.matrix()?;}}Ok(())});
    r.component("animation.player","animation",json!({"asset":"library asset ID","clip":"optional clip ID","retarget":"optional {source_asset,source_profile,target_profile,root_motion,require_complete}","speed":"-8..8","looping":"bool","playing":"bool","skinning":"auto|linear|dual_quaternion","twists":"stable constraint ID -> {enabled,driver,axis,bones:[{bone,fraction}],driver_fraction,min_angle,max_angle,preserve}","ik":"stable constraint ID -> {root,middle,end,target,pole,weight,end_rotation?,socket?:{entity,bone?,offset?} (target follows that bone, one fixed step lag)}","debug_skeleton":"bool","on_end":"optional behavior.program event emitted when the clip completes (each loop)"}),|v|serde_json::from_value::<Player>(v.clone())?.validate());
    r.resource_mergers
        .insert("animation.controllers".into(), |a, b| {
            axiom_core::package::merge_resource(a, b, &["controllers"], &[])
        });
    r.resource(
        "animation.controllers",
        "animation",
        json!({"controllers":"stable ID -> {parameters:{NAME:{kind:float|bool|trigger,default?,source?:speed}},initial,states:{NAME:{motion?:{kind:clip,clip,speed?,looping?}|{kind:blend1d,parameter,points:[{at,clip}],speed?,looping?}}},transitions:[{from:STATE|*,to,conditions:[{param,op:gt|lt|eq|ne|true|false|trigger,value?}],duration:seconds,exit_time?:normalized}]}"}),
        |v| serde_json::from_value::<controller::Controllers>(v.clone())?.validate(),
    );
    r.component(
        "animation.controller",
        "animation",
        json!({"controller":"animation.controllers ID; drives the entity's animation.player"}),
        |v| {
            let a: controller::Attach = serde_json::from_value(v.clone())?;
            axiom_core::valid_id(&a.controller)
        },
    );
    r.action(
        "animation.speed",
        "animation",
        json!({"entity":"stable ID","speed":"-8..8 playback rate (time is kept)","ramp_seconds":"0..60, default 0 = at once"}),
        |v| {
            let a: SpeedArgs = serde_json::from_value(v.clone())?;
            ensure!(
                a.speed.is_finite()
                    && a.speed.abs() <= 8.
                    && a.ramp_seconds.is_finite()
                    && (0.0..=60.).contains(&a.ramp_seconds),
                "invalid speed / ramp"
            );
            Ok(())
        },
    );
    r.action(
        "animation.param",
        "animation",
        json!({"entity":"stable ID","name":"controller parameter","value":"number (float) or true/false (bool, trigger)"}),
        |v| {
            let a: ParamArgs = serde_json::from_value(v.clone())?;
            axiom_core::valid_id(&a.name)
        },
    );
    r.systems.insert(
        "animation.evaluate".into(),
        SystemDescriptor {
            id: "animation.evaluate".into(),
            capability: "animation".into(),
            phase: 30,
            after: vec![],
            reads: vec![
                "animation.library".into(),
                "animation.player".into(),
                "animation.controllers".into(),
                "animation.controller".into(),
                rig::SPRING.into(),
                rig::LOOK_AT.into(),
                "core.transform".into(),
            ],
            writes: vec!["animation.pose".into()],
        },
    );
    r.world_validators.push(|w| {
        if !w.capabilities.contains_key("animation") {
            return Ok(());
        }
        let lib: Library = w.resource("animation.library")?;
        let compiled_players = w
            .entities
            .iter()
            .filter_map(|e| {
                let mut values = e
                    .prefab
                    .as_ref()
                    .and_then(|p| w.prefabs.get(p))
                    .map(|p| p.components.clone())
                    .unwrap_or_default();
                for (k, v) in &e.components {
                    if let Some(old) = values.get_mut(k) {
                        axiom_core::merge_json(old, v)
                    } else {
                        values.insert(k.clone(), v.clone());
                    }
                }
                let controller = values.get("animation.controller").cloned();
                match values.get("animation.player").cloned() {
                    Some(v) => Some(Ok((
                        e.id.clone(),
                        v,
                        values.contains_key("core.transform"),
                        controller,
                    ))),
                    None => controller.map(|_| {
                        Err(anyhow::anyhow!(
                            "animation.controller on {} requires an animation.player",
                            e.id
                        ))
                    }),
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let controllers: controller::Controllers = w
            .resources
            .get("animation.controllers")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?
            .unwrap_or_default();
        ensure!(
            compiled_players.len() <= 32 && w.systems.iter().any(|s| s == "animation.evaluate"),
            "animation system missing or player budget exceeded"
        );
        for (id, value, transform, attach) in compiled_players {
            let p: Player = serde_json::from_value(value)?;
            if let Some(attach) = attach {
                let a: controller::Attach = serde_json::from_value(attach)?;
                ensure!(
                    controllers.controllers.contains_key(&a.controller),
                    "animation controller {} missing for {id} (animation.controllers)",
                    a.controller
                );
            }
            ensure!(transform, "animation entity {id} requires transform");
            ensure!(
                lib.assets.contains_key(&p.asset),
                "animation asset missing for {id}"
            );
            if let Some(rt) = p.retarget {
                ensure!(
                    lib.assets.contains_key(&rt.source_asset)
                        && lib.profiles.contains_key(&rt.source_profile)
                        && lib.profiles.contains_key(&rt.target_profile),
                    "retarget asset/profile missing for {id}"
                );
            }
        }
        Ok(())
    });
    r.package_resolvers.push(|root, w| {
        if let Some(value) = w.resources.get_mut("animation.library") {
            let mut lib: Library = serde_json::from_value(value.clone())?;
            for source in lib.assets.values_mut() {
                source.path = if source.external {
                    source.path.canonicalize()?
                } else {
                    let raw = source.path.to_str().context("asset path is not UTF-8")?;
                    import::confined(root, raw)?
                };
            }
            *value = serde_json::to_value(lib)?;
        }
        Ok(())
    });
    r.action("animation.play","animation",json!({"entity":"stable ID","clip":"clip ID","fade_seconds":"0..10, default 0","looping":"default true","speed":"-8..8, default 1"}),|v|{let a:PlayArgs=serde_json::from_value(v.clone())?;ensure!(a.fade_seconds.is_finite()&&(0.0..=10.0).contains(&a.fade_seconds)&&a.speed.is_finite()&&a.speed.abs()<=8.,"invalid playback parameters");Ok(())});
    r.action(
        "animation.seek",
        "animation",
        json!({"entity":"stable ID","seconds":"0..86400","paused":"default false"}),
        |v| {
            let a: SeekArgs = serde_json::from_value(v.clone())?;
            ensure!(
                a.seconds.is_finite() && (0.0..=86400.).contains(&a.seconds),
                "invalid seek"
            );
            Ok(())
        },
    );
    r.action("animation.ik","animation",json!({"entity":"stable ID","id":"constraint ID","target":"[x,y,z] model meters (a fixed target; unbinds a socket)","socket":"{entity, bone?, offset?}: follow another entity's bone (one fixed step lag)","pole":"optional [x,y,z]","weight":"optional 0..1"}),|v|{let a:IkArgs=serde_json::from_value(v.clone())?;ensure!(a.target.is_some()!=a.socket.is_some(),"give either target or socket");if let Some(s)=&a.socket{s.validate()?;}ensure!(a.target.is_none_or(|t|t.is_finite())&&a.pole.is_none_or(|p|p.is_finite())&&a.weight.is_none_or(|w|w.is_finite()&&(0.0..=1.).contains(&w)),"invalid IK target");Ok(())});
    r.action(
        "animation.twist",
        "animation",
        json!({"entity":"stable ID","id":"constraint ID","enabled":"bool"}),
        |v| {
            let _: TwistArgs = serde_json::from_value(v.clone())?;
            Ok(())
        },
    );
    r.action("animation.bone","animation",json!({"entity":"stable ID","bone":"bone ID","rotation":"normalized xyzw local quaternion override"}),|v|{let a:BoneArgs=serde_json::from_value(v.clone())?;ensure!(a.rotation.is_finite()&&(a.rotation.length_squared()-1.).abs()<0.001,"invalid bone rotation");Ok(())});
    r.action("animation.inspect","animation",json!({"entity":"stable ID","bones":"optional up to 64 stable IDs; omitted returns summary only"}),|v|{let a:InspectArgs=serde_json::from_value(v.clone())?;ensure!(a.bones.len()<=64,"bone inspection budget exceeded");Ok(())});
    r.topics.insert("animation".into(),json!({"formats":["pmx 2.0/2.1","glTF 2.0","GLB"],"interchange":{"rigify":"bake evaluated DEF deform skeleton to glTF; controllers/constraints remain in Blender","unity":"Humanoid semantic mapping + baked bone TRS through the Unity export bridge; not binary Avatar or muscle-curve execution"},"coordinates":["axiom","gltf","blender","unity","mmd","unreal","custom signed orthonormal basis + meters_per_unit"],"humanoid_roles":humanoid::roles(),"actions":["play","seek","ik","twist","bone","inspect","param","speed"],"controller":"animation.controllers resource + animation.controller component: float/bool/trigger parameters (float source speed = planar m/s), states play a clip or a 1D blend tree, ordered transitions with conditions, exit_time and cross-fade duration; set parameters with animation.param","limits":{"players":32,"bones_per_asset":MAX_BONES,"vertices_per_asset":MAX_VERTICES,"influences":4,"ik_per_player":32},"pipeline":["sample/blend","reference-pose retarget","bone overrides","twist correction","two-bone IK","look-at (animation.look_at)","spring bones (animation.spring)","skinning"],"secondary":"animation.spring chains with bone colliders and animation.look_at chains run at the fixed step after IK; IK sockets and look-at entity / bone targets read the end of the previous step (one step lag); their state is checkpointed","skinning":"CPU LBS / SDEF / dual quaternion, immutable textured triangle snapshots","checkpoint":"playback (clip, time, speed, ramp, fade, overrides, IK targets, twist toggles) and controller state are saved in checkpoints (Checkpoint.systems)"}));
}
pub fn component_path(entity: &str) -> String {
    format!(
        "/entities/@{}/components/animation.player",
        axiom_core::local::escape(entity)
    )
}
pub fn controller_path(entity: &str) -> String {
    format!(
        "/entities/@{}/components/animation.controller",
        axiom_core::local::escape(entity)
    )
}
