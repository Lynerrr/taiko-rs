//! Finds beatmaps: Songs folders, single map folders, `.osz` archives and `.osu` files.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use crate::beatmap::{self, Meta};

pub struct Entry {
    pub path: PathBuf,
    pub meta: Meta,
    /// File size: orders difficulties within a set (more objects = bigger file) without parsing them.
    pub size: u64,
    /// Last modified (Unix seconds): "date added" for sorting.
    pub modified: u64,
    /// Content hash of the `.osu` for matching maps between multiplayer clients. Computed on first
    /// use: hashing every file up front would make startup with a big library crawl.
    hash: OnceLock<u64>,
}

impl Entry {
    pub fn new(path: PathBuf, meta: Meta) -> Entry {
        let (size, modified) = file_stats(&path);
        Entry { path, meta, size, modified, hash: OnceLock::new() }
    }

    pub fn label(&self) -> String {
        format!("{} - {} [{}]", self.meta.artist, self.meta.title, self.meta.version)
    }

    pub fn hash(&self) -> u64 {
        *self.hash.get_or_init(|| crate::net::map_hash(&std::fs::read(&self.path).unwrap_or_default()))
    }
}

/// (size, modified as Unix seconds) of a file.
pub fn file_stats(path: &Path) -> (u64, u64) {
    let Ok(md) = std::fs::metadata(path) else { return (0, 0) };
    let modified = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    (md.len(), modified)
}

/// Default osu! stable install locations that have a Songs folder.
pub fn default_song_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut candidates = Vec::new();
    #[cfg(windows)]
    {
        // osu! stable installs per-user here by default.
        if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            candidates.push(local.join("osu!").join("Songs"));
        }
        candidates.push(PathBuf::from(r"C:\osu!\Songs"));
        if let Some(cfg) = crate::config::user_dir(false) {
            candidates.push(cfg.join("Songs"));
        }
    }
    #[cfg(not(windows))]
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        // osu-winello records the install path here.
        if let Ok(p) = std::fs::read_to_string(home.join(".local/share/osuconfig/osupath")) {
            candidates.push(PathBuf::from(p.trim()).join("Songs"));
        }
        candidates.push(home.join(".local/share/osu-wine/osu!/Songs"));
        candidates.push(home.join("osu!/Songs"));
        candidates.push(home.join(".local/share/taiko-rs/Songs"));
    }
    for c in candidates {
        if c.is_dir() && !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

fn cache_dir() -> PathBuf {
    crate::config::user_dir(true).unwrap_or_else(|| std::env::temp_dir().join("taiko-rs"))
}

/// Extracts an `.osz` once into the cache and returns the folder.
fn extract_osz(path: &Path) -> Result<PathBuf, String> {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("map");
    let dest = cache_dir().join("osz").join(stem);
    if dest.is_dir() {
        return Ok(dest);
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let tmp = dest.with_extension("partial");
    let _ = std::fs::remove_dir_all(&tmp);
    // `extract` rejects entries that would escape the destination (zip-slip).
    zip.extract(&tmp).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

fn collect_osu(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                collect_osu(&p, depth - 1, out);
            }
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("osu")) {
            out.push(p);
        }
    }
}

/// Finds playable difficulties under `inputs`. `found` counts files read so far, for the UI.
pub fn scan(inputs: &[PathBuf], found: &AtomicUsize) -> Vec<Entry> {
    let mut files = Vec::new();
    for input in inputs {
        let ext = input.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("osu") if input.is_file() => files.push(input.clone()),
            Some("osz") if input.is_file() => match extract_osz(input) {
                Ok(dir) => collect_osu(&dir, 1, &mut files),
                Err(e) => eprintln!("{}: {e}", input.display()),
            },
            _ if input.is_dir() => {
                // A Songs folder: also pick up .osz files dropped into it.
                if let Ok(rd) = std::fs::read_dir(input) {
                    for e in rd.flatten() {
                        let p = e.path();
                        if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("osz")) {
                            if let Ok(dir) = extract_osz(&p) {
                                collect_osu(&dir, 1, &mut files);
                            }
                        }
                    }
                }
                collect_osu(input, 2, &mut files)
            }
            _ => eprintln!("skipping {}: not a .osu, .osz or folder", input.display()),
        }
    }
    files.sort();
    files.dedup();

    let read = |path: &PathBuf| -> Option<Entry> {
        // One malformed map must not take the whole scan down with it.
        let meta = std::panic::catch_unwind(|| beatmap::read_meta_file(path)).ok().flatten();
        found.fetch_add(1, Ordering::Relaxed);
        let meta = meta?;
        let (size, modified) = file_stats(path);
        meta.is_playable().then(|| Entry { path: path.clone(), meta, size, modified, hash: OnceLock::new() })
    };
    // Reading headers is I/O-bound: spread it over a thread per core.
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 16);
    let chunk = files.len().div_ceil(threads).max(1);
    let mut entries: Vec<Entry> = std::thread::scope(|s| {
        let handles: Vec<_> = files.chunks(chunk).map(|part| s.spawn(move || part.iter().filter_map(read).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    entries.sort_by(|a, b| {
        // Same-named songs from different folders stay separate sets.
        let key = |e: &Entry| (e.meta.artist.to_lowercase(), e.meta.title.to_lowercase(), e.path.parent().map(|p| p.to_path_buf()));
        key(a).cmp(&key(b)).then(a.size.cmp(&b.size))
    });
    entries
}
