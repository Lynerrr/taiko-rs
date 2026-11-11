//! The Lag Spike mod (LG).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "LG",
    name: "Lag Spike",
    description: "The lane freezes now and then, then skips ahead. Like a bad stream.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    ..ModDef::DEFAULT
};
