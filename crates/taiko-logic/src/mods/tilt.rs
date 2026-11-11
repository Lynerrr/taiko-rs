//! The Tilt mod (TL).

use super::{Mod::Multiverse, ModDef};

pub(super) const DEF: ModDef = ModDef {
    acronym: "TL",
    name: "Tilt",
    description: "The lane rocks from side to side with the beat.",
    // A little harder: the lane moves or bends, the notes read as usual.
    multiplier: 1.02,
    shuffle_cost: Some(1),
    // Its playfields are small windows side by side: nothing may move them off their spots,
    // light only one of them or take its colours away.
    conflicts: &[Multiverse],
    ..ModDef::DEFAULT
};
