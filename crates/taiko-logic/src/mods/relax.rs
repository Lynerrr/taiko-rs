//! The Relax mod (RX).

use super::{
    Mod::{Alternate, Autoplay, NoFail, Perfect, SuddenDeath},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "RX",
    name: "Relax",
    description: "Colours don't matter: any key hits any note. Has its own leaderboards and pp.",
    multiplier: 1.0,
    shuffle_cost: None,
    conflicts: &[Autoplay, NoFail, SuddenDeath, Perfect, Alternate],
    in_defense: false,
    ..ModDef::DEFAULT
};
