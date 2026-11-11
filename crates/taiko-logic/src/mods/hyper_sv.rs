//! The Hyper SV mod (HS).

use super::{
    Mod::{AntiSv, ConstantSpeed},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "HS",
    name: "Hyper SV",
    description: "The map's speed changes are exaggerated: 1.5x becomes 2.25x.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    conflicts: &[AntiSv, ConstantSpeed],
    ..ModDef::DEFAULT
};
