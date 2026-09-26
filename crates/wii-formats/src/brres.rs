//! NW4R resource archives (`bres`): a tree of named index groups holding
//! MDL0 models, TEX0 textures, PLT0 palettes and CHR0/CLR0/SRT0/PAT0/VIS0/SCN0 animations.

use anyhow::{ensure, Context, Result};

use crate::{be16, be32, gx_texture};

pub struct Brres<'a> {
    pub data: &'a [u8],
    /// (folder, name, file bytes starting at the sub-file header)
    pub files: Vec<(String, String, &'a [u8])>,
}

fn cstr(d: &[u8], off: usize) -> String {
    let s = d.get(off..).unwrap_or_default();
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    String::from_utf8_lossy(&s[..end]).into_owned()
}

/// Reads an NW4R index group (a binary search tree stored as an array) at `off`.
/// Returns (name, absolute data offset) pairs; offsets are relative to the group.
pub fn index_group(d: &[u8], off: usize) -> Result<Vec<(String, usize)>> {
    ensure!(off + 8 <= d.len(), "index group out of range");
    let count = be32(d, off + 4) as usize;
    (1..=count)
        .map(|i| {
            let e = off + 8 + i * 16;
            ensure!(e + 16 <= d.len(), "index group entry out of range");
            let name = off.wrapping_add(be32(d, e + 8) as i32 as usize);
            let data = off.wrapping_add(be32(d, e + 12) as i32 as usize);
            Ok((cstr(d, name), data))
        })
        .collect()
}

impl<'a> Brres<'a> {
    pub fn parse(d: &'a [u8]) -> Result<Self> {
        ensure!(d.len() >= 16 && &d[0..4] == b"bres", "not a BRRES file");
        let root = be16(d, 0xc) as usize;
        ensure!(&d[root..root + 4] == b"root", "BRRES root missing");
        let mut files = Vec::new();
        for (folder, group) in index_group(d, root + 8)? {
            for (name, off) in index_group(d, group)? {
                ensure!(off + 8 <= d.len(), "{folder}/{name} out of range");
                let size = be32(d, off + 4) as usize;
                let bytes = d.get(off..off + size).with_context(|| format!("{folder}/{name} truncated"))?;
                files.push((folder.clone(), name, bytes));
            }
        }
        Ok(Self { data: d, files })
    }

    pub fn folder<'s>(&'s self, folder: &'s str) -> impl Iterator<Item = (&'s str, &'a [u8])> + 's {
        self.files.iter().filter(move |f| f.0 == folder).map(|f| (f.1.as_str(), f.2))
    }
}

pub struct Tex0 {
    pub width: usize,
    pub height: usize,
    pub format: gx_texture::TexFormat,
    pub mip_count: u32,
    pub data_offset: usize,
}

impl Tex0 {
    pub fn parse(t: &[u8]) -> Result<Self> {
        ensure!(t.len() >= 0x30 && &t[0..4] == b"TEX0", "not a TEX0 section");
        Ok(Self {
            data_offset: be32(t, 0x10) as usize,
            width: be16(t, 0x1c) as usize,
            height: be16(t, 0x1e) as usize,
            format: gx_texture::TexFormat::from_id(be32(t, 0x20))?,
            mip_count: be32(t, 0x24),
        })
    }

    /// Decodes mip 0. Paletted textures need the matching PLT0 section.
    pub fn decode(&self, t: &[u8], plt0: Option<&[u8]>) -> Result<Vec<u8>> {
        let palette = match plt0 {
            Some(p) if self.format.is_paletted() => {
                ensure!(&p[0..4] == b"PLT0", "not a PLT0 section");
                let fmt = gx_texture::PaletteFormat::from_id(be32(p, 0x18))?;
                let count = be16(p, 0x1c) as usize;
                let off = be32(p, 0x10) as usize;
                Some((p.get(off..off + count * 2).context("palette truncated")?, fmt))
            }
            _ => None,
        };
        let data = t.get(self.data_offset..).context("TEX0 data out of range")?;
        gx_texture::decode(data, self.width, self.height, self.format, palette)
    }
}
