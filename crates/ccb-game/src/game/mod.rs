//! Gameplay: two chick teams on either side of the fence take turns attacking.
//!
//! A match starts with the clock arrow spinning to pick the first attacker. On a turn the
//! attacker picks an attack and traces its gesture on the blueprint panel (the better and
//! faster the trace, the stronger the attack). The attack then flies at the other team,
//! whose player draws ink barriers to block it. The turn then passes to the other side.
//! A team loses when all of its chicks are gone.
//!
//! Tuning comes from the game's `cfg` files (see [`tuning::Tuning`]). The player controls the
//! yellow team (left); the CPU ([`ai`]) controls black.

pub mod ai;
pub mod attack;
pub mod barrier;
pub mod chick;
pub mod flow;
pub mod gesture;
pub mod rng;
pub mod tuning;

use bevy::prelude::*;

use crate::Screen;

/// The two sides. Yellow plays on the left (x < 0), black on the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Team {
    Yellow = 0,
    Black = 1,
}

impl Team {
    pub fn other(self) -> Team {
        match self {
            Team::Yellow => Team::Black,
            Team::Black => Team::Yellow,
        }
    }

    /// -1 for the left side, +1 for the right.
    pub fn side(self) -> f32 {
        match self {
            Team::Yellow => -1.0,
            Team::Black => 1.0,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

}

/// Attack types, numbered like the game's `GESTURE_DEF_*` constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttackKind {
    Weight = 0,
    Bomb = 1,
    Lightning = 2,
    Plant = 3,
    Ufo = 4,
    Ghost = 5,
    Octopus = 6,
    Mushroom = 7,
}

impl AttackKind {
    pub fn from_id(id: i32) -> Option<Self> {
        Some(match id {
            0 => Self::Weight,
            1 => Self::Bomb,
            2 => Self::Lightning,
            3 => Self::Plant,
            4 => Self::Ufo,
            5 => Self::Ghost,
            6 => Self::Octopus,
            7 => Self::Mushroom,
            _ => return None,
        })
    }

    /// Name of the gesture group in the `blueprints` layout.
    pub fn gesture_group(self) -> &'static str {
        match self {
            Self::Weight => "weight",
            Self::Bomb => "bomb",
            Self::Lightning => "lightning",
            Self::Plant => "plant",
            Self::Ufo => "ufo",
            Self::Ghost => "ghost",
            Self::Octopus => "octopus",
            Self::Mushroom => "mushroom",
        }
    }

    /// Attacks this port can play so far.
    pub fn implemented(self) -> bool {
        matches!(self, Self::Weight | Self::Bomb | Self::Lightning | Self::Plant | Self::Ufo | Self::Octopus | Self::Ghost)
    }

    /// Bomb, weight and plant are every level's basic attacks; the rest are level specials.
    pub fn is_basic(self) -> bool {
        matches!(self, Self::Weight | Self::Bomb | Self::Plant)
    }
}

/// Marks the 3D camera used for gameplay picking.
#[derive(Component)]
pub struct WorldCamera;

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<rng::Rng>()
            .add_systems(OnEnter(Screen::Level), flow::start_match.after(crate::level::spawn_level))
            .add_systems(OnExit(Screen::Level), flow::end_match)
            .add_systems(Update, flow::showcase.run_if(in_state(Screen::Level)))
            .add_systems(
                Update,
                (
                    flow::run_match,
                    gesture::player_gesture,
                    flow::player_attack_buttons,
                    ai::cpu_turn,
                    barrier::player_draw_barrier,
                    barrier::update_barriers,
                    chick::move_chicks,
                    attack::update_attacks,
                    flow::update_hud,
                )
                    .chain()
                    .run_if(in_state(Screen::Level).and_then(resource_exists::<flow::Match>)),
            );
    }
}
