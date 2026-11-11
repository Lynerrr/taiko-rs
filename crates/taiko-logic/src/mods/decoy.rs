//! The Decoy mod (DY).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "DY",
    name: "Decoy",
    description: "See-through fake notes fly in between the real ones and vanish before the drum.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    obscures: true,
    ..ModDef::DEFAULT
};
