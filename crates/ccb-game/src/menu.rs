//! Clickable buttons on layouts: hover growth (`buttonRolloverSizeInc` from
//! `menu.model.general`), click detection against bounding panes, and menu sounds.
//! Screens read `MenuButtons::clicked` to act on a choice.

use bevy::prelude::*;

use crate::{
    layout::{self, LayoutPane, LayoutRoot},
    pointer::Pointer,
    sfx::Sfx,
};

#[derive(Clone, Debug)]
pub struct Button {
    /// Pane that grows on hover.
    pub pane: &'static str,
    /// Panes whose rectangles are clickable (usually `bnd1` bounding panes).
    pub hit: Vec<&'static str>,
    pub id: u32,
}

#[derive(Component)]
pub struct MenuButtons {
    pub buttons: Vec<Button>,
    pub rollover_time: f32,
    pub rollover_inc: f32,
    hover: Vec<f32>,
    /// Set for one frame when a button is clicked.
    pub clicked: Option<u32>,
    pub enabled: bool,
}

impl MenuButtons {
    /// Rollover timing and growth from `menu.model.general`.
    pub fn with_rollover(mut self, time: f32, inc: f32) -> Self {
        self.rollover_time = time;
        self.rollover_inc = inc;
        self
    }

    pub fn new(buttons: Vec<Button>) -> Self {
        let n = buttons.len();
        Self { buttons, rollover_time: 0.1, rollover_inc: 0.08, hover: vec![0.0; n], clicked: None, enabled: true }
    }
}

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, update_buttons);
    }
}

fn update_buttons(
    time: Res<Time<Real>>,
    pointer: Res<Pointer>,
    mut sfx: ResMut<Sfx>,
    mut menus: Query<(&LayoutRoot, &mut MenuButtons)>,
    mut panes: Query<(&mut LayoutPane, &GlobalTransform, &InheritedVisibility)>,
) {
    for (root, mut menu) in &mut menus {
        menu.clicked = None;
        if !menu.enabled {
            continue;
        }
        let step = time.delta_secs() / menu.rollover_time.max(1e-3);
        for i in 0..menu.buttons.len() {
            let b = menu.buttons[i].clone();
            let hovered = pointer.pos.is_some_and(|p| {
                b.hit.iter().filter_map(|n| root.pane(n)).any(|e| {
                    panes.get(e).is_ok_and(|(pane, gt, vis)| vis.get() && layout::pane_contains(&pane, gt, p))
                })
            });
            if hovered && menu.hover[i] == 0.0 {
                sfx.play("SFX_MENU_BTN_ROLLOVER");
            }
            let h = (menu.hover[i] + if hovered { step } else { -step }).clamp(0.0, 1.0);
            menu.hover[i] = h;
            if let Some(e) = root.pane(b.pane) {
                if let Ok((mut pane, _, _)) = panes.get_mut(e) {
                    pane.scale_mul = 1.0 + menu.rollover_inc * h;
                }
            }
            if hovered && pointer.just_pressed {
                sfx.play("SFX_MENU_BTN_CLICK");
                menu.clicked = Some(b.id);
            }
        }
    }
}
