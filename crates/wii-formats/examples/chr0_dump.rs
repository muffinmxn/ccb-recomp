//! Prints a CHR0's bone tracks at frame 0: `cargo run --example chr0_dump <brres> <anim>`.
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let raw = std::fs::read(&a[1])?;
    let data = if wii_formats::lz::is_lz(&raw) { wii_formats::lz::decompress(&raw)? } else { raw };
    let b = wii_formats::brres::Brres::parse(&data)?;
    for ((folder, name, _), &off) in b.files.iter().zip(&b.offsets) {
        if folder != "AnmChr(NW4R)" || name != &a[2] {
            continue;
        }
        let c = wii_formats::chr0::Chr0::parse(&data, off)?;
        println!("{} frames {}", c.name, c.frames);
        for t in &c.tracks {
            let ev = |ch: &Option<[wii_formats::chr0::Channel; 3]>| ch.as_ref().map(|c| [c[0].eval(0.0), c[1].eval(0.0), c[2].eval(0.0)]);
            println!("  {:16} s {:?} r {:?} t {:?}", t.bone, ev(&t.scale), ev(&t.rotation), ev(&t.translation));
        }
    }
    Ok(())
}
