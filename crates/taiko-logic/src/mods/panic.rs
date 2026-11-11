//! The Panic mod (PN).

use super::{Mod::Mercy, ModDef};

pub(super) const DEF: ModDef = ModDef {
    acronym: "PN",
    name: "Panic",
    description: "Every miss makes notes 15% faster for 4 seconds; misses stack.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(2),
    conflicts: &[Mercy],
    ..ModDef::DEFAULT
};
