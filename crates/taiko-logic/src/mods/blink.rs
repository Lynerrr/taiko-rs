//! The Blink mod (BL).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "BL",
    name: "Blink",
    description: "Approaching notes vanish on the second half of every beat.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    obscures: true,
    ..ModDef::DEFAULT
};
