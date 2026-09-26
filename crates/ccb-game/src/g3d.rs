//! Turns parsed G3D (MDL0/TEX0) data into Bevy meshes and materials.

use std::collections::HashMap;

use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    mesh::{
        skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
        Indices, PrimitiveTopology, VertexAttributeValues,
    },
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use wii_formats::{brres::{Brres, Tex0}, mdl0};

use crate::gx_material::GxMaterial;

fn mat4(m: &mdl0::Mtx34) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(m[0][0], m[1][0], m[2][0], 0.0),
        Vec4::new(m[0][1], m[1][1], m[2][1], 0.0),
        Vec4::new(m[0][2], m[1][2], m[2][2], 0.0),
        Vec4::new(m[0][3], m[1][3], m[2][3], 1.0),
    )
}

/// Decodes every texture in the archives into Bevy images, keyed by texture name.
pub fn load_textures(archives: &[&Brres], images: &mut Assets<Image>) -> HashMap<String, (Handle<Image>, [u32; 2])> {
    let mut out = HashMap::new();
    for b in archives {
        for (name, t) in b.folder("Textures(NW4R)") {
            let Ok(tex) = Tex0::parse(t) else { continue };
            let plt = b.folder("Palettes(NW4R)").find(|(n, _)| *n == name).map(|(_, p)| p);
            match tex.decode(t, plt) {
                Ok(rgba) => {
                    let img = Image::new(
                        Extent3d { width: tex.width as u32, height: tex.height as u32, depth_or_array_layers: 1 },
                        TextureDimension::D2,
                        rgba,
                        TextureFormat::Rgba8Unorm,
                        RenderAssetUsages::RENDER_WORLD,
                    );
                    out.insert(name.to_string(), (images.add(img), [0, 0]));
                }
                Err(e) => warn!("texture {name}: {e}"),
            }
        }
    }
    out
}

fn address_mode(gx: u32) -> ImageAddressMode {
    match gx {
        0 => ImageAddressMode::ClampToEdge,
        2 => ImageAddressMode::MirrorRepeat,
        _ => ImageAddressMode::Repeat,
    }
}

/// Builds one Bevy mesh per MDL0 draw. GX treats clockwise triangles as front-facing,
/// so indices are reversed to match Bevy's counter-clockwise convention.
pub fn build_mesh(m: &mdl0::Mesh) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, m.positions.clone());
    if m.normals.len() == m.positions.len() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, m.normals.clone());
    }
    if m.uvs.len() == m.positions.len() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, m.uvs.clone());
    }
    if m.colors.len() == m.positions.len() {
        let colors: Vec<[f32; 4]> = m
            .colors
            .iter()
            // Raw gamma-space values: the GX shader does its math in gamma space.
            .map(|c| c.map(|v| v as f32 / 255.0))
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    let idx: Vec<u32> = m.indices.chunks_exact(3).flat_map(|t| [t[0], t[2], t[1]]).collect();
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// Builds the GX material for an MDL0 material, applying each texture layer's wrap mode.
/// Returns `None` for materials GX would cull entirely.
pub fn build_material(
    mat: &mdl0::Material,
    translucent: bool,
    textures: &HashMap<String, (Handle<Image>, [u32; 2])>,
    images: &mut Assets<Image>,
) -> Option<GxMaterial> {
    if mat.cull == 3 {
        return None;
    }
    let mut layers = [None, None];
    let mut env = [false; 2];
    for (i, t) in mat.textures.iter().enumerate().take(2) {
        env[i] = t.map_mode == 1;
    }
    for (slot, t) in layers.iter_mut().zip(&mat.textures) {
        let Some((handle, _)) = textures.get(&t.texture) else { continue };
        if let Some(mut img) = images.get_mut(handle) {
            img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: address_mode(t.wrap[0]),
                address_mode_v: address_mode(t.wrap[1]),
                ..ImageSamplerDescriptor::linear()
            });
        }
        *slot = Some(handle.clone());
    }
    Some(GxMaterial::from_mdl0(mat, translucent, layers, env))
}

/// Per-bone entities of a spawned model, used by the animator.
#[derive(Component)]
pub struct ModelInstance {
    pub bone_names: Vec<String>,
    pub bones: Vec<Entity>,
    /// Bind-pose (scale, rotation in degrees, translation) per bone.
    pub bind: Vec<[[f32; 3]; 3]>,
    /// Per-bone visibility (bone flag 0x100, then driven by VIS0 clips).
    pub bone_visible: Vec<bool>,
    /// (draw bone, mesh entity) for every spawned mesh.
    pub meshes: Vec<(usize, Entity)>,
}

pub fn bone_transform(s: [f32; 3], r_deg: [f32; 3], t: [f32; 3]) -> Transform {
    // G3D applies rotations X, then Y, then Z.
    let r = Quat::from_euler(EulerRot::ZYX, r_deg[2].to_radians(), r_deg[1].to_radians(), r_deg[0].to_radians());
    Transform { translation: Vec3::from(t), rotation: r, scale: Vec3::from(s) }
}

/// A `tweak` that keeps every mesh unchanged.
pub fn no_tweak(_: &mdl0::Mesh, _: &str, _: &mut GxMaterial) -> bool {
    true
}

/// The inverse bind poses of a model's bones, if it has skinned (multi-bone) meshes.
pub fn inverse_binds(model: &mdl0::Model, store: &mut Assets<SkinnedMeshInverseBindposes>) -> Option<Handle<SkinnedMeshInverseBindposes>> {
    model
        .meshes
        .iter()
        .any(|m| m.rigid.is_none() && !m.weights.is_empty())
        .then(|| store.add(SkinnedMeshInverseBindposes::from(model.bones.iter().map(|b| mat4(&b.inverse)).collect::<Vec<_>>())))
}

/// Spawns a model under `parent`: one entity per bone (in hierarchy) and one per mesh.
/// `tweak` can adjust each mesh's material (or return false to skip the mesh).
/// Meshes bound rigidly to a single bone are parented to it so bone animation moves them.
pub fn spawn_model(
    commands: &mut Commands,
    parent: Entity,
    model: &mdl0::Model,
    textures: &HashMap<String, (Handle<Image>, [u32; 2])>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<GxMaterial>,
    images: &mut Assets<Image>,
    tweak: &dyn Fn(&mdl0::Mesh, &str, &mut GxMaterial) -> bool,
) -> Entity {
    spawn_model_skinned(commands, parent, model, textures, meshes, materials, images, tweak, None)
}

/// Like [`spawn_model`], with the asset store needed for skinned (multi-bone) meshes.
#[allow(clippy::too_many_arguments)]
pub fn spawn_model_skinned(
    commands: &mut Commands,
    parent: Entity,
    model: &mdl0::Model,
    textures: &HashMap<String, (Handle<Image>, [u32; 2])>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<GxMaterial>,
    images: &mut Assets<Image>,
    tweak: &dyn Fn(&mdl0::Mesh, &str, &mut GxMaterial) -> bool,
    inverse_binds: Option<Handle<SkinnedMeshInverseBindposes>>,
) -> Entity {
    let root = commands
        .spawn((Name::new(model.name.clone()), Transform::default(), Visibility::default()))
        .id();
    commands.entity(parent).add_child(root);
    let bones: Vec<Entity> = model
        .bones
        .iter()
        .map(|b| {
            commands
                .spawn((Name::new(b.name.clone()), bone_transform(b.scale, b.rotation, b.translation), Visibility::default()))
                .id()
        })
        .collect();
    for (i, b) in model.bones.iter().enumerate() {
        let p = b.parent.map_or(root, |p| bones[p]);
        commands.entity(p).add_child(bones[i]);
    }
    let bone_visible: Vec<bool> = model.bones.iter().map(|b| b.flags & 0x100 != 0).collect();
    // Skinned (envelope) meshes deform with the bones via `inverse_binds` (see [`inverse_binds`]).
    let mut mesh_entities = Vec::new();
    for m in &model.meshes {
        let Some(mat) = model.materials.get(m.material) else { continue };
        let Some(mut material) = build_material(mat, m.translucent, textures, images) else { continue };
        if !tweak(m, &mat.name, &mut material) {
            continue;
        }
        let (owner, mesh) = match &m.rigid {
            Some((bone, pos, nrm)) => {
                let local = mdl0::Mesh { positions: pos.clone(), normals: nrm.clone(), ..m.clone() };
                (bones[*bone], build_mesh(&local))
            }
            None => {
                let mut mesh = build_mesh(m);
                if m.weights.len() == m.positions.len() {
                    let joints: Vec<[u16; 4]> = m.weights.iter().map(|w| w.map(|(b, _)| b)).collect();
                    let weights: Vec<[f32; 4]> = m
                        .weights
                        .iter()
                        .map(|w| {
                            let sum: f32 = w.iter().map(|x| x.1).sum();
                            w.map(|(_, x)| if sum > 0.0 { x / sum } else { 0.0 })
                        })
                        .collect();
                    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(joints));
                    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
                }
                (root, mesh)
            }
        };
        let visible = bone_visible.get(m.bone).copied().unwrap_or(true);
        let child = commands
            .spawn((
                Name::new(format!("{}/{}", model.name, m.name)),
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(materials.add(material)),
                Transform::default(),
                if visible { Visibility::Inherited } else { Visibility::Hidden },
            ))
            .id();
        commands.entity(owner).add_child(child);
        if m.rigid.is_none() {
            if let Some(ib) = &inverse_binds {
                // Bind-pose bounds don't follow the bones, so don't cull skinned meshes.
                commands.entity(child).insert((
                    SkinnedMesh { inverse_bindposes: ib.clone(), joints: bones.clone() },
                    bevy::camera::visibility::NoFrustumCulling,
                ));
            }
        }
        mesh_entities.push((m.bone, child));
    }
    commands.entity(root).insert(ModelInstance {
        bone_names: model.bones.iter().map(|b| b.name.clone()).collect(),
        bones,
        bind: model.bones.iter().map(|b| [b.scale, b.rotation, b.translation]).collect(),
        bone_visible,
        meshes: mesh_entities,
    });
    root
}
