//! `taiko_logic::shapeshift`, and the setup the player plays Shapeshift with: the crate keeps
//! no process-wide settings, so it is kept here (from the config).

use std::sync::RwLock;

pub use taiko_logic::shapeshift::*;

static CURRENT: RwLock<Option<Config>> = RwLock::new(None);

/// The setup Shapeshift plays with (from the config).
pub fn set(c: Config) {
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = Some(c);
}

pub fn get() -> Config {
    CURRENT
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or_default()
}
