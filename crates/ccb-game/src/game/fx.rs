//! Chick feedback: faces from the model's own clips (`chick__blink`, `chick__flashhit`,
//! `chick__low`, `chick__ufo`), a splash (`spratzel`) where a chick gets hit, and the blue soul
//! (`chickDead`) that floats up from a chick that dies.

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};

use super::{
    attack::{Attack, AttackState},
    chick::Chick,
    flow::GameAssets,
    rng::Rng,
    tuning::Tuning,
    AttackKind,
};
use crate::{anim::BoneAnimator, gx_material::GxMaterial, Screen};

/// `chickLowHpThreshold`: below this share of its health a chick looks worn out.
const LOW_HP: f32 = 0.25;
/// `titleChickBlinkTime`.
const BLINK: (f32, f32) = (0.3, 2.5);
/// How close a threat must be to scare a chick.
const FEAR_RANGE: f32 = 3.0;
const SOUL_SIZE: f32 = 1.4;
const SPLASH_SIZE: f32 = 1.3;

#[derive(Component)]
pub struct Soul {
    t: f32,
    material: Handle<GxMaterial>,
}

#[derive(Component)]
pub struct Splash {
    t: f32,
    material: Handle<GxMaterial>,
}

/// A quad `size` wide showing the u range `u0..u1` of its texture.
fn quad(size: f32, u0: f32, u1: f32) -> Mesh {
    let h = size / 2.0;
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[-h, h, 0.0], [h, h, 0.0], [-h, -h, 0.0], [h, -h, 0.0]]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[u0, 0.0], [u1, 0.0], [u0, 1.0], [u1, 1.0]]);
    m.insert_indices(Indices::U32(vec![0, 2, 1, 1, 2, 3]));
    m
}

/// Where each attack threatens, for the scared faces.
fn threats(attacks: &Query<&Attack>) -> Vec<(super::Team, Vec2)> {
    attacks
        .iter()
        .filter(|a| match (a.kind, a.state) {
            (AttackKind::Bomb, AttackState::Roll) => true,
            (AttackKind::Bomb | AttackKind::Weight, AttackState::Travel) => true,
            (AttackKind::Plant | AttackKind::Ghost | AttackKind::Octopus | AttackKind::Ufo, AttackState::Travel | AttackState::Prepare) => true,
            (AttackKind::Lightning, AttackState::Prepare) => true,
            _ => false,
        })
        .map(|a| (a.target(), Vec2::new(a.target_x, a.pos.y)))
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn chick_fx(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<Tuning>,
    assets: Res<GameAssets>,
    mut rng: ResMut<Rng>,
    attacks: Query<&Attack>,
    mut chicks: Query<(Entity, &mut Chick)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
) {
    let dt = time.delta_secs();
    let z = tuning.plane_z + 0.8;
    let danger = threats(&attacks);
    for (e, mut c) in &mut chicks {
        // A hit: a splash where it was struck.
        if c.hurt {
            c.hurt = false;
            if let Some(tex) = assets.texture("spratzel") {
                let material = materials.add(GxMaterial::sprite(tex));
                let spin = rng.range((0.0, std::f32::consts::TAU));
                commands.spawn((
                    Splash { t: 0.0, material: material.clone() },
                    Mesh3d(meshes.add(quad(SPLASH_SIZE, 0.0, 1.0))),
                    MeshMaterial3d(material),
                    Transform::from_xyz(c.pos.x, c.pos.y, z).with_rotation(Quat::from_rotation_z(spin)),
                    DespawnOnExit(Screen::Level),
                ));
            }
        }
        // Death: the chick's soul floats up.
        if c.dying.is_some() {
            if c.face != "dead" {
                c.face = "dead";
                if let Some(tex) = assets.texture("chickDead") {
                    let material = materials.add(GxMaterial::sprite(tex));
                    let size = SOUL_SIZE * c.radius / tuning.chick_size;
                    commands.spawn((
                        Soul { t: 0.0, material: material.clone() },
                        Mesh3d(meshes.add(quad(size, 0.0, 0.5))),
                        MeshMaterial3d(material),
                        Transform::from_xyz(c.pos.x, c.pos.y, z),
                        DespawnOnExit(Screen::Level),
                    ));
                }
            }
            continue;
        }
        c.scared = danger.iter().any(|(t, p)| *t == c.team && (p.x - c.pos.x).abs() < FEAR_RANGE);
        // A confused chick is drawn to the nearest danger on its side.
        let here = c.pos.x;
        c.lure = danger.iter().filter(|(t, _)| *t == c.team).map(|(_, p)| p.x).min_by(|a, b| (a - here).abs().total_cmp(&(b - here).abs()));
        // The face for its state; a normal chick blinks now and then.
        c.blink -= dt;
        let want = if c.held || c.glued > 0.0 || c.confused > 0.0 || c.scared && c.flash <= 0.0 && c.health >= c.max_health * LOW_HP {
            "chick__ufo"
        } else if c.flash > 0.0 {
            "chick__flashhit"
        } else if c.health < c.max_health * LOW_HP || c.sick > 0.0 {
            "chick__low"
        } else {
            "normal"
        };
        let play = |commands: &mut Commands, name: &str, from_end: bool| {
            if let Some(clip) = assets.clip(name) {
                let mut anim = BoneAnimator::new(vec![clip]);
                if from_end {
                    // The blink clip ends on the open, normal face.
                    anim.frame = 1000.0;
                }
                commands.entity(e).insert(anim);
            }
        };
        if want != c.face {
            if want == "normal" {
                play(&mut commands, "chick__blink", true);
            } else {
                play(&mut commands, want, false);
            }
            c.face = want;
        } else if want == "normal" && c.blink <= 0.0 {
            c.blink = rng.range(BLINK);
            play(&mut commands, "chick__blink", false);
        }
    }
}

pub fn update_souls(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<Tuning>,
    mut souls: Query<(Entity, &mut Soul, &mut Transform), Without<Splash>>,
    mut splashes: Query<(Entity, &mut Splash, &mut Transform), Without<Soul>>,
    mut materials: ResMut<Assets<GxMaterial>>,
) {
    let dt = time.delta_secs();
    let life = tuning.chick_dying_time + tuning.chick_dying_fade;
    for (e, mut s, mut t) in &mut souls {
        s.t += dt;
        t.translation.y += tuning.chick_dying_speed * dt;
        t.translation.x += (s.t * 4.0).sin() * dt * 0.6;
        let fade = (1.0 - (s.t - tuning.chick_dying_time).max(0.0) / tuning.chick_dying_fade.max(0.1)).clamp(0.0, 1.0);
        if let Some(mut m) = materials.get_mut(&s.material) {
            m.params.konst[3].w = fade;
        }
        if s.t >= life {
            commands.entity(e).despawn();
        }
    }
    for (e, mut s, mut t) in &mut splashes {
        s.t += dt;
        let k = s.t / 0.35;
        t.scale = Vec3::splat(0.5 + k);
        if let Some(mut m) = materials.get_mut(&s.material) {
            m.params.konst[3].w = (1.0 - k).max(0.0);
        }
        if k >= 1.0 {
            commands.entity(e).despawn();
        }
    }
}
