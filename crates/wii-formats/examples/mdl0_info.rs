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
        if std::env::var("BP").is_ok() {
            for (label, start, len) in [("material", mat.dl_offset, 0x180), ("shader", mat.shader_offset + 0x20, 0x1e0)] {
                let mut p = start;
                let mut regs = Vec::new();
                while p + 5 <= (start + len).min(data.len()) {
                    if data[p] == 0x61 {
                        regs.push(format!("{:02x}={:06x}", data[p + 1], u32::from_be_bytes(data[p + 1..p + 5].try_into().unwrap()) & 0xffffff));
                        p += 5;
                    } else {
                        p += 1;
                    }
                }
                println!("  {label} BP: {}", regs.join(" "));
            }
        }
        for st in &mat.tev_stages {
            let c = st.color_env;
            let a = st.alpha_env;
            println!(
                "  stage tex{}{} chan {} kc {:#x} ka {:#x} | C: a{} b{} c{} d{} bias{} sub{} clamp{} scale{} dest{} | A: a{} b{} c{} d{} dest{}",
                st.tex_map, if st.tex_enabled { "" } else { "(off)" }, st.channel, st.konst_color_sel, st.konst_alpha_sel,
                c >> 12 & 15, c >> 8 & 15, c >> 4 & 15, c & 15, c >> 16 & 3, c >> 18 & 1, c >> 19 & 1, c >> 20 & 3, c >> 22 & 3,
                a >> 13 & 7, a >> 10 & 7, a >> 7 & 7, a >> 4 & 7, a >> 22 & 3
            );
        }
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
