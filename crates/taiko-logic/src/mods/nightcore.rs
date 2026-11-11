//! The Nightcore mod (NC).

use super::{
    Mod::{Daycore, DoubleTime, HalfTime, Nightcore, WindDown, WindUp},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "NC",
    name: "Nightcore",
    description: "1.5x speed, higher pitch.",
    multiplier: 1.12,
    shuffle_cost: Some(3),
    conflicts: &[HalfTime, Daycore, DoubleTime, Nightcore, WindUp, WindDown],
    lobby_wide: true,
    ..ModDef::DEFAULT
};
