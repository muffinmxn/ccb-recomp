//! Decoding of GX (Flipper/Hollywood) texture formats into RGBA8.
//!
//! Textures are stored in tiles (blocks) whose size depends on the format; all
//! multi-byte values are big-endian.

use anyhow::{bail, ensure, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TexFormat {
    I4,
    I8,
    IA4,
    IA8,
    Rgb565,
    Rgb5a3,
    Rgba8,
    C4,
    C8,
    C14x2,
    Cmpr,
}

impl TexFormat {
    pub fn from_id(id: u32) -> Result<Self> {
        Ok(match id {
            0x0 => Self::I4,
            0x1 => Self::I8,
            0x2 => Self::IA4,
            0x3 => Self::IA8,
            0x4 => Self::Rgb565,
            0x5 => Self::Rgb5a3,
            0x6 => Self::Rgba8,
            0x8 => Self::C4,
            0x9 => Self::C8,
            0xA => Self::C14x2,
            0xE => Self::Cmpr,
            _ => bail!("unknown GX texture format {id:#x}"),
        })
    }

    /// (block width, block height, bits per pixel)
    fn block(self) -> (usize, usize, usize) {
        match self {
            Self::I4 | Self::C4 | Self::Cmpr => (8, 8, 4),
            Self::I8 | Self::IA4 | Self::C8 => (8, 4, 8),
            Self::IA8 | Self::Rgb565 | Self::Rgb5a3 | Self::C14x2 => (4, 4, 16),
            Self::Rgba8 => (4, 4, 32),
        }
    }

    pub fn is_paletted(self) -> bool {
        matches!(self, Self::C4 | Self::C8 | Self::C14x2)
    }

    /// Size in bytes of one mip level of the given dimensions.
    pub fn data_size(self, width: usize, height: usize) -> usize {
        let (bw, bh, bpp) = self.block();
        width.div_ceil(bw) * bw * height.div_ceil(bh) * bh * bpp / 8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteFormat {
    IA8,
    Rgb565,
    Rgb5a3,
}

impl PaletteFormat {
    pub fn from_id(id: u32) -> Result<Self> {
        Ok(match id {
            0 => Self::IA8,
            1 => Self::Rgb565,
            2 => Self::Rgb5a3,
            _ => bail!("unknown palette format {id}"),
        })
    }
}

/// Scales an n-bit channel to 8 bits by bit replication (so all-ones maps to 255).
fn expand(v: u16, bits: i32) -> u8 {
    let v = v as u32;
    let mut r = 0;
    let mut shift = 8 - bits;
    while shift > -bits {
        r |= if shift >= 0 { v << shift } else { v >> -shift };
        shift -= bits;
    }
    r as u8
}

fn rgb565(c: u16) -> [u8; 4] {
    [expand(c >> 11, 5), expand((c >> 5) & 0x3f, 6), expand(c & 0x1f, 5), 255]
}

fn rgb5a3(c: u16) -> [u8; 4] {
    if c & 0x8000 != 0 {
        [expand((c >> 10) & 0x1f, 5), expand((c >> 5) & 0x1f, 5), expand(c & 0x1f, 5), 255]
    } else {
        [expand((c >> 8) & 0xf, 4), expand((c >> 4) & 0xf, 4), expand(c & 0xf, 4), expand((c >> 12) & 7, 3)]
    }
}

fn palette_color(fmt: PaletteFormat, c: u16) -> [u8; 4] {
    match fmt {
        PaletteFormat::IA8 => {
            let i = c as u8;
            [i, i, i, (c >> 8) as u8]
        }
        PaletteFormat::Rgb565 => rgb565(c),
        PaletteFormat::Rgb5a3 => rgb5a3(c),
    }
}

/// Decodes one image (mip level 0) to tightly packed RGBA8 rows.
pub fn decode(
    data: &[u8],
    width: usize,
    height: usize,
    fmt: TexFormat,
    palette: Option<(&[u8], PaletteFormat)>,
) -> Result<Vec<u8>> {
    ensure!(data.len() >= fmt.data_size(width, height), "texture data truncated ({fmt:?} {width}x{height})");
    ensure!(!fmt.is_paletted() || palette.is_some(), "paletted texture without palette");
    let mut out = vec![0u8; width * height * 4];
    let (bw, bh, bpp) = fmt.block();
    let block_bytes = bw * bh * bpp / 8;
    let blocks_x = width.div_ceil(bw);
    let pal = |i: usize| -> [u8; 4] {
        let (p, pf) = palette.unwrap();
        let c = p.get(i * 2..i * 2 + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]));
        palette_color(pf, c)
    };

    for by in 0..height.div_ceil(bh) {
        for bx in 0..blocks_x {
            let blk = &data[(by * blocks_x + bx) * block_bytes..][..block_bytes];
            let mut px = |x: usize, y: usize, c: [u8; 4]| {
                let (gx, gy) = (bx * bw + x, by * bh + y);
                if gx < width && gy < height {
                    out[(gy * width + gx) * 4..][..4].copy_from_slice(&c);
                }
            };
            let u16at = |i: usize| u16::from_be_bytes([blk[i * 2], blk[i * 2 + 1]]);
            match fmt {
                TexFormat::I4 | TexFormat::C4 => {
                    for i in 0..64 {
                        let n = (blk[i / 2] >> if i % 2 == 0 { 4 } else { 0 }) & 0xf;
                        let c = if fmt == TexFormat::I4 {
                            let v = n * 0x11;
                            [v, v, v, v]
                        } else {
                            pal(n as usize)
                        };
                        px(i % 8, i / 8, c);
                    }
                }
                TexFormat::I8 => {
                    for (i, &v) in blk.iter().enumerate() {
                        px(i % 8, i / 8, [v, v, v, v]);
                    }
                }
                TexFormat::C8 => {
                    for (i, &v) in blk.iter().enumerate() {
                        px(i % 8, i / 8, pal(v as usize));
                    }
                }
                TexFormat::IA4 => {
                    for (i, &v) in blk.iter().enumerate() {
                        let (a, l) = ((v >> 4) * 0x11, (v & 0xf) * 0x11);
                        px(i % 8, i / 8, [l, l, l, a]);
                    }
                }
                TexFormat::IA8 => {
                    for i in 0..16 {
                        let (a, l) = (blk[i * 2], blk[i * 2 + 1]);
                        px(i % 4, i / 4, [l, l, l, a]);
                    }
                }
                TexFormat::Rgb565 => (0..16).for_each(|i| px(i % 4, i / 4, rgb565(u16at(i)))),
                TexFormat::Rgb5a3 => (0..16).for_each(|i| px(i % 4, i / 4, rgb5a3(u16at(i)))),
                TexFormat::C14x2 => (0..16).for_each(|i| px(i % 4, i / 4, pal((u16at(i) & 0x3fff) as usize))),
                TexFormat::Rgba8 => {
                    for i in 0..16 {
                        let (a, r) = (blk[i * 2], blk[i * 2 + 1]);
                        let (g, b) = (blk[32 + i * 2], blk[32 + i * 2 + 1]);
                        px(i % 4, i / 4, [r, g, b, a]);
                    }
                }
                TexFormat::Cmpr => {
                    // Four DXT1 sub-blocks in Z order.
                    for sb in 0..4 {
                        let s = &blk[sb * 8..sb * 8 + 8];
                        let (c0, c1) = (u16::from_be_bytes([s[0], s[1]]), u16::from_be_bytes([s[2], s[3]]));
                        let (a, b) = (rgb565(c0), rgb565(c1));
                        let mix = |wa: u32, wb: u32, d: u32| -> [u8; 4] {
                            let m = |i: usize| ((a[i] as u32 * wa + b[i] as u32 * wb) / d) as u8;
                            [m(0), m(1), m(2), 255]
                        };
                        let colors = if c0 > c1 {
                            [a, b, mix(2, 1, 3), mix(1, 2, 3)]
                        } else {
                            [a, b, mix(1, 1, 2), [0, 0, 0, 0]]
                        };
                        let (ox, oy) = ((sb % 2) * 4, (sb / 2) * 4);
                        for y in 0..4 {
                            for x in 0..4 {
                                let idx = (s[4 + y] >> (6 - x * 2)) & 3;
                                px(ox + x, oy + y, colors[idx as usize]);
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb5a3_both_modes() {
        assert_eq!(rgb5a3(0xffff), [255, 255, 255, 255]);
        assert_eq!(rgb5a3(0x0f00), [255, 0, 0, 0]);
        assert_eq!(rgb5a3(0x7f00), [255, 0, 0, 255]);
    }

    #[test]
    fn i8_tile_order() {
        // 16x4 I8 image = two 8x4 tiles side by side.
        let data: Vec<u8> = (0..64).collect();
        let img = decode(&data, 16, 4, TexFormat::I8, None).unwrap();
        assert_eq!(img[8 * 4], 32); // pixel (8,0) is the first texel of tile 2
        assert_eq!(img[16 * 4], 8); // pixel (0,1) is the second row of tile 1
    }
}
