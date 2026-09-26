//! In-game HUD on top of the `hud` layout: health bars, rounds, team icons, clock state.
//!
//! The layout file holds placeholder state that the original game code overwrites at
//! runtime; this module does the same.

use bevy::{camera::visibility::RenderLayers, prelude::*};

use crate::{
    gx_material::GxMaterial,
    layout::{LayoutAssets, LayoutPane, LayoutRoot, UI_LAYER},
};

/// Game state the HUD shows.
#[derive(Component, Clone, Debug)]
pub struct Hud {
    /// Team health, 0..1 (yellow, black).
    pub health: [f32; 2],
    /// Rounds needed to win and rounds won per team.
    pub rounds_to_win: usize,
    pub rounds_won: [usize; 2],
    initialized: bool,
}

impl Default for Hud {
    fn default() -> Self {
        Self { health: [1.0, 1.0], rounds_to_win: 1, rounds_won: [0, 0], initialized: false }
    }
}

/// Width of the health bar fill (`_hb0` / `_hb1`), in layout units.
const BAR_WIDTH: f32 = 209.0;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, update_hud);
    }
}

#[allow(clippy::too_many_arguments)]
fn update_hud(
    mut commands: Commands,
    mut huds: Query<(&LayoutRoot, &mut Hud)>,
    mut panes: Query<&mut LayoutPane>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut assets: ResMut<LayoutAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
) {
    for (root, mut hud) in &mut huds {
        let mut set = |name: &str, f: &mut dyn FnMut(&mut LayoutPane)| {
            if let Some(mut p) = root.pane(name).and_then(|e| panes.get_mut(e).ok()) {
                f(&mut p);
            }
        };
        if !hud.initialized {
            hud.initialized = true;
            // Kill counters are only used in some modes; the flash layer over the clock
            // is animated in when time runs out.
            for name in ["killCounterPane0", "killCounterPane1"] {
                set(name, &mut |p| p.cur.visible = false);
            }
            for name in ["clockGlowBig", "clockGlow"] {
                set(name, &mut |p| p.cur.alpha = 0.0);
            }
            // Team icons sit in the empty `chickPos` panes.
            for (pane, tex) in [("chickPos0", "battles_chickY.tpl"), ("chickPos1", "battles_chickB.tpl")] {
                if let Some(parent) = root.pane(pane) {
                    let (mesh, mat) = assets.sprite(tex, &mut meshes, &mut materials, &mut images);
                    let icon = commands
                        .spawn((
                            Mesh3d(mesh),
                            MeshMaterial3d(mat),
                            Transform::from_xyz(-19.0, 19.0, 20.0).with_scale(Vec3::new(38.0, 38.0, 1.0)),
                            RenderLayers::layer(UI_LAYER),
                        ))
                        .id();
                    commands.entity(parent).add_child(icon);
                }
            }
        }
        // Stars: one per round needed; won rounds are lit, the rest dimmed.
        for team in 0..2 {
            for i in 0..5 {
                let name = format!("star{team}{i}");
                let visible = i < hud.rounds_to_win;
                set(&name, &mut |p| p.cur.visible = visible);
                let lit = i < hud.rounds_won[team];
                for h in root.pane_materials(&name) {
                    if let Some(mut m) = materials.get_mut(&h) {
                        let c = if lit { 1.0 } else { 0.3 };
                        if m.params.regs[2].x != c {
                            m.params.regs[2] = Vec4::new(c, c, c, 1.0);
                        }
                    }
                }
            }
        }
        // Health: the fill slides out of the bar while its texture scrolls the other
        // way, so the visible part always starts at the bar's inner edge.
        for team in 0..2 {
            let missing = 1.0 - hud.health[team].clamp(0.0, 1.0);
            let dir = if team == 0 { -1.0 } else { 1.0 };
            let name = format!("_hb{team}");
            set(&name, &mut |p| p.cur.translate.x = dir * missing * BAR_WIDTH);
            for h in root.pane_materials(&name) {
                if let Some(mut m) = materials.get_mut(&h) {
                    // The texture scrolls with the pane so its start stays on the bar's inner edge.
                    let t = dir * missing;
                    if (m.params.tex_mtx[0].z - t).abs() > 1e-4 {
                        m.params.set_tex_srt([t, 0.0, 0.0, 1.0, 1.0]);
                    }
                }
            }
        }
    }
}
