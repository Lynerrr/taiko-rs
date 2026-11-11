//! The Wave mod (WV).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "WV",
    name: "Wave",
    description: "Note speed swells and falls along the song.",
    // A little harder: the lane moves or bends, the notes read as usual.
    multiplier: 1.02,
    shuffle_cost: Some(1),
    ..ModDef::DEFAULT
};
