pub mod bundle;
pub mod gltf;
pub mod pmx;
use crate::*;
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ImportOptions {
    pub coordinates: Option<coordinates::Coordinates>,
    pub strict: bool,
}
pub fn load(path: &Path, options: &ImportOptions) -> Result<Asset> {
    ensure!(
        std::fs::metadata(path)?.len() <= 134_217_728,
        "asset file exceeds 128 MiB"
    );
    let mut asset=match path.extension().and_then(|s|s.to_str()).unwrap_or("").to_ascii_lowercase().as_str(){"pmx"=>pmx::load(path,options)?,"gltf"|"glb"=>gltf::load(path,options)?,"axrig"=>bundle::load(path,options)?,_=>anyhow::bail!("supported skeletal formats: .pmx, .gltf, .glb, .axrig; bake .blend/FBX/Unity curves through the export bridge")};
    let mut bytes = std::fs::read(path)?;
    bytes.extend(serde_json::to_vec(options)?);
    let mut hash = 0xcbf29ce484222325u64;
    let mut mix = |bytes: &[u8]| {
        for b in bytes {
            hash = (hash ^ *b as u64).wrapping_mul(0x100000001b3)
        }
    };
    mix(&bytes);
    for v in &asset.vertices {
        for f in v
            .position
            .to_array()
            .into_iter()
            .chain(v.normal.to_array())
            .chain(v.uv.to_array())
            .chain(v.weights)
        {
            mix(&f.to_le_bytes())
        }
        for j in v.joints {
            mix(&j.to_le_bytes())
        }
    }
    for index in &asset.indices {
        mix(&index.to_le_bytes())
    }
    for t in &asset.textures {
        mix(&t.width.to_le_bytes());
        mix(&t.height.to_le_bytes());
        mix(&t.rgba)
    }
    asset.key = format!("{hash:016x}");
    asset.validate()?;
    if options.strict {
        ensure!(
            asset.diagnostics.is_empty(),
            "strict asset import: {}",
            asset.diagnostics.join("; ")
        );
    }
    Ok(asset)
}
pub fn confined(root: &Path, uri: &str) -> Result<PathBuf> {
    ensure!(
        !uri.contains(':') && !uri.starts_with('/') && !uri.starts_with('\\'),
        "asset reference must be relative"
    );
    let root = root.canonicalize()?;
    let p = root
        .join(uri.replace('\\', "/"))
        .canonicalize()
        .with_context(|| format!("missing asset dependency {uri}"))?;
    ensure!(p.starts_with(root), "asset dependency escapes root: {uri}");
    Ok(p)
}
pub fn texture(bytes: &[u8]) -> Result<Texture> {
    ensure!(bytes.len() <= 33_554_432, "encoded image exceeds 32 MiB");
    let format = image::guess_format(bytes).unwrap_or(image::ImageFormat::Tga);
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(67_108_864);
    reader.limits(limits);
    let image = reader.decode()?.to_rgba8();
    Ok(Texture {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}
