//! The Accel mod (AC).

use super::{Mod::Brake, ModDef};

pub(super) const DEF: ModDef = ModDef {
    acronym: "AC",
    name: "Accel",
    description: "Notes start slow and rush in near the drum.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(1),
    conflicts: &[Brake],
    ..ModDef::DEFAULT
};
