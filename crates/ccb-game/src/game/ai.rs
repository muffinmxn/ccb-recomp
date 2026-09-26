//! The CPU opponent ("KI" in the original), driven by the `ki.*` blocks of `ki.cfg`.
//!
//! - Attacking: waits `kiAttackStartTimer` + `kiStartAttackReactionTime`, picks an attack
//!   (`kiIntelligentAttackSelection` > 0 aims for where the chicks bunch up), then traces
//!   it dot by dot; each dot takes `kiDrawingSkillTime` x `kiDrawingDifficultyModifier` x the
//!   gesture's `kiDrawingSkillAttackModifier`, and the quality comes from those timings the
//!   same way as the player's.
//! - Defending: every incoming attack gets the line its recipe calls for (bomb cage, weight
//!   lid at `kiDefendWeightBarrierY`, UFO lid, tentacle lid, ghost wall, plant ramp, lightning
//!   rod), after a reaction delay; lines are drawn over `kiBarrierDrawTime` with the points
//!   wobbled by `kiBarrierDistortion*`, cost ink like the player's, and wait
//!   `kiDefendCoolDownTimer` between them. An attack is looked at again after its
//!   `kiDReCheck*AttackTimer`.

use std::collections::HashMap;

use anyhow::Result;
use bevy::prelude::*;

use super::{
    attack::{Attack, AttackState},
    attack_ui::{IfcView, Mode},
    barrier::{spawn_barrier, Barrier, Ink},
    chick::Chick,
    flow::{Match, Phase},
    gesture::{trace_quality, Gestures},
    rng::Rng,
    tuning::{Tuning, DIFFICULTY},
    AttackKind, Team,
};
use crate::{data::GameData, gx_material::GxMaterial};

/// A line being drawn: its entity, the full point list, points shown, (elapsed, duration).
struct CpuLine {
    entity: Entity,
    points: Vec<Vec2>,
    shown: usize,
    t: f32,
    duration: f32,
}

/// Per-team CPU state. The CPU plays black; with `CCB_AUTOPLAY=1` it plays yellow too
/// (handy for testing whole matches headlessly).
#[derive(Default)]
struct Side {
    enabled: bool,
    /// Tracing the open special: (attack, seconds left, quality).
    special: Option<(AttackKind, f32, f32)>,
    think: f32,
    drawing: Option<(AttackKind, f32, f32)>,
    /// Total time of the current drawing / special tracing (for the panel's progress).
    drawing_total: f32,
    special_total: f32,
    /// Traced and about to fire: (attack, quality, seconds left).
    armed: Option<(AttackKind, f32, f32)>,
    /// Incoming attacks already answered: seconds until they're looked at again.
    handled: HashMap<Entity, f32>,
    /// A planned defence waiting out its reaction delay: (attack, seconds left).
    planned: Option<(Entity, f32)>,
    line: Option<CpuLine>,
    cooldown: f32,
    /// Seconds since the attack was traced, the upgrade it hit, and the one it will go for
    /// (upgrade, at seconds).
    armed_t: f32,
    upgrade: u8,
    upgrade_plan: Option<(u8, f32)>,
}

/// Wobble limits for a defence line (`kiBarrierDistortionDefend*Max{X,Y}`).
#[derive(Debug, Clone, Copy)]
struct Wobble(f32, f32);

#[derive(Debug, Clone)]
pub struct KiParams {
    attack_start: (f32, f32),
    start_reaction: (f32, f32),
    draw_skill_time: (f32, f32),
    draw_difficulty: (f32, f32),
    /// Per-gesture drawing time modifiers (`kiDrawingSkillAttackModifier`), by attack id.
    draw_modifier: Vec<f32>,
    intelligent: u32,
    defend_cooldown: (f32, f32),
    fac_cooldown: f32,
    barrier_draw_time: (f32, f32),
    recheck: HashMap<AttackKind, f32>,
    fac_recheck: f32,
    bomb_offsets: Vec<Vec2>,
    weight_barrier_y: f32,
    ufo_offset: Vec2,
    ghost_approach_y: f32,
    ghost_width: f32,
    ghost_distance: f32,
    tentacle_width: f32,
    tentacle_y: f32,
    tentacle_reaction: f32,
    plant_reaction: f32,
    plant_max_dots: usize,
    lightning_approach: f32,
    delay_tentacle: f32,
    delay_bomb: f32,
    delay_weight: (f32, f32),
    delay_lightning: (f32, f32),
    fac_delay: f32,
    wobble: HashMap<&'static str, Wobble>,
    fac_wobble: f32,
    /// `kiAttackUseUpgradePropability`, `kiAttackWithUpgradeRedPropability`, `kiUpgradeHitTime`.
    use_upgrade: f32,
    red_upgrade: f32,
    upgrade_hit_time: (f32, f32),
}

#[derive(Resource)]
pub struct Cpu {
    ki: KiParams,
    sides: [Side; 2],
}

impl Cpu {
    pub fn load(data: &GameData) -> Result<Self> {
        let name = ["ki.easy", "ki.medium", "ki.hard"][DIFFICULTY];
        let b = data.cfg.block(name).or_else(|_| data.cfg.block("ki.default"))?;
        let d = data.cfg.block("ki.default")?;
        let range = |label: &str, dflt: (f32, f32)| b.range(label).or_else(|_| d.range(label)).unwrap_or(dflt);
        let one = |label: &str, dflt: f32| b.f32(label).or_else(|_| d.f32(label)).unwrap_or(dflt);
        // "7 0.54 / 1.05 / ..." : count on the first line, then one value per line.
        let draw_modifier: Vec<f32> = {
            let src = if b.line("GESTURE_DEF_WEIGHT").is_some() { b } else { d };
            ["GESTURE_DEF_WEIGHT", "GESTURE_DEF_BOMB", "GESTURE_DEF_LIGHTNING", "GESTURE_DEF_PLANT", "GESTURE_DEF_UFO", "GESTURE_DEF_GHOST", "GESTURE_DEF_OCTOPUS"]
                .iter()
                .map(|l| src.floats(l).ok().and_then(|v| v.last().copied()).unwrap_or(1.0))
                .collect()
        };
        // kiDefendBombBarrierOffsets: "15" then three rows of five (x y z) points; the first row.
        let bomb_offsets = d
            .lines
            .iter()
            .skip_while(|l| !l.label.as_deref().is_some_and(|x| x.starts_with("kiDefendBombBarrierOffsetScaleRange")))
            .skip(1)
            .find(|l| l.values.len() >= 15)
            .map(|l| {
                let v: Vec<f32> = l.values.iter().filter_map(|x| x.parse().ok()).collect();
                let v = if v.len() % 3 == 1 { &v[1..] } else { &v[..] };
                v.chunks(3).take(5).map(|c| Vec2::new(c[0], c[1])).collect()
            })
            .unwrap_or_else(|| vec![Vec2::new(-2.0, -0.75), Vec2::new(-1.5, 0.7), Vec2::new(0.0, 0.9), Vec2::new(1.5, 0.7), Vec2::new(2.0, -0.75)]);
        let recheck = [
            (AttackKind::Plant, "kiDReCheckPlantAttackTimer"),
            (AttackKind::Ufo, "kiDReCheckUfoAttackTimer"),
            (AttackKind::Lightning, "kiDReCheckLightningAttackTimer"),
            (AttackKind::Ghost, "kiDReCheckGhostAttackTimer"),
            (AttackKind::Octopus, "kiDReCheckTentacleAttackTimer"),
        ]
        .into_iter()
        .map(|(k, l)| (k, one(l, 1.25)))
        .collect();
        let wobble = [
            ("shield", "kiBarrierDistortionDefendByShieldX", "kiBarrierDistortionDefendByShieldY"),
            ("plant", "kiBarrierDistortionDefendPlantMaxX", "kiBarrierDistortionDefendPlantMaxY"),
            ("weight", "", "kiBarrierDistortionDefendWeightMaxY"),
            ("ufo", "", "kiBarrierDistortionDefendUFOMaxY"),
            ("lightning", "kiBarrierDistortionDefendLightningMaxX", "kiBarrierDistortionDefendLightningMaxY"),
            ("ghost", "kiBarrierDistortionDefendGhostMaxX", "kiBarrierDistortionDefendGhostMaxY"),
            ("tentacle", "kiBarrierDistortionDefendTentacleMaxX", "kiBarrierDistortionDefendTentacleMaxY"),
        ]
        .into_iter()
        .map(|(k, x, y)| (k, Wobble(if x.is_empty() { 0.2 } else { one(x, 0.2) }, one(y, 0.2))))
        .collect();
        let ki = KiParams {
            attack_start: range("kiAttackStartTimer", (0.5, 0.8)),
            start_reaction: range("kiStartAttackReactionTime", (0.9, 1.5)),
            draw_skill_time: range("kiDrawingSkillTime", (0.08, 0.11)),
            draw_difficulty: range("kiDrawingDifficultyModifier", (3.0, 6.0)),
            draw_modifier,
            intelligent: one("kiIntelligentAttackSelection", 1.0) as u32,
            defend_cooldown: range("kiDefendCoolDownTimer", (0.75, 1.25)),
            fac_cooldown: one("kiFacCoolDownTimer", 1.0),
            barrier_draw_time: range("kiBarrierDrawTime", (0.15, 0.6)),
            recheck,
            fac_recheck: one("kiFacDReCheckAttack", 1.0),
            bomb_offsets,
            weight_barrier_y: one("kiDefendWeightBarrierY", 5.0),
            ufo_offset: Vec2::new(one("kiDefendUfoBarrierXOffset", 1.3), one("kiDefendUfoBarrierYOffset", 1.5)),
            ghost_approach_y: one("kiDefendGhostApproachThresholdY", 6.5),
            ghost_width: one("kiDefendGhostBarrierWidth", 2.5),
            ghost_distance: one("kiDefendGhostBarrierDistance", 2.5),
            tentacle_width: one("kiDefendTentacleBarrierWidth", 1.5),
            tentacle_y: one("kiDefendTentacleBarrierY", 4.0),
            tentacle_reaction: one("kiDefendTentacleReactionTime", 0.6),
            plant_reaction: one("kiDefendPlantReactionTime", 0.0),
            plant_max_dots: one("kiBarrierDefendPlantMaxDots", 6.0) as usize,
            lightning_approach: one("kiDefendLightningApproachThresholdPer", 0.6),
            delay_tentacle: one("kiDefendDelayTentacle", 0.1),
            delay_bomb: one("kiDefendDelayBombRangeNeg", -0.1),
            delay_weight: range("kiDefendDelayWeightRange", (-10.0, 30.0)),
            delay_lightning: range("kiDefendDelayLightningRange", (-0.35, 0.3)),
            fac_delay: one("kiFacDefendDelay", 1.0),
            wobble,
            fac_wobble: one("kiFacBarrierDistortion", 1.0),
            use_upgrade: one("kiAttackUseUpgradePropability", 0.35),
            red_upgrade: one("kiAttackWithUpgradeRedPropability", 0.35),
            upgrade_hit_time: range("kiUpgradeHitTime", (0.6, 0.9)),
        };
        Ok(Self {
            ki,
            sides: [Side { enabled: std::env::var("CCB_AUTOPLAY").is_ok(), ..default() }, Side { enabled: true, ..default() }],
        })
    }
}

/// How long the CPU shows a traced attack before firing (`CCB_CPU_ARM_TIME` overrides).
fn arm_time() -> f32 {
    std::env::var("CCB_CPU_ARM_TIME").ok().and_then(|v| v.parse().ok()).unwrap_or(0.5)
}

/// How `kiDrawingDifficultyModifier` scales the per-dot time (its exact use isn't known).
const DIFFICULTY_SCALE: f32 = 0.55;

/// Traces a gesture the CPU way: returns (duration, quality).
fn cpu_trace(ki: &KiParams, kind: AttackKind, dots: usize, timing: [f32; 5], rng: &mut Rng) -> (f32, f32) {
    let modifier = ki.draw_modifier.get(kind as usize).copied().unwrap_or(1.0);
    let mut t = 0.0;
    let mut times = vec![0.0];
    for _ in 1..dots.max(2) {
        // Scaled so a medium CPU lands around the 75-85% seen in the original's screens.
        t += rng.range(ki.draw_skill_time) * rng.range(ki.draw_difficulty) * DIFFICULTY_SCALE * modifier;
        times.push(t);
    }
    (t, trace_quality(&times, dots.max(2), timing))
}

/// Picks an attack: at random, or (`kiIntelligentAttackSelection`) by how the target's
/// chicks stand: a weight on a bunch, a bomb on a pair, the plant for strays.
fn choose_attack(ki: &KiParams, options: &[AttackKind], targets: &[Vec2], rng: &mut Rng) -> AttackKind {
    let random = rng.pick(options).unwrap_or(AttackKind::Bomb);
    if ki.intelligent == 0 || targets.is_empty() || rng.chance(0.25) {
        return random;
    }
    let best_group = targets.iter().map(|p| targets.iter().filter(|q| (q.x - p.x).abs() < 2.0).count()).max().unwrap_or(0);
    let want = match best_group {
        3.. => AttackKind::Weight,
        2 => AttackKind::Bomb,
        _ => AttackKind::Plant,
    };
    if options.contains(&want) {
        want
    } else {
        random
    }
}

/// The line a defence recipe calls for (before wobble), or None when it's not time yet.
fn defence_line(ki: &KiParams, a: &Attack, tuning: &Tuning, chicks: &[(Vec2, f32)], rng: &mut Rng) -> Option<(Vec<Vec2>, &'static str)> {
    let team = a.target();
    let (lo, hi) = tuning.side_range(team);
    let clamp_x = |x: f32| x.clamp(lo - 0.5, hi + 0.5);
    let horizontal = |cx: f32, y: f32, half: f32| {
        let cx = cx.clamp(lo + half - 0.8, hi - half + 0.8);
        (0..=6).map(|i| Vec2::new(cx - half + 2.0 * half * i as f32 / 6.0, y)).collect::<Vec<_>>()
    };
    let top = tuning.barrier_y_clip - 0.3;
    match (a.kind, a.state) {
        // A cage over the landed bomb: the blast can't pass the line.
        (AttackKind::Bomb, AttackState::Roll) => {
            let c = Vec2::new(a.pos.x, 0.75);
            Some((ki.bomb_offsets.iter().map(|o| c + *o).collect(), "shield"))
        }
        // A lid across the weight's path, wider than the weight.
        (AttackKind::Weight, AttackState::Travel) => Some((horizontal(a.target_x, ki.weight_barrier_y.min(top), weight_half(a, tuning) + 0.5), "weight")),
        // A ramp in front of the plant's head that leads it up past its height limit.
        (AttackKind::Plant, AttackState::Travel) => {
            let head = a.pos;
            let prey = chicks.iter().map(|c| c.0).min_by(|x, y| x.distance(head).total_cmp(&y.distance(head)))?;
            let dir = (prey.x - head.x).signum();
            let base = Vec2::new(clamp_x(head.x + dir * 0.6), (head.y - 0.2).max(0.3));
            let tip = Vec2::new(clamp_x(head.x - dir * 0.4), top);
            let n = ki.plant_max_dots.max(2);
            Some(((0..n).map(|i| base.lerp(tip, i as f32 / (n - 1) as f32)).collect(), "plant"))
        }
        // A lid over the chick the UFO is after.
        (AttackKind::Ufo, AttackState::Prepare) => {
            let c = chicks.iter().min_by(|x, y| (x.0.x - a.target_x).abs().total_cmp(&(y.0.x - a.target_x).abs())).map_or(Vec2::new(a.target_x, 0.6), |c| c.0);
            Some((horizontal(c.x, (c.y + ki.ufo_offset.y + 0.5).min(top), ki.ufo_offset.x), "ufo"))
        }
        // A lid over the spot the tentacle slaps.
        (AttackKind::Octopus, AttackState::Prepare | AttackState::Travel) => Some((horizontal(a.target_x, (ki.tentacle_y - 1.5).min(top), ki.tentacle_width), "tentacle")),
        // A wall between the ghost and its target, once it comes down.
        (AttackKind::Ghost, AttackState::Travel) if a.pos.y < ki.ghost_approach_y => {
            let goal = Vec2::new(a.target_x, 1.0);
            let dir = (a.pos - goal).normalize_or(Vec2::Y);
            let c = goal + dir * ki.ghost_distance.min(a.pos.distance(goal) * 0.6);
            let n = Vec2::new(-dir.y, dir.x) * ki.ghost_width / 2.0;
            Some(((0..=6).map(|i| c - n + n * 2.0 * i as f32 / 6.0).collect(), "ghost"))
        }
        // An earthed rod under the cloud.
        (AttackKind::Lightning, AttackState::Prepare) if a.t > tuning.attack_prepare_time * (1.0 - ki.lightning_approach) => {
            let x = clamp_x(a.target_x + rng.range((-0.6, 0.6)));
            Some(((0..=6).map(|i| Vec2::new(x, 0.05 + i as f32 * 0.5)).collect(), "lightning"))
        }
        _ => None,
    }
}

/// Half the width of a falling weight (for the lid).
fn weight_half(a: &Attack, tuning: &Tuning) -> f32 {
    let variant = tuning.weight_quality_thresholds.iter().rposition(|&th| a.quality >= th).unwrap_or(0);
    tuning.weight_widths.get(variant).copied().unwrap_or(4.0) / 2.0
}

/// Reaction delay before answering an attack (`kiDefendDelay*`, scaled by `kiFacDefendDelay`).
fn defence_delay(ki: &KiParams, kind: AttackKind, rng: &mut Rng) -> f32 {
    let d = match kind {
        AttackKind::Octopus => ki.delay_tentacle.max(ki.tentacle_reaction * 0.5),
        AttackKind::Bomb => ki.delay_bomb,
        // The weight range is in hundredths of a second.
        AttackKind::Weight => rng.range(ki.delay_weight) / 100.0,
        AttackKind::Lightning => rng.range(ki.delay_lightning),
        AttackKind::Plant => ki.plant_reaction,
        _ => 0.2,
    };
    (d * ki.fac_delay).max(0.0)
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
    mut view: ResMut<IfcView>,
    attacks: Query<(Entity, &Attack)>,
    chicks: Query<&Chick>,
    mut barriers: Query<&mut Barrier>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut sfx: ResMut<crate::sfx::Sfx>,
) {
    let dt = time.delta_secs();
    let ki = cpu.ki.clone();
    for team in [Team::Yellow, Team::Black] {
        let side = &mut cpu.sides[team.index()];
        if !side.enabled {
            continue;
        }
        // ---- attacking
        if m.phase == Phase::Attack && m.attacker == team && m.pending.is_none() && side.armed.is_none() {
            match side.drawing {
                None => {
                    if side.think <= 0.0 {
                        side.think = rng.range(ki.attack_start) + rng.range(ki.start_reaction);
                    }
                    side.think -= dt;
                    if side.think <= 0.0 {
                        let options: Vec<AttackKind> = m.available.iter().copied().filter(|k| k.implemented() && k.is_basic()).collect();
                        let targets: Vec<Vec2> = chicks.iter().filter(|c| c.team == team.other() && c.alive()).map(|c| c.pos).collect();
                        let kind = choose_attack(&ki, &options, &targets, &mut rng);
                        // Debug: CCB_FORCE_ATTACK=bomb|weight|plant makes the CPU always pick it.
                        let kind = std::env::var("CCB_FORCE_ATTACK").ok().and_then(|n| options.iter().copied().find(|k| k.gesture_group() == n)).unwrap_or(kind);
                        let (duration, quality) = cpu_trace(&ki, kind, gestures.dots(kind), tuning.gesture_timing, &mut rng);
                        side.drawing = Some((kind, duration, quality));
                        side.drawing_total = duration;
                    }
                }
                Some((kind, left, quality)) => {
                    let left = left - dt;
                    if left <= 0.0 {
                        side.armed = Some((kind, quality, arm_time()));
                        side.armed_t = 0.0;
                        side.upgrade = 0;
                        side.upgrade_plan = None;
                        // A good trace brings up the target; the CPU sometimes shoots it.
                        if quality >= tuning.up.a_min && rng.chance(ki.use_upgrade) {
                            let red = quality >= tuning.up.b_min && rng.chance(ki.red_upgrade);
                            let at = rng.range(ki.upgrade_hit_time);
                            side.upgrade_plan = Some((if red { 2 } else { 1 }, at));
                            side.armed = Some((kind, quality, arm_time().max(at + 0.4)));
                        }
                        // Debug: CCB_FORCE_UPGRADE=1|2 makes the CPU always take that upgrade.
                        if let Some(u) = std::env::var("CCB_FORCE_UPGRADE").ok().and_then(|v| v.parse::<u8>().ok()) {
                            side.upgrade_plan = Some((u, 0.7));
                            side.armed = Some((kind, quality, 1.1));
                        }
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
        // A traced attack sits on its platform for a moment, then fires.
        if let Some((kind, quality, left)) = side.armed {
            side.armed_t += dt;
            if let Some((u, at)) = side.upgrade_plan.filter(|p| side.armed_t >= p.1) {
                let _ = at;
                side.upgrade = u;
                side.upgrade_plan = None;
                sfx.play(if u == 2 { "SFX_ATTACK_IFC_UPGRADE_TARGET_02" } else { "SFX_ATTACK_IFC_UPGRADE_TARGET_01" });
            }
            let left = left - dt;
            if left > 0.0 {
                side.armed = Some((kind, quality, left));
            } else {
                side.armed = None;
                if kind.is_basic() {
                    if m.phase == Phase::Attack && m.attacker == team && m.pending.is_none() {
                        info!("cpu ({team:?}) attacks with {kind:?} at {:.0}% (upgrade {})", quality * 100.0, side.upgrade);
                        m.pending = Some((kind, quality, side.upgrade));
                    }
                } else if m.pending_special.is_none() && m.special.is_some_and(|(k, _)| k == kind) {
                    m.pending_special = Some((team, kind, quality));
                }
            }
        }

        // ---- racing for an open special
        match (m.special, side.special) {
            // Never call lightning down on your own side.
            (Some((kind, _)), None) if m.pending_special.is_none() && side.armed.is_none() && m.special_target != Some(team) => {
                let (trace, quality) = cpu_trace(&ki, kind, gestures.dots(kind), tuning.gesture_timing, &mut rng);
                let duration = rng.range(ki.start_reaction) + trace;
                side.special = Some((kind, duration, quality));
                side.special_total = duration;
            }
            (Some((open, _)), Some((kind, left, quality))) if open == kind => {
                let left = left - dt;
                if left <= 0.0 {
                    if side.armed.is_none() {
                        side.armed = Some((kind, quality, 0.35));
                    }
                    side.special = None;
                } else {
                    side.special = Some((kind, left, quality));
                }
            }
            (None, Some(_)) => side.special = None,
            _ => {}
        }
        // What this side's attack interface shows.
        view.target[team.index()] = side
            .armed
            .filter(|(kind, quality, _)| kind.is_basic() && *quality >= tuning.up.a_min && side.upgrade == 0)
            .map(|(_, quality, _)| super::attack_ui::upgrade_target(side.armed_t, quality >= tuning.up.b_min, tuning.up.target_appear));
        view.mode[team.index()] = if let Some((kind, quality, _)) = side.armed {
            Mode::Armed { kind, quality, upgrade: side.upgrade }
        } else if let Some((kind, left, quality)) = side.drawing.or(side.special) {
            let total = if side.drawing.is_some() { side.drawing_total } else { side.special_total };
            let k = (1.0 - left / total.max(1e-3)).clamp(0.0, 1.0);
            Mode::Tracing { kind, done: (k * gestures.dots(kind) as f32) as usize, quality: quality * k }
        } else {
            Mode::Idle
        };

        // ---- defending
        side.cooldown -= dt;
        for t in side.handled.values_mut() {
            *t -= dt;
        }
        side.handled.retain(|e, t| *t > 0.0 && attacks.get(*e).is_ok());
        // Keep drawing the current line.
        if let Some(line) = side.line.as_mut() {
            line.t += dt;
            let want = ((line.t / line.duration).min(1.0) * (line.points.len() - 1) as f32).ceil() as usize + 1;
            if let Ok(mut b) = barriers.get_mut(line.entity) {
                while line.shown < want.min(line.points.len()) {
                    let p = line.points[line.shown];
                    let cost = b.points.last().map_or(0.0, |l| l.distance(p));
                    if ink.0[team.index()] < cost {
                        line.shown = line.points.len();
                        break;
                    }
                    ink.0[team.index()] -= cost;
                    b.add_point(p);
                    line.shown += 1;
                }
                if line.shown >= line.points.len() {
                    b.drawing = false;
                }
            } else {
                line.shown = line.points.len();
            }
            if line.shown >= line.points.len() {
                side.line = None;
                side.cooldown = rng.range(ki.defend_cooldown) * ki.fac_cooldown;
            }
            continue;
        }
        if side.cooldown > 0.0 {
            continue;
        }
        let my_chicks: Vec<(Vec2, f32)> = chicks.iter().filter(|c| c.team == team && c.alive()).map(|c| (c.pos, c.radius)).collect();
        // Plan an answer to the next unanswered attack.
        if side.planned.is_none() {
            let next = attacks.iter().find(|(e, a)| a.target() == team && !side.handled.contains_key(e) && defence_line(&ki, a, &tuning, &my_chicks, &mut rng).is_some());
            if let Some((e, a)) = next {
                side.planned = Some((e, defence_delay(&ki, a.kind, &mut rng)));
            }
        }
        let Some((e, wait)) = side.planned else { continue };
        if wait > 0.0 {
            side.planned = Some((e, wait - dt));
            continue;
        }
        side.planned = None;
        let Ok((_, a)) = attacks.get(e) else { continue };
        let Some((points, kind)) = defence_line(&ki, a, &tuning, &my_chicks, &mut rng) else { continue };
        side.handled.insert(e, ki.recheck.get(&a.kind).copied().unwrap_or(30.0) * ki.fac_recheck);
        let w = ki.wobble.get(kind).copied().unwrap_or(Wobble(0.2, 0.2));
        let points: Vec<Vec2> = points
            .into_iter()
            .map(|p| {
                let off = Vec2::new(rng.range((-w.0, w.0)), rng.range((-w.1, w.1))) * ki.fac_wobble * 0.5;
                Vec2::new(p.x + off.x, (p.y + off.y).clamp(0.05, tuning.barrier_y_clip - 0.1))
            })
            .collect();
        if ink.0[team.index()] < 0.5 || points.len() < 2 {
            continue;
        }
        let entity = spawn_barrier(&mut commands, team, vec![points[0], points[0]], true, &tuning, &mut meshes, &mut materials);
        debug!("cpu ({team:?}) answers {:?} with a {kind} line", a.kind);
        side.line = Some(CpuLine { entity, points, shown: 1, t: 0.0, duration: rng.range(ki.barrier_draw_time) });
    }
}
