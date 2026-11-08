fn main() -> anyhow::Result<()> {
    println!("{}", axiom_runtime::vfx_gpu::validate_compute()?);
    Ok(())
}
