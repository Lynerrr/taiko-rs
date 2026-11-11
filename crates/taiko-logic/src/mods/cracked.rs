//! The Cracked mod (CK).

use super::{
    Mod::{Crossfire, Crumble},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "CK",
    name: "Cracked",
    description: "Every miss cracks the lane and knocks a piece out of line; 50 hits mend it.",
    // A little harder: the lane moves or bends, the notes read as usual.
    multiplier: 1.02,
    shuffle_cost: Some(1),
    conflicts: &[Crumble, Crossfire],
    ..ModDef::DEFAULT
};
