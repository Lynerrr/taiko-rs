//! Gameplay: judging, scoring and drawing the playfield.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

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

fn black_lane() -> bool {
    playfield_look().black_lane
}

use crate::beatmap::{self, Beatmap, Kind};
use crate::multiverse::{self, Shape as NoteShape, Universe};
use crate::tko::runtime::NoteFx;
use taiko_logic::calibrate;
use taiko_logic::clock::GameClock;
use taiko_logic::defense::{Defense, DefenseAct, DefenseItem, TOWER_BARS, TeamState};
use taiko_logic::mods::{Mod, Mods, Speed};
use taiko_logic::shapeshift;

/// Where a play's diagnostic lines go: the client's audio log, once it's open (`log_to`).
static LOG: std::sync::OnceLock<fn(&str)> = std::sync::OnceLock::new();

/// Sends the plays' diagnostic lines (one at the end of each play) to `sink`. The first sink
/// handed over stays; until then the lines aren't even written.
pub fn log_to(sink: fn(&str)) {
    let _ = LOG.set(sink);
}

/// The song a play keeps time to: how long it is and how fast it was made to play. The client's
/// `audio::Track` is one; the sound it's handed to (`Audio::play`) takes it back as its own.
pub trait Song: Send + Sync {
    /// Its length in frames at the output's rate.
    fn frames(&self) -> usize;
    /// The rate it was made to play at (1.0 = the map's own speed).
    fn rate(&self) -> f64;
    /// Itself, for the sound to find its own kind of song in.
    fn into_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync>;
}

/// The sounds a play makes: the drum's, a broken combo, Overtime's scream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Don,
    Kat,
    BigDon,
    BigKat,
    /// A combo of 20 or more broken by a miss.
    ComboBreak,
    /// Overtime's jumpscare.
    Scream,
}

/// What the play needs from the sound: its song played from a point, paused and moved, how far it
/// has got, and hitsounds now or at a point of the song. The client's `audio::Engine` is one; a
/// play without sound (a replay worked out for its results) has none.
pub trait Audio {
    /// The output's sample rate.
    fn rate(&self) -> u32;
    fn play(&self, track: Arc<dyn Song>, start_ms: f64);
    fn seek(&self, ms: f64);
    fn set_paused(&self, paused: bool);
    fn sfx_at(&self, s: Sound, volume: f32);
    fn sfx_at_song(&self, s: Sound, volume: f32, song_ms: f64);
    /// Output and input latency, ms.
    fn latencies_ms(&self) -> (f64, f64);
    /// Where the song has got at `now`, ms.
    fn position_ms(&self, now: Instant) -> Option<f64>;
}

mod field;
mod hud_draw;
mod rails;
mod setup;

pub use self::{field::*, rails::*, setup::*};
/// Centre of the hit target: x = 160 of osu!'s 480-tall gamefield, as in stable and lazer.
pub const HIT_X: f32 = 160.0;
/// Centre of the lane: where osu! stable has it, a little above the middle of the screen.
pub const LANE_Y: f32 = 196.0;
pub const LANE_H: f32 = 118.0;
pub const NOTE_R: f32 = 31.0;
pub const BIG_R: f32 = 46.0;
/// Second key of a big note must land within this window of the first.
const STRONG_WINDOW: f64 = 30.0;
/// The combo break plays a little louder than a hitsound (about +1.6 dB): noticeable over the
/// music without being jarring.
const COMBO_BREAK_GAIN: f32 = 1.2;

pub const DON: u32 = 0xEB452C;
pub const KAT: u32 = 0x438EAD;

pub const PAUSE_ITEMS: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Judge {
    Great,
    Ok,
    Miss,
}

impl Judge {
    pub(crate) fn color(self) -> u32 {
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
        if n == 0 {
            1.0
        } else {
            (self.great as f64 + 0.5 * self.ok as f64) / n as f64
        }
    }
    pub fn mean_error(&self) -> f64 {
        if self.errors.is_empty() {
            0.0
        } else {
            self.errors.iter().sum::<f64>() / self.errors.len() as f64
        }
    }
    /// osu!'s unstable rate: 10x the standard deviation of hit errors.
    pub fn unstable_rate(&self) -> f64 {
        if self.errors.len() < 2 {
            return 0.0;
        }
        let mean = self.mean_error();
        let var =
            self.errors.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / self.errors.len() as f64;
        var.sqrt() * 10.0
    }
}

impl Drop for Game {
    /// The audio log's line for a play, however it ended: the offset it really wanted, overall and
    /// by thirds of the hits (a clock drifting mid-song shows as thirds that differ).
    fn drop(&mut self) {
        let s = &self.stats;
        let Some(log) = LOG.get() else {
            return;
        };
        if self.replay.is_some() || s.errors.len() < 30 {
            return;
        }
        // An error e under offset o would have been e + o under no offset at all.
        let wanted: Vec<f64> = s
            .errors
            .iter()
            .zip(&s.error_offsets)
            .map(|(e, o)| e + o)
            .collect();
        let robust = |part: &[f64]| calibrate::estimate(part, 0.0).map_or(f64::NAN, |e| e.error_ms);
        let third = |k: usize| robust(&wanted[k * wanted.len() / 3..(k + 1) * wanted.len() / 3]);
        let (sum, frames, most) = self.clock_vs_audio;
        log(&format!(
            "end {} - {} [{}] hits={} mean_error={:.1} wanted={:.1} thirds={:.1}/{:.1}/{:.1} ur={:.0} played_s={:.0} clock_vs_audio={:.3}/{:.1}",
            self.map.meta.artist,
            self.map.meta.title,
            self.map.meta.version,
            wanted.len(),
            s.mean_error(),
            robust(&wanted),
            third(0),
            third(1),
            third(2),
            s.unstable_rate(),
            self.played_ms / 1000.0,
            sum / frames.max(1) as f64,
            most,
        ));
    }
}

#[derive(Clone, Copy)]
pub struct Windows {
    pub great: f64,
    pub ok: f64,
    pub miss: f64,
}

/// How wide the hit windows are made from the OD.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowRule {
    /// osu!'s: whole ms less a half (`beatmap::judged_window_ms`).
    #[default]
    Osu,
    /// Replays from before it (version 4 and below): the OD's widths as they come.
    Exact,
}

impl Windows {
    /// The windows plays are judged with.
    pub fn new(od: f64) -> Windows {
        Windows::with_rule(od, WindowRule::Osu)
    }

    pub fn with_rule(od: f64, rule: WindowRule) -> Windows {
        let width = match rule {
            WindowRule::Osu => beatmap::judged_window_ms,
            WindowRule::Exact => beatmap::window_ms,
        };
        Windows {
            great: width(od, 50.0, 35.0, 20.0),
            ok: width(od, 120.0, 80.0, 50.0),
            miss: width(od, 135.0, 95.0, 70.0),
        }
    }

    /// As wide as they are in real time at `rate`: they're timed in the song.
    pub fn at_rate(self, rate: f64) -> Windows {
        let rate = rate.max(0.01);
        Windows {
            great: self.great / rate,
            ok: self.ok / rate,
            miss: self.miss / rate,
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
        if anti_mash {
            Mash::Strict
        } else {
            Mash::Ignore
        }
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

struct NoteBlock {
    range: std::ops::Range<usize>,
    min_velocity: f64,
    last_end: f64,
    regular: bool,
}

impl NoteBlock {
    fn index(notes: &[beatmap::Note]) -> Vec<Self> {
        notes
            .chunks(128)
            .enumerate()
            .map(|(i, notes)| Self {
                range: i * 128..i * 128 + notes.len(),
                min_velocity: notes
                    .iter()
                    .map(|n| n.velocity)
                    .fold(f64::INFINITY, f64::min)
                    .max(1e-6),
                last_end: notes
                    .iter()
                    .map(|n| n.end_time)
                    .fold(f64::NEG_INFINITY, f64::max),
                regular: notes.iter().all(|n| {
                    matches!(n.kind, Kind::Don | Kind::Kat)
                        && n.fake_len == 0.0
                        && n.velocity.is_finite()
                        && n.velocity > 0.0
                        && n.end_time.is_finite()
                }),
            })
            .collect()
    }
}

pub struct Game {
    /// Random's configured colouring, kept with the replay so it plays back the same;
    /// 0 is the one per map from before seeds.
    pub seed: u64,
    /// Which notes Half took off.
    pub half: taiko_logic::mods::HalfRule,
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
    pub track: Arc<dyn Song>,
    /// The player asked for the results as soon as the last note was judged.
    outro_skipped: bool,
    first_note: f64,
    /// Game time of the last press per key: left rim, left centre, right centre, right rim.
    presses: [f64; 4],
    last_judge: Option<(Judge, f64)>,
    recent_errors: VecDeque<(f64, f64, Judge)>,
    strong: Option<StrongPending>,
    /// The beat length covering the most map time: what pop-up fake sliders fade in by.
    main_beat: f64,
    /// The client gimmick of a `.tko` map: visual motion only, never read by judging.
    tko: Option<crate::tko::runtime::Runtime>,
    /// Raw audio position seen by the last update, for diagnostics.
    pub last_audio_ms: Option<f64>,
    /// Over the frames with an audio position: how far the clock was from it (sum, frames, most
    /// either way).
    clock_vs_audio: (f64, u64, f64),
    pub raw_time: f64,
    /// How autoplay hits (Settings → Gameplay), and the keys that gives each note (by note index,
    /// empty for all but dons and kats), worked out once for those settings.
    pub autoplay_style: taiko_logic::autoplay::Settings,
    auto_plan: Option<(taiko_logic::autoplay::Settings, Vec<Vec<usize>>)>,
    /// Autoplay sounds already handed to the audio engine ahead of their hit: (note, key slot or
    /// roll tick). The hit itself then stays quiet.
    auto_sched: BTreeSet<(usize, usize)>,
    /// Pause menu cursor while paused.
    pub paused: Option<usize>,
    paused_at: Instant,
    /// Counting down to unpause: (started, ends). The menu is hidden meanwhile.
    pub resuming: Option<(Instant, Instant)>,
    /// Real milliseconds spent playing (not paused, nor counting down to resume, nor failed),
    /// for the profile's play time; and the frame they were last counted at.
    pub played_ms: f64,
    played_at: Option<Instant>,
    /// The map time of the frame before this one: a frame only lets notes go up to there (see
    /// `update`).
    last_frame_time: f64,
    /// The scroll speed setting this play is at: fixed when it starts (it counts for pp), and
    /// a replay's own.
    pub scroll_speed: f64,
    /// Calibration: play no hitsounds on presses.
    pub mute_hits: bool,
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
    /// A replay stopped where it is, with no pause menu (`toggle_held`).
    pub held: bool,
    pub inputs: Vec<(f64, u8)>,
    /// Playing a replay back: its presses and the next one due; and whose it is.
    pub replay: Option<(Vec<(f64, u8)>, usize)>,
    pub replay_of: Option<String>,
    /// The pp counter: works out the play's pp so far (`pp::Live`), and its latest answer.
    pub live_pp: Option<crate::pp::Live>,
    live_pp_value: Option<f64>,
    /// When set, every hitsound played is logged (map time, sound, volume), e.g. for video audio.
    pub sfx_log: Option<Vec<(f64, Sound, f32)>>,
    /// A miss broke a combo of 20 or more at this map time: the combo break sound is due.
    combo_break: Option<f64>,
    /// Scale the suggestion to current UR; off = plain average of the last 40 hits.
    pub ur_scaled_hint: bool,
    /// Offset used for the current frame / last press.
    offset: f64,
    /// The global offset as the player has it, on their scale (`Config::shown_offset`); set by
    /// the app every frame, for the advice to be given in it.
    pub shown_offset: f64,
    /// The output latency (ms) the play offset was worked out with (`Config::offset_latency_ms`);
    /// set by the app every frame. Autoplay hitsounds go this much later in the song than the
    /// clock's note time, so that they sound with the music as in osu!.
    pub latency: f64,
    /// Speed modifier: map milliseconds per real millisecond (1.5 = DT). Taken from the track,
    /// which was prepared at this rate. The game clock runs in track time; map time is
    /// `speed.map(track - offset)`, so the offset stays in real milliseconds.
    pub rate: f64,
    /// Map time ↔ song time: `rate` throughout, or ramping from it (Wind Up / Down).
    speed: Speed,
    /// Defense: our turns at the notes and the team's lives (None: playing alone).
    pub defense: Option<DefenseRun>,
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
    /// Seconds the FAILED sign stays before the play ends (shorter when it starts over by itself).
    pub fail_linger: f64,
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
    /// Replays: the seek bar's difficulty graph and the map time of every miss, worked out the
    /// first time the bar is drawn.
    pub replay_timeline: Option<(crate::difficulty::Graph, Vec<f64>)>,
    /// Drop / Barrel Roll: where each kiai starts (map ms) and the beat length there.
    kiai_starts: Vec<(f64, f64)>,
    /// Each note's rhythm: the stream it's part of (see `flows`).
    flows: Vec<u8>,
    /// Overtime: when (map ms) he shows up, and the next one due.
    scares: Vec<f64>,
    next_scare: usize,
    /// Shapeshift: its setup and the shape of each section (index into `SHAPESHIFT`).
    shapeshift: shapeshift::Config,
    /// Tact: how often notes move.
    tact: taiko_logic::tact::Step,
    shape_order: Vec<u8>,
    /// Multiverse's universes when the play brought its own (a lobby host's); None: the
    /// player's, as they set them up.
    universes: Option<[Universe; multiverse::COUNT]>,
}

/// Overtime: how long (ms) he stays on screen.
const SCARE_MS: f64 = 900.0;

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
/// Multiverse: how much of the screen's width a playfield spans in focus.
const FIELD_W: f32 = 0.78;
/// Multiverse: the playfield that has the focus first (near the middle).
pub(crate) const FIRST_FOCUS: usize = 4;

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
    const FULL: Focus = Focus {
        focus: 1.0,
        colour: 1.0,
        grow: 1.0,
        call: 0.0,
        burst: None,
        settle: 1.0,
        aim: 0.0,
    };
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
    /// A .tko map's shared scroll (mania-style SV), how far the game's clock is ahead of the
    /// chart's, and how far it has scrolled by now: notes go by how far the song scrolls before
    /// their hit instead of how long.
    shared: Option<(std::sync::Arc<crate::tko::scroll::ScrollTable>, f64, f64)>,
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
        let ahead = match &self.shared {
            Some((table, shift, now)) => table.x(time - shift) - now,
            None => time - self.t,
        };
        // Pulse / Stutter shift the time of far notes; near the drum the shift fades out, so
        // a note always reaches it exactly on time.
        let near = (ahead * v / self.lane).clamp(0.0, 1.0);
        let u = (ahead - self.warp * near) * v / self.lane;
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

/// 0 below `e0`, 1 above `e1`, smoothly in between.
fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    let x = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The bar map time `t` is in: its barline's index, start and length (the last bar is
/// `fallback` long). None before the first barline.
fn bar_at(bars: &[beatmap::Barline], t: f64, fallback: f64) -> Option<(usize, f64, f64)> {
    let i = bars.partition_point(|b| b.time <= t + 1.0).checked_sub(1)?;
    let start = bars[i].time;
    let len = bars
        .get(i + 1)
        .map_or(fallback, |b| b.time - start)
        .max(1.0);
    Some((i, start, len))
}
/// How far ahead of the clock (ms of song) autoplay hands a hitsound to the audio engine, on top
/// of the output latency: an audio buffer and a frame or two.
const AUTO_SOUND_LEAD_MS: f64 = 80.0;

/// Defense (a co-op lobby match): the team takes turns at the map's notes, `bars` bars each,
/// and every note one of them lets through costs the team one of its `lives`. What the team
/// does in the match (towers, slows, tag-ins, spotting) comes from the room in the same order
/// for everyone, and picks its notes by map time alone, so every player's game agrees.
pub struct DefenseRun {
    /// The team in turn order: (player id, name).
    pub team: Vec<(u32, String)>,
    /// Which of them we are.
    pub me: usize,
    pub lives: u32,
    bars: u32,
    /// The host's rules (tag-ins each, the shop...).
    pub rules: Defense,
    /// Whose turn each note is (into `team`).
    owner: Vec<u8>,
    /// Notes the map itself leaves unhittable (before a NaN slider): nobody's.
    map_unhittable: Vec<bool>,
    /// The room's numbers for the team (None until they first come).
    pub state: Option<TeamState>,
    /// Turns someone else took over: (turn, from bar, who).
    tags: Vec<(usize, usize, usize)>,
    towers: Vec<Tower>,
    /// Per note: the tower that takes it.
    shot: Vec<Option<u8>>,
    /// Map times slows were bought at.
    slows: Vec<f64>,
    /// What the team did lately: (map time, who, what).
    pub feed: Vec<(f64, usize, Done)>,
    /// When the team last lost a life (map time).
    lost_at: f64,
}

/// Defense: something a player of the team did, for the drawing to put in words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    Bought(DefenseItem),
    /// Took over the rest of a turn.
    TaggedIn,
}

/// Defense: a tower on a corner of the maze near the drum (`s` along the lane, in the middle of
/// the turn), shooting down every note of one colour due between `from` and `to`.
struct Tower {
    s: f32,
    side: f32,
    who: usize,
    kat: bool,
    from: f64,
    to: f64,
    /// The notes it takes.
    notes: Vec<usize>,
}

/// Defense: where the towers stand: beside the Maze's long straight into the drum (this far
/// out along it), a don tower above the lane and a kat tower below, where the player on turn
/// (zoomed in) sees them shoot.
const DON_TOWER_S: f32 = 250.0;
const KAT_TOWER_S: f32 = 470.0;
const TOWER_SIDE: f32 = LANE_H / 2.0 + 24.0;
/// Defense: a tower starts shooting at the first barline this long after it's bought, so the
/// notes it takes are still out on the lane before it.
const TOWER_LEAD_MS: f64 = 1500.0;
/// Defense: a tag-in takes over from the first barline this long after it (if that's still in
/// the turn), so the player before isn't cut off in the middle of a note.
const TAG_LEAD_MS: f64 = 1000.0;

/// Most ticks one roll keeps (`NoteState::ticks`).
const MAX_ROLL_TICKS: usize = 360_000;

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

/// `a` mixed `t` of the way to `b`.
pub(crate) fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |hex: u32, s: u32| ((hex >> s) & 0xff) as f32;
    [16, 8, 0].into_iter().fold(0, |out, s| {
        out | (((ch(a, s) + (ch(b, s) - ch(a, s)) * t).round() as u32) << s)
    })
}

/// Multiverse: where a playfield parks to be played (see `Game::draw_multiverse`): its left edge
/// (a part of the screen's width) and the middle of its lane, fully on screen and clear of the
/// score and HP bar.
fn parked(f: usize) -> (f32, f32) {
    (
        HOMES[f].0.min(0.97 - FIELD_W),
        HOMES[f].1.clamp(140.0, 350.0),
    )
}

/// Whether Random makes the note at `time` a don.
pub(crate) fn random_don(time: f64, seed: u64) -> bool {
    let salt = if seed == 0 {
        0.0
    } else {
        ((seed % 1_000_003) as f64 + 1.0) * 1.618
    };
    hash01(time + 0.5 + salt) < 0.5
}

impl Game {
    pub fn new(map: Beatmap, track: Arc<dyn Song>, audio: Option<&dyn Audio>) -> Game {
        Game::with_mods(map, track, audio, Mods::NONE)
    }

    /// A play with `mods` (the track is already at their speed, see `audio::change_speed`).
    pub fn with_mods(
        map: Beatmap,
        track: Arc<dyn Song>,
        audio: Option<&dyn Audio>,
        mods: Mods,
    ) -> Game {
        Game::with_seed(map, track, audio, mods, 0)
    }

    /// The same, with Random colouring the notes by `seed` (see `Game::seed`).
    pub fn with_seed(
        map: Beatmap,
        track: Arc<dyn Song>,
        audio: Option<&dyn Audio>,
        mods: Mods,
        seed: u64,
    ) -> Game {
        Game::with_half(map, track, audio, mods, seed, Default::default())
    }

    /// The same, with Half taking off the notes it did then (a replay's).
    pub fn with_half(
        mut map: Beatmap,
        track: Arc<dyn Song>,
        audio: Option<&dyn Audio>,
        mods: Mods,
        seed: u64,
        half: taiko_logic::mods::HalfRule,
    ) -> Game {
        apply_to_map(&mut map, mods, seed, half);
        let tko = map.gimmick.clone().map(|g| {
            let beats: Vec<crate::tko::runtime::BeatSection> = map
                .control_points
                .iter()
                .map(|c| crate::tko::runtime::BeatSection {
                    start: c.time,
                    origin: c.beat_origin,
                    length: c.beat_length,
                    kiai: c.kiai,
                })
                .collect();
            crate::tko::runtime::Runtime::with_beats(
                g,
                map.notes.iter().map(|n| crate::tko::runtime::NoteInfo {
                    time: n.time,
                    class: match n.kind {
                        Kind::Don => crate::tko::runtime::NoteClass::Don,
                        Kind::Kat => crate::tko::runtime::NoteClass::Kat,
                        _ => crate::tko::runtime::NoteClass::Long,
                    },
                    big: n.big,
                }),
                beatmap::OSU_TIMING_MS,
                &beats,
            )
        });
        let first_note = map.notes.first().map(|n| n.time).unwrap_or(0.0);
        let end_time = map.notes.iter().map(|n| n.end_time).fold(0.0, f64::max);
        let start = beatmap::play_start(first_note, map.meta.audio_lead_in);
        let rate = track.rate().clamp(0.25, 4.0);
        let speed = Speed::for_play(mods, rate, first_note, end_time);
        let track_ms = audio
            .map(|a| track.frames() as f64 / a.rate() as f64 * 1000.0)
            .unwrap_or(f64::INFINITY);
        // The pre-roll is in map time; the track plays `rate` map-ms per ms.
        let track_start = speed.track(start);
        if let Some(a) = audio {
            a.play(track.clone(), track_start);
        }
        let state = map
            .notes
            .iter()
            .map(|n| NoteState {
                // A fake slider has none (taiko_arranger's `drumroll_tick_times`).
                ticks: if n.kind == Kind::Roll && n.tick_spacing > 0.0 && !n.tiny {
                    // At most a tick every 10 ms of the longest roll: a tick spacing of next to
                    // nothing (a tiny beat length) mustn't ask for more than can be counted.
                    vec![
                        false;
                        (((n.end_time - n.time) / n.tick_spacing + 0.5).floor() as usize)
                            .min(MAX_ROLL_TICKS)
                            + 1
                    ]
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
        let main_beat = {
            let cps = &map.control_points;
            let mut spans: Vec<(f64, f64)> = Vec::new();
            for (i, cp) in cps.iter().enumerate() {
                let ms = (cps.get(i + 1).map_or(end_time, |n| n.time) - cp.time).max(0.0);
                match spans
                    .iter_mut()
                    .find(|s| (s.0 - cp.beat_length).abs() < 0.01)
                {
                    Some(s) => s.1 += ms,
                    None => spans.push((cp.beat_length, ms)),
                }
            }
            spans
                .into_iter()
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map_or(500.0, |s| s.0)
        };
        // osu!lazer's TaikoHealthProcessor.
        let hits = map
            .notes
            .iter()
            .filter(|n| matches!(n.kind, Kind::Don | Kind::Kat) && !n.unhittable)
            .count()
            .max(1) as f64;
        let hp_drain = mods.hp(map.meta.hp);
        let hp_hit = 1.0 / (3.0 * hits * beatmap::difficulty_range(hp_drain, 0.5, 0.75, 0.98));
        let hp_miss = beatmap::difficulty_range(hp_drain, 0.0018, 0.0075, 0.0120);
        let mut kiai_starts = Vec::new();
        let mut was_kiai = false;
        for cp in &map.control_points {
            if cp.kiai && !was_kiai {
                kiai_starts.push((cp.time, cp.beat_length));
            }
            was_kiai = cp.kiai;
        }
        let flows = flows(&map);
        // Overtime: once, plus once more per 75 s of map, anywhere from 8 s in to 4 s before the end.
        let scares = if mods.has(Mod::Overtime) {
            let span = (end_time - first_note - 12_000.0).max(0.0);
            let n = (1 + (span / 75_000.0) as usize).min(4);
            let mut at: Vec<f64> = (0..n)
                .map(|k| first_note + 8000.0 + span * hash01(first_note + k as f64 * 13.7 + 0.9))
                .collect();
            at.sort_by(f64::total_cmp);
            at
        } else {
            Vec::new()
        };
        let shapeshift = shapeshift::get();
        let shape_order = shape_order(&map, mods, shapeshift);
        Game {
            seed,
            half,
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
            outro_skipped: false,
            first_note,
            presses: [f64::NEG_INFINITY; 4],
            last_judge: None,
            recent_errors: VecDeque::new(),
            strong: None,
            main_beat,
            tko,
            last_audio_ms: None,
            clock_vs_audio: (0.0, 0, 0.0),
            raw_time: track_start,
            autoplay_style: taiko_logic::autoplay::Settings::default(),
            auto_plan: None,
            auto_sched: BTreeSet::new(),
            paused: None,
            paused_at: Instant::now(),
            resuming: None,
            played_ms: 0.0,
            played_at: None,
            last_frame_time: f64::NEG_INFINITY,
            scroll_speed: 1.0,
            mute_hits: false,
            mash: Mash::Nearest,
            scoring: Scoring::Standard,
            score_basis: None,
            combo_points: 0.0,
            extra_taps: 0,
            record: false,
            log_inputs: false,
            held: false,
            inputs: Vec::new(),
            replay: None,
            replay_of: None,
            live_pp: None,
            live_pp_value: None,
            sfx_log: None,
            combo_break: None,
            ur_scaled_hint: true,
            offset: 0.0,
            shown_offset: 0.0,
            latency: 0.0,
            rate,
            speed,
            mods,
            score_mult: mods.multiplier(),
            hp: 0.0,
            hp_hit,
            hp_miss,
            hp_exempt: false,
            failed: None,
            fail_linger: 1.6,
            last_hand: None,
            quake_at: f64::NEG_INFINITY,
            cracks: Vec::new(),
            crumbs: Vec::new(),
            misses: VecDeque::new(),
            replay_timeline: None,
            kiai_starts,
            flows,
            scares,
            next_scare: 0,
            shapeshift,
            tact: taiko_logic::tact::get(),
            shape_order,
            universes: None,
            defense: None,
        }
    }

    /// Plays with someone else's mod settings (a lobby host's): Multiverse's universes and
    /// Shapeshift's setup, each when given. The player's own config is left alone.
    pub fn use_mod_settings(
        &mut self,
        universes: Option<[Universe; multiverse::COUNT]>,
        shapeshift: Option<shapeshift::Config>,
    ) {
        if universes.is_some() {
            self.universes = universes;
        }
        if let Some(s) = shapeshift {
            self.shapeshift = s;
            self.shape_order = shape_order(&self.map, self.mods, s);
        }
    }

    /// Plays this map as one of a Defense team (`team` in turn order, we're `me`): the notes of
    /// the others' turns pass by without taking our presses or counting as our misses, and the
    /// team's lives (the room's), not HP, decide whether it fails.
    pub fn set_defense(&mut self, team: Vec<(u32, String)>, me: usize, rules: Defense) {
        let (lives, bars) = (rules.lives, rules.bars);
        let n = self.map.notes.len();
        let map_unhittable = self.map.notes.iter().map(|n| n.unhittable).collect();
        self.hp_exempt = true;
        self.defense = Some(DefenseRun {
            team,
            me,
            lives,
            bars,
            rules,
            owner: vec![0; n],
            map_unhittable,
            state: None,
            tags: Vec::new(),
            towers: Vec::new(),
            shot: vec![None; n],
            slows: Vec::new(),
            feed: Vec::new(),
            lost_at: f64::NEG_INFINITY,
        });
        self.refresh_defense();
    }

    /// Defense: whose turn bar `bar` is (a tag-in's, or the next in line).
    fn turn_of_bar(&self, bar: usize) -> usize {
        let Some(d) = &self.defense else { return 0 };
        let bars = d.bars.max(1) as usize;
        let turn = bar / bars;
        d.tags
            .iter()
            .find(|&&(t, from, _)| t == turn && bar >= from)
            .map_or(turn % d.team.len().max(1), |t| t.2)
    }

    /// The bar map time `t` is in (0 before the first barline; a note on a barline is the new
    /// bar's).
    fn bar_at(&self, t: f64) -> usize {
        self.map
            .barlines
            .partition_point(|b| b.time <= t + 1.0)
            .saturating_sub(1)
    }

    /// Defense: works out every note's owner again, and which ones take our presses: our own,
    /// unless a tower takes them.
    fn refresh_defense(&mut self) {
        let owners: Vec<u8> = self
            .map
            .notes
            .iter()
            .map(|n| self.turn_of_bar(self.bar_at(n.time)) as u8)
            .collect();
        let Some(d) = &mut self.defense else { return };
        d.owner = owners;
        for (i, n) in self.map.notes.iter_mut().enumerate() {
            if matches!(n.kind, Kind::Don | Kind::Kat) {
                n.unhittable =
                    d.map_unhittable[i] || d.owner[i] as usize != d.me || d.shot[i].is_some();
            }
        }
        self.score_basis = None;
    }

    /// Defense: whether note `i` is ours to hit (always, playing alone).
    fn mine(&self, i: usize) -> bool {
        self.defense
            .as_ref()
            .is_none_or(|d| d.owner[i] as usize == d.me)
    }

    /// Defense: a tower takes our note `i`, still to be judged: it counts as a GREAT for us.
    fn tower_takes(&self, i: usize) -> bool {
        self.defense
            .as_ref()
            .is_some_and(|d| d.shot[i].is_some() && d.owner[i] as usize == d.me)
            && self.state[i].judged.is_none()
    }

    /// Defense: the room's latest numbers for the team; it fails once its lives are gone.
    pub fn set_team(&mut self, state: TeamState) {
        let Some(d) = &mut self.defense else { return };
        if d.state.is_some_and(|old| state.lives < old.lives) {
            d.lost_at = self.time;
        }
        d.state = Some(state);
        if state.lives == 0 {
            self.fail(self.time);
        }
    }

    /// Defense: the team's lives left.
    pub fn lives_left(&self) -> Option<u32> {
        self.defense
            .as_ref()
            .map(|d| d.state.map_or(d.lives, |s| s.lives))
    }

    /// Defense: whose turn it is at map time `t` (into the team), and how many bars until ours
    /// next starts (0: it's ours now; past the end of the map, the bars left in it).
    pub fn defense_turn_at(&self, t: f64) -> Option<(usize, usize)> {
        let d = self.defense.as_ref()?;
        let bar = self.bar_at(t);
        let turn = self.turn_of_bar(bar);
        let reach = 2 * d.bars.max(1) as usize * d.team.len().max(1);
        let wait = (0..reach)
            .find(|&k| self.turn_of_bar(bar + k) == d.me)
            .unwrap_or(reach);
        Some((turn, wait))
    }

    /// Defense: we can take the turn being played at `t` over (it's someone else's, nobody took
    /// it over yet, and it has a barline left to start from).
    pub fn can_tag_in(&self, t: f64) -> bool {
        let Some(d) = &self.defense else { return false };
        let bars = d.bars.max(1) as usize;
        let (bar, from) = (self.bar_at(t), self.bar_at(t + TAG_LEAD_MS) + 1);
        self.turn_of_bar(bar) != d.me
            && from / bars == bar / bars
            && from < self.map.barlines.len()
            && !d.tags.iter().any(|g| g.0 == bar / bars)
            && self.tag_ins_left() > 0
    }

    /// Defense: the turns we may still take over this map.
    pub fn tag_ins_left(&self) -> u32 {
        self.defense.as_ref().map_or(0, |d| {
            let used = d.tags.iter().filter(|g| g.2 == d.me).count() as u32;
            d.rules.tag_ins.saturating_sub(used)
        })
    }

    /// Defense: the map time a tower bought at `at` stops shooting at (it starts at the first
    /// barline `TOWER_LEAD_MS` on and runs `TOWER_BARS` bars; infinity when the map runs out).
    pub fn tower_until(&self, at: f64) -> f64 {
        let bars = &self.map.barlines;
        let first = bars.partition_point(|b| b.time < at + TOWER_LEAD_MS);
        bars.get(first + TOWER_BARS)
            .map_or(f64::INFINITY, |b| b.time)
    }

    /// Defense: a tower is up (building or shooting) at map time `t`.
    pub fn tower_active(&self, t: f64) -> bool {
        self.defense
            .as_ref()
            .is_some_and(|d| d.towers.iter().any(|tw| t < tw.to))
    }

    /// Defense: something player `from` did, from the room (in the order it took them).
    pub fn defense_act(&mut self, from: u32, act: DefenseAct) {
        let Some(who) = self
            .defense
            .as_ref()
            .and_then(|d| d.team.iter().position(|p| p.0 == from))
        else {
            return;
        };
        let bars = self.defense.as_ref().map_or(1, |d| d.bars.max(1) as usize);
        let (at, done) = match act {
            DefenseAct::Buy { item, at, .. } => {
                match item {
                    DefenseItem::Slow => {
                        if let Some(d) = &mut self.defense {
                            d.slows.push(at);
                        }
                    }
                    DefenseItem::DonTower | DefenseItem::KatTower => {
                        let kat = item == DefenseItem::KatTower;
                        // `TOWER_BARS` bars from the first barline after the lead.
                        let bars = &self.map.barlines;
                        let first = bars.partition_point(|b| b.time < at + TOWER_LEAD_MS);
                        let from = bars.get(first).map_or(f64::INFINITY, |b| b.time);
                        let to = self.tower_until(at);
                        let notes = &self.map.notes;
                        let Some(d) = &mut self.defense else { return };
                        let k = d.towers.len() as u8;
                        let mut taken = Vec::new();
                        for (i, n) in notes.iter().enumerate() {
                            let colour = if kat { Kind::Kat } else { Kind::Don };
                            if n.kind == colour
                                && (from..to).contains(&n.time)
                                && !d.map_unhittable[i]
                                && d.shot[i].is_none()
                            {
                                d.shot[i] = Some(k);
                                taken.push(i);
                            }
                        }
                        // Beside the straight into the drum: dons above, kats below.
                        let side = if kat { TOWER_SIDE } else { -TOWER_SIDE };
                        d.towers.push(Tower {
                            s: if kat { KAT_TOWER_S } else { DON_TOWER_S },
                            side,
                            who,
                            kat,
                            from,
                            to,
                            notes: taken,
                        });
                        self.refresh_defense();
                    }
                    DefenseItem::Shield | DefenseItem::Heal => {}
                }
                (at, Done::Bought(item))
            }
            DefenseAct::TagIn { at } => {
                let (bar, from_bar) = (self.bar_at(at), self.bar_at(at + TAG_LEAD_MS) + 1);
                let Some(d) = &mut self.defense else { return };
                let turn = bar / bars;
                let used = d.tags.iter().filter(|g| g.2 == who).count();
                if from_bar / bars != turn
                    || d.tags.iter().any(|g| g.0 == turn)
                    || used >= d.rules.tag_ins as usize
                {
                    return;
                }
                d.tags.push((turn, from_bar, who));
                self.refresh_defense();
                (at, Done::TaggedIn)
            }
        };
        if let Some(d) = &mut self.defense {
            d.feed.push((at, who, done));
            if d.feed.len() > 4 {
                d.feed.remove(0);
            }
        }
    }

    /// Defense: the room's latest numbers for the team (all zero before the first).
    pub fn team_state(&self) -> Option<TeamState> {
        self.defense.as_ref().map(|d| {
            d.state.unwrap_or(TeamState {
                lives: d.lives,
                max: d.lives,
                ..TeamState::default()
            })
        })
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
        self.failed.is_some()
            || (!self.mods.has(Mod::NoFail)
                && !self.mods.has(Mod::Autoplay)
                && !self.hp_exempt
                && self.hp < 0.5)
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
            let n = self
                .map
                .notes
                .iter()
                .zip(&self.state)
                .filter(|(n, st)| {
                    matches!(n.kind, Kind::Don | Kind::Kat)
                        && !n.unhittable
                        && st.judged_at != f64::NEG_INFINITY
                })
                .count() as u32;
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
    pub fn update(&mut self, now: Instant, audio: Option<&dyn Audio>, offset: f64) -> bool {
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
        if self.paused.is_some() || self.held {
            self.clock.hold(now);
            self.time = self.speed.map(self.raw_time - offset);
            return false;
        }
        let audio_ms = audio.and_then(|a| a.position_ms(now));
        let raw = self.clock.update(now, audio_ms);
        self.last_audio_ms = audio_ms;
        if let Some(a) = audio_ms {
            let off = raw - a;
            self.clock_vs_audio = (
                self.clock_vs_audio.0 + off,
                self.clock_vs_audio.1 + 1,
                self.clock_vs_audio.2.max(off.abs()),
            );
        }
        self.raw_time = raw;
        self.time = self.speed.map(raw - offset);
        // Failed: the lane keeps scrolling under the FAILED sign, but nothing more is judged.
        if let Some((_, at)) = self.failed {
            return now.duration_since(at).as_secs_f64() > self.fail_linger;
        }
        self.feed_replay(audio);
        // Up to the frame before only: a press stamped before this frame may still be waiting for
        // the state lock while it's built, and it has to find its note still there. Presses let
        // notes go at their own time (`judge_press`), so they're judged exactly either way.
        let upto = self.last_frame_time.min(self.time);
        self.last_frame_time = self.time;
        self.expire(upto);
        self.play_combo_break(audio);
        self.play_scares(audio);
        self.outro_skipped || self.time > self.end_time + 1500.0 || raw > self.track_ms + 500.0
    }

    /// Whether every note has been judged, so all that's left is the wait for the results. Not in
    /// a replay, which plays out, or a failed play, which has its own ending.
    pub fn can_skip_outro(&self) -> bool {
        self.cursor >= self.map.notes.len()
            && !self.map.notes.is_empty()
            && self.replay.is_none()
            && self.failed.is_none()
    }

    /// Tests: every note goes by, as if nothing was hit.
    #[cfg(test)]
    pub(crate) fn run_out_of_notes(&mut self) {
        self.expire(self.end_time + 1000.0);
    }

    /// Ends the play on the next update instead of waiting out the last moments of the song; true
    /// when it did.
    pub fn skip_outro(&mut self) -> bool {
        let skipped = self.can_skip_outro();
        self.outro_skipped |= skipped;
        skipped
    }

    /// The keys autoplay hits each note with (`autoplay::note_keys`), worked out again only when
    /// its settings change. Groups and rests are judged against the map's main beat: a gimmick's
    /// 60000 BPM line would make every gap a rest. Unhittable notes aren't played.
    fn auto_keys(&mut self) -> &[Vec<usize>] {
        let style = self.autoplay_style;
        if self.auto_plan.as_ref().is_none_or(|(s, _)| *s != style) {
            let hits: Vec<usize> = (0..self.map.notes.len())
                .filter(|&i| {
                    matches!(self.map.notes[i].kind, Kind::Don | Kind::Kat)
                        && !self.map.notes[i].unhittable
                })
                .collect();
            let notes: Vec<(f64, bool, bool)> = hits
                .iter()
                .map(|&i| {
                    (
                        self.map.notes[i].time,
                        self.map.notes[i].kind == Kind::Kat,
                        self.map.notes[i].big,
                    )
                })
                .collect();
            let mut plan = vec![Vec::new(); self.map.notes.len()];
            for (&i, keys) in hits.iter().zip(taiko_logic::autoplay::note_keys(
                &notes,
                self.main_beat,
                style,
            )) {
                plan[i] = keys;
            }
            self.auto_plan = Some((style, plan));
        }
        &self.auto_plan.as_ref().unwrap().1
    }

    /// Autoplay: hits every object exactly on time, with the hands its playstyle says.
    pub fn autoplay(&mut self, audio: Option<&dyn Audio>) {
        let t = self.time;
        let running =
            self.paused.is_none() && self.resuming.is_none() && !self.held && self.failed.is_none();
        if !running {
            self.auto_sched.clear();
        } else if let Some(a) = audio.filter(|_| !self.mute_hits) {
            // The hitsound goes into the stream where its note is in the music: that moment is
            // `latency` ahead of what's audible now (and the clock), plus a buffer and a frame.
            let lead = a.latencies_ms().0 + AUTO_SOUND_LEAD_MS;
            for (song_ms, sfx, volume) in self.plan_auto_sounds(lead) {
                a.sfx_at_song(sfx, volume, song_ms);
            }
        }
        for i in self.cursor..self.map.notes.len() {
            let n = &self.map.notes[i];
            if n.time > t {
                break;
            }
            let (time, kind) = (n.time, n.kind);
            let st = &self.state[i];
            match kind {
                Kind::Don | Kind::Kat if st.judged.is_none() && !n.unhittable => {
                    for (j, key) in self.auto_keys()[i].clone().into_iter().enumerate() {
                        self.auto_hit(key, time, audio, (i, j));
                    }
                }
                // A roll mapped across notes is played as the notes alone (owner, 2026-10-28):
                // a tick press would land in a note's window and hit it early, or miss a kat.
                // A fake slider (written under 0.1 long, any negative) isn't played at all (owner,
                // 2026-10-28).
                Kind::Roll if !st.finished && !n.tiny && !self.roll_over_notes(i) => {
                    let spacing = self.map.notes[i].tick_spacing;
                    let k = ((t - time) / spacing).floor().max(0.0) as usize;
                    if self.state[i].ticks.get(k).is_some_and(|hit| !hit) {
                        let key =
                            taiko_logic::autoplay::roll_key(k, self.autoplay_style.left_handed);
                        self.auto_hit(key, time + k as f64 * spacing, audio, (i, k));
                    }
                }
                // One swell at a time (`judge_press` gives the earliest every press), at most every
                // 40 ms: a swell too rapid for that goes unfinished (owner, 2026-10-28). A press
                // that would take a note under it early waits for the note's own hit instead.
                Kind::Swell if !st.finished => {
                    let next_kat = st.last_was_kat != Some(true);
                    if t - self
                        .presses
                        .iter()
                        .cloned()
                        .fold(f64::NEG_INFINITY, f64::max)
                        > 40.0
                        && self.note_under_swell(t, Some(next_kat)).is_none()
                    {
                        self.auto_hit(if next_kat { 0 } else { 1 }, t, audio, (i, usize::MAX));
                    }
                }
                _ => {}
            }
        }
    }

    /// Autoplay sounds to start ahead of their hits: every hit within `lead` ms of song time of
    /// the clock, once, as (song ms to start at, sound, volume). Swell presses are decided on the
    /// fly and aren't planned. Judging is untouched: each hit still lands at its note's time.
    fn plan_auto_sounds(&mut self, lead: f64) -> Vec<(f64, Sound, f32)> {
        let mut out = Vec::new();
        let horizon = self.speed.track(self.time) + lead;
        self.auto_sched.retain(|&(i, _)| i >= self.cursor);
        for i in self.cursor..self.map.notes.len() {
            let n = &self.map.notes[i];
            if self.speed.track(n.time) > horizon {
                break;
            }
            let (time, kind, big) = (n.time, n.kind, n.big);
            let st = &self.state[i];
            match kind {
                Kind::Don | Kind::Kat if st.judged.is_none() && !n.unhittable => {
                    let keys = self.auto_keys()[i].clone();
                    for (j, &key) in keys.iter().enumerate() {
                        if !self.auto_sched.insert((i, j)) {
                            continue;
                        }
                        // The second hand's press of a big note is the finisher (`judge_press`).
                        let finisher = big && j > 0 && (keys[j - 1] < 2) != (key < 2);
                        let sfx = match (kind == Kind::Kat, finisher) {
                            (false, false) => Sound::Don,
                            (false, true) => Sound::BigDon,
                            (true, false) => Sound::Kat,
                            (true, true) => Sound::BigKat,
                        };
                        out.push((
                            self.speed.track(time) + self.offset + self.latency,
                            sfx,
                            self.volume_at(time),
                        ));
                    }
                }
                Kind::Roll if !st.finished && !n.tiny && !self.roll_over_notes(i) => {
                    let spacing = self.map.notes[i].tick_spacing;
                    // Too rapid to queue ticks for; they sound as they are hit.
                    if spacing < 10.0 {
                        continue;
                    }
                    let first = ((self.time - time) / spacing).floor().max(0.0) as usize;
                    for k in first..first + 16 {
                        let at = time + k as f64 * spacing;
                        if self.speed.track(at) > horizon
                            || self.state[i].ticks.get(k).is_none_or(|&hit| hit)
                        {
                            break;
                        }
                        if self.auto_sched.insert((i, k)) {
                            let key =
                                taiko_logic::autoplay::roll_key(k, self.autoplay_style.left_handed);
                            let sfx = if key == 0 || key == 3 {
                                Sound::Kat
                            } else {
                                Sound::Don
                            };
                            let song_ms = self.speed.track(at) + self.offset + self.latency;
                            out.push((song_ms, sfx, self.volume_at(at)));
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Forgets the autoplay sounds handed to the audio engine (it drops them when the song moves
    /// or pauses), so that they are planned again.
    fn drop_scheduled(&mut self) {
        self.auto_sched.clear();
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

    /// `id`: the note and key slot (or roll tick) this press is, to tell whether its sound went
    /// to the engine ahead of time.
    fn auto_hit(&mut self, key: usize, t: f64, audio: Option<&dyn Audio>, id: (usize, usize)) {
        if self.record || self.log_inputs {
            self.inputs.push((t, key as u8));
        }
        self.presses[key] = t;
        let sfx = self.judge_press(key, key == 0 || key == 3, t);
        let planned = self.auto_sched.remove(&id);
        if let Some(a) = audio.filter(|_| !planned && !self.mute_hits) {
            a.sfx_at(sfx, self.volume_at(t));
        }
    }

    /// Hitsound volume at map time `t`: the timing point's, like osu!.
    fn volume_at(&self, t: f64) -> f32 {
        beatmap::control_point_at(&self.map.control_points, t).volume
    }

    /// Plays (and logs) the combo break sound when a miss just broke a long combo.
    fn play_combo_break(&mut self, audio: Option<&dyn Audio>) {
        let Some(t) = self.combo_break.take() else {
            return;
        };
        if let Some(log) = &mut self.sfx_log {
            log.push((t, Sound::ComboBreak, COMBO_BREAK_GAIN));
        }
        if let Some(a) = audio.filter(|_| !self.mute_hits) {
            a.sfx_at(Sound::ComboBreak, COMBO_BREAK_GAIN);
        }
    }

    /// Overtime: screams for each scare whose time has come (and logs it).
    fn play_scares(&mut self, audio: Option<&dyn Audio>) {
        while let Some(&at) = self
            .scares
            .get(self.next_scare)
            .filter(|&&at| at <= self.time)
        {
            self.next_scare += 1;
            // One skipped over (a seek) stays quiet.
            if self.time - at > SCARE_MS {
                continue;
            }
            if let Some(log) = &mut self.sfx_log {
                log.push((at, Sound::Scream, 1.0));
            }
            if let Some(a) = audio.filter(|_| !self.mute_hits) {
                a.sfx_at(Sound::Scream, 1.0);
            }
        }
    }

    /// Stops a replay where it is, or sets it going again.
    pub fn toggle_held(&mut self, audio: Option<&dyn Audio>) {
        self.held = !self.held;
        self.drop_scheduled();
        if let Some(a) = audio {
            a.set_paused(self.held);
        }
    }

    /// Whether the music can be swapped for one at another constant speed.
    pub fn has_constant_speed(&self) -> bool {
        self.speed.is_constant()
    }

    /// Plays on from where it is with `track`, made at `rate`. Everything is judged in map time,
    /// so this changes how fast it goes by and nothing else.
    pub fn change_speed(
        &mut self,
        track: Arc<dyn Song>,
        rate: f64,
        audio: Option<&dyn Audio>,
        offset: f64,
    ) {
        self.speed = Speed::constant(rate);
        let target = self.speed.track(self.time) + offset;
        if let Some(a) = audio {
            self.track_ms = track.frames() as f64 / a.rate() as f64 * 1000.0;
            a.play(track.clone(), target);
            a.set_paused(self.held || self.paused.is_some());
        }
        self.drop_scheduled();
        self.track = track;
        self.clock = GameClock::new(target, audio.is_some());
        self.raw_time = target;
    }

    pub fn pause(&mut self, audio: Option<&dyn Audio>) {
        self.drop_scheduled();
        // Pausing again (e.g. losing focus) during the resume countdown shows the menu again.
        self.cancel_resume();
        // The menu's Continue is what sets it going now.
        self.held = false;
        if self.paused.is_none() {
            self.paused = Some(0);
            self.paused_at = Instant::now();
            if let Some(a) = audio {
                a.set_paused(true);
            }
        }
    }

    pub fn resume(&mut self, audio: Option<&dyn Audio>) {
        self.drop_scheduled();
        self.resuming = None;
        if self.paused.take().is_some()
            && let Some(a) = audio
        {
            a.set_paused(false);
        }
    }

    /// Leaves the pause menu: unpauses after `delay` (a countdown over the frozen playfield), or
    /// at once with no delay.
    pub fn resume_after(&mut self, delay: std::time::Duration, audio: Option<&dyn Audio>) {
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
        if s.errors.len() < MIN_HITS {
            return None;
        }
        // An error measured under offset `o` would have been `e - (current - o)` under the current one.
        let from = s.errors.len().saturating_sub(MAX_HITS);
        let corrected: Vec<f64> = s.errors[from..]
            .iter()
            .zip(&s.error_offsets[from..])
            .map(|(e, o)| e - (current_offset - o))
            .collect();
        let n = corrected.len();
        if !self.ur_scaled_hint {
            // Simple mode: average of the last 40 hits, applied in full.
            let estimate = calibrate::estimate(&corrected[n.saturating_sub(40)..], current_offset)?;
            let significant = estimate.error_ms.abs() >= 3.0;
            return Some(OffsetHint {
                estimate,
                needed: MIN_HITS,
                significant,
            });
        }
        // Current consistency from the latest 40 hits sizes the window.
        let probe = calibrate::estimate(&corrected[n.saturating_sub(40)..], current_offset)?;
        let sigma = probe.ur / 10.0;
        let needed = ((1.96 * sigma / TARGET_MARGIN_MS).powi(2).ceil() as usize)
            .clamp(MIN_HITS * 2, MAX_HITS);
        let mut estimate =
            calibrate::estimate(&corrected[n.saturating_sub(needed)..], current_offset)?;
        let se = estimate.margin_ms / 1.96;
        let shrink = PRIOR_MS.powi(2) / (PRIOR_MS.powi(2) + se.powi(2));
        estimate.suggested_offset = (current_offset + estimate.error_ms * shrink).round();
        let significant = estimate.error_ms.abs() > estimate.margin_ms.max(2.0);
        Some(OffsetHint {
            estimate,
            needed,
            significant,
        })
    }

    /// The global offset, on the player's scale, that makes up for `suggested`, an `OffsetHint`'s
    /// offset (the one the clock takes off, global and map's): positive ones bring the notes
    /// earlier, so wanting the clock's offset lowered by e means raising this one by e.
    pub fn suggested_shown_offset(&self, suggested: f64) -> f64 {
        (self.shown_offset - (suggested - self.offset)).round()
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
    pub fn start_from(&mut self, t: f64, audio: Option<&dyn Audio>, offset: f64) {
        let start = self.speed.map(self.speed.track(t) - 2000.0).max(self.time);
        let target = self.speed.track(start) + offset;
        self.hp_exempt = true;
        self.drop_scheduled();
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
    pub fn seek(&mut self, t: f64, audio: Option<&dyn Audio>, offset: f64) {
        let t = t.clamp(self.start, self.end_time);
        if t < self.time {
            self.rewind();
        }
        self.offset = offset;
        self.set_time(t);
        let target = self.speed.track(t) + offset;
        self.drop_scheduled();
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
            *st = NoteState {
                ticks: vec![false; st.ticks.len()],
                ..Default::default()
            };
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

    /// Replays: the map time of every note the whole replay misses, judged ahead from the
    /// recorded presses; play is put back where it was.
    pub fn replay_misses(&mut self) -> Vec<f64> {
        if self.replay.is_none() {
            return Vec::new();
        }
        let now = self.time;
        self.rewind();
        self.set_time(self.end_time);
        let misses = (self.map.notes.iter().zip(&self.state))
            .filter(|(_, st)| st.judged == Some(Judge::Miss))
            .map(|(n, _)| n.time)
            .collect();
        self.rewind();
        self.set_time(now);
        misses
    }

    /// Map time of the start and end of play, for a timeline.
    pub fn span(&self) -> (f64, f64) {
        (self.start, self.end_time)
    }

    pub fn can_skip(&self) -> bool {
        self.first_note - self.time > 3000.0
    }

    /// Skips the intro; true when it did.
    pub fn skip(&mut self, audio: Option<&dyn Audio>, offset: f64) -> bool {
        let skipped = self.can_skip();
        if skipped {
            let target = self.speed.track(self.skip_target()) + offset;
            self.drop_scheduled();
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

    /// How long ago (ms) a `.tko` graph's trigger node last fired, for the editor to light it.
    pub fn tko_fired_ago(&self, id: u32) -> Option<f64> {
        self.tko.as_ref()?.fired_ago(id, self.time)
    }

    fn expire(&mut self, t: f64) {
        for i in self.cursor..self.map.notes.len() {
            let n = &self.map.notes[i];
            if n.time > t {
                break;
            }
            // A tower takes it for us: a GREAT, on time.
            if self.tower_takes(i) {
                let at = n.time;
                self.apply(i, Judge::Great, at, None);
                continue;
            }
            let st = &mut self.state[i];
            match n.kind {
                // One before a NaN slider is never judged, not even a miss (owner, 2026-10-28).
                Kind::Don | Kind::Kat => {
                    if st.judged.is_none() && !n.unhittable && t - n.time > self.windows.ok {
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
        if self
            .strong
            .as_ref()
            .is_some_and(|s| t - s.at > STRONG_WINDOW)
        {
            self.strong = None;
        }
        while self.cursor < self.map.notes.len() {
            let st = &self.state[self.cursor];
            let n = &self.map.notes[self.cursor];
            let done = match n.kind {
                Kind::Don | Kind::Kat => {
                    st.judged.is_some() || (n.unhittable && t - n.time > self.windows.ok)
                }
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
        let judge = if judge == Judge::Ok && self.mods.has(Mod::GreatOrNothing) {
            Judge::Miss
        } else {
            judge
        };
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
            self.hp = (self.hp
                + if judge == Judge::Miss {
                    -self.hp_miss
                } else {
                    self.hp_hit * if judge == Judge::Great { 3.0 } else { 1.1 }
                })
            .clamp(0.0, 1.0);
        }
        if (judge == Judge::Miss && self.mods.has(Mod::SuddenDeath))
            || (judge != Judge::Great && self.mods.has(Mod::Perfect))
        {
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
        // Tell a .tko graph what happened; it only ever draws from it.
        if let Some(r) = self.tko.as_mut() {
            use crate::tko::runtime::Verdict;
            r.event(
                t,
                i,
                match judge {
                    Judge::Great => Verdict::Great,
                    Judge::Ok => Verdict::Ok,
                    Judge::Miss => Verdict::Miss,
                },
                self.stats.combo,
                self.stats.accuracy(),
            );
        }
        if self.scoring == Scoring::Standard {
            self.rescore(judge, combo_before);
        }
    }

    /// Handles a drum key (0 = left rim, 1 = left centre, 2 = right centre, 3 = right rim).
    pub fn press(&mut self, key: usize, at: Instant, audio: Option<&dyn Audio>, offset: f64) {
        let t = self.speed.map(self.clock.at(at) - offset);
        self.offset = offset;
        self.press_time(key, t, audio);
    }

    /// A live press at map time `t`.
    fn press_time(&mut self, key: usize, t: f64, audio: Option<&dyn Audio>) {
        if self.failed.is_some() {
            return;
        }
        // Alternate: the same hand twice in a row doesn't count (after a 2 s pause either may
        // start). Drumrolls and swells take either hand, as fast as you like.
        let alternate = self.mods.has(Mod::Alternate);
        let left = key < 2;
        let free_run = alternate
            && (self.find_active(t, Kind::Roll).is_some()
                || self.find_active(t, Kind::Swell).is_some());
        if alternate
            && !free_run
            && self
                .last_hand
                .is_some_and(|(hand, at)| hand == left && t - at < 2000.0)
        {
            return;
        }
        if self.record || self.log_inputs {
            self.inputs.push((t, key as u8));
        }
        let hits = self.stats.great + self.stats.ok;
        let sfx = self.press_at(key, t, audio);
        if alternate {
            let hit = self.stats.great + self.stats.ok > hits;
            let finisher = hit && matches!(sfx, Sound::BigDon | Sound::BigKat);
            // A finisher counts as two notes, so either hand may hit next; after a drumroll or
            // a swell, either hand may until one hits a note.
            self.last_hand = if free_run || finisher || (self.last_hand.is_none() && !hit) {
                None
            } else {
                Some((left, t))
            };
        }
    }

    /// A press judged at map time `t` (live, or from a replay); the sound it makes.
    fn press_at(&mut self, key: usize, t: f64, audio: Option<&dyn Audio>) -> Sound {
        let kat = if self.mods.has(Mod::Relax) {
            self.relax_colour(t, key == 0 || key == 3)
        } else {
            key == 0 || key == 3
        };
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

    /// Judges with the hit windows `rule` makes (a replay's own, see `Replay::window_rule`).
    pub fn set_window_rule(&mut self, rule: WindowRule) {
        self.windows = Windows::with_rule(self.mods.od(self.map.meta.od), rule);
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
    fn feed_replay(&mut self, audio: Option<&dyn Audio>) {
        let Some((inputs, mut next)) = self.replay.take() else {
            return;
        };
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
        if let Some(s) = self
            .strong
            .as_ref()
            .filter(|s| (t - s.at).abs() <= STRONG_WINDOW)
        {
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

    fn judge_press(&mut self, key: usize, kat: bool, t: f64) -> Sound {
        let normal = if kat { Sound::Kat } else { Sound::Don };
        let big = if kat { Sound::BigKat } else { Sound::BigDon };
        self.expire(t);

        if let Some(s) = &self.strong {
            let other_hand = (s.key < 2) != (key < 2);
            if s.kat == kat
                && other_hand
                && (t - s.at).abs() <= STRONG_WINDOW
                && self.state[s.note].judged.is_some()
            {
                self.strong = None;
                self.stats.strong += 1;
                self.add_score(300.0);
                self.quake_at = t;
                return big;
            }
        }

        // An active swell (the earliest, one at a time) takes every hit alternated centre/rim and
        // swallows the rest, except one that hits a note under it: that one goes on to the note,
        // and counts for the swell too if it swaps colour (owner, 2026-10-28).
        if let Some(i) = self.find_active(t, Kind::Swell) {
            let to_note = self.note_under_swell(t, Some(kat)).is_some();
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
            if !to_note {
                return normal;
            }
        }

        if let Some(i) = self.next_hittable(t) {
            let hit = match self.mash {
                Mash::Nearest => self.judge_nearest(i, kat, t),
                legacy => {
                    let n = &self.map.notes[i];
                    let wrong = (n.kind == Kind::Kat) != kat;
                    let judge = if wrong {
                        Judge::Miss
                    } else {
                        self.timing(t - n.time)
                    };
                    (judge != Judge::Miss || legacy == Mash::Strict).then_some((i, judge))
                }
            };
            if let Some((i, judge)) = hit {
                let n = &self.map.notes[i];
                let is_big = n.big;
                self.apply(i, judge, t, Some(t - n.time));
                if judge != Judge::Miss && is_big {
                    self.strong = Some(StrongPending {
                        note: i,
                        kat,
                        key,
                        at: t,
                    });
                    return big;
                }
                return normal;
            }
        }

        if let Some(i) = self.find_active(t, Kind::Roll) {
            let n = &self.map.notes[i];
            let k = ((t - n.time) / n.tick_spacing).round().max(0.0) as usize;
            if let Some(tick) = self.state[i].ticks.get_mut(k)
                && !*tick
            {
                *tick = true;
                self.stats.roll_ticks += 1;
                let points = if n.big { 200.0 } else { 100.0 };
                self.add_score(points);
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
        let skipped_to = self
            .next_unjudged(i, t)
            .filter(|&j| colour(j) && err(j).abs() <= ok && err(j).abs() < err(i).abs());
        let extra = self
            .prev_judged(i, t)
            .is_some_and(|p| err(p).abs() < err(i).abs());
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
            if matches!(n.kind, Kind::Don | Kind::Kat)
                && self.state[k].judged.is_none()
                && !n.unhittable
            {
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
            // An unhittable note (before a NaN slider) lets presses through and is never judged.
            if matches!(n.kind, Kind::Don | Kind::Kat)
                && self.state[i].judged.is_none()
                && !n.unhittable
            {
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
            let slack = if kind == Kind::Roll {
                n.tick_spacing / 2.0
            } else {
                0.0
            };
            if n.time - slack > t {
                return None;
            }
            if n.kind == kind && !self.state[i].finished && t <= n.end_time + slack && self.mine(i)
            {
                return Some(i);
            }
        }
        None
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
        let overlap = |a: f32| -> f32 {
            self.crumbs
                .iter()
                .map(|c| ((a + part).min(c.start + c.len / 900.0) - a.max(c.start)).max(0.0))
                .sum()
        };
        let start = (0..5)
            .map(|k| lo + (hi - lo - part).max(0.0) * hash01(n.time + k as f64 * 3.7 + 0.19) as f32)
            .min_by(|&a, &b| overlap(a).total_cmp(&overlap(b)))
            .unwrap_or(lo);
        let side = if hash01(n.time + 0.71) < 0.5 {
            -1.0
        } else {
            1.0
        };
        let shift = side * (4.0 + 30.0 * size) * (0.8 + 0.2 * hash01(n.time + 0.53) as f32);
        self.crumbs.push(Crumb {
            at: t,
            life: 350.0 + 1850.0 * size as f64,
            size,
            start,
            len,
            shift,
        });
    }

    /// Crumble's knocked-out pieces at map time `t` on a lane `len` long, like `cracks_at`'s:
    /// each snaps out and holds, then settles back over the rest of its life, or all at once
    /// when a miss breaks the combo.
    fn crumbs_at(&self, t: f64, len: f32) -> impl Iterator<Item = (f32, f32, f32, f32)> + '_ {
        let miss = self.misses.iter().rev().copied().find(|&m| m <= t);
        self.crumbs
            .iter()
            .filter(move |c| c.at <= t && t < c.at + c.life)
            .map(move |c| {
                let x = (t - c.at) / c.life;
                let mend = miss
                    .filter(|&m| m > c.at)
                    .map_or(1.0, |m| 1.0 - smooth(m, m + CRUMBLE_MEND_MS, t));
                let m =
                    (smooth(0.0, 70.0 / c.life, x) * (1.0 - smooth(0.35, 1.0, x)) * mend) as f32;
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
                Some((
                    len * (0.3 + 0.55 * hash01(at + 0.11) as f32),
                    50.0 + 80.0 * hash01(at + 0.23) as f32,
                    shift,
                    0.9,
                ))
            })
            .chain(self.crumbs_at(t, len))
            .collect()
    }
}

#[cfg(test)]
mod tests;
