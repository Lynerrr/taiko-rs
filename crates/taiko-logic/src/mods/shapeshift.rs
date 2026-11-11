//! The Shapeshift mod (SS).

use super::{
    Mod::{
        Crossfire, Echo, Elbow, Loop, Mania, Maze, Multiverse, Shapeshift, Spin, Spiral, UTurn,
        Zigzag,
    },
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "SS",
    name: "Shapeshift",
    description: "Every few bars the lane bends into another rails shape (C: how often, which order).",
    // Hard to read or to play: notes that lie about where or when they'll arrive.
    multiplier: 1.06,
    shuffle_cost: Some(2),
    conflicts: &[
        Zigzag, UTurn, Spiral, Loop, Elbow, Maze, Shapeshift, Mania, Spin, Multiverse, Crossfire,
        Echo,
    ],
    setup_tab: Some(2),
    ..ModDef::DEFAULT
};
