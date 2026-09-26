//! What upgraded attacks leave behind: acid from the Frog Cracker bomb (eats at chicks
//! standing in it) and glue from the Chick Fixer weight (sticks chicks in place).
//! Both fly as blobs first; a line catches them.

use bevy::prelude::*;

use super::{
    barrier::{sweep_hit, Barrier},
    chick::Chick,
    flow::{GameAssets, Match, Phase},
    tuning::Tuning,
    Team,
};
use crate::{gx_material::GxMaterial, Screen};

/// Blob sizes (model widths).
const ACID_BLOB: f32 = 1.0;
const GLUE_BLOB: f32 = 1.4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HazardKind {
    Acid,
    Glue,
}

#[derive(Component)]
pub struct Hazard {
    kind: HazardKind,
    /// The side it lands on.
    team: Team,
    pos: Vec2,
    vel: Vec2,
    /// Landed (acid puddle / glue on the floor or a chick): seconds since.
    landed: Option<f32>,
    /// Puddle width and time to the next acid bite.
    width: f32,
    tick: f32,
    visual: Entity,
    /// The chick the glue sticks to.
    stuck: Option<Entity>,
    /// Acid that splashed on a line runs down it to the floor.
    dripping: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_hazard(
    commands: &mut Commands,
    assets: &GameAssets,
    tuning: &Tuning,
    kind: HazardKind,
    team: Team,
    pos: Vec2,
    vel: Vec2,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<GxMaterial>,
    images: &mut Assets<Image>,
) {
    let visual = match kind {
        HazardKind::Acid => assets.spawn_bones(commands, "acid", team, ACID_BLOB, Some(&["fly"]), meshes, materials, images),
        HazardKind::Glue => assets.spawn_posed(commands, "glue", "glue__fall", team, GLUE_BLOB, meshes, materials, images),
    };
    let width = match kind {
        HazardKind::Acid => tuning.up.acid_width,
        HazardKind::Glue => tuning.up.glue_width,
    };
    commands.spawn((Hazard { kind, team, pos, vel, landed: None, width, tick: 0.0, visual, stuck: None, dripping: false }, DespawnOnExit(Screen::Level)));
}

#[allow(clippy::too_many_arguments)]
pub fn update_hazards(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<Tuning>,
    assets: Res<GameAssets>,
    m: Res<Match>,
    mut hazards: Query<(Entity, &mut Hazard)>,
    mut chicks: Query<(Entity, &mut Chick)>,
    barriers: Query<(Entity, &mut Barrier)>,
    mut sfx: ResMut<crate::sfx::Sfx>,
) {
    let dt = time.delta_secs();
    let z = tuning.plane_z + 0.75;
    let up = &tuning.up;
    // A new round clears the field.
    let clear = matches!(m.phase, Phase::RoundOver(_) | Phase::Spin(_) | Phase::GameOver(_));
    for (e, mut h) in &mut hazards {
        let life = match h.kind {
            HazardKind::Acid => up.acid_active + up.acid_fade,
            HazardKind::Glue => up.glue_active,
        };
        if clear || h.landed.is_some_and(|t| t >= life) {
            commands.entity(h.visual).despawn();
            commands.entity(e).despawn();
            continue;
        }
        let Some(t) = h.landed else {
            // Flying: gravity; a line catches it, the floor or a chick stops it.
            let prev = h.pos;
            h.vel.y += tuning.gravity * 0.5 * dt;
            let next = prev + h.vel * dt;
            if !h.dripping && sweep_hit(&barriers, h.team, prev, next, 0.2).is_some() {
                debug!("{:?} caught by a line at {next}", h.kind);
                if h.kind == HazardKind::Acid {
                    sfx.play("SFX_ATTACKS_BOMB_ACID_SPLASH");
                    h.dripping = true;
                    h.vel = Vec2::new(0.0, -1.0);
                } else {
                    sfx.play("SFX_ATTACKS_OCTOPUS_BLOCKED");
                    commands.entity(h.visual).despawn();
                    commands.entity(e).despawn();
                    continue;
                }
            }
            let next = prev + h.vel * dt;
            h.pos = next;
            if h.kind == HazardKind::Glue {
                let hit = chicks.iter().find(|(_, c)| c.team == h.team && c.alive() && c.pos.distance(h.pos) < c.radius + 0.3).map(|(ce, _)| ce);
                if let Some(ce) = hit {
                    h.stuck = Some(ce);
                    h.landed = Some(0.0);
                    if let Ok((_, mut c)) = chicks.get_mut(ce) {
                        c.glued = up.glue_active;
                    }
                    assets.play(&mut commands, h.visual, "glue__splash");
                    sfx.play("SFX_ATTACKS_WEIGHT_GLUE");
                }
            }
            if h.landed.is_none() && h.pos.y <= 0.1 {
                debug!("{:?} landed at {}", h.kind, h.pos);
                h.pos.y = 0.1;
                h.landed = Some(0.0);
                h.vel = Vec2::ZERO;
                match h.kind {
                    HazardKind::Acid => sfx.play("SFX_ATTACKS_BOMB_ACID_SPLASH"),
                    HazardKind::Glue => assets.play(&mut commands, h.visual, "glue__splash"),
                }
            }
            let s = assets.scale(if h.kind == HazardKind::Acid { "acid" } else { "glue" }, if h.kind == HazardKind::Acid { ACID_BLOB } else { GLUE_BLOB });
            commands.entity(h.visual).insert(Transform::from_xyz(h.pos.x, h.pos.y, z).with_scale(Vec3::splat(s)));
            continue;
        };
        let t = t + dt;
        h.landed = Some(t);
        match h.kind {
            HazardKind::Acid => {
                // Eats at chicks standing in it every `acidDmgCooldown`, shrinking each time.
                h.tick -= dt;
                let fading = t > up.acid_active;
                if h.tick <= 0.0 && !fading {
                    h.tick = up.acid_cooldown;
                    let (x, half) = (h.pos.x, h.width / 2.0);
                    let mut bit = false;
                    for (_, mut c) in chicks.iter_mut() {
                        if c.team == h.team && c.alive() && (c.pos.x - x).abs() < half && c.pos.y < c.radius + 0.6 {
                            c.damage(up.acid_dmg, Vec2::Y * 2.0);
                            bit = true;
                        }
                    }
                    if bit {
                        sfx.play("SFX_ATTACKS_BOMB_ACID_DMG");
                        h.width = (h.width - up.acid_shrink).max(0.5);
                    }
                }
                let fade = if fading { (1.0 - (t - up.acid_active) / up.acid_fade.max(0.1)).max(0.01) } else { 1.0 };
                let s = assets.scale("acid", h.width) * fade;
                let pulse = 1.0 + 0.08 * (t * 5.0).sin();
                commands.entity(h.visual).insert(Transform::from_xyz(h.pos.x, 0.12, z).with_scale(Vec3::new(s * pulse, s * 0.3, s)));
            }
            HazardKind::Glue => {
                // Stuck to a chick it rides along; on the floor it catches chicks walking in.
                let at = match h.stuck.and_then(|ce| chicks.get(ce).ok()).filter(|(_, c)| c.alive()) {
                    Some((_, c)) => c.pos - Vec2::Y * c.radius * 0.6,
                    None => {
                        let (x, half) = (h.pos.x, h.width / 4.0);
                        let left = up.glue_active - t;
                        for (_, mut c) in chicks.iter_mut() {
                            if c.team == h.team && c.alive() && c.glued <= 0.0 && (c.pos.x - x).abs() < half && c.pos.y < c.radius + 0.3 {
                                c.glued = left;
                            }
                        }
                        h.pos
                    }
                };
                let fade = ((up.glue_active - t) / 0.4).clamp(0.01, 1.0);
                let s = assets.scale("glue", GLUE_BLOB) * fade;
                commands.entity(h.visual).insert(Transform::from_xyz(at.x, at.y, z).with_scale(Vec3::new(s, s * 0.6, s)));
            }
        }
    }
}
