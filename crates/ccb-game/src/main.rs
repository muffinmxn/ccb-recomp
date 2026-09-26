//! Chick Chick BOOM, running natively on Bevy.
//!
//! ```text
//! ccb-game [--data extracted/files/0002] [--level city.1] [--screenshot out.png]
//! ```

mod anim;
mod data;
mod g3d;
mod level;
mod shot;

use bevy::prelude::*;

#[derive(Resource, Clone)]
pub struct Options {
    pub data_dir: std::path::PathBuf,
    pub level: String,
    pub screenshot: Option<std::path::PathBuf>,
}

fn parse_args() -> Options {
    let mut o = Options { data_dir: "extracted/files/0002".into(), level: "city.1".into(), screenshot: None };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data" => o.data_dir = args.next().expect("--data needs a path").into(),
            "--level" => o.level = args.next().expect("--level needs a name like city.1"),
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
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Chick Chick BOOM".into(),
            resolution: (1280u32, 720u32).into(),
            ..default()
        }),
        ..default()
    }))
    .insert_resource(ClearColor(Color::srgb_u8(33, 96, 168)))
    .insert_resource(game_data)
    .add_plugins((level::LevelPlugin, anim::AnimPlugin));
    if opts.screenshot.is_some() {
        app.add_plugins(shot::ScreenshotPlugin);
    }
    app.insert_resource(opts);
    app.run()
}
