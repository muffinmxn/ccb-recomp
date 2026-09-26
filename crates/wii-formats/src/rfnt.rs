//! NW4R bitmap fonts (BRFNT / RFNT): glyph sheets plus width and character-map tables.

use std::collections::HashMap;

use anyhow::{ensure, Context, Result};

use crate::{be16, be32, gx_texture};

#[derive(Debug, Clone, Copy)]
pub struct Glyph {
    pub index: u16,
    /// Blank pixels before the glyph.
    pub left: i8,
    pub glyph_width: u8,
    /// Advance.
    pub char_width: i8,
}

#[derive(Debug, Clone)]
pub struct Font {
    pub line_feed: i8,
    pub height: u8,
    pub width: u8,
    pub ascent: u8,
    pub cell_width: u8,
    pub cell_height: u8,
    pub baseline: i8,
    pub cols: u16,
    pub rows: u16,
    pub sheet_width: u16,
    pub sheet_height: u16,
    /// Decoded sheets, RGBA8.
    pub sheets: Vec<Vec<u8>>,
    pub default_width: (i8, u8, i8),
    pub alt_index: u16,
    widths: HashMap<u16, (i8, u8, i8)>,
    map: HashMap<u32, u16>,
}

impl Font {
    pub fn parse(d: &[u8]) -> Result<Self> {
        ensure!(d.len() >= 0x10 && &d[0..4] == b"RFNT", "not a BRFNT file");
        let f = be16(d, 0xc) as usize;
        ensure!(&d[f..f + 4] == b"FINF", "FINF missing");
        let sect = |off: u32| (off as usize).saturating_sub(8);
        let tglp = sect(be32(d, f + 0x10));
        ensure!(&d[tglp..tglp + 4] == b"TGLP", "TGLP missing");
        let t = tglp;
        let sheet_size = be32(d, t + 0xc) as usize;
        let sheet_count = be16(d, t + 0x10) as usize;
        let format = gx_texture::TexFormat::from_id(be16(d, t + 0x12) as u32 & 0x7fff)?;
        let (sw, sh) = (be16(d, t + 0x18), be16(d, t + 0x1a));
        let image = be32(d, t + 0x1c) as usize;
        let sheets = (0..sheet_count)
            .map(|i| {
                let data = d.get(image + i * sheet_size..).context("font sheet out of range")?;
                gx_texture::decode(data, sw as usize, sh as usize, format, None)
            })
            .collect::<Result<Vec<_>>>()?;

        let mut widths = HashMap::new();
        let mut off = be32(d, f + 0x14);
        while off != 0 {
            let c = sect(off);
            let (begin, end) = (be16(d, c + 8), be16(d, c + 0xa));
            for i in begin..=end {
                let e = c + 0x10 + (i - begin) as usize * 3;
                widths.insert(i, (d[e] as i8, d[e + 1], d[e + 2] as i8));
            }
            off = be32(d, c + 0xc);
        }
        let mut map = HashMap::new();
        let mut off = be32(d, f + 0x18);
        while off != 0 {
            let c = sect(off);
            let (begin, end, method) = (be16(d, c + 8) as u32, be16(d, c + 0xa) as u32, be16(d, c + 0xc));
            let data = c + 0x14;
            match method {
                0 => {
                    let base = be16(d, data);
                    for code in begin..=end {
                        map.insert(code, base + (code - begin) as u16);
                    }
                }
                1 => {
                    for code in begin..=end {
                        let idx = be16(d, data + (code - begin) as usize * 2);
                        if idx != 0xffff {
                            map.insert(code, idx);
                        }
                    }
                }
                _ => {
                    let n = be16(d, data) as usize;
                    for i in 0..n {
                        map.insert(be16(d, data + 2 + i * 4) as u32, be16(d, data + 4 + i * 4));
                    }
                }
            }
            off = be32(d, c + 0x10);
        }
        Ok(Self {
            line_feed: d[f + 9] as i8,
            alt_index: be16(d, f + 0xa),
            default_width: (d[f + 0xc] as i8, d[f + 0xd], d[f + 0xe] as i8),
            height: d[f + 0x1c],
            width: d[f + 0x1d],
            ascent: d[f + 0x1e],
            cell_width: d[t + 8],
            cell_height: d[t + 9],
            baseline: d[t + 0xa] as i8,
            cols: be16(d, t + 0x14),
            rows: be16(d, t + 0x16),
            sheet_width: sw,
            sheet_height: sh,
            sheets,
            widths,
            map,
        })
    }

    pub fn glyph(&self, ch: char) -> Glyph {
        let index = self.map.get(&(ch as u32)).copied().unwrap_or(self.alt_index);
        let (left, glyph_width, char_width) = self.widths.get(&index).copied().unwrap_or(self.default_width);
        Glyph { index, left, glyph_width, char_width }
    }

    /// (sheet, x, y) of a glyph's cell in pixels. Cells are separated by 1px.
    pub fn cell(&self, index: u16) -> (usize, u32, u32) {
        let per_sheet = (self.cols as u32 * self.rows as u32).max(1);
        let i = index as u32;
        let sheet = (i / per_sheet) as usize;
        let c = i % per_sheet;
        let (col, row) = (c % self.cols as u32, c / self.cols as u32);
        (sheet, col * (self.cell_width as u32 + 1), row * (self.cell_height as u32 + 1))
    }
}
