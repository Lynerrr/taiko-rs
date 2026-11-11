//! The Shatter mod (SH).

use super::{Mod::Crossfire, ModDef};

pub(super) const DEF: ModDef = ModDef {
    acronym: "SH",
    name: "Shatter",
    description: "The lane breaks into pieces that slide apart and back; further where streams are.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(1),
    conflicts: &[Crossfire],
    ..ModDef::DEFAULT
};
