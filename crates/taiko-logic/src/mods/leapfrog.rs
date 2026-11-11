//! The Leapfrog mod (LF).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "LF",
    name: "Leapfrog",
    description: "Every other note is faster: pairs swap places on the way in.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    ..ModDef::DEFAULT
};
