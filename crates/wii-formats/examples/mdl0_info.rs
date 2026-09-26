//! Prints a summary of every mesh in a BRRES model: `cargo run --example mdl0_info <brres> <model>`.
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let raw = std::fs::read(&a[1])?;
    let data = if wii_formats::lz::is_lz(&raw) { wii_formats::lz::decompress(&raw)? } else { raw };
    let b = wii_formats::brres::Brres::parse(&data)?;
    let m = b.model(&a[2])?;
    for (i, bone) in m.bones.iter().enumerate() {
        println!("bone {i} {} parent {:?} flags {:#x} billboard {} s {:?} t {:?}", bone.name, bone.parent, bone.flags, bone.billboard, bone.scale, bone.translation);
    }
    for mat in &m.materials {
        println!("material {} cull {} xlu {} {:?}\n  {:?}\n  tev {:?} konst {:?}\n  chan {:?}", mat.name, mat.cull, mat.translucent, mat.textures, mat.pixel, mat.tev_colors, mat.konst_colors, mat.channels);
        if std::env::var("MATDUMP").is_ok_and(|n| n == mat.name) {
            // Material struct words from 0x40 to 0x420, as hex and float.
            for off in (0x40..0x420).step_by(4) {
                let w = u32::from_be_bytes(data[mat.offset + off..mat.offset + off + 4].try_into().unwrap());
                let f = f32::from_bits(w);
                if w != 0 {
                    println!("  +{off:#05x} {w:08x} {}", if f.abs() > 1e-4 && f.abs() < 1e4 { format!("{f}") } else { String::new() });
                }
            }
        }
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
        let envelope = x.vertex_bones.iter().filter(|b| b.is_none()).count();
        let distinct: std::collections::BTreeSet<_> = x.vertex_bones.iter().flatten().collect();
        println!("  rigid {} envelope-verts {} distinct bones {}", x.rigid.is_some(), envelope, distinct.len());
        if std::env::var("WEIGHTS").is_ok() {
            for i in (0..x.weights.len()).step_by((x.weights.len() / 5).max(1)).take(5) {
                println!("  v{i} pos {:?} w {:?}", x.positions[i], x.weights[i]);
            }
        }
        if std::env::var("VERBOSE").is_ok() {
            println!("  positions {:?}\n  vertex bones {:?}", x.positions, x.vertex_bones);
        }
    }
    Ok(())
}
