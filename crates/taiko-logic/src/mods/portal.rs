//! The Portal mod (PT).

use super::{Mod::Crossfire, ModDef};

pub(super) const DEF: ModDef = ModDef {
    acronym: "PT",
    name: "Portal",
    description: "Notes dive into one portal and come out of another; they move every 8 bars.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    conflicts: &[Crossfire],
    ..ModDef::DEFAULT
};
