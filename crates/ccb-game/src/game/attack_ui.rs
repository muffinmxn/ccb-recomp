//! The attack interface, built from the original's own layouts:
//! - `attackIfc`: each team's three basic attacks in an arc on its side of the clock, with the
//!   level's special on the arc above them (`slots0` left for yellow, `slots1` right for black).
//! - `blueprints` (one per team, at `gestureDrawingPanelOffs`): the white drawing panel with the
//!   gesture dots, the quality bubble, the cancel button, and once traced the attack on its
//!   platform next to the fire button.
//!
//! The original animates these from code; the pane states below follow its `blueprints_*` anims.

use bevy::prelude::*;

use super::{flow::{Match, Phase}, tuning::Tuning, AttackKind, Team};
use crate::{
    gx_material::GxMaterial,
    layout::{self, LayoutAssets, LayoutPane, LayoutRoot, TextPane, TextSetup},
    pointer::Pointer,
    Screen,
};

/// The three basic attacks in slot order (left, middle, right).
pub const SLOTS: [AttackKind; 3] = [AttackKind::Bomb, AttackKind::Weight, AttackKind::Plant];

/// Where the blueprint panel's buttons sit relative to the panel (yellow; mirrored for black).
pub const CANCEL_POS: Vec2 = Vec2::new(85.0, -53.0);
pub const CONFIRM_POS: Vec2 = Vec2::new(-91.0, 21.0);

/// What a team's side of the interface shows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Mode {
    #[default]
    Idle,
    /// Tracing a gesture: dots done, quality so far.
    Tracing { kind: AttackKind, done: usize, quality: f32 },
    /// Traced; waiting for the trigger.
    Armed { kind: AttackKind, quality: f32 },
}

/// Per-team interface state, written by the player's input and the CPU.
#[derive(Resource, Default)]
pub struct IfcView {
    pub mode: [Mode; 2],
    /// Which team the player controls (None when the CPU plays both).
    pub player: Option<Team>,
}

#[derive(Component)]
pub struct AttackIfc;

#[derive(Component)]
pub struct Blueprint {
    pub team: Team,
    /// Panel openness, 0..1.
    open: f32,
}

fn attack_id(kind: AttackKind) -> u8 {
    kind as u8
}

/// Spawns the attack interface layouts.
pub fn spawn(
    commands: &mut Commands,
    lyt: &mut LayoutAssets,
    tuning: &Tuning,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<GxMaterial>,
    images: &mut Assets<Image>,
) -> anyhow::Result<()> {
    let texts = TextSetup { strings: Default::default(), color: None };
    let ifc = layout::spawn_layout(commands, lyt, "attackIfc", &texts, meshes, materials, images)?;
    commands.entity(ifc).insert((AttackIfc, Transform::from_xyz(0.0, 0.0, 200.0), DespawnOnExit(Screen::Level)));
    for team in [Team::Yellow, Team::Black] {
        let bp = layout::spawn_layout(commands, lyt, "blueprints", &texts, meshes, materials, images)?;
        let at = tuning.gesture_panel[team.index()];
        commands.entity(bp).insert((Blueprint { team, open: 0.0 }, Transform::from_xyz(at.x, at.y, 300.0), DespawnOnExit(Screen::Level)));
    }
    Ok(())
}

fn with_pane(root: &LayoutRoot, panes: &mut Query<&mut LayoutPane>, name: &str, f: impl FnOnce(&mut LayoutPane)) {
    if let Some(mut p) = root.pane(name).and_then(|e| panes.get_mut(e).ok()) {
        f(&mut p);
    }
}

fn show(root: &LayoutRoot, panes: &mut Query<&mut LayoutPane>, name: &str, visible: bool) {
    with_pane(root, panes, name, |p| p.cur.visible = visible);
}

/// Updates both teams' slots and handles the player's clicks on them.
#[allow(clippy::too_many_arguments)]
pub fn update_attack_ifc(
    time: Res<Time<Real>>,
    pointer: Res<Pointer>,
    mut m: ResMut<Match>,
    view: Res<IfcView>,
    roots: Query<&LayoutRoot, With<AttackIfc>>,
    mut panes: Query<&mut LayoutPane>,
    globals: Query<&GlobalTransform>,
    mut texts: Query<&mut TextPane>,
    mut hover: Local<[f32; 4]>,
    mut sfx: ResMut<crate::sfx::Sfx>,
) {
    let Ok(root) = roots.single() else { return };
    let dt = time.delta_secs();
    let t_now = time.elapsed_secs();
    let special_kind = m.specials.first().copied();
    for team in [Team::Yellow, Team::Black] {
        let t = team.index();
        let idle = view.mode[t] == Mode::Idle;
        show(root, &mut panes, &format!("visController{t}"), idle);
        let is_player = view.player == Some(team);
        let my_turn = m.phase == Phase::Attack && m.attacker == team && m.pending.is_none();
        for (i, kind) in SLOTS.iter().enumerate() {
            let id = attack_id(*kind);
            let slot = format!("slot{t}{i}");
            let have = m.available.contains(kind);
            let usable = have && my_turn && idle && m.selected.is_none();
            let slot_at = root.pane(&slot).and_then(|e| panes.get(e).ok()).map(|p| p.cur.translate);
            let over = is_player
                && root.pane(&slot).is_some_and(|e| {
                    let (Ok(p), Ok(g), Some(pt)) = (panes.get(e), globals.get(e), pointer.pos) else { return false };
                    layout::pane_contains(p, g, pt)
                });
            let h = &mut hover[i];
            if over && usable && *h == 0.0 {
                sfx.play("SFX_ATTACK_IFC_SELECTOR_ROLLOVER");
            }
            *h = (*h + if over && usable { 8.0 } else { -8.0 } * dt).clamp(0.0, 1.0);
            let h = *h;
            let alpha = if usable || (!is_player && my_turn) { 1.0 } else { 0.45 };
            for (name, a) in [(format!("attackIcon{t}{id}"), alpha), (format!("glowIcon{t}{id}"), h * 0.8)] {
                with_pane(root, &mut panes, &name, |p| {
                    if let Some(at) = slot_at {
                        p.cur.translate = at;
                    }
                    p.cur.visible = have;
                    p.cur.alpha = a;
                    let s = 1.0 + 0.1 * h;
                    p.cur.scale = Vec2::splat(s) * p.base.scale.signum();
                });
            }
            show(root, &mut panes, &format!("questionmark{t}{i}"), !have);
            if over && usable && pointer.just_pressed {
                info!("player selects {kind:?}");
                sfx.play("SFX_ATTACK_IFC_SELECTOR_ACTIVATED");
                m.selected = Some(*kind);
            }
        }
        // The special arc: inactive until a special window opens, then the level's arc with
        // its icon and the seconds left.
        let open = m.special.filter(|(k, _)| Some(*k) == special_kind);
        show(root, &mut panes, &format!("slotSpecBg{t}"), open.is_some());
        let theme = match special_kind {
            Some(AttackKind::Octopus) => 1,
            Some(AttackKind::Ghost) => 2,
            _ => 0,
        };
        for k in 0..3 {
            show(root, &mut panes, &format!("slotSpecSBg{t}{k}"), k == theme);
            show(root, &mut panes, &format!("slotSpecTimer{t}{k}"), k == theme);
            with_pane(root, &mut panes, &format!("glowSpecSBg{t}{k}"), |p| {
                p.cur.visible = k == theme;
                p.cur.alpha = 0.35 + 0.35 * (t_now * 5.0).sin();
            });
        }
        let sp_hover = is_player
            && open.is_some()
            && root.pane(&format!("bndSlotSpec{t}1")).is_some_and(|e| {
                let (Ok(p), Ok(g), Some(pt)) = (panes.get(e), globals.get(e), pointer.pos) else { return false };
                layout::pane_contains(p, g, pt)
            });
        for id in [2u8, 4, 5, 6] {
            let this = special_kind.is_some_and(|k| attack_id(k) == id);
            for (name, a) in [(format!("attackIcon{t}{id}"), 1.0), (format!("glowIcon{t}{id}"), if sp_hover { 0.7 } else { 0.0 })] {
                with_pane(root, &mut panes, &name, |p| {
                    p.cur.visible = this && open.is_some();
                    p.cur.translate = Vec3::new(2.0, 6.0, p.base.translate.z);
                    p.cur.alpha = a;
                });
            }
        }
        if let Some((kind, left)) = open {
            layout::set_text(root, &format!("txtSpecTimer{t}"), &format!("{}", left.ceil().max(0.0) as i32), &panes, &mut texts);
            if sp_hover && idle && m.selected.is_none() && pointer.just_pressed {
                info!("player races for {kind:?}");
                sfx.play("SFX_ATTACK_IFC_SELECTOR_ACTIVATED");
                m.selected = Some(kind);
            }
        }
    }
}

/// Opens/closes each team's drawing panel and shows the dots, quality and fire button.
#[allow(clippy::too_many_arguments)]
pub fn update_blueprints(
    time: Res<Time<Real>>,
    view: Res<IfcView>,
    gestures: Res<super::gesture::Gestures>,
    mut roots: Query<(&LayoutRoot, &mut Blueprint)>,
    mut panes: Query<&mut LayoutPane>,
    mut texts: Query<&mut TextPane>,
) {
    let dt = time.delta_secs();
    let clock = time.elapsed_secs();
    for (root, mut bp) in &mut roots {
        let t = bp.team.index();
        let mirror = if bp.team == Team::Yellow { 1.0 } else { -1.0 };
        let mode = view.mode[t];
        let tracing = matches!(mode, Mode::Tracing { .. });
        // The panel pops open with a small overshoot (blueprints_gesture_intro: 0 → 1.05 → 1).
        bp.open = (bp.open + if tracing { 3.0 } else { -5.0 } * dt).clamp(0.0, 1.0);
        let o = bp.open;
        let s = if !tracing { o } else if o < 0.45 { o / 0.45 * 1.05 } else { 1.05 - (o - 0.45) / 0.55 * 0.05 };
        with_pane(root, &mut panes, "panel", |p| {
            p.cur.scale = Vec2::splat(s);
            p.cur.visible = s > 0.01;
        });
        // Gesture dots: done ones shrink, the next one pulses.
        let (dots, done): (&[Vec2], usize) = match mode {
            Mode::Tracing { kind, done, .. } => (gestures.0.get(&kind).map_or(&[][..], |v| &v[..]), done),
            _ => (&[], 0),
        };
        show(root, &mut panes, "dots", tracing && o > 0.6);
        show(root, &mut panes, "dot", false);
        for i in 0..20 {
            with_pane(root, &mut panes, &format!("dot_{i:02}"), |p| {
                p.cur.visible = i < dots.len();
                if let Some(d) = dots.get(i) {
                    p.cur.translate = d.extend(p.base.translate.z);
                    let pulse = 1.0 + 0.35 * (clock * 8.0).sin().abs();
                    p.cur.scale = Vec2::splat(if i < done { 0.55 } else if i == done { pulse } else { 1.0 });
                }
            });
        }
        let quality = match mode {
            Mode::Tracing { quality, .. } | Mode::Armed { quality, .. } => Some(quality),
            Mode::Idle => None,
        };
        with_pane(root, &mut panes, "qualityPanel", |p| {
            p.cur.visible = quality.is_some() && (!tracing || o > 0.5);
            p.cur.translate = Vec3::new(-96.0 * mirror, -51.0, p.base.translate.z);
            p.cur.alpha = 1.0;
        });
        if let Some(q) = quality {
            layout::set_text(root, "tbQuality0", &format!("{}", (q * 100.0).round() as i32), &panes, &mut texts);
        }
        with_pane(root, &mut panes, "cancelButton", |p| {
            p.cur.visible = tracing && o > 0.3 && view.player == Some(bp.team);
            p.cur.translate = Vec3::new(CANCEL_POS.x * mirror, CANCEL_POS.y, p.base.translate.z);
        });
        show(root, &mut panes, "slotSpecTimer", false);
        // Traced: the attack on its platform, with the fire button.
        let armed = match mode {
            Mode::Armed { kind, .. } => Some(kind),
            _ => None,
        };
        with_pane(root, &mut panes, "upgradePanel", |p| {
            p.cur.visible = armed.is_some();
            p.cur.translate = Vec3::new(0.0, 0.0, p.base.translate.z);
        });
        with_pane(root, &mut panes, "confirmButton", |p| {
            p.cur.translate = Vec3::new(CONFIRM_POS.x * mirror, CONFIRM_POS.y, p.base.translate.z);
            p.cur.scale = Vec2::splat(1.0 + 0.06 * (clock * 6.0).sin());
        });
        for name in ["upgradeTarget0Bg", "upgradeTarget1Bg", "upgradeTarget0", "upgradeTarget1", "confirmHintArrow"] {
            show(root, &mut panes, name, false);
        }
        // The attack's big graphic on the platform (`attackIcon<id><variant>`; variants 1/2
        // are the upgrades).
        for id in 0..7u8 {
            for v in 0..3 {
                let on = armed.is_some_and(|k| attack_id(k) == id) && v == 0;
                with_pane(root, &mut panes, &format!("attackIcon{id}{v}"), |p| {
                    p.cur.visible = on;
                    p.cur.scale = p.base.scale * Vec2::new(mirror, 1.0);
                });
            }
        }
    }
}
