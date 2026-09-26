//! Gesture tracing on the blueprint panel. Each attack's path is the list of `cp_*`
//! control points in its group of the `blueprints` layout; the player drags the pointer
//! through them in order. Quality comes from the timings in `ingame.model.gesture`.

use std::collections::HashMap;

use anyhow::Result;
use bevy::prelude::*;

use super::{
    attack_ui::{IfcView, Mode, CANCEL_POS, CONFIRM_POS},
    flow::Match,
    tuning::Tuning,
    AttackKind, Team,
};
use crate::{layout::LayoutAssets, pointer::Pointer};

/// Control-point paths per attack, in layout units relative to the panel center.
#[derive(Resource)]
pub struct Gestures(pub HashMap<AttackKind, Vec<Vec2>>);

impl Gestures {
    pub fn load(assets: &LayoutAssets) -> Result<Self> {
        let l = assets.layout("blueprints")?;
        let mut paths = HashMap::new();
        for id in 0..8 {
            let Some(kind) = AttackKind::from_id(id) else { continue };
            let Some(g) = l.pane(kind.gesture_group()) else { continue };
            let pts: Vec<Vec2> = l
                .panes
                .iter()
                .filter(|p| p.parent == Some(g) && p.name.starts_with("cp_"))
                .map(|p| Vec2::new(p.translate[0], p.translate[1]))
                .collect();
            if !pts.is_empty() {
                paths.insert(kind, pts);
            }
        }
        Ok(Self(paths))
    }

    pub fn dots(&self, kind: AttackKind) -> usize {
        self.0.get(&kind).map_or(10, Vec::len)
    }
}

/// The player's trace in progress.
#[derive(Resource, Default)]
pub struct Trace {
    pub kind: Option<AttackKind>,
    dots: Vec<Vec2>,
    next: usize,
    times: Vec<f32>,
    clock: f32,
    started: bool,
    /// Closest the pointer came to each dot hit so far.
    miss: Vec<f32>,
    /// Traced and waiting for the trigger: quality.
    armed: Option<f32>,
    /// When it was traced (on `clock`), the upgrade hit on the target, and whether the
    /// target was announced.
    armed_at: f32,
    upgrade: u8,
    offered: bool,
}

/// Accuracy share of the quality: passing a dot within `upgradeDotsCollisionDistanceGreen`
/// counts fully, dropping to half at the outer (white) radius.
pub fn trace_accuracy(miss: &[f32], green: f32, white: f32) -> f32 {
    if miss.is_empty() {
        return 1.0;
    }
    let per = miss.iter().map(|&d| 1.0 - 0.5 * ((d - green) / (white - green).max(1e-3)).clamp(0.0, 1.0));
    per.sum::<f32>() / miss.len() as f32
}

pub fn trace_quality(times: &[f32], dots: usize, timing: [f32; 5]) -> f32 {
    let [total_best, total_worst, dot_best, dot_worst, dot_min] = timing;
    if times.len() < 2 || dots < 2 {
        return 0.0;
    }
    let per_dot: Vec<f32> = times
        .windows(2)
        .map(|w| {
            let dt = w[1] - w[0];
            let q = 1.0 - (dt - dot_best) / (dot_worst - dot_best).max(1e-3);
            q.clamp(dot_min, 1.0)
        })
        .collect();
    let mean = per_dot.iter().sum::<f32>() / per_dot.len() as f32;
    let total = times.last().unwrap() - times[0];
    let total_q = (1.0 - (total - total_best) / (total_worst - total_best).max(1e-3)).clamp(0.0, 1.0);
    let progress = (times.len() as f32 / dots as f32).min(1.0);
    ((0.5 * mean + 0.5 * total_q) * progress).clamp(0.0, 1.0)
}

/// The player's gesture: trace the dots in the panel (the red X cancels before starting),
/// then fire with the trigger (right mouse / space, or the fire button).
#[allow(clippy::too_many_arguments)]
pub fn player_gesture(
    time: Res<Time>,
    pointer: Res<Pointer>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    tuning: Res<Tuning>,
    gestures: Res<Gestures>,
    mut m: ResMut<Match>,
    mut trace: ResMut<Trace>,
    mut view: ResMut<IfcView>,
    mut sfx: ResMut<crate::sfx::Sfx>,
) {
    let Some(team) = view.player else { return };
    let mirror = if team == Team::Yellow { 1.0 } else { -1.0 };
    let panel = tuning.gesture_panel[team.index()];
    if m.selected != trace.kind {
        *trace = Trace { kind: m.selected, ..default() };
        if let Some(kind) = m.selected {
            trace.dots = gestures.0.get(&kind).cloned().unwrap_or_default().into_iter().map(|p| panel + p).collect();
        }
    }
    let Some(kind) = trace.kind else {
        view.mode[team.index()] = Mode::Idle;
        return;
    };
    let fire_btn = panel + CONFIRM_POS * Vec2::new(mirror, 1.0);
    if let Some(q) = trace.armed {
        let ti = team.index();
        trace.clock += time.delta_secs();
        // A good trace offers the upgrade target: click its green ring or red bullseye.
        let offer = kind.is_basic() && q >= tuning.up.a_min && trace.upgrade == 0;
        let target = offer.then(|| super::attack_ui::upgrade_target(trace.clock - trace.armed_at, q >= tuning.up.b_min, tuning.up.target_appear));
        if offer && !trace.offered {
            trace.offered = true;
            sfx.play("SFX_ATTACK_IFC_UPGRADE_TARGET_00");
        }
        view.target[ti] = target;
        let mut picked = false;
        if let (Some(g), true, Some(p)) = (target, pointer.just_pressed, pointer.pos) {
            let hit = super::attack_ui::target_hit(&g, (p - panel) * Vec2::new(mirror, 1.0), &tuning.up);
            if hit > 0 {
                info!("upgrade {hit} for {kind:?}");
                trace.upgrade = hit;
                picked = true;
                sfx.play(if hit == 2 { "SFX_ATTACK_IFC_UPGRADE_TARGET_02" } else { "SFX_ATTACK_IFC_UPGRADE_TARGET_01" });
                view.target[ti] = None;
            }
        }
        view.mode[ti] = Mode::Armed { kind, quality: q, upgrade: trace.upgrade };
        let clicked = !picked && pointer.just_pressed && pointer.pos.is_some_and(|p| p.distance(fire_btn) < 36.0);
        // Out of turn time: the traced attack goes off by itself.
        let timeout = kind.is_basic() && m.phase == super::flow::Phase::Attack && m.turn_timer < 0.2;
        if buttons.just_pressed(MouseButton::Right) || keys.just_pressed(KeyCode::Space) || clicked || timeout {
            if kind.is_basic() {
                m.pending = Some((kind, q, trace.upgrade));
            } else {
                m.pending_special = Some((team, kind, q));
            }
            m.selected = None;
            view.target[ti] = None;
        }
        return;
    }
    let cancel_btn = panel + CANCEL_POS * Vec2::new(mirror, 1.0);
    let on_cancel = pointer.pos.is_some_and(|p| p.distance(cancel_btn) < 25.0);
    if pointer.just_pressed && on_cancel && !trace.started {
        sfx.play("SFX_ATTACK_IFC_DRAWING_CANCELLED");
        m.selected = None;
        view.mode[team.index()] = Mode::Idle;
        return;
    }
    trace.clock += time.delta_secs();
    let radius = tuning.up.hit_r.max(tuning.gesture_dot_radius);
    let green = tuning.gesture_dot_radius.min(14.0);
    if let Some(p) = pointer.pos.filter(|_| pointer.pressed) {
        let next = trace.next;
        // Still near the last dot: keep the closest pass.
        if next > 0 {
            let d = p.distance(trace.dots[next - 1]);
            if let Some(m) = trace.miss.get_mut(next - 1) {
                *m = m.min(d);
            }
        }
        if next < trace.dots.len() && p.distance(trace.dots[next]) <= radius && (trace.started || next == 0) {
            trace.started = true;
            trace.next += 1;
            let d = p.distance(trace.dots[next]);
            trace.miss.push(d);
            sfx.play("SFX_ATTACK_IFC_DOT_HIT");
            let c = trace.clock;
            trace.times.push(c);
        }
    }
    let q = trace_quality(&trace.times, trace.dots.len(), tuning.gesture_timing) * trace_accuracy(&trace.miss, green, radius);
    view.mode[team.index()] = Mode::Tracing { kind, done: trace.next, quality: q };
    let finished = trace.next == trace.dots.len() && !trace.dots.is_empty();
    let released = trace.started && !pointer.pressed;
    if finished || released {
        info!("gesture {kind:?}: {}/{} dots, quality {:.0}%", trace.next, trace.dots.len(), q * 100.0);
        sfx.play(if !finished {
            "SFX_ATTACK_IFC_DRAWING_INTERRUPTED"
        } else if q >= 0.95 {
            "SFX_ATTACK_IFC_DRAWING_PERFECT"
        } else {
            "SFX_ATTACK_IFC_DRAWING_SUCCESS"
        });
        trace.armed = Some(q);
        trace.armed_at = trace.clock;
        view.mode[team.index()] = Mode::Armed { kind, quality: q, upgrade: 0 };
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fast_complete_trace_is_perfect() {
        let times: Vec<f32> = (0..10).map(|i| i as f32 * 0.15).collect();
        assert!((super::trace_quality(&times, 10, [5.0, 10.0, 0.2, 0.4, 0.05]) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn sloppy_dots_cost_accuracy() {
        assert_eq!(super::trace_accuracy(&[5.0, 10.0], 14.0, 24.0), 1.0);
        assert!((super::trace_accuracy(&[24.0, 24.0], 14.0, 24.0) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn slow_or_partial_traces_score_lower() {
        let slow: Vec<f32> = (0..10).map(|i| i as f32 * 0.6).collect();
        let half: Vec<f32> = (0..5).map(|i| i as f32 * 0.15).collect();
        let t = [5.0, 10.0, 0.2, 0.4, 0.05];
        assert!(super::trace_quality(&slow, 10, t) < 0.6);
        assert!(super::trace_quality(&half, 10, t) <= 0.5);
    }
}
