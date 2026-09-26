//! The CPU opponent ("KI" in the original): attacks and defends using the `ki.*` tuning.

use anyhow::Result;
use bevy::prelude::*;

use super::{
    attack::{Attack, AttackState},
    barrier::{spawn_barrier, Ink},
    flow::{Match, Phase},
    gesture::Gestures,
    rng::Rng,
    tuning::{Tuning, DIFFICULTY},
    AttackKind, Team,
};
use crate::{data::GameData, gx_material::GxMaterial};

/// Per-team CPU state. The CPU plays black; with `CCB_AUTOPLAY=1` it plays yellow too
/// (handy for testing whole matches headlessly).
#[derive(Default)]
struct Side {
    enabled: bool,
    /// Tracing the open special: (attack, seconds left, quality).
    special: Option<(AttackKind, f32, f32)>,
    /// Barriers drawn against the current attack (ghosts need two).
    defended_count: u32,
    think: f32,
    drawing: Option<(AttackKind, f32, f32)>,
    defended: Option<Entity>,
    defend_at: f32,
}

#[derive(Debug, Clone)]
pub struct KiParams {
    pub attack_start: (f32, f32),
    pub start_reaction: (f32, f32),
    pub draw_skill_time: (f32, f32),
    /// Per-gesture drawing quality modifiers (`kiDrawingSkillAttackModifier`), by attack id.
    pub draw_modifier: Vec<f32>,
    pub shield_height: (f32, f32),
    /// Chance the CPU fails to put a barrier in the right place.
    pub miss_chance: f32,
    /// Base drawing quality range for this difficulty.
    pub quality: (f32, f32),
}

#[derive(Resource)]
pub struct Cpu {
    pub ki: KiParams,
    sides: [Side; 2],
}


impl Cpu {
    pub fn load(data: &GameData) -> Result<Self> {
        let name = ["ki.easy", "ki.medium", "ki.hard"][DIFFICULTY];
        let b = data.cfg.block(name).or_else(|_| data.cfg.block("ki.default"))?;
        let d = data.cfg.block("ki.default")?;
        let range = |label: &str, dflt: (f32, f32)| b.range(label).or_else(|_| d.range(label)).unwrap_or(dflt);
        // "7 0.54 / 1.05 / ..." : count on the first line, then one value per line.
        let modifiers: Vec<f32> = {
            let src = if b.line("GESTURE_DEF_WEIGHT").is_some() { b } else { d };
            ["GESTURE_DEF_WEIGHT", "GESTURE_DEF_BOMB", "GESTURE_DEF_LIGHTNING", "GESTURE_DEF_PLANT", "GESTURE_DEF_UFO", "GESTURE_DEF_GHOST", "GESTURE_DEF_OCTOPUS"]
                .iter()
                .map(|l| src.floats(l).ok().and_then(|v| v.last().copied()).unwrap_or(1.0))
                .collect()
        };
        Ok(Self {
            ki: KiParams {
                attack_start: range("kiAttackStartTimer", (0.5, 0.8)),
                start_reaction: range("kiStartAttackReactionTime", (0.9, 1.5)),
                draw_skill_time: range("kiDrawingSkillTime", (0.08, 0.11)),
                draw_modifier: modifiers,
                shield_height: range("kiDefendByShieldHeight", (3.0, 5.0)),
                // Debug: CCB_CPU_MISS=1 makes every CPU shield miss.
                miss_chance: std::env::var("CCB_CPU_MISS").ok().and_then(|v| v.parse().ok()).unwrap_or([0.45, 0.25, 0.1][DIFFICULTY]),
                quality: [(0.35, 0.7), (0.5, 0.85), (0.7, 1.0)][DIFFICULTY],
            },
            sides: [
                Side { enabled: std::env::var("CCB_AUTOPLAY").is_ok(), ..default() },
                Side { enabled: true, ..default() },
            ],
        })
    }
}

/// Seconds until an attack reaches the defender's barrier height.
fn eta(a: &Attack, tuning: &Tuning) -> f32 {
    match (a.kind, a.state) {
        (AttackKind::Bomb, AttackState::Prepare) => tuning.attack_prepare_time - a.t + 1.9,
        (AttackKind::Bomb, _) => 1.9 - a.t,
        (AttackKind::Weight, AttackState::Prepare) => tuning.attack_prepare_time * 0.6 - a.t + 0.8,
        (AttackKind::Weight, _) => 0.8 - a.t,
        (AttackKind::Lightning, _) => tuning.attack_prepare_time + 0.8 - a.t,
        (AttackKind::Plant, AttackState::Prepare) => tuning.attack_prepare_time - a.t + 0.6,
        (AttackKind::Plant, _) => 0.6 - a.t,
        (AttackKind::Ufo, AttackState::Prepare) => tuning.attack_prepare_time - a.t,
        (AttackKind::Octopus, AttackState::Prepare) => tuning.attack_prepare_time - a.t + 0.3,
        (AttackKind::Octopus, _) => 0.3 - a.t,
        (AttackKind::Ghost, AttackState::Prepare) => tuning.attack_prepare_time - a.t + 1.0,
        (AttackKind::Ghost, _) => 0.8,
        _ => 0.0,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn cpu_turn(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<Tuning>,
    gestures: Res<Gestures>,
    mut m: ResMut<Match>,
    mut cpu: ResMut<Cpu>,
    mut rng: ResMut<Rng>,
    mut ink: ResMut<Ink>,
    attacks: Query<(Entity, &Attack)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
) {
    let dt = time.delta_secs();
    let ki = cpu.ki.clone();
    for team in [Team::Yellow, Team::Black] {
        let side = &mut cpu.sides[team.index()];
        if !side.enabled {
            continue;
        }
        // ---- attacking
        if m.phase == Phase::Attack && m.attacker == team && m.pending.is_none() {
            match side.drawing {
                None => {
                    if side.think <= 0.0 {
                        side.think = rng.range(ki.attack_start) + rng.range(ki.start_reaction);
                    }
                    side.think -= dt;
                    if side.think <= 0.0 {
                        let options: Vec<AttackKind> = m.available.iter().copied().filter(|k| k.implemented()).collect();
                        let kind = rng.pick(&options).unwrap_or(AttackKind::Bomb);
                        let dots = gestures.dots(kind) as f32;
                        // The original's skill time is per dot; add the time to move between dots.
                        let duration = dots * rng.range(ki.draw_skill_time) * 3.0;
                        let modifier = ki.draw_modifier.get(kind as usize).copied().unwrap_or(1.0);
                        let quality = (rng.range(ki.quality) * modifier).clamp(0.05, 1.0);
                        side.drawing = Some((kind, duration, quality));
                    }
                }
                Some((kind, left, quality)) => {
                    let left = left - dt;
                    if left <= 0.0 {
                        info!("cpu ({team:?}) attacks with {kind:?} at {:.0}%", quality * 100.0);
                        m.pending = Some((kind, quality));
                        side.drawing = None;
                        side.think = 0.0;
                    } else {
                        side.drawing = Some((kind, left, quality));
                    }
                }
            }
        } else if m.attacker != team {
            side.drawing = None;
        }

        // ---- racing for an open special
        match (m.special, side.special) {
            (Some((kind, _)), None) if m.pending_special.is_none() => {
                // Start tracing after a reaction delay folded into the drawing time.
                let dots = gestures.dots(kind) as f32;
                let duration = rng.range(ki.start_reaction) + dots * rng.range(ki.draw_skill_time) * 3.5;
                let quality = rng.range(ki.quality).clamp(0.05, 1.0);
                side.special = Some((kind, duration, quality));
            }
            (Some((open, _)), Some((kind, left, quality))) if open == kind => {
                let left = left - dt;
                if left <= 0.0 {
                    if m.pending_special.is_none() {
                        m.pending_special = Some((team, kind, quality));
                    }
                    side.special = None;
                } else {
                    side.special = Some((kind, left, quality));
                }
            }
            (None, Some(_)) => side.special = None,
            _ => {}
        }

        // ---- defending
        let Some((ae, a)) = attacks.iter().find(|(_, a)| a.target() == team) else { continue };
        if side.defended != Some(ae) {
            side.defended_count = 0;
        }
        let needed = if a.kind == AttackKind::Ghost { 2 } else { 1 };
        if side.defended == Some(ae) && (side.defended_count >= needed || a.kind != AttackKind::Ghost || a.eaten < side.defended_count) {
            continue;
        }
        if side.defend_at <= 0.0 {
            // React a little before the attack arrives.
            side.defend_at = rng.range((0.35, 1.1));
        }
        if eta(a, &tuning) > side.defend_at {
            continue;
        }
        side.defended = Some(ae);
        side.defended_count += 1;
        side.defend_at = 0.0;
        let mut x = a.predicted_x();
        if rng.chance(ki.miss_chance) {
            x += rng.range((1.6, 3.0)) * if rng.chance(0.5) { -1.0 } else { 1.0 };
        }
        let h = rng.range(ki.shield_height).min(tuning.barrier_y_clip - 0.3);
        let half = match a.kind {
            AttackKind::Weight => 2.3,
            AttackKind::Lightning => 1.2,
            _ => 1.6,
        };
        // Plants come from below: block them with a low lid.
        let h = if a.kind == AttackKind::Plant { rng.range((1.6, 2.4)) } else { h };
        // Tentacles sweep along the ground: stand a wall in their way.
        if a.kind == AttackKind::Octopus {
            let wx = (x + team.side() * 1.2).clamp(tuning.side_range(team).0, tuning.side_range(team).1);
            let pts: Vec<Vec2> = (0..=5).map(|i| Vec2::new(wx, 0.2 + i as f32 * 0.5)).collect();
            if ink.0[team.index()] >= 2.5 {
                ink.0[team.index()] -= 2.5;
                spawn_barrier(&mut commands, team, pts, false, &tuning, &mut meshes, &mut materials);
            }
            continue;
        }
        // Ghosts: a line across their path, a bit above the target.
        let h = if a.kind == AttackKind::Ghost { 2.5 + side.defended_count as f32 * 1.2 } else { h };
        // Tilt bomb shields so bombs glance back towards the thrower.
        let tilt = if a.kind == AttackKind::Bomb { 0.5 * -a.from.side() } else { 0.0 };
        // Keep the whole shield on our side.
        let (lo, hi) = tuning.side_range(team);
        let x = x.clamp(lo - 0.8 + half, hi + 0.8 - half);
        let p0 = Vec2::new(x - half, h - tilt);
        let p1 = Vec2::new(x + half, h + tilt);
        let len = p0.distance(p1);
        if ink.0[team.index()] < len * 0.5 {
            continue;
        }
        ink.0[team.index()] = (ink.0[team.index()] - len).max(0.0);
        let pts: Vec<Vec2> = (0..=6).map(|i| p0.lerp(p1, i as f32 / 6.0)).collect();
        spawn_barrier(&mut commands, team, pts, false, &tuning, &mut meshes, &mut materials);
    }
}
