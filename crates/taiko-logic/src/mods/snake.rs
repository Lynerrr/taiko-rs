//! The Snake mod (SK).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "SK",
    name: "Snake",
    description: "Streams slither along the lane, triplets the other way; 1/1 notes fly straight.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(1),
    ..ModDef::DEFAULT
};
