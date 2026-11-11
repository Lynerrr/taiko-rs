//! The Swap mod (SW).

use super::{Mod::Random, ModDef};

pub(super) const DEF: ModDef = ModDef {
    acronym: "SW",
    name: "Swap",
    description: "Dons become kats and kats become dons.",
    multiplier: 1.0,
    shuffle_cost: Some(1),
    conflicts: &[Random],
    ..ModDef::DEFAULT
};
