fn main() -> anyhow::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/fps");
    let w = axiom_fps::demo_world()?;
    w.compile(&axiom_fps::registry())?;
    std::fs::write(root.join("world.json"), serde_json::to_string_pretty(&w)?)?;
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_string_pretty(
            &serde_json::json!({"id":"axiom.fps","version":1,"world":"world.json","capabilities":w.capabilities}),
        )?,
    )?;
    Ok(())
}
