//! The Split mod (SL).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "SL",
    name: "Split",
    description: "Dons are 20% faster and kats 15% slower: colours overtake each other.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    ..ModDef::DEFAULT
};
