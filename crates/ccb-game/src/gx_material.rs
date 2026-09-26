//! A Bevy material that reproduces a G3D material: TEV combiner stages (see `gx.wgsl`),
//! alpha test, blending, culling and depth state.

use bevy::{
    asset::embedded_asset,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, ColorWrites, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};
use wii_formats::mdl0;

pub struct GxMaterialPlugin;

impl Plugin for GxMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "gx.wgsl");
        app.add_plugins(MaterialPlugin::<GxMaterial>::default());
    }
}

#[derive(Clone, Copy, ShaderType, Debug)]
pub struct GxParams {
    pub info: UVec4,
    pub color_env: [UVec4; 4],
    pub alpha_env: [UVec4; 4],
    pub order: [UVec4; 4],
    pub regs: [Vec4; 4],
    pub konst: [Vec4; 4],
    /// Texture-coordinate matrix rows: `uv' = (dot(row0.xyz, (u, v, 1)), dot(row1.xyz, (u, v, 1)))`.
    pub tex_mtx: [Vec4; 2],
    /// Lighting channel COLOR0A0 (the rasterized color when it comes from the material).
    pub chan0: Vec4,
}

impl Default for GxParams {
    fn default() -> Self {
        Self {
            info: UVec4::ZERO,
            color_env: [UVec4::ZERO; 4],
            alpha_env: [UVec4::ZERO; 4],
            order: [UVec4::ZERO; 4],
            regs: [Vec4::ZERO; 4],
            konst: [Vec4::ZERO; 4],
            tex_mtx: [Vec4::new(1.0, 0.0, 0.0, 0.0), Vec4::new(0.0, 1.0, 0.0, 0.0)],
            chan0: Vec4::ONE,
        }
    }
}

impl GxParams {
    /// Sets the texture matrix from an NW4R texture SRT (translate s/t, rotate degrees,
    /// scale s/t); scale and rotation pivot on the texture center.
    pub fn set_tex_srt(&mut self, srt: [f32; 5]) {
        let [tx, ty, rot, sx, sy] = srt;
        let (sin, cos) = rot.to_radians().sin_cos();
        let (a, b, c, d) = (sx * cos, -sy * sin, sx * sin, sy * cos);
        // uv' = M * (uv - 0.5) + 0.5 + t
        let ox = 0.5 - (a * 0.5 + b * 0.5) + tx;
        let oy = 0.5 - (c * 0.5 + d * 0.5) + ty;
        self.tex_mtx = [Vec4::new(a, b, ox, 0.0), Vec4::new(c, d, oy, 0.0)];
    }
}

/// Pipeline state that needs a separate render pipeline.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GxKey {
    /// GX cull mode: 0 none, 1 front, 2 back.
    pub cull: u32,
    pub depth_write: u32,
    pub color_write: u32,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(GxKey)]
pub struct GxMaterial {
    #[uniform(0)]
    pub params: GxParams,
    #[texture(1)]
    #[sampler(2)]
    pub tex0: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    pub tex1: Option<Handle<Image>>,
    pub alpha_mode: AlphaMode,
    pub key: GxKey,
}

impl From<&GxMaterial> for GxKey {
    fn from(m: &GxMaterial) -> Self {
        m.key
    }
}

impl Material for GxMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://ccb_game/gx.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let k = key.bind_group_data;
        descriptor.primitive.cull_mode = match k.cull {
            1 => Some(Face::Front),
            2 => Some(Face::Back),
            _ => None,
        };
        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_write_enabled = Some(k.depth_write != 0);
        }
        if k.color_write == 0 {
            if let Some(frag) = descriptor.fragment.as_mut() {
                for t in frag.targets.iter_mut().flatten() {
                    t.write_mask = ColorWrites::empty();
                }
            }
        }
        Ok(())
    }
}

impl GxMaterial {
    /// A flat, opaque color (unlit) with the given GX cull mode.
    pub fn flat(color: Vec4, cull: u32) -> Self {
        let mut p = GxParams::default();
        // One stage: color = C1, alpha = A1 (inputs 4 C1 / 2 A1, the rest zero).
        p.info = UVec4::new(1, 7 | 7 << 3, 0, 0);
        p.color_env[0].x = 4 | 15 << 4 | 15 << 8 | 15 << 12 | 1 << 19;
        p.alpha_env[0].x = 7 << 4 | 7 << 7 | 7 << 10 | 2 << 13 | 1 << 19;
        p.order[0].x = 0x1f << 16;
        p.regs[2] = color;
        Self { params: p, tex0: None, tex1: None, alpha_mode: AlphaMode::Opaque, key: GxKey { cull, depth_write: 1, color_write: 1 } }
    }

    /// `env` flags the texture slots that use environment mapping.
    pub fn from_mdl0(mat: &mdl0::Material, translucent: bool, textures: [Option<Handle<Image>>; 2], env: [bool; 2]) -> Self {
        let pe = &mat.pixel;
        let mut p = GxParams::default();
        let n = mat.tev_stages.len().min(16);
        let (at0, ref0, op, at1, ref1) = pe.alpha_test;
        p.info = UVec4::new(
            n as u32,
            at0 as u32 | (at1 as u32) << 3 | (op as u32) << 6 | (ref0 as u32) << 8 | (ref1 as u32) << 16,
            env[0] as u32 | (env[1] as u32) << 1,
            // bit0/bit1: COLOR0 / ALPHA0 come from the material color (GX_SRC_REG) instead of
            // vertex colors; other materials (layouts, lines) keep using vertex colors.
            (mat.channels[0].color_ctrl & 1 ^ 1) | (mat.channels[0].alpha_ctrl & 1 ^ 1) << 1,
        );
        // Without scene lights the lit channels come out at their material color.
        let c = mat.channels[0].mat_color;
        p.chan0 = Vec4::new(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32) / 255.0;
        for (i, st) in mat.tev_stages.iter().take(16).enumerate() {
            p.color_env[i / 4][i % 4] = st.color_env;
            p.alpha_env[i / 4][i % 4] = st.alpha_env;
            p.order[i / 4][i % 4] = (st.tex_map as u32 & 7)
                | (st.tex_enabled as u32) << 4
                | (st.channel as u32 & 7) << 5
                | (st.konst_color_sel as u32) << 8
                | (st.konst_alpha_sel as u32) << 16;
        }
        for i in 0..4 {
            let r = mat.tev_colors[i];
            p.regs[i] = Vec4::new(r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32) / 255.0;
            let k = mat.konst_colors[i];
            p.konst[i] = Vec4::new(k[0] as f32, k[1] as f32, k[2] as f32, k[3] as f32) / 255.0;
        }
        let writes_color = pe.color_update && !(pe.blend && pe.src_factor == 0 && pe.dst_factor == 1);
        // Blending follows the pixel state; opaque-pass decals that blend without depth writes
        // (the bomb's feathered stripes) blend too.
        let alpha_mode = if pe.blend && writes_color && (translucent || !pe.z_write) {
            if pe.dst_factor == 1 { AlphaMode::Add } else { AlphaMode::Blend }
        } else if pe.alpha_test != (7, 0, 0, 7, 0) {
            AlphaMode::Mask(0.5) // the shader does the real alpha test
        } else {
            AlphaMode::Opaque
        };
        let [tex0, tex1] = textures;
        Self {
            params: p,
            tex0,
            tex1,
            alpha_mode,
            key: GxKey { cull: mat.cull.min(3), depth_write: pe.z_write as u32, color_write: writes_color as u32 },
        }
    }
}
