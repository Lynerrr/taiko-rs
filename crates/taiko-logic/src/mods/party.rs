//! The Party mod (PY).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "PY",
    name: "Party",
    description: "Everything flashy at once; notes pulse to their own rhythm, 1/4s four times a beat.",
    // A little harder: the lane moves or bends, the notes read as usual.
    multiplier: 1.02,
    shuffle_cost: Some(1),
    ..ModDef::DEFAULT
};
