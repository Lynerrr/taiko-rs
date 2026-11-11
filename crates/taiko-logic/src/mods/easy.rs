//! The Easy mod (EZ).

use super::{Mod::HardRock, ModDef};

pub(super) const DEF: ModDef = ModDef {
    acronym: "EZ",
    name: "Easy",
    description: "Wider timing windows (OD x0.5), slower notes, a kinder HP bar.",
    multiplier: 0.5,
    shuffle_cost: Some(-2),
    conflicts: &[HardRock],
    ..ModDef::DEFAULT
};
