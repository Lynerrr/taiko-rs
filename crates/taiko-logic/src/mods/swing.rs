//! The Swing mod (SG).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "SG",
    name: "Swing",
    description: "Notes on the beat are faster, notes between beats slower.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(1),
    ..ModDef::DEFAULT
};
