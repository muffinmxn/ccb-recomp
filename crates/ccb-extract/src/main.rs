//! Decrypts a WiiWare WAD and unpacks it into a directory tree:
//!
//! ```text
//! extracted/
//!   title.txt            summary of the TMD
//!   contents/000N.app    raw decrypted contents
//!   main.dol             the game executable
//!   files/NNNN/...       unpacked U8 archives
//! ```
//!
//! The Wii common key is not shipped. Supply it via `--key <file>` (16 raw bytes or
//! 32 hex chars), `WII_COMMON_KEY` (hex), or `keys/common-key.bin`.

use std::{fmt::Write as _, fs, path::PathBuf};

use anyhow::{bail, Context, Result};
use wii_formats::{dol::Dol, lz, u8arc, wad::Wad};

fn load_key(explicit: Option<PathBuf>) -> Result<[u8; 16]> {
    let raw = if let Some(p) = explicit {
        fs::read(&p).with_context(|| format!("reading {}", p.display()))?
    } else if let Ok(hex) = std::env::var("WII_COMMON_KEY") {
        hex.into_bytes()
    } else if let Ok(b) = fs::read("keys/common-key.bin") {
        b
    } else {
        bail!("no common key: pass --key, set WII_COMMON_KEY, or create keys/common-key.bin");
    };
    if raw.len() == 16 {
        return Ok(raw.try_into().unwrap());
    }
    let hex: String = String::from_utf8_lossy(&raw).chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 32 {
        bail!("common key must be 16 bytes or 32 hex characters");
    }
    let mut k = [0u8; 16];
    for (i, b) in k.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)?;
    }
    Ok(k)
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (mut wad_path, mut out, mut key) = (None, PathBuf::from("extracted"), None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--key" => key = args.next().map(PathBuf::from),
            "-o" | "--out" => out = args.next().map(PathBuf::from).context("-o needs a path")?,
            _ => wad_path = Some(PathBuf::from(a)),
        }
    }
    let wad_path = wad_path.context("usage: ccb-extract <game.wad> [-o extracted] [--key common-key.bin]")?;
    let bytes = fs::read(&wad_path).with_context(|| format!("reading {}", wad_path.display()))?;
    let wad = Wad::parse(&bytes)?;
    let contents = wad.decrypt_contents(&wad.title_key(&load_key(key)?)?)?;

    fs::create_dir_all(out.join("contents"))?;
    let mut summary = String::new();
    writeln!(summary, "game code: {}", wad.tmd.game_code())?;
    writeln!(summary, "title id:  {:016x}", wad.tmd.title_id)?;
    writeln!(summary, "ios:       {}", wad.tmd.ios & 0xffff_ffff)?;
    writeln!(summary, "boot idx:  {}", wad.tmd.boot_index)?;

    let mut main_dol: Option<(u16, Vec<u8>)> = None;
    let mut unpacked: Vec<Vec<u8>> = Vec::new();
    for (rec, raw) in wad.tmd.contents.iter().zip(&contents) {
        let name = format!("{:08x}.app", rec.id);
        fs::write(out.join("contents").join(&name), raw)?;
        let mut prefix = String::new();
        let data: &[u8] = if lz::is_lz(raw) && !u8arc::is_u8(raw) {
            match lz::decompress(raw) {
                Ok(d) => {
                    prefix = "LZ-compressed ".into();
                    unpacked.push(d);
                    unpacked.last().unwrap()
                }
                Err(_) => raw,
            }
        } else {
            raw
        };
        let kind = if u8arc::is_u8(data) {
            let files = u8arc::parse(data)?;
            let dir = out.join("files").join(format!("{:04}", rec.index));
            for e in &files {
                let p = dir.join(&e.path);
                match e.data {
                    None => fs::create_dir_all(&p).with_context(|| p.display().to_string())?,
                    Some(d) => {
                        fs::create_dir_all(p.parent().unwrap())?;
                        fs::write(&p, d).with_context(|| p.display().to_string())?;
                        // Also store decompressed copies of `foo.LZ` / `foo_LZ.bin` assets.
                        let stem = e.path.strip_suffix(".LZ").or_else(|| e.path.strip_suffix("_LZ.bin"));
                        if let (Some(stem), true) = (stem, lz::is_lz(d)) {
                            if let Ok(dec) = lz::decompress(d) {
                                fs::write(dir.join(stem), dec)?;
                            }
                        }
                    }
                }
            }
            format!("U8 archive, {} entries", files.len())
        } else if data.windows(4).take(0x100).any(|w| w == b"IMET") {
            "banner (IMET)".into()
        } else if Dol::looks_like_dol(data) {
            // The boot content of a WiiWare title is the NAND loader; the game is the
            // largest other DOL.
            if rec.index != wad.tmd.boot_index && main_dol.as_ref().map_or(true, |(_, d)| data.len() > d.len()) {
                main_dol = Some((rec.index, data.to_vec()));
            }
            format!("DOL, entry {:#010x}", Dol::parse(data)?.entry)
        } else {
            "unknown".into()
        };
        writeln!(summary, "content {:2} {name} type {:#06x} {:>9} bytes  {prefix}{kind}", rec.index, rec.kind, rec.size)?;
    }
    if let Some((idx, dol)) = main_dol {
        fs::write(out.join("main.dol"), dol)?;
        writeln!(summary, "main.dol = content {idx}")?;
    }
    fs::write(out.join("title.txt"), &summary)?;
    print!("{summary}");
    Ok(())
}
