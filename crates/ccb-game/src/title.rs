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
        app.add_systems(OnEnter(Screen::Title), spawn_title);
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
        let root = commands.spawn((Name::new("title backdrop"), Transform::default(), Visibility::default())).id();
        level::spawn_sky(&mut commands, root, &common, &textures, &mut meshes, &mut materials, &mut images)?;
        level::spawn_camera(&mut commands, &data, true)?;

        let msg = |k: &str| layout::display_text(data.msgs.get(k).unwrap_or(k));
        let strings: HashMap<&str, String> = [
            ("txtSel0", msg("CCB_MENU_MAIN_STORY")),
            ("txtSel1", msg("CCB_MENU_MAIN_BATTLE")),
            ("txtSel2", msg("CCB_MENU_CHICKEN_HOUSE_TITLE")),
            ("txtSel3", msg("CCB_MENU_MAIN_CREDITS")),
        ]
        .into_iter()
        .collect();
        let texts = layout::TextSetup { strings, color: Some([40, 40, 40, 255]) };
        let menu = layout::spawn_layout(&mut commands, &mut lyt, "menu", &texts, &mut meshes, &mut materials, &mut images)?;
        let mut animator = LayoutAnimator::default();
        for a in ["menu_logoIntro", "menu_introMenu", "menu_platformIntro"] {
            animator.play(lyt.animation(a)?);
        }
        commands.entity(menu).insert(animator);
        commands.entity(menu).insert(HideOnSpawn(vec!["PROFILES", "chickClosedY", "chickAngryY", "chickClosedB", "chickAngryB", "logoBoomTrans"]));

        let music = data.read("sounds/menu_bg.ogg")?;
        commands.spawn((AudioPlayer::new(audio.add(AudioSource { bytes: music.into() })), PlaybackSettings::LOOP));
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
