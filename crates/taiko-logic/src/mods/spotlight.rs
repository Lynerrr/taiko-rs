//! The Spotlight mod (SP).

use super::{
    Mod::{Flashlight, Multiverse, Radar},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "SP",
    name: "Spotlight",
    description: "Flashlight whose circle breathes with the beat.",
    multiplier: 1.1,
    shuffle_cost: Some(3),
    conflicts: &[Flashlight, Multiverse, Radar],
    obscures: true,
    ..ModDef::DEFAULT
};
