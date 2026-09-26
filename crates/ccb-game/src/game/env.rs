//! Environment events ("environment assist"): every `envAssistPause` seconds something shows
//! up in the arena and makes its special attack available to both sides for a while:
//! a dark cloud drifting over one side (lightning, every level), UFOs circling over the city,
//! the octopus surfacing by the ship, ghosts haunting the graveyard. Whoever traces the
//! special first launches it; the event's object becomes the attack.

use bevy::prelude::*;

use super::{
    attack::{Attack, AttackState},
    chick::Chick,
    flow::{GameAssets, Match, Phase},
    rng::Rng,
    tuning::Tuning,
    AttackKind, Team,
};
use crate::{gx_material::GxMaterial, Screen};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EnvPhase {
    Appear,
    Active,
    Leave,
}

#[derive(Debug)]
pub struct EnvEvent {
    pub kind: AttackKind,
    pub phase: EnvPhase,
    pub t: f32,
    /// World position of the event's object.
    pub pos: Vec3,
    /// Cloud drift speed (signed).
    pub vel: f32,
    pub appear: f32,
    pub visual: Option<Entity>,
    /// A raining cloud (`cloudRainChance`) and its rain streaks.
    pub rain: Option<Entity>,
}

#[derive(Component)]
pub struct RainDrop;

#[derive(Resource, Default)]
pub struct Env {
    pub event: Option<EnvEvent>,
    /// Next entry of `cloudAppearList`.
    cloud_index: usize,
}

impl Env {
    /// The team a lightning strike would hit now (the side the cloud is over).
    pub fn cloud_side(&self) -> Option<Team> {
        self.event.as_ref().filter(|e| e.kind == AttackKind::Lightning).map(|e| if e.pos.x < 0.0 { Team::Yellow } else { Team::Black })
    }
}

/// Model, width and clip for an event's object.
fn event_model(kind: AttackKind) -> (&'static str, f32, Option<&'static str>) {
    match kind {
        AttackKind::Lightning => ("cloud", 5.0, None),
        AttackKind::Ufo => ("bgUfo", 3.0, None),
        AttackKind::Octopus => ("octopus", 5.5, Some("octopus__intro")),
        AttackKind::Ghost => ("ghost", 0.85, Some("ghost__loop")),
        _ => ("cloud", 5.0, None),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_env(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<Tuning>,
    assets: Res<GameAssets>,
    mut env: ResMut<Env>,
    mut m: ResMut<Match>,
    mut rng: ResMut<Rng>,
    chicks: Query<&Chick>,
    mut attacks: Query<&mut Attack>,
    mut drops: Query<&mut Transform, (With<RainDrop>, Without<Chick>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut sfx: ResMut<crate::sfx::Sfx>,
) {
    let dt = time.delta_secs();
    let sp = &tuning.sp;
    for mut t in &mut drops {
        t.translation.y -= dt * 12.0;
        if t.translation.y < -tuning.cloud_y {
            t.translation.y += tuning.cloud_y - 0.5;
        }
    }
    let z = tuning.plane_z;
    let running = !matches!(m.phase, Phase::Spin(_) | Phase::GameOver(_) | Phase::RoundOver(_));
    if !running {
        // Rounds end: the sky clears.
        if let Some(ev) = env.event.take() {
            for v in [ev.visual, ev.rain].into_iter().flatten() {
                commands.entity(v).despawn();
            }
        }
        m.special = None;
        m.special_target = None;
        return;
    }
    // A new event after `envAssistPause`.
    if env.event.is_none() && !m.specials.is_empty() {
        m.special_timer -= dt;
        if m.special_timer <= 0.0 {
            m.special_timer = rng.range(tuning.special_every);
            let specials = m.specials.clone();
            // Debug: CCB_ENV_KIND=lightning|ufo|octopus|ghost picks the event.
            let forced = std::env::var("CCB_ENV_KIND").ok().and_then(|n| specials.iter().copied().find(|k| k.gesture_group() == n));
            if let Some(kind) = forced.or_else(|| rng.pick(&specials)) {
                let (pos, vel, appear) = match kind {
                    AttackKind::Lightning => {
                        let side = sp.cloud_sides.get(env.cloud_index % sp.cloud_sides.len().max(1)).copied().unwrap_or(1.0);
                        env.cloud_index += 1;
                        let x = side * tuning.side_width * rng.range(sp.cloud_x_per);
                        let speed = rng.range(sp.cloud_speed) * if rng.chance(0.5) { -1.0 } else { 1.0 };
                        (Vec3::new(x, tuning.cloud_y, z), speed, sp.cloud_appear)
                    }
                    AttackKind::Ufo => (sp.ufo_circle_center, 0.0, rng.range(sp.ufo_appear)),
                    AttackKind::Octopus => {
                        let x = rng.range((-tuning.side_width * 0.6, tuning.side_width * 0.6));
                        (Vec3::new(x, sp.octopus_y_start, z - 6.0), 0.0, sp.octopus_appear)
                    }
                    _ => {
                        let (lo, hi) = sp.ghost_fly_range;
                        (Vec3::new(rng.range((lo.x, hi.x)), rng.range((lo.y, hi.y)), rng.range((lo.z, hi.z))), 0.0, 2.0)
                    }
                };
                let (model, width, clip) = event_model(kind);
                let visual = match clip {
                    Some(c) => assets.spawn_posed(&mut commands, model, c, Team::Yellow, width, &mut meshes, &mut materials, &mut images),
                    None => assets.spawn(&mut commands, model, Team::Yellow, width, &mut meshes, &mut materials, &mut images),
                };
                commands.entity(visual).insert(DespawnOnExit(Screen::Level));
                sfx.play(match kind {
                    AttackKind::Lightning => "SFX_ATTACKS_THUNDER_APPROACHING",
                    AttackKind::Ufo => "SFX_ATTACKS_UFO_APPEAR",
                    AttackKind::Octopus => "SFX_ATTACKS_OCTOPUS_APPEAR",
                    _ => "SFX_ATTACKS_GHOST_APPEAR",
                });
                info!("environment: {kind:?}");
                let rain = (kind == AttackKind::Lightning && rng.chance(sp.cloud_rain_chance)).then(|| {
                    sfx.play("SFX_ENV_RAIN");
                    let mut mat = super::barrier::solid_material(Vec4::new(0.55, 0.75, 1.0, 0.5));
                    mat.params.konst[3].w = 0.5;
                    let material = materials.add(mat);
                    let streak = meshes.add(Rectangle::new(0.05, 0.6));
                    commands
                        .spawn((Transform::from_translation(pos), Visibility::default(), DespawnOnExit(Screen::Level)))
                        .with_children(|p| {
                            for i in 0..14 {
                                p.spawn((Mesh3d(streak.clone()), MeshMaterial3d(material.clone()), Transform::from_xyz((i as f32 / 13.0 - 0.5) * 3.6, -(i * 7 % 13) as f32 * 0.5, 0.8), RainDrop));
                            }
                        })
                        .id()
                });
                env.event = Some(EnvEvent { kind, phase: EnvPhase::Appear, t: 0.0, pos, vel, appear, visual: Some(visual), rain });
            }
        }
    }
    // Rain falls under a raining cloud and puts out bombs there (`bombCloudRainDistance`).
    if let Some(ev) = env.event.as_ref() {
        if let Some(r) = ev.rain {
            commands.entity(r).insert(Transform::from_translation(ev.pos));
            for mut a in &mut attacks {
                if a.kind == AttackKind::Bomb && matches!(a.state, AttackState::Roll) && (a.pos.x - ev.pos.x).abs() < 1.8 + tuning.bomb_rain_distance {
                    a.state = AttackState::Finished;
                    sfx.play("SFX_ATTACKS_BOMB_DEFUSED");
                }
            }
        }
    }
    let Some(ev) = env.event.as_mut() else {
        m.special = None;
        m.special_target = None;
        return;
    };
    ev.t += dt;
    // Move the event's object.
    match ev.kind {
        AttackKind::Lightning => {
            // The cloud drifts and turns back near the arena's ends.
            ev.pos.x += ev.vel * dt;
            let limit = tuning.side_width * sp.cloud_x_per.1;
            if ev.pos.x.abs() > limit {
                ev.pos.x = ev.pos.x.clamp(-limit, limit);
                ev.vel = -ev.vel;
            }
        }
        AttackKind::Ufo => {
            // Circles behind the city (`ufoCircleRadius`, `ufoTimeNeededForFullCircle`).
            let a = ev.t / sp.ufo_circle_time.max(0.1) * std::f32::consts::TAU;
            let c = sp.ufo_circle_center;
            ev.pos = Vec3::new(c.x + sp.ufo_circle_radius * a.cos(), c.y + 0.6 * (a * 2.0).sin(), c.z + sp.ufo_circle_radius * 0.5 * a.sin());
        }
        AttackKind::Octopus => {
            let k = (ev.t / ev.appear.max(0.1)).min(1.0);
            ev.pos.y = sp.octopus_y_start + (sp.octopus_y - sp.octopus_y_start) * k;
        }
        _ => {
            // Ghosts drift about the background.
            let (lo, hi) = sp.ghost_fly_range;
            let w = ev.t * 0.4;
            ev.pos = Vec3::new(
                (lo.x + hi.x) / 2.0 + (hi.x - lo.x) / 2.0 * w.sin(),
                (lo.y + hi.y) / 2.0 + (hi.y - lo.y) / 2.0 * (w * 1.7).cos(),
                (lo.z + hi.z) / 2.0,
            );
        }
    }
    let fade = match ev.phase {
        EnvPhase::Appear => (ev.t / ev.appear.max(0.1)).min(1.0),
        EnvPhase::Active => 1.0,
        EnvPhase::Leave => (1.0 - ev.t / sp.ufo_disappear.max(0.1)).max(0.0),
    };
    let (model, width, _) = event_model(ev.kind);
    if let Some(v) = ev.visual {
        let spin = if ev.kind == AttackKind::Ufo { Quat::from_rotation_y(ev.t * 3.0) } else { Quat::IDENTITY };
        commands.entity(v).insert(Transform::from_translation(ev.pos).with_rotation(spin).with_scale(Vec3::splat(assets.scale(model, width) * fade.max(0.01))));
    }
    match ev.phase {
        EnvPhase::Appear if ev.t >= ev.appear => {
            ev.phase = EnvPhase::Active;
            ev.t = 0.0;
            sfx.play("SFX_ATTACK_IFC_SELECTOR_EVENT_ACTIVATED");
        }
        EnvPhase::Active => {
            let left = tuning.special_window - ev.t;
            if left <= 0.0 {
                ev.phase = EnvPhase::Leave;
                ev.t = 0.0;
                m.special = None;
            } else {
                m.special = Some((ev.kind, left));
            }
        }
        EnvPhase::Leave if ev.t >= sp.ufo_disappear => {
            for v in [ev.visual.take(), ev.rain.take()].into_iter().flatten() {
                commands.entity(v).despawn();
            }
            env.event = None;
            m.special = None;
            m.special_target = None;
            return;
        }
        _ => {}
    }
    m.special_target = env.cloud_side();
    if m.special.is_none() && m.selected.is_some_and(|k| !k.is_basic()) {
        m.selected = None;
    }
    // Launch: the event's object becomes the attack.
    if let Some((from, kind, quality)) = m.pending_special.take() {
        let Some(ev) = env.event.take() else { return };
        if ev.kind != kind {
            env.event = Some(ev);
            return;
        }
        // Lightning hits whichever side the cloud is over, possibly the launcher's own.
        let target = match kind {
            AttackKind::Lightning if ev.pos.x < 0.0 => Team::Yellow,
            AttackKind::Lightning => Team::Black,
            _ => from.other(),
        };
        let xs: Vec<f32> = chicks.iter().filter(|c| c.team == target && c.alive()).map(|c| c.pos.x).collect();
        let (lo, hi) = tuning.side_range(target);
        let aim = rng.pick(&xs).unwrap_or((lo + hi) / 2.0);
        let tx = if kind == AttackKind::Lightning { ev.pos.x } else { (aim + (1.0 - quality) * 2.0 * (rng.f32() * 2.0 - 1.0)).clamp(lo, hi) };
        info!("{from:?} wins the {kind:?} race against {target:?}");
        for v in [ev.visual, ev.rain].into_iter().flatten() {
            commands.entity(v).despawn();
        }
        commands.spawn((Attack::new(kind, from, quality, tx).against(target).from_origin(ev.pos), DespawnOnExit(Screen::Level)));
        sfx.play("SFX_ATTACK_IFC_LAUNCH");
        m.special = None;
        m.special_target = None;
        if m.selected == Some(kind) {
            m.selected = None;
        }
    }
}
