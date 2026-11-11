//! The Constant Speed mod (CS).

use super::{
    Mod::{AntiSv, HyperSv},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "CS",
    name: "Constant Speed",
    description: "All notes scroll at one speed: no SV changes.",
    multiplier: 0.9,
    shuffle_cost: Some(0),
    // The map's own speeds are what these bend, so they don't mix with each other or with one
    // speed for everything.
    conflicts: &[AntiSv, HyperSv],
    ..ModDef::DEFAULT
};
