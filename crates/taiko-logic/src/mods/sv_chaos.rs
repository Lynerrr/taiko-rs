//! The SV Chaos mod (CH).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "CH",
    name: "SV Chaos",
    description: "Every note gets its own speed (+-30%): they overtake each other.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.08,
    shuffle_cost: Some(2),
    ..ModDef::DEFAULT
};
