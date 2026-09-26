//! NW4R texture matrix animations (SRT0): per material and texture layer, scale S/T,
//! rotation (degrees) and translation S/T, each constant or a Hermite curve.

use anyhow::{bail, ensure, Result};

use crate::{
    be16, be32, bef32,
    brres::{cstr, index_group},
    chr0::{Channel, Key},
};

/// One animated texture layer: channels scale S, scale T, rotation, translation S, translation T.
#[derive(Debug, Clone)]
pub struct TexTrack {
    pub material: String,
    pub layer: usize,
    pub channels: [Channel; 5],
}

impl TexTrack {
    /// (translate s, translate t, rotation, scale s, scale t) at `frame`, the order used by
    /// layout/material texture SRTs.
    pub fn srt(&self, frame: f32) -> [f32; 5] {
        let c = &self.channels;
        [c[3].eval(frame), c[4].eval(frame), c[2].eval(frame), c[0].eval(frame), c[1].eval(frame)]
    }
}

#[derive(Debug, Clone)]
pub struct Srt0 {
    pub name: String,
    pub frames: u16,
    pub looping: bool,
    pub tracks: Vec<TexTrack>,
}

fn keys(d: &[u8], p: usize) -> Channel {
    let n = be16(d, p) as usize;
    let keys = (0..n)
        .map(|i| {
            let k = p + 8 + i * 12;
            Key { frame: bef32(d, k), value: bef32(d, k + 4), tangent: bef32(d, k + 8) }
        })
        .collect();
    Channel::Hermite(keys)
}

impl Srt0 {
    /// `d` is the whole BRRES, `s` the absolute offset of the SRT0 section.
    pub fn parse(d: &[u8], s: usize) -> Result<Self> {
        ensure!(&d[s..s + 4] == b"SRT0", "not an SRT0 section");
        let rel = |base: usize, o: u32| base.wrapping_add(o as i32 as usize);
        let (data, name, info) = match be32(d, s + 8) {
            5 => (be32(d, s + 0x10), be32(d, s + 0x18), s + 0x20),
            4 => (be32(d, s + 0x10), be32(d, s + 0x14), s + 0x18),
            v => bail!("unsupported SRT0 version {v}"),
        };
        let frames = be16(d, info);
        let looping = be32(d, info + 8) != 0;
        let mut tracks = Vec::new();
        for (material, m) in index_group(d, rel(s, data))? {
            let used = be32(d, m + 4);
            let mut slot = m + 12;
            for layer in 0..8 {
                if used >> layer & 1 == 0 {
                    continue;
                }
                let t = rel(m, be32(d, slot));
                slot += 4;
                let code = be32(d, t);
                let mut p = t + 4;
                // Each value is a float, or (bits clear) an offset to a curve relative to itself.
                let mut value = |fixed: bool| {
                    let v = if fixed { Channel::Const(bef32(d, p)) } else { keys(d, rel(p, be32(d, p))) };
                    p += 4;
                    v
                };
                let (sx, sy) = if code & 0x02 != 0 {
                    (Channel::Const(1.0), Channel::Const(1.0))
                } else if code & 0x10 != 0 {
                    let v = value(code & 0x20 != 0);
                    (v.clone(), v)
                } else {
                    let x = value(code & 0x20 != 0);
                    (x, value(code & 0x40 != 0))
                };
                let rot = if code & 0x04 != 0 { Channel::Const(0.0) } else { value(code & 0x80 != 0) };
                let (tx, ty) = if code & 0x08 != 0 {
                    (Channel::Const(0.0), Channel::Const(0.0))
                } else {
                    let x = value(code & 0x100 != 0);
                    (x, value(code & 0x200 != 0))
                };
                tracks.push(TexTrack { material: material.clone(), layer, channels: [sx, sy, rot, tx, ty] });
            }
        }
        Ok(Self { name: cstr(d, rel(s, name)), frames, looping, tracks })
    }
}
