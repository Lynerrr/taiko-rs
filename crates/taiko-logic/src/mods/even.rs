//! The Even mod (EV).

use super::ModDef;

pub(super) const DEF: ModDef = ModDef {
    acronym: "EV",
    name: "Even",
    description: "Notes are evenly spaced: bursts fly in fast, slow parts crawl. Play by ear.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.08,
    shuffle_cost: Some(2),
    ..ModDef::DEFAULT
};
