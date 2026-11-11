//! The Sudden mod (SU).

use super::{
    Mod::{Hidden, Memory, Radar},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "SU",
    name: "Sudden",
    description: "Notes only show up close to the drum.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    conflicts: &[Hidden, Memory, Radar],
    obscures: true,
    ..ModDef::DEFAULT
};
