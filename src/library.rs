//! Finds beatmaps: Songs folders, single map folders, `.osz` archives and `.osu` files.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{OnceLock, RwLock};

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

/// The Songs folder picked in settings (None = look in the usual places).
static SONGS_FOLDER: RwLock<Option<PathBuf>> = RwLock::new(None);

pub fn set_songs_folder(dir: Option<PathBuf>) {
    *SONGS_FOLDER.write().unwrap_or_else(|e| e.into_inner()) = dir;
}

/// The Songs folders to scan: the one picked in settings, else the osu! installs found.
pub fn song_dirs() -> Vec<PathBuf> {
    match SONGS_FOLDER.read().unwrap_or_else(|e| e.into_inner()).clone() {
        Some(dir) => vec![dir],
        None => default_song_dirs(),
    }
}

/// osu!'s own exe beside a Songs folder being used, else where osu! installs by default.
#[cfg(not(target_os = "android"))]
fn osu_exe() -> Option<PathBuf> {
    let beside = song_dirs().into_iter().filter_map(|d| d.parent().map(|p| p.join("osu!.exe")));
    #[cfg(windows)]
    let beside = beside.chain(std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("osu!").join("osu!.exe")));
    beside.into_iter().find(|p| p.is_file())
}

/// Starts osu!stable, on its own (krokodil_koban mode): osu-winello's `osu-wine` on Linux,
/// else its osu!.exe (through Wine off Windows).
pub fn launch_osu() -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        Err("osu!stable doesn't run on Android".into())
    }
    #[cfg(not(target_os = "android"))]
    {
        use std::process::{Command, Stdio};
        let quiet = |mut c: Command| c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(drop);
        #[cfg(not(windows))]
        {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
            for launcher in [home.join(".local/bin/osu-wine"), PathBuf::from("osu-wine")] {
                if quiet(Command::new(&launcher)).is_ok() {
                    return Ok(());
                }
            }
        }
        let exe = osu_exe().ok_or("couldn't find osu!")?;
        #[cfg(windows)]
        let mut c = Command::new(&exe);
        #[cfg(not(windows))]
        let mut c = {
            let mut c = Command::new("wine");
            c.arg(&exe);
            c
        };
        c.current_dir(exe.parent().unwrap_or(Path::new(".")));
        quiet(c).map_err(|e| format!("couldn't start osu!: {e}"))
    }
}

/// Asks for a folder with the system's folder dialog, starting in `start`. Blocks until it
/// closes: Ok(None) when cancelled, Err when there is no dialog to show.
pub fn pick_folder(start: &Path) -> Result<Option<PathBuf>, String> {
    use std::process::Command;
    const TITLE: &str = "Choose your osu! Songs folder";
    #[cfg_attr(target_os = "android", allow(unused_mut))]
    let mut tries: Vec<Command> = Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let start = start.display().to_string().replace('\'', "''");
        let script = format!(
            "[Console]::OutputEncoding = [Text.Encoding]::UTF8; Add-Type -AssemblyName System.Windows.Forms; \
             $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = '{TITLE}'; $d.SelectedPath = '{start}'; \
             $top = New-Object System.Windows.Forms.Form -Property @{{TopMost = $true}}; \
             if ($d.ShowDialog($top) -eq 'OK') {{ [Console]::Out.Write($d.SelectedPath) }}"
        );
        let mut ps = Command::new("powershell");
        // No console window flashing up behind the dialog.
        ps.args(["-NoProfile", "-STA", "-Command", &script]).creation_flags(0x0800_0000);
        tries.push(ps);
    }
    #[cfg(target_os = "macos")]
    {
        let mut osa = Command::new("osascript");
        osa.args(["-e", &format!("POSIX path of (choose folder with prompt \"{TITLE}\")")]);
        tries.push(osa);
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "android")))]
    {
        let mut zenity = Command::new("zenity");
        zenity.args(["--file-selection", "--directory", &format!("--title={TITLE}")]).arg(format!("--filename={}/", start.display()));
        let mut kdialog = Command::new("kdialog");
        kdialog.arg("--getexistingdirectory").arg(start).args(["--title", TITLE]);
        // KDE's own dialog on KDE, GNOME's elsewhere.
        let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_ascii_uppercase().contains("KDE"));
        tries.extend(if kde { [kdialog, zenity] } else { [zenity, kdialog] });
    }
    let _ = start;
    for mut cmd in tries {
        let Ok(out) = cmd.stderr(std::process::Stdio::null()).output() else { continue };
        let path = String::from_utf8_lossy(&out.stdout).trim_end_matches(['\r', '\n']).to_string();
        return Ok((out.status.success() && !path.is_empty()).then(|| PathBuf::from(path)));
    }
    Err("no folder dialog to show".into())
}

/// Default osu! stable install locations that have a Songs folder.
pub fn default_song_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut candidates = Vec::new();
    #[cfg(windows)]
    {
        // osu! stable installs per-user here by default, but can go anywhere: osu:// links and
        // .osz files are registered to wherever it is.
        if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            candidates.push(install_songs(&local.join("osu!")));
        }
        candidates.push(install_songs(Path::new(r"C:\osu!")));
        candidates.extend(registered_osu().map(|dir| install_songs(&dir)));
        candidates.extend(crate::config::own_dir("Songs"));
    }
    #[cfg(target_os = "android")]
    candidates.extend(crate::android::songs_dir());
    #[cfg(not(any(windows, target_os = "android")))]
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        // osu-winello records the install path here.
        if let Ok(p) = std::fs::read_to_string(home.join(".local/share/osuconfig/osupath")) {
            candidates.push(PathBuf::from(p.trim()).join("Songs"));
        }
        candidates.push(home.join(".local/share/osu-wine/osu!/Songs"));
        candidates.push(home.join("osu!/Songs"));
        candidates.extend(crate::config::own_dir("Songs"));
    }
    for c in candidates {
        if c.is_dir() && !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// The Songs folder of an osu! stable install: the one its settings name (`BeatmapDirectory` in
/// osu!.<user>.cfg) if it exists, else `Songs` next to osu!.exe.
pub fn install_songs(install: &Path) -> PathBuf {
    let user_cfg = |p: &Path| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("osu!.") && n.ends_with(".cfg") && n != "osu!.cfg");
    let beatmap_dir = |text: String| text.lines().find_map(|l| Some(l.strip_prefix("BeatmapDirectory")?.trim_start().strip_prefix('=')?.trim().to_string()));
    std::fs::read_dir(install)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| user_cfg(p))
        .find_map(|p| beatmap_dir(std::fs::read_to_string(p).ok()?))
        .filter(|d| !d.is_empty())
        // A relative one is relative to the install; an absolute one replaces it.
        .map(|d| install.join(d))
        .filter(|d| d.is_dir())
        .unwrap_or_else(|| install.join("Songs"))
}

/// What to scan for a folder picked in Settings: the Songs folder of an osu! install, else the
/// folder itself.
pub fn picked_songs(picked: &Path) -> PathBuf {
    let install = picked.join("osu!.exe").is_file() || picked.join("osu!.db").is_file();
    let songs = install_songs(picked);
    if install && songs.is_dir() { songs } else { picked.to_path_buf() }
}

/// The osu! stable install the registry opens osu:// links / .osz files with.
#[cfg(windows)]
fn registered_osu() -> Option<PathBuf> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    [r"HKCR\osu\shell\open\command", r"HKCR\osustable.File.osz\shell\open\command"].iter().find_map(|key| {
        let out = std::process::Command::new("reg").args(["query", key, "/ve"]).creation_flags(CREATE_NO_WINDOW).output().ok()?;
        // (Default)    REG_SZ    "C:\...\osu!.exe" "%1"
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let dir = Path::new(text.split('"').nth(1)?).parent()?.to_path_buf();
        dir.is_dir().then_some(dir)
    })
}

fn cache_dir() -> PathBuf {
    crate::config::cache_dir().unwrap_or_else(|| std::env::temp_dir().join("taiko-rs"))
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

/// Whether a directory entry is a folder (or a link to one). From the listing itself: asking the
/// file system about each of tens of thousands of maps again is slow on a hard drive.
fn is_dir(e: &std::fs::DirEntry) -> bool {
    e.file_type().is_ok_and(|t| t.is_dir() || (t.is_symlink() && e.path().is_dir()))
}

fn collect_osu(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if is_dir(&e) {
            if depth > 0 {
                collect_osu(&p, depth - 1, out);
            }
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("osu")) {
            out.push(p);
        }
    }
}

/// A map as the last scan read it.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct CachedMap {
    path: PathBuf,
    meta: Meta,
    size: u64,
    modified: u64,
}

/// A set folder as the last scan saw it: when it last changed (ns since 1970) and its maps, every
/// one read (not only the playable ones, so a mania set isn't read again either).
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedFolder {
    dir: PathBuf,
    modified: u128,
    maps: Vec<CachedMap>,
}

fn library_cache() -> PathBuf {
    cache_dir().join("library.json")
}

fn load_cache(path: &Path) -> std::collections::HashMap<PathBuf, CachedFolder> {
    let folders: Vec<CachedFolder> = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    folders.into_iter().map(|f| (f.dir.clone(), f)).collect()
}

fn save_cache(path: &Path, folders: Vec<CachedFolder>) {
    let Ok(json) = serde_json::to_vec(&folders) else { return };
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
    let tmp = path.with_extension("partial");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Finds playable difficulties under `inputs`. `found` counts files read so far, for the UI.
///
/// A Songs folder of tens of thousands of maps took over a minute just to list on a hard drive,
/// every launch. So each set folder's maps are kept in a cache with the folder's modified time,
/// and a folder whose time hasn't moved isn't opened again: adding, removing or renaming a file
/// in it moves it.
// ponytail: a .osu rewritten in place (same name) doesn't move its folder's time, so its new
// header shows after the folder next changes or the cache file is deleted.
pub fn scan(inputs: &[PathBuf], found: &AtomicUsize) -> Vec<Entry> {
    scan_cached(inputs, found, &library_cache())
}

fn scan_cached(inputs: &[PathBuf], found: &AtomicUsize, cache_path: &Path) -> Vec<Entry> {
    let mut cache = load_cache(cache_path);
    let mut kept: Vec<CachedFolder> = Vec::new();
    // Files to read, with the set folder (index into `walked`) they are cached under, if any.
    let mut files: Vec<(Option<usize>, PathBuf)> = Vec::new();
    let mut walked: Vec<(PathBuf, u128)> = Vec::new();
    let loose = |files: &mut Vec<(Option<usize>, PathBuf)>, dir: &Path| {
        let mut found = Vec::new();
        collect_osu(dir, 1, &mut found);
        files.extend(found.into_iter().map(|p| (None, p)));
    };
    for input in inputs {
        let ext = input.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("osu") if input.is_file() => files.push((None, input.clone())),
            Some("osz") if input.is_file() => match extract_osz(input) {
                Ok(dir) => loose(&mut files, &dir),
                Err(e) => eprintln!("{}: {e}", input.display()),
            },
            _ if input.is_dir() => {
                let Ok(rd) = std::fs::read_dir(input) else { continue };
                for e in rd.flatten() {
                    let p = e.path();
                    if is_dir(&e) {
                        // The folder's own time: the copy Windows lists with the parent lags it.
                        let modified = std::fs::metadata(&p).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
                        match cache.remove(&p) {
                            Some(f) if f.modified == modified && modified != 0 => {
                                found.fetch_add(f.maps.len(), Ordering::Relaxed);
                                kept.push(f);
                            }
                            _ => {
                                let mut found = Vec::new();
                                collect_osu(&p, 1, &mut found);
                                files.extend(found.into_iter().map(|f| (Some(walked.len()), f)));
                                walked.push((p, modified));
                            }
                        }
                    } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("osu")) {
                        files.push((None, p));
                    } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("osz")) {
                        // A Songs folder: also pick up .osz files dropped into it.
                        if let Ok(dir) = extract_osz(&p) {
                            loose(&mut files, &dir);
                        }
                    }
                }
            }
            _ => eprintln!("skipping {}: not a .osu, .osz or folder", input.display()),
        }
    }
    files.sort_by(|a, b| a.1.cmp(&b.1));
    files.dedup_by(|a, b| a.1 == b.1);

    let read = |(dir, path): &(Option<usize>, PathBuf)| -> Option<(Option<usize>, CachedMap)> {
        // One malformed map must not take the whole scan down with it.
        let meta = std::panic::catch_unwind(|| beatmap::read_meta_file(path)).ok().flatten();
        found.fetch_add(1, Ordering::Relaxed);
        let (size, modified) = file_stats(path);
        Some((*dir, CachedMap { path: path.clone(), meta: meta?, size, modified }))
    };
    // Reading headers is I/O-bound: spread it over a thread per core.
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 16);
    let chunk = files.len().div_ceil(threads).max(1);
    let read: Vec<(Option<usize>, CachedMap)> = std::thread::scope(|s| {
        let handles: Vec<_> = files.chunks(chunk).map(|part| s.spawn(move || part.iter().filter_map(read).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });

    let mut fresh: Vec<CachedFolder> = walked.into_iter().map(|(dir, modified)| CachedFolder { dir, modified, maps: Vec::new() }).collect();
    let mut maps: Vec<CachedMap> = kept.iter().flat_map(|f| f.maps.iter().cloned()).collect();
    for (dir, map) in read {
        if let Some(d) = dir {
            fresh[d].maps.push(map.clone());
        }
        maps.push(map);
    }
    // Folders of other scans (another Songs folder, a single map) stay cached; ones gone from
    // a folder just scanned are dropped.
    let scanned = |dir: &Path| dir.parent().is_some_and(|p| inputs.iter().any(|i| i == p));
    kept.extend(fresh);
    kept.extend(cache.into_values().filter(|f| !scanned(&f.dir)));
    save_cache(cache_path, kept);

    let mut entries: Vec<Entry> = maps
        .into_iter()
        .filter(|m| m.meta.is_playable())
        .map(|m| Entry { path: m.path, meta: m.meta, size: m.size, modified: m.modified, hash: OnceLock::new() })
        .collect();
    entries.sort_by(|a, b| {
        // Same-named songs from different folders stay separate sets.
        let key = |e: &Entry| (e.meta.artist.to_lowercase(), e.meta.title.to_lowercase(), e.path.parent().map(|p| p.to_path_buf()));
        key(a).cmp(&key(b)).then(a.size.cmp(&b.size))
    });
    entries
}

/// The same beatmap set: by set ID when both maps have one, else by artist, title and mapper.
fn same_set(a: &Meta, b: &Meta) -> bool {
    match (a.set_id.filter(|&id| id > 0), b.set_id.filter(|&id| id > 0)) {
        (Some(x), Some(y)) => x == y,
        _ => a.artist == b.artist && a.title == b.title && a.creator == b.creator,
    }
}

/// What unpacking an `.osz` (its difficulties: header and content hash) would add: how many of
/// its difficulties are neither in the list with the same content nor in `pending` (on their
/// way already), and the folder of the set if some of it is here.
pub fn osz_news<'a>(library: &'a [Entry], pending: &std::collections::HashSet<u64>, maps: &[(Meta, u64)]) -> (usize, Option<&'a Path>) {
    let mut home = None;
    let mut fresh = 0;
    for (meta, hash) in maps {
        let mut here = pending.contains(hash);
        for e in library.iter().filter(|e| same_set(&e.meta, meta)) {
            home = home.or(e.path.parent());
            // Only same-named difficulties get hashed: reading every file would take a while.
            here = here || (e.meta.version == meta.version && e.hash() == *hash);
        }
        fresh += usize::from(!here);
    }
    (fresh, home)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_osz_already_in_the_list_adds_nothing() {
        let root = std::env::temp_dir().join(format!("taiko-rs-news-{}", std::process::id()));
        let set = root.join("123 A - T");
        std::fs::create_dir_all(&set).unwrap();
        let meta = |version: &str, set_id| Meta { artist: "A".into(), title: "T".into(), creator: "c".into(), version: version.into(), set_id, ..Default::default() };
        std::fs::write(set.join("oni.osu"), b"oni").unwrap();
        let library = [Entry::new(set.join("oni.osu"), meta("Oni", Some(123)))];
        let hash = |b: &[u8]| crate::net::map_hash(b);
        let none = std::collections::HashSet::new();
        // The same file: nothing new, and the set's folder is known.
        assert_eq!(osz_news(&library, &none, &[(meta("Oni", Some(123)), hash(b"oni"))]), (0, Some(set.as_path())));
        // One more difficulty, and an updated Oni.
        let maps = [(meta("Oni", Some(123)), hash(b"oni v2")), (meta("Muzukashii", Some(123)), hash(b"muzu"))];
        assert_eq!(osz_news(&library, &none, &maps).0, 2);
        // Dropped twice: the first one is still unpacking.
        let pending = maps.iter().map(|m| m.1).collect();
        assert_eq!(osz_news(&library, &pending, &maps).0, 0);
        // Unsubmitted maps (no set ID) match by artist, title and mapper.
        assert_eq!(osz_news(&library, &none, &[(meta("Oni", None), hash(b"oni"))]), (0, Some(set.as_path())));
        assert_eq!(osz_news(&library, &none, &[(Meta { creator: "d".into(), ..meta("Oni", None) }, hash(b"oni"))]), (1, None));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_rescan_reads_only_the_folders_that_changed() {
        let root = std::env::temp_dir().join(format!("taiko-rs-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let songs = root.join("Songs");
        let osu = |title: &str, version: &str| format!("osu file format v14\n\n[General]\nMode: 1\n\n[Metadata]\nTitle:{title}\nVersion:{version}\n");
        for set in ["1 A", "2 B", "4 D"] {
            std::fs::create_dir_all(songs.join(set)).unwrap();
            std::fs::write(songs.join(set).join("oni.osu"), osu(set, "Oni")).unwrap();
        }
        let cache = root.join("library.json");
        let scan = || {
            let found = AtomicUsize::new(0);
            let mut v: Vec<(String, String)> = scan_cached(std::slice::from_ref(&songs), &found, &cache).into_iter().map(|e| (e.meta.title, e.meta.version)).collect();
            v.sort();
            v
        };
        assert_eq!(scan().len(), 3);
        // Rewritten in place, its folder unchanged: not read again (the cached header is shown).
        std::fs::write(songs.join("1 A").join("oni.osu"), osu("renamed", "Oni")).unwrap();
        // A new difficulty changes its folder, a new folder is read, a deleted one goes.
        std::fs::write(songs.join("2 B").join("muzu.osu"), osu("2 B", "Muzukashii")).unwrap();
        std::fs::create_dir_all(songs.join("3 C")).unwrap();
        std::fs::write(songs.join("3 C").join("kantan.osu"), osu("3 C", "Kantan")).unwrap();
        std::fs::remove_dir_all(songs.join("4 D")).unwrap();
        let names: Vec<(String, String)> = [("1 A", "Oni"), ("2 B", "Muzukashii"), ("2 B", "Oni"), ("3 C", "Kantan")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        assert_eq!(scan(), names);
        // And again, all from the cache.
        assert_eq!(scan(), names);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn picking_an_osu_install_scans_its_songs_folder() {
        let root = std::env::temp_dir().join(format!("taiko-rs-pick-{}", std::process::id()));
        let install = root.join("osu!");
        std::fs::create_dir_all(install.join("Songs")).unwrap();
        std::fs::write(install.join("osu!.db"), b"").unwrap();
        assert_eq!(picked_songs(&install), install.join("Songs"));
        // A Songs folder moved in osu!'s settings.
        std::fs::create_dir_all(root.join("Maps")).unwrap();
        std::fs::write(install.join("osu!.player.cfg"), format!("Skin = Default\nBeatmapDirectory = {}\n", root.join("Maps").display())).unwrap();
        assert_eq!(picked_songs(&install), root.join("Maps"));
        // Any other folder is scanned as it is.
        assert_eq!(picked_songs(&root.join("Maps")), root.join("Maps"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
