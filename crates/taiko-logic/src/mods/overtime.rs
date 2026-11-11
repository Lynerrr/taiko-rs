//! The Overtime mod (OT).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "OT",
    name: "Overtime",
    description: "It's me.",
    multiplier: 1.0,
    shuffle_cost: None,
    ..ModDef::DEFAULT
};
