//! The player's drawing cursor, from the original's `cursors` layout textures: over the
//! drawing area the hand becomes a paint point (`cursorPaintY`) with the ink ring floating
//! above it: the red `CursorNoInk` ring, covered by the full-ink ring (`CursorAllInkMask`) for the
//! share of ink left.

use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::RenderLayers,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};

use super::{
    barrier::{pointer_world, Ink},
    flow::Match,
    tuning::Tuning,
    Team, WorldCamera,
};
use crate::{
    gx_material::GxMaterial,
    layout::{LayoutAssets, UI_LAYER},
    pointer::Pointer,
    Screen,
};

/// Ring center relative to the paint point, and sizes, in layout units (from `cursors`).
const RING_OFFSET: Vec2 = Vec2::new(3.0, 49.0);
const RING_SIZE: f32 = 50.0;
const POINT_SIZE: f32 = 20.0;

#[derive(Component)]
pub enum CursorPart {
    Point,
    Ring,
    Ink,
}

/// The ink share the ring mesh currently shows (in 1/64 steps).
#[derive(Resource, Default)]
pub struct CursorInk(Option<u32>);

pub fn spawn_cursor(
    mut commands: Commands,
    mut lyt: ResMut<LayoutAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    for (part, tex, size, z) in [
        (CursorPart::Ring, "CursorNoInk.tpl", RING_SIZE, 401.0),
        (CursorPart::Ink, "CursorAllInkMask.tpl", RING_SIZE, 402.0),
        (CursorPart::Point, "cursorPaintY.tpl", POINT_SIZE, 403.0),
    ] {
        let (mesh, mat) = lyt.sprite(tex, &mut meshes, &mut materials, &mut images);
        commands.spawn((
            part,
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            Transform::from_xyz(0.0, 0.0, z).with_scale(Vec3::new(size, size, 1.0)),
            Visibility::Hidden,
            RenderLayers::layer(UI_LAYER),
            DespawnOnExit(Screen::Level),
        ));
    }
    commands.insert_resource(CursorInk::default());
}

/// A pie slice of the unit quad (x 0..1, y 0..-1) covering `share` of a full turn,
/// clockwise from 12 o'clock, UV-mapped like the quad.
fn pie(share: f32) -> Mesh {
    let n = 48;
    let steps = ((share.clamp(0.0, 1.0) * n as f32).round() as usize).max(1);
    let mut pos = vec![[0.5, -0.5, 0.0]];
    for k in 0..=steps {
        let a = std::f32::consts::FRAC_PI_2 - k as f32 / n as f32 * std::f32::consts::TAU;
        // Out to the quad's corners so the whole ring texture is covered.
        let d = 0.71;
        pos.push([0.5 + d * a.cos(), -0.5 + d * a.sin(), 0.0]);
    }
    let uvs: Vec<[f32; 2]> = pos.iter().map(|p| [p[0], -p[1]]).collect();
    let colors = vec![[1.0f32; 4]; pos.len()];
    let mut idx = Vec::new();
    for k in 1..=steps as u32 {
        idx.extend([0, k + 1, k]);
    }
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    m.insert_indices(Indices::U32(idx));
    m
}

#[allow(clippy::too_many_arguments)]
pub fn update_cursor(
    mut commands: Commands,
    mut pointer: ResMut<Pointer>,
    tuning: Res<Tuning>,
    ink: Res<Ink>,
    m: Res<Match>,
    view: Res<super::attack_ui::IfcView>,
    mut shown: ResMut<CursorInk>,
    cam: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    mut parts: Query<(Entity, &CursorPart, &mut Transform, &mut Visibility)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let team = Team::Yellow;
    // Over the drawing area (and not tracing a gesture) the hand turns into the paint cursor.
    let tracing = view.mode[team.index()] != super::attack_ui::Mode::Idle;
    let over_field = pointer_world(&pointer, &cam, tuning.plane_z).is_some_and(|w| {
        let (lo, hi) = tuning.side_range(team);
        w.x >= lo - 1.0 && w.x <= hi + tuning.chick_radius && w.y > 0.3 && w.y < tuning.barrier_y_clip
    });
    let paint = view.player == Some(team) && over_field && !tracing && !matches!(m.phase, super::flow::Phase::GameOver(_));
    pointer.paint = paint;
    let share = ink.0[team.index()] / tuning.ink_max.max(1e-3);
    let step = (share * 64.0).round() as u32;
    let p = pointer.pos.unwrap_or_default();
    for (e, part, mut t, mut v) in &mut parts {
        *v = if paint { Visibility::Visible } else { Visibility::Hidden };
        let (center, size) = match part {
            CursorPart::Point => (p, POINT_SIZE),
            CursorPart::Ring | CursorPart::Ink => (p + RING_OFFSET, RING_SIZE),
        };
        t.translation.x = center.x - size / 2.0;
        t.translation.y = center.y + size / 2.0;
        if matches!(part, CursorPart::Ink) && shown.0 != Some(step) {
            if step == 0 {
                *v = Visibility::Hidden;
            } else {
                commands.entity(e).insert(Mesh3d(meshes.add(pie(step as f32 / 64.0))));
            }
        }
        if matches!(part, CursorPart::Ink) && step == 0 {
            *v = Visibility::Hidden;
        }
    }
    shown.0 = Some(step);
}

pub fn reset_pointer(mut pointer: ResMut<Pointer>) {
    pointer.paint = false;
}

/// When an enemy plant grows on the player's side, pointing near its head rings it with the
/// `plantCircle` model: that's where to start a line for it to climb.
#[allow(clippy::too_many_arguments)]
pub fn plant_hint(
    mut commands: Commands,
    pointer: Res<Pointer>,
    tuning: Res<Tuning>,
    time: Res<Time>,
    assets: Res<super::flow::GameAssets>,
    attacks: Query<&super::attack::Attack>,
    cam: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    mut hint: Local<Option<Entity>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let team = Team::Yellow;
    let world = pointer_world(&pointer, &cam, tuning.plane_z);
    let head = attacks
        .iter()
        .find(|a| a.kind == super::AttackKind::Plant && a.target() == team && a.state == super::attack::AttackState::Travel)
        .map(|a| a.pos);
    let radius = std::env::var("CCB_HINT_RADIUS").ok().and_then(|v| v.parse().ok()).unwrap_or(HINT_RADIUS);
    let near = matches!((head, world), (Some(h), Some(w)) if h.distance(w) < radius);
    if hint.is_some_and(|e| commands.get_entity(e).is_err()) {
        *hint = None;
    }
    match (near, *hint) {
        (true, None) => {
            let e = assets.spawn(&mut commands, "plantCircle", team, HINT_SIZE, &mut meshes, &mut materials, &mut images);
            commands.entity(e).insert(DespawnOnExit(Screen::Level));
            *hint = Some(e);
        }
        (false, Some(e)) => {
            commands.entity(e).despawn();
            *hint = None;
        }
        _ => {}
    }
    if let (Some(e), Some(h)) = (*hint, head) {
        let pulse = 1.0 + 0.08 * (time.elapsed_secs() * 8.0).sin();
        commands.entity(e).insert(Transform::from_xyz(h.x, h.y + 0.4, tuning.plane_z + 0.75).with_scale(Vec3::splat(assets.scale("plantCircle", HINT_SIZE) * pulse)));
    }
}

/// How close the pointer must be to the plant's head, and the ring's size (world units).
const HINT_RADIUS: f32 = 2.0;
const HINT_SIZE: f32 = 2.4;
