//! The Mania mod (MN).

use super::{
    Mod::{
        Crossfire, Echo, Elbow, Loop, Maze, Multiverse, Shapeshift, Spin, Spiral, UTurn, Zigzag,
    },
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "MN",
    name: "Mania",
    description: "The lane stands up: notes fall down onto the drum.",
    // A little harder: the lane moves or bends, the notes read as usual.
    multiplier: 1.02,
    shuffle_cost: Some(1),
    conflicts: &[
        Spin, Zigzag, UTurn, Spiral, Loop, Elbow, Maze, Shapeshift, Multiverse, Crossfire, Echo,
    ],
    ..ModDef::DEFAULT
};
