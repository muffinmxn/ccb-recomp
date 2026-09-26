//! Prints SRT0 animations: `cargo run --example srt0_dump <brres> [name filter]`.
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let raw = std::fs::read(&a[1])?;
    let data = if wii_formats::lz::is_lz(&raw) { wii_formats::lz::decompress(&raw)? } else { raw };
    let b = wii_formats::brres::Brres::parse(&data)?;
    for s in b.srt_animations()? {
        if a.get(2).is_some_and(|f| !s.name.contains(f.as_str())) {
            continue;
        }
        println!("{} frames {} loop {}", s.name, s.frames, s.looping);
        for t in &s.tracks {
            let f: Vec<String> = [0.0, s.frames as f32 * 0.25, s.frames as f32 * 0.5, s.frames as f32 * 0.75, s.frames as f32].iter().map(|&x| format!("{:.2?}", t.srt(x))).collect();
            println!("  {} layer {}: {}", t.material, t.layer, f.join(" | "));
        }
    }
    Ok(())
}
