//! TPL texture palettes (magic `0x0020AF30`), used for standalone textures.

use anyhow::{ensure, Context, Result};

use crate::{be16, be32, gx_texture};

pub struct TplImage {
    pub width: usize,
    pub height: usize,
    pub format: gx_texture::TexFormat,
    /// Decoded mip level 0, RGBA8.
    pub rgba: Vec<u8>,
}

pub fn parse(d: &[u8]) -> Result<Vec<TplImage>> {
    ensure!(d.len() >= 12 && be32(d, 0) == 0x0020_AF30, "not a TPL file");
    let count = be32(d, 4) as usize;
    let table = be32(d, 8) as usize;
    (0..count)
        .map(|i| {
            let img = be32(d, table + i * 8) as usize;
            let pal = be32(d, table + i * 8 + 4) as usize;
            let (height, width) = (be16(d, img) as usize, be16(d, img + 2) as usize);
            let format = gx_texture::TexFormat::from_id(be32(d, img + 4))?;
            let data = d.get(be32(d, img + 8) as usize..).context("image data out of range")?;
            let palette = if pal != 0 && format.is_paletted() {
                let count = be16(d, pal) as usize;
                let pf = gx_texture::PaletteFormat::from_id(be32(d, pal + 4))?;
                let off = be32(d, pal + 8) as usize;
                Some((d.get(off..off + count * 2).context("palette out of range")?, pf))
            } else {
                None
            };
            let rgba = gx_texture::decode(data, width, height, format, palette)?;
            Ok(TplImage { width, height, format, rgba })
        })
        .collect()
}
