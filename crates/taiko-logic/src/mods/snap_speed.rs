//! The Snap Speed mod (SR).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "SR",
    name: "Snap Speed",
    description: "Speed by rhythm: 1/3s slower, 1/6s faster, 1/8s a bit faster than the rest.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(1),
    ..ModDef::DEFAULT
};
