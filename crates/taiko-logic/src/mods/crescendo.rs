//! The Crescendo mod (CC).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "CC",
    name: "Crescendo",
    description: "Notes speed up steadily over the map, from 0.8x to 1.6x.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(1),
    ..ModDef::DEFAULT
};
