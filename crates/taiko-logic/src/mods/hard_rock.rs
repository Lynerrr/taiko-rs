//! The Hard Rock mod (HR).

use super::{Mod::Easy, ModDef};

pub(super) const DEF: ModDef = ModDef {
    acronym: "HR",
    name: "Hard Rock",
    description: "Tighter windows (OD x1.4), much faster notes, a stricter HP bar.",
    multiplier: 1.06,
    shuffle_cost: Some(2),
    conflicts: &[Easy],
    ..ModDef::DEFAULT
};
