//! Prints a layout's pane tree: `cargo run --example lyt_tree <archive.arc> <layout> [max depth]`.
use wii_formats::{lyt, u8arc};

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let max_depth: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(99);
    let data = std::fs::read(&a[1])?;
    for e in u8arc::parse(&data)? {
        if !e.path.ends_with(&format!("/{}.brlyt", a[2])) {
            continue;
        }
        let l = lyt::Layout::parse(e.data.unwrap())?;
        for p in &l.panes {
            let mut depth = 0;
            let mut q = p.parent;
            while let Some(i) = q {
                depth += 1;
                q = l.panes[i].parent;
            }
            if depth > max_depth {
                continue;
            }
            let kind = match &p.kind {
                lyt::PaneKind::Null => "pan",
                lyt::PaneKind::Bounding => "bnd",
                lyt::PaneKind::Picture(_) => "pic",
                lyt::PaneKind::Text { .. } => "txt",
                lyt::PaneKind::Window { .. } => "wnd",
            };
            let mat = l.pane_material(l.panes.iter().position(|x| x.name == p.name).unwrap()).map(|m| {
                let m = &l.materials[m];
                format!(" mat[{} tex {:?} c0 {:?} c1 {:?} mc {:?} tev {} srt {:?}]", m.name, m.tex_maps.iter().map(|t| l.textures[t.texture].as_str()).collect::<Vec<_>>(), m.tev_colors[0], m.tev_colors[1], m.mat_color, m.tev_stages.len(), m.tex_srt)
            }).unwrap_or_default();
            println!("{}{kind} {} {}t({:.0},{:.0}) s({:.2},{:.2}) wh({:.0},{:.0}) o{} a{}{mat}", "  ".repeat(depth), p.name, if p.visible { "" } else { "[hidden] " },
                p.translate[0], p.translate[1], p.scale[0], p.scale[1], p.size[0], p.size[1], p.origin, p.alpha);
        }
        println!("groups: {:?}", l.groups.iter().map(|g| &g.0).collect::<Vec<_>>());
    }
    Ok(())
}
