//! Journey v1 session state, temporary on-demand map cache, and local fallback picker.
//!
//! Maps the player does not own are downloaded strictly on demand (at most 2 ahead in the queue)
//! into `config::journey_cache_dir()`. Clicking Save (or liking with `journey_save_on_like`) moves
//! the cached folder into taiko.rs's own Songs folder by default (or osu!'s Songs folder when set
//! in Settings > Library).

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::config::{self, JourneyFocus, JourneyLength, JourneySaveTo};
use crate::download;
use crate::library::Entry;
use crate::online::{self, JourneyBatch, JourneyItem, JourneyProfile};

/// Maximum number of upcoming maps allowed to download ahead at once (on-demand only).
pub const MAX_PREFETCH_AHEAD: usize = 2;
/// Maximum cached mapset folders kept in `JourneyCache` before oldest are pruned.
pub const MAX_CACHE_FOLDERS: usize = 20;
/// Backoff after a failed online `JourneyNext` request before trying again (doubled after each
/// failure in a row, up to `MAX_FETCH_BACKOFF`).
const FETCH_ERROR_BACKOFF: Duration = Duration::from_secs(2);
const MAX_FETCH_BACKOFF: Duration = Duration::from_secs(30);
/// How long online fetches may keep failing before a Journey with nothing left to show says the
/// server is down: it often drops for a few seconds, and a retry within that goes on unnoticed.
pub const OUTAGE_GRACE: Duration = Duration::from_secs(8);
/// How often a Journey gone offline for a server outage asks the server again.
const OFFLINE_PROBE: Duration = Duration::from_secs(30);
/// Maps already shown that a session keeps to go back to.
const BACK_LIMIT: usize = 16;
/// How long an exhausted pool waits before asking again (new maps may have been fingerprinted).
const EXHAUSTED_RETRY: Duration = Duration::from_secs(60);
/// Most maps (and sets) a session remembers to not suggest again; the oldest go first.
const MAX_EXCLUDES: usize = 400;

/// How far below the player's level a chill session picks its library maps (the server's own
/// `CHILL_DROP` for online ones).
const CHILL_DROP: f64 = 0.5;

/// Cold-start star rating choices shown when a player has too little history.
pub const LEVEL_CHOICES: [(&str, f64); 6] = [
    ("2 - 3*", 2.5),
    ("3 - 4*", 3.5),
    ("4 - 5*", 4.5),
    ("5 - 6*", 5.5),
    ("6 - 7*", 6.5),
    ("7* +", 7.5),
];

/// Outcome of a previously shown map in the current Journey session (drawn on the top mini-lane).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PastOutcome {
    Passed,
    Failed,
    Skipped,
}

/// What Journey asks the player before it goes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Not logged in: log in, make an account, or go on offline with the library.
    SignedOut,
    /// The server hasn't answered for [`OUTAGE_GRACE`] and nothing is left to play: wait for
    /// it, or go on offline.
    ServerDown,
}

/// The map Journey just moved on from.
pub struct Left {
    pub item: JourneyItem,
    /// It had been left once already and was gone back to: the player's verdict on it was
    /// already sent.
    pub revisited: bool,
}

#[derive(Clone, Debug)]
pub struct PastMap {
    pub title: String,
    pub role: String,
    pub outcome: PastOutcome,
}

/// Background task that checks the cache, prunes old cache folders, downloads the set if needed,
/// and parses the target difficulty — keeping all disk I/O off the render/gameplay thread.
pub struct DownloadTask {
    pub done: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
    pub cancel: Arc<AtomicBool>,
    pub rx: Receiver<Result<(PathBuf, Entry), String>>,
}

impl DownloadTask {
    pub fn fraction(&self) -> Option<f32> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| self.done.load(Ordering::Relaxed) as f32 / total as f32)
    }
}

/// Where a queued Journey map currently lives on disk.
/// Uses stable `PathBuf`s rather than library indices so `refresh_library` never shifts targets.
pub enum MapSource {
    /// Already in `State::library` at this `.osu`/`.tko` path.
    Local(PathBuf),
    /// Queued beyond the first `MAX_PREFETCH_AHEAD` slots; download starts when it shifts up.
    Pending,
    /// Currently checking cache / downloading / unpacking into `JourneyCache` on a worker thread.
    Downloading(Box<DownloadTask>),
    /// Unpacked in `JourneyCache/<folder>`, ready to play and save.
    Cached { folder: PathBuf, entry: Box<Entry> },
    /// Moved from cache into the player's permanent library at this `.osu`/`.tko` path.
    Saved(PathBuf),
    /// Download failed with a human-readable message.
    Failed(String),
}

impl MapSource {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Local(_) | Self::Cached { .. } | Self::Saved(_))
    }

    pub fn is_in_library(&self) -> bool {
        matches!(self, Self::Local(_) | Self::Saved(_))
    }

    pub fn download_progress(&self) -> Option<f32> {
        if let Self::Downloading(job) = self {
            Some(job.fraction().unwrap_or(0.0))
        } else {
            None
        }
    }
}

pub struct QueuedMap {
    pub item: JourneyItem,
    pub source: MapSource,
}

/// Active Journey session state held on `State`.
pub struct Journey {
    pub active: bool,
    pub needs_level: bool,
    pub level_cursor: usize,
    pub comfort_stars: f64,
    /// Last server-estimated comfort stars (when running without a manual target override).
    pub auto_comfort_stars: Option<f64>,
    pub comfort_scroll: Option<f64>,
    pub player_profile: Option<JourneyProfile>,
    pub target_stars: Option<f64>,
    pub step: u32,
    pub queue: VecDeque<QueuedMap>,
    pub history: Vec<PastMap>,
    pub exclude_md5s: Vec<String>,
    pub exclude_sets: Vec<String>,
    pub fetching: Option<Receiver<Result<JourneyBatch, String>>>,
    /// Set when the server or local library returned fewer maps than requested (stops per-frame refetching).
    pub pool_exhausted: bool,
    /// Earliest instant to retry after an online fetch error.
    pub retry_after: Option<Instant>,
    /// Online fetches failed in a row (drives the backoff).
    pub fetch_failures: u32,
    pub error: Option<String>,
    pub shown_at: Instant,
    /// When the Journey screen itself was opened (drives screen entrance animation).
    pub screen_entered: Instant,
    /// Smoothly decaying horizontal offset (virtual units) for the top session lane on advance.
    pub lane_slide: f32,
    /// Set to true when the current play was launched from Journey (tags `PlayedAttempt` and `ScoreSubmit`).
    pub in_play: bool,
    /// Temporary cached entry currently being played (kept so results screen can Save it).
    pub playing_cached: Option<(PathBuf, JourneyItem)>,
    /// True when the active session is running against local library fallback (offline / old server).
    pub local_mode: bool,
    /// The cached (not in the library) map the preview last played: another one is a new song
    /// even though it's always entry 0 of its one-entry list.
    pub preview_cached: Option<PathBuf>,
    /// The current map's library index, cached against `(path, library ptr, library len)`.
    library_index: Option<(PathBuf, usize, usize, Option<usize>)>,
    /// What Journey asks before it goes on, drawn instead of the map.
    pub gate: Option<Gate>,
    /// Why the player chose to go on offline, with the library only.
    pub offline: Option<Gate>,
    /// When the current run of failed online fetches began (none while the server answers).
    pub outage_since: Option<Instant>,
    /// When a Journey offline for an outage asks the server again.
    pub next_probe: Option<Instant>,
    /// Maps already shown, the last one last, each with what it ended with.
    back: Vec<(QueuedMap, PastOutcome)>,
    /// What the first maps of the queue ended with when they were left before (they were gone
    /// back to), in queue order.
    revisited: Vec<PastOutcome>,
    /// The kind of maps asked for (`Config::journey_focus`).
    pub focus: JourneyFocus,
    /// Map duration filter (`Config::journey_length`).
    pub length: JourneyLength,
    /// Whether Marathon's default 5-minute length cap applies when `length == JourneyLength::Any`.
    pub marathon_cap: bool,
    /// An easy session (`Config::journey_chill`).
    pub chill: bool,
    /// The settings panel is up, with this row chosen.
    pub settings: Option<usize>,
    /// Non-stop: the current map starts by itself as soon as it's ready (set when a play's
    /// results go on by themselves; any key on the Journey screen takes over).
    pub autostart: bool,
    /// The map about to load starts just before its first note (it was started by itself).
    pub skip_intro_next: bool,
    /// A marathon on: maps one after another with no results, until the player stops.
    pub marathon: Option<Marathon>,
    /// Smoothly animated 0..=1 visibility for the in-play "Up next" card (prevents pop-in and
    /// sudden combo chip jumps).
    pub up_next_alpha: f32,
}

/// What became of a map in a marathon.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MarathonOutcome {
    Passed,
    Failed,
    /// Left from the pause menu partway through.
    Skipped,
}

/// A map just played in a marathon: its card on the wheel, and what its keys work on (like,
/// save).
#[derive(Clone, Debug)]
pub struct MarathonPlay {
    pub item: JourneyItem,
    pub outcome: MarathonOutcome,
    /// 0 to 1.
    pub accuracy: f64,
    pub max_combo: u32,
    pub misses: u32,
    pub pp: Option<f64>,
    /// The map to like or dislike (logged in, an osu! map), and what was said of it.
    pub vote_on: Option<online::PlayedMap>,
    pub vote: Option<online::MapVote>,
    /// Its folder in Journey's cache with its item, to save it to the library; None once saved
    /// (or for a library map).
    pub cached: Option<(PathBuf, JourneyItem)>,
    /// True once moved from Journey's cache into the permanent library (auto-saved or via S/Like).
    pub saved: bool,
}

/// Why a marathon ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MarathonEnd {
    /// The player stopped it.
    Quit,
    /// Hardcore, and a map was failed.
    Hardcore,
    /// Nothing more to play.
    OutOfMaps,
}

/// The map up next in a marathon, loading during the one played now so that it can start the
/// moment that one ends.
pub(crate) struct Preload {
    /// Its chart file (a cached map isn't in the library yet: it joins when it's up).
    pub path: std::path::PathBuf,
    /// The mods and speed it was loaded for.
    pub mods: crate::mods::Mods,
    pub rate: f64,
    pub keep_pitch: bool,
    pub job: crate::Job<crate::Loaded>,
}

impl std::fmt::Debug for Preload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preload")
            .field("path", &self.path)
            .field("busy", &self.job.is_busy())
            .finish()
    }
}

/// A marathon: Journey's maps one after another with no results in between, and what it has come
/// to so far.
#[derive(Debug)]
pub struct Marathon {
    pub started: Instant,
    pub ended: Option<(MarathonEnd, Instant)>,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    accuracy_sum: f64,
    /// pp of the passed maps, summed.
    pub pp: f64,
    /// The play worth the most pp: its map's name and its pp.
    pub best: Option<(String, f64)>,
    pub streak: u32,
    pub best_streak: u32,
    pub last: Option<MarathonPlay>,
    /// The combo carried over from the maps before the one played now (0 once broken).
    pub combo: u32,
    /// The marathon combo as it stands now: `combo` and the map's own, or only the map's own
    /// once the map broke it.
    pub combo_now: u32,
    pub best_combo: u32,
    /// The map played now broke the carried combo (a miss, its own combo went back down, or it
    /// isn't the player's own play).
    map_broken: bool,
    /// The map's own combo, last seen.
    map_combo: u32,
    /// The map up next, loading ahead of time.
    pub(crate) preload: Option<Preload>,
    /// Outcome history of maps in this marathon (for the summary run strip).
    pub outcomes: Vec<MarathonOutcome>,
}

impl Marathon {
    pub fn new(now: Instant) -> Marathon {
        Marathon {
            started: now,
            ended: None,
            passed: 0,
            failed: 0,
            skipped: 0,
            accuracy_sum: 0.0,
            pp: 0.0,
            best: None,
            streak: 0,
            best_streak: 0,
            last: None,
            combo: 0,
            combo_now: 0,
            best_combo: 0,
            map_broken: false,
            map_combo: 0,
            preload: None,
            outcomes: Vec::new(),
        }
    }

    /// A map of the marathon starts (again, on a retry): the combo carried so far goes on into
    /// it.
    pub fn map_started(&mut self) {
        self.map_broken = false;
        self.map_combo = 0;
        self.combo_now = self.combo;
    }

    /// The map played now stands at its own `combo` with `misses`; `counts`: the player's own
    /// play (no Autoplay, no unranked mods), else it carries nothing on. The marathon combo
    /// before and after.
    pub fn track_combo(&mut self, combo: u32, misses: u32, counts: bool) -> (u32, u32) {
        let before = self.combo_now;
        if misses > 0 || combo < self.map_combo || !counts {
            self.map_broken = true;
        }
        self.map_combo = combo;
        self.combo_now = match (counts, self.map_broken) {
            (false, _) => 0,
            (true, true) => combo,
            (true, false) => self.combo + combo,
        };
        self.best_combo = self.best_combo.max(self.combo_now);
        (before, self.combo_now)
    }

    /// The map played now is over (played out or failed): its marathon combo carries on.
    pub fn map_ended(&mut self) {
        self.combo = self.combo_now;
    }

    /// The map played now was left partway (skipped): the combo breaks.
    pub fn combo_broken(&mut self) {
        self.combo = 0;
        self.combo_now = 0;
        self.map_broken = true;
    }

    pub fn record(&mut self, play: MarathonPlay) {
        match play.outcome {
            MarathonOutcome::Passed => {
                self.passed += 1;
                self.accuracy_sum += play.accuracy;
                self.streak += 1;
                self.best_streak = self.best_streak.max(self.streak);
                if let Some(pp) = play.pp {
                    self.pp += pp;
                    if self.best.as_ref().is_none_or(|(_, best)| pp > *best) {
                        let name = format!(
                            "{} - {} [{}]",
                            play.item.artist, play.item.title, play.item.version
                        );
                        self.best = Some((name, pp));
                    }
                }
            }
            MarathonOutcome::Failed => {
                self.failed += 1;
                self.streak = 0;
            }
            MarathonOutcome::Skipped => self.skipped += 1,
        }
        self.outcomes.push(play.outcome);
        self.last = Some(play);
    }

    /// Maps played to their end, passed or failed.
    pub fn played(&self) -> u32 {
        self.passed + self.failed
    }

    /// The passed maps' accuracy on average (0 to 1).
    pub fn average_accuracy(&self) -> Option<f64> {
        (self.passed > 0).then(|| self.accuracy_sum / self.passed as f64)
    }

    /// How long it has gone on (or went on).
    pub fn duration(&self, now: Instant) -> Duration {
        self.ended
            .map_or(now, |(_, at)| at)
            .saturating_duration_since(self.started)
    }

    /// Ends it, for `why`, unless it ended already.
    pub fn end(&mut self, why: MarathonEnd, now: Instant) {
        self.ended.get_or_insert((why, now));
    }
}

impl Default for Journey {
    fn default() -> Self {
        Self::new()
    }
}

impl Journey {
    pub fn new() -> Self {
        Self {
            active: false,
            needs_level: false,
            level_cursor: 2,
            comfort_stars: 4.5,
            auto_comfort_stars: None,
            comfort_scroll: None,
            player_profile: None,
            target_stars: None,
            step: 0,
            queue: VecDeque::new(),
            history: Vec::new(),
            exclude_md5s: Vec::new(),
            exclude_sets: Vec::new(),
            fetching: None,
            pool_exhausted: false,
            retry_after: None,
            fetch_failures: 0,
            error: None,
            shown_at: Instant::now(),
            screen_entered: Instant::now(),
            lane_slide: 0.0,
            in_play: false,
            playing_cached: None,
            local_mode: false,
            preview_cached: None,
            library_index: None,
            gate: None,
            offline: None,
            outage_since: None,
            next_probe: None,
            back: Vec::new(),
            revisited: Vec::new(),
            focus: JourneyFocus::Any,
            length: JourneyLength::Any,
            marathon_cap: false,
            chill: false,
            settings: None,
            autostart: false,
            skip_intro_next: false,
            marathon: None,
            up_next_alpha: 0.0,
        }
    }

    /// Where the current map is in `library`, when it's a library map (none while it's cached,
    /// downloading or failed).
    pub fn current_library_index(&mut self, library: &[Entry]) -> Option<usize> {
        let path = match self.queue.front().map(|q| &q.source) {
            Some(MapSource::Local(p) | MapSource::Saved(p)) => p,
            _ => return None,
        };
        let ptr = library.as_ptr() as usize;
        let len = library.len();
        if let Some((known, known_ptr, known_len, cached)) = &self.library_index
            && known.as_os_str() == path.as_os_str()
        {
            match *cached {
                Some(i)
                    if library
                        .get(i)
                        .is_some_and(|e| e.path.as_os_str() == path.as_os_str()) =>
                {
                    return Some(i);
                }
                None if *known_ptr == ptr && *known_len == len => return None,
                _ => {}
            }
        }
        let i = library
            .iter()
            .position(|e| e.path.as_os_str() == path.as_os_str())
            .or_else(|| library.iter().position(|e| &e.path == path));
        self.library_index = Some((path.clone(), ptr, len, i));
        i
    }

    /// Clears the cached library index after `State::library` is mutated or replaced.
    pub fn invalidate_library_index(&mut self) {
        self.library_index = None;
    }

    /// Resets session state for a fresh Journey run.
    pub fn reset(&mut self) {
        self.cancel_downloads();
        *self = Self::new();
        self.active = true;
    }

    /// Ends the Journey session and cancels any in-flight downloads.
    pub fn stop(&mut self) {
        self.cancel_downloads();
        self.active = false;
        self.marathon = None;
        self.in_play = false;
        self.fetching = None;
        self.playing_cached = None;
        self.skip_intro_next = false;
        self.pool_exhausted = false;
        self.retry_after = None;
    }

    fn cancel_downloads(&mut self) {
        for q in &self.queue {
            if let MapSource::Downloading(job) = &q.source {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Stops any background prefetch downloads/extractions before gameplay starts so weak CPUs
    /// never stutter during play; they resume back on the Journey screen, from the cache when
    /// one had already finished unpacking (`poll_downloads`).
    pub fn pause_background_downloads(&mut self) {
        for q in &mut self.queue {
            if let MapSource::Downloading(job) = &q.source {
                job.cancel.store(true, Ordering::Relaxed);
                q.source = MapSource::Pending;
            }
        }
    }

    /// Waits longer after each online fetch failure in a row.
    fn backoff(&mut self) {
        self.fetch_failures = self.fetch_failures.saturating_add(1);
        let wait = FETCH_ERROR_BACKOFF
            .saturating_mul(1 << self.fetch_failures.saturating_sub(1).min(5))
            .min(MAX_FETCH_BACKOFF);
        self.retry_after = Some(Instant::now() + wait);
    }

    /// `folder` was saved to `dest`: every queued map in it is now in the library there, and
    /// the play of it is no longer one to save.
    pub fn saved(&mut self, folder: &Path, dest: &Path) {
        for q in self
            .queue
            .iter_mut()
            .chain(self.back.iter_mut().map(|(q, _)| q))
        {
            if let MapSource::Cached { folder: f, entry } = &q.source
                && f == folder
            {
                let moved = moved_path(&entry.path, folder, dest);
                q.source = MapSource::Saved(moved);
            }
        }
        if self
            .playing_cached
            .as_ref()
            .is_some_and(|(f, _)| f == folder)
        {
            self.playing_cached = None;
        }
        self.library_index = None;
    }

    /// Drops the queue (and what's downloading for it) to be filled again for new wishes.
    fn rebuild(&mut self) {
        self.cancel_downloads();
        self.queue.clear();
        self.revisited.clear();
        self.fetching = None;
        self.pool_exhausted = false;
        self.retry_after = None;
        self.error = None;
        self.shown_at = Instant::now();
    }

    /// Switches the session to a manual target star rating and rebuilds the queue for it.
    pub fn set_manual_level(&mut self, stars: f64) {
        let clamped = ((stars * 4.0).round() / 4.0).clamp(1.0, 11.0);
        self.rebuild();
        self.target_stars = Some(clamped);
        self.comfort_stars = clamped;
        self.needs_level = false;
    }

    /// Switches the session back to server-estimated comfort stars and rebuilds the queue.
    pub fn set_auto_level(&mut self) {
        self.rebuild();
        self.target_stars = None;
        if let Some(auto) = self.auto_comfort_stars {
            self.comfort_stars = auto;
        }
        self.needs_level = false;
    }

    /// Effective inclusive `(min_length_ms, max_length_ms)` bounds for this session.
    pub fn length_bounds(&self) -> (Option<u64>, Option<u64>) {
        self.length.bounds(self.marathon_cap)
    }

    /// Asks for maps of another kind (`focus`), duration (`length`/`marathon_cap`), or pace
    /// (`chill`): the queue is filled again when the effective filter changed. True when it was.
    pub fn set_style(
        &mut self,
        focus: JourneyFocus,
        chill: bool,
        length: JourneyLength,
        marathon_cap: bool,
    ) -> bool {
        let old_bounds = self.length_bounds();
        let new_bounds = length.bounds(marathon_cap);
        if (focus, chill, old_bounds) == (self.focus, self.chill, new_bounds) {
            self.length = length;
            self.marathon_cap = marathon_cap;
            return false;
        }
        self.focus = focus;
        self.chill = chill;
        self.length = length;
        self.marathon_cap = marathon_cap;
        self.rebuild();
        true
    }

    /// Non-stop: says no to the map after the current one (the one played now) before it comes
    /// up. Returns it, to tell the server it was skipped.
    pub fn skip_upcoming(&mut self) -> Option<JourneyItem> {
        if self.queue.len() < 2 {
            return None;
        }
        let skipped = self.queue.remove(1)?;
        if self.revisited.len() > 1 {
            self.revisited.remove(1);
        }
        if let MapSource::Downloading(job) = &skipped.source {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.pool_exhausted = false;
        Some(skipped.item)
    }

    /// Folders referenced by the queue, the maps to go back to or the active play (protected
    /// from `prune_cache`).
    pub fn protected_cache_folders(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for q in self.queue.iter().chain(self.back.iter().map(|(q, _)| q)) {
            if let MapSource::Cached { folder, .. } = &q.source {
                out.push(folder.clone());
            }
        }
        if let Some((folder, _)) = &self.playing_cached {
            out.push(folder.clone());
        }
        out
    }

    pub fn current(&self) -> Option<&QueuedMap> {
        self.queue.front()
    }

    pub fn current_mut(&mut self) -> Option<&mut QueuedMap> {
        self.queue.front_mut()
    }

    /// Records the current map into session exclusions so it isn't recommended again.
    pub fn remember_item(&mut self, item: &JourneyItem) {
        if !item.md5.is_empty() && !self.exclude_md5s.iter().any(|m| m == &item.md5) {
            self.exclude_md5s.push(item.md5.clone());
        }
        let set_key = set_key_of(item);
        if !self.exclude_sets.iter().any(|s| s == &set_key) {
            self.exclude_sets.push(set_key);
        }
        for list in [&mut self.exclude_md5s, &mut self.exclude_sets] {
            let over = list.len().saturating_sub(MAX_EXCLUDES);
            list.drain(..over);
        }
    }

    /// Advances past the front map, records its outcome on the top mini-lane and keeps it to go
    /// back to. Moving on past a map gone back to without playing it keeps what it ended with
    /// before; playing it again counts.
    pub fn advance(&mut self, outcome: PastOutcome) -> Option<Left> {
        let mut popped = self.queue.pop_front()?;
        let before = (!self.revisited.is_empty()).then(|| self.revisited.remove(0));
        let outcome = match (before.clone(), outcome) {
            (Some(before), PastOutcome::Skipped) => before,
            (_, outcome) => outcome,
        };
        if let MapSource::Downloading(job) = &popped.source {
            job.cancel.store(true, Ordering::Relaxed);
            popped.source = MapSource::Pending;
        }
        self.playing_cached = None;
        self.history.push(PastMap {
            title: popped.item.title.clone(),
            role: popped.item.role.clone(),
            outcome: outcome.clone(),
        });
        if self.history.len() > 16 {
            self.history.remove(0);
        }
        self.step = self.step.saturating_add(1);
        self.shown_at = Instant::now();
        self.lane_slide = 28.0;
        let left = Left {
            item: popped.item.clone(),
            revisited: before.is_some(),
        };
        if !matches!(popped.source, MapSource::Failed(_)) {
            self.back.push((popped, outcome));
            let over = self.back.len().saturating_sub(BACK_LIMIT);
            self.back.drain(..over);
        }
        Some(left)
    }

    /// Whether there is a map to go back to.
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// Goes back to the map shown before the current one. False when there is none.
    pub fn go_back(&mut self) -> bool {
        let Some((map, outcome)) = self.back.pop() else {
            return false;
        };
        self.queue.push_front(map);
        self.revisited.insert(0, outcome);
        self.history.pop();
        self.step = self.step.saturating_sub(1);
        self.playing_cached = None;
        self.shown_at = Instant::now();
        self.lane_slide = -28.0;
        true
    }

    /// Whether online fetches have kept failing for [`OUTAGE_GRACE`].
    pub fn outage(&self, now: Instant) -> bool {
        self.outage_since
            .is_some_and(|since| now.saturating_duration_since(since) >= OUTAGE_GRACE)
    }

    /// Goes on with the library only, because of `reason`. Offline for an outage, the server is
    /// asked again every [`OFFLINE_PROBE`].
    pub fn go_offline(&mut self, reason: Gate) {
        self.gate = None;
        self.offline = Some(reason);
        self.local_mode = true;
        self.fetching = None;
        self.error = None;
        self.retry_after = None;
        self.pool_exhausted = false;
        self.next_probe = (reason == Gate::ServerDown).then(|| Instant::now() + OFFLINE_PROBE);
    }

    /// Whether a Journey offline for an outage asks the server again now.
    pub fn probe_due(&self, now: Instant) -> bool {
        self.offline == Some(Gate::ServerDown)
            && self.fetching.is_none()
            && self.next_probe.is_none_or(|at| now >= at)
    }

    /// Polls any in-flight `JourneyNext` RPC and returns `true` if the queue changed.
    pub fn poll_fetch(&mut self, library: &[Entry]) -> bool {
        let Some(rx) = &self.fetching else {
            return false;
        };
        match rx.try_recv() {
            Ok(Ok(batch)) => {
                self.fetching = None;
                self.error = None;
                self.retry_after = None;
                self.fetch_failures = 0;
                self.outage_since = None;
                if self.gate == Some(Gate::ServerDown) {
                    self.gate = None;
                }
                // Offline for an outage, the server answered again: back online.
                if self.offline == Some(Gate::ServerDown) {
                    self.offline = None;
                    self.local_mode = false;
                    self.pool_exhausted = false;
                    self.next_probe = None;
                }
                if batch.needs_level {
                    self.needs_level = true;
                }
                if batch.comfort_stars > 0.2 {
                    self.comfort_stars = batch.comfort_stars;
                    if !batch.needs_level && self.target_stars.is_none() {
                        self.auto_comfort_stars = Some(batch.comfort_stars);
                    }
                }
                if batch.comfort_scroll.is_some() {
                    self.comfort_scroll = batch.comfort_scroll;
                }
                if batch.player_profile.is_some() {
                    self.player_profile = batch.player_profile;
                }
                if !batch.needs_level && batch.items.is_empty() {
                    self.pool_exhausted = true;
                    self.retry_after = Some(Instant::now() + EXHAUSTED_RETRY);
                }
                for item in batch.items {
                    self.enqueue_item(item, library);
                }
                true
            }
            Ok(Err(err)) => {
                self.fetch_failed(err);
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.fetch_failed("Journey request interrupted".into());
                true
            }
        }
    }

    /// An online fetch failed: tried again after a backoff, while the outage is timed. Offline,
    /// a failed probe leaves the library's maps coming.
    fn fetch_failed(&mut self, err: String) {
        self.fetching = None;
        self.error = Some(err);
        self.outage_since.get_or_insert_with(Instant::now);
        if !self.local_mode {
            self.backoff();
        }
    }

    /// Enqueues a recommended item, matching against the local library first.
    /// Cache checks and downloads run on a worker thread once within the first `MAX_PREFETCH_AHEAD`
    /// slots so gameplay and UI frames never block on disk I/O.
    pub fn enqueue_item(&mut self, item: JourneyItem, library: &[Entry]) {
        self.remember_item(&item);
        let protected = self.protected_cache_folders();
        let source = if let Some(path) = find_in_library(&item, library) {
            MapSource::Local(path)
        } else if self.queue.len() < MAX_PREFETCH_AHEAD {
            start_item_download(&item, &protected)
        } else {
            MapSource::Pending
        };
        self.queue.push_back(QueuedMap { item, source });
    }

    /// Starts any `Pending` downloads in the first `MAX_PREFETCH_AHEAD` slots and polls active jobs.
    /// Performs no synchronous disk I/O on the calling thread.
    pub fn poll_downloads(&mut self) -> bool {
        let protected = self.protected_cache_folders();
        let mut changed = false;
        for q in self.queue.iter_mut().take(MAX_PREFETCH_AHEAD) {
            if matches!(q.source, MapSource::Pending) {
                // Worker thread checks the cache first before downloading from mirrors.
                q.source = start_item_download(&q.item, &protected);
                changed = true;
            }
            let finished = match &mut q.source {
                MapSource::Downloading(job) => job.rx.try_recv().ok(),
                _ => None,
            };
            if let Some(res) = finished {
                changed = true;
                q.source = match res {
                    Ok((folder, entry)) => MapSource::Cached {
                        folder,
                        entry: Box::new(entry),
                    },
                    Err(e) => MapSource::Failed(e),
                };
            }
        }
        changed
    }

    /// Whether another refill request is allowed right now (respects `pool_exhausted` and backoff).
    pub fn can_refill(&self, now: Instant) -> bool {
        if self.needs_level || self.fetching.is_some() || self.queue.len() >= MAX_PREFETCH_AHEAD {
            return false;
        }
        // An exhausted pool is asked again once its wait is over (online only: the local
        // library doesn't grow by waiting).
        if self.pool_exhausted && (self.local_mode || self.retry_after.is_none()) {
            return false;
        }
        self.retry_after.is_none_or(|until| now >= until)
    }

    /// Spawns a background `Request::JourneyNext` call when logged in.
    pub fn request_next(&mut self, server: String, token: String, scroll_speed: f64) {
        if !self.can_refill(Instant::now()) {
            return;
        }
        let want = (MAX_PREFETCH_AHEAD.saturating_sub(self.queue.len())).clamp(1, 2) as u8;
        self.spawn_next(server, token, scroll_speed, want);
    }

    /// Asks the server for one map while offline for an outage: an answer brings Journey back
    /// online (`poll_fetch`).
    pub fn probe(&mut self, server: String, token: String, scroll_speed: f64) {
        self.next_probe = Some(Instant::now() + OFFLINE_PROBE);
        self.spawn_next(server, token, scroll_speed, 1);
    }

    fn spawn_next(&mut self, server: String, token: String, scroll_speed: f64, want: u8) {
        let (min_length_ms, max_length_ms) = self.length_bounds();
        let req = online::Request::JourneyNext {
            token,
            target_stars: self.target_stars,
            scroll_speed: (scroll_speed.is_finite() && scroll_speed > 0.1).then_some(scroll_speed),
            exclude_md5s: self.exclude_md5s.clone(),
            exclude_sets: self.exclude_sets.clone(),
            step: self.step.saturating_add(self.queue.len() as u32),
            count: want,
            focus: self.focus.wire(),
            chill: self.chill,
            min_length_ms,
            max_length_ms,
        };
        let (tx, rx) = mpsc::channel();
        self.fetching = Some(rx);
        std::thread::spawn(move || {
            let res = online::call(&server, req).and_then(|reply| match reply {
                online::Reply::Journey(batch) => Ok(batch),
                online::Reply::Error(e) => Err(e),
                _ => Err("Unexpected reply from server".into()),
            });
            let _ = tx.send(res);
        });
    }

    /// Fills the queue from the player's local library (offline / server without `journey/1`).
    /// `info_of(i)` supplies `(stars, bpm, length_ms)` for `library[i]`.
    pub fn fill_from_local(
        &mut self,
        library: &[Entry],
        info_of: impl Fn(usize) -> (Option<f32>, f64, u64),
    ) {
        self.local_mode = true;
        if self.pool_exhausted
            || (self.target_stars.is_none()
                && self.step == 0
                && self.queue.is_empty()
                && self.needs_level)
        {
            return;
        }
        // A chill session plays below the player's level, as the server picks one.
        let comfort = self.target_stars.unwrap_or(self.comfort_stars)
            - if self.chill { CHILL_DROP } else { 0.0 };
        let mut excluded_osu: HashSet<u32> = HashSet::new();
        let mut excluded_dirs: HashSet<String> = HashSet::new();
        for s in &self.exclude_sets {
            if let Some(id) = s.strip_prefix("osu:").and_then(|v| v.parse::<u32>().ok()) {
                excluded_osu.insert(id);
            } else if let Some(dir) = s.strip_prefix("dir:") {
                excluded_dirs.insert(dir.to_owned());
            }
        }
        let mut used_osu: HashSet<u32> = HashSet::new();
        let mut used_dirs: HashSet<Option<&std::ffi::OsStr>> = HashSet::new();

        while self.queue.len() < MAX_PREFETCH_AHEAD {
            let step = self.step.saturating_add(self.queue.len() as u32);
            let (role, reason, target) = if step == 0 {
                (
                    "warmup",
                    tr!(
                        "Warm-up · from your library",
                        "ウォームアップ · ライブラリから",
                        "Разминка · из твоей библиотеки"
                    ),
                    comfort - 0.4,
                )
            } else if step % 5 == 4 && !self.chill {
                (
                    "stretch",
                    tr!(
                        "Stretch · slightly above comfort",
                        "チャレンジ · 少し難しめ",
                        "Челлендж · чуть выше уровня"
                    ),
                    comfort + 0.5,
                )
            } else if step % 5 == 2 {
                (
                    "explore",
                    tr!(
                        "Change of pace · from your library",
                        "気分転換 · ライブラリから",
                        "Смена стиля · из твоей библиотеки"
                    ),
                    comfort,
                )
            } else {
                (
                    "main",
                    tr!(
                        "Fits your comfort level",
                        "ちょうどいい難易度",
                        "Подходит под твой уровень"
                    ),
                    comfort,
                )
            };

            let mut best: Option<(usize, f64, f64, f64, u64)> = None;
            for (i, e) in library.iter().enumerate() {
                if let Some(set_id) = e.meta.set_id {
                    if excluded_osu.contains(&set_id) || used_osu.contains(&set_id) {
                        continue;
                    }
                } else {
                    let dir = e.path.parent().and_then(|p| p.file_name());
                    if used_dirs.contains(&dir) {
                        continue;
                    }
                    if !excluded_dirs.is_empty() {
                        let dir_str = dir.map(|n| n.to_string_lossy()).unwrap_or_default();
                        if excluded_dirs.contains(dir_str.as_ref()) {
                            continue;
                        }
                    }
                }
                // Strongly prefer maps with known stars so unrated maps don't always win.
                let (stars_opt, bpm, length_ms) = info_of(i);
                let (min_len, max_len) = self.length_bounds();
                let mut unknown_len_penalty = 0.0;
                if length_ms > 0 {
                    if min_len.is_some_and(|lo| length_ms < lo)
                        || max_len.is_some_and(|hi| length_ms > hi)
                    {
                        continue;
                    }
                } else if min_len.is_some() || max_len.is_some() {
                    // Prefer charts with measured duration when length filtering is active.
                    unknown_len_penalty = 50.0;
                }
                let (st, unrated_penalty) = match stars_opt {
                    Some(s) if s.is_finite() && s > 0.1 => (f64::from(s), 0.0),
                    _ => (0.0, 100.0),
                };
                let dist = if unrated_penalty == 0.0 {
                    (st - target).abs()
                } else {
                    unrated_penalty
                };
                let jitter = (((i as u64).wrapping_mul(2654435761) ^ (step as u64 * 97)) % 100)
                    as f64
                    * 0.004;
                let score = dist + unknown_len_penalty + jitter;
                if best.is_none_or(|(_, b_score, _, _, _)| score < b_score) {
                    best = Some((i, score, st, bpm, length_ms));
                }
            }
            let Some((idx, _, st, bpm, length_ms)) = best else {
                self.pool_exhausted = true;
                break;
            };
            let e = &library[idx];
            if let Some(set_id) = e.meta.set_id {
                used_osu.insert(set_id);
            } else {
                used_dirs.insert(e.path.parent().and_then(|p| p.file_name()));
            }
            let item = JourneyItem {
                md5: format!("local:{}", e.path.display()),
                map_hash: e.known_hash(),
                set_id: e.meta.set_id,
                custom_id: None,
                title: e.meta.title.clone(),
                artist: e.meta.artist.clone(),
                creator: e.meta.creator.clone(),
                version: e.meta.version.clone(),
                stars: st,
                bpm,
                length_ms,
                role: role.into(),
                reason: reason.into(),
                scroll_velocity: 0.0,
                profile: JourneyProfile::default(),
            };
            self.remember_item(&item);
            self.queue.push_back(QueuedMap {
                item,
                source: MapSource::Local(e.path.clone()),
            });
        }
    }
}

/// Canonical set key for deduplicating sets across a session.
pub fn set_key_of(item: &JourneyItem) -> String {
    if let Some(c) = item.custom_id {
        format!("custom:{c}")
    } else if let Some(s) = item.set_id {
        format!("osu:{s}")
    } else {
        format!("md5:{}", item.md5)
    }
}

/// Searches the player's local library for a matching map by exact `map_hash` or `set_id + version`.
pub fn find_in_library(item: &JourneyItem, library: &[Entry]) -> Option<PathBuf> {
    if let Some(h) = item.map_hash
        && let Some(e) = library.iter().find(|e| e.known_hash() == Some(h))
    {
        return Some(e.path.clone());
    }
    if let Some(set_id) = item.set_id
        && let Some(e) = library
            .iter()
            .find(|e| e.meta.set_id == Some(set_id) && e.meta.version == item.version)
    {
        return Some(e.path.clone());
    }
    None
}

fn find_in_cache(item: &JourneyItem) -> Option<(PathBuf, Entry)> {
    let cache = config::journey_cache_dir();
    let info = item_to_set_info(item)?;
    let folder = cache.join(info.folder_name());
    if folder.is_dir() {
        let entry = load_entry_from_folder(&folder, item)?;
        return Some((folder, entry));
    }
    None
}

fn item_to_set_info(item: &JourneyItem) -> Option<download::SetInfo> {
    let (id, custom) = match (item.custom_id, item.set_id) {
        (Some(cid), _) => (cid, Some(download::CustomSet::default())),
        (None, Some(sid)) => (sid, None),
        (None, None) => return None,
    };
    Some(download::SetInfo {
        id,
        title: item.title.clone(),
        artist: item.artist.clone(),
        creator: item.creator.clone(),
        status: String::new(),
        diffs: Vec::new(),
        play_count: 0,
        favourite_count: 0,
        rating: 0.0,
        ranked_date: String::new(),
        custom,
    })
}

fn start_item_download(item: &JourneyItem, protected: &[PathBuf]) -> MapSource {
    let Some(info) = item_to_set_info(item) else {
        return MapSource::Failed("Map has no downloadable set ID".into());
    };
    let done = Arc::new(AtomicU64::new(0));
    let total = Arc::new(AtomicU64::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let (d, t, c) = (done.clone(), total.clone(), cancel.clone());
    let item = item.clone();
    let protected = protected.to_vec();
    std::thread::spawn(move || {
        let res = if let Some(cached) = find_in_cache(&item) {
            Ok(cached)
        } else if c.load(Ordering::Relaxed) {
            Err(download::CANCELLED.into())
        } else {
            let cache = config::journey_cache_dir();
            let _ = std::fs::create_dir_all(&cache);
            prune_cache(&cache, MAX_CACHE_FOLDERS, &protected);
            let (dl, _) = download::fetch_set(&info, &cache, &d, &t, &c);
            dl.and_then(|folder| {
                load_entry_from_folder(&folder, &item)
                    .map(|entry| (folder, entry))
                    .ok_or_else(|| {
                        tr!(
                            "No playable taiko chart in downloaded set",
                            "ダウンロードしたセットに遊べる太鼓譜面がありません",
                            "В скачанном сете нет играбельной taiko-карты"
                        )
                        .into()
                    })
            })
        };
        let _ = tx.send(res);
    });
    MapSource::Downloading(Box::new(DownloadTask {
        done,
        total,
        cancel,
        rx,
    }))
}

/// The maps in one set folder (none when it's gone). Read with a scan cache of Journey's own, so
/// the library's cache isn't written from two places at once.
pub fn scan_single_folder(folder: &Path) -> Vec<Entry> {
    if !folder.is_dir() {
        return Vec::new();
    }
    let found = AtomicUsize::new(0);
    let cache = config::journey_cache_dir().join("scan-cache");
    crate::library::scan_cached(&[folder.to_path_buf()], &found, &cache, &mut |_| {})
}

/// Where `path`, inside `folder`, is once `folder` has moved to `dest`.
pub fn moved_path(path: &Path, folder: &Path, dest: &Path) -> PathBuf {
    path.strip_prefix(folder)
        .map_or_else(|_| path.to_path_buf(), |rel| dest.join(rel))
}

fn load_entry_from_folder(folder: &Path, item: &JourneyItem) -> Option<Entry> {
    pick_difficulty(scan_single_folder(folder), item)
}

/// The difficulty of a set that `item` is: by its hash, else by its name. None when neither
/// matches: another difficulty of the set would be played and scored as this one.
pub fn pick_difficulty(entries: Vec<Entry>, item: &JourneyItem) -> Option<Entry> {
    let by_hash = item
        .map_hash
        .and_then(|h| entries.iter().position(|e| e.known_hash() == Some(h)));
    let at = by_hash.or_else(|| {
        entries
            .iter()
            .position(|e| e.meta.version.eq_ignore_ascii_case(&item.version))
    })?;
    entries.into_iter().nth(at)
}

/// Moves a cached mapset folder into the player's permanent library folder:
/// - custom maps -> `config::custom_dir()`
/// - osu! maps -> `config::own_songs_dir()` when `save_to == JourneySaveTo::Own` (default),
///   or `osu_songs` when `save_to == JourneySaveTo::Osu` (an error without one: taiko.rs's own
///   folder isn't osu!'s).
pub fn save_cached_folder(
    cached_folder: &Path,
    is_custom: bool,
    save_to: JourneySaveTo,
    osu_songs: Option<&Path>,
) -> Result<PathBuf, String> {
    let dest_root = if is_custom {
        config::custom_dir()
    } else {
        match save_to {
            JourneySaveTo::Own => config::own_songs_dir(),
            JourneySaveTo::Osu => osu_songs.map(Path::to_path_buf).ok_or_else(|| {
                tr!(
                    "no osu! Songs folder found: pick one in Settings > Library",
                    "osu!のSongsフォルダが見つかりません: 設定 > ライブラリで選んでください",
                    "папка osu! Songs не найдена: выбери её в Настройки > Библиотека"
                )
                .to_string()
            })?,
        }
    };
    std::fs::create_dir_all(&dest_root)
        .map_err(|e| format!("create {}: {e}", dest_root.display()))?;
    let name = cached_folder
        .file_name()
        .ok_or_else(|| "invalid cached folder name".to_string())?;
    let dest = dest_root.join(name);
    if dest.exists() {
        return Ok(dest);
    }
    if std::fs::rename(cached_folder, &dest).is_ok() {
        return Ok(dest);
    }
    // Another drive: copied beside it first, so a copy cut short is never taken for the set.
    let mut part_name = name.to_os_string();
    part_name.push(".part");
    let part = dest_root.join(part_name);
    let _ = std::fs::remove_dir_all(&part);
    let copied = copy_dir_all(cached_folder, &part).and_then(|()| std::fs::rename(&part, &dest));
    if let Err(e) = copied {
        let _ = std::fs::remove_dir_all(&part);
        return Err(format!("copy to {}: {e}", dest.display()));
    }
    let _ = std::fs::remove_dir_all(cached_folder);
    Ok(dest)
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Keeps at most `max_keep` most recently modified folders in `JourneyCache`,
/// never removing any folder listed in `protected`.
pub fn prune_cache(cache_dir: &Path, max_keep: usize, protected: &[PathBuf]) {
    let Ok(rd) = std::fs::read_dir(cache_dir) else {
        return;
    };
    let mut dirs: Vec<(PathBuf, std::time::SystemTime)> = rd
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| {
            let mtime = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (e.path(), mtime)
        })
        .collect();
    if dirs.len() <= max_keep {
        return;
    }
    dirs.sort_by_key(|a| std::cmp::Reverse(a.1));
    for (old, _) in dirs.into_iter().skip(max_keep) {
        if protected.iter().any(|p| p == &old) {
            continue;
        }
        let _ = std::fs::remove_dir_all(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_cached_folder_respects_own_and_osu_destinations() {
        let base = std::env::temp_dir().join(format!("taiko-journey-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let osu = base.join("osu");

        // 1. JourneySaveTo::Own moves into config::own_songs_dir() (not osu!).
        let cache_own = base.join("cache/101 Own Artist - Own Title");
        std::fs::create_dir_all(&cache_own).unwrap();
        std::fs::write(cache_own.join("own.osu"), b"test-own").unwrap();
        let saved_own =
            save_cached_folder(&cache_own, false, JourneySaveTo::Own, Some(&osu)).unwrap();
        assert_eq!(
            saved_own,
            config::own_songs_dir().join("101 Own Artist - Own Title")
        );
        assert!(saved_own.join("own.osu").is_file());
        assert!(!cache_own.exists());
        let _ = std::fs::remove_dir_all(&saved_own);

        // 2. JourneySaveTo::Osu moves into the provided osu! Songs folder.
        let cache_osu = base.join("cache/123 Artist - Title");
        std::fs::create_dir_all(&cache_osu).unwrap();
        std::fs::write(cache_osu.join("map.osu"), b"test-osu").unwrap();
        let saved_osu =
            save_cached_folder(&cache_osu, false, JourneySaveTo::Osu, Some(&osu)).unwrap();
        assert_eq!(saved_osu, osu.join("123 Artist - Title"));
        assert!(saved_osu.join("map.osu").is_file());
        assert!(!cache_osu.exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn prune_cache_keeps_newest_and_never_deletes_protected_folders() {
        let base = std::env::temp_dir().join(format!("taiko-journey-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let mut folders = Vec::new();
        for i in 0..5 {
            let p = base.join(format!("set-{i}"));
            std::fs::create_dir_all(&p).unwrap();
            folders.push(p);
        }
        let protected = vec![folders[0].clone()];
        prune_cache(&base, 2, &protected);
        assert!(folders[0].is_dir(), "protected folder must survive pruning");
        let left = std::fs::read_dir(&base).unwrap().count();
        assert!(left <= 3, "only max_keep + protected remain: {left}");
        let _ = std::fs::remove_dir_all(&base);
    }

    fn test_item(version: &str, set: u32) -> JourneyItem {
        JourneyItem {
            md5: String::new(),
            map_hash: None,
            set_id: Some(set),
            custom_id: None,
            title: "T".into(),
            artist: "A".into(),
            creator: "C".into(),
            version: version.into(),
            stars: 4.0,
            bpm: 0.0,
            length_ms: 0,
            role: "main".into(),
            reason: String::new(),
            scroll_velocity: 0.0,
            profile: JourneyProfile::default(),
        }
    }

    const CHART: &str = "osu file format v14\n\n[General]\nAudioFilename: a.ogg\nMode: 1\n\n[Metadata]\nTitle:T\nArtist:A\nCreator:C\nVersion:VERSION\n\n[Difficulty]\nOverallDifficulty:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,400,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";

    #[test]
    fn a_difficulty_journey_did_not_ask_for_is_never_picked() {
        let dir = std::env::temp_dir().join(format!("taiko-journey-pick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.osu"), CHART.replace("VERSION", "Easy")).unwrap();
        std::fs::write(dir.join("b.osu"), CHART.replace("VERSION", "Oni")).unwrap();
        let found = scan_single_folder(&dir);
        assert_eq!(found.len(), 2);
        let oni = pick_difficulty(found, &test_item("oni", 1)).unwrap();
        assert_eq!(oni.meta.version, "Oni");
        assert!(pick_difficulty(scan_single_folder(&dir), &test_item("Inner Oni", 1)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
        // A folder gone (moved on Save) has no maps, rather than being read as a file.
        assert!(scan_single_folder(&dir).is_empty());
    }

    #[test]
    fn a_download_paused_for_a_play_resumes_from_the_cache_when_it_had_finished() {
        let item = test_item("Oni", 4242);
        let folder =
            config::journey_cache_dir().join(item_to_set_info(&item).unwrap().folder_name());
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("b.osu"), CHART.replace("VERSION", "Oni")).unwrap();
        let mut j = Journey::new();
        j.queue.push_back(QueuedMap {
            item,
            source: MapSource::Pending,
        });
        assert!(j.poll_downloads());
        for _ in 0..100 {
            if matches!(j.queue[0].source, MapSource::Cached { .. }) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
            j.poll_downloads();
        }
        assert!(
            matches!(&j.queue[0].source, MapSource::Cached { folder: f, .. } if *f == folder),
            "no download started again"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn saving_to_osu_without_an_osu_songs_folder_is_an_error() {
        let base =
            std::env::temp_dir().join(format!("taiko-journey-no-osu-{}", std::process::id()));
        let cached = base.join("7 A - T");
        std::fs::create_dir_all(&cached).unwrap();
        assert!(save_cached_folder(&cached, false, JourneySaveTo::Osu, None).is_err());
        assert!(cached.is_dir(), "kept in the cache");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn saving_moves_every_queued_map_of_the_set_and_ends_the_play_to_save() {
        let folder = PathBuf::from("/cache/5 A - T");
        let dest = PathBuf::from("/songs/5 A - T");
        let mut j = Journey::new();
        let entry = |f: &str| {
            let meta = crate::beatmap::Meta {
                version: f.into(),
                ..Default::default()
            };
            Box::new(Entry::new(folder.join(f), meta))
        };
        for f in ["a.osu", "b.osu"] {
            j.queue.push_back(QueuedMap {
                item: test_item("Oni", 5),
                source: MapSource::Cached {
                    folder: folder.clone(),
                    entry: entry(f),
                },
            });
        }
        j.playing_cached = Some((folder.clone(), test_item("Oni", 5)));
        j.saved(&folder, &dest);
        let saved: Vec<_> = j
            .queue
            .iter()
            .map(|q| match &q.source {
                MapSource::Saved(p) => p.clone(),
                _ => PathBuf::new(),
            })
            .collect();
        assert_eq!(saved, [dest.join("a.osu"), dest.join("b.osu")]);
        assert!(j.playing_cached.is_none());
    }

    #[test]
    fn failed_fetches_back_off_longer_and_the_exclusions_stay_bounded() {
        let mut j = Journey::new();
        let t0 = Instant::now();
        j.backoff();
        let first = j.retry_after.unwrap() - t0;
        j.backoff();
        j.backoff();
        let third = j.retry_after.unwrap() - t0;
        assert!(third > first * 3, "{first:?} then {third:?}");
        for i in 0..(MAX_EXCLUDES as u32 + 50) {
            let mut item = test_item("Oni", i);
            item.md5 = format!("{i:032}");
            j.remember_item(&item);
        }
        assert_eq!(j.exclude_md5s.len(), MAX_EXCLUDES);
        assert_eq!(j.exclude_sets.len(), MAX_EXCLUDES);
        assert_eq!(
            j.exclude_md5s.last().unwrap(),
            &format!("{:032}", MAX_EXCLUDES + 49)
        );
    }

    #[test]
    fn going_back_shows_the_last_map_again_and_moving_on_keeps_its_outcome() {
        let mut j = Journey::new();
        for set in 1..=3 {
            j.queue.push_back(QueuedMap {
                item: test_item("Oni", set),
                source: MapSource::Local(PathBuf::from(format!("{set}.osu"))),
            });
        }
        assert!(!j.can_go_back());
        let first = j.advance(PastOutcome::Passed).unwrap();
        assert!(!first.revisited);
        j.advance(PastOutcome::Skipped).unwrap();
        assert_eq!(j.step, 2);

        // Back twice: the first map is up again, with nothing on the lane before it.
        assert!(j.go_back() && j.go_back());
        assert!(!j.go_back());
        assert_eq!(j.step, 0);
        assert!(j.history.is_empty());
        assert_eq!(j.current().unwrap().item.set_id, Some(1));

        // Moving on past it without playing keeps its pass; the second was skipped before.
        let again = j.advance(PastOutcome::Skipped).unwrap();
        assert!(again.revisited);
        assert_eq!(j.history[0].outcome, PastOutcome::Passed);
        // Playing the second this time counts.
        assert!(j.advance(PastOutcome::Failed).unwrap().revisited);
        assert_eq!(j.history[1].outcome, PastOutcome::Failed);
        // The third was never left: a first verdict.
        assert!(!j.advance(PastOutcome::Skipped).unwrap().revisited);
    }

    #[test]
    fn a_failed_download_is_not_kept_to_go_back_to() {
        let mut j = Journey::new();
        j.queue.push_back(QueuedMap {
            item: test_item("Oni", 1),
            source: MapSource::Failed("gone".into()),
        });
        j.advance(PastOutcome::Skipped);
        assert!(!j.can_go_back());
    }

    #[test]
    fn current_library_index_caches_hits_misses_and_tracks_library_changes() {
        let entry = |p: &str, set: Option<u32>| {
            let meta = crate::beatmap::Meta {
                set_id: set,
                version: "Oni".into(),
                ..Default::default()
            };
            Entry::new(PathBuf::from(p), meta)
        };
        let mut library = vec![
            entry("/songs/1 a/a.osu", Some(1)),
            entry("/songs/2 b/b.osu", Some(2)),
        ];
        let mut j = Journey::new();
        j.queue.push_back(QueuedMap {
            item: test_item("Oni", 2),
            source: MapSource::Local(PathBuf::from("/songs/2 b/b.osu")),
        });
        assert_eq!(j.current_library_index(&library), Some(1));
        assert_eq!(j.current_library_index(&library), Some(1));

        // Reordering library invalidates the cached slot and finds the new index.
        library.swap(0, 1);
        assert_eq!(j.current_library_index(&library), Some(0));

        // Missing path caches None until the library changes and adds it.
        j.queue[0].source = MapSource::Local(PathBuf::from("/songs/3 c/c.osu"));
        assert_eq!(j.current_library_index(&library), None);
        assert_eq!(j.current_library_index(&library), None);
        library.push(entry("/songs/3 c/c.osu", Some(3)));
        assert_eq!(j.current_library_index(&library), Some(2));

        // Cached maps are not library entries.
        j.queue[0].source = MapSource::Cached {
            folder: PathBuf::from("/cache/3 c"),
            entry: Box::new(entry("/cache/3 c/c.osu", Some(3))),
        };
        assert_eq!(j.current_library_index(&library), None);
    }

    #[test]
    fn fill_from_local_respects_exclusions_and_populates_bpm_and_length() {
        let entry = |p: &str, set: Option<u32>| {
            let meta = crate::beatmap::Meta {
                title: p.into(),
                set_id: set,
                version: "Oni".into(),
                ..Default::default()
            };
            Entry::new(PathBuf::from(p), meta)
        };
        let library = vec![
            entry("/songs/10 a/a.osu", Some(10)),
            entry("/songs/20 b/b.osu", Some(20)),
            entry("/songs/local_dir/c1.osu", None),
            entry("/songs/local_dir/c2.osu", None),
        ];
        let mut j = Journey::new();
        j.target_stars = Some(5.0);
        j.comfort_stars = 5.0;
        j.exclude_sets.push("osu:10".into());

        j.fill_from_local(&library, |i| match i {
            0 => (Some(4.6), 200.0, 90_000),
            1 => (Some(4.7), 185.0, 120_000),
            2 => (Some(5.0), 170.0, 100_000),
            _ => (Some(5.0), 160.0, 110_000),
        });

        assert_eq!(j.queue.len(), MAX_PREFETCH_AHEAD);
        // Set 10 is excluded; index 1 (set 20) wins warmup (4.6 target), and one of local_dir wins main.
        assert_eq!(j.queue[0].item.set_id, Some(20));
        assert_eq!(j.queue[0].item.bpm, 185.0);
        assert_eq!(j.queue[0].item.length_ms, 120_000);
        assert_eq!(j.queue[1].item.set_id, None);
        assert_eq!(j.queue[1].item.bpm, 170.0);
        assert_eq!(j.queue[1].item.length_ms, 100_000);
    }

    #[test]
    fn fill_from_local_enforces_length_bounds_and_marathon_cap() {
        let entry = |p: &str, set: u32| {
            let meta = crate::beatmap::Meta {
                title: p.into(),
                set_id: Some(set),
                version: "Oni".into(),
                ..Default::default()
            };
            Entry::new(PathBuf::from(p), meta)
        };
        let library = vec![
            entry("/songs/1 short/a.osu", 1),
            entry("/songs/2 mid/b.osu", 2),
            entry("/songs/3 hour/c.osu", 3),
        ];
        let info = |i: usize| match i {
            0 => (Some(5.0), 180.0, 90_000),
            1 => (Some(5.0), 180.0, 180_000),
            _ => (Some(5.0), 180.0, 3_600_000),
        };

        // Marathon with Length::Any applies the 5-minute default cap, keeping the 1-hour map out.
        let mut j = Journey::new();
        j.target_stars = Some(5.0);
        j.comfort_stars = 5.0;
        j.marathon_cap = true;
        j.fill_from_local(&library, info);
        assert_eq!(j.queue.len(), 2);
        assert!(j.queue.iter().all(|q| q.item.length_ms <= 300_000));

        // Short (<= 2m): only the 90s chart qualifies.
        let mut j = Journey::new();
        j.target_stars = Some(5.0);
        j.comfort_stars = 5.0;
        j.length = JourneyLength::Short;
        j.fill_from_local(&library, info);
        assert_eq!(j.queue.len(), 1);
        assert_eq!(j.queue[0].item.set_id, Some(1));

        // Long (4m+): only the 1-hour chart qualifies.
        let mut j = Journey::new();
        j.target_stars = Some(5.0);
        j.comfort_stars = 5.0;
        j.length = JourneyLength::Long;
        j.fill_from_local(&library, info);
        assert_eq!(j.queue.len(), 1);
        assert_eq!(j.queue[0].item.set_id, Some(3));
    }
}
