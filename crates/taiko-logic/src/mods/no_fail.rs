//! The No Fail mod (NF).

use super::{
    Mod::{Autoplay, Perfect, Relax, SuddenDeath},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "NF",
    name: "No Fail",
    description: "You can't fail, whatever the HP bar says.",
    multiplier: 0.5,
    shuffle_cost: Some(0),
    conflicts: &[SuddenDeath, Perfect, Relax, Autoplay],
    in_defense: false,
    ..ModDef::DEFAULT
};
