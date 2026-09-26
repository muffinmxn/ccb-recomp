//! `--screenshot out.png`: render a few frames, save the window to a file, then quit.
//! Used for checking rendering without a display (e.g. under Xvfb in CI).

use bevy::{
    prelude::*,
    render::view::screenshot::{save_to_disk, Screenshot},
};

use crate::Options;

pub struct ScreenshotPlugin;

impl Plugin for ScreenshotPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, take_screenshot);
    }
}

fn take_screenshot(mut commands: Commands, opts: Res<Options>, mut frame: Local<u32>, mut exit: MessageWriter<AppExit>) {
    *frame += 1;
    let at: u32 = std::env::var("CCB_SHOT_FRAME").ok().and_then(|v| v.parse().ok()).unwrap_or(30);
    if *frame == at {
        if let Some(path) = opts.screenshot.clone() {
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
        }
    }
    if *frame == at + 10 {
        exit.write(AppExit::Success);
    }
}
