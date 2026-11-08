//! Semantic mapping and rest-pose calibrated, world-rotation retargeting.
use crate::*;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub mapping: BTreeMap<String, String>,
    /// Optional calibrated local reference transforms (e.g. T-pose on an A-pose mesh).
    pub reference: BTreeMap<String, Trs>,
}
pub fn roles() -> Vec<String> {
    let mut names = [
        "Hips",
        "Spine",
        "Chest",
        "UpperChest",
        "Neck",
        "Head",
        "Jaw",
        "LeftEye",
        "RightEye",
    ]
    .map(str::to_owned)
    .to_vec();
    for side in ["Left", "Right"] {
        for part in [
            "Shoulder", "UpperArm", "LowerArm", "Hand", "UpperLeg", "LowerLeg", "Foot", "Toes",
        ] {
            names.push(format!("{side}{part}"));
        }
        for finger in ["Thumb", "Index", "Middle", "Ring", "Little"] {
            for part in ["Proximal", "Intermediate", "Distal"] {
                names.push(format!("{side}{finger}{part}"));
            }
        }
    }
    names
}
pub fn required() -> Vec<String> {
    let mut out = vec!["Hips".into(), "Spine".into(), "Head".into()];
    for side in ["Left", "Right"] {
        for part in [
            "UpperArm", "LowerArm", "Hand", "UpperLeg", "LowerLeg", "Foot",
        ] {
            out.push(format!("{side}{part}"));
        }
    }
    out
}
fn normalize(s: &str) -> String {
    s.rsplit(':')
        .next()
        .unwrap_or(s)
        .chars()
        .map(|c| {
            if ('０'..='９').contains(&c) {
                char::from_u32(c as u32 - 0xfee0).unwrap()
            } else {
                c
            }
        })
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
pub fn aliases(role: &str) -> Vec<String> {
    let mut out = vec![role.to_owned()];
    let simple = match role {
        "Hips" => vec!["hips", "pelvis", "DEF-spine", "下半身"],
        "Spine" => vec!["spine", "spine1", "DEF-spine.001", "上半身"],
        "Chest" => vec!["chest", "spine2", "DEF-spine.002", "上半身2"],
        "UpperChest" => vec!["upperchest", "DEF-spine.003", "上半身3"],
        "Neck" => vec!["neck", "DEF-spine.004", "首"],
        "Head" => vec!["head", "DEF-spine.006", "頭"],
        "Jaw" => vec!["jaw", "顎"],
        "LeftEye" => vec!["eye.L", "DEF-eye.L", "左目"],
        "RightEye" => vec!["eye.R", "DEF-eye.R", "右目"],
        _ => vec![],
    };
    out.extend(simple.into_iter().map(str::to_owned));
    for (side, suffix, jp) in [("Left", "L", "左"), ("Right", "R", "右")] {
        if let Some(part) = role.strip_prefix(side) {
            let (rig, mmd, other) = match part {
                "Shoulder" => ("shoulder", "肩", "Shoulder"),
                "UpperArm" => ("upper_arm", "腕", "Arm"),
                "LowerArm" => ("forearm", "ひじ", "ForeArm"),
                "Hand" => ("hand", "手首", "Hand"),
                "UpperLeg" => ("thigh", "足", "UpLeg"),
                "LowerLeg" => ("shin", "ひざ", "Leg"),
                "Foot" => ("foot", "足首", "Foot"),
                "Toes" => ("toe", "つま先", "ToeBase"),
                _ => ("", "", ""),
            };
            if !rig.is_empty() {
                out.extend([
                    format!("DEF-{rig}.{suffix}"),
                    format!("{rig}.{suffix}"),
                    format!("{jp}{mmd}"),
                    format!("{side}{other}"),
                ]);
            }
            for (finger, rig, japanese) in [
                ("Thumb", "thumb", "親指"),
                ("Index", "f_index", "人指"),
                ("Middle", "f_middle", "中指"),
                ("Ring", "f_ring", "薬指"),
                ("Little", "f_pinky", "小指"),
            ] {
                for (segment, n) in [("Proximal", 1), ("Intermediate", 2), ("Distal", 3)] {
                    if part == format!("{finger}{segment}") {
                        out.push(format!("DEF-{rig}.0{n}.{suffix}"));
                        out.push(format!("{rig}.0{n}.{suffix}"));
                        out.push(format!("{jp}{japanese}{n}"));
                        out.push(format!("{side}Hand{finger}{n}"));
                    }
                }
            }
        }
    }
    out
}
pub fn auto_map(s: &CompiledSkeleton) -> (Profile, serde_json::Value) {
    let mut profile = Profile::default();
    let mut ambiguous = BTreeMap::new();
    for role in roles() {
        let names = aliases(&role)
            .iter()
            .map(|n| normalize(n))
            .collect::<Vec<_>>();
        let candidates = s
            .source
            .bones
            .iter()
            .filter(|b| {
                !b.id.starts_with("ORG-")
                    && !b.id.starts_with("MCH-")
                    && (names.contains(&normalize(&b.id))
                        || b.aliases.iter().any(|a| names.contains(&normalize(a))))
            })
            .map(|b| b.id.clone())
            .collect::<Vec<_>>();
        match candidates.as_slice() {
            [id] => {
                profile.mapping.insert(role, id.clone());
            }
            [] => {}
            _ => {
                ambiguous.insert(role, candidates);
            }
        }
    }
    // MMD commonly splits upper/lower body below a pelvis controller rather than parenting spine to lower body.
    if let (Some(&pelvis), Some(spine)) = (s.indices.get("腰"), profile.mapping.get("Spine")) {
        if ancestor(s, pelvis, s.indices[spine]) {
            profile.mapping.insert("Hips".into(), "腰".into());
        }
    }
    if let (Some(chest), Some(upper)) = (
        profile.mapping.get("Chest").cloned(),
        profile.mapping.get("UpperChest").cloned(),
    ) {
        if ancestor(s, s.indices[&upper], s.indices[&chest]) {
            profile.mapping.insert("Chest".into(), upper);
            profile.mapping.insert("UpperChest".into(), chest);
        }
    }
    for (side, jp) in [("Left", "左"), ("Right", "右")] {
        // A PMX leg can have an IK control chain and a parallel deform chain.
        let leg_roles = ["UpperLeg", "LowerLeg", "Foot"].map(|part| format!("{side}{part}"));
        let deform_names = ["足D", "ひざD", "足首D"].map(|part| format!("{jp}{part}"));
        let deform_chain = leg_roles
            .iter()
            .zip(&deform_names)
            .map(|(role, name)| {
                let original = *s.indices.get(profile.mapping.get(role)?)?;
                let deform = *s.indices.get(name)?;
                (s.rest_global[original]
                    .w_axis
                    .distance(s.rest_global[deform].w_axis)
                    < 0.001)
                    .then_some(deform)
            })
            .collect::<Option<Vec<_>>>();
        if let Some(chain) = deform_chain
            .filter(|chain| ancestor(s, chain[0], chain[1]) && ancestor(s, chain[1], chain[2]))
        {
            for (role, name) in leg_roles.into_iter().zip(deform_names) {
                profile.mapping.insert(role, name);
            }
            let toe = format!("{jp}足先EX");
            if s.indices
                .get(&toe)
                .is_some_and(|&i| ancestor(s, chain[2], i))
            {
                profile.mapping.insert(format!("{side}Toes"), toe);
            }
        }
        if s.source
            .bones
            .iter()
            .any(|b| normalize(&b.id) == normalize(&format!("{jp}親指0")))
        {
            for (part, n) in [("Proximal", 0), ("Intermediate", 1), ("Distal", 2)] {
                if let Some(b) = s
                    .source
                    .bones
                    .iter()
                    .find(|b| normalize(&b.id) == normalize(&format!("{jp}親指{n}")))
                {
                    profile
                        .mapping
                        .insert(format!("{side}Thumb{part}"), b.id.clone());
                }
            }
        }
    }
    let missing = required()
        .into_iter()
        .filter(|r| !profile.mapping.contains_key(r))
        .collect::<Vec<_>>();
    let report = serde_json::json!({"mapped":profile.mapping.len(),"missing_required":missing,"ambiguous":ambiguous,"mapping":profile.mapping});
    (profile, report)
}
pub fn ancestor(s: &CompiledSkeleton, a: usize, mut b: usize) -> bool {
    for _ in 0..s.source.bones.len() {
        if a == b {
            return true;
        }
        let Some(p) = s.source.bones[b].parent else {
            return false;
        };
        b = p;
    }
    false
}
impl Profile {
    pub fn reference_heading(&self, skeleton: &CompiledSkeleton) -> Result<Quat> {
        let (mapping, pose) = self.compile(skeleton, false)?;
        Ok(heading(&mapping, &skeleton.globals(&pose)?).unwrap_or(Quat::IDENTITY))
    }
    pub fn compile(
        &self,
        s: &CompiledSkeleton,
        complete: bool,
    ) -> Result<(BTreeMap<String, usize>, Pose)> {
        let supported = roles();
        let mut out = BTreeMap::new();
        let mut used = std::collections::BTreeSet::new();
        for (role, id) in &self.mapping {
            ensure!(supported.contains(role), "unknown humanoid role: {role}");
            let i = s.bone(id)?;
            ensure!(
                used.insert(i),
                "bone mapped to multiple humanoid roles: {id}"
            );
            out.insert(role.clone(), i);
        }
        if complete {
            for role in required() {
                ensure!(
                    out.contains_key(&role),
                    "humanoid mapping missing required {role}"
                );
            }
        }
        for side in ["Left", "Right"] {
            for chain in [
                ["UpperArm", "LowerArm", "Hand"],
                ["UpperLeg", "LowerLeg", "Foot"],
            ] {
                for pair in chain.windows(2) {
                    if let (Some(&a), Some(&b)) = (
                        out.get(&format!("{side}{}", pair[0])),
                        out.get(&format!("{side}{}", pair[1])),
                    ) {
                        ensure!(
                            ancestor(s, a, b),
                            "humanoid hierarchy mismatch: {side}{} -> {}",
                            pair[0],
                            pair[1]
                        );
                    }
                }
            }
        }
        if let (Some(&hips), Some(&spine), Some(&head)) =
            (out.get("Hips"), out.get("Spine"), out.get("Head"))
        {
            ensure!(
                ancestor(s, hips, spine) && ancestor(s, spine, head),
                "humanoid spine hierarchy mismatch"
            );
        }
        let mut reference = s.rest_pose();
        for (id, t) in &self.reference {
            t.validate()?;
            reference.local[s.bone(id)?] = *t;
        }
        s.globals(&reference)?;
        Ok((out, reference))
    }
}
fn heading(mapping: &BTreeMap<String, usize>, globals: &[Mat4]) -> Option<Quat> {
    let left = *mapping.get("LeftUpperArm")?;
    let right = *mapping.get("RightUpperArm")?;
    let lateral = (globals[left].w_axis - globals[right].w_axis).truncate();
    let lateral = Vec3::new(lateral.x, 0.0, lateral.z).try_normalize()?;
    Some(Quat::from_rotation_y((-lateral.z).atan2(lateral.x)))
}
pub struct Retargeter {
    pub source: CompiledSkeleton,
    pub target: CompiledSkeleton,
    source_ref: Vec<Mat4>,
    target_ref: Vec<Mat4>,
    target_pose: Pose,
    pairs: BTreeMap<usize, usize>,
    hips: Option<(usize, usize)>,
    heading_alignment: Quat,
    pub root_scale: f32,
}
impl Retargeter {
    pub fn new(
        source: &CompiledSkeleton,
        sp: &Profile,
        target: &CompiledSkeleton,
        tp: &Profile,
        require_complete: bool,
    ) -> Result<Self> {
        let (sm, sr) = sp.compile(source, require_complete)?;
        let (tm, tr) = tp.compile(target, require_complete)?;
        let mut pairs = BTreeMap::new();
        for (role, &ti) in &tm {
            if let Some(&si) = sm.get(role) {
                pairs.insert(ti, si);
            }
        }
        ensure!(!pairs.is_empty(), "retarget profiles share no mapped bones");
        let source_ref = source.globals(&sr)?;
        let target_ref = target.globals(&tr)?;
        let heading_alignment = heading(&sm, &source_ref)
            .zip(heading(&tm, &target_ref))
            .map_or(Quat::IDENTITY, |(source, target)| {
                (target * source.inverse()).normalize()
            });
        let length = |map: &BTreeMap<String, usize>, g: &[Mat4]| -> Option<f32> {
            let a = *map.get("LeftUpperLeg")?;
            let b = *map.get("LeftLowerLeg")?;
            let c = *map.get("LeftFoot")?;
            Some(
                g[a].w_axis.truncate().distance(g[b].w_axis.truncate())
                    + g[b].w_axis.truncate().distance(g[c].w_axis.truncate()),
            )
        };
        let root_scale = match (length(&sm, &source_ref), length(&tm, &target_ref)) {
            (Some(a), Some(b)) if a > 1e-5 => b / a,
            _ => 1.,
        };
        let hips = sm.get("Hips").zip(tm.get("Hips")).map(|(&s, &t)| (s, t));
        Ok(Self {
            source: source.clone(),
            target: target.clone(),
            source_ref,
            target_ref,
            target_pose: tr,
            pairs,
            hips,
            heading_alignment,
            root_scale,
        })
    }
    pub fn apply(&self, source: &Pose, root_motion: bool) -> Result<Pose> {
        let sg = self.source.globals(source)?;
        let mut target = self.target_pose.clone();
        let mut tg = vec![Mat4::IDENTITY; target.local.len()];
        for &ti in &self.target.order {
            let parent = self.target.source.bones[ti]
                .parent
                .map_or(Mat4::IDENTITY, |p| tg[p]);
            if let Some(&si) = self.pairs.get(&ti) {
                let sr = sg[si].to_scale_rotation_translation().1;
                let reference = self.source_ref[si].to_scale_rotation_translation().1;
                let target_ref = self.target_ref[ti].to_scale_rotation_translation().1;
                let alignment = self.heading_alignment;
                let desired =
                    (alignment * (sr * reference.inverse()) * alignment.inverse() * target_ref)
                        .normalize();
                let pr = parent.to_scale_rotation_translation().1;
                target.local[ti].rotation = (pr.inverse() * desired).normalize();
                if root_motion && self.hips == Some((si, ti)) {
                    let delta = alignment
                        * (sg[si].w_axis - self.source_ref[si].w_axis).truncate()
                        * self.root_scale;
                    target.local[ti].translation = parent
                        .inverse()
                        .transform_point3(self.target_ref[ti].w_axis.truncate() + delta);
                }
            }
            tg[ti] = parent * target.local[ti].matrix();
        }
        self.target.globals(&target)?;
        Ok(target)
    }
}
