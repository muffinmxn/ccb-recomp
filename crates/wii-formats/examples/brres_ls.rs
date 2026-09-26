//! Lists a BRRES file's entries by folder: `cargo run --example brres_ls <brres> [folder filter]`.
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let raw = std::fs::read(&a[1])?;
    let data = if wii_formats::lz::is_lz(&raw) { wii_formats::lz::decompress(&raw)? } else { raw };
    let b = wii_formats::brres::Brres::parse(&data)?;
    for (folder, name, _) in &b.files {
        if a.get(2).is_none_or(|f| folder.contains(f.as_str())) {
            println!("{folder:24} {name}");
        }
    }
    Ok(())
}
