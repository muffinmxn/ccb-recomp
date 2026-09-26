//! Gameplay constants read from the game's cfg files, with the field names used there.

use anyhow::Result;
use bevy::prelude::*;

use crate::data::GameData;

/// Difficulty index into the cfg's `[KI_SELECTABLE_COUNT]` tables (0 easy, 1 medium, 2 hard).
pub const DIFFICULTY: usize = 1;

/// Some fields are read for completeness but not used by gameplay yet.
#[allow(dead_code)]
#[derive(Resource, Clone, Debug)]
pub struct Tuning {
    // ingame.model.misc
    /// Half-width of one team's playing area (`w` of the virtual level space).
    pub side_width: f32,
    pub separator: f32,
    pub plane_z: f32,
    // ingame.model.chick
    pub chick_health: f32,
    pub chick_radius: f32,
    pub chick_dying_time: f32,
    pub chick_squashed_time: f32,
    /// `chickSize` (small chick radius) and `chickGeneralSizeFac`.
    pub chick_size: f32,
    pub general_size_fac: f32,
    pub chick_start_y: f32,
    pub chick_start_force: (Vec2, Vec2),
    pub chick_start_force_time: f32,
    pub chick_dying_fade: f32,
    pub chick_dying_speed: f32,
    /// Head turn: angle range (degrees), change-timer range, damping per 60 Hz frame.
    pub chick_rot_angles: (f32, f32),
    pub chick_rot_timer: (f32, f32),
    pub chick_rot_damp: f32,
    // ingame.model.particleSystem / *.physics.jumpCond
    pub gravity: f32,
    pub vel_damp: f32,
    pub physics_fps: f32,
    pub chick_jump: JumpCond,
    pub general_jump: JumpCond,
    // ingame.model.barrier.*
    pub ink_max: f32,
    pub ink_reload: f32,
    pub barrier_point_distance: f32,
    pub barrier_max_len: f32,
    pub barrier_full_time: f32,
    pub barrier_fade_time: f32,
    pub barrier_thickness: f32,
    pub barrier_y_clip: f32,
    // ingame.model.attack
    pub attack_time: f32,
    pub defend_time: f32,
    pub attack_starting_time: f32,
    pub attack_prepare_time: f32,
    pub attack_finish_time: f32,
    /// Seconds between special-attack windows (first window, later windows).
    pub special_first: (f32, f32),
    pub special_every: (f32, f32),
    /// How long a special window stays open (`ufoEffectiveTimer`).
    pub special_window: f32,
    // bomb
    pub bomb_appear: Vec2,
    pub bomb_damage: DamageSettings,
    pub bomb_exp_radius: (f32, f32),
    pub bomb_exp_full_radius: f32,
    pub bomb_roll_time: f32,
    // weight
    pub weight_damage: DamageSettings,
    pub weight_quality_thresholds: Vec<f32>,
    // environment
    pub cloud_y: f32,
    // gesture
    /// (total time for 100%, total time for 0%, per-dot time for 100%, per-dot time for min, min per-dot quality)
    pub gesture_timing: [f32; 5],
    pub gesture_dot_radius: f32,
    pub gesture_panel: [Vec2; 2],
}

/// `ingame.model.<chick|general>.physics.jumpCond`: chicks hop continuously; a jump starts
/// `delay` after landing and pushes with `vel` (an acceleration) for `duration`.
#[derive(Clone, Copy, Debug)]
pub struct JumpCond {
    pub duration: f32,
    pub delay: f32,
    pub vel: Vec2,
    /// `directionChangeAdd/Pow/Fac`: chance to turn around grows with jumps since the last turn.
    pub dir_add: f32,
    pub dir_pow: f32,
    pub dir_fac: f32,
    pub variance: f32,
    pub bounce: f32,
}

impl JumpCond {
    fn from(v: &[f32]) -> Self {
        let g = |i: usize, d: f32| v.get(i).copied().unwrap_or(d);
        Self {
            duration: g(1, 0.2),
            delay: g(2, 0.03),
            vel: Vec2::new(g(3, 120.0), g(4, 100.0)),
            dir_add: g(7, 4.0),
            dir_pow: g(8, 2.0),
            dir_fac: g(9, 1.0),
            variance: g(10, 0.0),
            bounce: g(11, 0.75),
        }
    }

    /// Chance to change direction on a jump after `count` jumps the same way.
    pub fn turn_chance(&self, count: u32) -> f32 {
        let c = count as f32;
        c.powf(self.dir_pow) / ((c + self.dir_add).powf(self.dir_pow) * self.dir_fac).max(1e-3)
    }
}

/// `min max threshold thresholdDamage perfectDamage`: damage scales from `min` at quality 0
/// to `max` at `threshold`...1, and a perfect gesture deals `perfect`.
#[derive(Clone, Copy, Debug)]
pub struct DamageSettings {
    pub min: f32,
    pub max: f32,
    pub threshold: f32,
    pub threshold_damage: f32,
    pub perfect: f32,
}

impl DamageSettings {
    fn from(v: &[f32]) -> Self {
        let g = |i: usize, d: f32| v.get(i).copied().unwrap_or(d);
        Self { min: g(0, 20.0), max: g(1, 35.0), threshold: g(2, 0.5), threshold_damage: g(3, 20.0), perfect: g(4, 40.0) }
    }

    pub fn damage(&self, quality: f32) -> f32 {
        if quality >= 0.999 {
            self.perfect
        } else if quality < self.threshold {
            self.min + (self.threshold_damage - self.min) * (quality / self.threshold.max(1e-3))
        } else {
            self.threshold_damage + (self.max - self.threshold_damage) * ((quality - self.threshold) / (1.0 - self.threshold).max(1e-3))
        }
    }
}

impl Tuning {
    pub fn load(data: &GameData) -> Result<Self> {
        let cfg = &data.cfg;
        let misc = cfg.block("ingame.model.misc")?;
        let chick = cfg.block("ingame.model.chick")?;
        let bc = cfg.block("ingame.model.barrier.common")?;
        let bs = cfg.block("ingame.model.barrier.standard")?;
        let at = cfg.block("ingame.model.attack")?;
        let bomb = cfg.block("ingame.model.attack.bomb.standard")?;
        let env = cfg.block("ingame.model.environment")?;
        let ges = cfg.block("ingame.model.gesture")?;
        let ps: Vec<f32> = cfg.block("ingame.model.particleSystem")?.values().filter_map(|v| v.parse().ok()).collect();
        let jump = |name: &str| -> Result<JumpCond> {
            Ok(JumpCond::from(&cfg.block(name)?.values().filter_map(|v| v.parse().ok()).collect::<Vec<f32>>()))
        };
        let v2 = |v: [f32; 3]| Vec2::new(v[0], v[1]);

        // Anonymous numbers are located by position in the value stream.
        let misc_v: Vec<f32> = misc.values().filter_map(|v| v.parse().ok()).collect();
        let at_v: Vec<&str> = at.values().collect();
        let num = |s: &str| s.parse::<f32>().unwrap_or(0.0);
        // attackModeTimersDuell: "3 35 25 15" -> count then values.
        let table = |label: &str| -> Vec<f32> {
            at.line(label).map(|l| l.values.iter().skip(1).map(|v| num(v)).collect()).unwrap_or_default()
        };
        let attack_times = table("attackModeTimersDuell");
        // envAssist*PauseDuell: "3  35 40 35 40 5 10" -> count, then a (min, max) pair per difficulty.
        let pair = |label: &str| -> (f32, f32) {
            let v = table(label);
            (v.get(DIFFICULTY * 2).copied().unwrap_or(30.0), v.get(DIFFICULTY * 2 + 1).copied().unwrap_or(40.0))
        };
        let defend_times = table("defendModeTimersDuell");
        let weight_thresholds = at
            .line("weightQualityTypeThresholds")
            .map(|l| l.values.iter().skip(1).map(|v| num(v) / 100.0).collect())
            .unwrap_or_else(|| vec![0.0, 0.6, 0.7, 0.8, 0.9, 1.0]);
        let _ = at_v;

        let ges_first: Vec<f32> = ges.lines.first().map(|l| l.values.iter().map(|v| num(v)).collect()).unwrap_or_default();
        // First line is "8 5 10.0 0.2 0.4 0.05" (count, then the weight gesture's values).
        let gesture_timing = if ges_first.len() >= 6 {
            [ges_first[1], ges_first[2], ges_first[3], ges_first[4], ges_first[5]]
        } else {
            [5.0, 10.0, 0.2, 0.4, 0.05]
        };
        let panel_lines: Vec<Vec<f32>> = ges
            .lines
            .iter()
            .filter(|l| l.values.len() == 3 && l.label.is_none())
            .map(|l| l.values.iter().map(|v| num(v)).collect())
            .collect();
        let gesture_panel = if panel_lines.len() >= 2 {
            [Vec2::new(panel_lines[0][0], panel_lines[0][1]), Vec2::new(panel_lines[1][0], panel_lines[1][1])]
        } else {
            [Vec2::new(-160.0, -125.0), Vec2::new(160.0, -125.0)]
        };

        Ok(Self {
            side_width: misc_v.get(3).copied().unwrap_or(10.8),
            separator: misc_v.get(6).copied().unwrap_or(0.3),
            plane_z: misc_v.get(2).copied().unwrap_or(2.0),
            chick_health: chick.f32("chickHealth")?,
            chick_radius: chick.f32("chickSize")? * chick.f32("chickGeneralSizeFac")?,
            chick_dying_time: chick.f32("chickDyingTimer")?,
            chick_squashed_time: chick.f32("chickSquashedTimer")?,
            chick_size: chick.f32("chickSize")? * chick.f32("chickSizeFac")?,
            general_size_fac: chick.f32("chickGeneralSizeFac")?,
            chick_start_y: chick.f32("chickStartPosY")?,
            chick_start_force: (v2(chick.vec3("chickStartForceMin")?), v2(chick.vec3("chickStartForceMax")?)),
            chick_start_force_time: chick.f32("chickStartForceDuration")?,
            chick_dying_fade: chick.f32("chickDyingFadeoutTimer")?,
            chick_dying_speed: chick.f32("chickDyingMoveSpeed")?,
            chick_rot_angles: chick.lines.iter().find(|l| l.label.as_deref().is_some_and(|x| x.starts_with("min/max angle"))).and_then(|l| Some((l.values.first()?.parse().ok()?, l.values.get(1)?.parse().ok()?))).unwrap_or((-20.0, 90.0)),
            chick_rot_timer: chick.lines.iter().find(|l| l.label.as_deref().is_some_and(|x| x.starts_with("min/max timer"))).and_then(|l| Some((l.values.first()?.parse().ok()?, l.values.get(1)?.parse().ok()?))).unwrap_or((0.2, 1.0)),
            chick_rot_damp: chick.lines.iter().find(|l| l.label.as_deref().is_some_and(|x| x.starts_with("dampening"))).and_then(|l| l.values.first()?.parse().ok()).unwrap_or(0.11),
            gravity: ps.get(1).copied().unwrap_or(-42.0),
            vel_damp: ps.get(3).copied().unwrap_or(0.06),
            physics_fps: ps.get(4).copied().unwrap_or(180.0),
            chick_jump: jump("ingame.model.chick.physics.jumpCond")?,
            general_jump: jump("ingame.model.general.physics.jumpCond")?,
            ink_max: bc.f32("barrierAccountMaxPower")?,
            ink_reload: bc.f32("barrierReloadSpeed")?,
            barrier_point_distance: bs.f32("BARRIER_POINT_DISTANCE_THRESHOLD")?,
            barrier_max_len: bs.f32("max")?,
            barrier_full_time: bs.f32("fullStrengthTimer")?,
            barrier_fade_time: bs.f32("fadeOutTimer")?,
            barrier_thickness: bs.f32("barrierSegmentFullThickness")?,
            barrier_y_clip: bs.f32("yClipping")?,
            attack_time: attack_times.get(DIFFICULTY).copied().unwrap_or(25.0),
            special_first: pair("envAssistInitPauseDuell"),
            special_every: pair("envAssistPauseDuell"),
            special_window: env.f32("ufoEffectiveTimer").unwrap_or(15.0),
            defend_time: defend_times.get(DIFFICULTY).copied().unwrap_or(7.0),
            attack_starting_time: at.f32("attackStartingTimer")?,
            attack_prepare_time: at.f32("attackPrepareTimer")?,
            attack_finish_time: at.f32("attackFinishTimer")?,
            bomb_appear: Vec2::new(bomb.f32("bombAppearPosX")?, bomb.f32("bombAppearPosY")?),
            bomb_damage: DamageSettings::from(&bomb.floats("bombDamageSettings")?),
            bomb_exp_radius: (bomb.f32("bombExpMinRadius")?, bomb.f32("bombExpMaxRadius")?),
            bomb_exp_full_radius: bomb.f32("bombExpFullDamageRadius")?,
            bomb_roll_time: bomb.f32("bombRollTimer")?,
            weight_damage: DamageSettings::from(&at.floats("weightDamageSettings")?),
            weight_quality_thresholds: weight_thresholds,
            cloud_y: env.f32("cloudY")?,
            gesture_timing,
            gesture_dot_radius: ges.f32("overrideCpSize").unwrap_or(15.0),
            gesture_panel,
        })
    }

    /// X range of a team's playing area.
    pub fn side_range(&self, team: super::Team) -> (f32, f32) {
        let inner = self.separator / 2.0 + self.chick_radius;
        let outer = self.side_width;
        match team {
            super::Team::Yellow => (-outer, -inner),
            super::Team::Black => (inner, outer),
        }
    }
}
