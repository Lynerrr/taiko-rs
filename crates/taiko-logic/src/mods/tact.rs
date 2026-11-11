//! The Tact mod (TC).

use super::{
    Mod::{Pulse, Stutter},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "TC",
    name: "Tact",
    description: "Notes crawl through each bar, then rush to where they belong (C: how often).",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    conflicts: &[Pulse, Stutter],
    setup_tab: Some(3),
    ..ModDef::DEFAULT
};
