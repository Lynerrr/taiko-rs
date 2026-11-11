//! The Crumble mod (CR).

use super::{
    Mod::{Cracked, Crossfire},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "CR",
    name: "Crumble",
    description: "Hits knock pieces out of the lane, harder as the combo grows; a miss puts them back.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(1),
    conflicts: &[Cracked, Crossfire],
    ..ModDef::DEFAULT
};
