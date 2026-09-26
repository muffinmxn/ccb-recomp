//! Chicks: the 3D `chick` model, hopping around their half of the field.

use std::collections::HashMap;

use bevy::prelude::*;
use wii_formats::mdl0::Model;

use super::{rng::Rng, tuning::Tuning, Team};
use crate::{g3d, gx_material::GxMaterial};

/// Gravity for chick hops, in world units/s² (tuned to match the original's feel).
const GRAVITY: f32 = 22.0;

#[derive(Component, Debug)]
pub struct Chick {
    pub team: Team,
    pub health: f32,
    pub pos: Vec2,
    pub vel: Vec2,
    pub on_ground: bool,
    hop_timer: f32,
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
        self.flash = 0.4;
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
        let radius = model
            .meshes
            .iter()
            .flat_map(|m| m.positions.iter())
            .map(|p| (p[0] * p[0] + p[1] * p[1]).sqrt())
            .fold(0.0f32, f32::max)
            .max(0.1)
            * 0.55;
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
    let (size, health) = if big { (1.35, 2.0) } else { (0.8, 1.0) };
    // Debug: CCB_CHICK_HEALTH=0.1 for quick matches.
    let health = health * std::env::var("CCB_CHICK_HEALTH").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    let radius = tuning.chick_radius * size;
    let scale = radius / assets.radius;
    let y = radius + 6.0;
    commands.entity(visual).insert((
        Chick {
            team,
            health: tuning.chick_health * health,
            max_health: tuning.chick_health * health,
            radius,
            abducted: false,
            pos: Vec2::new(x, y),
            vel: Vec2::ZERO,
            on_ground: false,
            hop_timer: 0.3,
            squashed: 0.0,
            flash: 0.0,
            dying: None,
            scale,
        },
        Transform::from_xyz(x, y, tuning.plane_z).with_scale(Vec3::splat(scale)),
    ));
    visual
}

pub fn move_chicks(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<Tuning>,
    mut rng: ResMut<Rng>,
    mut chicks: Query<(Entity, &mut Chick, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (e, mut c, mut t) in &mut chicks {
        let r = c.radius;
        if let Some(d) = c.dying {
            let d = d - dt / tuning.chick_dying_time.max(0.1);
            c.dying = Some(d);
            // Float up and shrink away.
            c.pos.y += dt * if c.abducted { 4.0 } else { 2.5 };
            t.translation = Vec3::new(c.pos.x, c.pos.y, tuning.plane_z);
            let shrink = if c.abducted { (d * 2.5).min(1.0) } else { d };
            t.scale = Vec3::splat(c.scale * shrink.max(0.01));
            if d <= 0.0 {
                commands.entity(e).despawn();
            }
            continue;
        }
        c.flash = (c.flash - dt).max(0.0);
        c.squashed = (c.squashed - dt).max(0.0);
        // Hop around at random while on the ground (not while squashed).
        if c.on_ground && c.squashed <= 0.0 {
            c.hop_timer -= dt;
            if c.hop_timer <= 0.0 {
                let dir = if rng.chance(0.5) { -1.0 } else { 1.0 };
                // Big chicks hop lower and slower.
                let k = if c.radius > tuning.chick_radius { 0.75 } else { 1.0 };
                c.vel = Vec2::new(dir * rng.range((1.2, 2.6)) * k, rng.range((4.0, 6.5)) * k);
                c.on_ground = false;
                c.hop_timer = rng.range((0.3, 1.3));
            }
        }
        if !c.on_ground {
            c.vel.y -= GRAVITY * dt;
        }
        let v = c.vel;
        c.pos += v * dt;
        // Keep bigger chicks the same distance from the walls.
        let (lo, hi) = tuning.side_range(c.team);
        let extra = r - tuning.chick_radius;
        let (lo, hi) = (lo + extra, hi - extra);
        if c.pos.x < lo || c.pos.x > hi {
            c.pos.x = c.pos.x.clamp(lo, hi);
            c.vel.x = -c.vel.x * 0.75;
        }
        if c.pos.y <= r {
            c.pos.y = r;
            if c.vel.y < 0.0 {
                c.vel = Vec2::ZERO;
                c.on_ground = true;
            }
        }
        // Squash when flattened; lean into the hop direction.
        let base = c.scale;
        let squash = if c.squashed > 0.0 { 0.5 } else { 1.0 };
        t.scale = Vec3::new(base, base * squash, base);
        t.translation = Vec3::new(c.pos.x, c.pos.y - r * (1.0 - squash), tuning.plane_z);
        let lean = (-c.vel.x * 0.08).clamp(-0.4, 0.4);
        let wobble = if c.flash > 0.0 { (c.flash * 40.0).sin() * 0.3 } else { 0.0 };
        t.rotation = Quat::from_rotation_z(lean + wobble);
    }
}
