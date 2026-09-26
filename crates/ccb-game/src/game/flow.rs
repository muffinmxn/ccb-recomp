//! Match flow: clock spin → turns (attack, then the other side defends) → game over.

use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result};
use bevy::prelude::*;
use wii_formats::{brres::Brres, mdl0::Model};

use super::{
    ai::Cpu,
    attack::Attack,
    barrier::Ink,
    chick::{spawn_chick, Chick, ChickAssets},
    gesture::{Gestures, Trace},
    rng::Rng,
    tuning::Tuning,
    AttackKind, Team,
};
use crate::{
    anim::{BoneAnimator, Clip},
    data::GameData,
    g3d,
    gx_material::GxMaterial,
    hud::Hud,
    layout::{self, LayoutAnimator, LayoutAssets, LayoutPane, LayoutRoot, TextPane},
    Screen,
};

/// Chicks per team: one big, the rest small.
const CHICKS_PER_TEAM: usize = 5;
/// Round wins needed to take a duel.
pub const ROUNDS_TO_WIN: usize = 2;
/// Pause between rounds.
const ROUND_BREAK: f32 = 3.5;
/// Length of the `hud_clockStart*` spin (131 frames at 60 fps).
const SPIN_TIME: f32 = 131.0 / 60.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// The clock arrow spins to pick the first attacker.
    Spin(f32),
    /// The attacker picks an attack and traces it; `turn_timer` counts down.
    Attack,
    /// An attack is on its way; the defender draws barriers.
    Incoming,
    /// Short pause after an attack resolves.
    Finish(f32),
    /// A team was wiped out but the duel goes on.
    RoundOver(f32),
    GameOver(f32),
}

#[derive(Resource)]
pub struct Match {
    pub phase: Phase,
    pub attacker: Team,
    pub turn_timer: f32,
    pub match_time: f32,
    pub available: Vec<AttackKind>,
    /// The player's selected attack while tracing its gesture.
    pub selected: Option<AttackKind>,
    /// An attack ready to launch (kind, quality, upgrade 0/1 green/2 red), from the player or the CPU.
    pub pending: Option<(AttackKind, f32, u8)>,
    pub attack: Option<Entity>,
    /// Team health when full (sum of the chicks' max health).
    pub max_health: [f32; 2],
    pub winner: Option<Team>,
    /// HUD animation to start on the next update.
    pub hud_anim: Option<String>,
    /// Level specials (UFO, sea monster, ghost, lightning).
    pub specials: Vec<AttackKind>,
    /// Open special window: (attack, seconds left). Both sides race to trace it.
    pub special: Option<(AttackKind, f32)>,
    /// Seconds until the next special window.
    pub special_timer: f32,
    /// A traced special ready to launch: (from team, attack, quality).
    pub pending_special: Option<(Team, AttackKind, f32)>,
    /// Who the open special would hit when not the launcher's opponent (the cloud's side).
    pub special_target: Option<Team>,
    /// Rounds won per team; the round winner of the last round.
    pub rounds_won: [usize; 2],
    pub round_winner: Option<Team>,
}

impl Match {
    /// Whether `team` may draw barriers now (while the other side has the turn).
    pub fn can_defend(&self, team: Team) -> bool {
        self.attacker != team && matches!(self.phase, Phase::Attack | Phase::Incoming | Phase::Finish(_))
    }

    fn start_turn(&mut self, attacker: Team, tuning: &Tuning) {
        self.attacker = attacker;
        self.phase = Phase::Attack;
        self.turn_timer = tuning.attack_time;
        self.selected = None;
        self.pending = None;
        let side = if attacker == Team::Yellow { "Yellow" } else { "Black" };
        self.hud_anim = Some(format!("hud_clockAttack{side}"));
    }
}

/// Models and textures used by gameplay, from `common.brres` and the level's theme archive.
#[derive(Resource)]
pub struct GameAssets {
    pub chick: ChickAssets,
    models: HashMap<String, Model>,
    /// Inverse bind poses of skinned models (e.g. the tentacle).
    skins: HashMap<String, Handle<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
    widths: HashMap<String, f32>,
    textures: HashMap<String, (Handle<Image>, [u32; 2])>,
    clips: Vec<Arc<Clip>>,
    root: Entity,
    /// Visible (opaque) width of each weight variant's card, in model units.
    pub weight_visible: Vec<f32>,
}

/// Visible width of the widest textured card under `root_bone`: its model width times the
/// share of the texture's columns that aren't transparent.
fn card_visible_width(model: &Model, root_bone: &str, textures: &HashMap<String, (Handle<Image>, [u32; 2])>, images: &Assets<Image>) -> Option<f32> {
    let r = model.bones.iter().position(|b| b.name == root_bone)?;
    let under = |mut b: usize| loop {
        if b == r {
            break true;
        }
        match model.bones[b].parent {
            Some(p) => b = p,
            None => break false,
        }
    };
    let (mesh, lo, hi) = model
        .meshes
        .iter()
        .filter(|m| under(m.bone) && !m.uvs.is_empty())
        .map(|m| {
            let (lo, hi) = m.positions.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p[0]), hi.max(p[0])));
            (m, lo, hi)
        })
        .max_by(|a, b| (a.2 - a.1).total_cmp(&(b.2 - b.1)))?;
    let tex = &model.materials.get(mesh.material)?.textures.first()?.texture;
    let img = images.get(&textures.get(tex)?.0)?;
    let data = img.data.as_ref()?;
    let (w, h) = (img.width() as usize, img.height() as usize);
    let opaque = |x: usize| (0..h).any(|y| data.get((y * w + x) * 4 + 3).is_some_and(|&a| a > 128));
    let x0 = (0..w).find(|&x| opaque(x))?;
    let x1 = (0..w).rev().find(|&x| opaque(x))?;
    // The card's UVs span its width.
    let (u0, u1) = mesh.uvs.iter().fold((f32::MAX, f32::MIN), |(a, b), uv| (a.min(uv[0]), b.max(uv[0])));
    let frac = ((x1 + 1 - x0) as f32 / w as f32 / (u1 - u0).abs().max(1e-3)).min(1.0);
    Some((hi - lo) * frac)
}

/// Bind-pose extent of a model along one axis.
fn model_extent(m: &Model, axis: usize) -> f32 {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for p in m.meshes.iter().flat_map(|x| x.positions.iter()) {
        lo = lo.min(p[axis]);
        hi = hi.max(p[axis]);
    }
    (hi - lo).max(0.01)
}

impl GameAssets {
    pub fn scale(&self, name: &str, width: f32) -> f32 {
        width / self.widths.get(name).copied().unwrap_or(1.0)
    }

    /// Spawns a gameplay model; black-team variants swap `...Yellow` textures for `...Black`.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        &self,
        commands: &mut Commands,
        name: &str,
        team: Team,
        width: f32,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<GxMaterial>,
        images: &mut Assets<Image>,
    ) -> Entity {
        self.spawn_bones(commands, name, team, width, None, meshes, materials, images)
    }

    /// Like [`Self::spawn`], keeping only the meshes on the given bones.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_bones(
        &self,
        commands: &mut Commands,
        name: &str,
        team: Team,
        width: f32,
        bones: Option<&[&str]>,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<GxMaterial>,
        images: &mut Assets<Image>,
    ) -> Entity {
        let Some(model) = self.models.get(name) else {
            return commands.spawn((Transform::default(), Visibility::default())).id();
        };
        let tex = self.textures.clone();
        let keep: Option<Vec<usize>> = bones.map(|list| model.bones.iter().enumerate().filter(|(_, b)| list.contains(&b.name.as_str())).map(|(i, _)| i).collect());
        let tweak = move |m: &wii_formats::mdl0::Mesh, mat_name: &str, mat: &mut GxMaterial| {
            if keep.as_ref().is_some_and(|k| !k.contains(&m.bone)) {
                return false;
            }
            // The plant hint ring's color comes from a konst register the game sets.
            if mat_name == "plantCircleMat" {
                mat.params.konst[0] = Vec4::new(1.0, 0.85, 0.1, 1.0);
            }
            // The flytrap's head colour is set from code too; its leaves' green.
            if mat_name == "plantHeadMat" {
                mat.params.konst[0] = Vec4::new(148.0 / 255.0, 184.0 / 255.0, 3.0 / 255.0, 1.0);
            }
            if team == Team::Black {
                if let Some(h) = mat.tex0.clone() {
                    if let Some((n, _)) = tex.iter().find(|(_, (th, _))| *th == h) {
                        if let Some((bh, _)) = n.strip_suffix("Yellow").and_then(|b| tex.get(&format!("{b}Black"))) {
                            mat.tex0 = Some(bh.clone());
                        }
                    }
                }
            }
            true
        };
        let e = g3d::spawn_model_skinned(commands, self.root, model, &self.textures, meshes, materials, images, &tweak, self.skins.get(name).cloned());
        // Attack models carry a black outline card that the game shows from code; models
        // spawned in parts show their parts (their bones may start hidden until an intro clip).
        match bones {
            Some(list) => {
                let mut show: Vec<&'static str> = vec!["outline"];
                show.extend(list.iter().filter_map(|n| model.bones.iter().find(|b| b.name == *n).map(|b| &*b.name.clone().leak())));
                commands.entity(e).insert(crate::anim::ShowBones(show));
            }
            None if model.bones.iter().any(|b| b.name == "outline") => {
                commands.entity(e).insert(crate::anim::ShowBones(vec!["outline"]));
            }
            None if name == "plantCircle" => {
                commands.entity(e).insert(crate::anim::ShowBones(vec!["*"]));
            }
            None => {}
        }
        commands.entity(e).insert(Transform::from_xyz(0.0, -100.0, 0.0).with_scale(Vec3::splat(self.scale(name, width))));
        e
    }

    /// Spawns a model and applies a one-frame pose clip.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_posed(
        &self,
        commands: &mut Commands,
        name: &str,
        clip: &str,
        team: Team,
        width: f32,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<GxMaterial>,
        images: &mut Assets<Image>,
    ) -> Entity {
        let e = self.spawn(commands, name, team, width, meshes, materials, images);
        self.play(commands, e, clip);
        e
    }

    pub fn clip(&self, name: &str) -> Option<Arc<Clip>> {
        self.clips.iter().find(|c| c.name == name).cloned()
    }

    pub fn texture(&self, name: &str) -> Option<Handle<Image>> {
        self.textures.get(name).map(|t| t.0.clone())
    }

    /// Plays a bone animation clip on a spawned model.
    pub fn play(&self, commands: &mut Commands, model: Entity, clip: &str) {
        if let Some(c) = self.clips.iter().find(|c| c.name == clip) {
            commands.entity(model).insert(BoneAnimator::new(vec![c.clone()]));
        }
    }

    /// Spawns the weight posed as one of its variants (`weight__weightDefault0N`).
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_weight(
        &self,
        commands: &mut Commands,
        variant: usize,
        team: Team,
        width: f32,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<GxMaterial>,
        images: &mut Assets<Image>,
    ) -> Entity {
        let e = self.spawn(commands, "weight", team, width, meshes, materials, images);
        let name = if variant == 7 { "weight__weightSumo".to_string() } else { format!("weight__weightDefault0{}", variant.clamp(1, 6)) };
        if let Some(c) = self.clips.iter().find(|c| c.name == name) {
            commands.entity(e).insert(BoneAnimator::new(vec![c.clone()]));
        }
        e
    }

    /// A translucent vertical beam (UFO tractor beam).
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_beam(&self, commands: &mut Commands, x: f32, bottom: f32, top: f32, z: f32, width: f32, color: Vec4, meshes: &mut Assets<Mesh>, materials: &mut Assets<GxMaterial>) -> Entity {
        let h = (top - bottom).max(0.1);
        let mut mat = super::barrier::solid_material(color);
        mat.params.konst[3].w = color.w;
        commands
            .spawn((
                Mesh3d(meshes.add(Rectangle::new(width, h))),
                MeshMaterial3d(materials.add(mat)),
                Transform::from_xyz(x, bottom + h / 2.0, z + 0.3),
                DespawnOnExit(Screen::Level),
            ))
            .id()
    }

    /// A soft dark ellipse lying on the ground (a falling weight's shadow); scale x for width.
    pub fn spawn_shadow(&self, commands: &mut Commands, z: f32, meshes: &mut Assets<Mesh>, materials: &mut Assets<GxMaterial>) -> Entity {
        let mut mat = super::barrier::solid_material(Vec4::new(0.0, 0.0, 0.0, 0.35));
        mat.params.konst[3].w = 0.35;
        commands
            .spawn((
                Mesh3d(meshes.add(Ellipse::new(0.5, 0.5))),
                MeshMaterial3d(materials.add(mat)),
                Transform::from_xyz(0.0, 0.03, z),
                DespawnOnExit(Screen::Level),
            ))
            .insert(Transform::from_xyz(0.0, 0.03, z).with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)))
            .id()
    }

    /// A plant's vine ribbon (its mesh is replaced as it grows).
    pub fn spawn_stem(&self, commands: &mut Commands, z: f32, color: Vec4, materials: &mut Assets<GxMaterial>) -> Entity {
        commands
            .spawn((
                MeshMaterial3d(materials.add(super::barrier::solid_material(color))),
                Transform::from_xyz(0.0, 0.0, z),
                Visibility::default(),
                bevy::camera::visibility::NoFrustumCulling,
                DespawnOnExit(Screen::Level),
            ))
            .id()
    }

    /// Model-space width of the meshes on a bone.
    pub fn bone_width(&self, name: &str, bone: &str) -> f32 {
        let Some(model) = self.models.get(name) else { return 1.0 };
        let xs = model.meshes.iter().filter(|m| model.bones.get(m.bone).is_some_and(|b| b.name == bone)).flat_map(|m| m.positions.iter().map(|p| p[0]));
        let (lo, hi) = xs.fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
        if hi > lo { hi - lo } else { 1.0 }
    }

    /// A lightning bolt: a bright quad from `bottom` to `top` at `x`.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_bolt(&self, commands: &mut Commands, x: f32, bottom: f32, top: f32, z: f32, meshes: &mut Assets<Mesh>, materials: &mut Assets<GxMaterial>) -> Entity {
        let h = (top - bottom).max(0.1);
        commands
            .spawn((
                Mesh3d(meshes.add(Rectangle::new(0.35, h))),
                MeshMaterial3d(materials.add(super::barrier::solid_material(Vec4::new(1.0, 1.0, 0.55, 1.0)))),
                Transform::from_xyz(x, bottom + h / 2.0, z + 0.4),
                DespawnOnExit(Screen::Level),
            ))
            .id()
    }
}

#[allow(clippy::too_many_arguments)]
pub fn start_match(
    mut commands: Commands,
    mut skin_store: ResMut<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
    opts: Res<crate::Options>,
    mut data: ResMut<GameData>,
    mut lyt: ResMut<LayoutAssets>,
    mut rng: ResMut<Rng>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let r: Result<()> = (|| {
        let tuning = Tuning::load(&data)?;
        let level = opts.level.clone().unwrap_or_else(|| "city.1".into());
        let block = data.cfg.block(&format!("level.{level}"))?;
        let values: Vec<String> = block.values().map(String::from).collect();
        let count: usize = values.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
        let available: Vec<AttackKind> = values[3..3 + count.min(values.len().saturating_sub(3))]
            .iter()
            .filter_map(|v| v.parse().ok().and_then(AttackKind::from_id))
            .collect();

        let common_bytes = data.brres("common.brres.LZ")?.to_vec();
        let common = Brres::parse(&common_bytes)?;
        let theme_name = values.first().context("level has no theme")?.to_string();
        let theme_bytes = data.brres(&theme_name)?.to_vec();
        let theme = Brres::parse(&theme_bytes)?;
        let textures = g3d::load_textures(&[&common, &theme], &mut images);
        let clips = crate::anim::load_clips(&common)?;
        let mut models = HashMap::new();
        for name in ["bomb", "bombA", "bombB", "weight", "explosion", "cloud", "cloudLightning", "plant", "plantA", "plantB", "plantCircle", "glue", "acid"] {
            if let Ok(m) = common.model(name) {
                models.insert(name.to_string(), m);
            }
        }
        // The level theme's special-attack models.
        for name in ["ufo", "bgUfoBeam", "tentacle", "octopus", "ghost"] {
            if let Ok(m) = theme.model(name) {
                models.insert(name.to_string(), m);
            }
        }
        let mut clips = clips;
        clips.extend(crate::anim::load_clips(&theme)?);
        // Tall models (the tentacle) are sized by height, everything else by width.
        let widths = models
            .iter()
            .map(|(n, m)| (n.clone(), if n == "tentacle" { model_extent(m, 1) } else { model_extent(m, 0) }))
            .collect();
        let root = commands.spawn((Name::new("gameplay"), Transform::default(), Visibility::default(), DespawnOnExit(Screen::Level))).id();
        let chick_assets = ChickAssets::new(common.model("chick")?, textures.clone());

        spawn_teams(&mut commands, root, &chick_assets, &tuning, &mut rng, &mut meshes, &mut materials, &mut images);

        // Both teams' attack interfaces (the original's attackIfc + blueprints layouts).
        super::attack_ui::spawn(&mut commands, &mut lyt, &tuning, &mut meshes, &mut materials, &mut images)?;
        let player = if std::env::var("CCB_AUTOPLAY").is_ok() { None } else { Some(Team::Yellow) };
        commands.insert_resource(super::attack_ui::IfcView { player, ..default() });

        let first = if rng.chance(0.5) { Team::Yellow } else { Team::Black };
        let side = if first == Team::Yellow { "Yellow" } else { "Black" };
        info!("match on {level}: attacks {available:?}, {first:?} starts");
        commands.insert_resource(Gestures::load(&lyt)?);
        commands.insert_resource(Trace::default());
        commands.insert_resource(Cpu::load(&data)?);
        commands.insert_resource(Ink([tuning.ink_max; 2]));
        commands.insert_resource(super::env::Env::default());
        commands.insert_resource(Match {
            phase: Phase::Spin(0.0),
            attacker: first,
            turn_timer: tuning.attack_time,
            match_time: 0.0,
            available: available.clone(),
            selected: None,
            pending: None,
            attack: None,
            max_health: [0.0; 2],
            winner: None,
            hud_anim: Some(format!("hud_clockStart{side}")),
            // One special per arena, as on the arena screen.
            // The level's environment special and lightning (from the drifting clouds).
            specials: available.iter().copied().filter(|k| !k.is_basic() && k.implemented()).collect(),
            special: None,
            // Debug: CCB_ENV_AT=<seconds> brings the first environment event forward.
            special_timer: std::env::var("CCB_ENV_AT").ok().and_then(|v| v.parse().ok()).unwrap_or_else(|| rng.range(tuning.special_first)),
            pending_special: None,
            special_target: None,
            rounds_won: [0, 0],
            round_winner: None,
        });
        let skins = models.iter().filter_map(|(n, m)| g3d::inverse_binds(m, &mut skin_store).map(|h| (n.clone(), h))).collect();
        // Variant 7 is the sumo chick (the red weight upgrade).
        let weight_visible = (1..=7)
            .map(|v| {
                let bone = if v == 7 { "weightSumo".to_string() } else { format!("weight0{v}") };
                models.get("weight").and_then(|m| card_visible_width(m, &bone, &textures, &images)).unwrap_or(1.5)
            })
            .collect::<Vec<_>>();
        debug!("weight visible widths {weight_visible:?}");
        commands.insert_resource(GameAssets { chick: chick_assets, models, skins, widths, textures, clips, root, weight_visible });
        debug!("special tuning {:?}", tuning.sp);
        commands.insert_resource(tuning);
        Ok(())
    })();
    if let Err(e) = r {
        error!("failed to start match: {e:#}");
    }
}

/// Spawns both teams' chicks spread over their halves (one big chick each).
#[allow(clippy::too_many_arguments)]
fn spawn_teams(
    commands: &mut Commands,
    root: Entity,
    chick_assets: &ChickAssets,
    tuning: &Tuning,
    rng: &mut Rng,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<GxMaterial>,
    images: &mut Assets<Image>,
) {
    for team in [Team::Yellow, Team::Black] {
        let (lo, hi) = tuning.side_range(team);
        for i in 0..CHICKS_PER_TEAM {
            let x = lo + (hi - lo) * (i as f32 + 0.5) / CHICKS_PER_TEAM as f32 + rng.range((-0.4, 0.4));
            spawn_chick(commands, root, chick_assets, team, x, i == 1, tuning, rng, meshes, materials, images);
        }
    }
}

/// `CCB_SHOWCASE=1`: places every attack model on the field (for checking visuals).
pub fn showcase(
    mut commands: Commands,
    assets: Option<Res<GameAssets>>,
    tuning: Option<Res<Tuning>>,
    mut done: Local<bool>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let (Some(assets), Some(tuning)) = (assets, tuning) else { return };
    if *done || std::env::var("CCB_SHOWCASE").is_err() {
        return;
    }
    *done = true;
    let z = tuning.plane_z;
    // `CCB_SHOWCASE=<model>[:clip]` shows one model up close instead.
    let arg = std::env::var("CCB_SHOWCASE").unwrap_or_default();
    if arg != "1" {
        let (name, clip) = arg.split_once(':').unwrap_or((arg.as_str(), ""));
        let e = if clip.is_empty() {
            assets.spawn(&mut commands, name, Team::Yellow, 7.0, &mut meshes, &mut materials, &mut images)
        } else {
            assets.spawn_posed(&mut commands, name, clip, Team::Yellow, 7.0, &mut meshes, &mut materials, &mut images)
        };
        commands.entity(e).insert(Transform::from_xyz(0.0, 4.0, z + 8.0).with_scale(Vec3::splat(assets.scale(name, 7.0))));
        return;
    }
    fn place(commands: &mut Commands, assets: &GameAssets, e: Entity, name: &str, p: Vec3, w: f32) {
        commands.entity(e).insert(Transform::from_translation(p).with_scale(Vec3::splat(assets.scale(name, w))));
    }
    let e = assets.spawn_posed(&mut commands, "bomb", "bomb__fly", Team::Yellow, 1.4, &mut meshes, &mut materials, &mut images);
    place(&mut commands, &assets, e, "bomb", Vec3::new(-8.0, 3.5, z), 3.5);
    let e = assets.spawn_posed(&mut commands, "bomb", "bomb__roll", Team::Black, 1.4, &mut meshes, &mut materials, &mut images);
    place(&mut commands, &assets, e, "bomb", Vec3::new(-4.5, 3.5, z), 3.5);
    let e = assets.spawn_weight(&mut commands, 3, Team::Yellow, 3.6, &mut meshes, &mut materials, &mut images);
    place(&mut commands, &assets, e, "weight", Vec3::new(-3.0, 3.5, z), 3.6);
    let e = assets.spawn(&mut commands, "cloud", Team::Yellow, 5.0, &mut meshes, &mut materials, &mut images);
    place(&mut commands, &assets, e, "cloud", Vec3::new(2.0, tuning.cloud_y, z), 5.0);
    let e = assets.spawn_posed(&mut commands, "cloudLightning", "cloudLightning__spratzel", Team::Yellow, 5.0, &mut meshes, &mut materials, &mut images);
    place(&mut commands, &assets, e, "cloudLightning", Vec3::new(2.0, tuning.cloud_y, z + 0.2), 5.0);
    let e = assets.spawn_posed(&mut commands, "plant", "plant__intro", Team::Yellow, 2.4, &mut meshes, &mut materials, &mut images);
    place(&mut commands, &assets, e, "plant", Vec3::new(5.0, 1.6, z), 2.4);
    let e = assets.spawn(&mut commands, "explosion", Team::Yellow, 5.0, &mut meshes, &mut materials, &mut images);
    place(&mut commands, &assets, e, "explosion", Vec3::new(8.5, 2.0, z), 5.0);
    let bolt = assets.spawn_bolt(&mut commands, 2.0, 0.0, tuning.cloud_y - 0.6, z, &mut meshes, &mut materials);
    let _ = bolt;
}

pub fn end_match(mut commands: Commands) {
    commands.remove_resource::<Match>();
    commands.remove_resource::<GameAssets>();
    commands.remove_resource::<Cpu>();
    commands.remove_resource::<Trace>();
}

#[allow(clippy::too_many_arguments)]
pub fn run_match(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<Tuning>,
    assets: Res<GameAssets>,
    mut m: ResMut<Match>,
    mut rng: ResMut<Rng>,
    chicks: Query<&Chick>,
    chick_entities: Query<Entity, With<Chick>>,
    attack_entities: Query<Entity, With<Attack>>,
    barriers: Query<Entity, With<super::barrier::Barrier>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut images: ResMut<Assets<Image>>,
    attacks: Query<&Attack>,
    mut next: ResMut<NextState<Screen>>,
    mut sfx: ResMut<crate::sfx::Sfx>,
) {
    let dt = time.delta_secs();
    if !matches!(m.phase, Phase::GameOver(_) | Phase::RoundOver(_)) {
        m.match_time += dt;
        let alive = |t: Team| chicks.iter().filter(|c| c.team == t && c.alive()).count();
        for t in [Team::Yellow, Team::Black] {
            let wiped = alive(t) == 0 && !chicks.iter().any(|c| c.team == t && c.dying.is_some());
            if wiped && !matches!(m.phase, Phase::GameOver(_) | Phase::RoundOver(_)) {
                let w = t.other();
                m.rounds_won[w.index()] += 1;
                m.round_winner = Some(w);
                m.selected = None;
                m.special = None;
                if m.rounds_won[w.index()] >= ROUNDS_TO_WIN {
                    m.winner = Some(w);
                    m.phase = Phase::GameOver(0.0);
                    info!("game over: {w:?} wins {}-{}", m.rounds_won[w.index()], m.rounds_won[t.index()]);
                    sfx.play(if w == Team::Yellow { "SFX_GAME_WIN" } else { "SFX_GAME_OVER" });
                } else {
                    m.phase = Phase::RoundOver(0.0);
                    info!("round to {w:?} ({}-{})", m.rounds_won[0], m.rounds_won[1]);
                    sfx.play("SFX_FIREWORK");
                }
            }
        }
    }
    match m.phase {
        Phase::Spin(t) => {
            m.phase = if t + dt >= SPIN_TIME {
                let first = m.attacker;
                m.start_turn(first, &tuning);
                Phase::Attack
            } else {
                Phase::Spin(t + dt)
            };
        }
        Phase::Attack => {
            let before = m.turn_timer;
            m.turn_timer -= dt;
            // Tick through the last five seconds.
            if before.ceil() != m.turn_timer.ceil() && m.turn_timer > 0.0 && m.turn_timer <= 5.0 {
                sfx.play("SFX_ATTACK_IFC_CLOCK_TICK");
            }
            if let Some((kind, quality, upgrade)) = m.pending.take() {
                // Aim at a living chick; a weaker trace aims worse.
                let target = m.attacker.other();
                let xs: Vec<f32> = chicks.iter().filter(|c| c.team == target && c.alive()).map(|c| c.pos.x).collect();
                let (lo, hi) = tuning.side_range(target);
                let aim = rng.pick(&xs).unwrap_or((lo + hi) / 2.0);
                let err = (1.0 - quality) * 2.5 * (rng.f32() * 2.0 - 1.0);
                let tx = (aim + err).clamp(lo, hi);
                let e = commands.spawn((Attack::new(kind, m.attacker, quality, tx).upgraded(upgrade), DespawnOnExit(Screen::Level))).id();
                m.attack = Some(e);
                m.phase = Phase::Incoming;
                sfx.play("SFX_ATTACK_IFC_LAUNCH");
            } else if m.turn_timer <= 0.0 {
                sfx.play("SFX_ATTACK_IFC_CLOCK_END");
                let next_team = m.attacker.other();
                m.start_turn(next_team, &tuning);
            }
        }
        Phase::Incoming => {
            // Done once the attack and its offspring (cluster bombs) are gone.
            if m.attack.is_none_or(|e| attacks.get(e).is_err()) && !attacks.iter().any(|a| a.child) {
                m.attack = None;
                m.phase = Phase::Finish(0.0);
            }
        }
        Phase::Finish(t) => {
            if t + dt >= tuning.attack_finish_time {
                let next_team = m.attacker.other();
                m.start_turn(next_team, &tuning);
            } else {
                m.phase = Phase::Finish(t + dt);
            }
        }
        Phase::RoundOver(t) => {
            if t + dt >= ROUND_BREAK {
                // Clear the field and start the next round with a fresh spin.
                for e in chick_entities.iter().chain(attack_entities.iter()).chain(barriers.iter()) {
                    commands.entity(e).despawn();
                }
                spawn_teams(&mut commands, assets.root, &assets.chick, &tuning, &mut rng, &mut meshes, &mut materials, &mut images);
                let first = if rng.chance(0.5) { Team::Yellow } else { Team::Black };
                m.attacker = first;
                m.attack = None;
                m.pending = None;
                m.pending_special = None;
                m.round_winner = None;
                m.max_health = [0.0; 2];
                m.special_timer = rng.range(tuning.special_first);
                m.hud_anim = Some(format!("hud_clockStart{}", if first == Team::Yellow { "Yellow" } else { "Black" }));
                m.phase = Phase::Spin(0.0);
            } else {
                m.phase = Phase::RoundOver(t + dt);
            }
        }
        Phase::GameOver(t) => {
            m.phase = Phase::GameOver(t + dt);
            if t + dt > 6.0 {
                next.set(Screen::Arenas);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_hud(
    mut m: ResMut<Match>,
    lyt: Res<LayoutAssets>,
    chicks: Query<&Chick>,
    mut huds: Query<(&LayoutRoot, &mut Hud, &mut LayoutAnimator)>,
    panes: Query<&mut LayoutPane>,
    mut texts: Query<&mut TextPane>,
) {
    let Ok((root, mut hud, mut anim)) = huds.single_mut() else { return };
    hud.rounds_to_win = ROUNDS_TO_WIN;
    hud.rounds_won = m.rounds_won;
    for t in [Team::Yellow, Team::Black] {
        let hp: f32 = chicks.iter().filter(|c| c.team == t).map(|c| c.health.max(0.0)).sum();
        let max = &mut m.max_health[t.index()];
        *max = max.max(chicks.iter().filter(|c| c.team == t).map(|c| c.max_health).sum());
        hud.health[t.index()] = if *max > 0.0 { hp / *max } else { 1.0 };
    }
    if let Some(name) = m.hud_anim.take() {
        if let Ok(a) = lyt.animation(&name) {
            anim.play(a);
        }
    }
    let remain = if m.phase == Phase::Attack { m.turn_timer.max(0.0).ceil() as i32 } else { 0 };
    layout::set_text(root, "txtTimeRemain", &format!("{remain:02}"), &panes, &mut texts);
    let secs = m.match_time as i32;
    layout::set_text(root, "txtTimer", &format!("{:02}:{:02}", secs / 60, secs % 60), &panes, &mut texts);

}
