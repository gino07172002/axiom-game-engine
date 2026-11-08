fn main() -> anyhow::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("model path required"))?;
    let asset = axiom_animation::import::load(std::path::Path::new(&path), &Default::default())?;
    let (_, mapping) = axiom_animation::humanoid::auto_map(&asset.skeleton);
    println!(
        "{}",
        serde_json::json!({"asset":asset.summary(),"mapping":mapping,"bones":asset.skeleton.source.bones.iter().map(|b|serde_json::json!({"id":b.id,"parent":b.parent.map(|p|&asset.skeleton.source.bones[p].id),"position":b.rest.translation})).collect::<Vec<_>>()})
    );
    Ok(())
}
