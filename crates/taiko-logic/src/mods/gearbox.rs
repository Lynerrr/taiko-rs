//! The Gearbox mod (GB).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "GB",
    name: "Gearbox",
    description: "Every beat of a bar shifts up a gear (+15%); the next bar starts over.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(1),
    ..ModDef::DEFAULT
};
