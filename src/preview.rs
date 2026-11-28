//! Song select preview: the selected map's background, its music from the preview point, and a
//! live autoplay of the selected difficulty with its details, all loaded in the background.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::audio::{self, Engine, Track};
use crate::beatmap::{self, Beatmap, Kind};
use crate::difficulty::{self, Graph};
use crate::game::{self, Game, Play};
use crate::hit::{Canvas, Hit};
use crate::library::Entry;
use crate::render::{VIRTUAL_HEIGHT, accent, rgba};
use crate::services::jobs::Job;
use crate::services::workers::Ticket;

mod classic;

/// While scrolling, wait this long after the selection settles before loading, so scrolling
/// stays cheap. A single change after a quiet moment loads at once.
const DEBOUNCE: Duration = Duration::from_millis(60);
const QUIET: Duration = Duration::from_millis(300);
/// A part of the bare details coming in (`Preview::enter`): how long it takes and how far right
/// of its place it starts.
const ENTER_MS: f32 = 180.0;
const ENTER_SHIFT: f32 = 8.0;
/// When the pp row (and the Leaderboard chip ending it) comes in after a song is picked.
const PP_ENTER_MS: f32 = 260.0;
/// The details of the song before a change go out as the new ones come in: `LEAVE_SHIFT`
/// to the left and faded out over `LEAVE_MS`.
const LEAVE_MS: f32 = 80.0;
const LEAVE_SHIFT: f32 = 6.0;

/// What the bare details drew in a frame (their shapes, and the clips among them from shape
/// 0), to go out as drawn when the song changes.
#[derive(Default)]
struct Drawn {
    shapes: Vec<crate::render::Shape>,
    clips: Vec<(std::ops::Range<usize>, [f32; 4])>,
}
/// How quickly a shown rating or pp counts to a new song's (`follow`, a time constant in ms).
const COUNT_TAU: f32 = 60.0;

/// `shown` following `target`: counting to it (`COUNT_TAU`), there at once from nothing or
/// without animations. With no target it keeps its value, for the next one to count from, and
/// gives nothing.
fn follow(shown: &mut Option<f32>, target: Option<f32>, dt_ms: f32) -> Option<f32> {
    let t = target?;
    match shown {
        Some(v) if crate::render::animations() => {
            crate::render::approach(v, t, dt_ms, COUNT_TAU);
            if (*v - t).abs() < 0.005 {
                *v = t;
            }
        }
        _ => *shown = Some(t),
    }
    *shown
}

/// `follow` for the three full-combo pp values.
fn follow_pp(
    shown: &mut Option<[f64; 3]>,
    target: Option<[f64; 3]>,
    dt_ms: f32,
) -> Option<[f64; 3]> {
    let t = target?;
    match shown {
        Some(v) if crate::render::animations() => {
            let k = 1.0 - (-f64::from(dt_ms) / f64::from(COUNT_TAU)).exp();
            for (v, t) in v.iter_mut().zip(t) {
                *v += (t - *v) * k;
                if (*v - t).abs() < 0.5 {
                    *v = t;
                }
            }
        }
        _ => *shown = Some(t),
    }
    *shown
}
/// The width (virtual units, 480 tall) the preview lays the playfield out at: a 4:3 screen's,
/// widening up to a 16:9 screen's as the panel is dragged wider than its default.
const FOUR_THREE: f32 = 640.0;
const SIXTEEN_NINE: f32 = 480.0 * 16.0 / 9.0;

/// The preview's scale at full size and the lane width (virtual units) it shows, for a panel
/// `w` wide whose default width is `base_w`. Up to the default it is a 4:3 lane scaled to fit;
/// wider, the lane keeps the default's height and shows more of itself, up to 16:9, then scales.
fn lane_fit(w: f32, base_w: f32) -> (f32, f32) {
    let k = (w - 20.0) / FOUR_THREE;
    let base = (base_w - 20.0) / FOUR_THREE;
    if k <= base || base <= 0.0 {
        return (k, FOUR_THREE);
    }
    let lane_w = ((w - 20.0) / base).min(SIXTEEN_NINE);
    ((w - 20.0) / lane_w, lane_w)
}

/// The preview music starts from a quick decode of this much audio at the preview point; the
/// full song replaces it a moment later without a gap.
const SNIPPET_MS: f64 = 15_000.0;

pub type Image = crate::render::BgImage;

/// Summary stats of the map currently loaded in `Preview`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LoadedStats {
    pub bpm: f64,
    pub length_ms: f64,
    pub dons: u32,
    pub kats: u32,
    pub rolls: u32,
    pub swells: u32,
}

/// Decoded music for the selected set: all of it, or (until the full decode arrives) the part
/// from the preview point, which belongs at song frame `base`.
struct Song {
    path: PathBuf,
    track: Arc<Track>,
    base: i64,
    full: bool,
    /// Where the preview starts (track ms).
    start: f64,
    /// Speed it was prepared for (the track plays `rate` map ms per ms) and how.
    rate: f64,
    keep_pitch: bool,
}

impl Song {
    /// End of the decoded audio, in song ms.
    fn end_ms(&self, rate: u32) -> f64 {
        (self.base + self.track.frames() as i64) as f64 / rate as f64 * 1000.0
    }
}

/// A decode result: (track, base frame, whether it is the whole song), or None if it failed.
type Decoded = (PathBuf, Option<(Arc<Track>, i64, bool)>);

#[derive(Clone, Copy)]
pub struct Details {
    pub bpm: (f64, f64),
    pub length_ms: f64,
    pub dons: usize,
    pub kats: usize,
    pub rolls: usize,
    pub swells: usize,
}

impl Details {
    fn of(map: &Beatmap) -> Details {
        // ADLibs and bombs aren't notes to hit.
        let count = |k: Kind| {
            (map.notes.iter())
                .filter(|n| n.kind == k && !n.extra())
                .count()
        };
        let first = map.notes.first().map(|n| n.time).unwrap_or(0.0);
        let last = map.notes.iter().map(|n| n.end_time).fold(0.0, f64::max);
        let bpms: Vec<f64> = map
            .control_points
            .iter()
            .filter(|p| p.time <= last)
            .map(|p| 60000.0 / p.beat_length)
            .collect();
        let bpm = bpms.iter().fold((f64::INFINITY, 0.0f64), |(lo, hi), &b| {
            (lo.min(b), hi.max(b))
        });
        Details {
            bpm: if bpms.is_empty() { (0.0, 0.0) } else { bpm },
            length_ms: last - first,
            dons: count(Kind::Don),
            kats: count(Kind::Kat),
            rolls: count(Kind::Roll),
            swells: count(Kind::Swell),
        }
    }
}

type LoadedMap = (
    usize,
    Result<Beatmap, String>,
    Option<(Vec<f64>, f64, f64)>,
    [f32; 2],
    Vec<taiko_logic::tja::Unsupported>,
);

/// What a TJA course leaves out, as its details list it: most used first, with how many times
/// when it's more than once (`#NOTESPAWN x839, #BGAON`).
pub(crate) fn left_out_text(left_out: &[taiko_logic::tja::Unsupported]) -> String {
    left_out
        .iter()
        .map(|u| match u.times {
            1 => u.what.clone(),
            n => format!("{} x{n}", u.what),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
type StatCell = (&'static str, String, f32);
type StatRow<'a> = Vec<&'a StatCell>;

/// The pp simulator's pick: the accuracy (0 to 1) and the misses of a play.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PpSim {
    pub accuracy: f64,
    pub misses: u32,
}

/// How far a step of the simulator moves its accuracy.
const SIM_ACC_STEP: f64 = 0.005;

impl PpSim {
    /// A step of field `field` (0: accuracy, 1: misses) `dir` (-1 or 1) on.
    pub fn step(&mut self, field: u8, dir: i32) {
        if field == 0 {
            self.accuracy = (self.accuracy + SIM_ACC_STEP * f64::from(dir)).clamp(0.5, 1.0);
        } else {
            self.misses = self.misses.saturating_add_signed(dir).min(9999);
        }
    }

    /// The play it stands for on a map of `notes` notes: its greats, oks and misses, the
    /// accuracy as close as those can come.
    pub fn counts(&self, notes: u32) -> (u32, u32, u32) {
        let miss = self.misses.min(notes);
        let hits = notes - miss;
        // accuracy = (great + ok / 2) / notes, great + ok = hits.
        let ok = (2.0 * (f64::from(hits) - self.accuracy * f64::from(notes)))
            .round()
            .clamp(0.0, f64::from(hits)) as u32;
        (hits - ok, ok, miss)
    }
}

#[derive(Default)]
pub struct Preview {
    seen: Option<usize>,
    changed_at: Option<Instant>,
    last_change: Option<Instant>,
    requested: Option<usize>,
    map_rx: Job<LoadedMap>,
    /// Calls off the map and background being read for a selection since moved on from: their
    /// threads stop before the slow part instead of working for nothing (a run of selections a
    /// little apart left every one of them parsing and decoding beside the game).
    map_ticket: Ticket,
    bg_ticket: Ticket,
    /// Speed (and keep-pitch) the loaded preview was prepared for.
    requested_rate: (f64, bool),
    /// The fullscreen preview is paused: the music holds, and the notes with it.
    pub held: bool,
    /// After a speed change: map time to continue the music from once the new track is ready.
    resume_at: Option<f64>,
    bg_rx: Job<(PathBuf, Option<Image>)>,
    audio_rx: Job<Decoded>,
    map: Option<(usize, Beatmap, Details, Graph)>,
    /// For a TJA course (by library index), what it uses that isn't played as OpenTaiko does
    /// (`left_out_text`): listed with its details.
    left_out: Option<(usize, String)>,
    /// Waiting for the music to reach a seek target before rebuilding the autoplay preview.
    pending_seek: Option<f64>,
    /// Where the difficulty timeline was drawn (virtual units) and the time range it spans.
    graph_rect: Option<[f32; 4]>,
    timeline: (f64, f64),
    /// Start point: where the timeline was last scrolled to (song file, map time). Kept while
    /// browsing the difficulties of that song.
    marker: Option<(PathBuf, f64)>,
    /// Where the "Play from" button was drawn.
    play_rect: Option<[f32; 4]>,
    /// Where the autoplay box was drawn (clicking it opens the fullscreen preview).
    box_rect: Option<[f32; 4]>,
    /// Split, where the lane's corridor was last drawn (a plain map's), if it was.
    corridor: Option<[f32; 4]>,
    /// Screen units to one of those the rects above were drawn in: song select draws the
    /// panel in a space of its own (`render::Canvas::begin_space`). 0 (none yet) is 1.
    drawn_scale: f32,
    /// And how far in from the screen's left edge that space started (its units).
    drawn_left: f32,
    /// When the shown map's details could first be shown (they fade in), and for which entry.
    ready_at: Option<Instant>,
    shown_for: Option<usize>,
    /// The entry whose autoplay has been ready: its music played.
    ready_for: Option<usize>,
    game: Option<Play>,
    /// How far up and down (virtual units) the autoplay box shows: easing from the first band
    /// to the second (the lane, or all a .tko map moves out of it) since the instant.
    reach: ([f32; 2], [f32; 2], Option<Instant>),
    last_time: f64,
    bg_path: Option<PathBuf>,
    /// New background for the renderer: `Some(None)` clears it.
    pub bg_upload: Option<Option<Image>>,
    pub has_background: bool,
    /// The screen shows its own map's background (the editor, a play): the selection's waits
    /// until it's left, so a slow preview load can't swap it for another map's.
    pub own_background: bool,
    owned_background: bool,
    /// The music player is open: songs play whole, and don't loop.
    pub player: bool,
    /// The main menu is showing: a song that ends gives way to the next too.
    pub menu: bool,
    /// The next song came on by itself: it plays from its start, not its preview point.
    pub from_top: bool,
    /// The last song decoded whole and its length in map ms, which no speed changes: while its
    /// quick part stands in after a new speed, that's still how long it is.
    known_length: Option<(PathBuf, f64)>,
    /// In the player or the main menu, the song has played to its end.
    finished: bool,
    ended_at: Option<Instant>,
    /// When the song now on was put on (a new one, or a replay): it can't be over just after, so
    /// a stale position from the song before never counts as this one's end.
    song_since: Option<Instant>,
    /// Passed on to the autoplay preview (the kiai effects setting, how autoplay hits).
    pub kiai_effects: bool,
    /// Bare, the lane without its drum (`Config::preview_drum` off).
    pub hide_drum: bool,
    pub autoplay: crate::autoplay::Settings,
    /// The selected map's own offset, shown with its details.
    pub map_offset: f64,
    /// The pp a full combo of the selected map gives with the mods picked, at `pp::POTENTIAL`'s
    /// accuracies (SS, 99%, 98%), shown with its details once worked out.
    pub potential: Option<[f64; 3]>,
    /// The mods picked, for the OD shown (EZ, HR and Precise change it).
    pub mods: crate::mods::Mods,
    /// Bottom of the song select panel as last drawn (virtual units).
    pub panel_bottom: f32,
    /// Room kept free at the right of the title line, for a button over the panel's corner
    /// (song select's way to the leaderboard); 0 elsewhere.
    pub title_room: f32,
    /// Drawn bare, in the main menu's look (song select outside the Legacy layout): no panel or
    /// boxes, the details straight over the background, the autoplay a lane of its own running
    /// off the screen's right edge and the timeline's bars on their own. Set by the caller
    /// before `draw_panel`.
    pub bare: bool,
    /// Bare: the details and the lane in sections of their own (`Split`), set before each draw.
    pub split: Option<Split>,
    /// How far left of its place the panel is drawn (song select sliding to its leaderboard):
    /// the bare lane's end, at the screen's right edge, goes along with it, so nothing in the
    /// panel stretches on the way. Set by the caller before `draw_panel`.
    pub slid: f32,
    /// The map shown is marked EPIC on taiko.rs (set by the caller before `draw_panel`): its pill
    /// stands in the status's place, EPIC being a status of its own.
    pub epic: bool,
    /// Where `draw_panel` put the EPIC pill (left, top), for the effects round it.
    pub epic_at: Option<[f32; 2]>,
    /// The picture the bare details' banner shows and when it was first drawn (it fades in).
    banner_seen: Option<(PathBuf, Instant)>,
    /// The song the bare details are coming in for, and when it was picked: they come in one
    /// after another (`enter`). None for a song picked in a quick run of them: those swap at once.
    entered: Option<(usize, Instant)>,
    /// The bare details as last drawn, and what of them is going out since a change (with
    /// when the change was).
    drawn: Drawn,
    leaving: Option<(Drawn, Instant)>,
    /// The star rating and full-combo pp the bare details show: they follow the map's, counting
    /// from the last song's to the new one's.
    stars_shown: Option<f32>,
    pp_shown: Option<[f64; 3]>,
    /// The pp simulator in place of the full-combo chips (bare): what's picked, and the pp it
    /// would give, which song select works out.
    pub sim: Option<PpSim>,
    pub sim_pp: Option<f64>,
    /// The autoplay isn't shown (song select's Local and Online tabs): it isn't played on, its
    /// notes and drum aren't worked out every frame for nothing. The music plays on.
    pub autoplay_hidden: bool,
    /// It wasn't played on while hidden: it starts over where the music is when shown again.
    autoplay_behind: bool,
    /// How big the autoplay preview is drawn (`PREVIEW_SIZE`; 0 = full), and how much its height
    /// changes per unit of that (for dragging the panel's bottom edge).
    pub size: f32,
    pub size_unit: f32,
    /// The size it was drawn at, and the most that fits the panel's height.
    pub size_shown: f32,
    pub size_cap: f32,
    audio: Option<Song>,
    /// Calls off the music being prepared.
    music_job: Option<Arc<AtomicBool>>,
    audio_active: bool,
}

/// How far map time `t` is past the red line the beats count from, and the beat's length. Green
/// lines are control points too, but they don't move the beat.
fn beat_at(points: &[crate::beatmap::ControlPoint], t: f64) -> Option<(f64, f64)> {
    let line = points
        .iter()
        .rev()
        .find(|p| p.time <= t)
        .or(points.first())?;
    Some((t - line.beat_origin, line.beat_length))
}

/// How long the player waits at a song's end for the next to load before playing it again.
const NEXT_SONG_WAIT: Duration = Duration::from_millis(1500);

/// Whether a song put on at `since` has played long enough (a few seconds, or half of a short
/// one) for its position to be taken as reaching its end: just after another song was on, the
/// position still reported can be that one's.
fn played_long_enough(since: Option<Instant>, now: Instant, end_ms: f64) -> bool {
    let wait = Duration::from_millis((end_ms / 2.0).clamp(0.0, 3000.0) as u64);
    since.is_none_or(|t| now.saturating_duration_since(t) >= wait)
}

/// How long a song is, in map ms: as decoded whole (`whole`), else as last known, else about what
/// its notes span.
fn song_length(
    whole: Option<f64>,
    known: Option<&(PathBuf, f64)>,
    path: &Path,
    map_end: f64,
) -> f64 {
    whole
        .or(known.filter(|k| k.0 == path).map(|k| k.1))
        .unwrap_or(map_end)
}

pub fn audio_path(e: &Entry) -> PathBuf {
    e.path
        .parent()
        .unwrap_or(Path::new("."))
        .join(&e.meta.audio_file)
}

pub(crate) fn background_path(e: &Entry) -> Option<PathBuf> {
    e.meta
        .background
        .as_ref()
        .map(|b| e.path.parent().unwrap_or(Path::new(".")).join(b))
}

/// Decodes a background, shrunk to at most 1920x1200 (GPU texture limits, memory); given up
/// (None) as soon as it isn't `wanted` any more: before the file is read, and between decoding
/// it and making the GPU's copy of it.
fn load_image_while(path: &Path, wanted: impl Fn() -> bool) -> Option<Image> {
    if !wanted() {
        return None;
    }
    let img = image::open(path).ok()?;
    if !wanted() {
        return None;
    }
    let img = if img.width() > 1920 || img.height() > 1200 {
        img.resize(1920, 1200, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Some(Image::from_rgba(rgba.width(), rgba.height(), &rgba))
}

/// The hit windows as they are in real time, in a box at `(x, y)`.
fn draw_hit_windows(c: &mut Canvas, (x, y): (f32, f32), windows: crate::game::Windows) {
    let rows = [
        ("300", windows.great, 0xffd34d),
        ("100", windows.ok, 0x86e05a),
        ("miss", windows.miss, 0xff4b4b),
    ];
    let text = rows.map(|(_, ms, _)| format!("±{ms:.1} ms"));
    let label_w = Canvas::text_width(7.0, "miss") + 8.0;
    let value_w = text
        .iter()
        .map(|t| Canvas::text_width(7.0, t))
        .fold(0.0, f32::max);
    let (w, h, row_h) = (label_w + value_w + 16.0, 12.0 * 3.0 + 10.0, 12.0);
    // Inside the screen, however far right the stat is.
    let x = x.min(c.width - w - 4.0).max(4.0);
    c.round_rect(
        [x, y],
        [w, h],
        5.0,
        rgba(0x0c0c12, 0.97),
        rgba(0x2a2a38, 1.0),
        1.0,
    );
    for (i, ((label, _, colour), value)) in rows.iter().zip(&text).enumerate() {
        let ry = y + 6.0 + i as f32 * row_h;
        c.text(x + 8.0, ry, 7.0, rgba(*colour, 1.0), label);
        c.text(x + 8.0 + label_w, ry, 7.0, rgba(0xe8e8f4, 1.0), value);
    }
}

impl Preview {
    /// The entry list changes meaning (song select <-> download previews): forget what was shown.
    pub fn forget(&mut self) {
        self.seen = None;
        self.requested = None;
        self.map = None;
        self.left_out = None;
        self.shown_for = None;
        self.ready_for = None;
        self.entered = None;
        self.game = None;
        self.map_rx.cancel();
        self.map_ticket.cancel();
        self.pending_seek = None;
    }

    /// The entry asked to be shown (tests).
    #[cfg(test)]
    pub fn requested(&self) -> Option<usize> {
        self.requested
    }

    /// The title of the map shown (tests).
    #[cfg(test)]
    pub fn shown_title(&self) -> Option<String> {
        self.map.as_ref().map(|m| m.1.meta.title.clone())
    }

    /// Whether no thread works for it: the map, background and music asked for are in (or
    /// failed).
    #[cfg(test)]
    pub fn is_idle(&self) -> bool {
        self.map_rx.is_idle() && self.bg_rx.is_idle() && self.audio_rx.is_idle()
    }

    /// Whether a newer selection is waiting out its debounce: frames to come, not a thread,
    /// will start its loads.
    #[cfg(test)]
    pub fn is_due(&self) -> bool {
        self.seen != self.requested
    }

    /// Something else (gameplay, calibration) took over the audio engine.
    pub fn invalidate_audio(&mut self) {
        self.audio_active = false;
    }

    /// Stops preparing the music of a song that isn't wanted any more.
    fn call_off_music(&mut self) {
        if let Some(job) = self.music_job.take() {
            job.store(true, Ordering::Relaxed);
        }
    }

    /// Lets go of the preview's music if it's long, for a play that needs its own copy of the song:
    /// that's gigabytes each. It's prepared again when the preview next comes up. True if it did.
    pub fn release_long_music(&mut self) -> bool {
        const LONG_FRAMES: usize = 48_000 * 60 * 10;
        let long = self
            .audio
            .as_ref()
            .is_some_and(|s| s.track.frames() > LONG_FRAMES);
        if long {
            self.call_off_music();
            self.audio_rx.cancel();
            self.audio = None;
            self.requested = None;
        }
        long
    }

    /// The decoded song, for reuse when that map is played at the same speed.
    pub fn cached_track(&self, e: &Entry, rate: f64, keep_pitch: bool) -> Option<Arc<Track>> {
        let same = |s: &&Song| {
            s.full
                && s.path == audio_path(e)
                && (s.rate - rate).abs() < 1e-6
                && (s.keep_pitch == keep_pitch || (rate - 1.0).abs() < 1e-6)
        };
        self.audio.as_ref().filter(same).map(|s| s.track.clone())
    }

    /// Whether the selection is still settling on library entry `i` (`DEBOUNCE`): the preview
    /// waits to read it, and other work for the map can wait with it.
    pub fn settling_on(&self, i: usize) -> bool {
        self.seen == Some(i) && self.requested != Some(i)
    }

    /// Puts `image` up in place of the background, until `forget_background`.
    pub fn show_image(&mut self, image: Image) {
        self.bg_upload = Some(Some(image));
    }

    /// The next background asked for is read again, whatever was up.
    pub fn forget_background(&mut self) {
        self.bg_path = None;
    }

    /// Makes sure the background shown is `e`'s (e.g. a multiplayer map picked by the host). A TJA
    /// chart without a picture, or whose picture can't be read, gets one drawn (`backdrop`).
    pub fn show_background_for(&mut self, e: &Entry) {
        let file = background_path(e);
        let drawn = crate::tja_maps::is_course(&e.path).then(|| e.meta.clone());
        let path = file
            .clone()
            .or_else(|| drawn.as_ref().map(crate::backdrop::key));
        if path == self.bg_path {
            return;
        }
        self.bg_path = path.clone();
        match path {
            Some(p) => {
                self.bg_ticket.cancel();
                let ticket = Ticket::default();
                self.bg_ticket = ticket.clone();
                self.bg_rx = Job::spawn(move || {
                    let wanted = || ticket.wanted();
                    let img = file
                        .as_deref()
                        .and_then(|f| load_image_while(f, wanted))
                        .or_else(|| {
                            wanted()
                                .then(|| drawn.as_ref().map(crate::backdrop::image))
                                .flatten()
                        });
                    (p, img)
                });
            }
            None => {
                self.bg_upload = Some(None);
                self.has_background = false;
            }
        }
    }

    /// Call every frame. `in_menu`: menus where preview music keeps playing; `in_select`: the
    /// autoplay preview is visible and runs. `rate`: speed the music and the autoplay play at
    /// (and whether it keeps the pitch).
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        now: Instant,
        library: &[Entry],
        selected: Option<usize>,
        engine: Option<&Engine>,
        in_menu: bool,
        in_select: bool,
        offset: f64,
        rate: (f64, bool),
    ) {
        let rate = if (rate.0 - 1.0).abs() < 1e-6 {
            (1.0, false)
        } else {
            rate
        };
        // A new speed changes the music and the strain curve: reload as if the selection
        // changed, carrying on from where the music was.
        if rate != self.requested_rate && self.requested.is_some() {
            if self.audio_active && self.requested == selected {
                // Where the music is now, in map time (the playing track's own speed).
                let playing = engine
                    .and_then(|e| e.position_ms(now))
                    .zip(self.audio.as_ref())
                    .map(|(p, s)| p * s.rate);
                self.resume_at = self.pending_seek.or(playing).or(self.resume_at);
            }
            self.requested = None;
            self.changed_at = Some(now.checked_sub(DEBOUNCE).unwrap_or(now));
        }
        // Back from a screen with its own background: the selection's again.
        if self.owned_background
            && !self.own_background
            && let Some(e) = self.requested.and_then(|i| library.get(i))
        {
            self.show_background_for(e);
        }
        self.owned_background = self.own_background;
        if selected != self.seen {
            // The start point picked ("Play from") is that map's: another one starts afresh, even
            // on the same song (another difficulty of the set), and so does going back to it.
            self.marker = None;
            // The first song shown (song select opening) comes with the screen, not on its own.
            let was = self.seen;
            self.seen = selected;
            self.resume_at = None;
            let quiet = self.last_change.is_none_or(|t| now - t >= QUIET);
            // A song picked on its own comes in; one in a quick run of them (a key held, Random
            // spammed) swaps in at once, so nothing queues up or flickers.
            self.entered = selected
                .filter(|_| quiet && was.is_some())
                .map(|i| (i, now));
            self.last_change = Some(now);
            self.changed_at = Some(if quiet {
                now.checked_sub(DEBOUNCE).unwrap_or(now)
            } else {
                now
            });
        }
        // Start loads once the selection has settled.
        if self.seen != self.requested && self.changed_at.is_some_and(|t| now - t >= DEBOUNCE) {
            self.requested = self.seen;
            self.requested_rate = rate;
            self.game = None;
            let (speed, keep_pitch) = rate;
            if let Some(i) = self.seen.filter(|&i| i < library.len()) {
                let e = &library[i];
                let path = e.path.clone();
                self.map_ticket.cancel();
                let ticket = Ticket::default();
                self.map_ticket = ticket.clone();
                self.map_rx = Job::spawn(move || {
                    // Called off (the selection moved on before this thread got going): nobody
                    // takes what it would make.
                    let off = || {
                        (
                            i,
                            Err("called off".to_string()),
                            None,
                            lane_reach(),
                            Vec::new(),
                        )
                    };
                    if !ticket.wanted() {
                        return off();
                    }
                    // A TJA course with what it leaves out, from the one read of its file.
                    let (map, left_out) = if crate::tja_maps::is_course(&path) {
                        match crate::tja_maps::course(&path) {
                            Ok(c) => (Ok(c.map), c.unsupported),
                            Err(e) => (Err(e), Vec::new()),
                        }
                    } else {
                        (beatmap::load(&path), Vec::new())
                    };
                    let first = map
                        .as_ref()
                        .ok()
                        .and_then(|m| m.notes.first())
                        .map(|n| n.time)
                        .unwrap_or(0.0);
                    // The strain curve is the slow part.
                    if !ticket.wanted() {
                        return off();
                    }
                    let curve = std::panic::catch_unwind(|| {
                        crate::stars::strain_curve(&path, speed, first)
                    })
                    .ok()
                    .flatten();
                    let reach = map.as_ref().map(tko_reach).unwrap_or_else(|_| lane_reach());
                    (i, map, curve, reach, left_out)
                });
                if !self.own_background {
                    self.show_background_for(e);
                }
                let song = audio_path(e);
                let loaded = self
                    .audio
                    .as_ref()
                    .is_some_and(|s| s.path == song && (s.rate, s.keep_pitch) == rate);
                if !loaded && let Some(sr) = engine.map(|a| a.rate) {
                    let (tx, rx) = mpsc::channel();
                    // Quick part first: from where the music was (speed change) or the preview point.
                    let part_from = self
                        .resume_at
                        .or((self.player || self.from_top).then_some(0.0))
                        .unwrap_or(e.meta.preview_time);
                    let clip_at = e.meta.preview_time.max(0.0);
                    let is_clip = song.file_name().is_some_and(|n| {
                        n.to_string_lossy()
                            .starts_with(crate::download::CLIP_PREFIX)
                    });
                    self.call_off_music();
                    let called_off = Arc::new(AtomicBool::new(false));
                    self.music_job = Some(called_off.clone());
                    let wanted_now = called_off.clone();
                    std::thread::spawn(move || {
                        audio::while_wanted(called_off);
                        if is_clip {
                            // A download preview: ~10 s of music from the preview point, placed there.
                            let clip = audio::decode(&song, sr).ok().map(|t| {
                                let base = (clip_at / 1000.0 * sr as f64 / speed).round() as i64;
                                (
                                    Arc::new(audio::change_rate(t, sr, speed, keep_pitch)),
                                    base,
                                    false,
                                )
                            });
                            let failed = clip.is_none();
                            let _ = tx.send((song.clone(), clip));
                            if !failed {
                                let _ = tx.send((song, None));
                            }
                            return;
                        }
                        if part_from >= 0.0 {
                            // SNIPPET_MS of playback covers that much more map time when sped up.
                            if let Ok((part, base)) =
                                audio::decode_part(&song, sr, Some((part_from, SNIPPET_MS * speed)))
                            {
                                let part = audio::change_rate(part, sr, speed, keep_pitch);
                                let base = (base as f64 / speed).round() as i64;
                                if tx
                                    .send((song.clone(), Some((Arc::new(part), base, false))))
                                    .is_err()
                                {
                                    return; // the selection moved on
                                }
                            }
                        }
                        // The whole song can take gigabytes: one at a time, and not for a song
                        // the selection has moved on from.
                        let Some(_alone) = audio::prepare_alone() else {
                            return;
                        };
                        // A long song plays as it's decoded.
                        let constant = crate::mods::Speed::constant(speed);
                        if let Ok(Some(track)) =
                            crate::stream::open(&song, sr, constant, keep_pitch)
                        {
                            if !wanted_now.load(Ordering::Relaxed) {
                                let _ = tx.send((song, Some((track, 0, true))));
                            }
                            return;
                        }
                        let track = audio::decode_shared(&song, sr)
                            .ok()
                            .map(|t| (audio::with_rate(t, sr, speed, keep_pitch), 0, true));
                        if wanted_now.load(Ordering::Relaxed) {
                            return;
                        }
                        let _ = tx.send((song, track));
                    });
                    self.audio_rx = Job::from_receiver(rx);
                }
            }
        }

        if let Some((i, result, curve, reach, left_out)) = self.map_rx.poll()
            && Some(i) == self.requested
            && let Ok(map) = result
        {
            self.left_out = (!left_out.is_empty()).then(|| (i, left_out_text(&left_out)));
            let details = Details::of(&map);
            let mut graph = difficulty::analyze(&map);
            if let Some(curve) = curve {
                graph.use_curve(curve);
            }
            // The details and the music, but no playfield for a map gone wild.
            self.game =
                (!map.too_wild_to_preview()).then(|| new_preview_game(&map, self.requested_rate.0));
            self.last_time = f64::NEG_INFINITY;
            self.map = Some((i, map, details, graph));
            if reach != self.reach.1 {
                self.reach = (self.shown_reach(), reach, Some(Instant::now()));
            }
        }
        if let Some((path, img)) = self.bg_rx.poll()
            && Some(&path) == self.bg_path.as_ref()
        {
            self.has_background = img.is_some();
            self.bg_upload = Some(img);
        }
        let decoded: Vec<Decoded> = self.audio_rx.poll_all();
        for (path, result) in decoded {
            if !matches!(result, Some((_, _, false))) {
                self.audio_rx.cancel(); // the full song (or a failure) is the last message
            }
            let wanted = self.requested.and_then(|i| library.get(i));
            let (Some((track, base, full)), Some(engine), Some(e)) =
                (result, engine, wanted.filter(|e| audio_path(e) == path))
            else {
                continue;
            };
            let (speed, keep_pitch) = self.requested_rate;
            // In map ms (the track plays `speed` map ms per ms).
            let len_ms = track.frames() as f64 / engine.rate as f64 * 1000.0 * speed;
            let preview_time = e.meta.preview_time;
            let start = if self.player || self.from_top {
                0.0
            } else {
                (if preview_time >= 0.0 && (!full || preview_time < len_ms) {
                    preview_time
                } else {
                    len_ms * 0.4
                }) / speed
            };
            // The full song takes over from the part that's already playing without a gap.
            let playing_part = self.audio.as_ref().is_some_and(|s| {
                s.path == path && !s.full && (s.rate, s.keep_pitch) == (speed, keep_pitch)
            });
            if full && playing_part && self.audio_active && in_menu {
                engine.swap(track.clone(), base);
            } else {
                self.audio_active = false;
                self.song_since = Some(now);
            }
            if full {
                self.known_length = Some((path.clone(), len_ms));
            }
            self.ended_at = None;
            // The quick part comes first; the whole song is the last of it.
            self.from_top &= !full;
            self.audio = Some(Song {
                path,
                track,
                base,
                full,
                start,
                rate: speed,
                keep_pitch,
            });
        }

        // Music: start at the preview point, loop back to it at the end.
        let (Some(engine), true) = (engine, in_menu) else {
            return;
        };
        if let Some(song) = &self.audio {
            let end_ms = song.end_ms(engine.rate);
            // A seek past the quick part waits, silent, for the full song; it's not the end.
            let ended = self.audio_rx.is_idle()
                && played_long_enough(self.song_since, now, end_ms)
                && engine.position_ms(now).is_some_and(|p| p > end_ms - 30.0);
            // There the next song takes over; if none does, this one goes round again.
            let next_coming = ended && (self.player || self.menu) && {
                let at = *self.ended_at.get_or_insert_with(|| {
                    self.finished = true;
                    now
                });
                now.duration_since(at) < NEXT_SONG_WAIT
            };
            if (!self.audio_active || ended) && !next_coming {
                let from_top = ended && (self.player || self.from_top);
                let at = self
                    .resume_at
                    .take()
                    .map(|t| t / song.rate)
                    .unwrap_or(if from_top { 0.0 } else { song.start });
                engine.play_part(song.track.clone(), song.base, at);
                self.song_since = Some(now);
                self.audio_active = true;
                self.ended_at = None;
                self.pending_seek = None; // the music won't reach it now
            }
        }

        // Autoplay preview, driven by the music's clock. It waits while the music is still the
        // previous song's or speed's (its clock would put the notes in the wrong place).
        let wanted = self.requested.and_then(|i| library.get(i)).map(audio_path);
        let stale = self.audio.as_ref().is_none_or(|s| {
            (s.rate, s.keep_pitch) != self.requested_rate || Some(&s.path) != wanted.as_ref()
        });
        if !in_select || !self.audio_active || stale {
            return;
        }
        let speed = self.requested_rate.0;
        if let Some(target) = self.pending_seek {
            // The engine applies seeks on its next callback; rebuild once the music is there so
            // the notes in between aren't lost.
            // Held, the music has no position to arrive at.
            let arrived = self.held
                || engine
                    .position_ms(now)
                    .is_some_and(|p| (p * speed - target).abs() < 150.0 * speed);
            if arrived {
                if let Some((_, map, _, _)) = &self.map
                    && !map.too_wild_to_preview()
                {
                    let mut fresh = new_preview_game(map, speed);
                    fresh.skip_to(target - 50.0);
                    // Held, there's no music to go by: it's put there by hand.
                    if self.held {
                        fresh.seek(target, None, offset);
                    } else {
                        fresh.update(now, Some(engine), offset);
                    }
                    self.last_time = fresh.time();
                    self.game = Some(fresh);
                }
                self.pending_seek = None;
            }
            return;
        }
        if self.held {
            return;
        }
        let Some(game) = &mut self.game else { return };
        let music = engine.position_ms(now).map(|p| (p - offset) * speed);
        if self.autoplay_hidden {
            // Nobody sees the autoplay (song select's Local and Online tabs): it isn't played,
            // only its clock is kept for the beat and kiai the rest of the screen goes by.
            if let Some(t) = music {
                self.last_time = t;
                self.autoplay_behind = true;
            }
            return;
        }
        // Where the music is, in map time, before the game expires anything: notes skipped over
        // are marked played first, so jumping in never shows misses.
        if let Some(t) = music {
            if std::mem::take(&mut self.autoplay_behind) {
                // Shown again: it starts over where the music is, as on a jump.
                if let Some((_, map, _, _)) = &self.map {
                    *game = new_preview_game(map, speed);
                    game.skip_to(t - 50.0);
                }
            } else if t < self.last_time - 1000.0 {
                // The music looped: start the preview over from the preview point.
                if let Some((_, map, _, _)) = &self.map {
                    *game = new_preview_game(map, speed);
                    game.skip_to(t - 50.0);
                }
            } else if t > self.last_time + 2000.0 {
                // Jumped in (first frame, or the song changed): don't replay the skipped part.
                game.skip_to(t - 50.0);
            }
        }
        game.update(now, Some(engine), offset);
        self.last_time = game.time();
        game.set_autoplay_style(self.autoplay);
        game.autoplay(None);
    }

    /// The band the autoplay box shows now (see `reach`).
    fn shown_reach(&self) -> [f32; 2] {
        let (from, to, at) = self.reach;
        let Some(at) = at else {
            return lane_reach();
        };
        let p = crate::render::progress(at, 400.0);
        [
            from[0] + (to[0] - from[0]) * p,
            from[1] + (to[1] - from[1]) * p,
        ]
    }

    /// Jumps the preview music (and the autoplay) to map time `t`, and remembers it as the
    /// start point (the blue flag): wherever you last scrolled to is where "Play from" starts.
    pub fn seek(&mut self, t: f64, engine: Option<&Engine>) {
        if let Some(t) = self.seek_music(t, engine) {
            self.marker = self.audio.as_ref().map(|s| (s.path.clone(), t));
        }
    }

    /// Jumps the preview music (and the autoplay) to map time `t`; returns where it landed.
    pub fn seek_music(&mut self, t: f64, engine: Option<&Engine>) -> Option<f64> {
        let (Some(engine), Some(song)) = (engine, &self.audio) else {
            return None;
        };
        if !self.audio_active {
            return None;
        }
        // Until the full decode replaces the quick part, the song's length isn't known; seeking
        // outside the part is silent for that moment instead of being refused.
        let last = if song.full {
            ((song.end_ms(engine.rate) - 1000.0) * song.rate).max(0.0)
        } else {
            f64::INFINITY
        };
        let t = t.clamp(0.0, last);
        engine.seek(t / song.rate);
        self.pending_seek = Some(t);
        self.ended_at = None;
        Some(t)
    }

    #[cfg(test)]
    pub fn song_ended(&mut self) {
        self.finished = true;
    }

    /// The song played to its end in the player or the main menu (once per end).
    pub fn take_finished(&mut self) -> bool {
        std::mem::take(&mut self.finished)
    }

    /// Where the music is and how long it is, in map ms. While only a part of the song is
    /// decoded its length is the map's.
    pub fn music_times(&self, engine: Option<&Engine>, now: Instant) -> Option<(f64, f64)> {
        let (engine, song) = (engine?, self.audio.as_ref().filter(|_| self.audio_active)?);
        let at = engine.position_ms(now)? * song.rate;
        let map_end = self.map.as_ref().map_or(0.0, |(_, _, d, _)| d.length_ms);
        let whole = song.full.then(|| song.end_ms(engine.rate) * song.rate);
        Some((
            at,
            song_length(whole, self.known_length.as_ref(), &song.path, map_end),
        ))
    }

    /// Fills `out` with the last mono samples played; false if nothing is.
    pub fn sounding(&self, engine: Option<&Engine>, now: Instant, out: &mut [f32]) -> bool {
        let (Some(engine), Some(song)) =
            (engine, self.audio.as_ref().filter(|_| self.audio_active))
        else {
            return false;
        };
        let Some(at) = engine.position_ms(now) else {
            return false;
        };
        let last = (at / 1000.0 * engine.rate as f64) as i64 - song.base;
        let first = last - out.len() as i64;
        for (i, s) in out.iter_mut().enumerate() {
            let frame = first + i as i64;
            *s = usize::try_from(frame)
                .ok()
                .and_then(|f| song.track.samples.get(f * 2..f * 2 + 2))
                .map_or(0.0, |lr| (lr[0] + lr[1]) / 2.0);
        }
        true
    }

    /// How far (ms) the song is into the current beat and how long that beat is, for things
    /// that pulse with the music.
    pub fn beat(&self) -> Option<(f64, f64)> {
        let (_, map, _, _) = self.map.as_ref().filter(|_| self.audio_active)?;
        // Only a beat that is one: NaN timing points (osu!team's FM Synthesis Experiment), or
        // no preview clock yet, made every selector and the menu's notes draw as black boxes.
        beat_at(&map.control_points, self.last_time)
            .filter(|&(into, len)| into.is_finite() && len.is_finite() && len > 0.0)
    }

    /// Whether the music playing is `e`'s (a new song's starts a moment after it's picked).
    #[cfg(target_os = "android")]
    pub fn plays(&self, e: &Entry) -> bool {
        self.audio_active && self.audio.as_ref().is_some_and(|s| s.path == audio_path(e))
    }

    /// The shown map's notes, once it's read.
    pub fn notes(&self) -> Option<u32> {
        self.map
            .as_ref()
            .filter(|(mi, ..)| Some(*mi) == self.seen)
            .map(|(_, _, d, _)| (d.dons + d.kats) as u32)
    }

    /// Opens the pp simulator (at 98%, no misses) or closes it.
    pub fn toggle_sim(&mut self) {
        self.sim = match self.sim {
            Some(_) => None,
            None => Some(PpSim {
                accuracy: 0.98,
                misses: 0,
            }),
        };
    }

    /// The panel isn't on screen: nothing of it is there to press.
    pub fn hide_panel(&mut self) {
        self.graph_rect = None;
        self.play_rect = None;
    }

    /// The map whose music plays, and where it is (map ms).
    pub fn playing_time(&self) -> Option<(usize, f64)> {
        let (i, ..) = self.map.as_ref().filter(|_| self.audio_active)?;
        self.last_time.is_finite().then_some((*i, self.last_time))
    }

    /// Whether the music is in a kiai now.
    pub fn kiai(&self) -> bool {
        let Some((_, map, _, _)) = self.map.as_ref().filter(|_| self.audio_active) else {
            return false;
        };
        map.control_points
            .iter()
            .rev()
            .find(|p| p.time <= self.last_time)
            .is_some_and(|p| p.kiai)
    }

    pub fn seek_by(&mut self, delta_ms: f64, engine: Option<&Engine>) {
        let now = self
            .pending_seek
            .or_else(|| self.game.as_ref().map(|g| g.time()));
        if let Some(t) = now {
            self.seek(t + delta_ms, engine);
        }
    }

    /// The start point picked for `e`'s song, if any.
    pub fn marker(&self, e: &Entry) -> Option<f64> {
        self.marker
            .as_ref()
            .filter(|(p, _)| *p == audio_path(e))
            .map(|(_, t)| *t)
    }

    /// A point of the screen (virtual units) in the units the panel was drawn in.
    fn drawn_at(&self, x: f32, y: f32) -> (f32, f32) {
        let k = if self.drawn_scale > 0.0 {
            self.drawn_scale
        } else {
            1.0
        };
        (x / k - self.drawn_left, y / k)
    }

    /// Notes the units of the canvas the panel's rects are drawn in.
    fn drawn_in(&mut self, c: &Canvas) {
        self.drawn_scale = VIRTUAL_HEIGHT / c.height.max(1.0);
        self.drawn_left = c.left;
    }

    pub fn play_button_at(&self, x: f32, y: f32) -> bool {
        let (x, y) = self.drawn_at(x, y);
        self.play_rect
            .is_some_and(|[bx, by, bw, bh]| x >= bx && x <= bx + bw && y >= by && y <= by + bh)
    }

    pub fn box_at(&self, x: f32, y: f32) -> bool {
        let (x, y) = self.drawn_at(x, y);
        self.box_rect
            .is_some_and(|[bx, by, bw, bh]| x >= bx && x <= bx + bw && y >= by && y <= by + bh)
    }

    /// Whether entry `i`'s preview can be shown: its map is loaded and its own music is playing
    /// (until then the autoplay would run on the previous song's clock).
    fn ready(&self, library: &[Entry], i: usize) -> bool {
        let Some(e) = library.get(i) else {
            return false;
        };
        let map = self.map.as_ref().is_some_and(|(mi, ..)| *mi == i)
            && self.seen == Some(i)
            && self.game.is_some();
        let music = self.audio_active
            && self.audio.as_ref().is_some_and(|s| {
                s.path == audio_path(e) && (s.rate, s.keep_pitch) == self.requested_rate
            });
        map && music
    }

    /// The autoplay preview of library entry `i`, once it is ready.
    pub fn game_for(&mut self, library: &[Entry], i: usize) -> Option<&mut Play> {
        let ready = self.ready(library, i);
        self.game.as_mut().filter(|_| ready)
    }

    /// Summary stats for the currently loaded map.
    pub(crate) fn loaded_stats(&self) -> Option<LoadedStats> {
        let (_, _, d, _) = self.map.as_ref()?;
        Some(LoadedStats {
            bpm: (d.bpm.0 + d.bpm.1) * 0.5,
            length_ms: d.length_ms,
            dons: d.dons as u32,
            kats: d.kats as u32,
            rolls: d.rolls as u32,
            swells: d.swells as u32,
        })
    }

    /// Draws an embedded live autoplay lane inside `lane_rect` and its density timeline inside
    /// `graph_rect` (used by the Journey screen). Returns `true` when the autoplay is active.
    pub(crate) fn draw_embedded_preview(
        &mut self,
        c: &mut Canvas,
        e: &Entry,
        lane_rect: [f32; 4],
        graph_rect: [f32; 4],
        scroll_speed: f64,
    ) -> bool {
        let [lx, ly, lw, lh] = lane_rect;
        // Modern style.rs tile matching song select's split preview.
        let r = crate::style::radius::SMALL;
        crate::style::tile(c, lane_rect, r, crate::style::Surface::Raised);
        let ready = self.map.is_some();
        if ready {
            // Always use the modern bare pixel timeline with no overflow hint below the card,
            // regardless of what `self.bare` was left at from another screen.
            self.draw_timeline_styled(c, e, graph_rect, None, false, false);
            self.drawn_in(c);
        } else {
            self.graph_rect = None;
        }
        let Some(game) = self.game.as_mut().filter(|_| ready) else {
            return false;
        };
        let (band_top, band_h) = game::lane_only();
        let k = ((lh * SPLIT_LANE_FILL) / band_h.max(1.0)).clamp(0.25, 0.72);
        // Corridor across the tile so the notes run along a framed track rather than empty room.
        let corr_h = band_h * k;
        let corr_y = crate::render::snap_to_grid(ly + (lh - corr_h) / 2.0);
        let fill = crate::style::surface_fill(crate::style::Surface::Lifted);
        c.rect(lx, corr_y, lw, corr_h, fill);
        let edge = rgba(crate::style::tone::EDGE, 1.0);
        let line = crate::style::tone::LINE;
        c.rect(lx, corr_y, lw, line, edge);
        c.rect(lx, corr_y + corr_h - line, lw, line, edge);

        let full_w = c.width;
        let start = c.shapes.len();
        let drum_w = crate::game::PANEL_W * k;
        let bx = lx - if self.hide_drum { drum_w } else { 0.0 };
        let notes_end = lx + lw;
        c.width = ((notes_end - bx) / k).max(200.0);
        game.view.kiai_effects = self.kiai_effects;
        crate::game::with_look(preview_look(crate::game::playfield_look()), || {
            game.draw_playfield(c, scroll_speed)
        });
        let centre = (lh - corr_h) / 2.0;
        let oy = ly - band_top * k + centre;
        c.transform_since(start, [0.0, 0.0], k, bx, oy);
        c.width = full_w;
        bare_lane_end(&mut c.shapes, start, bx, lx, notes_end, corr_h);
        c.clip_since(start, lx + 1.0, ly + 1.0, lw - 2.0, lh - 2.0);
        true
    }

    /// The difficulty timeline of `e` (the selected map) in `rect`, with the start point and,
    /// if `button` is given as (verb, key hint), a "<verb> 1:23" button under it.
    pub fn draw_timeline(
        &mut self,
        c: &mut Canvas,
        e: &Entry,
        rect: [f32; 4],
        button: Option<(&str, &str)>,
    ) {
        self.draw_timeline_styled(c, e, rect, button, !self.bare, !self.bare);
    }

    fn draw_timeline_styled(
        &mut self,
        c: &mut Canvas,
        e: &Entry,
        rect: [f32; 4],
        button: Option<(&str, &str)>,
        boxed: bool,
        hint: bool,
    ) {
        self.graph_rect = None;
        self.play_rect = None;
        let loaded = self.map.as_ref().filter(|(mi, ..)| Some(*mi) == self.seen);
        let (Some((_, _, _, graph)), Some(game)) = (loaded, self.game.as_ref()) else {
            return;
        };
        let marker = self.marker(e);
        // The playhead goes where it was sent at once, not when the music gets there.
        self.timeline = draw_graph(
            c,
            graph,
            rect,
            self.pending_seek.unwrap_or(game.time()),
            e.meta.preview_time,
            marker,
            boxed,
        );
        self.graph_rect = Some(rect);
        self.drawn_in(c);
        self.play_rect = if button.is_some() || hint {
            draw_play_button(
                c,
                rect[0] + rect[2] / 2.0,
                rect[1] + rect[3] + 17.0,
                marker,
                button,
                hint,
            )
        } else {
            None
        };
        // The same slack as `time_at`.
        c.hit(
            rect[0] - 2.0,
            rect[1] - 2.0,
            rect[2] + 4.0,
            rect[3] + 4.0,
            Hit::Preview,
        );
        if let Some([x, y, w, h]) = self.play_rect {
            c.hit(x, y, w, h, Hit::Preview);
        }
    }

    pub fn graph_rect(&self) -> Option<[f32; 4]> {
        self.graph_rect
    }

    /// Where the autoplay's lane box was last drawn (virtual units), if it was.
    #[cfg(test)]
    pub fn box_rect(&self) -> Option<[f32; 4]> {
        self.box_rect
    }

    /// Centre of the preview's Play button in the standard virtual units `mouse_virtual` returns.
    #[cfg(test)]
    pub fn play_button_virtual(&self) -> Option<(f32, f32)> {
        let [bx, by, bw, bh] = self.play_rect?;
        let k = if self.drawn_scale > 0.0 {
            self.drawn_scale
        } else {
            1.0
        };
        Some(((bx + bw / 2.0 + self.drawn_left) * k, (by + bh / 2.0) * k))
    }

    /// Loads `library[i]` synchronously with a start marker at `at` ms so headless tests
    /// without an audio device can draw and click the preview's Play button.
    #[cfg(test)]
    pub fn prime_with_marker(&mut self, library: &[Entry], i: usize, at: f64) {
        let map = crate::beatmap::load(&library[i].path).expect("load preview map");
        self.seen = Some(i);
        self.requested = Some(i);
        self.ready_for = Some(i);
        self.game = Some(new_preview_game(&map, 1.0));
        self.map = Some((
            i,
            map.clone(),
            Details::of(&map),
            crate::difficulty::analyze(&map),
        ));
        self.marker = Some((audio_path(&library[i]), at));
    }

    /// Split, where the lane's corridor was last drawn, if it was.
    #[cfg(test)]
    pub fn corridor(&self) -> Option<[f32; 4]> {
        self.corridor
    }

    /// Map time under a point of the difficulty timeline, if the point is on it.
    pub fn time_at(&self, x: f32, y: f32) -> Option<f64> {
        let (x, y) = self.drawn_at(x, y);
        let [gx, gy, gw, gh] = self.graph_rect?;
        if x < gx - 2.0 || x > gx + gw + 2.0 || y < gy - 2.0 || y > gy + gh + 2.0 {
            return None;
        }
        let f = ((x - gx) / gw).clamp(0.0, 1.0) as f64;
        Some(self.timeline.0 + f * (self.timeline.1 - self.timeline.0))
    }

    /// The details in song select's main menu look at `[x, y, w]` (`l` laid out for them):
    /// a banner of the map's picture fading out downwards with the title and the artist over
    /// it, the mapper's line (with the EPIC pill, the status, the heart at its right), the
    /// difficulty as a chip in its rating's colour beside its rating pill, the stats as tiles
    /// (a caption over a value), Don and Kat as one bar split in proportion, and the pp as
    /// three chips. Returns where the OD tile's hit windows go when it's under the pointer.
    fn draw_bare_details(
        &mut self,
        c: &mut Canvas,
        e: &Entry,
        l: &Bare,
        [x, y, w]: [f32; 3],
        shown: &BareShown,
    ) -> Option<(f32, f32)> {
        use crate::style::{Surface, ink, role, space, surface, surface_fill, text};
        let (lx, iw) = (x + 14.0, w - 28.0);
        let split = self.split.is_some();
        // Split, a tall banner for the picture to show, the title, the artist and the
        // difficulty's row down at its foot; else a short one, the title near its top.
        let (banner_h, look) = if split {
            (SPLIT_BANNER_H, SPLIT_BANNER)
        } else {
            (BANNER_H, (BANNER_WHOLE, BANNER_PLATE))
        };
        self.draw_banner(c, e, [x, y, w, banner_h], look);
        // A song picked on its own comes in a part at a time, from the title down (`enter`).
        let mut cy = if split {
            y + banner_h - SPLIT_BANNER_FOOT - l.banner_text_h()
        } else {
            y + BANNER_TOP
        };
        let part = c.shapes.len();
        for line in &l.title {
            shadowed_text(c, lx, cy, l.title_size, rgba(ink::PRIMARY, 1.0), line);
            cy += Canvas::text_height(l.title_size) + 4.0;
        }
        self.enter(c, part, 0.0);
        let part = c.shapes.len();
        let artist = rgba(ink::SECONDARY, 1.0);
        shadowed_text(c, lx, cy, l.artist_size, artist, &l.artist);
        cy += Canvas::text_height(l.artist_size) + 6.0;
        self.enter(c, part, 40.0);
        let part = c.shapes.len();

        if split {
            self.draw_compact_row(c, e, [x, cy, w], shown);
            // The stats under the banner, a little room over them.
            cy = y + banner_h + COMPACT_TOP;
        } else {
            // The mapper's line: the EPIC pill (bumping on the beat) or the status at its right,
            // the heart left of that.
            self.epic_at = None;
            let mut right = x + w - 14.0;
            if self.epic {
                let pw = crate::epic::pill_width();
                right -= pw;
                crate::epic::pill(c, right, cy, 1.0, crate::style::pulse());
                self.epic_at = Some([right, cy]);
                right -= 8.0;
            } else if let Some((label, col)) = shown.status.filter(|s| !s.0.is_empty()) {
                let cw = Canvas::text_width(text::CAPTION, label) + 12.0;
                right -= cw;
                let fill = rgba(col, 0.22);
                c.round_rect([right, cy], [cw, MAPPER_H], MAPPER_H / 2.0, fill, fill, 0.0);
                let ty = cy + (MAPPER_H - Canvas::text_height(text::CAPTION)) / 2.0;
                c.text_centered(right + cw / 2.0, ty, text::CAPTION, rgba(col, 1.0), label);
                right -= 8.0;
            }
            if let Some(liked) = shown.liked {
                const HEART_W: f32 = 16.0;
                right -= HEART_W;
                let hover = c.hovered(right, cy, HEART_W, MAPPER_H);
                let colour = match (liked, hover) {
                    (true, _) => 0xff5fa2,
                    (false, true) => ink::SECONDARY,
                    (false, false) => ink::CAPTION,
                };
                crate::player::draw_heart(c, right + 3.5, cy + 2.5, 1.0, colour);
                c.hit(right, cy, HEART_W, MAPPER_H, Hit::SelectLike);
                right -= 6.0;
            }
            let ty = cy + (MAPPER_H - Canvas::text_height(text::BODY)) / 2.0;
            // No mapper given (a TJA chart without MAKER): no empty "mapped by".
            if !e.meta.creator.trim().is_empty() {
                let by = tr!("mapped by ", "譜面: ", "маппер: ");
                let bw = c.text(lx, ty, text::BODY, rgba(ink::CAPTION, 1.0), by);
                let name = Canvas::fit(text::BODY, (right - lx - bw).max(0.0), &e.meta.creator);
                c.text(lx + bw, ty, text::BODY, rgba(ink::PRIMARY, 1.0), &name);
            }
            cy += MAPPER_H + BLOCK_GAP;
            self.enter(c, part, 70.0);

            // The difficulty: a chip of its rating's colour, its rating pill at the right. The
            // rating counts from the last song's to this one's; while it's being worked out, a
            // skeleton stands in the pill's place.
            let part = c.shapes.len();
            let level = e.meta.level;
            let stars = follow(&mut self.stars_shown, shown.stars, c.dt_ms);
            let pill_w = if level.is_none() && stars.is_none() {
                let pw = Canvas::text_width(text::HEADING, "0.00") + text::HEADING * 1.6 + 6.0;
                let ph = text::HEADING + 6.0;
                crate::style::skeleton(c, [x + w - 14.0 - pw, cy + 3.0, pw, ph], ph / 2.0);
                pw
            } else {
                crate::style::star_chip(c, x + w - 14.0, cy + 3.0, text::HEADING, level, stars)
            };
            let chip = role::rating(level, stars);
            let luma = 0.299 * (chip >> 16 & 0xff) as f32
                + 0.587 * (chip >> 8 & 0xff) as f32
                + 0.114 * (chip & 0xff) as f32;
            let chip_ink = if luma > 140.0 { 0x15151d } else { ink::PRIMARY };
            let room = (iw - pill_w - space::SECTION - 16.0).max(0.0);
            let name = Canvas::fit(text::BODY, room, &e.meta.version);
            let cw = Canvas::text_width(text::BODY, &name) + 16.0;
            let fill = rgba(chip, 1.0);
            c.round_rect(
                [lx, cy],
                [cw, CHIP_H],
                crate::style::radius::SMALL,
                fill,
                fill,
                0.0,
            );
            let ty = cy + (CHIP_H - Canvas::text_height(text::BODY)) / 2.0;
            c.text(lx + 8.0, ty, text::BODY, rgba(chip_ink, 1.0), &name);
            cy += CHIP_H + BLOCK_GAP;
            self.enter(c, part, 100.0);
        }
        // The map's name and difficulty, the stats, the pp and the preview: sections split by
        // a line of the panels' edge, each coming in with what's under it.
        let divider = |c: &mut Canvas, at: f32| {
            let at = crate::render::snap_to_grid(at);
            c.rect(
                lx,
                at,
                iw,
                crate::style::tone::LINE,
                rgba(crate::style::tone::EDGE, 1.0),
            );
        };
        // Compact, the stats, Don and Kat's bar and the pp have more room between them (their
        // text kept small), each line in the middle of its gap.
        let gap = if split { COMPACT_GAP } else { BLOCK_GAP };
        let part = c.shapes.len();
        let above = if split { COMPACT_TOP } else { BLOCK_GAP };
        divider(c, cy - above / 2.0);
        self.enter(c, part, 130.0);

        // The stats as tiles: a caption over its value. OD is known at once, the rest when the
        // map is read (they fade in with it).
        let speed = self.requested_rate.0.max(0.01);
        let od = crate::beatmap::od_at_rate(
            self.mods.od_with(e.meta.od, crate::od_override::get()),
            self.requested_rate.0,
        );
        let d = shown.details;
        let mut tiles: Vec<(&str, String, f32)> =
            vec![(tr!("OD", "OD", "OD"), format!("{od:.1}"), 1.0)];
        let later = |v: Option<String>| v.unwrap_or_default();
        tiles.push((
            tr!("BPM", "BPM", "BPM"),
            later(d.map(|d| {
                if (d.bpm.1 - d.bpm.0).abs() < 0.5 {
                    format!("{:.0}", d.bpm.0 * speed)
                } else {
                    format!("{:.0}-{:.0}", d.bpm.0 * speed, d.bpm.1 * speed)
                }
            })),
            shown.fade,
        ));
        tiles.push((
            tr!("LENGTH", "長さ", "ДЛИНА"),
            later(d.map(|d| fmt_time(d.length_ms / speed))),
            shown.fade,
        ));
        tiles.push((
            tr!("NOTES", "ノーツ", "НОТЫ"),
            later(d.map(|d| (d.dons + d.kats).to_string())),
            shown.fade,
        ));
        if self.map_offset != 0.0 {
            let offset = format!("{:+.0}", self.map_offset);
            tiles.push((tr!("OFFSET", "オフセット", "ОФФСЕТ"), offset, 1.0));
        }
        let mut od_hovered = None;
        if self.split.is_some() {
            // Compact: one dense line, each a caption and its value, the cells even across it.
            let line_h = stat_line_h();
            let cell = iw / tiles.len() as f32;
            for (k, (caption, value, alpha)) in tiles.iter().enumerate() {
                let part = c.shapes.len();
                let tx = lx + k as f32 * cell;
                let mid = cy + line_h / 2.0;
                let caption = Canvas::fit(text::CAPTION, cell * 0.5, caption);
                let cw = c.text(
                    tx,
                    Canvas::text_y_mid(mid, text::CAPTION),
                    text::CAPTION,
                    rgba(ink::CAPTION, 1.0),
                    &caption,
                );
                let vx = tx + cw + STAT_LINE_GAP;
                let room = (tx + cell - STAT_LINE_GAP - vx).max(0.0);
                let size = fit_size(value, room);
                let value = Canvas::fit(size, room, value);
                c.text(
                    vx,
                    Canvas::text_y_mid(mid, size),
                    size,
                    rgba(ink::PRIMARY, *alpha),
                    &value,
                );
                // A value still being read: a skeleton where it goes.
                if value.is_empty() && d.is_none() {
                    let vh = Canvas::text_height(text::BODY);
                    let sw = (room * 0.6).min(28.0);
                    crate::style::skeleton(c, [vx, mid - vh / 2.0, sw, vh], 2.0);
                }
                if k == 0 && c.hovered(tx, cy, cell, line_h) {
                    od_hovered = Some((tx, cy + line_h));
                }
                self.enter(c, part, 130.0 + 25.0 * k as f32);
            }
            cy += line_h + gap;
        } else {
            let n = tiles.len() as f32;
            let tile_h = tile_h();
            let tw = (iw - (n - 1.0) * space::CARD_GAP) / n;
            for (k, (caption, value, alpha)) in tiles.iter().enumerate() {
                let part = c.shapes.len();
                let tx = lx + k as f32 * (tw + space::CARD_GAP);
                crate::style::tile(
                    c,
                    [tx, cy, tw, tile_h],
                    crate::style::radius::SMALL,
                    Surface::Raised,
                );
                // Caption and value in the tile's middle: the columns even across the row, no
                // empty run right of a short value.
                let room = tw - 12.0;
                let mid = tx + tw / 2.0;
                let caption = Canvas::fit(text::CAPTION, room, caption);
                c.text_centered(
                    mid,
                    Canvas::text_y_at(cy + TILE_PAD, text::CAPTION),
                    text::CAPTION,
                    rgba(ink::CAPTION, 1.0),
                    &caption,
                );
                // A value too long for its tile (a BPM range) gets smaller rather than cut short,
                // on the same baseline.
                let size = fitting_size(value, room);
                let value = Canvas::fit(size, room, value);
                let value_top = cy + tile_h - TILE_PAD - Canvas::text_height(text::HEADING);
                let drop = Canvas::text_height(text::HEADING) - Canvas::text_height(size);
                c.text_centered(
                    mid,
                    Canvas::text_y_at(value_top + drop, size),
                    size,
                    rgba(ink::PRIMARY, *alpha),
                    &value,
                );
                // A value still being read: a skeleton where it goes.
                if value.is_empty() && d.is_none() {
                    let vh = Canvas::text_height(text::HEADING);
                    let sw = room * 0.6;
                    crate::style::skeleton(c, [mid - sw / 2.0, value_top, sw, vh], 2.0);
                }
                if k == 0 && c.hovered(tx, cy, tw, tile_h) {
                    od_hovered = Some((tx, cy + tile_h));
                }
                self.enter(c, part, 130.0 + 25.0 * k as f32);
            }
            cy += tile_h + BLOCK_GAP;
        }

        // Don and Kat as one bar split in proportion, the counts at its ends, rolls and swells
        // between them. Compact (`split`), the pp simulator open takes its place.
        let compact = self.split.is_some();
        let bar_shown = !(compact && self.sim.is_some());
        let part = c.shapes.len();
        let notes = crate::game::note_colours();
        let caption_h = Canvas::text_height(text::CAPTION);
        let bar_y = cy + caption_h + 3.0;
        let cap_y = Canvas::text_y_at(cy, text::CAPTION);
        let bar_h = BAR_H - caption_h - 3.0;
        match d.filter(|d| d.dons + d.kats > 0 && bar_shown) {
            Some(d) => {
                let a = shown.fade;
                let dw = iw * d.dons as f32 / (d.dons + d.kats) as f32;
                c.rect(lx, bar_y, dw, bar_h, rgba(notes.don(), a));
                c.rect(lx + dw, bar_y, iw - dw, bar_h, rgba(notes.kat(), a));
                let quiet = rgba(ink::CAPTION, a);
                let dons = format!("{} {}", d.dons, tr!("DON", "ドン", "ДОН"));
                let kats = format!("{} {}", d.kats, tr!("KAT", "カッ", "КАТ"));
                c.text(lx, cap_y, text::CAPTION, quiet, &dons);
                c.text_right(lx + iw, cap_y, text::CAPTION, quiet, &kats);
                let mut extra = Vec::new();
                if d.rolls > 0 {
                    extra.push(format!("{} {}", d.rolls, tr!("ROLLS", "連打", "РОЛЛЫ")));
                }
                if d.swells > 0 {
                    extra.push(format!("{} {}", d.swells, tr!("SWELLS", "風船", "БАЛЛОНЫ")));
                }
                // Compact, what a TJA course leaves out is said here (no row of its own).
                if let (true, Some(what)) = (compact, shown.left_out) {
                    let room = iw
                        - Canvas::text_width(text::CAPTION, &dons)
                        - Canvas::text_width(text::CAPTION, &kats)
                        - 24.0;
                    let line = format!("{} {what}", left_out_label());
                    let line = Canvas::fit(text::CAPTION, room.max(0.0), &line);
                    let ink = rgba(ink::SECONDARY, a);
                    c.text_centered(lx + iw / 2.0, cap_y, text::CAPTION, ink, &line);
                } else if !extra.is_empty() {
                    c.text_centered(
                        lx + iw / 2.0,
                        cap_y,
                        text::CAPTION,
                        quiet,
                        &extra.join("  "),
                    );
                }
            }
            None if !bar_shown => {}
            None if d.is_none() => crate::style::skeleton(c, [lx, bar_y, iw, bar_h], 0.0),
            None => {
                let fill = surface_fill(Surface::Lifted);
                c.rect(lx, bar_y, iw, bar_h, fill);
            }
        }
        if bar_shown {
            cy += BAR_H + gap;
        }
        self.enter(c, part, 230.0);

        // What a full combo would give, on one button that opens the pp simulator in its place,
        // counting from the last song's; a skeleton while the map is being read.
        let part = c.shapes.len();
        divider(c, cy - gap / 2.0);
        let pp_now = follow_pp(&mut self.pp_shown, self.potential, c.dt_ms);
        let row_w = iw;
        let ty = cy + (PP_H - Canvas::text_height(text::BODY)) / 2.0;
        let pointer = c.mouse;
        let over = |[x, y, w, h]: [f32; 4]| {
            pointer.0 >= x && pointer.0 < x + w && pointer.1 >= y && pointer.1 < y + h
        };
        let button = [lx, cy, row_w, PP_H];
        if let Some(sim) = self.sim {
            // Open: what the play set up below gives, on the same button (it closes it).
            pp_chip(c, button, over(button));
            c.hit(lx, cy, row_w, PP_H, Hit::PpSim);
            // What it is: the play under it works out this pp.
            let what = tr!("PP CALCULATOR", "PP計算", "РАСЧЁТ PP");
            let pp = self
                .sim_pp
                .map_or_else(|| "...".to_string(), |pp| format!("{pp:.0}pp"));
            let pw = Canvas::text_width(text::BODY, &pp);
            // Its name when it fits whole beside the pp.
            if Canvas::text_width(text::CAPTION, what) <= row_w - pw - 24.0 {
                c.text(
                    lx + 8.0,
                    ty + 1.0,
                    text::CAPTION,
                    rgba(ink::SECONDARY, 1.0),
                    what,
                );
            }
            c.text_right(
                lx + row_w - 8.0,
                ty,
                text::BODY,
                rgba(ink::PRIMARY, 1.0),
                &pp,
            );
            // Under it, the play: accuracy and misses, each with its - and +.
            let sy = cy + PP_H + SIM_GAP;
            let half = (iw - space::CARD_GAP) / 2.0;
            let fields = [
                (
                    tr!("ACC", "精度", "ТОЧН."),
                    format!("{:.2}%", sim.accuracy * 100.0),
                ),
                (tr!("MISS", "ミス", "ПРОМАХИ"), sim.misses.to_string()),
            ];
            for (field, (label, value)) in fields.iter().enumerate() {
                let fx = lx + field as f32 * (half + space::CARD_GAP);
                surface(
                    c,
                    [fx, sy, half, PP_H],
                    crate::style::radius::SMALL,
                    Surface::Lifted,
                );
                // The whole field, for the wheel (its - and +, drawn after, take their clicks).
                c.hit(fx, sy, half, PP_H, Hit::PpSimField(field as u8));
                // Its name when there's room for it beside the value and its - and +.
                let bw = 14.0;
                let value_w = Canvas::text_width(text::CAPTION, value) + 8.0;
                let name_w = Canvas::text_width(text::CAPTION, label);
                let lw = if 8.0 + name_w + 6.0 + 2.0 * bw + value_w + 2.0 <= half {
                    c.text(
                        fx + 8.0,
                        ty + PP_H + SIM_GAP + 1.0,
                        text::CAPTION,
                        rgba(ink::CAPTION, 1.0),
                        label,
                    )
                } else {
                    -6.0
                };
                let minus = [fx + 8.0 + lw + 6.0, sy + 2.0, bw, PP_H - 4.0];
                let plus = [fx + half - 2.0 - bw, sy + 2.0, bw, PP_H - 4.0];
                for (dir, mark, [bx, by, bw, bh]) in [(-1, "-", minus), (1, "+", plus)] {
                    let lit = over([bx, sy, bw, PP_H]);
                    let level = if lit {
                        Surface::Lifted
                    } else {
                        Surface::Raised
                    };
                    surface(c, [bx, by, bw, bh], crate::style::radius::SMALL, level);
                    let mw = Canvas::text_width(text::BODY, mark);
                    let ink = if lit { ink::PRIMARY } else { ink::SECONDARY };
                    c.text(
                        bx + (bw - mw) / 2.0,
                        ty + PP_H + SIM_GAP,
                        text::BODY,
                        rgba(ink, 1.0),
                        mark,
                    );
                    c.hit(bx, sy, bw, PP_H, Hit::PpSimStep(field as u8, dir));
                }
                // The value between them, whole.
                let (mx, room) = (minus[0] + bw, plus[0] - minus[0] - bw);
                let size = fit_size(value, room - 4.0);
                let vw = Canvas::text_width(size, value);
                let vy = sy + (PP_H - Canvas::text_height(size)) / 2.0;
                c.text(
                    mx + (room - vw) / 2.0,
                    vy,
                    size,
                    rgba(ink::PRIMARY, 1.0),
                    value,
                );
            }
        } else if let Some(pp) = pp_now {
            // Closed: what a full combo gives at SS, 99% and 98%, every number whole.
            pp_chip(c, button, over(button));
            c.hit(lx, cy, row_w, PP_H, Hit::PpSim);
            // As many as fit whole: a narrow panel leaves out 98%, then 99%.
            let labels = ["SS", "99%", "98%"];
            let least = |k: usize| {
                Canvas::text_width(text::CAPTION, labels[k])
                    + 16.0
                    + Canvas::text_width(text::CAPTION, &format!("{:.0}", pp[k]))
            };
            let shown = (1..=3)
                .rev()
                .find(|&n| (0..n).all(|k| least(k) <= row_w / n as f32))
                .unwrap_or(1);
            let third = row_w / shown as f32;
            for (k, (label, pp)) in labels.into_iter().zip(pp).take(shown).enumerate() {
                let px = lx + k as f32 * third;
                let lw = c.text(
                    px + 8.0,
                    ty + 1.0,
                    text::CAPTION,
                    rgba(ink::CAPTION, 1.0),
                    label,
                );
                let room = third - 12.0 - lw - 4.0;
                let with_unit = format!("{pp:.0}pp");
                let value = if Canvas::text_width(text::BODY, &with_unit) <= room {
                    with_unit
                } else {
                    format!("{pp:.0}")
                };
                let size = fit_size(&value, room);
                let vy = cy + (PP_H - Canvas::text_height(size)) / 2.0;
                c.text(px + 12.0 + lw, vy, size, rgba(ink::PRIMARY, 1.0), &value);
            }
        } else if d.is_none() {
            crate::style::skeleton(c, [lx, cy, row_w, PP_H], crate::style::radius::SMALL);
        }
        if self.sim.is_some() {
            cy += SIM_GAP + PP_H;
        }
        self.enter(c, part, PP_ENTER_MS);
        cy += PP_H;
        if let (true, Some(what), false) = (l.left_out, shown.left_out, compact) {
            cy += 4.0;
            let label = format!("{} ", left_out_label());
            let lw = c.text(
                lx,
                cy,
                text::CAPTION,
                rgba(ink::CAPTION, shown.fade),
                &label,
            );
            let what = Canvas::fit(text::CAPTION, iw - lw, what);
            c.text(
                lx + lw,
                cy,
                text::CAPTION,
                rgba(ink::SECONDARY, shown.fade),
                &what,
            );
        }
        // Under the pp, the line over the preview (in the gap the layout leaves above it); compact,
        // the preview is a section of its own.
        if !compact {
            let part = c.shapes.len();
            divider(c, y + l.head - PREVIEW_GAP / 2.0);
            self.enter(c, part, PP_ENTER_MS);
        }
        od_hovered
    }

    /// The compact details' (`split`) one row under the artist at `[x, y, w]`: the difficulty as
    /// a chip in its rating's colour and who mapped it at the left; the rating badge at the right,
    /// the EPIC pill or the status and the heart left of it. Returns where what's under it starts.
    fn draw_compact_row(
        &mut self,
        c: &mut Canvas,
        e: &Entry,
        [x, y, w]: [f32; 3],
        shown: &BareShown,
    ) -> f32 {
        use crate::style::{ink, role, text};
        let lx = x + 14.0;
        let part = c.shapes.len();
        let mut right = x + w - 14.0;
        let level = e.meta.level;
        let stars = follow(&mut self.stars_shown, shown.stars, c.dt_ms);
        let pill_w = if level.is_none() && stars.is_none() {
            let pw = Canvas::text_width(text::HEADING, "0.00") + text::HEADING * 1.6 + 6.0;
            let ph = text::HEADING + 6.0;
            crate::style::skeleton(c, [right - pw, y, pw, ph], ph / 2.0);
            pw
        } else {
            crate::style::star_chip(c, right, y + 3.0, text::HEADING, level, stars)
        };
        right -= pill_w + 8.0;
        // The small marks on the row's middle line.
        let my = y + (CHIP_H - MAPPER_H) / 2.0;
        self.epic_at = None;
        if self.epic {
            let pw = crate::epic::pill_width();
            right -= pw;
            crate::epic::pill(c, right, my, 1.0, crate::style::pulse());
            self.epic_at = Some([right, my]);
            right -= 8.0;
        } else if let Some((label, col)) = shown.status.filter(|s| !s.0.is_empty()) {
            let cw = Canvas::text_width(text::CAPTION, label) + 12.0;
            right -= cw;
            crate::style::badge(c, [right, my, cw, MAPPER_H], col);
            let ty = Canvas::text_y_mid(my + MAPPER_H / 2.0, text::CAPTION);
            let ink = rgba(crate::style::badge_ink(col), 1.0);
            c.text_centered(right + cw / 2.0, ty, text::CAPTION, ink, label);
            right -= 8.0;
        }
        if let Some(liked) = shown.liked {
            const HEART_W: f32 = 16.0;
            right -= HEART_W;
            let hover = c.hovered(right, my, HEART_W, MAPPER_H);
            let colour = match (liked, hover) {
                (true, _) => 0xff5fa2,
                (false, true) => ink::SECONDARY,
                (false, false) => ink::CAPTION,
            };
            crate::player::draw_heart(c, right + 3.5, my + 2.5, 1.0, colour);
            c.hit(right, my, HEART_W, MAPPER_H, Hit::SelectLike);
            right -= 6.0;
        }
        // The difficulty: a badge as the rating's (a dark tint of its colour, a line of it round,
        // its name in it), up to most of the room left.
        let chip = role::rating(level, stars);
        let room = (right - lx).max(0.0);
        let name = Canvas::fit(text::BODY, (room * 0.6 - 16.0).max(0.0), &e.meta.version);
        let cw = if name.is_empty() {
            0.0
        } else {
            Canvas::text_width(text::BODY, &name) + 16.0
        };
        if cw > 0.0 {
            crate::style::badge(c, [lx, y, cw, CHIP_H], chip);
            let ty = Canvas::text_y_mid(y + CHIP_H / 2.0, text::BODY);
            let ink = rgba(crate::style::badge_ink(chip), 1.0);
            c.text(lx + 8.0, ty, text::BODY, ink, &name);
        }
        // Who mapped it, after it, where there's room (no empty "mapped by").
        let mx = lx + cw + if cw > 0.0 { 10.0 } else { 0.0 };
        if !e.meta.creator.trim().is_empty() && right - mx > 40.0 {
            let ty = Canvas::text_y_mid(y + CHIP_H / 2.0, text::BODY);
            let by = tr!("by ", "譜面: ", "маппер: ");
            let bw = shadowed_text(c, mx, ty, text::BODY, rgba(ink::CAPTION, 1.0), by);
            let name = Canvas::fit(text::BODY, (right - mx - bw).max(0.0), &e.meta.creator);
            shadowed_text(c, mx + bw, ty, text::BODY, rgba(ink::PRIMARY, 1.0), &name);
        }
        self.enter(c, part, 70.0);
        y + CHIP_H + BLOCK_GAP
    }

    /// The bare details' part drawn since shape `start` coming in `delay` ms after its song was
    /// picked: from `ENTER_SHIFT` right of its place and faded out, into place over `ENTER_MS`.
    fn enter(&self, c: &mut Canvas, start: usize, delay: f32) {
        let Some((_, at)) = self.entered else {
            return;
        };
        use crate::style::motion;
        let since = at + Duration::from_secs_f32(delay / 1000.0);
        let p = motion::entrance(since, ENTER_MS, motion::out_quint);
        if p >= 1.0 {
            return;
        }
        c.transform_since(start, [0.0, 0.0], 1.0, ENTER_SHIFT * (1.0 - p), 0.0);
        c.fade_since(start, p, 1.0);
    }

    /// The map's picture over `[x, y, w, h]` (cover-cropped), whole down to `whole` of it and
    /// fading out downwards in its shader, with a shade over it darkening the left side under the
    /// title. It fades in when it first arrives; nothing until then.
    fn draw_banner(
        &mut self,
        c: &mut Canvas,
        e: &Entry,
        rect: [f32; 4],
        (whole, plate): (f32, (f32, f32)),
    ) {
        let Some(bg) = background_path(e) else {
            return;
        };
        // Songs changing fast (Random spammed, the list flown through): a picture not read yet
        // waits for them to settle, as the map itself does (`DEBOUNCE`).
        let settling = self
            .changed_at
            .is_some_and(|t| crate::render::frame_now().saturating_duration_since(t) < DEBOUNCE);
        let read = c
            .picture_slots
            .contains_key(&crate::render::path_picture_key(&bg));
        if settling && !read {
            return;
        }
        let since = self
            .banner_seen
            .as_ref()
            .filter(|(p, _)| *p == bg)
            .map(|(_, t)| *t);
        let a = match since {
            Some(t) => crate::render::progress(t, crate::style::motion::CONTENT_MS),
            None if crate::render::animations() => 0.0,
            None => 1.0,
        };
        let [x, y, w, h] = rect;
        let dim = BANNER_BRIGHT;
        // At the top of the details' panel, its top corners as round as the panel's, fading out
        // downwards in the shader (one quad instead of stacked clipped layers).
        let drawn = c.picture_cover_round_top(
            &bg,
            rect,
            rect,
            [dim, dim, dim, a],
            crate::style::radius::CARD,
            Some(whole),
        );
        // A plate of the panel's tone over the banner, dark under the title and artist at its
        // left and lighter to its right end in an eased sweep, fading out downwards with the
        // picture so no edge shows where the picture has gone.
        if drawn {
            let tone = crate::style::SURFACE_1;
            c.shade_eased_fade_down(
                [x, y],
                [w, h],
                crate::style::radius::CARD,
                rgba(tone, plate.0 * a),
                rgba(tone, plate.1 * a),
                whole,
            );
        }
        if drawn && since.is_none() {
            self.banner_seen = Some((bg, crate::render::frame_now()));
        }
    }

    /// Details and the autoplay preview in the right-hand panel (virtual units). Returns the
    /// shape range to clip and the clip area. `button`: the action under the timeline as (verb,
    /// key hint), e.g. ("Play from", "Shift+Enter"). `status`: the map's on osu! (label, colour),
    /// shown by the mapper's name. Bare, the details of the song before a change go out under the
    /// new ones.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_panel(
        &mut self,
        c: &mut Canvas,
        library: &[Entry],
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        scroll_speed: f64,
        stars: Option<f32>,
        button: Option<(&str, &str)>,
        status: Option<(&str, u32)>,
        liked: Option<bool>,
    ) -> Option<(std::ops::Range<usize>, crate::layout::Area)> {
        self.draw_leaving(c);
        let start = c.shapes.len();
        let clip = self.draw_details(
            c,
            library,
            x,
            y,
            w,
            h,
            scroll_speed,
            stars,
            button,
            status,
            liked,
        );
        if self.bare && crate::render::animations() {
            self.keep_drawn(c, start, clip.as_ref().map(|(r, _)| r.clone()));
        }
        clip
    }

    /// The details of the song before the one just picked on its own (`entered`), as they
    /// were last drawn, going out: `LEAVE_SHIFT` left and fading over `LEAVE_MS`.
    fn draw_leaving(&mut self, c: &mut Canvas) {
        let Some(at) = self.entered.map(|(_, at)| at) else {
            self.leaving = None;
            return;
        };
        if self.leaving.as_ref().is_none_or(|(_, t)| *t != at) {
            self.leaving = Some((std::mem::take(&mut self.drawn), at));
        }
        let Some((gone, at)) = &self.leaving else {
            return;
        };
        let p = crate::style::motion::entrance(*at, LEAVE_MS, |t| t * t);
        if p >= 1.0 || gone.shapes.is_empty() {
            return;
        }
        let start = c.shapes.len();
        c.shapes.extend_from_slice(&gone.shapes);
        for (r, rect) in &gone.clips {
            c.clips.push((r.start + start..r.end + start, *rect));
        }
        c.transform_since(start, [0.0, 0.0], 1.0, -LEAVE_SHIFT * p, 0.0);
        c.fade_since(start, 1.0 - p, 1.0);
    }

    /// Keeps what the details drew since shape `start` for `draw_leaving`, the preview's lane
    /// (`lane`, cut to its box by a clip of its own) left out.
    fn keep_drawn(&mut self, c: &Canvas, start: usize, lane: Option<std::ops::Range<usize>>) {
        let d = &mut self.drawn;
        d.shapes.clear();
        d.clips.clear();
        let Some(drawn) = c.shapes.get(start..) else {
            return;
        };
        d.shapes.extend_from_slice(drawn);
        if let Some(r) = lane {
            let n = d.shapes.len();
            let (a, b) = (
                r.start.saturating_sub(start).min(n),
                r.end.saturating_sub(start).min(n),
            );
            for sh in &mut d.shapes[a..b] {
                sh.fill[3] = 0.0;
                sh.border[3] = 0.0;
            }
        }
        d.clips.extend(
            c.clips
                .iter()
                .filter(|(r, _)| r.start >= start && r.start < r.end)
                .map(|(r, rect)| (r.start - start..r.end - start, *rect)),
        );
    }

    /// `draw_panel`'s details themselves.
    #[allow(clippy::too_many_arguments)]
    fn draw_details(
        &mut self,
        c: &mut Canvas,
        library: &[Entry],
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        scroll_speed: f64,
        stars: Option<f32>,
        button: Option<(&str, &str)>,
        status: Option<(&str, u32)>,
        liked: Option<bool>,
    ) -> Option<(std::ops::Range<usize>, crate::layout::Area)> {
        // The preview shows ~640 units of lane (several notes ahead), so its height follows the
        // panel width; the panel shrinks to fit instead of leaving empty space.
        // The lane alone fills the box, laid out as on a 4:3 screen (640 units wide): bigger notes,
        // fewer of them, whatever the panel's shape.
        let (band_top, band_h) = game::lane_only();
        // Bare, the timeline's bars are lower: quieter, and room under them.
        let graph_h = if self.bare { BARE_GRAPH_H } else { 40.0 };
        // Under the preview box: the timeline and its button, and bare, the board's strip.
        let after_box = 8.0 + graph_h + 44.0;
        // The preview's height per unit of its size. Smaller, it shows more of the lane ahead in
        // a shorter box: as small as dragged, and smaller still where the panel would otherwise
        // be taller than `h` (the details above it being `head` tall).
        let (k_full, lane_w) = lane_fit(
            w,
            crate::ui::Select::panel_width(c.width, crate::ui::LIST_SHARE.2),
        );
        let unit = band_h * k_full;
        self.size_unit = unit;
        let wanted = if self.size > 0.0 {
            self.size.clamp(PREVIEW_SIZE.0, PREVIEW_SIZE.1)
        } else {
            PREVIEW_SIZE.1
        };
        let fitted = |head: f32| {
            ((h - head - after_box - 12.0) / unit).clamp(PREVIEW_SIZE.0, PREVIEW_SIZE.1)
        };
        self.graph_rect = None;
        self.play_rect = None;
        self.box_rect = None;
        self.corridor = None;
        let Some(i) = self.seen.filter(|&i| i < library.len()) else {
            (self.size_cap, self.size_shown) = (fitted(110.0), wanted.min(fitted(110.0)));
            let below = unit * self.size_shown + 12.0 + after_box;
            self.panel_bottom = y + h.min(110.0 + below);
            if !self.bare {
                c.round_rect(
                    [x, y],
                    [w, h.min(110.0 + below)],
                    8.0,
                    rgba(0x15151d, 0.82),
                    rgba(0x2a2a38, 1.0),
                    1.0,
                );
            }
            return None;
        };
        let e = &library[i];
        // Map details and the timeline appear with the map; the autoplay runs on the map's music,
        // so it joins them when that plays.
        let ready = self.map.as_ref().is_some_and(|(mi, ..)| *mi == i);
        let playing = self.ready(library, i) || (self.ready_for == Some(i) && ready);
        if playing {
            self.ready_for = Some(i);
        }
        if ready && self.shown_for != Some(i) {
            self.shown_for = Some(i);
            self.ready_at = Some(Instant::now());
        }
        let fade = self
            .ready_at
            .filter(|_| ready)
            .map(|t| {
                // Bare: content's entrance from the design tokens; boxed as it was.
                use crate::style::motion;
                if self.bare {
                    motion::entrance(t, motion::CONTENT_MS, motion::out_quint)
                } else {
                    crate::render::progress(t, 180.0)
                }
            })
            .unwrap_or(0.0);
        let loaded = self.map.as_ref().filter(|(mi, _, _, _)| *mi == i && ready);

        // Text sits on a 14 unit inset; long names wrap onto a second line before they are cut.
        let (lx, iw) = (x + 14.0, w - 28.0);
        let (th_title, th_body, th_small) = (
            Canvas::text_height(12.0),
            Canvas::text_height(9.0),
            Canvas::text_height(8.0),
        );
        let title = wrap(12.0, iw - self.title_room, e.title(), 2);
        let artist = wrap(9.0, iw, e.artist(), 2);
        // The star pill is about this wide; the difficulty name wraps beside it.
        let pill_w = Canvas::text_width(9.0, "0.00") + 9.0 * 1.6 + 6.0;
        let version = wrap(9.0, iw - pill_w - 8.0, &format!("[{}]", e.meta.version), 2);
        // Stats as label + value, flowed left to right; the first group is the map's, the second its notes.
        let mut stats: [Vec<StatCell>; 4] = [
            // As it plays: EZ / HR / Precise, and the speed, which shrinks or grows the windows.
            vec![(
                "OD",
                format!(
                    "{:.1}",
                    crate::beatmap::od_at_rate(
                        self.mods.od_with(e.meta.od, crate::od_override::get()),
                        self.requested_rate.0,
                    )
                ),
                1.0,
            )],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ];
        // What a full combo would give: SS, 99%, 98%.
        if let Some(pp) = self.potential {
            for (label, pp) in ["SS", "99%", "98%"].into_iter().zip(pp) {
                stats[2].push((label, format!("{:.0}pp", pp), 1.0));
            }
        }
        if let Some((_, _, d, _)) = loaded {
            // At the speed played (DT, HT, the rate): as fast and as long as it'll be.
            let speed = self.requested_rate.0.max(0.01);
            let bpm = if (d.bpm.1 - d.bpm.0).abs() < 0.5 {
                format!("{:.0}", d.bpm.0 * speed)
            } else {
                format!("{:.0}-{:.0}", d.bpm.0 * speed, d.bpm.1 * speed)
            };
            stats[0].push(("BPM", bpm, fade));
            stats[0].push(("Length", fmt_time(d.length_ms / speed), fade));
            if self.map_offset != 0.0 {
                stats[0].push(("Offset", format!("{:+.0} ms", self.map_offset), fade));
            }
            stats[1].push(("Notes", (d.dons + d.kats).to_string(), fade));
            stats[1].push(("Don", d.dons.to_string(), fade));
            stats[1].push(("Kat", d.kats.to_string(), fade));
            if d.rolls > 0 {
                stats[1].push(("Rolls", d.rolls.to_string(), fade));
            }
            if d.swells > 0 {
                stats[1].push(("Swells", d.swells.to_string(), fade));
            }
            // What a TJA course uses that isn't supported yet: a row of its own.
            if let Some((_, what)) = self.left_out.as_ref().filter(|(li, _)| *li == i) {
                let label = left_out_label();
                let room = iw - Canvas::text_width(8.0, &format!("{label} "));
                stats[3].push((label, Canvas::fit(8.0, room, what), fade));
            }
        }
        let cell_w =
            |(label, value, _): &StatCell| Canvas::text_width(8.0, &format!("{label} {value}"));
        let rows: Vec<Vec<StatRow<'_>>> = stats
            .iter()
            // The first three always hold a row; what a TJA course leaves out only when it has some.
            .take(if stats[3].is_empty() { 3 } else { 4 })
            .map(|group| {
                let mut rows: Vec<StatRow<'_>> = vec![Vec::new()];
                let mut used = 0.0;
                for cell in group {
                    let cw = cell_w(cell);
                    if used > 0.0 && used + STAT_GAP + cw > iw {
                        rows.push(Vec::new());
                        used = 0.0;
                    }
                    used += if used > 0.0 { STAT_GAP } else { 0.0 } + cw;
                    rows.last_mut().unwrap().push(cell);
                }
                rows
            })
            .collect();
        // Three rows are always kept (the pp one too), so the panel doesn't jump when the details
        // arrive.
        let stat_rows = rows.iter().map(Vec::len).sum::<usize>().max(3);
        let (title_step, body_step, row_h) = (th_title + 4.0, th_body + 3.0, th_small + 4.0);
        let head = 12.0
            + title.len() as f32 * title_step
            + 3.0
            + artist.len() as f32 * body_step
            + 4.0
            + 13.0
            + 6.0
            + (version.len() as f32 * body_step).max(15.0)
            + 6.0
            + stat_rows as f32 * row_h
            + 6.0;
        // Bare: the banner layout, of its own height.
        let left_out_text = stats[3].first().map(|cell| cell.1.clone());
        let split = self.split.filter(|_| self.bare);
        let bare_layout = self.bare.then(|| {
            let sim = self.sim.is_some();
            Bare::new(e, iw, left_out_text.is_some(), sim, split.is_some())
        });
        let head = bare_layout.as_ref().map_or(head, |l| l.head);
        (self.size_cap, self.size_shown) = (fitted(head), wanted.min(fitted(head)));
        // The lane fills the panel's width (at the size chosen), however little room the details
        // leave: the panel grows down, into the leaderboard's room, rather than the lane shrink.
        // A .tko map that moves things off the lane gets a taller box, as far as the room left
        // under the panel goes (both ends drawn in alike when it doesn't all fit). Split, the
        // box is as tall as its section leaves it, and all the lane's reach fits it.
        let [top, bottom] = self.shown_reach();
        // Split, the preview's section laid out for the lane's band and its reach.
        let split_lane = split
            .and_then(|s| s.area)
            .map(|area| SplitLane::new(area, band_h, bottom - top, k_full * wanted));
        let graph_h = split_lane.map_or(graph_h, |l| l.graph_h);
        let after_box = 8.0 + graph_h + 44.0;
        let (k, fit) = match (split, split_lane) {
            (Some(_), Some(l)) => (l.k, l.fit),
            // The leaderboard has the section: no lane.
            (Some(_), None) => (k_full * wanted, 1.0),
            (None, _) => {
                let k = k_full * wanted;
                let extra = (bottom - top - band_h) * k;
                let room = (h - head - band_h * k - after_box).max(0.0);
                (k, if extra > room { room / extra } else { 1.0 })
            }
        };
        // A plain map's lane alone (no .tko reach past it): its corridor drawn, as in a play.
        let lane_alone = bottom - top <= band_h + 0.5;
        let band_top = band_top - (band_top - top) * fit;
        let band_h = band_h + (bottom - top - band_h) * fit;
        let preview_h = band_h * k;
        let h = head + preview_h + after_box;
        // Bare, the panel ends under what's on it: the room for "Play from" under the timeline
        // only once a start point is picked (it grows into it).
        let tail = if self.bare && self.marker(e).is_none() {
            PLAY_FROM_ROOM
        } else {
            0.0
        };
        self.panel_bottom = match (split, split_lane) {
            (Some(_), Some(l)) => l.lane[0] + l.lane[1] + after_box - tail,
            (Some(s), None) => y + s.head,
            (None, _) => y + h - tail,
        };
        // Bare, it's on a panel of its own that song select draws under the whole page.
        if !self.bare {
            c.round_rect(
                [x, y],
                [w, h],
                8.0,
                rgba(0x15151d, 0.82),
                rgba(0x2a2a38, 1.0),
                1.0,
            );
        }

        // The OD under the pointer: where its hit windows go, once all else is drawn.
        let mut od_hovered = None;
        let mut cy = y + 12.0;
        let is_loaded = loaded.is_some();
        if let Some(l) = &bare_layout {
            let shown = BareShown {
                stars,
                status,
                liked,
                details: loaded.map(|(_, _, d, _)| *d),
                left_out: left_out_text.as_deref(),
                fade,
            };
            od_hovered = self.draw_bare_details(c, e, l, [x, y, w], &shown);
            cy = y + head;
        } else {
            for line in &title {
                c.text(
                    lx,
                    cy,
                    crate::style::text::HEADING,
                    rgba(0xffffff, 1.0),
                    line,
                );
                cy += title_step;
            }
            cy += 3.0;
            // Outside the bare look the panel keeps its own greys.
            let bare = self.bare;
            let ink = |bare_ink: u32, boxed: u32| if bare { bare_ink } else { boxed };
            for line in &artist {
                let col = ink(crate::style::ink::SECONDARY, 0xc8c8d8);
                c.text(lx, cy, 9.0, rgba(col, 1.0), line);
                cy += body_step;
            }
            cy += 4.0;
            // The EPIC mark at the very right of the mapper's line, bumping on the beat. It stands in
            // the status's place, as on the website: the map's osu! status isn't shown with it.
            self.epic_at = None;
            let epic_w = if self.epic {
                let pw = crate::epic::pill_width();
                let ex = x + w - 14.0 - pw;
                crate::epic::pill(c, ex, cy, 1.0, crate::style::pulse());
                self.epic_at = Some([ex, cy]);
                pw + 8.0
            } else {
                0.0
            };
            // Otherwise the status on osu!.
            let status_w = epic_w
                + match status.filter(|s| !s.0.is_empty() && !self.epic) {
                    Some((label, col)) => {
                        let lw = Canvas::text_width(7.0, label) + 12.0;
                        let sx = x + w - 14.0 - epic_w - lw;
                        // Bare, the chip is its colour's tone alone, no edge.
                        let (fill, edge) = if bare {
                            (rgba(col, 0.22), 0.0)
                        } else {
                            (rgba(col, 0.15), 1.0)
                        };
                        c.round_rect([sx, cy], [lw, 13.0], 6.5, fill, rgba(col, 0.9), edge);
                        c.text_centered(sx + lw / 2.0, cy + 3.0, 7.0, rgba(col, 1.0), label);
                        lw + 8.0
                    }
                    None => 0.0,
                };
            // The heart, left of the status: whether the song's liked, and a click changes it.
            let heart_w = match liked {
                Some(liked) => {
                    const HEART_W: f32 = 16.0;
                    let hx = x + w - 14.0 - status_w - HEART_W;
                    let hover = c.hovered(hx, cy, HEART_W, 13.0);
                    let colour = match (liked, hover) {
                        (true, _) => 0xff5fa2,
                        (false, true) => 0xc8c8d8,
                        (false, false) => 0x5a5a70,
                    };
                    crate::player::draw_heart(c, hx + 3.5, cy + 2.5, 1.0, colour);
                    c.hit(hx, cy, HEART_W, 13.0, Hit::SelectLike);
                    HEART_W + 6.0
                }
                None => 0.0,
            };
            // No mapper given (a TJA chart without MAKER): no empty "mapped by".
            let mapper = if e.meta.creator.trim().is_empty() {
                String::new()
            } else {
                Canvas::fit(
                    8.0,
                    iw - status_w - heart_w,
                    &format!("mapped by {}", e.meta.creator),
                )
            };
            c.text(
                lx,
                cy + (13.0 - th_small) / 2.0,
                8.0,
                rgba(ink(crate::style::ink::CAPTION, 0x8a8aa0), 1.0),
                &mapper,
            );
            cy += 13.0 + 6.0;
            crate::style::map_pill(c, x + w - 14.0, cy + 3.0, 9.0, e.meta.level, stars);
            // The difficulty's name: bare, in its rating's colour (the accent is Play's and the
            // selection's), else the accent.
            let version_ink = if bare {
                crate::style::role::rating_ink(e.meta.level, stars)
            } else {
                accent()
            };
            for line in &version {
                c.text(lx, cy + 3.0, 9.0, rgba(version_ink, 1.0), line);
                cy += body_step;
            }
            cy += (15.0 - version.len() as f32 * body_step).max(0.0) + 6.0;
            // Muted labels, bright values.
            for group in &rows {
                for row in group {
                    let mut sx = lx;
                    for cell @ (label, value, alpha) in row.iter().copied() {
                        if *label == "OD" && c.hovered(sx, cy - 2.0, cell_w(cell), row_h) {
                            od_hovered = Some((sx, cy + row_h));
                        }
                        let (label_ink, value_ink) = if bare {
                            (crate::style::ink::CAPTION, crate::style::ink::PRIMARY)
                        } else {
                            (0x8a8aa0, 0xe8e8f4)
                        };
                        let size = crate::style::text::BODY;
                        sx += c.text(sx, cy, size, rgba(label_ink, *alpha), &format!("{label} "));
                        sx += c.text(sx, cy, size, rgba(value_ink, *alpha), value) + STAT_GAP;
                    }
                    cy += row_h;
                }
            }
            cy = cy.max(y + 12.0)
                + (stat_rows.saturating_sub(rows.iter().map(Vec::len).sum()) as f32) * row_h
                + 6.0;
        }

        // Split with the leaderboard in the preview's section: the details alone.
        if split.is_some() && split_lane.is_none() {
            self.graph_rect = None;
            self.play_rect = None;
            if let Some(at) = od_hovered {
                let windows = crate::game::Windows::for_map(
                    &e.meta,
                    self.mods,
                    crate::od_override::get(),
                    crate::game::WindowRule::Osu,
                );
                draw_hit_windows(c, at, windows.at_rate(self.requested_rate.0));
            }
            return None;
        }
        // Autoplay preview: just the lane band of the playfield, fitted to the box.
        // As wide as the lane at that scale (all of the panel at full size), centred.
        let bw = lane_w * k;
        // Bare, the lane's tile starts at the details' left edge: the drum at its start, or (the
        // drum off in the settings) off it to the left, cut off.
        let drum_w = crate::game::PANEL_W * k;
        let bx = if self.bare {
            x + 14.0 - if self.hide_drum { drum_w } else { 0.0 }
        } else {
            x + 10.0 + (w - 20.0 - bw) / 2.0
        };
        let (by, bh) = match split_lane {
            Some(l) => (l.lane[0], l.lane[1]),
            None => (cy, preview_h),
        };
        // Bare, the autoplay is a lane like the menu's, its drum at the left, running on to the
        // tiles' right edge; the notes come in from its end.
        let lane_end = if self.bare {
            x + w - 14.0 - LANE_FADE[0]
        } else {
            bx + bw
        };
        // Bare, the tile's left edge: the drum's, or where the lane starts without it.
        let lane_x = if self.bare { x + 14.0 } else { bx };
        // Bare: no lines round it, its corners as round as the tiles', the tile out past the
        // lane's end (the lane is cut square to its box, so a tile ending with it would lose its
        // right corners).
        let strip = [lane_x, by, lane_end - lane_x + LANE_FADE[0], bh];
        // A tile as the stats' are, not a dark box: on a bright picture that read as black.
        // No line round it: a tone a step lighter than the panel's sets it apart.
        if self.bare {
            let r = crate::style::radius::SMALL;
            crate::style::tile(c, strip, r, crate::style::Surface::Raised);
            if split.is_some() && lane_alone {
                // The lane's corridor across the tile, as tall as the notes and the drum, a
                // step lighter than the tile with a thin line along its top and bottom: the
                // notes run along a track, not through empty room.
                let lh = band_h * k;
                let ly = crate::render::snap_to_grid(by + (bh - lh) / 2.0);
                let [sx, _, sw, _] = strip;
                let fill = crate::style::surface_fill(crate::style::Surface::Lifted);
                c.rect(sx, ly, sw, lh, fill);
                let edge = rgba(crate::style::tone::EDGE, 1.0);
                let line = crate::style::tone::LINE;
                c.rect(sx, ly, sw, line, edge);
                c.rect(sx, ly + lh - line, sw, line, edge);
                self.corridor = Some([sx, ly, sw, lh]);
            }
        } else {
            c.round_rect(
                [bx, by],
                [bw, bh],
                6.0,
                rgba(0x0c0c12, 1.0),
                rgba(0x2a2a38, 1.0),
                1.0,
            );
        }
        self.box_rect = Some([bx, by, bw, bh]);
        self.drawn_in(c);
        c.hit(lane_x, by, lane_end.min(c.width) - lane_x, bh, Hit::Preview);
        // Bare, the timeline is as wide as the lane's tile, edge to edge.
        let (gx, gw) = if self.bare {
            (strip[0], strip[2])
        } else {
            (bx, bw)
        };
        let loaded = is_loaded;
        if loaded {
            let graph_start = c.shapes.len();
            self.draw_timeline(c, e, [gx, by + bh + 8.0, gw, graph_h], button);
            // Seeking goes by the music: until this map's plays, a click would move the last one.
            if !playing {
                self.graph_rect = None;
                self.play_rect = None;
            }
            if self.bare {
                // Nothing behind it to cover it with: it fades in itself.
                c.fade_since(graph_start, fade, 1.0);
            } else if fade < 1.0 {
                c.rect(
                    bx - 1.0,
                    by + bh + 4.0,
                    bw + 2.0,
                    graph_h + 40.0,
                    rgba(0x15151d, 1.0 - fade),
                );
            }
        } else {
            self.graph_rect = None;
            self.play_rect = None;
        }
        // What's over the panel goes last, on the way out with a lane or without.
        let tip = od_hovered.map(|at| {
            let windows = crate::game::Windows::for_map(
                &e.meta,
                self.mods,
                crate::od_override::get(),
                crate::game::WindowRule::Osu,
            );
            (at, windows.at_rate(self.requested_rate.0))
        });
        let Some(game) = self.game.as_mut().filter(|_| loaded && playing) else {
            if let Some((at, windows)) = tip {
                draw_hit_windows(c, at, windows);
            }
            return None;
        };
        // Bare, the notes run on under the tile's rounded end and come in out of its edge, not
        // out of the middle of it where the lane would be cut square.
        let notes_end = if self.bare {
            bare_notes_end(lane_end)
        } else {
            lane_end
        };
        let full_w = c.width;
        let start = c.shapes.len();
        c.width = (notes_end - bx) / k;
        game.view.kiai_effects = self.kiai_effects;
        // Never the black playfield here (a black box on the map's picture); plays keep it.
        crate::game::with_look(preview_look(crate::game::playfield_look()), || {
            game.draw_playfield(c, scroll_speed)
        });
        // Split, in the middle of its box (the band takes a share of its height).
        let centre = if split.is_some() {
            (bh - band_h * k) / 2.0
        } else {
            0.0
        };
        let oy = by - band_top * k + centre;
        c.transform_since(start, [0.0, 0.0], k, bx, oy);
        c.width = full_w;
        // Bare, the lane fits its round tile; its notes slide in from under the tile's edge (the
        // lane is clipped to it: faded in short of it, they seemed to appear in plain view).
        if self.bare {
            bare_lane_end(&mut c.shapes, start, bx, lane_x, notes_end, bh);
        }
        if button.is_some() {
            // Song select only (download previews have no fullscreen view). Only where it fits
            // the lane (narrow, it ran out past its left edge), shorter first.
            let right = if self.bare {
                lane_end - LANE_FADE[1] - 4.0
            } else {
                bx + bw - 6.0
            };
            // Right of the drum (the hit target's where it starts).
            let room = right - (bx + drum_w) - 6.0;
            let size = crate::style::text::CAPTION;
            let hint = if crate::mobile::active() {
                &["tap: fullscreen"][..]
            } else {
                &["click: fullscreen (Ctrl+Enter)", "click: fullscreen"][..]
            }
            .iter()
            .find(|s| Canvas::text_width(size, s) <= room);
            if let Some(hint) = hint {
                // Split, in the room under the notes, clear of them.
                let hy = if split.is_some() {
                    Canvas::text_y_at(by + bh - 3.0 - Canvas::text_height(size), size)
                } else {
                    by + bh - 10.0
                };
                c.text_right(right, hy, size, rgba(0x6f6f86, 1.0), hint);
            }
        }
        if fade < 1.0 {
            if self.bare {
                // Nothing dark to cover them with: the notes fade in themselves.
                c.fade_since(start, fade, 1.0);
            } else {
                c.round_rect(
                    [bx, by],
                    [bw, bh],
                    6.0,
                    rgba(0x0c0c12, 1.0 - fade),
                    rgba(0, 0.0),
                    0.0,
                );
            }
        }
        // Bare, from the tile's left edge: the drum and its panel left of it are cut off.
        let clip_x = if self.bare { lane_x } else { bx };
        let clip = crate::layout::Area {
            x: (clip_x + c.left) / (full_w + 2.0 * c.left),
            y: by / c.height,
            w: (notes_end - clip_x) / (full_w + 2.0 * c.left),
            h: bh / c.height,
        };
        // Only the lane is clipped to the box, hard: notes with a clip of their own (a TJA
        // chart's #NOTESPAWN brings them in from above or below the lane) are cut to it too.
        c.clip_since(start, clip_x, by, notes_end - clip_x, bh);
        let end = c.shapes.len();
        if let Some((at, windows)) = tip {
            draw_hit_windows(c, at, windows);
        }
        Some((start..end, clip))
    }
}

pub use crate::config::PREVIEW_SIZE;

/// Space between the stats in the details.
const STAT_GAP: f32 = 10.0;

/// Split (song select's sections), how much of its box's height the lane's band takes at most,
/// in the middle of it: room above and under the big notes and the drum, and for the hint under
/// them.
const SPLIT_LANE_FILL: f32 = 0.62;
/// Split, the density graph under the lane: its share of the section's room, between the bare
/// timeline's height and this much (tall enough to read its dense runs and peaks at a glance).
const SPLIT_GRAPH_SHARE: f32 = 0.28;
const SPLIT_GRAPH_MAX: f32 = 84.0;
/// Split, a .tko map whose playfield reaches past the lane: it is fitted whole into the box, its
/// notes never drawn smaller than this share of a plain map's (what's still out of reach then is
/// cut evenly at both ends).
const SPLIT_REACH_MIN: f32 = 0.5;

/// Bare: how far the autoplay's lane fades in at its left end (out from the drum's side) and its
/// right (where the notes come in), and how tall the timeline's bars are.
const LANE_FADE: [f32; 2] = [10.0, 28.0];

/// The look the preview's playfield is drawn with: the play's, but never the black playfield.
pub(crate) fn preview_look(play: crate::game::Look) -> crate::game::Look {
    crate::game::Look {
        black_lane: false,
        ..play
    }
}

/// Bare, where the lane's notes end, its tile's right edge (the lane ending at `lane_end`, the
/// tile running on past it): they come in out of it, not out of the middle of the tile.
pub(crate) fn bare_notes_end(lane_end: f32) -> f32 {
    lane_end + LANE_FADE[0]
}

/// Bare, the lane's shapes as drawn from `start` (its drum's panel from `lane_x`, the lane on
/// to `notes_end`, `band_h` tall) fitted to its tile, which starts at `tile_left`: at the drum
/// (`lane_x`), or right of it with the drum off. Its own bands (as tall as the tile, running its
/// length) start at the tile's edges and are as round (square, it made the lane read as a square
/// box on a dark picture); the drum's panel is round at the tile's edge and square against the
/// lane, or not drawn at all left of the tile; the lines along it aren't drawn.
pub(crate) fn bare_lane_end(
    shapes: &mut Vec<crate::render::Shape>,
    start: usize,
    lane_x: f32,
    tile_left: f32,
    notes_end: f32,
    band_h: f32,
) {
    let round = crate::style::radius::SMALL;
    let mut panels = Vec::new();
    for (i, sh) in shapes.iter_mut().enumerate().skip(start) {
        if sh.params != [0.0; 4] || sh.angle != 0.0 {
            continue;
        }
        let (left, right) = (sh.center[0] - sh.half[0], sh.center[0] + sh.half[0]);
        let tall = sh.half[1] * 2.0 >= band_h * 0.5;
        if tall && right >= notes_end - 0.5 {
            // From the tile's left edge, so it's round at both ends.
            if (left - lane_x).abs() < 0.5 {
                sh.center[0] = (tile_left + right) / 2.0;
                sh.half[0] = (right - tile_left) / 2.0;
            }
            sh.params[0] = round;
        } else if tall && (left - lane_x).abs() < 0.5 && right - left < (notes_end - lane_x) / 2.0 {
            if right <= tile_left + 0.5 {
                // Left of the tile, cut off with the drum: none of it shows.
                sh.fill[3] = 0.0;
                sh.border[3] = 0.0;
            } else {
                panels.push(i);
            }
        } else if !tall && left <= lane_x + 0.5 && right >= notes_end - 0.5 {
            // The lines along it: none (the tile is its frame). Cut by the tile's edge, what was
            // left of them flickered as a hairline as the panel followed the pointer.
            sh.fill[3] = 0.0;
            sh.border[3] = 0.0;
        }
    }
    // The drum's panel round at the tile's edge; a square copy over its right half keeps it
    // square against the lane.
    for i in panels.into_iter().rev() {
        let mut square = shapes[i];
        shapes[i].params[0] = round;
        square.half[0] /= 2.0;
        square.center[0] += square.half[0];
        shapes.insert(i + 1, square);
    }
}

/// The biggest size, from a tile's value size down to the body's, that `value` fits `room` at.
fn fitting_size(value: &str, room: f32) -> f32 {
    use crate::style::text;
    [text::HEADING, 10.0, 9.0, text::BODY]
        .into_iter()
        .find(|&s| Canvas::text_width(s, value) <= room)
        .unwrap_or(text::BODY)
}

/// The biggest size from the body's down to the caption's that `value` fits `room` at (the
/// caption's when nothing does: a number is never cut).
fn fit_size(value: &str, room: f32) -> f32 {
    use crate::style::text;
    [text::BODY, 7.0, text::CAPTION]
        .into_iter()
        .find(|&s| Canvas::text_width(s, value) <= room)
        .unwrap_or(text::CAPTION)
}

/// The pp row's button: a block as the way to the board beside it is, lighter under the pointer.
fn pp_chip(c: &mut Canvas, rect: [f32; 4], lit: bool) {
    use crate::style::{block_tones, radius, tone};
    let [x, y, w, h] = rect;
    // Its tone alone at rest (no line round a strip of numbers); the edge under the pointer,
    // as a button's.
    let (fill, edge) = block_tones(lit, false);
    let (edge, line) = if lit {
        (rgba(edge, 1.0), tone::LINE)
    } else {
        (rgba(fill, 1.0), 0.0)
    };
    c.round_rect([x, y], [w, h], radius::CONTROL, rgba(fill, 1.0), edge, line);
}

const BARE_GRAPH_H: f32 = 28.0;
/// The room under the timeline for its "Play from" button, past its times' line.
const PLAY_FROM_ROOM: f32 = 22.0;
/// Bare: the timeline's bars, a neutral ink (the accent is Play's and the selection's), kiai's in
/// gold; how strong they are ahead of the playhead and behind it.
const KIAI_GOLD: u32 = 0xFFD34D;
const BARS_AHEAD: f32 = 0.24;
const BARS_PLAYED: f32 = 0.6;
/// Bare, the density graph's bins: how wide, the steps their heights go in, and how much
/// stronger than their fill their top edge is.
const BIN_W: f32 = 5.0;
const BIN_STEP: f32 = 2.0;
const BIN_CAP: f32 = 0.3;

/// The details in song select's main menu look (`Preview::bare`): how tall the map picture's
/// banner is and where the title starts on it (under the leaderboard button's row), the rows'
/// heights (the mapper's line, the difficulty's chip, the stat tiles, the Don/Kat bar with its
/// counts, the pp chips, what a TJA course leaves out) and the gaps between the blocks.
const BANNER_H: f32 = 96.0;
/// The plate over the banner, under its text: how dark at its left and at its right end, an
/// eased sweep between. Dense under the title and the artist at the left, open towards the top
/// right where the picture shows; the picture's own fade down the banner darkens it downwards,
/// and the text has a soft shadow (`shadowed_text`) to read on a white or yellow picture.
const BANNER_PLATE: (f32, f32) = (0.82, 0.1);
const BANNER_TOP: f32 = 14.0;
const MAPPER_H: f32 = 13.0;
const CHIP_H: f32 = 18.0;
const TILE_H: f32 = 30.0;
/// How far in from a stat tile's top its caption is, and from its bottom its value.
const TILE_PAD: f32 = 5.0;

/// The compact details' (`Split`) one line of stats: its height (or taller where text comes out
/// taller than asked), and the room between a caption and its value.
const STAT_LINE_H: f32 = 14.0;
const STAT_LINE_GAP: f32 = 4.0;

fn stat_line_h() -> f32 {
    (Canvas::text_height(crate::style::text::BODY) + 4.0).max(STAT_LINE_H)
}

/// How tall a stat tile is: `TILE_H`, or taller where text comes out taller than asked
/// (`render::TextSizes`), for its caption and value to keep apart.
fn tile_h() -> f32 {
    use crate::style::text::{CAPTION, HEADING};
    let fits = 2.0 * TILE_PAD + Canvas::text_height(CAPTION) + 1.0 + Canvas::text_height(HEADING);
    fits.max(TILE_H)
}
const BAR_H: f32 = 14.0;
const PP_H: f32 = 16.0;
/// Between the pp button, open, and the play it works out under it.
const SIM_GAP: f32 = 4.0;
const LEFT_OUT_H: f32 = 12.0;
const BLOCK_GAP: f32 = 8.0;
/// Bare, the room between the pp row and the preview under it.
const PREVIEW_GAP: f32 = 10.0;
/// How far down the banner's picture stays whole before fading out.
const BANNER_WHOLE: f32 = 0.45;
/// Split (song select's right column), the banner's height, its look (how far down the picture
/// stays whole; the plate at its left and its right) and the room under its text. Tall enough
/// for the picture to show on 720, the text down at its foot over the picture fading out into
/// the panel, a plate at the left only: its top and right open.
const SPLIT_BANNER_H: f32 = 240.0;
const SPLIT_BANNER: (f32, (f32, f32)) = (0.58, (0.55, 0.0));
const SPLIT_BANNER_FOOT: f32 = 8.0;
/// How bright the banner's picture is drawn: near its own, the plate under the text does the rest.
const BANNER_BRIGHT: f32 = 0.85;

/// The banner text's shadow: how far down it is, and how strong.
const TEXT_SHADOW: (f32, f32) = (1.0, 0.6);

/// Text with a soft shadow under it (a dark copy `TEXT_SHADOW` down, drawn first), to read on
/// a bright picture. Hands back its width, as `Canvas::text`.
fn shadowed_text(c: &mut Canvas, x: f32, y: f32, size: f32, color: [f32; 4], s: &str) -> f32 {
    let (start, texts) = (c.shapes.len(), c.texts.len());
    let w = c.text(x, y, size, color, s);
    let shadow: Vec<crate::render::Shape> = c.shapes[start..]
        .iter()
        .map(|sh| {
            let mut sh = *sh;
            sh.center[1] += TEXT_SHADOW.0;
            for col in [&mut sh.fill, &mut sh.border] {
                *col = [0.0, 0.0, 0.0, col[3] * TEXT_SHADOW.1];
            }
            sh
        })
        .collect();
    let n = shadow.len();
    c.shapes.splice(start..start, shadow);
    // The text itself is after its shadow now.
    for (r, _) in &mut c.texts[texts..] {
        *r = r.start + n..r.end + n;
    }
    w
}

/// The bare details' text, laid out before anything is drawn (the panel's height follows it):
/// the title at the title size on one line, or two lines a step smaller; the artist one step
/// below the title, on one line.
struct Bare {
    title_size: f32,
    title: Vec<String>,
    artist_size: f32,
    artist: String,
    left_out: bool,
    head: f32,
}

impl Bare {
    /// Split, how tall the banner's text is: the title, the artist, the difficulty's row.
    fn banner_text_h(&self) -> f32 {
        self.title.len() as f32 * (Canvas::text_height(self.title_size) + 4.0)
            + Canvas::text_height(self.artist_size)
            + 6.0
            + CHIP_H
    }

    fn new(e: &Entry, iw: f32, left_out: bool, sim: bool, compact: bool) -> Bare {
        use crate::style::text::{BODY, HEADING, TITLE};
        let (title_size, title, artist_size) = if Canvas::text_width(TITLE, e.title()) <= iw {
            (TITLE, vec![e.title().to_string()], HEADING)
        } else if compact {
            // Compact, the title keeps to one line, a step smaller and cut short if need be.
            (HEADING, vec![Canvas::fit(HEADING, iw, e.title())], BODY)
        } else {
            (HEADING, wrap(HEADING, iw, e.title(), 2), BODY)
        };
        let artist = Canvas::fit(artist_size, iw, e.artist());
        if compact {
            return Bare {
                title_size,
                title,
                artist_size,
                artist,
                left_out,
                head: compact_head(),
            };
        }
        let head = BANNER_TOP
            + title.len() as f32 * (Canvas::text_height(title_size) + 4.0)
            + Canvas::text_height(artist_size)
            + 6.0
            + MAPPER_H
            + BLOCK_GAP
            + CHIP_H
            + BLOCK_GAP
            + tile_h()
            + BLOCK_GAP
            + BAR_H
            + BLOCK_GAP
            + PP_H
            + if sim { SIM_GAP + PP_H } else { 0.0 }
            + if left_out { 4.0 + LEFT_OUT_H } else { 0.0 }
            + PREVIEW_GAP;
        Bare {
            title_size,
            title,
            artist_size,
            artist,
            left_out,
            head,
        }
    }
}

/// How tall the compact details (`Split`) are, whatever the map: the banner (the title on one
/// line, the difficulty and its mapper on one row at its foot), the stats on one line, Don and
/// Kat's bar (or the pp simulator's play in its place) and the pp, with no room left under it.
pub fn compact_head() -> f32 {
    SPLIT_BANNER_H
        + COMPACT_TOP
        + stat_line_h()
        + COMPACT_GAP
        + BAR_H
        + COMPACT_GAP
        + PP_H
        + COMPACT_FOOT
}

/// The compact details' rows under the banner: the room over the stats' line, between it, Don
/// and Kat's bar and the pp row (air for the three small lines to read apart, rather than
/// bigger text), and under the pp before the section's end (a little room, no empty band).
const COMPACT_TOP: f32 = 12.0;
const COMPACT_GAP: f32 = 14.0;
const COMPACT_FOOT: f32 = 6.0;

/// The details split into sections of their own (song select's right column in the main menu's
/// look): the details `head` tall at the top, the autoplay's lane in a box of its own lower down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Split {
    pub head: f32,
    /// The section the preview has: its top and height (the lane's box, the density graph and
    /// "Play from" under it). None: the section is the leaderboard's, and no lane is drawn.
    pub area: Option<[f32; 2]>,
}

/// Split, the preview's section laid out (`SplitLane::new`): the lane's box (its top and
/// height), the density graph's height under it, the lane's scale and how much of its reach
/// past the lane's band shows (`fit`, 1: all of it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SplitLane {
    pub lane: [f32; 2],
    pub graph_h: f32,
    pub k: f32,
    pub fit: f32,
}

impl SplitLane {
    /// The section `[top, h]` for a lane whose band is `band_h` tall and whose playfield reaches
    /// `reach_h` (a .tko map's moves; the band's own height on a plain map), drawn at most at
    /// `k_max` (the lane's scale at full size outside the sections). The graph takes its share,
    /// the box the rest: the band at most `SPLIT_LANE_FILL` of it, and a .tko map's whole reach
    /// in it, at no less than `SPLIT_REACH_MIN` of a plain map's scale. The scale is worked out
    /// once, from the box, so nothing is made smaller twice.
    pub fn new([top, h]: [f32; 2], band_h: f32, reach_h: f32, k_max: f32) -> SplitLane {
        let under = 8.0 + TIMELINE_UNDER + PLAY_FROM_ROOM;
        let room = (h - SPLIT_PAD - under).max(24.0 + BARE_GRAPH_H);
        let graph_h = (room * SPLIT_GRAPH_SHARE).clamp(BARE_GRAPH_H, SPLIT_GRAPH_MAX);
        let box_h = (room - graph_h).max(24.0);
        let band_h = band_h.max(1.0);
        let reach_h = reach_h.max(band_h);
        let plain = (box_h * SPLIT_LANE_FILL / band_h).min(k_max.max(0.01));
        let k = (box_h / reach_h).min(plain).max(plain * SPLIT_REACH_MIN);
        let fit = if reach_h > band_h + 0.5 {
            ((box_h / k - band_h) / (reach_h - band_h)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        SplitLane {
            lane: [top + SPLIT_PAD, box_h],
            graph_h,
            k,
            fit,
        }
    }
}

/// Split: the room over the lane in its section, and under the timeline for its times.
const SPLIT_PAD: f32 = 10.0;
const TIMELINE_UNDER: f32 = 12.0;

/// What the bare details show of the map besides its entry.
struct BareShown<'a> {
    stars: Option<f32>,
    status: Option<(&'a str, u32)>,
    liked: Option<bool>,
    /// Once the map is read: its BPM, length and notes.
    details: Option<Details>,
    left_out: Option<&'a str>,
    /// How far the details read with the map have faded in.
    fade: f32,
}

/// The label of the row of what a TJA course leaves out, said plainly as the other details are.
fn left_out_label() -> &'static str {
    tr!("Not supported yet:", "未対応:", "Пока не поддерживается:")
}

/// `s` as up to `lines` lines of text `size` tall within `max_w`, broken at spaces (or mid-word
/// when a word alone is too long); what still doesn't fit ends the last line with "...".
fn wrap(size: f32, max_w: f32, s: &str, lines: usize) -> Vec<String> {
    let max_w = max_w.max(Canvas::char_width(size) * 4.0);
    let mut out: Vec<String> = Vec::new();
    let mut rest = s;
    while !rest.is_empty() && out.len() < lines {
        // At least one character a line, however narrow.
        let fits = Canvas::fitting_prefix(size, max_w, rest)
            .max(rest.chars().next().map_or(0, char::len_utf8));
        if fits == rest.len() {
            out.push(rest.to_string());
            rest = "";
            break;
        }
        // Break after the last space that keeps the line within the width, else at the width.
        let next = fits + rest[fits..].chars().next().map_or(0, char::len_utf8);
        let at = rest[..next].rfind(' ').filter(|&p| p > 0).unwrap_or(fits);
        out.push(rest[..at].trim_end().to_string());
        rest = rest[at..].trim_start_matches(' ');
    }
    if !rest.is_empty()
        && let Some(last) = out.last_mut()
    {
        *last = Canvas::fit(size, max_w, &format!("{last} {rest}"));
    }
    out
}

fn fmt_time(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Difficulty timeline like lazer's song progress graph: strain bars over the map's length,
/// kiai tinted, the preview point marked, and a playhead. Returns the time range it spans.
/// `boxed`: in a box of its own, else its bars bare over what's behind them.
fn draw_graph(
    c: &mut Canvas,
    g: &Graph,
    [x, y, w, h]: [f32; 4],
    now: f64,
    preview_at: f64,
    marker: Option<f64>,
    boxed: bool,
) -> (f64, f64) {
    let (t0, t1) = (g.start.min(0.0).max(g.start - 2000.0), g.end);
    let span = (t1 - t0).max(1.0);
    let x_of = |t: f64| x + (((t - t0) / span).clamp(0.0, 1.0) as f32) * w;
    // A line `lw` wide at `at`: its left edge, kept on the timeline (at its very ends, centred on
    // them, half of it stuck out past the playfield's edge).
    let line = |at: f32, lw: f32| (at - lw / 2.0).clamp(x, x + w - lw);
    if boxed {
        c.round_rect(
            [x, y],
            [w, h],
            5.0,
            rgba(0x0c0c12, 1.0),
            rgba(0x2a2a38, 1.0),
            1.0,
        );
    }
    // Boxed, kiai is a tint behind the bars; bare, it's their colour.
    if boxed {
        for &(s, e) in &g.kiai {
            c.rect(
                x_of(s),
                y + 1.0,
                (x_of(e) - x_of(s)).max(1.0),
                h - 2.0,
                rgba(0xff8a3d, 0.12),
            );
        }
    }
    let span_at = |px: f32| t0 + f64::from((px - x) / w.max(1.0)) * span;
    // Bars cover the strain sections; the axis starts a little earlier for the intro.
    let (bx0, bx1) = (
        x_of(g.peaks_start),
        x_of(g.peaks_start + g.peaks.len() as f64 * g.section),
    );
    // Bare, wider bins side by side with no room between them, their heights in whole steps:
    // a blocky stepped profile rather than a comb of thin bars.
    let n = (((bx1 - bx0) / if boxed { 2.5 } else { BIN_W }) as usize).max(1);
    let bar_w = (bx1 - bx0) / n as f32;
    let snap = crate::render::snap_to_grid;
    let base = snap(y + h - 3.0);
    for (i, v) in g.bins(n).into_iter().enumerate() {
        let bh = if boxed {
            2.0 + v * (h - 8.0)
        } else {
            ((2.0 + v * (h - 8.0)) / BIN_STEP).round().max(1.0) * BIN_STEP
        };
        let mid = bx0 + (i as f32 + 0.5) * bar_w;
        let played = mid <= x_of(now);
        let colour = if boxed {
            rgba(strain_colour(v), if played { 1.0 } else { 0.55 })
        } else {
            let t = span_at(mid);
            let kiai = g.kiai.iter().any(|&(s, e)| t >= s && t < e);
            let ink = match (kiai, played) {
                (true, _) => KIAI_GOLD,
                (false, true) => crate::style::ink::PRIMARY,
                (false, false) => crate::style::ink::SECONDARY,
            };
            rgba(ink, if played { BARS_PLAYED } else { BARS_AHEAD })
        };
        if boxed {
            let (left, right) = (bx0 + i as f32 * bar_w, bx0 + (i as f32 + 1.0) * bar_w - 0.5);
            c.rect(left, y + h - 3.0 - bh, (right - left).max(0.5), bh, colour);
            continue;
        }
        // Bare, each bin's ends on whole pixels, the next one starting where it ends: no gap.
        let (left, right) = (
            snap(bx0 + i as f32 * bar_w),
            snap(bx0 + (i as f32 + 1.0) * bar_w),
        );
        let top = snap(base - bh);
        c.rect(left, top, (right - left).max(0.5), base - top, colour);
        // A crisp top edge, a step stronger than the fill.
        let cap = [
            colour[0],
            colour[1],
            colour[2],
            (colour[3] + BIN_CAP).min(1.0),
        ];
        c.rect(
            left,
            top,
            (right - left).max(0.5),
            crate::style::tone::LINE,
            cap,
        );
    }
    // Bare, the bars stand on a line of the panels' edge.
    if !boxed {
        let edge = rgba(crate::style::tone::EDGE, 1.0);
        c.rect(x, base, w, crate::style::tone::LINE, edge);
    }
    if preview_at >= 0.0 {
        let px = x_of(preview_at);
        c.rect(line(px, 1.0), y + 1.0, 1.0, 5.0, rgba(0x86E05A, 1.0));
    }
    if let Some(m) = marker {
        // The picked start point: a blue line with a little flag.
        // The flag points right, or left where there's no room for it before the end.
        let mx = line(x_of(m), 1.5);
        c.rect(mx, y - 3.0, 1.5, h + 6.0, rgba(MARKER, 1.0));
        let flag = if mx + 1.5 + 6.0 <= x + w {
            mx + 1.5
        } else {
            mx - 6.0
        };
        c.rect(flag, y - 3.0, 6.0, 4.5, rgba(MARKER, 1.0));
    }
    let hx = x_of(now);
    // Bare, the playhead is a plain line of the bars' ink, as tall as they are: no glow round
    // it and no white past their ends, so it reads as part of the timeline beside the marker.
    if boxed {
        c.rect(line(hx, 1.5), y - 2.0, 1.5, h + 4.0, rgba(0xffffff, 0.95));
    } else {
        c.rect(
            line(hx, 1.0),
            y + 1.0,
            1.0,
            h - 4.0,
            rgba(crate::style::ink::PRIMARY, 0.9),
        );
    }
    let times = format!("{} / {}", fmt_time(now - t0.min(0.0)), fmt_time(t1));
    c.text(x, y + h + 5.0, 7.0, rgba(0xc8c8d8, 1.0), &times);
    // The peak at the right, shorter where the timeline is narrow, none where even that would
    // run into the times.
    let room = w - Canvas::text_width(7.0, &times) - 8.0;
    let peak = [
        format!(
            "peak {:.1} notes/s at {}",
            g.peak_nps,
            fmt_time(g.peak_nps_at)
        ),
        format!("peak {:.1} notes/s", g.peak_nps),
        format!("{:.1}/s", g.peak_nps),
    ]
    .into_iter()
    .find(|s| Canvas::text_width(7.0, s) <= room);
    if let Some(peak) = peak {
        c.text_right(x + w, y + h + 5.0, 7.0, rgba(0x8a8aa0, 1.0), &peak);
    }
    (t0, t1)
}

pub const MARKER: u32 = 0x4fb4ff;

/// A difficulty bar's colour for strain `v` (0..1): cool blue for easy parts, through yellow,
/// to red at the peaks.
pub fn strain_colour(v: f32) -> u32 {
    let lerp = |a: u32, b: u32, t: f32| -> u32 {
        let ch = |s: u32| {
            ((a >> s & 0xff) as f32 + ((b >> s & 0xff) as f32 - (a >> s & 0xff) as f32) * t) as u32
        };
        (ch(16) << 16) | (ch(8) << 8) | ch(0)
    };
    if v < 0.5 {
        lerp(0x3d6f8c, 0xFCB806, v * 2.0)
    } else {
        lerp(0xFCB806, 0xEB452C, (v - 0.5) * 2.0)
    }
}

/// "<verb> 1:23" under the timeline once a start point is picked (else a hint, when `show_hint`).
/// `button` is (verb, key hint); None shows no button (only the host picks a match's start).
/// Returns the button's rect.
fn draw_play_button(
    c: &mut Canvas,
    cx: f32,
    y: f32,
    marker: Option<f64>,
    button: Option<(&str, &str)>,
    show_hint: bool,
) -> Option<[f32; 4]> {
    let (Some(t), Some((verb, hint))) = (marker, button) else {
        if !show_hint {
            return None;
        }
        let text = if crate::mobile::active() {
            "tap or drag to scroll"
        } else if button.is_some() {
            "click / drag / wheel to scroll - play from where you stop"
        } else {
            "click / drag / wheel to scroll"
        };
        c.text_centered(cx, y + 4.0, 6.0, rgba(0x6f6f86, 1.0), text);
        return None;
    };
    let label = format!("{verb} {}", fmt_time(t));
    let (lw, hw) = (
        Canvas::text_width(8.0, &label),
        Canvas::text_width(6.0, hint),
    );
    let bw = 14.0 + 10.0 + lw + 10.0 + hw + 10.0;
    let (bx, bh) = (cx - bw / 2.0, 17.0);
    c.round_rect(
        [bx, y],
        [bw, bh],
        5.0,
        rgba(0x173049, 1.0),
        rgba(MARKER, 1.0),
        1.0,
    );
    // A play triangle out of thin slices (the font is plain ASCII).
    for i in 0..7 {
        let f = i as f32;
        c.rect(
            bx + 9.0 + f,
            y + 4.5 + f * 0.5,
            1.0,
            8.0 - f,
            rgba(0xffffff, 1.0),
        );
    }
    c.text(bx + 24.0, y + 5.0, 8.0, rgba(0xffffff, 1.0), &label);
    c.text(
        bx + 24.0 + lw + 10.0,
        y + 6.0,
        6.0,
        rgba(0x9ac8f0, 1.0),
        hint,
    );
    Some([bx, y, bw, bh])
}

/// The lane's band (top, bottom; virtual units): all the autoplay box shows of a plain map.
pub(crate) fn lane_reach() -> [f32; 2] {
    let (top, h) = game::lane_only();
    [top, top + h]
}

/// How far up and down (virtual units, within the 480 tall screen) anything of `map` is drawn
/// over the song: the lane's band, widened by where a .tko map moves, turns, zooms or copies the
/// playfield, flies its notes and puts its pictures and text. Worked out once, when the map is
/// picked, so the box doesn't change size as it plays.
pub(crate) fn tko_reach(map: &Beatmap) -> [f32; 2] {
    use crate::game::{BIG_R, HIT_X, LANE_Y};
    let mut reach = lane_reach();
    if map.gimmick.is_none() || map.notes.is_empty() {
        return reach;
    }
    let mut g = Game::new(
        map.clone(),
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
    let Some(rt) = g.tko_mut() else {
        return reach;
    };
    let calm = crate::tko::calm();
    let full = [0.0, VIRTUAL_HEIGHT];
    let notes = &map.notes;
    let end = notes.iter().map(|n| n.end_time).fold(0.0, f64::max) + 1000.0;
    // The lane's far end swings furthest: as wide as the box ever shows.
    let lane_w = SIXTEEN_NINE;
    let mut t = notes[0].time - 3000.0;
    while t < end && reach != full {
        rt.prepare(t);
        let fx = rt.field_with(t, calm);
        // The playfield before it is moved: the lane, and the notes coming down it.
        let mut local = lane_reach();
        let first = notes.partition_point(|n| n.time < t);
        for (i, n) in notes.iter().enumerate().skip(first) {
            if n.time - t > 2500.0 {
                break;
            }
            let Some(nfx) = rt.note(i, t).filter(|f| !f.hidden && f.opacity > 0.01) else {
                continue;
            };
            // shortcut: how far along the lane a note is, guessed from its time (not its scroll
            // speed); upgrade if notes on paths or swirls are under-measured.
            let s = ((n.time - t) * 0.5) as f32;
            let mut y = LANE_Y + nfx.dy + s * nfx.swirl.to_radians().sin();
            if let Some(p) = rt.note_path(i, t, s, lane_w) {
                y += p[1] * p[2];
            }
            let r = BIG_R * nfx.scale * nfx.stretch_y.abs();
            local = [local[0].min(y - r), local[1].max(y + r)];
        }
        // Where that goes, turned and zoomed about the drum and moved: the playfield, its trail
        // and its copies.
        let pivot = [HIT_X, LANE_Y];
        let mut place = |turn: f32, zoom: f32, sy: f32, dy: f32| {
            let (sin, cos) = turn.to_radians().sin_cos();
            for x in [0.0, lane_w] {
                for y in local {
                    let (px, py) = (x - pivot[0], (y - pivot[1]) * sy);
                    let y = pivot[1] + (px * sin + py * cos) * zoom + dy;
                    reach = [reach[0].min(y), reach[1].max(y)];
                }
            }
        };
        place(fx.rotation, fx.zoom, fx.stretch_y, fx.y);
        for k in 1..=(fx.echo.clamp(0.0, 8.0).ceil() as i32) {
            let k = k as f32;
            place(
                fx.rotation + fx.echo_rot * k,
                fx.zoom * fx.echo_zoom.clamp(0.5, 1.5).powf(k),
                fx.stretch_y,
                fx.y + fx.echo_y * k,
            );
        }
        for c in fx.copies.iter().filter(|c| c.show > 0.01) {
            place(c.turn, c.zoom, 1.0, c.y);
        }
        if fx.slices >= 2.0 {
            reach = [reach[0] - fx.slice_y.abs(), reach[1] + fx.slice_y.abs()];
        }
        // A kaleidoscope or a bent lane goes all over.
        if fx.kaleido >= 1.5 || (fx.lane_shape > 0 && fx.lane_bend > 0.001) {
            reach = full;
        }
        // Pictures and text, placed on the screen.
        for (_, at) in rt.layers(t) {
            let p = at.pose;
            if p.alpha >= 0.01 {
                let half = (p.size * at.scale / 2.0) as f32;
                let y = p.y as f32;
                reach = [reach[0].min(y - half), reach[1].max(y + half)];
            }
        }
        reach = [reach[0].max(0.0), reach[1].min(VIRTUAL_HEIGHT)];
        t += 100.0;
    }
    reach
}

fn new_preview_game(map: &Beatmap, rate: f64) -> Play {
    // No samples: the preview only borrows the music's clock; `rate` maps it to map time.
    let mut g = Game::new(
        map.clone(),
        Arc::new(Track {
            samples: Vec::new(),
            rate,
            stream: None,
        }),
        None,
    )
    .with_settings(&crate::game::settings());
    g.set_mute_hits(true);
    Play::new(g)
}

#[cfg(test)]
mod beat_tests {
    #[test]
    fn a_shown_rating_counts_to_the_new_one_and_keeps_its_value_while_none_comes() {
        let mut shown = None;
        assert_eq!(
            follow(&mut shown, Some(3.0), 16.0),
            Some(3.0),
            "at once from nothing"
        );
        assert_eq!(
            follow(&mut shown, None, 16.0),
            None,
            "nothing shown while it's worked out"
        );
        assert_eq!(shown, Some(3.0), "kept to count from");
        let mut last = 3.0;
        for _ in 0..40 {
            let v = follow(&mut shown, Some(5.5), 16.0).expect("a value");
            assert!(v >= last && v <= 5.5, "counting up: {last} then {v}");
            last = v;
        }
        assert_eq!(last, 5.5, "there in the end");
        let mut pp = Some([100.0, 90.0, 80.0]);
        for _ in 0..40 {
            follow_pp(&mut pp, Some([200.0, 180.0, 160.0]), 16.0);
        }
        assert_eq!(pp, Some([200.0, 180.0, 160.0]));
    }

    use super::*;
    use crate::beatmap::ControlPoint;

    #[test]
    fn details_show_with_the_map_and_the_autoplay_waits_for_its_music() {
        let map = beatmap::calibration_map(500.0, 4, 16);
        let library = [Entry::new(
            PathBuf::from("/nonexistent/a.osu"),
            map.meta.clone(),
        )];
        let mut p = Preview {
            seen: Some(0),
            ..Default::default()
        };
        let draw = |p: &mut Preview| {
            let mut c = Canvas::default();
            c.clear(853.3);
            let autoplay = p
                .draw_panel(
                    &mut c,
                    &library,
                    500.0,
                    40.0,
                    340.0,
                    400.0,
                    1.0,
                    None,
                    Some(("Play from", "Shift+Enter")),
                    None,
                    None,
                )
                .is_some();
            (autoplay, c.shapes.len())
        };
        let (_, empty) = draw(&mut p);
        p.game = Some(new_preview_game(&map, 1.0));
        p.map = Some((0, map.clone(), Details::of(&map), difficulty::analyze(&map)));
        let (autoplay, with_map) = draw(&mut p);
        assert!(!autoplay);
        assert!(with_map > empty, "the details are there before the music");
        assert!(
            p.graph_rect.is_none() && p.play_rect.is_none(),
            "nothing to seek in yet"
        );

        p.ready_for = Some(0);
        assert!(draw(&mut p).0);
        assert!(p.graph_rect.is_some());
    }

    #[test]
    fn a_map_reaching_off_the_lane_grows_the_box_into_the_room_below() {
        let map = beatmap::calibration_map(500.0, 4, 16);
        let library = [Entry::new(
            PathBuf::from("/nonexistent/a.osu"),
            map.meta.clone(),
        )];
        let mut p = Preview {
            seen: Some(0),
            map: Some((0, map.clone(), Details::of(&map), difficulty::analyze(&map))),
            ..Default::default()
        };
        let box_h = |p: &mut Preview, h: f32| {
            let mut c = Canvas::default();
            c.clear(853.3);
            p.draw_panel(
                &mut c, &library, 500.0, 40.0, 340.0, h, 1.0, None, None, None, None,
            );
            p.box_rect.unwrap()[3]
        };
        let plain = box_h(&mut p, 1000.0);
        // Picked long ago: no easing left.
        let lane = lane_reach();
        let at = Instant::now() - Duration::from_secs(5);
        p.reach = (lane, [lane[0] - 40.0, lane[1] + 60.0], Some(at));
        let tall = box_h(&mut p, 1000.0);
        let k = plain / (lane[1] - lane[0]);
        assert!((tall - plain - 100.0 * k).abs() < 0.5, "{plain} -> {tall}");
        // No room under the panel: the lane's height, no more.
        let cramped = box_h(&mut p, 0.0);
        assert!((cramped - plain).abs() < 0.5, "{plain} -> {cramped}");
    }

    #[test]
    fn whatever_is_drawn_over_the_preview_takes_the_pointer() {
        let map = beatmap::calibration_map(500.0, 4, 16);
        let library = [Entry::new(
            PathBuf::from("/nonexistent/a.osu"),
            map.meta.clone(),
        )];
        let mut p = Preview {
            seen: Some(0),
            ready_for: Some(0),
            game: Some(new_preview_game(&map, 1.0)),
            map: Some((0, map.clone(), Details::of(&map), difficulty::analyze(&map))),
            marker: Some((audio_path(&library[0]), 1000.0)),
            ..Default::default()
        };
        let mut c = Canvas::default();
        c.clear(853.3);
        p.draw_panel(
            &mut c,
            &library,
            500.0,
            40.0,
            340.0,
            400.0,
            1.0,
            None,
            Some(("Play from", "Shift+Enter")),
            None,
            None,
        );
        let top = |c: &Canvas, x: f32, y: f32| {
            c.hits
                .iter()
                .rev()
                .find(|h| h.contains(x, y))
                .map(|h| h.hit)
        };
        let [bx, by, bw, bh] = p.box_rect.unwrap();
        let [gx, gy, gw, gh] = p.graph_rect.unwrap();
        let [px, py, pw, ph] = p.play_rect.unwrap();
        let spots = [
            (bx + bw / 2.0, by + bh / 2.0),
            (gx + gw / 2.0, gy + gh / 2.0),
            (px + pw / 2.0, py + ph / 2.0),
        ];
        for (x, y) in spots {
            assert!(matches!(top(&c, x, y), Some(Hit::Preview)));
        }
        // An overlay over everything, like the friends panel.
        c.hit(0.0, 0.0, 853.3, 480.0, Hit::SocialPanel);
        for (x, y) in spots {
            assert!(matches!(top(&c, x, y), Some(Hit::SocialPanel)));
        }
    }

    /// Notes a TJA chart brings in from above or below the lane (#SCROLL with an imaginary
    /// part) are cut to the lane's box, hard: none of them shows over what's above or under it.
    #[test]
    fn notes_from_above_or_below_are_cut_to_the_lane() {
        // Without the interface's animations: the lane all there at once.
        crate::render::set_frame_theme(Some(crate::render::Theme {
            animations: false,
            ..crate::render::Theme::default()
        }));
        let text = format!(
            "TITLE:Sideways\nBPM:120\nOFFSET:0\nCOURSE:Oni\n#START\n#SCROLL 0.5+2i\n{}#END\n",
            "1111,\n".repeat(8)
        );
        let map = taiko_logic::tja::parse(&text)
            .unwrap()
            .courses
            .remove(0)
            .map;
        let library = [Entry::new(
            PathBuf::from("/nonexistent/a.tja"),
            map.meta.clone(),
        )];
        let mut game = new_preview_game(&map, 1.0);
        game.game.set_time(3000.0);
        let mut p = Preview {
            seen: Some(0),
            ready_for: Some(0),
            game: Some(game),
            map: Some((0, map.clone(), Details::of(&map), difficulty::analyze(&map))),
            bare: true,
            ..Default::default()
        };
        let mut c = Canvas::default();
        c.clear(853.3);
        p.draw_panel(
            &mut c, &library, 500.0, 40.0, 340.0, 400.0, 1.0, None, None, None, None,
        );
        let [bx, by, bw, bh] = p.box_rect.expect("the lane");
        let (mut seen, mut outside) = (0, 0);
        for (k, sh) in c.shapes.iter().enumerate() {
            let red = sh.fill[3] > 0.5 && sh.fill[0] > 0.8 && sh.fill[1] < 0.4 && sh.fill[2] < 0.4;
            let round = sh.half[0] > 1.5 && (sh.half[0] - sh.half[1]).abs() < 0.01;
            if !red || !round || sh.center[0] < bx || sh.center[0] > bx + bw {
                continue;
            }
            let drawn = [
                sh.center[0] - sh.half[0],
                sh.center[1] - sh.half[1],
                sh.center[0] + sh.half[0],
                sh.center[1] + sh.half[1],
            ];
            if drawn[1] < by - 0.5 || drawn[3] > by + bh + 0.5 {
                outside += 1;
            }
            // Where it shows: cut by every clip it's under.
            let mut r = drawn;
            for (range, [x, y, w, h]) in &c.clips {
                if range.contains(&k) {
                    r = [r[0].max(*x), r[1].max(*y), r[2].min(x + w), r[3].min(y + h)];
                }
            }
            if r[0] >= r[2] || r[1] >= r[3] {
                continue;
            }
            seen += 1;
            assert!(
                r[1] >= by - 0.5 && r[3] <= by + bh + 0.5,
                "a note shows at {r:?}, outside the lane {by}..{}",
                by + bh
            );
        }
        assert!(seen > 0, "notes in the lane");
        assert!(outside > 0, "notes drawn from outside the lane, to be cut");
    }

    /// Song select's preview section: the scale comes from the section's whole height, once. A
    /// plain map's lane at most its usual size; a .tko map reaching all over the screen fitted
    /// whole into the box, never smaller than half a plain map's (no dots), bigger the taller
    /// the section; the density graph taller than the bare timeline.
    #[test]
    fn split_the_lanes_scale_fits_a_tko_maps_whole_reach_and_never_makes_it_tiny() {
        let (band, k_max) = (100.0, 0.7);
        let plain = SplitLane::new([0.0, 400.0], band, band, k_max);
        assert!(plain.k <= k_max + 1e-6, "{plain:?}");
        assert!(
            plain.k * band <= plain.lane[1] * SPLIT_LANE_FILL + 1e-3,
            "{plain:?}"
        );
        assert_eq!(plain.fit, 1.0);
        assert!(plain.graph_h > BARE_GRAPH_H, "{plain:?}");
        let tko = SplitLane::new([0.0, 400.0], band, 480.0, k_max);
        assert!(
            tko.k * 480.0 <= tko.lane[1] + 0.01,
            "the whole reach in: {tko:?}"
        );
        assert_eq!(tko.fit, 1.0, "{tko:?}");
        assert!(tko.k >= plain.k * SPLIT_REACH_MIN - 1e-6, "{tko:?}");
        assert!(tko.k > 0.5 * plain.k, "not tiny: {tko:?} vs {plain:?}");
        let short = SplitLane::new([0.0, 200.0], band, 480.0, k_max);
        let short_plain = (short.lane[1] * SPLIT_LANE_FILL / band).min(k_max);
        assert!(
            tko.k > short.k,
            "bigger in a taller section: {tko:?} {short:?}"
        );
        assert!(
            (short.k - short_plain * SPLIT_REACH_MIN).abs() < 1e-4 && short.fit < 1.0,
            "at least half a plain map's, the rest cut: {short:?}"
        );
    }

    /// In song select's sections (split), the lane's notes and drum are scaled to the lane's box
    /// with room round them: big ones reach neither its edges nor the hint along its bottom.
    #[test]
    fn split_the_notes_fit_the_lane_with_room_and_clear_the_hint() {
        crate::render::set_frame_theme(Some(crate::render::Theme {
            animations: false,
            ..crate::render::Theme::default()
        }));
        let text = format!(
            "TITLE:Big\nBPM:120\nOFFSET:0\nCOURSE:Oni\n#START\n{}#END\n",
            "3434,\n".repeat(16)
        );
        let map = taiko_logic::tja::parse(&text)
            .unwrap()
            .courses
            .remove(0)
            .map;
        let library = [Entry::new(
            PathBuf::from("/nonexistent/a.tja"),
            map.meta.clone(),
        )];
        let mut game = new_preview_game(&map, 1.0);
        game.game.set_time(3000.0);
        let mut p = Preview {
            seen: Some(0),
            ready_for: Some(0),
            game: Some(game),
            map: Some((0, map.clone(), Details::of(&map), difficulty::analyze(&map))),
            bare: true,
            split: Some(Split {
                head: 120.0,
                area: Some([200.0, 200.0]),
            }),
            ..Default::default()
        };
        let mut c = Canvas {
            audit: true,
            ..Canvas::default()
        };
        c.clear(853.3);
        p.draw_panel(
            &mut c,
            &library,
            500.0,
            40.0,
            340.0,
            400.0,
            1.0,
            None,
            Some(("Play from", "Shift+Enter")),
            None,
            None,
        );
        let [bx, by, _, bh] = p.box_rect.expect("the lane");
        let hint = c
            .texts
            .iter()
            .find(|(_, t)| t.starts_with("click: fullscreen"))
            .map(|(r, _)| {
                c.shapes[r.clone()].iter().fold(
                    [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
                    |[x0, y0, x1, y1], g| {
                        [
                            x0.min(g.center[0] - g.half[0]),
                            y0.min(g.center[1] - g.half[1]),
                            x1.max(g.center[0] + g.half[0]),
                            y1.max(g.center[1] + g.half[1]),
                        ]
                    },
                )
            })
            .expect("the hint");
        let mut seen = 0;
        for sh in &c.shapes {
            let circle = sh.half[0] > 3.0
                && (sh.half[0] - sh.half[1]).abs() < 0.01
                && sh.params[0] >= sh.half[0] - 0.01
                && sh.params[3] < 10.0
                && sh.fill[3] > 0.5;
            let near = sh.center[0] >= bx && (sh.center[1] - (by + bh / 2.0)).abs() < bh;
            if !circle || !near {
                continue;
            }
            seen += 1;
            let r = [
                sh.center[0] - sh.half[0],
                sh.center[1] - sh.half[1],
                sh.center[0] + sh.half[0],
                sh.center[1] + sh.half[1],
            ];
            // An eighth of the lane at least above and under the biggest of them.
            let room = bh * 0.12;
            assert!(
                r[1] >= by + room && r[3] <= by + bh - room,
                "a circle at {}..{}, in the lane {by}..{}",
                r[1],
                r[3],
                by + bh
            );
            let apart = r[2] <= hint[0] || r[0] >= hint[2] || r[3] <= hint[1] || r[1] >= hint[3];
            assert!(apart, "a circle {r:?} over the hint {hint:?}");
        }
        assert!(seen > 3, "notes on the lane: {seen}");
    }

    #[test]
    fn bare_the_timeline_is_as_wide_as_the_lanes_tile() {
        let map = beatmap::calibration_map(500.0, 4, 16);
        let library = [Entry::new(
            PathBuf::from("/nonexistent/a.osu"),
            map.meta.clone(),
        )];
        let mut p = Preview {
            seen: Some(0),
            ready_for: Some(0),
            game: Some(new_preview_game(&map, 1.0)),
            map: Some((0, map.clone(), Details::of(&map), difficulty::analyze(&map))),
            bare: true,
            ..Default::default()
        };
        let mut c = Canvas::default();
        c.clear(853.3);
        p.draw_panel(
            &mut c, &library, 500.0, 40.0, 340.0, 400.0, 1.0, None, None, None, None,
        );
        let [_, by, _, bh] = p.box_rect.expect("the lane");
        let fill = crate::style::surface_fill(crate::style::Surface::Raised);
        let tile = c
            .shapes
            .iter()
            .find(|sh| {
                sh.fill == fill
                    && sh.params[0] == crate::style::radius::SMALL
                    && (sh.center[1] - (by + bh / 2.0)).abs() < 0.5
                    && (sh.half[1] * 2.0 - bh).abs() < 0.5
            })
            .expect("the lane's tile");
        let (left, right) = (tile.center[0] - tile.half[0], tile.center[0] + tile.half[0]);
        let [gx, _, gw, _] = p.graph_rect.expect("the timeline");
        assert!(
            (gx - left).abs() < 0.01 && (gx + gw - right).abs() < 0.01,
            "the timeline {gx}..{} under the tile {left}..{right}",
            gx + gw
        );
    }

    #[test]
    fn bare_the_drum_starts_the_lanes_tile_unless_its_off() {
        let map = beatmap::calibration_map(500.0, 4, 16);
        let library = [Entry::new(
            PathBuf::from("/nonexistent/a.osu"),
            map.meta.clone(),
        )];
        let panel_ink = crate::render::rgba(0x191921, 1.0);
        for hide_drum in [false, true] {
            let mut p = Preview {
                seen: Some(0),
                ready_for: Some(0),
                game: Some(new_preview_game(&map, 1.0)),
                map: Some((0, map.clone(), Details::of(&map), difficulty::analyze(&map))),
                bare: true,
                hide_drum,
                ..Default::default()
            };
            let mut c = Canvas::default();
            c.clear(853.3);
            let (_, clip) = p
                .draw_panel(
                    &mut c, &library, 500.0, 40.0, 340.0, 400.0, 1.0, None, None, None, None,
                )
                .expect("the lane drawn");
            let [_, by, _, bh] = p.box_rect.expect("the lane");
            let fill = crate::style::surface_fill(crate::style::Surface::Raised);
            let tile = c
                .shapes
                .iter()
                .find(|sh| {
                    sh.fill == fill
                        && sh.params[0] == crate::style::radius::SMALL
                        && (sh.center[1] - (by + bh / 2.0)).abs() < 0.5
                        && (sh.half[1] * 2.0 - bh).abs() < 0.5
                })
                .expect("the lane's tile");
            let left = tile.center[0] - tile.half[0];
            // The tile on the details' line either way, the lane cut at its edge.
            assert!((left - 514.0).abs() < 0.01, "the tile from {left}");
            assert!(
                (clip.x * 853.3 - left).abs() < 0.01,
                "cut from {}",
                clip.x * 853.3
            );
            let panels: Vec<&crate::render::Shape> = c
                .shapes
                .iter()
                .filter(|sh| {
                    sh.fill[3] > 0.0
                        && sh.fill[..3] == panel_ink[..3]
                        && (sh.half[1] * 2.0 - bh).abs() < 1.0
                })
                .collect();
            if hide_drum {
                assert!(panels.is_empty(), "no drum panel: {}", panels.len());
            } else {
                // Round at the tile's edge, square against the lane.
                assert!(
                    panels
                        .iter()
                        .any(|sh| (sh.center[0] - sh.half[0] - left).abs() < 0.01
                            && sh.params[0] == crate::style::radius::SMALL),
                    "the drum's panel from the tile's edge, round"
                );
                assert!(
                    panels.iter().any(|sh| sh.params[0] == 0.0),
                    "its square half"
                );
            }
        }
    }

    #[test]
    fn the_timelines_lines_stay_on_it_at_its_very_ends() {
        let g = difficulty::Graph {
            start: 0.0,
            end: 10_000.0,
            peaks: vec![0.5; 10],
            peaks_start: 0.0,
            section: 1000.0,
            peak_nps: 7.0,
            peak_nps_at: 1000.0,
            kiai: vec![(9000.0, 10_000.0)],
        };
        let rect = [100.0, 50.0, 300.0, 32.0];
        for boxed in [false, true] {
            // The playhead, the preview point and the start point at the start, then at the end.
            for (now, at) in [(0.0, 0.0), (10_000.0, 10_000.0)] {
                let mut c = Canvas::default();
                draw_graph(&mut c, &g, rect, now, at, Some(at), boxed);
                for sh in &c.shapes {
                    let (l, r) = (sh.center[0] - sh.half[0], sh.center[0] + sh.half[0]);
                    assert!(
                        l >= rect[0] - 1e-3 && r <= rect[0] + rect[2] + 1e-3,
                        "boxed {boxed}, at {at}: {l}..{r} out of {}..{}",
                        rect[0],
                        rect[0] + rect[2]
                    );
                }
            }
        }
    }

    #[test]
    fn another_map_picked_drops_the_start_point_even_on_the_same_song() {
        let map = beatmap::calibration_map(500.0, 4, 16);
        // Two difficulties of a set: one song.
        let library = [
            Entry::new(PathBuf::from("/nonexistent/a.osu"), map.meta.clone()),
            Entry::new(PathBuf::from("/nonexistent/b.osu"), map.meta.clone()),
        ];
        assert_eq!(audio_path(&library[0]), audio_path(&library[1]));
        let mut p = Preview {
            seen: Some(0),
            marker: Some((audio_path(&library[0]), 1000.0)),
            ..Default::default()
        };
        let now = Instant::now();
        let update = |p: &mut Preview, selected| {
            p.update(
                now,
                &library,
                selected,
                None,
                false,
                true,
                0.0,
                (1.0, false),
            );
        };
        update(&mut p, Some(0));
        assert_eq!(p.marker(&library[0]), Some(1000.0), "kept on its own map");
        update(&mut p, Some(1));
        assert_eq!(p.marker(&library[1]), None, "not on the other difficulty");
        update(&mut p, Some(0));
        assert_eq!(p.marker(&library[0]), None, "nor back on the first");
    }

    #[test]
    fn a_new_speed_doesnt_change_how_long_the_song_is() {
        let known = (PathBuf::from("a.mp3"), 214_000.0);
        assert_eq!(
            song_length(Some(214_500.0), Some(&known), Path::new("a.mp3"), 180_000.0),
            214_500.0
        );
        assert_eq!(
            song_length(None, Some(&known), Path::new("a.mp3"), 180_000.0),
            214_000.0,
            "its quick part stands in"
        );
        assert_eq!(
            song_length(None, Some(&known), Path::new("b.mp3"), 180_000.0),
            180_000.0,
            "a song not heard whole yet"
        );
        assert_eq!(
            song_length(None, None, Path::new("a.mp3"), 180_000.0),
            180_000.0
        );
    }

    #[test]
    fn green_lines_dont_move_the_beat() {
        let line = |time: f64, sv: f64| ControlPoint {
            time,
            beat_origin: 100.0,
            beat_length: 400.0,
            sv,
            kiai: false,
            volume: 1.0,
        };
        let points = [line(100.0, 1.0), line(1234.0, 1.5), line(2345.0, 0.75)];
        for t in [1200.0, 1300.0, 2400.0] {
            assert_eq!(beat_at(&points, t), Some((t - 100.0, 400.0)));
        }
    }
}

#[cfg(test)]
mod lane_fit_tests {
    use super::*;

    #[test]
    fn four_three_by_default_widening_to_sixteen_nine() {
        // At or under the default width: a 4:3 lane, scaled to the panel.
        assert_eq!(lane_fit(380.0, 380.0).1, FOUR_THREE);
        assert_eq!(lane_fit(300.0, 380.0).1, FOUR_THREE);
        // Wider: the same height, more lane.
        let (k, lane) = lane_fit(470.0, 380.0);
        assert!(
            (k - lane_fit(380.0, 380.0).0).abs() < 1e-5 && lane > FOUR_THREE && lane < SIXTEEN_NINE
        );
        // Much wider: 16:9 at most, scaled up past that.
        let (k, lane) = lane_fit(900.0, 380.0);
        assert_eq!(lane, SIXTEEN_NINE);
        assert!(k > lane_fit(380.0, 380.0).0);
    }

    #[test]
    fn a_song_just_put_on_cant_be_over_by_the_last_ones_position() {
        let now = Instant::now();
        let t = |ms: u64| now - Duration::from_millis(ms);
        assert!(
            !played_long_enough(Some(t(900)), now, 180_000.0),
            "under 3 s in"
        );
        assert!(played_long_enough(Some(t(3100)), now, 180_000.0));
        assert!(
            played_long_enough(None, now, 180_000.0),
            "nothing put on: no wait"
        );
        assert!(
            !played_long_enough(Some(t(900)), now, 4000.0),
            "half of a short one"
        );
        assert!(played_long_enough(Some(t(2100)), now, 4000.0));
    }
}

#[cfg(test)]
mod background_tests {
    use super::*;
    use crate::beatmap::Meta;

    /// The background asked for last, once it's read.
    fn shown(p: &mut Preview) -> (PathBuf, Option<Image>) {
        for _ in 0..500 {
            if let Some(read) = p.bg_rx.poll() {
                return read;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("no background read");
    }

    fn entry(path: &Path, title: &str, background: Option<&str>) -> Entry {
        let meta = Meta {
            title: title.into(),
            background: background.map(Into::into),
            ..Default::default()
        };
        Entry::new(path.to_path_buf(), meta)
    }

    #[test]
    fn a_tja_chart_without_a_picture_gets_one_drawn_and_an_osu_map_none() {
        let dir = std::env::temp_dir().join(format!("taiko-backdrop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        image::RgbaImage::from_pixel(4, 2, image::Rgba([200, 10, 10, 255]))
            .save(dir.join("bg.png"))
            .unwrap();
        let chart = dir.join("song.tja#oni");
        let mut p = Preview::default();

        // Its own picture, as the chart names it.
        p.show_background_for(&entry(&chart, "Song", Some("bg.png")));
        let (path, img) = shown(&mut p);
        assert_eq!(path, dir.join("bg.png"));
        assert_eq!(img.map(|i| (i.width, i.height)), Some((4, 2)));

        // None, or one that isn't there: one drawn.
        for named in [None, Some("missing.png")] {
            let e = entry(&chart, "Song", named);
            p.show_background_for(&e);
            let (_, img) = shown(&mut p);
            let img = img.expect("drawn");
            assert_eq!(
                (img.width, img.height),
                (crate::backdrop::WIDTH, crate::backdrop::HEIGHT)
            );
        }

        // An osu! map without one stays without.
        p.show_background_for(&entry(&dir.join("a.osu"), "Song", None));
        assert!(p.bg_path.is_none() && !p.has_background);
        assert!(matches!(p.bg_upload, Some(None)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod sim_tests {
    use super::PpSim;

    #[test]
    fn a_simulated_play_comes_as_near_its_accuracy_as_whole_hits_can() {
        let sim = PpSim {
            accuracy: 0.98,
            misses: 2,
        };
        let (great, ok, miss) = sim.counts(1000);
        assert_eq!(great + ok + miss, 1000);
        assert_eq!(miss, 2);
        let acc = (f64::from(great) + f64::from(ok) / 2.0) / 1000.0;
        assert!((acc - 0.98).abs() <= 0.0005, "{acc}");
        // More accuracy than the misses leave room for: all the rest greats.
        let sim = PpSim {
            accuracy: 1.0,
            misses: 10,
        };
        assert_eq!(sim.counts(100), (90, 0, 10));
        // More misses than notes.
        assert_eq!(
            PpSim {
                accuracy: 0.9,
                misses: 500
            }
            .counts(100),
            (0, 0, 100)
        );
    }

    #[test]
    fn its_steps_stay_in_reach() {
        let mut sim = PpSim {
            accuracy: 0.998,
            misses: 0,
        };
        sim.step(0, 1);
        assert_eq!(sim.accuracy, 1.0);
        sim.step(1, -1);
        assert_eq!(sim.misses, 0);
        sim.step(1, 1);
        sim.step(0, -1);
        assert_eq!(sim.misses, 1);
        assert!((sim.accuracy - 0.995).abs() < 1e-9);
    }
}
