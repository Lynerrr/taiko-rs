//! The Roulette mod (RL).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "RL",
    name: "Roulette",
    description: "Every 4 bars a random speed (0.5x-2x); a marker shows where it changes.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    ..ModDef::DEFAULT
};
