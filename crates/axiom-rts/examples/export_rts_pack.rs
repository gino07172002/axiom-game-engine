fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let population = args
        .next()
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(64);
    let directory = args
        .next()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "content/rts".into());
    anyhow::ensure!(
        args.next().is_none(),
        "usage: export_rts_pack [64|128|256] [directory]"
    );
    let world = axiom_rts::content::demo_world(population)?;
    std::fs::create_dir_all(&directory)?;
    std::fs::write(
        directory.join("world.json"),
        serde_json::to_vec_pretty(&world)?,
    )?;
    std::fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"id":"frontier","version":1,"world":"world.json","capabilities":world.capabilities}),
        )?,
    )?;
    Ok(())
}
