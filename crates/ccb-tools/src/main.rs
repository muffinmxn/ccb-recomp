//! Developer tools for inspecting extracted game data.
//!
//! ```text
//! ccb-tools textures [extracted] [out]   dump every TPL / BRRES texture to PNG
//! ccb-tools brres <file.brres>           list the contents of a BRRES archive
//! ```

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use wii_formats::{brres, lz, tpl, u8arc};

fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().unwrap())?;
    let mut enc = png::Encoder::new(fs::File::create(path)?, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}

/// Dumps textures found in `data` (any supported container) under `out/name`.
fn dump(name: &str, data: &[u8], out: &Path, count: &mut usize) -> Result<()> {
    if lz::is_lz(data) && name.ends_with(".LZ") {
        return Ok(()); // the extractor already wrote the decompressed copy
    }
    if data.len() >= 4 && u32::from_be_bytes(data[..4].try_into()?) == 0x0020_AF30 {
        for (i, img) in tpl::parse(data).with_context(|| name.to_string())?.iter().enumerate() {
            write_png(&out.join(format!("{name}.{i}.png")), img.width, img.height, &img.rgba)?;
            *count += 1;
        }
    } else if data.starts_with(b"bres") {
        let b = brres::Brres::parse(data).with_context(|| name.to_string())?;
        for (tex_name, t) in b.folder("Textures(NW4R)") {
            let tex = brres::Tex0::parse(t)?;
            let plt = b.folder("Palettes(NW4R)").find(|(n, _)| *n == tex_name).map(|(_, p)| p);
            match tex.decode(t, plt) {
                Ok(rgba) => {
                    write_png(&out.join(name).join(format!("{tex_name}.png")), tex.width, tex.height, &rgba)?;
                    *count += 1;
                }
                Err(e) => eprintln!("{name}/{tex_name}: {e}"),
            }
        }
    } else if u8arc::is_u8(data) {
        for e in u8arc::parse(data)? {
            if let Some(d) = e.data {
                dump(&format!("{name}/{}", e.path), d, out, count)?;
            }
        }
    }
    Ok(())
}

fn walk(dir: &Path, root: &Path, out: &Path, count: &mut usize) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();
        if p.is_dir() {
            walk(&p, root, out, count)?;
        } else {
            let rel = p.strip_prefix(root)?.to_string_lossy().replace('\\', "/");
            if let Err(e) = dump(&rel, &fs::read(&p)?, out, count) {
                eprintln!("{rel}: {e:#}");
            }
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("textures") => {
            let root = PathBuf::from(args.get(1).map_or("extracted", |s| s.as_str()));
            let out = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "extracted/png".into()));
            let mut count = 0;
            walk(&root.join("files"), &root.join("files"), &out, &mut count)?;
            println!("wrote {count} textures to {}", out.display());
        }
        Some("brres") => {
            let path = args.get(1).context("usage: ccb-tools brres <file>")?;
            let raw = fs::read(path)?;
            let data = if lz::is_lz(&raw) { lz::decompress(&raw)? } else { raw };
            for (folder, name, bytes) in brres::Brres::parse(&data)?.files {
                println!("{folder:18} {name:40} {:>9} bytes", bytes.len());
            }
        }
        _ => eprintln!("usage: ccb-tools <textures|brres> ..."),
    }
    Ok(())
}
