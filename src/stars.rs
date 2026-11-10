//! `taiko_logic::stars` over the map files: the crate takes a map's bytes, these read the file.

use std::path::Path;

use taiko_logic::stars as logic;
pub use taiko_logic::stars::*;

/// A whole map file, for the crate (its `ReadFile`).
pub fn read(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

pub fn star_rating(path: &Path, rate: f64) -> Option<f64> {
    logic::star_rating(&read(path)?, rate)
}

pub fn star_rating_played(
    path: &Path,
    rate: f64,
    mods: crate::mods::Mods,
    scroll_speed: f64,
    seed: u64,
    half_rule: crate::mods::HalfRule,
) -> Option<f64> {
    logic::star_rating_played(&read(path)?, rate, mods, scroll_speed, seed, half_rule)
}

pub fn strain_curve(path: &Path, rate: f64, first_object_ms: f64) -> Option<(Vec<f64>, f64, f64)> {
    logic::strain_curve(&read(path)?, rate, first_object_ms)
}

#[cfg(test)]
pub fn star_rating_for(
    path: &Path,
    rate: f64,
    mods: crate::mods::Mods,
    scroll_speed: f64,
) -> Option<f64> {
    logic::star_rating_for(&read(path)?, rate, mods, scroll_speed)
}
