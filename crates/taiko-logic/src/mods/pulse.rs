//! The Pulse mod (PU).

use super::{
    Mod::{Stutter, Tact},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "PU",
    name: "Pulse",
    description: "Notes jump forward on every beat.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(1),
    // They all move notes in steps with the beat: one at a time.
    conflicts: &[Stutter, Tact],
    ..ModDef::DEFAULT
};
