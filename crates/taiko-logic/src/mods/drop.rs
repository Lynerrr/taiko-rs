//! The Drop mod (DR).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "DR",
    name: "Drop",
    description: "Notes freeze two beats before every kiai, then slam in 1.4x faster.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(1),
    ..ModDef::DEFAULT
};
