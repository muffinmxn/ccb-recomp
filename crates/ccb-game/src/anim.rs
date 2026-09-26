//! Plays G3D bone animations on spawned models: CHR0 (transforms) and VIS0 (visibility),
//! paired by clip name like the original's animation sets (`chick__blink`, `plant__bite`, ...).

use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use bevy::prelude::*;
use wii_formats::{brres::Brres, chr0::Chr0, vis0::Vis0};

use crate::g3d::{bone_transform, ModelInstance};

/// The Wii game runs its animations at 60 frames per second.
pub const ANIM_FPS: f32 = 60.0;

/// A named animation: bone transforms and/or bone visibility.
#[derive(Debug)]
pub struct Clip {
    pub name: String,
    pub chr: Option<Chr0>,
    pub vis: Option<Vis0>,
}

impl Clip {
    pub fn frames(&self) -> u16 {
        self.chr.as_ref().map(|c| c.frames).or(self.vis.as_ref().map(|v| v.frames)).unwrap_or(1)
    }

    pub fn looping(&self) -> bool {
        self.chr.as_ref().map(|c| c.looping).or(self.vis.as_ref().map(|v| v.looping)).unwrap_or(false)
    }
}

/// All clips in an archive, pairing CHR0 and VIS0 animations with the same name.
pub fn load_clips(b: &Brres) -> Result<Vec<Arc<Clip>>> {
    let mut map: HashMap<String, Clip> = HashMap::new();
    for c in b.bone_animations()? {
        let name = c.name.clone();
        map.entry(name.clone()).or_insert_with(|| Clip { name, chr: None, vis: None }).chr = Some(c);
    }
    for v in b.vis_animations()? {
        let name = v.name.clone();
        map.entry(name.clone()).or_insert_with(|| Clip { name, chr: None, vis: None }).vis = Some(v);
    }
    Ok(map.into_values().map(Arc::new).collect())
}

/// Plays `queue` in order; the last entry keeps looping if it is a looping animation,
/// otherwise it holds its last frame.
#[derive(Component)]
pub struct BoneAnimator {
    pub queue: Vec<Arc<Clip>>,
    pub index: usize,
    pub frame: f32,
}

impl BoneAnimator {
    pub fn new(queue: Vec<Arc<Clip>>) -> Self {
        Self { queue, index: 0, frame: 0.0 }
    }
}

pub struct AnimPlugin;

impl Plugin for AnimPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (animate_bones, apply_visibility).chain());
    }
}

fn animate_bones(time: Res<Time>, mut models: Query<(&mut ModelInstance, &mut BoneAnimator)>, mut bones: Query<&mut Transform>) {
    for (mut inst, mut anim) in &mut models {
        let Some(current) = anim.queue.get(anim.index).cloned() else { continue };
        anim.frame += time.delta_secs() * ANIM_FPS;
        let len = current.frames().max(1) as f32;
        if anim.frame >= len {
            if anim.index + 1 < anim.queue.len() {
                anim.index += 1;
                anim.frame = 0.0;
            } else if current.looping() {
                anim.frame %= len;
            } else {
                anim.frame = len;
            }
        }
        let clip = anim.queue[anim.index].clone();
        let f = anim.frame;
        if let Some(chr) = &clip.chr {
            for track in &chr.tracks {
                let Some(i) = inst.bone_names.iter().position(|n| *n == track.bone) else { continue };
                let bind = inst.bind[i];
                let eval = |ch: &Option<[wii_formats::chr0::Channel; 3]>, fallback: [f32; 3]| -> [f32; 3] {
                    ch.as_ref().map_or(fallback, |c| [c[0].eval(f), c[1].eval(f), c[2].eval(f)])
                };
                let t = bone_transform(eval(&track.scale, bind[0]), eval(&track.rotation, bind[1]), eval(&track.translation, bind[2]));
                if let Ok(mut tr) = bones.get_mut(inst.bones[i]) {
                    *tr = t;
                }
            }
        }
        if let Some(vis) = &clip.vis {
            for (bone, track) in &vis.tracks {
                if let Some(i) = inst.bone_names.iter().position(|n| n == bone) {
                    let v = track.visible(f);
                    if inst.bone_visible[i] != v {
                        inst.bone_visible[i] = v;
                    }
                }
            }
        }
    }
}

/// A mesh is drawn only while its draw bone is visible (NW4R's per-bone visibility).
fn apply_visibility(models: Query<&ModelInstance, Changed<ModelInstance>>, mut vis: Query<&mut Visibility>) {
    for inst in &models {
        for &(bone, mesh) in &inst.meshes {
            if let Ok(mut v) = vis.get_mut(mesh) {
                let want = if inst.bone_visible.get(bone).copied().unwrap_or(true) { Visibility::Inherited } else { Visibility::Hidden };
                if *v != want {
                    *v = want;
                }
            }
        }
    }
}
