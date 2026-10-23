//! Finds beatmaps: Songs folders, single map folders, `.osz` archives and `.osu` files.

use std::path::{Path, PathBuf};

use crate::beatmap::{self, Meta};

pub struct Entry {
    pub path: PathBuf,
    pub meta: Meta,
    pub objects: usize,
    /// Content hash of the `.osu`, for matching maps between multiplayer clients.
    pub hash: u64,
}

impl Entry {
    pub fn label(&self) -> String {
        format!("{} - {} [{}]", self.meta.artist, self.meta.title, self.meta.version)
    }
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

pub fn scan(inputs: &[PathBuf]) -> Vec<Entry> {
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
    let mut entries: Vec<Entry> = files
        .into_iter()
        .filter_map(|path| {
            let bytes = std::fs::read(&path).ok()?;
            let (meta, objects) = beatmap::read_meta(&String::from_utf8_lossy(&bytes))?;
            let hash = crate::net::map_hash(&bytes);
            meta.is_playable().then_some(Entry { path, meta, objects, hash })
        })
        .collect();
    entries.sort_by(|a, b| {
        let key = |e: &Entry| (e.meta.artist.to_lowercase(), e.meta.title.to_lowercase());
        key(a).cmp(&key(b)).then(a.objects.cmp(&b.objects))
    });
    entries
}
