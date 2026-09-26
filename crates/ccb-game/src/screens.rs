//! Screens around a match: arena selection, the pause menu and the game-over banner.

use std::collections::HashMap;

use anyhow::Result;
use bevy::{audio::AudioSource, prelude::*};
use wii_formats::brres::Brres;

use crate::{
    data::GameData,
    g3d,
    game::flow::{Match, Phase},
    game::Team,
    gx_material::GxMaterial,
    layout::{self, LayoutAnimator, LayoutAssets},
    level,
    menu::{Button, MenuButtons},
    Screen,
};

pub struct ScreensPlugin;

impl Plugin for ScreensPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(Screen::Arenas), spawn_arenas)
            .add_systems(Update, arenas_actions.run_if(in_state(Screen::Arenas)))
            .add_systems(Update, (toggle_pause, pause_actions, game_over_banner).run_if(in_state(Screen::Level)))
            .add_systems(OnExit(Screen::Level), unpause);
    }
}

fn msg(data: &GameData, key: &str, fallback: &str) -> String {
    layout::display_text(data.msgs.get(key).unwrap_or(fallback))
}

// ------------------------------------------------------------------------------ arenas

/// Arena cards: (pane, level to load, name key, special-attack name key).
const ARENAS: [(&str, &str, &str, &str); 3] = [
    ("arena0", "city.1", "CCB_LEVEL_CITY_GLOBAL", "CCB_POPUP_ATTACK_01_TITLE"),
    ("arena1", "ship.1", "CCB_LEVEL_SHIP_GLOBAL", "CCB_POPUP_ATTACK_02_TITLE"),
    ("arena2", "graveyard.1", "CCB_LEVEL_GRAVEYARD_GLOBAL", "CCB_POPUP_ATTACK_03_TITLE"),
];
const BACK: u32 = 100;

#[allow(clippy::too_many_arguments)]
fn spawn_arenas(
    mut commands: Commands,
    mut data: ResMut<GameData>,
    mut lyt: ResMut<LayoutAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut audio: ResMut<Assets<AudioSource>>,
) {
    let r: Result<()> = (|| {
        // Same sky backdrop as the title.
        let common_bytes = data.brres("common.brres.LZ")?.to_vec();
        let common = Brres::parse(&common_bytes)?;
        let textures = g3d::load_textures(&[&common], &mut images);
        let root = commands.spawn((Transform::default(), Visibility::default(), DespawnOnExit(Screen::Arenas))).id();
        level::spawn_sky(&mut commands, root, &common, &textures, &mut meshes, &mut materials, &mut images)?;
        let cam = level::spawn_camera(&mut commands, &data, true)?;
        commands.entity(cam).insert(DespawnOnExit(Screen::Arenas));

        let mut strings: HashMap<&str, String> = [
            ("txtArenasTitle", msg(&data, "CCB_MENU_ARENAS_TITLE", "ARENAS")),
            ("txtBack", msg(&data, "CCB_MENU_BACK", "BACK")),
        ]
        .into_iter()
        .collect();
        for (i, (_, _, name, special)) in ARENAS.iter().enumerate() {
            strings.insert(["txtArena0", "txtArena1", "txtArena2"][i], msg(&data, name, ""));
            strings.insert(["txtAttack0", "txtAttack1", "txtAttack2"][i], msg(&data, special, ""));
        }
        let texts = layout::TextSetup { strings, color: Some([0, 0, 0, 255]) };
        let menu = layout::spawn_layout(&mut commands, &mut lyt, "arenas", &texts, &mut meshes, &mut materials, &mut images)?;
        let mut anim = LayoutAnimator::default();
        anim.play(lyt.animation("arenas_intro")?);
        let mut buttons: Vec<Button> =
            ARENAS.iter().enumerate().map(|(i, (pane, _, _, _))| Button { pane, hit: vec![pane], id: i as u32 }).collect();
        buttons.push(Button { pane: "btnBack", hit: vec!["bndBack"], id: BACK });
        commands.entity(menu).insert((anim, MenuButtons::new(buttons), DespawnOnExit(Screen::Arenas), ArenaMenu));
        let music = data.read("sounds/menu_bg.ogg")?;
        commands.spawn((AudioPlayer::new(audio.add(AudioSource { bytes: music.into() })), PlaybackSettings::LOOP, DespawnOnExit(Screen::Arenas)));
        Ok(())
    })();
    if let Err(e) = r {
        error!("failed to build the arena screen: {e:#}");
    }
}

#[derive(Component)]
struct ArenaMenu;

#[allow(clippy::too_many_arguments)]
fn arenas_actions(
    menus: Query<&MenuButtons, With<ArenaMenu>>,
    mut next: ResMut<NextState<Screen>>,
    mut opts: ResMut<crate::Options>,
) {
    let Ok(menu) = menus.single() else { return };
    match menu.clicked {
        Some(BACK) => next.set(Screen::Title),
        Some(i) => {
            if let Some((_, level, _, _)) = ARENAS.get(i as usize) {
                opts.level = Some(level.to_string());
                next.set(Screen::Level);
            }
        }
        None => {}
    }
}

// ------------------------------------------------------------------------------ pause

#[derive(Component)]
struct PauseMenu;

const CONTINUE: u32 = 0;
const EXIT: u32 = 1;

#[allow(clippy::too_many_arguments)]
fn toggle_pause(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    data: Res<GameData>,
    mut lyt: ResMut<LayoutAssets>,
    mut time: ResMut<Time<Virtual>>,
    existing: Query<Entity, With<PauseMenu>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    if let Ok(e) = existing.single() {
        commands.entity(e).despawn();
        time.unpause();
        return;
    }
    let strings: HashMap<&str, String> = [
        ("txtTitle", msg(&data, "CCB_INGAME_MENU_PAUSE", "PAUSE")),
        ("txtContinue", msg(&data, "CCB_INGAME_MENU_CONTINUE", "CONTINUE")),
        ("txtExit", msg(&data, "CCB_INGAME_MENU_EXIT", "EXIT GAME")),
    ]
    .into_iter()
    .collect();
    let texts = layout::TextSetup { strings, color: Some([0, 0, 0, 255]) };
    match layout::spawn_layout(&mut commands, &mut lyt, "ingame_pause", &texts, &mut meshes, &mut materials, &mut images) {
        Ok(menu) => {
            let mut anim = LayoutAnimator::default();
            for a in ["ingame_pause_intro", "ingame_pause_continuous"] {
                if let Ok(a) = lyt.animation(a) {
                    anim.play(a);
                }
            }
            commands.entity(menu).insert((
                PauseMenu,
                anim,
                // Above the HUD.
                Transform::from_xyz(0.0, 0.0, 200.0),
                MenuButtons::new(vec![
                    Button { pane: "btnContinue", hit: vec!["bndBack_01"], id: CONTINUE },
                    Button { pane: "btnExit", hit: vec!["bndBack_02"], id: EXIT },
                ]),
                crate::title::HideOnSpawn(vec!["request"]),
                DespawnOnExit(Screen::Level),
            ));
            time.pause();
        }
        Err(e) => error!("pause menu: {e:#}"),
    }
}

fn pause_actions(
    mut commands: Commands,
    menus: Query<(Entity, &MenuButtons), With<PauseMenu>>,
    mut time: ResMut<Time<Virtual>>,
    mut next: ResMut<NextState<Screen>>,
) {
    for (e, menu) in &menus {
        match menu.clicked {
            Some(CONTINUE) => {
                commands.entity(e).despawn();
                time.unpause();
            }
            Some(EXIT) => next.set(Screen::Title),
            _ => {}
        }
    }
}

fn unpause(mut time: ResMut<Time<Virtual>>) {
    time.unpause();
}

// ------------------------------------------------------------------------------ game over

#[derive(Component)]
struct GameOverBanner;

#[allow(clippy::too_many_arguments)]
fn game_over_banner(
    mut commands: Commands,
    m: Option<Res<Match>>,
    data: Res<GameData>,
    mut lyt: ResMut<LayoutAssets>,
    existing: Query<Entity, With<GameOverBanner>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(m) = m else { return };
    let (text, final_round) = match (m.phase, m.winner, m.round_winner) {
        (Phase::GameOver(_), Some(w), _) => (if w == Team::Yellow { "YELLOW WINS!" } else { "BLACK WINS!" }, true),
        (Phase::RoundOver(_), _, Some(w)) => (if w == Team::Yellow { "ROUND TO YELLOW" } else { "ROUND TO BLACK" }, false),
        _ => {
            // Clear a finished round's banner.
            for e in &existing {
                commands.entity(e).despawn();
            }
            return;
        }
    };
    if !existing.is_empty() {
        return;
    }
    let _ = final_round;
    let _ = &data;
    let strings: HashMap<&str, String> = [("txtMessage", text.to_string()), ("txtBackground", text.to_string())].into_iter().collect();
    let texts = layout::TextSetup { strings, color: None };
    if let Ok(banner) = layout::spawn_layout(&mut commands, &mut lyt, "gameover", &texts, &mut meshes, &mut materials, &mut images) {
        let mut anim = LayoutAnimator::default();
        if let Ok(a) = lyt.animation("gameover_introAnim") {
            anim.play(a);
        }
        commands.entity(banner).insert((GameOverBanner, anim, Transform::from_xyz(0.0, 0.0, 150.0), DespawnOnExit(Screen::Level)));
    }
}



