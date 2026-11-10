//! `taiko_logic::pp` over the map files: the crate takes a map's bytes, these read the file.

use std::path::Path;

use taiko_logic::pp as logic;
pub use taiko_logic::pp::*;

use crate::stars::read;

pub fn play_pp(path: &Path, p: &Play) -> Option<f64> {
    logic::play_pp(&read(path)?, p)
}

pub fn breakdown(path: &Path, p: &Play) -> Option<Breakdown> {
    logic::breakdown(&read(path)?, p)
}

#[cfg(test)]
pub fn potential(
    path: &Path,
    rate: f64,
    mods: crate::mods::Mods,
    scroll_speed: f64,
) -> Option<[f64; 3]> {
    logic::potential(&read(path)?, rate, mods, scroll_speed)
}
