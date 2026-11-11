//! The Colorless mod (CL).

use super::{
    Mod::{Echo, Multiverse},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "CL",
    name: "Colorless",
    description: "Dons and kats look the same. Play by ear and memory.",
    multiplier: 1.1,
    shuffle_cost: Some(3),
    conflicts: &[Multiverse, Echo],
    obscures: true,
    ..ModDef::DEFAULT
};
