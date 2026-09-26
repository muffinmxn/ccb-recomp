//! NW4R G3D models (MDL0, versions 9–11), flattened into triangle lists.
//!
//! All offsets inside an MDL0 are relative to the structure that holds them, and names
//! point into the string table at the end of the enclosing BRRES, so parsing works on the
//! whole BRRES buffer plus the model's absolute offset.

use anyhow::{bail, ensure, Context, Result};

use crate::{be16, be32, bef32, brres::cstr, brres::index_group};

fn rel(base: usize, off: u32) -> usize {
    base.wrapping_add(off as i32 as usize)
}

/// A 3x4 row-major affine matrix, as stored by G3D.
pub type Mtx34 = [[f32; 4]; 3];

pub fn mtx_apply(m: &Mtx34, p: [f32; 3]) -> [f32; 3] {
    let r = |i: usize| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + m[i][3];
    [r(0), r(1), r(2)]
}

fn mtx_rotate(m: &Mtx34, p: [f32; 3]) -> [f32; 3] {
    let r = |i: usize| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2];
    let v = [r(0), r(1), r(2)];
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-8);
    [v[0] / len, v[1] / len, v[2] / len]
}

#[derive(Debug, Clone)]
pub struct Bone {
    pub name: String,
    pub parent: Option<usize>,
    pub scale: [f32; 3],
    /// Euler XYZ, degrees.
    pub rotation: [f32; 3],
    pub translation: [f32; 3],
    /// Bind-pose model-space transform.
    pub world: Mtx34,
    /// Billboard mode (0 = none).
    pub billboard: u32,
    /// Raw bone flags (0x100 = visible).
    pub flags: u32,
}

#[derive(Debug, Clone)]
pub struct TextureRef {
    pub texture: String,
    pub palette: String,
    /// GX texture map slot this layer is bound to.
    pub map_id: u32,
    /// Texture-coordinate map mode from the layer's effect matrix: 0 = mesh UVs,
    /// 1 = environment (camera), 2 = projection, 3 = environment (light), 4 = specular.
    pub map_mode: u8,
    /// 0 clamp, 1 repeat, 2 mirror
    pub wrap: [u32; 2],
}

/// Pixel-engine state from a material's display list (GX BP registers 0x40, 0x41, 0xF3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelState {
    pub blend: bool,
    /// GX blend factors: 0 zero, 1 one, 2 src color, 3 inv src color, 4 src alpha,
    /// 5 inv src alpha, 6 dst alpha, 7 inv dst alpha.
    pub src_factor: u8,
    pub dst_factor: u8,
    pub subtract: bool,
    pub color_update: bool,
    pub alpha_update: bool,
    pub z_test: bool,
    /// GX compare: 0 never, 1 less, 2 equal, 3 lequal, 4 greater, 5 nequal, 6 gequal, 7 always.
    pub z_func: u8,
    pub z_write: bool,
    /// (comp0, ref0, op, comp1, ref1); op 0 and, 1 or, 2 xor, 3 xnor.
    pub alpha_test: (u8, u8, u8, u8, u8),
}

impl Default for PixelState {
    fn default() -> Self {
        Self {
            blend: false,
            src_factor: 1,
            dst_factor: 0,
            subtract: false,
            color_update: true,
            alpha_update: true,
            z_test: true,
            z_func: 3,
            z_write: true,
            alpha_test: (7, 0, 0, 7, 0),
        }
    }
}

impl PixelState {
    /// Evaluates the alpha test for an 8-bit alpha value.
    pub fn alpha_passes(&self, a: u8) -> bool {
        let cmp = |f: u8, r: u8| match f {
            0 => false,
            1 => a < r,
            2 => a == r,
            3 => a <= r,
            4 => a > r,
            5 => a != r,
            6 => a >= r,
            _ => true,
        };
        let (c0, r0, op, c1, r1) = self.alpha_test;
        let (x, y) = (cmp(c0, r0), cmp(c1, r1));
        match op {
            0 => x && y,
            1 => x || y,
            2 => x != y,
            _ => x == y,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Material {
    pub name: String,
    /// Absolute offsets of the material struct, its display list and shader (for debugging).
    pub offset: usize,
    pub dl_offset: usize,
    pub shader_offset: usize,
    /// Active TEV stages, in order.
    pub tev_stages: Vec<TevStage>,
    pub pixel: PixelState,
    /// TEV color registers (GX_TEVPREV, GX_TEVREG0..2, 10-bit signed) and konstant colors
    /// (KCOLOR0..3), RGBA.
    pub tev_colors: [[i16; 4]; 4],
    pub konst_colors: [[u8; 4]; 4],
    pub textures: Vec<TextureRef>,
    /// 0 none, 1 front, 2 back, 3 all (GX cull mode)
    pub cull: u32,
    /// Drawn in the translucent pass.
    pub translucent: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub name: String,
    pub material: usize,
    /// Bone that owns this draw (from the DrawOpa/DrawXlu list).
    pub bone: usize,
    pub translucent: bool,
    pub draw_priority: u8,
    /// Model-space bind-pose positions.
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// RGBA 0..=255; empty when the mesh has no vertex colors.
    pub colors: Vec<[u8; 4]>,
    /// First UV set; empty when the mesh has no texture coordinates.
    pub uvs: Vec<[f32; 2]>,
    /// Bone each vertex is rigidly bound to, `None` for envelope (multi-bone) vertices.
    pub vertex_bones: Vec<Option<usize>>,
    /// When every vertex is bound to the same bone: that bone, plus positions and normals
    /// in its local space (for animating the mesh by moving the bone).
    pub rigid: Option<(usize, Vec<[f32; 3]>, Vec<[f32; 3]>)>,
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct Model {
    pub name: String,
    pub bones: Vec<Bone>,
    pub materials: Vec<Material>,
    pub meshes: Vec<Mesh>,
    pub bounds: ([f32; 3], [f32; 3]),
}

/// A raw vertex attribute array.
struct Array {
    data_off: usize,
    comps: usize,
    format: u32,
    divisor: u32,
    stride: usize,
    count: usize,
}

impl Array {
    fn read(&self, d: &[u8], idx: usize) -> Result<[f32; 3]> {
        ensure!(idx < self.count, "vertex index {idx} out of range ({})", self.count);
        let base = self.data_off + idx * self.stride;
        let scale = 1.0 / (1u32 << self.divisor) as f32;
        let mut out = [0f32; 3];
        for (c, v) in out.iter_mut().enumerate().take(self.comps) {
            *v = match self.format {
                0 => d[base + c] as f32 * scale,
                1 => d[base + c] as i8 as f32 * scale,
                2 => be16(d, base + c * 2) as f32 * scale,
                3 => be16(d, base + c * 2) as i16 as f32 * scale,
                4 => bef32(d, base + c * 4),
                f => bail!("bad vertex component format {f}"),
            };
        }
        Ok(out)
    }
}

struct ColorArray {
    data_off: usize,
    format: u32,
    stride: usize,
    count: usize,
}

impl ColorArray {
    fn read(&self, d: &[u8], idx: usize) -> Result<[u8; 4]> {
        ensure!(idx < self.count, "color index {idx} out of range");
        let b = self.data_off + idx * self.stride;
        let x4 = |v: u32| (v * 0x11) as u8;
        let x6 = |v: u32| ((v << 2) | (v >> 4)) as u8;
        Ok(match self.format {
            0 => {
                let c = be16(d, b) as u32;
                let x5 = |v: u32| ((v << 3) | (v >> 2)) as u8;
                [x5(c >> 11), ((c >> 5 & 0x3f) << 2 | (c >> 9 & 3)) as u8, x5(c & 0x1f), 255]
            }
            1 | 2 => [d[b], d[b + 1], d[b + 2], 255],
            3 => {
                let c = be16(d, b) as u32;
                [x4(c >> 12), x4(c >> 8 & 0xf), x4(c >> 4 & 0xf), x4(c & 0xf)]
            }
            4 => {
                let c = (d[b] as u32) << 16 | (d[b + 1] as u32) << 8 | d[b + 2] as u32;
                [x6(c >> 18), x6(c >> 12 & 0x3f), x6(c >> 6 & 0x3f), x6(c & 0x3f)]
            }
            5 => [d[b], d[b + 1], d[b + 2], d[b + 3]],
            f => bail!("bad color format {f}"),
        })
    }
}

impl Model {
    pub fn parse(d: &[u8], m: usize) -> Result<Self> {
        ensure!(&d[m..m + 4] == b"MDL0", "not an MDL0 section");
        let version = be32(d, m + 8);
        let nsections = match version {
            8 | 9 => 11,
            10 => 13,
            11 => 14,
            v => bail!("unsupported MDL0 version {v}"),
        };
        let sec = |i: usize| -> Option<usize> {
            let o = be32(d, m + 0x10 + i * 4);
            (o != 0).then(|| rel(m, o))
        };
        // Section order: defs, bones, vertices, normals, colors, uvs, [furvec, furpos,]
        // materials, shaders, objects, texture links, palette links, [user data].
        let fur = if version >= 10 { 2 } else { 0 };
        let (s_defs, s_bones, s_pos, s_nrm, s_clr, s_uv) = (sec(0), sec(1), sec(2), sec(3), sec(4), sec(5));
        let (s_mat, s_obj) = (sec(6 + fur), sec(8 + fur));
        let name = cstr(d, rel(m, be32(d, m + 0x10 + nsections * 4)));
        let info = m + 0x10 + nsections * 4 + 4;
        let node_to_bone: Vec<Option<usize>> = {
            let arr = rel(info, be32(d, info + 0x24));
            (0..be32(d, arr) as usize)
                .map(|i| {
                    let v = be32(d, arr + 4 + i * 4) as i32;
                    (v >= 0).then_some(v as usize)
                })
                .collect()
        };
        let v3 = |o: usize| [bef32(d, o), bef32(d, o + 4), bef32(d, o + 8)];
        let bounds = (v3(info + 0x28), v3(info + 0x34));

        // Bones
        let mut bones = Vec::new();
        let mut bone_offsets = Vec::new();
        if let Some(g) = s_bones {
            for (bname, b) in index_group(d, g)? {
                let mut world = [[0f32; 4]; 3];
                for (r, row) in world.iter_mut().enumerate() {
                    for (c, v) in row.iter_mut().enumerate() {
                        *v = bef32(d, b + 0x70 + (r * 4 + c) * 4);
                    }
                }
                bone_offsets.push(b);
                bones.push(Bone {
                    name: bname,
                    parent: None,
                    scale: v3(b + 0x20),
                    rotation: v3(b + 0x2c),
                    translation: v3(b + 0x38),
                    world,
                    billboard: be32(d, b + 0x18),
                    flags: be32(d, b + 0x14),
                });
            }
            for (i, &b) in bone_offsets.iter().enumerate() {
                let p = be32(d, b + 0x5c);
                if p != 0 {
                    bones[i].parent = bone_offsets.iter().position(|&o| o == rel(b, p));
                }
            }
        }

        // Vertex attribute arrays, indexed by their `index` field.
        let arrays = |sec: Option<usize>, is_uv: bool| -> Result<Vec<Array>> {
            let Some(g) = sec else { return Ok(Vec::new()) };
            let mut v: Vec<(u32, Array)> = index_group(d, g)?
                .into_iter()
                .map(|(_, a)| {
                    let kind = be32(d, a + 0x14);
                    let comps = if is_uv { 1 + kind as usize } else { 2 + kind as usize };
                    (
                        be32(d, a + 0x10),
                        Array {
                            data_off: rel(a, be32(d, a + 8)),
                            comps: comps.min(3),
                            format: be32(d, a + 0x18),
                            divisor: d[a + 0x1c] as u32,
                            stride: d[a + 0x1d] as usize,
                            count: be16(d, a + 0x1e) as usize,
                        },
                    )
                })
                .collect();
            v.sort_by_key(|x| x.0);
            Ok(v.into_iter().map(|x| x.1).collect())
        };
        let positions = arrays(s_pos, false)?;
        let mut normals = arrays(s_nrm, false)?;
        for n in &mut normals {
            n.comps = 3; // NBT arrays: we only read the normal
        }
        let uvs = arrays(s_uv, true)?;
        let colors: Vec<ColorArray> = match s_clr {
            None => Vec::new(),
            Some(g) => {
                let mut v: Vec<(u32, ColorArray)> = index_group(d, g)?
                    .into_iter()
                    .map(|(_, a)| {
                        (
                            be32(d, a + 0x10),
                            ColorArray {
                                data_off: rel(a, be32(d, a + 8)),
                                format: be32(d, a + 0x18),
                                stride: d[a + 0x1c] as usize,
                                count: be16(d, a + 0x1e) as usize,
                            },
                        )
                    })
                    .collect();
                v.sort_by_key(|x| x.0);
                v.into_iter().map(|x| x.1).collect()
            }
        };

        // Materials
        let mut materials = Vec::new();
        let mut material_offsets = Vec::new();
        if let Some(g) = s_mat {
            for (mname, mt) in index_group(d, g)? {
                let ntex = be32(d, mt + 0x2c) as usize;
                let layers = rel(mt, be32(d, mt + 0x30));
                let textures = (0..ntex)
                    .map(|i| {
                        let l = layers + i * 0x34;
                        let name_at = |o: usize| {
                            let off = be32(d, l + o);
                            if off == 0 { String::new() } else { cstr(d, rel(l, off)) }
                        };
                        TextureRef {
                            texture: name_at(0),
                            palette: name_at(4),
                            map_id: be32(d, l + 0x10),
                            // Effect matrices follow the 8 texture SRTs at +0x1A8.
                            map_mode: d.get(mt + 0x250 + i * 0x34 + 2).copied().unwrap_or(0),
                            wrap: [be32(d, l + 0x18), be32(d, l + 0x1c)],
                        }
                    })
                    .collect();
                let dl = rel(mt, be32(d, mt + 0x3c));
                let (pixel, tev_colors, konst_colors) = parse_material_dl(d, dl);
                let stage_count = d[mt + 0x16] as usize;
                let shader_offset = rel(mt, be32(d, mt + 0x28));
                let tev_stages = parse_shader(d, shader_offset, stage_count);
                material_offsets.push(mt);
                materials.push(Material {
                    name: mname,
                    offset: mt,
                    dl_offset: dl,
                    shader_offset,
                    tev_stages,
                    pixel,
                    tev_colors,
                    konst_colors,
                    textures,
                    cull: be32(d, mt + 0x18),
                    translucent: be32(d, mt + 0x10) & 0x8000_0000 != 0,
                });
            }
        }

        // Draw lists: which material and bone each object uses.
        let mut draws: Vec<(usize, usize, usize, u8, bool)> = Vec::new(); // obj, mat, bone, prio, xlu
        if let Some(g) = s_defs {
            for (dname, mut p) in index_group(d, g)? {
                let xlu = dname == "DrawXlu";
                loop {
                    match d[p] {
                        0x00 => p += 1,
                        0x01 => break,
                        0x02 | 0x05 | 0x06 => p += 5,
                        0x03 => p += 4 + d[p + 3] as usize * 6,
                        0x04 => {
                            let (mat, obj, bone) =
                                (be16(d, p + 1) as usize, be16(d, p + 3) as usize, be16(d, p + 5) as usize);
                            draws.push((obj, mat, bone, d[p + 7], xlu));
                            p += 8;
                        }
                        op => bail!("{name}: unknown definition opcode {op:#x} in {dname}"),
                    }
                }
            }
        }

        // Objects
        let mut meshes = Vec::new();
        if let Some(g) = s_obj {
            for (oname, o) in index_group(d, g)? {
                let index = be32(d, o + 0x3c) as usize;
                let Some(&(_, mat, bone, prio, xlu)) = draws.iter().find(|x| x.0 == index) else {
                    continue; // not drawn
                };
                let mesh = decode_object(
                    d, o, version, &positions, &normals, &colors, &uvs, &bones, &node_to_bone,
                )
                .with_context(|| format!("{name}/{oname}"))?;
                meshes.push(Mesh {
                    name: oname,
                    material: mat,
                    bone,
                    translucent: xlu || materials.get(mat).is_some_and(|m| m.translucent),
                    draw_priority: prio,
                    ..mesh
                });
            }
        }
        let _ = material_offsets;
        Ok(Self { name, bones, materials, meshes, bounds })
    }
}

/// One TEV (texture environment) combiner stage.
///
/// `color_env`/`alpha_env` are the raw GX registers (BP 0xC0+2n / 0xC1+2n):
/// `out = (d ± lerp(a, b, c) + bias) * scale`, optionally clamped, written to `dest`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TevStage {
    pub color_env: u32,
    pub alpha_env: u32,
    pub tex_map: u8,
    pub tex_coord: u8,
    pub tex_enabled: bool,
    /// Rasterized color channel: 0 COLOR0, 1 COLOR1, 7 zero.
    pub channel: u8,
    pub konst_color_sel: u8,
    pub konst_alpha_sel: u8,
}

/// Collects the BP writes (`0x61 rr vvvvvv`) of a shader display list, honoring the
/// 0xFE write mask, into a register file.
fn bp_registers(d: &[u8], start: usize, end: usize) -> std::collections::HashMap<u8, u32> {
    let mut regs = std::collections::HashMap::new();
    let mut mask = 0x00ff_ffffu32;
    let mut p = start;
    while p + 5 <= end.min(d.len()) {
        if d[p] != 0x61 {
            p += 1;
            continue;
        }
        let reg = d[p + 1];
        let v = be32(d, p + 1) & 0x00ff_ffff;
        p += 5;
        if reg == 0xfe {
            mask = v;
            continue;
        }
        let old = regs.get(&reg).copied().unwrap_or(0);
        regs.insert(reg, (old & !mask) | (v & mask));
        mask = 0x00ff_ffff;
    }
    regs
}

fn parse_shader(d: &[u8], sh: usize, stage_count: usize) -> Vec<TevStage> {
    let size = be32(d, sh) as usize;
    let regs = bp_registers(d, sh + 0x20, sh + size.max(0x20));
    (0..stage_count.min(16))
        .map(|i| {
            let order = regs.get(&(0x28 + (i / 2) as u8)).copied().unwrap_or(0) >> ((i % 2) * 12);
            let ksel = regs.get(&(0xf6 + (i / 2) as u8)).copied().unwrap_or(0) >> ((i % 2) * 10);
            TevStage {
                color_env: regs.get(&(0xc0 + 2 * i as u8)).copied().unwrap_or(0),
                alpha_env: regs.get(&(0xc1 + 2 * i as u8)).copied().unwrap_or(0),
                tex_map: (order & 7) as u8,
                tex_coord: ((order >> 3) & 7) as u8,
                tex_enabled: order & 0x40 != 0,
                channel: ((order >> 7) & 7) as u8,
                konst_color_sel: ((ksel >> 4) & 0x1f) as u8,
                konst_alpha_sel: ((ksel >> 9) & 0x1f) as u8,
            }
        })
        .collect()
}

/// Reads the BP register writes (`0x61 rr vvvvvv`) in a material display list:
/// 0x20 bytes of pixel-engine state followed by 0x80 bytes of TEV colors.
fn parse_material_dl(d: &[u8], dl: usize) -> (PixelState, [[i16; 4]; 4], [[u8; 4]; 4]) {
    let mut ps = PixelState::default();
    let mut tev = [[0i16; 4]; 4];
    let mut konst = [[0u8; 4]; 4];
    let end = (dl + 0xa0).min(d.len());
    // BP register 0xFE sets a write mask for the next BP write; GX resets it to all-ones after use.
    let mut mask = 0x00ff_ffffu32;
    // Blend-mode register value implied by PixelState::default(), so masked writes merge correctly.
    let mut blend_reg: u32 = (1 << 3) | (1 << 4) | (1 << 8);
    let mut p = dl;
    while p + 5 <= end {
        if d[p] != 0x61 {
            p += 1;
            continue;
        }
        let reg = d[p + 1];
        let raw = be32(d, p + 1) & 0x00ff_ffff;
        p += 5;
        if reg == 0xfe {
            mask = raw;
            continue;
        }
        let m = std::mem::replace(&mut mask, 0x00ff_ffff);
        let v = raw & m;
        match reg {
            0x40 => {
                ps.z_test = v & 1 != 0;
                ps.z_func = ((v >> 1) & 7) as u8;
                ps.z_write = v & 0x10 != 0;
            }
            0x41 => {
                blend_reg = (blend_reg & !m) | v;
                let v = blend_reg;
                ps.blend = v & 1 != 0;
                ps.color_update = v & 8 != 0;
                ps.alpha_update = v & 0x10 != 0;
                ps.dst_factor = ((v >> 5) & 7) as u8;
                ps.src_factor = ((v >> 8) & 7) as u8;
                ps.subtract = v & 0x800 != 0;
            }
            0xf3 => {
                ps.alpha_test =
                    (((v >> 16) & 7) as u8, v as u8, ((v >> 22) & 3) as u8, ((v >> 19) & 7) as u8, (v >> 8) as u8);
            }
            // TEV color registers come as (RA, BG) pairs at 0xE0 + 2n; bit 23 selects konst.
            0xe0..=0xe7 => {
                let n = ((reg - 0xe0) / 2) as usize;
                let lo = (v & 0x7ff) as i16;
                let hi = ((v >> 12) & 0x7ff) as i16;
                let sx = |x: i16| (x << 5) >> 5; // sign-extend 11 bits
                let is_konst = v & 0x80_0000 != 0;
                let is_ra = (reg - 0xe0) % 2 == 0;
                if is_konst {
                    let k = &mut konst[n];
                    if is_ra { (k[0], k[3]) = (lo as u8, hi as u8) } else { (k[2], k[1]) = (lo as u8, hi as u8) }
                } else {
                    let t = &mut tev[n];
                    if is_ra { (t[0], t[3]) = (sx(lo), sx(hi)) } else { (t[2], t[1]) = (sx(lo), sx(hi)) }
                }
            }
            _ => {}
        }
    }
    (ps, tev, konst)
}

#[allow(clippy::too_many_arguments)]
fn decode_object(
    d: &[u8],
    o: usize,
    version: u32,
    positions: &[Array],
    normals: &[Array],
    colors: &[ColorArray],
    uvs: &[Array],
    bones: &[Bone],
    node_to_bone: &[Option<usize>],
) -> Result<Mesh> {
    let single_node = be32(d, o + 8) as i32;
    let vcd = (be32(d, o + 0xc) as u64) | (be32(d, o + 0x10) as u64) << 32;
    let prim_size = be32(d, o + 0x28) as usize;
    let prim = rel(o + 0x24, be32(d, o + 0x2c));
    let ids = |off: usize| be16(d, o + off) as i16;
    let (pos_id, nrm_id, clr_id, uv_id) = (ids(0x48), ids(0x4a), ids(0x4c), ids(0x50));
    let wt_off = if version >= 10 { 0x64 } else { 0x60 };
    let weight_table: Vec<u16> = {
        let t = rel(o, be32(d, o + wt_off));
        let n = be32(d, t) as usize;
        (0..n).map(|i| be16(d, t + 4 + i * 2)).collect()
    };
    let bone_of_node = |node: i32| -> Option<usize> {
        usize::try_from(node).ok().and_then(|n| node_to_bone.get(n).copied().flatten())
    };

    // Attribute layout from the CP vertex descriptor.
    // Low word: bit0 PNMTXIDX, bits1-8 TEXnMTXIDX (all direct u8), then 2-bit POS, NRM, CLR0, CLR1.
    // High word: 2 bits per TEX0..TEX7.
    let attr = |shift: u32| ((vcd >> shift) & 3) as u8;
    let has_pnmtx = vcd & 1 != 0;
    let texmtx_count = (1..=8).filter(|b| vcd >> b & 1 != 0).count();
    let layout: Vec<(char, u8)> = [
        ('p', attr(9)),
        ('n', attr(11)),
        ('c', attr(13)),
        ('C', attr(15)),
    ]
    .into_iter()
    .chain((0..8).map(|t| (char::from(b'0' + t as u8), attr(32 + t * 2))))
    .filter(|a| a.1 != 0)
    .collect();
    ensure!(
        layout.iter().all(|a| a.1 >= 2),
        "direct vertex attributes are not supported"
    );

    let pos_arr = positions.get(pos_id as usize).context("object has no position array")?;
    let nrm_arr = usize::try_from(nrm_id).ok().and_then(|i| normals.get(i));
    let clr_arr = usize::try_from(clr_id).ok().and_then(|i| colors.get(i));
    let uv_arr = usize::try_from(uv_id).ok().and_then(|i| uvs.get(i));

    let mut mesh = Mesh::default();
    let (mut local_pos, mut local_nrm) = (Vec::new(), Vec::new());
    let mut p = prim;
    let end = prim + prim_size;
    while p < end {
        let op = d[p];
        p += 1;
        match op {
            0x00 => {}
            0x08 => p += 5,
            0x10 => p += 4 + ((be16(d, p) as usize) + 1) * 4,
            0x20 | 0x28 | 0x30 | 0x38 => p += 4,
            0x80..=0xbf => {
                let count = be16(d, p) as usize;
                p += 2;
                let first = mesh.positions.len() as u32;
                for _ in 0..count {
                    let mut node = single_node;
                    if has_pnmtx {
                        let slot = d[p] as usize / 3;
                        node = weight_table.get(slot).map_or(-1, |&n| n as i32);
                        p += 1;
                    }
                    p += texmtx_count;
                    let (mut pos, mut nrm, mut clr, mut uv) = ([0.0; 3], None, None, None);
                    for &(a, fmt) in &layout {
                        let idx = if fmt == 2 {
                            p += 1;
                            d[p - 1] as usize
                        } else {
                            p += 2;
                            be16(d, p - 2) as usize
                        };
                        match a {
                            'p' => pos = pos_arr.read(d, idx)?,
                            'n' => nrm = nrm_arr.map(|n| n.read(d, idx)).transpose()?,
                            'c' => clr = clr_arr.map(|c| c.read(d, idx)).transpose()?,
                            '0' => uv = uv_arr.map(|u| u.read(d, idx)).transpose()?,
                            _ => {}
                        }
                    }
                    let bone = bone_of_node(node);
                    // Rigidly bound vertices are stored in bone space; envelope vertices
                    // are already in model space.
                    let world = bone.and_then(|b| bones.get(b)).map(|b| &b.world);
                    mesh.positions.push(world.map_or(pos, |m| mtx_apply(m, pos)));
                    if let Some(n) = nrm {
                        mesh.normals.push(world.map_or(n, |m| mtx_rotate(m, n)));
                    }
                    if let Some(c) = clr {
                        mesh.colors.push(c);
                    }
                    if let Some(t) = uv {
                        mesh.uvs.push([t[0], t[1]]);
                    }
                    mesh.vertex_bones.push(bone);
                    local_pos.push(pos);
                    if let Some(n) = nrm {
                        local_nrm.push(n);
                    }
                }
                let n = count as u32;
                match op & 0xf8 {
                    0x80 | 0x88 => {
                        for q in (0..n / 4 * 4).step_by(4) {
                            let b = first + q;
                            mesh.indices.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
                        }
                    }
                    0x90 => mesh.indices.extend(first..first + n / 3 * 3),
                    0x98 => {
                        for i in 2..n {
                            let b = first + i;
                            if i % 2 == 0 {
                                mesh.indices.extend([b - 2, b - 1, b]);
                            } else {
                                mesh.indices.extend([b - 1, b - 2, b]);
                            }
                        }
                    }
                    0xa0 => {
                        for i in 2..n {
                            mesh.indices.extend([first, first + i - 1, first + i]);
                        }
                    }
                    _ => {} // lines and points are not rendered
                }
            }
            op => bail!("unknown display list opcode {op:#x}"),
        }
    }
    if let Some(&Some(b)) = mesh.vertex_bones.first() {
        if mesh.vertex_bones.iter().all(|&x| x == Some(b)) {
            mesh.rigid = Some((b, local_pos, local_nrm));
        }
    }
    Ok(mesh)
}
