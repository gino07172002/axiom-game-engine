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
            axiom_core::package::merge_resource(a, b, &["assets", "clips", "profiles"], &[])
        });
    r.capabilities.insert("animation".into(), 1);
    r.resource("animation.library","animation",json!({"assets":"stable ID -> {path, external:false, options:{coordinates?, strict:false}}","clips":"stable ID -> {id,duration,tracks}","profiles":"stable ID -> {mapping:HumanBodyBones->bone ID,reference:bone ID->local TRS}"}),|v|{let lib:Library=serde_json::from_value(v.clone())?;ensure!(lib.assets.len()<=32&&lib.clips.len()<=128&&lib.profiles.len()<=64,"animation library budget exceeded");for id in lib.assets.keys().chain(lib.clips.keys()).chain(lib.profiles.keys()){axiom_core::valid_id(id)?;}for source in lib.assets.values(){ensure!(!source.path.as_os_str().is_empty(),"asset path missing");if let Some(c)=source.options.coordinates{c.matrix()?;}}Ok(())});
    r.component("animation.player","animation",json!({"asset":"library asset ID","clip":"optional clip ID","retarget":"optional {source_asset,source_profile,target_profile,root_motion,require_complete}","speed":"-8..8","looping":"bool","playing":"bool","skinning":"auto|linear|dual_quaternion","twists":"stable constraint ID -> {enabled,driver,axis,bones:[{bone,fraction}],driver_fraction,min_angle,max_angle,preserve}","ik":"stable constraint ID -> {root,middle,end,target,pole,weight,end_rotation?}","debug_skeleton":"bool"}),|v|serde_json::from_value::<Player>(v.clone())?.validate());
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
                values
                    .get("animation.player")
                    .cloned()
                    .map(|v| (e.id.clone(), v, values.contains_key("core.transform")))
            })
            .collect::<Vec<_>>();
        ensure!(
            compiled_players.len() <= 32 && w.systems.iter().any(|s| s == "animation.evaluate"),
            "animation system missing or player budget exceeded"
        );
        for (id, value, transform) in compiled_players {
            let p: Player = serde_json::from_value(value)?;
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
    r.topics.insert("animation".into(),json!({"formats":["pmx 2.0/2.1","glTF 2.0","GLB"],"interchange":{"rigify":"bake evaluated DEF deform skeleton to glTF; controllers/constraints remain in Blender","unity":"Humanoid semantic mapping + baked bone TRS through the Unity export bridge; not binary Avatar or muscle-curve execution"},"coordinates":["axiom","gltf","blender","unity","mmd","unreal","custom signed orthonormal basis + meters_per_unit"],"humanoid_roles":humanoid::roles(),"actions":["play","seek","ik","twist","bone","inspect"],"limits":{"players":32,"bones_per_asset":MAX_BONES,"vertices_per_asset":MAX_VERTICES,"influences":4,"ik_per_player":32},"pipeline":["sample/blend","reference-pose retarget","bone overrides","twist correction","two-bone IK","skinning"],"skinning":"CPU LBS / SDEF / dual quaternion, immutable textured triangle snapshots","checkpoint":"native animation state checkpoint is not yet supported"}));
}
pub fn component_path(entity: &str) -> String {
    format!(
        "/entities/@{}/components/animation.player",
        axiom_core::local::escape(entity)
    )
}
