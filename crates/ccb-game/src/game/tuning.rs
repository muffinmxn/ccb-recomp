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

        // Anonymous numbers are located by position in the value stream.
        let misc_v: Vec<f32> = misc.values().filter_map(|v| v.parse().ok()).collect();
        let at_v: Vec<&str> = at.values().collect();
        let num = |s: &str| s.parse::<f32>().unwrap_or(0.0);
        // attackModeTimersDuell: "3 35 25 15" -> count then values.
        let table = |label: &str| -> Vec<f32> {
            at.line(label).map(|l| l.values.iter().skip(1).map(|v| num(v)).collect()).unwrap_or_default()
        };
        let attack_times = table("attackModeTimersDuell");
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
            ink_max: bc.f32("barrierAccountMaxPower")?,
            ink_reload: bc.f32("barrierReloadSpeed")?,
            barrier_point_distance: bs.f32("BARRIER_POINT_DISTANCE_THRESHOLD")?,
            barrier_max_len: bs.f32("max")?,
            barrier_full_time: bs.f32("fullStrengthTimer")?,
            barrier_fade_time: bs.f32("fadeOutTimer")?,
            barrier_thickness: bs.f32("barrierSegmentFullThickness")?,
            barrier_y_clip: bs.f32("yClipping")?,
            attack_time: attack_times.get(DIFFICULTY).copied().unwrap_or(25.0),
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
