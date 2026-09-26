//! Parses every CHR0 in the given BRRES files and reports failures: `cargo run --example chr0_all <brres...>`.
fn main() -> anyhow::Result<()> {
    let (mut ok, mut failed) = (0, 0);
    for path in std::env::args().skip(1) {
        let raw = std::fs::read(&path)?;
        let data = if wii_formats::lz::is_lz(&raw) { wii_formats::lz::decompress(&raw)? } else { raw };
        let b = wii_formats::brres::Brres::parse(&data)?;
        for ((folder, name, _), &off) in b.files.iter().zip(&b.offsets) {
            if folder != "AnmChr(NW4R)" {
                continue;
            }
            match wii_formats::chr0::Chr0::parse(&data, off) {
                Ok(a) => {
                    ok += 1;
                    if std::env::var("LIST").is_ok() {
                        println!("{:32} frames {:4} loop {}", a.name, a.frames, a.looping);
                    }
                    if std::env::var("TRACKS").is_ok_and(|s| s == *name) {
                        for t in a.tracks.iter().take(8) {
                            let ev = |c: &Option<[wii_formats::chr0::Channel; 3]>| c.as_ref().map(|c| [c[0].eval(0.0), c[1].eval(0.0), c[2].eval(0.0)]);
                            println!("{} s {:?} r {:?} t {:?}", t.bone, ev(&t.scale), ev(&t.rotation), ev(&t.translation));
                        }
                    }
                    if std::env::var("SHOW").is_ok_and(|s| s == *name) {
                        for t in &a.tracks {
                            let s = t.scale.as_ref().map(|c| [0.0, a.frames as f32].map(|f| c[0].eval(f)));
                            println!("{} scale.x at start/end {s:?}", t.bone);
                        }
                    }
                }
                Err(e) => {
                    failed += 1;
                    println!("{path}: {name}: {e:#}");
                }
            }
        }
    }
    println!("{ok} parsed, {failed} failed");
    Ok(())
}
