//! The Half Time mod (HT).

use super::{
    Mod::{Daycore, DoubleTime, HalfTime, Nightcore, WindDown, WindUp},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "HT",
    name: "Half Time",
    description: "0.75x speed, same pitch.",
    multiplier: 0.3,
    shuffle_cost: Some(-2),
    conflicts: &[HalfTime, Daycore, DoubleTime, Nightcore, WindUp, WindDown],
    lobby_wide: true,
    ..ModDef::DEFAULT
};
