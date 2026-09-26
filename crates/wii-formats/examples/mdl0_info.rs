//! Prints a summary of every mesh in a BRRES model: `cargo run --example mdl0_info <brres> <model>`.
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let raw = std::fs::read(&a[1])?;
    let data = if wii_formats::lz::is_lz(&raw) { wii_formats::lz::decompress(&raw)? } else { raw };
    let b = wii_formats::brres::Brres::parse(&data)?;
    let m = b.model(&a[2])?;
    for (i, bone) in m.bones.iter().enumerate() {
        println!("bone {i} {} parent {:?} billboard {} s {:?} r {:?} t {:?}\n  world {:?}", bone.name, bone.parent, bone.billboard, bone.scale, bone.rotation, bone.translation, bone.world);
    }
    for mat in &m.materials {
        println!("material {} cull {} xlu {} {:?}\n  {:?}\n  tev {:?} konst {:?}", mat.name, mat.cull, mat.translucent, mat.textures, mat.pixel, mat.tev_colors, mat.konst_colors);
    }
    for x in &m.meshes {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for uv in &x.uvs {
            for k in 0..2 {
                lo[k] = lo[k].min(uv[k]);
                hi[k] = hi[k].max(uv[k]);
            }
        }
        println!(
            "mesh {} mat {} bone {} xlu {} verts {} uvs {} colors {} uv range {lo:?}..{hi:?} pos0 {:?}",
            x.name, x.material, x.bone, x.translucent, x.positions.len(), x.uvs.len(), x.colors.len(), x.positions.first()
        );
        if std::env::var("VERBOSE").is_ok() {
            println!("  positions {:?}\n  vertex bones {:?}", x.positions, x.vertex_bones);
        }
    }
    Ok(())
}
