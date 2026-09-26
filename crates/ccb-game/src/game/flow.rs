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
    gesture::{ui_sprite, Gestures, Trace},
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
    pointer::Pointer,
    Screen,
};

/// Chicks per team: one big, the rest small.
const CHICKS_PER_TEAM: usize = 5;
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
    /// An attack ready to launch (kind, quality), from the player or the CPU.
    pub pending: Option<(AttackKind, f32)>,
    pub attack: Option<Entity>,
    /// Team health when full (sum of the chicks' max health).
    pub max_health: [f32; 2],
    pub winner: Option<Team>,
    /// HUD animation to start on the next update.
    pub hud_anim: Option<String>,
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
    models: HashMap<String, Model>,
    widths: HashMap<String, f32>,
    textures: HashMap<String, (Handle<Image>, [u32; 2])>,
    clips: Vec<Arc<Clip>>,
    root: Entity,
}

fn model_width(m: &Model) -> f32 {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for p in m.meshes.iter().flat_map(|x| x.positions.iter()) {
        lo = lo.min(p[0]);
        hi = hi.max(p[0]);
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
        let Some(model) = self.models.get(name) else {
            return commands.spawn((Transform::default(), Visibility::default())).id();
        };
        let tex = self.textures.clone();
        let tweak = move |_: &wii_formats::mdl0::Mesh, _: &str, mat: &mut GxMaterial| {
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
        let e = g3d::spawn_model(commands, self.root, model, &self.textures, meshes, materials, images, &tweak);
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
        let name = format!("weight__weightDefault0{}", variant.clamp(1, 6));
        if let Some(c) = self.clips.iter().find(|c| c.name == name) {
            commands.entity(e).insert(BoneAnimator::new(vec![c.clone()]));
        }
        e
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

/// An attack button on the player's attack interface.
#[derive(Component)]
pub struct AttackButton {
    pub kind: AttackKind,
    pub center: Vec2,
    pub size: f32,
    /// Icon size relative to the button.
    pub icon_scale: f32,
}

fn button_icon(kind: AttackKind) -> &'static str {
    match kind {
        AttackKind::Bomb => "attackInterface_btn_bombe.tpl",
        AttackKind::Weight => "attackInterface_btn_gewicht.tpl",
        AttackKind::Plant => "attackInterface_btn_pflanze.tpl",
        AttackKind::Ufo => "attackInterface_icon_ufo.tpl",
        AttackKind::Lightning => "attackInterface_icon_lightning.tpl",
        AttackKind::Ghost => "attackInterface_icon_ghost.tpl",
        AttackKind::Octopus | AttackKind::Mushroom => "attackInterface_icon_octopus.tpl",
    }
}

#[allow(clippy::too_many_arguments)]
pub fn start_match(
    mut commands: Commands,
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
        for name in ["bomb", "weight", "explosion", "cloud", "cloudLightning", "plant"] {
            if let Ok(m) = common.model(name) {
                models.insert(name.to_string(), m);
            }
        }
        let widths = models.iter().map(|(n, m)| (n.clone(), model_width(m))).collect();
        let root = commands.spawn((Name::new("gameplay"), Transform::default(), Visibility::default(), DespawnOnExit(Screen::Level))).id();
        let chick_assets = ChickAssets::new(common.model("chick")?, textures.clone());

        // Chicks spread over each half.
        for team in [Team::Yellow, Team::Black] {
            let (lo, hi) = tuning.side_range(team);
            for i in 0..CHICKS_PER_TEAM {
                let x = lo + (hi - lo) * (i as f32 + 0.5) / CHICKS_PER_TEAM as f32 + rng.range((-0.4, 0.4));
                spawn_chick(&mut commands, root, &chick_assets, team, x, i == 1, &tuning, &mut meshes, &mut materials, &mut images);
            }
        }

        // The player's attack interface, laid out like the original's 1P screen: the three
        // basic attacks as round buttons bottom right, the level's special on the blue arc
        // above them, and the gesture panel bottom left.
        let special = available.iter().copied().find(|k| !k.is_basic() && k.implemented());
        let basics: Vec<AttackKind> = [AttackKind::Bomb, AttackKind::Weight, AttackKind::Plant].into_iter().filter(|k| available.contains(k)).collect();
        for (i, kind) in basics.iter().enumerate() {
            let center = Vec2::new(101.0 + 67.0 * i as f32, -158.0);
            let bg = ui_sprite(&mut commands, &mut lyt, "attackInterface_btnBg.tpl", center, Vec2::splat(64.0), 250.0, &mut meshes, &mut materials, &mut images);
            let icon = ui_sprite(&mut commands, &mut lyt, button_icon(*kind), center, Vec2::splat(64.0), 251.0, &mut meshes, &mut materials, &mut images);
            commands.entity(bg).insert(AttackButton { kind: *kind, center, size: 64.0, icon_scale: 1.0 });
            commands.entity(icon).insert(AttackButtonIcon(bg));
        }
        if let Some(kind) = special {
            let center = Vec2::new(170.0, -92.0);
            let theme = level.split('.').next().unwrap_or("city");
            let arc = match theme {
                "ship" => "attackInterface_special_shipLvl.tpl",
                "graveyard" => "attackInterface_special_ghostLvl.tpl",
                _ => "attackInterface_special_cityLvl.tpl",
            };
            let bg = ui_sprite(&mut commands, &mut lyt, arc, center, Vec2::new(110.0, 58.0), 250.0, &mut meshes, &mut materials, &mut images);
            let icon = ui_sprite(&mut commands, &mut lyt, button_icon(kind), center + Vec2::new(0.0, 4.0), Vec2::new(56.0, 46.0), 251.0, &mut meshes, &mut materials, &mut images);
            commands.entity(bg).insert(AttackButton { kind, center, size: 58.0, icon_scale: 0.8 });
            commands.entity(icon).insert(AttackButtonIcon(bg));
        }
        let panel = tuning.gesture_panel[Team::Yellow.index()];
        ui_sprite(&mut commands, &mut lyt, "attackInterface_btnBg.tpl", panel, Vec2::splat(176.0), 240.0, &mut meshes, &mut materials, &mut images);

        let first = if rng.chance(0.5) { Team::Yellow } else { Team::Black };
        let side = if first == Team::Yellow { "Yellow" } else { "Black" };
        info!("match on {level}: attacks {available:?}, {first:?} starts");
        commands.insert_resource(Gestures::load(&lyt)?);
        commands.insert_resource(Trace::default());
        commands.insert_resource(Cpu::load(&data)?);
        commands.insert_resource(Ink([tuning.ink_max; 2]));
        commands.insert_resource(Match {
            phase: Phase::Spin(0.0),
            attacker: first,
            turn_timer: tuning.attack_time,
            match_time: 0.0,
            available,
            selected: None,
            pending: None,
            attack: None,
            max_health: [0.0; 2],
            winner: None,
            hud_anim: Some(format!("hud_clockStart{side}")),
        });
        commands.insert_resource(GameAssets { models, widths, textures, clips, root });
        commands.insert_resource(tuning);
        Ok(())
    })();
    if let Err(e) = r {
        error!("failed to start match: {e:#}");
    }
}

/// Icon sprite belonging to an attack button (dimmed with it).
#[derive(Component)]
pub struct AttackButtonIcon(pub Entity);

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
    mut m: ResMut<Match>,
    mut rng: ResMut<Rng>,
    chicks: Query<&Chick>,
    attacks: Query<&Attack>,
    mut next: ResMut<NextState<Screen>>,
    mut sfx: ResMut<crate::sfx::Sfx>,
) {
    let dt = time.delta_secs();
    if !matches!(m.phase, Phase::GameOver(_)) {
        m.match_time += dt;
        let alive = |t: Team| chicks.iter().filter(|c| c.team == t && c.alive()).count();
        for t in [Team::Yellow, Team::Black] {
            if alive(t) == 0 && !chicks.iter().any(|c| c.team == t && c.dying.is_some()) {
                m.winner = Some(t.other());
                m.phase = Phase::GameOver(0.0);
                sfx.play(if t.other() == Team::Yellow { "SFX_GAME_WIN" } else { "SFX_GAME_OVER" });
                m.selected = None;
                info!("game over: {:?} wins", t.other());
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
            if let Some((kind, quality)) = m.pending.take() {
                // Aim at a living chick; a weaker trace aims worse.
                let target = m.attacker.other();
                let xs: Vec<f32> = chicks.iter().filter(|c| c.team == target && c.alive()).map(|c| c.pos.x).collect();
                let (lo, hi) = tuning.side_range(target);
                let aim = rng.pick(&xs).unwrap_or((lo + hi) / 2.0);
                let err = (1.0 - quality) * 2.5 * (rng.f32() * 2.0 - 1.0);
                let tx = (aim + err).clamp(lo, hi);
                let e = commands.spawn((Attack::new(kind, m.attacker, quality, tx), DespawnOnExit(Screen::Level))).id();
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
            if m.attack.is_none_or(|e| attacks.get(e).is_err()) {
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
        Phase::GameOver(t) => {
            m.phase = Phase::GameOver(t + dt);
            if t + dt > 6.0 {
                next.set(Screen::Title);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn player_attack_buttons(
    time: Res<Time>,
    pointer: Res<Pointer>,
    mut m: ResMut<Match>,
    buttons: Query<(Entity, &AttackButton)>,
    icons: Query<(Entity, &AttackButtonIcon)>,
    mut transforms: Query<(&mut Transform, &MeshMaterial3d<GxMaterial>)>,
    mut materials: ResMut<Assets<GxMaterial>>,
    mut hover: Local<HashMap<Entity, f32>>,
    mut sfx: ResMut<crate::sfx::Sfx>,
) {
    let my_turn = m.phase == Phase::Attack && m.attacker == Team::Yellow && m.pending.is_none();
    for (e, b) in &buttons {
        let usable = my_turn && b.kind.implemented() && m.selected.is_none();
        let over = pointer.pos.is_some_and(|p| p.distance(b.center) < b.size * 0.5);
        let h = hover.entry(e).or_insert(0.0);
        if over && usable && *h == 0.0 {
            sfx.play("SFX_ATTACK_IFC_SELECTOR_ROLLOVER");
        }
        *h = (*h + if over && usable { 10.0 } else { -10.0 } * time.delta_secs()).clamp(0.0, 1.0);
        let grow = (1.0 + 0.1 * *h) * if m.selected == Some(b.kind) { 1.12 } else { 1.0 };
        // Out of turn the icons grey out (the background stays solid).
        let icon_alpha = if usable || m.selected == Some(b.kind) { 1.0 } else { 0.45 };
        let mut apply = |ent: Entity, alpha: f32, icon: bool| {
            if let Ok((mut t, mat)) = transforms.get_mut(ent) {
                let s = t.scale.truncate() / t.scale.y.max(1e-3); // keep aspect
                let base = if icon { b.size * b.icon_scale } else { b.size };
                let size = Vec2::new(base * s.x, base) * grow;
                let off = if icon && b.icon_scale < 1.0 { 4.0 } else { 0.0 };
                *t = Transform::from_xyz(b.center.x - size.x / 2.0, b.center.y + off + size.y / 2.0, t.translation.z).with_scale(size.extend(1.0));
                if let Some(mut mm) = materials.get_mut(&mat.0) {
                    if mm.params.konst[3].w != alpha {
                        mm.params.konst[3].w = alpha;
                    }
                }
            }
        };
        apply(e, 1.0, false);
        for (ie, icon) in &icons {
            if icon.0 == e {
                apply(ie, icon_alpha, true);
            }
        }
        if over && usable && pointer.just_pressed {
            info!("player selects {:?}", b.kind);
            sfx.play("SFX_ATTACK_IFC_SELECTOR_ACTIVATED");
            m.selected = Some(b.kind);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_hud(
    mut m: ResMut<Match>,
    data: Res<GameData>,
    lyt: Res<LayoutAssets>,
    chicks: Query<&Chick>,
    mut huds: Query<(&LayoutRoot, &mut Hud, &mut LayoutAnimator)>,
    mut panes: Query<&mut LayoutPane>,
    mut texts: Query<&mut TextPane>,
    mut shown_over: Local<bool>,
) {
    let Ok((root, mut hud, mut anim)) = huds.single_mut() else { return };
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
    if let (Phase::GameOver(_), Some(w), false) = (m.phase, m.winner, *shown_over) {
        *shown_over = true;
        let msg = |k: &str, d: &str| layout::display_text(data.msgs.get(k).unwrap_or(d));
        let text = if w == Team::Yellow { msg("CCB_INGAME_WIN", "YOU WIN!") } else { msg("CCB_INGAME_LOSE", "YOU LOSE!") };
        if let Some(mut p) = root.pane("countdown").and_then(|e| panes.get_mut(e).ok()) {
            p.cur.alpha = 1.0;
            p.cur.scale = Vec2::splat(1.0);
        }
        layout::set_text(root, "txtCountdown", &text, &panes, &mut texts);
        layout::set_text(root, "txtBgCountdown", &text, &panes, &mut texts);
    }
    if !matches!(m.phase, Phase::GameOver(_)) {
        *shown_over = false;
    }
}
