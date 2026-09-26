//! Chicks: the 3D `chick` model, hopping around their half of the field.
//!
//! Movement follows the original's particle physics (`ingame.model.particleSystem`,
//! `*.physics.jumpCond`): a fixed 180 Hz step with gravity and a per-step velocity damping;
//! a chick jumps again right after landing (the jump force pushes for `jumpDuration`),
//! bounces off the walls and turns around more likely the longer it ran one way.

use std::collections::HashMap;

use bevy::prelude::*;
use wii_formats::mdl0::Model;

use super::{barrier::Barrier, rng::Rng, tuning::Tuning, Team};
use crate::{g3d, gx_material::GxMaterial};


#[derive(Component, Debug)]
pub struct Chick {
    pub team: Team,
    pub health: f32,
    pub pos: Vec2,
    pub vel: Vec2,
    pub on_ground: bool,
    /// The general: the team's big chick.
    pub general: bool,
    /// Hop direction (-1 / +1) and jumps since the last turn.
    dir: f32,
    same_dir: u32,
    /// Seconds since the current jump started (landing), while jumping.
    jump: Option<f32>,
    /// Extra force pushing the chick in at the start of a round: (force, seconds left).
    start_force: Option<(Vec2, f32)>,
    /// Landing squash (0 = none), for the model's scale.
    squash_anim: f32,
    yaw: f32,
    yaw_target: f32,
    yaw_timer: f32,
    /// Leftover time for the fixed physics step.
    step_acc: f32,
    /// Seconds left squashed flat by a weight.
    pub squashed: f32,
    /// Seconds left of the hit flash.
    pub flash: f32,
    /// Counts down once health hits zero; the chick is removed at zero.
    pub dying: Option<f32>,
    /// Model scale for the chick's size.
    pub scale: f32,
    pub max_health: f32,
    pub radius: f32,
    /// Taken by a UFO: floats up into it instead of fading away.
    pub abducted: bool,
    /// Held in a UFO beam: no physics, the attack moves it.
    pub held: bool,
    /// Took damage this frame (for the hit effect).
    pub hurt: bool,
    /// Something is about to hit it.
    pub scared: bool,
    /// Current face clip and seconds to the next blink.
    pub face: &'static str,
    pub blink: f32,
}

impl Chick {
    pub fn alive(&self) -> bool {
        self.dying.is_none() && self.health > 0.0
    }

    pub fn damage(&mut self, amount: f32, push: Vec2) {
        if !self.alive() {
            return;
        }
        self.health -= amount;
        self.flash = 0.6;
        self.hurt = true;
        self.vel += push;
        self.on_ground = false;
        if self.health <= 0.0 {
            self.health = 0.0;
            self.dying = Some(1.0);
        }
    }
}

/// Everything needed to spawn chick models.
pub struct ChickAssets {
    pub model: Model,
    pub textures: HashMap<String, (Handle<Image>, [u32; 2])>,
    /// Model-space radius of the chick's body sphere.
    pub radius: f32,
}

impl ChickAssets {
    pub fn new(model: Model, textures: HashMap<String, (Handle<Image>, [u32; 2])>) -> Self {
        // The body is the mesh on the `sphere` bone (the outline billboard around it is bigger);
        // positions are already in model space.
        let radius = model
            .meshes
            .iter()
            .filter(|m| model.bones.get(m.bone).is_some_and(|b| b.name == "sphere"))
            .map(|m| m.positions.iter().map(|p| (p[0] * p[0] + p[1] * p[1]).sqrt()).fold(0.0f32, f32::max))
            .fold(0.0f32, f32::max);
        debug!("chick body radius {radius}");
        let radius = if radius > 0.01 { radius } else { 0.45 };
        Self { model, textures, radius }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_chick(
    commands: &mut Commands,
    parent: Entity,
    assets: &ChickAssets,
    team: Team,
    x: f32,
    big: bool,
    tuning: &Tuning,
    rng: &mut Rng,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<GxMaterial>,
    images: &mut Assets<Image>,
) -> Entity {
    // The model carries the yellow team's textures ("...Y"); black chicks use the
    // matching "...B" textures.
    let tex = assets.textures.clone();
    let tweak = move |_: &wii_formats::mdl0::Mesh, mat_name: &str, mat: &mut GxMaterial| {
        let _ = mat_name;
        if team == Team::Black {
            for slot in [&mut mat.tex0, &mut mat.tex1] {
                let Some(h) = slot.clone() else { continue };
                let name = tex.iter().find(|(_, (th, _))| *th == h).map(|(n, _)| n.clone());
                if let Some(n) = name {
                    let swapped = n.strip_suffix('Y').map(|b| format!("{b}B")).or_else(|| n.strip_suffix("YL").map(|b| format!("{b}BL")));
                    if let Some((bh, _)) = swapped.and_then(|s| tex.get(&s)) {
                        *slot = Some(bh.clone());
                    }
                }
            }
        }
        true
    };
    let visual = g3d::spawn_model(commands, parent, &assets.model, &assets.textures, meshes, materials, images, &tweak);
    // Each team has one big chick (double health) and small ones.
    // Each team has one general (`chickGeneralSizeFac` bigger; the tips say it can take more
    // damage) and small chicks.
    let (size, health) = if big { (tuning.general_size_fac, 2.0) } else { (1.0, 1.0) };
    // Debug: CCB_CHICK_HEALTH=0.1 for quick matches.
    let health = health * std::env::var("CCB_CHICK_HEALTH").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    let radius = tuning.chick_size * size;
    let scale = radius / assets.radius;
    // Chicks drop in from `chickStartPosY`, pushed towards the middle of their side.
    let y = tuning.chick_start_y;
    let (fmin, fmax) = tuning.chick_start_force;
    let center = (tuning.side_range(team).0 + tuning.side_range(team).1) / 2.0;
    let f = Vec2::new(rng.range((fmin.x, fmax.x)) * (center - x).signum(), rng.range((fmin.y, fmax.y)));
    let dir = if rng.chance(0.5) { -1.0 } else { 1.0 };
    commands.entity(visual).insert((
        Chick {
            team,
            health: tuning.chick_health * health,
            max_health: tuning.chick_health * health,
            radius,
            abducted: false,
            held: false,
            hurt: false,
            scared: false,
            face: "",
            blink: 1.0,
            pos: Vec2::new(x, y),
            vel: Vec2::ZERO,
            on_ground: false,
            general: big,
            dir,
            same_dir: 0,
            jump: None,
            start_force: Some((f, tuning.chick_start_force_time)),
            squash_anim: 0.0,
            yaw: 0.0,
            yaw_target: 0.0,
            yaw_timer: 0.0,
            step_acc: 0.0,
            squashed: 0.0,
            flash: 0.0,
            dying: None,
            scale,
        },
        Transform::from_xyz(x, y, tuning.plane_z).with_scale(Vec3::splat(scale)),
    ));
    visual
}

/// Converts the cfg's jump values (`chickBaseVelX/Y`) into a take-off speed: a chick hops
/// about its own height and a chick-width sideways every ~0.45 s.
const JUMP_SCALE: f32 = 0.095;

pub fn move_chicks(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<Tuning>,
    mut rng: ResMut<Rng>,
    barriers: Query<&Barrier>,
    mut chicks: Query<(Entity, &mut Chick, &mut Transform)>,
) {
    let dt = time.delta_secs();
    let h = 1.0 / tuning.physics_fps.max(30.0);
    let damp = 1.0 - tuning.vel_damp;
    // Barrier segments, which chicks bump into and can stand on.
    let segments: Vec<(Vec2, Vec2)> = barriers.iter().flat_map(|b| b.points.windows(2).map(|w| (w[0], w[1])).collect::<Vec<_>>()).collect();
    for (e, mut c, mut t) in &mut chicks {
        let r = c.radius;
        if let Some(d) = c.dying {
            // `chickDyingTimer`, then fade out over `chickDyingFadeoutTimer` while floating up.
            let total = tuning.chick_dying_time + tuning.chick_dying_fade;
            let d = d - dt / total.max(0.1);
            c.dying = Some(d);
            c.pos.y += dt * if c.abducted { 4.0 } else { tuning.chick_dying_speed };
            t.translation = Vec3::new(c.pos.x, c.pos.y, tuning.plane_z);
            let fade = (d * total / tuning.chick_dying_fade.max(0.1)).min(1.0);
            // Its soul takes over (see `fx`): the body pops away quickly.
            let shrink = if c.abducted { (d * 2.5).min(1.0) } else { ((d - 0.9) / 0.1).clamp(0.0, 1.0) * fade };
            t.scale = Vec3::splat(c.scale * shrink.max(0.01));
            if d <= 0.0 {
                commands.entity(e).despawn();
            }
            continue;
        }
        c.flash = (c.flash - dt).max(0.0);
        c.squashed = (c.squashed - dt).max(0.0);
        if c.held {
            c.vel = Vec2::ZERO;
            c.jump = None;
            t.translation = Vec3::new(c.pos.x, c.pos.y, tuning.plane_z);
            t.rotation = Quat::from_rotation_z((c.flash * 20.0).sin() * 0.2 + 0.3);
            continue;
        }
        let jc = if c.general { tuning.general_jump } else { tuning.chick_jump };
        // The side's walls: the level border and the separator.
        let (wall_lo, wall_hi) = match c.team {
            Team::Yellow => (-tuning.side_width, -tuning.separator / 2.0),
            Team::Black => (tuning.separator / 2.0, tuning.side_width),
        };
        c.step_acc += dt.min(0.1);
        while c.step_acc >= h {
            c.step_acc -= h;
            // The jump values act as a push at take-off (scaled by JUMP_SCALE); only the
            // sideways speed is damped, so chicks drop freely and skid to a stop.
            if let Some((f, _)) = c.start_force.take() {
                c.vel += f * JUMP_SCALE;
            }
            if let Some(j) = c.jump {
                let j = j + h;
                if j >= jc.delay {
                    c.vel = Vec2::new(jc.vel.x * c.dir, jc.vel.y) * JUMP_SCALE;
                    c.jump = None;
                } else {
                    c.jump = Some(j);
                }
            }
            c.vel.x *= damp;
            c.vel.y += tuning.gravity * h;
            let v = c.vel;
            c.pos += v * h;
            // Walls: bounce back and turn around.
            if c.pos.x < wall_lo + r || c.pos.x > wall_hi - r {
                let inward = if c.pos.x < wall_lo + r { 1.0 } else { -1.0 };
                c.pos.x = c.pos.x.clamp(wall_lo + r, wall_hi - r);
                c.vel.x = inward * c.vel.x.abs() * jc.bounce;
                c.dir = inward;
                c.same_dir = 0;
            }
            // Ground and barriers.
            let mut landed = false;
            if c.pos.y <= r {
                c.pos.y = r;
                if c.vel.y < 0.0 {
                    c.vel.y = 0.0;
                    landed = true;
                }
            }
            for (p, q) in &segments {
                let seg = *q - *p;
                let s = ((c.pos - *p).dot(seg) / seg.length_squared().max(1e-6)).clamp(0.0, 1.0);
                let closest = *p + seg * s;
                let d = c.pos - closest;
                let dist = d.length();
                if dist < r && dist > 1e-4 {
                    let n = d / dist;
                    c.pos = closest + n * r;
                    let vn = c.vel.dot(n);
                    if vn < 0.0 {
                        // `particleCollisionVelReduction`
                        c.vel -= n * vn * 1.35;
                    }
                    if n.y > 0.707 {
                        landed = true;
                    } else if n.x.abs() > 0.707 {
                        c.dir = n.x.signum();
                        c.same_dir = 0;
                    }
                }
            }
            c.on_ground = landed;
            if landed && c.jump.is_none() && c.squashed <= 0.0 && c.start_force.is_none() {
                // Landed: squash, then jump again (maybe the other way).
                c.squash_anim = 1.0;
                c.same_dir += 1;
                if rng.chance(jc.turn_chance(c.same_dir)) {
                    c.dir = -c.dir;
                    c.same_dir = 0;
                }
                c.jump = Some(0.0);
            }
        }
        // Chicks of a team push each other apart.
        let _ = &mut rng;
        // Looks: landing squash springs back, a weight flattens; the head turns around.
        c.squash_anim = (c.squash_anim - dt * 6.0).max(0.0);
        c.yaw_timer -= dt;
        if c.yaw_timer <= 0.0 {
            c.yaw_timer = rng.range(tuning.chick_rot_timer);
            let (lo, hi) = tuning.chick_rot_angles;
            // The cfg range (-20..90 degrees) is for the original's own rotation animation;
            // turning the whole model that far hides the face, so keep a smaller turn.
            c.yaw_target = (rng.range((lo, hi)) * 0.35).to_radians() * c.dir;
        }
        let k = 1.0 - (1.0 - tuning.chick_rot_damp).powf(dt * 60.0);
        c.yaw += (c.yaw_target - c.yaw) * k;
        let base = c.scale;
        let squash = if c.squashed > 0.0 { 0.5 } else { 1.0 - 0.25 * c.squash_anim };
        let widen = if c.squashed > 0.0 { 1.3 } else { 1.0 + 0.12 * c.squash_anim };
        t.scale = Vec3::new(base * widen, base * squash, base * widen);
        t.translation = Vec3::new(c.pos.x, c.pos.y - r * (1.0 - squash), tuning.plane_z);
        let wobble = if c.flash > 0.0 { (c.flash * 40.0).sin() * 0.3 } else { 0.0 };
        t.rotation = Quat::from_rotation_y(c.yaw) * Quat::from_rotation_z(wobble);
    }
    separate_chicks(&mut chicks);
}

/// Keeps chicks of the same team from overlapping.
fn separate_chicks(chicks: &mut Query<(Entity, &mut Chick, &mut Transform)>) {
    let snapshot: Vec<(Entity, Team, Vec2, f32)> = chicks.iter().filter(|(_, c, _)| c.dying.is_none()).map(|(e, c, _)| (e, c.team, c.pos, c.radius)).collect();
    for (e, mut c, _) in chicks.iter_mut() {
        if c.dying.is_some() {
            continue;
        }
        for &(oe, team, pos, r) in &snapshot {
            if oe == e || team != c.team {
                continue;
            }
            let d = c.pos - pos;
            let min = (c.radius + r) * 0.9;
            let dist = d.length();
            if dist < min && dist > 1e-4 {
                let push = d / dist * (min - dist) * 0.5;
                c.pos += push;
                c.vel += push * 10.0;
            }
        }
    }
}
