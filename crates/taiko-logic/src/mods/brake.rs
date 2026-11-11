//! The Brake mod (BR).

use super::{Mod::Accel, ModDef};

pub(super) const DEF: ModDef = ModDef {
    acronym: "BR",
    name: "Brake",
    description: "Notes fly in fast and brake near the drum.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(1),
    conflicts: &[Accel],
    ..ModDef::DEFAULT
};
