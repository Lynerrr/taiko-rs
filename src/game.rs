//! Gameplay: judging, scoring and drawing the playfield.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

static HIDE_GREAT: AtomicBool = AtomicBool::new(true);

/// GREAT hits without their splash (no flash on the hit target, no "GREAT"), like osu! skins
/// with an empty taiko-hit300: only OKs and misses stand out (Settings -> Playfield).
pub fn set_hide_great(on: bool) {
    HIDE_GREAT.store(on, Ordering::Relaxed);
}

/// Whether `j` gets its flash and label.
fn splashes(j: Judge) -> bool {
    !(j == Judge::Great && HIDE_GREAT.load(Ordering::Relaxed))
}

use crate::audio::{Engine, Sfx, Track};
use crate::beatmap::{self, Beatmap, Kind};
use crate::calibrate;
use crate::clock::GameClock;
use crate::mods::{Mod, Mods, Speed};
use crate::hud::Part;
use crate::multiverse::{self, Shape as NoteShape, Universe};
use crate::shapeshift;
use crate::render::{accent, progress, rgba, Canvas, Font, Hit, Shape, Sprite, VIRTUAL_HEIGHT};

/// The drum on the left: lazer's input drum (180 of a 768-tall screen, in our 480 units).
pub const PANEL_W: f32 = 112.5;
/// Centre of the hit target: x = 160 of osu!'s 480-tall gamefield, as in stable and lazer.
pub const HIT_X: f32 = 160.0;
pub const LANE_Y: f32 = 160.0;
pub const LANE_H: f32 = 118.0;
const NOTE_R: f32 = 31.0;
const BIG_R: f32 = 46.0;
/// A spinner being hit: its circle over the drum (a skin's 128 px spinner image at this radius).
const SPIN_R: f32 = 50.0;
/// Second key of a big note must land within this window of the first.
const STRONG_WINDOW: f64 = 30.0;
const HIT_ANIM_MS: f64 = 160.0;
/// How long a short fake slider's body takes to fade once it reaches the drum.
const FAKE_FADE_MS: f64 = 250.0;
/// The kiai beat flash on each note: its strength on the beat (fading to 0 by the next), how
/// much of it a circle takes, and the shortest beat that still counts as one.
const KIAI_FLASH_ALPHA: f32 = 0.337;
const CIRCLE_FLASH: f32 = 0.3;
const KIAI_FLASH_MIN_BEAT_MS: f64 = 50.0;
/// The combo break plays a little louder than a hitsound (about +1.6 dB): noticeable over the
/// music without being jarring.
const COMBO_BREAK_GAIN: f32 = 1.2;

pub const DON: u32 = 0xEB452C;
pub const KAT: u32 = 0x438EAD;
const ROLL: u32 = 0xFCB806;
/// Drumroll ticks (`sliderscorepoint`), as stable tints them (owner's screenshot, IT'S SOOOO
/// CREATIVE): yellow on the beat, pale orange on the half beat, the image's own colour elsewhere.
const ROLL_TICK_BEAT: u32 = 0xFFFF00;
const ROLL_TICK_HALF: u32 = 0xFFE696;
const SWELL: u32 = 0xF39A2B;
const WHITE: u32 = 0xFFFFFF;
/// Colorless: dons and kats alike.
const GREY: u32 = 0x8c8ca6;

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

/// How presses that don't make a hit are judged (the mash penalty).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mash {
    /// The rule: a press goes to the nearest note (see `judge_nearest`).
    Nearest,
    /// Replays from before it (version 1), judged the way they were played: such presses
    /// leave the note alone (`Ignore`), or always miss the next note (`Strict`, osu!'s).
    Ignore,
    Strict,
}

impl Mash {
    /// A version 1 replay's `anti_mash` setting.
    pub fn legacy(anti_mash: bool) -> Mash {
        if anti_mash { Mash::Strict } else { Mash::Ignore }
    }
}

/// How a play's score is worked out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scoring {
    /// Replays from before it (versions 1 and 2): points per hit that grow with the combo, plus
    /// drumrolls and swells, so a play's top score depended on the map.
    Classic,
    /// Like osu!'s ScoreV2: a clean play of any map with no mods is 1,000,000, and the mods
    /// multiply that. 700,000 comes from the hits weighted by the combo they were made at, 300,000
    /// from accuracy (to the 10th power), each as far as the play has got. Drumrolls and swells
    /// don't count.
    Standard,
}

/// What a hit is worth in the combo part of `Scoring::Standard`: later hits in a combo more.
fn combo_weight(combo: u32) -> f64 {
    (combo as f64 + 1.0).sqrt()
}

struct StrongPending {
    note: usize,
    kat: bool,
    key: usize,
    at: f64,
}

pub struct Game {
    /// Random's colouring of the notes: a new one each play, kept with the replay so it plays
    /// back the same; 0 is the one per map from before seeds.
    pub seed: u64,
    pub map: Beatmap,
    clock: GameClock,
    state: Vec<NoteState>,
    cursor: usize,
    pub stats: Stats,
    pub windows: Windows,
    /// Gameplay time (ms) of the current frame.
    pub time: f64,
    /// Map time play starts at (before the first note, or the lead-in).
    start: f64,
    end_time: f64,
    track_ms: f64,
    /// The song as it plays (the results screen plays it on).
    pub track: Arc<Track>,
    first_note: f64,
    /// Game time of the last press per key: left rim, left centre, right centre, right rim.
    presses: [f64; 4],
    last_judge: Option<(Judge, f64)>,
    recent_errors: VecDeque<(f64, f64, Judge)>,
    strong: Option<StrongPending>,
    min_velocity: f64,
    /// The beat length covering the most map time: what pop-up fake sliders fade in by.
    main_beat: f64,
    visible: Vec<usize>,
    /// Raw audio position seen by the last update, for diagnostics.
    pub last_audio_ms: Option<f64>,
    pub raw_time: f64,
    auto_key: usize,
    /// Pause menu cursor while paused.
    pub paused: Option<usize>,
    paused_at: Instant,
    /// Counting down to unpause: (started, ends). The menu is hidden meanwhile.
    pub resuming: Option<(Instant, Instant)>,
    /// Real milliseconds spent playing (not paused, nor counting down to resume, nor failed),
    /// for the profile's play time; and the frame they were last counted at.
    pub played_ms: f64,
    played_at: Option<Instant>,
    /// The scroll speed setting this play is at: fixed when it starts (it counts for pp), and
    /// a replay's own.
    pub scroll_speed: f64,
    /// Calibration: draw no notes or barlines (play by ear) / play no hitsounds on presses.
    pub hide_notes: bool,
    /// Draw the HP bar (not in calibration).
    pub show_hp: bool,
    pub mute_hits: bool,
    /// Show the live offset suggestion under the hit error meter.
    pub show_offset_hint: bool,
    /// Kiai time tints the lane and pulses the hit target.
    pub kiai_effects: bool,
    /// Interface parts hidden (the interface toggle).
    pub hidden: crate::hud::Parts,
    /// Display latency to make up for (real ms): the lane is drawn as it will be this much later,
    /// when the frame reaches the screen. Judging stays on the audio clock.
    pub visual_offset: f64,
    /// How presses that don't make a hit are judged.
    pub mash: Mash,
    pub scoring: Scoring,
    /// Standard scoring: the combo points a clean play of the part played gives, and how many
    /// notes it has. Worked out at the first hit (after the part skipped is known).
    score_basis: Option<(f64, u32)>,
    /// Standard scoring: the combo points made so far.
    combo_points: f64,
    /// Presses let off as extra hits since a note was last judged (`judge_nearest`).
    extra_taps: u32,
    /// Record presses (map time judged at, key) for a replay.
    pub record: bool,
    /// Also keep `inputs` without saving a replay (a solo play streamed to watching friends).
    pub log_inputs: bool,
    /// Someone else's play being watched: no pause menu (it holds while they pause) and no
    /// skip prompt (it follows their skips).
    pub watched: bool,
    pub inputs: Vec<(f64, u8)>,
    /// Playing a replay back: its presses and the next one due; and whose it is.
    pub replay: Option<(Vec<(f64, u8)>, usize)>,
    pub replay_of: Option<String>,
    /// The pp counter: works out the play's pp so far (`pp::Live`), and its latest answer.
    pub live_pp: Option<crate::pp::Live>,
    live_pp_value: Option<f64>,
    /// Text shown at the top ("REPLAY - x", "SPECTATING - x").
    pub banner: Option<String>,
    /// When set, every hitsound played is logged (map time, sound, volume), e.g. for video audio.
    pub sfx_log: Option<Vec<(f64, Sfx, f32)>>,
    /// A miss broke a combo of 20 or more at this map time: the combo break sound is due.
    combo_break: Option<f64>,
    /// Scale the suggestion to current UR; off = plain average of the last 40 hits.
    pub ur_scaled_hint: bool,
    /// Offset used for the current frame / last press.
    offset: f64,
    /// Speed modifier: map milliseconds per real millisecond (1.5 = DT). Taken from the track,
    /// which was prepared at this rate. The game clock runs in track time; map time is
    /// `speed.map(track - offset)`, so the offset stays in real milliseconds.
    pub rate: f64,
    /// Map time ↔ song time: `rate` throughout, or ramping from it (Wind Up / Down).
    speed: Speed,
    pub mods: Mods,
    score_mult: f64,
    /// HP (0-1), like osu!lazer's taiko: starts empty, hits fill it, misses drain it, and the
    /// play fails if it's under half at the end.
    pub hp: f64,
    /// HP per GREAT / OK (×3.0 / ×1.1 of this) and per miss.
    hp_hit: f64,
    hp_miss: f64,
    /// Started mid-map (a start point): too few notes to judge HP by, so only SD/PF fail.
    hp_exempt: bool,
    /// Failed by Sudden Death / Perfect: map time and when (the play ends a moment later).
    failed: Option<(f64, Instant)>,
    /// Alternate: the hand of the last accepted press (true = left) and when; None while either
    /// hand may hit (after a finisher, a drumroll or a swell, until a press hits a note).
    last_hand: Option<(bool, f64)>,
    /// Party: map time of the last big hit (the lane shakes).
    quake_at: f64,
    /// Cracked: the misses that cracked the lane (map time) and how many notes had been hit
    /// then (it mends over the next 50).
    cracks: Vec<(f64, u32)>,
    /// Crumble: the pieces of the lane hits have knocked out, until they settle back.
    crumbs: Vec<Crumb>,
    /// Panic / Mercy: map times of the latest misses.
    misses: VecDeque<f64>,
    /// Drop / Barrel Roll: where each kiai starts (map ms) and the beat length there.
    kiai_starts: Vec<(f64, f64)>,
    /// Each kiai's (start, end) map ms; the last may run to infinity.
    kiai_bands: Vec<(f64, f64)>,
    /// Roulette: where each 4-bar section starts (map ms), its speed, and the velocity its
    /// marker scrolls at.
    sections: Vec<(f64, f64, f64)>,
    /// Decoy: the fake notes (never judged).
    decoys: Vec<beatmap::Note>,
    /// Each note's rhythm: the stream it's part of (see `flows`).
    flows: Vec<u8>,
    /// Decoy: the rhythm each fake note pretends to be part of (the note before it's).
    decoy_flows: Vec<u8>,
    /// Liar: the map time each note is drawn at far off (its own, or a 1/4's for a triplet).
    lies: Vec<f64>,
    /// Overtime: when (map ms) he shows up, and the next one due.
    scares: Vec<f64>,
    next_scare: usize,
    /// Multiverse: where the focus moves to another playfield.
    focus_switches: Vec<FocusSwitch>,
    /// Shapeshift: its setup and the shape of each section (index into `SHAPESHIFT`).
    shapeshift: shapeshift::Config,
    shape_order: Vec<u8>,
    /// Multiverse's universes when the play brought its own (a lobby host's); None: the
    /// player's, as they set them up.
    universes: Option<[Universe; multiverse::COUNT]>,
    /// Where the playfield in play was drawn last frame (x, y, w, h on the canvas): the lane,
    /// wherever the mods took it; with Multiverse, the playfield in focus.
    zone: Option<[f32; 4]>,
}

/// Decoy: how opaque the fake notes are next to real ones.
const DECOY_ALPHA: f32 = 0.6;

/// Overtime: how long (ms) he stays on screen.
const SCARE_MS: f64 = 900.0;

/// Mania: how much of the lane stands above the drum (virtual units).
const MANIA_LANE: f32 = 340.0;
/// Spin: how long the lane is as it turns about the middle of the screen.
const SPIN_LANE: f32 = 300.0;

/// Multiverse: how many playfields there are.
const FIELD_COUNT: usize = 8;
/// Multiverse: where each playfield circles about, all over the screen and over each other: the
/// left edge (a part of the screen's width), the middle of the lane (virtual units) and how far
/// it leans out of focus (radians).
const HOMES: [(f32, f32, f32); FIELD_COUNT] = [
    (0.02, 70.0, -0.04),
    (0.46, 105.0, 0.05),
    (0.18, 150.0, 0.03),
    (0.5, 205.0, -0.05),
    (0.05, 235.0, -0.03),
    (0.34, 285.0, 0.04),
    (0.12, 330.0, 0.035),
    (0.48, 365.0, -0.04),
];
/// Multiverse: how far (virtual units) a playfield out of focus circles from its home, across
/// and up and down.
const ORBIT: (f32, f32) = (46.0, 28.0);
/// Multiverse: how much of the screen's width a playfield spans in focus.
const FIELD_W: f32 = 0.78;
/// Multiverse: a playfield's size out of focus and in it.
const FIELD_ZOOM: (f32, f32) = (0.36, 0.6);
/// Multiverse: the playfield that has the focus first (near the middle).
pub(crate) const FIRST_FOCUS: usize = 4;
/// Multiverse: the colour of the focus (its halo, frame and burst).
const FOCUS_COL: u32 = 0xbfe9ff;
/// Multiverse: how long (ms) the burst lasts as a playfield takes the focus.
const FOCUS_BURST_MS: f64 = 600.0;

/// Multiverse: the focus moving to another playfield.
#[derive(Clone, Copy, Debug)]
struct FocusSwitch {
    /// The bar it lands on (map ms).
    at: f64,
    /// The playfield that gets it.
    field: usize,
    /// The beat length there.
    beat: f64,
}

/// Multiverse: how a playfield looks this frame.
#[derive(Clone, Copy, Default)]
struct Focus {
    /// How much it has the focus (0-1): brightness, steadiness.
    focus: f32,
    /// How much its dons and kats have their colours (0-1): ahead of the focus, so there's time
    /// to look over before it lands.
    colour: f32,
    /// How far it has grown to its size in focus (overshooting as it takes it).
    grow: f32,
    /// It's next: a flash on each of the two beats before it takes the focus.
    call: f32,
    /// How far its burst has spread (0-1), just after it took the focus.
    burst: Option<f32>,
    /// How still it is: 0 circling its home, 1 parked where it's played (it parks as it's
    /// called).
    settle: f32,
    /// The focus is heading for it (0-1, over the two beats before it takes it): it lights up,
    /// pings, and sparks run to it from the one in focus.
    aim: f32,
}

impl Focus {
    /// The one playfield there is without Multiverse.
    const FULL: Focus = Focus { focus: 1.0, colour: 1.0, grow: 1.0, call: 0.0, burst: None, settle: 1.0, aim: 0.0 };
}

/// osu!px per ms -> virtual units per ms at scroll speed 1.0, so that a note stays on screen
/// exactly as long as in osu!. Stable scrolls notes at their velocity across a 480-tall gamefield
/// (the same units as ours) from its right edge to the hit target at x = 160; lazer copies that as
/// a time range, with the aspect ratio clamped to 5:4..16:9. Our hit target is osu!'s too, so this
/// is 1.0 except outside that aspect range, where the lane is longer or shorter than osu!'s.
fn osu_speed_scale(w: f32) -> f64 {
    let aspect = (w / VIRTUAL_HEIGHT).clamp(5.0 / 4.0, 16.0 / 9.0) as f64;
    (w - HIT_X).max(1.0) as f64 / (aspect * VIRTUAL_HEIGHT as f64 - 160.0)
}

/// How much faster than on a plain lane notes go along a lane `len` long on a canvas `w` wide.
/// Rails only bend the lane: notes go along it as they would along a plain one, as many to a
/// stretch of lane, so a long rail (a spiral, a loop) shows more of the map ahead rather than
/// rushing it in. A playfield shrunk to fit the shape is the same picture, smaller: speeding
/// notes up to make up for it spread them out and rushed them along the rail. Mania's and Spin's
/// shorter lanes keep notes on them for the usual time.
fn lane_stretch(rail: bool, len: f32, w: f32) -> f64 {
    if rail { 1.0 } else { (len / (w - HIT_X)) as f64 }
}

/// What bends note positions this frame: drawing only, judging always uses real time.
struct Scroll {
    t: f64,
    /// Virtual units per (osu!pixel per ms × ms): scroll speed setting × EZ/HR.
    speed: f64,
    /// Length of the lane ahead of the hit target.
    lane: f64,
    mods: Mods,
    /// Pulse, Stutter, Drop, Lag Spike...: how far (ms) far-away notes are shifted in time right
    /// now.
    warp: f64,
    /// Speed factor of every note right now (Panic, Mercy).
    live: f64,
}

impl Scroll {
    /// Speed factor of a note at map time `time` from the scroll mods.
    fn factor(&self, time: f64) -> f64 {
        let mut k = self.live;
        if self.mods.has(Mod::Wave) {
            k *= 1.0 + 0.35 * (std::f64::consts::TAU * time / 4000.0).sin();
        }
        if self.mods.has(Mod::SvChaos) {
            k *= 0.7 + 0.6 * hash01(time);
        }
        k
    }

    /// The smallest factor `factor` can give, to know how far ahead notes may still be on screen.
    fn min_factor(&self) -> f64 {
        let mut k = self.live.min(1.0);
        if self.mods.has(Mod::Wave) {
            k *= 0.65;
        }
        if self.mods.has(Mod::SvChaos) {
            k *= 0.7;
        }
        k
    }

    /// Screen x of something at map time `time` scrolling at `velocity`.
    fn x(&self, time: f64, velocity: f64) -> f32 {
        let v = velocity * self.speed * self.factor(time);
        // Pulse / Stutter shift the time of far notes; near the drum the shift fades out, so
        // a note always reaches it exactly on time.
        let near = ((time - self.t) * v / self.lane).clamp(0.0, 1.0);
        let u = (time - self.t - self.warp * near) * v / self.lane;
        let u = if !(0.0..=1.0).contains(&u) {
            u
        } else if self.mods.has(Mod::Accel) {
            1.6 * u - 0.6 * u * u
        } else if self.mods.has(Mod::Brake) {
            0.4 * u + 0.6 * u * u
        } else {
            u
        };
        HIT_X + (u * self.lane) as f32
    }
}

/// Pulse / Stutter's time shift of far notes at map time `t` (see `Scroll::warp`): Pulse makes
/// them jump forward on each beat, Stutter hold, then catch up.
fn beat_warp(mods: Mods, cp: &beatmap::ControlPoint, t: f64) -> f64 {
    let p = cp.beat_phase(t);
    if mods.has(Mod::Pulse) {
        cp.beat_length * ((1.0 - (1.0 - p).powi(3)) - p)
    } else if mods.has(Mod::Stutter) {
        let x = ((p - 0.5) / 0.5).clamp(0.0, 1.0);
        cp.beat_length * (x * x * (3.0 - 2.0 * x) - p)
    } else {
        0.0
    }
}

/// 0 below `e0`, 1 above `e1`, smoothly in between.
fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    let x = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Multiverse's breath, 0 to 1 over a beat `beat_ms` long at `phase`: it swells over the first
/// 90 ms (less of a fast beat) and eases out by the next, so no frame jumps on the beat.
fn breath(phase: f64, beat_ms: f64) -> f32 {
    let rise = (90.0 / beat_ms.max(1.0)).min(0.4);
    if phase < rise { smooth(0.0, rise, phase) as f32 } else { (1.0 - smooth(rise, 1.0, phase)) as f32 }
}

/// The bar map time `t` is in: its barline's index, start and length (the last bar is
/// `fallback` long). None before the first barline.
fn bar_at(bars: &[beatmap::Barline], t: f64, fallback: f64) -> Option<(usize, f64, f64)> {
    let i = bars.partition_point(|b| b.time <= t + 1.0).checked_sub(1)?;
    let start = bars[i].time;
    let len = bars.get(i + 1).map_or(fallback, |b| b.time - start).max(1.0);
    Some((i, start, len))
}

/// Radar's beam.
const RADAR_COL: u32 = 0x5ef0a0;
/// Radar: notes this close to the drum are always fully lit.
const RADAR_NEAR: f32 = 100.0;

/// Radar: how many beats the beam takes to cross the lane, and in kiai.
const RADAR_BEATS: (f32, f32) = (2.0, 1.0);

/// Radar: where the beam is, 0 at the drum to 1 at the lane's end, `beats` since the red line.
/// It leaves the drum every `RADAR_BEATS` beats.
fn radar_sweep(beats: f32, kiai: bool) -> f32 {
    (beats / if kiai { RADAR_BEATS.1 } else { RADAR_BEATS.0 }).rem_euclid(1.0)
}

/// Radar: how far a note `s` along a lane `len` long that comes `u` closer every sweep is from
/// the beam's last pass (0) to its next (1), the beam at `sweep`. None if the note wasn't on
/// screen yet when the beam last went by.
fn radar_age(s: f32, u: f32, len: f32, sweep: f32) -> Option<f32> {
    let b = sweep * len;
    // How far the beam has gone since; the note came closer meanwhile, as the beam and the note
    // close in on each other at `len + u` a sweep.
    let back = if s <= b { b - s } else { b + len - s };
    let u = u.max(0.0);
    (s + u * back / (len + u) <= len).then_some(back / len)
}

/// Radar: how lit that note is: fully as the beam passes, gone by the next one (and unseen till
/// it first passes them); always fully by the drum.
fn radar_light(s: f32, u: f32, len: f32, sweep: f32) -> f32 {
    let near = smooth((RADAR_NEAR + 50.0) as f64, RADAR_NEAR as f64, s as f64) as f32;
    let lit = radar_age(s, u, len, sweep).map_or(0.0, |a| (1.0 - a).max(0.0).powf(1.5));
    lit.max(near)
}

/// Echo: where its bar sits, under the lane.
const ECHO_Y: f32 = LANE_Y + LANE_H / 2.0 + 50.0;
/// Echo: how long a note rings once the cursor reaches it.
const ECHO_RING_MS: f32 = 300.0;

/// Rails: the lane bent into a curve. Distances along it (`s`) count from the hit target; it
/// leaves the hit target heading right, so the drum stays where it is, and behind the hit
/// target it runs straight on to the left.
struct Rail {
    /// Position and heading (radians, anticlockwise from right) every `RAIL_STEP` from s = 0.
    pts: Vec<(f32, f32, f32)>,
}

const RAIL_STEP: f32 = 4.0;

impl Rail {
    /// From its curvature (radians per unit) along the way, `len` long.
    fn build(bend: impl Fn(f32) -> f32, len: f32) -> Rail {
        let n = (len / RAIL_STEP).ceil() as usize + 2;
        let mut pts = Vec::with_capacity(n);
        let (mut x, mut y, mut a) = (HIT_X, LANE_Y, 0.0f32);
        for k in 0..n {
            pts.push((x, y, a));
            let turn = bend((k as f32 + 0.5) * RAIL_STEP) * RAIL_STEP;
            let mid = a + turn / 2.0;
            x += mid.cos() * RAIL_STEP;
            y -= mid.sin() * RAIL_STEP;
            a += turn;
        }
        Rail { pts }
    }

    /// Point and heading `s` along.
    fn at(&self, s: f32) -> (f32, f32, f32) {
        if s <= 0.0 {
            return (HIT_X + s, LANE_Y, 0.0);
        }
        let f = s / RAIL_STEP;
        let k = f as usize;
        let Some(&(x1, y1, a1)) = self.pts.get(k + 1) else {
            let (x, y, a) = *self.pts.last().unwrap_or(&(HIT_X, LANE_Y, 0.0));
            let d = s - (self.pts.len() - 1) as f32 * RAIL_STEP;
            return (x + a.cos() * d, y - a.sin() * d, a);
        };
        let (x0, y0, a0) = self.pts[k];
        let u = f - k as f32;
        (x0 + (x1 - x0) * u, y0 + (y1 - y0) * u, a0 + (a1 - a0) * u)
    }
}

/// The lane shapes of the rails mods.
#[derive(Clone, Copy, PartialEq, Debug)]
enum RailShape {
    Straight,
    Zigzag,
    UTurn,
    Spiral,
    Loop,
    Elbow,
}

/// Loop: how far from the hit target it starts, its radius, and the angle the lane into it comes
/// up at (short of a full turn by that). Full size on a 16:9 screen, with the lane coming in right
/// of the hit error meter.
const LOOP_AT: f32 = 390.0;
const LOOP_R: f32 = 110.0;
const LOOP_EXIT: f32 = 0.6;
const LOOP_TURN: f32 = std::f32::consts::TAU - LOOP_EXIT;

/// Shapeshift's shapes, by the index `shapeshift::SHAPES` names them with.
const SHAPESHIFT: [RailShape; 6] = [RailShape::Zigzag, RailShape::UTurn, RailShape::Spiral, RailShape::Loop, RailShape::Elbow, RailShape::Straight];

/// Shapeshift shape `k`'s lane on a 16:9 screen, as points along its middle (for the setup's
/// thumbnails).
pub fn rail_outline(k: usize) -> Vec<(f32, f32)> {
    let shape = SHAPESHIFT[k % SHAPESHIFT.len()];
    let w = 853.3;
    let rail = Rail::build(|s| shape.bend(s), shape.length(w));
    rail.pts.iter().step_by(3).map(|&(x, y, _)| (x, y)).collect()
}

impl RailShape {
    fn of(mods: Mods) -> Option<RailShape> {
        [(Mod::Zigzag, RailShape::Zigzag), (Mod::UTurn, RailShape::UTurn), (Mod::Spiral, RailShape::Spiral), (Mod::Loop, RailShape::Loop), (Mod::Elbow, RailShape::Elbow)]
            .into_iter()
            .find(|&(m, _)| mods.has(m))
            .map(|(_, shape)| shape)
    }

    /// Curvature (radians per unit, anticlockwise) `s` along the lane.
    fn bend(self, s: f32) -> f32 {
        use std::f32::consts::PI;
        // Turning by `angle` over `len` units from `at`.
        let turn = |at: f32, len: f32, angle: f32| if (at..at + len).contains(&s) { angle / len } else { 0.0 };
        match self {
            RailShape::Straight => 0.0,
            RailShape::Zigzag => {
                // Up 30 degrees, then 60-degree corners every 150 units.
                if s < 120.0 {
                    return 0.0;
                }
                let k = ((s - 120.0) / 150.0).floor();
                let angle = if k == 0.0 { PI / 6.0 } else if k as i64 % 2 == 1 { -PI / 3.0 } else { PI / 3.0 };
                turn(120.0 + 150.0 * k, 36.0, angle)
            }
            RailShape::UTurn => turn(240.0, PI * 100.0, PI),
            // Far enough out that its turns pass well right of the hit target, each a lane wider
            // than the last.
            RailShape::Spiral => {
                if s < 340.0 { 0.0 } else { 1.0 / (90.0 + 0.15 * (s - 340.0)) }
            }
            // All the way round but for `LOOP_EXIT`: the lane coming in crosses the straight to
            // the drum at that angle, not along it (notes a second apart side by side).
            RailShape::Loop => turn(LOOP_AT, LOOP_TURN * LOOP_R, LOOP_TURN),
            RailShape::Elbow => turn(220.0, PI * 40.0, PI / 2.0),
        }
    }

    /// How long the lane is on a canvas `w` wide.
    fn length(self, w: f32) -> f32 {
        use std::f32::consts::PI;
        match self {
            RailShape::Straight => w - HIT_X,
            RailShape::Zigzag => (w - HIT_X) * 1.15,
            // Back along the top to the left edge.
            RailShape::UTurn => 240.0 + PI * 100.0 + HIT_X + 240.0,
            RailShape::Spiral => 1320.0,
            // On out of the bottom right of the screen.
            RailShape::Loop => LOOP_AT + LOOP_TURN * LOOP_R + 500.0,
            RailShape::Elbow => 220.0 + PI * 40.0 + 300.0,
        }
    }

    /// How far down the playfield moves to keep the shape on screen.
    fn drop(self) -> f32 {
        match self {
            RailShape::Straight => 0.0,
            RailShape::Zigzag => 35.0,
            RailShape::UTurn => 150.0,
            RailShape::Spiral => 105.0,
            RailShape::Loop => 135.0,
            RailShape::Elbow => 200.0,
        }
    }

    /// How much the playfield shrinks to fit the shape on a canvas `w` wide: the Spiral is
    /// tall, and narrow playfields squeeze the rest about the hit target.
    fn zoom(self, w: f32) -> f32 {
        // How far right the shape reaches, lane edge included.
        let right = match self {
            RailShape::Straight | RailShape::Zigzag => return 1.0,
            RailShape::UTurn => 560.0,
            RailShape::Spiral => 660.0,
            RailShape::Loop => HIT_X + LOOP_AT + LOOP_R + LANE_H / 2.0,
            RailShape::Elbow => 520.0,
        };
        let tall: f32 = if self == RailShape::Spiral { 0.85 } else { 1.0 };
        tall.min((w - 8.0 - HIT_X) / (right - HIT_X)).max(0.3)
    }
}

/// Crumble: a piece of the lane a hit knocked out of line.
#[derive(Clone, Copy, Debug)]
struct Crumb {
    /// Map time of the hit.
    at: f64,
    /// How long it stays out (ms).
    life: f64,
    /// 1 for a 1/1 note at 200 combo; smaller for faster rhythms and lower combos.
    size: f32,
    /// Where it starts, as a part of the lane's length, and how long it is (px).
    start: f32,
    len: f32,
    /// Sideways shift at its furthest.
    shift: f32,
}

/// Crumble: pieces only break off this part of the lane (as parts of its length), so notes
/// reach the drum where they're expected.
const CRUMBLE_SPAN: (f32, f32) = (0.25, 0.92);
/// Crumble: the most the knocked-out pieces at one spot add up to sideways.
const CRUMBLE_MAX_SHIFT: f32 = 55.0;
/// Crumble: how hard a hit knocks the lane apart at `combo`: a quarter as hard as at 200 combo
/// to begin with, then on up to 1.4x by 400.
fn crumble_power(combo: u32) -> f32 {
    let c = combo as f32;
    0.25 + 0.75 * (c / 200.0).min(1.0) + 0.4 * ((c - 200.0) / 200.0).clamp(0.0, 1.0)
}

/// How long (ms) Crumble's pieces take to snap back into line after a miss.
const CRUMBLE_MEND_MS: f64 = 250.0;

/// Where things on the lane go this frame: the rail, and how Shatter and Cracked knock pieces
/// of it sideways. `s` is the distance ahead of the hit target.
struct LaneGeo {
    rail: Option<Rail>,
    len: f32,
    /// Shatter: the sideways shift of each quarter of the lane (the first stays put).
    shatter: [f32; 4],
    /// Cracked and Crumble: (start, length, sideways shift, seam opacity) of each knocked-out
    /// piece.
    cracks: Vec<(f32, f32, f32, f32)>,
}

impl LaneGeo {
    /// Whether the lane is more than one straight rect.
    fn bent(&self) -> bool {
        self.rail.is_some() || self.shatter.iter().any(|&d| d != 0.0) || !self.cracks.is_empty()
    }

    fn shift(&self, s: f32) -> f32 {
        let mut d = 0.0;
        if s > 0.0 {
            d += self.shatter[((s / self.len * 4.0) as usize).min(3)];
        }
        let mut cracks = 0.0;
        for &(a, l, o, _) in &self.cracks {
            if (a..a + l).contains(&s) {
                cracks += o;
            }
        }
        d + cracks.clamp(-CRUMBLE_MAX_SHIFT, CRUMBLE_MAX_SHIFT)
    }

    /// Point and heading `s` along the lane, `side` units off its middle (down on a plain lane).
    fn at(&self, s: f32, side: f32) -> (f32, f32, f32) {
        let side = side + self.shift(s);
        let (x, y, a) = match &self.rail {
            Some(r) => r.at(s),
            None => (HIT_X + s, LANE_Y, 0.0),
        };
        (x + a.sin() * side, y + a.cos() * side, a)
    }
}

/// Portal: notes reaching `entry` (distance along the lane) come out at `exit` and cover the
/// rest of the way in the time left. Returns where a note `s` along is drawn and how big
/// (shrinking into the entry, growing out of the exit).
fn through_portal(s: f32, (entry, exit): (f32, f32)) -> (f32, f32) {
    if s <= 0.0 {
        (s, 1.0)
    } else if s >= entry {
        (s, ((s - entry) / 25.0).clamp(0.15, 1.0))
    } else {
        let s = s * exit / entry;
        (s, ((exit - s) / 25.0).clamp(0.15, 1.0))
    }
}

/// A rect `len` long and `thick` across, centred on (`x`, `y`) and turned to heading `a`.
fn lane_rect(c: &mut Canvas, (x, y, a): (f32, f32, f32), len: f32, thick: f32, fill: [f32; 4]) {
    c.rect(x - len / 2.0, y - thick / 2.0, len, thick, fill);
    if let Some(sh) = c.shapes.last_mut() {
        sh.angle = -a;
    }
}

/// A capsule of radius `r` from `p` to `q`.
fn segment(c: &mut Canvas, p: (f32, f32), q: (f32, f32), r: f32, fill: [f32; 4]) {
    let len = (q.0 - p.0).hypot(q.1 - p.1);
    let (x, y) = ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0);
    c.shapes.push(Shape { center: [x, y], half: [len / 2.0 + r, r], fill, border: fill, params: [r, 0.0, 0.0, 0.0], angle: (q.1 - p.1).atan2(q.0 - p.0) });
}

/// `base` with `over` laid on top at opacity `a`.
fn tint(base: [f32; 4], over: u32, a: f32) -> [f32; 4] {
    let o = rgba(over, 1.0);
    [base[0] + (o[0] - base[0]) * a, base[1] + (o[1] - base[1]) * a, base[2] + (o[2] - base[2]) * a, base[3]]
}

/// `a` mixed `t` of the way to `b`.
pub(crate) fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |hex: u32, s: u32| ((hex >> s) & 0xff) as f32;
    [16, 8, 0].into_iter().fold(0, |out, s| out | (((ch(a, s) + (ch(b, s) - ch(a, s)) * t).round() as u32) << s))
}

/// 0 at 0 and 1 at 1, overshooting by about 10% on the way and settling back.
fn overshoot(x: f32) -> f32 {
    const C: f32 = 1.70158;
    let y = x - 1.0;
    1.0 + (C + 1.0) * y * y * y + C * y * y
}

/// A pop-up fake slider at map time `vt`: (lane distance of its head, half its body, alpha), or
/// None for any other note. A long fake slider on a gimmick red line (a 1 ms beat) is too fast to
/// ever be on screen; stable shows it standing still, farther out the longer it is, for
/// |length| / (100 × SliderMultiplier × SV) of the map's main beats before its time (the mapping
/// doc: "-100 at x1.00 disappears in exactly 1 beat"), fading in from then over POPUP_FADE_MS and
/// gone at its time, no fade out (owner, checked in stable; senpai, notice me! [w], 2026-10-28).
/// `sm` is the map's SliderMultiplier, `sv` the note's.
// ponytail: "too fast" is crossing the lane within a 60 Hz frame; refine if a map shows a middle
// case.
fn popup(n: &beatmap::Note, vt: f64, lane_len: f32, speed: f64, main_beat: f64, sm: f64, sv: f64) -> Option<(f32, f32, f32)> {
    let fast = (lane_len as f64) < 16.0 * n.velocity * speed;
    let until = n.time - vt;
    (n.kind == Kind::Roll && n.fake_len > 0.0 && !n.fake_fades && fast && until > 0.0).then(|| {
        let per = POPUP_PER_LENGTH * lane_len;
        let len = n.fake_length as f32;
        let lead = n.fake_length / (100.0 * sm * sv) * main_beat;
        ((len - POPUP_FROM_LENGTH).max(0.0) * per, len * per, fade_from(lead, POPUP_FADE_MS, until))
    })
}

/// Where a pop-up head stands, fitted to the owner's stable screenshot (1080p): heads 95 px apart
/// per 30 of length, the -300 one 270 px past the target, on a 1560 px lane. So lane fractions per
/// unit of length, counted from length 215: -510, the farthest, at 60%.
// ponytail: a fit to one screenshot; re-measure if another map disagrees.
const POPUP_PER_LENGTH: f32 = 95.0 / 30.0 / 1560.0;
const POPUP_FROM_LENGTH: f32 = 215.0;
/// A pop-up head's fade in: lazer's swell 666 ms (owner: a half beat was too fast, 2026-10-28).
// ponytail: guessed from "too fast"; measure stable's if it still looks off.
const POPUP_FADE_MS: f64 = 666.0;

/// A roll tick's tint at map time `t`, on the beat grid of its red line (`origin`, `beat`):
/// yellow on a beat, pale orange on a half beat, white (the image's own colour) elsewhere.
fn tick_tint(t: f64, origin: f64, beat: f64) -> u32 {
    let at = |division: f64| {
        let x = (t - origin) / beat * division;
        (x - x.round()).abs() < 0.02 * division
    };
    if at(1.0) { ROLL_TICK_BEAT } else if at(2.0) { ROLL_TICK_HALF } else { WHITE }
}

/// A swell's alpha `until` ms before it starts: hidden, then fading in (easing in) from 1.5 beats
/// before it, over lazer's 666 ms or half a beat if shorter (ppy/osu#21769, owner 2026-10-28;
/// taiko_arranger's `spinner_fade` too).
fn swell_fade(beat: f64, until: f64) -> f32 {
    fade_from(1.5 * beat, (beat / 2.0).min(666.0), until)
}

/// Hidden until `lead` ms before, then fading in (easing in) over `fade` ms.
fn fade_from(lead: f64, fade: f64, until: f64) -> f32 {
    let p = ((lead - until) / fade).clamp(0.0, 1.0) as f32;
    p * p
}

/// Flip Flop's horizontal scale at map time `t`: 1, or -1 on every other 8-bar section, turning
/// over (through 0) in the first 350 ms of each.
fn flip_flop(bars: &[beatmap::Barline], t: f64) -> f32 {
    let Some((i, ..)) = bar_at(bars, t, 1.0) else { return 1.0 };
    let section = i / 8;
    if section == 0 {
        return 1.0;
    }
    let side = if section % 2 == 1 { -1.0 } else { 1.0 };
    let turn = ((t - bars[section * 8].time) / 350.0).clamp(0.0, 1.0);
    (-side * (std::f64::consts::PI * turn).cos()) as f32
}

/// A fully saturated colour of hue `h` (0-1), for Rave.
fn hue(h: f64) -> u32 {
    let h = h.rem_euclid(1.0) * 6.0;
    let x = ((1.0 - (h % 2.0 - 1.0).abs()) * 255.0) as u32;
    let (r, g, b) = match h as u32 {
        0 => (255, x, 0),
        1 => (x, 255, 0),
        2 => (0, 255, x),
        3 => (0, x, 255),
        4 => (x, 0, 255),
        _ => (255, 0, x),
    };
    r << 16 | g << 8 | b
}

/// Roulette's 4-bar sections: where each starts (map ms) and its speed. The first is 1x; no
/// two in a row are alike.
fn roulette_sections(map: &Beatmap) -> Vec<(f64, f64)> {
    const SPEEDS: [f64; 5] = [0.5, 0.75, 1.0, 1.5, 2.0];
    let mut out: Vec<(f64, f64)> = Vec::new();
    for b in map.barlines.iter().step_by(4) {
        let mut k = (hash01(b.time + 0.4) * SPEEDS.len() as f64) as usize % SPEEDS.len();
        let prev = out.last().map_or(1.0, |s| s.1);
        if out.is_empty() {
            k = 2;
        } else if SPEEDS[k] == prev {
            k = (k + 1) % SPEEDS.len();
        }
        out.push((b.time, SPEEDS[k]));
    }
    out
}

/// Multiverse: where a playfield parks to be played (see `Game::draw_multiverse`): its left edge
/// (a part of the screen's width) and the middle of its lane, fully on screen and clear of the
/// score and HP bar.
fn parked(f: usize) -> (f32, f32) {
    (HOMES[f].0.min(0.97 - FIELD_W), HOMES[f].1.clamp(140.0, 350.0))
}

/// Multiverse: where the focus moves to another playfield. Every 2 bars, every bar in kiai
/// time, from the first note to the last, to one that parks well above or below the one that
/// has it, so the one in focus never hides the one that's called.
fn focus_switches(map: &Beatmap) -> Vec<FocusSwitch> {
    let first = map.notes.first().map_or(0.0, |n| n.time);
    let last = map.notes.iter().map(|n| n.end_time).fold(f64::NEG_INFINITY, f64::max);
    let mut focus = FIRST_FOCUS;
    let mut out = Vec::new();
    for (i, b) in map.barlines.iter().enumerate().skip(1) {
        let cp = beatmap::control_point_at(&map.control_points, b.time);
        if i % if cp.kiai { 1 } else { 2 } != 0 || b.time <= first + 1000.0 || b.time >= last {
            continue;
        }
        let away: Vec<usize> = (0..FIELD_COUNT).filter(|&f| (parked(f).1 - parked(focus).1).abs() >= 80.0).collect();
        focus = away[(hash01(b.time + 0.83) * away.len() as f64) as usize % away.len()];
        out.push(FocusSwitch { at: b.time, field: focus, beat: cp.beat_length });
    }
    out
}

/// The speed most notes have: Constant Speed's, and what Anti-SV / Hyper SV bend around.
fn main_velocity(map: &Beatmap) -> f64 {
    // Counted to 4 decimals; the first exact velocity of the most common one is the answer.
    let mut counts: std::collections::HashMap<i64, (usize, f64)> = std::collections::HashMap::new();
    for n in &map.notes {
        counts.entry((n.velocity * 10_000.0).round() as i64).or_insert((0, n.velocity)).0 += 1;
    }
    counts.into_iter().max_by_key(|&(v, (c, _))| (c, v)).map(|(_, (_, v))| v).unwrap_or(1.0).max(0.01)
}

/// The scroll speed mods that only depend on the map change the notes' (and barlines') own
/// velocities, so drawing needs nothing more for them.
fn bake_speeds(map: &mut Beatmap, mods: Mods) {
    let main = main_velocity(map);
    let first = map.notes.first().map(|n| n.time).unwrap_or(0.0);
    let end = map.notes.iter().map(|n| n.end_time).fold(0.0, f64::max);
    let sections = roulette_sections(map);
    let bars = map.barlines.clone();
    let cps = map.control_points.clone();
    // By map time, for notes and barlines alike.
    let at = |v: f64, t: f64| -> f64 {
        let mut v = if mods.has(Mod::ConstantSpeed) { main } else { v };
        if mods.has(Mod::AntiSv) {
            v = (main * main / v).clamp(main / 10.0, main * 10.0);
        }
        if mods.has(Mod::HyperSv) {
            v = (v * v / main).clamp(main / 10.0, main * 10.0);
        }
        let cp = beatmap::control_point_at(&cps, t);
        if mods.has(Mod::Crescendo) {
            v *= 0.8 + 0.8 * ((t - first) / (end - first).max(1.0)).clamp(0.0, 1.0);
        }
        if mods.has(Mod::Drop) && cp.kiai {
            v *= 1.4;
        }
        if mods.has(Mod::Gearbox) {
            if let Some((_, start, _)) = bar_at(&bars, t, 1.0) {
                v *= 1.0 + 0.15 * ((t - start) / cp.beat_length + 0.01).floor().clamp(0.0, 7.0);
            }
        }
        if mods.has(Mod::Roulette) {
            let i = sections.partition_point(|s| s.0 <= t + 1.0);
            v *= i.checked_sub(1).map_or(1.0, |i| sections[i].1);
        }
        if mods.has(Mod::Swing) {
            v *= 1.0 + 0.15 * (std::f64::consts::TAU * cp.beat_phase(t)).cos();
        }
        v
    };
    for b in &mut map.barlines {
        b.velocity = at(b.velocity, b.time);
    }
    for n in &mut map.notes {
        n.velocity = at(n.velocity, n.time);
    }
    // Per note: dons and kats only.
    let hits: Vec<usize> = (0..map.notes.len()).filter(|&i| matches!(map.notes[i].kind, Kind::Don | Kind::Kat)).collect();
    // Even: the closer a note is to its neighbours, the faster, so they end up evenly spaced.
    let gaps: Vec<f64> = (0..hits.len())
        .map(|k| {
            let prev = k.checked_sub(1).map(|j| map.notes[hits[k]].time - map.notes[hits[j]].time);
            let next = hits.get(k + 1).map(|&j| map.notes[j].time - map.notes[hits[k]].time);
            match (prev, next) {
                (Some(a), Some(b)) => a.min(b),
                (a, b) => a.or(b).unwrap_or(250.0),
            }
            .clamp(15.0, 4000.0)
        })
        .collect();
    let typical = {
        let mut g = gaps.clone();
        g.sort_by(f64::total_cmp);
        g.get(g.len() / 2).copied().unwrap_or(250.0)
    };
    let flow = flows(map);
    for (k, &i) in hits.iter().enumerate() {
        let n = &mut map.notes[i];
        if mods.has(Mod::SnapSpeed) {
            n.velocity *= match flow[i] {
                3 => 0.8,
                6 | 12 => 1.25,
                8 | 16 => 1.1,
                _ => 1.0,
            };
        }
        if mods.has(Mod::Split) {
            n.velocity *= if n.kind == Kind::Don { 1.2 } else { 0.85 };
        }
        if mods.has(Mod::Leapfrog) {
            n.velocity *= if k % 2 == 0 { 1.25 } else { 0.8 };
        }
        if mods.has(Mod::Even) {
            n.velocity *= (typical / gaps[k]).clamp(0.4, 2.5);
        }
    }
}

/// Decoy's fake notes: in about half the gaps between notes, one halfway, of a random colour.
fn decoys(map: &Beatmap) -> Vec<beatmap::Note> {
    let hits: Vec<&beatmap::Note> = map.notes.iter().filter(|n| matches!(n.kind, Kind::Don | Kind::Kat)).collect();
    let mut out = Vec::new();
    // Inside streams (gaps under 120 ms) only now and then, and never in two gaps running, so
    // they stay readable.
    let mut last_in_stream = false;
    for p in hits.windows(2) {
        let gap = p[1].time - p[0].time;
        let stream = gap < 120.0;
        let chance = if !stream { 0.5 } else if gap >= 40.0 && !last_in_stream { 0.12 } else { 0.0 };
        if hash01(p[0].time + 0.25) >= chance {
            last_in_stream = false;
            continue;
        }
        last_in_stream = stream;
        let mut n = p[0].clone();
        n.time = (p[0].time + p[1].time) / 2.0;
        n.end_time = n.time;
        n.kind = if hash01(n.time + 0.75) < 0.5 { Kind::Don } else { Kind::Kat };
        n.big = !stream && hash01(n.time + 0.6) < 0.12;
        n.velocity *= 0.9 + 0.2 * hash01(n.time + 0.1);
        out.push(n);
    }
    out
}

/// The rhythm of a note at map time `t`: the smallest of 1, 2, 3, 4, 6, 8, 12 and 16 whose 1/n
/// beat grid (from its red line) it sits on, within 3 ms; 0 when on none.
fn snap_of(cps: &[beatmap::ControlPoint], t: f64) -> u8 {
    let cp = beatmap::control_point_at(cps, t);
    let beats = (t - cp.beat_origin) / cp.beat_length;
    [1u8, 2, 3, 4, 6, 8, 12, 16]
        .into_iter()
        .find(|&d| {
            let x = beats * d as f64;
            (x - x.round()).abs() * cp.beat_length / d as f64 <= 3.0
        })
        .unwrap_or(0)
}

/// The rhythm each note is part of: how far it is from its nearest neighbour, as the 1/n of a
/// beat that gap is closest to (1, 2, 3, 4, 6, 8, 12 or 16). Every note of a 1/4 stream gets 4,
/// of a triplet run 3, a lone note 1 or 2. 0 for rolls and swells.
fn flows(map: &Beatmap) -> Vec<u8> {
    let hits: Vec<usize> = (0..map.notes.len()).filter(|&i| matches!(map.notes[i].kind, Kind::Don | Kind::Kat)).collect();
    let mut out = vec![0u8; map.notes.len()];
    for (k, &i) in hits.iter().enumerate() {
        let t = map.notes[i].time;
        let prev = k.checked_sub(1).map(|j| t - map.notes[hits[j]].time);
        let next = hits.get(k + 1).map(|&j| map.notes[j].time - t);
        let Some(gap) = [prev, next].into_iter().flatten().reduce(f64::min) else { continue };
        let beats = gap / beatmap::control_point_at(&map.control_points, t).beat_length;
        out[i] = [1u8, 2, 3, 4, 6, 8, 12, 16].into_iter().min_by(|&a, &b| (beats * a as f64 - 1.0).abs().total_cmp(&(beats * b as f64 - 1.0).abs())).unwrap_or(1);
    }
    out
}

/// Liar's far-off times: each triplet note (1/3, 1/6, 1/12) moved to the nearest 1/4 (1/8,
/// 1/16) of the beat, unless a real note is already there; other notes keep their own.
fn lies(map: &Beatmap) -> Vec<f64> {
    let hit_at = |t: f64| {
        let i = map.notes.partition_point(|n| n.time < t - 5.0);
        map.notes[i..].iter().take_while(|n| n.time < t + 5.0).any(|n| matches!(n.kind, Kind::Don | Kind::Kat))
    };
    map.notes
        .iter()
        .map(|n| {
            let d = snap_of(&map.control_points, n.time);
            if !matches!(n.kind, Kind::Don | Kind::Kat) || d == 0 || d % 3 != 0 {
                return n.time;
            }
            let cp = beatmap::control_point_at(&map.control_points, n.time);
            let grid = (d / 3 * 4) as f64;
            let beats = (n.time - cp.beat_origin) / cp.beat_length;
            let lie = cp.beat_origin + (beats * grid).round() / grid * cp.beat_length;
            if hit_at(lie) { n.time } else { lie }
        })
        .collect()
}

/// A repeatable pseudo-random 0-1 for a map time (SV Chaos, Random).
fn hash01(t: f64) -> f64 {
    let mut x = (t * 16.0).round() as i64 as u64 ^ 0x9e37_79b9_7f4a_7c15;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// Shapeshift: the shape of each section, by `setup`.
fn shape_order(map: &Beatmap, mods: Mods, setup: shapeshift::Config) -> Vec<u8> {
    if !mods.has(Mod::Shapeshift) {
        return Vec::new();
    }
    // Each map its own shuffle, the same every time it's played.
    let m = &map.meta;
    let seed = crate::net::map_hash(format!("{}\n{}\n{}\n{}", m.artist, m.title, m.creator, m.version).as_bytes());
    setup.sequence(map.barlines.len() / setup.bars() + 1, seed)
}

/// The speed a play of `map` with `mods` at `rate` runs at (what `Game::with_mods` will use), to
/// prepare its audio with.
pub fn speed_for(map: &Beatmap, mods: Mods, rate: f64) -> Speed {
    if mods.wind().is_none() {
        return Speed::constant(rate);
    }
    let mut map = map.clone();
    apply_to_map(&mut map, mods, 0);
    let first = map.notes.first().map(|n| n.time).unwrap_or(0.0);
    let end = map.notes.iter().map(|n| n.end_time).fold(0.0, f64::max);
    Speed::for_play(mods, rate.clamp(0.25, 4.0), first, end)
}

/// Changes the notes themselves for the mods that do (before anything is judged).
fn apply_to_map(map: &mut Beatmap, mods: Mods, seed: u64) {
    let hit = |n: &beatmap::Note| matches!(n.kind, Kind::Don | Kind::Kat);
    if mods.has(Mod::Half) {
        let mut k = 0;
        map.notes.retain(|n| {
            if !hit(n) {
                return true;
            }
            k += 1;
            k % 2 == 1
        });
    }
    for n in map.notes.iter_mut().filter(|n| hit(n)) {
        if mods.has(Mod::Swap) {
            n.kind = if n.kind == Kind::Don { Kind::Kat } else { Kind::Don };
        }
        if mods.has(Mod::Random) {
            // Seed 0: the colouring every play of the map had before seeds (old replays).
            let salt = if seed == 0 { 0.0 } else { ((seed % 1_000_003) as f64 + 1.0) * 1.618 };
            n.kind = if hash01(n.time + 0.5 + salt) < 0.5 { Kind::Don } else { Kind::Kat };
        }
        if mods.has(Mod::NoFinishers) {
            n.big = false;
        }
        if mods.has(Mod::AllBig) {
            n.big = true;
        }
    }
    bake_speeds(map, mods);
}

impl Game {
    pub fn new(map: Beatmap, track: Arc<Track>, audio: Option<&Engine>) -> Game {
        Game::with_mods(map, track, audio, Mods::NONE)
    }

    /// A play with `mods` (the track is already at their speed, see `audio::change_speed`).
    pub fn with_mods(map: Beatmap, track: Arc<Track>, audio: Option<&Engine>, mods: Mods) -> Game {
        Game::with_seed(map, track, audio, mods, 0)
    }

    /// The same, with Random colouring the notes by `seed` (see `Game::seed`).
    pub fn with_seed(mut map: Beatmap, track: Arc<Track>, audio: Option<&Engine>, mods: Mods, seed: u64) -> Game {
        apply_to_map(&mut map, mods, seed);
        let first_note = map.notes.first().map(|n| n.time).unwrap_or(0.0);
        let end_time = map.notes.iter().map(|n| n.end_time).fold(0.0, f64::max);
        let start = (first_note - 2000.0).min(0.0).min(-map.meta.audio_lead_in);
        let rate = track.rate.clamp(0.25, 4.0);
        let speed = Speed::for_play(mods, rate, first_note, end_time);
        let track_ms = audio.map(|a| track.frames() as f64 / a.rate as f64 * 1000.0).unwrap_or(f64::INFINITY);
        // The pre-roll is in map time; the track plays `rate` map-ms per ms.
        let track_start = speed.track(start);
        if let Some(a) = audio {
            a.play(track.clone(), track_start);
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
        // Barlines too: a gimmick's bars can scroll slower than any note, and the lookahead is
        // sized from this.
        // Floored only against 0: SV 0.02 at 120 BPM is already under 0.01, and a fake slider
        // in it is on screen ~40 s ahead, and nothing caps the lookahead (the whole map can be).
        let min_velocity = map.notes.iter().map(|n| n.velocity).chain(map.barlines.iter().map(|b| b.velocity)).fold(f64::INFINITY, f64::min).max(1e-6);
        let main_beat = {
            let cps = &map.control_points;
            let mut spans: Vec<(f64, f64)> = Vec::new();
            for (i, cp) in cps.iter().enumerate() {
                let ms = (cps.get(i + 1).map_or(end_time, |n| n.time) - cp.time).max(0.0);
                match spans.iter_mut().find(|s| (s.0 - cp.beat_length).abs() < 0.01) {
                    Some(s) => s.1 += ms,
                    None => spans.push((cp.beat_length, ms)),
                }
            }
            spans.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)).map_or(500.0, |s| s.0)
        };
        // osu!lazer's TaikoHealthProcessor.
        let hits = map.notes.iter().filter(|n| matches!(n.kind, Kind::Don | Kind::Kat)).count().max(1) as f64;
        let hp_drain = mods.hp(map.meta.hp);
        let hp_hit = 1.0 / (3.0 * hits * beatmap::difficulty_range(hp_drain, 0.5, 0.75, 0.98));
        let hp_miss = beatmap::difficulty_range(hp_drain, 0.0018, 0.0075, 0.0120);
        let mut kiai_starts = Vec::new();
        let mut kiai_bands: Vec<(f64, f64)> = Vec::new();
        let mut was_kiai = false;
        for cp in &map.control_points {
            if cp.kiai && !was_kiai {
                kiai_starts.push((cp.time, cp.beat_length));
                kiai_bands.push((cp.time, f64::INFINITY));
            } else if !cp.kiai && was_kiai && let Some(band) = kiai_bands.last_mut() {
                band.1 = cp.time;
            }
            was_kiai = cp.kiai;
        }
        let sections = if mods.has(Mod::Roulette) {
            roulette_sections(&map).into_iter().zip(map.barlines.iter().step_by(4)).map(|((t, k), b)| (t, k, b.velocity)).collect()
        } else {
            Vec::new()
        };
        let decoys = if mods.has(Mod::Decoy) { decoys(&map) } else { Vec::new() };
        let flows = flows(&map);
        let decoy_flows = decoys.iter().map(|d| map.notes.partition_point(|n| n.time < d.time).checked_sub(1).map_or(1, |i| flows[i])).collect();
        let lies = if mods.has(Mod::Liar) { lies(&map) } else { Vec::new() };
        // Overtime: once, plus once more per 75 s of map, anywhere from 8 s in to 4 s before the end.
        let scares = if mods.has(Mod::Overtime) {
            let span = (end_time - first_note - 12_000.0).max(0.0);
            let n = (1 + (span / 75_000.0) as usize).min(4);
            let mut at: Vec<f64> = (0..n).map(|k| first_note + 8000.0 + span * hash01(first_note + k as f64 * 13.7 + 0.9)).collect();
            at.sort_by(f64::total_cmp);
            at
        } else {
            Vec::new()
        };
        let min_velocity = decoys.iter().map(|n| n.velocity).fold(min_velocity, f64::min);
        let focus_switches = if mods.has(Mod::Multiverse) { focus_switches(&map) } else { Vec::new() };
        let shapeshift = shapeshift::get();
        let shape_order = shape_order(&map, mods, shapeshift);
        Game {
            seed,
            windows: Windows::new(mods.od(map.meta.od)),
            map,
            clock: GameClock::new(track_start, audio.is_some()),
            state,
            cursor: 0,
            stats: Stats::default(),
            time: start,
            start,
            end_time,
            track_ms,
            track,
            first_note,
            presses: [f64::NEG_INFINITY; 4],
            last_judge: None,
            recent_errors: VecDeque::new(),
            strong: None,
            min_velocity,
            main_beat,
            visible: Vec::new(),
            last_audio_ms: None,
            raw_time: track_start,
            auto_key: 0,
            paused: None,
            paused_at: Instant::now(),
            resuming: None,
            played_ms: 0.0,
            played_at: None,
            scroll_speed: 1.0,
            hide_notes: false,
            show_hp: true,
            mute_hits: false,
            show_offset_hint: false,
            kiai_effects: true,
            hidden: crate::hud::Parts::NONE,
            visual_offset: 0.0,
            mash: Mash::Nearest,
            scoring: Scoring::Standard,
            score_basis: None,
            combo_points: 0.0,
            extra_taps: 0,
            record: false,
            log_inputs: false,
            watched: false,
            inputs: Vec::new(),
            replay: None,
            replay_of: None,
            live_pp: None,
            live_pp_value: None,
            banner: None,
            sfx_log: None,
            combo_break: None,
            ur_scaled_hint: true,
            offset: 0.0,
            rate,
            speed,
            mods,
            score_mult: mods.multiplier(),
            hp: 0.0,
            hp_hit,
            hp_miss,
            hp_exempt: false,
            failed: None,
            last_hand: None,
            quake_at: f64::NEG_INFINITY,
            cracks: Vec::new(),
            crumbs: Vec::new(),
            misses: VecDeque::new(),
            kiai_starts,
            kiai_bands,
            sections,
            decoys,
            flows,
            decoy_flows,
            lies,
            scares,
            next_scare: 0,
            focus_switches,
            shapeshift,
            shape_order,
            universes: None,
            zone: None,
        }
    }

    /// Where the playfield in play was drawn last frame (x, y, w, h on the canvas it was drawn
    /// on), for views that show only that (spectating everyone at once).
    pub fn play_zone(&self) -> Option<[f32; 4]> {
        self.zone
    }

    /// Plays with someone else's mod settings (a lobby host's): Multiverse's universes and
    /// Shapeshift's setup, each when given. The player's own config is left alone.
    pub fn use_mod_settings(&mut self, universes: Option<[Universe; multiverse::COUNT]>, shapeshift: Option<shapeshift::Config>) {
        if universes.is_some() {
            self.universes = universes;
        }
        if let Some(s) = shapeshift {
            self.shapeshift = s;
            self.shape_order = shape_order(&self.map, self.mods, s);
        }
    }

    /// Song (track) ms at map ms `m`, before the offset.
    pub fn track_at(&self, m: f64) -> f64 {
        self.speed.track(m)
    }

    /// How map time relates to song time in this play.
    pub fn speed(&self) -> Speed {
        self.speed
    }

    /// The play failed: Sudden Death / Perfect, or (at the end) HP under half without No Fail.
    pub fn failed(&self) -> bool {
        self.failed.is_some() || (!self.mods.has(Mod::NoFail) && !self.mods.has(Mod::Autoplay) && !self.hp_exempt && self.hp < 0.5)
    }

    /// Failed by Sudden Death / Perfect (the play stops early).
    pub fn failed_early(&self) -> bool {
        self.failed.is_some()
    }

    fn add_score(&mut self, base: f64) {
        if self.scoring == Scoring::Classic {
            self.stats.score += (base * self.score_mult) as u64;
        }
    }

    /// Standard scoring after a hit or miss: the score of the play so far, mods included. `combo`
    /// is the combo the note was judged at.
    fn rescore(&mut self, judge: Judge, combo: u32) {
        let (max_points, notes) = *self.score_basis.get_or_insert_with(|| {
            // Notes skipped by `skip_to` aren't part of the play.
            let n = self.map.notes.iter().zip(&self.state).filter(|(n, st)| matches!(n.kind, Kind::Don | Kind::Kat) && st.judged_at != f64::NEG_INFINITY).count() as u32;
            (300.0 * (0..n).map(combo_weight).sum::<f64>(), n)
        });
        self.combo_points += match judge {
            Judge::Great => 300.0 * combo_weight(combo),
            Judge::Ok => 150.0 * combo_weight(combo),
            Judge::Miss => 0.0,
        };
        let s = &self.stats;
        let judged = (s.great + s.ok + s.miss).min(notes);
        if max_points > 0.0 && notes > 0 {
            let combo_part = 700_000.0 * (self.combo_points / max_points).min(1.0);
            let accuracy_part = 300_000.0 * s.accuracy().powi(10) * judged as f64 / notes as f64;
            self.stats.score = ((combo_part + accuracy_part) * self.score_mult).round() as u64;
        }
    }

    fn fail(&mut self, t: f64) {
        if self.failed.is_none() {
            self.failed = Some((t, Instant::now()));
        }
    }

    /// Advances the clock. Returns true once the map is over.
    pub fn update(&mut self, now: Instant, audio: Option<&Engine>, offset: f64) -> bool {
        self.offset = offset;
        let playing = self.paused.is_none() && self.resuming.is_none() && self.failed.is_none();
        if let Some(at) = self.played_at.filter(|_| playing) {
            // A long stall (the window dragged, the machine asleep) counts as a second at most.
            self.played_ms += now.saturating_duration_since(at).as_secs_f64().min(1.0) * 1000.0;
        }
        self.played_at = playing.then_some(now);
        if let Some(live) = &mut self.live_pp {
            let st = &self.stats;
            self.live_pp_value = live.update(st.great, st.ok, st.miss, st.max_combo);
        }
        if self.resuming.is_some_and(|(_, end)| now >= end) {
            self.resume(audio);
        }
        if self.paused.is_some() {
            self.clock.hold(now);
            self.time = self.speed.map(self.raw_time - offset);
            return false;
        }
        let audio_ms = audio.and_then(|a| a.position_ms(now));
        let raw = self.clock.update(now, audio_ms);
        self.last_audio_ms = audio_ms;
        self.raw_time = raw;
        self.time = self.speed.map(raw - offset);
        // Failed: the lane keeps scrolling under the FAILED sign, but nothing more is judged.
        if let Some((_, at)) = self.failed {
            return now.duration_since(at).as_secs_f64() > 1.6;
        }
        self.feed_replay(audio);
        self.expire(self.time);
        self.play_combo_break(audio);
        self.play_scares(audio);
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
                Kind::Don | Kind::Kat if st.judged.is_none() && !n.unhittable => {
                    let kat = kind == Kind::Kat;
                    self.auto_key ^= 1;
                    let key = match (kat, self.auto_key) { (true, 0) => 0, (true, _) => 3, (false, 0) => 1, (false, _) => 2 };
                    self.auto_hit(key, time, audio);
                    if big {
                        let other = 3 - key;
                        self.auto_hit(other, time, audio);
                    }
                }
                // A roll mapped across notes is played as the notes alone (owner, 2026-10-28):
                // a tick press would land in a note's window and hit it early, or miss a kat.
                Kind::Roll if !st.finished && !self.roll_over_notes(i) => {
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

    /// Whether roll `i` has a hittable don/kat anywhere in its span.
    fn roll_over_notes(&self, i: usize) -> bool {
        let notes = &self.map.notes;
        let n = &notes[i];
        notes[notes.partition_point(|m| m.time < n.time)..]
            .iter()
            .take_while(|m| m.time <= n.end_time)
            .any(|m| matches!(m.kind, Kind::Don | Kind::Kat) && !m.unhittable)
    }

    fn auto_hit(&mut self, key: usize, t: f64, audio: Option<&Engine>) {
        if self.record || self.log_inputs {
            self.inputs.push((t, key as u8));
        }
        self.presses[key] = t;
        let sfx = self.judge_press(key, key == 0 || key == 3, t);
        if let Some(a) = audio {
            a.sfx_at(sfx, self.volume_at(t));
        }
    }

    /// Hitsound volume at map time `t`: the timing point's, like osu!.
    fn volume_at(&self, t: f64) -> f32 {
        beatmap::control_point_at(&self.map.control_points, t).volume
    }

    /// Plays (and logs) the combo break sound when a miss just broke a long combo.
    fn play_combo_break(&mut self, audio: Option<&Engine>) {
        let Some(t) = self.combo_break.take() else { return };
        if let Some(log) = &mut self.sfx_log {
            log.push((t, Sfx::ComboBreak, COMBO_BREAK_GAIN));
        }
        if let Some(a) = audio.filter(|_| !self.mute_hits) {
            a.sfx_at(Sfx::ComboBreak, COMBO_BREAK_GAIN);
        }
    }

    /// Overtime: screams for each scare whose time has come (and logs it).
    fn play_scares(&mut self, audio: Option<&Engine>) {
        while let Some(&at) = self.scares.get(self.next_scare).filter(|&&at| at <= self.time) {
            self.next_scare += 1;
            // One skipped over (a seek) stays quiet.
            if self.time - at > SCARE_MS {
                continue;
            }
            if let Some(log) = &mut self.sfx_log {
                log.push((at, Sfx::Scream, 1.0));
            }
            if let Some(a) = audio.filter(|_| !self.mute_hits) {
                a.sfx_at(Sfx::Scream, 1.0);
            }
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
        self.score_basis = None;
        self.next_scare = self.scares.partition_point(|&at| at < t);
    }

    /// Starts play at map time `t` instead of the beginning: the music starts two seconds
    /// before it, and everything that ends earlier counts as not played (no score, no misses).
    pub fn start_from(&mut self, t: f64, audio: Option<&Engine>, offset: f64) {
        let start = self.speed.map(self.speed.track(t) - 2000.0).max(self.time);
        let target = self.speed.track(start) + offset;
        self.hp_exempt = true;
        if let Some(a) = audio {
            a.seek(target);
        }
        self.clock = GameClock::new(target, audio.is_some());
        self.raw_time = target;
        self.time = start;
        self.skip_to(t);
    }

    /// Replays: jumps to map time `t`, backwards or forwards. Everything before `t` is judged
    /// again, silently, from the recorded presses, and the music carries on from there.
    pub fn seek(&mut self, t: f64, audio: Option<&Engine>, offset: f64) {
        let t = t.clamp(self.start, self.end_time);
        if t < self.time {
            self.rewind();
        }
        self.offset = offset;
        self.set_time(t);
        let target = self.speed.track(t) + offset;
        if let Some(a) = audio {
            a.seek(target);
        }
        self.clock = GameClock::new(target, audio.is_some());
        self.raw_time = target;
    }

    /// Back to the state before the first note: nothing judged, no score, the replay from its
    /// first press.
    fn rewind(&mut self) {
        for st in &mut self.state {
            *st = NoteState { ticks: vec![false; st.ticks.len()], ..Default::default() };
        }
        self.cursor = 0;
        self.stats = Stats::default();
        self.score_basis = None;
        self.combo_points = 0.0;
        self.presses = [f64::NEG_INFINITY; 4];
        self.last_judge = None;
        self.recent_errors.clear();
        self.strong = None;
        self.combo_break = None;
        self.hp = 0.0;
        self.failed = None;
        self.misses.clear();
        self.cracks.clear();
        self.crumbs.clear();
        self.next_scare = 0;
        if let Some((_, next)) = &mut self.replay {
            *next = 0;
        }
        self.time = self.start;
    }

    /// Map time of the start and end of play, for a timeline.
    pub fn span(&self) -> (f64, f64) {
        (self.start, self.end_time)
    }

    pub fn can_skip(&self) -> bool {
        self.first_note - self.time > 3000.0
    }

    /// Skips the intro; true when it did.
    pub fn skip(&mut self, audio: Option<&Engine>, offset: f64) -> bool {
        let skipped = self.can_skip();
        if skipped {
            let target = self.speed.track(self.skip_target()) + offset;
            if let Some(a) = audio {
                a.seek(target);
            }
            self.clock = GameClock::new(target, audio.is_some());
            }
        skipped
    }

    /// Map time skipping the intro jumps to (just before the first note).
    pub fn skip_target(&self) -> f64 {
        self.first_note - 2000.0
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
        // Great or Nothing: an OK is as good as a miss.
        let judge = if judge == Judge::Ok && self.mods.has(Mod::GreatOrNothing) { Judge::Miss } else { judge };
        self.extra_taps = 0;
        let st = &mut self.state[i];
        st.judged = Some(judge);
        st.judged_at = t;
        // After failing (Sudden Death / Perfect) nothing counts any more.
        if self.failed.is_some() {
            return;
        }
        let combo_mult = 1.0 + self.stats.combo.min(100) as f64 / 100.0;
        let combo_before = self.stats.combo;
        match judge {
            Judge::Great => self.add_score(300.0 * combo_mult),
            Judge::Ok => self.add_score(150.0 * combo_mult),
            Judge::Miss => {}
        }
        if self.failed.is_none() {
            self.hp = (self.hp + if judge == Judge::Miss { -self.hp_miss } else { self.hp_hit * if judge == Judge::Great { 3.0 } else { 1.1 } }).clamp(0.0, 1.0);
        }
        if (judge == Judge::Miss && self.mods.has(Mod::SuddenDeath)) || (judge != Judge::Great && self.mods.has(Mod::Perfect)) {
            self.fail(t);
        }
        if judge != Judge::Miss && self.map.notes[i].big {
            self.quake_at = t;
        }
        if self.mods.has(Mod::Crumble) {
            self.crumble(i, judge, t);
        }
        let s = &mut self.stats;
        match judge {
            Judge::Great => {
                s.great += 1;
                s.combo += 1;
            }
            Judge::Ok => {
                s.ok += 1;
                s.combo += 1;
            }
            Judge::Miss => {
                s.miss += 1;
                self.misses.push_back(t);
                if self.misses.len() > 32 {
                    self.misses.pop_front();
                }
                if self.mods.has(Mod::Cracked) {
                    self.cracks.push((t, s.great + s.ok));
                    if self.cracks.len() > 8 {
                        self.cracks.remove(0);
                    }
                }
                // Like osu!, losing a combo of 20 or more gets the combo break sound.
                if s.combo >= 20 {
                    self.combo_break = Some(t);
                }
                s.combo = 0;
            }
        }
        s.max_combo = s.max_combo.max(s.combo);
        if let Some(e) = error.filter(|_| judge != Judge::Miss) {
            // Stats are kept in real milliseconds, so UR and offset advice mean the same at any rate.
            s.errors.push(e / self.speed.rate_at(t));
            s.error_offsets.push(self.offset);
            self.recent_errors.push_back((e, t, judge));
            if self.recent_errors.len() > 64 {
                self.recent_errors.pop_front();
            }
        }
        self.last_judge = Some((judge, t));
        if self.scoring == Scoring::Standard {
            self.rescore(judge, combo_before);
        }
    }

    /// Handles a drum key (0 = left rim, 1 = left centre, 2 = right centre, 3 = right rim).
    pub fn press(&mut self, key: usize, at: Instant, audio: Option<&Engine>, offset: f64) {
        let t = self.speed.map(self.clock.at(at) - offset);
        self.offset = offset;
        self.press_time(key, t, audio);
    }

    /// A live press at map time `t`.
    fn press_time(&mut self, key: usize, t: f64, audio: Option<&Engine>) {
        if self.failed.is_some() {
            return;
        }
        // Alternate: the same hand twice in a row doesn't count (after a 2 s pause either may
        // start). Drumrolls and swells take either hand, as fast as you like.
        let alternate = self.mods.has(Mod::Alternate);
        let left = key < 2;
        let free_run = alternate && (self.find_active(t, Kind::Roll).is_some() || self.find_active(t, Kind::Swell).is_some());
        if alternate && !free_run && self.last_hand.is_some_and(|(hand, at)| hand == left && t - at < 2000.0) {
            return;
        }
        if self.record || self.log_inputs {
            self.inputs.push((t, key as u8));
        }
        let hits = self.stats.great + self.stats.ok;
        let sfx = self.press_at(key, t, audio);
        if alternate {
            let hit = self.stats.great + self.stats.ok > hits;
            let finisher = hit && matches!(sfx, Sfx::BigDon | Sfx::BigKat);
            // A finisher counts as two notes, so either hand may hit next; after a drumroll or
            // a swell, either hand may until one hits a note.
            self.last_hand = if free_run || finisher || (self.last_hand.is_none() && !hit) { None } else { Some((left, t)) };
        }
    }

    /// A press judged at map time `t` (live, or from a replay); the sound it makes.
    fn press_at(&mut self, key: usize, t: f64, audio: Option<&Engine>) -> Sfx {
        let kat = if self.mods.has(Mod::Relax) { self.relax_colour(t, key == 0 || key == 3) } else { key == 0 || key == 3 };
        self.presses[key] = t;
        let sfx = self.judge_press(key, kat, t);
        let volume = self.volume_at(t);
        if let Some(log) = &mut self.sfx_log {
            log.push((t, sfx, volume));
        }
        if let Some(a) = audio.filter(|_| !self.mute_hits) {
            a.sfx_at(sfx, volume);
        }
        self.play_combo_break(audio);
        sfx
    }

    /// Replays without audio: jumps to map time `t` (forwards only), judging the recorded presses
    /// on the way, e.g. to render a replay frame by frame.
    pub fn set_time(&mut self, t: f64) {
        self.time = t;
        self.raw_time = self.speed.track(t) + self.offset;
        if self.failed.is_some() {
            return;
        }
        self.feed_replay(None);
        self.expire(t);
        self.play_combo_break(None);
        self.play_scares(None);
    }

    /// The last moment anything happens in the map (map ms).
    pub fn end_time(&self) -> f64 {
        self.end_time
    }

    /// Replays: the recorded presses up to now, before anything expires, so each is judged
    /// exactly as it was.
    fn feed_replay(&mut self, audio: Option<&Engine>) {
        let Some((inputs, mut next)) = self.replay.take() else { return };
        while let Some(&(t, key)) = inputs.get(next) {
            if t > self.time {
                break;
            }
            self.press_at((key as usize).min(3), t, audio);
            next += 1;
        }
        self.replay = Some((inputs, next));
    }

    /// Relax: the colour a press counts as, whatever key it was: the big note's second hit, the
    /// other colour in a swell, else the next note's.
    fn relax_colour(&mut self, t: f64, pressed: bool) -> bool {
        self.expire(t);
        if let Some(s) = self.strong.as_ref().filter(|s| (t - s.at).abs() <= STRONG_WINDOW) {
            return s.kat;
        }
        if let Some(i) = self.find_active(t, Kind::Swell) {
            if let Some(j) = self.note_under_swell(t, None) {
                return self.map.notes[j].kind == Kind::Kat;
            }
            return self.state[i].last_was_kat != Some(true);
        }
        match self.next_hittable(t) {
            Some(i) => self.map.notes[i].kind == Kind::Kat,
            None => pressed,
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
                self.add_score(300.0);
                self.quake_at = t;
                return big;
            }
        }

        // An active swell swallows every hit and must be alternated centre/rim, except one that
        // hits a note under it.
        if let Some(i) = self.find_active(t, Kind::Swell).filter(|_| self.note_under_swell(t, Some(kat)).is_none()) {
            let required = self.map.notes[i].required_hits;
            let st = &mut self.state[i];
            if st.last_was_kat != Some(kat) {
                st.last_was_kat = Some(kat);
                st.hits += 1;
                let done = st.hits >= required;
                if done {
                    st.finished = true;
                    st.judged = Some(Judge::Great);
                    st.judged_at = t;
                    self.stats.swells += 1;
                }
                self.add_score(if done { 1100.0 } else { 100.0 });
            }
            return normal;
        }

        if let Some(i) = self.next_hittable(t) {
            let hit = match self.mash {
                Mash::Nearest => self.judge_nearest(i, kat, t),
                legacy => {
                    let n = &self.map.notes[i];
                    let wrong = (n.kind == Kind::Kat) != kat;
                    let judge = if wrong { Judge::Miss } else { self.timing(t - n.time) };
                    (judge != Judge::Miss || legacy == Mash::Strict).then_some((i, judge))
                }
            };
            if let Some((i, judge)) = hit {
                let n = &self.map.notes[i];
                let is_big = n.big;
                self.apply(i, judge, t, Some(t - n.time));
                if judge != Judge::Miss && is_big {
                    self.strong = Some(StrongPending { note: i, kat, key, at: t });
                    return big;
                }
                return normal;
            }
        }

        if let Some(i) = self.find_active(t, Kind::Roll) {
            let n = &self.map.notes[i];
            let k = ((t - n.time) / n.tick_spacing).round().max(0.0) as usize;
            if let Some(tick) = self.state[i].ticks.get_mut(k) {
                if !*tick {
                    *tick = true;
                    self.stats.roll_ticks += 1;
                    let points = if n.big { 200.0 } else { 100.0 };
                    self.add_score(points);
                }
            }
        }
        normal
    }

    /// Great, OK or Miss for a press `err` ms off its note, whatever the colour.
    fn timing(&self, err: f64) -> Judge {
        if err.abs() <= self.windows.great {
            Judge::Great
        } else if err.abs() <= self.windows.ok {
            Judge::Ok
        } else {
            Judge::Miss
        }
    }

    /// The mash penalty, made so one slip in a stream costs one note: the note this press is
    /// judged on and how, or None when it leaves the notes alone. `i` is the next note.
    ///
    /// - A press that hits `i` (its colour, inside the OK window) counts for it.
    /// - A press that would hit the note after it instead, and is nearer that one, means `i`
    ///   was skipped: `i` misses and the press counts for the later note.
    /// - Else a press nearer a note already judged is an extra hit on that one: ignored, once
    ///   between two notes (a slip). Mashing presses more than that, and the rest count.
    /// - Else it was aimed at `i`: the wrong colour inside the OK window, or the right colour
    ///   too early, misses it. A wrong colour further off is a stray and is ignored.
    ///
    /// osu! gives every press to the next note, so after one extra or skipped hit every press
    /// after it is a note off, and alternating colours turn the rest of a stream into misses.
    fn judge_nearest(&mut self, i: usize, kat: bool, t: f64) -> Option<(usize, Judge)> {
        let notes = &self.map.notes;
        let colour = |k: usize| (notes[k].kind == Kind::Kat) == kat;
        let err = |k: usize| t - notes[k].time;
        let ok = self.windows.ok;
        if colour(i) && err(i).abs() <= ok {
            return Some((i, self.timing(err(i))));
        }
        let skipped_to = self.next_unjudged(i, t).filter(|&j| colour(j) && err(j).abs() <= ok && err(j).abs() < err(i).abs());
        let extra = self.prev_judged(i, t).is_some_and(|p| err(p).abs() < err(i).abs());
        let aimed = colour(i) || err(i).abs() <= ok;
        if let Some(j) = skipped_to {
            let judge = self.timing(err(j));
            self.apply(i, Judge::Miss, t, None);
            return Some((j, judge));
        }
        if extra && self.extra_taps == 0 {
            self.extra_taps += 1;
            return None;
        }
        aimed.then_some((i, Judge::Miss))
    }

    /// The first unjudged don/kat after note `i` whose miss window has opened.
    fn next_unjudged(&self, i: usize, t: f64) -> Option<usize> {
        for k in i + 1..self.map.notes.len() {
            let n = &self.map.notes[k];
            if n.time - t > self.windows.miss {
                return None;
            }
            if matches!(n.kind, Kind::Don | Kind::Kat) && self.state[k].judged.is_none() && !n.unhittable {
                return Some(k);
            }
        }
        None
    }

    /// The last judged don/kat before note `i`, if it was recent enough to matter.
    fn prev_judged(&self, i: usize, t: f64) -> Option<usize> {
        for k in (0..i).rev() {
            let n = &self.map.notes[k];
            if t - n.time > 2.0 * self.windows.miss {
                return None;
            }
            if matches!(n.kind, Kind::Don | Kind::Kat) && self.state[k].judged.is_some() {
                return Some(k);
            }
        }
        None
    }

    /// First unjudged don/kat whose miss window has opened.
    fn next_hittable(&self, t: f64) -> Option<usize> {
        for i in self.cursor..self.map.notes.len() {
            let n = &self.map.notes[i];
            if n.time - t > self.windows.miss {
                return None;
            }
            // An unhittable note (before a NaN slider) lets presses through, then expires a miss.
            if matches!(n.kind, Kind::Don | Kind::Kat) && self.state[i].judged.is_none() && !n.unhittable {
                return Some(i);
            }
        }
        None
    }

    /// The next note, if a press at `t` (of colour `kat`, or either) would hit it: what a swell
    /// over it lets through.
    fn note_under_swell(&self, t: f64, kat: Option<bool>) -> Option<usize> {
        self.next_hittable(t).filter(|&i| {
            let n = &self.map.notes[i];
            kat.is_none_or(|k| (n.kind == Kind::Kat) == k) && self.timing(t - n.time) != Judge::Miss
        })
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

    /// Panic / Mercy's speed factor at map time `t`: each recent miss fades in over 250 ms and
    /// out over its span.
    fn miss_factor(&self, t: f64) -> f64 {
        let weight = |span: f64| -> f64 {
            self.misses.iter().map(|&at| t - at).filter(|age| (0.0..span).contains(age)).map(|age| (age / 250.0).min(1.0) * (1.0 - age / span)).sum()
        };
        if self.mods.has(Mod::Panic) {
            (1.0 + 0.15 * weight(4000.0)).min(2.0)
        } else if self.mods.has(Mod::Mercy) {
            1.0 / (1.0 + 0.25 * weight(3000.0)).min(2.0)
        } else {
            1.0
        }
    }

    /// The warp (see `Scroll::warp`) of Drop, Lag Spike and Moonwalk at map time `t`.
    fn extra_warp(&self, t: f64, cp: &beatmap::ControlPoint) -> f64 {
        let mods = self.mods;
        let mut warp = 0.0;
        if mods.has(Mod::Drop) {
            // Two beats before a kiai: held still for one and a half, then a rush to catch up.
            let i = self.kiai_starts.partition_point(|&(k, _)| k <= t);
            if let Some(&(k, b)) = self.kiai_starts.get(i) {
                let s = t - (k - 2.0 * b);
                if s >= 0.0 {
                    warp += if s < 1.5 * b { s } else { 1.5 * b * (1.0 - smooth(0.0, 1.0, (s - 1.5 * b) / (0.5 * b))) };
                }
            }
        }
        if mods.has(Mod::LagSpike) {
            // In 40% of 1.5 s slots, one freeze of 150-370 ms, then a jump to where notes should be.
            let slot = (t / 1500.0).floor();
            if hash01(slot * 1.37 + 0.31) < 0.4 {
                let at = slot * 1500.0 + 200.0 + 900.0 * hash01(slot * 2.11 + 0.17);
                let len = 150.0 + 220.0 * hash01(slot * 3.7 + 0.53);
                if (at..at + len).contains(&t) {
                    warp += t - at;
                }
            }
        }
        if mods.has(Mod::Moonwalk) {
            // The first eighth of a bar the notes back away, the rest of it they make up for it.
            if let Some((_, start, len)) = bar_at(&self.map.barlines, t, cp.beat_length * 4.0) {
                let back = 0.125 * len;
                let s = t - start;
                warp -= if s < back { 2.0 * s } else { 2.0 * back * (1.0 - (s - back) / (len - back)).max(0.0) };
            }
        }
        warp
    }

    /// The lane's shape at map time `t` on a canvas `w` wide: rail (rails mods, Shapeshift
    /// morphing from one to the next), length, and how far the playfield moves down and
    /// shrinks for it.
    fn rail_at(&self, t: f64, w: f32) -> (Option<Rail>, f32, f32, f32) {
        let mods = self.mods;
        if let Some(shape) = RailShape::of(mods) {
            return (Some(Rail::build(|s| shape.bend(s), shape.length(w))), shape.length(w), shape.drop(), shape.zoom(w));
        }
        if !mods.has(Mod::Shapeshift) {
            let len = if mods.has(Mod::Mania) {
                MANIA_LANE
            } else if mods.has(Mod::Crossfire) {
                // From the drum in the middle to either edge.
                w / 2.0
            } else if mods.has(Mod::Spin) {
                SPIN_LANE
            } else {
                w - HIT_X
            };
            return (None, len, 0.0, 1.0);
        }
        // A shape every so many bars (the player's setup), in the map's order; the first bars of
        // each morph into it from the last.
        let bars = &self.map.barlines;
        let (every, morph) = (self.shapeshift.bars(), self.shapeshift.morph_bars());
        let shape = |section: usize| SHAPESHIFT[self.shape_order.get(section).map_or(SHAPESHIFT.len() - 1, |&k| k as usize % SHAPESHIFT.len())];
        let (from, to, u) = match bar_at(bars, t, 2000.0) {
            None => (RailShape::Straight, RailShape::Straight, 1.0),
            Some((i, ..)) => {
                let section = i / every;
                let prev = if section == 0 { RailShape::Straight } else { shape(section - 1) };
                let first = section * every;
                let start = bars[first].time;
                // The morph ends part way into a bar (half a bar), or on a barline.
                let whole = morph.floor() as usize;
                let at = |k: usize| bars.get(first + k).map(|b| b.time);
                let base = at(whole).unwrap_or(start + 2000.0 * whole as f64);
                let bar_len = at(whole + 1).map_or(2000.0, |next| next - base);
                let end = base + (morph - whole as f64) * bar_len;
                (prev, shape(section), ((t - start) / (end - start).max(1.0)).clamp(0.0, 1.0) as f32)
            }
        };
        let ease = |x: f32| smooth(0.0, 1.0, x as f64) as f32;
        // The Loop can't be bent into or out of: half way its lane would swing round across the
        // drum and the HUD. So the other shape straightens out over the first half of the morph and
        // the loop slides in whole from off the right of the screen over the second (the other way
        // round on the way out). The lane's length doesn't jump in between.
        let lp = RailShape::Loop;
        if (from == lp) != (to == lp) {
            let other = if to == lp { from } else { to };
            // 0: the other shape, 1: straight with the loop off screen, 2: the loop in place.
            let p = 2.0 * if to == lp { u } else { 1.0 - u };
            let away = 1.5 * (w - HIT_X);
            if p < 1.0 {
                let k = ease(p);
                let mix = |a: f32, b: f32| a + (b - a) * k;
                let len = mix(other.length(w), lp.length(w) + away);
                return (Some(Rail::build(|s| other.bend(s) * (1.0 - k), len)), len, other.drop() * (1.0 - k), mix(other.zoom(w), 1.0));
            }
            let k = ease(p - 1.0);
            let shift = (1.0 - k) * away;
            let len = lp.length(w) + shift;
            return (Some(Rail::build(|s| lp.bend(s - shift), len)), len, lp.drop() * k, 1.0 + (lp.zoom(w) - 1.0) * k);
        }
        let m = ease(u);
        let mix = |a: f32, b: f32| a + (b - a) * m;
        let len = mix(from.length(w), to.length(w));
        (Some(Rail::build(|s| mix(from.bend(s), to.bend(s)), len)), len, mix(from.drop(), to.drop()), mix(from.zoom(w), to.zoom(w)))
    }

    /// Shatter's sideways shift of each quarter of the lane at map time `t`: apart on even bars
    /// (each piece its own way), back in line on odd ones, sliding over the first quarter-bar.
    fn shatter_at(&self, t: f64) -> [f32; 4] {
        let mut out = [0.0; 4];
        if !self.mods.has(Mod::Shatter) {
            return out;
        }
        let Some((i, start, len)) = bar_at(&self.map.barlines, t, 2000.0) else { return out };
        let target = |bar: usize, k: usize| -> f32 {
            if bar % 2 == 1 {
                return 0.0;
            }
            let h = hash01(bar as f64 * 7.1 + k as f64 * 1.7) as f32 * 2.0 - 1.0;
            h.signum() * (20.0 + 30.0 * h.abs())
        };
        let m = smooth(0.0, 0.25, (t - start) / len) as f32;
        for (k, d) in out.iter_mut().enumerate().skip(1) {
            let prev = if i == 0 { 0.0 } else { target(i - 1, k) };
            *d = prev + (target(i, k) - prev) * m;
        }
        out
    }

    /// Crumble: note `i` hit at `t` knocks a piece out of the lane. The slower its rhythm and the
    /// higher the combo, the bigger the piece, the further out and the longer it stays; it goes
    /// where it overlaps the pieces still out the least. A miss knocks nothing out: the combo is
    /// gone, and the pieces still out snap back (`crumbs_at`).
    fn crumble(&mut self, i: usize, judge: Judge, t: f64) {
        if judge == Judge::Miss {
            return;
        }
        let n = &self.map.notes[i];
        let rhythm = (self.flows[i].max(1) as f32).powf(-0.75) * if n.big { 1.25 } else { 1.0 };
        let size = rhythm * crumble_power(self.stats.combo);
        self.crumbs.retain(|c| c.at <= t && t < c.at + c.life);
        if self.crumbs.len() >= 24 {
            self.crumbs.remove(0);
        }
        // Lengths as parts of a typical lane, to compare with where the others start.
        let len = 20.0 + 110.0 * size;
        let part = len / 900.0;
        let (lo, hi) = CRUMBLE_SPAN;
        let overlap = |a: f32| -> f32 { self.crumbs.iter().map(|c| ((a + part).min(c.start + c.len / 900.0) - a.max(c.start)).max(0.0)).sum() };
        let start = (0..5)
            .map(|k| lo + (hi - lo - part).max(0.0) * hash01(n.time + k as f64 * 3.7 + 0.19) as f32)
            .min_by(|&a, &b| overlap(a).total_cmp(&overlap(b)))
            .unwrap_or(lo);
        let side = if hash01(n.time + 0.71) < 0.5 { -1.0 } else { 1.0 };
        let shift = side * (4.0 + 30.0 * size) * (0.8 + 0.2 * hash01(n.time + 0.53) as f32);
        self.crumbs.push(Crumb { at: t, life: 350.0 + 1850.0 * size as f64, size, start, len, shift });
    }

    /// Crumble's knocked-out pieces at map time `t` on a lane `len` long, like `cracks_at`'s:
    /// each snaps out and holds, then settles back over the rest of its life, or all at once
    /// when a miss breaks the combo.
    fn crumbs_at(&self, t: f64, len: f32) -> impl Iterator<Item = (f32, f32, f32, f32)> + '_ {
        let miss = self.misses.iter().rev().copied().find(|&m| m <= t);
        self.crumbs.iter().filter(move |c| c.at <= t && t < c.at + c.life).map(move |c| {
            let x = (t - c.at) / c.life;
            let mend = miss.filter(|&m| m > c.at).map_or(1.0, |m| 1.0 - smooth(m, m + CRUMBLE_MEND_MS, t));
            let m = (smooth(0.0, 70.0 / c.life, x) * (1.0 - smooth(0.35, 1.0, x)) * mend) as f32;
            (c.start * len, c.len, c.shift * m, 0.9 * m)
        })
    }

    /// Cracked's knocked-out pieces at map time `t` on a lane `len` long: (start, length,
    /// sideways shift, seam opacity), each fading back as the next 50 notes are hit.
    fn cracks_at(&self, t: f64, len: f32) -> Vec<(f32, f32, f32, f32)> {
        let hit = self.stats.great + self.stats.ok;
        self.cracks
            .iter()
            .filter(|&&(at, _)| at <= t)
            .filter_map(|&(at, hits_then)| {
                let mend = 1.0 - hit.saturating_sub(hits_then) as f32 / 50.0;
                if mend <= 0.0 {
                    return None;
                }
                let snap = ((t - at) / 120.0).min(1.0) as f32;
                let side = if hash01(at + 0.37) < 0.5 { -1.0 } else { 1.0 };
                let shift = side * (18.0 + 22.0 * hash01(at + 0.41) as f32) * mend * snap;
                Some((len * (0.3 + 0.55 * hash01(at + 0.11) as f32), 50.0 + 80.0 * hash01(at + 0.23) as f32, shift, 0.9))
            })
            .chain(self.crumbs_at(t, len))
            .collect()
    }

    /// Portal's entry and exit (distances along a lane `len` long) at map time `t`: a new pair
    /// every 8 bars.
    fn portal_at(&self, t: f64, len: f32) -> Option<(f32, f32)> {
        if !self.mods.has(Mod::Portal) {
            return None;
        }
        let section = bar_at(&self.map.barlines, t, 2000.0).map_or(0, |(i, ..)| i / 8) as f64;
        let entry = len * (0.5 + 0.3 * hash01(section * 1.3 + 0.2) as f32);
        let mut exit = len * (0.15 + 0.5 * hash01(section * 2.9 + 0.6) as f32);
        if (entry - exit).abs() < 0.15 * len {
            exit = entry * 0.45;
        }
        Some((entry, exit))
    }

    /// Lane, barlines, notes, drum, combo and judgements, laid out as if `c` were the window (the
    /// caller maps it into the playfield area). The scroll and visual mods apply here; Multiverse
    /// draws it three times over.
    pub fn draw_playfield(&mut self, c: &mut Canvas, scroll_speed: f64) {
        if self.mods.has(Mod::Multiverse) {
            self.draw_multiverse(c, scroll_speed);
        } else {
            let uni = Universe { colours: multiverse::note_colours(), ..Universe::CLASSIC };
            self.draw_field(c, scroll_speed, Focus::FULL, &uni);
        }
    }

    /// Multiverse at map time `t`, for each playfield (see `Focus`). The playfield the focus goes
    /// to next is called over the two beats before its bar: it parks, lights up and pings on each
    /// beat. Over the last three quarters of a beat the focus slides across.
    fn multiverse_at(&self, t: f64) -> [Focus; FIELD_COUNT] {
        let mut out = [Focus::default(); FIELD_COUNT];
        let sw = &self.focus_switches;
        let i = sw.partition_point(|s| s.at <= t);
        let now = i.checked_sub(1).map_or(FIRST_FOCUS, |i| sw[i].field);
        out[now].focus = 1.0;
        out[now].colour = 1.0;
        out[now].grow = 1.0;
        out[now].settle = 1.0;
        if let Some(s) = i.checked_sub(1).map(|i| &sw[i]).filter(|s| t - s.at < FOCUS_BURST_MS) {
            out[s.field].burst = Some(((t - s.at) / FOCUS_BURST_MS) as f32);
        }
        if let Some(&FocusSwitch { at, field: next, beat, .. }) = sw.get(i) {
            let beat = beat.max(100.0);
            let fade = (0.75 * beat).clamp(150.0, 450.0);
            let x = ((t - (at - fade)) / fade).clamp(0.0, 1.0);
            let p = smooth(0.0, 1.0, x) as f32;
            out[now].focus = 1.0 - p;
            out[now].colour = 1.0 - p;
            out[now].grow = 1.0 - p;
            out[now].settle = 1.0 - p;
            out[next].focus = p;
            out[next].grow = overshoot(x as f32);
            let called = at - 2.0 * beat;
            out[next].settle = smooth(called, at - fade, t) as f32;
            // Its notes bloom into colour over the first beat of the call, a beat before the
            // focus lands; the one in focus keeps its colours till then.
            out[next].colour = smooth(called, called + beat, t) as f32;
            out[next].aim = (smooth(called, called + 0.3 * beat, t) * (1.0 - smooth(at - 0.5 * fade, at, t))) as f32;
            // Called, it's already a little bigger than the rest.
            out[next].grow = out[next].grow.max(0.3 * out[next].aim);
            if t >= called {
                out[next].call = (1.0 - ((t - called) / beat).fract() as f32).powi(2);
            }
        }
        out
    }

    /// Multiverse: the playfield eight times over, each a window circling about its home, over
    /// each other. The one in focus is parked, big, bright, in colour and steady; the others
    /// small, dim and colourless. Sparks run from it to the one the focus goes to next.
    fn draw_multiverse(&mut self, c: &mut Canvas, scroll_speed: f64) {
        use std::f32::consts::{PI, TAU};
        // On the beat as it'll be seen, like the notes (see `draw_field`).
        let t = self.visual_time();
        let w = c.width;
        let inner = FIELD_W * w / FIELD_ZOOM.1;
        let looks = self.multiverse_at(t);
        let cp = beatmap::control_point_at(&self.map.control_points, t);
        let pulse = breath(cp.beat_phase(t), cp.beat_length);
        // Out of focus first, then the one it's heading for, then the one in focus on top.
        let mut order: [usize; FIELD_COUNT] = std::array::from_fn(|f| f);
        let rank = |f: usize| 2.0 * looks[f].focus + looks[f].aim;
        order.sort_by(|&a, &b| rank(a).total_cmp(&rank(b)));
        // Where each drum ends up on screen, for the sparks.
        let drum_x = if self.mods.has(Mod::Mirror) { inner - HIT_X } else { HIT_X };
        let mut drums = [(0.0f32, 0.0f32); FIELD_COUNT];
        let universes = self.universes.unwrap_or_else(multiverse::get);
        let base = self.mods;
        // The zone in play, summed weighted by focus, and the weights.
        let mut zone = ([0.0f32; 4], 0.0f32);
        for i in order {
            let l = looks[i];
            let (left, mid, lean) = HOMES[i];
            let start = c.shapes.len();
            // Drawn at its size in focus, so it reaches as far ahead in time whatever its zoom,
            // with its universe's mods on top of the play's for just as long.
            c.width = inner;
            self.mods = base.plus(universes[i].mods);
            self.draw_field(c, scroll_speed, l, &universes[i]);
            self.mods = base;
            c.width = w;
            let field_zone = self.zone.take();
            // Out of focus it circles its home, each its own way round and at its own pace,
            // leaning and rocking. Called or in focus it parks where it fits on screen, holds
            // still and breathes with the beat.
            let loose = 1.0 - l.settle;
            let dir = if i % 2 == 0 { 1.0 } else { -1.0 };
            let turn = dir * TAU * (t / (7000.0 + 1300.0 * i as f64)).rem_euclid(1.0) as f32 + i as f32 * 1.7;
            let parked = (parked(i).0 * w, parked(i).1);
            let circling = (left * w + ORBIT.0 * turn.cos(), mid + ORBIT.1 * turn.sin());
            let (x, y) = (parked.0 + (circling.0 - parked.0) * loose, parked.1 + (circling.1 - parked.1) * loose);
            let angle = loose * (lean + 0.08 * (1.5 * turn).sin());
            let size = FIELD_ZOOM.0 + (FIELD_ZOOM.1 - FIELD_ZOOM.0) * l.grow;
            let swell = 0.015 * pulse * l.focus;
            let zoom = size * (1.0 + swell);
            // It breathes about its drum, not the lane's left end, so its notes keep their pace.
            let k = size * swell * drum_x;
            let (x, y) = (x - k * angle.cos(), y - k * angle.sin());
            let lit = l.aim * (1.0 - l.focus);
            c.fade_since(start, 0.45 + 0.55 * l.focus + 0.4 * lit, 0.75 + 0.25 * l.focus + 0.2 * lit);
            c.rotate_since(start, [0.0, LANE_Y], angle, zoom, [x, y]);
            drums[i] = (x + drum_x * zoom * angle.cos(), y + drum_x * zoom * angle.sin());
            // The zone in play: the one in focus, sliding to the next as the focus does.
            if let Some(z) = field_zone.filter(|_| l.focus > 0.0) {
                let z = turned_box(z, [0.0, LANE_Y], angle, zoom, [x, y]);
                for (sum, v) in zone.0.iter_mut().zip(z) {
                    *sum += v * l.focus;
                }
                zone.1 += l.focus;
            }
        }
        self.zone = (zone.1 > 0.0).then(|| zone.0.map(|v| v / zone.1));
        // The sparks: from the drum in focus to the drum of the one it's heading for.
        let to = (0..FIELD_COUNT).find(|&f| looks[f].aim > 0.0);
        let from = (0..FIELD_COUNT).filter(|&f| looks[f].aim == 0.0).max_by(|&a, &b| looks[a].focus.total_cmp(&looks[b].focus));
        if let (Some(from), Some(to)) = (from, to) {
            let (a, b) = (drums[from], drums[to]);
            let n = ((b.0 - a.0).hypot(b.1 - a.1) / 18.0).ceil().max(1.0) as usize;
            let run = (t / 350.0).rem_euclid(1.0) as f32;
            for k in 0..n {
                let s = (k as f32 + run) / n as f32;
                let (x, y) = (a.0 + (b.0 - a.0) * s, a.1 + (b.1 - a.1) * s);
                let alpha = looks[to].aim * (PI * s).sin() * (0.4 + 0.6 * s);
                c.glow(x, y, 2.5, 7.0, rgba(FOCUS_COL, 0.6 * alpha));
                c.circle(x, y, 2.0, rgba(WHITE, alpha), rgba(WHITE, 0.0), 0.0);
            }
        }
    }

    /// One playfield (see `draw_playfield`). Multiverse's are windows, framed and lit by `look`,
    /// each in its universe's colours, note shape and speed; out of colour, dons and kats look
    /// alike.
    fn draw_field(&mut self, c: &mut Canvas, scroll_speed: f64, look: Focus, uni: &Universe) {
        use std::f32::consts::{PI, TAU};
        // Notes, barlines and the beat pulse are drawn at `vt`, where they'll be once the frame is
        // on screen; animations of what already happened (hits, presses) run on `t`.
        let t = self.time;
        let vt = self.visual_time();
        let w = c.width;
        let start_shape = c.shapes.len();
        let mods = self.mods;
        let party = mods.has(Mod::Party);
        // Crossfire: the drum in the middle, kats coming from the left (laid out here the other
        // way along the lane from it), dons from the right.
        let crossfire = mods.has(Mod::Crossfire);
        let side_of = |kind: Kind, s: f32| if crossfire && kind == Kind::Kat { -s } else { s };
        // Multiverse: a window, not the whole screen. Nothing may stick out of it.
        let boxed = mods.has(Mod::Multiverse);
        let lane_top = LANE_Y - LANE_H / 2.0;
        let cp = beatmap::control_point_at(&self.map.control_points, vt);
        let beat_phase = cp.beat_phase(vt) as f32;
        // Beats since the red line, for the mods that move with the beat.
        let beats = ((vt - cp.beat_origin) / cp.beat_length) as f32;
        let pulse = (1.0 - beat_phase).powi(3);
        let kiai = cp.kiai && self.kiai_effects;
        let in_kiai = cp.kiai;
        // Radar: where the beam is (see `radar_sweep`), and how long it takes to cross the lane.
        let radar = mods.has(Mod::Radar);
        let sweep = radar_sweep(beats, in_kiai);
        let sweep_ms = cp.beat_length * if in_kiai { RADAR_BEATS.1 } else { RADAR_BEATS.0 } as f64;

        let warp = beat_warp(mods, cp, vt) + self.extra_warp(vt, cp);
        // The lane's shape, and how fast notes go along it (see `lane_stretch`).
        let (rail, lane_len, rail_drop, rail_zoom) = self.rail_at(vt, w);
        // Where it's played before the whole-playfield moves (x0, y0, x1, y1): from the screen's
        // left edge (the drum) along the lane, with room for the judgements.
        let (band_top, band_h) = lane_band();
        let local_zone = match &rail {
            Some(r) => r.pts.iter().fold([0.0, band_top, HIT_X, band_top + band_h], |z, &(x, y, _)| {
                [z[0], z[1].min(y - LANE_H / 2.0), z[2].max(x + LANE_H / 2.0), z[3].max(y + LANE_H / 2.0)]
            }),
            None => [0.0, band_top, (HIT_X + lane_len).max(HIT_X + BIG_R), band_top + band_h],
        };
        let portal = self.portal_at(vt, lane_len);
        let scroll = Scroll {
            t: vt,
            speed: osu_speed_scale(w) * scroll_speed * uni.speed() * mods.scroll() * lane_stretch(rail.is_some(), lane_len, w),
            lane: lane_len as f64,
            mods,
            warp,
            live: self.miss_factor(t),
        };
        // Distance along the lane of something at map time `time` scrolling at `vel`.
        let s_of = |time: f64, vel: f64| scroll.x(time, vel) - HIT_X;
        // Uncapped: extremely slow notes are common, so the whole map may be on screen at once,
        // its last object included (owner, 2026-10-28).
        // ponytail: one map-wide lookahead, so a slow section anywhere widens every frame's scan
        // to the whole map; per-section reach (like barline_times) if that gets slow.
        let lookahead = (lane_len + BIG_R) as f64 / (self.min_velocity * scroll.speed * scroll.min_factor()) + warp.abs();

        // Notes on screen, nearest first.
        let start = self.map.notes.partition_point(|n| n.time < vt.min(t) - lookahead.max(20_000.0));
        self.visible.clear();
        for i in start..self.map.notes.len() {
            let n = &self.map.notes[i];
            if n.time - vt > lookahead {
                break;
            }
            if n.end_time + n.fake_len < vt.min(t) - 3000.0 && n.kind != Kind::Swell {
                continue;
            }
            self.visible.push(i);
        }
        if self.hide_notes {
            self.visible.clear();
        }
        // Distance along the lane of note `i`. Liar: far off, a triplet sits where a 1/4 would,
        // sliding to its real place on the way in.
        let note_s = |i: usize| -> f32 {
            let n = &self.map.notes[i];
            let s = s_of(n.time, n.velocity);
            match self.lies.get(i) {
                Some(&lie) if lie != n.time => {
                    let fake = s_of(lie, n.velocity);
                    s + (fake - s) * smooth(0.3, 0.55, (s / lane_len) as f64) as f32
                }
                _ => s,
            }
        };
        // Shatter: pieces of the lane with streams on them slide further.
        let mut shatter = self.shatter_at(vt);
        if mods.has(Mod::Shatter) {
            let mut on = [[0u32; 2]; 4];
            for &i in &self.visible {
                let s = note_s(i);
                if matches!(self.map.notes[i].kind, Kind::Don | Kind::Kat) && s > 0.0 && s < lane_len {
                    let piece = &mut on[((s / lane_len * 4.0) as usize).min(3)];
                    piece[0] += 1;
                    piece[1] += (self.flows[i] >= 3) as u32;
                }
            }
            for (d, [all, streams]) in shatter.iter_mut().zip(on) {
                *d *= 0.6 + 0.8 * streams as f32 / all.max(1) as f32;
            }
        }
        let geo = LaneGeo { rail, len: lane_len, shatter, cracks: self.cracks_at(t, lane_len) };
        let bent = geo.bent() || portal.is_some();
        // Where on screen something `s` along the lane is drawn, `side` off its middle: through
        // the portal, onto the (bent, broken) lane. Also its heading and size.
        let place = |s: f32, side: f32| -> (f32, f32, f32, f32) {
            let (s, size) = portal.map_or((s, 1.0), |p| through_portal(s, p));
            let (x, y, a) = geo.at(s, side);
            (x, y, a, size)
        };
        // A window's notes fade in over the end of its lane rather than poke out past it.
        let fade_in = |s: f32| -> f32 { if boxed { smooth(lane_len as f64, (lane_len - 60.0) as f64, s as f64) as f32 } else { 1.0 } };
        // How visible an approaching note `s` along the lane is under the visual mods.
        let memory_over = mods.has(Mod::Memory) && vt > self.first_note + 15_000.0;
        let note_alpha = |s: f32| -> f32 {
            let u = s / lane_len;
            let mut a = fade_in(s);
            if mods.has(Mod::Hidden) {
                // osu!lazer: fades out over the first 37.5% of its way in.
                a *= ((u - 0.625) / 0.375).clamp(0.0, 1.0);
            }
            if mods.has(Mod::Sudden) {
                a *= ((0.45 - u) / 0.12).clamp(0.0, 1.0);
            }
            if mods.has(Mod::Blink) && u > 0.15 && (0.5..0.95).contains(&beat_phase) {
                a = 0.0;
            }
            if memory_over {
                a = 0.0;
            }
            a
        };
        // Radar: how far a note at map time `time` scrolling at `vel` comes in a sweep, and how
        // lit it is `s` along the lane.
        let radar_u = |time: f64, vel: f64| s_of(time + sweep_ms, vel) - s_of(time, vel);
        let radar_lit = |s: f32, time: f64, vel: f64| if radar { radar_light(s, radar_u(time, vel), lane_len, sweep) } else { 1.0 };
        // A don or kat `s` along the lane (`k`th drawn, in a 1/`flow` stream): Snake swings it
        // sideways, Party pumps and squishes it. Nothing changes at the drum.
        let draw_hit = |c: &mut Canvas, s: f32, n: &beatmap::Note, k: usize, flow: u8, col: u32, a: f32| {
            let mut side = 0.0;
            if mods.has(Mod::Snake) {
                // Streams swing the most, triplets the other way, notes on the beat not at all.
                let (amp, flip) = match flow {
                    1 => (0.0, 0.0),
                    2 => (0.45, 0.0),
                    0 => (0.6, 0.0),
                    d if d % 3 == 0 => (1.0, PI),
                    _ => (1.0, 0.0),
                };
                // The wave jolts forward on each beat.
                let u = (s / lane_len).max(0.0);
                let f = smooth(0.0, 0.3, u as f64) as f32;
                let jolt = beats.floor() + 1.0 - (1.0 - beat_phase).powi(3);
                side += 32.0 * amp * f * (0.8 + 0.4 * pulse) * (TAU * 1.3 * u - PI * jolt + flip).sin();
            }
            let (x, y, _, mut size) = place(side_of(n.kind, s), side);
            let mut squish = 1.0;
            if party {
                // Each note pulses on its own grid: a 1/4 four times a beat.
                let own = (1.0 - (beats * flow.clamp(1, 8) as f32).rem_euclid(1.0)).powi(3);
                size *= 1.0 + 0.22 * own;
                squish = 1.0 + 0.3 * own * if k % 2 == 0 { 1.0 } else { -1.0 };
            }
            let r = if n.big { BIG_R } else { NOTE_R } * size;
            let first = c.shapes.len();
            if uni.shape != NoteShape::Round || !skin_note(c, x, y, r, n.big, col, a) {
                multiverse::note(c, uni.shape, x, y, r, col, a);
            }
            if squish != 1.0 {
                c.stretch_since(first, [x, y], squish, 1.0 / squish);
            }
        };
        let (don, kat) = if mods.has(Mod::Colorless) || mods.has(Mod::Echo) { (GREY, GREY) } else { (mix(uni.idle(), uni.don(), look.colour), mix(uni.idle(), uni.kat(), look.colour)) };
        // Text is drawn last, after Mirror flips the rest (letters must stay readable), and so
        // are the skin's numbers and judgements (centre x, top, height).
        let mut texts: Vec<(f32, f32, f32, [f32; 4], String)> = Vec::new();
        let mut upright: Vec<(f32, f32, f32, [f32; 4], Upright)> = Vec::new();

        // Multiverse: a halo round the window, for the focus (breathing with the beat) and for
        // the call (glowing, flaring on each beat).
        let halo = (look.focus * (0.6 + 0.4 * pulse)).max(look.aim * (0.75 + 0.25 * look.call));
        if boxed && halo > 0.01 {
            for (k, a) in [0.3, 0.16, 0.08, 0.03].into_iter().enumerate() {
                let d = 3.0 + 6.0 * k as f32;
                c.round_rect(-d, lane_top - 3.0 - d, w + 2.0 * d, LANE_H + 6.0 + 2.0 * d, 4.0 + d, rgba(0, 0.0), rgba(FOCUS_COL, a * halo), 6.0);
            }
        }

        // Lane. When it turns, it reaches past the screen so its ends never show.
        let turns = mods.has(Mod::Tilt) || mods.has(Mod::Spin) || mods.has(Mod::Mania) || mods.has(Mod::BarrelRoll) || crossfire;
        let mut lane_col = if kiai { rgba(0x2b2433, 1.0) } else { rgba(0x20202b, 1.0) };
        let mut edge_col = rgba(0x3a3a4e, 1.0);
        let mut skin_lane_tint = rgba(WHITE, 1.0);
        if radar {
            lane_col = if in_kiai { rgba(0x17150d, 1.0) } else { rgba(0x0b110f, 1.0) };
        }
        if kiai {
            // Radar's lane stays dark, a little warmer.
            lane_col = tint(lane_col, 0xff8a3d, (0.05 + 0.06 * pulse) * if radar { 0.35 } else { 1.0 });
        }
        if party {
            // The lane flashes a new colour on every beat.
            let col = hue(hash01(beats.floor() as f64 * 2.7 + 0.3));
            lane_col = tint(lane_col, col, (0.07 + 0.16 * pulse) * if in_kiai { 1.6 } else { 1.0 });
            edge_col = rgba(col, 0.9);
            skin_lane_tint = tint(skin_lane_tint, col, 0.15 + 0.25 * pulse);
        }
        if geo.bent() {
            // In short pieces along the way: edges first, so the lane's inside covers their joins.
            const PIECE: f32 = 8.0;
            let from = if turns { -w } else { -HIT_X };
            let to = lane_len + if boxed { 0.0 } else if geo.rail.is_some() { 60.0 } else { HIT_X };
            let n = ((to - from) / PIECE).ceil() as usize;
            for (thick, col) in [(LANE_H + 6.0, edge_col), (LANE_H, lane_col)] {
                for k in 0..n {
                    let s = from + (k as f32 + 0.5) * PIECE;
                    // Longer by how much the outside of a bend is longer than the middle, by the
                    // sharper turn to either neighbour: where a corner starts or ends (Zigzag's
                    // are tight) the turn within the piece is less than to the next one, and the
                    // outer edge tore there.
                    let a = |s: f32| geo.at(s, 0.0).2;
                    let turn = (a(s + PIECE) - a(s)).abs().max((a(s) - a(s - PIECE)).abs());
                    lane_rect(c, geo.at(s, 0.0), PIECE + 1.0 + turn * thick / 2.0, thick, col);
                }
            }
            // Cracks: dark seams where the knocked-out pieces start and end.
            for &(a, l, _, seam) in &geo.cracks {
                for s in [a, a + l] {
                    lane_rect(c, geo.at(s, 0.0), 3.0, LANE_H + 6.0, rgba(0x08080b, seam));
                }
            }
        } else {
            let (lane_x, lane_w) = if turns { (-w, 3.0 * w) } else { (0.0, w) };
            let glow = if kiai { 0.75 + 0.25 * pulse } else { 0.0 };
            // Radar's lane is its own dark one.
            if radar || !skin_lane(c, lane_x, lane_w, skin_lane_tint, glow) {
                draw_lane(c, lane_x, lane_w, lane_col, edge_col);
            }
        }
        // Crumble: crumbs fly off the edge each piece moves out to and fall away.
        for cr in &self.crumbs {
            let age = ((t - cr.at) / 1000.0) as f32;
            if !(0.0..0.7).contains(&age) {
                continue;
            }
            for k in 0..1 + (4.0 * cr.size).round() as usize {
                let h = |j: f64| hash01(cr.at + k as f64 * 5.3 + j) as f32;
                let s = cr.start * lane_len + cr.len * h(0.1);
                let (x, y, a) = geo.at(s, cr.shift.signum() * (LANE_H / 2.0 + 3.0));
                let (vx, vy) = ((h(0.2) * 2.0 - 1.0) * 70.0, cr.shift.signum() * (30.0 + 60.0 * h(0.3)));
                let p = (x + vx * age, y + vy * age + 700.0 * age * age, a + (h(0.4) * 2.0 - 1.0) * 8.0 * age);
                let side = 3.0 + 6.0 * cr.size * h(0.5);
                let col = tint(edge_col, WHITE, 0.3);
                lane_rect(c, p, side, side, [col[0], col[1], col[2], col[3] * (1.0 - age / 0.7)]);
            }
        }
        // Portal: the entry (orange) and the exit (blue), across the lane.
        if let Some((entry, exit)) = portal {
            for (s, col) in [(entry, 0xff8a3d), (exit, 0x4fb4ff)] {
                let p = geo.at(s, 0.0);
                c.glow(p.0, p.1, 22.0, 20.0, rgba(col, 0.3 + 0.2 * pulse));
                lane_rect(c, p, 7.0, LANE_H * 0.9, rgba(col, 0.9));
            }
        }

        // Barlines.
        // As far back as ahead: a slow bar can still be on screen long after it passed.
        let first_bar = self.map.barlines.partition_point(|b| b.time < vt - lookahead);
        let bars = if self.hide_notes { &[][..] } else { &self.map.barlines[first_bar..] };
        for b in bars {
            if b.time - vt > lookahead {
                break;
            }
            let s = s_of(b.time, b.velocity);
            let shown = if crossfire { s > 0.0 } else { s > PANEL_W - HIT_X };
            if shown && s < lane_len + 2.0 {
                for s in if crossfire { vec![s, -s] } else { vec![s] } {
                    let (x, y, a, _) = place(s, 0.0);
                    if let Some([bw, bh]) = c.sprite_size(Sprite::Barline) {
                        c.sprite_box(Sprite::Barline, x, y, (bw * LANE_PX / 2.0).max(0.6), bh * LANE_PX / 2.0, rgba(WHITE, 1.0));
                        if let Some(sh) = c.shapes.last_mut() {
                            sh.angle = -a;
                        }
                    } else {
                        lane_rect(c, (x, y, a), 1.6, LANE_H, rgba(WHITE, 0.28));
                    }
                }
            }
        }
        // Roulette: where the speed changes, and to what.
        let first_section = self.sections.partition_point(|s| s.0 < vt - 3000.0);
        for &(time, k, vel) in &self.sections[first_section..] {
            if time - vt > lookahead || self.hide_notes {
                break;
            }
            let s = s_of(time, vel);
            if s > PANEL_W - HIT_X && s < lane_len + 30.0 {
                let col = if k > 1.0 { 0xff5a3d } else if k < 1.0 { 0x4fb4ff } else { WHITE };
                let (x, y, a, _) = place(s, 0.0);
                lane_rect(c, (x, y, a), 4.0, LANE_H, rgba(col, 0.8));
                let (lx, ly, ..) = place(s + 18.0, -LANE_H / 2.0 + 10.0);
                texts.push((lx, ly - 4.0, 8.0, rgba(col, 1.0), format!("x{k}")));
            }
        }

        // Radar: the beam, a bright line with a trail fading out behind it.
        if radar && !self.hide_notes {
            let b = sweep * lane_len;
            for d in if crossfire { vec![1.0, -1.0] } else { vec![1.0] } {
                for k in 0..14 {
                    let back = b - 6.0 * k as f32 - 3.0;
                    if back > 0.0 {
                        let (x, y, a, _) = place(d * back, 0.0);
                        lane_rect(c, (x, y, a), 6.5, LANE_H, rgba(RADAR_COL, 0.16 * (1.0 - k as f32 / 14.0).powi(2)));
                    }
                }
                let (x, y, a, _) = place(d * b, 0.0);
                lane_rect(c, (x, y, a), 9.0, LANE_H, rgba(RADAR_COL, 0.2));
                lane_rect(c, (x, y, a), 2.5, LANE_H, rgba(0xc8ffe0, 0.95));
            }
        }

        // Hit target.
        draw_hit_target(c);
        if kiai {
            c.glow(HIT_X, LANE_Y, NOTE_R, 30.0, rgba(0xffb347, 0.25 * pulse));
        }
        if let Some((j, at)) = self.last_judge.filter(|_| !self.hidden.has(Part::Judgements)) {
            let a = (1.0 - (t - at) / 220.0).clamp(0.0, 1.0) as f32;
            if a > 0.0 && j != Judge::Miss && splashes(j) {
                c.glow(HIT_X, LANE_Y, NOTE_R + 2.0, 26.0, rgba(j.color(), 0.55 * a));
            }
        }

        // Decoys, under the real notes: gone before they'd have to be hit.
        if !self.hide_notes {
            let first = self.decoys.partition_point(|n| n.time < vt);
            for (k, n) in self.decoys.iter().enumerate().skip(first) {
                if n.time - vt > lookahead {
                    break;
                }
                let s = s_of(n.time, n.velocity);
                // A little see-through, so a careful eye can tell them apart.
                let a = DECOY_ALPHA * note_alpha(s) * radar_lit(s, n.time, n.velocity) * smooth(0.22, 0.38, (s / lane_len) as f64) as f32;
                if a > 0.01 && s < lane_len + BIG_R {
                    draw_hit(c, s, n, k, self.decoy_flows[k], if n.kind == Kind::Don { don } else { kat }, a);
                }
            }
        }

        // Notes, back to front so earlier notes sit on top. A window's leave it under the drum.
        // Hit ones fly out of the drum over its panel, so they're drawn after it.
        let mut flying: Vec<(f32, bool, u32)> = Vec::new();
        let behind = |r: f32| if boxed { r - HIT_X } else { -HIT_X - r };
        let on_lane = |s: f32, r: f32| s > behind(r) && s < lane_len + r;
        // On one ms, a fake slider sits over the don/kat it's stacked on (and, with the map's
        // own spinner-warning, a swell head too).
        let map_spinner = crate::skin::map_has_spinner_warning();
        let over = |n: &beatmap::Note| n.kind == Kind::Roll || (map_spinner && n.kind == Kind::Swell);
        let main_beat = self.main_beat;
        let sm = self.map.meta.slider_multiplier;
        let popup = |n: &beatmap::Note| popup(n, vt, lane_len, scroll.speed, main_beat, sm, beatmap::control_point_at(&self.map.control_points, n.time).sv);
        let mut order = self.visible.clone();
        order.sort_by(|&a, &b| {
            let (na, nb) = (&self.map.notes[a], &self.map.notes[b]);
            nb.time.total_cmp(&na.time).then(over(na).cmp(&over(nb))).then(b.cmp(&a))
        });
        for i in order {
            let n = &self.map.notes[i];
            let st = &self.state[i];
            let r = if n.big { BIG_R } else { NOTE_R };
            match n.kind {
                Kind::Don | Kind::Kat => {
                    let col = if n.kind == Kind::Don { don } else { kat };
                    match st.judged {
                        // Hit: the note flies up out of the drum (unless the interface toggle hides it).
                        Some(Judge::Great | Judge::Ok) if self.hidden.has(Part::HitAnimation) => {}
                        Some(Judge::Great | Judge::Ok) => {
                            let p = ((t - st.judged_at) / HIT_ANIM_MS).clamp(0.0, 1.0) as f32;
                            if p < 1.0 {
                                flying.push((p, n.big, col));
                            }
                        }
                        judged => {
                            let s = note_s(i);
                            if on_lane(s, r) {
                                let a = if judged.is_some() { 0.4 } else { note_alpha(s) * radar_lit(s, n.time, n.velocity) };
                                if a > 0.01 {
                                    draw_hit(c, s, n, i, self.flows[i], col, a);
                                }
                                // Radar: a note the beam just passed flashes and pings.
                                let ping = radar_age(s, radar_u(n.time, n.velocity), lane_len, sweep).filter(|&age| radar && judged.is_none() && s > RADAR_NEAR && age < 0.35);
                                if let Some(age) = ping {
                                    let (x, y, _, size) = place(side_of(n.kind, s), 0.0);
                                    let k = age / 0.35;
                                    let fade = note_alpha(s) * (1.0 - k);
                                    let sprite = if n.big { Sprite::BigCircle } else { Sprite::HitCircle };
                                    c.light(sprite, x, y, r * size, rgba(WHITE, 0.25 * fade * (1.0 - k)));
                                    c.ring(x, y, (r + 3.0 + 16.0 * k) * size, rgba(RADAR_COL, 0.85 * fade), 1.0 + 2.5 * (1.0 - k));
                                }
                            }
                        }
                    }
                }
                Kind::Roll => {
                    let pop = popup(n);
                    let s0 = pop.map_or_else(|| s_of(n.time, n.velocity), |p| p.0);
                    // A fake slider, as stable draws it: body right of the head by its length,
                    // cap *left* of it by the same. A long one's head and cap are cut when the cap
                    // reaches the drum; a short one's pass through it like a real roll's (owner,
                    // 2026-10-28). The body goes when it does (fading, if short). Nothing shows before the
                    // head is on screen, so a long one is only ever body. Still drawn at the
                    // head's speed, and gimmick sections run past the BPM / SV limits this game
                    // clamps to, so long ones aren't exact.
                    // A pop-up one (see `popup`) stands still, fading in, and is never cut.
                    let fake = n.fake_len > 0.0;
                    let mut fake_a = pop.map_or(1.0, |p| p.2);
                    if fake && pop.is_none() && vt >= n.time {
                        let p = ((vt - n.time) / FAKE_FADE_MS) as f32;
                        if !n.fake_fades || p >= 1.0 {
                            continue;
                        }
                        fake_a = 1.0 - p;
                    }
                    if fake_a < 0.01 {
                        continue;
                    }
                    let cut = pop.is_none() && fake && !n.fake_fades && vt >= n.time - n.fake_len;
                    let (s_cap, s1) = match pop {
                        Some((s, d, _)) => (s - d, s + d),
                        None => (s_of(n.time - n.fake_len, n.velocity), s_of(n.end_time + n.fake_len, n.velocity)),
                    };
                    if s1 < -HIT_X - r || s0 > lane_len + r {
                        continue;
                    }
                    let (s0, s1) = if boxed { (s0.max(behind(r)), s1.min(lane_len)) } else { (s0, s1) };
                    let a = if memory_over { 0.0 } else { fade_in(s0) * fake_a };
                    let body_r = r * 0.86;
                    if bent {
                        // The body follows the lane in short pieces (none across a portal).
                        let (from, to) = (s0.max(-HIT_X - r), s1.min(lane_len + r));
                        let steps = ((to - from) / 6.0).ceil().max(1.0) as usize;
                        let pts: Vec<(f32, f32)> = (0..=steps).map(|k| place(from + (to - from) * k as f32 / steps as f32, 0.0)).map(|p| (p.0, p.1)).collect();
                        for (rr, col) in [(body_r + 2.5, rgba(WHITE, a)), (body_r, rgba(ROLL, a))] {
                            for p in pts.windows(2) {
                                if (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1) < 20.0 {
                                    segment(c, p[0], p[1], rr, col);
                                }
                            }
                        }
                    } else if let Some([_, sy]) = c.sprite_size(Sprite::RollMiddle) {
                        // The skin's body stretched from the head to the end, its cap just past that.
                        let (x0, x1) = (HIT_X + s0, HIT_X + s1);
                        c.sprite_box(Sprite::RollMiddle, (x0 + x1) / 2.0, LANE_Y, (x1 - x0) / 2.0, r * sy, rgba(ROLL, a));
                        if let Some([ex, ey]) = c.sprite_size(Sprite::RollEnd).filter(|_| !cut) {
                            let xe = if fake { HIT_X + s_cap } else { x1 };
                            c.sprite_box(Sprite::RollEnd, xe + r * ex, LANE_Y, r * ex, r * ey, rgba(ROLL, a));
                        }
                    } else {
                        c.capsule(HIT_X + s0, HIT_X + s1, LANE_Y, body_r, rgba(ROLL, a), rgba(WHITE, a), 2.5);
                        if fake && !cut {
                            c.circle(HIT_X + s_cap, LANE_Y, body_r, rgba(ROLL, a), rgba(WHITE, a), 2.5);
                        }
                    }
                    if cut {
                        continue;
                    }
                    let cp = beatmap::control_point_at(&self.map.control_points, n.time);
                    for (k, hit) in st.ticks.iter().enumerate() {
                        let ts = s_of(n.time + k as f64 * n.tick_spacing, n.velocity);
                        if !hit && ts > s0 && ts < lane_len {
                            let (x, y, ..) = place(ts, 0.0);
                            let tint = tick_tint(n.time + k as f64 * n.tick_spacing, cp.beat_origin, cp.beat_length);
                            if !c.sprite(Sprite::RollTick, x, y, NOTE_R, rgba(tint, a)) {
                                c.circle(x, y, 4.0, rgba(WHITE, 0.85 * a), rgba(WHITE, 0.0), 0.0);
                            }
                        }
                    }
                    let (x, y, _, size) = place(s0, 0.0);
                    if !skin_note(c, x, y, r * size, n.big, ROLL, a) {
                        c.circle(x, y, r * size, rgba(ROLL, a), rgba(WHITE, a), r * size * 0.1);
                    }
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
                    if vt < n.time {
                        let s = s_of(n.time, n.velocity);
                        let a = note_alpha(s) * radar_lit(s, n.time, n.velocity) * swell_fade(beatmap::control_point_at(&self.map.control_points, n.time).beat_length, n.time - vt);
                        if s < lane_len + r && a > 0.01 {
                            let (x, y, _, size) = place(s, 0.0);
                            let r = r * size;
                            // A map shipping its own spinner-warning draws its swells as small notes.
                            let drawn = if map_spinner && c.sprite_size(Sprite::SpinnerWarning).is_some() {
                                c.sprite_box(Sprite::SpinnerWarning, x, y, NOTE_R * size, NOTE_R * size, rgba(WHITE, a));
                                true
                            } else {
                                c.sprite(Sprite::SpinnerWarning, x, y, r, rgba(WHITE, a))
                            };
                            if !drawn {
                                c.circle(x, y, r, rgba(SWELL, a), rgba(WHITE, a), r * 0.1);
                                c.ring(x, y, r * 0.55, rgba(WHITE, 0.8 * a), 3.0);
                            }
                        }
                    } else if c.sprite(Sprite::SpinnerCircle, HIT_X, LANE_Y, SPIN_R, rgba(WHITE, 1.0)) {
                        // The skin's ring closes in on its circle as the time to hit it runs out.
                        let left = 1.0 - ((t - n.time) / (n.end_time - n.time).max(1.0)).clamp(0.0, 1.0) as f32;
                        c.sprite(Sprite::SpinnerApproach, HIT_X, LANE_Y, SPIN_R * (1.0 + 1.5 * left), rgba(WHITE, 1.0));
                        let left = n.required_hits.saturating_sub(st.hits);
                        texts.push((HIT_X, LANE_Y - 12.0, 24.0, rgba(WHITE, 1.0), left.to_string()));
                    } else {
                        let progress = st.hits as f32 / n.required_hits.max(1) as f32;
                        c.circle(HIT_X, LANE_Y, SPIN_R, rgba(SWELL, 0.95), rgba(WHITE, 1.0), 4.0);
                        c.ring(HIT_X, LANE_Y, SPIN_R + 4.0 + 14.0 * progress, rgba(SWELL, 0.6), 3.0);
                        let left = n.required_hits.saturating_sub(st.hits);
                        texts.push((HIT_X, LANE_Y - 12.0, 24.0, rgba(WHITE, 1.0), left.to_string()));
                    }
                }
            }
        }

        // Kiai: every head on screen takes its own flash on the beat, added over everything drawn,
        // so a pile of fake sliders under one note (a "shiny") lights it brighter the deeper it
        // is. A circle glows a little white, a roll head fully in its own yellow (taiko_arranger).
        // Not on gimmick beats (60000 BPM lines), which would strobe.
        let flash = if self.kiai_effects { self.kiai_flash(vt) * KIAI_FLASH_ALPHA } else { 0.0 };
        if flash > 0.0 && !self.hide_notes {
            for &i in &self.visible {
                let n = &self.map.notes[i];
                let r = if n.big { BIG_R } else { NOTE_R };
                let sprite = if n.big { Sprite::BigCircle } else { Sprite::HitCircle };
                let (s, col, a) = match n.kind {
                    Kind::Don | Kind::Kat if self.state[i].judged.is_none() => {
                        let s = note_s(i);
                        (s, WHITE, CIRCLE_FLASH * note_alpha(s) * radar_lit(s, n.time, n.velocity))
                    }
                    Kind::Roll if popup(n).is_some() => {
                        let (s, _, pa) = popup(n).unwrap();
                        (s, ROLL, if memory_over { 0.0 } else { pa })
                    }
                    Kind::Roll if !(n.fake_len > 0.0 && !n.fake_fades && vt >= n.time - n.fake_len) => {
                        let s = s_of(n.time, n.velocity);
                        (s, ROLL, if memory_over { 0.0 } else { fade_in(s) })
                    }
                    _ => continue,
                };
                if a > 0.01 && on_lane(s, r) {
                    let (x, y, _, size) = place(side_of(n.kind, s), 0.0);
                    c.light(sprite, x, y, r * size, rgba(col, flash * a));
                }
            }
        }

        // Flashlight / Spotlight / Blackout: dark everywhere but a circle around the drum.
        let fl_combo = match self.stats.combo {
            0..100 => 1.0,
            100..200 => 0.9,
            _ => 0.8,
        };
        if mods.has(Mod::Flashlight) {
            darken_outside(c, HIT_X, LANE_Y, 190.0 * fl_combo, w);
        } else if mods.has(Mod::Spotlight) {
            darken_outside(c, HIT_X, LANE_Y, 190.0 * fl_combo * (0.85 + 0.3 * pulse), w);
        }
        if mods.has(Mod::Blackout) && in_kiai {
            darken_outside(c, HIT_X, LANE_Y, 70.0, w);
        }

        // Drum panel on top so passed notes slide underneath it.
        let flash = |at: f64| ((1.0 - (t - at) / 130.0).clamp(0.0, 1.0) * 0.95) as f32;
        let drum_x = if crossfire {
            for (k, f) in self.presses.map(flash).into_iter().enumerate() {
                if f > 0.0 {
                    let col = if k == 0 || k == 3 { uni.kat() } else { uni.don() };
                    c.glow(HIT_X, LANE_Y, NOTE_R, 18.0, rgba(col, 0.5 * f));
                }
            }
            HIT_X
        } else {
            draw_drum(c, self.presses.map(flash), uni.don(), uni.kat())
        };
        // The combo: on the drum, or with Crossfire under the hit target.
        let combo_y = |h: f32| if crossfire { lane_top + LANE_H + 8.0 } else { LANE_Y - h / 2.0 };
        for (p, big, col) in flying {
            let y = LANE_Y - 80.0 * p * (2.0 - p);
            let r = if big { BIG_R } else { NOTE_R } * (1.0 + 0.15 * p);
            if uni.shape != NoteShape::Round {
                multiverse::note(c, uni.shape, HIT_X, y, r, col, 1.0 - p);
            } else if !skin_note(c, HIT_X, y, r, big, col, 1.0 - p) {
                c.circle(HIT_X, y, r, rgba(col, 1.0 - p), rgba(WHITE, 1.0 - p), 3.0);
            }
        }
        if self.stats.combo >= 2 && !self.hidden.has(Part::Combo) {
            let s = self.stats.combo.to_string();
            let skin_size = if s.len() > 3 { 17.0 } else { 22.0 };
            if c.number_width(Font::Combo, skin_size, &s).is_some() {
                upright.push((drum_x, combo_y(skin_size), skin_size, rgba(WHITE, 1.0), Upright::Number(Font::Combo, s)));
            } else {
                let size = if s.len() > 3 { 12.0 } else { 16.0 };
                let col = if crossfire { 0xe8e8f0 } else { 0x1d1d26 };
                texts.push((drum_x, combo_y(size), size, rgba(col, 1.0), s));
            }
        }

        // Judgement text.
        if let Some((j, at)) = self.last_judge.filter(|&(j, _)| splashes(j) && !self.hidden.has(Part::Judgements)) {
            let age = (t - at) as f32;
            if age < 450.0 {
                let a = (1.0 - age / 450.0).min(1.0);
                let sprite = match j {
                    Judge::Great => Sprite::JudgeGreat,
                    Judge::Ok => Sprite::JudgeOk,
                    Judge::Miss => Sprite::JudgeMiss,
                };
                // The skin's sits in the hit target, popping a little as it shows. At its own size
                // next to the notes as osu! draws it: lazer fits the 128 px hitcircle into a 95
                // unit note but draws judgements pixel for pixel, so they're 128/95 of ours.
                if let Some([_, sh]) = c.sprite_size(sprite) {
                    const JUDGE_SCALE: f32 = 128.0 / 95.0;
                    let h = 2.0 * NOTE_R * sh * JUDGE_SCALE * (1.0 + 0.12 * (1.0 - age / 90.0).max(0.0));
                    upright.push((HIT_X, LANE_Y - h / 2.0 - age * 0.02, h, rgba(WHITE, a), Upright::Image(sprite)));
                } else {
                    texts.push((HIT_X, lane_top - 30.0 - age * 0.02, 15.0, rgba(j.color(), a), j.label().to_string()));
                }
            }
        }

        // Multiverse: the window's frame (unless the lane is broken), and the burst as it takes
        // the focus: a flash and a ring off the drum.
        if boxed && !geo.bent() {
            let frame = tint(rgba(0x3a3a4e, 1.0), FOCUS_COL, look.focus.max(look.aim));
            c.round_rect(0.0, lane_top - 3.0, w, LANE_H + 6.0, 4.0, rgba(0, 0.0), frame, 2.0);
        }
        // Called: a ping off the drum on each beat.
        if boxed && look.aim > 0.0 {
            let k = 1.0 - look.call.sqrt();
            c.ring(HIT_X, LANE_Y, NOTE_R + 140.0 * k, rgba(FOCUS_COL, look.aim * (1.0 - k)), 3.0 + 6.0 * (1.0 - k));
        }
        if let Some(b) = look.burst.filter(|_| boxed) {
            c.rect(0.0, lane_top, w, LANE_H, rgba(WHITE, 0.22 * (1.0 - b).powi(4)));
            c.ring(HIT_X, LANE_Y, NOTE_R + 130.0 * b.sqrt(), rgba(FOCUS_COL, 0.9 * (1.0 - b).powi(2)), 2.0 + 8.0 * (1.0 - b));
        }

        // Whole-playfield moves: Mirror and Flip Flop turn it over, the rest move it about the
        // hit target (`pivot`), which lands on `dest`.
        let mirror = mods.has(Mod::Mirror);
        if mirror {
            c.mirror_since(start_shape, w);
        }
        let fx = if mods.has(Mod::FlipFlop) { flip_flop(&self.map.barlines, vt) } else { 1.0 };
        if fx != 1.0 {
            c.squash_x_since(start_shape, w / 2.0, fx);
        }
        let flip_x = |x: f32| w / 2.0 + ((if mirror { w - x } else { x }) - w / 2.0) * fx;
        let pivot = [flip_x(HIT_X), LANE_Y];
        let (mut angle, mut zoom, mut dx, mut dy) = (0.0f32, rail_zoom, 0.0f32, rail_drop);
        let beat_no = beats.floor() as f64;
        let mut glitch = false;
        if party {
            // Sways, nods on the beat, shakes after big hits, pulses and glitches.
            dy += 14.0 * (t / 650.0).sin() as f32 + 10.0 * pulse;
            dx += 6.0 * (t / 1100.0).cos() as f32;
            let age = (t - self.quake_at) as f32;
            if (0.0..260.0).contains(&age) {
                let amp = 7.0 * (1.0 - age / 260.0);
                dx += amp * (age * 0.9).sin();
                dy += amp * (age * 1.3).cos();
            }
            zoom *= 1.0 + 0.05 * pulse;
            glitch = beat_phase < 0.18 && hash01(beat_no * 3.1 + 0.7) < 0.35;
            if glitch {
                dx += (hash01(beat_no * 5.3 + 0.1) as f32 - 0.5) * 44.0;
                dy += (hash01(beat_no * 6.1 + 0.2) as f32 - 0.5) * 12.0;
            }
        }
        if mods.has(Mod::Tilt) {
            angle += 0.11 * (PI * beats).cos();
        }
        if mods.has(Mod::Spin) {
            angle += TAU * (vt / 24_000.0).rem_euclid(1.0) as f32;
        }
        if mods.has(Mod::BarrelRoll) {
            let i = self.kiai_starts.partition_point(|&(k, _)| k <= vt);
            if let Some(&(k, b)) = i.checked_sub(1).and_then(|i| self.kiai_starts.get(i)) {
                if vt - k < 2.0 * b {
                    angle += TAU * smooth(0.0, 2.0 * b, vt - k) as f32;
                }
            }
        }
        let mut dest = pivot;
        if crossfire {
            dest = [w / 2.0, LANE_Y];
        }
        if mods.has(Mod::Spin) {
            dest = [w / 2.0, 240.0];
        }
        if mods.has(Mod::Mania) {
            // Upright, the drum at the bottom (clear of the hit error meter).
            angle -= PI / 2.0;
            zoom *= 0.85;
            dest = [w / 2.0, 290.0];
        }
        let dest = [dest[0] + dx, dest[1] + dy];
        if angle != 0.0 || zoom != 1.0 || dest != pivot {
            c.rotate_since(start_shape, pivot, angle, zoom, dest);
        }
        let [x0, y0, x1, y1] = local_zone;
        let (x0, x1) = (flip_x(x0).min(flip_x(x1)), flip_x(x0).max(flip_x(x1)));
        self.zone = Some(turned_box([x0, y0, x1 - x0, y1 - y0], pivot, angle, zoom, dest));
        // Texts go where the playfield took them, but stay upright and readable: their centre x,
        // top and height there.
        let (sin, cos) = angle.sin_cos();
        let moved = |x: f32, y: f32, size: f32| {
            let (rx, ry) = ((flip_x(x) - pivot[0]) * zoom, (y + size / 2.0 - pivot[1]) * zoom);
            (dest[0] + rx * cos - ry * sin, dest[1] + rx * sin + ry * cos - size * zoom / 2.0, size * zoom)
        };
        for (x, y, size, color, what) in upright {
            let (x, y, size) = moved(x, y, size);
            match what {
                Upright::Number(font, s) => {
                    if let Some(nw) = c.number_width(font, size, &s) {
                        c.number(font, x - nw / 2.0, y, size, color, &s);
                    }
                }
                Upright::Image(sprite) => {
                    if let Some([sw, sh]) = c.sprite_size(sprite) {
                        c.sprite_box(sprite, x, y + size / 2.0, size / 2.0 * sw / sh, size / 2.0, color);
                    }
                }
            }
        }
        for (x, y, size, color, text) in texts {
            let (x, y, size) = moved(x, y, size);
            c.text_centered(x, y, size, color, &text);
        }
        // Glitch: colour-split ghosts of everything but the big backdrops.
        if glitch {
            let end = c.shapes.len();
            for (tint, off) in [([1.0, 0.15, 0.3], -5.0), ([0.1, 0.9, 1.0], 5.0)] {
                for i in start_shape..end {
                    let mut sh = c.shapes[i];
                    if sh.half[0] > 150.0 {
                        continue;
                    }
                    sh.center[0] += off;
                    sh.fill = [tint[0], tint[1], tint[2], sh.fill[3] * 0.3];
                    sh.border = [tint[0], tint[1], tint[2], sh.border[3] * 0.3];
                    c.shapes.push(sh);
                }
            }
        }
        // Echo's bar stays where it is, whatever the playfield does.
        if mods.has(Mod::Echo) && !self.hide_notes {
            self.draw_echo(c, vt, w, uni);
        }
    }

    /// Echo: the bar map time `t` is in and the one after it (start, length each). Before the
    /// first barline, the bar leading up to it.
    fn echo_bars(&self, t: f64) -> Option<[(f64, f64); 2]> {
        let bars = &self.map.barlines;
        let fallback = 4.0 * beatmap::control_point_at(&self.map.control_points, t).beat_length;
        let len_of = |i: usize| bars.get(i + 1).map_or(fallback, |b| b.time - bars[i].time).max(1.0);
        match bar_at(bars, t, fallback) {
            Some((i, start, len)) => Some([(start, len), (start + len, if i + 1 < bars.len() { len_of(i + 1) } else { fallback })]),
            None => {
                let first = bars.first()?.time;
                let len = len_of(0);
                Some([(first - len, len), (first, len)])
            }
        }
    }

    /// Echo: the next bar's notes in colour on a line under the lane, and a cursor running along
    /// it with the bar being played. Each note rings as the cursor reaches it, a bar before it's
    /// hit; the line fades over to the next bar on the barline.
    fn draw_echo(&self, c: &mut Canvas, vt: f64, w: f32, uni: &Universe) {
        let Some([(start, len), (next, next_len)]) = self.echo_bars(vt) else { return };
        let p = ((vt - start) / len).clamp(0.0, 1.0);
        let fade = (smooth(0.0, 0.05, p) * (1.0 - smooth(0.94, 1.0, p))) as f32;
        let p = p as f32;
        let (x0, x1) = (HIT_X, w - 120.0);
        let x_at = |f: f32| x0 + (x1 - x0) * f;
        let line = rgba(0x4a4a60, fade);
        c.rect(x0, ECHO_Y - 0.75, x1 - x0, 1.5, line);
        for x in [x0, x1] {
            c.rect(x - 0.75, ECHO_Y - 5.0, 1.5, 10.0, line);
        }
        let notes = &self.map.notes;
        let from = notes.partition_point(|n| n.time < next - 1.0);
        for n in notes[from..].iter().take_while(|n| n.time < next + next_len - 1.0) {
            let col = match n.kind {
                Kind::Don => uni.don(),
                Kind::Kat => uni.kat(),
                _ => continue,
            };
            let f = ((n.time - next) / next_len) as f32;
            let (x, r) = (x_at(f), if n.big { 8.5 } else { 5.5 });
            // Ms since the cursor got to it.
            let age = (p - f) * len as f32;
            if age >= 0.0 {
                c.circle(x, ECHO_Y, r + 0.5, rgba(col, fade), rgba(WHITE, fade), 1.5);
                if age < ECHO_RING_MS {
                    let k = age / ECHO_RING_MS;
                    c.ring(x, ECHO_Y, r + 3.0 + 10.0 * k, rgba(col, 0.8 * (1.0 - k) * fade), 1.0 + 2.0 * (1.0 - k));
                }
            } else {
                c.circle(x, ECHO_Y, r, rgba(col, 0.6 * fade), rgba(col, 0.0), 0.0);
            }
        }
        c.rect(x_at(p) - 1.0, ECHO_Y - 11.0, 2.0, 22.0, rgba(WHITE, 0.85 * fade));
    }

    /// The kiai flash at map time `t`, taiko_arranger's rule: 1 on each beat counted from the
    /// kiai's start, fading linearly to 0 by the next. The beat is the last real one (gimmick
    /// lines under `KIAI_FLASH_MIN_BEAT_MS` are skipped). Past the kiai's end only a beat over a
    /// second long finishes fading; a faster one just stops, as a chorus ending should.
    fn kiai_flash(&self, t: f64) -> f32 {
        let i = self.kiai_bands.partition_point(|&(start, _)| start <= t);
        let Some(&(start, end)) = i.checked_sub(1).and_then(|i| self.kiai_bands.get(i)) else { return 0.0 };
        let cps = &self.map.control_points;
        let beat_at = |t: f64| cps[..cps.partition_point(|c| c.time <= t)].iter().rev().map(|c| c.beat_length).find(|&b| b >= KIAI_FLASH_MIN_BEAT_MS);
        if t >= end {
            let Some(b) = beat_at(start).filter(|&b| b > 1000.0) else { return 0.0 };
            if t >= start + ((end - start) / b).ceil() * b {
                return 0.0;
            }
        }
        let Some(beat) = beat_at(t) else { return 0.0 };
        1.0 - ((t - start) / beat).rem_euclid(1.0) as f32
    }

    /// Sets `visual_offset`, except while paused: the vsync lead in it moves a little every frame,
    /// and on a frozen clock that shook the stopped lane.
    pub fn set_visual_offset(&mut self, ms: f64) {
        if self.paused.is_none() {
            self.visual_offset = ms;
        }
    }

    /// Map time the lane is drawn at: `time`, `visual_offset` real ms ahead.
    fn visual_time(&self) -> f64 {
        if self.visual_offset == 0.0 { self.time } else { self.speed.map(self.raw_time - self.offset + self.visual_offset) }
    }

    /// Score, accuracy, progress, hit error meter, offset advice, skip hint and the pause menu (with
    /// the map's name), at fixed screen positions: resizing the playfield area never moves them.
    pub fn draw_hud(&self, c: &mut Canvas) {
        let t = self.time;
        let w = c.width;
        if let Some(text) = &self.banner {
            let tw = Canvas::text_width(9.0, &text);
            c.round_rect(w / 2.0 - tw / 2.0 - 12.0, 14.0, tw + 24.0, 20.0, 10.0, rgba(0x173049, 0.9), rgba(0x4fb4ff, 1.0), 1.0);
            c.text_centered(w / 2.0, 19.5, 9.0, rgba(WHITE, 1.0), text);
        }

        let shown = |p: Part| !self.hidden.has(p);
        // Score / accuracy.
        // In the skin's score font if it has one.
        let number = |c: &mut Canvas, y: f32, size: f32, col: u32, s: &str| match c.number_width(Font::Score, size, s) {
            Some(nw) => {
                c.number(Font::Score, w - 18.0 - nw, y, size, rgba(WHITE, 1.0), s);
            }
            None => c.text_right(w - 18.0, y, size, rgba(col, 1.0), s),
        };
        if shown(Part::Score) {
            number(c, 18.0, 24.0, WHITE, &format!("{:08}", self.stats.score));
        }
        let acc = format!("{:.2}%", self.stats.accuracy() * 100.0);
        if shown(Part::Accuracy) {
            number(c, 50.0, 14.0, 0xc8c8d8, &acc);
        }
        // The pp counter, on the accuracy's line, left of it.
        if let Some(pp) = self.live_pp_value.filter(|_| shown(Part::Pp)) {
            let acc_w = if shown(Part::Accuracy) { c.number_width(Font::Score, 14.0, &acc).unwrap_or_else(|| Canvas::text_width(14.0, &acc)) + 14.0 } else { 0.0 };
            let text = format!("{pp:.0}pp");
            let right = w - 18.0 - acc_w;
            match c.number_width(Font::Score, 14.0, &text) {
                Some(nw) => {
                    c.number(Font::Score, right - nw, 50.0, 14.0, rgba(0xff8ad8, 1.0), &text);
                }
                None => c.text_right(right, 50.0, 14.0, rgba(0xff8ad8, 1.0), &text),
            }
        }
        // Progress bar.
        if shown(Part::Progress) {
            c.rect(0.0, 0.0, w * self.progress() as f32, 3.0, rgba(WHITE, 0.45));
        }
        // HP: has to be at least half full at the end (the mark), like osu!taiko.
        if self.show_hp && shown(Part::Health) {
            // Under the score, accuracy and mods (the top-left corner is the FPS overlay's).
            let (hx, hy) = (hp_bar_x(w), HP_BAR_Y);
            let hw = w - 18.0 - hx;
            let hp = self.hp as f32;
            if let (Some([bw, bh]), Some([fw, fh])) = (c.sprite_size(Sprite::HealthBg), c.sprite_size(Sprite::HealthFill)) {
                // The skin's bar where osu! has it, in the top left at its own size (some are a
                // frame round the whole screen); the fill on it 5, 16 px in, cut off at the HP.
                let k = HUD_PX * 128.0;
                let (bg_w, bg_h) = (bw * k, bh * k);
                c.sprite_box(Sprite::HealthBg, bg_w / 2.0, bg_h / 2.0, bg_w / 2.0, bg_h / 2.0, rgba(WHITE, 1.0));
                let (fx, fy, fill_w, fill_h) = (5.0 * HUD_PX, 16.0 * HUD_PX, fw * k, fh * k);
                let start = c.shapes.len();
                c.sprite_box(Sprite::HealthFill, fx + fill_w / 2.0, fy + fill_h / 2.0, fill_w / 2.0, fill_h / 2.0, rgba(WHITE, 1.0));
                c.clip_since(start, fx, fy, fill_w * hp, fill_h);
                c.rect(fx + fill_w / 2.0 - 0.75, fy - 2.0, 1.5, fill_h + 4.0, rgba(WHITE, 0.8));
            } else {
                c.round_rect(hx, hy, hw, 8.0, 3.0, rgba(0x15151d, 0.85), rgba(0x3a3a4e, 1.0), 1.0);
                let fill = if hp >= 0.5 { 0xFFD34D } else { 0xEB452C };
                if hp > 0.0 {
                    c.round_rect(hx + 1.0, hy + 1.0, (hw - 2.0) * hp, 6.0, 2.5, rgba(fill, 0.95), rgba(0, 0.0), 0.0);
                }
                c.rect(hx + hw / 2.0 - 0.75, hy - 2.0, 1.5, 12.0, rgba(WHITE, 0.8));
            }
        }
        if !self.mods.is_empty() && shown(Part::Mods) {
            let label = self.mods.iter().map(|m| m.acronym()).collect::<Vec<_>>().join(" ");
            c.text_right(w - 18.0, 72.0, 9.0, rgba(0xc8c8d8, 0.9), &label);
        }

        // Hit error meter.
        let cx = w / 2.0;
        let my = 430.0;
        if shown(Part::HitError) {
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
        }
        if self.show_offset_hint
            && shown(Part::OffsetHint)
            && let Some(h) = self.offset_suggestion(self.offset)
        {
            let e = &h.estimate;
            let building = if e.hits < h.needed { format!("  confidence {}/{}", e.hits, h.needed) } else { String::new() };
            let stats = format!("UR {:.0}, {} hits", e.ur, e.hits);
            if h.significant {
                let dir = if e.error_ms > 0.0 { "late" } else { "early" };
                let text = format!(
                    "{:.1} +-{:.1} ms {dir} ({stats})  |  suggested offset {:+.0} ms  |  O: this map  Shift+O: all{building}",
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

        // Overtime: he's here, over everything but the pause menu.
        if let Some(&at) = self.scares.iter().find(|&&at| (at..at + SCARE_MS).contains(&t)) {
            draw_scare(c, ((t - at) / SCARE_MS) as f32, t);
        }

        if self.can_skip() && self.paused.is_none() && !self.mute_hits && !self.watched {
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
                c.circle(cx + a.cos() * 26.0, cy + a.sin() * 26.0, 2.2, rgba(accent(), 1.0), rgba(accent(), 1.0), 0.0);
            }
            c.text_centered(cx, cy - 7.0, 14.0, rgba(WHITE, 1.0), &format!("{:.1}", left));
            c.text_centered(cx, cy + 36.0, 8.0, rgba(0x9a9ab0, 1.0), "get ready   (Esc: back to menu)");
        } else if let Some(cursor) = self.paused.filter(|_| !self.watched) {
            let t = progress(self.paused_at, 180.0);
            c.rect(0.0, 0.0, w, 480.0, rgba(0x0b0b10, 0.78 * t));
            let rise = (1.0 - t) * 12.0;
            c.text_centered(w / 2.0, 98.0 - rise, 32.0, rgba(WHITE, t), "PAUSED");
            // The map's name lives here rather than under the lane, where moved lanes ran into it.
            let m = &self.map.meta;
            let rate = if (self.rate - 1.0).abs() > 1e-6 { format!("   {:.2}x", self.rate) } else { String::new() };
            let fit = (w - 80.0).min(520.0);
            c.text_centered(w / 2.0, 140.0 - rise, 9.0, rgba(0x9a9ab0, t), &Canvas::fit(9.0, fit, &format!("{} - {}", m.artist, m.title)));
            c.text_centered(w / 2.0, 153.0 - rise, 9.0, rgba(0x6f6f86, t), &Canvas::fit(9.0, fit, &format!("[{}]  OD {:.1}{rate}", m.version, m.od)));
            for (i, item) in PAUSE_ITEMS.iter().enumerate() {
                // Items slide up one after another.
                let ti = progress(self.paused_at + std::time::Duration::from_millis(25 * i as u64), 180.0);
                let y = 170.0 + i as f32 * 38.0 + (1.0 - ti) * 18.0;
                let selected = i == cursor;
                let (bg, border) = if selected { (0x34344a, accent()) } else { (0x1d1d27, 0x2a2a38) };
                c.round_rect(w / 2.0 - 140.0, y, 280.0, 32.0, 8.0, rgba(bg, 1.0), rgba(border, 1.0), if selected { 2.0 } else { 1.0 });
                c.hit(w / 2.0 - 140.0, y, 280.0, 32.0, Hit::PauseItem(i));
                c.text_centered(w / 2.0, y + 10.0, 12.0, rgba(if selected { WHITE } else { 0xc8c8d8 }, 1.0), item);
            }
            let hint_y = 170.0 + PAUSE_ITEMS.len() as f32 * 38.0 + 8.0;
            // Fades in once the items have slid past it.
            let th = progress(self.paused_at + std::time::Duration::from_millis(220), 150.0);
            c.text_centered(w / 2.0, hint_y, 9.0, rgba(0x8a8aa0, th), "Up/Down + Enter    Esc: continue");
        }
        if let Some((_, at)) = self.failed {
            let p = progress(at, 400.0);
            c.rect(0.0, 0.0, w, 480.0, rgba(0x0b0b10, 0.6 * p));
            c.text_centered(w / 2.0, 200.0, 32.0, rgba(0xEB452C, p), "FAILED");
        }
    }
}

/// Overtime's animatronic bear lunging at the screen, `p` (0-1) of the way through, shaking
/// with map time `t`. Drawn from shapes, centred on a canvas `c.width` wide.
/// The HP bar's top and left end (it is 8 tall and ends 18 from the right).
const HP_BAR_Y: f32 = 88.0;
fn hp_bar_x(w: f32) -> f32 {
    w - 18.0 - (w * 0.25).min(180.0)
}

fn draw_scare(c: &mut Canvas, p: f32, t: f64) {
    let w = c.width;
    let start = c.shapes.len();
    // Lunges in over the first 12%, then holds, shaking, and flickers out at the end.
    let k = 0.55 + 0.75 * (p / 0.12).min(1.0).powf(0.5);
    let shake = 14.0 * (1.0 - p * 0.5);
    let (sx, sy) = (shake * (t as f32 * 0.37).sin(), shake * (t as f32 * 0.53).cos());
    let flicker = if p > 0.85 && ((t / 40.0) as i64) % 2 == 0 { 0.4 } else { 1.0 };
    c.rect(0.0, 0.0, w, VIRTUAL_HEIGHT, rgba(0x050000, 0.92 * flicker));
    let (fur, dark, snout) = (0x5a3418, 0x3a2010, 0xb98552);
    // Ears, head, hat.
    for side in [-1.0, 1.0] {
        c.circle(side * 125.0, -130.0, 52.0, rgba(fur, 1.0), rgba(dark, 1.0), 6.0);
        c.circle(side * 125.0, -130.0, 26.0, rgba(dark, 1.0), rgba(dark, 1.0), 0.0);
    }
    c.circle(0.0, 0.0, 165.0, rgba(fur, 1.0), rgba(dark, 1.0), 8.0);
    c.rect(-95.0, -200.0, 190.0, 16.0, rgba(0x0b0b0b, 1.0));
    c.rect(-60.0, -268.0, 120.0, 72.0, rgba(0x0b0b0b, 1.0));
    // Eye sockets, black, with tiny white pupils.
    for side in [-1.0, 1.0] {
        c.circle(side * 62.0, -52.0, 40.0, rgba(0x000000, 1.0), rgba(dark, 1.0), 4.0);
        c.glow(side * 62.0, -50.0, 5.0, 10.0, rgba(0xffffff, 0.9));
        c.circle(side * 62.0, -50.0, 5.0, rgba(0xffffff, 1.0), rgba(0xffffff, 1.0), 0.0);
    }
    // Snout, nose, and the jaw hanging open, teeth top and bottom.
    c.round_rect(-95.0, -5.0, 190.0, 95.0, 45.0, rgba(snout, 1.0), rgba(dark, 1.0), 4.0);
    c.round_rect(-26.0, -8.0, 52.0, 30.0, 12.0, rgba(0x0b0b0b, 1.0), rgba(0x0b0b0b, 1.0), 0.0);
    c.round_rect(-85.0, 55.0, 170.0, 110.0, 30.0, rgba(0x1a0404, 1.0), rgba(dark, 1.0), 5.0);
    for i in 0..7 {
        let x = -72.0 + i as f32 * 21.0;
        c.rect(x, 58.0, 15.0, 22.0, rgba(0xe8e0c8, 1.0));
        c.rect(x, 140.0, 15.0, 22.0, rgba(0xe8e0c8, 1.0));
    }
    // Bow tie under the chin.
    for side in [-1.0, 1.0] {
        c.round_rect(side * 34.0 - 30.0, 168.0, 60.0, 34.0, 8.0, rgba(0x0b0b0b, 1.0), rgba(0x0b0b0b, 1.0), 0.0);
    }
    c.circle(0.0, 185.0, 12.0, rgba(0x0b0b0b, 1.0), rgba(0x0b0b0b, 1.0), 0.0);
    // Everything but the backdrop was drawn about (0, 0): scale and shake it into place.
    c.transform_since(start + 1, [0.0, 0.0], k, w / 2.0 + sx, 230.0 + sy);
    for sh in &mut c.shapes[start..] {
        sh.fill[3] *= flicker;
        sh.border[3] *= flicker;
    }
}

/// Flashlight-style darkness everywhere outside a soft-edged circle of radius `r` at (`x`, `y`)
/// on a playfield `w` wide.
fn darken_outside(c: &mut Canvas, x: f32, y: f32, r: f32, w: f32) {
    let far = w + 480.0;
    // Solid beyond the soft edge, then a few fainter rings fading into the lit circle.
    c.ring(x, y, far, rgba(0x08080b, 0.97), far - (r + 18.0));
    for k in 0..6 {
        let inner = r + k as f32 * 3.0;
        c.ring(x, y, inner + 3.0, rgba(0x08080b, 0.97 * (k as f32 + 0.5) / 6.0), 3.0);
    }
}

/// The box (x, y, w, h) around `rect` turned by `angle` and scaled by `scale` about `pivot`,
/// which lands on `dest` (as `Canvas::rotate_since` moves shapes).
fn turned_box(rect: [f32; 4], pivot: [f32; 2], angle: f32, scale: f32, dest: [f32; 2]) -> [f32; 4] {
    let (sin, cos) = angle.sin_cos();
    let [x, y, w, h] = rect;
    let corners = [(x, y), (x + w, y), (x, y + h), (x + w, y + h)].map(|(px, py)| {
        let (dx, dy) = ((px - pivot[0]) * scale, (py - pivot[1]) * scale);
        (dest[0] + dx * cos - dy * sin, dest[1] + dx * sin + dy * cos)
    });
    let (x0, x1) = corners.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (y0, y1) = corners.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
    [x0, y0, x1 - x0, y1 - y0]
}

/// Vertical band (top, height) of the lane with a little room for judgements, for previews that
/// show only the lane.
pub fn lane_band() -> (f32, f32) {
    (LANE_Y - LANE_H / 2.0 - 34.0, LANE_H + 42.0)
}

/// A don or kat drawn with the skin's images: the circle tinted `col`, then its overlay. False
/// (nothing drawn) when the skin has no circle for it.
fn skin_note(c: &mut Canvas, x: f32, y: f32, r: f32, big: bool, col: u32, alpha: f32) -> bool {
    let (circle, overlay) = if big { (Sprite::BigCircle, Sprite::BigCircleOverlay) } else { (Sprite::HitCircle, Sprite::HitCircleOverlay) };
    if !c.sprite(circle, x, y, r, rgba(col, alpha)) {
        return false;
    }
    c.sprite(overlay, x, y, r, rgba(WHITE, alpha));
    true
}

/// Units per sprite size unit (128 SD px) of osu!'s playfield images: its bar is 200 px tall.
const LANE_PX: f32 = LANE_H / 200.0 * 128.0;

/// Units per SD px of osu!'s interface images: its screen is 768 px tall.
const HUD_PX: f32 = VIRTUAL_HEIGHT / 768.0;

/// Drawn upright after the playfield's moves, like text.
enum Upright {
    /// A number in the skin's font.
    Number(Font, String),
    /// A skin image.
    Image(Sprite),
}

/// The skin's lane over `x`..`x + w` (stretched across, like osu!), its kiai glow over it at
/// `glow`. False (nothing drawn) when the skin has none.
fn skin_lane(c: &mut Canvas, x: f32, w: f32, tint: [f32; 4], glow: f32) -> bool {
    if c.sprite_size(Sprite::LaneBar).is_none() {
        return false;
    }
    c.sprite_box(Sprite::LaneBar, x + w / 2.0, LANE_Y, w / 2.0, LANE_H / 2.0, tint);
    if glow > 0.0 && c.sprite_size(Sprite::LaneKiai).is_some() {
        c.sprite_box(Sprite::LaneKiai, x + w / 2.0, LANE_Y, w / 2.0, LANE_H / 2.0, rgba(WHITE, glow));
    }
    true
}

/// The hit target: the skin's (osu!'s approach circle, and its big circle faintly round it, as
/// see-through as lazer has them) or the built-in rings.
fn draw_hit_target(c: &mut Canvas) {
    let big = c.sprite(Sprite::HitTargetBig, HIT_X, LANE_Y, BIG_R, rgba(WHITE, 0.22));
    let ring = c.sprite(Sprite::HitTarget, HIT_X, LANE_Y, NOTE_R, rgba(WHITE, 0.47));
    if !big && !ring {
        c.circle(HIT_X, LANE_Y, BIG_R, rgba(WHITE, 0.03), rgba(WHITE, 0.12), 2.0);
        c.circle(HIT_X, LANE_Y, NOTE_R + 1.0, rgba(WHITE, 0.06), rgba(WHITE, 0.4), 2.5);
    }
}

/// The drum panel and the drum, its halves lit by `flash` (left rim, left centre, right centre,
/// right rim: 0-1) in `don` / `kat` (the built-in drum; a skin's lights are its own). Returns
/// the drum's centre x.
fn draw_drum(c: &mut Canvas, flash: [f32; 4], don: u32, kat: u32) -> f32 {
    let lane_top = LANE_Y - LANE_H / 2.0;
    // The skin's panel at its own size (osu!'s is 181 x 200 SD px, but some reach out round the
    // hit target), as tall as the lane.
    let k = LANE_PX / 128.0;
    let panel = c.sprite_size(Sprite::DrumPanel);
    let skinned_panel = panel.is_some();
    if let Some([pw, _]) = panel {
        let hw = pw * LANE_PX / 2.0;
        c.sprite_box(Sprite::DrumPanel, hw, LANE_Y, hw, LANE_H / 2.0, rgba(WHITE, 1.0));
    } else {
        c.rect(0.0, lane_top, PANEL_W, LANE_H, rgba(0x191921, 1.0));
    }
    if c.sprite_size(Sprite::DrumInner).is_some() || c.sprite_size(Sprite::DrumOuter).is_some() {
        // Where each half's top-left goes (SD px into the panel): skins from before version 2.1
        // have smaller halves round a drum a little further left.
        let legacy = c.skin_layout.legacy_drum;
        let (centre, place) = if legacy {
            (85.5, [(Sprite::DrumOuterFlipped, 13.0, 37.0), (Sprite::DrumInner, 29.0, 50.0), (Sprite::DrumInnerFlipped, 86.0, 50.0), (Sprite::DrumOuter, 85.0, 37.0)])
        } else {
            (90.0, [(Sprite::DrumOuterFlipped, 0.0, 0.0), (Sprite::DrumInner, 0.0, 0.0), (Sprite::DrumInnerFlipped, 90.0, 0.0), (Sprite::DrumOuter, 90.0, 0.0)])
        };
        for ((sprite, x, y), a) in place.into_iter().zip(flash) {
            if let Some([sw, sh]) = c.sprite_size(sprite).filter(|_| a > 0.0) {
                let (hw, hh) = (sw * LANE_PX / 2.0, sh * LANE_PX / 2.0);
                c.sprite_box(sprite, x * k + hw, lane_top + y * k + hh, hw, hh, rgba(WHITE, a));
            }
        }
        return centre * k;
    }
    let drum_x = PANEL_W / 2.0;
    if skinned_panel {
        // The panel has its drum drawn on: light it the built-in way.
        for (side, r, col, a) in [(1, 46.0, kat, flash[0]), (2, 46.0, kat, flash[3]), (1, 32.0, don, flash[1]), (2, 32.0, don, flash[2])] {
            c.half_circle(drum_x, LANE_Y, r, side, rgba(col, a * 0.7), rgba(col, 0.0), 0.0);
        }
        return drum_x;
    }
    c.circle(drum_x, LANE_Y, 46.0, rgba(0x2f2f3e, 1.0), rgba(0x4a4a60, 1.0), 2.0);
    c.half_circle(drum_x, LANE_Y, 46.0, 1, rgba(kat, flash[0]), rgba(kat, 0.0), 0.0);
    c.half_circle(drum_x, LANE_Y, 46.0, 2, rgba(kat, flash[3]), rgba(kat, 0.0), 0.0);
    c.circle(drum_x, LANE_Y, 32.0, rgba(0xe9dcc3, 1.0), rgba(0x8a7b63, 1.0), 2.0);
    c.half_circle(drum_x, LANE_Y, 32.0, 1, rgba(don, flash[1]), rgba(don, 0.0), 0.0);
    c.half_circle(drum_x, LANE_Y, 32.0, 2, rgba(don, flash[2]), rgba(don, 0.0), 0.0);
    drum_x
}

/// The lane and its border. The border is two strips rather than a rect under the whole lane
/// (they reach a unit under the lane, so its edge blends into border colour just the same), and
/// the background isn't drawn under the lane at all (`mark_opaque`).
fn draw_lane(c: &mut Canvas, x: f32, w: f32, color: [f32; 4], border: [f32; 4]) {
    let lane_top = LANE_Y - LANE_H / 2.0;
    c.rect(x, lane_top - 3.0, w, 4.0, border);
    c.rect(x, lane_top + LANE_H - 1.0, w, 4.0, border);
    c.rect(x, lane_top, w, LANE_H, color);
    c.mark_opaque();
}

/// A still playfield (lane, drum, a few notes) for the playfield area editor.
pub fn draw_preview(c: &mut Canvas) {
    let w = c.width;
    if !skin_lane(c, 0.0, w, rgba(WHITE, 1.0), 0.0) {
        draw_lane(c, 0.0, w, rgba(0x20202b, 1.0), rgba(0x3a3a4e, 1.0));
    }
    draw_hit_target(c);
    // Back to front, like gameplay.
    c.capsule(HIT_X + 560.0, HIT_X + 760.0, LANE_Y, NOTE_R * 0.86, rgba(ROLL, 1.0), rgba(WHITE, 1.0), 2.5);
    c.circle(HIT_X + 560.0, LANE_Y, NOTE_R, rgba(ROLL, 1.0), rgba(WHITE, 1.0), NOTE_R * 0.1);
    let colours = multiverse::note_colours();
    for (dx, col, big) in [(400.0, colours.don(), true), (260.0, colours.kat(), false), (150.0, colours.don(), false)] {
        let r = if big { BIG_R } else { NOTE_R };
        if !skin_note(c, HIT_X + dx, LANE_Y, r, big, col, 1.0) {
            c.circle(HIT_X + dx, LANE_Y, r, rgba(col, 1.0), rgba(WHITE, 1.0), r * 0.1);
        }
    }
    draw_drum(c, [0.0; 4], colours.don(), colours.kat());
    c.text_centered(w / 2.0, 300.0, 14.0, rgba(WHITE, 0.85), "drag to move, edges to resize");
}

/// The editor's stand-in score, where the real HUD goes: fixed on screen, whatever the area.
pub fn draw_preview_hud(c: &mut Canvas) {
    let w = c.width;
    c.text_right(w - 18.0, 18.0, 24.0, rgba(WHITE, 1.0), "00000000");
    c.text_right(w - 18.0, 50.0, 14.0, rgba(0xc8c8d8, 1.0), "100.00%");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> Game {
        let map = beatmap::calibration_map(500.0, 0, 64);
        Game::new(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None)
    }

    #[test]
    fn notes_stay_on_screen_as_long_as_in_osu() {
        // A 180 BPM map at SV 1 and SliderMultiplier 1.4 (taiko's 1.4x on top).
        let v = 1.4 * 1.4 * 100.0 / (60_000.0 / 180.0);
        for (aspect, osu_aspect) in [(16.0 / 9.0, 16.0 / 9.0), (4.0 / 3.0, 4.0 / 3.0), (21.0 / 9.0, 16.0 / 9.0)] {
            let w = aspect as f32 * VIRTUAL_HEIGHT;
            let ours = (w - HIT_X) as f64 / (osu_speed_scale(w) * v);
            let osu = (osu_aspect * 480.0 - 160.0) / v;
            assert!((ours - osu).abs() < 0.01, "{aspect}: {ours} vs {osu}");
        }
    }

    fn modded(mods: &str) -> Game {
        let map = beatmap::calibration_map(500.0, 0, 64);
        Game::with_mods(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None, Mods::parse(mods))
    }

    #[test]
    fn random_colours_each_play_its_own_way_and_replays_the_same() {
        let colours = |seed: u64| {
            let g = Game::with_seed(beatmap::calibration_map(500.0, 0, 64), Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None, Mods::parse("RD"), seed);
            g.map.notes.iter().map(|n| n.kind == Kind::Don).collect::<Vec<_>>()
        };
        // Seed 0 is how every play of a map looked before seeds (replays from then).
        let before: Vec<bool> = modded("RD").map.notes.iter().map(|n| hash01(n.time + 0.5) < 0.5).collect();
        assert_eq!(colours(0), before);
        assert_eq!(colours(12345), colours(12345));
        let plays: Vec<_> = [1u64, 2, 777, 1 << 40].into_iter().map(colours).collect();
        for (i, a) in plays.iter().enumerate() {
            assert!(plays[i + 1..].iter().all(|b| b != a), "seeds give different colourings");
            let dons = a.iter().filter(|&&d| d).count();
            assert!((16..=48).contains(&dons), "about half dons: {dons} of 64");
        }
    }

    /// Hits every note `err` ms off (centre key, so dons), then lets the map end.
    fn play_all(g: &mut Game, err: f64) {
        let times: Vec<f64> = g.map.notes.iter().map(|n| n.time).collect();
        for t in times {
            g.press_at(1, t + err, None);
        }
        let end = g.end_time + 2000.0;
        g.set_time(end);
    }

    #[test]
    fn play_time_leaves_out_pauses_and_the_resume_countdown() {
        let mut g = game();
        let t0 = Instant::now();
        let at = |ms: u64| t0 + std::time::Duration::from_millis(ms);
        g.update(at(0), None, 0.0);
        g.update(at(1000), None, 0.0);
        g.update(at(1500), None, 0.0);
        assert!((g.played_ms - 1500.0).abs() < 1.0, "{}", g.played_ms);
        g.paused = Some(0);
        g.update(at(9000), None, 0.0);
        // Counting down to resume doesn't count either.
        g.paused = None;
        g.resuming = Some((at(9000), at(12_000)));
        g.update(at(11_000), None, 0.0);
        g.resuming = None;
        g.update(at(12_000), None, 0.0);
        g.update(at(12_500), None, 0.0);
        assert!((g.played_ms - 2000.0).abs() < 1.0, "{}", g.played_ms);
        // A stall counts as a second at most.
        g.update(at(60_000), None, 0.0);
        assert!((g.played_ms - 3000.0).abs() < 1.0, "{}", g.played_ms);
    }

    #[test]
    fn alternate_lets_either_hand_follow_finishers_drumrolls_and_swells() {
        // Dons 500 ms apart, with a finisher at 1000, a drumroll from 2500 to 3500 and a swell
        // from 5000 to 6000.
        let mut map = beatmap::calibration_map(500.0, 0, 15);
        let base = map.notes[0].clone();
        let at = |t: f64| beatmap::Note { time: t, end_time: t, ..base.clone() };
        map.notes = vec![
            at(0.0),
            at(500.0),
            beatmap::Note { big: true, ..at(1000.0) },
            at(1500.0),
            at(2000.0),
            beatmap::Note { kind: Kind::Roll, end_time: 3500.0, tick_spacing: 125.0, ..at(2500.0) },
            at(4000.0),
            at(4500.0),
            beatmap::Note { kind: Kind::Swell, end_time: 6000.0, required_hits: 6, ..at(5000.0) },
            at(6500.0),
        ];
        let mut g = Game::with_mods(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None, Mods::parse("AL"));
        let (l, r) = (1, 2); // left and right centre
        g.press_time(l, 0.0, None);
        g.press_time(r, 500.0, None);
        // The finisher with the left hand only, then the left hand again.
        g.press_time(l, 1000.0, None);
        g.press_time(l, 1500.0, None);
        // Back to alternating: a second left in a row is ignored, the right then hits.
        g.press_time(l, 1990.0, None);
        g.press_time(r, 2000.0, None);
        // The drumroll mashed with the left hand, then the left hand again after it.
        for k in 0..=8 {
            g.press_time(l, 2500.0 + 125.0 * k as f64, None);
        }
        g.press_time(l, 4000.0, None);
        g.press_time(l, 4490.0, None);
        g.press_time(r, 4500.0, None);
        // The swell with the left hand alone (centre, rim, centre...), then the left again.
        for k in 0..6 {
            g.press_time(if k % 2 == 0 { l } else { 0 }, 5100.0 + 100.0 * k as f64, None);
        }
        g.press_time(l, 6500.0, None);
        g.set_time(9000.0);
        let s = &g.stats;
        assert_eq!((s.great, s.ok, s.miss), (8, 0, 0), "every don hit");
        assert_eq!((s.roll_ticks, s.swells), (9, 1));
        // Without the finisher, drumroll and swell rule, the same hand twice is still out.
        let mut g = Game::with_mods(beatmap::calibration_map(500.0, 0, 4), Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None, Mods::parse("AL"));
        g.press_time(l, 0.0, None);
        g.press_time(l, 500.0, None);
        g.set_time(9000.0);
        assert_eq!((g.stats.great, g.stats.miss), (1, 3));
    }

    #[test]
    fn pulse_and_stutter_never_pull_notes_back_on_sv_heavy_maps() {
        // A red line, then green lines off the beat every 130 ms (as on SV-heavy maps).
        let red = beatmap::ControlPoint { time: 0.0, beat_origin: 0.0, beat_length: 500.0, sv: 1.0, kiai: false, volume: 1.0 };
        let cps: Vec<_> = std::iter::once(red).chain((0..150).map(|k| beatmap::ControlPoint { time: 65.0 + 130.0 * k as f64, ..red })).collect();
        for mods in ["PU", "ST"] {
            let mods = Mods::parse(mods);
            let mut last = f32::INFINITY;
            let mut t = 6000.0;
            while t < 10_000.0 {
                let warp = beat_warp(mods, beatmap::control_point_at(&cps, t), t);
                let s = Scroll { t, speed: osu_speed_scale(640.0 + HIT_X), lane: 640.0, mods, warp, live: 1.0 };
                let x = s.x(10_000.0, 0.5);
                assert!(x <= last + 1e-3, "{mods:?} at {t}: {last} -> {x}");
                last = x;
                t += 2.0;
            }
        }
    }

    #[test]
    fn multiverse_breathes_without_a_jump_on_the_beat() {
        // From 300 BPM to 60 BPM, a 60 fps frame never moves the breath more than a third of
        // the way (it used to jump all the way on every beat).
        for beat in [200.0, 300.0, 500.0, 1000.0] {
            let mut last = breath(0.0, beat);
            let mut t = 16.7;
            while t < 4.0 * beat {
                let b = breath((t / beat).rem_euclid(1.0), beat);
                assert!((0.0..=1.0).contains(&b));
                assert!((b - last).abs() < 0.34, "beat {beat} ms at {t}: {last} -> {b}");
                last = b;
                t += 16.7;
            }
        }
    }

    #[test]
    fn visual_offset_moves_the_lane_not_the_judging() {
        for rate in [1.0, 1.5] {
            let map = beatmap::calibration_map(500.0, 0, 64);
            let mut g = Game::new(map, Arc::new(Track { samples: Vec::new(), rate }), None);
            g.visual_offset = 30.0;
            g.set_time(2000.0);
            // Real milliseconds: at 1.5x, 30 ms of song is 45 ms of map.
            assert!((g.visual_time() - (2000.0 + 30.0 * rate)).abs() < 1e-6, "{rate}x: {}", g.visual_time());
        }
        let mut g = modded("");
        g.visual_offset = 30.0;
        play_all(&mut g, 0.0);
        assert!(g.stats.great == 64 && g.stats.mean_error().abs() < 1e-9);
    }

    #[test]
    fn a_paused_lane_holds_still_however_the_vsync_lead_moves() {
        let mut g = Game::new(beatmap::calibration_map(500.0, 0, 64), Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None);
        g.set_visual_offset(30.0);
        g.set_time(2000.0);
        g.pause(None);
        let frozen = g.visual_time();
        for lead in [30.4, 29.7, 31.2] {
            g.set_visual_offset(lead);
            assert_eq!(g.visual_time(), frozen);
        }
        g.resume(None);
        g.set_visual_offset(31.2);
        assert!((g.visual_time() - (2000.0 + 31.2)).abs() < 1e-6);
    }

    #[test]
    fn hp_passes_a_clean_play_and_fails_an_empty_one() {
        let mut g = modded("");
        play_all(&mut g, 0.0);
        assert!(g.hp >= 0.99 && !g.failed());
        // Nothing hit: HP never fills.
        let mut g = modded("");
        g.set_time(g.end_time + 2000.0);
        assert!(g.stats.miss == 64 && g.failed());
        // No Fail passes anyway.
        let mut g = modded("NF");
        g.set_time(g.end_time + 2000.0);
        assert!(!g.failed());
    }

    #[test]
    fn sudden_death_and_perfect_fail_early() {
        let mut g = modded("SD");
        let first = g.map.notes[0].time;
        g.press_at(1, first + 80.0, None); // an OK: fine for SD
        assert!(!g.failed_early());
        g.set_time(g.map.notes[2].time + 500.0); // lets a note pass
        assert!(g.failed_early());
        // Nothing after the failing miss counts.
        assert_eq!(g.stats.miss, 1);
        let mut g = modded("PF");
        g.press_at(1, first + 80.0, None);
        assert!(g.failed_early());
    }

    #[test]
    fn note_and_judging_mods() {
        // Great or Nothing: an OK is a miss.
        let mut g = modded("GN");
        g.press_at(1, g.map.notes[0].time + 80.0, None);
        assert_eq!((g.stats.ok, g.stats.miss), (0, 1));
        // Swap turns the dons into kats; Half keeps every other one.
        assert!(modded("SW").map.notes.iter().all(|n| n.kind == Kind::Kat));
        assert_eq!(modded("HA").map.notes.len(), 32);
        assert!(modded("AB").map.notes.iter().all(|n| n.big));
        // Random: a mix, the same every time.
        let kinds = |g: &Game| g.map.notes.iter().map(|n| n.kind).collect::<Vec<_>>();
        let r = kinds(&modded("RD"));
        assert!(r.contains(&Kind::Don) && r.contains(&Kind::Kat) && r == kinds(&modded("RD")));
        // Relax: rim keys on dons still hit.
        let mut g = modded("RX");
        let times: Vec<f64> = g.map.notes.iter().map(|n| n.time).collect();
        for t in times {
            g.press_at(0, t, None);
        }
        assert_eq!(g.stats.great, 64);
        // HR tightens the windows, EZ widens them, and both change the score multiplier.
        let great = |m: &str| Windows::new(Mods::parse(m).od(5.0)).great;
        assert!(great("HR") < great("") && great("EZ") > great("") && great("PR") < great(""));
        let (mut plain, mut hd) = (modded(""), modded("HD"));
        play_all(&mut plain, 0.0);
        play_all(&mut hd, 0.0);
        assert!(hd.stats.score > plain.stats.score);
    }

    #[test]
    fn a_clean_play_is_a_million_and_mods_go_above_or_below_it() {
        let mut g = modded("");
        let mut last = 0;
        let times: Vec<(f64, bool)> = g.map.notes.iter().filter(|n| matches!(n.kind, Kind::Don | Kind::Kat)).map(|n| (n.time, n.kind == Kind::Kat)).collect();
        for (t, kat) in times {
            g.press_at(if kat { 0 } else { 1 }, t, None);
            assert!(g.stats.score >= last, "the score only grows while nothing is missed");
            last = g.stats.score;
        }
        assert_eq!(g.stats.score, 1_000_000);
        // Rolls and swells aren't part of it.
        g.set_time(g.end_time + 2000.0);
        assert_eq!(g.stats.score, 1_000_000);
        let mut hard = modded("HD,HR");
        play_all(&mut hard, 0.0);
        assert!(hard.stats.score > 1_000_000);
        let mut easy = modded("EZ");
        play_all(&mut easy, 0.0);
        assert!(easy.stats.score < 1_000_000);
        // An OK, and a miss, each cost something; a miss more.
        let mut ok = modded("");
        let first = ok.map.notes[0].time;
        ok.press_at(1, first + 80.0, None);
        let mut miss = modded("");
        miss.set_time(first + 500.0);
        assert!(ok.stats.ok == 1 && miss.stats.miss == 1);
        play_all(&mut ok, 0.0);
        play_all(&mut miss, 0.0);
        assert!(miss.stats.score < ok.stats.score && ok.stats.score < 1_000_000, "{} {}", miss.stats.score, ok.stats.score);
    }

    #[test]
    fn wind_up_maps_song_time_to_faster_map_time() {
        let mut g = modded("WU");
        let last = g.end_time;
        // By the end the map runs 1.5x: a song second is 1.5 map seconds.
        let t = g.track_at(last);
        assert!(((g.speed.map(t + 1000.0) - last) - 1500.0).abs() < 1.0);
        play_all(&mut g, 0.0);
        assert_eq!(g.stats.great, 64);
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
    fn replay_reproduces_the_play() {
        // A sloppy play: early, late, wrong colour, doubled hits.
        let mut live = game();
        live.record = true;
        let notes: Vec<(f64, bool)> = live.map.notes.iter().filter(|n| matches!(n.kind, Kind::Don | Kind::Kat)).map(|n| (n.time, n.kind == Kind::Kat)).collect();
        for (k, &(t, kat)) in notes.iter().enumerate() {
            let jitter = [-30.0, 12.0, 45.0, -70.0, 0.0, 100.0][k % 6];
            let key = if (k % 7 == 3) != kat { 0 } else { 1 };
            let at = t + jitter;
            live.inputs.push((at, key as u8));
            live.press_at(key, at, None);
            if k % 5 == 0 {
                live.inputs.push((at + 8.0, 2));
                live.press_at(2, at + 8.0, None);
            }
        }
        let end = notes.last().unwrap().0 + 1000.0;
        live.time = end;
        live.expire(end);

        let mut back = game();
        back.replay = Some((live.inputs.clone(), 0));
        back.time = end;
        back.feed_replay(None);
        back.expire(end);
        let (a, b) = (&live.stats, &back.stats);
        assert_eq!((a.great, a.ok, a.miss, a.score, a.max_combo), (b.great, b.ok, b.miss, b.score, b.max_combo));
        assert!(a.miss > 0 && a.great > 0, "the test play should mix judgements");
    }

    #[test]
    fn seeking_a_replay_back_and_forth_lands_on_the_same_state() {
        let mut g = game();
        let inputs: Vec<(f64, u8)> = g
            .map
            .notes
            .iter()
            .filter(|n| matches!(n.kind, Kind::Don | Kind::Kat))
            .enumerate()
            .map(|(k, n)| (n.time + [-20.0, 10.0, 60.0][k % 3], if n.kind == Kind::Kat { 0 } else { 1 }))
            .collect();
        g.replay = Some((inputs, 0));
        let (start, end) = g.span();
        let mid = (start + end) / 2.0;
        g.seek(mid, None, 0.0);
        let at_mid = (g.stats.great, g.stats.ok, g.stats.miss, g.stats.score, g.stats.combo);
        assert!(at_mid.0 > 0);
        g.seek(end, None, 0.0);
        assert!(g.stats.score > at_mid.3);
        // Back to the middle: rebuilt from the start, same as getting there directly.
        g.seek(mid, None, 0.0);
        assert_eq!((g.stats.great, g.stats.ok, g.stats.miss, g.stats.score, g.stats.combo), at_mid);
        assert_eq!(g.time, mid);
        g.seek(start - 10_000.0, None, 0.0);
        assert_eq!((g.stats.score, g.time), (0, start));
    }

    #[test]
    fn a_long_combo_broken_plays_the_combo_break_once() {
        let mut g = game();
        g.sfx_log = Some(Vec::new());
        let notes: Vec<(f64, bool)> = g.map.notes.iter().filter(|n| matches!(n.kind, Kind::Don | Kind::Kat)).map(|n| (n.time, n.kind == Kind::Kat)).collect();
        // 25 good hits, then let notes pass: one combo break, nothing for the misses after it.
        for &(t, kat) in &notes[..25] {
            g.press_at(if kat { 0 } else { 1 }, t, None);
        }
        g.set_time(notes[28].0 + 1000.0);
        let log = g.sfx_log.take().unwrap();
        let breaks: Vec<f64> = log.iter().filter(|e| matches!(e.1, Sfx::ComboBreak)).map(|e| e.0).collect();
        assert_eq!(breaks.len(), 1, "{breaks:?}");
        assert!(log.iter().filter(|e| !matches!(e.1, Sfx::ComboBreak)).all(|e| e.2 == 1.0));
    }

    #[test]
    fn version_1_replays_keep_their_mash_setting() {
        let mut g = game();
        let first = g.map.notes.iter().position(|n| matches!(n.kind, Kind::Don | Kind::Kat)).unwrap();
        let (t0, kat) = (g.map.notes[first].time, g.map.notes[first].kind == Kind::Kat);
        let early = t0 - (g.windows.ok + g.windows.miss) / 2.0;
        // Off: too early or the wrong colour does nothing, the right hit still counts.
        g.mash = Mash::legacy(false);
        g.judge_press(1, kat, early);
        g.judge_press(1, !kat, t0);
        assert_eq!((g.stats.miss, g.state[first].judged), (0, None));
        g.judge_press(1, kat, t0);
        assert_eq!(g.state[first].judged, Some(Judge::Great));
        // On: the early hit is a miss, like osu!.
        let mut g = game();
        g.mash = Mash::legacy(true);
        g.judge_press(1, kat, early);
        assert_eq!((g.stats.miss, g.state[first].judged), (1, Some(Judge::Miss)));
    }

    /// A stream of alternating dons and kats, `spacing` ms apart, at OD 5.
    fn stream(spacing: f64) -> Game {
        let notes = (0..24)
            .map(|k| beatmap::Note {
                time: 2000.0 + k as f64 * spacing,
                end_time: 2000.0 + k as f64 * spacing,
                kind: if k % 2 == 0 { Kind::Don } else { Kind::Kat },
                big: false,
                velocity: 0.5,
                tick_spacing: 0.0,
                required_hits: 0,
                fake_len: 0.0,
                fake_fades: false,
                fake_length: 0.0,
                unhittable: false,
            })
            .collect();
        let mut map = beatmap::calibration_map(500.0, 4, 1);
        map.notes = notes;
        map.meta.od = 5.0;
        Game::with_mods(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None, Mods::NONE)
    }

    fn play(g: &mut Game, presses: &[(f64, bool)]) {
        for &(t, kat) in presses {
            g.judge_press(if kat { 0 } else { 1 }, kat, t);
        }
        g.set_time(g.map.notes.last().unwrap().time + 1000.0);
    }

    #[test]
    fn one_slip_in_a_stream_costs_at_most_one_note() {
        for spacing in [60.0, 75.0, 90.0, 110.0] {
            let notes: Vec<(f64, bool)> = stream(spacing).map.notes.iter().map(|n| (n.time, n.kind == Kind::Kat)).collect();
            // An extra tap of the same colour right after the 5th note.
            let mut g = stream(spacing);
            let mut presses = notes.clone();
            presses.insert(5, (notes[4].0 + 12.0, notes[4].1));
            play(&mut g, &presses);
            assert_eq!((g.stats.miss, g.stats.great), (0, 24), "extra tap @{spacing}");
            // An extra tap of the other colour: still nothing, it's nearer the note just hit.
            let mut g = stream(spacing);
            let mut presses = notes.clone();
            presses.insert(5, (notes[4].0 + 12.0, !notes[4].1));
            play(&mut g, &presses);
            assert_eq!(g.stats.miss, 0, "extra wrong tap @{spacing}");
            // The 5th note skipped: that one misses, the rest are hit.
            let mut g = stream(spacing);
            let mut presses = notes.clone();
            presses.remove(4);
            play(&mut g, &presses);
            assert_eq!((g.stats.miss, g.stats.great), (1, 23), "skipped note @{spacing}");
            // The 5th note hit with the wrong colour, on time: that one misses.
            let mut g = stream(spacing);
            let mut presses = notes.clone();
            presses[4].1 = !presses[4].1;
            play(&mut g, &presses);
            assert_eq!((g.stats.miss, g.stats.great), (1, 23), "wrong colour @{spacing}");
        }
    }

    #[test]
    fn mashing_is_still_punished() {
        // Both colours as fast as possible all through a stream.
        let mut g = stream(90.0);
        let end = g.map.notes.last().unwrap().time;
        let presses: Vec<(f64, bool)> = (0..).map(|k| (1950.0 + k as f64 * 20.0, k % 2 == 1)).take_while(|p| p.0 < end + 50.0).collect();
        play(&mut g, &presses);
        // No GREATs, some misses: worse than presses that would just be ignored (all OKs).
        let mut ignored = stream(90.0);
        ignored.mash = Mash::Ignore;
        play(&mut ignored, &presses);
        assert!(g.stats.great == 0 && g.stats.miss >= 4, "{:?}", (g.stats.great, g.stats.ok, g.stats.miss));
        assert!(g.stats.accuracy() < ignored.stats.accuracy() - 0.05);
        // A right colour too early, with nothing else near, misses like osu!.
        let mut g = stream(200.0);
        let t0 = g.map.notes[0].time;
        g.judge_press(1, false, t0 - (g.windows.ok + g.windows.miss) / 2.0);
        assert_eq!(g.state[0].judged, Some(Judge::Miss));
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
    /// The calibration map with a kiai from 8 s, for the mods that wait for one.
    fn kiai_game(mods: &str) -> Game {
        let mut map = beatmap::calibration_map(500.0, 0, 64);
        let cp = map.control_points[0];
        map.control_points.push(beatmap::ControlPoint { time: 8000.0, kiai: true, ..cp });
        Game::with_mods(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None, Mods::parse(mods))
    }

    #[test]
    fn in_kiai_each_fake_slider_under_a_note_adds_its_own_flash() {
        // Lights drawn with `stack` fake sliders under the note at 10 s, at map time `at`.
        let lights = |stack: usize, at: f64| {
            let mut map = beatmap::calibration_map(500.0, 0, 64);
            let cp = map.control_points[0];
            map.control_points.push(beatmap::ControlPoint { time: 8000.0, kiai: true, ..cp });
            let head = map.notes.iter().find(|n| n.time == 10000.0).unwrap().clone();
            for _ in 0..stack {
                map.notes.push(beatmap::Note { kind: Kind::Roll, fake_len: 0.0001, fake_fades: true, tick_spacing: 125.0, ..head.clone() });
            }
            map.notes.sort_by(|a, b| a.time.total_cmp(&b.time));
            let mut g = Game::new(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None);
            g.time = at;
            let mut c = Canvas::default();
            c.clear(853.3);
            g.draw_playfield(&mut c, 1.0);
            c.shapes.iter().filter(|s| s.params[2] < -0.5).count()
        };
        // On a kiai beat, the deeper the pile the more light; outside kiai, none at all.
        assert_eq!(lights(3, 9510.0) - lights(0, 9510.0), 3);
        assert_eq!(lights(3, 5010.0), 0);
    }

    #[test]
    fn the_kiai_flash_fades_over_each_beat_from_the_kiais_start() {
        let mut map = beatmap::calibration_map(500.0, 0, 64);
        let cp = map.control_points[0];
        // Kiai 8100..9000 (off the red line's beats), then a gimmick line inside it.
        map.control_points.push(beatmap::ControlPoint { time: 8100.0, kiai: true, ..cp });
        map.control_points.push(beatmap::ControlPoint { time: 8600.0, kiai: true, beat_length: 6.0, ..cp });
        map.control_points.push(beatmap::ControlPoint { time: 9000.0, kiai: false, ..cp });
        let g = Game::new(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None);
        let f = |t: f64| g.kiai_flash(t);
        assert_eq!(f(8000.0), 0.0);
        assert_eq!(f(8100.0), 1.0);
        assert!((f(8350.0) - 0.5).abs() < 1e-6, "linear, from the kiai's start");
        // The gimmick line's 6 ms beat is skipped for the real 500 ms one.
        assert!((f(8725.0) - 0.75).abs() < 1e-6);
        // A 120 BPM kiai ending mid-beat just stops.
        assert_eq!(f(9000.0), 0.0);
    }

    #[test]
    fn a_pop_up_fake_slider_stands_by_its_length_and_fades_in() {
        let head = beatmap::calibration_map(387.0, 0, 4).notes[0].clone();
        // On a 1 ms red line (velocity ~302), lengths -300, -450 and -510, at 1000.
        let fake = |len: f64, velocity: f64| beatmap::Note { time: 1000.0, kind: Kind::Roll, fake_len: 1.0, fake_fades: false, fake_length: len, velocity, ..head.clone() };
        let lane = 640.0;
        let at = |len: f64, vt: f64| popup(&fake(len, 302.0), vt, lane, 1.0, 387.0, 1.8, 1.0);
        let s = |len: f64| at(len, 950.0).unwrap().0 / lane;
        // Stable: -300 at ~17% of the lane, the farthest well inside its first 75%.
        assert!((s(300.0) - 0.17).abs() < 0.01 && s(510.0) < 0.75, "{} {}", s(300.0), s(510.0));
        assert!((s(450.0) - s(300.0) - 150.0 / 30.0 * 95.0 / 1560.0).abs() < 1e-4);
        // Shown 300 / 180 beats before (645 ms), -510 2.83 beats (1097 ms), fading in over
        // 666 ms; gone at its time, no fade out (stable).
        assert_eq!(at(300.0, 1000.0 - 650.0).unwrap().2, 0.0);
        assert!(at(300.0, 1000.0 - 600.0).unwrap().2 > 0.0);
        assert!(at(510.0, 1000.0 - 1050.0).unwrap().2 > 0.0);
        assert_eq!(at(510.0, 1000.0 - 400.0).unwrap().2, 1.0);
        assert!(at(300.0, 1000.0).is_none());
        // At an ordinary speed it's the usual fake slider.
        assert!(popup(&fake(300.0, 0.78), 950.0, lane, 1.0, 387.0, 1.8, 1.0).is_none());
    }

    #[test]
    fn a_note_before_a_nan_slider_cannot_be_hit() {
        let mut map = beatmap::calibration_map(500.0, 0, 64);
        map.notes[4].unhittable = true;
        let t = map.notes[4].time;
        let mut g = Game::new(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None);
        g.press_at(1, t, None);
        g.press_at(1, t + 500.0, None);
        assert_eq!(g.state[4].judged, Some(Judge::Miss));
        assert_eq!(g.state[5].judged, Some(Judge::Great), "the press went on to the next note");
    }

    #[test]
    fn roll_ticks_are_yellow_on_the_beat_and_orange_on_the_half() {
        // 1/4 ticks from a red line at 100 with a 400 ms beat: beat, 1/4, half, 3/4, beat.
        let tints: Vec<u32> = (0..5).map(|k| tick_tint(100.0 + k as f64 * 100.0, 100.0, 400.0)).collect();
        assert_eq!(tints, [ROLL_TICK_BEAT, WHITE, ROLL_TICK_HALF, WHITE, ROLL_TICK_BEAT]);
        // A roll off the grid takes none.
        assert_eq!(tick_tint(130.0, 100.0, 400.0), WHITE);
    }

    #[test]
    fn a_swell_fades_in_from_a_beat_and_a_half_before_it() {
        // 120 BPM: hidden until 750 ms before, fully in by 500 ms before.
        assert_eq!(swell_fade(500.0, 850.0), 0.0);
        assert_eq!(swell_fade(500.0, 625.0), 0.25);
        assert_eq!(swell_fade(500.0, 500.0), 1.0);
        assert_eq!(swell_fade(500.0, 0.0), 1.0);
        // 30 BPM: from 3000 ms before, in over lazer's 666 ms.
        assert_eq!(swell_fade(2000.0, 3000.0), 0.0);
        assert_eq!(swell_fade(2000.0, 2334.0), 1.0);
    }

    #[test]
    fn notes_under_a_swell_still_take_their_hits() {
        let mut map = beatmap::calibration_map(500.0, 0, 64);
        let swell = beatmap::Note { time: 1900.0, end_time: 4100.0, kind: Kind::Swell, required_hits: 50, ..map.notes[0].clone() };
        map.notes.insert(4, swell);
        let mut g = Game::new(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None);
        for t in [2000.0, 2500.0, 3000.0, 3500.0, 4000.0] {
            g.press_at(1, t, None);
        }
        let under: Vec<_> = (0..g.map.notes.len()).filter(|&i| g.map.notes[i].kind == Kind::Don && (2000.0..=4000.0).contains(&g.map.notes[i].time)).collect();
        assert_eq!(under.len(), 5);
        assert!(under.iter().all(|&i| g.state[i].judged == Some(Judge::Great)), "notes under the swell were swallowed");
    }

    #[test]
    fn a_roll_over_notes_is_played_as_the_notes() {
        // A roll from 1900 to 4100 over the calibration dons, one of them a kat.
        let mut map = beatmap::calibration_map(500.0, 0, 64);
        let kat = map.notes.iter().position(|n| n.time == 3000.0).unwrap();
        map.notes[kat].kind = Kind::Kat;
        let roll = beatmap::Note { time: 1900.0, end_time: 4100.0, kind: Kind::Roll, tick_spacing: 125.0, ..map.notes[0].clone() };
        map.notes.insert(4, roll);
        let under = |g: &Game| -> Vec<Option<Judge>> {
            (0..g.map.notes.len()).filter(|&i| matches!(g.map.notes[i].kind, Kind::Don | Kind::Kat) && (2000.0..=4000.0).contains(&g.map.notes[i].time)).map(|i| g.state[i].judged).collect()
        };
        // Autoplay: every note Great, no tick taken.
        let mut g = Game::new(map.clone(), Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None);
        let mut t = 1800.0;
        while t < 4300.0 {
            g.time = t;
            g.autoplay(None);
            t += 4.0;
        }
        assert_eq!(under(&g), vec![Some(Judge::Great); 5]);
        assert_eq!(g.stats.roll_ticks, 0);
        // A player: a press on the kat inside the roll is the kat's, not a tick.
        let mut g = Game::new(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None);
        g.press_at(0, 3000.0, None);
        assert_eq!(g.state[kat + 1].judged, Some(Judge::Great));
    }

    #[test]
    fn a_barline_slower_than_every_note_sizes_the_lookahead() {
        // An anti-barline wall / scenery bar scrolling a tenth of the notes' speed.
        let mut map = beatmap::calibration_map(500.0, 0, 64);
        let slow = map.notes[0].velocity / 10.0;
        map.barlines.push(beatmap::Barline { time: 40_000.0, velocity: slow });
        let g = Game::new(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None);
        assert_eq!(g.min_velocity, slow);

        // A fake slider at SV 0.02 (120 BPM): ~0.008, under the old 0.01 floor, so it was looked
        // for only once it was already well onto the lane.
        let mut map = beatmap::calibration_map(500.0, 0, 64);
        let crawl = map.notes[0].velocity * 0.02;
        map.notes[10].velocity = crawl;
        let g = Game::new(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None);
        assert!(crawl < 0.01);
        assert_eq!(g.min_velocity, crawl);
    }

    #[test]
    fn radar_lights_notes_as_the_beam_passes_them() {
        // The beam leaves the drum every other beat and crosses the lane in two, in kiai in one.
        assert_eq!(radar_sweep(0.5, false), 0.25);
        assert_eq!(radar_sweep(3.0, false), 0.5);
        assert_eq!(radar_sweep(4.5, true), 0.5);
        let (len, u) = (600.0, 100.0);
        // A note 300 along lights up as the beam gets there and is gone by the next one.
        assert!(radar_light(300.0, u, len, 0.5) > 0.99);
        assert!(radar_light(300.0, u, len, 0.75) > radar_light(300.0, u, len, 0.95));
        assert!(radar_light(300.0, u, len, 0.499) < 1e-3);
        // By the drum it's always lit.
        assert_eq!(radar_light(50.0, u, len, 0.9), 1.0);
        // One that came on screen after the beam went by stays dark till the next one.
        assert_eq!(radar_age(590.0, u, len, 0.3), None);
        assert_eq!(radar_light(590.0, u, len, 0.3), 0.0);
        // Hidden decides when far notes show too: picking it switches Radar off.
        assert!(!Mods::parse("RA,HD").has(Mod::Radar));
    }

    #[test]
    fn echo_shows_the_next_bar_in_colour_and_the_lane_in_grey() {
        let mut map = beatmap::calibration_map(500.0, 0, 64);
        for (k, n) in map.notes.iter_mut().enumerate() {
            if k % 3 == 1 {
                n.kind = Kind::Kat;
            }
        }
        let mut g = Game::with_mods(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None, Mods::parse("EC"));
        // Bars are 2 s: in the one from 2 s, the one from 4 s shows; before the first barline,
        // the first bar.
        assert_eq!(g.echo_bars(2500.0), Some([(2000.0, 2000.0), (4000.0, 2000.0)]));
        assert_eq!(g.echo_bars(-500.0), Some([(-2000.0, 2000.0), (0.0, 2000.0)]));
        g.time = 2500.0;
        let mut c = Canvas::default();
        c.clear(853.3);
        g.draw_playfield(&mut c, 1.0);
        let fill = |s: &Shape| (s.fill[0] * 255.0).round() as u32 * 65536 + (s.fill[1] * 255.0).round() as u32 * 256 + (s.fill[2] * 255.0).round() as u32;
        let notes: Vec<u32> = c.shapes.iter().filter(|s| s.center[1] == LANE_Y && s.half[0] == NOTE_R && s.center[0] > HIT_X + 40.0).map(fill).collect();
        assert!(!notes.is_empty() && notes.iter().all(|&f| f == GREY), "{notes:x?}");
        let dots: std::collections::HashSet<u32> = c.shapes.iter().filter(|s| s.center[1] == ECHO_Y && s.half[0] < 10.0 && s.half[0] > 3.0).map(fill).collect();
        assert!(dots.len() >= 2 && !dots.contains(&GREY), "{dots:x?}");
    }

    #[test]
    fn a_lobby_hosts_mod_settings_are_what_the_match_plays_with() {
        let mut g = kiai_game("MV");
        g.use_mod_settings(Some(multiverse::classic()), None);
        assert_eq!(g.universes, Some(multiverse::classic()));
        let mut g = kiai_game("SS");
        let host = shapeshift::Config::parse("every=4 order=fixed shapes=spiral,loop,-zigzag,-uturn,-elbow,-straight");
        g.use_mod_settings(None, Some(host));
        assert_eq!(g.shapeshift.bars(), 4);
        assert_eq!(&g.shape_order[..4], &[2, 3, 2, 3]);
        // Settings the host didn't send stay the player's own.
        let mut g = kiai_game("SS");
        let own = g.shape_order.clone();
        g.use_mod_settings(None, None);
        assert_eq!((g.universes, g.shape_order), (None, own));
    }

    #[test]
    fn every_mod_draws_a_sane_playfield_and_judges_the_same() {
        let all: Vec<&str> = crate::mods::ALL.iter().map(|m| m.acronym()).collect();
        let mut sets: Vec<String> = all.iter().map(|a| a.to_string()).collect();
        // And every one at once, as far as conflicts let them.
        sets.push(all.join(","));
        for mods in &sets {
            let mut g = kiai_game(mods);
            let mut c = Canvas::default();
            for k in 0..400 {
                g.time = -2000.0 + k as f64 * 87.3;
                g.press_at(k % 4, g.time, None);
                c.clear(853.3);
                g.draw_playfield(&mut c, 1.0);
                for sh in &c.shapes {
                    let v = [sh.center[0], sh.center[1], sh.half[0], sh.half[1], sh.angle];
                    assert!(v.iter().all(|x| x.is_finite()), "{mods} at {}: {v:?}", g.time);
                }
            }
            // Judging doesn't care how notes are drawn.
            if ["AT", "HA", "SW", "RD"].contains(&mods.as_str()) || mods.contains(',') {
                continue;
            }
            let mut g = kiai_game(mods);
            play_all(&mut g, 0.0);
            assert_eq!(g.stats.great, 64, "{mods}");
        }
    }

    #[test]
    fn baked_speeds_bend_the_maps_own() {
        let v = |mods: &str| kiai_game(mods).map.notes.iter().map(|n| n.velocity).collect::<Vec<_>>();
        let plain = v("");
        // One speed throughout: Anti-SV and Hyper SV leave it, Crescendo ramps it up.
        assert_eq!(v("AS"), plain);
        assert_eq!(v("HS"), plain);
        let c = v("CC");
        assert!(c[0] < plain[0] && c[63] > plain[63] && c.windows(2).all(|p| p[1] >= p[0]));
        // Leapfrog alternates; Drop speeds up the kiai.
        let l = v("LF");
        assert!(l[0] > plain[0] && l[1] < plain[1]);
        let d = v("DR");
        assert!(d[0] == plain[0] && d[63] > plain[63]);
        // Roulette changes every 4 bars, and never to the same speed.
        let g = kiai_game("RL");
        assert!(g.sections.len() > 2 && g.sections[0].1 == 1.0 && g.sections.windows(2).all(|p| p[0].1 != p[1].1));
        // Even: evenly spaced notes keep their speed.
        assert_eq!(v("EV"), plain);
    }

    #[test]
    fn drop_freezes_far_notes_and_releases_them_by_the_kiai() {
        let g = kiai_game("DR");
        let cp = g.map.control_points[0];
        // Held from two beats before (8000 - 1000) for a beat and a half, back to 0 at the kiai.
        assert_eq!(g.extra_warp(6900.0, &cp), 0.0);
        assert!((g.extra_warp(7500.0, &cp) - 500.0).abs() < 1e-9);
        assert!(g.extra_warp(7999.0, &cp) < 1.0);
        assert_eq!(g.extra_warp(8100.0, &cp), 0.0);
    }

    #[test]
    fn flip_flop_turns_over_every_8_bars() {
        let g = kiai_game("FF");
        let bars = &g.map.barlines;
        assert_eq!(flip_flop(bars, bars[3].time), 1.0);
        assert_eq!(flip_flop(bars, bars[9].time), -1.0);
        assert!(flip_flop(bars, bars[8].time + 175.0).abs() < 1e-3);
    }

    #[test]
    fn multiverse_universes_draw_with_their_own_mods_only_while_drawn() {
        multiverse::set(multiverse::defaults());
        for t in [2000.0, 3600.0, 9000.0] {
            let mut g = kiai_game("MV,HD");
            g.set_time(t);
            let mut c = Canvas { width: 853.3, ..Default::default() };
            g.draw_playfield(&mut c, 1.0);
            assert_eq!(g.mods, Mods::parse("MV,HD"));
            assert!(!c.shapes.is_empty());
        }
        // Sudden in a universe would hide every note with the play's Hidden: it's left out.
        assert_eq!(Mods::parse("HD").plus(Mods::parse("SU,SK")), Mods::parse("HD,SK"));
    }

    #[test]
    fn a_crawling_note_at_the_end_of_the_map_is_looked_for_from_the_start() {
        // Ten minutes out, slow enough to be on the lane already: no lookahead cap hides it.
        let mut map = beatmap::calibration_map(500.0, 0, 64);
        let last = map.notes.len() - 1;
        map.notes[last].time = 600_000.0;
        map.notes[last].end_time = 600_000.0;
        map.notes[last].velocity = 1e-4;
        let mut g = Game::new(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None);
        g.set_time(1000.0);
        g.draw_playfield(&mut Canvas { width: 853.3, ..Default::default() }, 1.0);
        assert!(g.visible.contains(&last));
    }

    #[test]
    fn a_faster_universe_shows_its_notes_further_out() {
        let mut g = kiai_game("");
        g.set_time(1000.0);
        let mut far = Vec::new();
        for speed in [2, 4] {
            let uni = Universe { speed, ..Universe::CLASSIC };
            let mut c = Canvas { width: 853.3, ..Default::default() };
            g.draw_field(&mut c, 1.0, Focus::FULL, &uni);
            far.push(g.visible.len());
        }
        // Faster notes cross the lane sooner: fewer of the map's are on it at once.
        assert!(far[1] < far[0], "notes on the lane at 1x and 1.5x: {far:?}");
    }

    #[test]
    fn multiverse_calls_the_next_playfield_and_moves_the_focus_on_bars() {
        let g = kiai_game("MV");
        // 2 s bars, kiai from 8 s: every 2 bars before it, every bar in it.
        let times: Vec<f64> = g.focus_switches.iter().map(|s| s.at).collect();
        let want: Vec<f64> = std::iter::once(4000.0).chain((4..16).map(|i| i as f64 * 2000.0)).collect();
        assert_eq!(times, want);
        let mut focus = FIRST_FOCUS;
        for s in &g.focus_switches {
            // Well above or below, so neither hides the other.
            assert!(s.field < FIELD_COUNT && (parked(s.field).1 - parked(focus).1).abs() >= 80.0);
            focus = s.field;
        }
        let next = g.focus_switches[0].field;
        let other = (0..FIELD_COUNT).find(|&f| f != next && f != FIRST_FOCUS).unwrap();
        let at = |t: f64| g.multiverse_at(t);
        let calm = at(2000.0);
        assert!(calm[FIRST_FOCUS].focus == 1.0 && calm[FIRST_FOCUS].settle == 1.0);
        assert!(calm[next].settle == 0.0 && calm[next].aim == 0.0);
        // Called over the two beats before: it parks, lights up and pings on each beat.
        let called = at(3510.0);
        assert!(called[next].aim == 1.0 && called[next].call > 0.9 && called[next].settle > 0.5);
        assert!(called[other].aim == 0.0 && called[other].settle == 0.0);
        assert!(at(3625.0)[next].settle == 1.0);
        // Its notes are in colour a beat before the focus lands; the one in focus keeps its own.
        assert!(at(2990.0)[next].colour == 0.0 && at(3500.0)[next].colour == 1.0 && at(3500.0)[next].focus == 0.0);
        assert!(at(3600.0)[FIRST_FOCUS].colour == 1.0);
        // The focus slides across over the last 375 ms and lands on the bar.
        let mid = at(3812.5);
        assert!((mid[next].focus - 0.5).abs() < 1e-6 && (mid[FIRST_FOCUS].focus - 0.5).abs() < 1e-6);
        let done = at(4000.0);
        assert!(done[next].focus == 1.0 && done[next].aim == 0.0 && done[next].burst == Some(0.0));
        assert!(done[FIRST_FOCUS].focus == 0.0 && done[FIRST_FOCUS].settle == 0.0);
        // It overshoots its size on the way in and settles.
        assert!(at(3950.0)[next].grow > 1.0 && (done[next].grow - 1.0).abs() < 1e-6);
    }

    /// `TAIKO_MOD_SHOTS=<dir>`: a few frames of every mod as PNGs (drawn on the CPU), to look at.
    #[test]
    fn mod_shots() {
        let Some(dir) = std::env::var_os("TAIKO_MOD_SHOTS") else { return };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for m in crate::mods::ALL {
            let mut g = kiai_game(m.acronym());
            let mut c = Canvas::default();
            for t in [5000.0, 5200.0, 7400.0, 8300.0, 18_100.0] {
                g.set_time(t - 400.0);
                g.press_at(1, t - 400.0, None);
                g.time = t;
                c.clear(853.3);
                g.draw_playfield(&mut c, 1.0);
                let (w, h, rgb) = crate::debug::rasterize(&c, 1.0);
                crate::debug::write_png(&dir.join(format!("{}-{t}.png", m.acronym())), w, h, &rgb).unwrap();
            }
            // Streams and triplets, for the mods that go by rhythm.
            if ["SK", "PY", "LI", "DY", "SH", "SR", "CR"].contains(&m.acronym()) {
                let mut g = rhythm_game(m.acronym());
                for t in [2000.0, 5600.0, 7400.0] {
                    // Every note so far hit on time (Crumble breaks the lane by them).
                    let times: Vec<f64> = g.map.notes.iter().map(|n| n.time).take_while(|&n| n < t).collect();
                    for n in times {
                        g.press_at(1, n, None);
                    }
                    g.time = t;
                    c.clear(853.3);
                    g.draw_playfield(&mut c, 1.0);
                    let (w, h, rgb) = crate::debug::rasterize(&c, 1.0);
                    crate::debug::write_png(&dir.join(format!("{}-rhythm-{t}.png", m.acronym())), w, h, &rgb).unwrap();
                }
            }
            if let Some(&at) = g.scares.first() {
                g.time = at + 300.0;
                c.clear(853.3);
                g.draw_playfield(&mut c, 1.0);
                g.draw_hud(&mut c);
                let (w, h, rgb) = crate::debug::rasterize(&c, 1.0);
                crate::debug::write_png(&dir.join("scare.png"), w, h, &rgb).unwrap();
            }
        }
    }
    #[test]
    fn rails_leave_the_hit_target_heading_right_and_never_jump() {
        for shape in [RailShape::Zigzag, RailShape::UTurn, RailShape::Spiral, RailShape::Loop, RailShape::Elbow] {
            let len = shape.length(853.3);
            let rail = Rail::build(|s| shape.bend(s), len);
            assert_eq!(rail.at(0.0), (HIT_X, LANE_Y, 0.0));
            assert_eq!(rail.at(-50.0), (HIT_X - 50.0, LANE_Y, 0.0));
            // One unit along the lane is one unit on screen (give or take the corners).
            let mut prev = rail.at(0.0);
            for k in 1..(len as usize + 100) {
                let p = rail.at(k as f32);
                let step = (p.0 - prev.0).hypot(p.1 - prev.1);
                assert!((0.9..=1.01).contains(&step), "{shape:?} at {k}: {step}");
                prev = p;
            }
        }
    }

    #[test]
    fn the_loop_is_full_size_on_screen_and_crosses_the_lane_at_an_angle() {
        let shape = RailShape::Loop;
        // As big as the other rails: on 16:9 nothing shrinks.
        assert_eq!(shape.zoom(853.3), 1.0);
        for w in [853.3, 600.0] {
            let (len, zoom, drop) = (shape.length(w), shape.zoom(w), shape.drop());
            let rail = Rail::build(|s| shape.bend(s), len);
            // Where `s` along, `side` off the middle, ends up on screen.
            let at = |s: f32, side: f32| {
                let (x, y, a) = rail.at(s);
                let (x, y) = (x - a.sin() * side, y - a.cos() * side);
                (HIT_X + zoom * (x - HIT_X), LANE_Y + drop + zoom * (y - LANE_Y))
            };
            let mut s = 0.0;
            while s < len {
                for side in [-62.0, 0.0, 62.0] {
                    let (x, y) = at(s, side);
                    // The lane in leaves by the bottom right.
                    if x > w || y > VIRTUAL_HEIGHT {
                        continue;
                    }
                    assert!(y > 4.0, "{w}: off the top at {s} ({x}, {y})");
                    if w > 800.0 {
                        // Its error ticks reach up to 419.
                        let meter = (x - w / 2.0).abs() < 158.0 && y > 415.0;
                        assert!(!meter, "over the hit error meter at {s} ({x}, {y})");
                    }
                }
                s += 2.0;
            }
            // The lane into the loop comes up across the straight, not along it.
            let after = rail.at(LOOP_AT + LOOP_TURN * LOOP_R + 10.0).2;
            assert!((after - LOOP_TURN).abs() < 0.05, "{w}: heading {after}");
        }
    }

    #[test]
    fn rails_bend_the_lane_without_speeding_notes_up() {
        // Along the lane, notes go as on a plain one (as many to a stretch of it), whatever the
        // shape, its length, and how much the playfield shrinks to fit it.
        for w in [853.3, 660.0, 600.0] {
            for mods in ["ZZ", "UT", "SI", "LP", "EB", "SS"] {
                let g = kiai_game(mods);
                for t in [0.0, 20_000.0, 50_000.0] {
                    let (rail, len, _, zoom) = g.rail_at(t, w);
                    assert!(zoom <= 1.0);
                    assert_eq!(lane_stretch(rail.is_some(), len, w), 1.0, "{mods} at {w}");
                }
            }
            // Plain lanes are as they were.
            assert_eq!(lane_stretch(false, w - HIT_X, w), 1.0);
        }
    }

    #[test]
    fn portal_keeps_the_drum_and_jumps_at_the_entry() {
        let p = (400.0, 150.0);
        assert_eq!(through_portal(0.0, p).0, 0.0);
        assert_eq!(through_portal(-30.0, p).0, -30.0);
        assert_eq!(through_portal(500.0, p), (500.0, 1.0));
        // Just past the entry it is at the exit, small; halfway in, halfway to the drum.
        let (s, size) = through_portal(399.9, p);
        assert!((s - 150.0).abs() < 0.1 && size < 0.2);
        assert!((through_portal(200.0, p).0 - 75.0).abs() < 1e-3);
    }

    #[test]
    fn a_miss_cracks_the_lane_and_50_hits_mend_it() {
        let mut g = kiai_game("CK");
        let times: Vec<f64> = g.map.notes.iter().map(|n| n.time).collect();
        g.set_time(times[0] + 200.0);
        assert_eq!(g.stats.miss, 1);
        assert_eq!(g.cracks_at(times[0] + 400.0, 700.0).len(), 1);
        for &t in &times[1..52] {
            g.press_at(1, t, None);
        }
        assert!(g.cracks_at(times[51], 700.0).is_empty());
    }
    #[test]
    fn every_hit_crumbles_the_lane_by_its_rhythm_and_it_settles_back() {
        let mut g = kiai_game("CR");
        let times: Vec<f64> = g.map.notes.iter().map(|n| n.time).collect();
        // A 1/1 note knocks out a bigger piece, further, for longer, than a 1/4 one.
        g.flows[0] = 1;
        g.flows[1] = 4;
        g.press_at(1, times[0], None);
        g.press_at(1, times[1], None);
        assert_eq!(g.crumbs.len(), 2);
        let (slow, fast) = (g.crumbs[0], g.crumbs[1]);
        assert!(slow.len > fast.len && slow.shift.abs() > fast.shift.abs() && slow.life > fast.life);
        // Never near the drum, and never further out than the cap however many pile up.
        for &t in &times[2..40] {
            g.press_at(1, t, None);
            let geo = LaneGeo { rail: None, len: 700.0, shatter: [0.0; 4], cracks: g.cracks_at(t + 80.0, 700.0) };
            for s in (0..700).step_by(5) {
                assert!(geo.shift(s as f32).abs() <= CRUMBLE_MAX_SHIFT);
            }
            assert!(geo.cracks.iter().all(|&(a, ..)| a >= 700.0 * CRUMBLE_SPAN.0 - 0.01));
        }
        assert!(g.cracks_at(times[39] + 3000.0, 700.0).is_empty());
    }

    #[test]
    fn crumble_grows_with_the_combo_and_a_miss_puts_it_back() {
        assert!(crumble_power(0) < 0.3 && crumble_power(200) == 1.0 && crumble_power(1000) > 1.3);
        let mut g = kiai_game("CR");
        let times: Vec<f64> = g.map.notes.iter().map(|n| n.time).collect();
        g.flows.iter_mut().for_each(|f| *f = 1);
        // A miss knocks nothing out.
        g.set_time(times[0] + 200.0);
        assert!(g.stats.miss == 1 && g.crumbs.is_empty());
        // The same 1/1 note knocks out more at a higher combo.
        g.press_at(1, times[1], None);
        let low = g.crumbs[0];
        g.stats.combo = 300;
        g.press_at(1, times[2], None);
        let high = *g.crumbs.last().unwrap();
        assert!(high.len > low.len && high.shift.abs() > low.shift.abs() && high.life > low.life);
        // A miss snaps the pieces still out back into line.
        let out = |g: &Game, t: f64| g.crumbs_at(t, 700.0).map(|c| c.2.abs()).fold(0.0, f32::max);
        g.set_time(times[3] + 200.0);
        assert_eq!(g.stats.miss, 2);
        let miss = *g.misses.back().unwrap();
        assert!(out(&g, miss - 1.0) > 10.0);
        assert_eq!(out(&g, miss + CRUMBLE_MEND_MS), 0.0);
    }

    #[test]
    fn overtime_screams_once_per_scare() {
        let mut g = kiai_game("OT");
        g.sfx_log = Some(Vec::new());
        assert_eq!(g.scares.len(), 1);
        let at = g.scares[0];
        assert!(at > g.first_note + 8000.0 && at < g.end_time);
        for k in 0..200 {
            g.set_time(at - 1000.0 + k as f64 * 10.0);
        }
        let screams = |g: &Game| g.sfx_log.as_ref().unwrap().iter().filter(|e| matches!(e.1, Sfx::Scream)).count();
        assert_eq!(screams(&g), 1);
        // Jumping past one leaves it quiet.
        let mut g = kiai_game("OT");
        g.sfx_log = Some(Vec::new());
        g.set_time(at + 5000.0);
        assert_eq!(screams(&g), 0);
    }
    /// 1/4s from 0 ms, triplets from 2880 ms and 1/6s from 6720 ms, at 125 BPM.
    fn rhythm_game(mods: &str) -> Game {
        let mut map = beatmap::calibration_map(480.0, 0, 64);
        for (k, n) in map.notes.iter_mut().enumerate() {
            n.time = match k {
                0..24 => k as f64 * 120.0,
                24..48 => 2880.0 + (k - 24) as f64 * 160.0,
                _ => 6720.0 + (k - 48) as f64 * 80.0,
            };
            n.end_time = n.time;
        }
        Game::with_mods(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None, Mods::parse(mods))
    }

    #[test]
    fn snaps_and_liar() {
        let g = rhythm_game("LI");
        let snaps: Vec<u8> = g.map.notes.iter().map(|n| snap_of(&g.map.control_points, n.time)).collect();
        assert_eq!(&snaps[..4], &[1, 4, 2, 4]);
        assert_eq!(&snaps[24..27], &[1, 3, 3]);
        assert_eq!(&snaps[48..51], &[1, 6, 3]);
        // A 1/3 pretends to be the 1/4 next to it; 1/4s and notes on the beat tell the truth.
        assert_eq!(g.lies[25], 3000.0);
        assert_eq!(g.lies[1], g.map.notes[1].time);
        assert_eq!(g.lies[24], g.map.notes[24].time);
        // Snap Speed: triplets slower, 1/6s faster.
        let v = rhythm_game("SR").map.notes.iter().map(|n| n.velocity).collect::<Vec<_>>();
        assert!(v[25] < v[1] && v[49] > v[1] && v[0] == v[1] && v[26] == v[25]);
        assert_eq!((g.flows[0], g.flows[2], g.flows[25], g.flows[26], g.flows[50]), (4, 4, 3, 3, 6));
    }

    #[test]
    fn decoys_get_into_streams_but_never_twice_running() {
        let g = rhythm_game("DY");
        let in_stream: Vec<f64> = g.decoys.iter().map(|n| n.time).filter(|&t| t > 6720.0).collect();
        assert!(!in_stream.is_empty() && in_stream.len() <= 5, "{in_stream:?}");
        assert!(in_stream.windows(2).all(|p| p[1] - p[0] > 100.0), "{in_stream:?}");
        assert!(g.decoys.iter().filter(|n| n.time > 6720.0).all(|n| !n.big));
    }
}
