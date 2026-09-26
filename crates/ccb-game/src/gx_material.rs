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

#[derive(Clone, Copy, Default, ShaderType, Debug)]
pub struct GxParams {
    pub info: UVec4,
    pub color_env: [UVec4; 4],
    pub alpha_env: [UVec4; 4],
    pub order: [UVec4; 4],
    pub regs: [Vec4; 4],
    pub konst: [Vec4; 4],
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
    pub fn from_mdl0(mat: &mdl0::Material, translucent: bool, textures: [Option<Handle<Image>>; 2]) -> Self {
        let pe = &mat.pixel;
        let mut p = GxParams::default();
        let n = mat.tev_stages.len().min(16);
        let (at0, ref0, op, at1, ref1) = pe.alpha_test;
        p.info = UVec4::new(
            n as u32,
            at0 as u32 | (at1 as u32) << 3 | (op as u32) << 6 | (ref0 as u32) << 8 | (ref1 as u32) << 16,
            0,
            0,
        );
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
        let alpha_mode = if translucent && pe.blend && writes_color {
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
