//! Gameplay as the client shows it: the play itself is taiko-logic's (`taiko_logic::sim`, all
//! of it here under its old name); this module draws it (`field`, `hud_draw`) with what the frame
//! being built says about the look of the playfield (`Look`).

pub use taiko_logic::sim::*;

// What the drawing below (`field`, `hud_draw`) and the tests take from here, as they did when the
// play lived in this module.
use std::sync::Arc;
use std::time::Instant;

use crate::beatmap::{self, Kind};
use crate::mods::{Mod, Mods};
use crate::multiverse::{self, Shape as NoteShape, Universe};
use crate::tko::runtime::NoteFx;
use taiko_logic::defense::DefenseItem;

mod field;
mod hud_draw;

pub use self::field::*;

/// How the playfield is drawn: the settings that change it, apart from the skin.
///
/// Like `render::Theme`: the thread building a frame draws with the look it started the frame
/// with (`set_frame_look`, from `build_frame`); other threads see the configured note colours
/// (`multiverse::set_note_colours`) and the defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// GREAT hits without their splash (no flash on the hit target, no "GREAT"), like osu! skins
    /// with an empty taiko-hit300: only OKs and misses stand out (Settings -> Playfield).
    pub hide_great: bool,
    /// The coloured flash on the hit target as a note is hit (Settings -> Skin): off, OKs don't
    /// light it up either.
    pub hit_flash: bool,
    /// The lane the notes scroll on, and the drum's panel beside it, drawn pure black (Settings ->
    /// View): no kiai or Party tint, no skin's lane picture.
    pub black_lane: bool,
    /// .tko notes drawn as what they are, whatever colour or size their map makes them show as
    /// (Settings -> Gameplay).
    pub true_notes: bool,
    /// The note colours of plays without Multiverse (Settings > Skin).
    pub note_colours: multiverse::Colours,
}

impl Default for Look {
    fn default() -> Look {
        Look {
            hide_great: true,
            hit_flash: true,
            black_lane: false,
            true_notes: false,
            note_colours: multiverse::Colours::CLASSIC,
        }
    }
}

thread_local! {
    /// The look of the frame this thread is building (`set_frame_look`).
    static FRAME_LOOK: std::cell::Cell<Option<Look>> = const { std::cell::Cell::new(None) };
}

/// The player's settings plays are made with (`Game::with_settings`), from the config.
pub fn settings() -> Settings {
    Settings {
        shapeshift: crate::shapeshift::get(),
        tact: crate::tact::get(),
        od_override: crate::od_override::get(),
        log: crate::audiolog::sink(),
    }
}

/// This thread draws the playfield with `look` from now on (`None`: the configured note colours
/// and the defaults).
pub fn set_frame_look(look: Option<Look>) {
    FRAME_LOOK.with(|l| l.set(look));
}

/// What this thread draws the playfield with.
pub fn playfield_look() -> Look {
    FRAME_LOOK
        .with(std::cell::Cell::get)
        .unwrap_or_else(|| Look {
            note_colours: multiverse::configured_note_colours(),
            ..Look::default()
        })
}

/// The note colours of plays without Multiverse, as this thread draws them (`game::playfield_look`).
pub fn note_colours() -> multiverse::Colours {
    playfield_look().note_colours
}

fn black_lane() -> bool {
    playfield_look().black_lane
}

#[cfg(test)]
mod tests;
