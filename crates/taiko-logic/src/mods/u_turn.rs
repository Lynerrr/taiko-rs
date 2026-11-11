//! The U-Turn mod (UT).

use super::{
    Mod::{
        Crossfire, Echo, Elbow, Loop, Mania, Maze, Multiverse, Shapeshift, Spin, Spiral, UTurn,
        Zigzag,
    },
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "UT",
    name: "U-Turn",
    description: "Notes come in along the top, make a U-turn and come back to the drum.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(1),
    conflicts: &[
        Zigzag, UTurn, Spiral, Loop, Elbow, Maze, Shapeshift, Mania, Spin, Multiverse, Crossfire,
        Echo,
    ],
    ..ModDef::DEFAULT
};
