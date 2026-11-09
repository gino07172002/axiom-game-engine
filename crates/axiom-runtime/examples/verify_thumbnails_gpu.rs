//! Asset thumbnails: off-screen render + readback on a headless GPU device.
//! `cargo run -p axiom-runtime --example verify_thumbnails_gpu -- OUT_DIR MODEL.glb...`
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let out =
        std::path::PathBuf::from(args.next().unwrap_or_else(|| "artifacts/thumbnails".into()));
    let mut models: Vec<std::path::PathBuf> = args.map(Into::into).collect();
    if models.is_empty() {
        models.push("content/axiom-town/axiom-short.glb".into());
    }
    println!("{}", axiom_runtime::validate_thumbnails_gpu(&models, &out)?);
    Ok(())
}
