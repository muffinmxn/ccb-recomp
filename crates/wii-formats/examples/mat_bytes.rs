//! Hex-dumps a material struct: `cargo run --example mat_bytes <brres> <model> <material>`.
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let raw = std::fs::read(&a[1])?;
    let data = if wii_formats::lz::is_lz(&raw) { wii_formats::lz::decompress(&raw)? } else { raw };
    let b = wii_formats::brres::Brres::parse(&data)?;
    let m = b.model(&a[2])?;
    let mat = m.materials.iter().find(|x| x.name == a[3]).expect("material");
    let _ = &data;
    let off = mat.offset;
    let d = &data;
    let size = u32::from_be_bytes(d[off..off + 4].try_into()?) as usize;
    for row in (0..size.min(0x480)).step_by(16) {
        let bytes: Vec<String> = d[off + row..off + row + 16].iter().map(|x| format!("{x:02x}")).collect();
        println!("{row:04x}: {}", bytes.join(" "));
    }
    Ok(())
}
