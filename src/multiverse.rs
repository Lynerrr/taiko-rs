//! `taiko_logic::multiverse`, and the client's configured universes and note colours: the
//! crate keeps no process-wide settings, so they are kept here (from the config, for every
//! thread).

use std::sync::RwLock;

pub use taiko_logic::multiverse::*;

static CURRENT: RwLock<[Universe; COUNT]> = RwLock::new([Universe::CLASSIC; COUNT]);
static NOTE_COLOURS: RwLock<Colours> = RwLock::new(Colours::CLASSIC);

/// The universes Multiverse plays are drawn with (from the config).
#[inline]
pub fn set(all: [Universe; COUNT]) {
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = all;
}

#[inline]
pub fn get() -> [Universe; COUNT] {
    *CURRENT.read().unwrap_or_else(|e| e.into_inner())
}

/// The configured note colours of plays without Multiverse (Settings > Skin), for threads not
/// building a frame (`game::Look`).
pub fn set_note_colours(colours: Colours) {
    *NOTE_COLOURS.write().unwrap_or_else(|e| e.into_inner()) = colours;
}

pub fn configured_note_colours() -> Colours {
    *NOTE_COLOURS.read().unwrap_or_else(|e| e.into_inner())
}
