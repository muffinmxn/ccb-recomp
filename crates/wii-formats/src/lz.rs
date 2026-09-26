//! Nintendo LZ77 variants (`0x10` LZ10 and `0x11` LZ11), used for the compressed
//! game DOL and most `.LZ` assets.

use anyhow::{bail, ensure, Result};

pub fn is_lz(d: &[u8]) -> bool {
    d.len() >= 4 && matches!(d[0], 0x10 | 0x11)
}

pub fn decompress(d: &[u8]) -> Result<Vec<u8>> {
    ensure!(d.len() >= 4, "LZ header truncated");
    let kind = d[0];
    let mut size = u32::from_le_bytes([d[1], d[2], d[3], 0]) as usize;
    let mut i = 4;
    if size == 0 {
        ensure!(d.len() >= 8, "LZ header truncated");
        size = u32::from_le_bytes(d[4..8].try_into().unwrap()) as usize;
        i = 8;
    }
    if !matches!(kind, 0x10 | 0x11) {
        bail!("unsupported LZ type {kind:#x}");
    }
    let mut out = Vec::with_capacity(size);
    let byte = |i: &mut usize| -> Result<u8> {
        let b = *d.get(*i).ok_or_else(|| anyhow::anyhow!("LZ stream truncated"))?;
        *i += 1;
        Ok(b)
    };
    while out.len() < size {
        let flags = byte(&mut i)?;
        for bit in (0..8).rev() {
            if out.len() >= size {
                break;
            }
            if flags & (1 << bit) == 0 {
                out.push(byte(&mut i)?);
                continue;
            }
            let b0 = byte(&mut i)? as usize;
            let (len, disp) = if kind == 0x10 {
                let b1 = byte(&mut i)? as usize;
                ((b0 >> 4) + 3, ((b0 & 0xf) << 8 | b1) + 1)
            } else {
                match b0 >> 4 {
                    0 => {
                        let (b1, b2) = (byte(&mut i)? as usize, byte(&mut i)? as usize);
                        (((b0 & 0xf) << 4 | b1 >> 4) + 0x11, ((b1 & 0xf) << 8 | b2) + 1)
                    }
                    1 => {
                        let (b1, b2, b3) =
                            (byte(&mut i)? as usize, byte(&mut i)? as usize, byte(&mut i)? as usize);
                        (((b0 & 0xf) << 12 | b1 << 4 | b2 >> 4) + 0x111, ((b2 & 0xf) << 8 | b3) + 1)
                    }
                    n => {
                        let b1 = byte(&mut i)? as usize;
                        (n + 1, ((b0 & 0xf) << 8 | b1) + 1)
                    }
                }
            };
            ensure!(disp <= out.len(), "LZ back-reference before start of output");
            let start = out.len() - disp;
            for k in 0..len.min(size - out.len()) {
                out.push(out[start + k]);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn lz10_literal_and_backref() {
        // "abcabcabc": 3 literals then a back-reference of length 6, distance 3.
        let data = [0x10, 9, 0, 0, 0b0001_0000, b'a', b'b', b'c', 0x30, 0x02];
        assert_eq!(super::decompress(&data).unwrap(), b"abcabcabc");
    }
}
