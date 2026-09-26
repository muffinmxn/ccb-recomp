//! A tiny CPU rasterizer used to eyeball parsed models without a GPU: orthographic
//! view down -Z, nearest-neighbour texturing, z-buffer, alpha test and simple blending.

use std::collections::HashMap;

use wii_formats::{brres, mdl0};

pub struct Texture {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

/// Decodes every texture in the given archives, keyed by name.
pub fn texture_pool(archives: &[brres::Brres]) -> HashMap<String, Texture> {
    let mut pool = HashMap::new();
    for b in archives {
        for (name, t) in b.folder("Textures(NW4R)") {
            let Ok(tex) = brres::Tex0::parse(t) else { continue };
            let plt = b.folder("Palettes(NW4R)").find(|(n, _)| *n == name).map(|(_, p)| p);
            if let Ok(rgba) = tex.decode(t, plt) {
                pool.insert(name.to_string(), Texture { w: tex.width, h: tex.height, rgba });
            }
        }
    }
    pool
}

fn wrap(v: f32, mode: u32) -> f32 {
    match mode {
        0 => v.clamp(0.0, 0.9999),
        2 => {
            let f = v.rem_euclid(2.0);
            if f > 1.0 { 2.0 - f } else { f }.min(0.9999)
        }
        _ => v.rem_euclid(1.0),
    }
}

pub fn render(models: &[mdl0::Model], textures: &HashMap<String, Texture>, width: usize) -> (usize, usize, Vec<u8>) {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for m in models {
        for mesh in &m.meshes {
            for p in &mesh.positions {
                for i in 0..3 {
                    lo[i] = lo[i].min(p[i]);
                    hi[i] = hi[i].max(p[i]);
                }
            }
        }
    }
    let span_x = (hi[0] - lo[0]).max(1e-3);
    let span_y = (hi[1] - lo[1]).max(1e-3);
    let height = ((width as f32 * span_y / span_x).ceil() as usize).clamp(16, 4096);
    let scale = width as f32 / span_x;
    let mut color = vec![40u8; width * height * 4];
    color.chunks_mut(4).for_each(|c| c[3] = 255);
    let mut depth = vec![f32::MIN; width * height];

    // Opaque first, then translucent sorted by priority.
    let mut draws: Vec<(&mdl0::Model, &mdl0::Mesh)> =
        models.iter().flat_map(|m| m.meshes.iter().map(move |x| (m, x))).collect();
    draws.sort_by_key(|(_, m)| (m.translucent, m.draw_priority));

    for (model, mesh) in draws {
        let mat = model.materials.get(mesh.material);
        let pe = mat.map(|m| m.pixel).unwrap_or_default();
        let tref = mat.and_then(|m| m.textures.first());
        let tex = tref.and_then(|t| textures.get(&t.texture));
        let wrap_mode = tref.map_or([1, 1], |t| t.wrap);
        let proj = |p: [f32; 3]| [(p[0] - lo[0]) * scale, (hi[1] - p[1]) * scale, p[2]];
        for tri in mesh.indices.chunks_exact(3) {
            let v: Vec<[f32; 3]> = tri.iter().map(|&i| proj(mesh.positions[i as usize])).collect();
            let area = (v[1][0] - v[0][0]) * (v[2][1] - v[0][1]) - (v[2][0] - v[0][0]) * (v[1][1] - v[0][1]);
            if area.abs() < 1e-9 {
                continue;
            }
            let minx = v.iter().map(|p| p[0]).fold(f32::MAX, f32::min).max(0.0) as usize;
            let maxx = (v.iter().map(|p| p[0]).fold(f32::MIN, f32::max).ceil() as usize).min(width - 1);
            let miny = v.iter().map(|p| p[1]).fold(f32::MAX, f32::min).max(0.0) as usize;
            let maxy = (v.iter().map(|p| p[1]).fold(f32::MIN, f32::max).ceil() as usize).min(height - 1);
            for y in miny..=maxy {
                for x in minx..=maxx {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let e = |a: &[f32; 3], b: &[f32; 3]| (b[0] - a[0]) * (py - a[1]) - (px - a[0]) * (b[1] - a[1]);
                    let w0 = e(&v[1], &v[2]) / area;
                    let w1 = e(&v[2], &v[0]) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = w0 * v[0][2] + w1 * v[1][2] + w2 * v[2][2];
                    let di = y * width + x;
                    if pe.z_test && z < depth[di] - 1e-4 {
                        continue;
                    }
                    let lerp = |f: &dyn Fn(usize) -> f32| w0 * f(0) + w1 * f(1) + w2 * f(2);
                    let mut c = [1.0f32; 4];
                    if !mesh.colors.is_empty() {
                        for (k, ck) in c.iter_mut().enumerate() {
                            *ck = lerp(&|i| mesh.colors[tri[i] as usize][k] as f32 / 255.0);
                        }
                    }
                    if let (Some(t), false) = (tex, mesh.uvs.is_empty()) {
                        let u = wrap(lerp(&|i| mesh.uvs[tri[i] as usize][0]), wrap_mode[0]);
                        let vv = wrap(lerp(&|i| mesh.uvs[tri[i] as usize][1]), wrap_mode[1]);
                        let ti = ((vv * t.h as f32) as usize * t.w + (u * t.w as f32) as usize) * 4;
                        for (k, ck) in c.iter_mut().enumerate() {
                            *ck *= t.rgba[ti + k] as f32 / 255.0;
                        }
                    }
                    if !pe.alpha_passes((c[3] * 255.0) as u8) {
                        continue;
                    }
                    if pe.color_update {
                        let a = c[3];
                        let factor = |f: u8, src: f32| match f {
                            0 => 0.0,
                            1 => 1.0,
                            2 => src,
                            3 => 1.0 - src,
                            4 => a,
                            5 => 1.0 - a,
                            _ => 1.0, // destination alpha: framebuffer has none
                        };
                        for k in 0..3 {
                            let dst = color[di * 4 + k] as f32 / 255.0;
                            let out = if pe.blend {
                                c[k] * factor(pe.src_factor, c[k]) + dst * factor(pe.dst_factor, c[k])
                            } else {
                                c[k]
                            };
                            color[di * 4 + k] = (out.clamp(0.0, 1.0) * 255.0) as u8;
                        }
                    }
                    if pe.z_write {
                        depth[di] = z;
                    }
                }
            }
        }
    }
    (width, height, color)
}
