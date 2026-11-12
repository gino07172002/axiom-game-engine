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
    /// Primitives hidden at start: material IDs or glTF mesh / material / node names
    /// (`animation.visibility` changes them at runtime).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<String>,
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
            hidden: vec![],
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
        ensure!(
            self.hidden.len() <= 64 && self.hidden.iter().all(|g| !g.is_empty() && g.len() <= 256),
            "hidden: at most 64 part names of 1..256 characters"
        );
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
    pub target: Vec3,
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
/// `animation.morph`: overrides one morph target's weight (by `target` name or `index`), at
/// once or ramped over `ramp_seconds`, without touching playback; `clear` hands the target back
/// to the clips (or its default weight).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorphArgs {
    pub entity: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub weight: Option<f32>,
    #[serde(default)]
    pub ramp_seconds: f32,
    #[serde(default)]
    pub clear: bool,
}
impl MorphArgs {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.target.is_some() != self.index.is_some(),
            "pass exactly one of target (name) or index"
        );
        ensure!(
            self.clear != self.weight.is_some(),
            "pass weight (-2..2) or clear:true"
        );
        ensure!(
            !self.clear || self.ramp_seconds == 0.0,
            "clear hands the target back at once (ramp to a weight first, then clear)"
        );
        ensure!(
            self.weight
                .is_none_or(|w| w.is_finite() && (-2.0..=2.0).contains(&w))
                && self.ramp_seconds.is_finite()
                && (0.0..=60.0).contains(&self.ramp_seconds),
            "invalid morph weight / ramp"
        );
        Ok(())
    }
}
/// `animation.visibility`: shows or hides the primitives `group` names (a material ID or a glTF
/// mesh / material / node name).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisibilityArgs {
    pub entity: String,
    pub group: String,
    pub visible: bool,
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
    r.resource("animation.library","animation",json!({"assets":"stable ID -> {path, external:false, options:{coordinates?, strict:false}}","clips":"stable ID -> {id,duration,tracks}","profiles":"stable ID -> {mapping:HumanBodyBones->bone ID,reference:bone ID->local TRS}","markers":"clip ID -> [{at:seconds, event?:behavior.program event, cue?:audio cue}] (fired once per loop iteration)"}),|v|{let lib:Library=serde_json::from_value(v.clone())?;ensure!(lib.assets.len()<=32&&lib.clips.len()<=128&&lib.profiles.len()<=64,"animation library budget exceeded");lib.validate_markers()?;for id in lib.assets.keys().chain(lib.clips.keys()).chain(lib.profiles.keys()){axiom_core::valid_id(id)?;}for source in lib.assets.values(){ensure!(!source.path.as_os_str().is_empty(),"asset path missing");if let Some(c)=source.options.coordinates{c.matrix()?;}}Ok(())});
    r.component("animation.player","animation",json!({"asset":"library asset ID","clip":"optional clip ID","retarget":"optional {source_asset,source_profile,target_profile,root_motion,require_complete}","speed":"-8..8","looping":"bool","playing":"bool","skinning":"auto|linear|dual_quaternion","twists":"stable constraint ID -> {enabled,driver,axis,bones:[{bone,fraction}],driver_fraction,min_angle,max_angle,preserve}","ik":"stable constraint ID -> {root,middle,end,target,pole,weight,end_rotation?}","debug_skeleton":"bool","on_end":"optional behavior.program event emitted when the clip completes (each loop)","hidden":"optional primitives hidden at start: material IDs or glTF mesh / material / node names"}),|v|serde_json::from_value::<Player>(v.clone())?.validate());
    r.resource_mergers
        .insert("animation.controllers".into(), |a, b| {
            axiom_core::package::merge_resource(a, b, &["controllers"], &[])
        });
    r.resource(
        "animation.controllers",
        "animation",
        json!({"controllers":"stable ID -> {parameters:{NAME:{kind:float|bool|trigger,default?,source?:speed}},initial,states:{NAME:{motion?:{kind:clip,clip,speed?,looping?}|{kind:blend1d,parameter,points:[{at,clip}],speed?,looping?}}},transitions:[{from:STATE|*,to,conditions:[{param,op:gt|lt|eq|ne|true|false|trigger,value?}],duration:seconds,exit_time?:normalized}],layers?:[{name,clip,mode?:override|additive,weight?:0..1,weight_parameter?,mask?:{bones?:[BONE],subtrees?:[BONE]},speed?,looping?}]}"}),
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
        "animation.morph",
        "animation",
        json!({"entity":"stable ID","target":"morph target name (or index)","index":"morph target index (or target)","weight":"-2..2 (usually 0..1); overrides clips until cleared","ramp_seconds":"0..60, default 0 = at once","clear":"true hands the target back to clips / its default"}),
        |v| serde_json::from_value::<MorphArgs>(v.clone())?.validate(),
    );
    r.action(
        "animation.visibility",
        "animation",
        json!({"entity":"stable ID","group":"primitive material ID or glTF mesh / material / node name","visible":"bool"}),
        |v| {
            let a: VisibilityArgs = serde_json::from_value(v.clone())?;
            ensure!(
                !a.group.is_empty() && a.group.len() <= 256,
                "group must be 1..256 characters"
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
    r.action("animation.ik","animation",json!({"entity":"stable ID","id":"constraint ID","target":"[x,y,z] model meters","pole":"optional [x,y,z]","weight":"optional 0..1"}),|v|{let a:IkArgs=serde_json::from_value(v.clone())?;ensure!(a.target.is_finite()&&a.pole.is_none_or(|p|p.is_finite())&&a.weight.is_none_or(|w|w.is_finite()&&(0.0..=1.).contains(&w)),"invalid IK target");Ok(())});
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
    r.topics.insert("animation".into(),json!({"formats":["pmx 2.0/2.1","glTF 2.0","GLB"],"interchange":{"rigify":"bake evaluated DEF deform skeleton to glTF; controllers/constraints remain in Blender","unity":"Humanoid semantic mapping + baked bone TRS through the Unity export bridge; not binary Avatar or muscle-curve execution"},"coordinates":["axiom","gltf","blender","unity","mmd","unreal","custom signed orthonormal basis + meters_per_unit"],"humanoid_roles":humanoid::roles(),"actions":["play","seek","ik","twist","bone","inspect","param","speed","morph","visibility"],"morphs":"glTF morph targets (names from extras.targetNames), weight channels become clip morph tracks; animation.morph overrides a weight (ramped, playback kept); at most 64 targets per mesh, 128 per asset, 1,000,000 deltas","parts":"animation.player.hidden + animation.visibility show / hide primitives by material ID or glTF mesh / material / node name","layers":"controller layers: one clip each, override or additive (relative to frame 0), weight (x weight_parameter), bone mask (bones / subtrees), applied in order after the states blend","controller":"animation.controllers resource + animation.controller component: float/bool/trigger parameters (float source speed = planar m/s), states play a clip or a 1D blend tree, ordered transitions with conditions, exit_time and cross-fade duration; set parameters with animation.param","limits":{"players":32,"bones_per_asset":MAX_BONES,"vertices_per_asset":MAX_VERTICES,"influences":4,"ik_per_player":32},"pipeline":["sample/blend","reference-pose retarget","bone overrides","twist correction","two-bone IK","skinning"],"skinning":"CPU LBS / SDEF / dual quaternion, immutable textured triangle snapshots","checkpoint":"playback (clip, time, speed, ramp, fade, overrides, IK targets, twist toggles) and controller state are saved in checkpoints (Checkpoint.systems)"}));
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
