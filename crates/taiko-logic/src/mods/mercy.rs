//! The Mercy mod (MC).

use super::{Mod::Panic, ModDef};

pub(super) const DEF: ModDef = ModDef {
    acronym: "MC",
    name: "Mercy",
    description: "Every miss slows notes down for a few seconds.",
    multiplier: 0.9,
    shuffle_cost: Some(0),
    conflicts: &[Panic],
    ..ModDef::DEFAULT
};
