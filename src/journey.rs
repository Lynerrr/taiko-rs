//! Journey v1 session state, temporary on-demand map cache, and local fallback picker.
//!
//! Maps the player does not own are downloaded strictly on demand (at most 2 ahead in the queue)
//! into `config::journey_cache_dir()`. Clicking Save (or liking with `journey_save_on_like`) moves
//! the cached folder into taiko.rs's own Songs folder by default (or osu!'s Songs folder when set
//! in Settings > Library).

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::config::{self, JourneySaveTo};
use crate::download;
use crate::library::Entry;
use crate::online::{self, JourneyBatch, JourneyItem, JourneyProfile};

/// Maximum number of upcoming maps allowed to download ahead at once (on-demand only).
pub const MAX_PREFETCH_AHEAD: usize = 2;
/// Maximum cached mapset folders kept in `JourneyCache` before oldest are pruned.
pub const MAX_CACHE_FOLDERS: usize = 20;
/// Backoff after a failed online `JourneyNext` request before trying again (doubled after each
/// failure in a row, up to `MAX_FETCH_BACKOFF`).
const FETCH_ERROR_BACKOFF: Duration = Duration::from_secs(5);
const MAX_FETCH_BACKOFF: Duration = Duration::from_secs(120);
/// How long an exhausted pool waits before asking again (new maps may have been fingerprinted).
const EXHAUSTED_RETRY: Duration = Duration::from_secs(60);
/// Most maps (and sets) a session remembers to not suggest again; the oldest go first.
const MAX_EXCLUDES: usize = 400;

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

#[derive(Clone, Debug)]
pub struct PastMap {
    pub title: String,
    pub role: String,
    pub outcome: PastOutcome,
}

/// Where a queued Journey map currently lives on disk.
/// Uses stable `PathBuf`s rather than library indices so `refresh_library` never shifts targets.
pub enum MapSource {
    /// Already in `State::library` at this `.osu`/`.tko` path.
    Local(PathBuf),
    /// Queued beyond the first `MAX_PREFETCH_AHEAD` slots; download starts when it shifts up.
    Pending,
    /// Currently downloading into `JourneyCache`.
    Downloading(Box<download::Job>),
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
    /// The current map's library index, found by its path (kept while it still matches).
    library_index: Option<(PathBuf, usize)>,
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
        }
    }

    /// Where the current map is in `library`, when it's a library map (none while it's cached,
    /// downloading or failed).
    pub fn current_library_index(&mut self, library: &[Entry]) -> Option<usize> {
        let path = match self.queue.front().map(|q| &q.source) {
            Some(MapSource::Local(p) | MapSource::Saved(p)) => p,
            _ => return None,
        };
        if let Some((known, i)) = &self.library_index
            && known == path
            && library.get(*i).is_some_and(|e| &e.path == path)
        {
            return Some(*i);
        }
        let i = library.iter().position(|e| &e.path == path)?;
        self.library_index = Some((path.clone(), i));
        Some(i)
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
        self.in_play = false;
        self.fetching = None;
        self.playing_cached = None;
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
        for q in &mut self.queue {
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
    }

    /// Switches the session to a manual target star rating and rebuilds the queue for it.
    pub fn set_manual_level(&mut self, stars: f64) {
        let clamped = ((stars * 4.0).round() / 4.0).clamp(1.0, 11.0);
        self.cancel_downloads();
        self.queue.clear();
        self.fetching = None;
        self.target_stars = Some(clamped);
        self.comfort_stars = clamped;
        self.needs_level = false;
        self.pool_exhausted = false;
        self.retry_after = None;
        self.error = None;
        self.shown_at = Instant::now();
    }

    /// Folders currently referenced by the queue or active play (protected from `prune_cache`).
    pub fn protected_cache_folders(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for q in &self.queue {
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

    /// Advances past the front map and records its outcome on the top mini-lane.
    pub fn advance(&mut self, outcome: PastOutcome) -> Option<QueuedMap> {
        let popped = self.queue.pop_front()?;
        if let MapSource::Downloading(job) = &popped.source {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.playing_cached = None;
        self.history.push(PastMap {
            title: popped.item.title.clone(),
            role: popped.item.role.clone(),
            outcome,
        });
        if self.history.len() > 16 {
            self.history.remove(0);
        }
        self.step = self.step.saturating_add(1);
        self.shown_at = Instant::now();
        self.lane_slide = 28.0;
        Some(popped)
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
                self.needs_level = batch.needs_level;
                if batch.comfort_stars > 0.2 {
                    self.comfort_stars = batch.comfort_stars;
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
                self.fetching = None;
                self.error = Some(err);
                self.backoff();
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.fetching = None;
                self.error = Some("Journey request interrupted".into());
                self.backoff();
                true
            }
        }
    }

    /// Enqueues a recommended item, matching against the local library first.
    /// Downloads only start once an item is within the first `MAX_PREFETCH_AHEAD` slots.
    pub fn enqueue_item(&mut self, item: JourneyItem, library: &[Entry]) {
        self.remember_item(&item);
        let protected = self.protected_cache_folders();
        let source = if let Some(path) = find_in_library(&item, library) {
            MapSource::Local(path)
        } else if let Some((folder, entry)) = find_in_cache(&item) {
            MapSource::Cached {
                folder,
                entry: Box::new(entry),
            }
        } else if self.queue.len() < MAX_PREFETCH_AHEAD {
            start_item_download(&item, &protected)
        } else {
            MapSource::Pending
        };
        self.queue.push_back(QueuedMap { item, source });
    }

    /// Starts any `Pending` downloads in the first `MAX_PREFETCH_AHEAD` slots and polls active jobs.
    pub fn poll_downloads(&mut self) -> bool {
        let protected = self.protected_cache_folders();
        let mut changed = false;
        for q in self.queue.iter_mut().take(MAX_PREFETCH_AHEAD) {
            if matches!(q.source, MapSource::Pending) {
                // A download paused for a play may have finished unpacking first.
                q.source = match find_in_cache(&q.item) {
                    Some((folder, entry)) => MapSource::Cached {
                        folder,
                        entry: Box::new(entry),
                    },
                    None => start_item_download(&q.item, &protected),
                };
                changed = true;
            }
            let finished = match &mut q.source {
                MapSource::Downloading(job) => job.rx.try_recv().ok(),
                _ => None,
            };
            if let Some(res) = finished {
                changed = true;
                q.source = match res {
                    Ok(folder) => match load_entry_from_folder(&folder, &q.item) {
                        Some(entry) => MapSource::Cached {
                            folder,
                            entry: Box::new(entry),
                        },
                        None => MapSource::Failed(
                            tr!(
                                "No playable taiko chart in downloaded set",
                                "ダウンロードしたセットに遊べる太鼓譜面がありません",
                                "В скачанном сете нет играбельной taiko-карты"
                            )
                            .into(),
                        ),
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
        let req = online::Request::JourneyNext {
            token,
            target_stars: self.target_stars,
            scroll_speed: (scroll_speed.is_finite() && scroll_speed > 0.1).then_some(scroll_speed),
            exclude_md5s: self.exclude_md5s.clone(),
            exclude_sets: self.exclude_sets.clone(),
            step: self.step.saturating_add(self.queue.len() as u32),
            count: want,
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
    pub fn fill_from_local(&mut self, library: &[Entry], stars_of: impl Fn(usize) -> Option<f32>) {
        self.local_mode = true;
        if self.pool_exhausted
            || (self.target_stars.is_none()
                && self.step == 0
                && self.queue.is_empty()
                && self.needs_level)
        {
            return;
        }
        let comfort = self.target_stars.unwrap_or(self.comfort_stars);
        let excluded: HashSet<String> = self.exclude_sets.iter().cloned().collect();
        let mut used_local: HashSet<String> = HashSet::new();

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
            } else if step % 5 == 4 {
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

            let mut best: Option<(usize, f64, f64)> = None;
            for (i, e) in library.iter().enumerate() {
                let sk = local_set_key(e);
                if excluded.contains(&sk) || used_local.contains(&sk) {
                    continue;
                }
                // Strongly prefer maps with known stars so unrated maps don't always win.
                let (st, unrated_penalty) = match stars_of(i) {
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
                let score = dist + jitter;
                if best.is_none_or(|(_, b_score, _)| score < b_score) {
                    best = Some((i, score, st));
                }
            }
            let Some((idx, _, st)) = best else {
                self.pool_exhausted = true;
                break;
            };
            let e = &library[idx];
            used_local.insert(local_set_key(e));
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
                bpm: 0.0,
                length_ms: 0,
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

fn local_set_key(e: &Entry) -> String {
    if let Some(s) = e.meta.set_id {
        format!("osu:{s}")
    } else {
        format!(
            "dir:{}",
            e.path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        )
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
    let cache = config::journey_cache_dir();
    let _ = std::fs::create_dir_all(&cache);
    prune_cache(&cache, MAX_CACHE_FOLDERS, protected);
    MapSource::Downloading(Box::new(download::start(info, cache)))
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
}
