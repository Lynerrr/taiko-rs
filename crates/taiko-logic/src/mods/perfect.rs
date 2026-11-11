//! The Perfect mod (PF).

use super::{
    Mod::{Autoplay, NoFail, Relax, SuddenDeath},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "PF",
    name: "Perfect",
    description: "Anything but a GREAT fails the play.",
    multiplier: 1.0,
    shuffle_cost: None,
    conflicts: &[NoFail, SuddenDeath, Relax, Autoplay],
    in_defense: false,
    ..ModDef::DEFAULT
};
