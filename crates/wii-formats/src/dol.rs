//! DOL executables: up to 7 text and 11 data sections plus a BSS range.

use anyhow::{ensure, Result};

use crate::be32;

#[derive(Debug, Clone)]
pub struct Section {
    pub is_text: bool,
    pub file_offset: u32,
    pub address: u32,
    pub size: u32,
}

#[derive(Debug, Clone)]
pub struct Dol {
    pub sections: Vec<Section>,
    pub bss_address: u32,
    pub bss_size: u32,
    pub entry: u32,
}

impl Dol {
    pub fn parse(d: &[u8]) -> Result<Self> {
        ensure!(d.len() >= 0x100, "DOL header truncated");
        let mut sections = Vec::new();
        for i in 0..18 {
            let size = be32(d, 0x90 + i * 4);
            if size == 0 {
                continue;
            }
            let s = Section {
                is_text: i < 7,
                file_offset: be32(d, i * 4),
                address: be32(d, 0x48 + i * 4),
                size,
            };
            ensure!(
                (s.file_offset + s.size) as usize <= d.len(),
                "section {i} extends past end of file"
            );
            sections.push(s);
        }
        ensure!(!sections.is_empty(), "DOL has no sections");
        Ok(Self {
            sections,
            bss_address: be32(d, 0xd8),
            bss_size: be32(d, 0xdc),
            entry: be32(d, 0xe0),
        })
    }

    /// Heuristic used to spot the game executable among WAD contents.
    pub fn looks_like_dol(d: &[u8]) -> bool {
        Self::parse(d).is_ok_and(|dol| {
            dol.sections.iter().any(|s| s.is_text && s.file_offset >= 0x100)
                && (dol.entry & 0x3fff_ffff) < 0x0180_0000
        })
    }
}
