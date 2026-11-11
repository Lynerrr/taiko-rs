//! The OD Override mod (OV).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "OV",
    name: "OD Override",
    description: "Play at the OD you pick, 0 to 20 (C: which). Not saved.",
    multiplier: 1.0,
    shuffle_cost: None,
    in_defense: false,
    setup_tab: Some(4),
    ..ModDef::DEFAULT
};
