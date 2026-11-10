//! What the client sets on a play: how it is shown and heard, what it keeps for a replay, the
//! replay it plays back, and the pause menu's cursor. Nothing here judges, scores or moves time:
//! that is the play's own (`press`, `update`, `seek`...).

use super::{Game, Mash, Scoring, Sound};
use crate::autoplay;
use crate::difficulty::Graph;
use crate::mods::Mods;
use crate::pp;

impl Game {
    /// No hitsounds (previews, the editor's test play, the field being watched in Multiverse).
    pub fn set_mute_hits(&mut self, mute: bool) {
        self.mute_hits = mute;
    }

    pub fn set_autoplay_style(&mut self, style: autoplay::Settings) {
        self.autoplay_style = style;
    }

    /// Plays back `inputs` (from the first) instead of the player's presses; None: the player's.
    pub fn set_replay(&mut self, replay: Option<(Vec<(f64, u8)>, usize)>) {
        self.replay = replay;
    }

    /// The replay being played back and how far in, to feed a live one as it comes.
    pub fn replay_mut(&mut self) -> &mut Option<(Vec<(f64, u8)>, usize)> {
        &mut self.replay
    }

    /// Who played the replay being watched.
    pub fn set_replay_of(&mut self, player: Option<String>) {
        self.replay_of = player;
    }

    /// The replay's difficulty graph and misses, for its timeline.
    pub fn set_replay_timeline(&mut self, timeline: Option<(Graph, Vec<f64>)>) {
        self.replay_timeline = timeline;
    }

    pub fn set_mash(&mut self, mash: Mash) {
        self.mash = mash;
    }

    pub fn set_scoring(&mut self, scoring: Scoring) {
        self.scoring = scoring;
    }

    /// Tests and tools: the mods the play goes on with.
    pub fn set_mods(&mut self, mods: Mods) {
        self.mods = mods;
    }

    /// Keeps every sound the play makes (`sfx_log`), for tools that put them in a file.
    pub fn set_sfx_log(&mut self, log: Option<Vec<(f64, Sound, f32)>>) {
        self.sfx_log = log;
    }

    /// The sounds kept since `set_sfx_log`, handed over; none are kept after.
    pub fn take_sfx_log(&mut self) -> Option<Vec<(f64, Sound, f32)>> {
        self.sfx_log.take()
    }

    pub fn set_shown_offset(&mut self, offset: f64) {
        self.shown_offset = offset;
    }

    pub fn set_latency(&mut self, latency: f64) {
        self.latency = latency;
    }

    pub fn set_scroll_speed(&mut self, speed: f64) {
        self.scroll_speed = speed;
    }

    /// Whether the presses are kept for a replay.
    pub fn set_record(&mut self, record: bool) {
        self.record = record;
    }

    /// Whether each press is written to the input log as it comes.
    pub fn set_log_inputs(&mut self, log: bool) {
        self.log_inputs = log;
    }

    pub fn set_live_pp(&mut self, live: Option<pp::Live>) {
        self.live_pp = live;
    }

    /// How long a failed play lingers before it ends (ms).
    pub fn set_fail_linger(&mut self, ms: f64) {
        self.fail_linger = ms;
    }

    /// Whether the offset hint scales with the play's unstable rate.
    pub fn set_ur_scaled_hint(&mut self, scaled: bool) {
        self.ur_scaled_hint = scaled;
    }

    /// Points the pause menu at `item` (the client only does while the menu is up; the play is
    /// paused from then on in any case).
    pub fn set_pause_cursor(&mut self, item: usize) {
        self.paused = Some(item);
    }

    /// The presses kept so far, handed over (for a replay file); none are kept after.
    pub fn take_inputs(&mut self) -> Vec<(f64, u8)> {
        std::mem::take(&mut self.inputs)
    }
}
