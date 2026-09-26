//! Parses every layout, animation and font in the layout archives and prints stats:
//! `cargo run --example lyt_all extracted/files/0002/layouts/*.arc`
use wii_formats::{lyt, rfnt, u8arc};

fn main() -> anyhow::Result<()> {
    let (mut nl, mut na, mut nf, mut custom_tev) = (0, 0, 0, 0);
    for path in std::env::args().skip(1).filter(|p| p.ends_with(".arc")) {
        let data = std::fs::read(&path)?;
        for e in u8arc::parse(&data)? {
            let Some(d) = e.data else { continue };
            let r: anyhow::Result<()> = (|| {
                if e.path.ends_with(".brlyt") {
                    let l = lyt::Layout::parse(d)?;
                    custom_tev += l.materials.iter().filter(|m| !m.tev_stages.is_empty()).count();
                    for m in &l.materials {
                        anyhow::ensure!(m.tex_maps.iter().all(|t| t.texture < l.textures.len()), "{}: bad texture index", m.name);
                    }
                    if std::env::var("SHOW").is_ok_and(|s| e.path.ends_with(&s)) {
                        for p in &l.panes {
                            if let lyt::PaneKind::Text { text, font, color_top, color_bottom, material, .. } = &p.kind {
                                let m = &l.materials[*material];
                                println!("  text {} font {} {:?} top {:?} bottom {:?} mat c0 {:?} c1 {:?} tev {}", p.name, l.fonts[*font], text, color_top, color_bottom, m.tev_colors[0], m.tev_colors[1], m.tev_stages.len());
                            }
                        }
                        for m in l.materials.iter().take(6) {
                            println!("  mat {} tex {:?} c0 {:?} c1 {:?} matcol {:?} tev {} ac {:?} blend {:?}", m.name,
                                m.tex_maps.iter().map(|t| &l.textures[t.texture]).collect::<Vec<_>>(), m.tev_colors[0], m.tev_colors[1], m.mat_color, m.tev_stages.len(), m.alpha_compare, m.blend_mode);
                        }
                    }
                    nl += 1;
                } else if e.path.ends_with(".brlan") {
                    lyt::Animation::parse(d)?;
                    na += 1;
                } else if e.path.ends_with(".brfnt") {
                    let f = rfnt::Font::parse(d)?;
                    let g = f.glyph('A');
                    anyhow::ensure!(g.glyph_width > 0, "glyph A has no width");
                    nf += 1;
                }
                Ok(())
            })();
            if let Err(err) = r {
                println!("{path}:{}: {err:#}", e.path);
            }
        }
    }
    println!("{nl} layouts, {na} animations, {nf} fonts parsed; {custom_tev} materials with custom TEV stages");
    Ok(())
}
