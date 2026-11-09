//! Defense, the team mode: the rules the host picks, what the team can buy and what it costs,
//! what a player does in a match, and the team's state as the room keeps it. They go over the
//! network as they are (serde).

use serde::{Deserialize, Serialize};

/// Defense's rules, picked by the host: the team's lives, how many bars each turn lasts, how
/// many turns each player may take over in a map, whether the team can buy things, and
/// whether it plays on the Maze (or each player's own lane).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Defense {
    pub lives: u32,
    pub bars: u32,
    #[serde(default = "two")]
    pub tag_ins: u32,
    #[serde(default = "yes")]
    pub shop: bool,
    #[serde(default = "yes")]
    pub maze: bool,
}

fn two() -> u32 {
    2
}

fn yes() -> bool {
    true
}

impl Defense {
    /// The lives the host steps through (C in the lobby), the turn lengths (V) and the tag-ins
    /// each player has per map (0: none).
    pub const LIVES: [u32; 4] = [10, 25, 50, 100];
    pub const BARS: [u32; 4] = [4, 8, 16, 32];
    pub const TAG_INS: [u32; 5] = [0, 1, 2, 3, 5];
    pub const DEFAULT: Defense = Defense {
        lives: 25,
        bars: 8,
        tag_ins: 2,
        shop: true,
        maze: true,
    };
}

/// Defense: what the team can buy with its gold (keys 1-5 in a match).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefenseItem {
    /// The lane crawls at half speed for a while (the notes still land on time).
    Slow,
    /// The next `SHIELD_CHARGES` notes to get through cost no lives.
    Shield,
    /// A life back.
    Heal,
    /// A tower by the drum that shoots down every don for `TOWER_BARS` bars (they count as hit).
    DonTower,
    /// The same for every kat.
    KatTower,
}

impl DefenseItem {
    pub const ALL: [DefenseItem; 5] = [
        DefenseItem::Slow,
        DefenseItem::Shield,
        DefenseItem::Heal,
        DefenseItem::DonTower,
        DefenseItem::KatTower,
    ];

    /// What it costs the team now. A life (and so a shield) is worth more the fewer the team
    /// started with, and every heal and shield costs more than the one before: with 10 lives a
    /// heal is 200 gold, then 320, 510...; with 50 it starts at 40. Towers start at 120 and
    /// cost half as much again each time.
    pub fn price(self, team: &TeamState) -> u32 {
        let scale = 25.0 / team.max.max(1) as f64;
        let to5 = |g: f64| ((g / 5.0).round() * 5.0).max(5.0) as u32;
        match self {
            DefenseItem::Slow => 40,
            DefenseItem::Shield => to5((60.0 * scale).max(15.0) * 1.6f64.powi(team.shields as i32)),
            DefenseItem::Heal => to5(80.0 * scale * 1.6f64.powi(team.heals as i32)),
            DefenseItem::DonTower | DefenseItem::KatTower => {
                to5(120.0 * 1.5f64.powi(team.tower_buys as i32))
            }
        }
    }
}

/// Defense: notes a shield takes, and the bars a tower shoots for.
pub const SHIELD_CHARGES: u32 = 3;
pub const TOWER_BARS: usize = 4;

/// Defense: something a player did in the match, which every player's game then applies the
/// same way. `at` is the map time it was done at (on the doer's clock).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum DefenseAct {
    Buy {
        item: DefenseItem,
        at: f64,
        /// A tower: the map time it stops shooting at (the buyer's barlines); 0 for the rest.
        /// The room keeps one tower up at a time by it.
        #[serde(default)]
        until: f64,
    },
    /// Takes over the rest of the turn being played, from the next barline.
    TagIn { at: f64 },
}

/// Defense: the team's lives, gold, shield and towers, as the room keeps them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TeamState {
    pub lives: u32,
    pub max: u32,
    pub gold: u32,
    pub shield: u32,
    pub towers: u32,
    /// Heals bought (each costs more).
    #[serde(default)]
    pub heals: u32,
    /// Shields bought (each costs more).
    #[serde(default)]
    pub shields: u32,
    /// Towers bought (each costs more).
    #[serde(default)]
    pub tower_buys: u32,
}
