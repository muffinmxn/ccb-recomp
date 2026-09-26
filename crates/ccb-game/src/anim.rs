//! Plays CHR0 bone animations on spawned models.

use std::sync::Arc;

use bevy::prelude::*;
use wii_formats::chr0::Chr0;

use crate::g3d::{bone_transform, ModelInstance};

/// The Wii game runs its animations at 60 frames per second.
pub const ANIM_FPS: f32 = 60.0;

/// Plays `queue` in order; the last entry keeps looping if it is a looping animation.
#[derive(Component)]
pub struct BoneAnimator {
    pub queue: Vec<Arc<Chr0>>,
    pub index: usize,
    pub frame: f32,
}

impl BoneAnimator {
    pub fn new(queue: Vec<Arc<Chr0>>) -> Self {
        Self { queue, index: 0, frame: 0.0 }
    }
}

pub struct AnimPlugin;

impl Plugin for AnimPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, animate_bones);
    }
}

fn animate_bones(time: Res<Time>, mut models: Query<(&ModelInstance, &mut BoneAnimator)>, mut bones: Query<&mut Transform>) {
    for (inst, mut anim) in &mut models {
        let Some(current) = anim.queue.get(anim.index).cloned() else { continue };
        anim.frame += time.delta_secs() * ANIM_FPS;
        let len = current.frames.max(1) as f32;
        if anim.frame >= len {
            if anim.index + 1 < anim.queue.len() {
                anim.index += 1;
                anim.frame = 0.0;
            } else if current.looping {
                anim.frame %= len;
            } else {
                anim.frame = len;
            }
        }
        let clip = anim.queue[anim.index].clone();
        let f = anim.frame;
        for track in &clip.tracks {
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
}
