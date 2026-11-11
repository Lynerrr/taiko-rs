//! The Multiverse mod (MV).

use super::{
    Mod::{
        BarrelRoll, Blackout, Colorless, Crossfire, Echo, Elbow, Flashlight, Loop, Mania, Maze,
        Radar, Shapeshift, Spin, Spiral, Spotlight, Tilt, UTurn, Zigzag,
    },
    ModDef,
};

pub(super) const DEF: ModDef = ModDef {
    acronym: "MV",
    name: "Multiverse",
    description: "Eight playfields circle about, each with its own look and mods (C sets them up).",
    multiplier: 1.12,
    shuffle_cost: Some(2),
    conflicts: &[
        Spin, Mania, Tilt, BarrelRoll, Zigzag, UTurn, Spiral, Loop, Elbow, Maze, Shapeshift,
        Colorless, Flashlight, Spotlight, Blackout, Crossfire, Radar, Echo,
    ],
    obscures: true,
    setup_tab: Some(1),
    ..ModDef::DEFAULT
};
