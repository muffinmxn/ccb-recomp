//! Loads a level as described by `cfg/levels.cfg` and sets up camera and music.

use std::sync::Arc;

use anyhow::{Context, Result};
use bevy::{audio::AudioSource, core_pipeline::tonemapping::Tonemapping, prelude::*};
use wii_formats::brres::Brres;

use crate::{anim::BoneAnimator, data::GameData, g3d, Options};

pub struct LevelPlugin;

impl Plugin for LevelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(crate::Screen::Level), spawn_level);
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
    mut materials: ResMut<Assets<crate::gx_material::GxMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut audio: ResMut<Assets<AudioSource>>,
) {
    let level = opts.level.clone().unwrap_or_else(|| "city.1".into());
    let result: Result<()> = (|| {
        let def = LevelDef::load(&data, &level)?;
        let common = data.brres("common.brres.LZ")?.to_vec();
        let theme = data.brres(&def.theme_brres)?.to_vec();
        let scene = data.brres(&def.level_brres)?.to_vec();
        let (common, theme, scene) = (Brres::parse(&common)?, Brres::parse(&theme)?, Brres::parse(&scene)?);
        let textures = g3d::load_textures(&[&common, &theme, &scene], &mut images);
        let clips: Vec<Arc<wii_formats::chr0::Chr0>> = scene.bone_animations()?.into_iter().map(Arc::new).collect();
        let clip = |name: &str| clips.iter().find(|c| c.name == name).cloned();

        let root = commands.spawn((Name::new(format!("level.{level}")), Transform::default(), Visibility::default())).id();
        for (name, _) in scene.folder("3DModels(NW4R)") {
            if name.ends_with("Blend") {
                continue; // full-screen fade overlay, driven by gameplay
            }
            let model = scene.model(name)?;
            let e = g3d::spawn_model(&mut commands, root, &model, &textures, &mut meshes, &mut materials, &mut images, &g3d::no_tweak);
            // `<model>__default` is a one-frame pre-intro pose; `<model>__intro` then plays
            // once and holds its last frame. (`won`/`lost`/`shock*` are triggered by gameplay.)
            let queue: Vec<_> = [format!("{name}__default"), format!("{name}__intro")].iter().filter_map(|n| clip(n)).collect();
            if !queue.is_empty() {
                commands.entity(e).insert(BoneAnimator::new(queue));
            }
        }

        spawn_sky(&mut commands, root, &common, &textures, &mut meshes, &mut materials, &mut images)?;
        spawn_camera(&mut commands, &data, false)?;

        let music = data.read(&format!("sounds/{}", def.music))?;
        commands.spawn((AudioPlayer::new(audio.add(AudioSource { bytes: music.into() })), PlaybackSettings::LOOP));
        info!("loaded level {level} ({} + {}, music {})", def.theme_brres, def.level_brres, def.music);
        Ok(())
    })();
    if let Err(e) = result {
        error!("failed to load level {level}: {e:#}");
    }
}

/// Spawns the sky dome and horizon rings shared by all levels and the title screen.
///
/// Their materials end with a fade stage, `color * (1 - C2.a) + K0.a`, that the game drives
/// at runtime (the file stores full white); it is zeroed here for normal play. The
/// `levelCityBlend` quad is a full-screen overlay used for level transitions and is skipped.
pub fn spawn_sky(
    commands: &mut Commands,
    parent: Entity,
    common: &Brres,
    textures: &std::collections::HashMap<String, (Handle<Image>, [u32; 2])>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<crate::gx_material::GxMaterial>,
    images: &mut Assets<Image>,
) -> Result<()> {
    let sky = common.model("sky")?;
    let sky_bones: Vec<String> = sky.bones.iter().map(|b| b.name.clone()).collect();
    let tweak = move |m: &wii_formats::mdl0::Mesh, _: &str, mat: &mut crate::gx_material::GxMaterial| {
        if sky_bones.get(m.bone).is_some_and(|b| b.ends_with("Blend")) {
            return false;
        }
        if mat.params.info.x == 3 {
            mat.params.regs[3].w = 0.0;
            mat.params.konst[0].w = 0.0;
        }
        true
    };
    g3d::spawn_model(commands, parent, &sky, textures, meshes, materials, images, &tweak);
    Ok(())
}

/// Spawns the 3D camera from `ingame.view.renderer` (16:9 variant). Menus raise the
/// camera by `camManagerMenuOffset`.
pub fn spawn_camera(commands: &mut Commands, data: &GameData, menu: bool) -> Result<()> {
    let view = data.cfg.block("ingame.view.renderer")?;
    let lines = |label: &str| -> Vec<Vec3> {
        view.lines
            .iter()
            .filter(|l| l.label.as_deref() == Some(label))
            .map(|l| {
                let f: Vec<f32> = l.values.iter().filter_map(|v| v.parse().ok()).collect();
                Vec3::new(f[0], f[1], f[2])
            })
            .collect()
    };
    let mut pos = *lines("camPos").last().context("no camPos")?;
    let mut aim = *lines("camAim").last().context("no camAim")?;
    if menu {
        let off = view.vec3("camManagerMenuOffset").map(Vec3::from).unwrap_or(Vec3::ZERO);
        pos += off;
        aim += off;
    }
    let fov = view.f32("camFov").unwrap_or(35.0);
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        Projection::Perspective(PerspectiveProjection { fov: fov.to_radians(), ..default() }),
        Transform::from_translation(pos).looking_at(aim, Vec3::Y),
    ));
    Ok(())
}
