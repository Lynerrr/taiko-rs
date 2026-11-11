//! The Memory mod (MM).

use super::{
    Mod::{Hidden, Radar, Sudden},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "MM",
    name: "Memory",
    description: "Notes are only shown for the first 15 seconds.",
    multiplier: 1.1,
    shuffle_cost: None,
    conflicts: &[Hidden, Sudden, Radar],
    ..ModDef::DEFAULT
};
