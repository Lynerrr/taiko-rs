//! The map editor: charts edited as `.osu` files that play exactly as they do in osu!stable.
//!
//! `state` is what the editor does to a chart (placing, selecting, moving, copying, timing lines).
//! `chart` holds the file and writes it back without touching what the editor doesn't model,
//! `timing` is the snap grid and stable's millisecond rounding, `history` is undo and redo.

// Until the editor scene is wired into the game.
#![allow(dead_code)]

pub mod chart;
pub mod history;
pub mod state;
pub mod timing;
