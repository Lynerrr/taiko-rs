//! The Precise mod (PR).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "PR",
    name: "Precise",
    description: "OD +2.",
    multiplier: 1.06,
    shuffle_cost: Some(1),
    ..ModDef::DEFAULT
};
