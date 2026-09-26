//! Turns parsed G3D (MDL0/TEX0) data into Bevy meshes and materials.

use std::collections::HashMap;

use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{Extent3d, Face, TextureDimension, TextureFormat},
};
use wii_formats::{brres::{Brres, Tex0}, mdl0};

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
                        TextureFormat::Rgba8UnormSrgb,
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
            .map(|c| Color::srgba_u8(c[0], c[1], c[2], c[3]).to_linear().to_f32_array())
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    let idx: Vec<u32> = m.indices.chunks_exact(3).flat_map(|t| [t[0], t[2], t[1]]).collect();
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// Approximates a GX material with an unlit `StandardMaterial`. Returns `None` for
/// materials that don't write color (depth/stencil masks).
pub fn build_material(
    mat: &mdl0::Material,
    translucent: bool,
    textures: &HashMap<String, (Handle<Image>, [u32; 2])>,
    images: &mut Assets<Image>,
) -> Option<StandardMaterial> {
    let pe = &mat.pixel;
    if !pe.color_update || (pe.blend && pe.src_factor == 0 && pe.dst_factor == 1) || mat.cull == 3 {
        return None;
    }
    let texture = mat.textures.first().and_then(|t| {
        let (handle, _) = textures.get(&t.texture)?;
        // Apply the layer's wrap modes to the image sampler.
        if let Some(mut img) = images.get_mut(handle) {
            img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: address_mode(t.wrap[0]),
                address_mode_v: address_mode(t.wrap[1]),
                ..ImageSamplerDescriptor::linear()
            });
        }
        Some(handle.clone())
    });
    let alpha_mode = if translucent && pe.blend {
        if pe.src_factor == 1 && pe.dst_factor == 1 { AlphaMode::Add } else { AlphaMode::Blend }
    } else if pe.alpha_test != (7, 0, 0, 7, 0) {
        AlphaMode::Mask(0.5)
    } else {
        AlphaMode::Opaque
    };
    Some(StandardMaterial {
        base_color_texture: texture,
        unlit: true,
        alpha_mode,
        cull_mode: match mat.cull {
            1 => Some(Face::Front),
            2 => Some(Face::Back),
            _ => None,
        },
        double_sided: mat.cull == 0,
        ..default()
    })
}

/// Per-bone entities of a spawned model, used by the animator.
#[derive(Component)]
pub struct ModelInstance {
    pub bone_names: Vec<String>,
    pub bones: Vec<Entity>,
    /// Bind-pose (scale, rotation in degrees, translation) per bone.
    pub bind: Vec<[[f32; 3]; 3]>,
}

pub fn bone_transform(s: [f32; 3], r_deg: [f32; 3], t: [f32; 3]) -> Transform {
    // G3D applies rotations X, then Y, then Z.
    let r = Quat::from_euler(EulerRot::ZYX, r_deg[2].to_radians(), r_deg[1].to_radians(), r_deg[0].to_radians());
    Transform { translation: Vec3::from(t), rotation: r, scale: Vec3::from(s) }
}

/// Spawns a model under `parent`: one entity per bone (in hierarchy) and one per mesh.
/// Meshes bound rigidly to a single bone are parented to it so bone animation moves them.
pub fn spawn_model(
    commands: &mut Commands,
    parent: Entity,
    model: &mdl0::Model,
    textures: &HashMap<String, (Handle<Image>, [u32; 2])>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
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
    for m in &model.meshes {
        let Some(mat) = model.materials.get(m.material) else { continue };
        let Some(material) = build_material(mat, m.translucent, textures, images) else { continue };
        let (owner, mesh) = match &m.rigid {
            Some((bone, pos, nrm)) => {
                let local = mdl0::Mesh { positions: pos.clone(), normals: nrm.clone(), ..m.clone() };
                (bones[*bone], build_mesh(&local))
            }
            None => (root, build_mesh(m)),
        };
        let child = commands
            .spawn((
                Name::new(format!("{}/{}", model.name, m.name)),
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(materials.add(material)),
                Transform::default(),
            ))
            .id();
        commands.entity(owner).add_child(child);
    }
    commands.entity(root).insert(ModelInstance {
        bone_names: model.bones.iter().map(|b| b.name.clone()).collect(),
        bones,
        bind: model.bones.iter().map(|b| [b.scale, b.rotation, b.translation]).collect(),
    });
    root
}
