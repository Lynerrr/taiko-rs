//! The Echo mod (EC).

use super::{
    Mod::{
        Colorless, Elbow, Loop, Mania, Maze, Multiverse, Shapeshift, Spin, Spiral, UTurn, Zigzag,
    },
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "EC",
    name: "Echo",
    description: "Notes are grey; the next bar shows in colour under the lane, a cursor running along it.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    // Echo's bar sits under the lane: nothing may move the lane over it. Colorless does its
    // grey already, and Multiverse's colours are what the windows go by.
    conflicts: &[
        Colorless, Multiverse, Spin, Mania, Zigzag, UTurn, Spiral, Loop, Elbow, Maze, Shapeshift,
    ],
    obscures: true,
    ..ModDef::DEFAULT
};
