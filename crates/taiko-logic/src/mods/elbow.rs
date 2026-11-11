//! The Elbow mod (EB).

use super::{
    Mod::{
        Crossfire, Echo, Elbow, Loop, Mania, Maze, Multiverse, Shapeshift, Spin, Spiral, UTurn,
        Zigzag,
    },
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "EB",
    name: "Elbow",
    description: "Notes fall from the top and turn into the lane at a corner.",
    // A little harder: the lane moves or bends, the notes read as usual.
    multiplier: 1.02,
    shuffle_cost: Some(1),
    conflicts: &[
        Zigzag, UTurn, Spiral, Loop, Elbow, Maze, Shapeshift, Mania, Spin, Multiverse, Crossfire,
        Echo,
    ],
    ..ModDef::DEFAULT
};
