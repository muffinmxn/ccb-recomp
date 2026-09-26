//! Prints each mesh's bone and model-space bounds: `cargo run --example mesh_bounds <brres> <model>`.
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let raw = std::fs::read(&a[1])?;
    let data = if wii_formats::lz::is_lz(&raw) { wii_formats::lz::decompress(&raw)? } else { raw };
    let b = wii_formats::brres::Brres::parse(&data)?;
    let m = b.model(&a[2])?;
    for mesh in &m.meshes {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &mesh.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        println!("{:14} bone {:14} verts {:4} x {:6.2}..{:6.2} y {:6.2}..{:6.2}", mesh.name, m.bones[mesh.bone].name, mesh.positions.len(), lo[0], hi[0], lo[1], hi[1]);
    }
    Ok(())
}
