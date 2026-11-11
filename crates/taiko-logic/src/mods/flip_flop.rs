//! The Flip Flop mod (FF).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "FF",
    name: "Flip Flop",
    description: "Every 8 bars the lane flips over to the other side.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(2),
    ..ModDef::DEFAULT
};
