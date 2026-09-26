//! Title screen: the sky backdrop with the `menu` layout (logo, chicks, main buttons).

use std::collections::HashMap;

use anyhow::Result;
use bevy::{audio::AudioSource, prelude::*};
use wii_formats::brres::Brres;

use crate::{
    data::GameData,
    g3d,
    gx_material::GxMaterial,
    layout::{self, LayoutAnimator, LayoutAssets, LayoutRoot},
    level, Screen,
};

pub struct TitlePlugin;

impl Plugin for TitlePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(Screen::Title), spawn_title).add_systems(Update, menu_buttons.run_if(in_state(Screen::Title)));
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_title(
    mut commands: Commands,
    mut data: ResMut<GameData>,
    mut lyt: ResMut<LayoutAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut audio: ResMut<Assets<AudioSource>>,
) {
    let r: Result<()> = (|| {
        let common_bytes = data.brres("common.brres.LZ")?.to_vec();
        let common = Brres::parse(&common_bytes)?;
        let textures = g3d::load_textures(&[&common], &mut images);
        let root = commands
            .spawn((Name::new("title backdrop"), Transform::default(), Visibility::default(), DespawnOnExit(Screen::Title)))
            .id();
        level::spawn_sky(&mut commands, root, &common, &textures, &mut meshes, &mut materials, &mut images)?;
        let cam = level::spawn_camera(&mut commands, &data, true)?;
        commands.entity(cam).insert(DespawnOnExit(Screen::Title));

        let msg = |k: &str| layout::display_text(data.msgs.get(k).unwrap_or(k));
        let strings: HashMap<&str, String> = [
            ("txtSel0", msg("CCB_MENU_MAIN_STORY")),
            ("txtSel1", msg("CCB_MENU_MAIN_BATTLE")),
            ("txtSel2", msg("CCB_MENU_CHICKEN_HOUSE_TITLE")),
            ("txtSel3", msg("CCB_MENU_MAIN_CREDITS")),
        ]
        .into_iter()
        .collect();
        let texts = layout::TextSetup { strings, color: Some([0, 0, 0, 255]) };
        let menu = layout::spawn_layout(&mut commands, &mut lyt, "menu", &texts, &mut meshes, &mut materials, &mut images)?;
        let mut animator = LayoutAnimator::default();
        for a in ["menu_logoIntro", "menu_introMenu", "menu_platformIntro"] {
            animator.play(lyt.animation(a)?);
        }
        let general = data.cfg.block("menu.model.general")?;
        commands.entity(menu).insert((
            animator,
            DespawnOnExit(Screen::Title),
            MenuButtons {
                // (button pane, bounding panes, action)
                buttons: vec![
                    ("button0", vec!["selBound0"], MenuAction::Tutorial),
                    ("button1", vec!["selBound1a", "selBound1b"], MenuAction::Play),
                    ("button2", vec!["selBound2"], MenuAction::ChickenCoop),
                    ("button3", vec!["selBound3"], MenuAction::Credits),
                ],
                rollover_time: general.f32("buttonRolloverAnimTime").unwrap_or(0.1),
                rollover_inc: general.f32("buttonRolloverSizeInc").unwrap_or(0.08),
                hover: vec![0.0; 4],
            },
        ));
        commands.entity(menu).insert(HideOnSpawn(vec!["PROFILES", "chickClosedY", "chickAngryY", "chickClosedB", "chickAngryB", "logoBoomTrans"]));

        let music = data.read("sounds/menu_bg.ogg")?;
        commands.spawn((
            AudioPlayer::new(audio.add(AudioSource { bytes: music.into() })),
            PlaybackSettings::LOOP,
            DespawnOnExit(Screen::Title),
        ));
        Ok(())
    })();
    if let Err(e) = r {
        error!("failed to build title screen: {e:#}");
    }
}

/// Panes the game hides until they're needed (profile list, alternate chick faces).
#[derive(Component)]
pub struct HideOnSpawn(pub Vec<&'static str>);

pub fn hide_panes(
    mut commands: Commands,
    roots: Query<(Entity, &LayoutRoot, &HideOnSpawn)>,
    mut panes: Query<&mut layout::LayoutPane>,
) {
    for (e, root, hide) in &roots {
        for name in &hide.0 {
            if let Some(p) = root.pane(name) {
                if let Ok(mut pane) = panes.get_mut(p) {
                    pane.cur.visible = false;
                    pane.base.visible = false;
                }
            }
        }
        commands.entity(e).remove::<HideOnSpawn>();
    }
}

#[derive(Clone, Copy, Debug)]
pub enum MenuAction {
    Tutorial,
    Play,
    ChickenCoop,
    Credits,
}

/// Main-menu buttons: rollover growth (from `menu.model.general`) and click actions.
#[derive(Component)]
pub struct MenuButtons {
    pub buttons: Vec<(&'static str, Vec<&'static str>, MenuAction)>,
    pub rollover_time: f32,
    pub rollover_inc: f32,
    /// 0..1 rollover progress per button.
    pub hover: Vec<f32>,
}

fn menu_buttons(
    time: Res<Time>,
    pointer: Res<crate::pointer::Pointer>,
    mut menus: Query<(&LayoutRoot, &mut MenuButtons)>,
    mut panes: Query<(&mut layout::LayoutPane, &GlobalTransform, &InheritedVisibility)>,
    mut next: ResMut<NextState<Screen>>,
    mut opts: ResMut<crate::Options>,
    mut sfx: ResMut<crate::sfx::Sfx>,
) {
    for (root, mut menu) in &mut menus {
        let step = time.delta_secs() / menu.rollover_time.max(1e-3);
        for i in 0..menu.buttons.len() {
            let (button, bounds, action) = menu.buttons[i].clone();
            let hovered = pointer.pos.is_some_and(|p| {
                bounds.iter().filter_map(|b| root.pane(b)).any(|e| {
                    panes.get(e).is_ok_and(|(pane, gt, vis)| vis.get() && layout::pane_contains(&pane, gt, p))
                })
            });
            if hovered && menu.hover[i] == 0.0 {
                sfx.play("SFX_MENU_BTN_ROLLOVER");
            }
            let h = (menu.hover[i] + if hovered { step } else { -step }).clamp(0.0, 1.0);
            menu.hover[i] = h;
            let mul = 1.0 + menu.rollover_inc * h;
            if let Some(e) = root.pane(button) {
                if let Ok((mut pane, _, _)) = panes.get_mut(e) {
                    pane.scale_mul = mul;
                }
            }
            if hovered && pointer.just_pressed {
                info!("menu: {action:?}");
                sfx.play("SFX_MENU_BTN_CLICK");
                match action {
                    MenuAction::Play => {
                        opts.level = Some("city.1".into());
                        next.set(Screen::Level);
                    }
                    MenuAction::Tutorial => {
                        opts.level = Some("story.1".into());
                        next.set(Screen::Level);
                    }
                    MenuAction::ChickenCoop | MenuAction::Credits => warn!("{action:?} screen is not implemented yet"),
                }
            }
        }
    }
}
