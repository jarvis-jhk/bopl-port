//! The logical `Player` record (player.md §3, match-flow.md §3).

use crate::fix::Fix;
use crate::input::InputProfile;
use crate::object::ObjId;
use crate::vec2::Vec2;

/// `CauseOfDeath`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CauseOfDeath {
    #[default]
    Other = 0,
    Squished = 1,
    Accident = 2,
    Eaten = 3,
    Beamed = 4,
    NotDeadYet = 5,
}

/// `ReviveStatus`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ExtraLife {
    #[default]
    NoRevive = 0,
    WillReviveOnDeath = 1,
    IsBeingRevived = 2,
    PermanentRevive = 3,
}

/// Session-level choices of a player (inputs to the match).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlayerSetup {
    /// 1-based id.
    pub id: i32,
    /// Team 0..11.
    pub team: i32,
    /// Player colour 0..11 (presentation).
    pub color: i32,
    /// Ability-list indices per slot (1 = Random).
    pub abilities: Vec<usize>,
    pub uses_keyboard_and_mouse: bool,
}

/// One participant.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Player {
    pub id: i32,
    pub team: i32,
    pub color: i32,
    pub uses_mouse: bool,
    /// Chosen ability indices (`Abilities`).
    pub abilities: Vec<usize>,
    /// Resolved this round (`CurrentAbilities`), one per slot.
    pub current_abilities: Vec<usize>,
    pub inputs: InputProfile,
    scale: Fix,
    pub players_and_clones_still_alive: i32,
    pub kills: i32,
    pub deaths: i32,
    pub kills_at_start_of_round: i32,
    pub games_won: i32,
    pub won_this_round: bool,
    pub is_most_recent_winner: bool,
    pub timestamp_when_killed: i64,
    pub cause_of_death: CauseOfDeath,
    pub still_alive_this_round: bool,
    pub extra_life: ExtraLife,
    /// `RespawnPositions`: the revive markers (RevivePositionIndicator objects).
    pub respawn_positions: Vec<ObjId>,
    /// `ReviveInstance`: the Revival ability object that placed the last marker.
    pub revive_instance: Option<ObjId>,
    pub spawn_position: Vec2,
    pub is_protected_from_time_stop: bool,
    pub is_invisible: bool,
    pub time_spent_invisible: Fix,
    pub time_since_entered_ability: Fix,
    pub can_use_abilities: bool,
    pub in_spark_shot: bool,
    pub in_platform_transform: bool,
    pub in_machine_throw: bool,
    pub custom_aim_ability_position: Vec2,
    /// `Position` (reference position for aiming and the camera).
    pub position: Vec2,
    pub players_eaten_count_this_round: i32,
    /// Position-setter bookkeeping: (this tick, last tick).
    pub setter_this_frame: Option<ObjId>,
    pub setter_last_frame: Option<ObjId>,
    /// The bopl object of this round (if spawned).
    pub bopl: Option<ObjId>,
    /// Becomes true on the tick visibility returned (`invisibility ended`).
    pub became_visible: bool,
}

impl Player {
    pub fn new(s: &PlayerSetup) -> Player {
        Player {
            id: s.id,
            team: s.team,
            color: s.color,
            uses_mouse: s.uses_keyboard_and_mouse,
            abilities: s.abilities.clone(),
            current_abilities: Vec::new(),
            inputs: InputProfile {
                uses_mouse: s.uses_keyboard_and_mouse,
                ..InputProfile::default()
            },
            scale: Fix::ONE,
            players_and_clones_still_alive: 0,
            kills: 0,
            deaths: 0,
            kills_at_start_of_round: 0,
            games_won: 0,
            won_this_round: false,
            is_most_recent_winner: false,
            timestamp_when_killed: i64::MAX,
            cause_of_death: CauseOfDeath::NotDeadYet,
            still_alive_this_round: true,
            extra_life: ExtraLife::NoRevive,
            respawn_positions: Vec::new(),
            revive_instance: None,
            spawn_position: Vec2::ZERO,
            is_protected_from_time_stop: false,
            is_invisible: false,
            time_spent_invisible: Fix::ZERO,
            time_since_entered_ability: Fix::ZERO,
            can_use_abilities: true,
            in_spark_shot: false,
            in_platform_transform: false,
            in_machine_throw: false,
            custom_aim_ability_position: Vec2::ZERO,
            position: Vec2::ZERO,
            players_eaten_count_this_round: 0,
            setter_this_frame: None,
            setter_last_frame: None,
            bopl: None,
            became_visible: false,
        }
    }

    /// `IsAlive`.
    pub fn is_alive(&self) -> bool {
        self.players_and_clones_still_alive > 0
    }

    /// `Scale` (clamped on write to `[(Fix)0.3, 3]`).
    pub fn scale(&self) -> Fix {
        self.scale
    }

    pub fn set_scale(&mut self, s: Fix) {
        self.scale = s.clamp(Fix(1_288_490_188), Fix::from_int(3));
    }

    /// Per-level reset before a level loads (match-flow.md §6 step 1).
    pub fn reset_for_level(&mut self) {
        self.position = Vec2::ZERO;
        self.set_scale(Fix::ONE);
        self.still_alive_this_round = true;
        self.in_spark_shot = false;
        self.is_invisible = false;
        self.time_spent_invisible = Fix::ZERO;
        self.players_eaten_count_this_round = 0;
        self.respawn_positions.clear();
        self.is_protected_from_time_stop = false;
        self.bopl = None;
        self.setter_this_frame = None;
        self.setter_last_frame = None;
    }
}
