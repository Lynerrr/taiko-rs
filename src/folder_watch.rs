//! Watches one map folder for `.osu` and `.tko` files that appear, change or go, so a file dropped
//! into the folder of the map being edited joins the library without a restart.
//!
//! It polls the folder listing (a set folder is flat and small) instead of using file system
//! events, which behave differently on every platform. A file is reported once its size and
//! modified time held still for two polls in a row: one still being copied in isn't read half
//! done.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How often the folder is looked at.
const EVERY: Duration = Duration::from_millis(1000);

/// Size and modified time (ns since 1970) of a file.
type Sig = (u64, u64);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// Map files that are new or were rewritten, and have settled.
    pub changed: Vec<PathBuf>,
    /// Map files that are no longer there.
    pub gone: Vec<PathBuf>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.gone.is_empty()
    }
}

pub struct Watch {
    folder: PathBuf,
    /// What was last reported (or there at the start).
    known: HashMap<PathBuf, Sig>,
    /// What changed at the last look, waiting to hold still.
    waiting: HashMap<PathBuf, Sig>,
    next: Instant,
}

impl Watch {
    /// Starts watching `folder`; the maps in it now are the ones already known.
    pub fn new(folder: &Path, now: Instant) -> Watch {
        Watch {
            folder: folder.to_path_buf(),
            known: listing(folder),
            waiting: HashMap::new(),
            next: now + EVERY,
        }
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// What changed since the last time, looked at no more often than once a second.
    pub fn poll(&mut self, now: Instant) -> Option<Changes> {
        if now < self.next {
            return None;
        }
        self.next = now + EVERY;
        Some(self.look()).filter(|c| !c.is_empty())
    }

    fn look(&mut self) -> Changes {
        let now = listing(&self.folder);
        let mut out = Changes::default();
        for (path, sig) in &now {
            if self.known.get(path) == Some(sig) {
                self.waiting.remove(path);
            } else if self.waiting.get(path) == Some(sig) {
                self.waiting.remove(path);
                self.known.insert(path.clone(), *sig);
                out.changed.push(path.clone());
            } else {
                self.waiting.insert(path.clone(), *sig);
            }
        }
        self.waiting.retain(|p, _| now.contains_key(p));
        let gone: Vec<PathBuf> = self
            .known
            .keys()
            .filter(|p| !now.contains_key(*p))
            .cloned()
            .collect();
        for path in gone {
            self.known.remove(&path);
            out.gone.push(path);
        }
        out.changed.sort();
        out.gone.sort();
        out
    }
}

/// The map files directly in `folder` with their size and modified time.
fn listing(folder: &Path) -> HashMap<PathBuf, Sig> {
    let Ok(rd) = std::fs::read_dir(folder) else {
        return HashMap::new();
    };
    rd.flatten()
        .filter_map(|e| {
            let path = e.path();
            if !crate::library::is_map_file(&path) {
                return None;
            }
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            let ns = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos() as u64);
            Some((path, (meta.len(), ns)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("taiko-rs-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn maps_already_there_are_not_news() {
        let dir = folder("start");
        std::fs::write(dir.join("a [Oni].osu"), "x").unwrap();
        let mut w = Watch::new(&dir, Instant::now());
        assert_eq!(w.look(), Changes::default());
        assert_eq!(w.look(), Changes::default());
    }

    #[test]
    fn a_new_map_is_reported_once_it_holds_still() {
        let dir = folder("new");
        let mut w = Watch::new(&dir, Instant::now());
        let file = dir.join("b [Inner].tko");
        std::fs::write(&file, "one").unwrap();
        assert!(w.look().is_empty(), "seen once: maybe still being copied");
        // Still growing: starts over.
        std::fs::write(&file, "one two").unwrap();
        assert!(w.look().is_empty());
        let got = w.look();
        assert_eq!(got.changed, std::slice::from_ref(&file));
        assert!(got.gone.is_empty());
        assert!(w.look().is_empty(), "told once");
    }

    #[test]
    fn rewritten_and_removed_maps_are_reported() {
        let dir = folder("edit");
        let file = dir.join("a [Oni].osu");
        std::fs::write(&file, "x").unwrap();
        let mut w = Watch::new(&dir, Instant::now());
        std::fs::write(&file, "longer now").unwrap();
        w.look();
        assert_eq!(w.look().changed, std::slice::from_ref(&file));
        std::fs::remove_file(&file).unwrap();
        assert_eq!(w.look().gone, [file]);
        assert!(w.look().is_empty());
    }

    #[test]
    fn only_map_files_count() {
        let dir = folder("kinds");
        let mut w = Watch::new(&dir, Instant::now());
        for name in ["a.osu.bak", "a.osu.old", "audio.mp3", "bg.png", "notes.txt"] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        std::fs::create_dir(dir.join("sub.osu")).unwrap();
        w.look();
        assert!(w.look().is_empty());
    }

    #[test]
    fn it_looks_at_most_once_a_second() {
        let dir = folder("rate");
        let t = Instant::now();
        let mut w = Watch::new(&dir, t);
        let file = dir.join("a.osu");
        std::fs::write(&file, "x").unwrap();
        assert!(w.poll(t + Duration::from_millis(500)).is_none(), "too soon");
        assert!(
            w.poll(t + Duration::from_millis(1000)).is_none(),
            "first sight"
        );
        let got = w.poll(t + Duration::from_millis(2000)).unwrap();
        assert_eq!(got.changed, [file]);
    }
}
