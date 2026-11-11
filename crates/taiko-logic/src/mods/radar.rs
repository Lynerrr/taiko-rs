//! The Radar mod (RA).

use super::{
    Mod::{Blackout, Flashlight, Hidden, Memory, Multiverse, Spotlight, Sudden},
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "RA",
    name: "Radar",
    description: "The lane is dark: a beam sweeps it every other beat and lights notes up as it passes.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    // Radar decides when far notes show and lights the whole lane: nothing else may hide them
    // or keep the light to one spot. Its window-sized lanes are Multiverse's own.
    conflicts: &[
        Hidden, Sudden, Memory, Flashlight, Spotlight, Blackout, Multiverse,
    ],
    obscures: true,
    ..ModDef::DEFAULT
};
