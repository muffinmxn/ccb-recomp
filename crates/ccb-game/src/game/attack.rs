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
/// A weight drops slowly at first so the defender can react (about 1.4 s from the top).
const WEIGHT_GRAVITY: f32 = 14.0;
const WEIGHT_START_Y: f32 = 16.0;
/// Take-off speed of the sumo's second jump.
const SUMO_BOUNCE: f32 = 9.0;
const STRIKE_RADIUS: f32 = 1.3;
/// Plant head size (`plantHeadSize`) and vine thickness.
const PLANT_HEAD: f32 = 1.7;
const STEM_WIDTH: f32 = 0.24;
const STEM_OUTLINE: f32 = 0.38;
/// `plantSpiralR` / `plantSpiralSegLen`: how the vine winds around a line it climbs.
const SPIRAL_R: f32 = 0.25;
const SPIRAL_LEN: f32 = 1.0;
/// `plantBarrierSegmentDistance`.
const PLANT_GRAB: f32 = 1.1;
const PLANT_LIFETIME: f32 = 10.0;
/// `plantInitialPenducle` is in the original's plant units; this makes the stem about a
/// chick and a half tall.
const PENDUCLE_SCALE: f32 = 1.3;
/// Model sizes for the specials (the ghost's loop clip scales it up ~2.6x).
const GHOST_LOOK: f32 = 0.85;
const OCTOPUS_LOOK: f32 = 5.5;
const TENTACLE_LEN: f32 = 7.0;

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
    vine_key: u32,
    /// The team this attack hits, when not the attacker's opponent (lightning).
    against: Option<Team>,
    /// Where the environment object that became this attack was.
    pub origin: Vec3,
    /// Specials: the chick being targeted, hits done, time budget, wait timer.
    victim: Option<Entity>,
    hits: u32,
    life: f32,
    wait: f32,
    /// Blown up early (lightning, touched while drawing): quality to explode with.
    pub detonate: Option<f32>,
    /// Upgrade: 0 none, 1 green (A), 2 red (B).
    pub upgrade: u8,
    /// Spawned by another attack (cluster bombs); the turn waits for these too.
    pub child: bool,
    /// The model used for its look (upgrades swap it).
    model: &'static str,
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
            size: 1.0,
            path: Vec::new(),
            climbing: None,
            starving: 0.0,
            starve_limit: 5.0,
            cooldown: 0.0,
            bite: None,
            high: 0.0,
            aux: None,
            vine_key: 0,
            against: None,
            origin: Vec3::ZERO,
            victim: None,
            hits: 0,
            life: 0.0,
            wait: 0.0,
            detonate: None,
            upgrade: 0,
            child: false,
            model: "",
        }
    }

    pub fn upgraded(mut self, upgrade: u8) -> Self {
        self.upgrade = upgrade;
        self
    }

    pub fn target(&self) -> Team {
        self.against.unwrap_or(self.from.other())
    }

    /// Hits `team` instead of the attacker's opponent.
    pub fn against(mut self, team: Team) -> Self {
        self.against = Some(team);
        self
    }

    pub fn from_origin(mut self, origin: Vec3) -> Self {
        self.origin = origin;
        self
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

/// The UFO tilts into its flight direction (it faces the camera; the original doesn't spin it).
fn ufo_tilt(dx: f32) -> Quat {
    Quat::from_rotation_z((dx * 0.08).clamp(-0.25, 0.25))
}

/// Model scale for a weight variant `size` wide.
fn weight_scale(assets: &GameAssets, variant: usize, size: f32) -> f32 {
    size / assets.weight_visible.get(variant.clamp(1, 7) - 1).copied().unwrap_or(1.5)
}

/// A falling weight's shadow: a flat ellipse on the ground, `width` wide.
fn shadow_transform(x: f32, z: f32, width: f32) -> Transform {
    Transform::from_xyz(x, 0.03, z).with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)).with_scale(Vec3::new(width, 1.2, 1.0))
}

/// Model scale that makes the flytrap head `plantHeadSize` wide.
fn plant_scale(assets: &GameAssets, model: &str) -> f32 {
    let model = if model.is_empty() { "plant" } else { model };
    PLANT_HEAD / assets.bone_width(model, "head").max(1e-3)
}

/// Rebuilds the vine's ribbons along the plant's path.
fn draw_vine(commands: &mut Commands, meshes: &mut Assets<Mesh>, a: &mut Attack, width: f32) {
    // Only rebuild when the vine changed (new meshes take a frame to reach the GPU).
    let key = a.path.len() as u32 * 1000 + (width * 999.0) as u32;
    if a.path.len() < 2 || a.vine_key == key {
        return;
    }
    a.vine_key = key;
    for (e, w) in [(a.effect, STEM_OUTLINE), (a.aux, STEM_WIDTH)] {
        if let Some(e) = e {
            commands.entity(e).insert(Mesh3d(meshes.add(super::barrier::stroke(&a.path, w * width))));
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

/// The plant's intro pop (`plant__intro`: its root scale over 16 frames, relative to the
/// final 2.49), as (x, y) per frame 0, 4, 8, 12, 16; it starts from nothing.
const PLANT_INTRO: [(f32, f32); 5] = [(0.0, 0.0), (0.53, 0.59), (0.93, 1.05), (0.98, 0.92), (1.0, 1.0)];
const PLANT_INTRO_TIME: f32 = 16.0 / 60.0;

/// Head scale (x, y) `t` seconds after the head appeared.
fn plant_pop(t: f32) -> Vec2 {
    let f = (t / PLANT_INTRO_TIME).clamp(0.0, 1.0) * 4.0;
    let i = (f.floor() as usize).min(3);
    let (a, b) = (PLANT_INTRO[i], PLANT_INTRO[i + 1]);
    let k = f - i as f32;
    Vec2::new(a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k)
}

/// One frame of a plant. As in the original it sprouts a short stem (`plantInitialPenducle`)
/// where it was aimed and bites every chick that comes close; it only grows by climbing a line
/// within `plantBarrierSegmentDistance` of its head, along it and only upwards.
/// Returns true when it dies (starved, too long above `plantGrowYThreshold`).
fn grow_plant(a: &mut Attack, tuning: &Tuning, barriers: &Query<(Entity, &mut Barrier)>, chicks: &mut Query<(Entity, &mut Chick)>, sfx: &mut crate::sfx::Sfx, dt: f32) -> bool {
    let target = a.target();
    a.starving += dt;
    a.cooldown -= dt;
    let head = a.pos;
    let prey = chicks
        .iter()
        .filter(|(_, c)| c.team == target && c.alive())
        .map(|(_, c)| c.pos)
        .min_by(|x, y| x.distance(head).total_cmp(&y.distance(head)));
    // Biting: a short windup, then the snap hits every chick in range.
    if let Some(b) = a.bite {
        let b = b + dt;
        if b >= tuning.plant_bite_time && b - dt < tuning.plant_bite_time {
            let dmg = tuning.plant_damage.damage(a.quality);
            let reach = head + a.vel * 0.5;
            let mut bit = 0;
            for (_, mut c) in chicks.iter_mut() {
                if c.team != target || !c.alive() || c.pos.distance(reach) > tuning.plant_bite_range + c.radius * 0.5 {
                    continue;
                }
                let push = (c.pos - head).normalize_or(Vec2::Y) * 1.5 + Vec2::Y;
                c.damage(dmg, push);
                sfx.play(if c.dying.is_some() { "SFX_CHICKS_DIE" } else { OUCH[bit % OUCH.len()] });
                bit += 1;
                // Scorpion Fern poisons, Fire Flower confuses.
                let mid = |r: (f32, f32)| (r.0 + r.1) / 2.0;
                match a.upgrade {
                    1 => {
                        c.sick = mid(tuning.up.sick_time);
                        c.sick_extra = tuning.up.sick_dmg;
                    }
                    2 => c.confused = mid(tuning.up.confused_time),
                    _ => {}
                }
            }
            if bit > 0 {
                a.starving = 0.0;
                if a.upgrade == 1 {
                    sfx.play("SFX_ATTACKS_PLANT_TOXIC_BITE");
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
    // Sprouting: the stem unrolls along its initial shape.
    if a.t < tuning.plant_sprout_time {
        return false;
    }
    // A chick close by (`plantBiteRangePrepare` .. `plantBiteRange`): snap at it.
    if let Some(p) = prey {
        if p.distance(head) <= tuning.plant_bite_range && a.cooldown <= 0.0 {
            a.vel = (p - head).normalize_or(a.vel);
            a.bite = Some(0.0);
            return false;
        }
    }
    let len: f32 = a.path.windows(2).map(|w| w[0].distance(w[1])).sum();
    let step = tuning.plant_grow_speed * dt;
    if len < tuning.plant_grow_max_len {
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
                        a.victim = Some(be);
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
            // A line within reach of the head (not the one it just left) pulls it over;
            // otherwise the plant stays put, turning towards the chicks.
            let left_line = a.victim;
            let near = barriers
                .iter()
                .filter(|(e, b)| b.team == target && b.points.len() >= 2 && Some(*e) != left_line)
                .flat_map(|(e, b)| {
                    b.points.windows(2).enumerate().map(move |(i, w)| {
                        let seg = w[1] - w[0];
                        let t = ((head - w[0]).dot(seg) / seg.length_squared().max(1e-6)).clamp(0.0, 1.0);
                        (e, i, w[0] + seg * t, if w[1].y >= w[0].y { 1 } else { -1 })
                    })
                })
                .filter(|x| x.2.y >= head.y - 0.3)
                .min_by(|x, y| x.2.distance(head).total_cmp(&y.2.distance(head)))
                .filter(|x| x.2.distance(head) < PLANT_GRAB);
            match near {
                Some((be, seg, p, dir)) => {
                    if a.pos.distance(p) <= step {
                        a.pos = p;
                        a.climbing = Some((be, seg, dir));
                    } else {
                        a.vel = (p - a.pos).normalize_or(Vec2::Y);
                        a.pos += a.vel * step;
                    }
                }
                None => {
                    let want = prey.map_or(a.vel, |p| (p - head).normalize_or(Vec2::Y));
                    let k = 1.0 - (1.0 - tuning.plant_head_dir_interp).powf(dt * 60.0);
                    a.vel = a.vel.lerp(want, k).normalize_or(Vec2::Y);
                }
            }
        }
        // Stay on the target side.
        let (lo, hi) = tuning.side_range(target);
        a.pos.x = a.pos.x.clamp(lo - 0.5, hi + 0.5);
        // Climbing, the vine winds around the line.
        let shown = if a.climbing.is_some() {
            a.life += step;
            let n = Vec2::new(-a.vel.y, a.vel.x);
            a.pos + n * SPIRAL_R * (a.life / SPIRAL_LEN * std::f32::consts::TAU).sin()
        } else {
            a.pos
        };
        if a.path.last().is_none_or(|l| l.distance(shown) > 0.08) {
            a.path.push(shown);
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
    // Lightning strikes this frame: (team, x range); bombs and plants there get hit too.
    let mut strikes: Vec<(Team, f32, f32)> = Vec::new();
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
                    // Frog Cracker (green) and Ladybug Boom (red) have their own looks.
                    a.model = match a.upgrade {
                        1 => "bombA",
                        2 => "bombB",
                        _ => "bomb",
                    };
                    a.visual = Some(assets.spawn_posed(&mut commands, a.model, &format!("{}__fly", a.model), a.from, a.size * BOMB_LOOK, &mut meshes, &mut materials, &mut images));
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
                // Mini bombs flutter down on their wings (`bomb.mini` lanes damp hard).
                let air_damp = if a.child { tuning.up.mini.lanes.first().map_or(0.06, |l| l.1) } else { BOMB_DAMP };
                let damp = if rolling { 1.0 - 3.0 * h } else { 1.0 - air_damp };
                let model = if a.model.is_empty() { "bomb" } else { a.model };
                let (roll_time, tick_time) = if a.child { (tuning.up.mini.roll_time + a.wait, tuning.up.mini.tick_time) } else { (tuning.bomb_roll_time, tuning.bomb_tick_time) };
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
                if !rolling && a.vel.y <= 0.0 && a.pos.y <= tuning.bomb_roll_y + r && a.pos.x * target.side() > 0.0 {
                    // `bombRollYThreshold`: down on the field, the fuse is burning.
                    if let Some(v) = a.visual {
                        assets.play(&mut commands, v, &format!("{model}__roll"));
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
                if rolling && (a.t / tick_time).floor() != ((a.t - dt) / tick_time).floor() {
                    sfx.play("SFX_ATTACKS_BOMB_COUNTDOWN");
                }
                let spin = -a.pos.x / r;
                let pulse = if rolling { 1.0 + 0.12 * (a.t * 18.0).sin() } else { 1.0 };
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_rotation(Quat::from_rotation_z(spin)).with_scale(Vec3::splat(assets.scale(model, r * BOMB_LOOK) * pulse)));
                if a.state != AttackState::Finished && ((rolling && a.t >= roll_time) || a.detonate.is_some()) {
                    let q = a.detonate.take().unwrap_or(a.quality).max(a.quality);
                    let mini = &tuning.up.mini;
                    let ((rmin, rmax), damage, move_by) = if a.child { (mini.exp_radius, mini.damage, mini.move_by) } else { (tuning.bomb_exp_radius, tuning.bomb_damage, tuning.bomb_move_by) };
                    let radius = rmin + (rmax - rmin) * q;
                    let dmg = damage.damage(q);
                    let push = move_by.0 + (move_by.1 - move_by.0) * q;
                    blast(&mut chicks, &barriers, &mut sfx, a.pos, radius, tuning.bomb_exp_full_radius, dmg, push * 30.0);
                    if let Some(v) = a.visual.take() {
                        commands.entity(v).despawn();
                    }
                    sfx.play(match (a.child, a.upgrade) {
                        (true, _) => "SFX_ATTACKS_BOMB_CLUSTER_EXPL",
                        (_, 1) => "SFX_ATTACKS_BOMB_ACID_EXPLOSION",
                        _ => "SFX_ATTACKS_BOMB_EXPL",
                    });
                    match a.upgrade {
                        // Frog Cracker: acid splashes out and eats at chicks where it lands.
                        1 => {
                            let n = tuning.up.bomb_acid_count;
                            for i in 0..n {
                                let side = i as f32 - (n as f32 - 1.0) / 2.0;
                                let vel = Vec2::new(side * 5.0 + rng.range((-1.0, 1.0)), rng.range((7.0, 10.0)));
                                super::upgrade::spawn_hazard(&mut commands, &assets, &tuning, super::upgrade::HazardKind::Acid, target, a.pos + Vec2::Y * 0.3, vel, &mut meshes, &mut materials, &mut images);
                            }
                        }
                        // Ladybug Boom: its babies flutter out and go off one after another.
                        2 if !a.child => {
                            for i in 0..tuning.up.bomb_cluster_count {
                                let lanes = &tuning.up.mini.lanes;
                                let lane = lanes.get(rng.range((0.0, lanes.len() as f32)) as usize).map_or(Vec2::new(5.0, 20.0), |l| l.0);
                                let dir = if i % 2 == 0 { -1.0 } else { 1.0 };
                                let mut b = Attack::new(AttackKind::Bomb, a.from, a.quality, a.pos.x);
                                b.against = a.against;
                                b.child = true;
                                b.model = "bombB";
                                b.state = AttackState::Travel;
                                b.pos = a.pos + Vec2::Y * 0.3;
                                b.vel = Vec2::new(lane.x * dir, lane.y);
                                b.size = tuning.bomb_min_size * mini.scale;
                                b.wait = tuning.up.mini.start_ticking - tuning.up.mini.roll_time + i as f32 * 0.5;
                                b.visual = Some(assets.spawn_posed(&mut commands, "bombB", "bombB__flutter", a.from, b.size * BOMB_LOOK, &mut meshes, &mut materials, &mut images));
                                commands.spawn((b, DespawnOnExit(crate::Screen::Level)));
                            }
                        }
                        _ => {}
                    }
                    a.size = radius;
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
                    // Sumo Chick (red) is its own weight.
                    if a.upgrade == 2 {
                        a.variant = 7;
                    }
                    let (lo, hi) = tuning.side_range(target);
                    let border = tuning.weight_border_offset.min((hi - lo) / 2.0);
                    a.target_x = a.target_x.clamp(lo + border, hi - border);
                    a.pos = Vec2::new(a.target_x, WEIGHT_START_Y);
                    // Its shadow shows where it will land.
                    a.aux = Some(assets.spawn_shadow(&mut commands, z, &mut meshes, &mut materials));
                    // Chick Fixer (green): glue drops on the chicks nearest the drop first.
                    if a.upgrade == 1 {
                        let mut near: Vec<Vec2> = chicks.iter().filter(|(_, c)| c.team == target && c.alive()).map(|(_, c)| c.pos).collect();
                        let tx = a.target_x;
                        near.sort_by(|p, q| (p.x - tx).abs().total_cmp(&(q.x - tx).abs()));
                        for p in near.iter().take(tuning.up.weight_glue_count) {
                            let at = Vec2::new(p.x, WEIGHT_START_Y * 0.6);
                            super::upgrade::spawn_hazard(&mut commands, &assets, &tuning, super::upgrade::HazardKind::Glue, target, at, Vec2::new(0.0, -10.0), &mut meshes, &mut materials, &mut images);
                        }
                        sfx.play("SFX_ATTACKS_WEIGHT_GLUE");
                    }
                }
                let k = (a.t / (tuning.attack_prepare_time * 0.6)).min(1.0);
                set_transform(&mut commands, a.aux, shadow_transform(a.target_x, z, a.size * (0.3 + 0.3 * k)));
                if a.t >= tuning.attack_prepare_time * 0.6 {
                    a.vel = Vec2::ZERO;
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
                // (Only on the way down: a bouncing sumo rises through lines.)
                let hit = (0..=6)
                    .map(|i| -half + a.size * i as f32 / 6.0)
                    .filter(|_| a.vel.y < 0.0)
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
                if a.state == AttackState::Travel && a.pos.y <= low && a.vel.y <= 0.0 {
                    a.pos.y = low;
                    let dmg = tuning.weight_damage.damage(a.quality);
                    sfx.play(if a.variant == 7 { "SFX_ATTACKS_WEIGHT_SUMO" } else { WEIGHT_SOUNDS[a.variant.clamp(1, 6) - 1] });
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
                    if a.variant == 7 && a.hits == 0 {
                        // The sumo bounces and comes down again on the nearest chick.
                        a.hits = 1;
                        let here = a.pos.x;
                        let aim = chicks
                            .iter()
                            .filter(|(_, c)| c.team == target && c.alive())
                            .map(|(_, c)| c.pos.x)
                            .min_by(|p, q| (p - here).abs().total_cmp(&(q - here).abs()))
                            .unwrap_or(here);
                        let (lo, hi) = tuning.side_range(target);
                        let border = tuning.weight_border_offset.min((hi - lo) / 2.0);
                        let tx = aim.clamp(lo + border, hi - border);
                        let up = SUMO_BOUNCE;
                        a.vel = Vec2::new((tx - a.pos.x) / (2.0 * up / WEIGHT_GRAVITY), up);
                        a.target_x = tx;
                        if let Some(v) = a.visual {
                            assets.play(&mut commands, v, "weight__weightSumoJump");
                        }
                    } else {
                        a.state = AttackState::Impact;
                        a.t = 0.0;
                    }
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
                // The dark cloud hangs over its side and charges for `lightningApproachTimer`.
                if a.visual.is_none() {
                    a.pos = Vec2::new(a.target_x, tuning.cloud_y);
                    a.visual = Some(assets.spawn(&mut commands, "cloud", a.from, 5.0, &mut meshes, &mut materials, &mut images));
                    sfx.play("SFX_ATTACKS_THUNDER_APPROACHING");
                }
                let shake = if a.t > tuning.sp.lightning_approach - 1.0 { (a.t * 40.0).sin() * 0.06 } else { 0.0 };
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x + shake, tuning.cloud_y, z).with_scale(Vec3::splat(assets.scale("cloud", 5.0))));
                if a.t >= tuning.sp.lightning_approach {
                    // The bolt hits the highest point under the cloud: an earthed line grounds it.
                    let span = 2.5;
                    let (lo, hi) = (a.pos.x - span, a.pos.x + span);
                    let rod = barriers
                        .iter()
                        .filter(|(_, b)| b.team == target && b.points.iter().any(|p| p.y <= 0.35))
                        .filter_map(|(e, b)| b.points.iter().copied().filter(|p| p.x >= lo && p.x <= hi).max_by(|p, q| p.y.total_cmp(&q.y)).map(|top| (e, top)))
                        .max_by(|x, y| x.1.y.total_cmp(&y.1.y));
                    let chick_top = chicks
                        .iter()
                        .filter(|(_, c)| c.team == target && c.alive() && c.pos.x >= lo && c.pos.x <= hi)
                        .map(|(_, c)| c.pos + Vec2::Y * c.radius)
                        .max_by(|p, q| p.y.total_cmp(&q.y));
                    let (hit, grounded) = match (rod, chick_top) {
                        (Some((be, top)), c) if c.is_none_or(|c| top.y >= c.y) => {
                            if let Ok((_, mut b)) = barriers.get_mut(be) {
                                b.flash = 0.4;
                            }
                            sfx.play("SFX_ENV_LINE_GROUNDED");
                            (top, true)
                        }
                        (_, Some(c)) => (c, false),
                        _ => (Vec2::new(a.pos.x, 0.0), false),
                    };
                    if !grounded {
                        let dmg = tuning.sp.lightning_damage.damage(a.quality);
                        let hits = damage_chicks(&mut chicks, &mut sfx, hit - Vec2::Y * 0.3, STRIKE_RADIUS, STRIKE_RADIUS * 0.6, dmg, 2.0, false);
                        if hits > 0 {
                            sfx.play("SFX_CHICKS_ELECTRIFIED");
                        }
                        // Bombs and plants under the cloud get it too.
                        strikes.push((target, lo, hi));
                    }
                    sfx.play("SFX_ATTACKS_THUNDER");
                    let flash = assets.spawn_posed(&mut commands, "cloudLightning", "cloudLightning__spratzel", a.from, 5.0, &mut meshes, &mut materials, &mut images);
                    commands.entity(flash).insert(Transform::from_xyz(a.pos.x, tuning.cloud_y, z + 0.2).with_scale(Vec3::splat(assets.scale("cloudLightning", 5.0))));
                    a.flash = Some(flash);
                    a.effect = Some(assets.spawn_bolt(&mut commands, hit.x, hit.y, tuning.cloud_y - 0.6, z + 0.4, &mut meshes, &mut materials));
                    a.state = if grounded { AttackState::Blocked } else { AttackState::Impact };
                    a.t = 0.0;
                }
            }
            (AttackKind::Lightning, AttackState::Blocked | AttackState::Impact) => {
                if a.t > tuning.sp.lightning_effective {
                    if let Some(fx) = a.effect.take() {
                        commands.entity(fx).despawn();
                    }
                }
                let fade = (1.0 - a.t / 1.0).max(0.01);
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, tuning.cloud_y, z).with_scale(Vec3::splat(assets.scale("cloud", 5.0) * fade)));
                if a.t >= 1.0 {
                    a.state = AttackState::Finished;
                }
            }
            // ---------------------------------------------------------------- plant
            (AttackKind::Plant, AttackState::Prepare) => {
                if a.t >= tuning.attack_prepare_time {
                    // Sprouts at its anchor, its stem leaning towards the nearest chick.
                    a.pos = Vec2::new(a.target_x, 0.0);
                    a.path = vec![a.pos];
                    let here = a.pos.x;
                    let (lo, hi) = tuning.side_range(target);
                    let toward = chicks
                        .iter()
                        .filter(|(_, c)| c.team == target && c.alive())
                        .map(|(_, c)| c.pos.x)
                        .min_by(|p, q| (p - here).abs().total_cmp(&(q - here).abs()))
                        .unwrap_or((lo + hi) / 2.0);
                    a.wait = if toward >= here { 1.0 } else { -1.0 };
                    a.vel = Vec2::new(a.wait, 1.0).normalize();
                    a.starve_limit = rng.range(tuning.plant_starving);
                    a.cooldown = tuning.plant_start_bite_cooldown;
                    sfx.play("SFX_ATTACKS_PLANT_APPEAR");
                    // The flytrap head rides the vine's tip; the leaves stay at the root.
                    // Scorpion Fern (green) and Fire Flower (red) have their own heads.
                    a.model = match a.upgrade {
                        1 => "plantA",
                        2 => "plantB",
                        _ => "plant",
                    };
                    let bones: &[&str] = if a.model == "plant" { &["head", "plantInnerMouth"] } else { &["head"] };
                    let head = assets.spawn_bones(&mut commands, a.model, a.from, 1.0, Some(bones), &mut meshes, &mut materials, &mut images);
                    a.visual = Some(head);
                    let leaves = assets.spawn_bones(&mut commands, "plant", a.from, 1.0, Some(&["leaves"]), &mut meshes, &mut materials, &mut images);
                    commands.entity(leaves).insert(Transform::from_xyz(a.pos.x, 0.0, z + 0.1).with_scale(Vec3::splat(plant_scale(&assets, "plant"))));
                    a.flash = Some(leaves);
                    // The vine: a dark outline under a lighter green fill.
                    a.effect = Some(assets.spawn_stem(&mut commands, z + 0.70, Vec4::new(0.08, 0.25, 0.04, 1.0), &mut materials));
                    a.aux = Some(assets.spawn_stem(&mut commands, z + 0.71, Vec4::new(0.42, 0.72, 0.16, 1.0), &mut materials));
                    a.state = AttackState::Travel;
                    a.t = 0.0;
                }
            }
            (AttackKind::Plant, AttackState::Travel) => {
                // Sprouting: the stem unrolls along `plantInitialPenducle`.
                if a.t < tuning.plant_sprout_time {
                    let shape = tuning.plant_penducle.get(a.upgrade as usize).or(tuning.plant_penducle.first()).cloned().unwrap_or_else(|| vec![Vec2::new(0.2, 0.6), Vec2::new(0.3, 1.1)]);
                    let root = a.path[0];
                    let mut full = vec![root];
                    full.extend(shape.iter().map(|p| root + Vec2::new(p.x * a.wait, p.y) * PENDUCLE_SCALE));
                    let total: f32 = full.windows(2).map(|w| w[0].distance(w[1])).sum();
                    let mut left = total * (a.t / tuning.plant_sprout_time).min(1.0);
                    let mut path = vec![root];
                    for w in full.windows(2) {
                        let d = w[0].distance(w[1]);
                        if left >= d {
                            path.push(w[1]);
                            left -= d;
                        } else {
                            path.push(w[0] + (w[1] - w[0]) * (left / d.max(1e-4)));
                            break;
                        }
                    }
                    a.pos = *path.last().unwrap();
                    if a.t + dt >= tuning.plant_sprout_time {
                        a.path = full;
                        a.pos = *a.path.last().unwrap();
                    } else {
                        a.path = path;
                    }
                    a.vine_key = 0;
                }
                let died = grow_plant(&mut a, &tuning, &barriers, &mut chicks, &mut sfx, dt);
                draw_vine(&mut commands, &mut meshes, &mut a, 1.0);
                // The head sits on the vine's tip, leaning along it; a bite lunges it forward.
                let d = a.vel;
                let s = plant_scale(&assets, a.model);
                let lunge = a.bite.map_or(0.0, |b| (b / tuning.plant_bite_time * std::f32::consts::FRAC_PI_2).sin().max(0.0) * 0.5);
                let tip = a.pos + d * lunge;
                // Upright (the mouth stays level), leaning a little along the vine and turned
                // towards its prey.
                let rot = Quat::from_rotation_y(d.x.clamp(-1.0, 1.0) * 0.6) * Quat::from_rotation_z((-d.x * 0.25).clamp(-0.3, 0.3));
                let snap = 1.0 + lunge * 0.3;
                // The head pops out like `plant__intro` as the stem finishes sprouting.
                let pop = plant_pop(a.t - tuning.plant_sprout_time * 0.5);
                let scale = Vec3::new(pop.x.max(0.01), pop.y.max(0.01), pop.x.max(0.01)) * s * snap;
                set_transform(&mut commands, a.visual, Transform::from_xyz(tip.x, tip.y - 0.15 * s, z + 0.35).with_rotation(rot).with_scale(scale));
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
                draw_vine(&mut commands, &mut meshes, &mut a, 1.0 - k * 0.5);
                let head = *a.path.last().unwrap_or(&a.pos);
                let s = plant_scale(&assets, a.model) * (1.0 - k).max(0.01);
                let droop = Quat::from_rotation_z(k * 1.3 * -a.vel.x.signum());
                set_transform(&mut commands, a.visual, Transform::from_xyz(head.x, head.y - k * 0.4, z + 0.35).with_rotation(droop).with_scale(Vec3::splat(s)));
                if let Some(l) = a.flash {
                    commands.entity(l).insert(Transform::from_xyz(a.path[0].x, 0.0, z + 0.1).with_scale(Vec3::splat(plant_scale(&assets, "plant") * (1.0 - k).max(0.01))));
                }
                if a.t >= tuning.plant_rot_time {
                    a.state = AttackState::Finished;
                }
            }
            // ---------------------------------------------------------------- UFO (city special)
            (AttackKind::Ufo, AttackState::Prepare) => {
                // Flies in from the sky over its victim, down to `ufoBeamLowestPoint`.
                if a.visual.is_none() {
                    a.pos = tuning.sp.ufo_start + Vec2::X * a.origin.x.clamp(-12.0, 12.0) * 0.5;
                    a.life = rng.range(tuning.sp.ufo_attack_time);
                    a.visual = Some(assets.spawn_posed(&mut commands, "ufo", "ufo__look", a.from, 3.6, &mut meshes, &mut materials, &mut images));
                    sfx.play("SFX_ATTACKS_UFO_APPEAR");
                }
                a.victim = pick_victim(&chicks, target, a.target_x, a.victim);
                let goal = a.victim.and_then(|v| chicks.get(v).ok()).map_or(Vec2::new(a.target_x, tuning.sp.ufo_beam_y), |(_, c)| Vec2::new(c.pos.x, tuning.sp.ufo_beam_y));
                a.target_x = goal.x;
                let step = (goal - a.pos).clamp_length_max(6.0 * dt);
                a.pos += step;
                if a.pos.distance(goal) < 0.1 {
                    sfx.play("SFX_ATTACKS_UFO_BEAM");
                    a.state = AttackState::Travel;
                    a.t = 0.0;
                }
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_rotation(ufo_tilt(a.pos.x - a.target_x)).with_scale(Vec3::splat(assets.scale("ufo", 3.6))));
            }
            (AttackKind::Ufo, AttackState::Travel) => {
                // Beams its victim up (`ufoBeamSpeed`); a line between them breaks the beam.
                a.life -= dt;
                let victim = a.victim.and_then(|v| chicks.get(v).ok()).filter(|(_, c)| c.alive()).map(|(e, c)| (e, c.pos, c.radius));
                let Some((ve, vpos, vr)) = victim else {
                    a.victim = None;
                    a.state = if a.life > 0.0 { AttackState::Prepare } else { AttackState::Impact };
                    continue;
                };
                a.pos.x += (vpos.x - a.pos.x).clamp(-3.0 * dt, 3.0 * dt);
                let beam_bottom = vpos.y + vr;
                let blocked = vertical_block(&barriers, target, vpos.x, a.pos.y - 0.6, beam_bottom).map(|(_, be)| be);
                if let Some(be) = blocked {
                    if let Ok((_, mut b)) = barriers.get_mut(be) {
                        b.flash = 0.4;
                    }
                    if let Ok((_, mut c)) = chicks.get_mut(ve) {
                        c.held = false;
                    }
                    sfx.play("SFX_ATTACKS_OCTOPUS_BLOCKED");
                    a.wait = tuning.sp.ufo_beam_shutdown;
                    a.state = AttackState::Blocked;
                    a.t = 0.0;
                } else if let Ok((_, mut c)) = chicks.get_mut(ve) {
                    c.held = true;
                    c.pos.x += (a.pos.x - c.pos.x).clamp(-2.0 * dt, 2.0 * dt);
                    c.pos.y += tuning.sp.ufo_beam_speed * dt;
                    if c.pos.y >= a.pos.y - tuning.sp.ufo_kidnap_offset - 0.6 {
                        // Taken for good.
                        c.held = false;
                        c.abducted = true;
                        c.health = 0.0;
                        c.dying = Some(1.0);
                        sfx.play("SFX_ATTACKS_UFO_SUCCESS");
                        a.state = AttackState::Impact;
                        a.t = 0.0;
                    }
                }
                if a.state == AttackState::Travel && a.life <= 0.0 {
                    if let Ok((_, mut c)) = chicks.get_mut(ve) {
                        c.held = false;
                    }
                    a.state = AttackState::Impact;
                    a.t = 0.0;
                }
                if a.effect.is_none() {
                    a.effect = Some(assets.spawn_beam(&mut commands, a.pos.x, 0.0, a.pos.y - 0.6, z, 1.3, Vec4::new(0.5, 0.9, 1.0, 0.45), &mut meshes, &mut materials));
                }
                if let Some(fx) = a.effect {
                    let h = (a.pos.y - 0.6 - beam_bottom + vr * 2.0).max(0.1);
                    commands.entity(fx).insert(Transform::from_xyz(a.pos.x, beam_bottom - vr * 2.0 + h / 2.0, z + 0.75).with_scale(Vec3::new(1.0, h / (a.pos.y - 0.6).max(0.1), 1.0)));
                }
                if a.state != AttackState::Travel {
                    if let Some(fx) = a.effect.take() {
                        commands.entity(fx).despawn();
                        sfx.play("SFX_ATTACKS_UFO_DISAPPEAR");
                    }
                }
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_rotation(ufo_tilt(a.pos.x - a.target_x)).with_scale(Vec3::splat(assets.scale("ufo", 3.6))));
            }
            (AttackKind::Ufo, AttackState::Blocked) => {
                // Beam shut down (`ufoBeamShutdownTime`), then it tries again while it has time.
                a.life -= dt;
                if let Some(fx) = a.effect.take() {
                    commands.entity(fx).despawn();
                }
                if a.t >= a.wait + 0.6 {
                    a.state = if a.life > 0.0 { AttackState::Prepare } else { AttackState::Impact };
                    a.victim = None;
                    a.t = 0.0;
                }
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_rotation(ufo_tilt(a.pos.x - a.target_x)).with_scale(Vec3::splat(assets.scale("ufo", 3.6))));
            }
            (AttackKind::Ufo, AttackState::Impact) => {
                // Flies away (`ufoFlyAwaySpeed`).
                if let Some(fx) = a.effect.take() {
                    commands.entity(fx).despawn();
                }
                a.pos += Vec2::new(0.4, 1.0) * tuning.sp.ufo_fly_away_speed * 2.0 * dt;
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z).with_rotation(ufo_tilt(-1.0)).with_scale(Vec3::splat(assets.scale("ufo", 3.6))));
                if a.t >= 1.8 {
                    a.state = AttackState::Finished;
                }
            }
            // ---------------------------------------------------------------- sea monster (ship special)
            (AttackKind::Octopus, AttackState::Prepare) => {
                // The octopus surfaces behind the target side, then starts slapping.
                if a.visual.is_none() {
                    let (lo, hi) = tuning.side_range(target);
                    a.pos = Vec2::new((lo + hi) / 2.0, tuning.sp.octopus_y_start);
                    a.visual = Some(assets.spawn_posed(&mut commands, "octopus", "octopus__intro", a.from, OCTOPUS_LOOK, &mut meshes, &mut materials, &mut images));
                    sfx.play("SFX_ATTACKS_OCTOPUS_APPEAR");
                }
                let k = (a.t / 1.05).min(1.0);
                a.pos.y = tuning.sp.octopus_y_start + (tuning.sp.octopus_y - tuning.sp.octopus_y_start) * k;
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z - 6.0).with_scale(Vec3::splat(assets.scale("octopus", OCTOPUS_LOOK))));
                if a.t >= tuning.sp.octopus_initial_delay {
                    a.state = AttackState::Travel;
                    a.t = 0.0;
                    a.wait = 0.0;
                }
            }
            (AttackKind::Octopus, AttackState::Travel) => {
                // `octopusAttackCounter` slaps; lines over the chicks parry them.
                if a.effect.is_none() && a.t >= a.wait {
                    a.victim = pick_victim(&chicks, target, rng.range(tuning.side_range(target)), None);
                    a.target_x = a.victim.and_then(|v| chicks.get(v).ok()).map_or(a.target_x, |(_, c)| c.pos.x);
                    let out = target.side() * 2.2;
                    let e = assets.spawn_posed(&mut commands, "tentacle", "tentacle__slap", a.from, TENTACLE_LEN, &mut meshes, &mut materials, &mut images);
                    // Mirrored by turning around (a negative scale would break the skinning).
                    let turn = if target.side() > 0.0 { std::f32::consts::PI } else { 0.0 };
                    commands.entity(e).insert(Transform::from_xyz(a.target_x + out, -0.8, z + 0.2).with_rotation(Quat::from_rotation_y(turn)).with_scale(Vec3::splat(assets.scale("tentacle", TENTACLE_LEN))));
                    a.effect = Some(e);
                    a.t = 0.0;
                    a.wait = f32::MAX;
                    sfx.play("SFX_ATTACKS_OCTOPUS_WHIP");
                }
                let hit_at = tuning.sp.tentacle_time_till_hit;
                if a.effect.is_some() && a.t >= hit_at && a.t - dt < hit_at {
                    let r = tuning.sp.tentacle_hit_size;
                    let block = [-r * 0.5, 0.0, r * 0.5].iter().find_map(|dx| vertical_block(&barriers, target, a.target_x + dx, 6.0, 0.3));
                    if let Some((_, be)) = block {
                        if let Ok((_, mut b)) = barriers.get_mut(be) {
                            b.flash = 0.4;
                        }
                        if let Some(e) = a.effect {
                            assets.play(&mut commands, e, "tentacle__block");
                        }
                        sfx.play("SFX_ATTACKS_OCTOPUS_BLOCKED");
                    } else {
                        let dmg = tuning.sp.tentacle_damage.damage(a.quality);
                        let hits = damage_chicks(&mut chicks, &mut sfx, Vec2::new(a.target_x, 0.6), r + 0.3, r * 0.6, dmg, 2.5, false);
                        if hits > 0 {
                            sfx.play("SFX_ATTACKS_OCTOPUS_HIT");
                        }
                    }
                }
                if a.effect.is_some() && a.t >= hit_at + tuning.sp.tentacle_hit_time + 0.6 {
                    if let Some(e) = a.effect.take() {
                        commands.entity(e).despawn();
                    }
                    a.hits += 1;
                    a.t = 0.0;
                    a.wait = rng.range(tuning.sp.octopus_delay);
                    if a.hits >= tuning.sp.octopus_hits {
                        if let Some(v) = a.visual {
                            assets.play(&mut commands, v, "octopus__outro");
                        }
                        a.state = AttackState::Impact;
                    }
                }
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z - 6.0).with_scale(Vec3::splat(assets.scale("octopus", OCTOPUS_LOOK))));
            }
            (AttackKind::Octopus, AttackState::Impact | AttackState::Blocked) => {
                // Sinks back into the sea.
                a.pos.y -= dt * 2.0;
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z - 6.0).with_scale(Vec3::splat(assets.scale("octopus", OCTOPUS_LOOK))));
                if a.t >= 2.0 {
                    sfx.play("SFX_ATTACKS_OCTOPUS_DISAPPEAR");
                    a.state = AttackState::Finished;
                }
            }
            // ---------------------------------------------------------------- ghost (haunted wood special)
            (AttackKind::Ghost, AttackState::Prepare) => {
                // Comes in from the background and haunts the target side for `ghostAttackDuration`.
                if a.visual.is_none() {
                    a.pos = Vec2::new(a.target_x, tuning.sp.ghost_start_y);
                    a.life = rng.range(tuning.sp.ghost_duration);
                    a.wait = tuning.sp.ghost_start_delay;
                    a.visual = Some(assets.spawn_posed(&mut commands, "ghost", "ghost__loop", a.from, GHOST_LOOK, &mut meshes, &mut materials, &mut images));
                    sfx.play("SFX_ATTACKS_GHOST_APPEAR");
                }
                let (lo, hi) = tuning.side_range(target);
                let hover = Vec2::new((lo + hi) / 2.0 + (a.t * 0.7).sin() * (hi - lo) * 0.35, 6.5 + (a.t * 1.3).sin() * 0.6);
                let step = (hover - a.pos).clamp_length_max(tuning.sp.ghost_speed.1 * dt);
                a.pos += step;
                a.life -= dt;
                if a.t >= a.wait && a.life > 0.0 {
                    a.victim = pick_victim(&chicks, target, a.pos.x, None);
                    if a.victim.is_some() {
                        a.state = AttackState::Travel;
                        a.t = 0.0;
                    }
                }
                if a.life <= 0.0 {
                    a.state = AttackState::Impact;
                    a.t = 0.0;
                }
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z + 0.4).with_scale(Vec3::splat(assets.scale("ghost", GHOST_LOOK) * (a.t * 2.0).min(1.0).max(0.05))));
            }
            (AttackKind::Ghost, AttackState::Travel) => {
                // Dives at a chick; touching a line disintegrates it.
                a.life -= dt;
                let goal = a.victim.and_then(|v| chicks.get(v).ok()).filter(|(_, c)| c.alive()).map(|(_, c)| c.pos);
                let Some(goal) = goal else {
                    a.state = AttackState::Prepare;
                    a.t = 0.0;
                    continue;
                };
                a.target_x = goal.x;
                let speed = rng.range(tuning.sp.ghost_speed);
                let next = a.pos + (goal - a.pos).clamp_length_max(speed * dt);
                if let Some((_, _, be)) = sweep_hit(&barriers, target, a.pos, next, 0.6) {
                    if let Ok((_, mut b)) = barriers.get_mut(be) {
                        b.flash = 0.4;
                    }
                    sfx.play("SFX_ATTACKS_GHOST_DEFEND");
                    a.state = AttackState::Blocked;
                    a.t = 0.0;
                } else {
                    a.pos = next;
                }
                if a.state == AttackState::Travel && a.pos.distance(goal) < 0.8 {
                    // Drains the chick and gives the energy to the other side.
                    let dmg = tuning.sp.ghost_damage.damage(a.quality);
                    let mut drained = 0.0;
                    if let Some(v) = a.victim {
                        if let Ok((_, mut c)) = chicks.get_mut(v) {
                            let before = c.health;
                            c.damage(dmg, Vec2::Y * 2.0);
                            drained = before - c.health;
                            sfx.play(if c.dying.is_some() { "SFX_CHICKS_DIE" } else { "SFX_ATTACKS_GHOST_DMG" });
                        }
                    }
                    let weakest = chicks.iter().filter(|(_, c)| c.team == a.from && c.alive()).min_by(|x, y| (x.1.health / x.1.max_health).total_cmp(&(y.1.health / y.1.max_health))).map(|(e, _)| e);
                    if let Some(w) = weakest {
                        if let Ok((_, mut c)) = chicks.get_mut(w) {
                            c.health = (c.health + drained).min(c.max_health);
                            sfx.play("SFX_ATTACKS_GHOST_CHICKTRANSFER");
                        }
                    }
                    a.hits += 1;
                    a.wait = tuning.sp.ghost_shock + tuning.sp.ghost_start_delay + tuning.sp.ghost_delay_increase * a.hits as f32;
                    a.state = AttackState::Prepare;
                    a.t = 0.0;
                }
                if a.state == AttackState::Travel && a.life <= 0.0 {
                    a.state = AttackState::Impact;
                    a.t = 0.0;
                }
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z + 0.4).with_scale(Vec3::splat(assets.scale("ghost", GHOST_LOOK))));
            }
            (AttackKind::Ghost, AttackState::Blocked | AttackState::Impact) => {
                // Disintegrates (on a line) or drifts off when its time is up.
                a.pos.y += dt * 3.0;
                let fade = (1.0 - a.t).max(0.01);
                set_transform(&mut commands, a.visual, Transform::from_xyz(a.pos.x, a.pos.y, z + 0.4).with_scale(Vec3::splat(assets.scale("ghost", GHOST_LOOK) * fade)));
                if a.t >= 1.0 {
                    a.state = AttackState::Finished;
                }
            }
            // ---------------------------------------------------------------- common
            (_, AttackState::Impact) => {
                if let Some(fx) = a.effect {
                    // A bomb keeps its blast radius in `size`.
                    let radius = a.size;
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
    for (team, lo, hi) in strikes {
        for (_, mut a) in &mut attacks {
            if a.target() != team || a.pos.x < lo || a.pos.x > hi {
                continue;
            }
            match (a.kind, a.state) {
                // `bombLightningHitQuality`: a struck bomb goes off at full power.
                (AttackKind::Bomb, AttackState::Travel | AttackState::Roll) => a.detonate = Some(tuning.bomb_lightning_quality),
                // Charred.
                (AttackKind::Plant, AttackState::Travel) => {
                    a.state = AttackState::Blocked;
                    a.t = 0.0;
                    sfx.play("SFX_ATTACKS_PLANT_EXPL");
                }
                _ => {}
            }
        }
    }
}

/// The target chick nearest `x` (keeping `keep` while it's alive).
fn pick_victim(chicks: &Query<(Entity, &mut Chick)>, team: Team, x: f32, keep: Option<Entity>) -> Option<Entity> {
    if let Some(k) = keep.filter(|k| chicks.get(*k).is_ok_and(|(_, c)| c.alive() && !c.abducted)) {
        return Some(k);
    }
    chicks
        .iter()
        .filter(|(_, c)| c.team == team && c.alive())
        .min_by(|a, b| (a.1.pos.x - x).abs().total_cmp(&(b.1.pos.x - x).abs()))
        .map(|(e, _)| e)
}
