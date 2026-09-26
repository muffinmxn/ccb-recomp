//! Sound effects from `sounds/ccb_ingame.brsar`, decoded once at startup.
//! Gameplay code queues effects by their original names (`SFX_ATTACKS_BOMB_EXPL`, ...).

use std::collections::HashMap;

use bevy::{
    audio::{AudioSource, PlaybackMode, Volume},
    prelude::*,
};
use wii_formats::rsar::SoundArchive;

use crate::data::GameData;

#[derive(Resource, Default)]
pub struct Sfx {
    sounds: HashMap<String, (Handle<AudioSource>, f32)>,
    queue: Vec<String>,
}

impl Sfx {
    pub fn play(&mut self, name: &str) {
        self.queue.push(name.to_string());
    }

    /// Plays one of several variations (e.g. `SFX_CHICKS_AUA01`..`05`).
    pub fn play_one_of(&mut self, names: &[&str], pick: usize) {
        if !names.is_empty() {
            self.play(names[pick % names.len()]);
        }
    }
}

pub struct SfxPlugin;

impl Plugin for SfxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Sfx>().add_systems(Startup, load_sfx).add_systems(PostUpdate, play_queued);
    }
}

fn load_sfx(data: Res<GameData>, mut sfx: ResMut<Sfx>, mut audio: ResMut<Assets<AudioSource>>) {
    let bytes = match data.read("sounds/ccb_ingame.brsar") {
        Ok(b) => b,
        Err(e) => {
            warn!("no sound effects: {e:#}");
            return;
        }
    };
    let Ok(archive) = SoundArchive::parse(&bytes) else {
        warn!("could not parse the sound archive");
        return;
    };
    for s in &archive.sounds {
        match archive.decode(s) {
            Ok(pcm) => {
                let handle = audio.add(AudioSource { bytes: pcm.to_wav().into() });
                // Archive volumes are 0..127 with 96 as the common default.
                sfx.sounds.insert(s.name.clone(), (handle, s.volume as f32 / 110.0));
            }
            Err(e) => warn!("{}: {e:#}", s.name),
        }
    }
    info!("loaded {} sound effects", sfx.sounds.len());
}

fn play_queued(mut commands: Commands, mut sfx: ResMut<Sfx>) {
    let queue = std::mem::take(&mut sfx.queue);
    for name in queue {
        match sfx.sounds.get(&name) {
            Some((h, vol)) => {
                commands.spawn((
                    AudioPlayer::new(h.clone()),
                    PlaybackSettings { mode: PlaybackMode::Despawn, volume: Volume::Linear(*vol), ..default() },
                ));
            }
            None => debug!("unknown sound {name}"),
        }
    }
}
