//! Loads a level as described by `cfg/levels.cfg` and sets up camera and music.

use anyhow::{Context, Result};
use bevy::{audio::AudioSource, prelude::*};
use wii_formats::brres::Brres;

use crate::{data::GameData, g3d, Options};

pub struct LevelPlugin;

impl Plugin for LevelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_level);
    }
}

/// The parts of a `level.*` block we use so far.
pub struct LevelDef {
    pub theme_brres: String,
    pub level_brres: String,
    pub music: String,
}

impl LevelDef {
    pub fn load(data: &GameData, name: &str) -> Result<Self> {
        let block = data.cfg.block(&format!("level.{name}"))?;
        let values: Vec<&str> = block.values().collect();
        Ok(Self {
            theme_brres: values.first().context("level has no theme archive")?.to_string(),
            level_brres: values.get(1).context("level has no scene archive")?.to_string(),
            music: values.iter().rev().find(|v| v.ends_with(".ogg")).context("level has no music")?.to_string(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_level(
    mut commands: Commands,
    opts: Res<Options>,
    mut data: ResMut<GameData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut audio: ResMut<Assets<AudioSource>>,
) {
    let result: Result<()> = (|| {
        let def = LevelDef::load(&data, &opts.level)?;
        let theme = data.brres(&def.theme_brres)?.to_vec();
        let scene = data.brres(&def.level_brres)?.to_vec();
        let (theme, scene) = (Brres::parse(&theme)?, Brres::parse(&scene)?);
        let textures = g3d::load_textures(&[&theme, &scene], &mut images);

        let root = commands.spawn((Name::new(format!("level.{}", opts.level)), Transform::default(), Visibility::default())).id();
        for (name, _) in scene.folder("3DModels(NW4R)") {
            if name.ends_with("Blend") {
                continue; // full-screen fade overlay, driven by gameplay
            }
            let model = scene.model(name)?;
            g3d::spawn_model(&mut commands, root, &model, &textures, &mut meshes, &mut materials, &mut images);
        }

        // Camera from ingame.view.renderer (16:9 variant).
        let view = data.cfg.block("ingame.view.renderer")?;
        let cams: Vec<&ccb_assets::cfg::Line> =
            view.lines.iter().filter(|l| l.label.as_deref() == Some("camPos")).collect();
        let aims: Vec<&ccb_assets::cfg::Line> =
            view.lines.iter().filter(|l| l.label.as_deref() == Some("camAim")).collect();
        let v3 = |l: &ccb_assets::cfg::Line| -> Vec3 {
            let f: Vec<f32> = l.values.iter().filter_map(|v| v.parse().ok()).collect();
            Vec3::new(f[0], f[1], f[2])
        };
        let (pos, aim) = (v3(cams.last().context("no camPos")?), v3(aims.last().context("no camAim")?));
        let fov = view.f32("camFov").unwrap_or(35.0);
        commands.spawn((
            Camera3d::default(),
            Projection::Perspective(PerspectiveProjection { fov: fov.to_radians(), ..default() }),
            Transform::from_translation(pos).looking_at(aim, Vec3::Y),
        ));

        let music = data.read(&format!("sounds/{}", def.music))?;
        commands.spawn((AudioPlayer::new(audio.add(AudioSource { bytes: music.into() })), PlaybackSettings::LOOP));
        info!("loaded level {} ({} + {}, music {})", opts.level, def.theme_brres, def.level_brres, def.music);
        Ok(())
    })();
    if let Err(e) = result {
        error!("failed to load level {}: {e:#}", opts.level);
    }
}
