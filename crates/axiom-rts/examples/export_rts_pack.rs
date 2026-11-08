//! `export_rts_pack frontier [dir]` writes the playable skirmish (default `content/rts`);
//! `export_rts_pack 64|128|256 [dir]` writes the benchmark fixture (default `content/rts-bench`).
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let kind = args.next().unwrap_or_else(|| "64".into());
    let (world, default_dir) = if kind == "frontier" {
        (axiom_rts::content::frontier_world()?, "content/rts")
    } else {
        (
            axiom_rts::content::demo_world(kind.parse::<usize>()?)?,
            "content/rts-bench",
        )
    };
    let directory = args
        .next()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| default_dir.into());
    anyhow::ensure!(
        args.next().is_none(),
        "usage: export_rts_pack [frontier|64|128|256] [directory]"
    );
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
