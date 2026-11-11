//! The No Finishers mod (NB).

use super::{Mod::AllBig, ModDef};

pub(super) const DEF: ModDef = ModDef {
    acronym: "NB",
    name: "No Finishers",
    description: "Big notes become normal ones.",
    multiplier: 0.9,
    shuffle_cost: Some(0),
    conflicts: &[AllBig],
    ..ModDef::DEFAULT
};
