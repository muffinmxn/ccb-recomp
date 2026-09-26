//! Parses every VIS0 in the given BRRES files: `cargo run --example vis0_all <brres...>`.
fn main() -> anyhow::Result<()> {
    let (mut ok, mut failed) = (0, 0);
    for path in std::env::args().skip(1) {
        let raw = std::fs::read(&path)?;
        let data = if wii_formats::lz::is_lz(&raw) { wii_formats::lz::decompress(&raw)? } else { raw };
        let b = wii_formats::brres::Brres::parse(&data)?;
        match b.vis_animations() {
            Ok(v) => {
                ok += v.len();
                if let Ok(show) = std::env::var("SHOW") {
                    for a in v.iter().filter(|a| a.name == show) {
                        for (bone, t) in &a.tracks {
                            println!("{} {bone}: frame0 {} end {}", a.name, t.visible(0.0), t.visible(a.frames as f32));
                        }
                    }
                }
            }
            Err(e) => {
                failed += 1;
                println!("{path}: {e:#}");
            }
        }
    }
    println!("{ok} parsed, {failed} files failed");
    Ok(())
}
