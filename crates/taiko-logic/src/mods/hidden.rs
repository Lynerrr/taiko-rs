//! The Hidden mod (HD).

use super::{
    Mod::{Memory, Radar, Sudden},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "HD",
    name: "Hidden",
    description: "Notes fade out long before they reach the drum.",
    multiplier: 1.06,
    shuffle_cost: Some(2),
    conflicts: &[Sudden, Memory, Radar],
    obscures: true,
    ..ModDef::DEFAULT
};
