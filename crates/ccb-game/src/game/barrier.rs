//! Ink barriers: lines drawn in front of your chicks that block incoming attacks.
//! Each team has an ink account (`barrierAccountMaxPower`) that refills over time;
//! a barrier stays solid for `fullStrengthTimer` seconds after it's drawn, then fades.

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};

use super::{flow::Match, tuning::Tuning, Team, WorldCamera};
use crate::{
    gx_material::{GxKey, GxMaterial, GxParams},
    pointer::Pointer,
};

#[derive(Component)]
pub struct Barrier {
    pub team: Team,
    pub points: Vec<Vec2>,
    /// Still being drawn (grows, doesn't age).
    pub drawing: bool,
    pub age: f32,
    /// Seconds of hit flash left.
    pub flash: f32,
    mesh: Handle<Mesh>,
    material: Handle<GxMaterial>,
    dirty: bool,
}

impl Barrier {
    pub fn length(&self) -> f32 {
        self.points.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    /// Axis-aligned bounds: (min, max).
    pub fn bounds(&self) -> (Vec2, Vec2) {
        self.points.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)))
    }

    /// The highest point of the line.
    pub fn top(&self) -> Vec2 {
        self.points.iter().copied().fold(Vec2::new(0.0, f32::MIN), |a, p| if p.y > a.y { p } else { a })
    }
}

/// Ink left per team.
#[derive(Resource)]
pub struct Ink(pub [f32; 2]);

pub fn solid_material(color: Vec4) -> GxMaterial {
    let mut p = GxParams::default();
    // Stage 0: color = C1, alpha = A1 * KONST_A (K3_A carries the fade).
    // Color inputs: 4 C1, 15 ZERO. Alpha inputs: 2 A1, 6 KONST, 7 ZERO.
    let cenv = 4 | 15 << 4 | 15 << 8 | 15 << 12 | 1 << 19;
    let aenv = 7 << 4 | 6 << 7 | 2 << 10 | 7 << 13 | 1 << 19;
    p.info = UVec4::new(1, 7 | 7 << 3, 0, 0);
    p.color_env[0].x = cenv;
    p.alpha_env[0].x = aenv;
    p.order[0].x = 0x1f << 16;
    p.regs[2] = color;
    p.konst[3] = Vec4::ONE;
    GxMaterial { params: p, tex0: None, tex1: None, alpha_mode: AlphaMode::Blend, key: GxKey { cull: 0, depth_write: 0, color_write: 1 } }
}

/// Builds a ribbon mesh along the barrier's points.
pub(super) fn ribbon(points: &[Vec2], thickness: f32) -> Mesh {
    let mut pos = Vec::new();
    let mut idx = Vec::new();
    for (i, p) in points.iter().enumerate() {
        let a = points[i.saturating_sub(1)];
        let b = points[(i + 1).min(points.len() - 1)];
        let dir = (b - a).normalize_or(Vec2::X);
        let n = Vec2::new(-dir.y, dir.x) * thickness * 0.5;
        pos.push([p.x + n.x, p.y + n.y, 0.0]);
        pos.push([p.x - n.x, p.y - n.y, 0.0]);
        if i > 0 {
            let k = (i as u32 - 1) * 2;
            idx.extend([k, k + 1, k + 2, k + 1, k + 3, k + 2]);
        }
    }
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_indices(Indices::U32(idx));
    m
}

pub fn team_color(team: Team) -> Vec4 {
    match team {
        Team::Yellow => Vec4::new(1.0, 0.8, 0.0, 1.0),
        Team::Black => Vec4::new(0.12, 0.12, 0.12, 1.0),
    }
}

pub fn spawn_barrier(
    commands: &mut Commands,
    team: Team,
    points: Vec<Vec2>,
    drawing: bool,
    tuning: &Tuning,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<GxMaterial>,
) -> Entity {
    debug!("{team:?} barrier from {:?} to {:?}", points.first(), points.last());
    let mesh = meshes.add(ribbon(&points, tuning.barrier_thickness * 1.5));
    let material = materials.add(solid_material(team_color(team)));
    commands
        .spawn((
            Barrier { team, points, drawing, age: 0.0, flash: 0.0, mesh: mesh.clone(), material: material.clone(), dirty: false },
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::from_xyz(0.0, 0.0, tuning.plane_z + 0.6),
            DespawnOnExit(crate::Screen::Level),
        ))
        .id()
}

/// Projects the pointer onto the gameplay plane.
pub fn pointer_world(pointer: &Pointer, cam: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>, plane_z: f32) -> Option<Vec2> {
    let screen = pointer.screen?;
    let (camera, gt) = cam.single().ok()?;
    let ray = camera.viewport_to_world(gt, screen).ok()?;
    let t = ray.intersect_plane(Vec3::new(0.0, 0.0, plane_z), InfinitePlane3d::new(Vec3::Z))?;
    Some(ray.get_point(t).truncate())
}

/// The player (yellow) draws barriers with the pointer while the other team attacks.
#[allow(clippy::too_many_arguments)]
pub fn player_draw_barrier(
    mut commands: Commands,
    pointer: Res<Pointer>,
    tuning: Res<Tuning>,
    m: Res<Match>,
    mut ink: ResMut<Ink>,
    cam: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    mut barriers: Query<&mut Barrier>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut current: Local<Option<Entity>>,
    mut sfx: ResMut<crate::sfx::Sfx>,
) {
    let team = Team::Yellow;
    let can_draw = m.can_defend(team);
    let world = pointer_world(&pointer, &cam, tuning.plane_z);
    let in_area = world.filter(|w| {
        let (lo, hi) = tuning.side_range(team);
        w.x >= lo - 1.0 && w.x <= hi + tuning.chick_radius && w.y > 0.3 && w.y < tuning.barrier_y_clip
    });
    if !pointer.pressed || !can_draw {
        if let Some(e) = current.take() {
            if let Ok(mut b) = barriers.get_mut(e) {
                b.drawing = false;
            }
        }
        return;
    }
    let Some(p) = in_area else { return };
    match *current {
        None if pointer.just_pressed && ink.0[team.index()] > 0.2 => {
            sfx.play("SFX_GAME_DRAW");
            *current = Some(spawn_barrier(&mut commands, team, vec![p, p], true, &tuning, &mut meshes, &mut materials));
        }
        Some(e) => {
            let Ok(mut b) = barriers.get_mut(e) else {
                *current = None;
                return;
            };
            let last = *b.points.last().unwrap();
            let step = last.distance(p);
            if step >= tuning.barrier_point_distance {
                let ink_left = ink.0[team.index()];
                let room = (tuning.barrier_max_len - b.length()).min(ink_left);
                if room <= 0.01 {
                    b.drawing = false;
                    *current = None;
                    return;
                }
                let p = last + (p - last).clamp_length_max(room);
                ink.0[team.index()] -= last.distance(p);
                if b.points.len() == 2 && b.points[0] == b.points[1] {
                    b.points[1] = p;
                } else {
                    b.points.push(p);
                }
                b.dirty = true;
            }
        }
        _ => {}
    }
}

pub fn update_barriers(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<Tuning>,
    mut ink: ResMut<Ink>,
    mut barriers: Query<(Entity, &mut Barrier)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
) {
    let dt = time.delta_secs();
    let mut drawing = [false; 2];
    for (e, mut b) in &mut barriers {
        if b.drawing {
            drawing[b.team.index()] = true;
        } else {
            b.age += dt;
        }
        b.flash = (b.flash - dt).max(0.0);
        let life = tuning.barrier_full_time + tuning.barrier_fade_time;
        if b.age >= life {
            commands.entity(e).despawn();
            continue;
        }
        if b.dirty {
            if let Some(mut mesh) = meshes.get_mut(&b.mesh) {
                *mesh = ribbon(&b.points, tuning.barrier_thickness * 1.5);
            }
            b.dirty = false;
        }
        let fade = ((life - b.age) / tuning.barrier_fade_time.max(0.01)).clamp(0.0, 1.0);
        // Blink as the barrier nears its end.
        let blink = if b.age > tuning.barrier_full_time * 0.5 { 0.6 + 0.4 * (b.age * 30.0).sin().abs() } else { 1.0 };
        let flash = b.flash > 0.0;
        if let Some(mut m) = materials.get_mut(&b.material) {
            m.params.konst[3].w = fade * blink;
            m.params.regs[2] = if flash { Vec4::ONE } else { team_color(b.team) };
        }
    }
    for team in 0..2 {
        if !drawing[team] {
            ink.0[team] = (ink.0[team] + tuning.ink_reload * dt).min(tuning.ink_max);
        }
    }
}

/// Closest barrier segment of `team` hit by a circle moving from `a` to `b`: returns the
/// contact point, the segment normal (facing `a`) and the barrier entity.
pub fn sweep_hit(
    barriers: &Query<(Entity, &mut Barrier)>,
    team: Team,
    a: Vec2,
    b: Vec2,
    radius: f32,
) -> Option<(Vec2, Vec2, Entity)> {
    let mut best: Option<(f32, Vec2, Vec2, Entity)> = None;
    for (e, bar) in barriers.iter() {
        if bar.team != team || bar.points.len() < 2 {
            continue;
        }
        for w in bar.points.windows(2) {
            let (p, q) = (w[0], w[1]);
            // Sample the motion; barriers are thin and the step is small.
            for k in 0..=4 {
                let t = k as f32 / 4.0;
                let c = a.lerp(b, t);
                let seg = q - p;
                let s = ((c - p).dot(seg) / seg.length_squared().max(1e-6)).clamp(0.0, 1.0);
                let closest = p + seg * s;
                let d = c.distance(closest);
                if d <= radius + 0.1 && best.as_ref().is_none_or(|x| t < x.0) {
                    let mut n = Vec2::new(-seg.y, seg.x).normalize_or(Vec2::Y);
                    if n.dot(a - closest) < 0.0 {
                        n = -n;
                    }
                    best = Some((t, closest, n, e));
                    break;
                }
            }
        }
    }
    best.map(|(_, p, n, e)| (p, n, e))
}

/// Whether a vertical strike at `x` from `top` down to `bottom` passes through a barrier of `team`.
/// Returns the height where it's blocked.
pub fn vertical_block(barriers: &Query<(Entity, &mut Barrier)>, team: Team, x: f32, top: f32, bottom: f32) -> Option<(f32, Entity)> {
    let mut best: Option<(f32, Entity)> = None;
    for (e, bar) in barriers.iter() {
        if bar.team != team {
            continue;
        }
        for w in bar.points.windows(2) {
            let (p, q) = (w[0], w[1]);
            if (p.x - x) * (q.x - x) <= 0.0 && (p.x - q.x).abs() > 1e-4 {
                let y = p.y + (q.y - p.y) * (x - p.x) / (q.x - p.x);
                if y <= top && y >= bottom && best.is_none_or(|b| y > b.0) {
                    best = Some((y, e));
                }
            }
        }
    }
    best
}

/// First barrier of `team` crossed by the segment `a`–`b` (closest to `a`): the crossing point,
/// the index of the barrier segment it crosses and the barrier entity.
pub fn segment_hit(barriers: &Query<(Entity, &mut Barrier)>, team: Team, a: Vec2, b: Vec2) -> Option<(Vec2, usize, Entity)> {
    let mut best: Option<(f32, Vec2, usize, Entity)> = None;
    let r = b - a;
    for (e, bar) in barriers.iter() {
        if bar.team != team {
            continue;
        }
        for (i, w) in bar.points.windows(2).enumerate() {
            let (p, q) = (w[0], w[1]);
            let s = q - p;
            let den = r.perp_dot(s);
            if den.abs() < 1e-6 {
                continue;
            }
            let t = (p - a).perp_dot(s) / den;
            let u = (p - a).perp_dot(r) / den;
            if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) && best.is_none_or(|x| t < x.0) {
                best = Some((t, a + r * t, i, e));
            }
        }
    }
    best.map(|(_, p, i, e)| (p, i, e))
}
