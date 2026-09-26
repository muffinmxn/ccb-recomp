//! NW4R 2D layouts: BRLYT (pane trees), BRLAN (pane/material animations).
//!
//! Coordinates are in layout units with the origin at the screen center and +Y up;
//! the game's layouts are 608x456.

use anyhow::{bail, ensure, Context, Result};

use crate::{be16, be32, bef32};

fn cstr_fixed(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn rgba(d: &[u8], o: usize) -> [u8; 4] {
    [d[o], d[o + 1], d[o + 2], d[o + 3]]
}

fn s16x4(d: &[u8], o: usize) -> [i16; 4] {
    [0, 1, 2, 3].map(|i| be16(d, o + i * 2) as i16)
}

/// A textured quad: vertex colors and UVs in TL, TR, BL, BR order.
#[derive(Debug, Clone)]
pub struct Quad {
    pub colors: [[u8; 4]; 4],
    pub material: usize,
    pub uvs: Option<[[f32; 2]; 4]>,
}

fn read_quad(d: &[u8], o: usize) -> (Quad, usize) {
    let colors = [0, 1, 2, 3].map(|i| rgba(d, o + i * 4));
    let material = be16(d, o + 0x10) as usize;
    let nuv = d[o + 0x12] as usize;
    let uvs = (nuv > 0).then(|| [0, 1, 2, 3].map(|i| [bef32(d, o + 0x14 + i * 8), bef32(d, o + 0x18 + i * 8)]));
    (Quad { colors, material, uvs }, o + 0x14 + nuv * 32)
}

#[derive(Debug, Clone)]
pub enum PaneKind {
    Null,
    Bounding,
    Picture(Quad),
    Text {
        material: usize,
        font: usize,
        text: String,
        /// Where the text block sits in the pane: x = pos % 3 (left/center/right), y = pos / 3 (top/center/bottom).
        position: u8,
        /// Line alignment: 0 not specified, 1 left, 2 center, 3 right.
        alignment: u8,
        color_top: [u8; 4],
        color_bottom: [u8; 4],
        font_size: [f32; 2],
        char_space: f32,
        line_space: f32,
    },
    Window {
        content: Quad,
        /// Frame materials with their flip mode (0 none, 1 h, 2 v, 3 rot90, 4 rot180, 5 rot270).
        frames: Vec<(usize, u8)>,
        /// Frame inset (left, right, top, bottom).
        inflation: [f32; 4],
    },
}

#[derive(Debug, Clone)]
pub struct Pane {
    pub name: String,
    pub kind: PaneKind,
    pub parent: Option<usize>,
    pub visible: bool,
    /// Children multiply by this pane's alpha.
    pub influenced_alpha: bool,
    /// Base position: x = origin % 3 (left/center/right), y = origin / 3 (top/center/bottom).
    pub origin: u8,
    pub alpha: u8,
    pub translate: [f32; 3],
    /// Degrees.
    pub rotate: [f32; 3],
    pub scale: [f32; 2],
    pub size: [f32; 2],
}

#[derive(Debug, Clone)]
pub struct TexMap {
    pub texture: usize,
    /// 0 clamp, 1 repeat, 2 mirror
    pub wrap: [u8; 2],
}

#[derive(Debug, Clone)]
pub struct LytMaterial {
    pub name: String,
    /// TEV registers C0 ("black color"), C1 ("white color"), C2; 10-bit signed.
    pub tev_colors: [[i16; 4]; 3],
    pub konst: [[u8; 4]; 4],
    pub tex_maps: Vec<TexMap>,
    /// (translate s,t; rotate; scale s,t) per texture SRT.
    pub tex_srt: Vec<[f32; 5]>,
    /// Material color from channel control; multiplies vertex colors.
    pub mat_color: [u8; 4],
    /// Custom TEV stages (raw 16-byte NW4R LYT records), empty for the default combiner.
    pub tev_stages: Vec<[u8; 16]>,
    /// (comp0, ref0, op, comp1, ref1) if the material overrides the alpha test.
    pub alpha_compare: Option<(u8, u8, u8, u8, u8)>,
    /// (type, src, dst, logic op) if the material overrides blending.
    pub blend_mode: Option<[u8; 4]>,
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub size: [f32; 2],
    pub textures: Vec<String>,
    pub fonts: Vec<String>,
    pub materials: Vec<LytMaterial>,
    /// Panes in file (draw) order; the first is the root.
    pub panes: Vec<Pane>,
    /// (group name, pane names)
    pub groups: Vec<(String, Vec<String>)>,
}

fn name_list(d: &[u8], s: usize) -> Vec<String> {
    let n = be16(d, s + 8) as usize;
    let base = s + 0xc;
    (0..n)
        .map(|i| {
            let off = be32(d, base + i * 8) as usize;
            let b = &d[base + off..];
            cstr_fixed(&b[..b.len().min(256)])
        })
        .collect()
}

fn parse_material(d: &[u8], m: usize) -> LytMaterial {
    let flags = be32(d, m + 0x3c);
    let mut p = m + 0x40;
    let n_texmap = (flags & 0xf) as usize;
    let n_srt = ((flags >> 4) & 0xf) as usize;
    let n_coordgen = ((flags >> 8) & 0xf) as usize;
    let has_swap = flags >> 12 & 1 != 0;
    let n_ind_srt = ((flags >> 13) & 3) as usize;
    let n_ind_stage = ((flags >> 15) & 7) as usize;
    let n_tev = ((flags >> 18) & 0x1f) as usize;
    let has_alpha_cmp = flags >> 23 & 1 != 0;
    let has_blend = flags >> 24 & 1 != 0;
    let has_chan = flags >> 25 & 1 != 0;
    let has_matcol = flags >> 27 & 1 != 0;
    let tex_maps = (0..n_texmap)
        .map(|i| TexMap { texture: be16(d, p + i * 4) as usize, wrap: [d[p + i * 4 + 2], d[p + i * 4 + 3]] })
        .collect();
    p += n_texmap * 4;
    let tex_srt = (0..n_srt).map(|i| [0, 1, 2, 3, 4].map(|k| bef32(d, p + i * 20 + k * 4))).collect();
    p += n_srt * 20 + n_coordgen * 4;
    if has_chan {
        p += 4;
    }
    let mut mat_color = [255; 4];
    if has_matcol {
        mat_color = rgba(d, p);
        p += 4;
    }
    if has_swap {
        p += 4;
    }
    p += n_ind_srt * 20 + n_ind_stage * 4;
    let tev_stages = (0..n_tev).map(|i| d[p + i * 16..p + i * 16 + 16].try_into().unwrap()).collect();
    p += n_tev * 16;
    let alpha_compare = has_alpha_cmp.then(|| {
        let v = (d[p], d[p + 1], d[p + 2], d[p + 3]);
        p += 4;
        // Packed as comp0 | comp1 << 4, op, ref0, ref1.
        (v.0 & 0xf, v.2, v.1, v.0 >> 4, v.3)
    });
    let blend_mode = has_blend.then(|| rgba(d, p));
    LytMaterial {
        name: cstr_fixed(&d[m..m + 20]),
        tev_colors: [s16x4(d, m + 0x14), s16x4(d, m + 0x1c), s16x4(d, m + 0x24)],
        konst: [0, 1, 2, 3].map(|i| rgba(d, m + 0x2c + i * 4)),
        tex_maps,
        tex_srt,
        mat_color,
        tev_stages,
        alpha_compare,
        blend_mode,
    }
}

impl Layout {
    pub fn parse(d: &[u8]) -> Result<Self> {
        ensure!(d.len() >= 0x10 && &d[0..4] == b"RLYT", "not a BRLYT file");
        let mut p = be16(d, 0xc) as usize;
        let mut out = Layout {
            size: [608.0, 456.0],
            textures: Vec::new(),
            fonts: Vec::new(),
            materials: Vec::new(),
            panes: Vec::new(),
            groups: Vec::new(),
        };
        let mut parent_stack: Vec<Option<usize>> = vec![None];
        let mut last_pane: Option<usize> = None;
        while p + 8 <= d.len() {
            let magic = &d[p..p + 4];
            let size = be32(d, p + 4) as usize;
            ensure!(size >= 8 && p + size <= d.len(), "bad section size at {p:#x}");
            match magic {
                b"lyt1" => out.size = [bef32(d, p + 0xc), bef32(d, p + 0x10)],
                b"txl1" => out.textures = name_list(d, p),
                b"fnl1" => out.fonts = name_list(d, p),
                b"mat1" => {
                    let n = be16(d, p + 8) as usize;
                    out.materials = (0..n).map(|i| parse_material(d, p + be32(d, p + 0xc + i * 4) as usize)).collect();
                }
                b"pan1" | b"pic1" | b"txt1" | b"wnd1" | b"bnd1" => {
                    let kind = match magic {
                        b"pic1" => PaneKind::Picture(read_quad(d, p + 0x4c).0),
                        b"txt1" => {
                            let t = p + 0x4c;
                            let str_len = be16(d, t + 2) as usize;
                            let text_off = be32(d, t + 0xc) as usize;
                            let units: Vec<u16> =
                                (0..str_len / 2).map(|i| be16(d, p + text_off + i * 2)).take_while(|&c| c != 0).collect();
                            PaneKind::Text {
                                material: be16(d, t + 4) as usize,
                                font: be16(d, t + 6) as usize,
                                position: d[t + 8],
                                alignment: d[t + 9],
                                text: String::from_utf16_lossy(&units),
                                color_top: rgba(d, t + 0x10),
                                color_bottom: rgba(d, t + 0x14),
                                font_size: [bef32(d, t + 0x18), bef32(d, t + 0x1c)],
                                char_space: bef32(d, t + 0x20),
                                line_space: bef32(d, t + 0x24),
                            }
                        }
                        b"wnd1" => {
                            let w = p + 0x4c;
                            let inflation = [0, 1, 2, 3].map(|i| bef32(d, w + i * 4));
                            let nframes = d[w + 0x10] as usize;
                            let content = read_quad(d, p + be32(d, w + 0x14) as usize).0;
                            let table = p + be32(d, w + 0x18) as usize;
                            let frames = (0..nframes)
                                .map(|i| {
                                    let f = p + be32(d, table + i * 4) as usize;
                                    (be16(d, f) as usize, d[f + 2])
                                })
                                .collect();
                            PaneKind::Window { content, frames, inflation }
                        }
                        b"bnd1" => PaneKind::Bounding,
                        _ => PaneKind::Null,
                    };
                    let v3 = |o: usize| [bef32(d, o), bef32(d, o + 4), bef32(d, o + 8)];
                    out.panes.push(Pane {
                        name: cstr_fixed(&d[p + 0xc..p + 0x1c]),
                        kind,
                        parent: *parent_stack.last().unwrap(),
                        visible: d[p + 8] & 1 != 0,
                        influenced_alpha: d[p + 8] & 2 != 0,
                        origin: d[p + 9],
                        alpha: d[p + 0xa],
                        translate: v3(p + 0x24),
                        rotate: v3(p + 0x30),
                        scale: [bef32(d, p + 0x3c), bef32(d, p + 0x40)],
                        size: [bef32(d, p + 0x44), bef32(d, p + 0x48)],
                    });
                    last_pane = Some(out.panes.len() - 1);
                }
                b"pas1" => parent_stack.push(last_pane),
                b"pae1" => {
                    parent_stack.pop();
                    ensure!(!parent_stack.is_empty(), "unbalanced pas1/pae1");
                }
                b"grp1" => {
                    let name = cstr_fixed(&d[p + 8..p + 0x18]);
                    let n = be16(d, p + 0x18) as usize;
                    let panes = (0..n).map(|i| cstr_fixed(&d[p + 0x1c + i * 16..p + 0x2c + i * 16])).collect();
                    out.groups.push((name, panes));
                }
                _ => {}
            }
            p += size;
        }
        ensure!(!out.panes.is_empty(), "layout has no panes");
        Ok(out)
    }

    pub fn pane(&self, name: &str) -> Option<usize> {
        self.panes.iter().position(|p| p.name == name)
    }

    /// Pane indices an animation binds to: members of its groups, plus their
    /// descendants when `child_binding` is set. `None` means every pane.
    pub fn bound_panes(&self, anim: &Animation) -> Option<Vec<bool>> {
        if anim.groups.is_empty() {
            return None;
        }
        let mut bound = vec![false; self.panes.len()];
        for (g, members) in &self.groups {
            if anim.groups.contains(g) {
                for m in members {
                    if let Some(i) = self.pane(m) {
                        bound[i] = true;
                    }
                }
            }
        }
        if anim.child_binding {
            // Parents always precede children, so one pass propagates down the tree.
            for i in 0..self.panes.len() {
                if let Some(p) = self.panes[i].parent {
                    bound[i] |= bound[p];
                }
            }
        }
        Some(bound)
    }

    /// Material index drawn by a pane, if any.
    pub fn pane_material(&self, i: usize) -> Option<usize> {
        match &self.panes[i].kind {
            PaneKind::Picture(q) | PaneKind::Window { content: q, .. } => Some(q.material),
            PaneKind::Text { material, .. } => Some(*material),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------------------
// BRLAN

#[derive(Debug, Clone, Copy)]
pub struct Key {
    pub frame: f32,
    pub value: f32,
    pub slope: f32,
}

#[derive(Debug, Clone)]
pub struct Curve {
    /// For RLMC/RLTS/RLTP: which material color / texture SRT / texture map.
    pub index: u8,
    pub target: u8,
    /// Step curves (visibility, texture pattern) hold each value until the next key.
    pub step: bool,
    pub keys: Vec<Key>,
}

impl Curve {
    pub fn eval(&self, frame: f32) -> f32 {
        let k = &self.keys;
        if k.is_empty() {
            return 0.0;
        }
        if frame <= k[0].frame {
            return k[0].value;
        }
        let last = k[k.len() - 1];
        if frame >= last.frame {
            return last.value;
        }
        let i = k.windows(2).position(|w| frame >= w[0].frame && frame < w[1].frame).unwrap_or(0);
        let (a, b) = (k[i], k[i + 1]);
        if self.step {
            return a.value;
        }
        let span = b.frame - a.frame;
        if span <= 0.0 {
            return b.value;
        }
        let t = (frame - a.frame) / span;
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * a.value
            + (t3 - 2.0 * t2 + t) * span * a.slope
            + (-2.0 * t3 + 3.0 * t2) * b.value
            + (t3 - t2) * span * b.slope
    }
}

/// Animation kinds (the BRLAN tag magic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimKind {
    /// Pane SRT: 0-2 translate, 3-5 rotate, 6-7 scale, 8-9 size.
    PaneSrt,
    /// Vertex colors: 0-15 (TL, TR, BL, BR) x RGBA; 16 pane alpha.
    VertexColor,
    /// 0 visible.
    Visibility,
    /// Material colors: 0-3 material color, 4-7 C0, 8-11 C1, 12-15 C2, 16-31 konst 0-3.
    MaterialColor,
    /// Texture SRT: 0-1 translate, 2 rotate, 3-4 scale.
    TextureSrt,
    /// Texture pattern: value indexes `Animation::textures`.
    TexturePattern,
    IndirectSrt,
}

#[derive(Debug, Clone)]
pub struct AnimTag {
    pub kind: AnimKind,
    pub curves: Vec<Curve>,
}

#[derive(Debug, Clone)]
pub struct AnimTarget {
    /// Pane name, or material name when `is_material`.
    pub name: String,
    pub is_material: bool,
    pub tags: Vec<AnimTag>,
}

#[derive(Debug, Clone)]
pub struct Animation {
    /// Layout groups this animation binds to (empty: every pane).
    pub groups: Vec<String>,
    /// Also bind the descendants of the groups' panes.
    pub child_binding: bool,
    /// Window of the shared timeline this clip plays; keys use absolute frames.
    pub start: i16,
    pub end: i16,
    pub frames: u16,
    pub looping: bool,
    pub textures: Vec<String>,
    pub targets: Vec<AnimTarget>,
}

impl Animation {
    pub fn parse(d: &[u8]) -> Result<Self> {
        ensure!(d.len() >= 0x10 && &d[0..4] == b"RLAN", "not a BRLAN file");
        let mut p = be16(d, 0xc) as usize;
        let (mut groups, mut child_binding, mut start, mut end) = (Vec::new(), false, 0i16, -1i16);
        while p + 8 <= d.len() && &d[p..p + 4] != b"pai1" {
            if &d[p..p + 4] == b"pat1" {
                let n = be16(d, p + 0xa) as usize;
                let go = p + be32(d, p + 0x10) as usize;
                groups = (0..n).map(|i| cstr_fixed(&d[go + i * 0x14..go + i * 0x14 + 16])).collect();
                start = be16(d, p + 0x14) as i16;
                end = be16(d, p + 0x16) as i16;
                child_binding = d[p + 0x18] != 0;
            }
            p += be32(d, p + 4) as usize;
        }
        ensure!(p + 0x14 <= d.len(), "BRLAN has no pai1 section");
        let frames = be16(d, p + 8);
        let looping = d[p + 0xa] != 0;
        let ntex = be16(d, p + 0xc) as usize;
        let nent = be16(d, p + 0xe) as usize;
        let table = p + be32(d, p + 0x10) as usize;
        let textures = (0..ntex)
            .map(|i| {
                let base = p + 0x14;
                let off = be32(d, base + i * 4) as usize;
                cstr_fixed(&d[base + off..(base + off + 128).min(d.len())])
            })
            .collect();
        let mut targets = Vec::new();
        for i in 0..nent {
            let e = p + be32(d, table + i * 4) as usize;
            let ntags = d[e + 0x14] as usize;
            let is_material = d[e + 0x15] != 0;
            let mut tags = Vec::new();
            for t in 0..ntags {
                let tag = e + be32(d, e + 0x18 + t * 4) as usize;
                let kind = match &d[tag..tag + 4] {
                    b"RLPA" => AnimKind::PaneSrt,
                    b"RLVC" => AnimKind::VertexColor,
                    b"RLVI" => AnimKind::Visibility,
                    b"RLMC" => AnimKind::MaterialColor,
                    b"RLTS" => AnimKind::TextureSrt,
                    b"RLTP" => AnimKind::TexturePattern,
                    b"RLIM" => AnimKind::IndirectSrt,
                    m => bail!("unknown BRLAN tag {:?}", String::from_utf8_lossy(m)),
                };
                let ncurves = d[tag + 4] as usize;
                let curves = (0..ncurves)
                    .map(|c| {
                        let ce = tag + be32(d, tag + 8 + c * 4) as usize;
                        let step = d[ce + 2] == 1;
                        let nkeys = be16(d, ce + 4) as usize;
                        let kp = ce + be32(d, ce + 8) as usize;
                        let keys = (0..nkeys)
                            .map(|k| {
                                if step {
                                    let o = kp + k * 8;
                                    Key { frame: bef32(d, o), value: be16(d, o + 4) as f32, slope: 0.0 }
                                } else {
                                    let o = kp + k * 12;
                                    Key { frame: bef32(d, o), value: bef32(d, o + 4), slope: bef32(d, o + 8) }
                                }
                            })
                            .collect();
                        Curve { index: d[ce], target: d[ce + 1], step, keys }
                    })
                    .collect();
                tags.push(AnimTag { kind, curves });
            }
            targets.push(AnimTarget { name: cstr_fixed(&d[e..e + 20]), is_material, tags });
        }
        if end < start {
            end = start + frames as i16;
        }
        Ok(Self { groups, child_binding, start, end, frames, looping, textures, targets })
    }
}

/// Pairs a layout with the animations found next to it in a layout archive.
pub fn parse_all(files: &[(&str, &[u8])]) -> Result<(Vec<(String, Layout)>, Vec<(String, Animation)>)> {
    let mut layouts = Vec::new();
    let mut anims = Vec::new();
    for (path, data) in files {
        let stem = path.rsplit('/').next().unwrap_or(path);
        if let Some(n) = stem.strip_suffix(".brlyt") {
            layouts.push((n.to_string(), Layout::parse(data).with_context(|| path.to_string())?));
        } else if let Some(n) = stem.strip_suffix(".brlan") {
            anims.push((n.to_string(), Animation::parse(data).with_context(|| path.to_string())?));
        }
    }
    Ok((layouts, anims))
}
