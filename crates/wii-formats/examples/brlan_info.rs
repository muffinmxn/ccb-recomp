//! Prints what a BRLAN animates: `cargo run --example brlan_info <layout.arc> <anim name>`.
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let raw = std::fs::read(&a[1])?;
    let data = if wii_formats::lz::is_lz(&raw) { wii_formats::lz::decompress(&raw)? } else { raw };
    for e in wii_formats::u8arc::parse(&data)? {
        if !e.path.ends_with(&format!("{}.brlan", a[2])) {
            continue;
        }
        let an = wii_formats::lyt::Animation::parse(e.data.unwrap())?;
        println!("{} frames {} loop {}", e.path, an.frames, an.looping);
        for t in &an.targets {
            for tag in &t.tags {
                for c in &tag.curves {
                    let keys: Vec<String> = c.keys.iter().map(|k| format!("{}:{:.2}", k.frame, k.value)).collect();
                    println!("  {}{} {:?} t{} {}", if t.is_material { "mat " } else { "" }, t.name, tag.kind, c.target, keys.join(" "));
                }
            }
        }
    }
    Ok(())
}
