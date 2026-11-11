//! The Spin mod (SN).

use super::{
    Mod::{
        Crossfire, Echo, Elbow, Loop, Mania, Maze, Multiverse, Shapeshift, Spiral, UTurn, Zigzag,
    },
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "SN",
    name: "Spin",
    description: "The lane moves to the middle and turns full circle every 24 seconds.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    conflicts: &[
        Mania, Zigzag, UTurn, Spiral, Loop, Elbow, Maze, Shapeshift, Multiverse, Crossfire, Echo,
    ],
    ..ModDef::DEFAULT
};
