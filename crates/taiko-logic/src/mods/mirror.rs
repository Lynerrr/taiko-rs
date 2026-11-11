//! The Mirror mod (MR).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "MR",
    name: "Mirror",
    description: "The lane is flipped: notes come from the left.",
    // A little harder: the lane moves or bends, the notes read as usual.
    multiplier: 1.02,
    shuffle_cost: Some(1),
    ..ModDef::DEFAULT
};
