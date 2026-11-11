//! The Half Notes mod (HA).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "HA",
    name: "Half Notes",
    description: "Streams lose their finest notes, on the beat. For learning hard maps.",
    multiplier: 0.5,
    shuffle_cost: Some(-3),
    ..ModDef::DEFAULT
};
