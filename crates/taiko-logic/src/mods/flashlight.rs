//! The Flashlight mod (FL).

use super::{
    Mod::{Multiverse, Radar, Spotlight},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "FL",
    name: "Flashlight",
    description: "Only a circle around the drum is lit; it shrinks at 100 and 200 combo.",
    multiplier: 1.12,
    shuffle_cost: Some(3),
    conflicts: &[Spotlight, Multiverse, Radar],
    obscures: true,
    ..ModDef::DEFAULT
};
