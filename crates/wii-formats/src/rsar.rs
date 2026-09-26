//! NW4R sound archives (BRSAR) holding wave sounds: SYMB (names), INFO (sounds, files,
//! groups), RWSD (wave sound data) and RWAR/RWAV (DSP-ADPCM or PCM waves).

use anyhow::{bail, ensure, Context, Result};

use crate::{be16, be32};

/// A reference as stored in INFO/RWSD tables: (is-offset, data type, pad, value).
fn reference(d: &[u8], o: usize) -> u32 {
    be32(d, o + 4)
}

#[derive(Debug, Clone)]
pub struct Sound {
    pub name: String,
    pub file_id: u32,
    /// Index of the wave sound inside its RWSD file.
    pub sub_index: u32,
    /// 0..=127
    pub volume: u8,
    /// 3 = wave sound (the only kind decoded here).
    pub kind: u8,
}

#[derive(Debug, Clone, Copy)]
struct GroupItem {
    file_id: u32,
    rwsd: usize,
    rwar: usize,
}

pub struct SoundArchive<'a> {
    data: &'a [u8],
    pub sounds: Vec<Sound>,
    items: Vec<GroupItem>,
}

/// Decoded audio: interleaved signed 16-bit samples.
#[derive(Debug, Clone)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl Pcm {
    /// A RIFF WAVE file, for handing to audio players.
    pub fn to_wav(&self) -> Vec<u8> {
        let data_len = (self.samples.len() * 2) as u32;
        let mut w = Vec::with_capacity(44 + data_len as usize);
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data_len).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&self.channels.to_le_bytes());
        w.extend_from_slice(&self.rate.to_le_bytes());
        w.extend_from_slice(&(self.rate * self.channels as u32 * 2).to_le_bytes());
        w.extend_from_slice(&(self.channels * 2).to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&data_len.to_le_bytes());
        for s in &self.samples {
            w.extend_from_slice(&s.to_le_bytes());
        }
        w
    }
}

impl<'a> SoundArchive<'a> {
    pub fn parse(d: &'a [u8]) -> Result<Self> {
        ensure!(d.len() >= 0x40 && &d[0..4] == b"RSAR", "not a BRSAR file");
        let symb = be32(d, 0x10) as usize;
        let info = be32(d, 0x18) as usize;
        ensure!(&d[symb..symb + 4] == b"SYMB" && &d[info..info + 4] == b"INFO", "BRSAR sections missing");
        let sb = symb + 8;
        let strings = sb + be32(d, sb) as usize;
        let string = |i: u32| -> String {
            let n = be32(d, strings);
            if i >= n {
                return String::new();
            }
            let o = sb + be32(d, strings + 4 + i as usize * 4) as usize;
            crate::brres::cstr(d, o)
        };
        let ib = info + 8;
        let table = |r: usize| -> Vec<usize> {
            let t = ib + reference(d, ib + r * 8) as usize;
            let n = be32(d, t) as usize;
            (0..n).map(|i| ib + reference(d, t + 4 + i * 8) as usize).collect()
        };
        let sounds = table(0)
            .into_iter()
            .map(|e| {
                let info_ref = ib + reference(d, e + 0x18) as usize;
                Sound {
                    name: string(be32(d, e)),
                    file_id: be32(d, e + 4),
                    volume: d[e + 0x14],
                    kind: d[e + 0x16],
                    sub_index: be32(d, info_ref),
                }
            })
            .collect();
        let mut items = Vec::new();
        for g in table(4) {
            let (goff, woff) = (be32(d, g + 0x10) as usize, be32(d, g + 0x18) as usize);
            let it = ib + reference(d, g + 0x20) as usize;
            for k in 0..be32(d, it) as usize {
                let e = ib + reference(d, it + 4 + k * 8) as usize;
                items.push(GroupItem {
                    file_id: be32(d, e),
                    rwsd: goff + be32(d, e + 4) as usize,
                    rwar: woff + be32(d, e + 0xc) as usize,
                });
            }
        }
        Ok(Self { data: d, sounds, items })
    }

    pub fn find(&self, name: &str) -> Option<&Sound> {
        self.sounds.iter().find(|s| s.name == name)
    }

    /// Decodes a wave sound's (first note's) wave.
    pub fn decode(&self, sound: &Sound) -> Result<Pcm> {
        ensure!(sound.kind == 3, "{} is not a wave sound", sound.name);
        let d = self.data;
        let item = self.items.iter().find(|i| i.file_id == sound.file_id).context("sound file not in any group")?;
        // RWSD: DATA table -> wave sound -> note table -> first note's wave index.
        let r = item.rwsd;
        ensure!(&d[r..r + 4] == b"RWSD", "expected RWSD");
        let data = r + be32(d, r + 0x10) as usize;
        let db = data + 8;
        let n = be32(d, db) as usize;
        ensure!((sound.sub_index as usize) < n, "wave sound index out of range");
        let wsd = db + reference(d, db + 4 + sound.sub_index as usize * 8) as usize;
        let notes = db + reference(d, wsd + 0x10) as usize;
        ensure!(be32(d, notes) > 0, "wave sound has no notes");
        let note = db + reference(d, notes + 4) as usize;
        let wave_index = be32(d, note) as usize;
        // RWAR: TABL -> RWAV.
        let a = item.rwar;
        ensure!(&d[a..a + 4] == b"RWAR", "expected RWAR");
        let tabl = a + be32(d, a + 0x10) as usize;
        let adata = a + be32(d, a + 0x18) as usize;
        let count = be32(d, tabl + 8) as usize;
        ensure!(wave_index < count, "wave index {wave_index} out of range ({count})");
        let rwav = adata + reference(d, tabl + 0xc + wave_index * 12) as usize;
        decode_rwav(&d[rwav..])
    }
}

/// Decodes an RWAV file.
pub fn decode_rwav(d: &[u8]) -> Result<Pcm> {
    ensure!(d.len() >= 0x20 && &d[0..4] == b"RWAV", "not an RWAV");
    let info = be32(d, 0x10) as usize;
    let wi = info + 8;
    let format = d[wi];
    let channels = d[wi + 2] as usize;
    let rate = be16(d, wi + 4) as u32 | ((d[wi + 3] as u32) << 16);
    let length = be32(d, wi + 0xc) as usize; // nibbles (ADPCM) or samples (PCM)
    let chan_table = wi + be32(d, wi + 0x10) as usize;
    let data = wi + be32(d, wi + 0x14) as usize;
    ensure!(channels > 0 && channels <= 2, "unsupported channel count {channels}");
    let mut per_channel: Vec<Vec<i16>> = Vec::new();
    for c in 0..channels {
        let ci = wi + be32(d, chan_table + c * 4) as usize;
        let start = data + 8 + be32(d, ci) as usize;
        let samples = match format {
            2 => {
                let ad = wi + be32(d, ci + 4) as usize;
                let coefs: Vec<i32> = (0..16).map(|i| be16(d, ad + i * 2) as i16 as i32).collect();
                let (mut yn1, mut yn2) = (be16(d, ad + 0x24) as i16 as i32, be16(d, ad + 0x26) as i16 as i32);
                let total = length * 14 / 16;
                let mut out = Vec::with_capacity(total);
                let mut p = start;
                while out.len() < total && p + 8 <= d.len() {
                    let h = d[p];
                    let scale = 1i32 << (h & 0xf);
                    let (c1, c2) = (coefs[((h >> 4) as usize & 7) * 2], coefs[((h >> 4) as usize & 7) * 2 + 1]);
                    for k in 0..14 {
                        if out.len() >= total {
                            break;
                        }
                        let byte = d[p + 1 + k / 2];
                        let nib = if k % 2 == 0 { byte >> 4 } else { byte & 0xf } as i32;
                        let nib = if nib >= 8 { nib - 16 } else { nib };
                        let s = ((nib * scale) << 11) + 1024 + c1 * yn1 + c2 * yn2;
                        let s = (s >> 11).clamp(-32768, 32767);
                        yn2 = yn1;
                        yn1 = s;
                        out.push(s as i16);
                    }
                    p += 8;
                }
                out
            }
            1 => (0..length).map(|i| be16(d, start + i * 2) as i16).collect(),
            0 => (0..length).map(|i| (d[start + i] as i8 as i16) << 8).collect(),
            f => bail!("unknown RWAV format {f}"),
        };
        per_channel.push(samples);
    }
    let len = per_channel.iter().map(Vec::len).min().unwrap_or(0);
    let mut samples = Vec::with_capacity(len * channels);
    for i in 0..len {
        for ch in &per_channel {
            samples.push(ch[i]);
        }
    }
    Ok(Pcm { rate, channels: channels as u16, samples })
}
