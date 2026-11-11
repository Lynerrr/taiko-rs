//! The Maze mod (MZ).

use super::{
    Mod::{
        Crossfire, Echo, Elbow, Loop, Mania, Maze, Multiverse, Shapeshift, Spin, Spiral, UTurn,
        Zigzag,
    },
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "MZ",
    name: "Maze",
    description: "The lane winds round corner after corner across the whole screen to the drum.",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    conflicts: &[
        Zigzag, UTurn, Spiral, Loop, Elbow, Maze, Shapeshift, Mania, Spin, Multiverse, Crossfire,
        Echo,
    ],
    ..ModDef::DEFAULT
};
