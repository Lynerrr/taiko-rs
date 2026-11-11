//! The Barrel Roll mod (BA).

use super::{Mod::Multiverse, ModDef};

pub(super) const DEF: ModDef = ModDef {
    acronym: "BA",
    name: "Barrel Roll",
    description: "Every kiai starts with a full spin of the lane.",
    // A little harder: the lane moves or bends, the notes read as usual.
    multiplier: 1.02,
    shuffle_cost: Some(1),
    // Its playfields are small windows side by side: nothing may move them off their spots,
    // light only one of them or take its colours away.
    conflicts: &[Multiverse],
    ..ModDef::DEFAULT
};
