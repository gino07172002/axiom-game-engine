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
/// Importable skeletal formats: (file extension, description). [`load`] dispatches on these
/// extensions and `discover animation` lists them.
pub const FORMATS: [(&str, &str); 4] = [
    ("pmx", "PMX 2.0/2.1"),
    ("gltf", "glTF 2.0"),
    ("glb", "GLB (binary glTF 2.0)"),
    ("axrig", "Unity export bridge bundle (.axrig v1)"),
];
pub fn load(path: &Path, options: &ImportOptions) -> Result<Asset> {
    ensure!(
        std::fs::metadata(path)?.len() <= 134_217_728,
        "asset file exceeds 128 MiB"
    );
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut asset = match extension.as_str() {
        "pmx" => pmx::load(path, options)?,
        "gltf" | "glb" => gltf::load(path, options)?,
        "axrig" => bundle::load(path, options)?,
        _ => anyhow::bail!(
            "supported skeletal formats: {}; bake .blend/FBX/Unity curves through the export bridge",
            FORMATS.map(|(e, _)| format!(".{e}")).join(", ")
        ),
    };
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
/// The clip IDs importing `path` yields (glTF animations, `.axrig` clips; PMX has none),
/// without importing it: only the glTF JSON / the bundle's clip IDs are parsed. Cached per
/// process by path, length and modification time. `Ok(None)` for an unknown extension.
pub fn clip_ids(path: &Path) -> Result<Option<std::sync::Arc<Vec<String>>>> {
    use std::sync::{Arc, Mutex};
    type Entry = (u64, Option<std::time::SystemTime>, Arc<Vec<String>>);
    static CACHE: Mutex<BTreeMap<PathBuf, Entry>> = Mutex::new(BTreeMap::new());
    let meta = std::fs::metadata(path)?;
    ensure!(meta.len() <= 134_217_728, "asset file exceeds 128 MiB");
    let modified = meta.modified().ok();
    if let Some((len, time, ids)) = CACHE.lock().ok().and_then(|c| c.get(path).cloned()) {
        if len == meta.len() && time == modified {
            return Ok(Some(ids));
        }
    }
    #[derive(Deserialize)]
    struct Named {
        #[serde(default)]
        name: Option<String>,
    }
    #[derive(Deserialize)]
    struct Animations {
        #[serde(default)]
        animations: Vec<Named>,
    }
    #[derive(Deserialize)]
    struct Id {
        id: String,
    }
    #[derive(Deserialize)]
    struct Clips {
        #[serde(default)]
        clips: Vec<Id>,
    }
    let gltf_ids = |json: &[u8]| -> Result<Vec<String>> {
        let doc: Animations = serde_json::from_slice(json)?;
        Ok(doc
            .animations
            .into_iter()
            .enumerate()
            .map(|(i, a)| a.name.unwrap_or_else(|| format!("animation/{i}")))
            .collect())
    };
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let ids = match extension.as_str() {
        "pmx" => vec![],
        "gltf" => gltf_ids(&std::fs::read(path)?)?,
        "glb" => {
            use std::io::Read;
            let mut file = std::fs::File::open(path)?;
            let mut header = [0u8; 20];
            file.read_exact(&mut header)?;
            ensure!(
                &header[0..4] == b"glTF" && &header[16..20] == b"JSON",
                "invalid GLB header"
            );
            let len = u32::from_le_bytes(header[12..16].try_into()?) as u64;
            ensure!(len <= meta.len(), "truncated GLB JSON chunk");
            let mut json = vec![];
            file.take(len).read_to_end(&mut json)?;
            ensure!(json.len() as u64 == len, "truncated GLB JSON chunk");
            gltf_ids(&json)?
        }
        "axrig" => {
            let bundle: Clips = serde_json::from_slice(&std::fs::read(path)?)?;
            bundle.clips.into_iter().map(|c| c.id).collect()
        }
        _ => return Ok(None),
    };
    let ids = Arc::new(ids);
    if let Ok(mut cache) = CACHE.lock() {
        if cache.len() >= 64 {
            cache.clear();
        }
        cache.insert(path.to_path_buf(), (meta.len(), modified, ids.clone()));
    }
    Ok(Some(ids))
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
/// Largest encoded image an importer accepts.
pub const MAX_ENCODED_TEXTURE: usize = 33_554_432;
/// Decoded RGBA bytes all textures of one asset may hold together.
pub const MAX_DECODED_TEXTURES: usize = 268_435_456;
/// Reads an image file, never more than [`MAX_ENCODED_TEXTURE`] + 1 bytes (a larger file is
/// refused without reading it whole).
pub fn read_texture_file(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take(MAX_ENCODED_TEXTURE as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_ENCODED_TEXTURE,
        "encoded image exceeds 32 MiB"
    );
    Ok(bytes)
}
/// Appends a decoded texture, keeping the asset's total within [`MAX_DECODED_TEXTURES`].
pub fn push_texture(textures: &mut Vec<Texture>, decoded: &mut usize, t: Texture) -> Result<usize> {
    let total = decoded.saturating_add(t.rgba.len());
    ensure!(
        total <= MAX_DECODED_TEXTURES,
        "decoded textures exceed 256 MiB"
    );
    *decoded = total;
    textures.push(t);
    Ok(textures.len() - 1)
}
pub fn texture(bytes: &[u8]) -> Result<Texture> {
    ensure!(
        bytes.len() <= MAX_ENCODED_TEXTURE,
        "encoded image exceeds 32 MiB"
    );
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
