//! The Wind Down mod (WD).

use super::{
    Mod::{Daycore, DoubleTime, HalfTime, Nightcore, WindDown, WindUp},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "WD",
    name: "Wind Down",
    description: "The song slows down from 1.0x to 0.75x of your speed.",
    multiplier: 0.85,
    shuffle_cost: Some(0),
    conflicts: &[HalfTime, Daycore, DoubleTime, Nightcore, WindUp, WindDown],
    lobby_wide: true,
    ..ModDef::DEFAULT
};
