//! CPU-side material authoring and bounded WGSL surface validation.
use anyhow::{ensure, Context, Result};
use axiom_core::{local::escape, Diagnostic, Registry, World};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, io::Read, path::Path};

const SOURCE_LIMIT: usize = 16 * 1024;
const MAX_PROGRAMS: usize = 16;
pub(crate) const MAX_MATERIALS: usize = 128;
const ABI: &str = include_str!("surface.wgsl");
const VERTEX_DEFAULT: &str = "fn vertex_offset(p: vec3<f32>, n: vec3<f32>, uv: vec2<f32>, time: f32, params: vec4<f32>) -> vec3<f32> { return p; }";
const PROBE: &str = "@fragment fn axiom_validate() -> @location(0) vec4<f32> { let displaced: vec3<f32> = vertex_offset(vec3<f32>(0.),vec3<f32>(0.,1.,0.),vec2<f32>(0.),0.,vec4<f32>(0.)); return surface(SurfaceInput(vec4<f32>(1.), vec3<f32>(0.), vec3<f32>(0.,1.,0.), vec2<f32>(0.), vec3<f32>(0.,0.,1.), 0., 1., 1., vec4<f32>(0.))); }";

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Alpha {
    #[default]
    Inherit,
    Opaque,
    Mask,
    Blend,
    Additive,
    Multiply,
}
impl Alpha {
    pub fn blended(self) -> bool {
        matches!(self, Self::Blend | Self::Additive | Self::Multiply)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub slot: usize,
    pub default: f32,
    pub min: f32,
    pub max: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Program {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wgsl: Option<String>,
    #[serde(default)]
    pub parameters: BTreeMap<String, Parameter>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub shader: String,
    #[serde(default = "white")]
    pub tint: [f32; 4],
    #[serde(default)]
    pub parameters: BTreeMap<String, f32>,
    #[serde(default)]
    pub alpha: Alpha,
    #[serde(default = "cutoff")]
    pub alpha_cutoff: f32,
    #[serde(default)]
    pub double_sided: Option<bool>,
}
fn white() -> [f32; 4] {
    [1.0; 4]
}
fn cutoff() -> f32 {
    0.5
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Library {
    shaders: BTreeMap<String, Program>,
    materials: BTreeMap<String, Material>,
}

#[derive(Clone)]
pub struct PreparedMaterial {
    pub shader: String,
    pub values: [f32; 4],
    pub tint: [f32; 4],
    pub alpha: Alpha,
    pub cutoff: f32,
    pub double_sided: Option<bool>,
}

#[derive(Default)]
pub struct Prepared {
    pub programs: BTreeMap<String, String>,
    pub materials: BTreeMap<String, PreparedMaterial>,
}

fn builtins() -> BTreeMap<String, Program> {
    [
        ("lit", "fn surface(s: SurfaceInput) -> vec4<f32> { return vec4<f32>(s.base_color.rgb * (vec3<f32>(0.25 + 0.75 * s.diffuse * s.shadow) + axiom_local_light(s.world_position, s.normal)), s.base_color.a); }", BTreeMap::new()),
        ("unlit", "fn surface(s: SurfaceInput) -> vec4<f32> { return s.base_color; }", BTreeMap::new()),
        ("toon", "fn surface(s: SurfaceInput) -> vec4<f32> { let bands = s.params.x; let level = floor(s.diffuse * bands) / bands; let rim = pow(1.0 - clamp(dot(s.normal, s.view_direction), 0.0, 1.0), 3.0); return vec4<f32>(s.base_color.rgb * (0.25 + 0.75 * level * s.shadow) + vec3<f32>(rim * s.params.y), s.base_color.a); }", BTreeMap::from([
            ("bands".into(), Parameter {slot:0, default:3.0, min:1.0, max:8.0}),
            ("rim".into(), Parameter {slot:1, default:0.15, min:0.0, max:1.0}),
        ])),
    ].into_iter().map(|(id,wgsl,parameters)| (id.into(), Program {file:None, wgsl:Some(wgsl.into()), parameters})).collect()
}

fn valid_params(params: &BTreeMap<String, Parameter>) -> Result<()> {
    let mut slots = [false; 4];
    for (name, p) in params {
        axiom_core::valid_id(name)?;
        ensure!(
            p.slot < slots.len() && !slots[p.slot],
            "parameter {name} needs a unique slot in 0..4"
        );
        slots[p.slot] = true;
        ensure!(
            p.min.is_finite()
                && p.max.is_finite()
                && p.default.is_finite()
                && p.min <= p.default
                && p.default <= p.max,
            "invalid default/range for parameter {name}"
        );
    }
    Ok(())
}

pub(crate) fn shader_path(id: &str) -> String {
    format!("/resources/render.shaders/shaders/{}/wgsl", escape(id))
}
fn format_location(line: u32, column: u32) -> String {
    let start = ABI.lines().count() as u32;
    if line > start {
        format!("line {}, column {column}: ", line - start)
    } else {
        String::new()
    }
}

/// The hook line and column of a `validate_hook` message (the `format_location` prefix), if
/// the error has one.
pub(crate) fn error_location(message: &str) -> Option<(u32, u32)> {
    let rest = message.strip_prefix("line ")?;
    let (line, rest) = rest.split_once(", column ")?;
    let (column, _) = rest.split_once(": ")?;
    Some((line.parse().ok()?, column.parse().ok()?))
}

fn bounded(block: &naga::Block) -> bool {
    block.iter().all(|statement| match statement {
        naga::Statement::Loop { .. } | naga::Statement::Kill | naga::Statement::Barrier(_) => false,
        naga::Statement::Block(block) => bounded(block),
        naga::Statement::If { accept, reject, .. } => bounded(accept) && bounded(reject),
        naga::Statement::Switch { cases, .. } => cases.iter().all(|case| bounded(&case.body)),
        _ => true,
    })
}

pub fn validate_hook(id: &str, hook: &str) -> Result<()> {
    let path = shader_path(id);
    let error = |message: String| Diagnostic::error("invalid_shader", &path, message);
    if hook.is_empty() || hook.len() > SOURCE_LIMIT {
        return Err(error(format!(
            "WGSL source must be 1..{SOURCE_LIMIT} bytes"
        )));
    }
    let vertex = if has_vertex_hook(hook) {
        ""
    } else {
        VERTEX_DEFAULT
    };
    let source = format!("{ABI}{hook}\n{vertex}\n{PROBE}");
    let module = naga::front::wgsl::parse_str(&source).map_err(|e| {
        let location = e
            .location(&source)
            .map(|p| format_location(p.line_number, p.line_position))
            .unwrap_or_default();
        error(format!("{location}{}", e.message()))
    })?;
    if !module.global_variables.is_empty()
        || !module.overrides.is_empty()
        || module.entry_points.len() != 1
    {
        return Err(error("surface code may contain functions and constants only. Bindings, overrides and entry points belong to Axiom".into()));
    }
    let Some((_, surface)) = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some("surface"))
    else {
        return Err(error(
            "expected fn surface(s: SurfaceInput) -> vec4<f32>".into(),
        ));
    };
    let signature_ok = surface.arguments.len() == 1
        && module.types[surface.arguments[0].ty].name.as_deref() == Some("SurfaceInput")
        && surface.result.as_ref().is_some_and(|r| {
            matches!(
                module.types[r.ty].inner,
                naga::TypeInner::Vector {
                    size: naga::VectorSize::Quad,
                    scalar: naga::Scalar {
                        kind: naga::ScalarKind::Float,
                        width: 4
                    }
                }
            )
        });
    if !signature_ok {
        return Err(error(
            "expected fn surface(s: SurfaceInput) -> vec4<f32>".into(),
        ));
    }
    let engine_functions = 1 + usize::from(!has_vertex_hook(hook));
    if module.functions.len().saturating_sub(engine_functions) > 16
        || module
            .functions
            .iter()
            .map(|(_, f)| f.expressions.len())
            .sum::<usize>()
            > 2048
        || !module.functions.iter().all(|(_, f)| bounded(&f.body))
    {
        return Err(error("surface shader exceeds the function/expression budget or uses loops, discard or barriers".into()));
    }
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| {
        let location = e
            .location(&source)
            .map(|p| format_location(p.line_number, p.line_position))
            .unwrap_or_default();
        error(format!("{location}{}", e.as_inner()))
    })?;
    Ok(())
}

impl Prepared {
    pub fn from_world(world: &World) -> Result<Self> {
        if !world.resources.contains_key("render.shaders") {
            return Ok(Self::default());
        }
        let library: Library = world.resource("render.shaders")?;
        ensure!(
            library.shaders.len() <= MAX_PROGRAMS && library.materials.len() <= MAX_MATERIALS,
            "shader library exceeds budget"
        );
        let mut programs = builtins();
        for (id, program) in library.shaders {
            axiom_core::valid_id(&id)?;
            ensure!(
                !programs.contains_key(&id),
                "built-in shader {id} cannot be replaced"
            );
            axiom_core::local::at(
                &format!(
                    "/resources/render.shaders/shaders/{}/parameters",
                    escape(&id)
                ),
                valid_params(&program.parameters),
            )?;
            if program.file.is_some() || program.wgsl.is_none() {
                return Err(Diagnostic::error(
                    "invalid_shader_source",
                    shader_path(&id),
                    "use inline wgsl or resolve a package-relative file at pack import",
                ));
            }
            validate_hook(&id, program.wgsl.as_deref().unwrap())?;
            programs.insert(id, program);
        }
        let mut out = Self::default();
        for (id, material) in library.materials {
            axiom_core::valid_id(&id)?;
            let path = format!("/resources/render.shaders/materials/{}", escape(&id));
            let p = programs.get(&material.shader).ok_or_else(|| {
                Diagnostic::error("unknown_shader", format!("{path}/shader"), &material.shader)
            })?;
            let unit = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
            for (field, valid) in [
                ("tint", material.tint.into_iter().all(unit)),
                ("alpha_cutoff", unit(material.alpha_cutoff)),
            ] {
                if !valid {
                    return Err(Diagnostic::error(
                        "invalid_material",
                        format!("{path}/{field}"),
                        "expected finite 0..1 value",
                    ));
                }
            }
            let mut values = [0.; 4];
            for param in p.parameters.values() {
                values[param.slot] = param.default;
            }
            for (name, value) in material.parameters {
                let path = format!("{path}/parameters/{}", escape(&name));
                let param = p
                    .parameters
                    .get(&name)
                    .ok_or_else(|| Diagnostic::error("unknown_shader_parameter", &path, &name))?;
                if !value.is_finite() || !(param.min..=param.max).contains(&value) {
                    return Err(Diagnostic::error(
                        "invalid_shader_parameter",
                        path,
                        format!("expected {}..{}", param.min, param.max),
                    ));
                }
                values[param.slot] = value;
            }
            out.programs
                .entry(material.shader.clone())
                .or_insert_with(|| p.wgsl.clone().unwrap());
            out.materials.insert(
                id,
                PreparedMaterial {
                    shader: material.shader,
                    values,
                    tint: material.tint,
                    alpha: material.alpha,
                    cutoff: material.alpha_cutoff,
                    double_sided: material.double_sided,
                },
            );
        }
        Ok(out)
    }
}

fn resolve_files(root: &Path, world: &mut World) -> Result<()> {
    let Some(resource) = world.resources.get_mut("render.shaders") else {
        return Ok(());
    };
    let mut library: Library = serde_json::from_value(resource.clone())?;
    ensure!(
        library.shaders.len() <= MAX_PROGRAMS,
        "shader count exceeds budget"
    );
    for (id, shader) in &mut library.shaders {
        if let Some(file) = shader.file.take() {
            let source = (|| -> Result<String> {
                ensure!(
                    shader.wgsl.is_none(),
                    "shader must supply either file or wgsl"
                );
                let path = root
                    .join(&file)
                    .canonicalize()
                    .with_context(|| format!("missing shader file {file}"))?;
                ensure!(
                    !Path::new(&file).is_absolute()
                        && path.starts_with(root)
                        && path
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("wgsl")),
                    "shader file must be a .wgsl inside the package"
                );
                let mut source = String::new();
                std::fs::File::open(path)?
                    .take((SOURCE_LIMIT + 1) as u64)
                    .read_to_string(&mut source)?;
                ensure!(source.len() <= SOURCE_LIMIT, "shader file exceeds 16 KiB");
                Ok(source)
            })();
            shader.wgsl = Some(axiom_core::local::at(
                &format!("/resources/render.shaders/shaders/{}/file", escape(id)),
                source,
            )?);
        }
    }
    *resource = serde_json::to_value(library)?;
    Ok(())
}

pub fn register(registry: &mut Registry) {
    registry.capabilities.insert("shader".into(), 1);
    registry.component(
        "render.material",
        "shader",
        json!({"id":"stable material ID in render.shaders.materials"}),
        |v| {
            let binding: axiom_core::MaterialBinding = serde_json::from_value(v.clone())?;
            axiom_core::valid_id(&binding.id)
        },
    );
    registry.resource("render.shaders", "shader", json!({"shaders":"id -> {wgsl or package file, parameters: name -> {slot,default,min,max}}", "materials":"id -> {shader,tint?,parameters?,alpha?,alpha_cutoff?,double_sided?}"}), |v| {
        let _: Library = serde_json::from_value(v.clone())?;
        Ok(())
    });
    registry.package_resolvers.push(resolve_files);
    registry.world_validators.push(|w| {
        if !w.capabilities.contains_key("shader") {
            return Ok(());
        }
        ensure!(
            w.capabilities.contains_key("render"),
            "shader capability requires render"
        );
        let book = Prepared::from_world(w)?;
        for (kind, id, components) in w
            .entities
            .iter()
            .map(|e| ("entities", e.id.as_str(), &e.components))
            .chain(
                w.prefabs
                    .iter()
                    .map(|(id, p)| ("prefabs", id.as_str(), &p.components)),
            )
        {
            if let Some(value) = components.get("render.material") {
                let binding: axiom_core::MaterialBinding = serde_json::from_value(value.clone())?;
                if !book.materials.contains_key(&binding.id) {
                    let id = escape(id);
                    let prefix = if kind == "entities" { "@" } else { "" };
                    return Err(Diagnostic::error(
                        "unknown_material",
                        format!("/{kind}/{prefix}{id}/components/render.material/id"),
                        binding.id,
                    ));
                }
            }
        }
        Ok(())
    });
    registry.topics.insert("shader".into(), json!({
        "binding": {"render.material":{"id":"material_id"}},
        "library":"render.shaders", "abi":ABI, "hook":"fn surface(s: SurfaceInput) -> vec4<f32>",
        "builtins":builtins(), "limits":{"source_bytes":SOURCE_LIMIT,"custom_shaders":MAX_PROGRAMS,"materials":MAX_MATERIALS,"float_parameters":4},
        "alpha":["inherit","opaque","mask","blend","additive","multiply"], "geometry":["cube parts","imported triangle meshes"],
        "reload":"transactional apply/undo, CPU validate and GPU stage before runtime activation"
    }));
}

fn has_vertex_hook(hook: &str) -> bool {
    naga::front::wgsl::parse_str(&format!("{ABI}\n{hook}")).is_ok_and(|m| {
        m.functions
            .iter()
            .any(|(_, f)| f.name.as_deref() == Some("vertex_offset"))
    })
}
pub(crate) fn fragment(template: &str, hook: &str, wrapper: &str) -> String {
    let vertex = if has_vertex_hook(hook) {
        ""
    } else {
        VERTEX_DEFAULT
    };
    let template = if template.contains("@vertex fn vs_main") {
        template.replace("let world = model * vec4<f32>(v.position, 1.0);", "let world = model * vec4<f32>(vertex_offset(v.position,v.normal,vec2<f32>(0.0),g.eye_time.w,material_surface.params), 1.0);")
    } else {
        template.replace("let world=d.model*vec4<f32>(position,1.);", "let world=d.model*vec4<f32>(vertex_offset(position,normal,uv,g.eye_time.w,material_surface.params),1.);")
    };
    let abi = ABI.replace("return vec3<f32>(0.0);", "return axiom_lighting(p,n);");
    format!("{template}\n{abi}\n{hook}\n{vertex}\n{wrapper}")
}
