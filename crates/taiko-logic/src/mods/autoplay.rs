//! The Autoplay mod (AT).

use super::{
    Mod::{Alternate, NoFail, Perfect, Relax, SuddenDeath},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "AT",
    name: "Autoplay",
    description: "Watch the map play itself.",
    multiplier: 1.0,
    shuffle_cost: None,
    conflicts: &[Relax, NoFail, SuddenDeath, Perfect, Alternate],
    in_defense: false,
    ..ModDef::DEFAULT
};
