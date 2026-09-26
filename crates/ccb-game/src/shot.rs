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
        app.init_resource::<ShotTrigger>().add_systems(Update, take_screenshot);
    }
}

/// Set by gameplay when `CCB_SHOT_ON=<kind>:<state>[:frames]` matches an attack event:
/// the screenshot is taken that many frames later instead of at `CCB_SHOT_FRAME`.
#[derive(Resource, Default)]
pub struct ShotTrigger(pub Option<u32>);

fn take_screenshot(mut commands: Commands, opts: Res<Options>, mut trigger: ResMut<ShotTrigger>, mut frame: Local<u32>, mut exit: MessageWriter<AppExit>) {
    *frame += 1;
    if let Some(n) = trigger.0 {
        if n == 0 {
            if let Some(path) = opts.screenshot.clone() {
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
            }
            trigger.0 = Some(u32::MAX);
            *frame = u32::MAX - 20;
        } else if n != u32::MAX {
            trigger.0 = Some(n - 1);
        }
        if *frame >= u32::MAX - 10 {
            exit.write(AppExit::Success);
        }
        return;
    }
    if std::env::var("CCB_SHOT_ON").is_ok() {
        // Waiting for the gameplay event; give up after a long while.
        if *frame > 60 * 120 {
            exit.write(AppExit::Success);
        }
        return;
    }
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
