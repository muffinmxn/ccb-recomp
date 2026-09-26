//! NW4R bone animations (CHR0, versions 4–5).
//!
//! Each animated bone has nine channels (scale XYZ, rotation XYZ in degrees, translation
//! XYZ), each either constant or a curve. Curves are Hermite keyframes (I4/I6/I12) or,
//! for rotation, per-frame linear tables (L1/L2/L4).

use anyhow::{bail, ensure, Result};

use crate::{be16, be32, bef32, brres::cstr, brres::index_group};

#[derive(Debug, Clone, Copy)]
pub struct Key {
    pub frame: f32,
    pub value: f32,
    pub tangent: f32,
}

#[derive(Debug, Clone)]
pub enum Channel {
    Const(f32),
    Hermite(Vec<Key>),
    /// One value per frame.
    Baked(Vec<f32>),
}

impl Channel {
    pub fn eval(&self, frame: f32) -> f32 {
        match self {
            Channel::Const(v) => *v,
            Channel::Baked(v) => {
                if v.is_empty() {
                    return 0.0;
                }
                let f = frame.clamp(0.0, (v.len() - 1) as f32);
                let i = f.floor() as usize;
                let t = f - i as f32;
                let next = v.get(i + 1).copied().unwrap_or(v[i]);
                v[i] + (next - v[i]) * t
            }
            Channel::Hermite(keys) => {
                let Some(first) = keys.first() else { return 0.0 };
                if frame <= first.frame {
                    return first.value;
                }
                let last = keys.last().unwrap();
                if frame >= last.frame {
                    return last.value;
                }
                let i = keys.windows(2).position(|w| frame >= w[0].frame && frame < w[1].frame).unwrap_or(0);
                let (a, b) = (keys[i], keys[i + 1]);
                let span = b.frame - a.frame;
                if span <= 0.0 {
                    return b.value;
                }
                let t = (frame - a.frame) / span;
                let (t2, t3) = (t * t, t * t * t);
                let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
                let h10 = t3 - 2.0 * t2 + t;
                let h01 = -2.0 * t3 + 3.0 * t2;
                let h11 = t3 - t2;
                h00 * a.value + h10 * span * a.tangent + h01 * b.value + h11 * span * b.tangent
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct BoneTrack {
    pub bone: String,
    /// `None` means "use the model's bind value" for that component.
    pub scale: Option<[Channel; 3]>,
    pub rotation: Option<[Channel; 3]>,
    pub translation: Option<[Channel; 3]>,
}

#[derive(Debug, Clone)]
pub struct Chr0 {
    pub name: String,
    pub frames: u16,
    pub looping: bool,
    pub tracks: Vec<BoneTrack>,
}

fn rel(base: usize, off: u32) -> usize {
    base.wrapping_add(off as i32 as usize)
}

/// Reads a keyframe curve. `fmt`: 1 I4, 2 I6, 3 I12, 4 L1, 5 L2, 6 L4 (4–6 rotation only).
fn read_curve(d: &[u8], p: usize, fmt: u32, frames: u16) -> Result<Channel> {
    ensure!(p + 8 <= d.len(), "CHR0 curve out of range");
    let count = be16(d, p) as usize;
    Ok(match fmt {
        1 | 2 => {
            let (step, base) = (bef32(d, p + 8), bef32(d, p + 12));
            let e = p + 16;
            let keys = (0..count)
                .map(|i| {
                    if fmt == 1 {
                        let v = be32(d, e + i * 4);
                        let tan = ((v & 0xfff) as i32) << 20 >> 20;
                        Key { frame: (v >> 24) as f32, value: base + step * ((v >> 12) & 0xfff) as f32, tangent: tan as f32 / 32.0 }
                    } else {
                        let o = e + i * 6;
                        Key {
                            frame: be16(d, o) as f32 / 32.0,
                            value: base + step * be16(d, o + 2) as f32,
                            tangent: be16(d, o + 4) as i16 as f32 / 256.0,
                        }
                    }
                })
                .collect();
            Channel::Hermite(keys)
        }
        3 => Channel::Hermite(
            (0..count)
                .map(|i| {
                    let o = p + 8 + i * 12;
                    Key { frame: bef32(d, o), value: bef32(d, o + 4), tangent: bef32(d, o + 8) }
                })
                .collect(),
        ),
        4 | 5 => {
            let (step, base) = (bef32(d, p), bef32(d, p + 4));
            let n = frames as usize + 1;
            Channel::Baked(
                (0..n)
                    .map(|i| {
                        let raw = if fmt == 4 { d[p + 8 + i] as f32 } else { be16(d, p + 8 + i * 2) as f32 };
                        base + step * raw
                    })
                    .collect(),
            )
        }
        6 => Channel::Baked((0..frames as usize + 1).map(|i| bef32(d, p + i * 4)).collect()),
        f => bail!("unknown CHR0 curve format {f}"),
    })
}

impl Chr0 {
    /// `d` is the whole BRRES, `c` the absolute offset of the CHR0 section.
    pub fn parse(d: &[u8], c: usize) -> Result<Self> {
        ensure!(&d[c..c + 4] == b"CHR0", "not a CHR0 section");
        let version = be32(d, c + 8);
        let (data, name, info) = match version {
            5 => (be32(d, c + 0x10), be32(d, c + 0x18), c + 0x20),
            3 | 4 => (be32(d, c + 0x10), be32(d, c + 0x14), c + 0x18),
            v => bail!("unsupported CHR0 version {v}"),
        };
        let frames = be16(d, info);
        let looping = be32(d, info + 4) != 0;
        let mut tracks = Vec::new();
        for (bone, b) in index_group(d, rel(c, data))? {
            let flags = be32(d, b + 4);
            let bit = |n: u32| flags >> n & 1 != 0;
            let mut p = b + 8;
            // One component group: `exists` bit, isotropic bit, three fixed bits, format.
            let mut group = |exists: bool, iso: bool, fixed_bit: u32, fmt: u32, identity: f32, use_model: bool|
             -> Result<Option<[Channel; 3]>> {
                if use_model {
                    return Ok(None);
                }
                if !exists {
                    return Ok(Some([Channel::Const(identity), Channel::Const(identity), Channel::Const(identity)]));
                }
                let n = if iso { 1 } else { 3 };
                let mut ch = Vec::with_capacity(3);
                for i in 0..n {
                    let word = be32(d, p);
                    let c = if bit(fixed_bit + i) {
                        Channel::Const(f32::from_bits(word))
                    } else {
                        read_curve(d, rel(b, word), fmt, frames)?
                    };
                    ch.push(c);
                    p += 4;
                }
                if iso {
                    ch = vec![ch[0].clone(), ch[0].clone(), ch[0].clone()];
                }
                Ok(Some([ch[0].clone(), ch[1].clone(), ch[2].clone()]))
            };
            let scale = group(bit(22), bit(4), 13, flags >> 25 & 3, 1.0, bit(7))?;
            let rotation = group(bit(23), bit(5), 16, flags >> 27 & 7, 0.0, bit(8))?;
            let translation = group(bit(24), bit(6), 19, flags >> 30 & 3, 0.0, bit(9))?;
            tracks.push(BoneTrack { bone, scale, rotation, translation });
        }
        Ok(Self { name: cstr(d, rel(c, name)), frames, looping, tracks })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hermite_hits_keys_and_is_smooth() {
        let c = Channel::Hermite(vec![
            Key { frame: 0.0, value: 0.0, tangent: 0.0 },
            Key { frame: 10.0, value: 1.0, tangent: 0.0 },
        ]);
        assert_eq!(c.eval(0.0), 0.0);
        assert_eq!(c.eval(10.0), 1.0);
        assert!((c.eval(5.0) - 0.5).abs() < 1e-6);
        assert_eq!(c.eval(20.0), 1.0);
    }
}
