//! What the play shows: read-only access for whoever draws it (the client's playfield and HUD),
//! to the play and to the parts it hands out (notes' states, Defense's towers, the crumbs).
//! Small and called every frame across the crate line, so each is `#[inline]`.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use super::{Crumb, DefenseRun, Game, Judge, NoteState, Song, Sound, Stats, Tower, Windows};
use crate::beatmap::Beatmap;
use crate::difficulty::Graph;
use crate::mods::{HalfRule, Mods};
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
    pub fn od_override(&self) -> f64 {
        self.od_override
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

    /// The replay being played back and how far in (None: the player is playing).
    #[inline]
    pub fn replay(&self) -> &Option<(Vec<(f64, u8)>, usize)> {
        &self.replay
    }

    /// Who played the replay being watched.
    #[inline]
    pub fn replay_of(&self) -> &Option<String> {
        &self.replay_of
    }

    #[inline]
    pub fn replay_timeline(&self) -> &Option<(Graph, Vec<f64>)> {
        &self.replay_timeline
    }

    /// The presses kept so far (map time, key).
    #[inline]
    pub fn inputs(&self) -> &[(f64, u8)] {
        &self.inputs
    }

    #[inline]
    pub fn scroll_speed(&self) -> f64 {
        self.scroll_speed
    }

    #[inline]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    #[inline]
    pub fn record(&self) -> bool {
        self.record
    }

    #[inline]
    pub fn track(&self) -> &Arc<dyn Song> {
        &self.track
    }

    #[inline]
    pub fn sfx_log(&self) -> &Option<Vec<(f64, Sound, f32)>> {
        &self.sfx_log
    }

    /// How long the map has been played (ms), pauses left out.
    #[inline]
    pub fn played_ms(&self) -> f64 {
        self.played_ms
    }

    /// A replay held still (watching).
    #[inline]
    pub fn held(&self) -> bool {
        self.held
    }

    #[inline]
    pub fn last_audio_ms(&self) -> Option<f64> {
        self.last_audio_ms
    }

    /// How long a failed play lingers before it ends (ms).
    #[inline]
    pub fn fail_linger(&self) -> f64 {
        self.fail_linger
    }

    #[inline]
    pub fn half(&self) -> HalfRule {
        self.half
    }
}

impl NoteState {
    #[inline]
    pub fn judged(&self) -> Option<Judge> {
        self.judged
    }

    /// Map time of the judgement.
    #[inline]
    pub fn judged_at(&self) -> f64 {
        self.judged_at
    }

    #[inline]
    pub fn hits(&self) -> u32 {
        self.hits
    }

    #[inline]
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// A roll's ticks: whether each was hit.
    #[inline]
    pub fn ticks(&self) -> &[bool] {
        &self.ticks
    }
}

impl Tower {
    #[inline]
    pub fn s(&self) -> f32 {
        self.s
    }

    #[inline]
    pub fn side(&self) -> f32 {
        self.side
    }

    #[inline]
    pub fn who(&self) -> usize {
        self.who
    }

    #[inline]
    pub fn kat(&self) -> bool {
        self.kat
    }

    #[inline]
    pub fn from(&self) -> f64 {
        self.from
    }

    #[inline]
    pub fn to(&self) -> f64 {
        self.to
    }

    /// The notes it takes.
    #[inline]
    pub fn notes(&self) -> &[usize] {
        &self.notes
    }
}

impl Crumb {
    /// Map time of the hit.
    #[inline]
    pub fn at(&self) -> f64 {
        self.at
    }

    /// 1 for a 1/1 note at 200 combo; smaller for faster rhythms and lower combos.
    #[inline]
    pub fn size(&self) -> f32 {
        self.size
    }

    /// Where it starts, as a part of the lane's length.
    #[inline]
    pub fn start(&self) -> f32 {
        self.start
    }

    /// How long it is (px).
    #[inline]
    pub fn length(&self) -> f32 {
        self.len
    }

    /// Sideways shift at its furthest.
    #[inline]
    pub fn shift(&self) -> f32 {
        self.shift
    }
}
