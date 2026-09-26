//! Chick Chick BOOM, running natively on Bevy.
//!
//! ```text
//! ccb-game [--data extracted/files/0002] [--level city.1] [--screenshot out.png]
//!
//! Without `--level` the game starts at the title screen.
//! ```

mod anim;
mod data;
mod game;
mod g3d;
mod gx_material;
mod hud;
mod layout;
mod level;
mod pointer;
mod shot;
mod title;

use bevy::prelude::*;

/// Top-level game screens.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Screen {
    #[default]
    Title,
    Level,
}

#[derive(Resource, Clone)]
pub struct Options {
    pub data_dir: std::path::PathBuf,
    /// Start directly in this level instead of the title screen.
    pub level: Option<String>,
    pub screenshot: Option<std::path::PathBuf>,
}

fn parse_args() -> Options {
    let mut o = Options { data_dir: "extracted/files/0002".into(), level: None, screenshot: None };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data" => o.data_dir = args.next().expect("--data needs a path").into(),
            "--level" => o.level = Some(args.next().expect("--level needs a name like city.1")),
            "--screenshot" => o.screenshot = args.next().map(Into::into),
            other => eprintln!("ignoring unknown argument {other}"),
        }
    }
    o
}

fn main() -> AppExit {
    let opts = parse_args();
    let game_data = match data::GameData::load(&opts.data_dir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to load game data from {}: {e:#}", opts.data_dir.display());
            eprintln!("run ccb-extract on your WAD first (see README)");
            return AppExit::error();
        }
    };
    let layouts = match layout::LayoutAssets::load(&game_data) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("failed to load layouts: {e:#}");
            return AppExit::error();
        }
    };
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Chick Chick BOOM".into(),
            resolution: (1280u32, 720u32).into(),
            ..default()
        }),
        ..default()
    }))
    // Clamp long frames (slow machines, software rendering) so every gameplay system
    // advances by the same bounded step and physics stays stable.
    .insert_resource(Time::<Virtual>::from_max_delta(std::time::Duration::from_millis(50)))
    .insert_resource(ClearColor(Color::WHITE))
    .insert_resource(game_data)
    .insert_resource(layouts)
    .insert_state(if opts.level.is_some() { Screen::Level } else { Screen::Title })
    .add_plugins((
        gx_material::GxMaterialPlugin,
        level::LevelPlugin,
        anim::AnimPlugin,
        layout::LayoutPlugin,
        title::TitlePlugin,
        pointer::PointerPlugin,
        hud::HudPlugin,
        game::GamePlugin,
    ))
    .add_systems(Update, title::hide_panes);
    if opts.screenshot.is_some() {
        app.add_plugins(shot::ScreenshotPlugin);
    }
    app.insert_resource(opts);
    app.run()
}
