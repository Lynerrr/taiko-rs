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
use std::time::Instant;

use crate::config::{self, JourneySaveTo};
use crate::download;
use crate::library::Entry;
use crate::online::{self, JourneyBatch, JourneyItem};

/// Maximum number of upcoming maps allowed to download ahead at once (on-demand only).
pub const MAX_PREFETCH_AHEAD: usize = 2;
/// Maximum cached mapset folders kept in `JourneyCache` before oldest are pruned.
pub const MAX_CACHE_FOLDERS: usize = 20;

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
pub enum MapSource {
    /// Already in `State::library` at index `usize`.
    Local(usize),
    /// Currently downloading into `JourneyCache`.
    Downloading(Box<download::Job>),
    /// Unpacked in `JourneyCache/<folder>`, ready to play and save.
    Cached { folder: PathBuf, entry: Box<Entry> },
    /// Moved from cache into the player's library at index `usize`.
    Saved(usize),
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
    pub target_stars: Option<f64>,
    pub step: u32,
    pub queue: VecDeque<QueuedMap>,
    pub history: Vec<PastMap>,
    pub exclude_md5s: Vec<String>,
    pub exclude_sets: Vec<String>,
    pub fetching: Option<Receiver<Result<JourneyBatch, String>>>,
    pub error: Option<String>,
    pub shown_at: Instant,
    /// Set to true when the current play was launched from Journey (tags `PlayedAttempt`).
    pub in_play: bool,
    /// Temporary cached entry currently being played (kept so results screen can Save it).
    pub playing_cached: Option<(PathBuf, JourneyItem)>,
    /// True when the active session is running against local library fallback (offline / old server).
    pub local_mode: bool,
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
            target_stars: None,
            step: 0,
            queue: VecDeque::new(),
            history: Vec::new(),
            exclude_md5s: Vec::new(),
            exclude_sets: Vec::new(),
            fetching: None,
            error: None,
            shown_at: Instant::now(),
            in_play: false,
            playing_cached: None,
            local_mode: false,
        }
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
    }

    fn cancel_downloads(&mut self) {
        for q in &self.queue {
            if let MapSource::Downloading(job) = &q.source {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
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
    }

    /// Advances past the front map and records its outcome on the top mini-lane.
    pub fn advance(&mut self, outcome: PastOutcome) -> Option<QueuedMap> {
        let popped = self.queue.pop_front()?;
        if let MapSource::Downloading(job) = &popped.source {
            job.cancel.store(true, Ordering::Relaxed);
        }
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
                self.needs_level = batch.needs_level;
                if batch.comfort_stars > 0.2 {
                    self.comfort_stars = batch.comfort_stars;
                }
                for item in batch.items {
                    self.enqueue_item(item, library);
                }
                true
            }
            Ok(Err(err)) => {
                self.fetching = None;
                self.error = Some(err);
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.fetching = None;
                self.error = Some("Journey request interrupted".into());
                true
            }
        }
    }

    /// Enqueues a recommended item, matching against the local library first.
    pub fn enqueue_item(&mut self, item: JourneyItem, library: &[Entry]) {
        self.remember_item(&item);
        let source = if let Some(idx) = find_in_library(&item, library) {
            MapSource::Local(idx)
        } else if let Some((folder, entry)) = find_in_cache(&item) {
            MapSource::Cached {
                folder,
                entry: Box::new(entry),
            }
        } else {
            // Only start downloading if within the first MAX_PREFETCH_AHEAD slots.
            if self.queue.len() < MAX_PREFETCH_AHEAD {
                start_item_download(&item)
            } else {
                // Will be started once it shifts into the first 2 slots.
                start_item_download(&item)
            }
        };
        self.queue.push_back(QueuedMap { item, source });
    }

    /// Polls active downloads in the first `MAX_PREFETCH_AHEAD` slots.
    pub fn poll_downloads(&mut self) -> bool {
        let mut changed = false;
        for q in self.queue.iter_mut().take(MAX_PREFETCH_AHEAD) {
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
                        None => {
                            MapSource::Failed("No playable taiko chart in downloaded set".into())
                        }
                    },
                    Err(e) => MapSource::Failed(e),
                };
            }
        }
        changed
    }

    /// Spawns a background `Request::JourneyNext` call when logged in.
    pub fn request_next(&mut self, server: String, token: String) {
        if self.fetching.is_some() || self.queue.len() >= MAX_PREFETCH_AHEAD {
            return;
        }
        let want = (MAX_PREFETCH_AHEAD.saturating_sub(self.queue.len())).clamp(1, 2) as u8;
        let req = online::Request::JourneyNext {
            token,
            target_stars: self.target_stars,
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
        if self.target_stars.is_none()
            && self.step == 0
            && self.queue.is_empty()
            && self.needs_level
        {
            return;
        }
        let comfort = self.target_stars.unwrap_or(self.comfort_stars);
        let excluded: HashSet<String> = self.exclude_sets.iter().cloned().collect();
        let mut used_local: HashSet<String> = HashSet::new();

        while self.queue.len() < MAX_PREFETCH_AHEAD {
            let step = self.step.saturating_add(self.queue.len() as u32);
            let (role, reason, target) = if step == 0 {
                ("warmup", "Warm-up · from your library", comfort - 0.4)
            } else if step % 5 == 4 {
                ("stretch", "Stretch · slightly above comfort", comfort + 0.5)
            } else if step % 5 == 2 {
                ("explore", "Change of pace · from your library", comfort)
            } else {
                ("main", "Fits your comfort level", comfort)
            };

            let mut best: Option<(usize, f64, f64)> = None;
            for (i, e) in library.iter().enumerate() {
                let sk = local_set_key(e);
                if excluded.contains(sk.as_str()) || used_local.contains(&sk) {
                    continue;
                }
                let st = stars_of(i).map(f64::from).unwrap_or(comfort);
                let dist = (st - target).abs();
                // Deterministic pseudo-variety using entry index + step.
                let jitter = (((i as u64).wrapping_mul(2654435761) ^ (step as u64 * 97)) % 100)
                    as f64
                    * 0.004;
                let score = dist + jitter;
                if best.is_none_or(|(_, b_score, _)| score < b_score) {
                    best = Some((i, score, st));
                }
            }
            let Some((idx, _, st)) = best else {
                break;
            };
            let e = &library[idx];
            used_local.insert(local_set_key(e));
            let item = JourneyItem {
                md5: format!("local:{idx}"),
                map_hash: None,
                set_id: e.meta.set_id,
                custom_id: None,
                title: e.meta.title.clone(),
                artist: e.meta.artist.clone(),
                creator: e.meta.creator.clone(),
                version: e.meta.version.clone(),
                stars: st,
                bpm: 140.0,
                length_ms: 90_000,
                role: role.into(),
                reason: reason.into(),
            };
            self.remember_item(&item);
            self.queue.push_back(QueuedMap {
                item,
                source: MapSource::Local(idx),
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

/// Searches the player's local library for a matching map (by set_id + version).
pub fn find_in_library(item: &JourneyItem, library: &[Entry]) -> Option<usize> {
    if let Some(set_id) = item.set_id
        && let Some(i) = library
            .iter()
            .position(|e| e.meta.set_id == Some(set_id) && e.meta.version == item.version)
    {
        return Some(i);
    }
    library.iter().position(|e| {
        e.meta.title.eq_ignore_ascii_case(&item.title)
            && e.meta.artist.eq_ignore_ascii_case(&item.artist)
            && e.meta.version.eq_ignore_ascii_case(&item.version)
    })
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

fn start_item_download(item: &JourneyItem) -> MapSource {
    let Some(info) = item_to_set_info(item) else {
        return MapSource::Failed("Map has no downloadable set ID".into());
    };
    let cache = config::journey_cache_dir();
    let _ = std::fs::create_dir_all(&cache);
    prune_cache(&cache, MAX_CACHE_FOLDERS);
    MapSource::Downloading(Box::new(download::start(info, cache)))
}

pub fn scan_single_folder(folder: &Path) -> Vec<Entry> {
    let found = AtomicUsize::new(0);
    crate::library::scan(&[folder.to_path_buf()], &found, &mut |_| {})
}

fn load_entry_from_folder(folder: &Path, item: &JourneyItem) -> Option<Entry> {
    let entries = scan_single_folder(folder);
    let idx = entries
        .iter()
        .position(|e| e.meta.version.eq_ignore_ascii_case(&item.version))
        .unwrap_or(0);
    entries.into_iter().nth(idx)
}

/// Moves a cached mapset folder into the player's permanent library folder:
/// - custom maps -> `config::custom_dir()`
/// - osu! maps -> `config::own_songs_dir()` when `save_to == JourneySaveTo::Own` (default),
///   or `osu_songs` when `save_to == JourneySaveTo::Osu`.
pub fn save_cached_folder(
    cached_folder: &Path,
    is_custom: bool,
    save_to: JourneySaveTo,
    osu_songs: &Path,
) -> Result<PathBuf, String> {
    let dest_root = if is_custom {
        config::custom_dir()
    } else {
        match save_to {
            JourneySaveTo::Own => config::own_songs_dir(),
            JourneySaveTo::Osu => osu_songs.to_path_buf(),
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
    // Cross-device fallback: copy directory recursively then remove cache copy.
    copy_dir_all(cached_folder, &dest).map_err(|e| format!("copy to {}: {e}", dest.display()))?;
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

/// Keeps at most `max_keep` most recently modified folders in `JourneyCache`.
pub fn prune_cache(cache_dir: &Path, max_keep: usize) {
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
        let _ = std::fs::remove_dir_all(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_cached_folder_defaults_to_own_dir_and_moves_files() {
        let base = std::env::temp_dir().join(format!("taiko-journey-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let cache = base.join("cache/123 Artist - Title");
        let osu = base.join("osu");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("map.osu"), b"test").unwrap();

        let saved = save_cached_folder(&cache, false, JourneySaveTo::Osu, &osu).unwrap();
        assert_eq!(saved, osu.join("123 Artist - Title"));
        assert!(saved.join("map.osu").is_file());
        assert!(!cache.exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn prune_cache_keeps_newest_folders() {
        let base = std::env::temp_dir().join(format!("taiko-journey-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        for i in 0..5 {
            std::fs::create_dir_all(base.join(format!("set-{i}"))).unwrap();
        }
        prune_cache(&base, 2);
        let left = std::fs::read_dir(&base).unwrap().count();
        assert_eq!(left, 2);
        let _ = std::fs::remove_dir_all(&base);
    }
}
