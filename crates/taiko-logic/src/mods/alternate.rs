//! The Alternate mod (AL).

use super::{
    Mod::{Autoplay, Relax},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "AL",
    name: "Alternate",
    description: "Hands must alternate; either may follow a finisher, drumroll or swell.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    conflicts: &[Relax, Autoplay],
    ..ModDef::DEFAULT
};
