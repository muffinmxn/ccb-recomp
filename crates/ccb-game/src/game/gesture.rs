//! Gesture tracing on the blueprint panel. Each attack's path is the list of `cp_*`
//! control points in its group of the `blueprints` layout; the player drags the pointer
//! through them in order. Quality comes from the timings in `ingame.model.gesture`.

use std::collections::HashMap;

use anyhow::Result;
use bevy::{camera::visibility::RenderLayers, prelude::*};

use super::{flow::Match, tuning::Tuning, AttackKind, Team};
use crate::{
    gx_material::GxMaterial,
    layout::{LayoutAssets, UI_LAYER},
    pointer::Pointer,
};

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
    sprites: Vec<Entity>,
}

#[derive(Component)]
pub struct GestureSprite;

/// Spawns a centered UI sprite of `size` layout units.
#[allow(clippy::too_many_arguments)]
pub fn ui_sprite(
    commands: &mut Commands,
    assets: &mut LayoutAssets,
    texture: &str,
    center: Vec2,
    size: Vec2,
    z: f32,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<GxMaterial>,
    images: &mut Assets<Image>,
) -> Entity {
    let (mesh, mat) = assets.sprite(texture, meshes, materials, images);
    commands
        .spawn((
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            Transform::from_xyz(center.x - size.x / 2.0, center.y + size.y / 2.0, z).with_scale(size.extend(1.0)),
            RenderLayers::layer(UI_LAYER),
            DespawnOnExit(crate::Screen::Level),
        ))
        .id()
}

/// Quality of a finished trace: per-dot speed and total time, per `ingame.model.gesture`.
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

#[allow(clippy::too_many_arguments)]
pub fn player_gesture(
    mut commands: Commands,
    time: Res<Time>,
    pointer: Res<Pointer>,
    buttons: Res<ButtonInput<MouseButton>>,
    tuning: Res<Tuning>,
    gestures: Res<Gestures>,
    mut m: ResMut<Match>,
    mut trace: ResMut<Trace>,
    mut assets: ResMut<LayoutAssets>,
    mut transforms: Query<&mut Transform, With<GestureSprite>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut sfx: ResMut<crate::sfx::Sfx>,
) {
    let panel = tuning.gesture_panel[Team::Yellow.index()];
    // Start a new trace when an attack is selected.
    if m.selected != trace.kind {
        for e in trace.sprites.drain(..) {
            commands.entity(e).despawn();
        }
        *trace = Trace { kind: m.selected, ..default() };
        if let Some(kind) = m.selected {
            // The red cancel button sits at the panel's lower right, as in the blueprint layout.
            let cancel = ui_sprite(&mut commands, &mut assets, "attack_cancelBtn.tpl", panel + Vec2::new(85.0, -53.0), Vec2::new(51.0, 50.0), 300.0, &mut meshes, &mut materials, &mut images);
            commands.entity(cancel).insert(GestureSprite);
            trace.sprites.push(cancel);
            let dots: Vec<Vec2> = gestures.0.get(&kind).cloned().unwrap_or_default().into_iter().map(|p| panel + p).collect();
            for (i, d) in dots.iter().enumerate() {
                let s = ui_sprite(&mut commands, &mut assets, "ifcAttackDotB.tpl", *d, Vec2::splat(16.0), 301.0 + i as f32 * 0.01, &mut meshes, &mut materials, &mut images);
                commands.entity(s).insert(GestureSprite);
                trace.sprites.push(s);
            }
            trace.dots = dots;
        }
    }
    let Some(kind) = trace.kind else { return };
    let on_cancel = pointer.pos.is_some_and(|p| p.distance(panel + Vec2::new(85.0, -53.0)) < 25.0);
    if buttons.just_pressed(MouseButton::Right) || (pointer.just_pressed && on_cancel && !trace.started) {
        sfx.play("SFX_ATTACK_IFC_DRAWING_CANCELLED");
        m.selected = None;
        return;
    }
    trace.clock += time.delta_secs();
    let radius = tuning.gesture_dot_radius;
    if let Some(p) = pointer.pos.filter(|_| pointer.pressed) {
        let next = trace.next;
        if next < trace.dots.len() && p.distance(trace.dots[next]) <= radius && (trace.started || next == 0) {
            trace.started = true;
            trace.next += 1;
            sfx.play("SFX_ATTACK_IFC_DOT_HIT");
            let c = trace.clock;
            trace.times.push(c);
        }
    }
    // Reached dots shrink; the next one pulses.
    let pulse = 1.0 + 0.35 * (trace.clock * 8.0).sin().abs();
    for (i, d) in trace.dots.clone().iter().enumerate() {
        if let Ok(mut t) = transforms.get_mut(trace.sprites[i + 1]) {
            let size = if i < trace.next { 8.0 } else if i == trace.next { 16.0 * pulse } else { 16.0 };
            *t = Transform::from_xyz(d.x - size / 2.0, d.y + size / 2.0, t.translation.z).with_scale(Vec3::new(size, size, 1.0));
        }
    }
    let finished = trace.next == trace.dots.len() && !trace.dots.is_empty();
    let released = trace.started && !pointer.pressed;
    if finished || released {
        let q = trace_quality(&trace.times, trace.dots.len(), tuning.gesture_timing);
        info!("gesture {kind:?}: {}/{} dots, quality {:.0}%", trace.next, trace.dots.len(), q * 100.0);
        sfx.play(if !finished {
            "SFX_ATTACK_IFC_DRAWING_INTERRUPTED"
        } else if q >= 0.95 {
            "SFX_ATTACK_IFC_DRAWING_PERFECT"
        } else {
            "SFX_ATTACK_IFC_DRAWING_SUCCESS"
        });
        if kind.is_basic() {
            m.pending = Some((kind, q));
        } else {
            m.pending_special = Some((Team::Yellow, kind, q));
        }
        m.selected = None;
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
    fn slow_or_partial_traces_score_lower() {
        let slow: Vec<f32> = (0..10).map(|i| i as f32 * 0.6).collect();
        let half: Vec<f32> = (0..5).map(|i| i as f32 * 0.15).collect();
        let t = [5.0, 10.0, 0.2, 0.4, 0.05];
        assert!(super::trace_quality(&slow, 10, t) < 0.6);
        assert!(super::trace_quality(&half, 10, t) <= 0.5);
    }
}
