//! What the play shows: read-only access for whoever draws it (the client's playfield and HUD).
//! Small and called every frame across the crate line, so each is `#[inline]`.

use std::collections::VecDeque;
use std::time::Instant;

use super::{Crumb, DefenseRun, Game, Judge, NoteState, Stats, Windows};
use crate::beatmap::Beatmap;
use crate::mods::Mods;
use crate::multiverse::{self, Universe};
use crate::shapeshift;
use crate::tact;
use crate::tko::runtime::Runtime;

impl Game {
    /// The map as it plays: the mods' changes to its notes and speeds made.
    #[inline]
    pub fn map(&self) -> &Beatmap {
        &self.map
    }

    #[inline]
    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// Map time (ms), the offset applied.
    #[inline]
    pub fn time(&self) -> f64 {
        self.time
    }

    /// The track's own time (ms), before the offset.
    #[inline]
    pub fn raw_time(&self) -> f64 {
        self.raw_time
    }

    #[inline]
    pub fn mods(&self) -> Mods {
        self.mods
    }

    #[inline]
    pub fn windows(&self) -> Windows {
        self.windows
    }

    #[inline]
    pub fn rate(&self) -> f64 {
        self.rate
    }

    #[inline]
    pub fn offset(&self) -> f64 {
        self.offset
    }

    /// Each note's judgement and hits so far.
    #[inline]
    pub fn state(&self) -> &[NoteState] {
        &self.state
    }

    #[inline]
    pub fn tko(&self) -> Option<&Runtime> {
        self.tko.as_ref()
    }

    /// A `.tko` map's runtime, which keeps what it worked out for the frame being drawn.
    #[inline]
    pub fn tko_mut(&mut self) -> Option<&mut Runtime> {
        self.tko.as_mut()
    }

    #[inline]
    pub fn defense(&self) -> Option<&DefenseRun> {
        self.defense.as_ref()
    }

    #[inline]
    pub fn recent_errors(&self) -> &VecDeque<(f64, f64, Judge)> {
        &self.recent_errors
    }

    #[inline]
    pub fn flows(&self) -> &[u8] {
        &self.flows
    }

    #[inline]
    pub fn kiai_starts(&self) -> &[(f64, f64)] {
        &self.kiai_starts
    }

    #[inline]
    pub fn paused(&self) -> Option<usize> {
        self.paused
    }

    #[inline]
    pub fn paused_at(&self) -> Instant {
        self.paused_at
    }

    #[inline]
    pub fn resuming(&self) -> Option<(Instant, Instant)> {
        self.resuming
    }

    #[inline]
    pub fn last_judge(&self) -> Option<(Judge, f64)> {
        self.last_judge
    }

    #[inline]
    pub fn presses(&self) -> [f64; 4] {
        self.presses
    }

    #[inline]
    pub fn shapeshift(&self) -> shapeshift::Config {
        self.shapeshift
    }

    #[inline]
    pub fn live_pp_value(&self) -> Option<f64> {
        self.live_pp_value
    }

    #[inline]
    pub fn hp(&self) -> f64 {
        self.hp
    }

    #[inline]
    pub fn universes(&self) -> Option<[Universe; multiverse::COUNT]> {
        self.universes
    }

    #[inline]
    pub fn first_note(&self) -> f64 {
        self.first_note
    }

    #[inline]
    pub fn crumbs(&self) -> &[Crumb] {
        &self.crumbs
    }

    #[inline]
    pub fn main_beat(&self) -> f64 {
        self.main_beat
    }

    #[inline]
    pub fn quake_at(&self) -> f64 {
        self.quake_at
    }

    #[inline]
    pub fn misses(&self) -> &VecDeque<f64> {
        &self.misses
    }

    #[inline]
    pub fn tact(&self) -> tact::Step {
        self.tact
    }

    #[inline]
    pub fn shape_order(&self) -> &[u8] {
        &self.shape_order
    }

    #[inline]
    pub fn scares(&self) -> &[f64] {
        &self.scares
    }

    #[inline]
    pub fn mute_hits(&self) -> bool {
        self.mute_hits
    }

    /// When and where the play failed, if it did.
    #[inline]
    pub fn failed_at(&self) -> Option<(f64, Instant)> {
        self.failed
    }
}
