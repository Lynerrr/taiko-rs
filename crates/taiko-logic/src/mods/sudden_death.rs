//! The Sudden Death mod (SD).

use super::{
    Mod::{Autoplay, NoFail, Perfect, Relax},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "SD",
    name: "Sudden Death",
    description: "The first miss fails the play.",
    multiplier: 1.0,
    shuffle_cost: None,
    conflicts: &[NoFail, Perfect, Relax, Autoplay],
    in_defense: false,
    ..ModDef::DEFAULT
};
