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
    let meta = std::fs::metadata(path)?;
    ensure!(meta.len() <= 134_217_728, "asset file exceeds 128 MiB");
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
    // A successful import also fills the clip index, so validating a revision that uses an
    // asset this process already imported parses nothing. Keyed by the content clip IDs are
    // read from, as `clip_ids` reads it.
    let clip_key =
        clip_source(&extension, &bytes).map(|source| (source.len(), content_hash(source)));
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
    if let Some((len, hash)) = clip_key {
        remember_clip_ids(
            path,
            (len, hash),
            Arc::new(asset.clips.iter().map(|c| c.id.clone()).collect()),
        );
    }
    Ok(asset)
}
use std::sync::{Arc, Mutex};
/// (length, hash) of the bytes clip IDs are read from: the GLB JSON chunk, else the file.
type ClipKey = (usize, u64);
type ClipEntry = (ClipKey, Arc<Vec<String>>);
/// Clip IDs per asset path with the content version they were read from. A content key, not
/// the file's length and modification time: a replaced file that kept both must not reuse the
/// old IDs.
static CLIP_INDEX: Mutex<BTreeMap<PathBuf, ClipEntry>> = Mutex::new(BTreeMap::new());
fn remember_clip_ids(path: &Path, key: ClipKey, ids: Arc<Vec<String>>) {
    if let Ok(mut cache) = CLIP_INDEX.lock() {
        if cache.len() >= 64 && !cache.contains_key(path) {
            cache.clear();
        }
        cache.insert(path.to_path_buf(), (key, ids));
    }
}
/// Hash of `bytes` (std's SipHash, optimized even in debug builds; stable within a process).
fn content_hash(bytes: &[u8]) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write(bytes);
    h.finish()
}
/// The part of an asset file clip IDs come from: the JSON chunk of a GLB, the whole file of a
/// `.gltf` / `.axrig`; `None` for formats without clips in the file (PMX) or a malformed GLB.
fn clip_source<'a>(extension: &str, bytes: &'a [u8]) -> Option<&'a [u8]> {
    match extension {
        "gltf" | "axrig" => Some(bytes),
        "glb" => {
            let len = u32::from_le_bytes(bytes.get(12..16)?.try_into().ok()?) as usize;
            (bytes.get(16..20)? == b"JSON").then_some(())?;
            bytes.get(20..20usize.checked_add(len)?)
        }
        _ => None,
    }
}
/// The clip IDs importing `path` yields (glTF animations, `.axrig` clips; PMX has none),
/// without importing it: only the glTF JSON / the bundle's clip IDs are parsed. Cached per
/// process by path and a hash of that JSON (an import of the same file fills the cache too),
/// so a hit still reads the JSON (of a GLB only its JSON chunk) but does not parse it.
/// `Ok(None)` for an unknown extension.
pub fn clip_ids(path: &Path) -> Result<Option<Arc<Vec<String>>>> {
    let meta = std::fs::metadata(path)?;
    ensure!(meta.len() <= 134_217_728, "asset file exceeds 128 MiB");
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
    // The JSON clip IDs are read from (the whole file except for a GLB).
    let source = match extension.as_str() {
        "pmx" => return Ok(Some(Arc::new(vec![]))),
        "gltf" | "axrig" => std::fs::read(path)?,
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
            json
        }
        _ => return Ok(None),
    };
    let key = (source.len(), content_hash(&source));
    if let Some((cached, ids)) = CLIP_INDEX.lock().ok().and_then(|c| c.get(path).cloned()) {
        if cached == key {
            return Ok(Some(ids));
        }
    }
    let ids = if extension == "axrig" {
        let bundle: Clips = serde_json::from_slice(&source)?;
        bundle.clips.into_iter().map(|c| c.id).collect()
    } else {
        gltf_ids(&source)?
    };
    let ids = Arc::new(ids);
    remember_clip_ids(path, key, ids.clone());
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
    read_texture(std::fs::File::open(path)?)
}
/// Reads an encoded image from `input`, consuming at most [`MAX_ENCODED_TEXTURE`] + 1 bytes.
pub fn read_texture(input: impl std::io::Read) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = vec![];
    input
        .take(MAX_ENCODED_TEXTURE as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_ENCODED_TEXTURE,
        "encoded image exceeds 32 MiB"
    );
    Ok(bytes)
}
/// Decodes an encoded image and appends it, keeping the asset's total within
/// [`MAX_DECODED_TEXTURES`]. The RGBA size is checked from the image header before decoding,
/// so an image that would exceed the remaining budget is never decoded.
pub fn push_texture(
    textures: &mut Vec<Texture>,
    decoded: &mut usize,
    bytes: &[u8],
) -> Result<usize> {
    let t = decode(bytes, |width, height| {
        let total = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| n.checked_add(*decoded));
        ensure!(
            total.is_some_and(|t| t <= MAX_DECODED_TEXTURES),
            "decoded textures exceed 256 MiB"
        );
        Ok(())
    })?;
    // The decoder may disagree with the header; the budget holds for what was decoded.
    let total = decoded.saturating_add(t.rgba.len());
    ensure!(
        total <= MAX_DECODED_TEXTURES,
        "decoded textures exceed 256 MiB"
    );
    *decoded = total;
    textures.push(t);
    Ok(textures.len() - 1)
}
fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(67_108_864);
    limits
}
/// Decodes `bytes` with one decoder (a JPEG decoder copies the whole input when it is
/// created), calling `check` with the header's dimensions before any pixel is decoded.
fn decode(bytes: &[u8], check: impl FnOnce(u32, u32) -> Result<()>) -> Result<Texture> {
    use image::ImageDecoder;
    ensure!(
        bytes.len() <= MAX_ENCODED_TEXTURE,
        "encoded image exceeds 32 MiB"
    );
    let format = image::guess_format(bytes).unwrap_or(image::ImageFormat::Tga);
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    reader.limits(limits());
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    check(width, height)?;
    // As `ImageReader::decode` does: the output buffer counts against the allocation limit.
    let mut limits = limits();
    limits.reserve(decoder.total_bytes())?;
    decoder.set_limits(limits)?;
    let image = image::DynamicImage::from_decoder(decoder)?.to_rgba8();
    Ok(Texture {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}
pub fn texture(bytes: &[u8]) -> Result<Texture> {
    decode(bytes, |_, _| Ok(()))
}
