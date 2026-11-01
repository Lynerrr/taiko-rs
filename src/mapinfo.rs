//! Per-map numbers for sorting the whole library (taiko star rating at 1.0x, length, BPM),
//! computed in the background with rosu-pp and cached on disk, so a big library only pays once.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rosu_pp::taiko::Taiko;
use rosu_pp::{Beatmap, Difficulty};

#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub stars: f32,
    pub length_ms: f32,
    pub bpm: f32,
}

/// Results by library index (None: not known yet or the map couldn't be read).
pub type Shared = Arc<Mutex<Vec<Option<Info>>>>;

pub struct MapInfo {
    pub results: Shared,
    /// Maps handled so far (cached or computed) out of `total`.
    pub done: Arc<AtomicUsize>,
    pub total: Arc<AtomicUsize>,
    /// The work waits while this is on: a song is being played.
    held: Arc<AtomicBool>,
    /// When it began.
    pub started: Instant,
}

/// How long the work has to take before it's worth showing: a library already in the cache is
/// through it in a moment.
const SHOW_AFTER: Duration = Duration::from_secs(2);

/// How often the cache is written while working (maps): a big library can take longer than a
/// session, and what's done shouldn't have to be done again.
const SAVE_EVERY: usize = 2000;

/// How many threads to work on: half the machine at most, so a first start with a big library
/// leaves it usable, however many maps there are.
fn thread_count(cores: usize) -> usize {
    (cores / 2).clamp(1, 4)
}

fn cache_path() -> Option<PathBuf> {
    Some(crate::config::cache_dir()?.join("mapinfo-v1.tsv"))
}

/// (mtime, size, info) per path, as last saved.
type Cache = HashMap<PathBuf, (u64, u64, Info)>;

fn load_cache() -> Cache {
    let Some(text) = cache_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return HashMap::new() };
    text.lines()
        .filter_map(|line| {
            let mut f = line.split('\t');
            let path = PathBuf::from(f.next()?);
            let mut num = || f.next()?.parse::<f64>().ok();
            let (mtime, size, stars, length_ms, bpm) = (num()? as u64, num()? as u64, num()? as f32, num()? as f32, num()? as f32);
            Some((path, (mtime, size, Info { stars, length_ms, bpm })))
        })
        .collect()
}

fn save_cache(cache: &Cache) {
    let Some(path) = cache_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    let Ok(mut file) = std::fs::File::create(&tmp) else { return };
    let mut out = String::with_capacity(cache.len() * 96);
    for (p, (mtime, size, i)) in cache {
        // Tabs and newlines can't appear in paths we care about; skip any that do.
        let s = p.to_string_lossy();
        if !s.contains(['\t', '\n']) {
            out.push_str(&format!("{s}\t{mtime}\t{size}\t{:.4}\t{:.0}\t{:.2}\n", i.stars, i.length_ms, i.bpm));
        }
    }
    if file.write_all(out.as_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Star rating (taiko, converts included), drain length and BPM of one map.
fn analyze(path: &Path) -> Option<Info> {
    let map = Beatmap::from_path(path).ok()?;
    map.check_suspicion().ok()?;
    let stars = Difficulty::new().calculate_for_mode::<Taiko>(&map).ok()?.stars as f32;
    let first = map.hit_objects.first().map(|h| h.start_time).unwrap_or(0.0);
    let last = map.hit_objects.last().map(|h| h.start_time).unwrap_or(0.0);
    Some(Info { stars, length_ms: (last - first).max(0.0) as f32, bpm: map.bpm() as f32 })
}

impl MapInfo {
    /// Starts working through `maps` (library index, path, mtime, size) on background threads,
    /// waiting first if `held`.
    pub fn spawn(maps: Vec<(usize, PathBuf, u64, u64)>, len: usize, held: bool) -> MapInfo {
        let results: Shared = Arc::new(Mutex::new(vec![None; len]));
        let done = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(maps.len()));
        let held = Arc::new(AtomicBool::new(held));
        let info = MapInfo { results: results.clone(), done: done.clone(), total, held: held.clone(), started: Instant::now() };
        std::thread::Builder::new()
            .name("mapinfo".into())
            .spawn(move || run(maps, results, done, held))
            .expect("spawn map info");
        info
    }

    /// Adds maps appended to the library later (downloads).
    pub fn extend(&self, maps: Vec<(usize, PathBuf, u64, u64)>) {
        if let Some(max) = maps.iter().map(|m| m.0 + 1).max() {
            let mut r = self.results.lock().unwrap();
            if r.len() < max {
                r.resize(max, None);
            }
        }
        self.total.fetch_add(maps.len(), Ordering::Relaxed);
        let (results, done, held) = (self.results.clone(), self.done.clone(), self.held.clone());
        std::thread::spawn(move || run(maps, results, done, held));
    }

    /// Stops the work for now, or lets it go on.
    pub fn hold(&self, on: bool) {
        self.held.store(on, Ordering::Relaxed);
    }

    /// How far it's got (done, total), while it's worth showing.
    pub fn progress(&self, now: Instant) -> Option<(usize, usize)> {
        let shown = !self.finished() && now.saturating_duration_since(self.started) >= SHOW_AFTER;
        shown.then(|| (self.done.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed)))
    }

    pub fn finished(&self) -> bool {
        self.done.load(Ordering::Relaxed) >= self.total.load(Ordering::Relaxed)
    }
}

fn run(maps: Vec<(usize, PathBuf, u64, u64)>, results: Shared, done: Arc<AtomicUsize>, held: Arc<AtomicBool>) {
    let cache = load_cache();
    let mut todo = Vec::new();
    {
        let mut r = results.lock().unwrap();
        for (i, path, mtime, size) in maps {
            match cache.get(&path) {
                Some(&(m, s, info)) if m == mtime && s == size => {
                    if let Some(slot) = r.get_mut(i) {
                        *slot = Some(info);
                    }
                    done.fetch_add(1, Ordering::Relaxed);
                }
                _ => todo.push((i, path, mtime, size)),
            }
        }
    }
    if todo.is_empty() {
        return;
    }
    let threads = thread_count(std::thread::available_parallelism().map_or(2, |n| n.get()));
    let next = AtomicUsize::new(0);
    let unsaved = AtomicUsize::new(0);
    let cache = Mutex::new(cache);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                while held.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(100));
                }
                let k = next.fetch_add(1, Ordering::Relaxed);
                let Some((i, path, mtime, size)) = todo.get(k) else { break };
                let info = std::panic::catch_unwind(|| analyze(path)).ok().flatten();
                if let Some(info) = info {
                    if let Some(slot) = results.lock().unwrap().get_mut(*i) {
                        *slot = Some(info);
                    }
                    let mut cache = cache.lock().unwrap();
                    cache.insert(path.clone(), (*mtime, *size, info));
                    if unsaved.fetch_add(1, Ordering::Relaxed) + 1 >= SAVE_EVERY {
                        unsaved.store(0, Ordering::Relaxed);
                        save_cache(&cache);
                    }
                }
                done.fetch_add(1, Ordering::Relaxed);
            });
        }
    });
    save_cache(&cache.lock().unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maps(n: usize) -> Vec<(usize, PathBuf, u64, u64)> {
        (0..n).map(|i| (i, PathBuf::from(format!("/nonexistent/{i}.osu")), 0, 0)).collect()
    }

    fn wait_for(info: &MapInfo) -> bool {
        (0..300).any(|_| {
            std::thread::sleep(Duration::from_millis(10));
            info.finished()
        })
    }

    #[test]
    fn it_waits_while_held_and_goes_on_after() {
        let info = MapInfo::spawn(maps(5), 5, true);
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(info.done.load(Ordering::Relaxed), 0, "nothing while a song plays");
        info.hold(false);
        assert!(wait_for(&info), "{} done", info.done.load(Ordering::Relaxed));
    }

    #[test]
    fn progress_shows_only_when_it_takes_a_while_and_until_it_ends() {
        let info = MapInfo::spawn(maps(3), 3, true);
        assert_eq!(info.progress(info.started), None, "a moment in: maybe it's all cached");
        assert_eq!(info.progress(info.started + SHOW_AFTER), Some((0, 3)));
        info.hold(false);
        assert!(wait_for(&info));
        assert_eq!(info.progress(info.started + SHOW_AFTER), None, "done");
    }

    #[test]
    fn it_leaves_half_the_machine_alone() {
        assert_eq!(thread_count(1), 1);
        assert_eq!(thread_count(2), 1);
        assert_eq!(thread_count(4), 2);
        assert_eq!(thread_count(8), 4);
        assert_eq!(thread_count(32), 4);
    }
}
