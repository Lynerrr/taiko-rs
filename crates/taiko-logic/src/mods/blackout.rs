//! The Blackout mod (BO).

use super::{
    Mod::{Multiverse, Radar},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "BO",
    name: "Blackout",
    description: "Kiai time turns the lights off: only the drum stays lit.",
    multiplier: 1.1,
    shuffle_cost: Some(1),
    conflicts: &[Multiverse, Radar],
    ..ModDef::DEFAULT
};
