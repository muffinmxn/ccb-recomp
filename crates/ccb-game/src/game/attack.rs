//! Attacks in flight: bomb, weight and lightning. Each runs a small state machine:
//! prepare (telegraphed) → travel → blocked by a barrier or hits → resolve.

use bevy::prelude::*;

use super::{
    barrier::{sweep_hit, vertical_block, Barrier},
    chick::Chick,
    flow::GameAssets,
    rng::Rng,
    tuning::Tuning,
    AttackKind, Team,
};
use crate::gx_material::GxMaterial;

const BOMB_GRAVITY: f32 = 9.0;
const BOMB_FLIGHT_TIME: f32 = 2.4;
const BOMB_RADIUS: f32 = 0.95;
const WEIGHT_GRAVITY: f32 = 30.0;
const WEIGHT_HALF_WIDTH: f32 = 1.8;
const WEIGHT_START_Y: f32 = 16.0;
const STRIKE_RADIUS: f32 = 1.3;
const PLANT_HEIGHT: f32 = 3.2;
const PLANT_GROW_TIME: f32 = 1.0;
const PLANT_BITE_RADIUS: f32 = 1.7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AttackState {
    /// Telegraphing; the attack isn't moving yet.
    Prepare,
    Travel,
    /// A bomb on the ground, fuse burning.
    Roll,
    /// Explosion / impact animation; damage was dealt when entering this state.
    Impact,
    /// Stopped by a barrier; fading out.
    Blocked,
    Finished,
}

#[derive(Component, Debug)]
pub struct Attack {
    pub kind: AttackKind,
    pub from: Team,
    pub quality: f32,
    pub state: AttackState,
    pub t: f32,
    pub pos: Vec2,
    pub vel: Vec2,
    pub target_x: f32,
    visual: Option<Entity>,
    effect: Option<Entity>,
}

impl Attack {
    pub fn new(kind: AttackKind, from: Team, quality: f32, target_x: f32) -> Self {
        Self { kind, from, quality, state: AttackState::Prepare, t: 0.0, pos: Vec2::ZERO, vel: Vec2::ZERO, target_x, visual: None, effect: None }
    }

    pub fn target(&self) -> Team {
        self.from.other()
    }

    /// Where the attack will come down, for the CPU's defence.
    pub fn predicted_x(&self) -> f32 {
        self.target_x
    }
}

fn set_transform(commands: &mut Commands, e: Option<Entity>, t: Transform) {
    if let Some(e) = e {
        commands.entity(e).insert(t);
    }
}

fn damage_chicks(chicks: &mut Query<&mut Chick>, center: Vec2, radius: f32, full_radius: f32, damage: f32, push: f32, squash: bool) -> usize {
    let mut hits = 0;
    for mut c in chicks.iter_mut() {
        if !c.alive() {
            continue;
        }
        let d = c.pos.distance(center);
        if d > radius {
            continue;
        }
        let falloff = if d <= full_radius { 1.0 } else { 1.0 - (d - full_radius) / (radius - full_radius).max(1e-3) };
        let dir = (c.pos - center).normalize_or(Vec2::Y);
        let amount = damage * falloff.clamp(0.25, 1.0);
        c.damage(amount, (dir + Vec2::Y) * push * falloff);
        if squash {
            c.squashed = 2.0;
        }
        hits += 1;
    }
    hits
}

#[allow(clippy::too_many_arguments)]
pub fn update_attacks(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<Tuning>,
    assets: Res<GameAssets>,
    mut rng: ResMut<Rng>,
    mut attacks: Query<(Entity, &mut Attack)>,
    mut chicks: Query<&mut Chick>,
    mut barriers: Query<(Entity, &mut Barrier)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let dt = time.delta_secs();
    let z = tuning.plane_z;
    for (e, mut a) in &mut attacks {
        a.t += dt;
        let before = a.state;
        let target = a.target();
        match (a.kind, a.state) {
            // ---------------------------------------------------------------- bomb
            (AttackKind::Bomb, AttackState::Prepare) => {
                if a.t >= tuning.attack_prepare_time {
                    // Thrown in from off-screen on the attacker's side.
                    let start = Vec2::new(tuning.bomb_appear.x * -a.from.side() * -1.0, tuning.bomb_appear.y);
                    let start = Vec2::new(start.x.abs() * a.from.side(), start.y);
                    let tx = a.target_x;
                    let t = BOMB_FLIGHT_TIME;
                    a.pos = start;
                    a.vel = Vec2::new((tx - start.x) / t, (BOMB_RADIUS - start.y + 0.5 * BOMB_GRAVITY * t * t) / t);
                    a.visual = Some(assets.spawn(&mut commands, "bomb", a.from, BOMB_RADIUS * 2.0, &mut meshes, &mut materials, &mut images));
                    a.state = AttackState::Travel;
                    a.t = 0.0;
                }
            }
            (AttackKind::Bomb, AttackState::Travel) => {
                let prev = a.pos;
                a.vel.y -= BOMB_GRAVITY * dt;
                let next = prev + a.vel * dt;
                if let Some((p, n, be)) = sweep_hit(&barriers, target, prev, next, BOMB_RADIUS) {
                    // Deflected: reflect off the barrier and lose energy.
                    let v = a.vel;
                    a.vel = (v - 2.0 * v.dot(n) * n) * 0.6;
                    a.pos = p + n * (BOMB_RADIUS + 0.12);
                    if let Ok((_, mut b)) = barriers.get_mut(be) {
                        b.flash = 0.4;
                    }
                } else {
                    a.pos = next;
                }
                if a.pos.y <= BOMB_RADIUS {
                    a.pos.y = BOMB_RADIUS;
                    a.vel = Vec2::new(a.vel.x * 0.4, 0.0);
                    a.state = AttackState::Roll;
                    a.t = 0.0;
                }
                if a.pos.x.abs() > 15.0 || a.pos.y < -3.0 {
                    a.state = AttackState::Finished;
                }
                let spin = a.t * -a.from.side() * 4.0;
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_rotation(Quat::from_rotation_z(spin)).with_scale(Vec3::splat(assets.scale("bomb", BOMB_RADIUS * 2.0))));
            }
            (AttackKind::Bomb, AttackState::Roll) => {
                let vx = a.vel.x;
                a.pos.x += vx * dt;
                a.vel.x *= 1.0 - 2.0 * dt;
                let side = tuning.side_width + 1.0;
                a.pos.x = a.pos.x.clamp(-side, side);
                let pulse = 1.0 + 0.12 * (a.t * 18.0).sin();
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_scale(Vec3::splat(assets.scale("bomb", BOMB_RADIUS * 2.0) * pulse)));
                if a.t >= tuning.bomb_roll_time {
                    let (rmin, rmax) = tuning.bomb_exp_radius;
                    let radius = rmin + (rmax - rmin) * a.quality;
                    let dmg = tuning.bomb_damage.damage(a.quality);
                    damage_chicks(&mut chicks, a.pos, radius, tuning.bomb_exp_full_radius, dmg, 4.0, false);
                    if let Some(v) = a.visual.take() {
                        commands.entity(v).despawn();
                    }
                    a.effect = Some(assets.spawn(&mut commands, "explosion", a.from, radius * 2.0, &mut meshes, &mut materials, &mut images));
                    a.state = AttackState::Impact;
                    a.t = 0.0;
                }
            }
            // ---------------------------------------------------------------- weight
            (AttackKind::Weight, AttackState::Prepare) => {
                if a.t >= tuning.attack_prepare_time * 0.6 {
                    a.pos = Vec2::new(a.target_x, WEIGHT_START_Y);
                    a.vel = Vec2::new(0.0, -2.0);
                    let variant = tuning.weight_quality_thresholds.iter().rposition(|&th| a.quality >= th).unwrap_or(0) + 1;
                    let v = assets.spawn_weight(&mut commands, variant, a.from, WEIGHT_HALF_WIDTH * 2.0, &mut meshes, &mut materials, &mut images);
                    a.visual = Some(v);
                    a.state = AttackState::Travel;
                    a.t = 0.0;
                }
            }
            (AttackKind::Weight, AttackState::Travel) => {
                let prev = a.pos;
                a.vel.y -= WEIGHT_GRAVITY * dt;
                let next = prev + a.vel * dt;
                // The weight's underside sweeps across barriers.
                let bottom = |p: Vec2| p - Vec2::Y * 1.2;
                let hit = [-WEIGHT_HALF_WIDTH * 0.8, 0.0, WEIGHT_HALF_WIDTH * 0.8]
                    .iter()
                    .find_map(|&dx| sweep_hit(&barriers, target, bottom(prev) + Vec2::X * dx, bottom(next) + Vec2::X * dx, 0.1));
                if let Some((_, _, be)) = hit {
                    if let Ok((_, mut b)) = barriers.get_mut(be) {
                        b.flash = 0.4;
                    }
                    a.vel = Vec2::new(rng.range((-1.0, 1.0)), 3.0);
                    a.state = AttackState::Blocked;
                    a.t = 0.0;
                } else {
                    a.pos = next;
                }
                if a.pos.y <= 1.2 {
                    a.pos.y = 1.2;
                    let dmg = tuning.weight_damage.damage(a.quality);
                    let center = Vec2::new(a.pos.x, 0.5);
                    damage_chicks(&mut chicks, center, WEIGHT_HALF_WIDTH + 0.4, WEIGHT_HALF_WIDTH, dmg, 1.0, true);
                    a.state = AttackState::Impact;
                    a.t = 0.0;
                }
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_scale(Vec3::splat(assets.scale("weight", WEIGHT_HALF_WIDTH * 2.0))));
            }
            (AttackKind::Weight, AttackState::Blocked) => {
                a.vel.y -= WEIGHT_GRAVITY * 0.5 * dt;
                let v = a.vel;
                a.pos += v * dt;
                let fade = (1.0 - a.t).max(0.01);
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_rotation(Quat::from_rotation_z(a.t * 2.0)).with_scale(Vec3::splat(assets.scale("weight", WEIGHT_HALF_WIDTH * 2.0) * fade)));
                if a.t >= 1.0 {
                    a.state = AttackState::Finished;
                }
            }
            // ---------------------------------------------------------------- lightning
            (AttackKind::Lightning, AttackState::Prepare) => {
                if a.visual.is_none() {
                    a.pos = Vec2::new(a.target_x, tuning.cloud_y);
                    a.visual = Some(assets.spawn(&mut commands, "cloud", a.from, 5.0, &mut meshes, &mut materials, &mut images));
                }
                // The cloud drifts in over its target.
                let k = (a.t / tuning.attack_prepare_time).min(1.0);
                let x = a.target_x + (1.0 - k) * 3.0 * -a.from.side();
                set_transform(&mut commands, a.visual, Transform::from_xyz(x, tuning.cloud_y, z).with_scale(Vec3::splat(assets.scale("cloud", 5.0) * k.max(0.05))));
                if a.t >= tuning.attack_prepare_time + 0.8 {
                    let x = a.target_x;
                    let (bottom, blocked) = match vertical_block(&barriers, target, x, tuning.cloud_y - 0.5, 0.0) {
                        Some((y, be)) => {
                            if let Ok((_, mut b)) = barriers.get_mut(be) {
                                b.flash = 0.4;
                            }
                            (y, true)
                        }
                        None => (0.0, false),
                    };
                    if !blocked {
                        let dmg = tuning.bomb_damage.damage(a.quality);
                        damage_chicks(&mut chicks, Vec2::new(x, 0.8), STRIKE_RADIUS, STRIKE_RADIUS * 0.6, dmg, 2.0, false);
                    }
                    a.effect = Some(assets.spawn_bolt(&mut commands, x, bottom, tuning.cloud_y - 0.6, z, &mut meshes, &mut materials));
                    a.state = if blocked { AttackState::Blocked } else { AttackState::Impact };
                    a.t = 0.0;
                }
            }
            (AttackKind::Lightning, AttackState::Blocked | AttackState::Impact) => {
                if a.t > 0.25 {
                    if let Some(fx) = a.effect.take() {
                        commands.entity(fx).despawn();
                    }
                }
                let fade = (1.0 - a.t / 0.8).max(0.01);
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.target_x, tuning.cloud_y, z).with_scale(Vec3::splat(assets.scale("cloud", 5.0) * fade)));
                if a.t >= 0.8 {
                    a.state = AttackState::Finished;
                }
            }
            // ---------------------------------------------------------------- plant
            (AttackKind::Plant, AttackState::Prepare) => {
                // A mound telegraphs where the plant will come up.
                if a.t >= tuning.attack_prepare_time {
                    a.pos = Vec2::new(a.target_x, -PLANT_HEIGHT);
                    a.visual = Some(assets.spawn_posed(&mut commands, "plant", "plant__default", a.from, 2.4, &mut meshes, &mut materials, &mut images));
                    a.state = AttackState::Travel;
                    a.t = 0.0;
                }
            }
            (AttackKind::Plant, AttackState::Travel) => {
                // Grows up out of the ground; a barrier across its path stops it.
                let k = (a.t / PLANT_GROW_TIME).min(1.0);
                let head = -PLANT_HEIGHT + (PLANT_HEIGHT * 2.0) * k;
                if let Some((y, be)) = vertical_block(&barriers, target, a.target_x, head + 0.4, 0.2) {
                    if let Ok((_, mut b)) = barriers.get_mut(be) {
                        b.flash = 0.4;
                    }
                    a.pos.y = y - 1.2 - PLANT_HEIGHT * 0.5;
                    a.state = AttackState::Blocked;
                    a.t = 0.0;
                } else {
                    a.pos.y = head - PLANT_HEIGHT * 0.5;
                    if k >= 1.0 {
                        // Two bites at chicks around the stem.
                        let dmg = tuning.bomb_damage.damage(a.quality) * 0.5;
                        for _ in 0..2 {
                            damage_chicks(&mut chicks, Vec2::new(a.target_x, 0.8), PLANT_BITE_RADIUS, PLANT_BITE_RADIUS * 0.6, dmg, 1.5, false);
                        }
                        if let Some(v) = a.visual {
                            assets.play(&mut commands, v, "plant__bite");
                        }
                        a.state = AttackState::Impact;
                        a.t = 0.0;
                    }
                }
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.target_x, a.pos.y, z - 0.3).with_scale(Vec3::splat(assets.scale("plant", 2.4))));
            }
            (AttackKind::Plant, AttackState::Blocked) => {
                let sink = a.t * 3.0;
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.target_x, a.pos.y - sink, z - 0.3).with_scale(Vec3::splat(assets.scale("plant", 2.4))));
                if a.t >= 1.2 {
                    a.state = AttackState::Finished;
                }
            }
            (AttackKind::Plant, AttackState::Impact) => {
                let sink = (a.t - 1.0).max(0.0) * 4.0;
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.target_x, a.pos.y - sink, z - 0.3).with_scale(Vec3::splat(assets.scale("plant", 2.4))));
                if a.t >= 2.0 {
                    a.state = AttackState::Finished;
                }
            }
            // ---------------------------------------------------------------- common
            (_, AttackState::Impact) => {
                if let Some(fx) = a.effect {
                    let (rmin, rmax) = tuning.bomb_exp_radius;
                    let radius = rmin + (rmax - rmin) * a.quality;
                    let grow = (a.t / 0.4).min(1.0);
                    set_transform(&mut commands, Some(fx), Transform::from_xyz(a.pos.x, a.pos.y, z + 0.5).with_scale(Vec3::splat(assets.scale("explosion", radius * 2.0) * (0.3 + 0.7 * grow) * (1.0 - (a.t - 0.5).max(0.0) * 2.0).max(0.01))));
                }
                if a.kind == AttackKind::Weight {
                    let fade = (1.0 - (a.t - 1.0).max(0.0) * 2.0).max(0.01);
                    set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_scale(Vec3::splat(assets.scale("weight", WEIGHT_HALF_WIDTH * 2.0) * fade)));
                }
                if a.t >= 1.5 {
                    a.state = AttackState::Finished;
                }
            }
            (_, AttackState::Finished) => {
                for v in [a.visual.take(), a.effect.take()].into_iter().flatten() {
                    commands.entity(v).despawn();
                }
                commands.entity(e).despawn();
            }
            _ => a.state = AttackState::Finished,
        }
        if a.state != before {
            debug!("{:?} from {:?}: {:?} -> {:?} at ({:.1}, {:.1}) q {:.2}", a.kind, a.from, before, a.state, a.pos.x, a.pos.y, a.quality);
        }
    }
}
