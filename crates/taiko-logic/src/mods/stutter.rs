//! The Stutter mod (ST).

use super::{
    Mod::{Pulse, Tact},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "ST",
    name: "Stutter",
    description: "Notes freeze for half a beat, then catch up.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    conflicts: &[Pulse, Tact],
    ..ModDef::DEFAULT
};
