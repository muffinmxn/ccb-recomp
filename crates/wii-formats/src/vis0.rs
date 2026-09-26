//! NW4R bone visibility animations (VIS0): per bone, either a constant or one bit per frame.

use anyhow::{bail, ensure, Result};

use crate::{be16, be32, brres::cstr, brres::index_group};

#[derive(Debug, Clone)]
pub enum VisTrack {
    Const(bool),
    /// Visibility for frames 0..=frames.
    Frames(Vec<bool>),
}

impl VisTrack {
    pub fn visible(&self, frame: f32) -> bool {
        match self {
            VisTrack::Const(v) => *v,
            VisTrack::Frames(f) => f.get((frame.max(0.0) as usize).min(f.len().saturating_sub(1))).copied().unwrap_or(true),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Vis0 {
    pub name: String,
    pub frames: u16,
    pub looping: bool,
    pub tracks: Vec<(String, VisTrack)>,
}

impl Vis0 {
    /// `d` is the whole BRRES, `v` the absolute offset of the VIS0 section.
    pub fn parse(d: &[u8], v: usize) -> Result<Self> {
        ensure!(&d[v..v + 4] == b"VIS0", "not a VIS0 section");
        let rel = |o: u32| v.wrapping_add(o as i32 as usize);
        let (data, name, info) = match be32(d, v + 8) {
            4 => (be32(d, v + 0x10), be32(d, v + 0x18), v + 0x20),
            3 => (be32(d, v + 0x10), be32(d, v + 0x14), v + 0x18),
            ver => bail!("unsupported VIS0 version {ver}"),
        };
        let frames = be16(d, info);
        let looping = be32(d, info + 4) != 0;
        let mut tracks = Vec::new();
        for (bone, e) in index_group(d, rel(data))? {
            let flags = be32(d, e + 4);
            let track = if flags & 2 != 0 {
                VisTrack::Const(flags & 1 != 0)
            } else {
                let n = frames as usize + 1;
                VisTrack::Frames((0..n).map(|i| be32(d, e + 8 + (i / 32) * 4) >> (31 - i % 32) & 1 != 0).collect())
            };
            tracks.push((bone, track));
        }
        Ok(Self { name: cstr(d, rel(name)), frames, looping, tracks })
    }
}
