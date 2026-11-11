//! The Moonwalk mod (MW).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "MW",
    name: "Moonwalk",
    description: "On every bar the notes slide backwards for a moment.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    ..ModDef::DEFAULT
};
