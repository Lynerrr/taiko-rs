//! The Random mod (RD).

use super::{Mod::Swap, ModDef};

pub(super) const DEF: ModDef = ModDef {
    acronym: "RD",
    name: "Random",
    description: "Every note gets a random colour.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(2),
    conflicts: &[Swap],
    setup_tab: Some(0),
    ..ModDef::DEFAULT
};
