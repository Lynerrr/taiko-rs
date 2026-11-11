//! Profile pictures from the website (`/a/<name>`), shrunk to a square cell of the renderer's
//! avatar texture. A picture is looked up on a background thread the first time it's drawn
//! (`Canvas::avatar` asks); until then, and for players without one, their initial shows.
//!
//! Cells are kept on disk (`DIR` in the cache folder), so a picture seen before shows at once,
//! after a restart, after it lost its slot in the texture, and offline. The website is asked
//! again only once the copy is older than `REFRESH`, with the old picture up meanwhile. An empty
//! file there: they had none.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime};

/// A cell's side (pixels), and how many cells to a side of the texture.
pub const CELL: u32 = 64;
pub const GRID: u32 = 8;
const SLOTS: usize = (GRID * GRID) as usize;
const CELL_BYTES: usize = (CELL * CELL * 4) as usize;
/// The folder in the cache the cells are kept in (named for `CELL`: another size starts afresh).
pub const DIR: &str = "avatars-64";
/// Fetches at once.
const IN_FLIGHT: usize = 4;
/// A picture is asked for again this long after it was last had from the website (it may have
/// changed).
const REFRESH: Duration = Duration::from_secs(60 * 60);
/// A player without one is asked about again after this; a failed fetch after less.
const RETRY_MISSING: Duration = Duration::from_secs(10 * 60);
const RETRY_FAILED: Duration = Duration::from_secs(60);
/// The largest picture read (the server takes up to 2 MB).
const MAX_BYTES: u64 = 3 << 20;
/// Cells on disk not had from the website for this long are deleted.
const PRUNE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

enum Entry {
    /// Being looked up for the first time (or again, after it was missing).
    Fetching,
    /// In its slot (`slots`), as the website had it at `at`.
    Shown { at: Instant },
    /// None on the server, or it couldn't be had: asked again after `retry`.
    Missing { at: Instant, retry: Duration },
}

/// What a lookup found.
enum Found {
    Picture(Vec<u8>),
    None,
    Failed,
}

struct Fetched {
    name: String,
    found: Found,
    /// How long ago the website had it (a copy from disk), so it's asked about again in time.
    age: Duration,
    /// The lookup is over: a stale copy from disk comes first, the website's answer after.
    last: bool,
}

pub struct Avatars {
    /// Where the pictures are (`online::WEBSITE`); None: nowhere (another server, tests).
    base: Option<String>,
    /// Where cells are kept between runs (`DIR`); None: nowhere.
    dir: Option<PathBuf>,
    entries: HashMap<String, Entry>,
    /// What the renderer has, by name: their slots (handed to the canvas each frame).
    slots: Arc<HashMap<String, u32>>,
    /// Each slot's name and when it was last drawn.
    owners: Vec<Option<(String, Instant)>>,
    fetching: usize,
    tx: Sender<Fetched>,
    rx: Receiver<Fetched>,
    /// Pictures for the renderer: slot, `CELL` square premultiplied RGBA.
    uploads: Vec<(u32, Vec<u8>)>,
}

impl Avatars {
    pub fn new(base: Option<String>, dir: Option<PathBuf>) -> Self {
        if let Some(dir) = dir.clone() {
            std::thread::spawn(move || prune(&dir, SystemTime::now()));
        }
        let (tx, rx) = mpsc::channel();
        Avatars {
            base,
            dir,
            entries: HashMap::new(),
            slots: Arc::default(),
            owners: vec![None; SLOTS],
            fetching: 0,
            tx,
            rx,
            uploads: Vec::new(),
        }
    }

    /// The pictures the renderer has, for the canvas.
    pub fn slots(&self) -> Arc<HashMap<String, u32>> {
        self.slots.clone()
    }

    /// Pictures to put in the renderer's texture.
    pub fn take_uploads(&mut self) -> Vec<(u32, Vec<u8>)> {
        std::mem::take(&mut self.uploads)
    }

    /// After a frame: takes in finished lookups and starts the ones `wanted` (lowercased names
    /// drawn) needs.
    pub fn update(&mut self, wanted: &[String]) {
        let now = Instant::now();
        while let Ok(f) = self.rx.try_recv() {
            if f.last {
                self.fetching -= 1;
            }
            let at = now.checked_sub(f.age).unwrap_or(now);
            match f.found {
                Found::Picture(rgba) => self.store(f.name, rgba, now, at),
                Found::None => self.forget(f.name, at, RETRY_MISSING),
                Found::Failed => self.failed(f.name, now),
            }
        }
        for name in wanted {
            if let Some(&slot) = self.slots.get(name) {
                self.owners[slot as usize] = Some((name.clone(), now));
            }
        }
        let Some(base) = self.base.clone() else {
            return;
        };
        for name in wanted {
            if self.fetching >= IN_FLIGHT {
                break;
            }
            let due = match self.entries.get(name) {
                None => true,
                Some(Entry::Fetching) => false,
                Some(Entry::Shown { at, .. }) => now.duration_since(*at) >= REFRESH,
                Some(Entry::Missing { at, retry }) => now.duration_since(*at) >= *retry,
            };
            if !due || !valid_name(name) {
                continue;
            }
            // A picture being looked at again stays up meanwhile.
            if !matches!(self.entries.get(name), Some(Entry::Shown { .. })) {
                self.entries.insert(name.clone(), Entry::Fetching);
            } else if let Some(Entry::Shown { at, .. }) = self.entries.get_mut(name) {
                *at = now;
            }
            self.fetching += 1;
            let tx = self.tx.clone();
            let (name, url) = (name.clone(), format!("{base}/a/{name}"));
            let path = self.dir.as_ref().map(|d| d.join(&name));
            std::thread::spawn(move || lookup(name, path.as_deref(), || fetch(&url), &tx));
        }
    }

    /// `name`'s picture goes up (drawn `now`, had from the website `at`).
    fn store(&mut self, name: String, rgba: Vec<u8>, now: Instant, at: Instant) {
        let slot = match self.slots.get(&name) {
            Some(&slot) => slot,
            None => {
                // A free slot, else the one drawn longest ago.
                let slot = self
                    .owners
                    .iter()
                    .position(Option::is_none)
                    .unwrap_or_else(|| {
                        self.owners
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, o)| o.as_ref().map(|(_, at)| *at))
                            .map_or(0, |(i, _)| i)
                    }) as u32;
                if let Some((old, _)) = self.owners[slot as usize].take() {
                    Arc::make_mut(&mut self.slots).remove(&old);
                    // Looked up again (on disk) when next drawn.
                    self.entries.remove(&old);
                }
                slot
            }
        };
        self.owners[slot as usize] = Some((name.clone(), now));
        Arc::make_mut(&mut self.slots).insert(name.clone(), slot);
        self.entries.insert(name, Entry::Shown { at });
        self.uploads.push((slot, rgba));
    }

    fn forget(&mut self, name: String, now: Instant, retry: Duration) {
        // A picture that was there and is gone (taken off) leaves its slot.
        if let Some(slot) = Arc::make_mut(&mut self.slots).remove(&name) {
            self.owners[slot as usize] = None;
        }
        let _ = self.entries.insert(name, Entry::Missing { at: now, retry });
    }

    /// The website couldn't be asked: a picture that's up stays, and is asked about again soon.
    fn failed(&mut self, name: String, now: Instant) {
        match self.entries.get_mut(&name) {
            Some(Entry::Shown { at }) => {
                *at = now.checked_sub(REFRESH - RETRY_FAILED).unwrap_or(now)
            }
            _ => self.forget(name, now, RETRY_FAILED),
        }
    }

    #[cfg(test)]
    fn slot_of(&self, name: &str) -> Option<u32> {
        self.slots.get(name).copied()
    }
}

/// Names as accounts have them (so nothing odd goes into a URL or a file name).
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Looks `name` up: the copy on disk at `path` while it's fresh, else the website (`fetch`),
/// keeping what it says. A stale copy is sent first, so it's up while the website is asked.
fn lookup(
    name: String,
    path: Option<&Path>,
    fetch: impl FnOnce() -> Result<Option<Vec<u8>>, String>,
    tx: &Sender<Fetched>,
) {
    let send = |found, age, last| {
        let name = name.clone();
        let _ = tx.send(Fetched {
            name,
            found,
            age,
            last,
        });
    };
    if let Some((cached, age)) = path.and_then(read_cached) {
        let keep = if cached.is_some() {
            REFRESH
        } else {
            RETRY_MISSING
        };
        let fresh = age < keep;
        match cached {
            Some(px) if fresh => return send(Found::Picture(px), age, true),
            None if fresh => return send(Found::None, age, true),
            Some(px) => send(Found::Picture(px), Duration::ZERO, false),
            None => {}
        }
    }
    let found = match fetch() {
        Ok(Some(px)) => {
            if let Some(path) = path {
                write_cached(path, &px);
            }
            Found::Picture(px)
        }
        Ok(None) => {
            if let Some(path) = path {
                write_cached(path, &[]);
            }
            Found::None
        }
        Err(_) => Found::Failed,
    };
    send(found, Duration::ZERO, true);
}

/// The copy at `path` (a cell, or None: they had none) and how long ago the website had it.
/// A time in the future (a clock set back) counts as long ago.
fn read_cached(path: &Path) -> Option<(Option<Vec<u8>>, Duration)> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::MAX);
    let bytes = std::fs::read(path).ok()?;
    match bytes.len() {
        0 => Some((None, age)),
        CELL_BYTES => Some((Some(bytes), age)),
        _ => None,
    }
}

/// Keeps `px` (empty: none) at `path`, whole or not at all.
fn write_cached(path: &Path, px: &[u8]) {
    let tmp = path.with_extension("tmp");
    let ok = path
        .parent()
        .is_some_and(|d| std::fs::create_dir_all(d).is_ok())
        && std::fs::write(&tmp, px).is_ok()
        && std::fs::rename(&tmp, path).is_ok();
    if !ok {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Deletes the cells in `dir` the website hasn't been asked about for `PRUNE` (players not seen
/// in a long while), and anything half written.
fn prune(dir: &Path, now: SystemTime) {
    let Ok(files) = std::fs::read_dir(dir) else {
        return;
    };
    for file in files.flatten() {
        let path = file.path();
        let old = file
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|m| now.duration_since(m).is_ok_and(|age| age >= PRUNE));
        if old || path.extension().is_some_and(|e| e == "tmp") {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The picture at `url` as a cell (None: the player has none).
fn fetch(url: &str) -> Result<Option<Vec<u8>>, String> {
    crate::offline::url(url)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into();
    let mut response = match agent
        .get(url)
        .header(
            "User-Agent",
            concat!("taiko-rs/", env!("CARGO_PKG_VERSION")),
        )
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    cell(&bytes).map(Some)
}

/// `bytes` (an image file) as a `CELL` square: the middle of it, premultiplied RGBA.
pub fn cell(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let image = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let mut px = image
        .resize_to_fill(CELL, CELL, image::imageops::FilterType::Triangle)
        .to_rgba8()
        .into_raw();
    for p in px.as_chunks_mut::<4>().0 {
        let a = u16::from(p[3]);
        for v in &mut p[..3] {
            *v = ((u16::from(*v) * a + 127) / 255) as u8;
        }
    }
    Ok(px)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn temp_dir(what: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("taiko-rs-avatars-{what}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// What a lookup sends, in order.
    fn looked_up(
        path: &Path,
        fetch: impl FnOnce() -> Result<Option<Vec<u8>>, String>,
    ) -> Vec<Fetched> {
        let (tx, rx) = mpsc::channel();
        lookup("p".into(), Some(path), fetch, &tx);
        drop(tx);
        rx.iter().collect()
    }

    fn set_age(path: &Path, age: Duration) {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    #[test]
    fn a_picture_becomes_a_premultiplied_square() {
        let px = cell(&png(300, 120, [200, 100, 50, 128])).unwrap();
        assert_eq!(px.len(), CELL_BYTES);
        assert_eq!(&px[..4], &[100, 50, 25, 128]);
        assert!(cell(b"not an image").is_err());
    }

    #[test]
    fn slots_go_to_the_pictures_drawn_last() {
        let mut a = Avatars::new(None, None);
        let blank = vec![0u8; CELL_BYTES];
        // In the past, so that being drawn now is the latest.
        let t0 = Instant::now() - Duration::from_secs(1);
        for i in 0..SLOTS {
            let at = t0 + Duration::from_millis(i as u64);
            a.store(format!("p{i}"), blank.clone(), at, at);
        }
        assert!(a.owners.iter().all(Option::is_some));
        // p0 is drawn again, so p1 is the one drawn longest ago.
        a.update(&["p0".to_string()]);
        let now = Instant::now();
        a.store("new".into(), blank.clone(), now, now);
        assert_eq!(a.slot_of("new"), Some(1));
        assert!(!a.slots().contains_key("p1") && a.slots().contains_key("p0"));
        assert_eq!(a.take_uploads().len(), SLOTS + 1);
        // Taken off the website: its slot is free again.
        a.forget("new".into(), Instant::now(), RETRY_MISSING);
        assert!(a.owners[1].is_none() && !a.slots().contains_key("new"));
    }

    #[test]
    fn only_account_names_are_fetched() {
        assert!(valid_name("auneu") && valid_name("a_b-c9"));
        assert!(!valid_name("") && !valid_name("../x") && !valid_name("a b") && !valid_name("x?y"));
    }

    #[test]
    fn a_picture_had_once_comes_from_disk_after() {
        let dir = temp_dir("disk");
        let path = dir.join("p");
        let px = vec![7u8; CELL_BYTES];
        // Nothing on disk: the website, and it's kept.
        let got = looked_up(&path, || Ok(Some(px.clone())));
        assert!(
            matches!(&got[..], [Fetched { found: Found::Picture(p), last: true, .. }] if *p == px)
        );
        assert_eq!(std::fs::read(&path).unwrap(), px);
        // Fresh on disk: the website isn't asked.
        let got = looked_up(&path, || panic!("asked the website"));
        assert!(
            matches!(&got[..], [Fetched { found: Found::Picture(p), last: true, .. }] if *p == px)
        );
        // Stale: the copy goes up, then the website's answer, which is kept.
        set_age(&path, REFRESH + Duration::from_secs(5));
        let newer = vec![9u8; CELL_BYTES];
        let got = looked_up(&path, || Ok(Some(newer.clone())));
        assert!(matches!(
            &got[..],
            [
                Fetched { found: Found::Picture(old), last: false, .. },
                Fetched { found: Found::Picture(new), last: true, .. },
            ] if *old == px && *new == newer
        ));
        assert_eq!(std::fs::read(&path).unwrap(), newer);
        // Taken off the website: an empty file, which counts as none while fresh.
        set_age(&path, REFRESH + Duration::from_secs(5));
        looked_up(&path, || Ok(None));
        assert_eq!(std::fs::read(&path).unwrap(), Vec::<u8>::new());
        let got = looked_up(&path, || panic!("asked the website"));
        assert!(matches!(
            &got[..],
            [Fetched {
                found: Found::None,
                last: true,
                ..
            }]
        ));
        // Something else in the file: as if there were nothing.
        std::fs::write(&path, b"junk").unwrap();
        let got = looked_up(&path, || Err("offline".into()));
        assert!(matches!(
            &got[..],
            [Fetched {
                found: Found::Failed,
                last: true,
                ..
            }]
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_picture_stays_up_when_the_website_cant_be_reached() {
        let mut a = Avatars::new(None, None);
        a.fetching = 1;
        let send = |found, last| {
            a.tx.send(Fetched {
                name: "p".into(),
                found,
                age: Duration::ZERO,
                last,
            })
            .unwrap()
        };
        send(Found::Picture(vec![0u8; CELL_BYTES]), false);
        send(Found::Failed, true);
        a.update(&[]);
        assert_eq!(a.fetching, 0);
        assert!(a.slot_of("p").is_some());
        // And it's asked about again soon, not after a whole `REFRESH`.
        let Some(Entry::Shown { at }) = a.entries.get("p") else {
            panic!("not shown");
        };
        assert!(at.elapsed() + RETRY_FAILED + Duration::from_secs(1) >= REFRESH);
    }

    #[test]
    fn old_and_half_written_cells_are_pruned() {
        let dir = temp_dir("prune");
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["old", "new", "half.tmp"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        set_age(&dir.join("old"), PRUNE + Duration::from_secs(60));
        prune(&dir, SystemTime::now());
        assert!(!dir.join("old").exists() && !dir.join("half.tmp").exists());
        assert!(dir.join("new").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
