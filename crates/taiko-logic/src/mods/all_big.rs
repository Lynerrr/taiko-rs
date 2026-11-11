//! The All Big mod (AB).

use super::{Mod::NoFinishers, ModDef};

pub(super) const DEF: ModDef = ModDef {
    acronym: "AB",
    name: "All Big",
    description: "Every don and kat is a big note.",
    // A little harder: the lane moves or bends, the notes read as usual.
    multiplier: 1.02,
    shuffle_cost: Some(1),
    conflicts: &[NoFinishers],
    ..ModDef::DEFAULT
};
