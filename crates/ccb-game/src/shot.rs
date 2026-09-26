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
    // CCB_SHOT_FRAME=N or a list "N1,N2,..." (extra shots get a `.N` suffix).
    let frames: Vec<u32> = std::env::var("CCB_SHOT_FRAME")
        .ok()
        .map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .filter(|v: &Vec<u32>| !v.is_empty())
        .unwrap_or_else(|| vec![30]);
    let last = *frames.iter().max().unwrap();
    if frames.contains(&*frame) {
        if let Some(path) = opts.screenshot.clone() {
            let path = if *frame == last { path } else { path.with_extension(format!("{}.png", *frame)) };
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
        }
    }
    if *frame == last + 10 {
        exit.write(AppExit::Success);
    }
}
