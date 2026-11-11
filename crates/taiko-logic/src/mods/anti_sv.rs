//! The Anti-SV mod (AS).

use super::{
    Mod::{ConstantSpeed, HyperSv},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "AS",
    name: "Anti-SV",
    description: "The map's speed changes are turned upside down: 2x becomes 0.5x.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(1),
    conflicts: &[HyperSv, ConstantSpeed],
    ..ModDef::DEFAULT
};
