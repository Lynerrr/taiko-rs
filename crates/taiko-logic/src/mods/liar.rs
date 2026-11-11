//! The Liar mod (LI).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "LI",
    name: "Liar",
    description: "Far off, triplets are spaced like 1/4s; their real spacing shows near the drum.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(1),
    obscures: true,
    ..ModDef::DEFAULT
};
