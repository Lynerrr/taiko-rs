//! The Daycore mod (DC).

use super::{
    Mod::{Daycore, DoubleTime, HalfTime, Nightcore, WindDown, WindUp},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "DC",
    name: "Daycore",
    description: "0.75x speed, lower pitch.",
    multiplier: 0.3,
    shuffle_cost: Some(-2),
    conflicts: &[HalfTime, Daycore, DoubleTime, Nightcore, WindUp, WindDown],
    lobby_wide: true,
    ..ModDef::DEFAULT
};
