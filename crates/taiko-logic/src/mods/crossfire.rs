//! The Crossfire mod (XF).

use super::{
    Mod::{
        Cracked, Crumble, Elbow, Loop, Mania, Maze, Multiverse, Portal, Shapeshift, Shatter, Spin,
        Spiral, UTurn, Zigzag,
    },
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "XF",
    name: "Crossfire",
    description: "The drum sits in the middle: kats come from the left, dons from the right.",
    // Noticeably harder.
    multiplier: 1.04,
    shuffle_cost: Some(1),
    // The drum in the middle, a lane each way: nothing else may move the drum, bend the lane or
    // break it into pieces.
    conflicts: &[
        Spin, Mania, Zigzag, UTurn, Spiral, Loop, Elbow, Maze, Shapeshift, Multiverse, Shatter,
        Cracked, Crumble, Portal,
    ],
    ..ModDef::DEFAULT
};
