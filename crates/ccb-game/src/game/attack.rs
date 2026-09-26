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

/// Bomb flight: the original's bomb lanes start at ~15 up and lose `bombDampFac` per 180 Hz step.
const BOMB_START_VY: f32 = 15.0;
const BOMB_DAMP: f32 = 0.02;
/// Model width per unit of bomb size (the model includes the fuse).
const BOMB_LOOK: f32 = 2.8;
const WEIGHT_GRAVITY: f32 = 30.0;
const WEIGHT_START_Y: f32 = 16.0;
const STRIKE_RADIUS: f32 = 1.3;
/// Plant head size (`plantHeadSize`) and vine thickness.
const PLANT_HEAD: f32 = 1.0;
const STEM_WIDTH: f32 = 0.16;
const STEM_OUTLINE: f32 = 0.28;
const PLANT_LIFETIME: f32 = 10.0;
const UFO_Y: f32 = 6.2;
const UFO_BEAM_TIME: f32 = 1.4;
const TENTACLE_Y: f32 = 1.1;
const TENTACLE_LEN: f32 = 7.0;
const TENTACLE_SWEEP_TIME: f32 = 1.1;
const GHOST_SPEED: f32 = 4.5;
const GHOST_BARRIERS_TO_STOP: u32 = 2;

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
    flash: Option<Entity>,
    /// Weight variant (1..=6: TV, sofa, piano, metal, elephant, whale).
    variant: usize,
    /// Chicks already hit (sweeping attacks hit each chick once).
    hit: Vec<Entity>,
    /// Barriers eaten by a ghost.
    pub eaten: u32,
    /// Weight width; bomb radius.
    size: f32,
    /// Plant: stem points (root first), the barrier it's climbing (entity, segment, direction),
    /// seconds since the last bite, bite cooldown, pending bite, seconds above the Y limit.
    path: Vec<Vec2>,
    climbing: Option<(Entity, usize, i32)>,
    starving: f32,
    starve_limit: f32,
    cooldown: f32,
    bite: Option<f32>,
    high: f32,
    /// A second entity: the weight's shadow, the plant's stem.
    aux: Option<Entity>,
    /// Blown up early (lightning, touched while drawing): quality to explode with.
    pub detonate: Option<f32>,
}

impl Attack {
    pub fn new(kind: AttackKind, from: Team, quality: f32, target_x: f32) -> Self {
        Self {
            kind,
            from,
            quality,
            state: AttackState::Prepare,
            t: 0.0,
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            target_x,
            visual: None,
            effect: None,
            flash: None,
            variant: 1,
            hit: Vec::new(),
            eaten: 0,
            size: 1.0,
            path: Vec::new(),
            climbing: None,
            starving: 0.0,
            starve_limit: 5.0,
            cooldown: 0.0,
            bite: None,
            high: 0.0,
            aux: None,
            detonate: None,
        }
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

const OUCH: [&str; 5] = ["SFX_CHICKS_AUA01", "SFX_CHICKS_AUA02", "SFX_CHICKS_AUA03", "SFX_CHICKS_AUA04", "SFX_CHICKS_AUA05"];
const WEIGHT_SOUNDS: [&str; 6] = [
    "SFX_ATTACKS_WEIGHT_TV",
    "SFX_ATTACKS_WEIGHT_SOFA",
    "SFX_ATTACKS_WEIGHT_PIANO",
    "SFX_ATTACKS_WEIGHT_METAL",
    "SFX_ATTACKS_WEIGHT_ELEPHANT",
    "SFX_ATTACKS_WEIGHT_WHALE",
];

#[allow(clippy::too_many_arguments)]
fn damage_chicks(chicks: &mut Query<(Entity, &mut Chick)>, sfx: &mut crate::sfx::Sfx, center: Vec2, radius: f32, full_radius: f32, damage: f32, push: f32, squash: bool) -> usize {
    let squash_time = 2.0;
    let mut hits = 0;
    for (_, mut c) in chicks.iter_mut() {
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
        if c.dying.is_some() {
            sfx.play("SFX_CHICKS_DIE");
        } else {
            sfx.play_one_of(&OUCH, hits + (c.pos.x * 7.0).abs() as usize);
        }
        if squash {
            c.squashed = squash_time;
        }
        hits += 1;
    }
    hits
}

/// Model scale for a weight variant `size` wide.
fn weight_scale(assets: &GameAssets, variant: usize, size: f32) -> f32 {
    size / assets.weight_visible.get(variant.clamp(1, 6) - 1).copied().unwrap_or(1.5)
}

/// A falling weight's shadow: a flat ellipse on the ground, `width` wide.
fn shadow_transform(x: f32, z: f32, width: f32) -> Transform {
    Transform::from_xyz(x, 0.03, z).with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)).with_scale(Vec3::new(width, 1.2, 1.0))
}

/// Model scale that makes the flytrap head `plantHeadSize` wide.
fn plant_scale(assets: &GameAssets) -> f32 {
    PLANT_HEAD / assets.bone_width("plant", "head").max(1e-3)
}

/// Rebuilds the vine's ribbons along the plant's path.
fn draw_vine(commands: &mut Commands, meshes: &mut Assets<Mesh>, a: &Attack, width: f32) {
    if a.path.len() < 2 {
        return;
    }
    for (e, w) in [(a.effect, STEM_OUTLINE), (a.aux, STEM_WIDTH)] {
        if let Some(e) = e {
            commands.entity(e).insert(Mesh3d(meshes.add(super::barrier::ribbon(&a.path, w * width))));
        }
    }
}

/// Horizontal throw speed that lands a bomb (damped arc, `BOMB_START_VY` up) at `target_x`.
fn bomb_throw_vx(start: Vec2, target_x: f32, r: f32, g: f32) -> f32 {
    let land = |vx: f32| {
        let h = 1.0 / 180.0;
        let (mut p, mut v) = (start, Vec2::new(vx, BOMB_START_VY));
        for _ in 0..2000 {
            v.y += g * h;
            v *= 1.0 - BOMB_DAMP;
            p += v * h;
            if p.y <= r && v.y < 0.0 {
                break;
            }
        }
        p.x
    };
    let dir = (target_x - start.x).signum();
    let (mut lo, mut hi) = (0.0f32, 400.0f32);
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        if (land(mid * dir) - start.x).abs() < (target_x - start.x).abs() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo * dir
}

/// Whether a line of either team lies between `a` and `b` (blasts don't pass lines).
fn line_between(barriers: &Query<(Entity, &mut Barrier)>, a: Vec2, b: Vec2) -> bool {
    [Team::Yellow, Team::Black].iter().any(|&t| super::barrier::segment_hit(barriers, t, a, b).is_some())
}

/// A bomb blast: full damage within `full_radius`, less further out, none behind a line.
#[allow(clippy::too_many_arguments)]
fn blast(chicks: &mut Query<(Entity, &mut Chick)>, barriers: &Query<(Entity, &mut Barrier)>, sfx: &mut crate::sfx::Sfx, center: Vec2, radius: f32, full_radius: f32, damage: f32, push: f32) {
    let mut hits = 0;
    for (_, mut c) in chicks.iter_mut() {
        if !c.alive() {
            continue;
        }
        let d = c.pos.distance(center);
        if d > radius + c.radius || line_between(barriers, center, c.pos) {
            continue;
        }
        let falloff = if d <= full_radius { 1.0 } else { 1.0 - (d - full_radius) / (radius - full_radius).max(1e-3) };
        let dir = (c.pos - center).normalize_or(Vec2::Y);
        c.damage(damage * falloff.clamp(0.05, 1.0), (dir + Vec2::Y * 0.5) * push * falloff.max(0.3));
        if c.dying.is_some() {
            sfx.play("SFX_CHICKS_DIE");
        } else {
            sfx.play_one_of(&OUCH, hits + (c.pos.x * 7.0).abs() as usize);
        }
        hits += 1;
    }
}

/// One frame of a growing plant: it bites chicks in range, otherwise grows its head towards
/// the nearest chick (never downwards) and climbs lines it meets towards their higher end.
/// Returns true when it dies (starved, too long above `plantGrowYThreshold`).
fn grow_plant(a: &mut Attack, tuning: &Tuning, barriers: &Query<(Entity, &mut Barrier)>, chicks: &mut Query<(Entity, &mut Chick)>, sfx: &mut crate::sfx::Sfx, dt: f32) -> bool {
    let target = a.target();
    a.starving += dt;
    a.cooldown -= dt;
    let head = a.pos;
    // Biting: a short windup, then the snap.
    if let Some(b) = a.bite {
        let b = b + dt;
        if b >= tuning.plant_bite_time && b - dt < tuning.plant_bite_time {
            let dmg = tuning.plant_damage.damage(a.quality);
            let victim = chicks
                .iter_mut()
                .filter(|(_, c)| c.team == target && c.alive())
                .min_by(|x, y| x.1.pos.distance(head).total_cmp(&y.1.pos.distance(head)));
            if let Some((_, mut c)) = victim {
                if c.pos.distance(head) <= tuning.plant_bite_range + c.radius {
                    let push = (c.pos - head).normalize_or(Vec2::Y) * 1.5 + Vec2::Y;
                    c.damage(dmg, push);
                    sfx.play(if c.dying.is_some() { "SFX_CHICKS_DIE" } else { "SFX_CHICKS_AUA02" });
                    a.starving = 0.0;
                }
            }
            sfx.play_one_of(&["SFX_ATTACKS_PLANT_BITE_1", "SFX_ATTACKS_PLANT_BITE_2", "SFX_ATTACKS_PLANT_BITE_3"], a.path.len());
        }
        if b >= tuning.plant_bite_time * 2.0 {
            a.cooldown = tuning.plant_bite_cooldown;
            a.bite = None;
        } else {
            a.bite = Some(b);
        }
        return false;
    }
    let prey = chicks
        .iter()
        .filter(|(_, c)| c.team == target && c.alive())
        .map(|(_, c)| c.pos)
        .min_by(|x, y| x.distance(head).total_cmp(&y.distance(head)));
    if let Some(p) = prey {
        if p.distance(head) <= tuning.plant_bite_range && a.cooldown <= 0.0 {
            a.bite = Some(0.0);
            return false;
        }
    }
    let len: f32 = a.path.windows(2).map(|w| w[0].distance(w[1])).sum();
    let sprouting = a.t < tuning.plant_sprout_time;
    let speed = tuning.plant_grow_speed * if sprouting { 2.0 } else { 1.0 };
    if len < tuning.plant_grow_max_len {
        let step = speed * dt;
        if let Some((be, seg, dir)) = a.climbing {
            // Climbing a line: follow it towards its higher end, only upwards.
            match barriers.get(be).ok().map(|(_, b)| b.points.clone()) {
                Some(pts) if seg + 1 < pts.len() => {
                    let (from, to) = if dir > 0 { (pts[seg], pts[seg + 1]) } else { (pts[seg + 1], pts[seg]) };
                    let d = (to - from).normalize_or(Vec2::Y);
                    a.vel = d;
                    if a.pos.distance(to) <= step {
                        a.pos = to;
                        let nseg = seg as i32 + dir;
                        a.climbing = None;
                        if nseg >= 0 && (nseg as usize) + 1 < pts.len() {
                            let s2 = nseg as usize;
                            let (f2, t2) = if dir > 0 { (pts[s2], pts[s2 + 1]) } else { (pts[s2 + 1], pts[s2]) };
                            // Only upwards: let go once the line turns down.
                            if t2.y >= f2.y - 0.05 {
                                a.climbing = Some((be, s2, dir));
                            }
                        }
                    } else {
                        a.pos += d * step;
                    }
                }
                _ => a.climbing = None,
            }
        } else {
            // Head for the nearest chick, but never downwards.
            let want = prey.map_or(Vec2::Y, |p| (p - head).normalize_or(Vec2::Y));
            let want = if sprouting { Vec2::Y } else { Vec2::new(want.x, want.y.max(0.0)).normalize_or(Vec2::X * want.x.signum()) };
            let k = 1.0 - (1.0 - tuning.plant_head_dir_interp).powf(dt * 60.0);
            a.vel = a.vel.lerp(want, k).normalize_or(Vec2::Y);
            let next = a.pos + a.vel * step;
            if let Some((p, seg, be)) = super::barrier::segment_hit(barriers, target, a.pos, next + a.vel * 0.1) {
                // Met a line: climb it towards its higher end.
                if let Ok((_, b)) = barriers.get(be) {
                    let dir = if b.points[seg + 1].y >= b.points[seg].y { 1 } else { -1 };
                    a.pos = p;
                    a.climbing = Some((be, seg, dir));
                }
            } else {
                a.pos = next;
                if !sprouting {
                    a.pos.y = a.pos.y.max(0.35);
                }
            }
        }
        // Stay on the target side.
        let (lo, hi) = tuning.side_range(target);
        a.pos.x = a.pos.x.clamp(lo - 0.5, hi + 0.5);
        if a.path.last().is_none_or(|l| l.distance(a.pos) > 0.08) {
            a.path.push(a.pos);
        }
    }
    if a.pos.y > tuning.plant_grow_y_threshold {
        a.high += dt;
    }
    // `plantEffectiveTimer` (10 s) caps its life.
    a.starving > a.starve_limit || a.high > tuning.plant_threshold_living || a.t > PLANT_LIFETIME
}

#[allow(clippy::too_many_arguments)]
pub fn update_attacks(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<Tuning>,
    assets: Res<GameAssets>,
    mut rng: ResMut<Rng>,
    mut attacks: Query<(Entity, &mut Attack)>,
    mut chicks: Query<(Entity, &mut Chick)>,
    mut barriers: Query<(Entity, &mut Barrier)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut sfx: ResMut<crate::sfx::Sfx>,
    mut shot: Option<ResMut<crate::shot::ShotTrigger>>,
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
                    // Thrown in from off-screen on the attacker's side (`bombAppearPos`), on a
                    // damped arc like the original's bomb lanes.
                    let start = Vec2::new(tuning.bomb_appear.x.abs() * a.from.side(), tuning.bomb_appear.y);
                    a.size = tuning.bomb_min_size;
                    a.pos = start;
                    a.vel = Vec2::new(bomb_throw_vx(start, a.target_x, a.size, tuning.gravity), BOMB_START_VY);
                    sfx.play("SFX_ATTACKS_BOMB_FLY");
                    a.visual = Some(assets.spawn_posed(&mut commands, "bomb", "bomb__fly", a.from, a.size * BOMB_LOOK, &mut meshes, &mut materials, &mut images));
                    a.state = AttackState::Travel;
                    a.t = 0.0;
                }
            }
            (AttackKind::Bomb, AttackState::Travel | AttackState::Roll) => {
                let r = a.size;
                let rolling = a.state == AttackState::Roll;
                // 180 Hz steps: gravity, damping, bounces off lines and the floor.
                let h = 1.0 / tuning.physics_fps;
                let steps = ((dt / h).round() as usize).clamp(1, 20);
                let damp = if rolling { 1.0 - 3.0 * h } else { 1.0 - BOMB_DAMP };
                for _ in 0..steps {
                    a.vel.y += tuning.gravity * h;
                    a.vel *= damp;
                    let prev = a.pos;
                    let next = prev + a.vel * h;
                    if let Some((p, n, be)) = sweep_hit(&barriers, target, prev, next, r) {
                        let v = a.vel;
                        let vn = v.dot(n);
                        if vn < 0.0 {
                            // `particleCollisionVelReduction`
                            a.vel = (v - vn * n) * 0.9 - vn * n * 0.35;
                            if let Ok((_, mut b)) = barriers.get_mut(be) {
                                if b.flash <= 0.0 {
                                    sfx.play("SFX_ATTACKS_OCTOPUS_BLOCKED");
                                }
                                b.flash = 0.4;
                                // Touching the bomb while drawing sets it off.
                                if b.drawing && rolling {
                                    a.detonate = Some(a.quality);
                                }
                            }
                        }
                        a.pos = p + n * (r + 0.02);
                    } else {
                        a.pos = next;
                    }
                    if a.pos.y <= r {
                        a.pos.y = r;
                        if a.vel.y < 0.0 {
                            a.vel.y = -a.vel.y * 0.3;
                            if a.vel.y < 1.0 {
                                a.vel.y = 0.0;
                            }
                        }
                    }
                    // The target side's walls keep a landed bomb in.
                    if rolling {
                        let (lo, hi) = tuning.side_range(target);
                        let (lo, hi) = (lo - tuning.chick_radius, hi + tuning.chick_radius);
                        if a.pos.x < lo + r || a.pos.x > hi - r {
                            a.pos.x = a.pos.x.clamp(lo + r, hi - r);
                            a.vel.x = -a.vel.x * 0.5;
                        }
                    }
                }
                if !rolling && a.pos.y <= tuning.bomb_roll_y + r && a.pos.x * target.side() > 0.0 {
                    // `bombRollYThreshold`: down on the field, the fuse is burning.
                    if let Some(v) = a.visual {
                        assets.play(&mut commands, v, "bomb__roll");
                    }
                    a.state = AttackState::Roll;
                    a.t = 0.0;
                    sfx.play("SFX_ATTACKS_BOMB_COUNTDOWN");
                }
                if a.pos.x.abs() > 17.0 || a.pos.y < -3.0 || (!rolling && a.t > 4.0) {
                    sfx.play("SFX_ATTACKS_BOMB_DEFUSED");
                    a.state = AttackState::Finished;
                }
                // Ticks every `bombTickTimer`.
                if rolling && (a.t / tuning.bomb_tick_time).floor() != ((a.t - dt) / tuning.bomb_tick_time).floor() {
                    sfx.play("SFX_ATTACKS_BOMB_COUNTDOWN");
                }
                let spin = -a.pos.x / r;
                let pulse = if rolling { 1.0 + 0.12 * (a.t * 18.0).sin() } else { 1.0 };
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_rotation(Quat::from_rotation_z(spin)).with_scale(Vec3::splat(assets.scale("bomb", r * BOMB_LOOK) * pulse)));
                if a.state != AttackState::Finished && ((rolling && a.t >= tuning.bomb_roll_time) || a.detonate.is_some()) {
                    let q = a.detonate.take().unwrap_or(a.quality).max(a.quality);
                    let (rmin, rmax) = tuning.bomb_exp_radius;
                    let radius = rmin + (rmax - rmin) * q;
                    let dmg = tuning.bomb_damage.damage(q);
                    let push = tuning.bomb_move_by.0 + (tuning.bomb_move_by.1 - tuning.bomb_move_by.0) * q;
                    blast(&mut chicks, &barriers, &mut sfx, a.pos, radius, tuning.bomb_exp_full_radius, dmg, push * 30.0);
                    if let Some(v) = a.visual.take() {
                        commands.entity(v).despawn();
                    }
                    sfx.play("SFX_ATTACKS_BOMB_EXPL");
                    a.quality = q;
                    a.effect = Some(assets.spawn(&mut commands, "explosion", a.from, radius * 2.0, &mut meshes, &mut materials, &mut images));
                    a.state = AttackState::Impact;
                    a.t = 0.0;
                }
            }
            // ---------------------------------------------------------------- weight
            (AttackKind::Weight, AttackState::Prepare) => {
                if a.aux.is_none() {
                    // Heavier types for better traces (`weightQualityTypeThresholds`).
                    let variant = tuning.weight_quality_thresholds.iter().rposition(|&th| a.quality >= th).unwrap_or(0) + 1;
                    a.variant = variant;
                    a.size = tuning.weight_widths.get(variant - 1).copied().unwrap_or(4.0);
                    let (lo, hi) = tuning.side_range(target);
                    let border = tuning.weight_border_offset.min((hi - lo) / 2.0);
                    a.target_x = a.target_x.clamp(lo + border, hi - border);
                    a.pos = Vec2::new(a.target_x, WEIGHT_START_Y);
                    // Its shadow shows where it will land.
                    a.aux = Some(assets.spawn_shadow(&mut commands, z, &mut meshes, &mut materials));
                }
                let k = (a.t / (tuning.attack_prepare_time * 0.6)).min(1.0);
                set_transform(&mut commands, a.aux, shadow_transform(a.target_x, z, a.size * (0.3 + 0.3 * k)));
                if a.t >= tuning.attack_prepare_time * 0.6 {
                    a.vel = Vec2::new(0.0, -2.0);
                    a.visual = Some(assets.spawn_weight(&mut commands, a.variant, a.from, a.size, &mut meshes, &mut materials, &mut images));
                    sfx.play("SFX_ATTACKS_WEIGHT_FLY");
                    a.state = AttackState::Travel;
                    a.t = 0.0;
                }
            }
            (AttackKind::Weight, AttackState::Travel) => {
                let prev = a.pos;
                a.vel.y -= WEIGHT_GRAVITY * dt;
                let next = prev + a.vel * dt;
                let half = a.size / 2.0;
                let low = half * 0.6;
                let bottom = |p: Vec2| p - Vec2::Y * low;
                // The weight's underside sweeps across barriers.
                let hit = (0..=6)
                    .map(|i| -half + a.size * i as f32 / 6.0)
                    .find_map(|dx| sweep_hit(&barriers, target, bottom(prev) + Vec2::X * dx, bottom(next) + Vec2::X * dx, 0.1));
                a.pos = next;
                if let Some((_, _, be)) = hit {
                    // The line has to be at least as wide as the weight.
                    let cover = barriers
                        .get(be)
                        .map(|(_, b)| {
                            let (lo, hi) = b.bounds();
                            ((hi.x.min(a.pos.x + half) - lo.x.max(a.pos.x - half)) / a.size).max(0.0)
                        })
                        .unwrap_or(0.0);
                    if cover >= 0.95 {
                        if let Ok((_, mut b)) = barriers.get_mut(be) {
                            b.flash = 0.4;
                        }
                        a.pos = prev;
                        a.vel = Vec2::new(rng.range((-1.0, 1.0)), 3.0);
                        sfx.play("SFX_ATTACKS_OCTOPUS_BLOCKED");
                        a.state = AttackState::Blocked;
                        a.t = 0.0;
                    } else {
                        // Too narrow: smashed through, and the weight loses some punch.
                        commands.entity(be).despawn();
                        a.quality *= 1.0 - tuning.weight_quality_decrease * cover;
                        sfx.play("SFX_CHICK_DESTROY_LINE");
                    }
                }
                if a.state == AttackState::Travel && a.pos.y <= low {
                    a.pos.y = low;
                    let dmg = tuning.weight_damage.damage(a.quality);
                    sfx.play(WEIGHT_SOUNDS[a.variant.clamp(1, 6) - 1]);
                    let mut hits = 0;
                    for (_, mut c) in chicks.iter_mut() {
                        if c.team == target && c.alive() && (c.pos.x - a.pos.x).abs() < half + c.radius * 0.5 {
                            let push = Vec2::new((c.pos.x - a.pos.x).signum() * 3.0, 0.0);
                            c.damage(dmg, push);
                            c.squashed = tuning.chick_squashed_time;
                            sfx.play(if c.dying.is_some() { "SFX_CHICKS_DIE" } else { OUCH[hits % OUCH.len()] });
                            hits += 1;
                        }
                    }
                    a.state = AttackState::Impact;
                    a.t = 0.0;
                }
                let k = (1.0 - (a.pos.y / WEIGHT_START_Y)).clamp(0.0, 1.0);
                set_transform(&mut commands, a.aux, shadow_transform(a.pos.x, z, a.size * (0.6 + 0.4 * k)));
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_scale(Vec3::splat(weight_scale(&assets, a.variant, a.size))));
            }
            (AttackKind::Weight, AttackState::Blocked) => {
                a.vel.y -= WEIGHT_GRAVITY * 0.5 * dt;
                let v = a.vel;
                a.pos += v * dt;
                let fade = (1.0 - a.t).max(0.01);
                if let Some(e) = a.aux.take() {
                    commands.entity(e).despawn();
                }
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_rotation(Quat::from_rotation_z(a.t * 2.0)).with_scale(Vec3::splat(weight_scale(&assets, a.variant, a.size) * fade)));
                if a.t >= 1.0 {
                    a.state = AttackState::Finished;
                }
            }
            // ---------------------------------------------------------------- lightning
            (AttackKind::Lightning, AttackState::Prepare) => {
                if a.visual.is_none() {
                    sfx.play("SFX_ATTACKS_THUNDER_APPROACHING");
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
                            sfx.play("SFX_ENV_LINE_GROUNDED");
                            (y, true)
                        }
                        None => (0.0, false),
                    };
                    if !blocked {
                        let dmg = tuning.bomb_damage.damage(a.quality);
                        damage_chicks(&mut chicks, &mut sfx, Vec2::new(x, 0.8), STRIKE_RADIUS, STRIKE_RADIUS * 0.6, dmg, 2.0, false);
                    }
                    sfx.play("SFX_ATTACKS_THUNDER");
                    // The strike flash on the cloud.
                    let flash = assets.spawn_posed(&mut commands, "cloudLightning", "cloudLightning__spratzel", a.from, 5.0, &mut meshes, &mut materials, &mut images);
                    commands.entity(flash).insert(Transform::from_xyz(x, tuning.cloud_y, z + 0.2).with_scale(Vec3::splat(assets.scale("cloudLightning", 5.0))));
                    a.flash = Some(flash);
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
                if a.t >= tuning.attack_prepare_time {
                    // Sprouts at its anchor and grows as a vine towards the chicks.
                    a.pos = Vec2::new(a.target_x, 0.0);
                    a.path = vec![a.pos];
                    a.vel = Vec2::Y;
                    a.starve_limit = rng.range(tuning.plant_starving);
                    a.cooldown = tuning.plant_start_bite_cooldown;
                    sfx.play("SFX_ATTACKS_PLANT_APPEAR");
                    // The flytrap head rides the vine's tip; the leaves stay at the root.
                    let head = assets.spawn_bones(&mut commands, "plant", a.from, 1.0, Some(&["head", "plantInnerMouth"]), &mut meshes, &mut materials, &mut images);
                    a.visual = Some(head);
                    let leaves = assets.spawn_bones(&mut commands, "plant", a.from, 1.0, Some(&["leaves"]), &mut meshes, &mut materials, &mut images);
                    commands.entity(leaves).insert(Transform::from_xyz(a.pos.x, 0.0, z + 0.1).with_scale(Vec3::splat(plant_scale(&assets))));
                    a.flash = Some(leaves);
                    // The vine: a dark outline under a lighter green fill.
                    a.effect = Some(assets.spawn_stem(&mut commands, z + 0.25, Vec4::new(0.08, 0.25, 0.04, 1.0), &mut materials));
                    a.aux = Some(assets.spawn_stem(&mut commands, z + 0.26, Vec4::new(0.42, 0.72, 0.16, 1.0), &mut materials));
                    a.state = AttackState::Travel;
                    a.t = 0.0;
                }
            }
            (AttackKind::Plant, AttackState::Travel) => {
                let died = grow_plant(&mut a, &tuning, &barriers, &mut chicks, &mut sfx, dt);
                draw_vine(&mut commands, &mut meshes, &a, 1.0);
                // The head sits on the vine's tip, leaning along it; a bite lunges it forward.
                let d = a.vel;
                let s = plant_scale(&assets);
                let lunge = a.bite.map_or(0.0, |b| (b / tuning.plant_bite_time * std::f32::consts::FRAC_PI_2).sin().max(0.0) * 0.5);
                let tip = a.pos + d * lunge;
                let rot = Quat::from_rotation_z(f32::atan2(-d.x, d.y) * 0.6);
                let flip = if d.x < 0.0 { -1.0 } else { 1.0 };
                let snap = 1.0 + lunge * 0.3;
                set_transform(&mut commands, a.visual, Transform::from_xyz(tip.x, tip.y - 0.15 * s, z + 0.35).with_rotation(rot).with_scale(Vec3::new(s * flip * snap, s * snap, s)));
                if died {
                    sfx.play("SFX_ATTACKS_PLANT_ROTT");
                    a.state = AttackState::Blocked;
                    a.t = 0.0;
                }
            }
            (AttackKind::Plant, AttackState::Blocked) => {
                // Rots (`plantRottingTimer`): the head droops and the vine withers away.
                let k = (a.t / tuning.plant_rot_time).min(1.0);
                let keep = ((1.0 - k) * a.path.len() as f32).ceil() as usize;
                a.path.truncate(keep.max(2));
                draw_vine(&mut commands, &mut meshes, &a, 1.0 - k * 0.5);
                let head = *a.path.last().unwrap_or(&a.pos);
                let s = plant_scale(&assets) * (1.0 - k).max(0.01);
                let droop = Quat::from_rotation_z(k * 1.3 * -a.vel.x.signum());
                set_transform(&mut commands, a.visual, Transform::from_xyz(head.x, head.y - k * 0.4, z + 0.35).with_rotation(droop).with_scale(Vec3::splat(s)));
                if let Some(l) = a.flash {
                    commands.entity(l).insert(Transform::from_xyz(a.path[0].x, 0.0, z + 0.1).with_scale(Vec3::splat(plant_scale(&assets) * (1.0 - k).max(0.01))));
                }
                if a.t >= tuning.plant_rot_time {
                    a.state = AttackState::Finished;
                }
            }
            // ---------------------------------------------------------------- UFO (city special)
            (AttackKind::Ufo, AttackState::Prepare) => {
                if a.visual.is_none() {
                    sfx.play("SFX_ATTACKS_UFO_APPEAR");
                    a.visual = Some(assets.spawn(&mut commands, "ufo", a.from, 3.6, &mut meshes, &mut materials, &mut images));
                }
                // Descends over its target.
                let k = (a.t / tuning.attack_prepare_time).min(1.0);
                a.pos = Vec2::new(a.target_x, UFO_Y + (1.0 - k) * 8.0);
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_rotation(Quat::from_rotation_y(a.t * 3.0)).with_scale(Vec3::splat(assets.scale("ufo", 3.6))));
                if a.t >= tuning.attack_prepare_time {
                    sfx.play("SFX_ATTACKS_UFO_BEAM");
                    let x = a.target_x;
                    let (bottom, blocked) = match vertical_block(&barriers, target, x, UFO_Y - 0.6, 0.0) {
                        Some((y, be)) => {
                            if let Ok((_, mut b)) = barriers.get_mut(be) {
                                b.flash = 0.4;
                            }
                            (y, true)
                        }
                        None => (0.0, false),
                    };
                    a.effect = Some(assets.spawn_beam(&mut commands, x, bottom, UFO_Y - 0.6, z, 1.3, Vec4::new(0.5, 0.9, 1.0, 0.6), &mut meshes, &mut materials));
                    if blocked {
                        a.state = AttackState::Blocked;
                    } else {
                        sfx.play("SFX_ATTACKS_UFO_SUCCESS");
                        let dmg = tuning.weight_damage.damage(a.quality);
                        damage_chicks(&mut chicks, &mut sfx, Vec2::new(x, 0.8), 1.4, 1.0, dmg, 0.5, false);
                        a.state = AttackState::Impact;
                    }
                    a.t = 0.0;
                }
            }
            (AttackKind::Ufo, AttackState::Blocked | AttackState::Impact) => {
                if a.t > UFO_BEAM_TIME {
                    if let Some(fx) = a.effect.take() {
                        commands.entity(fx).despawn();
                        sfx.play("SFX_ATTACKS_UFO_DISAPPEAR");
                    }
                    // Flies off.
                    a.pos.y += (a.t - UFO_BEAM_TIME) * 0.6;
                }
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_rotation(Quat::from_rotation_y(a.t * 3.0)).with_scale(Vec3::splat(assets.scale("ufo", 3.6))));
                if a.t >= UFO_BEAM_TIME + 1.5 {
                    a.state = AttackState::Finished;
                }
            }
            // ---------------------------------------------------------------- sea monster (ship special)
            (AttackKind::Octopus, AttackState::Prepare) => {
                // A tentacle rises at the outer edge of the target side.
                let outer = target.side() * (tuning.side_width + 1.2);
                if a.visual.is_none() {
                    sfx.play("SFX_ATTACKS_OCTOPUS_APPEAR");
                    a.visual = Some(assets.spawn_posed(&mut commands, "tentacle", "tentacle__intro", a.from, TENTACLE_LEN, &mut meshes, &mut materials, &mut images));
                }
                let k = (a.t / tuning.attack_prepare_time).min(1.0);
                a.pos = Vec2::new(outer, -3.0 + (TENTACLE_Y + 3.0) * k);
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y - TENTACLE_LEN * 0.5, z).with_scale(Vec3::splat(assets.scale("tentacle", TENTACLE_LEN))));
                if a.t >= tuning.attack_prepare_time {
                    sfx.play("SFX_ATTACKS_OCTOPUS_WHIP");
                    if let Some(v) = a.visual {
                        assets.play(&mut commands, v, "tentacle__slap");
                    }
                    a.state = AttackState::Travel;
                    a.t = 0.0;
                }
            }
            (AttackKind::Octopus, AttackState::Travel) => {
                // The tip sweeps inward along the ground; a wall across its path stops it.
                let outer = target.side() * (tuning.side_width + 1.2);
                let inner = target.side() * (tuning.separator / 2.0 + 0.3);
                let k = (a.t / TENTACLE_SWEEP_TIME).min(1.0);
                let prev = a.pos.x;
                let tip = outer + (inner - outer) * k;
                let seg_a = Vec2::new(prev, TENTACLE_Y);
                let seg_b = Vec2::new(tip, TENTACLE_Y);
                if let Some((p, _, be)) = sweep_hit(&barriers, target, seg_a, seg_b, 0.5) {
                    if let Ok((_, mut b)) = barriers.get_mut(be) {
                        b.flash = 0.4;
                    }
                    sfx.play("SFX_ATTACKS_OCTOPUS_BLOCKED");
                    if let Some(v) = a.visual {
                        assets.play(&mut commands, v, "tentacle__block");
                    }
                    a.pos.x = p.x;
                    a.state = AttackState::Blocked;
                    a.t = 0.0;
                } else {
                    a.pos.x = tip;
                    let dmg = tuning.bomb_damage.damage(a.quality) * 0.8;
                    for (ce, mut c) in chicks.iter_mut() {
                        if c.team == target && c.alive() && (c.pos.x - tip).abs() < 0.9 && !a.hit.contains(&ce) {
                            c.damage(dmg, Vec2::new(-target.side() * 3.0, 3.0));
                            sfx.play_one_of(&OUCH, a.hit.len());
                            a.hit.push(ce);
                        }
                    }
                    if k >= 1.0 {
                        a.state = AttackState::Impact;
                        a.t = 0.0;
                    }
                }
                // The tentacle leans in from the edge, its tip following the sweep.
                let reach = (a.pos.x - outer).abs().max(0.5);
                let angle = (reach / TENTACLE_LEN).clamp(0.0, 1.0).asin();
                set_transform(&mut commands, a.visual, Transform::from_xyz(outer, 0.0, z).with_rotation(Quat::from_rotation_z(target.side() * angle)).with_scale(Vec3::splat(assets.scale("tentacle", TENTACLE_LEN))));
            }
            (AttackKind::Octopus, AttackState::Blocked | AttackState::Impact) => {
                let outer = target.side() * (tuning.side_width + 1.2);
                // Withdraws.
                let k = (a.t / 0.8).min(1.0);
                let x = a.pos.x + (outer - a.pos.x) * k;
                let reach = (x - outer).abs().max(0.1);
                let angle = (reach / TENTACLE_LEN).clamp(0.0, 1.0).asin();
                set_transform(&mut commands, a.visual, Transform::from_xyz(outer, -k * 3.0, z).with_rotation(Quat::from_rotation_z(target.side() * angle)).with_scale(Vec3::splat(assets.scale("tentacle", TENTACLE_LEN))));
                if a.t >= 1.0 {
                    sfx.play("SFX_ATTACKS_OCTOPUS_DISAPPEAR");
                    a.state = AttackState::Finished;
                }
            }
            // ---------------------------------------------------------------- ghost (haunted wood special)
            (AttackKind::Ghost, AttackState::Prepare) => {
                if a.visual.is_none() {
                    sfx.play("SFX_ATTACKS_GHOST_APPEAR");
                    a.pos = Vec2::new(target.side() * (tuning.side_width + 3.0), 8.0);
                    a.visual = Some(assets.spawn_posed(&mut commands, "ghost", "ghost__loop", a.from, 2.2, &mut meshes, &mut materials, &mut images));
                }
                let fade = (a.t / tuning.attack_prepare_time).min(1.0);
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z + 0.4).with_scale(Vec3::splat(assets.scale("ghost", 2.2) * fade.max(0.05))));
                if a.t >= tuning.attack_prepare_time {
                    a.state = AttackState::Travel;
                    a.t = 0.0;
                }
            }
            (AttackKind::Ghost, AttackState::Travel) => {
                // Swoops at its target, eating any barrier it touches.
                let goal = Vec2::new(a.target_x, 1.0);
                let dir = (goal - a.pos).normalize_or(Vec2::NEG_Y);
                let wobble = Vec2::new(0.0, (a.t * 6.0).sin() * 0.8);
                let next = a.pos + (dir * GHOST_SPEED + wobble) * dt;
                if let Some((_, _, be)) = sweep_hit(&barriers, target, a.pos, next, 0.8) {
                    commands.entity(be).despawn();
                    a.eaten += 1;
                    sfx.play_one_of(&["SFX_ATTACKS_GHOST_LINEMUNCHER_EAT_1", "SFX_ATTACKS_GHOST_LINEMUNCHER_EAT_2", "SFX_ATTACKS_GHOST_LINEMUNCHER_EAT_3"], a.eaten as usize);
                    if a.eaten >= GHOST_BARRIERS_TO_STOP {
                        sfx.play("SFX_ATTACKS_GHOST_DEFEND");
                        a.state = AttackState::Blocked;
                        a.t = 0.0;
                    }
                }
                a.pos = next;
                if a.pos.distance(goal) < 0.4 {
                    sfx.play("SFX_ATTACKS_GHOST_DMG");
                    let dmg = tuning.weight_damage.damage(a.quality);
                    damage_chicks(&mut chicks, &mut sfx, goal, 1.4, 1.0, dmg, 1.0, false);
                    a.state = AttackState::Impact;
                    a.t = 0.0;
                }
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z + 0.4).with_scale(Vec3::splat(assets.scale("ghost", 2.2))));
            }
            (AttackKind::Ghost, AttackState::Blocked | AttackState::Impact) => {
                // Drifts up and fades away.
                a.pos.y += dt * 4.0;
                let fade = (1.0 - a.t).max(0.01);
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z + 0.4).with_scale(Vec3::splat(assets.scale("ghost", 2.2) * fade)));
                if a.t >= 1.0 {
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
                    set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_scale(Vec3::splat(weight_scale(&assets, a.variant, a.size) * fade)));
                    set_transform(&mut commands, a.aux, shadow_transform(a.pos.x, z, a.size * fade));
                }
                if a.t >= 1.5 {
                    a.state = AttackState::Finished;
                }
            }
            (_, AttackState::Finished) => {
                for v in [a.visual.take(), a.effect.take(), a.flash.take(), a.aux.take()].into_iter().flatten() {
                    commands.entity(v).despawn();
                }
                commands.entity(e).despawn();
            }
            _ => a.state = AttackState::Finished,
        }
        if a.state != before {
            debug!("{:?} from {:?}: {:?} -> {:?} at ({:.1}, {:.1}) q {:.2}", a.kind, a.from, before, a.state, a.pos.x, a.pos.y, a.quality);
            if let Ok(on) = std::env::var("CCB_SHOT_ON") {
                let mut it = on.split(':');
                let (k, st, n) = (it.next().unwrap_or(""), it.next().unwrap_or(""), it.next().and_then(|v| v.parse().ok()).unwrap_or(10));
                if let Some(shot) = shot.as_mut().filter(|s| s.0.is_none()) {
                    if a.kind.gesture_group() == k && format!("{:?}", a.state).eq_ignore_ascii_case(st) {
                        shot.0 = Some(n);
                    }
                }
            }
        }
    }
}
