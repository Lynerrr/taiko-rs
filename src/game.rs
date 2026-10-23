//! Gameplay: judging, scoring and drawing the playfield.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use crate::audio::{Engine, Sfx, Track};
use crate::beatmap::{self, Beatmap, Kind};
use crate::calibrate;
use crate::clock::GameClock;
use crate::render::{progress, rgba, Canvas, Hit};

pub const PANEL_W: f32 = 150.0;
pub const HIT_X: f32 = PANEL_W + 64.0;
pub const LANE_Y: f32 = 160.0;
pub const LANE_H: f32 = 118.0;
const NOTE_R: f32 = 31.0;
const BIG_R: f32 = 46.0;
/// osu!px per ms -> virtual units per ms at scroll speed 1.0.
const SPEED_SCALE: f64 = 0.7;
/// Second key of a big note must land within this window of the first.
const STRONG_WINDOW: f64 = 30.0;
const HIT_ANIM_MS: f64 = 160.0;

const DON: u32 = 0xEB452C;
const KAT: u32 = 0x438EAD;
const ROLL: u32 = 0xFCB806;
const SWELL: u32 = 0xF39A2B;
const WHITE: u32 = 0xFFFFFF;

pub const PAUSE_ITEMS: [&str; 5] = ["Continue", "Retry", "Playfield size", "Settings", "Quit to song select"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Judge {
    Great,
    Ok,
    Miss,
}

impl Judge {
    fn color(self) -> u32 {
        match self {
            Judge::Great => 0xFFD34D,
            Judge::Ok => 0x86E05A,
            Judge::Miss => 0xFF4B4B,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Judge::Great => "GREAT",
            Judge::Ok => "OK",
            Judge::Miss => "MISS",
        }
    }
}

#[derive(Default, Clone)]
struct NoteState {
    judged: Option<Judge>,
    judged_at: f64,
    hits: u32,
    finished: bool,
    last_was_kat: Option<bool>,
    ticks: Vec<bool>,
}

#[derive(Clone, Default)]
pub struct Stats {
    pub great: u32,
    pub ok: u32,
    pub miss: u32,
    pub combo: u32,
    pub max_combo: u32,
    pub score: u64,
    pub roll_ticks: u32,
    pub swells: u32,
    pub strong: u32,
    pub errors: Vec<f64>,
    /// Audio offset in effect when each error was measured, so suggestions stay right after the
    /// offset changes mid-song.
    pub error_offsets: Vec<f64>,
}

impl Stats {
    pub fn accuracy(&self) -> f64 {
        let n = self.great + self.ok + self.miss;
        if n == 0 { 1.0 } else { (self.great as f64 + 0.5 * self.ok as f64) / n as f64 }
    }
    pub fn mean_error(&self) -> f64 {
        if self.errors.is_empty() { 0.0 } else { self.errors.iter().sum::<f64>() / self.errors.len() as f64 }
    }
    /// osu!'s unstable rate: 10x the standard deviation of hit errors.
    pub fn unstable_rate(&self) -> f64 {
        if self.errors.len() < 2 {
            return 0.0;
        }
        let mean = self.mean_error();
        let var = self.errors.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / self.errors.len() as f64;
        var.sqrt() * 10.0
    }
}

#[derive(Clone, Copy)]
pub struct Windows {
    pub great: f64,
    pub ok: f64,
    pub miss: f64,
}

impl Windows {
    pub fn new(od: f64) -> Windows {
        Windows {
            great: beatmap::difficulty_range(od, 50.0, 35.0, 20.0),
            ok: beatmap::difficulty_range(od, 120.0, 80.0, 50.0),
            miss: beatmap::difficulty_range(od, 135.0, 95.0, 70.0),
        }
    }
}

/// Live offset suggestion during play.
pub struct OffsetHint {
    pub estimate: calibrate::Estimate,
    /// Hits wanted for the target confidence at the current UR.
    pub needed: usize,
    /// Whether the average error is clearly larger than its uncertainty.
    pub significant: bool,
}

struct StrongPending {
    note: usize,
    kat: bool,
    key: usize,
    at: f64,
}

pub struct Game {
    pub map: Beatmap,
    clock: GameClock,
    state: Vec<NoteState>,
    cursor: usize,
    pub stats: Stats,
    pub windows: Windows,
    /// Gameplay time (ms) of the current frame.
    pub time: f64,
    end_time: f64,
    track_ms: f64,
    first_note: f64,
    /// Game time of the last press per key: left rim, left centre, right centre, right rim.
    presses: [f64; 4],
    last_judge: Option<(Judge, f64)>,
    recent_errors: VecDeque<(f64, f64, Judge)>,
    strong: Option<StrongPending>,
    min_velocity: f64,
    visible: Vec<usize>,
    /// Raw audio position seen by the last update, for diagnostics.
    pub last_audio_ms: Option<f64>,
    pub raw_time: f64,
    auto_key: usize,
    /// After a seek the audio thread still reports the old position until its next callback.
    ignore_audio_until: Option<Instant>,
    /// Pause menu cursor while paused.
    pub paused: Option<usize>,
    paused_at: Instant,
    /// Counting down to unpause: (started, ends). The menu is hidden meanwhile.
    pub resuming: Option<(Instant, Instant)>,
    /// Calibration: draw no notes or barlines (play by ear) / play no hitsounds on presses.
    pub hide_notes: bool,
    pub mute_hits: bool,
    /// Show the live offset suggestion under the hit error meter.
    pub show_offset_hint: bool,
    /// Scale the suggestion to current UR; off = plain average of the last 40 hits.
    pub ur_scaled_hint: bool,
    /// Offset used for the current frame / last press.
    offset: f64,
    /// Speed modifier: map milliseconds per real millisecond (1.5 = DT). Taken from the track,
    /// which was prepared at this rate. The game clock runs in track time; map time is
    /// `(track - offset) * rate`, so the offset stays in real milliseconds.
    pub rate: f64,
}

impl Game {
    pub fn new(map: Beatmap, track: Arc<Track>, audio: Option<&Engine>) -> Game {
        let first_note = map.notes.first().map(|n| n.time).unwrap_or(0.0);
        let end_time = map.notes.iter().map(|n| n.end_time).fold(0.0, f64::max);
        let start = (first_note - 2000.0).min(0.0).min(-map.meta.audio_lead_in);
        let rate = track.rate.clamp(0.25, 4.0);
        let track_ms = audio.map(|a| track.frames() as f64 / a.rate as f64 * 1000.0).unwrap_or(f64::INFINITY);
        // The pre-roll is in map time; the track plays `rate` map-ms per ms.
        let track_start = start / rate;
        if let Some(a) = audio {
            a.play(track, track_start);
        }
        let state = map
            .notes
            .iter()
            .map(|n| NoteState {
                ticks: if n.kind == Kind::Roll && n.tick_spacing > 0.0 {
                    vec![false; ((n.end_time - n.time) / n.tick_spacing + 0.5).floor() as usize + 1]
                } else {
                    Vec::new()
                },
                ..Default::default()
            })
            .collect();
        let min_velocity = map.notes.iter().map(|n| n.velocity).fold(f64::INFINITY, f64::min).max(0.01);
        Game {
            windows: Windows::new(map.meta.od),
            map,
            clock: GameClock::new(track_start, audio.is_some()),
            state,
            cursor: 0,
            stats: Stats::default(),
            time: start,
            end_time,
            track_ms,
            first_note,
            presses: [f64::NEG_INFINITY; 4],
            last_judge: None,
            recent_errors: VecDeque::new(),
            strong: None,
            min_velocity,
            visible: Vec::new(),
            last_audio_ms: None,
            raw_time: track_start,
            auto_key: 0,
            ignore_audio_until: None,
            paused: None,
            paused_at: Instant::now(),
            resuming: None,
            hide_notes: false,
            mute_hits: false,
            show_offset_hint: false,
            ur_scaled_hint: true,
            offset: 0.0,
            rate,
        }
    }

    /// Advances the clock. Returns true once the map is over.
    pub fn update(&mut self, now: Instant, audio: Option<&Engine>, offset: f64) -> bool {
        self.offset = offset;
        if self.resuming.is_some_and(|(_, end)| now >= end) {
            self.resume(audio);
        }
        if self.paused.is_some() {
            self.clock.hold(now);
            self.time = (self.raw_time - offset) * self.rate;
            return false;
        }
        let stale = self.ignore_audio_until.is_some_and(|u| now < u);
        let audio_ms = audio.and_then(|a| a.position_ms(now)).filter(|_| !stale);
        let raw = self.clock.update(now, audio_ms);
        self.last_audio_ms = audio_ms;
        self.raw_time = raw;
        self.time = (raw - offset) * self.rate;
        self.expire(self.time);
        self.time > self.end_time + 1500.0 || raw > self.track_ms + 500.0
    }

    /// Autoplay: hits every object exactly on time, alternating hands. Used to sanity-check judging.
    pub fn autoplay(&mut self, audio: Option<&Engine>) {
        let t = self.time;
        for i in self.cursor..self.map.notes.len() {
            let n = &self.map.notes[i];
            if n.time > t {
                break;
            }
            let (time, kind, big) = (n.time, n.kind, n.big);
            let st = &self.state[i];
            match kind {
                Kind::Don | Kind::Kat if st.judged.is_none() => {
                    let kat = kind == Kind::Kat;
                    self.auto_key ^= 1;
                    let key = match (kat, self.auto_key) { (true, 0) => 0, (true, _) => 3, (false, 0) => 1, (false, _) => 2 };
                    self.auto_hit(key, time, audio);
                    if big {
                        let other = 3 - key;
                        self.auto_hit(other, time, audio);
                    }
                }
                Kind::Roll if !st.finished => {
                    let spacing = self.map.notes[i].tick_spacing;
                    let k = ((t - time) / spacing).floor().max(0.0) as usize;
                    if self.state[i].ticks.get(k).is_some_and(|hit| !hit) {
                        self.auto_key ^= 1;
                        self.auto_hit(if self.auto_key == 0 { 1 } else { 2 }, time + k as f64 * spacing, audio);
                    }
                }
                Kind::Swell if !st.finished => {
                    let next_kat = st.last_was_kat != Some(true);
                    if t - self.presses.iter().cloned().fold(f64::NEG_INFINITY, f64::max) > 40.0 {
                        self.auto_hit(if next_kat { 0 } else { 1 }, t, audio);
                    }
                }
                _ => {}
            }
        }
    }

    fn auto_hit(&mut self, key: usize, t: f64, audio: Option<&Engine>) {
        self.presses[key] = t;
        let sfx = self.judge_press(key, key == 0 || key == 3, t);
        if let Some(a) = audio {
            a.sfx(sfx);
        }
    }

    pub fn pause(&mut self, audio: Option<&Engine>) {
        // Pausing again (e.g. losing focus) during the resume countdown shows the menu again.
        self.cancel_resume();
        if self.paused.is_none() {
            self.paused = Some(0);
            self.paused_at = Instant::now();
            if let Some(a) = audio {
                a.set_paused(true);
            }
        }
    }

    pub fn resume(&mut self, audio: Option<&Engine>) {
        self.resuming = None;
        if self.paused.take().is_some() {
            if let Some(a) = audio {
                a.set_paused(false);
            }
        }
    }

    /// Leaves the pause menu: unpauses after `delay` (a countdown over the frozen playfield), or
    /// at once with no delay.
    pub fn resume_after(&mut self, delay: std::time::Duration, audio: Option<&Engine>) {
        if delay.is_zero() {
            self.resume(audio);
        } else if self.paused.is_some() {
            let now = Instant::now();
            self.resuming = Some((now, now + delay));
        }
    }

    /// Back to the pause menu from the resume countdown.
    pub fn cancel_resume(&mut self) {
        if self.resuming.take().is_some() {
            self.paused_at = Instant::now();
        }
    }

    /// Offset suggestion from recent hits, re-expressed relative to `current_offset` and scaled to
    /// how consistent the player currently is (UR): noisier hitting uses more hits, only counts as
    /// off when the error exceeds its confidence margin, and suggests a more cautious change.
    pub fn offset_suggestion(&self, current_offset: f64) -> Option<OffsetHint> {
        const MIN_HITS: usize = 10;
        const MAX_HITS: usize = 200;
        /// Aim to know the average error to within this (95% confidence half-width).
        const TARGET_MARGIN_MS: f64 = 3.0;
        /// Typical size of a real offset error; sets how strongly noisy estimates are shrunk.
        const PRIOR_MS: f64 = 10.0;

        let s = &self.stats;
        let n = s.errors.len();
        if n < MIN_HITS {
            return None;
        }
        // An error measured under offset `o` would have been `e - (current - o)` under the current one.
        let corrected: Vec<f64> = s.errors.iter().zip(&s.error_offsets).map(|(e, o)| e - (current_offset - o)).collect();
        if !self.ur_scaled_hint {
            // Simple mode: average of the last 40 hits, applied in full.
            let estimate = calibrate::estimate(&corrected[n.saturating_sub(40)..], current_offset)?;
            let significant = estimate.error_ms.abs() >= 3.0;
            return Some(OffsetHint { estimate, needed: MIN_HITS, significant });
        }
        // Current consistency from the latest 40 hits sizes the window.
        let probe = calibrate::estimate(&corrected[n.saturating_sub(40)..], current_offset)?;
        let sigma = probe.ur / 10.0;
        let needed = ((1.96 * sigma / TARGET_MARGIN_MS).powi(2).ceil() as usize).clamp(MIN_HITS * 2, MAX_HITS);
        let mut estimate = calibrate::estimate(&corrected[n.saturating_sub(needed)..], current_offset)?;
        let se = estimate.margin_ms / 1.96;
        let shrink = PRIOR_MS.powi(2) / (PRIOR_MS.powi(2) + se.powi(2));
        estimate.suggested_offset = (current_offset + estimate.error_ms * shrink).round();
        let significant = estimate.error_ms.abs() > estimate.margin_ms.max(2.0);
        Some(OffsetHint { estimate, needed, significant })
    }

    /// Marks everything ending before `t` as already played, without scoring or animations (the
    /// song select preview jumps into the middle of the song).
    pub fn skip_to(&mut self, t: f64) {
        for (n, st) in self.map.notes.iter().zip(self.state.iter_mut()) {
            if n.end_time < t {
                st.judged = Some(Judge::Great);
                st.judged_at = f64::NEG_INFINITY;
                st.finished = true;
            }
        }
        while self.cursor < self.map.notes.len() && self.state[self.cursor].finished {
            self.cursor += 1;
        }
    }

    /// Starts play at map time `t` instead of the beginning: the music starts two seconds
    /// before it, and everything that ends earlier counts as not played (no score, no misses).
    pub fn start_from(&mut self, t: f64, audio: Option<&Engine>, offset: f64) {
        let start = (t - 2000.0 * self.rate).max(self.time);
        let target = start / self.rate + offset;
        if let Some(a) = audio {
            a.seek(target);
        }
        self.clock = GameClock::new(target, audio.is_some());
        self.ignore_audio_until = Some(Instant::now() + std::time::Duration::from_millis(80));
        self.raw_time = target;
        self.time = start;
        self.skip_to(t);
    }

    pub fn can_skip(&self) -> bool {
        self.first_note - self.time > 3000.0
    }

    pub fn skip(&mut self, audio: Option<&Engine>, offset: f64) {
        if self.can_skip() {
            let target = (self.first_note - 2000.0) / self.rate + offset;
            if let Some(a) = audio {
                a.seek(target);
            }
            self.clock = GameClock::new(target, audio.is_some());
            self.ignore_audio_until = Some(Instant::now() + std::time::Duration::from_millis(80));
        }
    }

    pub fn progress(&self) -> f64 {
        ((self.time - self.first_note) / (self.end_time - self.first_note).max(1.0)).clamp(0.0, 1.0)
    }

    fn expire(&mut self, t: f64) {
        for i in self.cursor..self.map.notes.len() {
            let n = &self.map.notes[i];
            if n.time > t {
                break;
            }
            let st = &mut self.state[i];
            match n.kind {
                Kind::Don | Kind::Kat => {
                    if st.judged.is_none() && t - n.time > self.windows.ok {
                        self.apply(i, Judge::Miss, t, None);
                    }
                }
                Kind::Roll => {
                    if t > n.end_time + n.tick_spacing / 2.0 {
                        st.finished = true;
                    }
                }
                Kind::Swell => {
                    if !st.finished && t > n.end_time {
                        st.finished = true;
                    }
                }
            }
        }
        if self.strong.as_ref().is_some_and(|s| t - s.at > STRONG_WINDOW) {
            self.strong = None;
        }
        while self.cursor < self.map.notes.len() {
            let st = &self.state[self.cursor];
            let done = match self.map.notes[self.cursor].kind {
                Kind::Don | Kind::Kat => st.judged.is_some(),
                Kind::Roll | Kind::Swell => st.finished,
            };
            if !done {
                break;
            }
            self.cursor += 1;
        }
    }

    fn apply(&mut self, i: usize, judge: Judge, t: f64, error: Option<f64>) {
        let st = &mut self.state[i];
        st.judged = Some(judge);
        st.judged_at = t;
        let s = &mut self.stats;
        let combo_mult = 1.0 + s.combo.min(100) as f64 / 100.0;
        match judge {
            Judge::Great => {
                s.great += 1;
                s.combo += 1;
                s.score += (300.0 * combo_mult) as u64;
            }
            Judge::Ok => {
                s.ok += 1;
                s.combo += 1;
                s.score += (150.0 * combo_mult) as u64;
            }
            Judge::Miss => {
                s.miss += 1;
                s.combo = 0;
            }
        }
        s.max_combo = s.max_combo.max(s.combo);
        if let Some(e) = error.filter(|_| judge != Judge::Miss) {
            // Stats are kept in real milliseconds, so UR and offset advice mean the same at any rate.
            s.errors.push(e / self.rate);
            s.error_offsets.push(self.offset);
            self.recent_errors.push_back((e, t, judge));
            if self.recent_errors.len() > 64 {
                self.recent_errors.pop_front();
            }
        }
        self.last_judge = Some((judge, t));
    }

    /// Handles a drum key (0 = left rim, 1 = left centre, 2 = right centre, 3 = right rim).
    pub fn press(&mut self, key: usize, at: Instant, audio: Option<&Engine>, offset: f64) {
        let t = (self.clock.at(at) - offset) * self.rate;
        self.offset = offset;
        let kat = key == 0 || key == 3;
        self.presses[key] = t;
        let sfx = self.judge_press(key, kat, t);
        if let Some(a) = audio.filter(|_| !self.mute_hits) {
            a.sfx(sfx);
        }
    }

    fn judge_press(&mut self, key: usize, kat: bool, t: f64) -> Sfx {
        let normal = if kat { Sfx::Kat } else { Sfx::Don };
        let big = if kat { Sfx::BigKat } else { Sfx::BigDon };
        self.expire(t);

        if let Some(s) = &self.strong {
            let other_hand = (s.key < 2) != (key < 2);
            if s.kat == kat && other_hand && (t - s.at).abs() <= STRONG_WINDOW && self.state[s.note].judged.is_some() {
                self.strong = None;
                self.stats.strong += 1;
                self.stats.score += 300;
                return big;
            }
        }

        // An active swell swallows every hit and must be alternated centre/rim.
        if let Some(i) = self.find_active(t, Kind::Swell) {
            let required = self.map.notes[i].required_hits;
            let st = &mut self.state[i];
            if st.last_was_kat != Some(kat) {
                st.last_was_kat = Some(kat);
                st.hits += 1;
                self.stats.score += 100;
                if st.hits >= required {
                    st.finished = true;
                    st.judged = Some(Judge::Great);
                    st.judged_at = t;
                    self.stats.swells += 1;
                    self.stats.score += 1000;
                }
            }
            return normal;
        }

        if let Some(i) = self.next_hittable(t) {
            let n = &self.map.notes[i];
            let err = t - n.time;
            let wrong = (n.kind == Kind::Kat) != kat;
            let judge = if wrong || err.abs() > self.windows.ok {
                Judge::Miss
            } else if err.abs() <= self.windows.great {
                Judge::Great
            } else {
                Judge::Ok
            };
            let is_big = n.big;
            self.apply(i, judge, t, Some(err));
            if judge != Judge::Miss && is_big {
                self.strong = Some(StrongPending { note: i, kat, key, at: t });
                return big;
            }
            return normal;
        }

        if let Some(i) = self.find_active(t, Kind::Roll) {
            let n = &self.map.notes[i];
            let k = ((t - n.time) / n.tick_spacing).round().max(0.0) as usize;
            if let Some(tick) = self.state[i].ticks.get_mut(k) {
                if !*tick {
                    *tick = true;
                    self.stats.roll_ticks += 1;
                    self.stats.score += if n.big { 200 } else { 100 };
                }
            }
        }
        normal
    }

    /// First unjudged don/kat whose miss window has opened.
    fn next_hittable(&self, t: f64) -> Option<usize> {
        for i in self.cursor..self.map.notes.len() {
            let n = &self.map.notes[i];
            if n.time - t > self.windows.miss {
                return None;
            }
            if matches!(n.kind, Kind::Don | Kind::Kat) && self.state[i].judged.is_none() {
                return Some(i);
            }
        }
        None
    }

    fn find_active(&self, t: f64, kind: Kind) -> Option<usize> {
        for i in self.cursor..self.map.notes.len() {
            let n = &self.map.notes[i];
            let slack = if kind == Kind::Roll { n.tick_spacing / 2.0 } else { 0.0 };
            if n.time - slack > t {
                return None;
            }
            if n.kind == kind && !self.state[i].finished && t <= n.end_time + slack {
                return Some(i);
            }
        }
        None
    }

    /// Lane, barlines, notes, drum, combo and judgements, laid out as if `c` were the window (the
    /// caller maps it into the playfield area).
    pub fn draw_playfield(&mut self, c: &mut Canvas, scroll_speed: f64) {
        let t = self.time;
        let w = c.width;
        let speed = SPEED_SCALE * scroll_speed;
        let x_of = |time: f64, vel: f64| HIT_X + ((time - t) * vel * speed) as f32;
        let lookahead = ((w - HIT_X + BIG_R) as f64 / (self.min_velocity * speed)).min(30_000.0);
        let lane_top = LANE_Y - LANE_H / 2.0;
        let cp = beatmap::control_point_at(&self.map.control_points, t);
        let beat_phase = ((t - cp.time) / cp.beat_length).rem_euclid(1.0) as f32;
        let pulse = (1.0 - beat_phase).powi(3);

        // Lane.
        let lane_col = if cp.kiai { rgba(0x2b2433, 1.0) } else { rgba(0x20202b, 1.0) };
        c.rect(0.0, lane_top - 3.0, w, LANE_H + 6.0, rgba(0x3a3a4e, 1.0));
        c.rect(0.0, lane_top, w, LANE_H, lane_col);
        if cp.kiai {
            c.rect(PANEL_W, lane_top, w - PANEL_W, LANE_H, rgba(0xff8a3d, 0.05 + 0.06 * pulse));
        }

        // Barlines.
        let first_bar = self.map.barlines.partition_point(|b| b.time < t - 3000.0);
        let bars = if self.hide_notes { &[][..] } else { &self.map.barlines[first_bar..] };
        for b in bars {
            if b.time - t > lookahead {
                break;
            }
            let x = x_of(b.time, b.velocity);
            if x > PANEL_W && x < w + 2.0 {
                c.rect(x - 0.8, lane_top, 1.6, LANE_H, rgba(WHITE, 0.28));
            }
        }

        // Hit target.
        c.circle(HIT_X, LANE_Y, BIG_R, rgba(WHITE, 0.03), rgba(WHITE, 0.12), 2.0);
        c.circle(HIT_X, LANE_Y, NOTE_R + 1.0, rgba(WHITE, 0.06), rgba(WHITE, 0.4), 2.5);
        if cp.kiai {
            c.glow(HIT_X, LANE_Y, NOTE_R, 30.0, rgba(0xffb347, 0.25 * pulse));
        }
        if let Some((j, at)) = self.last_judge {
            let a = (1.0 - (t - at) / 220.0).clamp(0.0, 1.0) as f32;
            if a > 0.0 && j != Judge::Miss {
                c.glow(HIT_X, LANE_Y, NOTE_R + 2.0, 26.0, rgba(j.color(), 0.55 * a));
            }
        }

        // Notes: collect visible, draw back to front so earlier notes sit on top.
        let start = self.map.notes.partition_point(|n| n.time < t - 20_000.0);
        self.visible.clear();
        for i in start..self.map.notes.len() {
            let n = &self.map.notes[i];
            if n.time - t > lookahead {
                break;
            }
            if n.end_time < t - 3000.0 && n.kind != Kind::Swell {
                continue;
            }
            self.visible.push(i);
        }
        if self.hide_notes {
            self.visible.clear();
        }
        for &i in self.visible.iter().rev() {
            let n = &self.map.notes[i];
            let st = &self.state[i];
            let r = if n.big { BIG_R } else { NOTE_R };
            match n.kind {
                Kind::Don | Kind::Kat => {
                    let col = if n.kind == Kind::Don { DON } else { KAT };
                    match st.judged {
                        Some(Judge::Great | Judge::Ok) => {
                            let p = ((t - st.judged_at) / HIT_ANIM_MS).clamp(0.0, 1.0) as f32;
                            if p < 1.0 {
                                let y = LANE_Y - 80.0 * p * (2.0 - p);
                                c.circle(HIT_X, y, r * (1.0 + 0.15 * p), rgba(col, 1.0 - p), rgba(WHITE, 1.0 - p), 3.0);
                            }
                        }
                        judged => {
                            let x = x_of(n.time, n.velocity);
                            if x > -r && x < w + r {
                                let a = if judged.is_some() { 0.4 } else { 1.0 };
                                c.circle(x, LANE_Y, r + 2.0, rgba(0x000000, 0.35 * a), rgba(0x000000, 0.0), 0.0);
                                c.circle(x, LANE_Y, r, rgba(col, a), rgba(WHITE, a), r * 0.1);
                            }
                        }
                    }
                }
                Kind::Roll => {
                    let x0 = x_of(n.time, n.velocity);
                    let x1 = x_of(n.end_time, n.velocity);
                    if x1 < -r || x0 > w + r {
                        continue;
                    }
                    let body_r = r * 0.86;
                    c.capsule(x0, x1, LANE_Y, body_r, rgba(ROLL, 1.0), rgba(WHITE, 1.0), 2.5);
                    for (k, hit) in st.ticks.iter().enumerate() {
                        let tx = x_of(n.time + k as f64 * n.tick_spacing, n.velocity);
                        if !hit && tx > x0 && tx < w {
                            c.circle(tx, LANE_Y, 4.0, rgba(WHITE, 0.85), rgba(WHITE, 0.0), 0.0);
                        }
                    }
                    c.circle(x0, LANE_Y, r, rgba(ROLL, 1.0), rgba(WHITE, 1.0), r * 0.1);
                }
                Kind::Swell => {
                    if st.finished {
                        if st.judged == Some(Judge::Great) {
                            let p = ((t - st.judged_at) / 300.0).clamp(0.0, 1.0) as f32;
                            if p < 1.0 {
                                c.ring(HIT_X, LANE_Y, 52.0 + 40.0 * p, rgba(SWELL, 1.0 - p), 6.0 * (1.0 - p) + 1.0);
                            }
                        }
                        continue;
                    }
                    if t < n.time {
                        let x = x_of(n.time, n.velocity);
                        if x < w + r {
                            c.circle(x, LANE_Y, r, rgba(SWELL, 1.0), rgba(WHITE, 1.0), r * 0.1);
                            c.ring(x, LANE_Y, r * 0.55, rgba(WHITE, 0.8), 3.0);
                        }
                    } else {
                        let progress = st.hits as f32 / n.required_hits.max(1) as f32;
                        c.circle(HIT_X, LANE_Y, 50.0, rgba(SWELL, 0.95), rgba(WHITE, 1.0), 4.0);
                        c.ring(HIT_X, LANE_Y, 54.0 + 14.0 * progress, rgba(SWELL, 0.6), 3.0);
                        let left = n.required_hits.saturating_sub(st.hits);
                        c.text_centered(HIT_X, LANE_Y - 12.0, 24.0, rgba(WHITE, 1.0), &left.to_string());
                    }
                }
            }
        }

        // Drum panel on top so passed notes slide underneath it.
        c.rect(0.0, lane_top, PANEL_W, LANE_H, rgba(0x191921, 1.0));
        let drum_x = PANEL_W / 2.0;
        let flash = |at: f64| ((1.0 - (t - at) / 130.0).clamp(0.0, 1.0) * 0.95) as f32;
        c.circle(drum_x, LANE_Y, 46.0, rgba(0x2f2f3e, 1.0), rgba(0x4a4a60, 1.0), 2.0);
        c.half_circle(drum_x, LANE_Y, 46.0, 1, rgba(KAT, flash(self.presses[0])), rgba(KAT, 0.0), 0.0);
        c.half_circle(drum_x, LANE_Y, 46.0, 2, rgba(KAT, flash(self.presses[3])), rgba(KAT, 0.0), 0.0);
        c.circle(drum_x, LANE_Y, 32.0, rgba(0xe9dcc3, 1.0), rgba(0x8a7b63, 1.0), 2.0);
        c.half_circle(drum_x, LANE_Y, 32.0, 1, rgba(DON, flash(self.presses[1])), rgba(DON, 0.0), 0.0);
        c.half_circle(drum_x, LANE_Y, 32.0, 2, rgba(DON, flash(self.presses[2])), rgba(DON, 0.0), 0.0);
        if self.stats.combo >= 2 {
            let s = self.stats.combo.to_string();
            let size = if s.len() > 3 { 12.0 } else { 16.0 };
            c.text_centered(drum_x, LANE_Y - size / 2.0, size, rgba(0x1d1d26, 1.0), &s);
        }

        // Judgement text.
        if let Some((j, at)) = self.last_judge {
            let age = (t - at) as f32;
            if age < 450.0 {
                let a = (1.0 - age / 450.0).min(1.0);
                c.text_centered(HIT_X, lane_top - 30.0 - age * 0.02, 15.0, rgba(j.color(), a), j.label());
            }
        }

    }

    /// Score, accuracy, progress, hit error meter, offset advice, map info, skip hint and the pause
    /// menu. Drawn with the playfield, or at fixed screen positions (HUD position setting).
    pub fn draw_hud(&self, c: &mut Canvas) {
        let t = self.time;
        let w = c.width;
        let lane_top = LANE_Y - LANE_H / 2.0;

        // Score / accuracy.
        c.text_right(w - 18.0, 18.0, 24.0, rgba(WHITE, 1.0), &format!("{:08}", self.stats.score));
        c.text_right(w - 18.0, 50.0, 14.0, rgba(0xc8c8d8, 1.0), &format!("{:.2}%", self.stats.accuracy() * 100.0));
        // Progress bar.
        c.rect(0.0, 0.0, w * self.progress() as f32, 3.0, rgba(WHITE, 0.45));

        // Hit error meter.
        let cx = w / 2.0;
        let my = 430.0;
        let scale = 150.0 / self.windows.miss as f32;
        let wnd = |ms: f64| ms as f32 * scale;
        c.round_rect(cx - wnd(self.windows.miss), my - 3.0, wnd(self.windows.miss) * 2.0, 6.0, 3.0, rgba(Judge::Miss.color(), 0.25), rgba(0, 0.0), 0.0);
        c.round_rect(cx - wnd(self.windows.ok), my - 3.0, wnd(self.windows.ok) * 2.0, 6.0, 3.0, rgba(Judge::Ok.color(), 0.4), rgba(0, 0.0), 0.0);
        c.round_rect(cx - wnd(self.windows.great), my - 3.0, wnd(self.windows.great) * 2.0, 6.0, 3.0, rgba(Judge::Great.color(), 0.6), rgba(0, 0.0), 0.0);
        for &(err, at, j) in &self.recent_errors {
            let age = (t - at) as f32;
            if age < 4000.0 {
                let a = 1.0 - age / 4000.0;
                c.rect(cx + err as f32 * scale - 1.0, my - 11.0, 2.0, 22.0, rgba(j.color(), a));
            }
        }
        if !self.recent_errors.is_empty() {
            let n = self.recent_errors.len().min(20);
            let mean = self.recent_errors.iter().rev().take(n).map(|e| e.0).sum::<f64>() / n as f64;
            c.circle(cx + mean as f32 * scale, my - 18.0, 3.5, rgba(WHITE, 0.9), rgba(WHITE, 0.0), 0.0);
        }
        c.rect(cx - 1.0, my - 14.0, 2.0, 28.0, rgba(WHITE, 0.9));
        if self.show_offset_hint {
            if let Some(h) = self.offset_suggestion(self.offset) {
                let e = &h.estimate;
                let building = if e.hits < h.needed { format!("  confidence {}/{}", e.hits, h.needed) } else { String::new() };
                let stats = format!("UR {:.0}, {} hits", e.ur, e.hits);
                if h.significant {
                    let dir = if e.error_ms > 0.0 { "late" } else { "early" };
                    let text = format!(
                        "{:.1} +-{:.1} ms {dir} ({stats})  |  suggested offset {:+.0} ms  |  O to apply{building}",
                        e.error_ms.abs(),
                        e.margin_ms,
                        e.suggested_offset
                    );
                    c.text_centered(cx, my + 20.0, 8.0, rgba(0xFFD34D, 0.95), &text);
                } else {
                    let text = format!("offset looks good: {:+.1} +-{:.1} ms ({stats}){building}", e.error_ms, e.margin_ms);
                    c.text_centered(cx, my + 20.0, 8.0, rgba(0x86E05A, 0.9), &text);
                }
            }
        }

        // Map info.
        let m = &self.map.meta;
        let info_y = lane_top + LANE_H + 22.0;
        let rate = if (self.rate - 1.0).abs() > 1e-6 { format!("   {:.2}x", self.rate) } else { String::new() };
        c.text(18.0, info_y, 10.0, rgba(0x9a9ab0, 1.0), &format!("{} - {}", m.artist, m.title));
        c.text(18.0, info_y + 16.0, 10.0, rgba(0x6f6f86, 1.0), &format!("[{}]  OD {:.1}{rate}", m.version, m.od));

        if self.can_skip() && self.paused.is_none() && !self.mute_hits {
            c.text_centered(w / 2.0, 330.0, 12.0, rgba(WHITE, 0.7), "SPACE to skip intro");
        }
        if let (Some(_), Some((start, end))) = (self.paused, self.resuming) {
            // Resume countdown: the playfield stays visible, lightly dimmed.
            let now = Instant::now();
            let left = end.saturating_duration_since(now).as_secs_f32();
            let total = (end - start).as_secs_f32().max(0.001);
            c.rect(0.0, 0.0, w, 480.0, rgba(0x0b0b10, 0.35 * (left / total).min(1.0)));
            let (cx, cy) = (w / 2.0, 330.0);
            c.ring(cx, cy, 26.0, rgba(0x2a2a38, 0.9), 4.0);
            // Shrinking arc, drawn as dots around the ring.
            let n = 48;
            for k in 0..((left / total * n as f32).ceil() as usize).min(n) {
                let a = -std::f32::consts::FRAC_PI_2 + k as f32 / n as f32 * std::f32::consts::TAU;
                c.circle(cx + a.cos() * 26.0, cy + a.sin() * 26.0, 2.2, rgba(DON, 1.0), rgba(DON, 1.0), 0.0);
            }
            c.text_centered(cx, cy - 7.0, 14.0, rgba(WHITE, 1.0), &format!("{:.1}", left));
            c.text_centered(cx, cy + 36.0, 8.0, rgba(0x9a9ab0, 1.0), "get ready   (Esc: back to menu)");
        } else if let Some(cursor) = self.paused {
            let t = progress(self.paused_at, 180.0);
            c.rect(0.0, 0.0, w, 480.0, rgba(0x0b0b10, 0.78 * t));
            c.text_centered(w / 2.0, 120.0 - (1.0 - t) * 12.0, 32.0, rgba(WHITE, t), "PAUSED");
            for (i, item) in PAUSE_ITEMS.iter().enumerate() {
                // Items slide up one after another.
                let ti = progress(self.paused_at + std::time::Duration::from_millis(25 * i as u64), 180.0);
                let y = 170.0 + i as f32 * 38.0 + (1.0 - ti) * 18.0;
                let selected = i == cursor;
                let (bg, border) = if selected { (0x34344a, DON) } else { (0x1d1d27, 0x2a2a38) };
                c.round_rect(w / 2.0 - 140.0, y, 280.0, 32.0, 8.0, rgba(bg, 1.0), rgba(border, 1.0), if selected { 2.0 } else { 1.0 });
                c.hit(w / 2.0 - 140.0, y, 280.0, 32.0, Hit::PauseItem(i));
                c.text_centered(w / 2.0, y + 10.0, 12.0, rgba(if selected { WHITE } else { 0xc8c8d8 }, 1.0), item);
            }
            let hint_y = 170.0 + PAUSE_ITEMS.len() as f32 * 38.0 + 8.0;
            // Fades in once the items have slid past it.
            let th = progress(self.paused_at + std::time::Duration::from_millis(220), 150.0);
            c.text_centered(w / 2.0, hint_y, 9.0, rgba(0x8a8aa0, th), "Up/Down + Enter    Esc: continue");
        }
    }
}

/// Vertical band (top, height) of the lane with a little room for judgements, for previews that
/// show only the lane.
pub fn lane_band() -> (f32, f32) {
    (LANE_Y - LANE_H / 2.0 - 34.0, LANE_H + 42.0)
}

/// A still playfield (lane, drum, a few notes, score) for the playfield area editor.
pub fn draw_preview(c: &mut Canvas) {
    let w = c.width;
    let lane_top = LANE_Y - LANE_H / 2.0;
    c.rect(0.0, lane_top - 3.0, w, LANE_H + 6.0, rgba(0x3a3a4e, 1.0));
    c.rect(0.0, lane_top, w, LANE_H, rgba(0x20202b, 1.0));
    c.circle(HIT_X, LANE_Y, BIG_R, rgba(WHITE, 0.03), rgba(WHITE, 0.12), 2.0);
    c.circle(HIT_X, LANE_Y, NOTE_R + 1.0, rgba(WHITE, 0.06), rgba(WHITE, 0.4), 2.5);
    // Back to front, like gameplay.
    c.capsule(HIT_X + 560.0, HIT_X + 760.0, LANE_Y, NOTE_R * 0.86, rgba(ROLL, 1.0), rgba(WHITE, 1.0), 2.5);
    c.circle(HIT_X + 560.0, LANE_Y, NOTE_R, rgba(ROLL, 1.0), rgba(WHITE, 1.0), NOTE_R * 0.1);
    for (dx, col, r) in [(400.0, DON, BIG_R), (260.0, KAT, NOTE_R), (150.0, DON, NOTE_R)] {
        c.circle(HIT_X + dx, LANE_Y, r, rgba(col, 1.0), rgba(WHITE, 1.0), r * 0.1);
    }
    c.rect(0.0, lane_top, PANEL_W, LANE_H, rgba(0x191921, 1.0));
    c.circle(PANEL_W / 2.0, LANE_Y, 46.0, rgba(0x2f2f3e, 1.0), rgba(0x4a4a60, 1.0), 2.0);
    c.circle(PANEL_W / 2.0, LANE_Y, 32.0, rgba(0xe9dcc3, 1.0), rgba(0x8a7b63, 1.0), 2.0);
    c.text_right(w - 18.0, 18.0, 24.0, rgba(WHITE, 1.0), "00000000");
    c.text_right(w - 18.0, 50.0, 14.0, rgba(0xc8c8d8, 1.0), "100.00%");
    c.text_centered(w / 2.0, 300.0, 14.0, rgba(WHITE, 0.85), "drag to move, edges to resize");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> Game {
        let map = beatmap::calibration_map(500.0, 0, 64);
        Game::new(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None)
    }

    fn hits(g: &mut Game, errors: &[f64], offset: f64) {
        g.stats.errors.extend_from_slice(errors);
        g.stats.error_offsets.extend(std::iter::repeat(offset).take(errors.len()));
    }

    /// Deterministic spread around `mean` with standard deviation close to `sd`.
    fn spread(mean: f64, sd: f64, n: usize) -> Vec<f64> {
        (0..n).map(|i| mean + if i % 2 == 0 { sd } else { -sd }).collect()
    }

    #[test]
    fn needs_ten_hits() {
        let mut g = game();
        hits(&mut g, &[9.0; 9], 0.0);
        assert!(g.offset_suggestion(0.0).is_none());
    }

    #[test]
    fn consistent_player_gets_full_correction_quickly() {
        let mut g = game();
        hits(&mut g, &spread(8.0, 2.0, 25), 0.0);
        let h = g.offset_suggestion(0.0).unwrap();
        assert!(h.significant);
        assert_eq!(h.needed, 20);
        assert_eq!(h.estimate.suggested_offset, 8.0);
    }

    #[test]
    fn noisy_player_needs_more_hits_and_gets_no_false_alarm() {
        let mut g = game();
        // UR ~200 with a 4 ms bias: over 30 hits that is within the noise.
        hits(&mut g, &spread(4.0, 20.0, 30), 0.0);
        let h = g.offset_suggestion(0.0).unwrap();
        assert!(h.needed > 100, "needed {}", h.needed);
        assert!(!h.significant);
        // A real 12 ms bias at the same UR, once enough hits exist, is flagged but shrunk a little.
        let mut g = game();
        hits(&mut g, &spread(12.0, 20.0, 200), 0.0);
        let h = g.offset_suggestion(0.0).unwrap();
        assert!(h.significant);
        assert!((11.0..=12.0).contains(&h.estimate.suggested_offset), "{}", h.estimate.suggested_offset);
    }

    #[test]
    fn simple_mode_uses_plain_last_40_average() {
        let mut g = game();
        g.ur_scaled_hint = false;
        hits(&mut g, &spread(4.0, 20.0, 30), 0.0);
        let h = g.offset_suggestion(0.0).unwrap();
        // Same noisy hits the UR-scaled mode calls "looks good": simple mode flags and applies all of it.
        assert!(h.significant);
        assert_eq!(h.estimate.suggested_offset, 4.0);
        assert_eq!(h.estimate.hits, 30);
    }

    #[test]
    fn applying_mid_song_does_not_double_count() {
        let mut g = game();
        hits(&mut g, &spread(10.0, 1.0, 30), 20.0);
        assert_eq!(g.offset_suggestion(20.0).unwrap().estimate.suggested_offset, 30.0);
        // After applying, the same hits re-expressed under +30 suggest staying at +30.
        let h = g.offset_suggestion(30.0).unwrap();
        assert_eq!(h.estimate.suggested_offset, 30.0);
        assert!(!h.significant);
    }
}
