//! The Great or Nothing mod (GN).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "GN",
    name: "Great or Nothing",
    description: "An OK counts as a miss (but doesn't fail you).",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: None,
    ..ModDef::DEFAULT
};
